; Instalador do FeatherWorks Launcher para Windows (NSIS). Gerado por pacote/gerar.sh:
;   makensis -DVERSAO=0.0.1 -DEXE=caminho/featherlauncher.exe -DSAIDA=caminho/instalador.exe pacote/featherlauncher.nsi
; Instala para todos os usuários em "Arquivos de Programas\FeatherWorks\Launcher", a pasta da
; suíte (os programas vão para FeatherWorks\<Programa>). Pede administrador uma vez: só o
; administrador altera os .exe. Cria atalhos e registra o desinstalador em "Aplicativos
; instalados". Não abre arquivos: sem associação de formato.

Unicode true
!include "MUI2.nsh"

!ifndef VERSAO
  !define VERSAO "0.0.0"
!endif

Name "FeatherWorks Launcher ${VERSAO}"
OutFile "${SAIDA}"
InstallDir "$PROGRAMFILES64\FeatherWorks\Launcher"
InstallDirRegKey HKLM "Software\FeatherWorks\FeatherWorks Launcher" "Pasta"
RequestExecutionLevel admin
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

; Atalhos de todos os usuários e o registro de 64 bits (o programa é de 64 bits).
Function .onInit
  SetShellVarContext all
  SetRegView 64
FunctionEnd

Function un.onInit
  SetShellVarContext all
  SetRegView 64
FunctionEnd

Section "FeatherWorks Launcher"
  SetOutPath "$INSTDIR"
  ; A autoatualização roda este instalador com o launcher aberto: o .exe em uso não se
  ; sobrescreve, mas se renomeia; o .old sai na próxima instalação ou na desinstalação.
  Delete "$INSTDIR\featherlauncher.old"
  Rename "$INSTDIR\featherlauncher.exe" "$INSTDIR\featherlauncher.old"
  File "/oname=featherlauncher.exe" "${EXE}"
  File "icones\featherlauncher.ico"
  File "/oname=LICENSE.txt" "..\LICENSE"
  WriteUninstaller "$INSTDIR\desinstalar.exe"

  CreateShortCut "$SMPROGRAMS\FeatherWorks Launcher.lnk" "$INSTDIR\featherlauncher.exe" "" "$INSTDIR\featherlauncher.ico"
  CreateShortCut "$DESKTOP\FeatherWorks Launcher.lnk" "$INSTDIR\featherlauncher.exe" "" "$INSTDIR\featherlauncher.ico"

  WriteRegStr HKLM "Software\FeatherWorks\FeatherWorks Launcher" "Pasta" "$INSTDIR"
  WriteRegStr HKLM "${DESINSTALAR}" "DisplayName" "FeatherWorks Launcher"
  WriteRegStr HKLM "${DESINSTALAR}" "DisplayVersion" "${VERSAO}"
  WriteRegStr HKLM "${DESINSTALAR}" "Publisher" "FeatherWorks"
  ; Tamanho em "Programas e Recursos" (KB, do que foi instalado).
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKLM "${DESINSTALAR}" "EstimatedSize" "$0"
  WriteRegStr HKLM "${DESINSTALAR}" "DisplayIcon" "$INSTDIR\featherlauncher.ico"
  WriteRegStr HKLM "${DESINSTALAR}" "URLInfoAbout" "https://github.com/feather-works/featherlauncher"
  WriteRegStr HKLM "${DESINSTALAR}" "UninstallString" '"$INSTDIR\desinstalar.exe"'
  WriteRegDWORD HKLM "${DESINSTALAR}" "NoModify" 1
  WriteRegDWORD HKLM "${DESINSTALAR}" "NoRepair" 1
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
  ; A pasta da suíte só sai se ficou vazia (sem nenhum programa instalado).
  RMDir "$INSTDIR\.."
  ; "Abrir com o sistema" do launcher (src/instalar.rs, iniciar_com_sistema) apontaria para um .exe apagado.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "FeatherWorksLauncher"
  DeleteRegKey HKLM "Software\FeatherWorks\FeatherWorks Launcher"
  DeleteRegKey /ifempty HKLM "Software\FeatherWorks"
  DeleteRegKey HKLM "${DESINSTALAR}"
SectionEnd
