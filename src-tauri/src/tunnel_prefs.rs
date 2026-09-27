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
    /// Значение для `dns.queryStrategy` в конфигурации xray.
    pub fn query_strategy(self) -> &'static str {
        match self {
            Self::Ipv4 => "UseIPv4",
            Self::Ipv6 => "UseIPv6",
            Self::Both => "UseIP",
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
    pub fn custom_dns_missing(&self) -> bool {
        self.dns == DnsChoice::Custom && self.custom_dns.trim().is_empty()
    }
}

fn paths() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    out.push(
        std::path::PathBuf::from("/Library/Application Support/ProxysVPN").join("tunnel-prefs.json"),
    );
    if let Ok(home) = std::env::var("HOME") {
        let mut base = std::path::PathBuf::from(home).join("Library/Application Support");
        if cfg!(target_os = "macos") {
            base = base.join("com.proxysvpn.desktop");
        }
        out.push(base.join("tunnel-prefs.json"));
    }
    out
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
    for path in paths() {
        if let Some(dir) = path.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                continue;
            }
        }
        if std::fs::write(&path, format!("{text}\n")).is_ok() {
            return true;
        }
    }
    false
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
}
