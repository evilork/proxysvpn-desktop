// src-tauri/src/location_name.rs
//
// Location names as the tray shows them: the Rust copy of
// src/locationName.ts.
//
// The service names every location in Russian ("Германия"), and the core
// keeps that name (`Session::location`) because it matches locations by it.
// The window translates it for display; the tray is drawn by the core, so an
// English window's tray read "Protected · Германия". The table is the
// window's own, and a test below fails the build when the two drift apart.

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

/// The name to show for `label`: a known Russian name, alone or followed by
/// more ("Германия 2", "Германия · XHTTP"), is translated for an English
/// window; anything else is returned unchanged.
pub fn display_location(label: &str, en: bool) -> String {
    if !en {
        return label.to_string();
    }
    for (ru, english) in EN_NAMES {
        if label == ru {
            return english.to_string();
        }
        if let Some(rest) = label.strip_prefix(ru) {
            if rest.starts_with(' ') {
                return format!("{english}{rest}");
            }
        }
    }
    label.to_string()
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
    }

    /// The tray and the window must call a location the same thing.
    #[test]
    fn the_table_is_the_windows_own() {
        let ts = include_str!("../../src/locationName.ts");
        let window: Vec<(&str, &str)> = ts
            .lines()
            .filter_map(|line| {
                let row = line.trim().strip_prefix("[\"")?.strip_suffix("\"],")?;
                row.split_once("\", \"")
            })
            .collect();
        assert!(!window.is_empty(), "src/locationName.ts changed shape; update this parser");
        assert_eq!(window, EN_NAMES.to_vec());
    }
}
