//! Tiny file-based localization.
//!
//! Each language is a `key = value` text file embedded at build time. English
//! ships now; adding a language is just a sibling file (`fr.txt`, …) plus a
//! [`Lang`] arm. Lookups return `&'static str` — slices of the embedded file, so
//! there's no allocation and the result is usable anywhere a static string is
//! (e.g. effect frames). A missing key falls back to English, then to the key
//! itself, so an un-translated string shows up visibly instead of panicking.
//!
//! ```
//! use scamper::strings::{t, tr, Lang};
//! assert_eq!(t("fx.bonk"), "BONK!");          // English shortcut
//! assert_eq!(tr(Lang::En, "fx.pop"), "POP!");
//! assert_eq!(t("nope.missing"), "nope.missing"); // visible fallback
//! ```

use std::collections::HashMap;
use std::sync::OnceLock;

/// A supported language. Add a variant (and its file + `catalog` arm) to localize.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    En,
}

static EN_SRC: &str = include_str!("strings/en.txt");

/// The parsed table for a language (built once, then cached).
fn catalog(lang: Lang) -> &'static HashMap<&'static str, &'static str> {
    match lang {
        Lang::En => {
            static EN: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
            EN.get_or_init(|| parse(EN_SRC))
        }
    }
}

/// Parse `key = value` lines (ignoring `#` comments and blanks) into a table of
/// `&'static` slices of `src`.
fn parse(src: &'static str) -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            m.insert(k.trim(), v.trim());
        }
    }
    m
}

/// Translate `key` into `lang`, falling back to English, then to `key` itself.
pub fn tr(lang: Lang, key: &'static str) -> &'static str {
    if let Some(v) = catalog(lang).get(key) {
        return v;
    }
    if let Some(v) = catalog(Lang::En).get(key) {
        return v;
    }
    key
}

/// Translate `key` in the default language (English). Shorthand for [`tr`].
pub fn t(key: &'static str) -> &'static str {
    tr(Lang::default(), key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_keys_resolve_and_unknown_falls_back() {
        assert_eq!(t("fx.bonk"), "BONK!");
        assert_eq!(tr(Lang::En, "fx.woah"), "WOAH!");
        // A missing key returns itself (visible, not a panic).
        assert_eq!(t("does.not.exist"), "does.not.exist");
    }

    #[test]
    fn comments_and_blanks_are_skipped() {
        let m = parse("# a comment\n\nk = v\n  spaced  =  trimmed  \n");
        assert_eq!(m.get("k"), Some(&"v"));
        assert_eq!(m.get("spaced"), Some(&"trimmed"));
        assert_eq!(m.len(), 2);
    }
}
