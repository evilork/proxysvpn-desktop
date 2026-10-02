; src-tauri/nsis/installer-hooks.nsh
;
; Stops a running ProxysVPN before the installer copies files and before the
; uninstaller removes them, and stops the arm64 installer on a PC that is not
; ARM64 (PVPN_REFUSE_FOREIGN_CPU, below). Wired in by bundle.windows.nsis.installerHooks
; (tauri.windows.conf.json); Tauri's installer.nsi runs each hook right BEFORE
; its own CheckIfAppIsRunning, in Section Install and Section Uninstall.
;
; Why the template's own check is not enough: it asks "ProxysVPN is running,
; press OK to close it" and stops the app with nsis_tauri_utils::KillProcess.
; On the Windows 11 VM (02.10.2026, app 0.3.2) that answered "Failed to close
; ProxysVPN" once in three upgrades and the installer aborted. The app runs
; elevated (requireAdministrator) and lives in the tray, so there is no window
; to close; the installer is elevated too (perMachine), so it may terminate it.
;
; What it does, and deliberately no more:
;   * taskkill /F /T /IM <main binary>: our own process and its tree. xray,
;     tun2socks and hysteria are its children, and they also sit in a
;     kill-on-close job object whose only handle the app holds
;     (pvpn_platform::process::tie_to_app), so they go down with it. Nothing is
;     killed by an engine's name: that would also stop another VPN client's
;     xray.exe or tun2socks.exe, and taskkill cannot filter by image path.
;   * The exit code is ignored on purpose: 128 ("not found") is the normal
;     case of an install with the app closed and must never fail the install.
;   * Then a bounded wait (at most ~5 s) until the process is really gone, so
;     the files are no longer locked and the template's check finds nothing.
;     If it is somehow still there, the template's check runs exactly as
;     before, prompt included.
;
; Known limit: on an upgrade the reinstall page by default first runs the OLD
; version's uninstaller, and the uninstallers of 0.3.2 and earlier carry no
; hook. That one step still depends on the old check; should it fail, quitting
; ProxysVPN from the tray and running the setup again gets past it. From the
; first version that ships this file on, both halves are covered.

!macro PVPN_STOP_RUNNING_APP
  Push $0
  Push $1
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /T /IM "${MAINBINARYNAME}.exe"'
  Pop $0
  StrCpy $1 0
  ${Do}
    ; The same plugin and call the template's CheckIfAppIsRunning uses for a
    ; perMachine install: 0 means "still running".
    nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 <> 0
      ${ExitDo}
    ${EndIf}
    IntOp $1 $1 + 1
    ${If} $1 > 20
      ${ExitDo}
    ${EndIf}
    Sleep 250
  ${Loop}
  Pop $1
  Pop $0
!macroend

; The arm64 installer on a PC that is not ARM64. It is a 32-bit x86 program,
; like every NSIS installer, so it starts on an x64 PC as well, and would
; install arm64 binaries that cannot run there. Said and stopped here instead,
; before anything is copied and before a running ProxysVPN is stopped.
;
; Which installer this is, is known when makensis compiles it: Tauri's
; installer.nsi does `!define ARCH "{{arch}}"` ("x64", "arm64" or "x86").
; This file is !included above that line, so the test sits inside the macro,
; whose `!if` is evaluated where the macro is inserted, in Section Install;
; in the x64 installer the macro is empty. ${IsNativeARM64} is x64.nsh's
; (NSIS 3.11, which Tauri downloads; the template includes x64.nsh before
; this file): the OS's own CPU via IsWow64Process2, not the installer's.
; makensis runs with -INPUTCHARSET UTF8 (tauri-bundler nsis/mod.rs), so the
; Russian line reads as written. /SD IDOK keeps a silent install from waiting
; on the box; it still aborts. SetOutPath has already made $INSTDIR: it is
; left again and removed only if empty (RMDir without /r), so an x64
; installation already there is not touched.
!macro PVPN_REFUSE_FOREIGN_CPU
  !if "${ARCH}" == "arm64"
    ${IfNot} ${IsNativeARM64}
      MessageBox MB_ICONSTOP|MB_OK "This is the ProxysVPN installer for Windows on ARM (arm64), and this PC has a different processor. Download the installer for x64 from the same release: ProxysVPN_${VERSION}_x64-setup.exe.$\r$\n$\r$\nЭто установщик ProxysVPN для Windows на ARM (arm64), а у этого компьютера другой процессор. Скачайте из того же выпуска установщик для x64: ProxysVPN_${VERSION}_x64-setup.exe." /SD IDOK
      SetOutPath "$TEMP"
      RMDir "$INSTDIR"
      Abort
    ${EndIf}
  !endif
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro PVPN_REFUSE_FOREIGN_CPU
  !insertmacro PVPN_STOP_RUNNING_APP
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro PVPN_STOP_RUNNING_APP
!macroend
