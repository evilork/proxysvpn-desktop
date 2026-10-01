// src-tauri/src/pair_code.rs
//
// Pair code v1, the app's half.
//
// The cabinet and the bot show an eight-character code ("Код для
// приложения"); the person types it on the sign-in screen and the service
// answers with the subscription link of the device it was issued for - once,
// within ten minutes. It covers what the QR cannot: the cabinet and the app
// on the same phone, with nothing to point a camera at.
//
// Contract pair-code v1, shared with the service (frontend /api/pair/code)
// and the window (src/pairCode.ts):
//   • eight characters of 23456789ABCDEFGHJKMNPQRSTUVWXYZ (no 0 1 I L O),
//     shown as XXXX-XXXX; input is uppercased and loses whitespace and
//     dashes before it is judged;
//   • POST /api/pair/code {"code": "K7QM2XPA"}
//       200 {"status":"ready","subUrl":"https://..."}
//       400 {"error":"bad_code"}
//       404 {"error":"not_found"}    wrong, expired or used: one answer
//       429 {"error":"rate_limited"} with Retry-After
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
// the link; pair-code v1 does not.
//
// Never logged: the code, the link, the token in it.

use url::{Host, Url};

use crate::errors::{AppError, ErrorCode};
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

/// Redeem a typed code over the ladder `hosts`; the link it delivered, or why
/// not. The link is checked against the same `hosts`.
pub(crate) async fn redeem<W: SiteWire>(
    wire: &W,
    hosts: &[&str],
    raw_code: &str,
) -> Result<String, AppError> {
    let code =
        normalise_pair_code(raw_code).ok_or_else(|| AppError::new(ErrorCode::PairCodeMalformed))?;
    let body = serde_json::json!({ "code": code });
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
        let link = redeem(&wire, &OURS, "k7qm 2xpa").await.expect("redeemed");
        assert_eq!(link, LINK);
        assert_eq!(
            wire.asked(),
            vec!["proxysvpn.com", "proxysvnovich.vercel.app"]
        );
        // Every name was sent the normalised code in a JSON body, never in the path.
        for body in wire.bodies() {
            assert_eq!(body, Some(serde_json::json!({ "code": "K7QM2XPA" })));
        }
        let calls = wire.calls.lock().expect("calls");
        assert!(calls
            .iter()
            .all(|(method, url, _)| *method == Method::Post && url.ends_with(REDEEM_PATH)));
    }

    #[tokio::test]
    async fn a_malformed_code_is_never_sent() {
        let wire = ScriptWire::default().answer("proxysvpn.com", 200, &ready(LINK));
        let err = redeem(&wire, &OURS, "K7QM-2XP0")
            .await
            .expect_err("0 is not in the alphabet");
        assert_eq!(err.code, ErrorCode::PairCodeMalformed);
        assert!(wire.asked().is_empty());
    }

    #[tokio::test]
    async fn not_found_stops_the_ladder_at_the_first_name() {
        let wire = ScriptWire::default()
            .answer("proxysvpn.com", 404, r#"{"error":"not_found"}"#)
            .answer("proxysvnovich.vercel.app", 200, &ready(LINK));
        let err = redeem(&wire, &OURS, "K7QM2XPA")
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
        let err = redeem(&wire, &OURS, "K7QM2XPA").await.expect_err("limited");
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
        let err = redeem(&wire, &OURS, "K7QM2XPA").await.expect_err("refused");
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
        let err = redeem(&wire, &OURS, "K7QM2XPA").await.expect_err("down");
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
        let err = redeem(&wire, &OURS, "K7QM2XPA").await.expect_err("refused");
        let payload = err.to_payload();
        assert!(!payload.contains("K7QM2XPA"), "{payload}");
        assert!(!payload.contains("secret0token"), "{payload}");
    }
}
