Unicode True
!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef APP_VERSION
  !define APP_VERSION "2.3.0"
!endif

!ifndef EXE_PATH
  !define EXE_PATH "target/x86_64-pc-windows-msvc/release/videodownloader.exe"
!endif

!ifndef OUT_FILE
  !define OUT_FILE "dist-release/VideoDownloader-${APP_VERSION}-Setup.exe"
!endif

Name "VideoDownloader ${APP_VERSION}"
OutFile "${OUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\VideoDownloader"
InstallDirRegKey HKCU "Software\VideoDownloader" "Install_Dir"
RequestExecutionLevel user
SetCompressor /FINAL zlib
ManifestDPIAware true
ManifestSupportedOS all

!define MUI_ABORTWARNING
!define MUI_ICON "assets/icons/icon.ico"
!define MUI_UNICON "assets/icons/icon.ico"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Hungarian"
!insertmacro MUI_LANGUAGE "German"

LangString UNINSTALL_REMOVE_DATA ${LANG_ENGLISH} "Do you also want to remove saved settings, history, and downloaded helper engines (yt-dlp, ffmpeg, deno)?$\r$\n$\r$\nYour downloaded videos and music will NOT be deleted."
LangString UNINSTALL_REMOVE_DATA ${LANG_HUNGARIAN} "Szeretné törölni a mentett beállításokat, az előzményeket és a letöltött segédalkalmazásokat (yt-dlp, ffmpeg, deno) is?$\r$\n$\r$\nA letöltött videói és zenéi NEM kerülnek törlésre."
LangString UNINSTALL_REMOVE_DATA ${LANG_GERMAN} "Möchten Sie auch gespeicherte Einstellungen, den Verlauf und heruntergeladene Hilfsprogramme (yt-dlp, ffmpeg, deno) entfernen?$\r$\n$\r$\nIhre heruntergeladenen Videos und Musikdateien werden NICHT gelöscht."

VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductName" "VideoDownloader"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "CompanyName" "VideoDownloader"
VIAddVersionKey /LANG=${LANG_ENGLISH} "LegalCopyright" "VideoDownloader Contributors"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileDescription" "VideoDownloader Installer"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "InternalName" "VideoDownloader-Setup"
VIAddVersionKey /LANG=${LANG_ENGLISH} "OriginalFilename" "VideoDownloader-${APP_VERSION}-Setup.exe"
VIAddVersionKey /LANG=${LANG_ENGLISH} "Comments" "Native Rust video and music downloader installer"

Section "VideoDownloader" SecMain
  SetOutPath "$INSTDIR"
  File "/oname=videodownloader.exe" "${EXE_PATH}"
  File "assets/icons/icon.ico"

  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateDirectory "$SMPROGRAMS\VideoDownloader"
  CreateShortcut "$SMPROGRAMS\VideoDownloader\VideoDownloader.lnk" "$INSTDIR\videodownloader.exe" "" "$INSTDIR\icon.ico"
  CreateShortcut "$SMPROGRAMS\VideoDownloader\Uninstall.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\VideoDownloader.lnk" "$INSTDIR\videodownloader.exe" "" "$INSTDIR\icon.ico"

  WriteRegStr HKCU "Software\VideoDownloader" "Install_Dir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VideoDownloader" "DisplayName" "VideoDownloader"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VideoDownloader" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VideoDownloader" "Publisher" "VideoDownloader"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VideoDownloader" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VideoDownloader" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VideoDownloader" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VideoDownloader" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VideoDownloader" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$DESKTOP\VideoDownloader.lnk"
  RMDir /r "$SMPROGRAMS\VideoDownloader"
  Delete "$INSTDIR\videodownloader.exe"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VideoDownloader"
  DeleteRegKey HKCU "Software\VideoDownloader"

  ${If} ${FileExists} "$APPDATA\videodownloader\*.*"
  ${OrIf} ${FileExists} "$LOCALAPPDATA\videodownloader\*.*"
    MessageBox MB_YESNO|MB_ICONQUESTION "$(UNINSTALL_REMOVE_DATA)" /SD IDNO IDNO _skip_user_data
      RMDir /r "$APPDATA\videodownloader"
      RMDir /r "$LOCALAPPDATA\videodownloader"
    _skip_user_data:
  ${EndIf}
SectionEnd
