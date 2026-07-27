!macro NSIS_HOOK_PREUNINSTALL
  ; Tauri upgrades invoke the old uninstaller with /UPDATE. Keep the user's
  ; index and cache during upgrades, but remove every runtime file on a normal
  ; uninstall so the chosen installation directory is left clean.
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/UPDATE" $R1
  ${If} ${Errors}
    RMDir /r "$INSTDIR\data"
  ${EndIf}
!macroend
