// src-tauri/src/site_ladder.rs
//
// The ladder of site names for the pairing endpoints: ask each name in turn
// until one gives an answer that settles the question.
//
// Split out of lib.rs so the walk can be tested without a network. The wire
// is a trait - `HttpWire` in the app, a script in tests - and what an answer
// MEANS belongs to the caller's judge. For `/api/pair/new` anything but a 2xx
// means "try the next name". For `/api/pair/code` a 404 carrying the
// service's own body is the final word: every name reaches the same service,
// and asking a second one would only spend another of the person's attempts.
//
// Sequential on purpose: four simultaneous handshakes to four of our names is
// a pattern worth recognising on a filtering network, and the first name
// almost always works.
//
// Never logged here: the URL (a pairing path carries a token) or a body.

use std::future::Future;

use crate::errors::{AppError, ErrorCode};
use crate::logger;

/// Largest answer read from a pairing endpoint. The real ones are a few
/// hundred bytes; a captive portal's login page is not read into memory whole.
pub(crate) const MAX_REPLY_BYTES: usize = 64 * 1024;

/// One site's answer, whatever its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SiteReply {
    pub status: u16,
    /// `Retry-After` as the site sent it, when it sent one.
    pub retry_after: Option<String>,
    pub body: String,
}

impl SiteReply {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// What the caller makes of one site's answer.
#[derive(Debug)]
pub(crate) enum Verdict<T> {
    /// The question is settled in our favour.
    Done(T),
    /// The question is settled against us; another name would say the same.
    Stop(AppError),
    /// This name did not answer the question; try the next one.
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
}

/// The wire under the ladder.
pub(crate) trait SiteWire {
    /// Send one request. `Err` is the transport failing - no answer at all -
    /// and its text must not contain the URL.
    fn send(
        &self,
        method: Method,
        url: &str,
        json: Option<&serde_json::Value>,
    ) -> impl Future<Output = Result<SiteReply, String>> + Send;
}

/// Walk `hosts` in order with the same request until the judge settles it.
///
/// Every name that leaves the question open is written to the log by name
/// and status; when none settles it the answer is `SubUnreachable` - almost
/// always the person's own network, which is why the ladder exists.
pub(crate) async fn walk<W, T, J>(
    wire: &W,
    hosts: &[&str],
    method: Method,
    path: &str,
    json: Option<&serde_json::Value>,
    mut judge: J,
) -> Result<T, AppError>
where
    W: SiteWire,
    J: FnMut(&str, SiteReply) -> Verdict<T>,
{
    for host in hosts {
        let url = format!("https://{host}{path}");
        match wire.send(method, &url, json).await {
            Ok(reply) => {
                let status = reply.status;
                match judge(host, reply) {
                    Verdict::Done(value) => return Ok(value),
                    Verdict::Stop(err) => return Err(err),
                    Verdict::Next => {
                        logger::log("warn", "pair", &format!("{host} answered {status}"));
                    }
                }
            }
            Err(e) => logger::log("warn", "pair", &format!("{host} unreachable: {e}")),
        }
    }
    Err(AppError::new(ErrorCode::SubUnreachable))
}

/// The real wire: HTTPS with the app's client (User-Agent, timeout).
pub(crate) struct HttpWire {
    client: reqwest::Client,
}

impl HttpWire {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl SiteWire for HttpWire {
    async fn send(
        &self,
        method: Method,
        url: &str,
        json: Option<&serde_json::Value>,
    ) -> Result<SiteReply, String> {
        let mut request = match method {
            Method::Get => self.client.get(url),
            Method::Post => self.client.post(url),
        };
        if let Some(value) = json {
            // reqwest is built without its `json` feature; two lines do the same.
            let bytes = serde_json::to_vec(value).map_err(|e| format!("body: {e}"))?;
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes);
        }
        let mut response = request.send().await.map_err(describe)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_string());
        let body = read_capped(&mut response, MAX_REPLY_BYTES).await?;
        Ok(SiteReply {
            status,
            retry_after,
            body,
        })
    }
}

/// The body, refused past `cap` bytes rather than cut: half a JSON object is
/// not an answer either.
async fn read_capped(response: &mut reqwest::Response, cap: usize) -> Result<String, String> {
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(describe)? {
        if buf.len() + chunk.len() > cap {
            return Err(format!("answer larger than {cap} bytes"));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// A transport failure in words, with its causes ("dns error", "timed out")
/// and without the URL: a pairing path carries a token.
fn describe(err: reqwest::Error) -> String {
    let err = err.without_url();
    let mut text = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A scripted wire: one answer per host, every request recorded.
    #[derive(Default)]
    pub(crate) struct ScriptWire {
        pub answers: HashMap<String, Result<SiteReply, String>>,
        pub calls: Mutex<Vec<(Method, String, Option<serde_json::Value>)>>,
    }

    impl ScriptWire {
        pub fn answer(mut self, host: &str, status: u16, body: &str) -> Self {
            self.answers.insert(
                host.to_string(),
                Ok(SiteReply {
                    status,
                    retry_after: None,
                    body: body.to_string(),
                }),
            );
            self
        }

        pub fn answer_with_retry(mut self, host: &str, status: u16, retry_after: &str) -> Self {
            self.answers.insert(
                host.to_string(),
                Ok(SiteReply {
                    status,
                    retry_after: Some(retry_after.to_string()),
                    body: r#"{"error":"rate_limited"}"#.to_string(),
                }),
            );
            self
        }

        pub fn dead(mut self, host: &str) -> Self {
            self.answers.insert(
                host.to_string(),
                Err("error sending request: timed out".into()),
            );
            self
        }

        /// Hosts asked, in order.
        pub fn asked(&self) -> Vec<String> {
            self.calls
                .lock()
                .map(|calls| {
                    calls
                        .iter()
                        .map(|(_, url, _)| {
                            url.trim_start_matches("https://")
                                .split('/')
                                .next()
                                .unwrap_or_default()
                                .to_string()
                        })
                        .collect()
                })
                .unwrap_or_default()
        }

        pub fn bodies(&self) -> Vec<Option<serde_json::Value>> {
            self.calls
                .lock()
                .map(|calls| calls.iter().map(|(_, _, json)| json.clone()).collect())
                .unwrap_or_default()
        }
    }

    impl SiteWire for ScriptWire {
        async fn send(
            &self,
            method: Method,
            url: &str,
            json: Option<&serde_json::Value>,
        ) -> Result<SiteReply, String> {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push((method, url.to_string(), json.cloned()));
            }
            let host = url
                .trim_start_matches("https://")
                .split('/')
                .next()
                .unwrap_or_default();
            self.answers
                .get(host)
                .cloned()
                .unwrap_or_else(|| Err("no route to host".into()))
        }
    }

    const HOSTS: [&str; 3] = ["a.example", "b.example", "c.example"];

    fn success_only(host: &str, reply: SiteReply) -> Verdict<(String, String)> {
        if reply.is_success() {
            Verdict::Done((host.to_string(), reply.body))
        } else {
            Verdict::Next
        }
    }

    #[tokio::test]
    async fn the_first_name_that_answers_wins_and_the_rest_are_not_asked() {
        let wire = ScriptWire::default()
            .dead("a.example")
            .answer("b.example", 200, "ok")
            .answer("c.example", 200, "never");
        let got = walk(
            &wire,
            &HOSTS,
            Method::Post,
            "/api/pair/new",
            None,
            success_only,
        )
        .await
        .expect("b answers");
        assert_eq!(got, ("b.example".to_string(), "ok".to_string()));
        assert_eq!(wire.asked(), vec!["a.example", "b.example"]);
    }

    #[tokio::test]
    async fn a_name_that_answers_badly_is_passed_over() {
        // A firewall 403 and a 503 are this name's trouble, not the service's.
        let wire = ScriptWire::default()
            .answer("a.example", 403, "This request was blocked")
            .answer("b.example", 503, "")
            .answer("c.example", 200, "ok");
        let got = walk(&wire, &HOSTS, Method::Post, "/x", None, success_only)
            .await
            .expect("c answers");
        assert_eq!(got.0, "c.example");
        assert_eq!(wire.asked().len(), 3);
    }

    #[tokio::test]
    async fn a_stop_ends_the_walk_at_once() {
        let wire =
            ScriptWire::default()
                .answer("a.example", 404, "")
                .answer("b.example", 200, "ok");
        let err = walk(
            &wire,
            &HOSTS,
            Method::Get,
            "/x",
            None,
            |_, reply: SiteReply| {
                if reply.status == 404 {
                    Verdict::<()>::Stop(AppError::new(ErrorCode::SubEmpty))
                } else {
                    Verdict::Next
                }
            },
        )
        .await
        .expect_err("404 is final");
        assert_eq!(err.code, ErrorCode::SubEmpty);
        assert_eq!(wire.asked(), vec!["a.example"]);
    }

    #[tokio::test]
    async fn no_answer_anywhere_is_unreachable() {
        let wire = ScriptWire::default()
            .dead("a.example")
            .answer("b.example", 500, "")
            .dead("c.example");
        let err = walk(&wire, &HOSTS, Method::Post, "/x", None, success_only)
            .await
            .expect_err("nobody answered");
        assert_eq!(err.code, ErrorCode::SubUnreachable);
        assert_eq!(wire.asked().len(), 3);
    }

    #[tokio::test]
    async fn an_empty_ladder_is_unreachable_not_a_panic() {
        let wire = ScriptWire::default();
        let err = walk(&wire, &[], Method::Post, "/x", None, success_only)
            .await
            .expect_err("no names");
        assert_eq!(err.code, ErrorCode::SubUnreachable);
    }

    #[tokio::test]
    async fn every_name_gets_the_same_method_path_and_body() {
        let wire = ScriptWire::default()
            .dead("a.example")
            .dead("b.example")
            .dead("c.example");
        let body = serde_json::json!({ "k": "v" });
        let _ = walk(
            &wire,
            &HOSTS,
            Method::Post,
            "/api/p",
            Some(&body),
            success_only,
        )
        .await;
        let calls = wire.calls.lock().expect("calls");
        assert_eq!(calls.len(), 3);
        for (method, url, json) in calls.iter() {
            assert_eq!(*method, Method::Post);
            assert!(
                url.starts_with("https://") && url.ends_with("/api/p"),
                "{url}"
            );
            assert_eq!(json.as_ref(), Some(&body));
        }
    }

    // ── the real wire, against a one-shot server on loopback ───────────────

    /// Serve exactly one raw HTTP response on loopback; hand back the request.
    async fn one_shot(response: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return String::new();
            };
            let mut buf = vec![0u8; 8192];
            let mut seen = Vec::new();
            // Read until the headers and the declared body are in.
            while let Ok(n) = socket.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let declared = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if seen.len() >= end + 4 + declared {
                        break;
                    }
                }
            }
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
            String::from_utf8_lossy(&seen).to_string()
        });
        (format!("http://{addr}"), server)
    }

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("client")
    }

    #[tokio::test]
    async fn the_real_wire_reads_status_retry_after_and_body() {
        let (base, server) = one_shot(
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 42\r\nContent-Type: application/json\r\nContent-Length: 24\r\nConnection: close\r\n\r\n{\"error\":\"rate_limited\"}",
        )
        .await;
        let wire = HttpWire::new(test_client());
        let body = serde_json::json!({ "code": "K7QM2XPA" });
        let reply = wire
            .send(Method::Post, &format!("{base}/api/pair/code"), Some(&body))
            .await
            .expect("an answer");
        assert_eq!(reply.status, 429);
        assert_eq!(reply.retry_after.as_deref(), Some("42"));
        assert_eq!(reply.body, r#"{"error":"rate_limited"}"#);

        let request = server.await.expect("server");
        assert!(request.starts_with("POST /api/pair/code "), "{request}");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/json"),
            "{request}"
        );
        assert!(request.ends_with(r#"{"code":"K7QM2XPA"}"#), "{request}");
    }

    #[tokio::test]
    async fn the_real_wire_refuses_an_oversized_answer() {
        // A login page of a hotel network, say: it must not be read whole.
        static BIG: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let page = BIG.get_or_init(|| {
            let body = "x".repeat(MAX_REPLY_BYTES + 1);
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        });
        let (base, _server) = one_shot(page.as_str()).await;
        let err = HttpWire::new(test_client())
            .send(Method::Get, &format!("{base}/api/pair/x"), None)
            .await
            .expect_err("too large");
        assert!(err.contains("larger than"), "{err}");
    }

    #[tokio::test]
    async fn a_transport_failure_never_carries_the_url() {
        // Nothing listens on port 1: the cheapest honest "network dropped us".
        let wire = HttpWire::new(
            reqwest::Client::builder()
                .timeout(Duration::from_millis(800))
                .build()
                .expect("client"),
        );
        let err = wire
            .send(
                Method::Get,
                "http://127.0.0.1:1/api/pair/0123456789abcdef0123456789abcdef",
                None,
            )
            .await
            .expect_err("nothing listens on port 1");
        assert!(!err.contains("0123456789abcdef"), "{err}");
        assert!(!err.contains("/api/pair"), "{err}");
    }
}
