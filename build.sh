#!/usr/bin/env bash
set -euo pipefail

# VideoDownloader — 100% Pure Rust Build & Release Packaging Script
# Built natively with eframe/egui + tokio + reqwest (no Node.js, npm, or Tauri required)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

APP_NAME="VideoDownloader"
BIN_NAME="videodownloader"
RELEASE_DIR="$SCRIPT_DIR/dist-release"
BUILD_WINDOWS=false
BUILD_APPIMAGE=false
BUILD_DEB=false
ASK_PACKAGE=true

for arg in "$@"; do
  case "$arg" in
    --windows) BUILD_WINDOWS=true ;;
    --appimage)
      BUILD_APPIMAGE=true
      ASK_PACKAGE=false
      ;;
    --deb)
      BUILD_DEB=true
      ASK_PACKAGE=false
      ;;
    --both)
      BUILD_APPIMAGE=true
      BUILD_DEB=true
      ASK_PACKAGE=false
      ;;
    --no-package)
      ASK_PACKAGE=false
      ;;
    --help|-h)
      echo "Használat / Usage: ./build.sh [opciók]"
      echo "  --windows     Windows (x86_64-pc-windows-msvc) bináris + NSIS telepítő készítése"
      echo "  --appimage    AppImage csomag készítése rákérdezés nélkül"
      echo "  --deb         Debian/Ubuntu (.deb) csomag készítése rákérdezés nélkül"
      echo "  --both        AppImage és .deb csomag készítése egyszerre"
      echo "  --no-package  Csak bináris fordítása, csomagolási kérdés nélkül"
      echo ""
      echo "Opcionális Windows digitális aláírás (Authenticode, osslsigncode szükséges):"
      echo "  SIGN_PFX=/utvonal/cert.pfx SIGN_PASS=jelszo ./build.sh --windows"
      exit 0
      ;;
  esac
done

APP_VERSION="$(grep -m1 '^version' "$SCRIPT_DIR/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/' || echo "2.3.0")"

echo "==> [1/4] Rust fordítási eszközök ellenőrzése..."
for cmd in rustc cargo; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "HIBA: Hiányzó eszköz: $cmd. Telepítse a Rust eszközkészletet (rustup) a folytatás előtt."
    exit 1
  fi
done

if ! command -v tar >/dev/null 2>&1; then
  echo "FIGYELEM: A 'tar' parancs nem található (Linuxon az alkalmazáson belüli ffmpeg kicsomagoláshoz szükséges)."
fi

chmod +x "$SCRIPT_DIR/uninstall.sh" "$SCRIPT_DIR/install.sh"

echo "==> [2/4] 100% natív Rust alkalmazás fordítása (release)..."
cargo build --release

if [ "$BUILD_WINDOWS" = true ]; then
  echo "==> [3/4] Windows (x86_64-pc-windows-msvc) keresztfordítás..."
  if [ -d "$HOME/.cargo/bin" ] && [[ ":$PATH:" != *":$HOME/.cargo/bin:"* ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
  fi
  if ! command -v llvm-rc >/dev/null 2>&1; then
    for llvm_rc_candidate in /usr/lib/llvm*/bin/llvm-rc /usr/local/opt/llvm/bin/llvm-rc /opt/homebrew/opt/llvm/bin/llvm-rc; do
      if [ -x "$llvm_rc_candidate" ]; then
        export RC_PATH="$llvm_rc_candidate"
        export PATH="$(dirname "$llvm_rc_candidate"):$PATH"
        break
      fi
    done
  fi
  if ! command -v llvm-rc >/dev/null 2>&1 && [ -z "${RC_PATH:-}" ]; then
    echo "FIGYELEM: Az 'llvm-rc' nem található a PATH-ban (a Windows ikon és verzió-metaadat beágyazásához szükséges)."
    echo "  Telepítés: sudo apt install llvm   vagy   sudo pacman -S llvm   vagy   sudo dnf install llvm"
  fi
  if command -v cargo-xwin >/dev/null 2>&1; then
    if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -q "x86_64-pc-windows-msvc"; then
      rustup target add x86_64-pc-windows-msvc
    fi
    cargo xwin build --release --target x86_64-pc-windows-msvc
  else
    echo "HIBA: Linux alatti Windows keresztfordításhoz szükséges:"
    echo "  1) cargo install cargo-xwin"
    echo "  2) rustup target add x86_64-pc-windows-msvc"
    echo "  3) clang + llvm + lld + nsis (Arch: sudo pacman -S --needed clang llvm lld nsis | Ubuntu/Debian: sudo apt install clang lld nsis | Fedora: sudo dnf install clang lld mingw32-nsis)"
    exit 1
  fi
else
  echo "==> [3/4] Natív Linux célfordítás kész (Windows kihagyva; használja a --windows kapcsolót)."
fi

sign_windows_pe() {
  local target_file="$1"
  local display_name="$2"
  if [ -z "${SIGN_PFX:-}" ]; then
    return 0
  fi
  if [ ! -f "$SIGN_PFX" ]; then
    echo "FIGYELEM: A SIGN_PFX fájl nem található ($SIGN_PFX), aláírás kihagyva."
    return 0
  fi
  if ! command -v osslsigncode >/dev/null 2>&1; then
    echo "FIGYELEM: A SIGN_PFX meg van adva, de az 'osslsigncode' nincs telepítve (aláírás kihagyva)."
    echo "  Telepítés: sudo apt install osslsigncode   vagy   sudo pacman -S osslsigncode"
    return 0
  fi

  local signed_tmp="${target_file}.signed.tmp"
  local ts_url="${SIGN_TIMESTAMP_URL:-http://timestamp.digicert.com}"
  local -a sign_args=(
    sign
    -pkcs12 "$SIGN_PFX"
    -h sha256
    -n "$display_name"
  )
  if [ -n "${SIGN_PASS:-}" ]; then
    sign_args+=(-pass "$SIGN_PASS")
  fi

  echo "==> Windows bináris digitális aláírása: $(basename "$target_file")"
  if osslsigncode "${sign_args[@]}" -t "$ts_url" -in "$target_file" -out "$signed_tmp" >/dev/null 2>&1; then
    mv -f "$signed_tmp" "$target_file"
  elif osslsigncode "${sign_args[@]}" -in "$target_file" -out "$signed_tmp"; then
    mv -f "$signed_tmp" "$target_file"
    echo "FIGYELEM: Időbélyegző nélkül aláírva (a timestamp szerver nem volt elérhető: $ts_url)."
  else
    rm -f "$signed_tmp"
    echo "FIGYELEM: A digitális aláírás sikertelen maradt ennél a fájlnál: $target_file"
  fi
}

echo "==> [4/4] Kiadási csomagok összegyűjtése a dist-release/ mappába..."
rm -rf "$RELEASE_DIR"
mkdir -p "$RELEASE_DIR"

if [ -f "$SCRIPT_DIR/target/release/$BIN_NAME" ]; then
  cp -f "$SCRIPT_DIR/target/release/$BIN_NAME" "$RELEASE_DIR/$BIN_NAME"
  chmod 0755 "$RELEASE_DIR/$BIN_NAME"
elif [ -f "$SCRIPT_DIR/target/release/$BIN_NAME.exe" ]; then
  cp -f "$SCRIPT_DIR/target/release/$BIN_NAME.exe" "$RELEASE_DIR/$BIN_NAME.exe"
fi

WIN_EXE="$SCRIPT_DIR/target/x86_64-pc-windows-msvc/release/$BIN_NAME.exe"
if [ -f "$WIN_EXE" ]; then
  RELEASE_WIN_EXE="$RELEASE_DIR/$BIN_NAME-windows-x64.exe"
  cp -f "$WIN_EXE" "$RELEASE_WIN_EXE"
  sign_windows_pe "$RELEASE_WIN_EXE" "$APP_NAME"
  if [ "$BUILD_WINDOWS" = true ] && [ -f "$SCRIPT_DIR/installer.nsi" ]; then
    if command -v makensis >/dev/null 2>&1; then
      SETUP_EXE="$RELEASE_DIR/${APP_NAME}-${APP_VERSION}-Setup.exe"
      echo "==> Windows telepítő (Setup.exe) készítése NSIS-szel..."
      makensis \
        -DAPP_VERSION="$APP_VERSION" \
        -DEXE_PATH="$RELEASE_WIN_EXE" \
        -DOUT_FILE="$SETUP_EXE" \
        "$SCRIPT_DIR/installer.nsi"
      sign_windows_pe "$SETUP_EXE" "$APP_NAME Installer"
    else
      echo "FIGYELEM: A 'makensis' (NSIS) nem található, így a Setup.exe telepítő kimaradt."
      echo "  Arch Linux:    sudo pacman -S nsis"
      echo "  Debian/Ubuntu: sudo apt install nsis"
      echo "  Fedora:        sudo dnf install mingw32-nsis"
    fi
  fi
fi

cp -f "$SCRIPT_DIR/install.sh" "$RELEASE_DIR/install.sh"
cp -f "$SCRIPT_DIR/uninstall.sh" "$RELEASE_DIR/uninstall.sh"
chmod 0755 "$RELEASE_DIR/install.sh" "$RELEASE_DIR/uninstall.sh"

if [ -d "$SCRIPT_DIR/assets/icons" ]; then
  mkdir -p "$RELEASE_DIR/icons"
  cp -f "$SCRIPT_DIR/assets/icons/"*.png "$RELEASE_DIR/icons/" 2>/dev/null || true
  cp -f "$SCRIPT_DIR/assets/icons/"*.svg "$RELEASE_DIR/icons/" 2>/dev/null || true
  cp -f "$SCRIPT_DIR/assets/icons/"*.ico "$RELEASE_DIR/icons/" 2>/dev/null || true
fi

if [ "$ASK_PACKAGE" = true ] && [ -t 0 ] && [ -f "$RELEASE_DIR/$BIN_NAME" ]; then
  echo ""
  echo "Szeretne csomagot készíteni a lefordított programból?"
  echo "  1) AppImage (.AppImage)"
  echo "  2) Debian/Ubuntu csomag (.deb)"
  echo "  3) Mindkettő (.AppImage + .deb)"
  echo "  4) Nem kérek csomagot (csak a bináris)"
  read -r -p "Választás [1-4, alapértelmezett: 4]: " pkg_choice
  case "${pkg_choice:-4}" in
    1) BUILD_APPIMAGE=true ;;
    2) BUILD_DEB=true ;;
    3)
      BUILD_APPIMAGE=true
      BUILD_DEB=true
      ;;
    *) ;;
  esac
fi

build_deb_package() {
  if [ ! -f "$RELEASE_DIR/$BIN_NAME" ]; then
    echo "HIBA: Nem található Linux bináris a .deb csomaghoz."
    return 1
  fi
  if ! command -v dpkg-deb >/dev/null 2>&1; then
    echo "HIBA: A 'dpkg-deb' parancs nem található."
    echo "  Debian/Ubuntu: sudo apt install dpkg"
    echo "  Arch Linux:    sudo pacman -S dpkg"
    return 1
  fi

  local host_arch deb_arch
  host_arch="$(uname -m)"
  case "$host_arch" in
    x86_64) deb_arch="amd64" ;;
    aarch64|arm64) deb_arch="arm64" ;;
    armv7l) deb_arch="armhf" ;;
    *) deb_arch="$host_arch" ;;
  esac

  local deb_root="$RELEASE_DIR/deb-pkg"
  local deb_out="$RELEASE_DIR/${BIN_NAME}_${APP_VERSION}_${deb_arch}.deb"
  rm -rf "$deb_root"

  mkdir -p \
    "$deb_root/DEBIAN" \
    "$deb_root/usr/bin" \
    "$deb_root/usr/share/applications" \
    "$deb_root/usr/share/icons/hicolor/32x32/apps" \
    "$deb_root/usr/share/icons/hicolor/128x128/apps" \
    "$deb_root/usr/share/icons/hicolor/256x256/apps" \
    "$deb_root/usr/share/icons/hicolor/scalable/apps"

  cp -f "$RELEASE_DIR/$BIN_NAME" "$deb_root/usr/bin/$BIN_NAME"
  chmod 0755 "$deb_root/usr/bin/$BIN_NAME"

  if [ -f "$SCRIPT_DIR/assets/icons/32x32.png" ]; then
    cp -f "$SCRIPT_DIR/assets/icons/32x32.png" "$deb_root/usr/share/icons/hicolor/32x32/apps/$BIN_NAME.png"
  fi
  if [ -f "$SCRIPT_DIR/assets/icons/128x128.png" ]; then
    cp -f "$SCRIPT_DIR/assets/icons/128x128.png" "$deb_root/usr/share/icons/hicolor/128x128/apps/$BIN_NAME.png"
  fi
  if [ -f "$SCRIPT_DIR/assets/icons/128x128@2x.png" ]; then
    cp -f "$SCRIPT_DIR/assets/icons/128x128@2x.png" "$deb_root/usr/share/icons/hicolor/256x256/apps/$BIN_NAME.png"
  elif [ -f "$SCRIPT_DIR/assets/icons/icon.png" ]; then
    cp -f "$SCRIPT_DIR/assets/icons/icon.png" "$deb_root/usr/share/icons/hicolor/256x256/apps/$BIN_NAME.png"
  fi
  if [ -f "$SCRIPT_DIR/assets/icons/$BIN_NAME.svg" ]; then
    cp -f "$SCRIPT_DIR/assets/icons/$BIN_NAME.svg" "$deb_root/usr/share/icons/hicolor/scalable/apps/$BIN_NAME.svg"
  fi

  cat > "$deb_root/usr/share/applications/$BIN_NAME.desktop" <<EOF
[Desktop Entry]
Name=$APP_NAME
Comment=100% Rust video, music & album downloader with YT Music & Bandcamp search
Exec=/usr/bin/$BIN_NAME %U
Icon=$BIN_NAME
Terminal=false
Type=Application
Categories=AudioVideo;Network;Utility;
StartupWMClass=$BIN_NAME
EOF
  chmod 0644 "$deb_root/usr/share/applications/$BIN_NAME.desktop"

  local installed_size
  installed_size="$(du -sk "$deb_root" | cut -f1)"

  cat > "$deb_root/DEBIAN/control" <<EOF
Package: $BIN_NAME
Version: $APP_VERSION
Section: video
Priority: optional
Architecture: $deb_arch
Installed-Size: $installed_size
Maintainer: VideoDownloader <noreply@example.com>
Depends: libc6, libgtk-3-0 | libgtk-3-0t64, libgl1, libxkbcommon0
Recommends: ffmpeg
Description: 100% Rust video, music and playlist downloader
 Native desktop application built with Rust (eframe/egui) and yt-dlp.
EOF

  echo "==> .deb csomag létrehozása: $deb_out"
  if dpkg-deb --help 2>&1 | grep -q -- '--root-owner-group'; then
    dpkg-deb --build --root-owner-group "$deb_root" "$deb_out"
  else
    dpkg-deb --build "$deb_root" "$deb_out"
  fi
  rm -rf "$deb_root"
  echo "    Kész: $deb_out"
}

build_appimage_package() {
  if [ ! -f "$RELEASE_DIR/$BIN_NAME" ]; then
    echo "HIBA: Nem található Linux bináris az AppImage csomaghoz."
    return 1
  fi

  local host_arch appimage_arch
  host_arch="$(uname -m)"
  case "$host_arch" in
    x86_64) appimage_arch="x86_64" ;;
    aarch64|arm64) appimage_arch="aarch64" ;;
    *) appimage_arch="$host_arch" ;;
  esac

  local appimagetool_bin=""
  local cache_tool="$HOME/.cache/videodownloader-build/appimagetool-${appimage_arch}.AppImage"
  if command -v appimagetool >/dev/null 2>&1; then
    appimagetool_bin="$(command -v appimagetool)"
  elif [ -x "$SCRIPT_DIR/appimagetool" ]; then
    appimagetool_bin="$SCRIPT_DIR/appimagetool"
  elif [ -x "$cache_tool" ]; then
    appimagetool_bin="$cache_tool"
  else
    echo "==> Az 'appimagetool' letöltése ($appimage_arch)..."
    mkdir -p "$(dirname "$cache_tool")"
    local tool_url="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${appimage_arch}.AppImage"
    if command -v curl >/dev/null 2>&1; then
      curl -fL "$tool_url" -o "$cache_tool"
    elif command -v wget >/dev/null 2>&1; then
      wget -O "$cache_tool" "$tool_url"
    else
      echo "HIBA: Sem 'appimagetool', sem 'curl'/'wget' nem található az AppImage készítéshez."
      return 1
    fi
    chmod 0755 "$cache_tool"
    appimagetool_bin="$cache_tool"
  fi

  local appdir="$RELEASE_DIR/${APP_NAME}.AppDir"
  local appimage_out="$RELEASE_DIR/${APP_NAME}-${APP_VERSION}-${appimage_arch}.AppImage"
  rm -rf "$appdir"

  mkdir -p \
    "$appdir/usr/bin" \
    "$appdir/usr/share/applications" \
    "$appdir/usr/share/icons/hicolor/128x128/apps" \
    "$appdir/usr/share/icons/hicolor/256x256/apps" \
    "$appdir/usr/share/icons/hicolor/scalable/apps"

  cp -f "$RELEASE_DIR/$BIN_NAME" "$appdir/usr/bin/$BIN_NAME"
  chmod 0755 "$appdir/usr/bin/$BIN_NAME"

  cat > "$appdir/AppRun" <<EOF
#!/bin/sh
HERE="\$(dirname "\$(readlink -f "\$0")")"
export PATH="\$HERE/usr/bin:\$PATH"
exec "\$HERE/usr/bin/$BIN_NAME" "\$@"
EOF
  chmod 0755 "$appdir/AppRun"

  cat > "$appdir/$BIN_NAME.desktop" <<EOF
[Desktop Entry]
Name=$APP_NAME
Comment=100% Rust video, music & album downloader with YT Music & Bandcamp search
Exec=$BIN_NAME %U
Icon=$BIN_NAME
Terminal=false
Type=Application
Categories=AudioVideo;Network;Utility;
StartupWMClass=$BIN_NAME
EOF
  cp -f "$appdir/$BIN_NAME.desktop" "$appdir/usr/share/applications/$BIN_NAME.desktop"

  if [ -f "$SCRIPT_DIR/assets/icons/128x128@2x.png" ]; then
    cp -f "$SCRIPT_DIR/assets/icons/128x128@2x.png" "$appdir/$BIN_NAME.png"
    cp -f "$SCRIPT_DIR/assets/icons/128x128@2x.png" "$appdir/usr/share/icons/hicolor/256x256/apps/$BIN_NAME.png"
  elif [ -f "$SCRIPT_DIR/assets/icons/icon.png" ]; then
    cp -f "$SCRIPT_DIR/assets/icons/icon.png" "$appdir/$BIN_NAME.png"
    cp -f "$SCRIPT_DIR/assets/icons/icon.png" "$appdir/usr/share/icons/hicolor/256x256/apps/$BIN_NAME.png"
  elif [ -f "$SCRIPT_DIR/assets/icons/128x128.png" ]; then
    cp -f "$SCRIPT_DIR/assets/icons/128x128.png" "$appdir/$BIN_NAME.png"
  fi
  if [ -f "$SCRIPT_DIR/assets/icons/128x128.png" ]; then
    cp -f "$SCRIPT_DIR/assets/icons/128x128.png" "$appdir/usr/share/icons/hicolor/128x128/apps/$BIN_NAME.png"
  fi
  if [ -f "$SCRIPT_DIR/assets/icons/$BIN_NAME.svg" ]; then
    cp -f "$SCRIPT_DIR/assets/icons/$BIN_NAME.svg" "$appdir/usr/share/icons/hicolor/scalable/apps/$BIN_NAME.svg"
  fi
  if [ -f "$appdir/$BIN_NAME.png" ]; then
    ln -sf "$BIN_NAME.png" "$appdir/.DirIcon"
  fi

  echo "==> AppImage csomag létrehozása: $appimage_out"
  if ! ARCH="$appimage_arch" "$appimagetool_bin" --no-appstream "$appdir" "$appimage_out"; then
    echo "==> Újrapróbálás APPIMAGE_EXTRACT_AND_RUN=1 módban..."
    APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$appimage_arch" "$appimagetool_bin" --no-appstream "$appdir" "$appimage_out"
  fi
  chmod 0755 "$appimage_out"
  rm -rf "$appdir"
  echo "    Kész: $appimage_out"
}

if [ "$BUILD_DEB" = true ]; then
  build_deb_package
fi

if [ "$BUILD_APPIMAGE" = true ]; then
  build_appimage_package
fi

(
  cd "$RELEASE_DIR"
  rm -f SHA256SUMS.txt SHA256SUMS.txt.tmp
  for f in *; do
    if [ -f "$f" ]; then
      sha256sum "$f" >> SHA256SUMS.txt.tmp
    fi
  done
  if [ -f SHA256SUMS.txt.tmp ]; then
    mv -f SHA256SUMS.txt.tmp SHA256SUMS.txt
  fi
)

echo ""
echo "============================================================"
echo "  $APP_NAME v$APP_VERSION (100% Rust) sikeresen lefordítva!"
echo "  Kimeneti mappa: $RELEASE_DIR"
ls -lh "$RELEASE_DIR"
echo "  Telepítéshez futtassa: ./install.sh"
echo "============================================================"
