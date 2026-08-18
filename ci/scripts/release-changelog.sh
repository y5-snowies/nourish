#!/usr/bin/env bash
# Print the markdown "changes since the previous release" section for a release body.
#
# Two things make this more than `git log <prev>..HEAD`:
#
#   REBASE SAFETY. The release branches are rebased, so the previous release's tag often
#   points at a commit that is no longer an ancestor of HEAD — its content is in the
#   history under different SHAs. A two-dot range would then re-list the entire rebased
#   series as "new". This uses the symmetric difference with `--cherry-pick`, which
#   compares PATCH IDS rather than SHAs: a commit that was merely rewritten is recognised
#   on both sides and dropped, and only genuinely new work survives. A rebase that also
#   CHANGED a commit's content leaves it listed, which is correct — the content is new.
#
#   RUN COLLAPSING. This history routinely carries consecutive commits with a byte-identical
#   subject ("add suspend protocol" twice in a row, "introspection sampler optimization"
#   twice). Listing those separately reads like the same thing shipped twice, so a run of
#   equal subjects becomes ONE line carrying every commit link, comma separated. Only
#   ADJACENT commits collapse: two identical subjects with unrelated work between them are
#   two separate events and stay two lines.
#
# Platform-agnostic like the rest of ci/scripts: git tags are the only state read, and the
# commit URL is derived from CI env when present, else from the `origin` remote.
#
# Usage:
#   release-changelog.sh [--current <tag>] [--previous <tag>] [--rc] [--limit <n>]
#     --current   tag being created now; excluded when auto-picking the previous one
#     --previous  force the comparison base instead of auto-picking
#     --rc        consider `v*-rc.*` tags as candidates for the base (rc channel).
#                 Default (stable channel) compares against the previous STABLE tag, so a
#                 stable release lists everything since the last stable, not since an rc.
#     --limit     cap the number of lines (default 200), with a tail line for the rest
#
# Never fails a release: anything unresolvable degrades to a short honest note on stdout.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

current=""; previous=""; include_rc=0; limit=200
while [ $# -gt 0 ]; do
    case "$1" in
        --current)  current="${2:-}"; shift 2 ;;
        --previous) previous="${2:-}"; shift 2 ;;
        --rc)       include_rc=1; shift ;;
        --limit)    limit="${2:-}"; shift 2 ;;
        *) die "unknown argument '$1'" ;;
    esac
done
case "$limit" in '' | *[!0-9]*) die "--limit must be a number (got '$limit')" ;; esac

cd "$REPO_ROOT"

# Tags are the only state; a shallow CI clone usually lacks them. Best effort, as elsewhere.
git fetch --tags --quiet 2>/dev/null || true

# --- the comparison base ---------------------------------------------------------------
# Highest version-sorted `v*` tag that is neither the release being created nor already on
# HEAD (a re-run of an already-tagged commit). `sort -V` puts `v1.4.4-rc.1` below `v1.4.4`,
# which is what we want in both channels.
if [ -z "$previous" ]; then
    on_head="$(git tag --points-at HEAD --list 'v*' 2>/dev/null || true)"
    while IFS= read -r tag; do
        [ -n "$tag" ] || continue
        [ "$tag" = "$current" ] && continue
        printf '%s\n' "$on_head" | grep -qxF "$tag" && continue
        if [ "$include_rc" -eq 0 ]; then
            case "$tag" in *-rc.*) continue ;; esac
        fi
        previous="$tag"
        break
    done < <(git tag --list 'v*' 2>/dev/null | sort -V -r)
fi

if [ -z "$previous" ]; then
    log "no previous release tag found — emitting a first-release note"
    printf '## Changes\n\nFirst tracked release — there is no previous tag to compare against.\n'
    exit 0
fi
if ! git rev-parse --verify --quiet "$previous^{commit}" >/dev/null; then
    log "previous tag '$previous' is not in this clone (shallow fetch?) — skipping the list"
    printf '## Changes since %s\n\nCommit list unavailable: `%s` was not fetched into the build clone.\n' \
        "$previous" "$previous"
    exit 0
fi

# --- commit URL base --------------------------------------------------------------------
commit_base=""
if [ -n "${GITHUB_SERVER_URL:-}" ] && [ -n "${GITHUB_REPOSITORY:-}" ]; then
    commit_base="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/commit"
elif [ -n "${CI_PROJECT_URL:-}" ]; then
    commit_base="$CI_PROJECT_URL/-/commit"
else
    remote="$(git remote get-url origin 2>/dev/null || true)"
    case "$remote" in
        git@*:*) remote="https://${remote#git@}"; remote="${remote/:/\/}" ;;
    esac
    remote="${remote%.git}"
    [ -n "$remote" ] && commit_base="$remote/commit"
fi

# --- the commits ------------------------------------------------------------------------
# `A...B --right-only --cherry-pick` = "in B, and not patch-equivalent to anything in A".
# --no-merges because a merge has no patch id to compare and adds nothing to a subject list.
printf '## Changes since %s\n\n' "$previous"

emitted=0; total=0; run_subject=""; run_links=""

flush_run() {
    [ -n "$run_subject" ] || return 0
    if [ "$emitted" -lt "$limit" ]; then
        printf -- '- %s — %s\n' "$run_subject" "$run_links"
        emitted=$((emitted + 1))
    fi
    run_subject=""; run_links=""
}

while IFS=$'\t' read -r sha short subject; do
    [ -n "$sha" ] || continue
    total=$((total + 1))
    # `%s` is the whole first PARAGRAPH with newlines flattened, and some commits here carry
    # a dozen sentences before the first blank line. Trim to a headline; the link has the rest.
    subject="${subject#"${subject%%[![:space:]-]*}"}"   # drop leading spaces / list dashes
    if [ "${#subject}" -gt 100 ]; then
        subject="$(printf '%.100s' "$subject")"
        subject="${subject% *}…"
    fi
    if [ -n "$commit_base" ]; then
        link="[\`$short\`]($commit_base/$sha)"
    else
        link="\`$short\`"
    fi
    if [ "$subject" = "$run_subject" ]; then
        # Adjacent duplicate: same line, one more link.
        run_links="$run_links, $link"
    else
        flush_run
        run_subject="$subject"; run_links="$link"
    fi
done < <(git log --right-only --cherry-pick --no-merges \
             --format='%H%x09%h%x09%s' "$previous...HEAD" 2>/dev/null || true)
flush_run

if [ "$total" -eq 0 ]; then
    printf 'No new commits since %s.\n' "$previous"
elif [ "$emitted" -ge "$limit" ]; then
    printf '\n…and more — %s commits in total. See the full range on the compare view.\n' "$total"
fi
