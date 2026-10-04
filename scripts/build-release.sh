#!/usr/bin/env bash
# Build all Linux release assets. Run on x86_64 Debian/Ubuntu.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
    echo 'This script requires x86_64 Linux.' >&2
    exit 1
fi
for tool in cargo rustup cc pkg-config curl tar dpkg-deb python3; do
    command -v "$tool" >/dev/null || { echo "Missing prerequisite: $tool (see README.md)" >&2; exit 1; }
done
pkg-config --exists wayland-client xkbcommon || { echo 'Install libwayland-dev and libxkbcommon-dev first (see README.md).' >&2; exit 1; }

target=x86_64-unknown-linux-gnu
# Pin the output location even if CARGO_TARGET_DIR is set by the caller.
export CARGO_TARGET_DIR="$PWD/target"
version="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "firmware-gui"))')"
out="$PWD/target/release-assets/$target"
name="snout-v$version-$target"
mkdir -p "$out"
rustup target add "$target"
if ! cargo deb --version >/dev/null 2>&1; then
    cargo install cargo-deb --locked
fi
cargo build --locked --release --workspace --target "$target"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/$name"
cp "target/$target/release/firmware-gui" "target/$target/release/firmware-explorer" README.md LICENSE "$stage/$name/"
tar czf "$out/$name.tar.gz" -C "$stage" "$name"
cp "target/$target/release/firmware-gui" "$out/$name"
cargo deb -p firmware-gui --target "$target" --no-build --output "$out/snout_${version}-1_amd64.deb"
bash crates/firmware-gui/packaging/linux/build-appimage.sh \
    "$PWD/target/$target/release/firmware-gui" "$version" "$out"
printf '\nRelease assets: %s\n' "$out"
