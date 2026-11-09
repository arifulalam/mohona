; Extra steps of the Mohona installer (Tauri's NSIS installer calls these macros).
; Everything is per user: no administrator rights, nothing outside the user's own profile.

!macro NSIS_HOOK_PREINSTALL
  ; An old engine that is still running would keep its program file locked. Its downloads are
  ; kept and continue the next time Mohona starts.
  nsExec::Exec 'taskkill /F /IM mohona.exe'
  Pop $0
  ; The browser starts the native host for the extension and keeps it running, which also
  ; locks its program file. The browser starts it again when the extension next needs it.
  nsExec::Exec 'taskkill /F /IM mohona-host.exe'
  Pop $0
  Sleep 700
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Let the browsers find the native host (Chrome, Chromium, Edge, Firefox; current user only).
  nsExec::ExecToLog '"$INSTDIR\mohona-host.exe" install --browser all'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'taskkill /F /IM mohona.exe'
  Pop $0
  ; Take the host out of the browsers first (it needs its own program file), then stop any
  ; copy a browser still has running so that the file can be deleted.
  nsExec::ExecToLog '"$INSTDIR\mohona-host.exe" uninstall --browser all'
  Pop $0
  nsExec::Exec 'taskkill /F /IM mohona-host.exe'
  Pop $0
  Sleep 700
!macroend
