#!/usr/bin/env bash
# Same as scripts/package.ps1, for a POSIX shell (Git Bash / WSL / Linux / macOS).
#
#   bash scripts/package.sh            # build the wasm component and zip a plugin package
#   bash scripts/package.sh --source   # zip the repository sources instead
#
# Produces dist/<id>-<version>.zip in the runtime layout (plugin.toml,
# settings.schema.json, icon.svg, README.md, LICENSE, dist/plugin.wasm).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

id="$(sed -n 's/^id[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' plugin.toml | head -n1)"
version="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' plugin.toml | head -n1)"
[ -n "$id" ] && [ -n "$version" ] || { echo "plugin.toml: cannot read id/version" >&2; exit 1; }
echo "packaging $id $version"
mkdir -p dist

if [ "${1:-}" = "--source" ]; then
    src_zip="dist/$id-$version-source.zip"
    rm -f "$src_zip"
    if [ -d .git ]; then
        git archive --format=zip -v --prefix="$id-$version/" -o "$src_zip" HEAD
    else
        echo "not a git repository; zip the working tree instead" >&2
        (cd .. && 7z a -tzip "$root/$src_zip" "$(basename "$root")" \
            -xr'!target' -xr'!.git' -xr'!dist' -xr'!*.wasm' >/dev/null)
    fi
    echo "source package: $src_zip ($(wc -c <"$src_zip") bytes)"
    exit 0
fi

echo "cargo test (crates/jvm-params-core)"
(cd crates/jvm-params-core && cargo test --offline)

if ! rustup target list --installed | grep -qx wasm32-wasip2; then
    echo "installing the wasm32-wasip2 target"
    rustup target add wasm32-wasip2
fi

wasm="dist/plugin.wasm"
if command -v openidea-plugin >/dev/null 2>&1; then
    echo "openidea-plugin build"
    openidea-plugin build
else
    echo "openidea-plugin not on PATH; falling back to cargo build + copy" >&2
    echo "install it with: cargo install --locked --git https://github.com/l0ng-ai/openidea ide-host --bin openidea-plugin" >&2
    cargo build --release --target wasm32-wasip2
    built="target/wasm32-wasip2/release/openidea_plugin_jvm_params.wasm"
    [ -f "$built" ] || { echo "expected $built" >&2; exit 1; }
    mkdir -p dist
    cp "$built" "$wasm"
fi
[ -f "$wasm" ] || { echo "no wasm component at $wasm" >&2; exit 1; }
echo "wasm component: $(wc -c <"$wasm") bytes"

stage_root="target/package"
stage="$stage_root/$id"
rm -rf "$stage_root"
mkdir -p "$stage/dist"
for f in plugin.toml settings.schema.json icon.svg README.md LICENSE; do
    [ -f "$f" ] && cp "$f" "$stage/$f"
done
cp "$wasm" "$stage/dist/plugin.wasm"

mkdir -p dist
zip_path="dist/$id-$version.zip"
rm -f "$zip_path"
if command -v zip >/dev/null 2>&1; then
    (cd "$stage_root" && zip -qr "../$zip_path" "$id" -x '*.DS_Store')
elif command -v 7z >/dev/null 2>&1; then
    (cd "$stage_root" && 7z a -tzip "../$zip_path" "$id" >/dev/null)
else
    echo "neither zip nor 7z found" >&2
    exit 1
fi
echo "package: $zip_path ($(wc -c <"$zip_path") bytes)"
