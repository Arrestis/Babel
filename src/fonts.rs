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
