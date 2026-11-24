# VideoDownloader 2.3

A fast, lightweight, **100% pure Rust** desktop video, music, and playlist downloader & **Chromecast / Android TV caster** built with **`eframe` / `egui`**, **`tokio`**, **`reqwest`**, **`yt-dlp`**, and **`ffmpeg`**.

> Zero Node.js, Electron, WebKit, or external mDNS/Bonjour dependencies are required to compile or run the native desktop executable.

---

## Key Features

- **100% Pure Rust Native Application**: Compiles into a single fast native binary (`Cargo.toml` + `src/`) for Linux, Windows, and macOS using `eframe`/`egui` GPU-accelerated rendering and `tokio` async process orchestration.
- **Native Chromecast & Android TV Casting (Zero Extra Installs)**: Click **📺 Cast** on any search result or inspected link to stream directly to Chromecast or Android TV devices on your local network:
  - **Built-in Pure-Rust mDNS & CastV2**: Discovers `_googlecast._tcp.local.` devices automatically via `mdns-sd` (plus manual IP fallback) and communicates natively over TLS + CastV2 Protobuf (`rustls` + `ring`) — no Bonjour, Avahi, or external tools required.
  - **Two Selectable Cast Modes (Settings → Cast Mode)**:
    - **Local Proxy / FFmpeg Relay (Default)**: Resolves media streams via `yt-dlp`, serves direct streams with HTTP `Range` seeking support (including `ok.ru` progressive MP4s and HLS fallback remuxing), and muxes split HD video/audio streams (e.g., 720p/1080p YouTube) via zero-reencode `ffmpeg` fragmented MP4/MP3 piping.
    - **Direct Link / Standalone TV Playback (Phone-Style)**: Sends the external stream URL (or native YouTube TV Cast session) straight to your Chromecast/Android TV so playback continues uninterrupted on the TV even after closing VideoDownloader.
  - **Persistent Bottom Player Controls**: Live playback status (`Connecting`, `Buffering`, `Playing`, `Paused`), **Play/Pause**, **interactive Seekbar**, **Volume / Mute**, and **Stop Cast** controls. *(Note: allow local network/LAN traffic in your OS firewall for Relay mode so the TV can reach the local stream relay.)*
- **Unified Main-Menu Multi-Site Search with Full Pagination (`v2.3`)**: Type any keywords directly into the main Quick Download input bar and press **Enter** (or click **Search**) to search across **YouTube**, **YouTube Music**, **Bandcamp**, **Dailymotion**, **Videa.hu**, and **ok.ru**, complete with **Video / Album / Song** filters, platform badges, full un-capped per-page results, and **Load more** continuation support.
- **Dedicated YouTube Music & Bandcamp Search + Multi-Select Queue**: Search both **individual songs** and **full albums/EPs** on the Music Search tab (or main search) with album cover thumbnails, checkboxes, and **Ctrl + Left-Click** multi-selection to queue multiple downloads at once. Full playlists and albums are saved into **`<Album Artist>/<Album Title>`** with numbered tracks (`01 - Track Title.ext`), metadata tags, and **one album cover (`cover.jpg`)** per album folder.
- **Maximum Video Resolution by Default**: Videos default to **Highest available** resolution (`bestvideo+bestaudio/best`), with detected resolutions (`2160p`, `1440p`, `1080p`, `720p`, `480p`, etc.) in `MP4`, `MKV`, or `WEBM`.
- **Default MP3 (320 kbps) & Source-Aware Format Selection**: Music downloads default to **MP3 (320 kbps)**. After inspecting a link, the format, resolution, and quality selectors automatically display only the options genuinely available for that media (e.g., `FLAC`/`WAV` only appear when lossless streams are offered by the source).
- **Broad Video Platform Support**: Full inspection and downloading across **YouTube**, **YouTube Music**, **ok.ru** (Odnoklassniki), **videa.hu**, **Vimeo**, **TikTok**, **Instagram**, **SoundCloud**, **Bandcamp**, **Twitch**, and 1,000+ other sites supported by `yt-dlp` (with automatic `deno` JS runtime installation / detection for YouTube EJS challenges).
- **In-App `yt-dlp`, `ffmpeg` & `deno` Provisioning and Updates**: Automatically detects missing `yt-dlp`, `ffmpeg`, or JS runtime (`deno`) builds on startup, fetches official GitHub releases directly from within the app (Sidebar & Settings), and lets you install or update **both `yt-dlp` and `ffmpeg`** with one click at any time.
- **Full Download Manager**: Live progress (`%`), download speed, ETA, file size, status filtering (`All`, `Active`, `Completed`, `Failed`), search, concurrent download queue control, cancel, retry, open folder, and persistent download history (`history.json`).
- **Clipboard Integration, Themes & Multi-Language UI**: Paste or inspect directly from the clipboard, switch between **Light & Dark** themes, and toggle **English (`EN`)**, **German (`DE`)**, or **Hungarian (`HU`)**.
- **YouTube Cookie Authentication**: Supports local desktop browser cookies (`--cookies-from-browser`: Firefox, Chrome, Chromium, Brave, Edge, Opera, Vivaldi) and Netscape `cookies.txt` import.

---

## 1. Installing Build Dependencies

Because VideoDownloader is **100% Rust**, you do **not** need Node.js, npm, or WebKitGTK to build or run it. Only the Rust toolchain and Linux GUI/TLS development headers are needed for compiling from source.

### Arch Linux (and Manjaro / EndeavourOS)

1. Install build tools, OpenGL/Wayland/X11/GTK3 (for native file picker & clipboard), OpenSSL, `tar`, `ffmpeg`, and `rustup`:
```bash
sudo pacman -Syu --needed base-devel pkgconf openssl gtk3 libxkbcommon wayland libx11 libxcursor libxrandr libxi mesa tar ffmpeg rustup
```

2. Activate the stable Rust toolchain:
```bash
rustup default stable
```

---

### Debian / Ubuntu (and Linux Mint / Pop!_OS)

1. Update package lists and install build essentials, GUI/EGL/GTK3 dev headers, OpenSSL, `tar`, and `ffmpeg`:
```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libwayland-dev libxrandr-dev libxi-dev libgl1-mesa-dev tar ffmpeg curl
```

2. Install and activate Rust (`rustup`, `rustc`, `cargo`):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup default stable
```

3. Verify toolchain versions:
```bash
rustc --version
cargo --version
```

---

### Fedora (Workstation / KDE)

1. Install development tools, OpenSSL, GTK3, Wayland/X11/Mesa headers, `tar`, and `rustup`:
```bash
sudo dnf install -y @development-tools pkgconf-pkg-config openssl-devel gtk3-devel libxkbcommon-devel wayland-devel libX11-devel libXcursor-devel libXrandr-devel libXi-devel mesa-libGL-devel tar rustup
```

2. *(Optional)* Enable RPM Fusion and install system `ffmpeg` (or let VideoDownloader download a local `ffmpeg` build automatically inside the app):
```bash
sudo dnf install -y https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm
sudo dnf install -y ffmpeg
```

3. Initialize Rust:
```bash
rustup-init -y
source "$HOME/.cargo/env"
rustup default stable
```

---

## 2. Building & Packaging (`build.sh`)

From the project root directory, make the scripts executable and run the build script:

```bash
chmod +x build.sh install.sh uninstall.sh
./build.sh
```

The script automatically:
1. Verifies `rustc` and `cargo`.
2. Compiles the 100% pure Rust desktop application via `cargo build --release`.
3. Prompts whether you want to package an **AppImage** (`.AppImage`), a **Debian/Ubuntu `.deb`** package, **both**, or just the standalone binary.
   - Non-interactive flags: `./build.sh --appimage`, `./build.sh --deb`, `./build.sh --both`, `./build.sh --no-package`, and `--windows` (flags can be combined, e.g. `./build.sh --windows --appimage` or `./build.sh --windows --both`).
   - For `.deb` creation, `dpkg-deb` (`dpkg` package) is required (`sudo apt install dpkg` on Debian/Ubuntu or `sudo pacman -S dpkg` on Arch).
   - For `.AppImage` creation, `build.sh` uses `appimagetool` if installed or automatically downloads and caches it to `~/.cache/videodownloader-build/`.
4. Cleans and recreates `dist-release/` on every run, then collects the compiled binary (`videodownloader`), generated `.AppImage` / `.deb` packages, Windows `.exe` / `Setup.exe` (when `--windows` is used), icons (`assets/icons`), `install.sh`, `uninstall.sh`, and `SHA256SUMS.txt`.

#### Using the Generated Packages
- **AppImage**: `chmod +x dist-release/VideoDownloader-*.AppImage && ./dist-release/VideoDownloader-*.AppImage`
- **Debian / Ubuntu `.deb`**: `sudo apt install ./dist-release/videodownloader_*.deb`

*(You can also run it directly during development via `cargo run --release`.)*

### Cross-Compiling & Building a Windows Installer (`.exe` / `Setup.exe`)

The repository includes `installer.nsi` to package the compiled Windows binary into a standard Windows installer (`dist-release/VideoDownloader-2.3.0-Setup.exe`) with Start Menu & Desktop shortcuts, an application icon (`assets/icons/icon.ico`), and an uninstaller registered in Windows Apps & Features (per-user install, no administrator rights required).

Both the portable executable (`videodownloader-windows-x64.exe`) and the installed application automatically download `yt-dlp.exe`, `ffmpeg.exe` + `ffprobe.exe`, and `deno.exe` (required by `yt-dlp` for YouTube extraction) into `%LOCALAPPDATA%\videodownloader\bin` on first launch — no manual Python, Deno, `yt-dlp`, or `ffmpeg` setup is required on Windows.

#### Option A: Building the Windows Binary & Installer on Linux

1. Install the cross-compilation toolchain and **NSIS** (`makensis`):
   ```bash
   # Arch Linux (and Manjaro / EndeavourOS)
   sudo pacman -S --needed clang llvm lld nsis

   # Debian / Ubuntu
   sudo apt install -y clang lld nsis

   # Fedora
   sudo dnf install -y clang lld mingw32-nsis
   ```

2. Add the Windows MSVC target and install `cargo-xwin`:
   ```bash
   rustup target add x86_64-pc-windows-msvc
   cargo install cargo-xwin
   ```

3. Build the Windows portable `.exe` and NSIS `Setup.exe` installer together:
   ```bash
   # Builds Linux + Windows portable .exe + Windows Setup.exe (without Linux AppImage/.deb prompt):
   ./build.sh --windows --no-package

   # Or build Windows + Linux AppImage (or --both for AppImage + .deb) in one run:
   ./build.sh --windows --appimage
   ```
   When `makensis` is installed, `./build.sh --windows` automatically creates both `dist-release/videodownloader-windows-x64.exe` (portable) and `dist-release/VideoDownloader-2.3.0-Setup.exe` (installer).

   *(Alternative — Windows-only manual build without `build.sh`:)*
   ```bash
   cargo xwin build --release --target x86_64-pc-windows-msvc
   mkdir -p dist-release
   makensis installer.nsi
   ```

#### Option B: Building the Windows Binary & Installer on Native Windows

1. Install prerequisites on Windows:
   - [Rust (`rustup`)](https://rustup.rs/) with the default MSVC toolchain (`x86_64-pc-windows-msvc`) and Visual Studio Build Tools (C++ workload).
   - [NSIS (Nullsoft Scriptable Install System)](https://nsis.sourceforge.io/Download) — make sure `makensis.exe` is in your `PATH` (e.g. `winget install NSIS.NSIS` or `choco install nsis`, and add `C:\Program Files (x86)\NSIS` to `PATH`).

2. Build the release executable in PowerShell:
   ```powershell
   cargo build --release
   ```
   The standalone `target\release\videodownloader.exe` is completely self-contained and automatically downloads `yt-dlp.exe`, `ffmpeg.exe` + `ffprobe.exe`, and `deno.exe` into `%LOCALAPPDATA%\videodownloader\bin` on first launch.

3. Package it into a Windows installer (`dist-release\VideoDownloader-2.3.0-Setup.exe`):
   ```powershell
   New-Item -ItemType Directory -Force dist-release | Out-Null
   makensis /DEXE_PATH=target/release/videodownloader.exe installer.nsi
   ```
   *(If you cross-compiled or copied the binary elsewhere, adjust `/DEXE_PATH=...`, or override `/DAPP_VERSION=...` and `/DOUT_FILE=...` as needed.)*

---

## 3. Installing on Linux (`install.sh`)

### User-Level Installation (Recommended, no root required)
```bash
./install.sh
```
This script:
- Installs the compiled `videodownloader` binary into `~/.local/opt/videodownloader/`.
- Creates a CLI symlink at `~/.local/bin/videodownloader`.
- Installs desktop icons and the launcher entry (`~/.local/share/applications/videodownloader.desktop`).
- **Copies `uninstall.sh` into `~/.local/opt/videodownloader/uninstall.sh`** for clean uninstallation at any time.

### System-Wide Installation
```bash
sudo ./install.sh --system
```
Installs into `/opt/videodownloader/` with the uninstaller at `/opt/videodownloader/uninstall.sh`.

---

## 4. Complete Uninstallation (`uninstall.sh`)

Run the uninstaller from the installation directory:
```bash
~/.local/opt/videodownloader/uninstall.sh
```
*(Pass `--yes` to skip confirmation, or run `sudo /opt/videodownloader/uninstall.sh` for system-wide installs.)*

This removes:
- The `videodownloader` binary and installation directory.
- The CLI symlink (`videodownloader`), `.desktop` launcher, and application icons.
- All configuration, downloaded engine binaries (`yt-dlp`, `ffmpeg`, `deno`), and cache directories (`~/.config/videodownloader`, `~/.local/share/videodownloader`, `~/.cache/videodownloader`).
- **User-downloaded videos and music files are always preserved.**
