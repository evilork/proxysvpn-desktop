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
// renamed into place, so neither side can be redirected through a link.
//
// Fail closed: if the copy cannot be made, no engine is found at all and
// connecting fails with ENGINE_START_FAILED until the next launch. Falling
// back to the bundle would let anyone who can make the copy fail (a FIFO in
// place of xray is enough) choose what root runs a moment later.
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
            stage(&source_dirs(&own), &parents, 0).map(|count| (home, count))
        });
    let result = match outcome {
        Ok((home, count)) => {
            crate::logger::log("info", "engine", &format!("{count} engine files copied to the root-owned folder"));
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

/// Create (if missing) and check each folder in `chain`, then copy every
/// engine and asset found in `sources` into the last one. Returns how many
/// files were copied. `owner` is the uid every folder must belong to: 0 in
/// the app, the test runner's own uid in tests.
fn stage(sources: &[PathBuf], chain: &[PathBuf], owner: u32) -> Result<usize, String> {
    let home = chain.last().ok_or_else(|| "no destination".to_string())?;
    for dir in chain {
        ensure_private_dir(dir, owner)?;
    }

    let mut copied = 0usize;
    for stem in ENGINES {
        let names = pvpn_platform::triple::sidecar_file_names(stem);
        if let Some(src) = first_present(sources, &names) {
            copy_private(&src, &home.join(stem), 0o755).map_err(|e| format!("{stem}: {e}"))?;
            copied += 1;
        }
    }
    for asset in ASSETS {
        if let Some(src) = first_present(sources, &[asset.to_string()]) {
            copy_private(&src, &home.join(asset), 0o644).map_err(|e| format!("{asset}: {e}"))?;
            copied += 1;
        }
    }
    if copied == 0 {
        return Err("no engine found in the bundle".to_string());
    }
    Ok(copied)
}

fn first_present(dirs: &[PathBuf], names: &[String]) -> Option<PathBuf> {
    dirs.iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.symlink_metadata().is_ok())
}

/// The folder exists (created 0755 if it did not), is a real folder and not a
/// link, belongs to `owner`, and nobody else may write into it.
fn ensure_private_dir(dir: &Path, owner: u32) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    match std::fs::DirBuilder::new().mode(0o755).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("create {}: {e}", dir.display())),
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("stat {}: {e}", dir.display()))?;
    if !meta.file_type().is_dir() {
        return Err(format!("{} is not a plain folder", dir.display()));
    }
    if meta.uid() != owner {
        return Err(format!("{} belongs to uid {}, not {owner}", dir.display(), meta.uid()));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(format!("{} is writable by others (mode {:o})", dir.display(), meta.mode() & 0o7777));
    }
    Ok(())
}

/// Copy `src` to `dst` through one descriptor on each side, with `mode`.
fn copy_private(src: &Path, dst: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut from = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(src)?;
    if !from.metadata()?.file_type().is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
    }

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
    result
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

        let copied = stage(&source_dirs(&own), &chain, me()).expect("staged");
        assert_eq!(copied, ENGINES.len() + ASSETS.len());

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
    fn a_fifo_or_folder_in_place_of_an_engine_fails_the_whole_copy() {
        let base = temp_dir("fifo");
        let own = bundle(&base);
        std::fs::remove_file(own.join("hysteria")).expect("remove");
        std::fs::create_dir(own.join("hysteria")).expect("folder");
        let chain = [base.join("stage")];
        assert!(stage(&source_dirs(&own), &chain, me()).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_folder_someone_else_may_write_into_is_refused() {
        let base = temp_dir("perm");
        let own = bundle(&base);
        let open = base.join("open");
        std::fs::create_dir(&open).expect("dir");
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).expect("chmod");
        assert!(stage(&source_dirs(&own), std::slice::from_ref(&open), me()).is_err());

        let link = base.join("link");
        std::os::unix::fs::symlink(&open, &link).expect("link");
        assert!(stage(&source_dirs(&own), &[link], me()).is_err(), "a linked folder is refused");

        let theirs = base.join("theirs");
        std::fs::create_dir(&theirs).expect("dir");
        assert!(
            stage(&source_dirs(&own), &[theirs], me().wrapping_add(1)).is_err(),
            "a folder of another uid is refused"
        );
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
