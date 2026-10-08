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
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "AirPlay Bridge requires Windows x64."
    Abort
  ${EndIf}
FunctionEnd

Section "AirPlay Bridge"
  IfFileExists "$INSTDIR\portable.flag" 0 +3
    MessageBox MB_ICONSTOP "This is a portable folder. Choose another installation directory."
    Abort
  ; Microsoft documents machine registration in the 32-bit view, user in native view.
  SetRegView 32
  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    SetRegView 64
    ReadRegStr $0 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${EndIf}
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    DetailPrint "Installing Microsoft WebView2 Runtime (internet required)..."
    InitPluginsDir
    SetOutPath "$PLUGINSDIR"
    File /oname=MicrosoftEdgeWebview2Setup.exe "${WEBVIEW_BOOTSTRAPPER}"
    ClearErrors
    ExecWait '"$PLUGINSDIR\MicrosoftEdgeWebview2Setup.exe" /silent /install' $1
    ${If} ${Errors}
    ${OrIf} $1 != 0
      MessageBox MB_ICONSTOP "WebView2 installation failed. Install Microsoft WebView2 Runtime, then run setup again."
      Abort
    ${EndIf}
  ${EndIf}
  SetRegView 64
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD}\*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
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
