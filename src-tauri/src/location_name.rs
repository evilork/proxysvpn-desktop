// src-tauri/src/location_name.rs
//
// Location names as the tray shows them: the Rust copy of
// src/locationName.ts.
//
// The service names every location in Russian ("Германия"), and the core
// keeps that name (`Session::location`) because it matches locations by it.
// The window translates it for display; the tray is drawn by the core, so an
// English window's tray read "Protected · Германия". Both tables are the
// window's own, and a test below fails the build when the two drift apart.
//
// The words around the names are translated too («США (50 ГБ/мес)» is
// "United States (50 GB/mo)"); src/locationName.ts says why and which.

/// The service's Russian names, and what an English window calls them.
const EN_NAMES: [(&str, &str); 50] = [
    ("Нидерланды", "Netherlands"),
    ("Германия", "Germany"),
    ("Великобритания", "United Kingdom"),
    ("Британия", "United Kingdom"),
    ("США", "United States"),
    ("Франция", "France"),
    ("Финляндия", "Finland"),
    ("Швеция", "Sweden"),
    ("Польша", "Poland"),
    ("Казахстан", "Kazakhstan"),
    ("Турция", "Türkiye"),
    ("Россия", "Russia"),
    ("Латвия", "Latvia"),
    ("Литва", "Lithuania"),
    ("Эстония", "Estonia"),
    ("Швейцария", "Switzerland"),
    ("Испания", "Spain"),
    ("Италия", "Italy"),
    ("Австрия", "Austria"),
    ("Норвегия", "Norway"),
    ("Дания", "Denmark"),
    ("Чехия", "Czechia"),
    ("Румыния", "Romania"),
    ("Болгария", "Bulgaria"),
    ("Сербия", "Serbia"),
    ("Грузия", "Georgia"),
    ("Армения", "Armenia"),
    ("Молдова", "Moldova"),
    ("Украина", "Ukraine"),
    ("Ирландия", "Ireland"),
    ("Бельгия", "Belgium"),
    ("Люксембург", "Luxembourg"),
    ("Венгрия", "Hungary"),
    ("Португалия", "Portugal"),
    ("Израиль", "Israel"),
    ("ОАЭ", "UAE"),
    ("Индия", "India"),
    ("Япония", "Japan"),
    ("Сингапур", "Singapore"),
    ("Гонконг", "Hong Kong"),
    ("Корея", "Korea"),
    ("Канада", "Canada"),
    ("Бразилия", "Brazil"),
    ("Австралия", "Australia"),
    ("Москва", "Moscow"),
    ("Амстердам", "Amsterdam"),
    ("Франкфурт", "Frankfurt"),
    ("Лондон", "London"),
    ("Хельсинки", "Helsinki"),
    ("Стокгольм", "Stockholm"),
];

/// Words the service writes around the names, keyed in lower case: badges
/// («резерв»), quota lines («США · 0.2/50 ГБ»), and the «недоступен» that
/// manifest.rs adds. Matched as whole words in any case; the case of the
/// service's word is kept.
const EN_WORDS: [(&str, &str); 12] = [
    ("резерв", "reserve"),
    ("люкс", "deluxe"),
    ("квота", "quota"),
    ("использовано", "used"),
    ("сброс", "resets"),
    ("из", "of"),
    ("недоступен", "unavailable"),
    ("автовыбор", "auto"),
    ("гб", "GB"),
    ("мб", "MB"),
    ("тб", "TB"),
    ("мес", "mo"),
];

/// `english` in the case the service wrote `word` in: «ЛЮКС» → "DELUXE",
/// «Резерв» → "Reserve", «резерв» → "reserve".
fn in_case_of(word: &str, english: &str) -> String {
    let has_upper = word.chars().any(char::is_uppercase);
    let all_upper = has_upper && !word.chars().any(char::is_lowercase);
    if all_upper && word.chars().count() > 1 {
        return english.to_uppercase();
    }
    if word.chars().next().is_some_and(char::is_uppercase) {
        let mut chars = english.chars();
        return match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        };
    }
    english.to_string()
}

/// One whole word: a known name exactly as the service spells it, then a
/// known word in any case, otherwise the word itself.
fn translate_word(word: &str) -> String {
    if let Some((_, english)) = EN_NAMES.iter().find(|(ru, _)| *ru == word) {
        return (*english).to_string();
    }
    let lower = word.to_lowercase();
    match EN_WORDS.iter().find(|(ru, _)| *ru == lower) {
        Some((_, english)) => in_case_of(word, english),
        None => word.to_string(),
    }
}

/// The name to show for `label`. In an English window every known Russian
/// word — a location name ("Германия 2", "Германия · XHTTP") or a word of a
/// badge ("Амстердам · резерв", "США (50 ГБ/мес)") — is translated where it
/// stands; everything else, numbers included, is returned unchanged. A word is
/// a whole run of letters (`\p{L}+` in the window, `is_alphabetic` here — the
/// same runs for every name and word in the tables), so "Германияx" is not
/// "Германия".
pub fn display_location(label: &str, en: bool) -> String {
    if !en {
        return label.to_string();
    }
    let mut out = String::with_capacity(label.len());
    let mut word = String::new();
    for c in label.chars() {
        if c.is_alphabetic() {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            out.push_str(&translate_word(&word));
            word.clear();
        }
        out.push(c);
    }
    if !word.is_empty() {
        out.push_str(&translate_word(&word));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same answers tests/locationName.test.ts expects of the window.
    #[test]
    fn known_names_are_translated_for_an_english_tray_only() {
        assert_eq!(display_location("Германия", true), "Germany");
        assert_eq!(display_location("Британия", true), "United Kingdom");
        assert_eq!(display_location("Германия 2", true), "Germany 2");
        assert_eq!(display_location("Британия · XHTTP", true), "United Kingdom · XHTTP");
        assert_eq!(display_location("Германия", false), "Германия");
        assert_eq!(display_location("Atlantis-1", true), "Atlantis-1");
        assert_eq!(display_location("Германияx", true), "Германияx", "a prefix of a word is not the name");
        assert_eq!(display_location("", true), "");
        assert_eq!(display_location("Россия & Москва", true), "Russia & Moscow");
        assert_eq!(display_location("Нидерланды-2", true), "Netherlands-2");
    }

    /// "Amsterdam · резерв" in an English window (Windows VM, 02.10.2026),
    /// and the quota lines the service writes for the United States.
    #[test]
    fn badges_and_quota_lines_are_english_in_an_english_tray() {
        assert_eq!(display_location("Амстердам · резерв", true), "Amsterdam · reserve");
        assert_eq!(display_location("США (50 ГБ/мес)", true), "United States (50 GB/mo)");
        assert_eq!(display_location("США · 0.2/50 ГБ", true), "United States · 0.2/50 GB");
        assert_eq!(display_location("12,4 из 50 ГБ", true), "12,4 of 50 GB");
        assert_eq!(
            display_location("США · использовано 50/50 ГБ · сброс 05.10", true),
            "United States · used 50/50 GB · resets 05.10"
        );
        assert_eq!(display_location("Британия · XHTTP недоступен", true), "United Kingdom · XHTTP unavailable");
        assert_eq!(display_location("Германия · PRO", true), "Germany · PRO");
        assert_eq!(display_location("🌐 Автовыбор", true), "🌐 Auto");
    }

    #[test]
    fn the_case_of_the_services_word_is_kept() {
        assert_eq!(display_location("резерв", true), "reserve");
        assert_eq!(display_location("Резерв", true), "Reserve");
        assert_eq!(display_location("РЕЗЕРВ", true), "RESERVE");
        assert_eq!(display_location("ЛЮКС", true), "DELUXE");
        assert_eq!(display_location("Люкс", true), "Deluxe");
        assert_eq!(display_location("50 гб", true), "50 GB");
    }

    #[test]
    fn only_whole_known_words_change_and_russian_stays_russian() {
        assert_eq!(display_location("Амстердам · новый", true), "Amsterdam · новый");
        assert_eq!(display_location("резервный", true), "резервный");
        assert_eq!(display_location("Изумруд", true), "Изумруд");
        for label in ["Амстердам · резерв", "США (50 ГБ/мес)", "12,4 из 50 ГБ", "ЛЮКС"] {
            assert_eq!(display_location(label, false), label);
        }
    }

    /// One table of the window's file, as `(russian, english)` rows.
    fn window_table(name: &str) -> Vec<(&'static str, &'static str)> {
        let ts: &'static str = include_str!("../../src/locationName.ts");
        let start = ts
            .find(&format!("const {name}"))
            .unwrap_or_else(|| panic!("src/locationName.ts has no {name}; update this parser"));
        ts[start..]
            .lines()
            .skip(1)
            .take_while(|line| line.trim() != "];")
            .filter_map(|line| {
                let row = line.trim().strip_prefix("[\"")?.strip_suffix("\"],")?;
                row.split_once("\", \"")
            })
            .collect()
    }

    /// The tray and the window must call a location the same thing.
    #[test]
    fn the_tables_are_the_windows_own() {
        let names = window_table("EN_NAMES");
        assert!(!names.is_empty(), "src/locationName.ts changed shape; update this parser");
        assert_eq!(names, EN_NAMES.to_vec());
        let words = window_table("EN_WORDS");
        assert!(!words.is_empty(), "src/locationName.ts changed shape; update this parser");
        assert_eq!(words, EN_WORDS.to_vec());
    }
}
