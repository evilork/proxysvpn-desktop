// src/notify_prefs.rs
//
// «Уведомления» (MoreScreen): показывать ли системное уведомление macOS,
// когда защита пропадает и когда она восстанавливается.
//
// Отдельный файл, а не поле в `tunnel_prefs.rs`: у того модуля своё правило —
// «настройка попадает туда, только если она ДЕЙСТВИТЕЛЬНО меняет работу
// туннеля». Эта ничего в туннеле не меняет, она решает только одно — говорит
// ли приложение об этом вслух. Формат хранения тот же (JSON рядом с
// tunnel-prefs.json, та же лестница путей), чтобы не плодить второй способ
// решать «куда писать».

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NotifyPrefs {
    pub enabled: bool,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        // Включено по умолчанию: узнать, что защита пропала, важнее, чем не
        // видеть лишний тост, и это как раз тот случай, где молчание не
        // нейтрально - человек может решить, что всё ещё под защитой.
        Self { enabled: true }
    }
}

fn paths() -> Vec<std::path::PathBuf> {
    crate::appdirs::state_dirs()
        .into_iter()
        .map(|dir| dir.join("notify-prefs.json"))
        .collect()
}

/// Прочитать настройку. Любая беда - значение по умолчанию (включено): то же
/// правило, что и у `tunnel_prefs::load` - испорченный файл не должен мешать
/// человеку, только тихо вернуться к тому, что было бы без него.
pub fn load() -> NotifyPrefs {
    for path in paths() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(prefs) = serde_json::from_str::<NotifyPrefs>(&text) {
                return prefs;
            }
        }
    }
    NotifyPrefs::default()
}

/// Записать настройку. Возвращает true, если удалось хоть куда-то - как и
/// `tunnel_prefs::store`.
pub fn store(prefs: &NotifyPrefs) -> bool {
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

    #[test]
    fn defaults_to_enabled() {
        assert!(NotifyPrefs::default().enabled);
    }

    #[test]
    fn an_unknown_field_does_not_lose_the_rest() {
        let text = r#"{"enabled":false,"somethingNew":42}"#;
        let p: NotifyPrefs = serde_json::from_str(text).expect("читается");
        assert!(!p.enabled);
    }

    #[test]
    fn an_empty_file_falls_back_to_the_default() {
        let p: Result<NotifyPrefs, _> = serde_json::from_str("");
        assert!(p.is_err(), "test setup: an empty file must not parse as valid JSON");
        // `load()` itself is not exercised here (it touches the real
        // filesystem) - the invariant it depends on is that a parse failure
        // never panics and the caller gets `NotifyPrefs::default()`, which is
        // exactly what `unwrap_or_default` below stands in for.
        assert_eq!(p.unwrap_or_default(), NotifyPrefs::default());
    }
}
