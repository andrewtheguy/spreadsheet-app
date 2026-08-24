#!/usr/bin/env bash
# Assembles `spreadsheet-app.app` (and optionally a .dmg) around already-built binaries.
#
# Tauri used to produce the bundle; with the Slint rewrite there is no bundler in the tree, so
# this script does the (small) job by hand. It takes one or more binaries and lipos them into a
# universal executable, which is what lets one release artifact cover Apple silicon and Intel.
#
#   scripts/bundle-macos.sh OUT_DIR BINARY [BINARY...]
#
# e.g. scripts/bundle-macos.sh target/macos \
#        target/aarch64-apple-darwin/release/spreadsheet-app \
#        target/x86_64-apple-darwin/release/spreadsheet-app
set -euo pipefail

if [ "$#" -lt 2 ]; then
    echo "usage: $0 OUT_DIR BINARY [BINARY...]" >&2
    exit 2
fi

out_dir=$1
shift
repo_root=$(cd "$(dirname "$0")/.." && pwd)
app="$out_dir/spreadsheet-app.app"

# Take the version from Cargo.toml so the bundle can't drift from the crate.
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo_root/Cargo.toml" | head -1)
if [ -z "$version" ]; then
    echo "could not read version from Cargo.toml" >&2
    exit 1
fi

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

# `lipo -create` accepts a single input too, so the one-arch and universal cases share a path.
lipo -create "$@" -output "$app/Contents/MacOS/spreadsheet-app"
chmod +x "$app/Contents/MacOS/spreadsheet-app"

sed "s/__VERSION__/$version/g" "$repo_root/packaging/macos/Info.plist" \
    > "$app/Contents/Info.plist"
cp "$repo_root/icons/icon.icns" "$app/Contents/Resources/icon.icns"

# Ad-hoc signature. Without it, arm64 macOS refuses to launch the binary at all; it is not a
# substitute for a Developer ID signature, so users still get the Gatekeeper prompt.
codesign --force --deep --sign - "$app"

echo "built $app (version $version)"

if [ "${MAKE_DMG:-}" = "1" ]; then
    dmg="$out_dir/spreadsheet-app-$version.dmg"
    staging=$(mktemp -d)
    cp -R "$app" "$staging/"
    ln -s /Applications "$staging/Applications"
    rm -f "$dmg"
    hdiutil create -volname "spreadsheet-app" -srcfolder "$staging" -ov -format UDZO "$dmg"
    rm -rf "$staging"
    echo "built $dmg"
fi
