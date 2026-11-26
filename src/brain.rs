// SPDX-License-Identifier: GPL-3.0-or-later
//! The offline AI brain: llama.cpp's `llama-server` plus an open AI model (GGUF file) that
//! BabelCraft downloads into its own folder and runs on this computer. No account, no key,
//! and no internet once it is downloaded. BabelCraft talks to it like any OpenAI-compatible server.

use crate::config::{self, Config};
use crate::services;
use crate::tools::{self, Progress};
use crate::util::CancelToken;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::time::Duration;

pub const SERVICE: &str = "AI brain";

pub struct BrainModel {
    pub id: &'static str,
    pub name: &'static str,
    pub size: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    /// The model's own page (licence, details).
    pub page: &'static str,
    pub licence: &'static str,
    pub note: &'static str,
}

pub const MODELS: &[BrainModel] = &[
    BrainModel {
        id: "qwen2.5-1.5b",
        name: "Qwen 2.5 · 1.5B (small and fast)",
        size: "1.0 GB",
        file: "Qwen2.5-1.5B-Instruct-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/Qwen2.5-1.5B-Instruct-Q4_K_M.gguf",
        page: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct",
        licence: "Apache-2.0",
        note: "For older computers or 4 GB of memory. Simpler Japanese.",
    },
    BrainModel {
        id: "qwen3-4b",
        name: "Qwen 3 · 4B (recommended)",
        size: "2.5 GB",
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        page: "https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507",
        licence: "Apache-2.0",
        note: "Good natural Japanese; runs on most computers with 8 GB of memory.",
    },
    BrainModel {
        id: "gemma3-4b",
        name: "Gemma 3 · 4B (alternative)",
        size: "2.5 GB",
        file: "google_gemma-3-4b-it-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/google_gemma-3-4b-it-GGUF/resolve/main/google_gemma-3-4b-it-Q4_K_M.gguf",
        page: "https://huggingface.co/google/gemma-3-4b-it",
        licence: "Gemma Terms of Use",
        note: "Google's open model; a different style of Japanese to compare with Qwen.",
    },
    BrainModel {
        id: "qwen2.5-7b",
        name: "Qwen 2.5 · 7B (best quality)",
        size: "4.7 GB",
        file: "Qwen2.5-7B-Instruct-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/Qwen2.5-7B-Instruct-GGUF/resolve/main/Qwen2.5-7B-Instruct-Q4_K_M.gguf",
        page: "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct",
        licence: "Apache-2.0",
        note: "Best translations; wants 16 GB of memory or a graphics card.",
    },
];

pub const DEFAULT_MODEL: &str = "qwen3-4b";

pub fn model(id: &str) -> &'static BrainModel {
    MODELS.iter().find(|m| m.id == id).unwrap_or(&MODELS[1])
}

pub fn engine_dir() -> PathBuf {
    tools::tools_dir().join("llama")
}

pub fn engine_exe() -> Option<PathBuf> {
    let p = engine_dir().join(tools::exe_name("llama-server"));
    p.is_file().then_some(p)
}

/// The graphics-card build is only used on Windows and Linux (macOS builds always use Metal).
fn wants_gpu_build(gpu: bool) -> bool {
    gpu && !cfg!(target_os = "macos")
}

fn variant_file() -> PathBuf {
    engine_dir().join("babelcraft-variant.txt")
}

/// The engine is there and is the right kind (graphics card or processor only).
pub fn engine_ready(gpu: bool) -> bool {
    if engine_exe().is_none() {
        return false;
    }
    let want = if wants_gpu_build(gpu) { "gpu" } else { "cpu" };
    std::fs::read_to_string(variant_file()).map(|v| v.trim() == want).unwrap_or(true)
}

pub fn model_file(id: &str) -> PathBuf {
    config::models_dir().join("brain").join(model(id).file)
}

pub fn model_ready(id: &str) -> bool {
    model_file(id).is_file()
}

/// Which llama.cpp download fits this computer.
fn engine_asset_suffix(gpu: bool) -> Option<&'static str> {
    let g = wants_gpu_build(gpu);
    let os64 = std::env::var("PROCESSOR_ARCHITEW6432").is_ok();
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => if g { "-bin-win-vulkan-x64.zip" } else { "-bin-win-cpu-x64.zip" },
        // 32-bit BabelCraft on 64-bit Windows runs the 64-bit engine.
        ("windows", "x86") if os64 => if g { "-bin-win-vulkan-x64.zip" } else { "-bin-win-cpu-x64.zip" },
        ("windows", "aarch64") => if g { "-bin-win-vulkan-arm64.zip" } else { "-bin-win-cpu-arm64.zip" },
        ("linux", "x86_64") => if g { "-bin-ubuntu-vulkan-x64.tar.gz" } else { "-bin-ubuntu-x64.tar.gz" },
        ("linux", "aarch64") => if g { "-bin-ubuntu-vulkan-arm64.tar.gz" } else { "-bin-ubuntu-arm64.tar.gz" },
        ("macos", "aarch64") => "-bin-macos-arm64.tar.gz",
        ("macos", "x86_64") => "-bin-macos-x64.tar.gz",
        _ => return None,
    })
}

pub fn engine_installable() -> bool {
    engine_asset_suffix(false).is_some()
}

/// llama.cpp publishes its programs in nightly builds; its newest stable release names the
/// nightly build that matches it (nightly-tag.txt). Prefer that build.
fn stable_engine_files(wanted: &dyn Fn(&str) -> bool) -> Result<Vec<(String, String)>> {
    let latest = tools::github_json("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest")?;
    let (_, pointer) = tools::release_files(&latest, &|n: &str| n == "nightly-tag.txt").into_iter().next().context("no nightly pointer")?;
    let mut resp = crate::util::http_agent(30).get(&pointer).call()?;
    let tag = crate::util::read_body_string(&mut resp)?.trim().to_string();
    if tag.is_empty() || !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        bail!("unexpected nightly tag");
    }
    let release = tools::github_json(&format!("https://api.github.com/repos/ggml-org/llama.cpp/releases/tags/{tag}"))?;
    Ok(tools::release_files(&release, wanted))
}

pub fn install_engine(gpu: bool, cancel: &CancelToken, progress: Progress) -> Result<()> {
    let suffix = engine_asset_suffix(gpu).context(
        "no ready-made AI brain engine for this computer — install Ollama or LM Studio instead, or use an online service",
    )?;
    progress("Looking up the AI brain engine (llama.cpp)", -1.0);
    let wanted = |n: &str| n.starts_with("llama-b") && n.ends_with(suffix);
    let assets = match stable_engine_files(&wanted) {
        Ok(v) if !v.is_empty() => v,
        _ => tools::github_assets("ggml-org/llama.cpp", true, wanted)?,
    };
    let (name, url) = assets.into_iter().next().context("llama.cpp download not found")?;
    let tmp = tools::tools_dir().join("_download_llama");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let file = tmp.join(&name);
    tools::fetch(&url, &file, "AI brain engine (llama.cpp)", cancel, progress)?;
    progress("Unpacking the AI brain engine", -1.0);
    let unpack = tmp.join("unpacked");
    tools::extract(&file, &unpack)?;
    let exe = tools::find_file(&unpack, &tools::exe_name("llama-server")).context("llama-server not found in the download")?;
    let folder = exe.parent().unwrap_or(&unpack).to_path_buf();
    let dest = engine_dir();
    let _ = std::fs::remove_dir_all(&dest);
    tools::copy_dir(&folder, &dest)?;
    if let Some(parent) = folder.parent() {
        for lib in ["lib", "lib64"] {
            let l = parent.join(lib);
            if l.is_dir() {
                tools::copy_dir(&l, &dest)?;
            }
        }
    }
    for entry in std::fs::read_dir(&dest)?.flatten() {
        tools::make_executable(&entry.path());
    }
    std::fs::write(variant_file(), if wants_gpu_build(gpu) { "gpu" } else { "cpu" })?;
    let _ = std::fs::remove_dir_all(&tmp);
    if engine_exe().is_none() {
        bail!("the AI brain engine could not be set up");
    }
    Ok(())
}

pub fn install_model(id: &str, cancel: &CancelToken, progress: Progress) -> Result<()> {
    let m = model(id);
    let dest = model_file(m.id);
    if dest.is_file() {
        return Ok(());
    }
    tools::fetch(m.url, &dest, &format!("AI brain model {}", m.name), cancel, progress)
}

pub fn remove_model(id: &str) -> Result<()> {
    let p = model_file(id);
    if p.is_file() {
        std::fs::remove_file(&p).with_context(|| format!("could not delete {}", p.display()))?;
    }
    Ok(())
}

/// Starts the brain (if it isn't running) and returns its OpenAI-compatible address.
pub fn start(cfg: &Config, cancel: &CancelToken, status: &mut dyn FnMut(String)) -> Result<String> {
    let exe = engine_exe().context("The offline AI brain is not downloaded yet — open \"AI models & downloads\" and press Download")?;
    let m = model(&cfg.translation.brain_model);
    let file = model_file(m.id);
    if !file.is_file() {
        bail!("The AI brain model {} is not downloaded yet — open \"AI models & downloads\" and press Download", m.name);
    }
    let port = services::free_port();
    let base = format!("http://127.0.0.1:{port}");
    let mut args: Vec<String> = vec![
        "-m".into(),
        file.display().to_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        "4096".into(),
        "-np".into(),
        "1".into(),
    ];
    if wants_gpu_build(cfg.translation.brain_gpu) || cfg!(target_os = "macos") {
        args.extend(["-ngl".into(), "99".into()]);
    }
    status(format!("Starting the offline AI brain ({})…", m.name));
    services::start(SERVICE, &exe, &args, &base, "/health", Duration::from_secs(600), cancel)?;
    status("The offline AI brain is ready.".into());
    Ok(format!("{base}/v1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloads the llama.cpp engine and a model into a scratch folder, starts the brain,
    /// translates a few lines, and stops it again.
    /// Run with: BABELCRAFT_DATA_DIR=target/testdata cargo test offline_brain -- --ignored --nocapture
    /// (BABELCRAFT_TEST_BRAIN=qwen2.5-1.5b picks another model.)
    #[test]
    #[ignore]
    fn offline_brain_translates() {
        let data = std::env::var("BABELCRAFT_DATA_DIR").expect("set BABELCRAFT_DATA_DIR to a scratch folder");
        assert!(data.contains("target"), "use a scratch folder inside target/");
        let id = std::env::var("BABELCRAFT_TEST_BRAIN").unwrap_or_else(|_| DEFAULT_MODEL.into());
        let mut cfg = Config::default();
        cfg.translation.brain_model = id.clone();
        let cancel = CancelToken::default();
        let mut progress = |what: &str, f: f32| {
            if f == 0.0 || f < 0.0 {
                println!("setup: {what}");
            }
        };
        if !engine_ready(false) {
            install_engine(false, &cancel, &mut progress).unwrap();
        }
        install_model(&id, &cancel, &mut progress).unwrap();
        assert!(crate::tools::missing(&cfg).iter().all(|c| !matches!(c, crate::tools::Component::BrainEngine | crate::tools::Component::BrainModel)));

        let mut tr = crate::translate::Translator::new(&cfg);
        let t0 = std::time::Instant::now();
        let guard = tr.prepare(&cancel, &mut |m| println!("{m}")).unwrap();
        println!("brain ready after {:.1} s", t0.elapsed().as_secs_f32());
        let texts: Vec<String> = [
            "Welcome to the course",
            "Welcome to the course. Today we will learn how to cook perfect rice.",
            "Thanks for watching, see you next time!",
        ]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let ja = tr.translate_all(&texts, crate::translate::TextKind::Speech, &cancel, |_| {}, |w| println!("warn: {w}")).unwrap();
        println!("translated in {:.1} s", t0.elapsed().as_secs_f32());
        for (e, j) in texts.iter().zip(&ja) {
            println!("  {e}  →  {j}");
        }
        let japanese = |s: &str| s.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{4e00}'..='\u{9fff}').contains(&c));
        assert!(ja.iter().all(|j| japanese(j)), "every line should come back in Japanese");
        drop(guard);
        assert!(crate::services::free_port() > 0);
    }
}
