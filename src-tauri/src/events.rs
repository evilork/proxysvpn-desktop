// src-tauri/src/events.rs
//
// The whole contract between the core and the window.
//
// What changed and why. The old app asked `vpn_status` once, at mount, and
// never again (App.tsx:118-126), so a tunnel that died left a green screen
// forever. And `vpn_connect` returned Ok the moment routes were up, without a
// single byte having crossed — on a blocked node the screen said "connected"
// while nothing opened. Both are the same bug: the UI was told about
// INTENTIONS, not about FACTS.
//
// So the core now pushes facts:
//   vpn:state   — the phase changed (the only thing the shield renders)
//   vpn:step    — progress inside `Starting`, so the wait is honest
//   vpn:metric  — round trip, bytes, when the last proof of life was
//   vpn:event   — a one-off notice, usually an AppError
//   vpn:meta    — what the subscription told us about the account
//
// Events can be missed (window reload, backgrounded app), so `vpn_snapshot`
// returns the same truth on demand. Events are for liveness, the snapshot is
// for correctness; the UI uses both and trusts the snapshot.

use serde::Serialize;

use crate::errors::AppError;

pub const EV_STATE: &str = "vpn:state";
pub const EV_STEP: &str = "vpn:step";
pub const EV_METRIC: &str = "vpn:metric";
pub const EV_EVENT: &str = "vpn:event";
pub const EV_META: &str = "vpn:meta";

/// What the user is looking at. Five states, not three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VpnPhase {
    /// Nothing running.
    Off,
    /// Working on it — see `VpnStep` for where we are.
    Starting,
    /// A byte has crossed the tunnel and come back. This and only this is
    /// green.
    On,
    /// Tunnel is up and counters move, but our probe page did not answer.
    /// Not success, not failure — and the UI must not pretend otherwise.
    Unconfirmed,
    /// Something broke and we are fixing it without asking. Silent for the
    /// first seconds; the UI only whispers once it drags on.
    Healing,
    /// We gave up. There is an `AppError` with this one.
    Failed,
}

/// Progress inside `Starting`. Emitted as it actually happens — the old UI
/// showed a spinner and invented reassurance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VpnStep {
    /// Downloading the subscription (possibly walking the fallback ladder).
    FetchingSub,
    /// Deciding which node to use.
    PickingServer,
    /// Spawning xray / hysteria, or handing config to the Network Extension.
    StartingEngine,
    /// Raising utun and installing routes — the step that asks for a password.
    RaisingTun,
    /// Sending the first real request through the tunnel.
    Probing,
}

/// How good the link feels. Words, not milliseconds: "38 мс" means nothing to
/// most of our users, and a number invites them to chase it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkQuality {
    Good,
    Ok,
    Poor,
    /// No measurement yet. It may still arrive.
    Unknown,
    /// This link cannot be measured this way and never will be. Hysteria2 is
    /// pure UDP, so a TCP handshake to its port never completes — the old
    /// client showed "измерение…" on the Netherlands node forever because it
    /// could not tell this apart from `Unknown`.
    NotApplicable,
}

impl LinkQuality {
    /// Thresholds are deliberately generous. Mobile networks in Russia sit
    /// around 120-250 ms on a good day, and calling that "poor" would paint
    /// the screen orange for a working connection.
    /// Never returns `NotApplicable`: that verdict comes from knowing WHAT
    /// is being measured, not from a number.
    pub fn from_rtt_ms(rtt: u32) -> Self {
        match rtt {
            0..=150 => Self::Good,
            151..=400 => Self::Ok,
            _ => Self::Poor,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatePayload {
    pub phase: VpnPhase,
    /// Node label as the subscription named it, e.g. "Нидерланды".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// "vless" | "hy2" — shown only in the details sheet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proto: Option<String>,
    /// Present when `phase == Failed`, and sometimes on `Healing` to explain
    /// what we are repairing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AppError>,
    /// How long we have been in `Healing`. The UI stays silent under 8 s,
    /// whispers until 45 s, and speaks up after that.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub healing_for_ms: Option<u64>,
}

// Constructors for the payload: `of` is exercised by the tests, `at` and
// `failed` by nothing yet — the core builds the payload from the session.
#[allow(dead_code)]
impl StatePayload {
    pub fn of(phase: VpnPhase) -> Self {
        Self {
            phase,
            location: None,
            proto: None,
            error: None,
            healing_for_ms: None,
        }
    }

    pub fn at(phase: VpnPhase, location: impl Into<String>, proto: impl Into<String>) -> Self {
        Self {
            phase,
            location: Some(location.into()),
            proto: Some(proto.into()),
            error: None,
            healing_for_ms: None,
        }
    }

    pub fn failed(error: AppError) -> Self {
        Self {
            phase: VpnPhase::Failed,
            location: None,
            proto: None,
            error: Some(error),
            healing_for_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepPayload {
    pub step: VpnStep,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricPayload {
    /// Round trip through the tunnel, milliseconds. `None` while unmeasured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
    pub quality: Option<LinkQuality>,
    /// Interface counters, cumulative for this session.
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    /// Unix milliseconds of the last successful proof of life. The UI ages
    /// this out loud — "проверено 9 секунд назад" — so a stale green is
    /// visibly stale instead of silently wrong.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_proof_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPayload {
    pub error: AppError,
    /// True when the core is already handling it and the user need not act.
    pub handled: bool,
}

/// What the subscription response told us about the account.
///
/// All of this already travels in headers we were throwing away
/// (subscription.rs read only `.text()`), which is why the app could not show
/// an expiry date or an outage notice that the service had already written.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubMeta {
    /// `profile-title` — the account's display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `announce` — service-wide notice. The one line allowed to shout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub announce: Option<String>,
    /// `announce-url` — where the notice leads, if anywhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub announce_url: Option<String>,
    /// Expiry as unix seconds, from `subscription-userinfo`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// Free-form status line from `sub-info-text`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info_text: Option<String>,
    /// `support-url` — used by the "written to support" button.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_url: Option<String>,
    /// `routing-enable` — whether the service wants split routing on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_enabled: Option<bool>,
    /// How often the service wants the list refreshed, in hours.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_interval_hours: Option<u32>,
}

/// Everything at once, for a window that just mounted or woke up.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VpnSnapshot {
    pub state: StatePayload,
    pub metric: MetricPayload,
    pub meta: SubMeta,
    /// Set while `phase == Starting`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<VpnStep>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_serialise_as_the_frontend_expects() {
        let json = serde_json::to_string(&StatePayload::of(VpnPhase::Unconfirmed)).unwrap();
        assert!(json.contains(r#""phase":"unconfirmed""#), "{json}");
    }

    #[test]
    fn absent_fields_do_not_reach_the_window() {
        let json = serde_json::to_string(&StatePayload::of(VpnPhase::Off)).unwrap();
        assert_eq!(json, r#"{"phase":"off"}"#);
    }

    #[test]
    fn metric_keys_are_camel_case() {
        let json = serde_json::to_string(&MetricPayload::default()).unwrap();
        assert!(json.contains("rxBytes") && json.contains("txBytes"), "{json}");
    }

    #[test]
    fn unmeasurable_is_not_the_same_as_unmeasured() {
        // The window must be able to say "\u043d\u0435 \u043f\u0440\u0438\u043c\u0435\u043d\u0438\u043c\u043e" instead of spinning
        // forever, so the two cases have to serialise differently.
        let na = serde_json::to_string(&LinkQuality::NotApplicable).unwrap();
        let unknown = serde_json::to_string(&LinkQuality::Unknown).unwrap();
        assert_eq!(na, r#""notApplicable""#);
        assert_ne!(na, unknown);
    }

    #[test]
    fn mobile_latency_is_not_called_poor() {
        assert_eq!(LinkQuality::from_rtt_ms(38), LinkQuality::Good);
        assert_eq!(LinkQuality::from_rtt_ms(220), LinkQuality::Ok);
        assert_eq!(LinkQuality::from_rtt_ms(900), LinkQuality::Poor);
    }
}
