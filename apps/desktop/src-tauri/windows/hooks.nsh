!define SILK_MEDIA_SETUP "${__FILEDIR__}\install-media-tools.ps1"
!macro NSIS_HOOK_PREINSTALL
  Push $0
  Push $1
  InitPluginsDir
  File /oname=$PLUGINSDIR\install-media-tools.ps1 "${SILK_MEDIA_SETUP}"
  DetailPrint "Preparing clip sharing tools. The first install downloads about 240 MB."
  ; NSIS is 32-bit. Use the native Windows PowerShell host on x64 Windows.
  StrCpy $1 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  nsExec::ExecToLog '"$1" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\install-media-tools.ps1" -InstallDir "$INSTDIR"'
  Pop $0
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "Silk could not prepare its sharing tools. Check your internet connection and run setup again. Your settings and clips have not been removed." /SD IDOK
    SetErrorLevel 1
    Abort "Sharing tools setup failed."
  ${EndIf}
  Pop $1
  Pop $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\tools\ffmpeg\bin\ffmpeg.exe"
  Delete "$INSTDIR\tools\ffmpeg\bin\ffprobe.exe"
  Delete "$INSTDIR\tools\ffmpeg\LICENSE"
  Delete "$INSTDIR\tools\ffmpeg\README.txt"
  RMDir "$INSTDIR\tools\ffmpeg\bin"
  RMDir "$INSTDIR\tools\ffmpeg"
  RMDir "$INSTDIR\tools"
!macroend
