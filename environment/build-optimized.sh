#!/usr/bin/env bash
# Build the SHIPPED y5_compositor: the fat-LTO `release` profile.
#
# Usage: ./build-optimized.sh [winit|udev|native] [--deploy[=NAME]]
#   Takes exactly the arguments build.sh takes — this only swaps the profile.
#
# For release tags and published artifacts. NOT for iteration: fat LTO with one
# codegen unit optimizes across all ~1400 units and the final link is serial, so a
# clean build is 10-20 minutes (see the measurements on `[profile.release]` in
# compositor.kernel/kernel.loader/Cargo.toml). Day to day, `build.sh` gives you the
# same optimizations without LTO in a fraction of the time.
#
# Writes to target/release/, which is a different tree from build.sh's
# target/release-fast/ — the two never invalidate each other.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Answer --help here: forwarding it would print build.sh's, which says release-fast.
for arg in "$@"; do
    case "$arg" in
        -h|--help) sed -n '2,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        # NATIVE ONLY. winit is the nested dev backend — it runs inside an existing
        # Wayland session and is never a shipped artifact, so spending a 12-minute
        # serial fat-LTO link on one buys nothing and costs CI minutes we are capped
        # on. Refused rather than merely discouraged, so no future call site can
        # quietly start paying for it.
        winit)
            echo "build-optimized.sh: winit is the nested DEV backend and is never shipped." >&2
            echo "                    Fat LTO on it wastes a ~12 min serial link for nothing." >&2
            echo "                    Use: environment/build.sh winit   (release-fast)" >&2
            exit 1 ;;
    esac
done

exec env Y5_PROFILE=release "$HERE/build.sh" "$@"
