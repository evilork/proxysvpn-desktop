#!/usr/bin/env bash
# src-tauri/scripts/build-installer.sh
# Builds "ProxysVPN Installer.app" from AppleScript.
# This installer auto-copies ProxysVPN.app to /Applications, strips the
# quarantine flag, and launches it — eliminating the need for `xattr -cr`
# from the user's perspective.
#
# Usage:  bash src-tauri/scripts/build-installer.sh <output-dir>
# Output: <output-dir>/ProxysVPN Installer.app

set -euo pipefail

OUTPUT_DIR="${1:-}"
[[ -n "$OUTPUT_DIR" ]] || { echo "usage: $0 <output-dir>"; exit 1; }
mkdir -p "$OUTPUT_DIR"

INSTALLER_NAME="ProxysVPN Installer.app"
INSTALLER_PATH="$OUTPUT_DIR/$INSTALLER_NAME"
TMP_SCRIPT=$(mktemp /tmp/proxysvpn-installer.XXXXXX.applescript)

# AppleScript source. Any "tell user" UI uses native dialogs.
cat > "$TMP_SCRIPT" <<'APPLESCRIPT'
on run
    set appName to "ProxysVPN.app"
    set targetPath to "/Applications/" & appName

    -- Find source: the .app must be in the same directory as this installer
    set installerPath to POSIX path of (path to me)
    set installerDir to do shell script "dirname " & quoted form of installerPath
    set sourcePath to installerDir & "/" & appName

    -- Verify source exists
    try
        do shell script "test -d " & quoted form of sourcePath
    on error
        display dialog ¬
            "Не могу найти ProxysVPN.app рядом с установщиком." & return & return & ¬
            "Убедитесь что вы запускаете установщик из примонтированного DMG-образа, а не скопированного отдельно." ¬
            buttons {"OK"} default button 1 with icon stop ¬
            with title "ProxysVPN Installer"
        return
    end try

    -- Confirm with user
    set confirmText to ¬
        "Установить ProxysVPN на этот Mac?" & return & return & ¬
        "Приложение будет скопировано в /Applications." & return & ¬
        "После установки оно сразу запустится."
    try
        display dialog confirmText ¬
            buttons {"Отмена", "Установить"} default button 2 cancel button 1 ¬
            with icon note ¬
            with title "ProxysVPN Installer"
    on error
        return -- user cancelled
    end try

    -- If app is already installed and possibly running, ask to quit it.
    try
        do shell script "test -d " & quoted form of targetPath
        try
            tell application "ProxysVPN" to quit
        end try
        delay 1
        -- Hard kill remaining processes
        do shell script "pkill -9 -x proxysvpn-desktop 2>/dev/null; pkill -9 -x proxysvpn-desktop-bin 2>/dev/null; true"
    end try

    -- Copy with admin privileges if needed (target dir is system-protected
    -- on some Macs). Use ditto to preserve attrs and resource forks.
    try
        do shell script ¬
            "rm -rf " & quoted form of targetPath & " && " & ¬
            "ditto " & quoted form of sourcePath & " " & quoted form of targetPath & " && " & ¬
            "xattr -cr " & quoted form of targetPath ¬
            with administrator privileges
    on error errMsg number errNum
        if errNum is -128 then return -- user cancelled password prompt
        display dialog ¬
            "Установка не удалась:" & return & return & errMsg ¬
            buttons {"OK"} default button 1 with icon stop ¬
            with title "ProxysVPN Installer"
        return
    end try

    -- Launch the app
    try
        do shell script "open " & quoted form of targetPath
    on error errMsg
        display dialog ¬
            "Установлено, но автозапуск не сработал:" & return & errMsg & return & return & ¬
            "Запустите ProxysVPN из папки Программы." ¬
            buttons {"OK"} default button 1 ¬
            with title "ProxysVPN Installer"
        return
    end try

    -- Done — quit silently. The app itself will show its own UI.
end run
APPLESCRIPT

# Compile the script into an .app bundle
rm -rf "$INSTALLER_PATH"
osacompile -o "$INSTALLER_PATH" "$TMP_SCRIPT"
rm -f "$TMP_SCRIPT"

# Set bundle identifier and display name in Info.plist
INFO_PLIST="$INSTALLER_PATH/Contents/Info.plist"
if [[ -f "$INFO_PLIST" ]]; then
    /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier com.proxysvpn.installer" "$INFO_PLIST" 2>/dev/null \
      || /usr/libexec/PlistBuddy -c "Add :CFBundleIdentifier string com.proxysvpn.installer" "$INFO_PLIST"
    /usr/libexec/PlistBuddy -c "Set :CFBundleName ProxysVPN Installer" "$INFO_PLIST" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName ProxysVPN Installer" "$INFO_PLIST" 2>/dev/null \
      || /usr/libexec/PlistBuddy -c "Add :CFBundleDisplayName string ProxysVPN Installer" "$INFO_PLIST"
    /usr/libexec/PlistBuddy -c "Set :LSUIElement true" "$INFO_PLIST" 2>/dev/null \
      || /usr/libexec/PlistBuddy -c "Add :LSUIElement bool true" "$INFO_PLIST"
fi

# Use the main app's icon for the installer if available
APP_ICON="${APP_ICON:-}"
if [[ -n "$APP_ICON" && -f "$APP_ICON" ]]; then
    cp "$APP_ICON" "$INSTALLER_PATH/Contents/Resources/applet.icns"
fi

# Ad-hoc sign the installer (same approach as the main app, since we don't
# have a Developer ID yet). Without this, Gatekeeper still flags it but
# allows it via right-click → Open.
codesign --force --deep --sign - "$INSTALLER_PATH" 2>/dev/null || true

echo "Built: $INSTALLER_PATH"
