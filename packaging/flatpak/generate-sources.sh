#!/usr/bin/env bash
# Generates cargo-sources.json (the Cargo.lock dependencies, to build offline in Flatpak)
# with flatpak-cargo-generator.py, from https://github.com/flatpak/flatpak-builder-tools.
# Requires: python with aiohttp and tomlkit (on Arch: sudo pacman -S python-aiohttp python-tomlkit).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

curl -fsSL -o "$work/flatpak-cargo-generator.py" \
  https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py

python3 "$work/flatpak-cargo-generator.py" "$root/Cargo.lock" -o "$here/cargo-sources.json"
echo "Generated: $here/cargo-sources.json"
