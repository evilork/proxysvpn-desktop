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
SHELL_CMD="/bin/launchctl asuser $USER_UID '$BIN' >/dev/null 2>&1 &"

# osascript "with administrator privileges" displays the system password
# dialog. After auth, the inner shell runs with sudo, allowing
# launchctl asuser to spawn the binary in the user's GUI session.
# We background (&) the launchctl call so osascript exits cleanly and our
# launcher script terminates — the real binary keeps running.
if ! /usr/bin/osascript -e "do shell script \"$SHELL_CMD\" with prompt \"$PROMPT\" with administrator privileges" 2>/dev/null; then
    # User cancelled dialog or wrong password — silent exit
    exit 0
fi
