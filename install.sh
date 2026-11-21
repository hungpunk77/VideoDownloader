#!/usr/bin/env bash
set -euo pipefail

# VideoDownloader — Linux telepítő szkript (100% Rust natív bináris)
# A lefordított Rust binárist a megfelelő helyre másolja, létrehozza a menüparancsikont,
# és bemásolja az uninstall.sh szkriptet a telepített program könyvtárába.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_NAME="VideoDownloader"
BIN_NAME="videodownloader"
SYSTEM_INSTALL=false

for arg in "$@"; do
  case "$arg" in
    --system) SYSTEM_INSTALL=true ;;
    --help|-h)
      echo "Használat: ./install.sh [--system]"
      echo "  Alapértelmezett: felhasználói telepítés (~/.local/opt/videodownloader)"
      echo "  --system: rendszerszintű telepítés (/opt/videodownloader, root jogosultság szükséges)"
      exit 0
      ;;
  esac
done

if [ "$SYSTEM_INSTALL" = true ] || [ "${EUID:-$(id -u)}" -eq 0 ]; then
  INSTALL_DIR="/opt/videodownloader"
  BIN_LINK_DIR="/usr/local/bin"
  DESKTOP_DIR="/usr/local/share/applications"
  ICON_THEME_BASE="/usr/local/share/icons/hicolor"
else
  INSTALL_DIR="$HOME/.local/opt/videodownloader"
  BIN_LINK_DIR="$HOME/.local/bin"
  DESKTOP_DIR="$HOME/.local/share/applications"
  ICON_THEME_BASE="$HOME/.local/share/icons/hicolor"
fi

# Forrás Rust bináris felkutatása
SOURCE_ARTIFACT=""
for candidate in \
  "$SCRIPT_DIR/dist-release/$BIN_NAME" \
  "$SCRIPT_DIR/target/release/$BIN_NAME" \
  "$SCRIPT_DIR/$BIN_NAME"; do
  if [ -f "$candidate" ]; then
    SOURCE_ARTIFACT="$candidate"
    break
  fi
done

if [ -z "$SOURCE_ARTIFACT" ]; then
  echo "HIBA: Nem található lefordított '$BIN_NAME' bináris fájl."
  echo "Futtassa először a ./build.sh vagy a 'cargo build --release' parancsot!"
  exit 1
fi

echo "==> $APP_NAME telepítése ide: $INSTALL_DIR"
mkdir -p \
  "$INSTALL_DIR" \
  "$BIN_LINK_DIR" \
  "$DESKTOP_DIR" \
  "$ICON_THEME_BASE/32x32/apps" \
  "$ICON_THEME_BASE/128x128/apps" \
  "$ICON_THEME_BASE/256x256/apps" \
  "$ICON_THEME_BASE/scalable/apps"

TARGET_BIN="$INSTALL_DIR/$BIN_NAME"
cp -f "$SOURCE_ARTIFACT" "$TARGET_BIN"
chmod 0755 "$TARGET_BIN"

# Parancssori szimbolikus link létrehozása
ln -sf "$TARGET_BIN" "$BIN_LINK_DIR/$BIN_NAME"

# Uninstall szkript kötelező bemásolása a telepített program könyvtárába
if [ -f "$SCRIPT_DIR/uninstall.sh" ]; then
  cp -f "$SCRIPT_DIR/uninstall.sh" "$INSTALL_DIR/uninstall.sh"
  chmod 0755 "$INSTALL_DIR/uninstall.sh"
fi

# Ikon könyvtár felkutatása (assets/icons vagy dist-release/icons)
ICON_DIR_SRC=""
for dir_cand in \
  "$SCRIPT_DIR/assets/icons" \
  "$SCRIPT_DIR/icons" \
  "$SCRIPT_DIR/../assets/icons"; do
  if [ -d "$dir_cand" ]; then
    ICON_DIR_SRC="$dir_cand"
    break
  fi
done

DESKTOP_ICON_VALUE="$BIN_NAME"
if [ -n "$ICON_DIR_SRC" ]; then
  if [ -f "$ICON_DIR_SRC/32x32.png" ]; then
    cp -f "$ICON_DIR_SRC/32x32.png" "$ICON_THEME_BASE/32x32/apps/$BIN_NAME.png"
  fi
  if [ -f "$ICON_DIR_SRC/128x128.png" ]; then
    cp -f "$ICON_DIR_SRC/128x128.png" "$ICON_THEME_BASE/128x128/apps/$BIN_NAME.png"
  fi
  if [ -f "$ICON_DIR_SRC/$BIN_NAME.svg" ]; then
    cp -f "$ICON_DIR_SRC/$BIN_NAME.svg" "$ICON_THEME_BASE/scalable/apps/$BIN_NAME.svg"
  fi

  if [ -f "$ICON_DIR_SRC/128x128@2x.png" ]; then
    cp -f "$ICON_DIR_SRC/128x128@2x.png" "$ICON_THEME_BASE/256x256/apps/$BIN_NAME.png"
    cp -f "$ICON_DIR_SRC/128x128@2x.png" "$INSTALL_DIR/$BIN_NAME.png"
  elif [ -f "$ICON_DIR_SRC/icon.png" ]; then
    cp -f "$ICON_DIR_SRC/icon.png" "$ICON_THEME_BASE/256x256/apps/$BIN_NAME.png"
    cp -f "$ICON_DIR_SRC/icon.png" "$INSTALL_DIR/$BIN_NAME.png"
  elif [ -f "$ICON_DIR_SRC/128x128.png" ]; then
    cp -f "$ICON_DIR_SRC/128x128.png" "$INSTALL_DIR/$BIN_NAME.png"
  fi

  if [ -f "$INSTALL_DIR/$BIN_NAME.png" ]; then
    chmod 0644 "$INSTALL_DIR/$BIN_NAME.png"
    DESKTOP_ICON_VALUE="$INSTALL_DIR/$BIN_NAME.png"
  fi
fi

if [ ! -f "$ICON_THEME_BASE/index.theme" ] && [ -f "/usr/share/icons/hicolor/index.theme" ]; then
  cp -f "/usr/share/icons/hicolor/index.theme" "$ICON_THEME_BASE/index.theme"
fi

cat > "$DESKTOP_DIR/$BIN_NAME.desktop" <<EOF
[Desktop Entry]
Name=$APP_NAME
Comment=100% Rust video, music, FLAC & album downloader with YouTube Music & Bandcamp search
Exec="$TARGET_BIN" %U
Icon=$DESKTOP_ICON_VALUE
Terminal=false
Type=Application
Categories=AudioVideo;Network;Utility;
StartupWMClass=$BIN_NAME
EOF

chmod 0644 "$DESKTOP_DIR/$BIN_NAME.desktop"

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$ICON_THEME_BASE" >/dev/null 2>&1 || true
fi

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
fi

echo ""
echo "============================================================"
echo "  $APP_NAME sikeresen telepítve!"
echo "  Programkönyvtár: $INSTALL_DIR"
echo "  Indítás parancssorból: $BIN_NAME"
echo "  Eltávolító szkript helye: $INSTALL_DIR/uninstall.sh"
echo "============================================================"
