; src-tauri/nsis/installer-hooks.nsh
;
; Stops a running ProxysVPN before the installer copies files and before the
; uninstaller removes them, and stops the arm64 installer on a PC that is not
; ARM64 before anything at all happens (PvpnRefuseForeignCpu, below). Wired in
; by bundle.windows.nsis.installerHooks (tauri.windows.conf.json); Tauri's
; installer.nsi runs the two hooks right BEFORE its own CheckIfAppIsRunning, in
; Section Install and Section Uninstall.
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
; install arm64 binaries that cannot run there. Said and stopped before the
; installer does anything.
;
; Not in NSIS_HOOK_PREINSTALL: Tauri 2.12's installer.nsi runs that inside
; Section Install, and before it come the reinstall page, whose leave
; function runs the OLD version's uninstaller (the default choice on an
; upgrade), and Section WebView2, which may download and install the runtime.
; An arm64 installer started on an x64 PC would first remove the working x64
; app and only then refuse. So the check runs at the two points that precede
; all of that:
;   * with a window: MUI2's .onGUIInit, which calls MUI_CUSTOMFUNCTION_GUIINIT
;     after .onInit and before the first page. The template includes this file
;     after MUI2.nsh and x64.nsh, before every page macro and before the
;     MUI_LANGUAGE that generates .onGUIInit, and does not define
;     MUI_CUSTOMFUNCTION_GUIINIT itself (tests/installerHooks.test.ts reads
;     the template out of the installed Tauri CLI and checks all of it);
;   * silent (/S): no .onGUIInit and no page or page callback runs (NSIS
;     manual, 4.12), only the sections, in the order they are defined. The
;     hidden section below is defined here, before the template's EarlyChecks,
;     WebView2 and Install, so it is the first.
; Quit, not Abort: it ends the installer at once from either place, and
; SetErrorLevel 2 ("aborted by script") is what a silent install returns.
;
; Which installer this is, is known when makensis compiles it, but this file
; is included before the template's `!define ARCH "{{arch}}"`, and a function
; is compiled where it is written. So the two values are read from the
; template itself: tauri-bundler writes it as installer.nsi and runs makensis
; in that folder (nsis/mod.rs, current_dir), and !searchparse /file without
; /noerrors stops the build if the line is not there. NSIS_HOOK_PREINSTALL,
; expanded after the template's own defines, stops it too if what was read
; differs from them. In the x64 installer nothing below is compiled.
;
; ${IsNativeARM64} is x64.nsh's (NSIS 3.11, which Tauri downloads): the OS's
; own CPU via IsWow64Process2, not the installer's. makensis runs with
; -INPUTCHARSET UTF8 (tauri-bundler nsis/mod.rs), so the Russian line reads as
; written. /SD IDOK keeps a silent install from waiting on the box; it prints
; the English line to the console it was started from instead, the way the
; template's EarlyChecks reports a refused silent downgrade.
!include LogicLib.nsh
!include x64.nsh

!searchparse /file "installer.nsi" `!define ARCH "` PVPN_ARCH `"`
!searchparse /file "installer.nsi" `!define VERSION "` PVPN_VERSION `"`

!if "${PVPN_ARCH}" == "arm64"
  !ifdef MUI_CUSTOMFUNCTION_GUIINIT
    !error "installer-hooks.nsh: MUI_CUSTOMFUNCTION_GUIINIT is already defined; the arm64 CPU check needs it"
  !endif
  !define MUI_CUSTOMFUNCTION_GUIINIT PvpnRefuseForeignCpu

  Function PvpnRefuseForeignCpu
    ${IfNot} ${IsNativeARM64}
      ${If} ${Silent}
        System::Call 'kernel32::AttachConsole(i -1)i.r0'
        ${If} $0 <> 0
          System::Call 'kernel32::GetStdHandle(i -11)i.r0'
          FileWrite $0 "This is the ProxysVPN installer for Windows on ARM (arm64), and this PC has a different processor. Use ProxysVPN_${PVPN_VERSION}_x64-setup.exe from the same release.$\r$\n"
        ${EndIf}
      ${EndIf}
      MessageBox MB_ICONSTOP|MB_OK "This is the ProxysVPN installer for Windows on ARM (arm64), and this PC has a different processor. Download the installer for x64 from the same release: ProxysVPN_${PVPN_VERSION}_x64-setup.exe.$\r$\n$\r$\nЭто установщик ProxysVPN для Windows на ARM (arm64), а у этого компьютера другой процессор. Скачайте из того же выпуска установщик для x64: ProxysVPN_${PVPN_VERSION}_x64-setup.exe." /SD IDOK
      SetErrorLevel 2
      Quit
    ${EndIf}
  FunctionEnd

  Section "-PvpnCpuCheck"
    Call PvpnRefuseForeignCpu
  SectionEnd
!endif

; What was read from installer.nsi above is what the template defines.
!macro PVPN_CHECK_TEMPLATE_DEFINES
  !if "${PVPN_ARCH}" != "${ARCH}"
    !error "installer-hooks.nsh read ARCH ${PVPN_ARCH} from installer.nsi, but the template defines ${ARCH}"
  !endif
  !if "${PVPN_VERSION}" != "${VERSION}"
    !error "installer-hooks.nsh read VERSION ${PVPN_VERSION} from installer.nsi, but the template defines ${VERSION}"
  !endif
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro PVPN_CHECK_TEMPLATE_DEFINES
  !insertmacro PVPN_STOP_RUNNING_APP
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro PVPN_STOP_RUNNING_APP
!macroend
