// src-tauri/src/engine_stage.rs
//
// macOS: the engines root runs are copies in a root-owned folder, never the
// files inside the app bundle.
//
// ── Why ────────────────────────────────────────────────────────────────────
// The whole macOS app runs as root (launcher.sh), and it starts xray,
// tun2socks and hysteria on every connect, reconnect, location change and
// repair. Those files sat in Contents/MacOS of a bundle the person dragged
// into /Applications, which leaves it owned by them: any program running as
// that user — no admin rights, no password — could replace
// /Applications/ProxysVPN.app/Contents/MacOS/xray, and the next connect ran
// the replacement as root without a new prompt.
//
// ── What happens now ───────────────────────────────────────────────────────
// Right after a root launch, before anything can connect, every engine and
// both geo files are copied into
//
//     /Library/Application Support/ProxysVPN/engines/<id of this copy>/
//
// a folder root creates and owns, under parents only root can write
// (/Library and /Library/Application Support are root-owned and not group-
// or world-writable). From then on the engines are found only there. What
// happens to the bundle afterwards changes nothing about what root runs.
// Tampering with the bundle *before* the person types the password is not
// covered and cannot be by this app: the launcher that shows the password
// dialog lives in the same bundle, and without a Developer ID signature
// nothing tells a person which dialog is ours.
//
// The copy is read from one open file descriptor (O_NOFOLLOW, checked to be a
// regular file) and written to a fresh temp file (O_EXCL|O_NOFOLLOW) that is
// renamed into place, so neither side can be redirected through a link. The
// source is opened O_NONBLOCK, so a FIFO in place of an engine is opened and
// refused at once instead of blocking the root process before any window.
//
// Fail closed: if the copy cannot be made, no engine is found at all and
// connecting fails with ENGINE_START_FAILED until the next launch. Falling
// back to the bundle would let anyone who can make the copy fail (a FIFO in
// place of xray is enough) choose what root runs a moment later.
//
// Causes a relaunch would not cure are kept out of that path: a staged file
// that already holds exactly the bundle's bytes is left as it is (so a full
// disk or an unwritable folder does not lock out a copy that is complete),
// and a root-owned folder of the chain that is merely group- or world-
// writable (umask 002 when 0.3.1 created it) is tightened, not refused.
//
// <id of this copy> is derived from the bundle's own folder, so two copies of
// the app keep separate engines (pidfile.rs tells them apart by folder) and a
// relaunch of the same copy after a crash finds — and stops — what the crash
// left running.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

/// Root-owned parent of every staged copy. Created by the root process with
/// the folder below it; the state stores already live one level up.
const STAGE_PARENT: &str = "/Library/Application Support/ProxysVPN";
const STAGE_DIR_NAME: &str = "engines";

/// The three engines, staged under their bundle names.
const ENGINES: [&str; 3] = ["xray", "tun2socks", "hysteria"];
/// xray's rule data; xray starts without them and only fails on the first
/// rule that needs them, so they are copied when present.
const ASSETS: [&str; 2] = ["geoip.dat", "geosite.dat"];

static STAGED: OnceLock<Result<PathBuf, String>> = OnceLock::new();

fn effective_uid() -> u32 {
    // SAFETY: geteuid(2) has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Where this copy's engines go when it runs as root.
pub fn home_for(own_dir: &Path) -> PathBuf {
    let digest = Sha256::digest(own_dir.as_os_str().as_encoded_bytes());
    let id: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    Path::new(STAGE_PARENT).join(STAGE_DIR_NAME).join(id)
}

/// The canonical folder of our own executable.
fn own_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()?;
    exe.parent().map(Path::to_path_buf)
}

/// This copy's engine folder, when the process is root (whether or not the
/// copy has been made yet). pidfile.rs uses it to recognise engines a crashed
/// run of this same copy left behind.
pub fn current_home() -> Option<PathBuf> {
    if effective_uid() != 0 {
        return None;
    }
    own_dir().map(|dir| home_for(&dir))
}

/// `None` when the process is not root and the bundle is used as before;
/// otherwise the outcome of the copy made at launch.
pub fn staged() -> Option<&'static Result<PathBuf, String>> {
    STAGED.get()
}

/// Folders the bundle keeps the engines and geo files in, as `sidecars::dirs`
/// lists them, minus the ones that need a Tauri handle (they are the same
/// folders inside a bundle).
fn source_dirs(own: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![own.to_path_buf()];
    if let Some(contents) = own.parent() {
        let resources = contents.join("Resources");
        dirs.push(resources.join("binaries"));
        dirs.push(resources.join("_up_").join("binaries"));
        dirs.push(resources);
    }
    #[cfg(debug_assertions)]
    dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries"));
    dirs
}

/// Make the copy. Called once, right after the launch-time cleanup and
/// before anything can connect. A no-op for a process that is not root.
pub fn prepare() {
    if effective_uid() != 0 {
        return;
    }
    let outcome = own_dir()
        .ok_or_else(|| "own folder unknown".to_string())
        .and_then(|own| {
            let home = home_for(&own);
            let parents = [
                PathBuf::from(STAGE_PARENT),
                Path::new(STAGE_PARENT).join(STAGE_DIR_NAME),
                home.clone(),
            ];
            stage(&source_dirs(&own), &parents, 0).map(|staged| (home, staged))
        });
    let result = match outcome {
        Ok((home, staged)) => {
            crate::logger::log(
                "info",
                "engine",
                &format!(
                    "engine files in the root-owned folder: {} copied, {} already up to date",
                    staged.copied, staged.unchanged
                ),
            );
            Ok(home)
        }
        Err(reason) => {
            crate::logger::log(
                "error",
                "engine",
                &format!("engines could not be copied to the root-owned folder, none will start: {reason}"),
            );
            Err(reason)
        }
    };
    let _ = STAGED.set(result);
}

/// What one `stage` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Staged {
    /// Files written afresh.
    copied: usize,
    /// Files whose staged copy already held exactly the bundle's bytes.
    unchanged: usize,
}

/// Create (if missing) and check each folder in `chain`, then copy every
/// engine and asset found in `sources` into the last one, skipping those
/// whose staged copy is already identical. `owner` is the uid every folder
/// must belong to: 0 in the app, the test runner's own uid in tests.
fn stage(sources: &[PathBuf], chain: &[PathBuf], owner: u32) -> Result<Staged, String> {
    let home = chain.last().ok_or_else(|| "no destination".to_string())?;
    for dir in chain {
        ensure_private_dir(dir, owner)?;
    }

    let mut plan: Vec<(PathBuf, &str, u32)> = Vec::new();
    for stem in ENGINES {
        let names = pvpn_platform::triple::sidecar_file_names(stem);
        if let Some(src) = first_present(sources, &names) {
            plan.push((src, stem, 0o755));
        }
    }
    for asset in ASSETS {
        if let Some(src) = first_present(sources, &[asset.to_string()]) {
            plan.push((src, asset, 0o644));
        }
    }
    if plan.is_empty() {
        return Err("no engine found in the bundle".to_string());
    }

    let mut staged = Staged { copied: 0, unchanged: 0 };
    for (src, name, mode) in plan {
        match copy_private(&src, &home.join(name), mode, owner).map_err(|e| format!("{name}: {e}"))? {
            CopyOutcome::Copied => staged.copied += 1,
            CopyOutcome::Unchanged => staged.unchanged += 1,
        }
    }
    Ok(staged)
}

fn first_present(dirs: &[PathBuf], names: &[String]) -> Option<PathBuf> {
    dirs.iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.symlink_metadata().is_ok())
}

/// The folder exists (created 0755 if it did not), is a real folder and not a
/// link, belongs to `owner`, and nobody else may write into it.
///
/// A folder of `owner` that group or others may write into is tightened
/// (their write bits dropped) instead of refused: 0.3.1 created
/// /Library/Application Support/ProxysVPN with create_dir_all, and under a
/// umask of 002 that left it root:admin 775 — refusing it locked every
/// engine out on every launch until someone ran chmod by hand. Everything is
/// done through one descriptor opened O_DIRECTORY|O_NOFOLLOW, so the folder
/// checked is the folder changed, and a link is still refused.
fn ensure_private_dir(dir: &Path, owner: u32) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

    match std::fs::DirBuilder::new().mode(0o755).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("create {}: {e}", dir.display())),
    }
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)
        .map_err(|e| format!("{} is not a plain folder: {e}", dir.display()))?;
    let meta = handle.metadata().map_err(|e| format!("stat {}: {e}", dir.display()))?;
    if !meta.file_type().is_dir() {
        return Err(format!("{} is not a plain folder", dir.display()));
    }
    if meta.uid() != owner {
        return Err(format!("{} belongs to uid {}, not {owner}", dir.display(), meta.uid()));
    }
    let mode = meta.mode() & 0o7777;
    if mode & 0o022 != 0 {
        let tightened = mode & !0o022;
        handle
            .set_permissions(std::fs::Permissions::from_mode(tightened))
            .map_err(|e| format!("{} is writable by others (mode {mode:o}) and chmod failed: {e}", dir.display()))?;
        let after = handle.metadata().map_err(|e| format!("stat {}: {e}", dir.display()))?;
        if after.mode() & 0o022 != 0 {
            return Err(format!("{} is still writable by others (mode {:o})", dir.display(), after.mode() & 0o7777));
        }
        crate::logger::log(
            "warn",
            "engine",
            &format!("{} was writable by others (mode {mode:o}); now {tightened:o}", dir.display()),
        );
    }
    Ok(())
}

/// Open `src` for reading only if it is a regular file, never through a link.
///
/// O_NONBLOCK on the open itself: without it a FIFO planted in place of an
/// engine blocks open(2) until someone opens it for writing, and `prepare`
/// runs before the window exists — the root process would hang there, every
/// relaunch adding another. With it the open returns at once, the fstat
/// below refuses the FIFO, and the flag is cleared again for the real read.
fn open_regular_source(src: &Path) -> io::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let from = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(src)?;
    if !from.metadata()?.file_type().is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
    }
    let fd = from.as_raw_fd();
    // SAFETY: fcntl(2) on a descriptor `from` owns for the whole call; F_GETFL
    // and F_SETFL read and write only the descriptor's status flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(from)
}

/// What `copy_private` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyOutcome {
    Copied,
    /// The staged file already held exactly these bytes, as a regular file of
    /// `owner` with `mode`; nothing was written.
    Unchanged,
}

/// Does `dst` already hold exactly what `from` holds, as a regular file of
/// `owner` with exactly `mode`? Any doubt reads as "no", and the caller then
/// copies as before. Leaves `from` at an unspecified offset.
fn staged_copy_matches(from: &mut std::fs::File, dst: &Path, mode: u32, owner: u32) -> io::Result<bool> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let Ok(mut staged) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(dst)
    else {
        return Ok(false);
    };
    let theirs = staged.metadata()?;
    let ours = from.metadata()?;
    if !theirs.file_type().is_file()
        || theirs.uid() != owner
        || theirs.mode() & 0o7777 != mode
        || theirs.len() != ours.len()
    {
        return Ok(false);
    }

    const CHUNK: usize = 64 * 1024;
    let mut a = vec![0u8; CHUNK];
    let mut b = vec![0u8; CHUNK];
    loop {
        let n = from.read(&mut a)?;
        if n == 0 {
            // Both ends at once: the staged file must not be longer.
            return Ok(staged.read(&mut b[..1])? == 0);
        }
        if staged.read_exact(&mut b[..n]).is_err() || a[..n] != b[..n] {
            return Ok(false);
        }
    }
}

/// Copy `src` to `dst` through one descriptor on each side, with `mode`,
/// unless `dst` already holds exactly the same bytes.
fn copy_private(src: &Path, dst: &Path, mode: u32, owner: u32) -> io::Result<CopyOutcome> {
    use std::io::{Seek, SeekFrom};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut from = open_regular_source(src)?;
    if staged_copy_matches(&mut from, dst, mode, owner)? {
        return Ok(CopyOutcome::Unchanged);
    }
    from.seek(SeekFrom::Start(0))?;

    let dir = dst
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no folder"))?;
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no name"))?;
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    // A leftover from a copy this pid never finished; the folder is ours
    // alone, so whatever is there is our own debris.
    let _ = std::fs::remove_file(&tmp);

    let result = (|| -> io::Result<()> {
        let mut to = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)?;
        io::copy(&mut from, &mut to)?;
        // The umask may have taken bits away; it can never have added any,
        // but say exactly what the file is.
        to.set_permissions(std::fs::Permissions::from_mode(mode))?;
        std::fs::rename(&tmp, dst)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map(|()| CopyOutcome::Copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("pvpn-stage-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::canonicalize(&dir).expect("canonical temp dir")
    }

    fn me() -> u32 {
        effective_uid()
    }

    fn bundle(base: &Path) -> PathBuf {
        let macos = base.join("ProxysVPN.app/Contents/MacOS");
        let res = base.join("ProxysVPN.app/Contents/Resources/binaries");
        std::fs::create_dir_all(&macos).expect("macos");
        std::fs::create_dir_all(&res).expect("resources");
        for stem in ENGINES {
            std::fs::write(macos.join(stem), format!("#!/bin/sh\necho {stem}\n")).expect("engine");
        }
        std::fs::write(res.join("geoip.dat"), b"geoip").expect("geoip");
        std::fs::write(res.join("geosite.dat"), b"geosite").expect("geosite");
        macos
    }

    /// The point of the module: after the copy, rewriting the bundle (which
    /// the logged-in user owns) changes nothing about what root will run.
    #[test]
    fn the_staged_engines_do_not_follow_later_changes_to_the_bundle() {
        let base = temp_dir("follow");
        let own = bundle(&base);
        let home = base.join("stage/engines/x");
        let chain = [base.join("stage"), base.join("stage/engines"), home.clone()];

        let staged = stage(&source_dirs(&own), &chain, me()).expect("staged");
        assert_eq!(staged.copied, ENGINES.len() + ASSETS.len());
        assert_eq!(staged.unchanged, 0);

        std::fs::write(own.join("xray"), b"#!/bin/sh\necho evil\n").expect("tamper");
        let staged = std::fs::read_to_string(home.join("xray")).expect("staged xray");
        assert!(staged.contains("echo xray"), "{staged}");
        assert_eq!(std::fs::read(home.join("geoip.dat")).expect("geoip"), b"geoip");

        let meta = std::fs::metadata(home.join("tun2socks")).expect("meta");
        assert_eq!(meta.permissions().mode() & 0o7777, 0o755);
        assert_eq!(meta.uid(), me());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_link_in_the_bundle_is_refused_not_followed() {
        let base = temp_dir("link");
        let own = bundle(&base);
        std::fs::remove_file(own.join("xray")).expect("remove");
        std::os::unix::fs::symlink("/bin/sh", own.join("xray")).expect("link");
        let chain = [base.join("stage")];
        let err = stage(&source_dirs(&own), &chain, me()).expect_err("a link must not be copied");
        assert!(err.contains("xray"), "{err}");
        assert!(!base.join("stage/xray").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_folder_in_place_of_an_engine_fails_the_whole_copy() {
        let base = temp_dir("folder");
        let own = bundle(&base);
        std::fs::remove_file(own.join("hysteria")).expect("remove");
        std::fs::create_dir(own.join("hysteria")).expect("folder");
        let chain = [base.join("stage")];
        assert!(stage(&source_dirs(&own), &chain, me()).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A FIFO with no writer must fail the copy at once. Before O_NONBLOCK,
    /// open(2) blocked on it forever and `prepare` hung the root process
    /// before any window appeared.
    #[test]
    fn a_fifo_in_place_of_an_engine_fails_the_copy_without_blocking() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::sync::mpsc;
        use std::time::Duration;

        let base = temp_dir("fifo");
        let own = bundle(&base);
        let fifo = own.join("hysteria");
        std::fs::remove_file(&fifo).expect("remove");
        let c_path = CString::new(fifo.as_os_str().as_bytes()).expect("path");
        // SAFETY: `c_path` is a valid NUL-terminated string for the call.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) }, 0, "mkfifo");

        let chain = [base.join("stage")];
        let sources = source_dirs(&own);
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = tx.send(stage(&sources, &chain, me()));
        });
        let outcome = rx.recv_timeout(Duration::from_secs(10));
        if outcome.is_err() {
            // Unblock the stuck open so the test process can finish, then fail.
            let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
        }
        let _ = worker.join();
        let result = outcome.expect("stage blocked on a FIFO instead of refusing it");
        let err = result.expect_err("a FIFO must not be copied");
        assert!(err.contains("hysteria"), "{err}");
        assert!(!base.join("stage/hysteria").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Our own folder that others may write into (0.3.1 under umask 002 left
    /// it root:admin 775) is tightened and used, not refused forever.
    #[test]
    fn our_folder_that_others_may_write_into_is_tightened() {
        let base = temp_dir("perm");
        let own = bundle(&base);
        for (name, loose) in [("open", 0o777), ("group", 0o775), ("sticky", 0o1777)] {
            let dir = base.join(name);
            std::fs::create_dir(&dir).expect("dir");
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(loose)).expect("chmod");
            stage(&source_dirs(&own), std::slice::from_ref(&dir), me()).expect("tightened and staged");
            let mode = std::fs::symlink_metadata(&dir).expect("meta").permissions().mode() & 0o7777;
            assert_eq!(mode, loose & !0o022, "{name}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_linked_folder_or_one_of_another_uid_is_refused() {
        let base = temp_dir("refuse");
        let own = bundle(&base);
        let open = base.join("open");
        std::fs::create_dir(&open).expect("dir");
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).expect("chmod");

        let link = base.join("link");
        std::os::unix::fs::symlink(&open, &link).expect("link");
        assert!(stage(&source_dirs(&own), &[link], me()).is_err(), "a linked folder is refused");
        let mode = std::fs::symlink_metadata(&open).expect("meta").permissions().mode() & 0o7777;
        assert_eq!(mode, 0o777, "nothing is changed through a link");

        let theirs = base.join("theirs");
        std::fs::create_dir(&theirs).expect("dir");
        std::fs::set_permissions(&theirs, std::fs::Permissions::from_mode(0o777)).expect("chmod");
        assert!(
            stage(&source_dirs(&own), std::slice::from_ref(&theirs), me().wrapping_add(1)).is_err(),
            "a folder of another uid is refused"
        );
        let mode = std::fs::symlink_metadata(&theirs).expect("meta").permissions().mode() & 0o7777;
        assert_eq!(mode, 0o777, "and left as it was");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A relaunch with the same bundle writes nothing: a full disk, or a
    /// folder nothing can be written into, must not lock out a staged set
    /// that is already complete.
    #[test]
    fn an_unchanged_bundle_keeps_the_staged_set_without_writing() {
        use std::os::unix::fs::MetadataExt;

        let base = temp_dir("same");
        let own = bundle(&base);
        let home = base.join("stage");
        let chain = [home.clone()];
        stage(&source_dirs(&own), &chain, me()).expect("first launch");
        let inode = std::fs::metadata(home.join("xray")).expect("meta").ino();

        // Nothing can be created in the folder any more.
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        let again = stage(&source_dirs(&own), &chain, me()).expect("second launch");
        assert_eq!(again, Staged { copied: 0, unchanged: ENGINES.len() + ASSETS.len() });
        assert_eq!(std::fs::metadata(home.join("xray")).expect("meta").ino(), inode);

        // A changed engine does need a copy, and that one fails closed.
        std::fs::write(own.join("xray"), b"#!/bin/sh\necho newer\n").expect("update");
        let err = stage(&source_dirs(&own), &chain, me()).expect_err("cannot write the newer xray");
        assert!(err.contains("xray"), "{err}");

        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).expect("chmod back");
        let updated = stage(&source_dirs(&own), &chain, me()).expect("third launch");
        assert_eq!(updated.copied, 1);
        let staged = std::fs::read_to_string(home.join("xray")).expect("staged xray");
        assert!(staged.contains("echo newer"), "{staged}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Same length, different bytes, or a staged copy whose mode drifted:
    /// copied again, never trusted.
    #[test]
    fn a_staged_copy_that_differs_in_any_way_is_replaced() {
        let base = temp_dir("differs");
        let own = bundle(&base);
        let home = base.join("stage");
        let chain = [home.clone()];
        stage(&source_dirs(&own), &chain, me()).expect("first launch");

        let original = std::fs::read(own.join("tun2socks")).expect("source");
        let mut flipped = original.clone();
        if let Some(last) = flipped.last_mut() {
            *last ^= 0x01;
        }
        std::fs::write(home.join("tun2socks"), &flipped).expect("same length, other bytes");
        std::fs::set_permissions(home.join("hysteria"), std::fs::Permissions::from_mode(0o775)).expect("chmod");

        let again = stage(&source_dirs(&own), &chain, me()).expect("second launch");
        assert_eq!(again.copied, 2);
        assert_eq!(std::fs::read(home.join("tun2socks")).expect("staged"), original);
        let mode = std::fs::metadata(home.join("hysteria")).expect("meta").permissions().mode() & 0o7777;
        assert_eq!(mode, 0o755);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A planted link where the staged file will go is replaced, not
    /// written through.
    #[test]
    fn a_link_planted_at_the_destination_is_replaced_not_written_through() {
        let base = temp_dir("dst");
        let own = bundle(&base);
        let home = base.join("stage");
        std::fs::create_dir(&home).expect("dir");
        let victim = base.join("victim");
        std::fs::write(&victim, b"keep").expect("victim");
        std::os::unix::fs::symlink(&victim, home.join("xray")).expect("link");

        stage(&source_dirs(&own), std::slice::from_ref(&home), me()).expect("staged");
        assert_eq!(std::fs::read(&victim).expect("victim"), b"keep");
        assert!(!std::fs::symlink_metadata(home.join("xray")).expect("meta").file_type().is_symlink());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn each_copy_of_the_app_gets_its_own_folder_under_the_root_owned_parent() {
        let a = home_for(Path::new("/Applications/ProxysVPN.app/Contents/MacOS"));
        let b = home_for(Path::new("/Volumes/ProxysVPN/ProxysVPN.app/Contents/MacOS"));
        assert_ne!(a, b);
        assert_eq!(a, home_for(Path::new("/Applications/ProxysVPN.app/Contents/MacOS")));
        assert!(a.starts_with("/Library/Application Support/ProxysVPN/engines"));
    }
}
