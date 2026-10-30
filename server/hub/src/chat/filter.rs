//! Public-chat-only filtering, including common separator and homoglyph evasions.
pub(super) fn filtered(text: &str) -> bool {
    let mut normalized = String::new();
    let mut last = '\0';
    let mut repeats = 0;
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'ё' => 'е',
            'x' => 'х',
            'y' => 'у',
            'e' => 'е',
            'o' => 'о',
            'a' => 'а',
            'c' => 'с',
            'p' => 'р',
            'k' => 'к',
            'm' => 'м',
            't' => 'т',
            'b' => 'в',
            _ => c,
        };
        if !c.is_alphanumeric() {
            continue;
        }
        if c == last {
            repeats += 1;
            if repeats > 1 {
                continue;
            }
        } else {
            last = c;
            repeats = 1;
        }
        normalized.push(c);
    }
    [
        "хуй",
        "хуя",
        "хуе",
        "пизд",
        "пезд",
        "еба",
        "ебу",
        "ебл",
        "бля",
        "сука",
        "сучк",
        "мудак",
        "мудил",
        "долбо",
        "пидор",
        "пидар",
        "гандон",
        "гондон",
        "дебил",
        "идиот",
        "шлюх",
    ]
    .iter()
    .any(|root| normalized.contains(root))
}
#[cfg(test)]
mod tests {
    #[test]
    fn normalization() {
        for text in ["Х.У.Й", "хххуууййй", "п-и-з-д-а", "идиот"] {
            assert!(super::filtered(text), "{text}");
        }
        assert!(!super::filtered("Привет, играем вместе?"));
    }
}
