#!/usr/bin/env bash
set -euo pipefail

# VideoDownloader — Teljes eltávolító szkript (100% Rust kiadás)
# Eltávolítja a programfájlokat, ikonokat, parancslinkeket és minden konfigurációt/gyorsítótárat.

APP_NAME="VideoDownloader"
BIN_NAME="videodownloader"
AUTO_YES=false

for arg in "$@"; do
  case "$arg" in
    -y|--yes) AUTO_YES=true ;;
    --help|-h)
      echo "Használat: ./uninstall.sh [-y|--yes]"
      echo "  Eltávolítja a $APP_NAME binárist, a telepített mappát, az asztali ikont,"
      echo "  valamint a konfigurációs, motor (yt-dlp/ffmpeg/deno) és gyorsítótár fájlokat a rendszerről."
      exit 0
      ;;
  esac
done

if [ "$AUTO_YES" = false ]; then
  echo "Figyelem: Ez a művelet teljes mértékben eltávolítja a $APP_NAME programot"
  echo "és minden hozzá tartozó konfigurációs fájlt a rendszerről (a letöltött videók/zenék megmaradnak)."
  read -r -p "Biztosan folytatja az eltávolítást? [i/N] " REPLY
  case "$REPLY" in
    [iI]|[iI][gG][eE][nN]|[yY]|[yY][eE][sS]) ;;
    *)
      echo "Eltávolítás megszakítva."
      exit 0
      ;;
  esac
fi

echo "==> Futó $APP_NAME folyamatok leállítása..."
pkill -x "$BIN_NAME" >/dev/null 2>&1 || true
pkill -f "VideoDownloader-.*\\.AppImage" >/dev/null 2>&1 || true

echo "==> Telepített binárisok és asztali integráció törlése..."
rm -f "$HOME/.local/bin/$BIN_NAME"
rm -f "$HOME/.local/share/applications/$BIN_NAME.desktop"
rm -f "$HOME/.local/share/icons/hicolor/32x32/apps/$BIN_NAME.png"
rm -f "$HOME/.local/share/icons/hicolor/128x128/apps/$BIN_NAME.png"
rm -f "$HOME/.local/share/icons/hicolor/256x256/apps/$BIN_NAME.png"
rm -f "$HOME/.local/share/icons/hicolor/scalable/apps/$BIN_NAME.svg"
rm -rf "$HOME/.local/opt/videodownloader"

if [ -d "/opt/videodownloader" ] || [ -L "/usr/local/bin/$BIN_NAME" ]; then
  if [ "${EUID:-$(id -u)}" -eq 0 ]; then
    rm -f "/usr/local/bin/$BIN_NAME"
    rm -f "/usr/local/share/applications/$BIN_NAME.desktop"
    rm -f "/usr/local/share/icons/hicolor/32x32/apps/$BIN_NAME.png"
    rm -f "/usr/local/share/icons/hicolor/128x128/apps/$BIN_NAME.png"
    rm -f "/usr/local/share/icons/hicolor/256x256/apps/$BIN_NAME.png"
    rm -f "/usr/local/share/icons/hicolor/scalable/apps/$BIN_NAME.svg"
    rm -rf "/opt/videodownloader"
  else
    echo "MEGJEGYZÉS: Rendszerszintű telepítés található (/opt/videodownloader). Törléséhez futtassa sudo-val."
  fi
fi

echo "==> Konfigurációs, adat- és gyorsítótár könyvtárak törlése..."
rm -rf "$HOME/.config/videodownloader"
rm -rf "$HOME/.config/com.videodownloader.app"
rm -rf "$HOME/.local/share/videodownloader"
rm -rf "$HOME/.local/share/com.videodownloader.app"
rm -rf "$HOME/.cache/videodownloader"
rm -rf "$HOME/.cache/com.videodownloader.app"

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$HOME/.local/share/applications" >/dev/null 2>&1 || true
fi

echo ""
echo "============================================================"
echo "  $APP_NAME és minden konfigurációja sikeresen eltávolítva!"
echo "============================================================"
