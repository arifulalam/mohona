; Extra steps of the Mohona installer (Tauri's NSIS installer calls these macros).
; Everything is per user: no administrator rights, nothing outside the user's own profile.

!macro NSIS_HOOK_PREINSTALL
  ; An old engine that is still running would keep its program file locked. Its downloads are
  ; kept and continue the next time Mohona starts.
  nsExec::Exec 'taskkill /F /IM mohona.exe'
  Pop $0
  ; The browser starts the native host for the extension and starts it again within a moment
  ; whenever it is stopped, so stopping it is not enough to free its file. Windows does allow a
  ; program file that is in use to be renamed, so the old copies are moved aside (and deleted
  ; now, or by a later install) and the new ones are written under the real names.
  nsExec::Exec 'taskkill /F /IM mohona-host.exe'
  Pop $0
  Sleep 300
  ${If} ${FileExists} "$INSTDIR\mohona-host.exe"
    Delete "$INSTDIR\mohona-host.old"
    ClearErrors
    Rename "$INSTDIR\mohona-host.exe" "$INSTDIR\mohona-host.old"
    ${If} ${Errors}
      Delete "$INSTDIR\mohona-host.old2"
      Rename "$INSTDIR\mohona-host.exe" "$INSTDIR\mohona-host.old2"
    ${EndIf}
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\mohona.exe"
    Delete "$INSTDIR\mohona.old"
    ClearErrors
    Rename "$INSTDIR\mohona.exe" "$INSTDIR\mohona.old"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Let the browsers find the native host (Chrome, Chromium, Edge, Firefox; current user only).
  nsExec::ExecToLog '"$INSTDIR\mohona-host.exe" install --browser all'
  Pop $0
  ; The copies moved aside before (a copy still running cannot be deleted yet: the next install does it).
  Delete "$INSTDIR\mohona-host.old"
  Delete "$INSTDIR\mohona-host.old2"
  Delete "$INSTDIR\mohona.old"
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
  Sleep 300
  ; A browser may have started it again: move it aside (a program in use can be renamed, not
  ; deleted) so that the uninstaller does not stop on it.
  ${If} ${FileExists} "$INSTDIR\mohona-host.exe"
    Delete "$INSTDIR\mohona-host.old"
    Rename "$INSTDIR\mohona-host.exe" "$INSTDIR\mohona-host.old"
  ${EndIf}
  Delete "$INSTDIR\mohona-host.old"
!macroend
