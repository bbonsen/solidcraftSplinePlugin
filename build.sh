#!/usr/bin/env bash
# Builds dist/spline.zip, ready for SolidCraft's Preferences > Plugins > Install plugin…
# Needs: the Rust toolchain in rust-toolchain.toml (rustup installs it) and wasm-tools
# (cargo install wasm-tools --locked).
set -euo pipefail
cd "$(dirname "$0")"
cargo test --quiet
cargo build --release --target wasm32-unknown-unknown
mkdir -p package/spline dist
wasm-tools component new target/wasm32-unknown-unknown/release/solidcraft_spline.wasm -o package/spline/spline.wasm
wasm-tools validate package/spline/spline.wasm
python3 tools/manifest.py package/spline/plugin.json
rm -f dist/spline.zip
(cd package && python3 -c "
import zipfile
with zipfile.ZipFile('../dist/spline.zip', 'w', zipfile.ZIP_DEFLATED) as z:
    for f in ['spline/plugin.json', 'spline/spline.wasm']:
        z.write(f)
")
echo "built dist/spline.zip"
