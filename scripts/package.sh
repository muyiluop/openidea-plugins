#!/usr/bin/env bash
# Builds one plugin of this repository and packages it into a shareable zip.
#
#   ./scripts/package.sh jvm-default-parameters
#   ./scripts/package.sh                    # works when there is exactly one plugin
#
# Steps: unit tests of the plugin's SDK-free core crate -> wasm component ->
# staging directory in the runtime layout -> <plugin>/dist/<id>-<version>.zip
# plus a copy in <repo>/release/.
#
# The zip is a plain plugin directory: extract it into
#   ~/.local/share/ai.l0ng.openidea/plugins/<anything>/   (Linux)
#   ~/Library/Application Support/ai.l0ng.openidea/plugins/<anything>/   (macOS)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
plugin="${1:-}"

# 0. Find the plugin: every repository folder with a plugin.toml is one.
mapfile -t plugins < <(cd "$root" && for d in */; do
  d="${d%/}"
  [ -f "$d/plugin.toml" ] && echo "$d"
done)
if [ -z "$plugin" ]; then
  if [ "${#plugins[@]}" -eq 1 ]; then
    plugin="${plugins[0]}"
  elif [ "${#plugins[@]}" -eq 0 ]; then
    echo "no plugin found: a plugin is a repository folder containing plugin.toml" >&2
    exit 1
  else
    echo "more than one plugin; pass one of: ${plugins[*]}" >&2
    exit 1
  fi
fi
dir="$root/$plugin"
[ -f "$dir/plugin.toml" ] || { echo "$plugin: no plugin.toml" >&2; exit 1; }

cd "$dir"
manifest="$(cat plugin.toml)"
id="$(printf '%s\n' "$manifest" | sed -n 's/^id *= *"\([^"]*\)".*/\1/p' | head -n1)"
version="$(printf '%s\n' "$manifest" | sed -n 's/^version *= *"\([^"]*\)".*/\1/p' | head -n1)"
[ -n "$id" ] && [ -n "$version" ] || { echo "plugin.toml: cannot read id/version" >&2; exit 1; }
echo "packaging $plugin -> $id $version"

# 1. Tests of the SDK-free core crate (it has its own workspace, no SDK needed).
if [ -d crates/jvm-params-core ]; then
  echo "cargo test (crates/jvm-params-core)"
  (cd crates/jvm-params-core && cargo test --offline)
fi

# 2. Build the wasm component.
if command -v openidea-plugin >/dev/null 2>&1; then
  echo "openidea-plugin build"
  openidea-plugin build
else
  echo "warning: openidea-plugin not on PATH; falling back to cargo build" >&2
  cargo build --release --target wasm32-wasip2
  crate="$(sed -n 's/^name *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n1)"
  mkdir -p dist
  cp "target/wasm32-wasip2/release/${crate//-/_}.wasm" dist/plugin.wasm
fi
[ -f dist/plugin.wasm ] || { echo "no wasm component at $plugin/dist/plugin.wasm" >&2; exit 1; }
echo "wasm component: $(wc -c < dist/plugin.wasm) bytes"

# 3. Stage the runtime layout: exactly what the plugin needs at run time.
stage="$dir/target/package/$id"
rm -rf "$dir/target/package"
mkdir -p "$stage/dist"
for f in plugin.toml settings.schema.json icon.svg README.md; do
  [ -f "$f" ] && cp "$f" "$stage/"
done
[ -f "$root/LICENSE" ] && cp "$root/LICENSE" "$stage/"
cp dist/plugin.wasm "$stage/dist/plugin.wasm"

# 4. Zip the stage directory itself (the archive root is the plugin folder).
mkdir -p "$dir/dist"
zip_path="$dir/dist/$id-$version.zip"
rm -f "$zip_path"
(cd "$stage/.." && zip -qr "$zip_path" "$id")
echo "package: $zip_path ($(wc -c < "$zip_path") bytes)"

# 5. Keep a committed copy in release/.
mkdir -p "$root/release"
cp "$zip_path" "$root/release/"
echo "release copy: $root/release/$id-$version.zip"
