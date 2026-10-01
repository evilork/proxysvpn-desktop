// src-tauri/crates/pvpn-platform/src/process.rs
// Command plumbing shared by every platform implementation.
//
// `Argv` exists so that *what* a platform runs is a pure value that tests can
// pin, separate from *running* it. The golden tests in `net::plan` compare
// `Argv`s, which is how "macOS behaviour is unchanged" is actually enforced.

use anyhow::{anyhow, Context, Result};
use std::process::{Output, Stdio};
use tokio::process::Command;

/// `CREATE_NO_WINDOW` — without it every netsh/taskkill/sidecar spawn from a
/// `windows_subsystem = "windows"` GUI app flashes (or keeps) a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Suppresses the console window of a child process on Windows; no-op elsewhere.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argv {
    pub program: String,
    pub args: Vec<String>,
}

impl Argv {
    pub fn new<P, I, S>(program: P, args: I) -> Self
    where
        P: Into<String>,
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Argv {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        no_window(&mut cmd);
        cmd
    }

    /// Runs the command, discarding output, and fails on a non-zero exit.
    ///
    /// The error text is deliberately identical to the pre-split `run_cmd` in
    /// tun.rs: some of these strings reach the user through connect errors.
    pub async fn run(&self) -> Result<()> {
        let status = self
            .command()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .with_context(|| format!("spawn {} {:?}", self.program, self.args))?;
        if !status.success() {
            return Err(anyhow!(
                "{} {:?} failed: {}",
                self.program,
                self.args,
                status
            ));
        }
        Ok(())
    }

    /// Runs the command ignoring failures — for teardown steps where "already
    /// gone" and "removed" are the same outcome.
    pub async fn run_best_effort(&self) {
        if let Err(e) = self.run().await {
            crate::log::log("info", "net", &format!("ignored: {}", e));
        }
    }

    pub async fn output(&self) -> Result<Output> {
        self.command()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .with_context(|| format!("spawn {} {:?}", self.program, self.args))
    }

    /// stdout as UTF-8 (lossy), or `None` when the command could not run.
    /// Query helpers want "no answer" rather than an error type.
    pub async fn stdout_lossy(&self) -> Option<String> {
        let out = self.output().await.ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// True when the command exited 0 — used for existence probes.
    pub async fn succeeds(&self) -> bool {
        matches!(self.output().await, Ok(o) if o.status.success())
    }
}

impl std::fmt::Display for Argv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.program)?;
        for a in &self.args {
            write!(f, " {}", a)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_keeps_program_and_args() {
        let a = Argv::new("/sbin/route", ["-n", "add", "-host", "203.0.113.7"]);
        assert_eq!(a.program, "/sbin/route");
        assert_eq!(a.args, vec!["-n", "add", "-host", "203.0.113.7"]);
        assert_eq!(a.to_string(), "/sbin/route -n add -host 203.0.113.7");
    }

    /// The failure text embeds `{:?}` of the argument vector. Pinning it keeps
    /// user-visible connect errors identical to the pre-split build.
    #[test]
    fn args_debug_shape_matches_slice_debug() {
        let a = Argv::new("x", ["-n", "delete"]);
        assert_eq!(format!("{:?}", a.args), r#"["-n", "delete"]"#);
    }
}
