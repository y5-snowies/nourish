#!/usr/bin/env bash
# Discover y5 "workspace entries": the roots declared in workspace.catalog.json.
#
# That file is the authority for what a workspace root IS — it drives manifest
# generation — so reading it here means CI's matrix cannot drift from the tree.
#
# It used to look for directories containing both a Cargo.toml and a link.json.
# Both halves of that test are now wrong: Cargo.toml is a generated artifact and
# is absent from a fresh checkout until `workspace.generate.js` runs, and the
# link.json marker was already stale — four roots never had one, so CI silently
# skipped them.
#
# Output (stdout):
#   default     compact JSON array, e.g. ["compositor","compositor.loader",...]
#               (consumed by the GitHub matrix via fromJson and by gen-child-pipeline.sh)
#   --lines     one entry path per line (for shell `while read` loops)
#
# Paths are relative to the repo root.

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

cd "$REPO_ROOT"

mode="${1:-json}"

command -v node >/dev/null 2>&1 || die "node is required to read workspace.catalog.json"

entries=()
while IFS= read -r dir; do
    entries+=("$dir")
done < <(node -e '
const jsonc = require("./compositor.workspace/jsonc.js");
for (const r of Object.keys(jsonc.read("compositor.workspace/workspace.catalog.json").roots).sort()) console.log(r);
')

[ "${#entries[@]}" -gt 0 ] || die "no workspace entries found in workspace.catalog.json"

case "$mode" in
    --lines)
        printf '%s\n' "${entries[@]}"
        ;;
    json)
        # Build a JSON array by hand (entry paths are simple, no escaping needed) so the
        # script has zero runtime dependencies.
        out="["; sep=""
        for e in "${entries[@]}"; do out+="$sep\"$e\""; sep=","; done
        out+="]"
        printf '%s\n' "$out"
        ;;
    *)
        die "usage: discover-workspaces.sh [--lines|json]"
        ;;
esac
