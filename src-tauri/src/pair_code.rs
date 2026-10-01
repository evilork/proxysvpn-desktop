// src-tauri/src/pair_code.rs
//
// Pair code v1.1, the app's half.
//
// The cabinet and the bot show an eight-character code ("Код для
// приложения"); the person types it on the sign-in screen and the service
// answers with the subscription link of the device it was issued for - once,
// within ten minutes. It covers what the QR cannot: the cabinet and the app
// on the same phone, with nothing to point a camera at.
//
// Contract pair-code v1.1, shared with the service (frontend /api/pair/code)
// and the window (src/pairCode.ts):
//   • eight characters of 23456789ABCDEFGHJKMNPQRSTUVWXYZ (no 0 1 I L O),
//     shown as XXXX-XXXX; input is uppercased and loses whitespace and
//     dashes before it is judged;
//   • POST /api/pair/code {"code": "K7QM2XPA", "requestId": "<43 chars>"}
//       200 {"status":"ready","subUrl":"https://..."}
//       400 {"error":"bad_code"}
//       404 {"error":"not_found"}    wrong, expired or used: one answer
//       429 {"error":"rate_limited"} with Retry-After
//
// What v1.1 adds over v1 is the request id, and with it a redeem that can be
// repeated. In v1 the service spent the code before its 200 left; an answer
// lost on the way (a timeout, a reset) left the app asking the next name and
// hearing "not found" about a code typed correctly. Now the app sends one
// random id per code the person submits, the same id to every name and on
// every press of the button for that code, and for five minutes after a
// redeem the service gives the same 200 again to the same id - and to no
// other. The id is optional on the wire: without one a redeem is v1's.
//
// The link that comes back is kept only when it is https on one of our own
// site names. Whoever answered, a link anywhere else would make some other
// server the source of this person's settings, and the settings decide where
// all of their traffic goes.
//
// What that check does not do: tell whose account a code is from. A code
// someone issues on their own account and hands over brings their link, on
// our name, and signs this device into their subscription. Only the service
// can close that, by naming the account for the app to show before it keeps
// the link; pair-code v1.1 does not.
//
// Never logged: the code, the link, the token in it, the request id.

use std::fmt;
use std::sync::{Mutex, PoisonError};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use url::{Host, Url};

use crate::errors::{AppError, ErrorCode};
use crate::logger;
use crate::site_ladder::{self, Method, SiteReply, SiteWire, Verdict};

/// The code alphabet: digits and capitals without 0 1 I L O, which read alike.
pub(crate) const CODE_ALPHABET: &str = "23456789ABCDEFGHJKMNPQRSTUVWXYZ";

pub(crate) const CODE_LEN: usize = 8;

pub(crate) const REDEEM_PATH: &str = "/api/pair/code";

/// Longer input is a pasted paragraph, not a code with spaces in it.
const MAX_INPUT_CHARS: usize = 64;

/// A subscription link is about 70 characters; anything near this is not one.
const MAX_LINK_LEN: usize = 2048;

/// When a 429 names no wait, or names it as a date: the limiter's own window.
pub(crate) const DEFAULT_RETRY_AFTER_SECS: u64 = 60;

/// The longest wait worth repeating: the daily limit.
const MAX_RETRY_AFTER_SECS: u64 = 24 * 3600;

/// Randomness in one request id: 32 bytes, 43 characters of base64url.
const REQUEST_ID_BYTES: usize = 32;

/// The lengths the service accepts as a request id; it ignores anything else.
const REQUEST_ID_CHARS: std::ops::RangeInclusive<usize> = 22..=64;

/// Where a request id's bytes come from: the system CSPRNG in the app, a
/// script in tests.
pub(crate) type RandomSource = fn(&mut [u8; REQUEST_ID_BYTES]) -> std::io::Result<()>;

/// The system CSPRNG. `/dev/urandom` rather than a crate, as for the device
/// id and the race password: on macOS and iOS it is always there and never
/// blocks. `read_exact` retries an interrupted read and refuses a short one.
fn os_random(buf: &mut [u8; REQUEST_ID_BYTES]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(buf)
}

/// The contract's form of a request id: `^[A-Za-z0-9_-]{22,64}$`.
pub(crate) fn is_valid_request_id(raw: &str) -> bool {
    REQUEST_ID_CHARS.contains(&raw.len())
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// One redeem's request id (pair-code v1.1): the service repeats its 200 only
/// to the same id. Made only from fresh randomness, so every value of this
/// type has the contract's form.
///
/// `Debug` hides the value: whoever holds the id and the code can take the
/// link again for five minutes, so it is never written down.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct RequestId(String);

impl RequestId {
    fn generate(random: RandomSource) -> std::io::Result<Self> {
        let mut bytes = [0u8; REQUEST_ID_BYTES];
        random(&mut bytes)?;
        let text = URL_SAFE_NO_PAD.encode(bytes);
        // An encoder change must not put an id on the wire that the service
        // would quietly ignore: the redeem would lose its replay unnoticed.
        if !is_valid_request_id(&text) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request id out of the contract's form",
            ));
        }
        Ok(Self(text))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RequestId(..)")
    }
}

/// The code the person is signing in with and its request id - the pairing
/// state of the sign-in screen.
///
/// One id per normalised code: pressing the button again with the same code
/// sends the same id, so a retry after a lost answer is repeated rather than
/// refused; another code replaces it with a new one; a sign-in that worked
/// (`forget`) drops it. One slot, not a map: the screen has one field.
pub(crate) struct RedeemIds {
    slot: Mutex<Option<(String, RequestId)>>,
    random: RandomSource,
}

impl Default for RedeemIds {
    fn default() -> Self {
        Self::with_source(os_random)
    }
}

impl RedeemIds {
    pub(crate) fn with_source(random: RandomSource) -> Self {
        Self {
            slot: Mutex::new(None),
            random,
        }
    }

    /// The id for `code` (already normalised): the one made for it before,
    /// or a fresh one that takes the place of whatever another code had.
    /// When the randomness fails the slot is left as it was.
    fn for_code(&self, code: &str) -> std::io::Result<RequestId> {
        // Nothing here can panic while the lock is held, but a poisoned slot
        // still holds a valid pair: take it rather than refuse to sign in.
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((kept, id)) = slot.as_ref() {
            if kept == code {
                return Ok(id.clone());
            }
        }
        let id = RequestId::generate(self.random)?;
        *slot = Some((code.to_string(), id.clone()));
        Ok(id)
    }

    /// The person is signed in, by this code or any other way: the next code
    /// starts over with a new id.
    pub(crate) fn forget(&self) {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The code as the service expects it, or `None` when it cannot be one.
///
/// Same rules as the service and src/pairCode.ts: uppercase, drop whitespace
/// (Unicode White_Space plus U+FEFF, the set JavaScript's `\s` matches) and
/// dashes, then exactly eight characters of the alphabet. "k7qm 2xpa" and
/// "K7QM-2XPA" are the same code.
pub(crate) fn normalise_pair_code(raw: &str) -> Option<String> {
    if raw.chars().count() > MAX_INPUT_CHARS {
        return None;
    }
    let code: String = raw
        .to_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{feff}' && *c != '-')
        .collect();
    // Byte length: a non-ASCII letter is several bytes, none of them in the
    // alphabet, so eight bytes that all are means eight ASCII characters.
    let valid =
        code.len() == CODE_LEN && code.bytes().all(|b| CODE_ALPHABET.as_bytes().contains(&b));
    valid.then_some(code)
}

/// The link a redeemed code delivered, if we are willing to keep it.
///
/// https only, a host from `hosts` (our site names) exactly, the default
/// port, no user part, a path. Returns the parsed form, which is what was
/// checked: the parser drops tabs and newlines the raw text may carry.
/// Anything else is `SubInvalid`: the service answered, and we refuse it.
pub(crate) fn accept_pair_link(raw: &str, hosts: &[&str]) -> Result<String, AppError> {
    let refuse = || AppError::new(ErrorCode::SubInvalid);
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_LINK_LEN {
        return Err(refuse());
    }
    let url = Url::parse(trimmed).map_err(|_| refuse())?;
    if url.scheme() != "https" {
        return Err(refuse());
    }
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return Err(refuse());
    }
    // A name, never an address literal: none of ours is one.
    let Some(Host::Domain(host)) = url.host() else {
        return Err(refuse());
    };
    if !hosts.iter().any(|ours| ours.eq_ignore_ascii_case(host)) {
        return Err(refuse());
    }
    if url.path().trim_matches('/').is_empty() {
        return Err(refuse());
    }
    Ok(url.to_string())
}

/// Seconds to wait from a `Retry-After` header.
///
/// Only the delta-seconds form is read; a missing header or the HTTP-date
/// form gives the limiter's minute. Clamped to 1 s .. one day.
pub(crate) fn retry_after_secs(header: Option<&str>) -> u64 {
    header
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|secs| secs.clamp(1, MAX_RETRY_AFTER_SECS))
        .unwrap_or(DEFAULT_RETRY_AFTER_SECS)
}

/// What one site's answer to `POST /api/pair/code` means.
///
/// 400, 404 and 429 are final only with the service's own body (or, for 429,
/// from anyone: every name shares one limiter, and "slow down" is advice
/// worth taking from any of them). The same status from something else - a
/// platform 404 page of a deployment without this route, a firewall - leaves
/// the question to the next name.
pub(crate) fn judge_redeem(reply: SiteReply, hosts: &[&str]) -> Verdict<String> {
    let parsed: Option<serde_json::Value> = serde_json::from_str(&reply.body).ok();
    let field = |name: &str| -> Option<&str> {
        parsed
            .as_ref()
            .and_then(|value| value.get(name))
            .and_then(serde_json::Value::as_str)
    };
    match reply.status {
        200..=299 => match (field("status"), field("subUrl")) {
            (Some("ready"), Some(link)) => match accept_pair_link(link, hosts) {
                Ok(link) => Verdict::Done(link),
                Err(err) => Verdict::Stop(err),
            },
            // JSON, but not the answer we know. The service has probably
            // spent the code already, so another name would only say "not
            // found" about it.
            _ if parsed.is_some() => Verdict::Stop(AppError::new(ErrorCode::SubInvalid)),
            // Not JSON at all: something between us and the service (a
            // captive portal) answered for it.
            _ => Verdict::Next,
        },
        400 if field("error") == Some("bad_code") => {
            Verdict::Stop(AppError::new(ErrorCode::PairCodeMalformed))
        }
        404 if field("error") == Some("not_found") => {
            Verdict::Stop(AppError::new(ErrorCode::PairCodeNotFound))
        }
        429 => Verdict::Stop(AppError::with_detail(
            ErrorCode::PairRateLimited,
            retry_after_secs(reply.retry_after.as_deref()).to_string(),
        )),
        _ => Verdict::Next,
    }
}

/// The body of `POST /api/pair/code`: the code, and the request id when
/// there is one.
fn redeem_body(code: &str, request_id: Option<&RequestId>) -> serde_json::Value {
    match request_id {
        Some(id) => serde_json::json!({ "code": code, "requestId": id.as_str() }),
        None => serde_json::json!({ "code": code }),
    }
}

/// Redeem a typed code over the ladder `hosts`; the link it delivered, or why
/// not. The link is checked against the same `hosts`.
///
/// The request id comes from `ids`: the same one for every name of this walk
/// and for every later walk with the same code, until `ids.forget()`. So when
/// no name answers - the answer may have been lost after the service spent
/// the code - the person sees the network line, presses again, and the
/// service repeats the link it already gave.
pub(crate) async fn redeem<W: SiteWire>(
    wire: &W,
    hosts: &[&str],
    ids: &RedeemIds,
    raw_code: &str,
) -> Result<String, AppError> {
    let code =
        normalise_pair_code(raw_code).ok_or_else(|| AppError::new(ErrorCode::PairCodeMalformed))?;
    let request_id = match ids.for_code(&code) {
        Ok(id) => Some(id),
        Err(e) => {
            // The id is optional on the wire: without it this is a v1 redeem,
            // which works and only cannot be repeated if its answer is lost.
            logger::log(
                "warn",
                "pair",
                &format!("no request id, redeeming without replay: {e}"),
            );
            None
        }
    };
    let body = redeem_body(&code, request_id.as_ref());
    site_ladder::walk(
        wire,
        hosts,
        Method::Post,
        REDEEM_PATH,
        Some(&body),
        |_, reply| judge_redeem(reply, hosts),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site_ladder::tests::ScriptWire;

    /// The names of the real ladder, in its order (lib.rs SITE_LADDER).
    const OURS: [&str; 4] = [
        "proxysvpn.com",
        "proxysvnovich.vercel.app",
        "proksya.com",
        "proksya.xyz",
    ];

    const LINK: &str = "https://proxysvpn.com/api/sub/abcdef0123456789";

    fn ready(link: &str) -> String {
        serde_json::json!({ "status": "ready", "subUrl": link }).to_string()
    }

    fn reply(status: u16, body: &str) -> SiteReply {
        SiteReply {
            status,
            retry_after: None,
            body: body.to_string(),
        }
    }

    fn stopped_with(verdict: Verdict<String>) -> ErrorCode {
        match verdict {
            Verdict::Stop(err) => err.code,
            other => panic!("expected a stop, got {other:?}"),
        }
    }

    /// The request id of every request the wire saw, in order.
    fn sent_ids(wire: &ScriptWire) -> Vec<String> {
        wire.bodies()
            .into_iter()
            .map(|body| {
                body.as_ref()
                    .and_then(|json| json.get("requestId"))
                    .and_then(serde_json::Value::as_str)
                    .expect("a request id in the body")
                    .to_string()
            })
            .collect()
    }

    fn no_randomness(_: &mut [u8; REQUEST_ID_BYTES]) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no /dev/urandom",
        ))
    }

    fn all_ones(buf: &mut [u8; REQUEST_ID_BYTES]) -> std::io::Result<()> {
        buf.fill(0xff);
        Ok(())
    }

    // ── normalisation ──────────────────────────────────────────────────────

    #[test]
    fn the_shown_form_the_bare_form_and_a_lowercase_paste_are_one_code() {
        for raw in [
            "K7QM-2XPA",
            "K7QM2XPA",
            "k7qm 2xpa",
            "  k7qm-2xpa\n",
            "K7QM - 2XPA",
            "k-7-q-m-2-x-p-a",
        ] {
            assert_eq!(
                normalise_pair_code(raw).as_deref(),
                Some("K7QM2XPA"),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn unicode_spaces_from_a_copied_page_are_dropped_too() {
        // A no-break space, a thin space and a BOM: what copying from a web
        // page or a chat tends to drag along.
        assert_eq!(
            normalise_pair_code("K7QM\u{a0}2XPA").as_deref(),
            Some("K7QM2XPA")
        );
        assert_eq!(
            normalise_pair_code("\u{feff}K7QM\u{2009}2XPA").as_deref(),
            Some("K7QM2XPA")
        );
    }

    #[test]
    fn the_wrong_length_is_not_a_code() {
        for raw in [
            "",
            "   ",
            "-",
            "K7QM",
            "K7QM2XP",
            "K7QM2XPAB",
            "K7QM-2XPA-K7QM",
        ] {
            assert_eq!(normalise_pair_code(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn the_look_alikes_left_out_of_the_alphabet_are_refused() {
        for raw in [
            "K7QM2XP0", "K7QM2XP1", "K7QM2XPI", "K7QM2XPL", "K7QM2XPO", "k7qm2xpo",
        ] {
            assert_eq!(normalise_pair_code(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn punctuation_and_other_scripts_are_not_stripped_into_a_code() {
        // Only whitespace and dashes go; everything else makes it malformed,
        // Cyrillic look-alikes of Latin capitals included.
        for raw in [
            "K7QM_2XPA",
            "K7QM.2XPA",
            "K7QM/2XPA",
            "К7QМ2ХРА",
            "K7QM2XP😀",
            "K7QM\u{2013}2XPA",
        ] {
            assert_eq!(normalise_pair_code(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn a_pasted_paragraph_is_refused_before_it_is_read() {
        let raw = format!("K7QM2XPA{}", " ".repeat(MAX_INPUT_CHARS));
        assert_eq!(normalise_pair_code(&raw), None);
    }

    #[test]
    fn every_character_of_the_alphabet_is_accepted() {
        for chunk in CODE_ALPHABET.as_bytes().chunks(CODE_LEN) {
            let mut code = String::from_utf8_lossy(chunk).to_string();
            while code.len() < CODE_LEN {
                code.push('2');
            }
            assert_eq!(
                normalise_pair_code(&code.to_lowercase()).as_deref(),
                Some(code.as_str())
            );
        }
        assert_eq!(CODE_ALPHABET.len(), 31);
    }

    // ── the link that comes back ───────────────────────────────────────────

    #[test]
    fn a_link_on_any_of_our_names_is_kept() {
        for host in OURS {
            let link = format!("https://{host}/api/sub/abcdef0123456789");
            assert_eq!(
                accept_pair_link(&link, &OURS).map_err(|e| e.code),
                Ok(link.clone())
            );
        }
        // Case and the default port spelled out are the same place.
        assert!(accept_pair_link("https://PROXYSVPN.com:443/api/sub/x", &OURS).is_ok());
        // The person's own query flags survive.
        assert!(accept_pair_link("https://proxysvpn.com/api/sub/x?lang=en", &OURS).is_ok());
    }

    #[test]
    fn plain_http_is_refused_even_on_our_name() {
        let err = accept_pair_link("http://proxysvpn.com/api/sub/abcdef", &OURS).expect_err("http");
        assert_eq!(err.code, ErrorCode::SubInvalid);
    }

    #[test]
    fn a_host_outside_the_ladder_is_refused() {
        for link in [
            "https://evil.example/api/sub/abcdef",
            "https://proxysvpn.com.evil.example/api/sub/abcdef",
            "https://evilproxysvpn.com/api/sub/abcdef",
            "https://www.proxysvpn.com/api/sub/abcdef",
            "https://proxysvpn.com./api/sub/abcdef",
            "https://vercel.app/api/sub/abcdef",
            "https://other.vercel.app/api/sub/abcdef",
        ] {
            assert_eq!(
                accept_pair_link(link, &OURS).map_err(|e| e.code),
                Err(ErrorCode::SubInvalid),
                "{link}"
            );
        }
    }

    #[test]
    fn hostile_shapes_of_a_link_are_refused() {
        for link in [
            // Our name as the user part, someone else's as the host.
            "https://proxysvpn.com@evil.example/api/sub/abcdef",
            "https://proxysvpn.com:pw@evil.example/api/sub/abcdef",
            // Our name with a user part in front of it.
            "https://user@proxysvpn.com/api/sub/abcdef",
            // A port of our name that is not the site.
            "https://proxysvpn.com:8443/api/sub/abcdef",
            // Address literals and other schemes.
            "https://203.0.113.7/api/sub/abcdef",
            "https://[2001:db8::1]/api/sub/abcdef",
            "vless://abcdef@proxysvpn.com:443",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "//proxysvpn.com/api/sub/abcdef",
            // No path: nothing to fetch.
            "https://proxysvpn.com",
            "https://proxysvpn.com/",
            "",
            "   ",
            "not a url",
        ] {
            assert_eq!(
                accept_pair_link(link, &OURS).map_err(|e| e.code),
                Err(ErrorCode::SubInvalid),
                "{link}"
            );
        }
        let long = format!("https://proxysvpn.com/api/sub/{}", "a".repeat(MAX_LINK_LEN));
        assert!(accept_pair_link(&long, &OURS).is_err());
    }

    #[test]
    fn what_is_kept_is_what_was_checked() {
        // The parser drops a newline inside the text; the stored link must be
        // the parsed form, not the raw text with the newline in it.
        let kept =
            accept_pair_link("https://proxysvpn.com/api/sub/abc\ndef", &OURS).expect("parsed");
        assert_eq!(kept, "https://proxysvpn.com/api/sub/abcdef");
    }

    // ── Retry-After ────────────────────────────────────────────────────────

    #[test]
    fn retry_after_reads_seconds_and_falls_back_to_a_minute() {
        assert_eq!(retry_after_secs(Some("42")), 42);
        assert_eq!(retry_after_secs(Some(" 7 ")), 7);
        assert_eq!(retry_after_secs(None), DEFAULT_RETRY_AFTER_SECS);
        assert_eq!(retry_after_secs(Some("")), DEFAULT_RETRY_AFTER_SECS);
        assert_eq!(retry_after_secs(Some("-5")), DEFAULT_RETRY_AFTER_SECS);
        assert_eq!(
            retry_after_secs(Some("Wed, 21 Oct 2026 07:28:00 GMT")),
            DEFAULT_RETRY_AFTER_SECS
        );
        assert_eq!(
            retry_after_secs(Some("0")),
            1,
            "zero would mean hammer at once"
        );
        assert_eq!(retry_after_secs(Some("99999999")), MAX_RETRY_AFTER_SECS);
    }

    // ── judging one answer ─────────────────────────────────────────────────

    #[test]
    fn ready_with_our_link_is_done() {
        match judge_redeem(reply(200, &ready(LINK)), &OURS) {
            Verdict::Done(link) => assert_eq!(link, LINK),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ready_with_a_hostile_link_stops_and_keeps_nothing() {
        for link in [
            "https://evil.example/api/sub/abcdef",
            "http://proxysvpn.com/api/sub/abcdef",
            "https://proxysvpn.com@evil.example/api/sub/abcdef",
        ] {
            assert_eq!(
                stopped_with(judge_redeem(reply(200, &ready(link)), &OURS)),
                ErrorCode::SubInvalid,
                "{link}"
            );
        }
    }

    #[test]
    fn json_of_another_shape_stops_as_invalid() {
        for body in [
            r#"{"status":"pending"}"#,
            r#"{"status":"ready"}"#,
            r#"{"status":"ready","subUrl":42}"#,
            r#"{"subUrl":"https://proxysvpn.com/api/sub/x"}"#,
            r#"[]"#,
        ] {
            assert_eq!(
                stopped_with(judge_redeem(reply(200, body), &OURS)),
                ErrorCode::SubInvalid,
                "{body}"
            );
        }
    }

    #[test]
    fn a_portal_page_with_200_leaves_it_to_the_next_name() {
        assert!(matches!(
            judge_redeem(reply(200, "<html>Hotel Wi-Fi login</html>"), &OURS),
            Verdict::Next
        ));
    }

    #[test]
    fn not_found_is_final_and_says_nothing_more() {
        let code = stopped_with(judge_redeem(reply(404, r#"{"error":"not_found"}"#), &OURS));
        assert_eq!(code, ErrorCode::PairCodeNotFound);
    }

    #[test]
    fn bad_code_from_the_service_is_malformed() {
        let code = stopped_with(judge_redeem(reply(400, r#"{"error":"bad_code"}"#), &OURS));
        assert_eq!(code, ErrorCode::PairCodeMalformed);
    }

    #[test]
    fn rate_limited_carries_the_wait_in_seconds() {
        let mut limited = reply(429, r#"{"error":"rate_limited"}"#);
        limited.retry_after = Some("37".into());
        match judge_redeem(limited, &OURS) {
            Verdict::Stop(err) => {
                assert_eq!(err.code, ErrorCode::PairRateLimited);
                assert_eq!(err.detail.as_deref(), Some("37"));
            }
            other => panic!("{other:?}"),
        }
        // No header: the limiter's minute.
        match judge_redeem(reply(429, ""), &OURS) {
            Verdict::Stop(err) => assert_eq!(err.detail.as_deref(), Some("60")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_platform_404_or_400_is_not_the_service_speaking() {
        // A deployment without the route answers its own 404 page; that is a
        // reason to ask the next name, not to tell the person the code is bad.
        for (status, body) in [
            (
                404,
                "<!DOCTYPE html><title>404: This page could not be found</title>",
            ),
            (404, r#"{"error":"something else"}"#),
            (400, "Bad Request"),
            (403, "This request was blocked"),
            (405, ""),
            (500, r#"{"error":"internal"}"#),
            (502, ""),
        ] {
            assert!(
                matches!(judge_redeem(reply(status, body), &OURS), Verdict::Next),
                "{status} {body}"
            );
        }
    }

    // ── the whole redeem over a scripted ladder ────────────────────────────

    #[tokio::test]
    async fn a_good_code_on_a_dead_primary_comes_back_from_a_reserve() {
        let wire = ScriptWire::default().dead("proxysvpn.com").answer(
            "proxysvnovich.vercel.app",
            200,
            &ready(LINK),
        );
        let ids = RedeemIds::default();
        let link = redeem(&wire, &OURS, &ids, "k7qm 2xpa")
            .await
            .expect("redeemed");
        assert_eq!(link, LINK);
        assert_eq!(
            wire.asked(),
            vec!["proxysvpn.com", "proxysvnovich.vercel.app"]
        );
        // Every name was sent the normalised code and one request id in a
        // JSON body of exactly these two fields, never in the path.
        let sent = sent_ids(&wire);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], sent[1], "one id for the whole walk");
        assert!(is_valid_request_id(&sent[0]), "{}", sent[0].len());
        for body in wire.bodies() {
            assert_eq!(
                body,
                Some(serde_json::json!({ "code": "K7QM2XPA", "requestId": sent[0] }))
            );
        }
        let calls = wire.calls.lock().expect("calls");
        assert!(calls.iter().all(|(method, url, _)| *method == Method::Post
            && url.ends_with(REDEEM_PATH)
            && !url.contains(&sent[0])
            && !url.contains("K7QM2XPA")));
    }

    #[tokio::test]
    async fn a_malformed_code_is_never_sent() {
        let wire = ScriptWire::default().answer("proxysvpn.com", 200, &ready(LINK));
        let ids = RedeemIds::default();
        let err = redeem(&wire, &OURS, &ids, "K7QM-2XP0")
            .await
            .expect_err("0 is not in the alphabet");
        assert_eq!(err.code, ErrorCode::PairCodeMalformed);
        assert!(wire.asked().is_empty());
        // Nor is an id made for it.
        assert!(ids.slot.lock().expect("slot").is_none());
    }

    #[tokio::test]
    async fn not_found_stops_the_ladder_at_the_first_name() {
        let wire = ScriptWire::default()
            .answer("proxysvpn.com", 404, r#"{"error":"not_found"}"#)
            .answer("proxysvnovich.vercel.app", 200, &ready(LINK));
        let err = redeem(&wire, &OURS, &RedeemIds::default(), "K7QM2XPA")
            .await
            .expect_err("not found");
        assert_eq!(err.code, ErrorCode::PairCodeNotFound);
        assert_eq!(
            wire.asked(),
            vec!["proxysvpn.com"],
            "a second name would spend a second attempt"
        );
    }

    #[tokio::test]
    async fn rate_limited_stops_the_ladder_with_retry_after() {
        let wire = ScriptWire::default()
            .answer_with_retry("proxysvpn.com", 429, "45")
            .answer("proxysvnovich.vercel.app", 200, &ready(LINK));
        let err = redeem(&wire, &OURS, &RedeemIds::default(), "K7QM2XPA")
            .await
            .expect_err("limited");
        assert_eq!(err.code, ErrorCode::PairRateLimited);
        assert_eq!(err.detail.as_deref(), Some("45"));
        assert_eq!(wire.asked().len(), 1);
    }

    #[tokio::test]
    async fn a_hostile_link_is_not_handed_on() {
        let wire = ScriptWire::default()
            .answer(
                "proxysvpn.com",
                200,
                &ready("https://evil.example/api/sub/abcdef"),
            )
            .answer("proxysvnovich.vercel.app", 200, &ready(LINK));
        let err = redeem(&wire, &OURS, &RedeemIds::default(), "K7QM2XPA")
            .await
            .expect_err("refused");
        assert_eq!(err.code, ErrorCode::SubInvalid);
        // The code is spent at the first name; asking the next would only
        // turn "refused" into "not found".
        assert_eq!(wire.asked().len(), 1);
    }

    #[tokio::test]
    async fn every_name_down_is_unreachable() {
        let wire = ScriptWire::default()
            .dead("proxysvpn.com")
            .answer("proxysvnovich.vercel.app", 503, "")
            .answer("proksya.com", 403, "This request was blocked")
            .dead("proksya.xyz");
        let err = redeem(&wire, &OURS, &RedeemIds::default(), "K7QM2XPA")
            .await
            .expect_err("down");
        assert_eq!(err.code, ErrorCode::SubUnreachable);
        assert_eq!(wire.asked().len(), 4);
    }

    #[tokio::test]
    async fn the_error_payload_carries_neither_code_nor_link() {
        let wire = ScriptWire::default().answer(
            "proxysvpn.com",
            200,
            &ready("http://proxysvpn.com/api/sub/secret0token"),
        );
        let err = redeem(&wire, &OURS, &RedeemIds::default(), "K7QM2XPA")
            .await
            .expect_err("refused");
        let payload = err.to_payload();
        assert!(!payload.contains("K7QM2XPA"), "{payload}");
        assert!(!payload.contains("secret0token"), "{payload}");
    }

    // ── the request id (pair-code v1.1) ────────────────────────────────────

    #[test]
    fn a_request_id_is_32_random_bytes_as_unpadded_base64url() {
        let id = RequestId::generate(os_random).expect("urandom");
        assert_eq!(id.as_str().len(), 43);
        assert!(is_valid_request_id(id.as_str()));
        let bytes = URL_SAFE_NO_PAD.decode(id.as_str()).expect("base64url");
        assert_eq!(bytes.len(), REQUEST_ID_BYTES);
    }

    #[test]
    fn the_encoding_is_the_url_safe_alphabet_without_padding() {
        // All-ones bytes use the two characters where base64url differs from
        // base64 ('_' for '/'); 32 bytes leave a partial group, which plain
        // base64 would pad with '='.
        let id = RequestId::generate(all_ones).expect("scripted");
        assert_eq!(id.as_str(), format!("{}8", "_".repeat(42)));
        assert!(is_valid_request_id(id.as_str()));
    }

    #[test]
    fn request_ids_from_the_system_do_not_repeat() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            let id = RequestId::generate(os_random).expect("urandom");
            assert!(seen.insert(id.as_str().to_string()), "a repeated id");
        }
    }

    #[test]
    fn the_contract_form_of_a_request_id() {
        for good in [
            "a".repeat(22),
            "Z".repeat(64),
            "AZaz09_-AZaz09_-AZaz09".to_string(),
        ] {
            assert!(is_valid_request_id(&good), "{good}");
        }
        for bad in [
            String::new(),
            "a".repeat(21),
            "a".repeat(65),
            format!("{}+", "a".repeat(30)),
            format!("{}/", "a".repeat(30)),
            format!("{}=", "a".repeat(42)),
            format!("{} ", "a".repeat(30)),
            format!("{}é", "a".repeat(30)),
            format!("{}\n", "a".repeat(30)),
        ] {
            assert!(!is_valid_request_id(&bad), "{bad:?}");
        }
    }

    #[test]
    fn debug_never_shows_the_id() {
        let id = RequestId::generate(os_random).expect("urandom");
        let shown = format!("{id:?} {:?}", Some(id.clone()));
        assert!(!shown.contains(id.as_str()), "{shown}");
    }

    // ── one id per code, for as long as that code is tried ────────────────

    #[test]
    fn pressing_again_with_the_same_code_keeps_the_id() {
        let ids = RedeemIds::default();
        let first = ids.for_code("K7QM2XPA").expect("id");
        let again = ids.for_code("K7QM2XPA").expect("id");
        assert_eq!(first, again);
    }

    #[test]
    fn another_code_gets_a_new_id_and_the_old_one_is_gone() {
        let ids = RedeemIds::default();
        let first = ids.for_code("K7QM2XPA").expect("id");
        let other = ids.for_code("ABCD2345").expect("id");
        assert_ne!(first, other);
        // One slot: back to the first code is a new attempt with a new id.
        let back = ids.for_code("K7QM2XPA").expect("id");
        assert_ne!(back, first);
        assert_ne!(back, other);
    }

    #[test]
    fn a_sign_in_forgets_the_id() {
        let ids = RedeemIds::default();
        let first = ids.for_code("K7QM2XPA").expect("id");
        ids.forget();
        assert!(ids.slot.lock().expect("slot").is_none());
        let next = ids.for_code("K7QM2XPA").expect("id");
        assert_ne!(first, next);
        // Forgetting an empty slot is harmless.
        ids.forget();
        ids.forget();
    }

    #[test]
    fn failed_randomness_keeps_nothing_new() {
        let ids = RedeemIds::with_source(no_randomness);
        assert!(ids.for_code("K7QM2XPA").is_err());
        assert!(ids.slot.lock().expect("slot").is_none());
    }

    #[test]
    fn a_poisoned_slot_still_answers() {
        let ids = std::sync::Arc::new(RedeemIds::default());
        let first = ids.for_code("K7QM2XPA").expect("id");
        let holder = std::sync::Arc::clone(&ids);
        let _ = std::thread::spawn(move || {
            let _guard = holder.slot.lock().expect("slot");
            panic!("poison the slot");
        })
        .join();
        assert!(ids.slot.is_poisoned());
        assert_eq!(ids.for_code("K7QM2XPA").expect("id"), first);
        ids.forget();
        assert_ne!(ids.for_code("K7QM2XPA").expect("id"), first);
    }

    // ── the id on the wire ─────────────────────────────────────────────────

    #[tokio::test]
    async fn every_name_and_every_retry_hear_the_same_request_id() {
        let wire = ScriptWire::default()
            .dead("proxysvpn.com")
            .dead("proxysvnovich.vercel.app")
            .dead("proksya.com")
            .dead("proksya.xyz");
        let ids = RedeemIds::default();
        // The same code as typed and as pasted: one normalised code.
        for raw in ["K7QM-2XPA", "k7qm 2xpa"] {
            let err = redeem(&wire, &OURS, &ids, raw).await.expect_err("down");
            assert_eq!(err.code, ErrorCode::SubUnreachable);
        }
        let sent = sent_ids(&wire);
        assert_eq!(sent.len(), 8, "four names, twice");
        assert!(sent.iter().all(|id| *id == sent[0]), "one id throughout");
    }

    #[tokio::test]
    async fn a_new_code_is_sent_with_a_new_request_id() {
        let wire = ScriptWire::default().answer("proxysvpn.com", 404, r#"{"error":"not_found"}"#);
        let ids = RedeemIds::default();
        for raw in ["K7QM2XPA", "ABCD2345", "K7QM2XPA"] {
            let err = redeem(&wire, &OURS, &ids, raw).await.expect_err("404");
            assert_eq!(err.code, ErrorCode::PairCodeNotFound);
        }
        let sent = sent_ids(&wire);
        assert_eq!(sent.len(), 3);
        assert_ne!(sent[0], sent[1]);
        assert_ne!(sent[1], sent[2]);
        assert_ne!(sent[0], sent[2], "a changed code was a new attempt");
    }

    #[tokio::test]
    async fn no_answer_from_any_name_is_the_network_not_a_spent_code() {
        // Pure transport failures on every name: the person must read "could
        // not reach the service" and may press again, never "code not valid".
        let wire = ScriptWire::default()
            .dead("proxysvpn.com")
            .dead("proxysvnovich.vercel.app")
            .dead("proksya.com")
            .dead("proksya.xyz");
        let ids = RedeemIds::default();
        let err = redeem(&wire, &OURS, &ids, "K7QM2XPA")
            .await
            .expect_err("down");
        assert_eq!(err.code, ErrorCode::SubUnreachable);
        assert_ne!(err.code, ErrorCode::PairCodeNotFound);
        // The id stays for the next press.
        let kept = ids.slot.lock().expect("slot").clone();
        assert_eq!(kept.map(|(code, _)| code).as_deref(), Some("K7QM2XPA"));
    }

    #[tokio::test]
    async fn without_randomness_the_redeem_is_v1s() {
        let wire = ScriptWire::default().answer("proxysvpn.com", 200, &ready(LINK));
        let ids = RedeemIds::with_source(no_randomness);
        let link = redeem(&wire, &OURS, &ids, "K7QM2XPA")
            .await
            .expect("v1 still works");
        assert_eq!(link, LINK);
        assert_eq!(
            wire.bodies(),
            vec![Some(serde_json::json!({ "code": "K7QM2XPA" }))]
        );
    }

    #[test]
    fn the_body_has_exactly_the_contracts_fields() {
        let id = RequestId::generate(all_ones).expect("scripted");
        let body = redeem_body("K7QM2XPA", Some(&id));
        let fields: Vec<&String> = body.as_object().expect("object").keys().collect();
        assert_eq!(fields.len(), 2);
        assert_eq!(body["code"], "K7QM2XPA");
        assert_eq!(body["requestId"], id.as_str());
        assert_eq!(
            redeem_body("K7QM2XPA", None),
            serde_json::json!({ "code": "K7QM2XPA" })
        );
    }

    // ── against a service that keeps contract v1.1 ─────────────────────────

    /// The service as the contract describes it, in memory: every name
    /// reaches the same one. A name in `lose` delivers the request and loses
    /// the answer on the way back.
    #[derive(Default)]
    struct V11Service {
        /// paircode:<CODE> -> the link it was issued for.
        codes: Mutex<std::collections::HashMap<String, String>>,
        /// paircode:done:<CODE> -> (link, sha256 hex of the request id).
        done: Mutex<std::collections::HashMap<String, (String, String)>>,
        lose: Mutex<std::collections::HashSet<String>>,
        calls: Mutex<Vec<String>>,
    }

    impl V11Service {
        fn issued(code: &str, link: &str) -> Self {
            let service = Self::default();
            service
                .codes
                .lock()
                .expect("codes")
                .insert(code.to_string(), link.to_string());
            service
        }

        fn losing(self, host: &str) -> Self {
            self.lose.lock().expect("lose").insert(host.to_string());
            self
        }

        /// Answers reach the app again; the calls so far are forgotten.
        fn network_back(&self) {
            self.lose.lock().expect("lose").clear();
            self.calls.lock().expect("calls").clear();
        }

        fn hash(id: &str) -> String {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(id.as_bytes()))
        }

        fn handle(&self, json: Option<&serde_json::Value>) -> SiteReply {
            let field = |name: &str| {
                json.and_then(|v| v.get(name))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            };
            let not_found = reply(404, r#"{"error":"not_found"}"#);
            let Some(code) = field("code").and_then(|c| normalise_pair_code(&c)) else {
                return reply(400, r#"{"error":"bad_code"}"#);
            };
            let rid = field("requestId").filter(|id| is_valid_request_id(id));
            // GETDEL paircode:<CODE>
            let taken = self.codes.lock().expect("codes").remove(&code);
            if let Some(link) = taken {
                if let Some(rid) = rid {
                    self.done
                        .lock()
                        .expect("done")
                        .insert(code, (link.clone(), Self::hash(&rid)));
                }
                return reply(200, &ready(&link));
            }
            let Some(rid) = rid else { return not_found };
            match self.done.lock().expect("done").get(&code) {
                Some((link, hash)) if *hash == Self::hash(&rid) => reply(200, &ready(link)),
                _ => not_found,
            }
        }
    }

    impl SiteWire for V11Service {
        async fn send(
            &self,
            _method: Method,
            url: &str,
            json: Option<&serde_json::Value>,
        ) -> Result<SiteReply, String> {
            let host = url
                .trim_start_matches("https://")
                .split('/')
                .next()
                .unwrap_or_default()
                .to_string();
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(host.clone());
            }
            let answer = self.handle(json);
            if self
                .lose
                .lock()
                .map(|lose| lose.contains(&host))
                .unwrap_or(false)
            {
                return Err("error decoding response body: connection reset".into());
            }
            Ok(answer)
        }
    }

    #[tokio::test]
    async fn an_answer_lost_on_the_way_is_repeated_for_the_same_request_id() {
        // v1 pinned the opposite: the first name spent the code, its answer
        // never arrived, and the next name said "not found" about a code
        // typed correctly. With the request id the next name repeats it.
        let service = V11Service::issued("K7QM2XPA", LINK).losing("proxysvpn.com");
        let ids = RedeemIds::default();
        let link = redeem(&service, &OURS, &ids, "K7QM-2XPA")
            .await
            .expect("repeated by the next name");
        assert_eq!(link, LINK);
        assert_eq!(
            *service.calls.lock().expect("calls"),
            vec!["proxysvpn.com", "proxysvnovich.vercel.app"]
        );
        assert!(
            service.codes.lock().expect("codes").is_empty(),
            "spent once"
        );
    }

    #[tokio::test]
    async fn answers_lost_on_every_name_are_repeated_on_the_next_press() {
        let mut service = V11Service::issued("K7QM2XPA", LINK);
        for host in OURS {
            service = service.losing(host);
        }
        let ids = RedeemIds::default();
        let err = redeem(&service, &OURS, &ids, "K7QM2XPA")
            .await
            .expect_err("no answer came back");
        assert_eq!(err.code, ErrorCode::SubUnreachable, "the network line");

        // The network is back; the person presses "Подключить" again.
        service.network_back();
        let link = redeem(&service, &OURS, &ids, "K7QM2XPA")
            .await
            .expect("repeated");
        assert_eq!(link, LINK);
        assert_eq!(service.calls.lock().expect("calls").len(), 1);
    }

    #[tokio::test]
    async fn another_requester_with_the_same_code_hears_not_found() {
        let service = V11Service::issued("K7QM2XPA", LINK);
        let mine = RedeemIds::default();
        redeem(&service, &OURS, &mine, "K7QM2XPA")
            .await
            .expect("mine");
        // Someone else who saw the code: a different id, the v1 answer.
        let theirs = RedeemIds::default();
        let err = redeem(&service, &OURS, &theirs, "K7QM2XPA")
            .await
            .expect_err("not theirs");
        assert_eq!(err.code, ErrorCode::PairCodeNotFound);
        // And this app after its sign-in forgot the id: nothing to repeat.
        mine.forget();
        let err = redeem(&service, &OURS, &mine, "K7QM2XPA")
            .await
            .expect_err("forgotten");
        assert_eq!(err.code, ErrorCode::PairCodeNotFound);
    }

    #[tokio::test]
    async fn the_same_requester_hears_the_same_link_again() {
        let service = V11Service::issued("K7QM2XPA", LINK);
        let ids = RedeemIds::default();
        for _ in 0..3 {
            let link = redeem(&service, &OURS, &ids, "K7QM2XPA")
                .await
                .expect("ready, then repeated");
            assert_eq!(link, LINK);
        }
    }
}
