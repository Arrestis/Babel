// SPDX-License-Identifier: GPL-3.0-or-later
//! Piper — free, offline voices for about forty languages (https://github.com/rhasspy/piper).
//! When a video is dubbed into a language VOICEVOX doesn't speak, BabelCraft downloads the Piper
//! program and one voice for that language into its own folder (once), and runs it hidden.
//! Nothing is ever played through the speakers; Piper writes WAV files.

use crate::langs::Lang;
use crate::tools::{self, Progress};
use crate::util::CancelToken;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const RELEASE: &str = "https://github.com/rhasspy/piper/releases/download/2023.11.14-2";
const VOICES: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main";
pub const VOICES_PAGE: &str = "https://huggingface.co/rhasspy/piper-voices";

/// Languages Piper has voices for (used before the voice list has been downloaded).
const KNOWN_FAMILIES: &[&str] = &[
    "ar", "ca", "cs", "cy", "da", "de", "el", "en", "es", "fa", "fi", "fr", "hu", "is", "it", "ka", "kk", "lb", "lv", "ml", "ne", "nl",
    "no", "pl", "pt", "ro", "ru", "sk", "sl", "sr", "sv", "sw", "te", "tr", "uk", "vi", "zh",
];

fn asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("piper_windows_amd64.zip"),
        ("linux", "x86_64") => Some("piper_linux_x86_64.tar.gz"),
        ("linux", "aarch64") => Some("piper_linux_aarch64.tar.gz"),
        ("macos", "x86_64") => Some("piper_macos_x64.tar.gz"),
        ("macos", "aarch64") => Some("piper_macos_aarch64.tar.gz"),
        _ => None,
    }
}

/// A ready-made Piper exists for this computer.
pub fn installable() -> bool {
    asset().is_some()
}

pub fn engine_dir() -> PathBuf {
    tools::tools_dir().join("piper")
}

/// BabelCraft's own Piper program, once downloaded.
pub fn engine_exe() -> Option<PathBuf> {
    tools::find_file(&engine_dir(), &tools::exe_name("piper"))
}

pub fn install_engine(cancel: &CancelToken, progress: Progress) -> Result<()> {
    let asset = asset().context("no ready-made Piper for this computer")?;
    let tmp = tools::tools_dir().join("_download_piper");
    std::fs::create_dir_all(&tmp)?;
    let file = tmp.join(asset);
    tools::fetch(&format!("{RELEASE}/{asset}"), &file, "Piper voice program", cancel, progress)?;
    progress("Unpacking Piper", -1.0);
    let dest = engine_dir();
    let _ = std::fs::remove_dir_all(&dest);
    tools::extract(&file, &dest)?;
    let exe = engine_exe().context("piper not found in the download")?;
    tools::make_executable(&exe);
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

/// Where downloaded Piper voices live.
pub fn voices_dir() -> PathBuf {
    crate::config::models_dir().join("piper")
}

/// The code Piper groups a language's voices under ("zh" for both kinds of Chinese).
fn family(lang: &Lang) -> &'static str {
    lang.code.split('-').next().unwrap_or(lang.code)
}

/// A voice for `lang` that is already downloaded.
pub fn installed_voice(lang: &Lang) -> Option<PathBuf> {
    let fam = family(lang);
    let mut found: Vec<PathBuf> = std::fs::read_dir(voices_dir())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "onnx"))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().split(['_', '-']).next() == Some(fam)))
        .filter(|p| p.with_extension("onnx.json").is_file())
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Piper has (or probably has) a voice for `lang`.
pub fn has_voice_for(lang: &Lang) -> bool {
    if installed_voice(lang).is_some() {
        return true;
    }
    match cached_catalogue() {
        Some(cat) => best_voice(&cat, lang).is_some(),
        None => KNOWN_FAMILIES.contains(&family(lang)),
    }
}

fn catalogue_file() -> PathBuf {
    voices_dir().join("voices.json")
}

fn cached_catalogue() -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(catalogue_file()).ok()?).ok()
}

/// The list of every Piper voice (downloaded again after 30 days).
fn catalogue(cancel: &CancelToken) -> Result<serde_json::Value> {
    let path = catalogue_file();
    let fresh = path
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < 30 * 86_400);
    if !fresh {
        std::fs::create_dir_all(voices_dir())?;
        let _ = std::fs::remove_file(&path);
        if let Err(e) = crate::util::download(&format!("{VOICES}/voices.json"), &path, cancel, |_, _| {}) {
            if !path.is_file() {
                return Err(e.context("could not download the list of Piper voices"));
            }
        }
    }
    cached_catalogue().context("the list of Piper voices could not be read")
}

/// The files of one voice in the voices repository.
struct VoiceFiles {
    key: String,
    onnx: String,
    json: String,
}

/// The best voice for `lang`: medium quality and a single speaker preferred, plus a few
/// well-known natural voices.
fn best_voice(cat: &serde_json::Value, lang: &Lang) -> Option<VoiceFiles> {
    let fam = family(lang);
    let region = match lang.code {
        "en" => "en_US",
        "es" => "es_ES",
        "pt" => "pt_BR",
        "fr" => "fr_FR",
        "de" => "de_DE",
        "nl" => "nl_NL",
        "zh-Hans" | "zh-Hant" => "zh_CN",
        _ => "",
    };
    const FAVOURITES: &[&str] = &[
        "en_US-lessac-medium",
        "de_DE-thorsten-medium",
        "fr_FR-siwis-medium",
        "es_ES-davefx-medium",
        "pt_BR-faber-medium",
        "ru_RU-irina-medium",
        "zh_CN-huayan-medium",
        "it_IT-paola-medium",
    ];
    let mut best: Option<(i32, VoiceFiles)> = None;
    for (key, v) in cat.as_object()? {
        if v["language"]["family"].as_str() != Some(fam) {
            continue;
        }
        let mut score = match v["quality"].as_str().unwrap_or("") {
            "medium" => 30,
            "high" => 25,
            "low" => 15,
            "x_low" => 5,
            _ => 0,
        };
        if v["num_speakers"].as_u64().unwrap_or(1) == 1 {
            score += 10;
        }
        if !region.is_empty() && v["language"]["code"].as_str() == Some(region) {
            score += 8;
        }
        if FAVOURITES.contains(&key.as_str()) {
            score += 50;
        }
        let Some(files) = v["files"].as_object() else { continue };
        let onnx = files.keys().find(|p| p.ends_with(".onnx"));
        let json = files.keys().find(|p| p.ends_with(".onnx.json"));
        if let (Some(onnx), Some(json)) = (onnx, json) {
            let better = match &best {
                None => true,
                Some((s, b)) => score > *s || (score == *s && key.as_str() < b.key.as_str()),
            };
            if better {
                best = Some((score, VoiceFiles { key: key.clone(), onnx: onnx.clone(), json: json.clone() }));
            }
        }
    }
    best.map(|(_, v)| v)
}

/// Downloads the best voice for `lang` (unless one is already there) and returns its model file.
pub fn install_voice(lang: &Lang, cancel: &CancelToken, progress: Progress) -> Result<PathBuf> {
    if let Some(p) = installed_voice(lang) {
        return Ok(p);
    }
    progress(&format!("Looking up a Piper voice for {}", lang.english), -1.0);
    let cat = catalogue(cancel)?;
    let v = best_voice(&cat, lang).with_context(|| format!("Piper has no voice for {}", lang.english))?;
    let dir = voices_dir();
    let name = |p: &str| Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let model = dir.join(name(&v.onnx));
    tools::fetch(&format!("{VOICES}/{}", v.json), &dir.join(name(&v.json)), &format!("Voice for {} (Piper {})", lang.english, v.key), cancel, progress)?;
    tools::fetch(&format!("{VOICES}/{}", v.onnx), &model, &format!("Voice for {} (Piper {})", lang.english, v.key), cancel, progress)?;
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloads Piper and a Spanish voice into a scratch folder and speaks one line into a file.
    /// Run with: BABELCRAFT_DATA_DIR=target/testdata cargo test piper_speaks -- --ignored --nocapture
    #[test]
    #[ignore]
    fn piper_speaks() {
        let data = std::env::var("BABELCRAFT_DATA_DIR").expect("set BABELCRAFT_DATA_DIR to a scratch folder");
        assert!(data.contains("target"), "use a scratch folder inside target/");
        let code = std::env::var("BABELCRAFT_TEST_VOICE_LANG").unwrap_or_else(|_| "es".into());
        let lang = crate::langs::find(&code).unwrap();
        let cancel = CancelToken::default();
        let mut progress = |what: &str, f: f32| {
            if f <= 0.0 {
                println!("{what}");
            }
        };
        if engine_exe().is_none() {
            install_engine(&cancel, &mut progress).unwrap();
        }
        let model = install_voice(lang, &cancel, &mut progress).unwrap();
        println!("piper {}\nvoice {}", engine_exe().unwrap().display(), model.display());
        let out = std::path::absolute("target/e2e").unwrap().join(format!("piper_{code}.wav"));
        let _ = std::fs::create_dir_all(out.parent().unwrap());
        let mut cmd = crate::util::silent_command(engine_exe().unwrap());
        cmd.arg("--model").arg(&model).arg("--output_file").arg(&out);
        crate::util::run_process(cmd, &cancel, Some("Hola, bienvenidos al curso de cocina."), |_, _| {}).unwrap().ok_or_err("piper").unwrap();
        let len = crate::tts::wav_duration(&out);
        println!("wrote {} ({len:.1} s)", out.display());
        assert!(len > 1.0);
    }
}
