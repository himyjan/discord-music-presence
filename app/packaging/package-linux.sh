#!/usr/bin/env bash
set -euo pipefail

version="$(awk -F '"' '/^version = / { print $2; exit }' app/Cargo.toml)"
assets="$RUNNER_TEMP/release-assets"
appdir="$RUNNER_TEMP/music-presence.AppDir"
mkdir -p "$assets" "$appdir/usr/bin" "$appdir/usr/share/applications" \
  "$appdir/usr/share/icons/hicolor/scalable/apps" "$appdir/usr/share/doc/music-presence"

binary="app/target/${TARGET}/release/music-presence"
cp "$binary" "$appdir/usr/bin/music-presence"
cp app/packaging/music-presence.desktop "$appdir/usr/share/applications/"
cp app/packaging/music-presence.svg "$appdir/usr/share/icons/hicolor/scalable/apps/"
cp app/README.md "$appdir/usr/share/doc/music-presence/README.md"

for format in deb rpm; do
  config="$RUNNER_TEMP/nfpm-${format}.yaml"
  sed "s/__VERSION__/${version}/g" "app/packaging/nfpm-${format}.yaml" > "$config"
  nfpm package \
    --config "$config" \
    --packager "$format" \
    --target "$assets/music-presence-${GITHUB_REF_NAME}-linux-x86_64.${format}"
done

linuxdeploy="$RUNNER_TEMP/linuxdeploy-x86_64.AppImage"
plugin="$RUNNER_TEMP/linuxdeploy-plugin-appimage-x86_64.AppImage"
curl --fail --location --silent --show-error --retry 3 \
  https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage \
  --output "$linuxdeploy"
curl --fail --location --silent --show-error --retry 3 \
  https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-x86_64.AppImage \
  --output "$plugin"
chmod +x "$linuxdeploy" "$plugin"

export APPIMAGE_EXTRACT_AND_RUN=1
export LINUXDEPLOY_OUTPUT_APP_NAME=music-presence
export LINUXDEPLOY_OUTPUT_VERSION="$version"
export LDAI_COMP=xz
export LDAI_OUTPUT="$assets/music-presence-${GITHUB_REF_NAME}-linux-x86_64.AppImage"
"$linuxdeploy" \
  --appdir "$appdir" \
  --executable "$appdir/usr/bin/music-presence" \
  --desktop-file "$appdir/usr/share/applications/music-presence.desktop" \
  --icon-file "$appdir/usr/share/icons/hicolor/scalable/apps/music-presence.svg" \
  --output appimage
