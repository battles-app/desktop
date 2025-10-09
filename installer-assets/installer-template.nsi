!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"

!define PRODUCTNAME "{{product_name}}"
!define VERSION "{{version}}"
!define MANUFACTURER "BATTLES.app™"
!define INSTALLMODE "{{install_mode}}"
!define LICENSE "{{license}}"
!define INSTALLERICON "{{installer_icon}}"
!define SIDEBARIMAGE "{{sidebar_image}}"
!define HEADERIMAGE "{{header_image}}"
!define MAINBINARYNAME "{{main_binary_name}}"
!define MAINBINARYSRCPATH "{{main_binary_path}}"
!define OUTFILE "{{out_file}}"
!define ARCH "{{arch}}"
!define PLUGINSPATH "{{additional_plugins_path}}"
!define ALLOWDOWNGRADES "{{allow_downgrades}}"
!define DISPLAYLANGUAGESELECTOR "{{display_language_selector}}"
!define INSTALLWEBVIEW2MODE "{{install_webview2_mode}}"
!define WEBVIEW2INSTALLERARGS "{{webview2_installer_args}}"
!define WEBVIEW2BOOTSTRAPPERPATH "{{webview2_bootstrapper_path}}"
!define WEBVIEW2INSTALLERPATH "{{webview2_installer_path}}"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
!define MANUPRODUCTKEY "Software\${MANUFACTURER}\${PRODUCTNAME}"
!define RESOURCESPATH "{{resources_path}}"
!define BINARIESPATH "{{binaries_path}}"

# Installer attributes
Name "${PRODUCTNAME}"
OutFile "${OUTFILE}"
Unicode true
SetCompressor lzma

!if "${INSTALLMODE}" == "perMachine"
  RequestExecutionLevel admin
  InstallDir "$PROGRAMFILES64\${PRODUCTNAME}"
!else if "${INSTALLMODE}" == "currentUser"
  RequestExecutionLevel user
  InstallDir "$LOCALAPPDATA\${PRODUCTNAME}"
!else
  !error "INSTALLMODE must be 'perMachine' or 'currentUser'"
!endif

# Modern UI Settings
!define MUI_ABORTWARNING
!define MUI_ICON "${INSTALLERICON}"
!define MUI_UNICON "${INSTALLERICON}"

# Custom Welcome Page
!define MUI_WELCOMEPAGE_TITLE "Welcome to Battles.app Desktop Setup"
!define MUI_WELCOMEPAGE_TEXT "This wizard will guide you through the installation of Battles.app Desktop.$\r$\n$\r$\n✨ Real-time FX Control$\r$\n🎮 Elgato Stream Deck Support$\r$\n🎬 Professional Streaming Tools$\r$\n🔥 GPU-Accelerated Performance$\r$\n$\r$\nClick Next to continue."

# Custom Finish Page  
!define MUI_FINISHPAGE_TITLE "Installation Complete!"
!define MUI_FINISHPAGE_TEXT "Battles.app Desktop has been successfully installed!$\r$\n$\r$\n🚀 Launch the application to get started$\r$\n💡 Visit battles.app for support and updates"
!define MUI_FINISHPAGE_RUN "$INSTDIR\${MAINBINARYNAME}.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch Battles.app Desktop"
!define MUI_FINISHPAGE_LINK "Visit Battles.app"
!define MUI_FINISHPAGE_LINK_LOCATION "https://battles.app"

# Pages
!insertmacro MUI_PAGE_WELCOME
!if "${LICENSE}" != ""
  !insertmacro MUI_PAGE_LICENSE "${LICENSE}"
!endif
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

# Languages
!insertmacro MUI_LANGUAGE "English"

# Version Info
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "Battles.app Desktop Installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "${MANUFACTURER}"
VIAddVersionKey "LegalCopyright" "© 2025 ${MANUFACTURER}"
VIAddVersionKey "ProductVersion" "${VERSION}"

# Installer sections
!include "${PLUGINSPATH}\FileAssociation.nsh"
!include "${PLUGINSPATH}\StrFunc.nsh"

Section "Install"
  SetOutPath "$INSTDIR"
  
  {{#each binaries}}
  File "{{this}}"
  {{/each}}
  
  {{#each resources}}
  CreateDirectory "$INSTDIR\{{this.[0]}}"
  {{#each this.[1]}}
  File /a "/oname={{this.[0]}}" "{{this.[1]}}"
  {{/each}}
  {{/each}}
  
  # Create shortcuts
  CreateDirectory "$SMPROGRAMS\${PRODUCTNAME}"
  CreateShortCut "$SMPROGRAMS\${PRODUCTNAME}\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  CreateShortCut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  
  # Write uninstaller
  WriteUninstaller "$INSTDIR\uninstall.exe"
  
  # Registry
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\${MAINBINARYNAME}.exe"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr SHCTX "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"
  WriteRegStr SHCTX "${UNINSTKEY}" "URLInfoAbout" "https://battles.app"
  
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize" "$0"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\${MAINBINARYNAME}.exe"
  Delete "$INSTDIR\uninstall.exe"
  
  {{#each resources}}
  RMDir /r "$INSTDIR\{{this.[0]}}"
  {{/each}}
  
  RMDir "$INSTDIR"
  
  Delete "$SMPROGRAMS\${PRODUCTNAME}\${PRODUCTNAME}.lnk"
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  RMDir "$SMPROGRAMS\${PRODUCTNAME}"
  
  DeleteRegKey SHCTX "${UNINSTKEY}"
  DeleteRegKey SHCTX "${MANUPRODUCTKEY}"
SectionEnd

