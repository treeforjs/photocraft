#!/usr/bin/env bash
# Exercise the real staging/tar path without compiling Rust or downloading tools.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Model a checkout cloned under umask 077, without changing the real checkout.
FIXTURE="$TMP/checkout"
mkdir -p "$FIXTURE/packaging" "$FIXTURE/assets/app-icon"
cp -R "$HERE" "$FIXTURE/packaging/linux"
cp "$HERE/../env.sh" "$FIXTURE/packaging/env.sh"
cp -R "$ROOT/assets/app-icon/hicolor" "$FIXTURE/assets/app-icon/"
cp "$ROOT/Cargo.toml" "$ROOT/README.md" "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$FIXTURE/"
find "$FIXTURE" -type d -exec chmod 700 {} +
find "$FIXTURE" -type f -exec chmod 600 {} +
PACKAGE="$FIXTURE/packaging/linux/package.sh"
chmod 755 "$PACKAGE"

mkdir -p "$TMP/target/release"
for binary in photocraft photocraft-cli; do
  printf '#!/bin/sh\nprintf "photocraft 0.0.0\\n"\n' >"$TMP/target/release/$binary"
  chmod 755 "$TMP/target/release/$binary"
done

# A developer's private umask must not make a system-wide package unreadable to
# other users. Validate archive metadata, not just the build user's file access.
(
  umask 077
  CARGO_TARGET_DIR="$TMP/target" DIST="$TMP/dist" PHOTOCRAFT_VERSION=0.0.0 \
    "$PACKAGE" --skip-build --formats tar >"$TMP/package.log" 2>&1
) || { cat "$TMP/package.log" >&2; exit 1; }

archive=("$TMP"/dist/*.tar.gz)
tar -tvzf "${archive[0]}" >"$TMP/contents"
tar -tzf "${archive[0]}" >"$TMP/paths"
awk '
  substr($1, 1, 1) == "d" && $1 != "drwxr-xr-x" { print "bad directory permissions: " $0; bad = 1 }
  substr($1, 1, 1) == "-" && substr($1, 8, 1) != "r" { print "file not world-readable: " $0; bad = 1 }
  END { exit bad }
' "$TMP/contents"
for path in bin/photocraft bin/photocraft-cli share/applications/ai.storyteller.photocraft.desktop \
  share/metainfo/ai.storyteller.photocraft.metainfo.xml share/icons/hicolor/256x256/apps/ai.storyteller.photocraft.png; do
  grep -Fq "/$path" "$TMP/paths"
done
echo "Linux package staging and permissions ok"

if command -v nfpm >/dev/null && command -v readelf >/dev/null && command -v dpkg-deb >/dev/null; then
  # Real ELF fixtures exercise nFPM's dependency substitution and both binaries'
  # symbol requirements without an expensive app build. They are never shipped.
  cp /bin/bash "$TMP/target/release/photocraft"
  cp /bin/true "$TMP/target/release/photocraft-cli"
  CARGO_TARGET_DIR="$TMP/target" DIST="$TMP/dist" PHOTOCRAFT_VERSION=0.0.0 \
    "$PACKAGE" --skip-build --formats deb >"$TMP/deb.log" 2>&1 || { cat "$TMP/deb.log" >&2; exit 1; }
  packages=("$TMP"/dist/*.deb)
  dependencies="$(dpkg-deb --field "${packages[0]}" Depends)"
  # Both loaders are needed: Vulkan is the default and safe-gpu uses OpenGL.
  # An alternative dependency can leave the fallback's EGL loader uninstalled.
  for loader in libvulkan1 libegl1; do
    printf '%s\n' "$dependencies" | tr ',' '\n' | grep -Eq "^[[:space:]]*${loader}[[:space:]]*$"
  done
  minimum="$(printf '%s\n' "$dependencies" | sed -n 's/.*libc6 (>= \([0-9.]*\)).*/\1/p')"
  [ -n "$minimum" ] || { echo "Debian package has no glibc minimum" >&2; exit 1; }
  dpkg --compare-versions "$minimum" ge 2.35
  LC_ALL=C readelf --version-info /bin/bash /bin/true >"$TMP/versions"
  while IFS= read -r required; do
    dpkg --compare-versions "$minimum" ge "$required"
  done < <(sed -n 's/.*Name: GLIBC_\([0-9][0-9.]*\).*/\1/p' "$TMP/versions")
  dpkg-deb --contents "${packages[0]}" >"$TMP/deb-contents"
  awk '
    substr($1, 1, 1) == "d" && $1 != "drwxr-xr-x" { print "bad Debian directory permissions: " $0; bad = 1 }
    substr($1, 1, 1) == "-" && substr($1, 8, 1) != "r" { print "Debian file not world-readable: " $0; bad = 1 }
    END { exit bad }
  ' "$TMP/deb-contents"
  echo "Debian permissions and ELF-derived glibc dependency ok ($minimum)"
else
  echo "Debian metadata check skipped: install nfpm, binutils and dpkg"
fi

# Exercise AppDir assembly without downloading appimagetool or building an image.
# Top-level integration assets must be readable too, not just usr/share copies.
cat >"$TMP/appimagetool" <<'TOOL'
#!/usr/bin/env bash
set -euo pipefail
appdir="${@: -2:1}"
output="${@: -1}"
for asset in ai.storyteller.photocraft.desktop ai.storyteller.photocraft.png; do
  [ "$(stat -c %a "$appdir/$asset")" = 644 ] || { echo "private AppDir asset: $asset" >&2; exit 1; }
done
printf 'test AppImage\n' >"$output"
printf 'test zsync\n' >"$output.zsync"
TOOL
chmod 755 "$TMP/appimagetool"
CARGO_TARGET_DIR="$TMP/target" DIST="$TMP/dist" PHOTOCRAFT_VERSION=0.0.0 APPIMAGETOOL="$TMP/appimagetool" \
  "$PACKAGE" --skip-build --formats appimage >"$TMP/appimage.log" 2>&1 || { cat "$TMP/appimage.log" >&2; exit 1; }
echo "AppDir integration asset permissions ok"
