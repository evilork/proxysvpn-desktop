// src-tauri/crates/pvpn-platform/src/shell.rs
//
// Opening a link from an elevated process without handing the browser our
// rights (Windows).
//
// The whole app runs as administrator on Windows (privilege.rs). The opener
// plugin calls ShellExecute from that process, so a browser or Telegram that
// was not already running starts elevated too — and so does every page and
// extension in that session. Explorer is the person's own, unelevated shell:
// `explorer.exe <url>` from an elevated process hands the URL to the running
// shell, which starts the default handler with the person's normal rights,
// and the explorer.exe we started exits at once.
//
// What reaches Explorer is checked here, because Explorer's command line is
// not a plain argument vector: it takes switches (`/select,`, `/root,`) and
// splits on commas. Only the four schemes the opener plugin's default scope
// lets through are accepted (src/externalUrl.ts mirrors the same list), a
// comma is percent-encoded, and a quote, whitespace or control character is
// refused rather than repaired. A refusal never names the URL: links can
// carry a subscription token, and errors end up in the log.

use anyhow::{bail, Result};

/// Longer than any link the app builds; a guard, not a format rule.
const MAX_URL_LEN: usize = 4096;

/// The schemes `opener:default` allows, lower-case, with their separators.
const SCHEMES: [&str; 4] = ["https://", "http://", "mailto:", "tel:"];

/// The single argument Explorer gets for `url`.
pub fn explorer_arg(url: &str) -> Result<String> {
    if url.len() > MAX_URL_LEN {
        bail!("link too long for the shell ({} bytes)", url.len());
    }
    let scheme_ok = SCHEMES.iter().any(|scheme| {
        url.get(..scheme.len())
            .map(|head| head.eq_ignore_ascii_case(scheme))
            .unwrap_or(false)
    });
    if !scheme_ok {
        bail!("link scheme is not one the app opens");
    }
    if url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '"')
    {
        bail!("link holds a character the shell would read as syntax");
    }
    Ok(url.replace(',', "%2C"))
}

/// Open `url` with the person's default handler, unelevated.
///
/// Explorer's own exit code means nothing here (it exits 1 after a
/// successful hand-off), so only a failure to start it is an error.
#[cfg(windows)]
pub fn open_url_unelevated(url: &str) -> Result<()> {
    use anyhow::Context;

    let arg = explorer_arg(url)?;
    std::process::Command::new(crate::net::plan::windows::explorer_path())
        .arg(arg)
        .spawn()
        .map(drop)
        .context("start Explorer to open a link")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_schemes_pass_unchanged() {
        for url in [
            "https://proxysvpn.com/dashboard?src=app",
            "http://example.com/",
            "HTTPS://t.me/proxysvpn_bot",
            "mailto:support@proxysvpn.com?subject=help",
            "tel:+10000000000",
        ] {
            assert_eq!(explorer_arg(url).expect("allowed"), url);
        }
    }

    /// Anything Explorer would take as a path or a switch never reaches it.
    #[test]
    fn other_schemes_and_switches_are_refused() {
        for url in [
            "file:///C:/Windows/System32/cmd.exe",
            "/select,C:\\Windows",
            "C:\\Windows\\System32\\cmd.exe",
            "shell:startup",
            "ms-settings:network",
            "javascript:alert(1)",
            "",
            "http:/",
        ] {
            assert!(explorer_arg(url).is_err(), "{url}");
        }
    }

    #[test]
    fn commas_are_encoded_and_syntax_is_refused() {
        assert_eq!(
            explorer_arg("https://example.com/a,b?x=1,2").expect("allowed"),
            "https://example.com/a%2Cb?x=1%2C2"
        );
        for url in [
            "https://example.com/a b",
            "https://example.com/\"x",
            "https://example.com/\tx",
            "https://example.com/\u{7}",
        ] {
            assert!(explorer_arg(url).is_err(), "{url:?}");
        }
        assert!(explorer_arg(&format!("https://e.com/{}", "a".repeat(MAX_URL_LEN))).is_err());
    }

    /// The refusal goes to the log; the link, which may carry a token, must
    /// not go with it.
    #[test]
    fn a_refusal_does_not_repeat_the_link() {
        let secret = "https://example.com/sub/0123456789abcdef \"";
        let err = explorer_arg(secret).expect_err("refused");
        assert!(!format!("{err:#}").contains("0123456789abcdef"));
    }
}
