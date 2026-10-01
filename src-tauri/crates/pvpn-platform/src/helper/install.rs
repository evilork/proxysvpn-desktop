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
//! One pkexec, one password window, running a fixed script ([`SETUP_SCRIPT`])
//! under the system's own `/bin/sh`:
//!
//! 1. The GUI hashes both files where they are, in its read-only mount, and
//!    copies them into a private folder of its own ([`staging_dir`]) that root
//!    can read and the mount is not.
//! 2. Root copies each staged file into a 0600 temp file inside the root-owned
//!    folder, hashes *that copy* against the sum the GUI passed, and only then
//!    makes it executable and renames it into place. Swapping a staged file at
//!    any moment — for a link to /etc/shadow, for another program — can only
//!    fail the hash: the copy is never readable by others and never runs.
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
//! that needs a password fails at once with "No authentication agent found".
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
//! Mode broken with nothing to say why. The record is only written by the
//! first setup (the folder did not exist yet): removing it switches the rule
//! off for good, through app updates too, until /home/.proxysvpn is removed.
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
/// The setup's fifth argument when the record should be written.
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
/// A staged file is missing or does not hash to its sum.
pub const EXIT_FILES: i32 = 65;
/// A copy or a hash could not be made (a full disk).
pub const EXIT_COPY: i32 = 74;

/// The script root runs, through pkexec, to make the copy and start it.
///
/// Arguments: `$1` the install folder ([`INSTALL_DIR`]), `$2` the staging
/// folder, `$3` and `$4` the SHA-256 of the helper and of tun2socks as the GUI
/// read them in its mount, `$5` [`GAMING_FLAG`] on SteamOS and `-` elsewhere.
/// The user the record names is pkexec's `PKEXEC_UID`, the person who typed
/// the password, never an argument. Nothing in it reads stdin and nothing writes to
/// stdout: both are the protocol pipe the exec'd helper inherits, and the
/// GUI's first request is already waiting in it. POSIX sh and coreutils only,
/// so dash (Debian) and bash as sh (SteamOS, Arch) run it the same; the unit
/// tests run it too. The first line is a comment because the generic polkit
/// dialog shows the start of the command line.
pub const SETUP_SCRIPT: &str = r#"# ProxysVPN: copy the tunnel helper into a folder only root can change, then start it (docs/STEAMDECK.md)
set -eu
dest=$1 staged=$2 helper_sum=$3 engine_sum=$4 gaming=$5
umask 077
me=$(id -u)
say() { printf 'proxysvpn-setup: %s\n' "$*" >&2; }
private() {
    [ -d "$1" ] && [ ! -L "$1" ] || return 1
    [ -n "$(find "$1" -maxdepth 0 -user "$me" ! -perm -0020 ! -perm -0002 -print)" ]
}
parent=$(dirname -- "$dest")
private "$parent" || { say "$parent is not a folder only uid $me can change"; exit 73; }
fresh=yes
[ ! -e "$dest" ] && [ ! -L "$dest" ] || fresh=no
for dir in "$dest" "$dest/bin"; do
    [ -e "$dir" ] || [ -L "$dir" ] || mkdir -m 0755 -- "$dir"
    private "$dir" || { say "$dir is not a folder only uid $me can change"; exit 73; }
done
if [ "$gaming" != gaming-mode ]; then
    rm -f -- "$dest/gaming-mode-user"
elif [ "$fresh" = yes ]; then
    user=$(id -nu "${PKEXEC_UID:?}")
    printf '%s\n' "$user" > "$dest/.gaming-mode-user.new"
    chmod 0644 "$dest/.gaming-mode-user.new"
    mv -f -- "$dest/.gaming-mode-user.new" "$dest/gaming-mode-user"
fi
take() {
    src=$staged/$1 tmp=$dest/bin/.$1.new
    rm -f -- "$tmp"
    [ -f "$src" ] && [ ! -L "$src" ] || { say "$1 was not staged"; exit 65; }
    head -c 268435456 -- "$src" > "$tmp" || { rm -f -- "$tmp"; say "$1 could not be copied"; exit 74; }
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

/// The arguments for `pkexec /bin/sh …` that run [`SETUP_SCRIPT`].
pub fn setup_args(
    install_dir: &Path,
    staged: &Path,
    helper_sum: &str,
    engine_sum: &str,
    gaming: bool,
) -> Vec<OsString> {
    vec![
        OsString::from("-c"),
        OsString::from(SETUP_SCRIPT),
        OsString::from(SETUP_NAME),
        install_dir.as_os_str().to_os_string(),
        staged.as_os_str().to_os_string(),
        OsString::from(helper_sum),
        OsString::from(engine_sum),
        OsString::from(if gaming { GAMING_FLAG } else { "-" }),
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
// at every start. To withdraw it, remove that record as well as this file, or
// remove {INSTALL_DIR} entirely.
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

    let user = match read_record(&install_dir.join(GAMING_RECORD), owner) {
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
#[cfg(target_os = "linux")]
fn inspect(install_dir: &Path, wants: &[(&'static str, &str)]) -> (Installed, PathBuf) {
    // Physical paths throughout: pkexec runs realpath(3) on the program it is
    // given, and a /home that is a link (/var/home on some systems) must be
    // judged as the folder it really is.
    let physical = std::fs::canonicalize(install_dir).unwrap_or_else(|_| {
        let parent = install_dir.parent().unwrap_or(Path::new("/"));
        let name = install_dir.file_name().unwrap_or_default();
        std::fs::canonicalize(parent)
            .unwrap_or_else(|_| parent.to_path_buf())
            .join(name)
    });
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
    (judge(&parents, &own, &files), installed_helper(&physical))
}

/// Decide how the AppImage starts its helper: the installed copy when it is
/// current, else the setup script with freshly staged files.
///
/// `own_exe` is the GUI executable inside the mount; the helper and tun2socks
/// sit beside it in the AppImage's `usr/bin`. `gaming` asks the setup to write
/// the Gaming Mode record (SteamOS).
#[cfg(target_os = "linux")]
pub fn plan_appimage_spawn(own_exe: &Path, gaming: bool) -> Result<SpawnPlan, crate::privilege::HelperSetupFailed> {
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

    let install_dir = Path::new(INSTALL_DIR);
    let (state, helper_path) = inspect(install_dir, &[(HELPER_BIN, &helper_sum), (ENGINE_BIN, &engine_sum)]);
    match state {
        Installed::Current => Ok(SpawnPlan {
            program: helper_path,
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
                args: setup_args(install_dir, &staged, &helper_sum, &engine_sum, gaming),
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
        let args = setup_args(Path::new(INSTALL_DIR), Path::new("/run/user/1000/s"), H, E, true);
        let text: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(text[0], "-c");
        assert_eq!(text[1], SETUP_SCRIPT);
        assert_eq!(&text[2..], [SETUP_NAME, INSTALL_DIR, "/run/user/1000/s", H, E, GAMING_FLAG]);
        let off = setup_args(Path::new(INSTALL_DIR), Path::new("/s"), H, E, false);
        assert_eq!(off.last().map(|a| a.to_string_lossy().into_owned()).as_deref(), Some("-"));
        assert!(!SETUP_SCRIPT.contains("$6"), "the user is PKEXEC_UID, never an argument");
        assert!(SETUP_SCRIPT.starts_with("# ProxysVPN:"), "the polkit dialog shows the start");
        for code in [EXIT_FOLDER, EXIT_FILES, EXIT_COPY] {
            assert!(SETUP_SCRIPT.contains(&format!("exit {code}")), "{code}");
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
        let helper = installed_helper(Path::new(INSTALL_DIR));
        let rule = polkit_rule("deck", &helper).expect("rule");
        assert!(rule.contains(r#"action.id === "org.freedesktop.policykit.exec""#));
        assert!(rule.contains(r#"action.lookup("program") === "/home/.proxysvpn/bin/proxysvpn-helper""#));
        assert!(rule.contains(r#"action.lookup("command_line") === "/home/.proxysvpn/bin/proxysvpn-helper --helper""#));
        assert!(rule.contains(r#"action.lookup("user") === "root""#));
        assert!(rule.contains(r#"subject.user === "deck""#));
        assert!(rule.contains("subject.local && subject.active"));
        assert_eq!(rule.matches("polkit.Result.YES").count(), 1);
        assert!(!rule.contains("Result.AUTH"), "it never changes how anything else is asked");

        assert_eq!(polkit_rule("de\"ck", &helper), None);
        assert_eq!(polkit_rule("deck", Path::new("/home/x\"/proxysvpn-helper")), None);
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

        /// `sha256sum` is coreutils on Linux and /sbin/sha256sum on recent
        /// macOS; an older Mac only has shasum, so the test brings a shim.
        fn path_with_sha256sum(base: &Path) -> std::ffi::OsString {
            let system = std::env::var_os("PATH").unwrap_or_default();
            let found = std::env::split_paths(&system)
                .chain([PathBuf::from("/sbin")])
                .any(|dir| dir.join("sha256sum").is_file());
            let mut dirs: Vec<PathBuf> = std::env::split_paths(&system).collect();
            if found {
                dirs.push(PathBuf::from("/sbin"));
            } else {
                let shim = base.join("shim");
                std::fs::create_dir_all(&shim).expect("shim");
                std::fs::write(shim.join("sha256sum"), "#!/bin/sh\nexec shasum -a 256 \"$@\"\n").expect("shim");
                std::fs::set_permissions(shim.join("sha256sum"), std::fs::Permissions::from_mode(0o755))
                    .expect("chmod");
                dirs.insert(0, shim);
            }
            std::env::join_paths(dirs).expect("PATH")
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

            let mut child = Command::new(SHELL)
                .args(setup_args(dest, staged, helper_sum, engine_sum, gaming))
                .env("PATH", path_with_sha256sum(base))
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
            let out = child.wait_with_output().expect("wait");
            Run {
                code: out.status.code(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }
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
        /// one keeps whatever the record says now, so removing it stays a
        /// decision; elsewhere the record is removed.
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

            std::fs::remove_file(&record).expect("switched off");
            let update = run_setup_as(&base, &dest, &staged, &hs, &es, true);
            assert_eq!(update.code, Some(0), "{}", update.stderr);
            assert!(!record.exists(), "an update does not switch it back on");

            std::fs::write(&record, "deck\n").expect("record");
            let elsewhere = run_setup_as(&base, &dest, &staged, &hs, &es, false);
            assert_eq!(elsewhere.code, Some(0), "{}", elsewhere.stderr);
            assert!(!record.exists(), "not SteamOS: no record");
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
