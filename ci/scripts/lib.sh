#!/usr/bin/env bash
# Shared helpers for the y5 CI scripts. Source this; do not execute it.
#
# Everything here is platform-agnostic. The only platform awareness in the whole
# ci/scripts/ tree is the trio of `is_github` / `is_gitlab` predicates below, used by
# the few scripts that must post PR/MR notes or open a promotion request.

set -euo pipefail

# Repo root = two levels up from ci/scripts/. Overridable for tests.
y5_repo_root() {
    if [ -n "${Y5_REPO_ROOT:-}" ]; then printf '%s\n' "$Y5_REPO_ROOT"; return; fi
    cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd
}

REPO_ROOT="$(y5_repo_root)"

log()  { printf '>> %s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

is_github() { [ "${GITHUB_ACTIONS:-}" = "true" ]; }
is_gitlab() { [ -n "${GITLAB_CI:-}" ]; }

# In CI the checkout is owned by a different uid than the container user, so git refuses the
# work tree ("dubious ownership", sometimes surfacing as "not a git repository"). Trust it
# once, here, before any git-using ci script runs — covers link-drift, gen-report,
# open-promotion-pr, package-release, doc-suggest. CI-only so it never touches a dev's config.
if is_github || is_gitlab; then
    git config --global --add safe.directory "$REPO_ROOT" 2>/dev/null || true
fi

# Directory that owns the y5_compositor [[bin]] (rename-proof: keyed on the bin name),
# printed relative to REPO_ROOT.
y5_bin_crate_dir() {
    local abs
    abs="$(dirname "$(grep -rl --include=Cargo.toml 'name *= *"y5_compositor"' \
        "$REPO_ROOT"/compositor* | head -n1)")"
    [ -n "$abs" ] || die "could not find the y5_compositor crate"
    printf '%s\n' "${abs#"$REPO_ROOT"/}"
}

# Workspace-root dir for a given path (nearest ancestor whose Cargo.toml has [workspace]).
y5_workspace_root_of() {
    local d="$1"
    while [ "$d" != "/" ] && ! grep -qs '^\[workspace\]' "$d/Cargo.toml"; do
        d="$(dirname "$d")"
    done
    printf '%s\n' "$d"
}
