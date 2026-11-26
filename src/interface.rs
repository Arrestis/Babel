// SPDX-License-Identifier: GPL-3.0-or-later
//! Puts BabelCraft's own buttons and menus into any language: downloads a font for the
//! language's writing system when the computer has none, and has the translator chosen in
//! Settings (the offline AI brain unless something else is picked) translate the interface
//! text, which is then saved so the switch is instant next time.

use crate::config::{Config, TranslationBackend};
use crate::i18n;
use crate::langs::Lang;
use crate::tools::{self, Component, Progress};
use crate::translate::{TextKind, Translator};
use crate::util::CancelToken;
use anyhow::{Context, Result};

/// Whether `lang` still needs a font download and/or a translation before the interface can use it.
pub fn needs(lang: &Lang) -> (bool, bool) {
    let font = !lang.script.built_in() && crate::fonts::find_script_font(lang.script).is_none();
    let text = i18n::availability(lang.code) == i18n::Availability::NotYet;
    (font, text)
}

/// Gets everything ready for showing the interface in `lang` (`again`: translate it anew even
/// if a translation was saved before). Returns a short summary for the log.
pub fn prepare(cfg: &Config, lang: &'static Lang, again: bool, cancel: &CancelToken, progress: Progress) -> Result<String> {
    let (font, text) = needs(lang);
    if font {
        let (file, url) = crate::fonts::script_font_download(lang.script).context("no font download for this writing system")?;
        tools::fetch(url, &crate::config::fonts_dir().join(file), &format!("Font for {}", lang.english), cancel, progress)?;
    }
    if text || (again && i18n::availability(lang.code) != i18n::Availability::BuiltIn) {
        let n = translate(cfg, lang, cancel, progress)?;
        return Ok(format!("{} ({n} texts translated by BabelCraft's AI)", lang.native));
    }
    Ok(lang.native.to_string())
}

/// Translates every interface text into `lang` and saves the result. Returns how many texts it covers.
fn translate(cfg: &Config, lang: &'static Lang, cancel: &CancelToken, progress: Progress) -> Result<usize> {
    // The offline AI brain is downloaded the first time it's needed (about 2.5 GB).
    if cfg.translation.backend == TranslationBackend::BuiltinBrain {
        if !crate::brain::engine_ready(cfg.translation.brain_gpu) && crate::brain::engine_installable() {
            tools::install(Component::BrainEngine, cfg, cancel, progress)?;
        }
        if !crate::brain::model_ready(&cfg.translation.brain_model) {
            tools::install(Component::BrainModel, cfg, cancel, progress)?;
        }
    }
    crate::translate::preflight(cfg)?;
    let english = i18n::english_texts();
    let texts: Vec<String> = english.iter().map(|s| s.to_string()).collect();
    let label = format!("Translating BabelCraft into {}", lang.native);
    let mut translator = Translator::new(cfg);
    let _running = translator.prepare(cancel, &mut |s| progress(&s, -1.0))?;
    progress(&label, 0.0);
    let total = texts.len().max(1);
    let out = translator.translate_all(&texts, TextKind::Interface(lang), cancel, |n| progress(&label, n as f32 / total as f32), |_| {})?;
    let made_by = crate::ai::provider(&cfg.translation.provider).map(|p| p.name).unwrap_or("its translator");
    let n = i18n::save_ai_translation(lang.code, &english, &out, made_by)?;
    if n * 2 < english.len() {
        anyhow::bail!("the translator returned only {n} of {} texts — try again, or choose another translator in Settings → Translation", english.len());
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Translates the whole interface into Indonesian with the offline AI brain (downloads it
    /// into a scratch folder if needed) and checks the saved result.
    /// Run with: BABELCRAFT_DATA_DIR=target/testdata cargo test interface_into_any_language -- --ignored --nocapture
    #[test]
    #[ignore]
    fn interface_into_any_language() {
        let data = std::env::var("BABELCRAFT_DATA_DIR").expect("set BABELCRAFT_DATA_DIR to a scratch folder");
        assert!(data.contains("target"), "use a scratch folder inside target/");
        let code = std::env::var("BABELCRAFT_TEST_UI_LANG").unwrap_or_else(|_| "id".into());
        let lang = crate::langs::find(&code).expect("language code");
        let _ = std::fs::remove_file(i18n::saved_file(lang.code));
        let cfg = Config::default();
        let t0 = std::time::Instant::now();
        let mut last = String::new();
        let mut progress = |what: &str, f: f32| {
            if what != last {
                println!("{what}");
                last = what.to_string();
            }
            let _ = f;
        };
        let summary = prepare(&cfg, lang, false, &CancelToken::default(), &mut progress).unwrap();
        println!("{summary} in {:.0} s", t0.elapsed().as_secs_f64());
        assert_eq!(i18n::availability(lang.code), i18n::Availability::Saved);
        let saved = std::fs::read_to_string(i18n::saved_file(lang.code)).unwrap();
        let lines = saved.lines().filter(|l| !l.starts_with('#')).count();
        assert!(lines * 100 >= i18n::english_texts().len() * 90, "only {lines} texts translated");
        for l in saved.lines().filter(|l| !l.starts_with('#')).take(12) {
            println!("  {l}");
        }
    }
}
