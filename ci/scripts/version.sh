#!/usr/bin/env bash
# Compute the effective release version for the current commit and print it to stdout.
#
# Source of truth = the committed repo-root VERSION file. It holds the human-owned
# constant MAJOR.MINOR.PATCH (1.0.0 today). You bump MINOR/MAJOR there by hand for an
# explicit 1.1.0 / 2.0.0; CI never edits or commits it.
#
# CI auto-increments only the PATCH, derived from the existing `v<MAJOR>.<MINOR>.*` git
# tags published by the release job (no commit-back, so the working tree stays clean):
#   - no tag in this series yet  -> use the VERSION base verbatim   (e.g. 1.0.0)
#   - tags exist                 -> highest published patch + 1     (1.0.0 -> 1.0.1 -> ...)
# Bumping VERSION's MINOR/MAJOR starts a fresh series, so the patch resets to that base.
#
# Idempotent: if HEAD already carries a `v<MAJOR>.<MINOR>.*` tag (a re-run of an
# already-released commit) that exact version is reused instead of bumping again.
#
# Platform-agnostic: tags are the only state it reads, so GitHub and GitLab agree.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

VERSION_FILE="$REPO_ROOT/VERSION"
[ -r "$VERSION_FILE" ] || die "missing VERSION file at $VERSION_FILE"
base="$(tr -d '[:space:]' < "$VERSION_FILE")"
case "$base" in
    [0-9]*.[0-9]*.[0-9]*) : ;;
    *) die "VERSION must be MAJOR.MINOR.PATCH (got '$base')" ;;
esac
major="${base%%.*}"; rest="${base#*.}"; minor="${rest%%.*}"; basepatch="${rest#*.}"
series="v$major.$minor."

# CI shallow clones frequently omit tags; pull them so the series is complete. Best
# effort — a dev running this offline still gets whatever tags are already local.
git -C "$REPO_ROOT" fetch --tags --quiet 2>/dev/null || true

# Re-run guard: a version already pinned to this commit wins — don't double-bump.
#
# Only pure-integer patches count, using the same filter as the series scan below. The
# `$series*` glob also matches PRE-RELEASE tags — for series `v1.0.` it matches
# `v1.0.5-rc.2` — and `sort -V` orders those AFTER `v1.0.5`, so an unfiltered guard hands
# an `-rc.N` string to the STABLE release job whenever an rc-tagged commit becomes the tip
# of `upstream` (a fast-forward promotion of a validated rc). That would publish the stable
# release as `v1.0.5-rc.2` and mark it Latest.
on_head=""
while IFS= read -r tag; do
    p="${tag#"$series"}"
    case "$p" in '' | *[!0-9]*) continue ;; esac
    on_head="$tag"
done < <(git -C "$REPO_ROOT" tag --points-at HEAD --list "$series*" 2>/dev/null | sort -V)
if [ -n "$on_head" ]; then
    printf '%s\n' "${on_head#v}"
    exit 0
fi

# Highest patch already published in this MAJOR.MINOR series.
maxpatch=-1
while IFS= read -r tag; do
    p="${tag#"$series"}"
    case "$p" in '' | *[!0-9]*) continue ;; esac
    [ "$p" -gt "$maxpatch" ] && maxpatch="$p"
done < <(git -C "$REPO_ROOT" tag --list "$series*" 2>/dev/null)

if [ "$maxpatch" -ge "$basepatch" ]; then
    patch=$((maxpatch + 1))
else
    patch="$basepatch"
fi
printf '%s.%s.%s\n' "$major" "$minor" "$patch"
