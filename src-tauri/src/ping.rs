// src-tauri/src/ping.rs
// TCP-ping to the VPN server itself (not through the tunnel).
// Measures latency from the user's machine to the VPN edge — same semantic
// as classic ICMP ping, but over TCP so it works through firewalls and
// doesn't need raw-socket privileges.

use anyhow::{anyhow, Context, Result};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TIMEOUT_MS: u64 = 2000;

#[derive(Default)]
struct PingTarget {
    host: String,
    port: u16,
    /// Cached resolved address — skip DNS on every probe.
    addr: Option<SocketAddr>,
}

static TARGET: Mutex<Option<PingTarget>> = Mutex::new(None);

/// Called from lib.rs on successful connect.
pub fn set_target(host: String, port: u16) {
    let mut guard = TARGET.lock().unwrap();
    *guard = Some(PingTarget { host, port, addr: None });
}

pub fn clear_target() {
    *TARGET.lock().unwrap() = None;
}

fn resolve(host: &str, port: u16) -> Result<SocketAddr> {
    format!("{}:{}", host, port)
        .to_socket_addrs()
        .with_context(|| format!("resolve {}:{}", host, port))?
        .next()
        .ok_or_else(|| anyhow!("no addresses for {}:{}", host, port))
}

fn one_probe(addr: SocketAddr) -> Result<u64> {
    let start = Instant::now();
    let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(TIMEOUT_MS))
        .with_context(|| format!("connect {}", addr))?;
    let elapsed = start.elapsed();
    drop(stream);
    Ok(elapsed.as_micros() as u64)
}

pub fn tcp_ping() -> Result<u32> {
    let (host, port) = {
        let guard = TARGET.lock().unwrap();
        let t = guard.as_ref().ok_or_else(|| anyhow!("no target set"))?;
        (t.host.clone(), t.port)
    };

    let addr = {
        let mut guard = TARGET.lock().unwrap();
        let t = guard.as_mut().unwrap();
        if let Some(a) = t.addr {
            a
        } else {
            let a = resolve(&host, port)?;
            t.addr = Some(a);
            a
        }
    };

    let mut samples: Vec<u64> = Vec::with_capacity(3);
    for _ in 0..3 {
        if let Ok(us) = one_probe(addr) {
            samples.push(us);
        }
    }
    if samples.is_empty() {
        anyhow::bail!("all probes failed");
    }
    let best_us = *samples.iter().min().unwrap();
    let ms = ((best_us as f64) / 1000.0).round() as u32;
    Ok(ms.max(1))
}

pub async fn tcp_ping_async() -> Result<u32> {
    tokio::task::spawn_blocking(tcp_ping)
        .await
        .context("ping task panicked")?
}
