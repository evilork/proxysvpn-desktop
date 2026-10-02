// src-tauri/crates/pvpn-platform/src/helper/install.rs
//! The AppImage's own copy of the root helper.
//!
//! ── Why a copy ─────────────────────────────────────────────────────────────
//! The .deb starts the GUI's own executable, `/usr/bin/proxysvpn-desktop`,
//! through pkexec with `--helper`. An AppImage cannot do that. It runs from a
//! FUSE mount (`/tmp/.mount_*`) that only the user who mounted it may enter —
//! root included, because the runtime mounts it without `allow_other` — so
//! pkexec asks for the password and then fails to execute the helper (exit
//! 127). Up to 0.3.3 the app therefore refused to start from an AppImage at
//! all (`ElevationUnavailable::FuseMount`).
//!
//! Running the files from somewhere root *can* reach is not the answer either:
//! an AppImage extracted with `--appimage-extract-and-run`, or one sitting in
//! `~/Applications`, belongs to the person, and root must never execute a file
//! an unprivileged process can change — the 0.3.2 audit took exactly that out
//! of the helper (helper/proto.rs, `UpParams`).
//!
//! So the first connect from an AppImage copies two files into a folder only
//! root can change, and every later connect runs them from there:
//!
//! ```text
//! /home/.proxysvpn/                          root:root 0755
//! /home/.proxysvpn/bin/proxysvpn-helper      root:root 0755
//! /home/.proxysvpn/bin/tun2socks             root:root 0755
//! ```
//!
//! `/home` because on SteamOS, the system this exists for, it is the one place
//! Valve keeps across OS updates: `/usr` and `/opt` are a read-only image that
//! every update replaces, so nothing can be installed there the way the .deb
//! installs into `/usr/bin`. `bin/` and not the folder itself: linuxdeploy
//! sets the RUNPATH `$ORIGIN/../lib` on the ELF files it bundles (the CI log
//! prints the helper's), and with the helper one level down that resolves to
//! `/home/.proxysvpn/lib` — inside the same root-owned tree — instead of
//! `/home/lib`.
//!
//! `proxysvpn-helper` is its own small binary (src/bin/proxysvpn-helper.rs),
//! not the GUI executable: the GUI links webkit2gtk, GTK and the rest of what
//! the AppImage carries in `usr/lib`, so outside the AppImage it would not
//! even start on a system that lacks them.
//!
//! ── How the copy is made ───────────────────────────────────────────────────
//! One pkexec, one password window, running a fixed script ([`setup_script`])
//! under the system's own `/bin/sh`:
//!
//! 1. The GUI hashes both files where they are, in its read-only mount, and
//!    copies them into a private folder of its own ([`staging_dir`]) that root
//!    can read and the mount is not.
//! 2. Root checks that the staging folder is a real folder of the person who
//!    typed the password (`PKEXEC_UID`) that nobody else may change, then
//!    copies each staged file into a 0600 temp file inside the root-owned
//!    folder, hashes *that copy* against the sum the GUI passed, and only then
//!    makes it executable and renames it into place. The person still owns
//!    the staged files and may swap them at any moment, so root opens each one
//!    exactly once, with dd's `iflag=nofollow,nonblock`: a link put in its
//!    place — to /etc/shadow, to a device — is not followed, and a FIFO does
//!    not hold root waiting for a writer. Whatever is read can only fail the
//!    hash: the copy is never readable by others and never runs.
//! 3. The script `exec`s the installed helper, which speaks the protocol on
//!    the pipe the GUI already holds. The setup is the first connect.
//!
//! The sums are taken at run time from the mount rather than compiled into the
//! app. linuxdeploy rewrites the RUNPATH of (and may strip) the ELF files it
//! bundles, so the bytes in the AppImage are not the bytes cargo produced and a
//! compiled-in sum would not match; and the mount is read-only and comes from
//! the same image as the GUI binary itself, so a sum read from it is exactly as
//! trustworthy as one compiled into that binary. What it protects is the same
//! as the macOS copy (src/engine_stage.rs): the window between the person's
//! "yes" and root running the files. Tampering with the AppImage file before
//! the person starts it is not covered, and cannot be by anything inside it.
//!
//! The next connect finds both copies current ([`judge`]: root-owned, 0755,
//! the mount's sums) and runs `pkexec /home/.proxysvpn/bin/proxysvpn-helper
//! --helper` — no copy, the same one window the .deb shows. An update of the
//! AppImage changes the sums, and the next connect copies again.
//!
//! ── Gaming Mode on SteamOS ─────────────────────────────────────────────────
//! Gaming Mode (gamescope) runs no polkit authentication agent, so any pkexec
//! that needs a password fails at once with "No authentication agent found"
//! (the app starts pkexec with `--disable-internal-agent`, so it does not try
//! a terminal prompt first: `privilege::PKEXEC_NO_TEXT_AGENT`).
//! On SteamOS only (`ID=steamos` in /etc/os-release), the setup — made in
//! Desktop Mode, behind the password — also writes
//!
//! ```text
//! /home/.proxysvpn/gaming-mode-user          root:root 0644, one user name
//! ```
//!
//! naming the user who authorized it, and every start of the installed helper
//! keeps one polkit rule in step with that record ([`sync_rule`]):
//!
//! ```text
//! /etc/polkit-1/rules.d/49-proxysvpn.rules   root:root 0644
//! ```
//!
//! It lets that one user, at the device and in the active session
//! (`subject.local && subject.active`), run exactly
//! `/home/.proxysvpn/bin/proxysvpn-helper --helper` as root without a
//! password: that program, that argv, as root, nothing else ([`polkit_rule`]).
//!
//! What that leaves on the device, precisely: any program running as that
//! user may start the helper as root without asking. The helper does only
//! what its protocol lets a peer ask (helper/proto.rs): raise our TUN device,
//! send the machine's traffic to a SOCKS listener on 127.0.0.1:10808 or 10809
//! that belongs to root or to that same user (`check_socks_listener`), pin a
//! host route to one unicast IPv4 address, publish resolvers on our device,
//! and take all of it down. It executes nothing but the tun2socks beside it in
//! the same root-owned folder. So such a program gains one thing it could not
//! do before: route the whole machine through a proxy it runs itself. It does
//! not gain code execution as root, any file, or anything that outlasts the
//! helper — the device, its routes and resolved's per-link servers go with it
//! (crash recovery in net/linux_priv.rs). That is the same power a .deb install
//! hands out for polkit's `auth_admin_keep` minutes after every connect, here
//! without the time limit, which is the price of a Gaming Mode that cannot ask.
//!
//! Why acceptable and not worse: no file that carries privilege sits where a
//! normal user can change it — no setuid bit, no file capability, the rule
//! names a path whose every folder is root's alone (and [`sync_rule`] removes
//! the rule instead of writing it if one is not). Rejected: `setcap` on a
//! binary (a capability-carrying file, and `cap_net_admin` is not inherited by
//! tun2socks anyway, docs/LINUX.md); a sudoers drop-in (the same grant through
//! a second mechanism the rest of the app does not use); a root service that
//! runs all the time (more surface, and a design of its own).
//!
//! The rule is written by the helper, not by the setup script, so that one
//! function renders it and a SteamOS update that resets /etc (it usually keeps
//! it) is repaired by the next start in Desktop Mode instead of leaving Gaming
//! Mode broken with nothing to say why.
//!
//! The choice is kept in a file of its own, never read from the folder's age:
//! the setup writes the record only when it finds neither the record nor
//! [`GAMING_OFF`], the explicit "no". So a first setup cut short after it made
//! the folder still records the user when it is run again, and switching the
//! grant off (writing [`GAMING_OFF`], docs/STEAMDECK.md) holds through app
//! updates, until /home/.proxysvpn is removed.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
// Reason for the allow: only the Linux GUI and helper call into this, but the
// rules and the setup script are tested on the developer's Mac as well — the
// only machine available for the Linux port.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The root-owned folder the AppImage's helper is copied into.
pub const INSTALL_DIR: &str = "/home/.proxysvpn";
/// The folder under [`INSTALL_DIR`] the two executables live in.
pub const BIN_DIR: &str = "bin";
/// The standalone helper, as the AppImage carries it in `usr/bin`.
pub const HELPER_BIN: &str = "proxysvpn-helper";
/// The only engine root runs; the helper finds it beside itself.
pub const ENGINE_BIN: &str = super::proto::SIDECAR_NAME;
/// The record, in [`INSTALL_DIR`], of the user Gaming Mode may start the
/// helper for (SteamOS only).
pub const GAMING_RECORD: &str = "gaming-mode-user";
/// The record, in [`INSTALL_DIR`], that Gaming Mode was switched off: the
/// "no" that keeps a later setup from writing [`GAMING_RECORD`] again. Its
/// presence is all it says.
pub const GAMING_OFF: &str = "gaming-mode-off";
/// The setup's fifth argument on SteamOS: write the record if nothing was
/// decided yet.
pub const GAMING_FLAG: &str = "gaming-mode";
/// The polkit rule the installed helper keeps in step with the record. 49, so
/// it is read before the distribution's 50-default.rules.
pub const POLKIT_RULE: &str = "/etc/polkit-1/rules.d/49-proxysvpn.rules";
/// The longest record that can hold a user name.
const RECORD_MAX: u64 = 64;
/// The shell the setup script runs under. A system file in a root-owned
/// directory on every Linux, and on SteamOS part of the read-only image.
pub const SHELL: &str = "/bin/sh";
/// `$0` of the setup script. The shell prefixes its own error messages with
/// it, and the script prefixes its own, so a failure of the setup is told
/// apart from pkexec's own exit codes by this word on stderr
/// (`privilege::handshake_failure`).
pub const SETUP_NAME: &str = "proxysvpn-setup";

/// The setup script's own exit codes, from sysexits.h, clear of pkexec's
/// 126 and 127: a folder of the chain is not root's alone.
pub const EXIT_FOLDER: i32 = 73;
/// The staging folder is not the person's alone, or a staged file is missing
/// or does not hash to its sum.
pub const EXIT_FILES: i32 = 65;
/// A copy or a hash could not be made (a full disk).
pub const EXIT_COPY: i32 = 74;

/// How [`SETUP_BODY`] reads a staged file `$src` into its temp copy `$tmp`:
/// the one open root makes of a path the person controls. `nofollow`: a link
/// in the file's place fails the open (ELOOP) instead of leading root to
/// /etc/shadow or a device. `nonblock`: a FIFO in its place gives nothing or
/// EAGAIN at once instead of holding root until someone writes to it. 256 MiB
/// at most, far above the helper and tun2socks. GNU coreutils: SteamOS,
/// Debian, Fedora, Arch; the tests run it as written (macOS gets a stand-in).
pub const STAGED_OPEN: &str = r#"dd if="$src" of="$tmp" bs=1048576 count=256 iflag=nofollow,nonblock status=none"#;

/// What the setup does about Gaming Mode, as the GUI found the device. Only
/// the words the password window shows depend on it ([`setup_script`],
/// [`setup_args`]); the script itself decides from the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gaming {
    /// Not SteamOS: no record, no rule.
    Off,
    /// SteamOS, and the record or the "no" is already there: kept as it is.
    Kept,
    /// SteamOS, and nothing was decided yet: this setup writes the record, so
    /// from then on this user starts the helper without a password.
    Granted,
}

// ── What the password window says ─────────────────────────────────────────
// The AppImage cannot install a polkit policy of its own, so pkexec asks with
// its generic action, and its message is "Authentication is needed to run
// `$(cmdline_short)' as the super user" (pkexec.c). `cmdline_short` is the
// command line — `/bin/sh -c <script> <arguments>` joined with spaces — cut,
// once longer than 80 bytes, to its first 38 bytes, " ... " and its last 37.
// KDE Plasma's agent, the one in SteamOS's Desktop Mode, shows that message
// and no other detail of the command. So the script's first line starts with
// the 27 bytes that follow `/bin/sh -c ` in it, and the last argument, which
// the script never reads, is the 37 bytes that end it: "/bin/sh -c #
// ProxysVPN sets up its VPN ... so Gaming Mode then needs no password". The
// whole first line says it in full for whoever reads the command (the
// journal, `ps`, an agent that shows it).
//
// English and ASCII only: pkexec cuts at byte offsets, and a cut through a
// Cyrillic letter would hand polkit a message that is not UTF-8. The app's
// own screen says it first, in the person's language (`gamingMode` in
// onboarding, src/components/OnboardingScreen.tsx).

/// The first line of a setup that grants nothing new.
const SETUP_HEAD: &str = "# ProxysVPN sets up its VPN helper as root in /home/.proxysvpn and starts it. To undo: ProxysVPN > More > Remove system files";
/// [`SETUP_HEAD`] for a setup that grants Gaming Mode.
const SETUP_HEAD_GRANT: &str = "# ProxysVPN sets up its VPN helper as root in /home/.proxysvpn and lets this user start it without a password from now on, so Gaming Mode can connect. To undo: ProxysVPN > More > Remove system files";
/// The end of the window's message for a setup that grants nothing new.
pub const DIALOG_TAIL: &str = "helper as root and starts the tunnel.";
/// The end of the window's message for a setup that grants Gaming Mode.
pub const DIALOG_TAIL_GRANT: &str = "so Gaming Mode then needs no password";

/// The script root runs, through pkexec, to make the copy and start it:
/// [`SETUP_BODY`] after its first line.
///
/// Arguments: `$1` the install folder ([`INSTALL_DIR`] with its parent
/// resolved to the physical folder, so `/var/home/.proxysvpn` where `/home`
/// is a link), `$2` the staging
/// folder, `$3` and `$4` the SHA-256 of the helper and of tun2socks as the GUI
/// read them in its mount, `$5` [`GAMING_FLAG`] on SteamOS and `-` elsewhere,
/// `$6` the end of what the password window shows ([`DIALOG_TAIL`]), never
/// read. The user the record names is pkexec's `PKEXEC_UID`, the person who
/// typed the password, never an argument. Nothing in it reads stdin and
/// nothing writes to stdout: both are the protocol pipe the exec'd helper
/// inherits, and the GUI's first request is already waiting in it. POSIX sh
/// and GNU coreutils only, so dash (Debian) and bash as sh (SteamOS, Arch) run
/// it the same; the unit tests run it too. GNU, not just POSIX, for one thing:
/// dd's `iflag=nofollow,nonblock`, the one open of a staged file root makes
/// ([`STAGED_OPEN`]).
pub fn setup_script(gaming: Gaming) -> String {
    let head = if gaming == Gaming::Granted { SETUP_HEAD_GRANT } else { SETUP_HEAD };
    format!("{head}\n{SETUP_BODY}")
}

/// The setup script below its first line ([`setup_script`]).
pub const SETUP_BODY: &str = r#"set -eu
dest=$1 staged=$2 helper_sum=$3 engine_sum=$4 gaming=$5
umask 077
me=$(id -u) uid=${PKEXEC_UID:?}
say() { printf 'proxysvpn-setup: %s\n' "$*" >&2; }
owned() {
    [ -d "$1" ] && [ ! -L "$1" ] || return 1
    [ -n "$(find "$1" -maxdepth 0 -user "$2" ! -perm -0020 ! -perm -0002 -print)" ]
}
parent=$(dirname -- "$dest")
owned "$parent" "$me" || { say "$parent is not a folder only uid $me can change"; exit 73; }
owned "$staged" "$uid" || { say "$staged is not a folder only uid $uid can change"; exit 65; }
for dir in "$dest" "$dest/bin"; do
    [ -e "$dir" ] || [ -L "$dir" ] || mkdir -m 0755 -- "$dir"
    owned "$dir" "$me" || { say "$dir is not a folder only uid $me can change"; exit 73; }
done
decided() { [ -e "$dest/$1" ] || [ -L "$dest/$1" ]; }
if [ "$gaming" != gaming-mode ]; then
    rm -f -- "$dest/gaming-mode-user"
elif ! decided gaming-mode-user && ! decided gaming-mode-off; then
    user=$(id -nu "$uid")
    printf '%s\n' "$user" > "$dest/.gaming-mode-user.new"
    chmod 0644 "$dest/.gaming-mode-user.new"
    mv -f -- "$dest/.gaming-mode-user.new" "$dest/gaming-mode-user"
fi
take() {
    src=$staged/$1 tmp=$dest/bin/.$1.new
    rm -f -- "$tmp"
    [ -f "$src" ] && [ ! -L "$src" ] || { say "$1 was not staged"; exit 65; }
    dd if="$src" of="$tmp" bs=1048576 count=256 iflag=nofollow,nonblock status=none || { rm -f -- "$tmp"; say "$1 could not be copied"; exit 74; }
    sum=$(sha256sum < "$tmp") || { rm -f -- "$tmp"; say "$1 could not be hashed"; exit 74; }
    if [ "${sum%% *}" != "$2" ]; then rm -f -- "$tmp"; say "$1 does not match its checksum"; exit 65; fi
    chmod 0755 "$tmp"
    [ ! -d "$dest/bin/$1" ] || rm -rf -- "$dest/bin/$1"
    mv -f -- "$tmp" "$dest/bin/$1"
}
take proxysvpn-helper "$helper_sum"
take tun2socks "$engine_sum"
exec "$dest/bin/proxysvpn-helper" --helper
"#;

/// `$0` of the removal script ([`REMOVE_SCRIPT`]); its own lines and the
/// shell's start with it, as the setup's start with [`SETUP_NAME`].
pub const REMOVE_NAME: &str = "proxysvpn-remove";
/// The end of the password window's message for the removal (see "What the
/// password window says" above): "/bin/sh -c # ProxysVPN: remove helper,
/// ... and the Gaming Mode no-password rule."
pub const REMOVE_TAIL: &str = "and the Gaming Mode no-password rule.";

/// What "Remove system files" (More, the Linux AppImage only) runs as root
/// through one pkexec: the rule first, so that from that moment nothing runs
/// as root without a password, then the install folder with the helper, the
/// engine, the record and the "no" in it.
///
/// Arguments: `$1` the physical install folder, `$2` [`POLKIT_RULE`], `$3`
/// [`REMOVE_TAIL`], never read. It removes nothing but those two paths and
/// refuses any other name, so it can only ever remove ProxysVPN's own: a
/// folder named `.proxysvpn` and a file named `49-proxysvpn.rules`, both
/// absolute and without `.` or `..` in them. Links are removed, never
/// followed: `rm` unlinks the link itself, and `rm -rf` does not follow links
/// inside the folder. The folder must be root's alone, as the setup left it;
/// one that is not was not made by the setup, and is left for a person to
/// look at. Nothing there at all is success: removing twice is fine.
///
/// The helper's crash-recovery notes (/run/proxysvpn, /var/lib/proxysvpn) are
/// left alone: the second may hold the resolv.conf to put back, and the .deb
/// shares them. docs/STEAMDECK.md lists them for removal by hand.
pub const REMOVE_SCRIPT: &str = r#"# ProxysVPN: remove helper, the copy in /home/.proxysvpn and the Gaming Mode no-password rule /etc/polkit-1/rules.d/49-proxysvpn.rules (More > Remove system files)
set -eu
dest=$1 rule=$2
me=$(id -u)
say() { printf 'proxysvpn-remove: %s\n' "$*" >&2; }
case $dest in /*/.proxysvpn) ;; *) say "$dest is not ProxysVPN's folder"; exit 64 ;; esac
case $rule in /*/49-proxysvpn.rules) ;; *) say "$rule is not ProxysVPN's rule"; exit 64 ;; esac
case "$dest/ $rule/" in *//*|*/./*|*/../*) say "$dest or $rule is not a plain path"; exit 64 ;; esac
if [ -L "$rule" ] || [ -f "$rule" ]; then
    rm -f -- "$rule"
elif [ -e "$rule" ]; then
    say "$rule is not a file"; exit 65
fi
if [ -L "$dest" ]; then
    rm -f -- "$dest"
elif [ -d "$dest" ]; then
    [ -n "$(find "$dest" -maxdepth 0 -user "$me" ! -perm -0020 ! -perm -0002 -print)" ] || { say "$dest is not a folder only uid $me can change"; exit 73; }
    rm -rf -- "$dest"
elif [ -e "$dest" ]; then
    say "$dest is not a folder"; exit 65
fi
"#;

/// The arguments for `pkexec /bin/sh …` that run [`REMOVE_SCRIPT`].
pub fn remove_args(install_dir: &Path, rule: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-c"),
        OsString::from(REMOVE_SCRIPT),
        OsString::from(REMOVE_NAME),
        install_dir.as_os_str().to_os_string(),
        rule.as_os_str().to_os_string(),
        OsString::from(REMOVE_TAIL),
    ]
}

/// Why "Remove system files" did not remove them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveFailure {
    /// pkexec is not installed.
    NoPkexec,
    /// The password window was dismissed, or the password refused.
    Refused,
    /// No polkit agent, so no window could appear.
    NoAgent,
    /// No polkit agent on SteamOS: Gaming Mode. Desktop Mode has one.
    UseDesktopMode,
    /// The script stopped, or pkexec failed some other way; the text says
    /// why, for the log.
    Failed(String),
}

impl std::fmt::Display for RemoveFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPkexec => f.write_str("pkexec is not installed"),
            Self::Refused => f.write_str("the password window was dismissed or refused"),
            Self::NoAgent => f.write_str("no polkit agent in this session"),
            Self::UseDesktopMode => f.write_str("no polkit agent in Gaming Mode"),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for RemoveFailure {}

/// Read a finished removal from pkexec's exit code and stderr. The script's
/// own lines (and the shell's, which start with [`REMOVE_NAME`] too) are
/// checked first: 126 and 127 from the shell are not the person's answer.
pub fn removal_outcome(exit: Option<i32>, stderr: &str, steamos: bool) -> Result<(), RemoveFailure> {
    use crate::privilege::{handshake_failure, HandshakeFailure};

    if exit == Some(0) {
        return Ok(());
    }
    let own: Vec<&str> = stderr.lines().filter(|line| line.contains(REMOVE_NAME)).collect();
    if !own.is_empty() {
        return Err(RemoveFailure::Failed(own.join("; ")));
    }
    match handshake_failure(exit, stderr, false) {
        HandshakeFailure::Dismissed | HandshakeFailure::NotAuthorized => Err(RemoveFailure::Refused),
        HandshakeFailure::NoAgent if steamos => Err(RemoveFailure::UseDesktopMode),
        HandshakeFailure::NoAgent => Err(RemoveFailure::NoAgent),
        HandshakeFailure::SetupFailed(_) | HandshakeFailure::Other => Err(RemoveFailure::Failed(format!(
            "pkexec ended with {exit:?}: {}",
            stderr.lines().next().unwrap_or("no message")
        ))),
    }
}

/// Where an installed helper lives under an install folder.
pub fn installed_helper(install_dir: &Path) -> PathBuf {
    install_dir.join(BIN_DIR).join(HELPER_BIN)
}

/// How this copy of the app was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// From the .deb, a dev checkout, or anything else that is not an
    /// AppImage: the helper is our own executable, as before.
    Package,
    /// From an AppImage: the helper is the root-owned copy.
    AppImage,
}

/// Is this process the AppImage's own executable?
///
/// The runtime exports `APPIMAGE` (the image file) and `APPDIR` (where it is
/// mounted, or extracted). Both are inherited by whatever the AppImage starts,
/// so their presence alone does not say that *we* are inside it: the .deb's
/// app started from a terminal that is itself an AppImage would inherit them.
/// Only an executable under `APPDIR` counts. `appdir` must already be
/// canonical, as `exe` from /proc/self/exe is.
pub fn launch_kind(appimage: Option<&OsStr>, appdir: Option<&Path>, exe: &Path) -> Launch {
    let (Some(image), Some(dir)) = (appimage, appdir) else {
        return Launch::Package;
    };
    if image.is_empty() || !dir.is_absolute() || dir == Path::new("/") {
        return Launch::Package;
    }
    if exe != dir && exe.starts_with(dir) {
        Launch::AppImage
    } else {
        Launch::Package
    }
}

/// The arguments for `pkexec /bin/sh …` that run [`setup_script`].
pub fn setup_args(
    install_dir: &Path,
    staged: &Path,
    helper_sum: &str,
    engine_sum: &str,
    gaming: Gaming,
) -> Vec<OsString> {
    vec![
        OsString::from("-c"),
        OsString::from(setup_script(gaming)),
        OsString::from(SETUP_NAME),
        install_dir.as_os_str().to_os_string(),
        staged.as_os_str().to_os_string(),
        OsString::from(helper_sum),
        OsString::from(engine_sum),
        OsString::from(if gaming == Gaming::Off { "-" } else { GAMING_FLAG }),
        OsString::from(if gaming == Gaming::Granted { DIALOG_TAIL_GRANT } else { DIALOG_TAIL }),
    ]
}

/// Is this SteamOS? `ID=steamos` in os-release(5), quoted or not.
pub fn is_steamos(os_release: &str) -> bool {
    os_release
        .lines()
        .filter_map(|line| line.trim().strip_prefix("ID="))
        .any(|value| value.trim().trim_matches(|c| c == '"' || c == '\'') == "steamos")
}

/// A user name the rule may quote: what useradd accepts, and nothing that
/// could close the JavaScript string it is put into.
pub fn valid_user_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    name.len() <= 32
        && (first.is_ascii_alphanumeric() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// A path the rule may quote: absolute, and only characters that need no
/// escaping in a JavaScript string.
pub fn is_plain_path(path: &str) -> bool {
    path.starts_with('/')
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
}

/// The user a record names, if it holds exactly one valid name.
pub fn parse_gaming_record(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let name = text.strip_suffix('\n').unwrap_or(text);
    valid_user_name(name).then(|| name.to_string())
}

/// The polkit rule that lets `user` start the helper at `helper` without a
/// password, or `None` when either would need escaping. `helper` is the
/// physical path: pkexec matches `program` after realpath(3), and passes
/// `command_line` as the GUI spelled it, which is the same physical path.
pub fn polkit_rule(user: &str, helper: &Path) -> Option<String> {
    if !valid_user_name(user) {
        return None;
    }
    let helper = helper.to_str().filter(|path| is_plain_path(path))?;
    Some(format!(
        r#"// {POLKIT_RULE}
//
// Written by the ProxysVPN tunnel helper (docs/STEAMDECK.md). It lets {user},
// at this device and in the active session, start
//   {helper} --helper
// as root without a password, so the VPN can connect in Gaming Mode, where no
// password window can appear. It allows nothing else.
//
// The helper rewrites this file from {INSTALL_DIR}/{GAMING_RECORD}
// at every start. To withdraw it, create {INSTALL_DIR}/{GAMING_OFF}
// and remove this file, or remove {INSTALL_DIR} entirely.
polkit.addRule(function (action, subject) {{
    if (action.id === "org.freedesktop.policykit.exec" &&
        action.lookup("program") === "{helper}" &&
        action.lookup("command_line") === "{helper} --helper" &&
        action.lookup("user") === "root" &&
        subject.user === "{user}" &&
        subject.local && subject.active) {{
        return polkit.Result.YES;
    }}
}});
"#
    ))
}

/// A lowercase hex SHA-256, the only shape a sum may have on the command line.
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// What `lstat` says about one path, kept to what the rules below need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeFacts {
    pub uid: u32,
    /// st_mode as reported by lstat(2).
    pub mode: u32,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
    /// A link, a FIFO, a device: never acceptable anywhere in the install.
    Other,
}

/// May root trust a folder not to change under it: a real folder (not a
/// link), owned by root, with no group or world write bit?
pub fn folder_is_root_only(path: &Path, facts: &NodeFacts) -> Result<(), String> {
    if facts.kind != NodeKind::Dir {
        return Err(format!("{} is not a plain folder", path.display()));
    }
    if facts.uid != 0 {
        return Err(format!("{} belongs to uid {}, not root", path.display(), facts.uid));
    }
    if facts.mode & 0o022 != 0 {
        return Err(format!(
            "{} is writable by others (mode {:o})",
            path.display(),
            facts.mode & 0o7777
        ));
    }
    Ok(())
}

/// One installed file as found, against the sum it must have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCheck<'a> {
    pub name: &'a str,
    /// `None` when the file is not there.
    pub found: Option<(NodeFacts, Option<String>)>,
    pub want: &'a str,
}

/// The state of the root-owned copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// Both files are root's, 0755, and hold exactly what this AppImage
    /// carries: run the installed helper.
    Current,
    /// Never installed (or partly): run the setup.
    Missing,
    /// Installed, but from another release, or with a mode or owner the setup
    /// would not have left: run the setup, which replaces them.
    Stale(String),
    /// A folder on the way is not root's alone. The setup would refuse it,
    /// and running anything from there as root would be the very hole this
    /// module exists to avoid. Someone has to look at the machine.
    Unsafe(String),
}

/// Decide what to start, from facts gathered on the machine.
///
/// `parents` are the folders above the install folder (`/`, `/home`): they
/// must exist and be root's alone. `own` are the install folder and its `bin`:
/// missing is fine (the setup creates them), present but not root's alone is
/// not. `files` are the helper and tun2socks.
pub fn judge(
    parents: &[(PathBuf, Option<NodeFacts>)],
    own: &[(PathBuf, Option<NodeFacts>)],
    files: &[FileCheck<'_>],
) -> Installed {
    for (path, facts) in parents {
        let Some(facts) = facts else {
            return Installed::Unsafe(format!("{} is missing", path.display()));
        };
        if let Err(reason) = folder_is_root_only(path, facts) {
            return Installed::Unsafe(reason);
        }
    }
    for (path, facts) in own {
        let Some(facts) = facts else {
            return Installed::Missing;
        };
        if let Err(reason) = folder_is_root_only(path, facts) {
            return Installed::Unsafe(format!("{reason} — remove it and connect again"));
        }
    }
    let mut stale = None;
    for check in files {
        let Some((facts, sum)) = &check.found else {
            return Installed::Missing;
        };
        if stale.is_some() {
            continue;
        }
        if facts.kind != NodeKind::File || facts.uid != 0 || facts.mode & 0o7777 != 0o755 {
            stale = Some(format!("{} is not a root-owned 0755 file", check.name));
        } else if sum.as_deref() != Some(check.want) {
            stale = Some(format!("{} is from another release", check.name));
        }
    }
    match stale {
        Some(reason) => Installed::Stale(reason),
        None => Installed::Current,
    }
}

/// What `lstat` says about `path`, or `None` when there is nothing there.
#[cfg(unix)]
pub fn node_facts(path: &Path) -> Option<NodeFacts> {
    use std::os::unix::fs::MetadataExt;

    let meta = std::fs::symlink_metadata(path).ok()?;
    let kind = if meta.file_type().is_file() {
        NodeKind::File
    } else if meta.file_type().is_dir() {
        NodeKind::Dir
    } else {
        NodeKind::Other
    };
    Some(NodeFacts {
        uid: meta.uid(),
        mode: meta.mode(),
        kind,
    })
}

/// Was Gaming Mode decided on this device: is the record or the "no" in
/// `install_dir`, whatever it holds? Read by the GUI, which may look into the
/// root-owned folder (0755) but not change it, to say what the setup will do.
#[cfg(unix)]
pub fn gaming_decided(install_dir: &Path) -> bool {
    [GAMING_RECORD, GAMING_OFF]
        .iter()
        .any(|name| node_facts(&install_dir.join(name)).is_some())
}

/// What the setup does about Gaming Mode on this device ([`Gaming`]).
#[cfg(unix)]
pub fn gaming_for(steamos: bool, install_dir: &Path) -> Gaming {
    if !steamos {
        Gaming::Off
    } else if gaming_decided(install_dir) {
        Gaming::Kept
    } else {
        Gaming::Granted
    }
}

/// Open `path` for reading only if it is a regular file, never through a
/// link. O_NONBLOCK so a FIFO in its place is refused at once instead of
/// blocking the open (the reasoning of src/engine_stage.rs).
#[cfg(unix)]
fn open_regular(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    Ok(file)
}

/// Lowercase hex SHA-256 of a regular file.
#[cfg(unix)]
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};

    let mut file = open_regular(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Copy `src` into the fresh, private `dir` under its own name, through one
/// descriptor on each side.
#[cfg(unix)]
fn stage_one(src: &Path, dir: &Path, name: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut from = open_regular(src)?;
    let mut to = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(name))?;
    std::io::copy(&mut from, &mut to)?;
    Ok(())
}

/// Put the two files where root can read them: a fresh owner-only folder
/// `dir`, replacing whatever an earlier attempt left there.
#[cfg(unix)]
pub fn stage(dir: &Path, helper: &Path, engine: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_dir() => std::fs::remove_dir_all(dir)?,
        Ok(_) => std::fs::remove_file(dir)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::DirBuilder::new().mode(0o700).create(dir)?;
    stage_one(helper, dir, HELPER_BIN)?;
    stage_one(engine, dir, ENGINE_BIN)?;
    Ok(())
}

// ------------------------------------------------------------------ root side

/// What the installed helper did about the Gaming Mode rule as it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleSync {
    /// This helper is not the AppImage's installed copy — the .deb's, a dev
    /// run, a copy replaced while it ran: polkit is not ours to touch.
    NotOurs,
    /// The rule already says what the record says (or both are absent).
    Unchanged,
    /// The rule was written for this user.
    Written(String),
    /// The record is gone, so the rule went too.
    Removed,
    /// A folder on the rule's path is not root's alone, the record is not a
    /// root-only file holding one user name, or a write failed. The rule is
    /// removed in the first two cases; the text says what.
    Failed(String),
}

/// A folder of the chain the rule names: a real folder of root (or, in tests,
/// of `owner`), with no group or world write bit.
#[cfg(unix)]
fn trusted_folder(path: &Path, owner: u32) -> Result<(), String> {
    let facts = node_facts(path).ok_or_else(|| format!("{} is missing", path.display()))?;
    let as_root = NodeFacts { uid: if facts.uid == owner { 0 } else { facts.uid }, ..facts };
    folder_is_root_only(path, &as_root)
}

/// Read the record if it is a regular file of `owner` that nobody else may
/// write; `Ok(None)` when there is none.
#[cfg(unix)]
fn read_record(path: &Path, owner: u32) -> Result<Option<String>, String> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;

    let file = match open_regular(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let meta = file.metadata().map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.uid() != owner || meta.mode() & 0o022 != 0 {
        return Err(format!("{} is not root's alone", path.display()));
    }
    let mut bytes = Vec::new();
    file.take(RECORD_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > RECORD_MAX {
        return Err(format!("{} is too long to be a user name", path.display()));
    }
    parse_gaming_record(&bytes)
        .map(Some)
        .ok_or_else(|| format!("{} does not hold a user name", path.display()))
}

/// Remove the rule file if there is one. It is ours by name.
#[cfg(unix)]
fn remove_rule(rule: &Path) -> Result<bool, String> {
    match std::fs::remove_file(rule) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("could not remove {}: {e}", rule.display())),
    }
}

/// Write `text` to `rule` through a temp file in the same folder, renamed into
/// place. polkit loads only names ending in `.rules`, so it never reads the
/// temp file half written.
#[cfg(unix)]
fn write_rule(rule: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let dir = rule.parent().ok_or_else(|| "the rule has no folder".to_string())?;
    if !dir.is_dir() {
        return Err(format!("{} does not exist; is polkit installed?", dir.display()));
    }
    let name = rule.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)?;
        file.write_all(text.as_bytes())?;
        // polkitd reads rules as its own user; the umask must not take that away.
        file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
        std::fs::rename(&tmp, rule)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("could not write {}: {e}", rule.display()))
}

/// Bring the rule at `rule` in step with the record in `install_dir`, when
/// `exe` is the helper installed there. `install_dir` is physical; `top` is
/// the highest folder whose trust is checked (`/` for real, the test's own
/// folder in tests); `owner` is root's uid for real, the test runner's in
/// tests.
#[cfg(unix)]
pub fn sync_rule(exe: &Path, install_dir: &Path, top: &Path, rule: &Path, owner: u32) -> RuleSync {
    let helper = installed_helper(install_dir);
    if exe != helper {
        return RuleSync::NotOurs;
    }
    // Fail closed: when it is not clear whom the rule may name, or the path
    // it names could be swapped, there is no rule.
    let withdraw = |reason: String| match remove_rule(rule) {
        Ok(_) => RuleSync::Failed(format!("{reason}; no Gaming Mode rule while it is")),
        Err(e) => RuleSync::Failed(format!("{reason}; {e}")),
    };
    // Every folder the rule's path runs through must be root's alone, or the
    // rule would hand out root to whoever can swap the file under it.
    let chain = helper.parent().map(Path::ancestors).into_iter().flatten();
    for dir in chain {
        if let Err(reason) = trusted_folder(dir, owner) {
            return withdraw(reason);
        }
        if dir == top {
            break;
        }
    }

    // The explicit "no" outweighs any record: Gaming Mode was switched off.
    let user = if node_facts(&install_dir.join(GAMING_OFF)).is_some() {
        Ok(None)
    } else {
        read_record(&install_dir.join(GAMING_RECORD), owner)
    };
    let user = match user {
        Ok(user) => user,
        Err(reason) => return withdraw(reason),
    };
    let Some(user) = user else {
        return match remove_rule(rule) {
            Ok(true) => RuleSync::Removed,
            Ok(false) => RuleSync::Unchanged,
            Err(e) => RuleSync::Failed(e),
        };
    };
    let Some(text) = polkit_rule(&user, &helper) else {
        return RuleSync::Failed(format!("{} cannot be named in a rule", helper.display()));
    };
    let current = open_regular(rule).ok().and_then(|mut file| {
        use std::io::Read;
        let mut held = String::new();
        file.read_to_string(&mut held).ok().map(|_| held)
    });
    if current.as_deref() == Some(text.as_str()) {
        return RuleSync::Unchanged;
    }
    match write_rule(rule, &text) {
        Ok(()) => RuleSync::Written(user),
        Err(e) => RuleSync::Failed(e),
    }
}

/// [`sync_rule`] for the running helper, with the real paths.
#[cfg(target_os = "linux")]
pub fn sync_gaming_rule() -> RuleSync {
    let (Ok(exe), Ok(install_dir)) = (std::env::current_exe(), std::fs::canonicalize(INSTALL_DIR)) else {
        return RuleSync::NotOurs;
    };
    sync_rule(&exe, &install_dir, Path::new("/"), Path::new(POLKIT_RULE), 0)
}

// ------------------------------------------------------------- GUI side, Linux

/// Is this process the AppImage's own executable? [`launch_kind`] with this
/// process's environment and executable.
#[cfg(target_os = "linux")]
pub fn this_launch() -> Launch {
    let Ok(exe) = std::env::current_exe() else {
        return Launch::Package;
    };
    let appdir = std::env::var_os("APPDIR")
        .filter(|dir| !dir.is_empty())
        .and_then(|dir| std::fs::canonicalize(dir).ok());
    launch_kind(std::env::var_os("APPIMAGE").as_deref(), appdir.as_deref(), &exe)
}

/// `install_dir` with its parent resolved to the physical folder and its own
/// name kept as it is. pkexec runs realpath(3) on the program it is given,
/// and a /home that is a link (/var/home on Fedora's atomic desktops, Bazzite
/// among them) must be judged — and handed to the scripts, which refuse a
/// linked parent — as the folder it really is. The install folder itself is
/// never resolved: a link in its place (nothing of ours makes one) is judged
/// as a link ([`judge`]: unsafe) and removed as a link by [`REMOVE_SCRIPT`],
/// never followed to what it points at, which the removal's `rm -rf` would
/// otherwise take.
#[cfg(unix)]
pub fn physical_install_dir(install_dir: &Path) -> PathBuf {
    let (Some(parent), Some(name)) = (install_dir.parent(), install_dir.file_name()) else {
        return install_dir.to_path_buf();
    };
    std::fs::canonicalize(parent)
        .unwrap_or_else(|_| parent.to_path_buf())
        .join(name)
}

/// The person read the app's screen about Gaming Mode in this run of the
/// app (`gamingMode` in onboarding). Not kept on disk: the screen comes back
/// at the next start for as long as nothing was decided on the device.
static GAMING_NOTICE_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Must the window explain, before the first setup, that it will let this
/// user start the helper without a password? On the SteamOS AppImage, while
/// nothing was decided on the device ([`gaming_decided`]) and the screen was
/// not read in this run. Never anywhere else.
pub fn gaming_notice_pending() -> bool {
    #[cfg(target_os = "linux")]
    {
        !GAMING_NOTICE_SEEN.load(std::sync::atomic::Ordering::Relaxed)
            && running_on_steamos()
            && this_launch() == Launch::AppImage
            && !gaming_decided(&physical_install_dir(Path::new(INSTALL_DIR)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// "Continue" on that screen.
pub fn acknowledge_gaming_notice() {
    GAMING_NOTICE_SEEN.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Does More offer "Remove system files"? On the Linux AppImage only: the
/// .deb's files belong to the package manager (`apt remove`), and macOS and
/// Windows put nothing of this kind into the system.
pub fn system_files_removable() -> bool {
    #[cfg(target_os = "linux")]
    {
        this_launch() == Launch::AppImage
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// How long the password window may stay open for the removal, as for the
/// first connect (net/linux.rs).
#[cfg(target_os = "linux")]
const REMOVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// "Remove system files": stop the helper, then remove the install folder and
/// the Gaming Mode rule through one pkexec of [`REMOVE_SCRIPT`]. The tunnel
/// must already be down; the next connect sets everything up again.
#[cfg(target_os = "linux")]
pub async fn remove_system_files() -> Result<(), RemoveFailure> {
    use std::process::Stdio;

    // The helper runs from the folder about to go and holds the tunnel: its
    // stdin closes, and it takes everything down and exits.
    crate::net::release_helper().await;
    let pkexec = crate::privilege::which("pkexec").ok_or(RemoveFailure::NoPkexec)?;
    let dest = physical_install_dir(Path::new(INSTALL_DIR));
    let mut child = tokio::process::Command::new(pkexec)
        .args(crate::privilege::pkexec_args(Path::new(SHELL), remove_args(&dest, Path::new(POLKIT_RULE))))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| RemoveFailure::Failed(format!("could not start pkexec: {e}")))?;
    let mut stderr = String::new();
    let mut pipe = child.stderr.take();
    let waited = tokio::time::timeout(REMOVE_TIMEOUT, async {
        if let Some(pipe) = pipe.as_mut() {
            use tokio::io::AsyncReadExt;
            // Bounded: pkexec and the script say a few lines at most.
            let _ = pipe.take(64 * 1024).read_to_string(&mut stderr).await;
        }
        child.wait().await
    })
    .await;
    let status = match waited {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => return Err(RemoveFailure::Failed(format!("could not wait for pkexec: {e}"))),
        Err(_) => return Err(RemoveFailure::Failed("the password window stayed open too long".to_string())),
    };
    for line in stderr.lines().filter(|line| !line.trim().is_empty()) {
        crate::log::warn("helper", line);
    }
    let outcome = removal_outcome(status.code(), &stderr, running_on_steamos());
    if outcome.is_ok() {
        crate::log::info("helper", "AppImage: removed /home/.proxysvpn and the Gaming Mode rule");
        // Nothing is decided on the device any more: before the next setup
        // grants Gaming Mode again, the window says so again.
        GAMING_NOTICE_SEEN.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    outcome
}

/// Elsewhere there is nothing of this kind to remove.
#[cfg(not(target_os = "linux"))]
pub async fn remove_system_files() -> Result<(), RemoveFailure> {
    Err(RemoveFailure::Failed("only the Linux AppImage puts files into the system".to_string()))
}

/// Is this SteamOS? Read once per process.
#[cfg(target_os = "linux")]
pub fn running_on_steamos() -> bool {
    static STEAMOS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *STEAMOS.get_or_init(|| {
        ["/etc/os-release", "/usr/lib/os-release"]
            .iter()
            .find_map(|path| std::fs::read_to_string(path).ok())
            .is_some_and(|text| is_steamos(&text))
    })
}

/// What the GUI hands pkexec, and what to clean up once it answered.
#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct SpawnPlan {
    /// The program pkexec runs: the installed helper, or [`SHELL`].
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// The staging folder, removed after the handshake either way.
    pub staged: Option<PathBuf>,
    /// True when this start makes the copy first.
    pub setup: bool,
}

/// Where the GUI stages the two files for root: `$XDG_RUNTIME_DIR` (a private
/// tmpfs of this user) when the session has one, else the state folder.
/// Local either way, so root can read it even where /home is on NFS with
/// root squashed.
#[cfg(target_os = "linux")]
fn staging_dir() -> anyhow::Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute() && dir.is_dir());
    let base = match runtime {
        Some(dir) => dir,
        None => crate::paths::state_dir()?,
    };
    Ok(base.join("proxysvpn-helper-setup"))
}

/// Gather the facts [`judge`] needs about `install_dir` on this machine.
/// Returns the verdict and the physical install folder.
#[cfg(target_os = "linux")]
fn inspect(install_dir: &Path, wants: &[(&'static str, &str)]) -> (Installed, PathBuf) {
    // Physical paths throughout (`physical_install_dir`).
    let physical = physical_install_dir(install_dir);
    let parents: Vec<(PathBuf, Option<NodeFacts>)> = physical
        .ancestors()
        .skip(1)
        .map(|dir| (dir.to_path_buf(), node_facts(dir)))
        .collect();
    let bin = physical.join(BIN_DIR);
    let own = [
        (physical.clone(), node_facts(&physical)),
        (bin.clone(), node_facts(&bin)),
    ];
    let files: Vec<FileCheck<'_>> = wants
        .iter()
        .map(|&(name, want)| {
            let path = bin.join(name);
            FileCheck {
                name,
                found: node_facts(&path).map(|facts| (facts, sha256_file(&path).ok())),
                want,
            }
        })
        .collect();
    (judge(&parents, &own, &files), physical)
}

/// Decide how the AppImage starts its helper: the installed copy when it is
/// current, else the setup script with freshly staged files.
///
/// `own_exe` is the GUI executable inside the mount; the helper and tun2socks
/// sit beside it in the AppImage's `usr/bin`. `steamos` asks the setup to
/// write the Gaming Mode record if nothing was decided yet ([`Gaming`]).
#[cfg(target_os = "linux")]
pub fn plan_appimage_spawn(own_exe: &Path, steamos: bool) -> Result<SpawnPlan, crate::privilege::HelperSetupFailed> {
    use crate::privilege::HelperSetupFailed as Fail;

    let own_dir = own_exe
        .parent()
        .ok_or_else(|| Fail("the app's own folder is unknown".to_string()))?;
    let helper = own_dir.join(HELPER_BIN);
    let engine = crate::triple::find_sidecar(ENGINE_BIN, &[own_dir.to_path_buf()])
        .map_err(|e| Fail(format!("{e:#}")))?;
    let helper_sum = sha256_file(&helper)
        .map_err(|e| Fail(format!("this AppImage carries no usable {HELPER_BIN}: {e}")))?;
    let engine_sum = sha256_file(&engine).map_err(|e| Fail(format!("{}: {e}", engine.display())))?;

    let (state, install_dir) = inspect(Path::new(INSTALL_DIR), &[(HELPER_BIN, &helper_sum), (ENGINE_BIN, &engine_sum)]);
    match state {
        Installed::Current => Ok(SpawnPlan {
            program: installed_helper(&install_dir),
            args: vec![OsString::from(super::HELPER_FLAG)],
            staged: None,
            setup: false,
        }),
        Installed::Missing | Installed::Stale(_) => {
            if let Installed::Stale(reason) = &state {
                crate::log::info("helper", &format!("the installed helper is out of date ({reason}); copying it again"));
            }
            let staged = staging_dir().map_err(|e| Fail(format!("no folder to stage the helper in: {e:#}")))?;
            stage(&staged, &helper, &engine).map_err(|e| {
                let _ = std::fs::remove_dir_all(&staged);
                Fail(format!("could not stage the helper in {}: {e}", staged.display()))
            })?;
            Ok(SpawnPlan {
                program: PathBuf::from(SHELL),
                args: setup_args(&install_dir, &staged, &helper_sum, &engine_sum, gaming_for(steamos, &install_dir)),
                staged: Some(staged),
                setup: true,
            })
        }
        Installed::Unsafe(reason) => Err(Fail(reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(uid: u32, mode: u32) -> Option<NodeFacts> {
        Some(NodeFacts { uid, mode: 0o040000 | mode, kind: NodeKind::Dir })
    }

    fn file(uid: u32, mode: u32) -> NodeFacts {
        NodeFacts { uid, mode: 0o100000 | mode, kind: NodeKind::File }
    }

    const H: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const E: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn parents() -> Vec<(PathBuf, Option<NodeFacts>)> {
        vec![(PathBuf::from("/home"), dir(0, 0o755)), (PathBuf::from("/"), dir(0, 0o755))]
    }

    fn own() -> Vec<(PathBuf, Option<NodeFacts>)> {
        vec![
            (PathBuf::from("/home/.proxysvpn"), dir(0, 0o755)),
            (PathBuf::from("/home/.proxysvpn/bin"), dir(0, 0o755)),
        ]
    }

    fn files(helper: Option<(NodeFacts, &str)>, engine: Option<(NodeFacts, &str)>) -> Vec<FileCheck<'static>> {
        let found = |f: Option<(NodeFacts, &str)>| f.map(|(facts, sum)| (facts, Some(sum.to_string())));
        vec![
            FileCheck { name: HELPER_BIN, found: found(helper), want: H },
            FileCheck { name: ENGINE_BIN, found: found(engine), want: E },
        ]
    }

    // Unix only, like the function's one caller: "/tmp/…" is not an absolute
    // path on Windows, where the Windows CI jobs run these tests too.
    #[cfg(unix)]
    #[test]
    fn only_an_executable_inside_appdir_is_the_appimage() {
        let image = Some(OsStr::new("/home/deck/Applications/ProxysVPN.AppImage"));
        let mount = Some(Path::new("/tmp/.mount_ProxysAbC123"));
        let inside = Path::new("/tmp/.mount_ProxysAbC123/usr/bin/proxysvpn-desktop");
        assert_eq!(launch_kind(image, mount, inside), Launch::AppImage);

        // The .deb's app, started from a terminal that is itself an AppImage,
        // inherits both variables but does not run from the mount.
        let deb = Path::new("/usr/bin/proxysvpn-desktop");
        assert_eq!(launch_kind(image, mount, deb), Launch::Package);
        // A sibling whose name merely starts the same is not inside.
        let sibling = Path::new("/tmp/.mount_ProxysAbC123x/usr/bin/proxysvpn-desktop");
        assert_eq!(launch_kind(image, mount, sibling), Launch::Package);

        assert_eq!(launch_kind(None, mount, inside), Launch::Package);
        assert_eq!(launch_kind(image, None, inside), Launch::Package);
        assert_eq!(launch_kind(Some(OsStr::new("")), mount, inside), Launch::Package);
        assert_eq!(launch_kind(image, Some(Path::new("relative")), Path::new("relative/x")), Launch::Package);
        assert_eq!(launch_kind(image, Some(Path::new("/")), deb), Launch::Package, "APPDIR=/ is everything");
    }

    /// The extracted form (--appimage-extract-and-run) is an AppImage too,
    /// and must not fall back to running a user-owned file as root.
    #[cfg(unix)]
    #[test]
    fn an_extracted_appimage_is_still_the_appimage() {
        let image = Some(OsStr::new("/home/deck/ProxysVPN.AppImage"));
        let extracted = Some(Path::new("/tmp/appimage_extracted_0123abcd"));
        let exe = Path::new("/tmp/appimage_extracted_0123abcd/usr/bin/proxysvpn-desktop");
        assert_eq!(launch_kind(image, extracted, exe), Launch::AppImage);
    }

    #[test]
    fn a_current_install_is_run_as_it_is() {
        let f = files(Some((file(0, 0o755), H)), Some((file(0, 0o755), E)));
        assert_eq!(judge(&parents(), &own(), &f), Installed::Current);
    }

    #[test]
    fn nothing_installed_yet_means_setup() {
        let none = vec![
            (PathBuf::from("/home/.proxysvpn"), None),
            (PathBuf::from("/home/.proxysvpn/bin"), None),
        ];
        assert_eq!(judge(&parents(), &none, &files(None, None)), Installed::Missing);
        // Half a copy: the helper landed, tun2socks did not.
        let half = files(Some((file(0, 0o755), H)), None);
        assert_eq!(judge(&parents(), &own(), &half), Installed::Missing);
    }

    /// A new AppImage carries new files: their sums differ and the copy is
    /// made again. So is one whose mode or owner the setup would not leave.
    #[test]
    fn another_release_or_a_drifted_mode_means_setup_again() {
        let other = files(Some((file(0, 0o755), E)), Some((file(0, 0o755), E)));
        assert!(matches!(judge(&parents(), &own(), &other), Installed::Stale(r) if r.contains(HELPER_BIN)));

        let loose = files(Some((file(0, 0o775), H)), Some((file(0, 0o755), E)));
        assert!(matches!(judge(&parents(), &own(), &loose), Installed::Stale(_)));
        let setuid = files(Some((file(0, 0o4755), H)), Some((file(0, 0o755), E)));
        assert!(matches!(judge(&parents(), &own(), &setuid), Installed::Stale(_)));
        let theirs = files(Some((file(1000, 0o755), H)), Some((file(0, 0o755), E)));
        assert!(matches!(judge(&parents(), &own(), &theirs), Installed::Stale(_)));
        let link = NodeFacts { uid: 0, mode: 0o120777, kind: NodeKind::Other };
        let linked = files(Some((link, H)), Some((file(0, 0o755), E)));
        assert!(matches!(judge(&parents(), &own(), &linked), Installed::Stale(_)));
        // An unreadable file has no sum and is never taken for current.
        let unread = vec![
            FileCheck { name: HELPER_BIN, found: Some((file(0, 0o755), None)), want: H },
            FileCheck { name: ENGINE_BIN, found: Some((file(0, 0o755), Some(E.to_string()))), want: E },
        ];
        assert!(matches!(judge(&parents(), &own(), &unread), Installed::Stale(_)));
    }

    /// The point of the module: nothing is run as root from a folder anyone
    /// but root can change, however current the files in it look.
    #[test]
    fn a_folder_others_can_change_is_never_used() {
        let ok = files(Some((file(0, 0o755), H)), Some((file(0, 0o755), E)));

        let mut open_home = parents();
        open_home[0].1 = dir(0, 0o777);
        assert!(matches!(judge(&open_home, &own(), &ok), Installed::Unsafe(r) if r.contains("/home")));

        let mut no_home = parents();
        no_home[0].1 = None;
        assert!(matches!(judge(&no_home, &own(), &ok), Installed::Unsafe(_)));

        for bad in [dir(1000, 0o755), dir(0, 0o775), dir(0, 0o757)] {
            let mut theirs = own();
            theirs[0].1 = bad;
            assert!(matches!(judge(&parents(), &theirs, &ok), Installed::Unsafe(_)), "{bad:?}");
            let mut bin = own();
            bin[1].1 = bad;
            assert!(matches!(judge(&parents(), &bin, &ok), Installed::Unsafe(_)), "bin {bad:?}");
        }

        let link = Some(NodeFacts { uid: 0, mode: 0o120777, kind: NodeKind::Other });
        let mut linked = own();
        linked[0].1 = link;
        assert!(matches!(judge(&parents(), &linked, &ok), Installed::Unsafe(_)));
    }

    #[test]
    fn sums_on_the_command_line_are_plain_hex() {
        assert!(is_sha256_hex(H));
        assert!(!is_sha256_hex(&H[1..]));
        assert!(!is_sha256_hex(&H.to_uppercase().replace('1', "A")));
        assert!(!is_sha256_hex("--helper"));
    }

    #[test]
    fn the_setup_is_one_fixed_script_with_its_arguments_after_it() {
        let args = setup_args(Path::new(INSTALL_DIR), Path::new("/run/user/1000/s"), H, E, Gaming::Granted);
        let text: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(text[0], "-c");
        assert_eq!(text[1], setup_script(Gaming::Granted));
        assert_eq!(
            &text[2..],
            [SETUP_NAME, INSTALL_DIR, "/run/user/1000/s", H, E, GAMING_FLAG, DIALOG_TAIL_GRANT]
        );
        let kept = setup_args(Path::new(INSTALL_DIR), Path::new("/s"), H, E, Gaming::Kept);
        let kept: Vec<String> = kept.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(&kept[7..], [GAMING_FLAG, DIALOG_TAIL]);
        let off = setup_args(Path::new(INSTALL_DIR), Path::new("/s"), H, E, Gaming::Off);
        let off: Vec<String> = off.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(&off[7..], ["-", DIALOG_TAIL]);
        // One body for all three; only the comment on the first line differs.
        for gaming in [Gaming::Off, Gaming::Kept, Gaming::Granted] {
            let script = setup_script(gaming);
            assert!(script.starts_with("# ProxysVPN sets up its VPN helper as root"), "{gaming:?}");
            assert_eq!(script.split_once('\n').map(|(_, body)| body), Some(SETUP_BODY));
        }
        assert!(!SETUP_BODY.contains("$6"), "the user is PKEXEC_UID, never an argument; $6 is for the window");
        assert!(SETUP_BODY.contains(STAGED_OPEN), "the tests run the open the script makes");
        assert!(!SETUP_BODY.contains("head -c") && !SETUP_BODY.contains("cat "), "no other open of a staged file");
        for code in [EXIT_FOLDER, EXIT_FILES, EXIT_COPY] {
            assert!(SETUP_BODY.contains(&format!("exit {code}")), "{code}");
            assert!(code != 126 && code != 127, "pkexec's own codes");
        }
        assert_eq!(installed_helper(Path::new(INSTALL_DIR)), Path::new("/home/.proxysvpn/bin/proxysvpn-helper"));
    }

    #[test]
    fn steamos_is_found_by_its_id_only() {
        let deck = "NAME=\"SteamOS\"\nPRETTY_NAME=\"SteamOS\"\nVERSION_CODENAME=holo\nID=steamos\nID_LIKE=arch\nVARIANT_ID=steamdeck\n";
        assert!(is_steamos(deck));
        assert!(is_steamos("ID=\"steamos\"\n"));
        assert!(is_steamos("ID='steamos'"));
        assert!(!is_steamos("ID=arch\nID_LIKE=steamos\n"), "ID_LIKE is not ID");
        assert!(!is_steamos("ID=debian\nVERSION_ID=\"13\"\n"));
        assert!(!is_steamos(""));
    }

    #[test]
    fn only_plain_names_and_paths_reach_the_rule() {
        for good in ["deck", "_svc", "a.b-c_1", "Deck"] {
            assert!(valid_user_name(good), "{good}");
        }
        for bad in ["", "-deck", "de\"ck", "deck\\", "de ck", "deck\n", &"x".repeat(33), "dëck"] {
            assert!(!valid_user_name(bad), "{bad:?}");
        }
        assert!(is_plain_path("/home/.proxysvpn/bin/proxysvpn-helper"));
        assert!(!is_plain_path("home/x"));
        assert!(!is_plain_path("/home/x\"); alert(1); //"));
        assert!(!is_plain_path("/home/x y"));

        assert_eq!(parse_gaming_record(b"deck\n").as_deref(), Some("deck"));
        assert_eq!(parse_gaming_record(b"deck").as_deref(), Some("deck"));
        assert_eq!(parse_gaming_record(b"deck\n\n"), None);
        assert_eq!(parse_gaming_record(b"deck\nroot\n"), None);
        assert_eq!(parse_gaming_record(b"\xff"), None);
        assert_eq!(parse_gaming_record(b""), None);
    }

    /// The rule allows one program, with one argv, as root, for one user at
    /// the device — and quotes nothing it was not sure of.
    #[test]
    fn the_rule_allows_exactly_the_installed_helper() {
        // A literal, not installed_helper(): a joined path has backslashes on
        // Windows, where this test runs as well.
        let helper = Path::new("/home/.proxysvpn/bin/proxysvpn-helper");
        let rule = polkit_rule("deck", helper).expect("rule");
        assert!(rule.contains(r#"action.id === "org.freedesktop.policykit.exec""#));
        assert!(rule.contains(r#"action.lookup("program") === "/home/.proxysvpn/bin/proxysvpn-helper""#));
        assert!(rule.contains(r#"action.lookup("command_line") === "/home/.proxysvpn/bin/proxysvpn-helper --helper""#));
        assert!(rule.contains(r#"action.lookup("user") === "root""#));
        assert!(rule.contains(r#"subject.user === "deck""#));
        assert!(rule.contains("subject.local && subject.active"));
        assert_eq!(rule.matches("polkit.Result.YES").count(), 1);
        assert!(!rule.contains("Result.AUTH"), "it never changes how anything else is asked");

        assert_eq!(polkit_rule("de\"ck", helper), None);
        assert_eq!(polkit_rule("deck", Path::new("/home/x\"/proxysvpn-helper")), None);
    }

    /// `command_line` as pkexec.c builds it from its own argv: its options
    /// (`--help`, `--version`, `--user`/`-u` and a value,
    /// `--disable-internal-agent`, `--keep-cwd`) are read until the first
    /// argument that is none of them, and that argument and everything after
    /// it are joined with spaces.
    fn pkexec_command_line(argv: &[OsString]) -> String {
        let mut rest = argv.iter().map(|a| a.to_string_lossy().into_owned()).peekable();
        while let Some(arg) = rest.peek() {
            match arg.as_str() {
                "--help" | "--version" | "--disable-internal-agent" | "--keep-cwd" => {
                    rest.next();
                }
                "--user" | "-u" => {
                    rest.next();
                    rest.next();
                }
                _ => break,
            }
        }
        rest.collect::<Vec<_>>().join(" ")
    }

    /// pkexec.c's `cmdline_short`, what its message quotes: the command line
    /// as it is, or, past 80 bytes, its first 38 bytes, " ... " and its last
    /// 37 — cut at bytes, so the window can only show it if both cuts land
    /// between characters.
    fn pkexec_cmdline_short(command_line: &str) -> String {
        let bytes = command_line.as_bytes();
        if bytes.len() <= 80 {
            return command_line.to_string();
        }
        let mut short = bytes[..38].to_vec();
        short.extend_from_slice(b" ... ");
        short.extend_from_slice(&bytes[bytes.len() - 37..]);
        String::from_utf8(short).expect("a message polkit can carry")
    }

    /// What the password window quotes says what the setup is about to do,
    /// in full words at both ends, and says it before the person types.
    #[test]
    fn the_password_window_says_what_the_setup_does() {
        let window = |gaming| {
            let args = setup_args(
                Path::new("/var/home/.proxysvpn"),
                Path::new("/run/user/1000/proxysvpn-helper-setup"),
                H,
                E,
                gaming,
            );
            pkexec_cmdline_short(&pkexec_command_line(&crate::privilege::pkexec_args(Path::new(SHELL), args)))
        };
        assert_eq!(
            window(Gaming::Granted),
            "/bin/sh -c # ProxysVPN sets up its VPN ... so Gaming Mode then needs no password"
        );
        let plain = "/bin/sh -c # ProxysVPN sets up its VPN ... helper as root and starts the tunnel.";
        assert_eq!(window(Gaming::Kept), plain);
        assert_eq!(window(Gaming::Off), plain);
        for text in [SETUP_HEAD, SETUP_HEAD_GRANT, DIALOG_TAIL, DIALOG_TAIL_GRANT] {
            assert!(text.is_ascii(), "pkexec cuts at bytes: {text}");
        }
        assert!(SETUP_HEAD_GRANT.contains("without a password") && SETUP_HEAD_GRANT.contains("Remove system files"));
        assert!(SETUP_HEAD.contains("Remove system files") && !SETUP_HEAD.contains("password"));
    }

    /// "Remove system files" says what it removes in the window too, and
    /// reads its end like the setup's: its own lines first, then pkexec's.
    #[test]
    fn the_removal_is_read_like_the_setup() {
        let argv = crate::privilege::pkexec_args(
            Path::new(SHELL),
            remove_args(Path::new("/var/home/.proxysvpn"), Path::new(POLKIT_RULE)),
        );
        assert_eq!(
            pkexec_cmdline_short(&pkexec_command_line(&argv)),
            "/bin/sh -c # ProxysVPN: remove helper, ... and the Gaming Mode no-password rule."
        );
        assert!(REMOVE_SCRIPT.is_ascii() && REMOVE_TAIL.len() == 37);
        assert!(!REMOVE_SCRIPT.contains("$3"), "$3 is for the window");

        assert_eq!(removal_outcome(Some(0), "", true), Ok(()));
        let no_agent = "Error executing command as another user: No authentication agent found.\n";
        assert_eq!(removal_outcome(Some(127), no_agent, true), Err(RemoveFailure::UseDesktopMode));
        assert_eq!(removal_outcome(Some(127), no_agent, false), Err(RemoveFailure::NoAgent));
        assert_eq!(removal_outcome(Some(126), "", true), Err(RemoveFailure::Refused));
        assert_eq!(removal_outcome(Some(127), "Not authorized", false), Err(RemoveFailure::Refused));
        let own = "proxysvpn-remove: /home/.proxysvpn is not a folder only uid 0 can change\n";
        assert!(matches!(removal_outcome(Some(73), own, true), Err(RemoveFailure::Failed(why)) if why.contains("only uid 0")));
        let shell = "proxysvpn-remove: line 3: rm: Read-only file system\n";
        assert!(matches!(removal_outcome(Some(127), shell, false), Err(RemoveFailure::Failed(_))));
        assert!(matches!(removal_outcome(None, "", false), Err(RemoveFailure::Failed(_))));
    }

    /// The option that stops pkexec's terminal prompt is pkexec's own and
    /// never reaches `command_line`, so the Gaming Mode rule still matches
    /// the helper the GUI starts — and only it.
    #[test]
    fn the_rule_matches_what_pkexec_reports_for_the_installed_helper() {
        let helper = Path::new("/home/.proxysvpn/bin/proxysvpn-helper");
        let argv = crate::privilege::pkexec_args(helper, [super::super::HELPER_FLAG]);
        assert_eq!(argv[0], OsStr::new(crate::privilege::PKEXEC_NO_TEXT_AGENT));
        let command_line = pkexec_command_line(&argv);
        assert_eq!(command_line, "/home/.proxysvpn/bin/proxysvpn-helper --helper");
        let rule = polkit_rule("deck", helper).expect("rule");
        assert!(rule.contains(&format!(r#"action.lookup("command_line") === "{command_line}""#)), "{rule}");

        // The setup and anything else started through pkexec do not match.
        let setup = crate::privilege::pkexec_args(
            Path::new(SHELL),
            setup_args(Path::new(INSTALL_DIR), Path::new("/s"), "h", "e", Gaming::Granted),
        );
        assert!(!rule.contains(&format!(r#"=== "{}""#, pkexec_command_line(&setup))));
        let dev = crate::privilege::pkexec_args(helper, ["--helper", "--dev"]);
        assert!(!rule.contains(&format!(r#"=== "{}""#, pkexec_command_line(&dev))));
    }

    #[cfg(unix)]
    mod on_disk {
        use super::super::*;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        use std::process::{Command, Stdio};

        fn temp(tag: &str) -> PathBuf {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let base = std::env::temp_dir().join(format!("pvpn-install-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&base).expect("temp dir");
            std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).expect("chmod");
            std::fs::canonicalize(&base).expect("canonical")
        }

        /// A stand-in for the AppImage's usr/bin: a helper that says it
        /// started and with what, and a tun2socks of some other bytes.
        fn mount(base: &Path) -> (PathBuf, PathBuf) {
            let usr_bin = base.join("mount/usr/bin");
            std::fs::create_dir_all(&usr_bin).expect("mount");
            let helper = usr_bin.join(HELPER_BIN);
            std::fs::write(&helper, "#!/bin/sh\necho \"helper started $*\"\nread -r line\necho \"helper read $line\"\n")
                .expect("helper");
            let engine = usr_bin.join(ENGINE_BIN);
            std::fs::write(&engine, b"tun2socks bytes").expect("engine");
            (helper, engine)
        }

        /// A stand-in for GNU dd where the system's has no
        /// `iflag=nofollow,nonblock` (macOS): the operands [`STAGED_OPEN`]
        /// uses, through the same open(2) flags, and nothing else. On Linux
        /// the tests run coreutils' own dd.
        const DD_STAND_IN: &str = r#"#!/usr/bin/perl
use strict; use warnings; use Fcntl;
my %op = map { split /=/, $_, 2 } @ARGV;
my $flags = O_RDONLY;
for (split /,/, $op{iflag} // "") {
    if ($_ eq "nofollow") { $flags |= O_NOFOLLOW } elsif ($_ eq "nonblock") { $flags |= O_NONBLOCK } else { die "dd: iflag $_\n" }
}
sysopen(my $in, $op{if}, $flags) or die "dd: $op{if}: $!\n";
open(my $out, ">", $op{of}) or die "dd: $op{of}: $!\n";
my $left = ($op{bs} // 512) * ($op{count} // 1);
while ($left > 0) {
    my $n = sysread($in, my $buf, $left < 65536 ? $left : 65536);
    die "dd: $op{if}: $!\n" unless defined $n;
    last if $n == 0;
    print {$out} $buf or die "dd: $op{of}: $!\n";
    $left -= $n;
}
close($out) or die "dd: $op{of}: $!\n";
"#;

        fn shim(base: &Path, name: &str, text: &str) -> PathBuf {
            let dir = base.join("shim");
            std::fs::create_dir_all(&dir).expect("shim");
            std::fs::write(dir.join(name), text).expect("shim");
            std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755)).expect("chmod");
            dir
        }

        /// `sha256sum` is coreutils on Linux and /sbin/sha256sum on recent
        /// macOS; an older Mac only has shasum, so the test brings a shim. dd
        /// gets one where it lacks GNU's flags ([`DD_STAND_IN`]).
        fn path_with_tools(base: &Path) -> std::ffi::OsString {
            let system = std::env::var_os("PATH").unwrap_or_default();
            let found = std::env::split_paths(&system)
                .chain([PathBuf::from("/sbin")])
                .any(|dir| dir.join("sha256sum").is_file());
            let mut dirs: Vec<PathBuf> = std::env::split_paths(&system).collect();
            if found {
                dirs.push(PathBuf::from("/sbin"));
            } else {
                dirs.insert(0, shim(base, "sha256sum", "#!/bin/sh\nexec shasum -a 256 \"$@\"\n"));
            }
            let gnu_dd = Command::new("dd")
                .args(["if=/dev/null", "of=/dev/null", "iflag=nofollow,nonblock", "status=none"])
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            if !gnu_dd {
                dirs.insert(0, shim(base, "dd", DD_STAND_IN));
            }
            std::env::join_paths(dirs).expect("PATH")
        }

        /// Wait for `child`, but fail the test instead of hanging when it
        /// blocks: what a FIFO in the wrong place would do to root.
        fn finish(mut child: std::process::Child, what: &str) -> Run {
            use std::io::Read;

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let status = loop {
                if let Some(status) = child.try_wait().expect("wait") {
                    break status;
                }
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{what} blocked");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            };
            let mut stdout = String::new();
            let mut stderr = String::new();
            if let Some(mut out) = child.stdout.take() {
                out.read_to_string(&mut stdout).expect("stdout");
            }
            if let Some(mut err) = child.stderr.take() {
                err.read_to_string(&mut stderr).expect("stderr");
            }
            Run { code: status.code(), stdout, stderr }
        }

        struct Run {
            code: Option<i32>,
            stdout: String,
            stderr: String,
        }

        /// Run the setup exactly as pkexec would, minus root: same shell,
        /// same arguments, a request already waiting on stdin.
        fn run_setup(base: &Path, dest: &Path, staged: &Path, helper_sum: &str, engine_sum: &str) -> Run {
            run_setup_as(base, dest, staged, helper_sum, engine_sum, false)
        }

        fn run_setup_as(
            base: &Path,
            dest: &Path,
            staged: &Path,
            helper_sum: &str,
            engine_sum: &str,
            gaming: bool,
        ) -> Run {
            use std::io::Write;

            let gaming = if gaming { Gaming::Kept } else { Gaming::Off };
            let mut child = Command::new(SHELL)
                .args(setup_args(dest, staged, helper_sum, engine_sum, gaming))
                .env("PATH", path_with_tools(base))
                .env("PKEXEC_UID", me().to_string())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("sh");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(b"{\"request\":{\"id\":1,\"req\":\"hello\"}}\n")
                .expect("request");
            finish(child, "the setup")
        }

        fn me() -> u32 {
            // SAFETY: geteuid(2) has no preconditions and cannot fail.
            unsafe { libc::geteuid() }
        }

        fn my_name() -> String {
            let out = Command::new("id").arg("-nu").output().expect("id");
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }

        fn staged_from(base: &Path) -> (PathBuf, String, String) {
            let (helper, engine) = mount(base);
            let staged = base.join("staged");
            stage(&staged, &helper, &engine).expect("stage");
            (staged, sha256_file(&helper).expect("sum"), sha256_file(&engine).expect("sum"))
        }

        #[test]
        fn the_setup_installs_both_files_and_hands_the_pipe_to_the_helper() {
            let base = temp("ok");
            let (staged, hs, es) = staged_from(&base);
            let dest = base.join(".proxysvpn");

            let run = run_setup(&base, &dest, &staged, &hs, &es);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            assert!(run.stdout.contains("helper started --helper"), "{}", run.stdout);
            // The request the GUI wrote before the password reached the
            // helper whole: nothing in the script read stdin.
            assert!(run.stdout.contains(r#"helper read {"request":{"id":1,"req":"hello"}}"#), "{}", run.stdout);

            for name in [HELPER_BIN, ENGINE_BIN] {
                let path = dest.join(BIN_DIR).join(name);
                let meta = std::fs::symlink_metadata(&path).expect("installed");
                assert!(meta.file_type().is_file(), "{name}");
                assert_eq!(meta.mode() & 0o7777, 0o755, "{name}");
                assert_eq!(meta.uid(), std::fs::metadata(&base).expect("me").uid());
                assert_eq!(sha256_file(&path).expect("sum"), if name == HELPER_BIN { hs.clone() } else { es.clone() });
            }
            for folder in [dest.clone(), dest.join(BIN_DIR)] {
                let mode = std::fs::symlink_metadata(&folder).expect("folder").mode() & 0o7777;
                assert_eq!(mode, 0o755, "{}", folder.display());
            }
            let leftovers: Vec<_> = std::fs::read_dir(dest.join(BIN_DIR))
                .expect("list")
                .flatten()
                .map(|e| e.file_name())
                .filter(|n| n.to_string_lossy().starts_with('.'))
                .collect();
            assert!(leftovers.is_empty(), "{leftovers:?}");

            // Again, over the first copy: replaced in place, same result.
            let again = run_setup(&base, &dest, &staged, &hs, &es);
            assert_eq!(again.code, Some(0), "{}", again.stderr);
            let _ = std::fs::remove_dir_all(&base);
        }

        /// A staged file swapped after the GUI hashed the mount — another
        /// program, or a link to a secret — is refused, and nothing of it is
        /// left where root would run it.
        #[test]
        fn a_swapped_staged_file_is_refused_and_leaves_nothing() {
            let base = temp("swap");
            let (staged, hs, es) = staged_from(&base);
            let dest = base.join(".proxysvpn");

            std::fs::write(staged.join(ENGINE_BIN), b"something else").expect("swap");
            let run = run_setup(&base, &dest, &staged, &hs, &es);
            assert_eq!(run.code, Some(EXIT_FILES), "{}", run.stderr);
            assert!(run.stderr.contains("proxysvpn-setup: tun2socks does not match its checksum"), "{}", run.stderr);
            assert!(!run.stdout.contains("helper started"), "the helper must not start");
            assert!(!dest.join(BIN_DIR).join(ENGINE_BIN).exists());
            assert!(!dest.join(BIN_DIR).join(".tun2socks.new").exists());

            let secret = base.join("secret");
            std::fs::write(&secret, b"root:$6$hash").expect("secret");
            std::fs::remove_file(staged.join(HELPER_BIN)).expect("remove");
            std::os::unix::fs::symlink(&secret, staged.join(HELPER_BIN)).expect("link");
            let fresh = base.join(".fresh");
            let run = run_setup(&base, &fresh, &staged, &hs, &es);
            assert_eq!(run.code, Some(EXIT_FILES), "{}", run.stderr);
            assert!(!fresh.join(BIN_DIR).join(HELPER_BIN).exists());
            assert!(!fresh.join(BIN_DIR).join(".proxysvpn-helper.new").exists());
            let _ = std::fs::remove_dir_all(&base);
        }

        fn mkfifo(path: &Path) {
            let status = Command::new("mkfifo").arg(path).status().expect("mkfifo");
            assert!(status.success(), "mkfifo {}", path.display());
        }

        /// A FIFO in place of a staged file — a reader root would wait on
        /// for ever — and links to a device or to a FIFO are refused at once,
        /// and nothing of them is left where root would run it.
        #[test]
        fn a_fifo_or_a_link_in_place_of_a_staged_file_is_refused_without_waiting() {
            let base = temp("fifo");
            let (staged, hs, es) = staged_from(&base);
            let dest = base.join(".proxysvpn");
            let fifo = base.join("fifo");
            mkfifo(&fifo);

            std::fs::remove_file(staged.join(HELPER_BIN)).expect("remove");
            mkfifo(&staged.join(HELPER_BIN));
            let run = run_setup(&base, &dest, &staged, &hs, &es);
            assert_eq!(run.code, Some(EXIT_FILES), "{}", run.stderr);
            assert!(!run.stdout.contains("helper started"), "the helper must not start");
            assert!(!dest.join(BIN_DIR).join(HELPER_BIN).exists());

            for target in [Path::new("/dev/zero"), fifo.as_path()] {
                std::fs::remove_file(staged.join(HELPER_BIN)).expect("remove");
                std::os::unix::fs::symlink(target, staged.join(HELPER_BIN)).expect("link");
                let run = run_setup(&base, &dest, &staged, &hs, &es);
                assert_eq!(run.code, Some(EXIT_FILES), "{}: {}", target.display(), run.stderr);
                assert!(!dest.join(BIN_DIR).join(HELPER_BIN).exists());
                assert!(!dest.join(BIN_DIR).join(".proxysvpn-helper.new").exists());
            }
            let _ = std::fs::remove_dir_all(&base);
        }

        /// The staged files are the person's, so the folder they are in must
        /// be theirs alone and a real folder: not one others may write into,
        /// not a link somewhere else.
        #[test]
        fn a_staging_folder_others_can_change_or_a_linked_one_stops_the_setup() {
            let base = temp("staged-folder");
            let (staged, hs, es) = staged_from(&base);
            let dest = base.join(".proxysvpn");

            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o770)).expect("chmod");
            let run = run_setup(&base, &dest, &staged, &hs, &es);
            assert_eq!(run.code, Some(EXIT_FILES), "{}", run.stderr);
            assert!(run.stderr.contains("is not a folder only uid"), "{}", run.stderr);
            assert!(!dest.exists(), "nothing is created before the check");
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o700)).expect("chmod");

            let linked = base.join("linked");
            std::os::unix::fs::symlink(&staged, &linked).expect("link");
            let run = run_setup(&base, &dest, &linked, &hs, &es);
            assert_eq!(run.code, Some(EXIT_FILES), "{}", run.stderr);
            assert!(!dest.exists());

            let run = run_setup(&base, &dest, &staged, &hs, &es);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            let _ = std::fs::remove_dir_all(&base);
        }

        /// The open itself, as the script makes it ([`STAGED_OPEN`]), for a
        /// file swapped after the script's checks: a link is not followed and
        /// a FIFO does not hold it, with or without a writer. Coreutils' dd on
        /// Linux; on macOS this checks the stand-in the other tests use.
        #[test]
        fn the_one_open_of_a_staged_file_never_follows_a_link_or_waits_on_a_fifo() {
            let base = temp("open");
            let copy = |src: &Path| {
                let tmp = base.join("copy");
                let _ = std::fs::remove_file(&tmp);
                let child = Command::new(SHELL)
                    .args(["-c", &format!("src=$1 tmp=$2; {STAGED_OPEN}"), "sh"])
                    .arg(src)
                    .arg(&tmp)
                    .env("PATH", path_with_tools(&base))
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("sh");
                let run = finish(child, "the staged open");
                (run, std::fs::read(&tmp).ok())
            };

            let plain = base.join("plain");
            std::fs::write(&plain, b"helper bytes").expect("plain");
            let (run, copied) = copy(&plain);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            assert_eq!(copied.as_deref(), Some(&b"helper bytes"[..]));

            let link = base.join("link");
            std::os::unix::fs::symlink(&plain, &link).expect("link");
            let (run, _) = copy(&link);
            assert_ne!(run.code, Some(0), "a link is not followed");

            let fifo = base.join("fifo");
            mkfifo(&fifo);
            let (run, copied) = copy(&fifo);
            assert!(copied.unwrap_or_default().is_empty(), "no writer: nothing, at once ({})", run.stderr);

            // A writer that holds the FIFO open and never writes.
            #[cfg(target_os = "linux")]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let _writer = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&fifo)
                    .expect("writer");
                let (run, copied) = copy(&fifo);
                assert!(copied.unwrap_or_default().is_empty(), "a silent writer: nothing, at once ({})", run.stderr);
            }
            let _ = std::fs::remove_dir_all(&base);
        }

        #[test]
        fn a_folder_others_can_change_stops_the_setup() {
            let base = temp("folder");
            let (staged, hs, es) = staged_from(&base);

            // The parent is writable by the group.
            let open = base.join("open");
            std::fs::create_dir(&open).expect("dir");
            std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o775)).expect("chmod");
            let run = run_setup(&base, &open.join(".proxysvpn"), &staged, &hs, &es);
            assert_eq!(run.code, Some(EXIT_FOLDER), "{}", run.stderr);
            assert!(!open.join(".proxysvpn").exists(), "nothing is created under it");

            // The install folder is a link to somewhere else.
            let elsewhere = base.join("elsewhere");
            std::fs::create_dir(&elsewhere).expect("dir");
            let linked = base.join(".linked");
            std::os::unix::fs::symlink(&elsewhere, &linked).expect("link");
            let run = run_setup(&base, &linked, &staged, &hs, &es);
            assert_eq!(run.code, Some(EXIT_FOLDER), "{}", run.stderr);
            assert!(std::fs::read_dir(&elsewhere).expect("list").next().is_none());

            // Its bin is world-writable.
            let dest = base.join(".proxysvpn");
            std::fs::create_dir_all(dest.join(BIN_DIR)).expect("dir");
            std::fs::set_permissions(dest.join(BIN_DIR), std::fs::Permissions::from_mode(0o777)).expect("chmod");
            let run = run_setup(&base, &dest, &staged, &hs, &es);
            assert_eq!(run.code, Some(EXIT_FOLDER), "{}", run.stderr);
            let _ = std::fs::remove_dir_all(&base);
        }

        #[test]
        fn nothing_staged_stops_the_setup() {
            let base = temp("empty");
            let (_, hs, es) = staged_from(&base);
            let empty = base.join("empty");
            std::fs::create_dir(&empty).expect("dir");
            let run = run_setup(&base, &base.join(".proxysvpn"), &empty, &hs, &es);
            assert_eq!(run.code, Some(EXIT_FILES), "{}", run.stderr);
            assert!(run.stderr.contains("proxysvpn-helper was not staged"), "{}", run.stderr);
            let _ = std::fs::remove_dir_all(&base);
        }

        /// Whatever an earlier attempt left in the staging folder goes, and
        /// the staged copies are the mount's bytes, owner-only.
        #[test]
        fn staging_starts_from_an_empty_private_folder() {
            let base = temp("stage");
            let (helper, engine) = mount(&base);
            let staged = base.join("staged");
            std::fs::create_dir(&staged).expect("dir");
            std::fs::write(staged.join("leftover"), b"x").expect("leftover");

            stage(&staged, &helper, &engine).expect("stage");
            let mut names: Vec<String> = std::fs::read_dir(&staged)
                .expect("list")
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            assert_eq!(names, [HELPER_BIN, ENGINE_BIN]);
            assert_eq!(std::fs::symlink_metadata(&staged).expect("meta").mode() & 0o777, 0o700);
            assert_eq!(std::fs::read(staged.join(ENGINE_BIN)).expect("read"), b"tun2socks bytes");

            // A link where a file of the mount should be is not followed.
            std::fs::remove_file(&engine).expect("remove");
            std::os::unix::fs::symlink(&helper, &engine).expect("link");
            assert!(stage(&staged, &helper, &engine).is_err());
            let _ = std::fs::remove_dir_all(&base);
        }

        /// SteamOS: the first setup records who typed the password; a later
        /// one keeps whatever was decided — the record as it is, or the
        /// explicit "no" — so switching it off stays a decision; elsewhere
        /// the record is removed.
        #[test]
        fn the_first_setup_on_steamos_records_its_user_and_later_ones_keep_the_choice() {
            let base = temp("record");
            let (staged, hs, es) = staged_from(&base);
            let dest = base.join(".proxysvpn");
            let record = dest.join(GAMING_RECORD);

            let run = run_setup_as(&base, &dest, &staged, &hs, &es, true);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            assert_eq!(std::fs::read_to_string(&record).expect("record"), format!("{}\n", my_name()));
            assert_eq!(std::fs::metadata(&record).expect("meta").mode() & 0o7777, 0o644);
            assert!(parse_gaming_record(&std::fs::read(&record).expect("read")).is_some());

            // Another name in the record (written by hand): kept as it is.
            std::fs::write(&record, "deck\n").expect("record");
            let update = run_setup_as(&base, &dest, &staged, &hs, &es, true);
            assert_eq!(update.code, Some(0), "{}", update.stderr);
            assert_eq!(std::fs::read_to_string(&record).expect("record"), "deck\n");

            // Switched off as docs/STEAMDECK.md says: the "no", and no record.
            std::fs::write(dest.join(GAMING_OFF), b"").expect("off");
            std::fs::remove_file(&record).expect("switched off");
            let update = run_setup_as(&base, &dest, &staged, &hs, &es, true);
            assert_eq!(update.code, Some(0), "{}", update.stderr);
            assert!(!record.exists(), "an update does not switch it back on");
            assert!(dest.join(GAMING_OFF).exists());

            std::fs::write(&record, "deck\n").expect("record");
            let elsewhere = run_setup_as(&base, &dest, &staged, &hs, &es, false);
            assert_eq!(elsewhere.code, Some(0), "{}", elsewhere.stderr);
            assert!(!record.exists(), "not SteamOS: no record");
            let _ = std::fs::remove_dir_all(&base);
        }

        /// A first setup cut short after it made the folder — the password
        /// window's process killed, the Deck switched off — has decided
        /// nothing yet, so the next one still records the user. The old
        /// script took "the folder exists" for "decided" and lost Gaming
        /// Mode for good here.
        #[test]
        fn a_first_setup_cut_short_records_the_user_when_it_runs_again() {
            let base = temp("interrupted");
            let (staged, hs, es) = staged_from(&base);
            let dest = base.join(".proxysvpn");
            let record = dest.join(GAMING_RECORD);

            // What the script leaves when it stops right after its mkdir, and
            // a temp record from a stop in the middle of writing it.
            std::fs::create_dir_all(dest.join(BIN_DIR)).expect("bin");
            for dir in [dest.clone(), dest.join(BIN_DIR)] {
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
            }
            std::fs::write(dest.join(".gaming-mode-user.new"), b"half").expect("temp");

            let again = run_setup_as(&base, &dest, &staged, &hs, &es, true);
            assert_eq!(again.code, Some(0), "{}", again.stderr);
            assert_eq!(std::fs::read_to_string(&record).expect("record"), format!("{}\n", my_name()));
            assert!(!dest.join(".gaming-mode-user.new").exists());
            let _ = std::fs::remove_dir_all(&base);
        }

        /// What the window says, and whether the app's own screen comes
        /// first, follows what is decided on the device: nothing yet is a
        /// grant; the record or the "no", whatever they hold, is kept.
        #[test]
        fn the_grant_is_announced_only_while_nothing_is_decided() {
            let base = temp("decided");
            let dest = base.join(".proxysvpn");
            assert_eq!(gaming_for(true, &dest), Gaming::Granted, "no folder yet");
            std::fs::create_dir_all(dest.join(BIN_DIR)).expect("bin");
            assert_eq!(gaming_for(true, &dest), Gaming::Granted, "a folder alone decides nothing");
            assert_eq!(gaming_for(false, &dest), Gaming::Off);

            std::fs::write(dest.join(GAMING_RECORD), "deck\n").expect("record");
            assert_eq!(gaming_for(true, &dest), Gaming::Kept);
            std::fs::remove_file(dest.join(GAMING_RECORD)).expect("remove");
            std::os::unix::fs::symlink(base.join("nowhere"), dest.join(GAMING_OFF)).expect("link");
            assert!(gaming_decided(&dest), "a dangling \"no\" is still a no");
            assert_eq!(gaming_for(true, &dest), Gaming::Kept);
            assert_eq!(gaming_for(false, &dest), Gaming::Off);
            let _ = std::fs::remove_dir_all(&base);
        }

        /// Run the removal as pkexec would, minus root.
        fn run_remove(base: &Path, dest: &Path, rule: &Path) -> Run {
            let child = Command::new(SHELL)
                .args(remove_args(dest, rule))
                .env("PATH", path_with_tools(base))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("sh");
            finish(child, "the removal")
        }

        /// A setup's whole tree and the rule go; links in the way are
        /// removed, not followed; removing twice is fine. The folder is
        /// named as the GUI names it ([`physical_install_dir`]).
        #[test]
        fn the_removal_takes_the_folder_and_the_rule_and_nothing_else() {
            let base = temp("remove");
            let (staged, hs, es) = staged_from(&base);
            let dest = physical_install_dir(&base.join(".proxysvpn"));
            assert_eq!(dest, base.join(".proxysvpn"));
            let run = run_setup_as(&base, &dest, &staged, &hs, &es, true);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            std::fs::write(dest.join(GAMING_OFF), b"").expect("off");
            let rules = base.join("rules.d");
            std::fs::create_dir(&rules).expect("rules.d");
            let rule = rules.join("49-proxysvpn.rules");
            std::fs::write(&rule, b"polkit.addRule(function () {});").expect("rule");
            let neighbour = rules.join("50-default.rules");
            std::fs::write(&neighbour, b"keep").expect("neighbour");
            // A link inside the folder to something outside it.
            let outside = base.join("outside");
            std::fs::write(&outside, b"keep").expect("outside");
            std::os::unix::fs::symlink(&outside, dest.join(BIN_DIR).join("link")).expect("link");

            let run = run_remove(&base, &dest, &rule);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            assert!(std::fs::symlink_metadata(&dest).is_err(), "the folder is gone");
            assert!(std::fs::symlink_metadata(&rule).is_err(), "the rule is gone");
            assert_eq!(std::fs::read(&neighbour).expect("neighbour"), b"keep");
            assert_eq!(std::fs::read(&outside).expect("outside"), b"keep", "a link is not followed");
            assert!(staged.exists(), "nothing but the two paths");

            let again = run_remove(&base, &dest, &rule);
            assert_eq!(again.code, Some(0), "{}", again.stderr);

            // Links where the folder and the rule should be: the links go,
            // what they point at stays. The GUI names the link itself, not
            // its target — resolving the whole path handed the removal the
            // target, and `rm -rf` took a folder that was never ours.
            let elsewhere = base.join("elsewhere");
            std::fs::create_dir(&elsewhere).expect("dir");
            std::fs::write(elsewhere.join("f"), b"keep").expect("f");
            std::os::unix::fs::symlink(&elsewhere, base.join(".proxysvpn")).expect("link");
            std::os::unix::fs::symlink(&outside, &rule).expect("link");
            let dest = physical_install_dir(&base.join(".proxysvpn"));
            assert_eq!(dest, base.join(".proxysvpn"), "the link, not {}", elsewhere.display());
            let run = run_remove(&base, &dest, &rule);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            assert!(std::fs::symlink_metadata(&dest).is_err() && std::fs::symlink_metadata(&rule).is_err());
            assert_eq!(std::fs::read(elsewhere.join("f")).expect("f"), b"keep");
            assert_eq!(std::fs::read(&outside).expect("outside"), b"keep");
            let _ = std::fs::remove_dir_all(&base);
        }

        /// /home a link, as on Fedora's atomic desktops: the GUI hands the
        /// scripts the physical parent, which they accept, and the install
        /// folder's own name — a link there included — as it is.
        #[test]
        fn a_linked_parent_is_resolved_and_a_linked_install_folder_is_not() {
            let base = temp("linked-home");
            let real_home = base.join("var-home");
            std::fs::create_dir(&real_home).expect("dir");
            std::fs::set_permissions(&real_home, std::fs::Permissions::from_mode(0o755)).expect("chmod");
            let home = base.join("home");
            std::os::unix::fs::symlink(&real_home, &home).expect("link");

            // What the GUI passes for /home/.proxysvpn.
            let dest = physical_install_dir(&home.join(".proxysvpn"));
            assert_eq!(dest, real_home.join(".proxysvpn"));
            let (staged, hs, es) = staged_from(&base);
            let run = run_setup(&base, &dest, &staged, &hs, &es);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            let rule = base.join("49-proxysvpn.rules");
            let run = run_remove(&base, &dest, &rule);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            assert!(std::fs::symlink_metadata(&dest).is_err(), "the physical folder is gone");

            // A link someone put where the install folder goes: judged as a
            // link, so never run from, and removed without following it.
            let target = base.join("target");
            std::fs::create_dir(&target).expect("dir");
            std::fs::write(target.join("f"), b"keep").expect("f");
            std::os::unix::fs::symlink(&target, real_home.join(".proxysvpn")).expect("link");
            let dest = physical_install_dir(&home.join(".proxysvpn"));
            assert_eq!(dest, real_home.join(".proxysvpn"), "not {}", target.display());
            let facts = node_facts(&dest).expect("there");
            assert_eq!(facts.kind, NodeKind::Other, "lstat sees the link");
            let own = [(dest.clone(), Some(facts)), (dest.join(BIN_DIR), node_facts(&dest.join(BIN_DIR)))];
            assert!(matches!(judge(&[], &own, &[]), Installed::Unsafe(_)));
            let run = run_remove(&base, &dest, &rule);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            assert!(std::fs::symlink_metadata(&dest).is_err(), "the link is gone");
            assert_eq!(std::fs::read(target.join("f")).expect("f"), b"keep", "its target is not");
            let _ = std::fs::remove_dir_all(&base);
        }

        /// Only ProxysVPN's own names, as plain absolute paths, and only a
        /// folder that is root's alone, as the setup made it.
        #[test]
        fn the_removal_refuses_anything_that_is_not_ours() {
            let base = temp("remove-not-ours");
            let rule = base.join("49-proxysvpn.rules");
            let other = base.join("data");
            std::fs::create_dir(&other).expect("dir");
            std::fs::write(other.join("f"), b"keep").expect("f");

            for (dest, rule_path) in [
                (other.clone(), rule.clone()),
                (base.join(".proxysvpn"), base.join("50-default.rules")),
                (PathBuf::from(".proxysvpn"), rule.clone()),
                (base.join("data/../.proxysvpn"), rule.clone()),
                (base.join(".proxysvpn"), base.join("./49-proxysvpn.rules")),
            ] {
                let run = run_remove(&base, &dest, &rule_path);
                assert_eq!(run.code, Some(64), "{} {}: {}", dest.display(), rule_path.display(), run.stderr);
                assert!(run.stderr.contains(REMOVE_NAME), "{}", run.stderr);
            }
            assert_eq!(std::fs::read(other.join("f")).expect("f"), b"keep");

            let open = base.join(".proxysvpn");
            std::fs::create_dir(&open).expect("dir");
            std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).expect("chmod");
            std::fs::write(&rule, b"rule").expect("rule");
            let run = run_remove(&base, &open, &rule);
            assert_eq!(run.code, Some(73), "{}", run.stderr);
            assert!(open.exists(), "a folder others can change is left for a person");
            assert!(!rule.exists(), "the rule goes first");
            let _ = std::fs::remove_dir_all(&base);
        }

        /// The installed tree of a test, owned by the runner instead of root.
        fn installed(base: &Path, user: Option<&str>) -> (PathBuf, PathBuf, PathBuf) {
            let install = base.join(".proxysvpn");
            std::fs::create_dir_all(install.join(BIN_DIR)).expect("bin");
            let helper = installed_helper(&install);
            std::fs::write(&helper, b"helper").expect("helper");
            if let Some(user) = user {
                std::fs::write(install.join(GAMING_RECORD), format!("{user}\n")).expect("record");
            }
            let rules = base.join("rules.d");
            std::fs::create_dir_all(&rules).expect("rules.d");
            (install, helper, rules.join("49-proxysvpn.rules"))
        }

        #[test]
        fn the_helper_writes_the_rule_its_record_asks_for_once() {
            let base = temp("rule");
            let (install, helper, rule) = installed(&base, Some("deck"));

            assert_eq!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Written("deck".to_string()));
            let text = std::fs::read_to_string(&rule).expect("rule");
            assert_eq!(Some(text), polkit_rule("deck", &helper));
            assert_eq!(std::fs::metadata(&rule).expect("meta").mode() & 0o7777, 0o644);
            assert_eq!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Unchanged);

            // An /etc reset by an OS update: the next start puts it back.
            std::fs::remove_file(&rule).expect("reset");
            assert_eq!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Written("deck".to_string()));
            // Edited by hand: put back as the record says.
            std::fs::write(&rule, "polkit.addRule(function () { return polkit.Result.YES; });").expect("edit");
            assert_eq!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Written("deck".to_string()));

            // The record removed: the rule goes too, once.
            std::fs::remove_file(install.join(GAMING_RECORD)).expect("off");
            assert_eq!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Removed);
            assert!(!rule.exists());
            assert_eq!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Unchanged);

            // The explicit "no" outweighs a record that is still there.
            std::fs::write(install.join(GAMING_RECORD), "deck\n").expect("record");
            assert_eq!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Written("deck".to_string()));
            std::fs::write(install.join(GAMING_OFF), b"").expect("off");
            assert_eq!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Removed);
            assert!(!rule.exists());
            let leftovers = std::fs::read_dir(rule.parent().expect("dir")).expect("list").count();
            assert_eq!(leftovers, 0, "no temp file is left in rules.d");
            let _ = std::fs::remove_dir_all(&base);
        }

        /// The .deb's helper, or any program that is not the installed copy,
        /// never touches polkit.
        #[test]
        fn only_the_installed_copy_touches_polkit() {
            let base = temp("notours");
            let (install, _, rule) = installed(&base, Some("deck"));
            let deb = Path::new("/usr/bin/proxysvpn-desktop");
            assert_eq!(sync_rule(deb, &install, &base, &rule, me()), RuleSync::NotOurs);
            let beside = install.join(BIN_DIR).join(ENGINE_BIN);
            assert_eq!(sync_rule(&beside, &install, &base, &rule, me()), RuleSync::NotOurs);
            assert!(!rule.exists());
            let _ = std::fs::remove_dir_all(&base);
        }

        /// A folder of the chain others can change: no rule, and an existing
        /// one is taken away. A record that is not the owner's alone, or not
        /// a name, changes nothing.
        #[test]
        fn a_rule_is_never_written_for_a_path_others_can_swap() {
            let base = temp("swap-rule");
            let (install, helper, rule) = installed(&base, Some("deck"));
            assert_eq!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Written("deck".to_string()));

            std::fs::set_permissions(install.join(BIN_DIR), std::fs::Permissions::from_mode(0o777)).expect("chmod");
            assert!(matches!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Failed(r) if r.contains("writable")));
            assert!(!rule.exists(), "the grant goes while the path is not safe");
            std::fs::set_permissions(install.join(BIN_DIR), std::fs::Permissions::from_mode(0o755)).expect("chmod");

            let record = install.join(GAMING_RECORD);
            assert_eq!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Written("deck".to_string()));
            std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o666)).expect("chmod");
            assert!(matches!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Failed(_)));
            assert!(!rule.exists(), "a record others may write names nobody");
            std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o644)).expect("chmod");

            std::fs::write(&record, "deck\"; return polkit.Result.YES; //\n").expect("forged");
            assert!(matches!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Failed(_)));
            assert!(!rule.exists());

            std::fs::remove_file(&record).expect("remove");
            std::os::unix::fs::symlink(base.join("elsewhere"), &record).expect("link");
            assert!(matches!(sync_rule(&helper, &install, &base, &rule, me()), RuleSync::Failed(_)));
            let _ = std::fs::remove_dir_all(&base);
        }

        #[test]
        fn sums_match_sha256sum() {
            let base = temp("sum");
            let path = base.join("f");
            std::fs::write(&path, b"hi").expect("write");
            assert_eq!(
                sha256_file(&path).expect("sum"),
                "8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4"
            );
            assert!(is_sha256_hex(&sha256_file(&path).expect("sum")));
            assert!(sha256_file(&base).is_err(), "a folder has no sum");
            let _ = std::fs::remove_dir_all(&base);
        }
    }
}
