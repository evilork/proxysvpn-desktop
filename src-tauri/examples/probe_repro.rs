//! Разведение «виноват клиент» и «виноват путь».
//!
//! Берёт ТОТ ЖЕ reqwest и ТО ЖЕ построение клиента, что проба приложения, и
//! ходит по тем же адресам - но снаружи, отдельным процессом, где ничего не
//! перезапускается и нечему мешать. Рядом ходит curl. Если оба ведут себя
//! одинаково, дело в пути; если расходятся - в клиенте.
//!
//! Запуск: cargo run --example probe_repro -- [адрес ...]

use std::time::{Duration, Instant};

const LADDER: &[&str] = &[
    // Без TLS вовсе: показывает цену ОДНОГО соединения через туннель.
    "http://example.com/",
    "http://proxysvpn.store/api/exit-ip",
    // С TLS: разница с парой выше и есть цена рукопожатия.
    "https://proxysvpn.store/api/exit-ip",
    "https://proksya.com/api/tools/whoami",
    // Единственный адрес с записью AAAA - проверяем, он ли один виснет.
    "https://example.com/",
];

fn classify(e: &reqwest::Error) -> String {
    let mut tags = Vec::new();
    if e.is_connect() {
        tags.push("connect");
    }
    if e.is_timeout() {
        tags.push("timeout");
    }
    if e.is_request() {
        tags.push("request");
    }
    if e.is_body() {
        tags.push("body");
    }
    if e.is_decode() {
        tags.push("decode");
    }
    let mut chain = Vec::new();
    let mut src: Option<&dyn std::error::Error> = std::error::Error::source(e);
    while let Some(s) = src {
        chain.push(s.to_string());
        src = std::error::Error::source(s);
    }
    format!("[{}] {} | причины: {}", tags.join("+"), e, chain.join(" <- "))
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let urls: Vec<String> = if args.is_empty() {
        LADDER.iter().map(|s| s.to_string()).collect()
    } else {
        args
    };

    let total = Duration::from_secs(8);
    let connect = total * 3 / 4;

    // Ровно то же построение, что в probe.rs.
    let client = reqwest::Client::builder()
        .no_proxy()
        .pool_max_idle_per_host(0)
        .timeout(total)
        .connect_timeout(connect)
        .user_agent("ProxysVPN-0.1.0 (probe)")
        .build()
        .expect("клиент собрался");

    println!("бюджеты: общий {total:?}, на установку {connect:?}\n");

    for round in 1..=3 {
        println!("── круг {round} ──");
        for url in &urls {
            // Разрешение имени ОТДЕЛЬНО от запроса. Внутри reqwest оно спрятано
            // в бюджет на установку, и отличить «имя не разрешается» от «сокет
            // не открывается» по его ошибке невозможно.
            if let Some(host) = url.split('/').nth(2) {
                let hostport = if host.contains(':') {
                    host.to_string()
                } else if url.starts_with("https") {
                    format!("{host}:443")
                } else {
                    format!("{host}:80")
                };
                let t = Instant::now();
                // Владеющая строка: заимствование не переживёт печать ниже.
                match tokio::net::lookup_host(hostport.clone()).await {
                    Ok(addrs) => {
                        let list: Vec<String> = addrs.map(|a| a.to_string()).collect();
                        println!("    имя {hostport}: {:?} за {:?}", list, t.elapsed());
                    }
                    Err(e) => println!("    имя {hostport}: НЕ РАЗРЕШИЛОСЬ за {:?}: {e}", t.elapsed()),
                }
            }
            let started = Instant::now();
            match client.get(url).send().await {
                Ok(r) => {
                    let status = r.status();
                    let body = r.bytes().await.map(|b| b.len()).unwrap_or(0);
                    println!(
                        "  {:<48} {} за {:?}, тело {} Б",
                        url,
                        status,
                        started.elapsed(),
                        body
                    );
                }
                Err(e) => println!("  {:<48} ОШИБКА за {:?}: {}", url, started.elapsed(), classify(&e)),
            }
        }
        println!();
    }
}
