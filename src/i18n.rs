// SPDX-License-Identifier: GPL-3.0-or-later
//! Interface language for BabelCraft's own buttons, menus and labels.
//!
//! Every piece of interface text is written in English and passed through `tr`, which returns
//! it in the chosen language. Translations are plain text files, one text per line:
//! `<id><TAB><translation>`, where the id is `text_id` of the English text
//! (`assets/i18n/en.txt` lists them all). Hand-made translations are built into the program
//! (`assets/i18n/<code>.txt`); for any other language BabelCraft's AI makes one the first time
//! it is chosen and saves it in the data folder (`i18n/<code>.txt`). A text without a
//! translation shows in English.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// Every English interface text, with its id.
const EN_TXT: &str = include_str!("../assets/i18n/en.txt");

/// Languages translated by hand and built into the program.
const BUILT_IN: &[(&str, &str)] = &[
    ("ja", include_str!("../assets/i18n/ja.txt")),
    ("es", include_str!("../assets/i18n/es.txt")),
    ("fr", include_str!("../assets/i18n/fr.txt")),
    ("de", include_str!("../assets/i18n/de.txt")),
    ("pt", include_str!("../assets/i18n/pt.txt")),
    ("it", include_str!("../assets/i18n/it.txt")),
    ("ru", include_str!("../assets/i18n/ru.txt")),
    ("zh-Hans", include_str!("../assets/i18n/zh-Hans.txt")),
    ("ko", include_str!("../assets/i18n/ko.txt")),
];

struct Pack {
    code: String,
    map: HashMap<u32, &'static str>,
}

/// The language in use (`None` = English).
static ACTIVE: RwLock<Option<Arc<Pack>>> = RwLock::new(None);
/// Languages read so far, so each is read (and kept in memory) only once.
static LOADED: Mutex<Vec<Arc<Pack>>> = Mutex::new(Vec::new());

/// The id of an English interface text: its FNV-1a (32-bit) hash.
pub fn text_id(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Interface text in the chosen language (falls back to English when a line has no translation).
pub fn tr(en: &'static str) -> &'static str {
    let active = ACTIVE.read().unwrap_or_else(|e| e.into_inner());
    match active.as_ref() {
        Some(p) => p.map.get(&text_id(en)).copied().unwrap_or(en),
        None => en,
    }
}

/// `tr` for texts with blanks: `trf("{0} of {1}", &["2", "3"])`.
pub fn trf(en: &'static str, args: &[&str]) -> String {
    let mut s = tr(en).to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), a);
    }
    s
}

/// The interface language in use ("en" for English).
pub fn current() -> String {
    let active = ACTIVE.read().unwrap_or_else(|e| e.into_inner());
    active.as_ref().map(|p| p.code.clone()).unwrap_or_else(|| "en".into())
}

/// Switches the interface language. English always works; another language needs its built-in
/// or saved translation — without one this returns false and the interface stays as it is.
pub fn set_language(code: &str) -> bool {
    if code.is_empty() || code.eq_ignore_ascii_case("en") {
        *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = None;
        return true;
    }
    if current().eq_ignore_ascii_case(code) {
        return true;
    }
    match load(code) {
        Some(p) => {
            *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = Some(p);
            true
        }
        None => false,
    }
}

/// Where a language's translation comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Availability {
    /// English itself, or translated by hand and built in.
    BuiltIn,
    /// Translated earlier by BabelCraft's AI and saved on this computer.
    Saved,
    /// Not translated yet.
    NotYet,
}

pub fn availability(code: &str) -> Availability {
    if code.eq_ignore_ascii_case("en") || BUILT_IN.iter().any(|(c, _)| c.eq_ignore_ascii_case(code)) {
        Availability::BuiltIn
    } else if saved_file(code).is_file() {
        Availability::Saved
    } else {
        Availability::NotYet
    }
}

/// Codes of the hand-made translations built into the program.
pub fn built_in_codes() -> impl Iterator<Item = &'static str> {
    BUILT_IN.iter().map(|(c, _)| *c)
}

/// Where BabelCraft keeps a translation its AI made.
pub fn saved_file(code: &str) -> std::path::PathBuf {
    crate::config::data_dir().join("i18n").join(format!("{code}.txt"))
}

fn load(code: &str) -> Option<Arc<Pack>> {
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = loaded.iter().find(|p| p.code.eq_ignore_ascii_case(code)) {
        return Some(p.clone());
    }
    let text: &'static str = match BUILT_IN.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)) {
        Some((_, t)) => t,
        None => {
            let s = std::fs::read_to_string(saved_file(code)).ok()?;
            // Kept for the rest of the run, like the built-in ones.
            Box::leak(s.into_boxed_str())
        }
    };
    let pack = Arc::new(Pack { code: code.to_string(), map: parse(text).into_iter().collect() });
    loaded.push(pack.clone());
    Some(pack)
}

/// Forgets a saved translation that was read earlier (it is about to be made again).
pub fn forget(code: &str) {
    LOADED.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| !p.code.eq_ignore_ascii_case(code));
}

/// `<id><TAB><text>` lines; `#` starts a comment.
fn parse(text: &'static str) -> Vec<(u32, &'static str)> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (id, value) = l.split_once('\t')?;
            let id = u32::from_str_radix(id.trim(), 16).ok()?;
            let value = value.trim_end_matches('\r');
            let value: &'static str = if value.contains('\\') { Box::leak(unescape(value).into_boxed_str()) } else { value };
            (!value.is_empty()).then_some((id, value))
        })
        .collect()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "")
}

/// Every English interface text (for translating BabelCraft into a new language).
pub fn english_texts() -> Vec<&'static str> {
    parse(EN_TXT).into_iter().map(|(_, t)| t).collect()
}

/// Blanks like `{0}` a text has.
fn placeholders(s: &str) -> Vec<String> {
    (0..4).map(|i| format!("{{{i}}}")).filter(|p| s.contains(p.as_str())).collect()
}

/// Saves a translation BabelCraft's AI made (`english[i]` → `translated[i]`) and returns how
/// many texts it covers. Lines that came back empty, or lost a blank like `{0}`, are left out
/// (they show in English).
pub fn save_ai_translation(code: &str, english: &[&str], translated: &[String], made_by: &str) -> Result<usize> {
    let mut out = format!("# BabelCraft interface text - {code}, translated by BabelCraft's AI ({made_by}).\n# Delete this file to have it translated again.\n");
    let mut n = 0;
    for (en, t) in english.iter().zip(translated) {
        let t = t.trim();
        if t.is_empty() || placeholders(en).iter().any(|p| !t.contains(p.as_str())) {
            continue;
        }
        out.push_str(&format!("{:08x}\t{}\n", text_id(en), escape(t)));
        n += 1;
    }
    let path = saved_file(code);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, out).with_context(|| format!("could not write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)?;
    forget(code);
    Ok(n)
}

// ------------------------------------------------------------------ texts with numbers in them

/// Installer step names ("AI brain model Qwen 3 · 4B", "Unpacking FFmpeg", …) in the chosen language.
pub fn tr_step(en: &str) -> String {
    if let Some(name) = en.strip_prefix("AI brain model ") {
        return trf("AI brain model {0}", &[name]);
    }
    if let Some(file) = en.strip_prefix("Whisper model ") {
        return trf("Whisper model {0}", &[file]);
    }
    // "VOICEVOX voice engine (part 1 of 3)"
    if let Some((a, b)) = en.strip_prefix("VOICEVOX voice engine (part ").and_then(|r| r.trim_end_matches(')').split_once(" of ")) {
        return trf("VOICEVOX voice engine (part {0} of {1})", &[a, b]);
    }
    let active = ACTIVE.read().unwrap_or_else(|e| e.into_inner());
    match active.as_ref().and_then(|p| p.map.get(&text_id(en))) {
        Some(t) => t.to_string(),
        None => en.to_string(),
    }
}

/// "One-time setup · download 2 of 3…"
pub fn setup_headline(index: usize, count: usize) -> String {
    trf("One-time setup · download {0} of {1}…", &[&index.to_string(), &count.to_string()])
}

/// "1.3 GB of 2.5 GB · 2.7 MB/s" (plus a note when it carries on from an earlier try).
pub fn download_numbers(d: &crate::util::DownloadStats) -> String {
    use crate::util::fmt_bytes;
    let mut s = match d.total {
        Some(t) => trf("{0} of {1}", &[&fmt_bytes(d.done), &fmt_bytes(t)]),
        None => fmt_bytes(d.done),
    };
    if d.per_sec > 0.0 {
        let speed = if d.per_sec >= 1e6 { format!("{:.1} MB", d.per_sec / 1e6) } else { format!("{:.0} KB", (d.per_sec / 1e3).max(1.0)) };
        s.push_str(" · ");
        s.push_str(&trf("{0}/s", &[&speed]));
    }
    if d.resumed_from > 0 {
        s.push_str(" · ");
        s.push_str(tr("carrying on from last time"));
    }
    s
}

/// " · 07:12 left"
pub fn time_left(t: &str) -> String {
    format!(" · {}", trf("{0} left", &[t]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The language switch is shared by the whole program, so everything that changes it runs
    /// in this one test.
    #[test]
    fn switches_language() {
        assert!(set_language("en"));
        assert_eq!(tr("Settings"), "Settings");
        assert!(set_language("ja"));
        assert_eq!(current(), "ja");
        assert_eq!(tr("Settings"), "設定");
        assert_eq!(tr("not in the table"), "not in the table");
        assert!(!set_language("xx-no-such-language"));
        assert_eq!(current(), "ja", "an unknown language leaves the interface as it was");
        download_lines_read_well();
        assert!(set_language("en"));
        assert_eq!(tr("Settings"), "Settings");
    }

    fn download_lines_read_well() {
        use crate::util::DownloadStats;
        let d = DownloadStats { done: 1_325_158_635, total: Some(2_497_281_120), per_sec: 2_700_000.0, resumed_from: 0 };
        set_language("en");
        assert_eq!(download_numbers(&d), "1.3 GB of 2.5 GB · 2.7 MB/s");
        assert_eq!(setup_headline(2, 3), "One-time setup · download 2 of 3…");
        assert_eq!(tr_step("AI brain model Qwen 3 · 4B"), "AI brain model Qwen 3 · 4B");
        // 1.17 GB left at 2.7 MB/s is about 7 minutes.
        let eta = d.eta_secs().unwrap();
        assert!((430.0..440.0).contains(&eta), "{eta}");
        let resumed = DownloadStats { resumed_from: 1_000_000_000, per_sec: 0.0, ..d };
        assert_eq!(download_numbers(&resumed), "1.3 GB of 2.5 GB · carrying on from last time");
        let small = DownloadStats { done: 526_000, total: Some(2_499_479), per_sec: 0.0, resumed_from: 0 };
        assert_eq!(download_numbers(&small), "526 KB of 2.5 MB");
        assert_eq!(crate::util::fmt_bytes(147_964_211), "148 MB");
        assert_eq!(time_left("07:12"), " · 07:12 left");
        set_language("ja");
        assert_eq!(download_numbers(&d), "1.3 GB / 2.5 GB · 2.7 MB/秒");
        assert_eq!(setup_headline(2, 3), "初回セットアップ：ダウンロード（2 / 3）…");
        assert_eq!(tr_step("AI brain model Qwen 3 · 4B"), "AI モデル Qwen 3 · 4B");
        assert_eq!(tr_step("VOICEVOX voice engine (part 2 of 3)"), "VOICEVOX 音声エンジン（2 / 3）");
        assert_eq!(tr_step("Unpacking FFmpeg"), "FFmpeg を展開中");
        assert_eq!(time_left("07:12"), " · 残り 07:12");
    }

    #[test]
    fn english_list_and_translations_fit_together() {
        let english = english_texts();
        assert!(english.len() > 300);
        let ids: std::collections::HashSet<u32> = parse(EN_TXT).into_iter().map(|(id, t)| {
            assert_eq!(id, text_id(t), "id of {t:?} in en.txt");
            id
        }).collect();
        assert_eq!(ids.len(), english.len(), "two English texts share an id");
        assert_eq!(text_id("Settings"), 0x4b05_8728);
        for (code, text) in BUILT_IN {
            assert!(crate::langs::find(code).is_some(), "{code} is not in the language list");
            let lines = parse(text);
            let by_id: HashMap<u32, &str> = lines.iter().copied().collect();
            for (id, _) in &lines {
                assert!(ids.contains(id), "{code}: unknown id {id:08x}");
            }
            for en in &english {
                if let Some(t) = by_id.get(&text_id(en)) {
                    for p in placeholders(en) {
                        assert!(t.contains(p.as_str()), "{code}: {en:?} → {t:?} lost {p}");
                    }
                }
            }
            // Hand-made translations cover (almost) everything.
            assert!(by_id.len() * 100 >= english.len() * 97, "{code} covers only {} of {} texts", by_id.len(), english.len());
        }
    }

    /// Every text the code passes to `tr` is in the English list, so new languages translate it too.
    #[test]
    fn every_interface_text_is_listed() {
        let english: std::collections::HashSet<&str> = english_texts().into_iter().collect();
        let tool_names = ["ffmpeg", "ffprobe", "whisper-cli", "piper", "edge-tts"];
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        for entry in std::fs::read_dir(src).unwrap().flatten() {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            let mut rest = text.as_str();
            while let Some(i) = rest.find("tr(\"") {
                let before = rest[..i].chars().last();
                rest = &rest[i + 4..];
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue; // e.g. trf(" or str("
                }
                let mut s = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => s.push(chars.next().unwrap_or('\\')),
                        '"' => break,
                        c => s.push(c),
                    }
                }
                if !english.contains(s.as_str()) && !tool_names.contains(&s.as_str()) && !s.starts_with("not in the table") {
                    missing.push(format!("{}: {s}", entry.file_name().to_string_lossy()));
                }
            }
        }
        assert!(missing.is_empty(), "add these to assets/i18n/en.txt:\n{}", missing.join("\n"));
    }
}
