// SPDX-License-Identifier: GPL-3.0-or-later
//! Finds a font that really contains Japanese glyphs (no □□□ tofu), on every OS.

use ab_glyph::{Font, FontRef};
use std::path::PathBuf;

pub const NOTO_FILE: &str = "NotoSansJP-Bold.otf";
/// Noto Sans JP (SIL Open Font License) from the official Noto CJK repository.
pub const NOTO_URL: &str = "https://github.com/notofonts/noto-cjk/raw/main/Sans/SubsetOTF/JP/NotoSansJP-Bold.otf";

#[derive(Clone, Debug)]
pub struct FoundFont {
    pub path: PathBuf,
    pub index: u32,
}

/// Font files tried in order (with the collection index for .ttc files).
fn candidates(user: &str) -> Vec<(PathBuf, u32)> {
    let mut v: Vec<(PathBuf, u32)> = Vec::new();
    if !user.trim().is_empty() {
        v.push((PathBuf::from(user.trim()), 0));
    }
    // Downloaded / bundled copies.
    v.push((crate::config::fonts_dir().join(NOTO_FILE), 0));
    v.push((crate::config::fonts_dir().join("NotoSansJP-Regular.otf"), 0));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for sub in ["assets/fonts", "fonts", "../Resources/fonts", "../share/babelcraft/fonts"] {
                v.push((dir.join(sub).join(NOTO_FILE), 0));
                v.push((dir.join(sub).join("NotoSansJP-Regular.otf"), 0));
            }
        }
    }
    v.push((PathBuf::from("assets/fonts").join(NOTO_FILE), 0));

    #[cfg(target_os = "windows")]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let f = PathBuf::from(windir).join("Fonts");
        for (name, idx) in [
            ("YuGothB.ttc", 0),
            ("YuGothM.ttc", 0),
            ("YuGothR.ttc", 0),
            ("meiryob.ttc", 0),
            ("meiryo.ttc", 0),
            ("NotoSansJP-VF.ttf", 0),
            ("BIZ-UDGothicB.ttc", 0),
            ("msgothic.ttc", 0),
        ] {
            v.push((f.join(name), idx));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            v.push((PathBuf::from(local).join("Microsoft\\Windows\\Fonts").join("NotoSansJP-Bold.otf"), 0));
        }
    }
    #[cfg(target_os = "macos")]
    {
        for p in [
            "/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/Library/Fonts/Arial Unicode.ttf",
        ] {
            v.push((PathBuf::from(p), 0));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for p in [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Bold.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc",
            "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/ipaexfont-gothic/ipaexg.ttf",
            "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
            "/usr/share/fonts/truetype/takao-gothic/TakaoPGothic.ttf",
            "/usr/share/fonts/vlgothic/VL-Gothic-Regular.ttf",
        ] {
            v.push((PathBuf::from(p), 0));
        }
        if let Some(home) = std::env::var_os("HOME") {
            v.push((PathBuf::from(home).join(".local/share/fonts").join(NOTO_FILE), 0));
        }
        // Ask fontconfig as a last resort.
        if let Ok(out) = std::process::Command::new("fc-match").args(["-f", "%{file}", ":lang=ja"]).output() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                v.push((PathBuf::from(p), 0));
            }
        }
    }
    v
}

/// True when the font file has real glyphs for kana and common kanji.
pub fn has_japanese(data: &[u8], index: u32) -> bool {
    match FontRef::try_from_slice_and_index(data, index) {
        Ok(f) => ['あ', 'ア', '日', '本', '語'].iter().all(|c| f.glyph_id(*c).0 != 0),
        Err(_) => false,
    }
}

/// Same search, but prefers regular weights (nicer for the app's own interface text).
pub fn find_ui_font(user_path: &str) -> Option<FoundFont> {
    let mut c = candidates(user_path);
    let is_bold = |p: &PathBuf| {
        let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        n.contains("bold") || n.ends_with("b.ttc") || n.contains("w6")
    };
    c.sort_by_key(|(p, _)| is_bold(p));
    first_with_japanese(c)
}

pub fn find_cjk_font(user_path: &str) -> Option<FoundFont> {
    first_with_japanese(candidates(user_path))
}

fn first_with_japanese(list: Vec<(PathBuf, u32)>) -> Option<FoundFont> {
    for (path, index) in list {
        if !path.is_file() {
            continue;
        }
        if let Ok(data) = std::fs::read(&path) {
            if has_japanese(&data, index) {
                return Some(FoundFont { path, index });
            }
        }
    }
    None
}

pub fn load(found: &FoundFont) -> Option<Vec<u8>> {
    std::fs::read(&found.path).ok()
}

// ---------------------------------------------------------------- other writing systems

use crate::langs::Script;

/// True when the font has every letter in `chars`.
pub fn has_chars(data: &[u8], index: u32, chars: &[char]) -> bool {
    match FontRef::try_from_slice_and_index(data, index) {
        Ok(f) => chars.iter().all(|c| f.glyph_id(*c).0 != 0),
        Err(_) => false,
    }
}

/// A font for BabelCraft's own interface in `script`: one the computer has, or one downloaded
/// earlier into BabelCraft's fonts folder. `None` for writing systems egui's own font covers.
pub fn find_script_font(script: Script) -> Option<FoundFont> {
    if script.built_in() {
        return None;
    }
    if script == Script::Japanese {
        return find_ui_font("");
    }
    for (path, index) in script_candidates(script) {
        if !path.is_file() {
            continue;
        }
        if let Ok(data) = std::fs::read(&path) {
            if has_chars(&data, index, script.sample()) {
                return Some(FoundFont { path, index });
            }
        }
    }
    None
}

/// The free Noto font for `script` (SIL Open Font License): file name and download address.
pub fn script_font_download(script: Script) -> Option<(&'static str, &'static str)> {
    Some(match script {
        Script::Latin | Script::Greek | Script::Cyrillic => return None,
        Script::Japanese => ("NotoSansJP-Regular.otf", "https://github.com/notofonts/noto-cjk/raw/main/Sans/SubsetOTF/JP/NotoSansJP-Regular.otf"),
        Script::ChineseSimplified => ("NotoSansSC-Regular.otf", "https://github.com/notofonts/noto-cjk/raw/main/Sans/SubsetOTF/SC/NotoSansSC-Regular.otf"),
        Script::ChineseTraditional => ("NotoSansTC-Regular.otf", "https://github.com/notofonts/noto-cjk/raw/main/Sans/SubsetOTF/TC/NotoSansTC-Regular.otf"),
        Script::Korean => ("NotoSansKR-Regular.otf", "https://github.com/notofonts/noto-cjk/raw/main/Sans/SubsetOTF/KR/NotoSansKR-Regular.otf"),
        Script::Armenian => ("NotoSansArmenian-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansArmenian/hinted/ttf/NotoSansArmenian-Regular.ttf"),
        Script::Georgian => ("NotoSansGeorgian-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansGeorgian/hinted/ttf/NotoSansGeorgian-Regular.ttf"),
        Script::Hebrew => ("NotoSansHebrew-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansHebrew/hinted/ttf/NotoSansHebrew-Regular.ttf"),
        Script::Arabic => ("NotoSansArabic-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansArabic/hinted/ttf/NotoSansArabic-Regular.ttf"),
        Script::Thaana => ("NotoSansThaana-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansThaana/hinted/ttf/NotoSansThaana-Regular.ttf"),
        Script::Devanagari => ("NotoSansDevanagari-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansDevanagari/hinted/ttf/NotoSansDevanagari-Regular.ttf"),
        Script::Bengali => ("NotoSansBengali-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansBengali/hinted/ttf/NotoSansBengali-Regular.ttf"),
        Script::Gurmukhi => ("NotoSansGurmukhi-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansGurmukhi/hinted/ttf/NotoSansGurmukhi-Regular.ttf"),
        Script::Gujarati => ("NotoSansGujarati-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansGujarati/hinted/ttf/NotoSansGujarati-Regular.ttf"),
        Script::Odia => ("NotoSansOriya-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansOriya/hinted/ttf/NotoSansOriya-Regular.ttf"),
        Script::Tamil => ("NotoSansTamil-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansTamil/hinted/ttf/NotoSansTamil-Regular.ttf"),
        Script::Telugu => ("NotoSansTelugu-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansTelugu/hinted/ttf/NotoSansTelugu-Regular.ttf"),
        Script::Kannada => ("NotoSansKannada-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansKannada/hinted/ttf/NotoSansKannada-Regular.ttf"),
        Script::Malayalam => ("NotoSansMalayalam-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansMalayalam/hinted/ttf/NotoSansMalayalam-Regular.ttf"),
        Script::Sinhala => ("NotoSansSinhala-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansSinhala/hinted/ttf/NotoSansSinhala-Regular.ttf"),
        Script::Thai => ("NotoSansThai-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansThai/hinted/ttf/NotoSansThai-Regular.ttf"),
        Script::Lao => ("NotoSansLao-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansLao/hinted/ttf/NotoSansLao-Regular.ttf"),
        Script::Khmer => ("NotoSansKhmer-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansKhmer/hinted/ttf/NotoSansKhmer-Regular.ttf"),
        Script::Myanmar => ("NotoSansMyanmar-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansMyanmar/hinted/ttf/NotoSansMyanmar-Regular.ttf"),
        Script::Ethiopic => ("NotoSansEthiopic-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSansEthiopic/hinted/ttf/NotoSansEthiopic-Regular.ttf"),
        Script::Tibetan => ("NotoSerifTibetan-Regular.ttf", "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSerifTibetan/hinted/ttf/NotoSerifTibetan-Regular.ttf"),
    })
}

/// Font files that usually have `script`'s letters, best first (each is checked before use).
fn script_candidates(script: Script) -> Vec<(PathBuf, u32)> {
    let mut v: Vec<(PathBuf, u32)> = Vec::new();
    if let Some((file, _)) = script_font_download(script) {
        v.push((crate::config::fonts_dir().join(file), 0));
    }
    #[cfg(target_os = "windows")]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let f = PathBuf::from(windir).join("Fonts");
        let names: &[&str] = match script {
            Script::Armenian | Script::Georgian => &["segoeui.ttf", "sylfaen.ttf", "arial.ttf"],
            Script::Hebrew | Script::Arabic => &["segoeui.ttf", "arial.ttf", "tahoma.ttf"],
            Script::Thaana => &["mvboli.ttf"],
            Script::Devanagari
            | Script::Bengali
            | Script::Gurmukhi
            | Script::Gujarati
            | Script::Odia
            | Script::Tamil
            | Script::Telugu
            | Script::Kannada
            | Script::Malayalam
            | Script::Sinhala => &["Nirmala.ttc", "Nirmala.ttf", "mangal.ttf", "vrinda.ttf", "raavi.ttf", "shruti.ttf", "kalinga.ttf", "latha.ttf", "gautami.ttf", "tunga.ttf", "kartika.ttf", "iskpota.ttf"],
            Script::Thai | Script::Lao | Script::Khmer => &["LeelawUI.ttf", "leelawad.ttf", "tahoma.ttf", "LaoUI.ttf", "KhmerUI.ttf"],
            Script::Myanmar => &["mmrtext.ttf"],
            Script::Ethiopic => &["ebrima.ttf", "nyala.ttf"],
            Script::Tibetan => &["himalaya.ttf"],
            Script::ChineseSimplified => &["msyh.ttc", "Deng.ttf", "simhei.ttf", "simsun.ttc"],
            Script::ChineseTraditional => &["msjh.ttc", "mingliub.ttc", "mingliu.ttc"],
            Script::Korean => &["malgun.ttf", "gulim.ttc", "batang.ttc"],
            _ => &[],
        };
        for n in names {
            v.push((f.join(n), 0));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let names: &[&str] = match script {
            Script::Arabic => &["/System/Library/Fonts/GeezaPro.ttc", "/System/Library/Fonts/SFArabic.ttf"],
            Script::Hebrew => &["/System/Library/Fonts/ArialHB.ttc", "/System/Library/Fonts/SFHebrew.ttf"],
            Script::Armenian => &["/System/Library/Fonts/SFArmenian.ttf", "/System/Library/Fonts/Supplemental/Mshtakan.ttc"],
            Script::Georgian => &["/System/Library/Fonts/SFGeorgian.ttf"],
            Script::Devanagari => &["/System/Library/Fonts/Kohinoor.ttc", "/System/Library/Fonts/Supplemental/DevanagariMT.ttc"],
            Script::Bengali => &["/System/Library/Fonts/KohinoorBangla.ttc", "/System/Library/Fonts/Supplemental/Bangla Sangam MN.ttc"],
            Script::Gujarati => &["/System/Library/Fonts/KohinoorGujarati.ttc", "/System/Library/Fonts/Supplemental/Gujarati Sangam MN.ttc"],
            Script::Telugu => &["/System/Library/Fonts/KohinoorTelugu.ttc", "/System/Library/Fonts/Supplemental/Telugu Sangam MN.ttc"],
            Script::Gurmukhi => &["/System/Library/Fonts/Supplemental/Gurmukhi Sangam MN.ttc"],
            Script::Odia => &["/System/Library/Fonts/Supplemental/Oriya Sangam MN.ttc"],
            Script::Tamil => &["/System/Library/Fonts/Supplemental/Tamil Sangam MN.ttc"],
            Script::Kannada => &["/System/Library/Fonts/Supplemental/Kannada Sangam MN.ttc"],
            Script::Malayalam => &["/System/Library/Fonts/Supplemental/Malayalam Sangam MN.ttc"],
            Script::Sinhala => &["/System/Library/Fonts/Supplemental/Sinhala Sangam MN.ttc"],
            Script::Thai => &["/System/Library/Fonts/Thonburi.ttc", "/System/Library/Fonts/Supplemental/Ayuthaya.ttf"],
            Script::Lao => &["/System/Library/Fonts/Supplemental/Lao Sangam MN.ttf"],
            Script::Khmer => &["/System/Library/Fonts/Supplemental/Khmer Sangam MN.ttf"],
            Script::Myanmar => &["/System/Library/Fonts/Supplemental/Myanmar Sangam MN.ttc"],
            Script::Ethiopic => &["/System/Library/Fonts/Supplemental/Kefa.ttc"],
            Script::Tibetan => &["/System/Library/Fonts/Supplemental/Kailasa.ttc"],
            Script::ChineseSimplified => &["/System/Library/Fonts/PingFang.ttc", "/System/Library/Fonts/Hiragino Sans GB.ttc", "/System/Library/Fonts/STHeiti Light.ttc"],
            Script::ChineseTraditional => &["/System/Library/Fonts/PingFang.ttc", "/System/Library/Fonts/STHeiti Light.ttc"],
            Script::Korean => &["/System/Library/Fonts/AppleSDGothicNeo.ttc"],
            _ => &[],
        };
        for n in names {
            v.push((PathBuf::from(n), 0));
        }
        v.push((PathBuf::from("/System/Library/Fonts/Supplemental/Arial Unicode.ttf"), 0));
        v.push((PathBuf::from("/Library/Fonts/Arial Unicode.ttf"), 0));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some((file, _)) = script_font_download(script) {
            for dir in ["/usr/share/fonts/truetype/noto", "/usr/share/fonts/noto", "/usr/share/fonts/google-noto", "/usr/share/fonts/opentype/noto"] {
                v.push((PathBuf::from(dir).join(file), 0));
            }
            if let Some(home) = std::env::var_os("HOME") {
                v.push((PathBuf::from(home).join(".local/share/fonts").join(file), 0));
            }
        }
        let lang = match script {
            Script::Arabic => "ar",
            Script::Hebrew => "he",
            Script::Armenian => "hy",
            Script::Georgian => "ka",
            Script::Thaana => "dv",
            Script::Devanagari => "hi",
            Script::Bengali => "bn",
            Script::Gurmukhi => "pa",
            Script::Gujarati => "gu",
            Script::Odia => "or",
            Script::Tamil => "ta",
            Script::Telugu => "te",
            Script::Kannada => "kn",
            Script::Malayalam => "ml",
            Script::Sinhala => "si",
            Script::Thai => "th",
            Script::Lao => "lo",
            Script::Khmer => "km",
            Script::Myanmar => "my",
            Script::Ethiopic => "am",
            Script::Tibetan => "bo",
            Script::ChineseSimplified => "zh-cn",
            Script::ChineseTraditional => "zh-tw",
            Script::Korean => "ko",
            _ => "",
        };
        if !lang.is_empty() {
            // Ask fontconfig which installed font covers the language.
            if let Ok(out) = std::process::Command::new("fc-match").args(["-f", "%{file}", &format!(":lang={lang}")]).output() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    v.push((PathBuf::from(p), 0));
                }
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every writing system in the language list has a font on this computer, or a download.
    #[test]
    fn every_script_has_a_font_or_a_download() {
        for l in crate::langs::LANGUAGES {
            let s = l.script;
            assert!(s.built_in() || script_font_download(s).is_some(), "{:?} has no font download", s);
        }
        #[cfg(target_os = "windows")]
        for l in crate::langs::LANGUAGES {
            if !l.script.built_in() && l.script != Script::Japanese {
                assert!(find_script_font(l.script).is_some(), "Windows has no font for {} ({:?})", l.english, l.script);
            }
        }
    }
}
