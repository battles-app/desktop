!include "MUI2.nsh"
!include "FileFunc.nsh"

# Battles.app Branded NSIS Installer
# =====================================
# Professional Windows installer with custom branding

# Installer Configuration
!define PRODUCT_NAME "Battles.app Desktop"
!define PRODUCT_VERSION "{{version}}"
!define PRODUCT_PUBLISHER "BATTLES.app™"
!define PRODUCT_WEB_SITE "https://battles.app"
!define PRODUCT_UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"

# Brand Colors (RGB format)
!define BRAND_BG_COLOR "0B0F1A"
!define BRAND_NEON_COLOR "00F3FF"
!define BRAND_NEON2_COLOR "FF00E6"
!define BRAND_GOLD_COLOR "FFD166"

# MUI Settings - Modern Interface
!define MUI_ABORTWARNING
!define MUI_ICON "${NSISDIR}\Contrib\Graphics\Icons\nsis3-install.ico"
!define MUI_UNICON "${NSISDIR}\Contrib\Graphics\Icons\nsis3-uninstall.ico"

# Welcome page customization
!define MUI_WELCOMEPAGE_TITLE "Welcome to Battles.app Desktop Setup"
!define MUI_WELCOMEPAGE_TEXT "This wizard will guide you through the installation of Battles.app Desktop.$\r$\n$\r$\nPro TikTok Live Tools with Stream Deck Integration$\r$\n$\r$\n✨ Real-time FX Control$\r$\n🎮 Elgato Stream Deck Support$\r$\n🎬 Professional Streaming Tools$\r$\n🔥 GPU-Accelerated Performance$\r$\n$\r$\nClick Next to continue."

# Finish page customization
!define MUI_FINISHPAGE_TITLE "Battles.app Desktop Installation Complete"
!define MUI_FINISHPAGE_TEXT "Battles.app Desktop has been successfully installed.$\r$\n$\r$\n🚀 Launch the application to get started!$\r$\n💡 Visit battles.app for support and updates.$\r$\n$\r$\nClick Finish to exit Setup."
!define MUI_FINISHPAGE_RUN "$INSTDIR\battles-desktop.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch Battles.app Desktop"
!define MUI_FINISHPAGE_LINK "Visit Battles.app"
!define MUI_FINISHPAGE_LINK_LOCATION "${PRODUCT_WEB_SITE}"

# Directory page customization
!define MUI_DIRECTORYPAGE_TEXT_TOP "Setup will install Battles.app Desktop in the following folder.$\r$\n$\r$\nTo install in a different folder, click Browse and select another folder."

# Pages
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "LICENSE.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

# Language
!insertmacro MUI_LANGUAGE "English"

# Installer Info
Name "${PRODUCT_NAME}"
OutFile "{{out_file}}"
InstallDir "$PROGRAMFILES64\Battles.app"
InstallDirRegKey HKLM "Software\Battles.app" "InstallDir"
ShowInstDetails show
ShowUnInstDetails show
RequestExecutionLevel admin

# Version Info
VIProductVersion "{{version}}.0"
VIAddVersionKey "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey "Comments" "Pro TikTok Live Tools"
VIAddVersionKey "CompanyName" "${PRODUCT_PUBLISHER}"
VIAddVersionKey "LegalCopyright" "© 2025 BATTLES.app™"
VIAddVersionKey "FileDescription" "Battles.app Desktop Installer"
VIAddVersionKey "FileVersion" "${PRODUCT_VERSION}"

# Modern UI Configuration
!define MUI_BGCOLOR "${BRAND_BG_COLOR}"
!define MUI_TEXTCOLOR "FFFFFF"

Section "MainSection" SEC01
  SetOutPath "$INSTDIR"
  SetOverwrite try
  
  # Main executable
  File "{{exe_path}}"
  
  # GStreamer runtime
  SetOutPath "$INSTDIR\gstreamer-runtime"
  File /r "{{gstreamer_runtime_path}}\*.*"
  
  # Additional resources
  {{additional_files}}
  
  # Create shortcuts
  CreateDirectory "$SMPROGRAMS\Battles.app"
  CreateShortCut "$SMPROGRAMS\Battles.app\Battles.app Desktop.lnk" "$INSTDIR\battles-desktop.exe"
  CreateShortCut "$DESKTOP\Battles.app Desktop.lnk" "$INSTDIR\battles-desktop.exe"
  
  # Write uninstaller
  WriteUninstaller "$INSTDIR\uninstall.exe"
  
  # Registry entries
  WriteRegStr HKLM "${PRODUCT_UNINST_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKLM "${PRODUCT_UNINST_KEY}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "${PRODUCT_UNINST_KEY}" "DisplayIcon" "$INSTDIR\battles-desktop.exe"
  WriteRegStr HKLM "${PRODUCT_UNINST_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKLM "${PRODUCT_UNINST_KEY}" "URLInfoAbout" "${PRODUCT_WEB_SITE}"
  WriteRegStr HKLM "${PRODUCT_UNINST_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKLM "${PRODUCT_UNINST_KEY}" "EstimatedSize" "$0"
SectionEnd

Section "Uninstall"
  # Remove files
  Delete "$INSTDIR\battles-desktop.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir /r "$INSTDIR\gstreamer-runtime"
  RMDir /r "$INSTDIR"
  
  # Remove shortcuts
  Delete "$SMPROGRAMS\Battles.app\Battles.app Desktop.lnk"
  Delete "$DESKTOP\Battles.app Desktop.lnk"
  RMDir "$SMPROGRAMS\Battles.app"
  
  # Remove registry entries
  DeleteRegKey HKLM "${PRODUCT_UNINST_KEY}"
  DeleteRegKey HKLM "Software\Battles.app"
  
  SetAutoClose true
SectionEnd

Function .onInit
  # Check if already installed
  ReadRegStr $R0 HKLM "${PRODUCT_UNINST_KEY}" "UninstallString"
  StrCmp $R0 "" done
  
  MessageBox MB_OKCANCEL|MB_ICONEXCLAMATION \
  "Battles.app Desktop is already installed.$\n$\nClick OK to remove the previous version or Cancel to cancel this installation." \
  IDOK uninst
  Abort
  
uninst:
  ExecWait '$R0 _?=$INSTDIR'
  
done:
FunctionEnd

