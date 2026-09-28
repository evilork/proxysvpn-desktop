// src-tauri/src/errors.rs
//
// Every failure the user can meet, as a CODE — never as a sentence.
//
// Why codes and not strings. The app ships in Russian first and in nine more
// languages after; a Russian sentence baked into Rust cannot be translated,
// cannot be matched in a test, and cannot be mapped to an action. So the core
// answers "what happened" and the UI owns "how we say it" (src/i18n.ts) and
// "what the single button does" (src/types.ts → ErrorAction).
//
// `detail` carries text the SERVER wrote — the refusal notice we put in the
// stub link, a host name, a port number. It is shown verbatim under the
// translated phrase, never instead of it.

use serde::Serialize;

/// A failure with a stable identity.
///
/// The discriminants are part of the contract with the frontend: renaming one
/// silently breaks a screen, so they are spelled out rather than derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // ── Subscription: nothing to connect to ────────────────────────────────
    /// No subscription link stored yet. First run, or the user cleared it.
    NoSubscription,
    /// The link is not one of ours / not a URL at all.
    SubMalformed,
    /// Every address on the fallback ladder failed. Usually the user's own
    /// network, which is exactly why the ladder exists.
    SubUnreachable,
    /// Server answered, body is not a subscription we can read.
    SubInvalid,
    /// Server answered with a valid, genuinely empty list.
    SubEmpty,
    /// Every entry of a readable subscription needs a transport this build's
    /// engine does not have (the iOS build on sing-box, which had no XHTTP;
    /// none since iOS runs Xray-core, see `subscription::engine_supports_on`).
    /// Not "unreadable": the settings are fine, the app is behind them — and
    /// asking the server again, as for SubInvalid, would get the same list.
    EngineUnsupported,

    // ── Subscription: the server is refusing us on purpose ─────────────────
    // These arrive as a stub link (127.0.0.1:1) whose remark holds the notice.
    // Before this module they all surfaced as "no servers in subscription".
    /// Balance ran out.
    BalanceEmpty,
    /// Paid period is over.
    Expired,
    /// The link is already bound to another device (one link = one device).
    DeviceTaken,
    /// The account has no devices at all.
    NoDevices,
    /// Some other notice from the server; `detail` holds its text.
    SubNotice,

    // ── Bringing the tunnel up ─────────────────────────────────────────────
    /// The user dismissed the administrator prompt.
    PermissionDenied,
    /// Engine binary missing or refused to start.
    EngineStartFailed,
    /// Engine was running and died under us.
    EngineDied,
    /// A previous run left a process holding our SOCKS port.
    PortBusy,
    /// Routes or the utun device could not be set up.
    TunFailed,
    /// iOS Simulator build: Network Extensions do not run there at all, so
    /// "press again" would be a lie. Seen only by developers.
    SimulatorNoVpn,

    // ── The tunnel is up but the internet is not ───────────────────────────
    /// Machine has no network at all — not our fault, and saying so saves a
    /// support ticket.
    NetworkOffline,
    /// We cannot even reach the node's address. Address burned, or the ISP
    /// drops it — the single most common failure in Russia.
    NoRoute,
    /// Handshake completes, bytes do not flow. Classic DPI throttling.
    Blocked,
    /// Tunnel looks alive, our probe page did not answer. Neither success nor
    /// failure, and the UI says exactly that instead of guessing.
    ProbeUnconfirmed,

    // ──────────────────── Not a failure, an absence ────────────────────
    /// A latency figure does not exist for this link and never will:
    /// Hysteria2 is pure UDP, and a TCP handshake to its port cannot
    /// complete. Kept as a code so the legacy `vpn_ping` command can
    /// answer honestly instead of leaving the window on «измерение…»
    /// forever, which is what the Netherlands node did.
    ///
    /// This is a METRIC state, not a screen: its one action is to offer
    /// nothing. It must never open [7].
    PingNotApplicable,

    // ── Last resort ────────────────────────────────────────────────────────
    /// Nothing above fits. `detail` carries the raw text for the report.
    Unknown,
}

/// A failure as it crosses into the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    /// Server-written or diagnostic text. Shown under the phrase, not instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode) -> Self {
        Self { code, detail: None }
    }

    pub fn with_detail(code: ErrorCode, detail: impl Into<String>) -> Self {
        let text = detail.into();
        Self {
            code,
            // An empty detail renders as a stray blank line under the phrase.
            detail: if text.trim().is_empty() { None } else { Some(text) },
        }
    }

    /// Serialised form for `Result<_, String>` command signatures.
    ///
    /// Tauri commands in this crate return `Result<T, String>`; changing all of
    /// them to a typed error is a wider edit than this change should make, so
    /// the error travels as JSON in that string and the frontend parses it.
    /// `parseAppError` in src/types.ts is the other half.
    pub fn to_payload(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            // Serialising two plain fields cannot realistically fail, but a
            // panic here would take down a command instead of reporting one.
            r#"{"code":"UNKNOWN"}"#.to_string()
        })
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_payload())
    }
}

impl std::error::Error for AppError {}

impl From<ErrorCode> for AppError {
    fn from(code: ErrorCode) -> Self {
        Self::new(code)
    }
}

/// Classify an `anyhow` chain that has no better origin.
///
/// Deliberately narrow: it only recognises shapes we have actually seen in
/// logs. Anything else stays `Unknown` WITH its text attached, so the support
/// report still carries the truth even when we failed to name it.
pub fn classify(err: &anyhow::Error) -> AppError {
    let text = format!("{err:#}");
    let lower = text.to_lowercase();

    let code = if lower.contains("dns error")
        || lower.contains("failed to lookup address")
        || lower.contains("nodename nor servname")
    {
        ErrorCode::SubUnreachable
    } else if lower.contains("connection refused") || lower.contains("address in use") {
        ErrorCode::PortBusy
    } else if lower.contains("timed out") || lower.contains("timeout") {
        ErrorCode::NoRoute
    } else if lower.contains("operation not permitted") || lower.contains("permission denied") {
        ErrorCode::PermissionDenied
    } else {
        ErrorCode::Unknown
    };

    AppError::with_detail(code, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_screaming_snake_case() {
        let payload = AppError::new(ErrorCode::DeviceTaken).to_payload();
        assert!(payload.contains(r#""code":"DEVICE_TAKEN""#), "{payload}");
    }

    #[test]
    fn blank_detail_is_dropped() {
        let err = AppError::with_detail(ErrorCode::SubNotice, "   ");
        assert!(err.detail.is_none());
        assert_eq!(err.to_payload(), r#"{"code":"SUB_NOTICE"}"#);
    }

    #[test]
    fn detail_survives_round_trip() {
        let err = AppError::with_detail(ErrorCode::BalanceEmpty, "Пополните баланс");
        assert!(err.to_payload().contains("Пополните баланс"));
    }

    #[test]
    fn unknown_keeps_the_text_for_the_report() {
        let err = classify(&anyhow::anyhow!("something we have never seen"));
        assert_eq!(err.code, ErrorCode::Unknown);
        assert!(err.detail.is_some());
    }

    #[test]
    fn ping_not_applicable_keeps_its_wire_name() {
        let payload = AppError::new(ErrorCode::PingNotApplicable).to_payload();
        assert_eq!(payload, r#"{"code":"PING_NOT_APPLICABLE"}"#);
    }

    #[test]
    fn timeouts_read_as_no_route() {
        let err = classify(&anyhow::anyhow!("connect 1.2.3.4:443: operation timed out"));
        assert_eq!(err.code, ErrorCode::NoRoute);
    }
}
