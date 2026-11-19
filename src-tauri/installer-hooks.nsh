; Extra steps for Atoll's Windows installer (Tauri NSIS hooks).
; $UpdateMode is 1 when a newer installer removes the old version before upgrading:
; then we keep the user's hooks, start-with-Windows and data.

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    ; Take Atoll's hooks and status line out of %USERPROFILE%\.claude\settings.json.
    nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-cleanup'
    Pop $0
    ; Start with Windows (current name, and the one used before the rename).
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Atoll"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Crest"
    ; Toast identity.
    DeleteRegKey HKCU "Software\Classes\AppUserModelId\local.crest.island"
    ; The copy of crest-bridge.exe the hooks pointed at.
    RMDir /r "$LOCALAPPDATA\Crest\bin"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; "Delete the application data" ticked: also remove settings, history, clipboard and board.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    RMDir /r "$LOCALAPPDATA\Crest"
    RMDir /r "$APPDATA\Crest"
  ${EndIf}
!macroend
