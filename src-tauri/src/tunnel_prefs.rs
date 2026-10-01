// src/tunnel_prefs.rs
//
// Настройки туннеля, которые человек меняет руками.
//
// Хранятся рядом со ссылкой подписки и тем же способом (см. link_paths в
// lib.rs): сначала общесистемная папка, потом домашняя. Приложение работает от
// root, поэтому пишется первая доступная.
//
// Правило для всего файла: настройка попадает сюда, только если она
// ДЕЙСТВИТЕЛЬНО что-то меняет в работе. Тумблер, который ничего не делает,
// хуже отсутствующего - он врёт. Поэтому мультиплексирования здесь нет: наши
// узлы идут с `flow=xtls-rprx-vision`, а он с мультиплексированием
// несовместим, и включение не дало бы ничего. Окно говорит об этом прямо.

use serde::{Deserialize, Serialize};

/// Какие адреса спрашивать у резолвера.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum IpKind {
    /// Только IPv4. По умолчанию, и на то есть причина: IPv6-выхода нет ни у
    /// одного нашего узла (проверено по флоту 07.09.2026), поэтому запрос
    /// AAAA - это ожидание впустую на каждом имени.
    #[default]
    Ipv4,
    /// Только IPv6. Осмысленно лишь там, где узел его умеет.
    Ipv6,
    /// Спрашивать оба и брать что ответит.
    Both,
}

impl IpKind {
    /// What the engine may actually use on this platform.
    ///
    /// Desktop: always IPv4. No desktop tunnel carries IPv6 yet — macOS and
    /// Linux install only 0.0.0.0/1 and 128.0.0.0/1, Windows leaves IPv6
    /// alone — while the system resolver on macOS and Linux asks xray. With
    /// "IPv6" or "Both" xray handed out AAAA records, Happy Eyeballs picked
    /// IPv6 on a dual-stack network, and nearly all traffic left through the
    /// physical interface while the shield stayed green. A choice stored by an
    /// earlier version is therefore ignored here rather than rewritten, so it
    /// comes back the day the tunnel captures IPv6.
    pub fn effective(self) -> Self {
        if cfg!(desktop) {
            Self::Ipv4
        } else {
            self
        }
    }

    /// Значение для `dns.queryStrategy` в конфигурации xray.
    pub fn query_strategy(self) -> &'static str {
        match self {
            Self::Ipv4 => "UseIPv4",
            Self::Ipv6 => "UseIPv6",
            Self::Both => "UseIP",
        }
    }
}

/// Какой транспорт брать у локации (TunnelScreen «Способ подключения»).
///
/// Осмысленно только для VLESS: у Hysteria2 транспорта на выбор нет, и эта
/// настройка его не касается вовсе - ни в списке, ни при выборе узла.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TransportPref {
    /// Первый кандидат, который умеет этот движок - сегодняшнее поведение,
    /// без разбора протокола.
    #[default]
    Auto,
    /// Только XHTTP (в приложении - Watafast). Локация, где XHTTP нет или его
    /// не умеет движок, не пропадает из списка - берётся то, что есть, и это
    /// видно в её собственной пометке (manifest.rs::build_location).
    XhttpOnly,
    /// Только Vision (`flow=xtls-rprx-vision`), тот же принцип запасного
    /// варианта.
    VisionOnly,
}

impl TransportPref {
    /// Слово кандидата манифеста (`CandidateWire.protocol`), которое считать
    /// предпочитаемым. `None` для `Auto` - разбирать нечего, любой кандидат
    /// в своём порядке годится, как и до появления этой настройки.
    pub fn candidate_protocol(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::XhttpOnly => Some("xhttp"),
            Self::VisionOnly => Some("vision"),
        }
    }

    /// Имя для пометки локации, когда выбранного транспорта не нашлось и
    /// пришлось взять то, что есть («XHTTP недоступен»). Пустое для `Auto`,
    /// где такой пометки не бывает вовсе - падать назад не с чего.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "",
            Self::XhttpOnly => "XHTTP",
            Self::VisionOnly => "Vision",
        }
    }
}

/// Чей резолвер спрашивает xray.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DnsChoice {
    /// Свой, через узел: DoH к 1.1.1.1 с запасным TCP к 8.8.8.8.
    ///
    /// По умолчанию, и это не вкусовщина. Без собственного резолвера xray
    /// спрашивает системный, запрос уходит в маршрут по умолчанию - то есть
    /// обратно в туннель, к самому xray, - и разрывается только по таймеру.
    /// Замер 22.09.2026: 1,2 секунды на КАЖДОЕ новое соединение.
    #[default]
    Internal,
    /// Системный резолвер машины. Оставлен ради случая, когда человеку нужен
    /// именно домашний DNS (корпоративные имена, локальные зоны).
    System,
    /// Свой адрес, введённый руками.
    Custom,
}

/// Настройки туннеля.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TunnelPrefs {
    /// Дробить первый пакет рукопожатия TLS.
    ///
    /// Помогает там, где DPI узнаёт запрещённое имя в ClientHello: имя
    /// оказывается разрезанным между пакетами, и простая проверка по образцу
    /// его не находит. Стоит лишних пакетов на каждое соединение, поэтому по
    /// умолчанию выключено.
    pub fragment: bool,
    pub ip_kind: IpKind,
    pub dns: DnsChoice,
    /// Адрес резолвера для `DnsChoice::Custom`. Пустая строка означает, что
    /// человек выбрал «свой», но ещё ничего не ввёл, - тогда работает
    /// внутренний, и окно об этом говорит.
    pub custom_dns: String,
    /// «Способ подключения». См. `TransportPref`.
    pub transport: TransportPref,
    /// «Свои правила» → «Всегда напрямую»: домены в форме xray
    /// (`domain:example.com`, `full:sub.example.com`), уже проверенные
    /// `parse_custom_rules` - здесь только то, что прошло проверку. Идут в
    /// маршрутизацию ПЕРЕД профилем подписки (subscription.rs).
    pub direct_domains: Vec<String>,
    /// «Свои правила» → «Всегда через VPN», та же форма и то же место в
    /// маршрутизации, но с исходящим `proxy`.
    pub proxy_domains: Vec<String>,
}

impl Default for TunnelPrefs {
    fn default() -> Self {
        Self {
            // Всё по умолчанию повторяет поведение ДО появления этого файла:
            // включение настроек не должно ничего менять само по себе.
            fragment: false,
            ip_kind: IpKind::Ipv4,
            dns: DnsChoice::Internal,
            custom_dns: String::new(),
            transport: TransportPref::Auto,
            direct_domains: Vec::new(),
            proxy_domains: Vec::new(),
        }
    }
}

impl TunnelPrefs {
    /// Адреса резолверов для `dns.servers` в конфигурации xray.
    ///
    /// Транспорт обязан быть TCP-совместимым: запросы идут через `proxy`, а
    /// Vision несёт только TCP. Голый UDP-адрес здесь означает мёртвые первые
    /// секунды после подъёма туннеля - на этом уже обожглись 22.09.2026.
    pub fn dns_servers(&self) -> Vec<String> {
        match self.dns {
            DnsChoice::Internal => vec![
                "https://1.1.1.1/dns-query".into(),
                "tcp://8.8.8.8".into(),
            ],
            DnsChoice::System => vec!["localhost".into()],
            DnsChoice::Custom => {
                let own = self.custom_dns.trim();
                if own.is_empty() {
                    // Человек выбрал «свой», но не ввёл. Молча остаться без
                    // резолвера нельзя - это мёртвый туннель.
                    return Self::default().dns_servers();
                }
                // Голый адрес без схемы получает tcp://: см. выше, почему не
                // UDP. Со схемой - как ввели.
                if own.contains("://") {
                    vec![own.to_string()]
                } else {
                    vec![format!("tcp://{own}")]
                }
            }
        }
    }

    /// Показывать ли окну, что выбранный «свой» резолвер не задан.
    // Exercised by the tests; no caller in the app itself yet.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn custom_dns_missing(&self) -> bool {
        self.dns == DnsChoice::Custom && self.custom_dns.trim().is_empty()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// «Свои правила»: два списка доменов, которыми человек управляет сам
// ─────────────────────────────────────────────────────────────────────────
//
// Проверяется на границе (эти функции), а не в xray: неверная строка в
// правиле маршрутизации - это либо молчаливо нерабочее правило, либо конфиг,
// который xray откажется поднимать целиком, и окно узнало бы об этом только
// по тому, что кнопка перестала подключать. Здесь неверная строка просто не
// попадает в список, а окно показывает какие строки не приняты - ни один
// туннель из-за этого не ломается.

/// Столько строк принимается на каждый список. Не техническое ограничение
/// xray - предохранитель от вставленного по ошибке файла на десять тысяч
/// строк, который отрисовать и объяснить было бы уже нечем.
pub const MAX_CUSTOM_DOMAINS: usize = 200;

/// Одна строка «Своих правил» после проверки - или почему её не приняли.
///
/// Форма на выходе - ровно то, что понимает `domain` в правиле маршрутизации
/// xray: голое имя становится `domain:` (совпадение по суффиксу, как и в
/// профиле подписки - `to_xray_domain` в subscription.rs), `full:`/`domain:`,
/// введённые рукой, остаются как есть. Всё, что xray прочтёт не как имя -
/// адрес, ссылка, порт, звёздочка - отклоняется здесь, а не превращается в
/// правило, которое никогда не сработает.
pub fn validate_custom_domain(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (prefix, rest) = if let Some(r) = trimmed.strip_prefix("full:") {
        ("full:", r)
    } else if let Some(r) = trimmed.strip_prefix("domain:") {
        ("domain:", r)
    } else {
        ("domain:", trimmed)
    };
    let rest = rest.trim();
    if rest.is_empty() || rest.len() > 253 {
        return None;
    }
    // Ни схемы, ни пути, ни порта, ни звёздочки, ни пробела - только имя.
    // Любое из этого xray прочтёт не так, как человек имел в виду.
    if rest.contains("://")
        || rest.contains('/')
        || rest.contains('*')
        || rest.contains(' ')
        || rest.contains(':')
        || rest.contains('\\')
    {
        return None;
    }
    // Адрес - это правило для ДРУГОГО списка (по IP), которого этот экран не
    // предлагает; молча принять его как «домен» значило бы завести правило,
    // которое никогда не совпадёт ни с одним именем.
    if rest.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let labels: Vec<&str> = rest.split('.').collect();
    if labels.len() < 2 {
        // Голое имя без точки ("localhost", опечатка) - не то, что сюда
        // обычно вводят, и почти наверняка ошибка, а не домен.
        return None;
    }
    for label in &labels {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return None;
        }
    }
    Some(format!("{prefix}{}", rest.to_ascii_lowercase()))
}

/// Разобрать список «по строке на правило»: что приняли (в форме xray, без
/// повторов) и что отклонили (как ввели, для показа рядом с полем). Строки
/// сверх `MAX_CUSTOM_DOMAINS` тоже уходят в отклонённые - без этого список,
/// вставленный целиком из чужого файла, тихо обрезался бы с непонятной
/// стороны.
pub fn parse_custom_domain_list(raw: &str) -> (Vec<String>, Vec<String>) {
    let mut accepted = Vec::new();
    let mut ignored = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if accepted.len() >= MAX_CUSTOM_DOMAINS {
            ignored.push(line.to_string());
            continue;
        }
        match validate_custom_domain(line) {
            Some(v) if seen.insert(v.clone()) => accepted.push(v),
            // Повтор - не ошибка ввода, просто вторая строка ничего не
            // добавляет; в отклонённые её отправлять было бы вводящим в
            // заблуждение ("а что в ней не так?").
            Some(_) => {}
            None => ignored.push(line.to_string()),
        }
    }
    (accepted, ignored)
}

/// Итог разбора ОБОИХ списков сразу.
pub struct CustomRules {
    pub direct: Vec<String>,
    pub proxy: Vec<String>,
    pub direct_ignored: Vec<String>,
    pub proxy_ignored: Vec<String>,
}

/// Разобрать «Всегда напрямую» и «Всегда через VPN» вместе и развести домен,
/// оказавшийся в обоих сразу.
///
/// Это противоречивая инструкция ("всегда напрямую" и "всегда через VPN" для
/// одного и того же имени), и xray в любом случае взял бы только первое по
/// порядку правило - молча решать это порядком строк в конфиге, который
/// человек не видит, хуже, чем сказать в окне: строка отклонена на ОБОИХ
/// списках, а не тихо угадать которое из двух желаний важнее.
pub fn parse_custom_rules(direct_raw: &str, proxy_raw: &str) -> CustomRules {
    let (mut direct, mut direct_ignored) = parse_custom_domain_list(direct_raw);
    let (mut proxy, mut proxy_ignored) = parse_custom_domain_list(proxy_raw);

    let direct_set: std::collections::HashSet<&str> = direct.iter().map(String::as_str).collect();
    let clashing: Vec<String> = proxy
        .iter()
        .filter(|d| direct_set.contains(d.as_str()))
        .cloned()
        .collect();
    if !clashing.is_empty() {
        let clash_set: std::collections::HashSet<&str> = clashing.iter().map(String::as_str).collect();
        direct.retain(|d| !clash_set.contains(d.as_str()));
        proxy.retain(|d| !clash_set.contains(d.as_str()));
        direct_ignored.extend(clashing.iter().cloned());
        proxy_ignored.extend(clashing);
    }

    CustomRules { direct, proxy, direct_ignored, proxy_ignored }
}

fn paths() -> Vec<std::path::PathBuf> {
    crate::appdirs::state_dirs()
        .into_iter()
        .map(|dir| dir.join("tunnel-prefs.json"))
        .collect()
}

/// Прочитать настройки. Любая беда - значения по умолчанию.
///
/// Молчаливый возврат к умолчаниям здесь правильный: испорченный файл не
/// должен мешать человеку подключиться. Разница видна в окне, а не в отказе.
pub fn load() -> TunnelPrefs {
    for path in paths() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(prefs) = serde_json::from_str::<TunnelPrefs>(&text) {
                return prefs;
            }
        }
    }
    TunnelPrefs::default()
}

/// Записать настройки. Возвращает true, если удалось хоть куда-то.
pub fn store(prefs: &TunnelPrefs) -> bool {
    let Ok(text) = serde_json::to_string_pretty(prefs) else {
        return false;
    };
    // Owner-only: on macOS the first folder is shared by every account.
    paths()
        .iter()
        .any(|path| crate::appdirs::write_private(path, format!("{text}\n").as_bytes()).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Умолчания обязаны повторять поведение ДО появления настроек.
    #[test]
    fn defaults_change_nothing() {
        let p = TunnelPrefs::default();
        assert!(!p.fragment, "дробление пакетов стоит денег и включается руками");
        assert_eq!(p.ip_kind.query_strategy(), "UseIPv4");
        assert_eq!(
            p.dns_servers(),
            vec!["https://1.1.1.1/dns-query", "tcp://8.8.8.8"]
        );
    }

    /// Резолвер НИКОГДА не должен получиться голым UDP-адресом.
    ///
    /// Запросы идут через `proxy`, а Vision несёт только TCP: UDP-адрес - это
    /// мёртвые первые секунды после подъёма туннеля.
    #[test]
    fn a_resolver_is_never_plain_udp() {
        for prefs in [
            TunnelPrefs::default(),
            TunnelPrefs { dns: DnsChoice::Custom, custom_dns: "9.9.9.9".into(), ..Default::default() },
            TunnelPrefs { dns: DnsChoice::Custom, custom_dns: "  ".into(), ..Default::default() },
            TunnelPrefs { dns: DnsChoice::Custom, custom_dns: "https://dns.quad9.net/dns-query".into(), ..Default::default() },
        ] {
            for server in prefs.dns_servers() {
                assert!(
                    server.starts_with("https://")
                        || server.starts_with("tcp://")
                        || server == "localhost",
                    "{server}: такой резолвер не пройдёт через выход, который несёт только TCP"
                );
            }
        }
    }

    /// Пустой «свой» резолвер не оставляет туннель без DNS.
    #[test]
    fn an_empty_custom_resolver_falls_back_instead_of_dying() {
        let p = TunnelPrefs {
            dns: DnsChoice::Custom,
            custom_dns: String::new(),
            ..Default::default()
        };
        assert!(p.custom_dns_missing(), "окно обязано сказать, что адрес не введён");
        assert_eq!(p.dns_servers(), TunnelPrefs::default().dns_servers());
    }

    /// Свой адрес со схемой берётся как есть, без схемы - получает tcp://.
    #[test]
    fn a_custom_resolver_keeps_its_scheme_or_gets_tcp() {
        let bare = TunnelPrefs {
            dns: DnsChoice::Custom,
            custom_dns: "9.9.9.9".into(),
            ..Default::default()
        };
        assert_eq!(bare.dns_servers(), vec!["tcp://9.9.9.9"]);

        let doh = TunnelPrefs {
            dns: DnsChoice::Custom,
            custom_dns: "https://dns.quad9.net/dns-query".into(),
            ..Default::default()
        };
        assert_eq!(doh.dns_servers(), vec!["https://dns.quad9.net/dns-query"]);
    }

    #[test]
    fn every_ip_kind_maps_to_a_strategy_xray_knows() {
        for (kind, want) in [
            (IpKind::Ipv4, "UseIPv4"),
            (IpKind::Ipv6, "UseIPv6"),
            (IpKind::Both, "UseIP"),
        ] {
            assert_eq!(kind.query_strategy(), want);
        }
    }

    /// Незнакомое поле в файле не должно ронять настройки целиком.
    #[test]
    fn an_unknown_field_does_not_lose_the_rest() {
        let text = r#"{"fragment":true,"ipKind":"both","somethingNew":42}"#;
        let p: TunnelPrefs = serde_json::from_str(text).expect("читается");
        assert!(p.fragment);
        assert_eq!(p.ip_kind, IpKind::Both);
        // Не названное берётся из умолчаний.
        assert_eq!(p.dns, DnsChoice::Internal);
    }

    /// Файл, записанный ДО появления «Способа подключения» и «Своих правил»,
    /// не должен превращаться в пустые настройки при следующей загрузке.
    #[test]
    fn a_file_from_before_these_settings_still_loads() {
        let text = r#"{"fragment":true,"ipKind":"ipv4","dns":"internal","customDns":""}"#;
        let p: TunnelPrefs = serde_json::from_str(text).expect("читается");
        assert_eq!(p.transport, TransportPref::Auto);
        assert!(p.direct_domains.is_empty());
        assert!(p.proxy_domains.is_empty());
    }

    #[test]
    fn transport_pref_defaults_to_auto_and_asks_for_nothing_specific() {
        assert_eq!(TunnelPrefs::default().transport, TransportPref::Auto);
        assert_eq!(TransportPref::Auto.candidate_protocol(), None);
        assert_eq!(TransportPref::XhttpOnly.candidate_protocol(), Some("xhttp"));
        assert_eq!(TransportPref::VisionOnly.candidate_protocol(), Some("vision"));
    }

    // ── «Свои правила»: проверка одной строки ───────────────────────────────

    #[test]
    fn a_bare_name_becomes_a_domain_suffix_rule() {
        assert_eq!(validate_custom_domain("example.com").as_deref(), Some("domain:example.com"));
        assert_eq!(validate_custom_domain("Sub.Example.COM").as_deref(), Some("domain:sub.example.com"));
    }

    #[test]
    fn full_and_domain_prefixes_typed_by_hand_are_kept() {
        assert_eq!(validate_custom_domain("full:example.com").as_deref(), Some("full:example.com"));
        assert_eq!(validate_custom_domain("domain:example.com").as_deref(), Some("domain:example.com"));
    }

    #[test]
    fn an_ip_a_url_a_wildcard_and_a_port_are_all_rejected() {
        for bad in [
            "1.2.3.4",
            "2001:db8::1",
            "https://example.com",
            "example.com/path",
            "*.example.com",
            "example.com:443",
            "not a domain",
            "",
            "   ",
            "localhost",
        ] {
            assert!(validate_custom_domain(bad).is_none(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn a_label_over_63_characters_or_a_name_over_253_is_rejected() {
        let long_label = format!("{}.com", "a".repeat(64));
        assert!(validate_custom_domain(&long_label).is_none());
        // Five 60-character labels: each one is legal on its own (<=63), but
        // the assembled name is well past the 253-character ceiling.
        let long_name = format!("{}.com", vec!["a".repeat(60); 5].join("."));
        assert!(long_name.len() > 253, "test setup: the name must actually be too long");
        assert!(validate_custom_domain(&long_name).is_none());
    }

    #[test]
    fn parsing_a_list_separates_accepted_from_ignored_and_drops_duplicates() {
        let raw = "example.com\nexample.com\n1.2.3.4\n\nfull:api.example.com\nbad line";
        let (accepted, ignored) = parse_custom_domain_list(raw);
        assert_eq!(accepted, vec!["domain:example.com", "full:api.example.com"]);
        assert_eq!(ignored, vec!["1.2.3.4", "bad line"]);
    }

    #[test]
    fn a_list_longer_than_the_cap_ignores_the_overflow_not_the_start() {
        let raw = (0..MAX_CUSTOM_DOMAINS + 5)
            .map(|i| format!("host{i}.example.com"))
            .collect::<Vec<_>>()
            .join("\n");
        let (accepted, ignored) = parse_custom_domain_list(&raw);
        assert_eq!(accepted.len(), MAX_CUSTOM_DOMAINS);
        assert_eq!(ignored.len(), 5);
    }

    #[test]
    fn a_domain_in_both_lists_is_dropped_from_both_not_guessed_at() {
        let result = parse_custom_rules("shared.com\ndirect-only.com", "shared.com\nproxy-only.com");
        assert_eq!(result.direct, vec!["domain:direct-only.com"]);
        assert_eq!(result.proxy, vec!["domain:proxy-only.com"]);
        assert!(result.direct_ignored.contains(&"domain:shared.com".to_string()));
        assert!(result.proxy_ignored.contains(&"domain:shared.com".to_string()));
    }
}
