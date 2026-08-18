//! Text matching shared by the sheet picker and search: substring after
//! folding case and full-width characters, so an IME left on full-width
//! still finds `IT-01`.

/// Folds a query once; haystacks are folded on the fly by `contains_folded`.
pub(super) fn fold(text: &str) -> String {
    text.chars().flat_map(fold_char).collect()
}

fn fold_char(c: char) -> impl Iterator<Item = char> {
    let c = match c {
        '０'..='９' => char::from(b'0' + (c as u32 - '０' as u32) as u8),
        'Ａ'..='Ｚ' => char::from(b'A' + (c as u32 - 'Ａ' as u32) as u8),
        'ａ'..='ｚ' => char::from(b'a' + (c as u32 - 'ａ' as u32) as u8),
        '－' => '-',
        '　' => ' ',
        c => c,
    };
    c.to_lowercase()
}

/// Substring test against the folded haystack, without building it — search
/// folds every cell of a 100k-row sheet per keystroke, and a fresh `String`
/// per cell costs seconds; folding char-by-char while scanning stays cheap.
/// Matches start at the haystack's own char boundaries: a needle beginning
/// mid-expansion (e.g. the bare combining dot of `İ` → `i̇`) never matches,
/// which is stricter than `fold(haystack).contains(...)` and closer to what
/// a user pointing at visible characters means.
pub(super) fn contains_folded(haystack: &str, needle_folded: &str) -> bool {
    let mut start = haystack.chars();
    loop {
        let mut hay = start.clone().flat_map(fold_char);
        if needle_folded.chars().all(|n| hay.next() == Some(n)) {
            return true;
        }
        if start.next().is_none() {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_maps_case_and_full_width_to_one_form() {
        assert_eq!(fold("ＩＴ－０１"), "it-01");
        assert_eq!(fold("Login　OK"), "login ok");
        assert_eq!(fold("合計"), "合計");
    }

    #[test]
    fn contains_folded_agrees_with_the_folding_reference() {
        let cases = [
            ("IT-01 Login", "ｉｔ－01"),
            ("IT-01 Login", "login"),
            ("合計金額", "計金"),
            ("abc", "zzz"),
            ("", "a"),
            ("abc", ""),
            // 'İ' lowercases to two chars — the fold must not split matches
            ("İstanbul", "i\u{307}stan"),
            ("İ", "istan"),
        ];
        for (haystack, query) in cases {
            let needle = fold(query);
            assert_eq!(
                contains_folded(haystack, &needle),
                fold(haystack).contains(&needle),
                "diverged on ({haystack:?}, {query:?})"
            );
        }
    }

    /// The documented divergence from the reference: matches only start at
    /// the haystack's own char boundaries, never inside one char's fold.
    #[test]
    fn a_needle_starting_mid_expansion_does_not_match() {
        let needle = fold("\u{307}stan");
        assert!(!contains_folded("İstanbul", &needle));
        assert!(fold("İstanbul").contains(&needle), "the reference would");
    }
}
