; Instalador do FeatherWorks Launcher para Windows (NSIS). Gerado por pacote/gerar.sh:
;   makensis -DVERSAO=0.0.1 -DEXE=caminho/featherlauncher.exe -DSAIDA=caminho/instalador.exe pacote/featherlauncher.nsi
; Instala por usuário (sem pedir administrador), cria atalhos e registra o desinstalador
; em "Aplicativos instalados". Não abre arquivos: sem associação de formato.

Unicode true
!include "MUI2.nsh"

!ifndef VERSAO
  !define VERSAO "0.0.0"
!endif

Name "FeatherWorks Launcher ${VERSAO}"
OutFile "${SAIDA}"
InstallDir "$LOCALAPPDATA\Programs\FeatherWorks Launcher"
InstallDirRegKey HKCU "Software\FeatherWorks\FeatherWorks Launcher" "Pasta"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ICON "icones\featherlauncher.ico"
!define MUI_UNICON "icones\featherlauncher.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\featherlauncher.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Abrir o FeatherWorks Launcher"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "PortugueseBR"

!include "FileFunc.nsh"
!define DESINSTALAR "Software\Microsoft\Windows\CurrentVersion\Uninstall\FeatherWorks Launcher"

Section "FeatherWorks Launcher"
  SetOutPath "$INSTDIR"
  File "/oname=featherlauncher.exe" "${EXE}"
  File "icones\featherlauncher.ico"
  File "/oname=LICENSE.txt" "..\LICENSE"
  WriteUninstaller "$INSTDIR\desinstalar.exe"

  CreateShortCut "$SMPROGRAMS\FeatherWorks Launcher.lnk" "$INSTDIR\featherlauncher.exe" "" "$INSTDIR\featherlauncher.ico"
  CreateShortCut "$DESKTOP\FeatherWorks Launcher.lnk" "$INSTDIR\featherlauncher.exe" "" "$INSTDIR\featherlauncher.ico"

  WriteRegStr HKCU "Software\FeatherWorks\FeatherWorks Launcher" "Pasta" "$INSTDIR"
  WriteRegStr HKCU "${DESINSTALAR}" "DisplayName" "FeatherWorks Launcher"
  WriteRegStr HKCU "${DESINSTALAR}" "DisplayVersion" "${VERSAO}"
  WriteRegStr HKCU "${DESINSTALAR}" "Publisher" "FeatherWorks"
  ; Tamanho em "Programas e Recursos" (KB, do que foi instalado).
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${DESINSTALAR}" "EstimatedSize" "$0"
  WriteRegStr HKCU "${DESINSTALAR}" "DisplayIcon" "$INSTDIR\featherlauncher.ico"
  WriteRegStr HKCU "${DESINSTALAR}" "URLInfoAbout" "https://github.com/feather-works/featherlauncher"
  WriteRegStr HKCU "${DESINSTALAR}" "UninstallString" '"$INSTDIR\desinstalar.exe"'
  WriteRegDWORD HKCU "${DESINSTALAR}" "NoModify" 1
  WriteRegDWORD HKCU "${DESINSTALAR}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\FeatherWorks Launcher.lnk"
  Delete "$DESKTOP\FeatherWorks Launcher.lnk"
  Delete "$INSTDIR\featherlauncher.exe"
  ; Sobra da autoatualização do próprio launcher (src/instalar.rs, trocar_launcher); sem apagar, a pasta fica.
  Delete "$INSTDIR\featherlauncher.old"
  Delete "$INSTDIR\featherlauncher.ico"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\desinstalar.exe"
  RMDir "$INSTDIR"
  ; "Abrir com o sistema" do launcher (src/instalar.rs, iniciar_com_sistema) apontaria para um .exe apagado.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "FeatherWorksLauncher"
  DeleteRegKey HKCU "Software\FeatherWorks\FeatherWorks Launcher"
  DeleteRegKey HKCU "${DESINSTALAR}"
SectionEnd
