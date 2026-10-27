//! Room codes (`ABCD-1234`) and `mineded://join/<code>` links (`mineup://`
//! from before the rename is still accepted).

use lc_types::{AppError, AppResult, ErrorCode};

/// URL scheme registered by the desktop app for deep links.
pub const LINK_PREFIX: &str = "mineded://join/";

/// Accepted link schemes: current and pre-rename.
const SCHEMES: [&str; 2] = ["mineded:", "mineup:"];

/// Shareable https form (messengers don't make custom schemes clickable); the
/// site page opens `mineded://join/<code>`.
pub const SHARE_PREFIX: &str = "https://mineded.dedseckms.ru/join/";

/// Normalises user input into a canonical room code `XXXX-XXXX`.
///
/// Accepts a bare code (any case, with or without the dash, surrounding
/// whitespace) or a `mineded://` (or `mineup://`) join link (optionally with a trailing
/// slash, query or fragment).
pub fn parse_code(input: &str) -> AppResult<String> {
    let mut s = input.trim();
    if let Some(rest) = strip_prefix_ci(s, SHARE_PREFIX) {
        s = rest;
    } else if let Some(rest) = SCHEMES.iter().find_map(|p| strip_prefix_ci(s, p)) {
        // `scheme://join/<code>`, `scheme:join/<code>` and `scheme:///join/<code>`.
        s = rest.trim_start_matches('/');
        s = strip_prefix_ci(s, "join/").ok_or_else(|| bad_code(input))?;
    }
    let s = s.split(['?', '#']).next().unwrap_or_default();
    let s = s.trim_end_matches('/');
    let compact: String = s
        .chars()
        .filter(|c| *c != '-' && *c != ' ')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if compact.len() != 8 || !compact.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(bad_code(input));
    }
    Ok(format!("{}-{}", &compact[..4], &compact[4..]))
}

/// Shareable `https://mineded.dedseckms.ru/join/<code>` link for a canonical code.
pub fn join_link(code: &str) -> String {
    format!("{SHARE_PREFIX}{code}")
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

fn bad_code(input: &str) -> AppError {
    // Codes are not secret; the input is short and safe to echo back.
    let shown: String = input.chars().take(64).collect();
    AppError::new(ErrorCode::InvalidInput, "invalid room code").detail("code", shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codes_and_links() {
        for input in [
            "ABCD-1234",
            "abcd-1234",
            "  abcd1234 ",
            "ABCD 1234",
            "mineup://join/ABCD-1234",
            "MINEUP://JOIN/abcd-1234/",
            "mineded://join/abcd1234?from=chat",
            "mineup:join/ABCD-1234",
            "mineup:///join/ABCD-1234",
        ] {
            assert_eq!(parse_code(input).unwrap(), "ABCD-1234", "{input}");
        }
    }

    #[test]
    fn rejects_garbage() {
        for input in [
            "",
            "ABC-123",
            "ABCD-12345",
            "ABCD-12!4",
            "https://join/ABCD-1234",
            "mineup://other/ABCD-1234",
            "mineup:ABCD-1234",
            "АБВГ-1234",
        ] {
            let err = parse_code(input).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{input}");
        }
    }

    #[test]
    fn link_roundtrip() {
        let link = join_link("ABCD-1234");
        assert_eq!(link, "https://mineded.dedseckms.ru/join/ABCD-1234");
        assert_eq!(parse_code(&link).unwrap(), "ABCD-1234");
    }
}
