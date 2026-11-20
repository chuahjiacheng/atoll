; Extra steps for Atoll's Windows installer (Tauri NSIS hooks).
; $UpdateMode is 1 when a newer installer removes the old version before upgrading:
; then we keep the user's hooks, start-with-Windows and data.

; ---------------------------------------------------------------------------
; "Is Atoll running?" check, replacing Tauri's own CheckIfAppIsRunning.
;
; Tauri's version asks Windows Restart Manager for every process that has
; crest.exe open. Antivirus can keep that file open long after Atoll has quit
; (McAfee's firewall host, mc-fw-host.exe, does on this PC), and Restart
; Manager can never close a protected process like that. The installer then
; said "Atoll is running" even when it wasn't, and "Failed to kill Atoll" after
; OK, so neither install nor uninstall could finish. Holding the file open
; doesn't stop it being replaced or deleted, so here only a running Atoll
; process (matched by its full path) counts.
;
; This file is included after Tauri's utils.nsh and before the macro is used,
; so redefining it here replaces it in both the installer and the uninstaller.
; ---------------------------------------------------------------------------

; $0 = 1 when Atoll is running, else 0. Only used inside CheckIfAppIsRunning,
; which defines ATOLL_PROCS.
!macro AtollIsRunning
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "if (${ATOLL_PROCS}) { exit 1 }; exit 0"`
  Pop $0
  ${If} $0 != 1
    StrCpy $0 0 ; 0, or PowerShell couldn't run: don't block on a guess
  ${EndIf}
!macroend

!macroundef CheckIfAppIsRunning
!macro CheckIfAppIsRunning executablePath productName
  !define UniqueID ${__LINE__}
  ; PowerShell filter for Atoll processes started from this exact exe. Uses CIM,
  ; not Get-Process, so a 32-bit installer still sees a 64-bit Atoll's path.
  ; Defined here, not at the top: MAINBINARYNAME exists only after this file is included.
  !define /ifndef ATOLL_PROCS "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq '${MAINBINARYNAME}.exe' -and $$_.ExecutablePath -eq '${executablePath}' }"

  nsis_tauri_utils::StrReplace "$(appRunning)" "{{product_name}}" "${productName}"
  Pop $R1
  nsis_tauri_utils::StrReplace "$(appRunningOkKill)" "{{product_name}}" "${productName}"
  Pop $R2
  nsis_tauri_utils::StrReplace "$(failedToKillApp)" "{{product_name}}" "${productName}"
  Pop $R3

  !insertmacro AtollIsRunning
  ${If} $0 = 1
    IfSilent close_${UniqueID} 0
    ${IfThen} $PassiveMode != 1 ${|} MessageBox MB_OKCANCEL $R2 IDOK close_${UniqueID} IDCANCEL cancel_${UniqueID} ${|}

    close_${UniqueID}:
      ; 1. Ask Atoll to quit (Restart Manager), so it saves settings on the way out.
      ;    Its result is ignored: it also "fails" on antivirus holding the file.
      !insertmacro RestartManager_StartSession $R0
      ${If} $R0 != ""
        !insertmacro RestartManager_RegisterFile $R0 "${executablePath}"
        System::Call 'RSTRTMGR::RmShutdown(p R0, i ${RmForceShutdown}, p 0) i .r0'
        !insertmacro RestartManager_EndSession $R0
      ${EndIf}
      ; 2. Give it up to 5 s, then end it.
      StrCpy $R4 0
      wait_${UniqueID}:
        !insertmacro AtollIsRunning
        ${If} $0 = 1
        ${AndIf} $R4 < 10
          Sleep 500
          IntOp $R4 $R4 + 1
          Goto wait_${UniqueID}
        ${EndIf}
      ${If} $0 = 1
        nsExec::Exec `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "${ATOLL_PROCS} | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
        Pop $0
        Sleep 500
        !insertmacro AtollIsRunning
      ${EndIf}
      ${If} $0 = 1
        IfSilent +2 0
        Abort $R3
        Abort
      ${EndIf}
      Goto done_${UniqueID}

    cancel_${UniqueID}:
      Abort $R1

    done_${UniqueID}:
  ${EndIf}

  !undef UniqueID
!macroend

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
    ; The copy of crest-bridge.exe the hooks pointed at (current and pre-1.0.3 place),
    ; and the runtime file.
    RMDir /r "$PROFILE\.atoll\bin"
    Delete "$PROFILE\.atoll\runtime.json"
    RMDir "$PROFILE\.atoll"
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
