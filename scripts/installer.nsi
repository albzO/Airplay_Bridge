Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
Name "AirPlay Bridge"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\AirPlay Bridge"
InstallDirRegKey HKCU "Software\AirPlay Bridge" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "AirPlay Bridge"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "AirPlay Bridge Setup"
VIAddVersionKey "LegalCopyright" "See NOTICE"
!define MUI_ICON "..\airplay-frontend\src-tauri\icons\icon.ico"
!define MUI_UNICON "..\airplay-frontend\src-tauri\icons\icon.ico"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  SetShellVarContext current
  SetRegView 64
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "AirPlay Bridge requires Windows x64."
    Abort
  ${EndIf}
  ; Let the user pick the setup language, which also becomes the app's UI language.
  !insertmacro MUI_LANGDLL_DISPLAY
FunctionEnd

Section "AirPlay Bridge"
  IfFileExists "$INSTDIR\portable.flag" 0 +3
    MessageBox MB_ICONSTOP "This is a portable folder. Choose another installation directory."
    Abort
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD}\*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  ; Record the setup language in the user data directory (2052 = Simplified Chinese). The app
  ; reads it at startup; it can be changed later in Settings.
  CreateDirectory "$APPDATA\AirPlay Hub"
  FileOpen $0 "$APPDATA\AirPlay Hub\language" w
  ${If} $LANGUAGE == 2052
    FileWrite $0 "zh"
  ${Else}
    FileWrite $0 "en"
  ${EndIf}
  FileClose $0
  CreateDirectory "$SMPROGRAMS\AirPlay Bridge"
  CreateShortcut "$SMPROGRAMS\AirPlay Bridge\AirPlay Bridge.lnk" "$INSTDIR\airplay-bridge.exe"
  CreateShortcut "$SMPROGRAMS\AirPlay Bridge\Uninstall.lnk" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\AirPlay Bridge" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AirPlayBridge" "DisplayName" "AirPlay Bridge"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AirPlayBridge" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AirPlayBridge" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AirPlayBridge" "DisplayIcon" "$INSTDIR\airplay-bridge.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AirPlayBridge" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AirPlayBridge" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AirPlayBridge" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  SetRegView 64
  !include "${UNINSTALL_FILES}"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\AirPlay Bridge\AirPlay Bridge.lnk"
  Delete "$SMPROGRAMS\AirPlay Bridge\Uninstall.lnk"
  RMDir "$SMPROGRAMS\AirPlay Bridge"
  ; Remove startup only when it refers to this installation.
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "AirPlay Hub"
  ${If} $0 == '$\"$INSTDIR\airplay-bridge.exe$\"'
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "AirPlay Hub"
  ${EndIf}
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AirPlayBridge"
  DeleteRegKey HKCU "Software\AirPlay Bridge"
SectionEnd
