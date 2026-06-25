//! Dead-simple persistence: a tab-separated `key\tvalue` file. For save data,
//! settings, or high scores — anything a game wants to remember between runs.
//! Values are opaque strings; callers parse/format their own types. I/O is
//! best-effort: a missing or garbled file loads as empty, and write errors are
//! swallowed (a game shouldn't crash because a save dir is read-only).

use std::collections::HashMap;
use std::path::Path;

/// Load `key\tvalue` lines from `path` (blank lines and lines without a tab are
/// skipped). A missing/unreadable file yields an empty map.
pub fn load(path: &Path) -> HashMap<String, String> {
    let mut m = HashMap::new();
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('\t') {
                m.insert(k.to_string(), v.to_string());
            }
        }
    }
    m
}

/// Persist `map` as `key\tvalue` lines (best-effort; errors are ignored). Keys
/// are sorted so the file is stable/diffable across saves.
pub fn save(path: &Path, map: &HashMap<String, String>) {
    let mut pairs: Vec<(&String, &String)> = map.iter().collect();
    pairs.sort();
    let mut out = String::new();
    for (k, v) in pairs {
        out.push_str(k);
        out.push('\t');
        out.push_str(v);
        out.push('\n');
    }
    let _ = std::fs::write(path, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_garbage() {
        let mut dir = std::env::temp_dir();
        dir.push("scamper_store_test.kv");
        let mut m = HashMap::new();
        m.insert("level-1".to_string(), "42".to_string());
        m.insert("level-2".to_string(), "hello world".to_string());
        save(&dir, &m);
        let back = load(&dir);
        assert_eq!(back.get("level-1").map(String::as_str), Some("42"));
        assert_eq!(back.get("level-2").map(String::as_str), Some("hello world"));
        let _ = std::fs::remove_file(&dir);
        // A path that doesn't exist loads empty, not a panic.
        assert!(load(Path::new("/no/such/scamper/file")).is_empty());
    }
}
