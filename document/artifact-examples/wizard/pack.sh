#!/bin/sh
# Build the wizard artifact and pack it into wizard.y5 (the importable extension).
# Stand-in for the future `y5-pack` tool, which will also STAMP the compat
# fingerprints from the actual build instead of trusting y5.manifest.json.
set -e
cd "$(dirname "$0")"

cargo build --release

rm -rf build wizard.y5
mkdir -p build/payload
cp target/release/liby5_artifact_wizard.so build/payload/plugin.so
cp y5.manifest.json build/y5.manifest.json

# ZIP with exact internal paths: y5.manifest.json + payload/plugin.so.
(cd build && python3 -m zipfile -c ../wizard.y5 y5.manifest.json payload/)

echo "packed: $(pwd)/wizard.y5"
echo "install: cp wizard.y5 ~/.local/share/y5/artifact/"
