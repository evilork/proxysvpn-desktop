#!/bin/bash
# src-tauri/scripts/launcher.sh
# Bootstrap launcher installed as Contents/MacOS/<EXEC> inside ProxysVPN.app.
#
# Why this exists:
#   The Rust binary needs root (to create utun + modify routes), AND it needs
#   to run inside the user's GUI session (for tray icon / NSPasteboard / dialogs).
#   `sudo binary` keeps the process in root's session — no GUI access.
#   `launchctl asuser <uid> binary` runs binary as if from user's session,
#   inheriting current euid (root, since launchctl was elevated via osascript).

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SCRIPT_NAME="$(basename "$0")"
BIN="$SCRIPT_DIR/${SCRIPT_NAME}-bin"

if [[ ! -x "$BIN" ]]; then
    /usr/bin/osascript -e 'display dialog "ProxysVPN: внутренний бинарь не найден. Переустановите приложение." buttons {"OK"} default button 1 with icon stop'
    exit 1
fi

# Already root (e.g. launched via sudo manually) — just exec
if [[ "$EUID" -eq 0 ]]; then
    exec "$BIN" "$@"
fi

# Cached sudo from recent terminal — try without password prompt
if /usr/bin/sudo -n true 2>/dev/null; then
    exec /usr/bin/sudo -E "$BIN" "$@"
fi

USER_UID="$(/usr/bin/id -u)"
PROMPT='ProxysVPN запрашивает права администратора для создания VPN-туннеля.'

# osascript "with administrator privileges" displays the system password
# dialog. After auth, the inner shell runs as root, allowing
# launchctl asuser to spawn the binary in the user's GUI session.
# We background (&) the launchctl call so osascript exits cleanly and our
# launcher script terminates — the real binary keeps running.
#
# The path and the uid travel as arguments and are quoted by AppleScript's
# `quoted form of`. They used to be spliced into the AppleScript string inside
# single quotes, so an install path with ' or " in it (a folder called
# "Alexej's Apps") broke the command, and the failure was swallowed as if the
# person had pressed Cancel.
RESULT="$(/usr/bin/osascript - "$BIN" "$USER_UID" "$PROMPT" 2>/dev/null <<'APPLESCRIPT'
on run argv
    set binPath to item 1 of argv
    set userId to item 2 of argv
    set promptText to item 3 of argv
    try
        do shell script "/bin/launchctl asuser " & quoted form of userId & " " & quoted form of binPath & " >/dev/null 2>&1 &" with prompt promptText with administrator privileges
        return "ok"
    on error errText number errNum
        if errNum is -128 then return "cancelled"
        return "failed: " & errText
    end try
end run
APPLESCRIPT
)" || RESULT="failed: osascript did not run"

case "$RESULT" in
    ok|cancelled)
        # Cancel in the password dialog is a choice, not an error: silent exit.
        exit 0
        ;;
    *)
        /usr/bin/osascript - "$RESULT" >/dev/null 2>&1 <<'APPLESCRIPT' || true
on run argv
    display dialog "ProxysVPN не удалось запустить: " & (item 1 of argv) buttons {"OK"} default button 1 with icon stop
end run
APPLESCRIPT
        exit 1
        ;;
esac
