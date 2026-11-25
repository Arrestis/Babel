//! Catalogue of the local and online AI services BabelCraft can use for translation,
//! speech recognition and the Japanese voice, plus small helpers (opening a web page,
//! listing a service's models, downloading an Ollama model).

use crate::config::TranslationBackend;
use crate::util::{CancelToken, Cancelled, http_agent, read_body_string};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::BufRead;

/// A translation service: an AI chat model (local or online) or a translation API.
#[derive(Clone, Copy, Debug)]
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub backend: TranslationBackend,
    /// Runs on this computer (no account, works offline).
    pub local: bool,
    pub url: &'static str,
    pub model: &'static str,
    pub needs_key: bool,
    /// Where to get the app (local) or an API key (online).
    pub link: &'static str,
    /// Environment variables checked when no key is typed in.
    pub env: &'static [&'static str],
    /// What BabelCraft can use it for (shown on the Downloads page).
    pub uses: &'static str,
}

use TranslationBackend as B;

pub const PROVIDERS: &[Provider] = &[
    // ---- on this computer
    Provider { id: "brain", name: "BabelCraft offline AI brain (built in)", backend: B::BuiltinBrain, local: true, url: "", model: "", needs_key: false, link: "https://github.com/ggml-org/llama.cpp", env: &[], uses: "translation" },
    Provider { id: "ollama", name: "Ollama", backend: B::Ollama, local: true, url: "http://localhost:11434", model: "qwen2.5:7b", needs_key: false, link: "https://ollama.com/download", env: &[], uses: "translation" },
    Provider { id: "lmstudio", name: "LM Studio", backend: B::OpenAiCompatible, local: true, url: "http://localhost:1234/v1", model: "", needs_key: false, link: "https://lmstudio.ai/download", env: &[], uses: "translation" },
    Provider { id: "jan", name: "Jan", backend: B::OpenAiCompatible, local: true, url: "http://localhost:1337/v1", model: "", needs_key: false, link: "https://jan.ai/download", env: &[], uses: "translation" },
    Provider { id: "llamacpp", name: "llama.cpp server", backend: B::OpenAiCompatible, local: true, url: "http://localhost:8080/v1", model: "", needs_key: false, link: "https://github.com/ggml-org/llama.cpp/releases", env: &[], uses: "translation" },
    Provider { id: "libre", name: "LibreTranslate", backend: B::LibreTranslate, local: true, url: "http://localhost:5000", model: "", needs_key: false, link: "https://github.com/LibreTranslate/LibreTranslate", env: &["LIBRETRANSLATE_API_KEY"], uses: "translation" },
    // ---- online services (need an account and an API key)
    Provider { id: "openai", name: "OpenAI (ChatGPT)", backend: B::OpenAiCompatible, local: false, url: "https://api.openai.com/v1", model: "gpt-4o-mini", needs_key: true, link: "https://platform.openai.com/api-keys", env: &["OPENAI_API_KEY", "BABELCRAFT_OPENAI_API_KEY"], uses: "translation · speech recognition · voice" },
    Provider { id: "anthropic", name: "Anthropic Claude", backend: B::Anthropic, local: false, url: "https://api.anthropic.com/v1", model: "claude-opus-5-5", needs_key: true, link: "https://console.anthropic.com/settings/keys", env: &["ANTHROPIC_API_KEY"], uses: "translation" },
    Provider { id: "gemini", name: "Google Gemini", backend: B::OpenAiCompatible, local: false, url: "https://generativelanguage.googleapis.com/v1beta/openai", model: "gemini-2.5-flash", needs_key: true, link: "https://aistudio.google.com/apikey", env: &["GEMINI_API_KEY", "GOOGLE_API_KEY"], uses: "translation" },
    Provider { id: "deepl", name: "DeepL", backend: B::DeepL, local: false, url: "", model: "", needs_key: true, link: "https://www.deepl.com/pro-api", env: &["DEEPL_AUTH_KEY", "BABELCRAFT_DEEPL_KEY"], uses: "translation" },
    Provider { id: "openrouter", name: "OpenRouter (hundreds of models)", backend: B::OpenAiCompatible, local: false, url: "https://openrouter.ai/api/v1", model: "openai/gpt-4o-mini", needs_key: true, link: "https://openrouter.ai/keys", env: &["OPENROUTER_API_KEY"], uses: "translation" },
    Provider { id: "groq", name: "Groq", backend: B::OpenAiCompatible, local: false, url: "https://api.groq.com/openai/v1", model: "llama-3.3-70b-versatile", needs_key: true, link: "https://console.groq.com/keys", env: &["GROQ_API_KEY"], uses: "translation · speech recognition" },
    Provider { id: "deepseek", name: "DeepSeek", backend: B::OpenAiCompatible, local: false, url: "https://api.deepseek.com/v1", model: "deepseek-chat", needs_key: true, link: "https://platform.deepseek.com/api_keys", env: &["DEEPSEEK_API_KEY"], uses: "translation" },
    Provider { id: "mistral", name: "Mistral AI", backend: B::OpenAiCompatible, local: false, url: "https://api.mistral.ai/v1", model: "mistral-small-latest", needs_key: true, link: "https://console.mistral.ai/api-keys", env: &["MISTRAL_API_KEY"], uses: "translation" },
    // ---- anything else that speaks the OpenAI API (vLLM, LocalAI, a company server…)
    Provider { id: "custom", name: "Other OpenAI-compatible server", backend: B::OpenAiCompatible, local: false, url: "", model: "", needs_key: false, link: "", env: &["OPENAI_API_KEY", "BABELCRAFT_OPENAI_API_KEY"], uses: "translation" },
];

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// The preset a backend + address most likely belongs to (used for settings saved by older versions).
pub fn guess_provider(backend: TranslationBackend, url: &str) -> &'static str {
    if backend != TranslationBackend::OpenAiCompatible {
        return PROVIDERS.iter().find(|p| p.backend == backend).map(|p| p.id).unwrap_or("brain");
    }
    let url = url.trim().trim_end_matches('/');
    PROVIDERS
        .iter()
        .find(|p| p.backend == backend && p.id != "custom" && p.url.trim_end_matches('/').eq_ignore_ascii_case(url))
        .map(|p| p.id)
        .unwrap_or("custom")
}

/// Online speech recognition services (Whisper API compatible: /audio/transcriptions).
#[derive(Clone, Copy, Debug)]
pub struct SpeechService {
    pub id: &'static str,
    pub name: &'static str,
    pub local: bool,
    pub url: &'static str,
    pub model: &'static str,
    pub link: &'static str,
}

pub const SPEECH_SERVICES: &[SpeechService] = &[
    SpeechService { id: "openai", name: "OpenAI Whisper API", local: false, url: "https://api.openai.com/v1", model: "whisper-1", link: "https://platform.openai.com/api-keys" },
    SpeechService { id: "groq", name: "Groq (very fast Whisper)", local: false, url: "https://api.groq.com/openai/v1", model: "whisper-large-v3-turbo", link: "https://console.groq.com/keys" },
    SpeechService { id: "whisper-server", name: "Whisper server on this computer (Speaches…)", local: true, url: "http://localhost:8000/v1", model: "Systran/faster-whisper-small.en", link: "https://github.com/speaches-ai/speaches" },
];

pub fn speech_service(id: &str) -> &'static SpeechService {
    SPEECH_SERVICES.iter().find(|s| s.id == id).unwrap_or(&SPEECH_SERVICES[0])
}

/// Ready-made settings for OpenAI-compatible voice servers (/audio/speech).
pub struct VoicePreset {
    pub name: &'static str,
    pub url: &'static str,
    pub model: &'static str,
    pub voice: &'static str,
    pub link: &'static str,
}

pub const VOICE_PRESETS: &[VoicePreset] = &[
    VoicePreset { name: "Kokoro-FastAPI (on this computer)", url: "http://localhost:8880/v1", model: "kokoro", voice: "jf_alpha", link: "https://github.com/remsky/Kokoro-FastAPI" },
    VoicePreset { name: "OpenAI voices (online)", url: "https://api.openai.com/v1", model: "gpt-4o-mini-tts", voice: "alloy", link: "https://platform.openai.com/api-keys" },
];

/// A web page about an optional helper app (shown on the Downloads page).
pub struct Link {
    pub name: &'static str,
    pub what: &'static str,
    pub url: &'static str,
}

pub const OPTIONAL_APPS: &[Link] = &[
    Link { name: "Ollama", what: "Runs many open AI models on your computer (translation).", url: "https://ollama.com/download" },
    Link { name: "LM Studio", what: "Desktop app for downloading and running AI models (translation).", url: "https://lmstudio.ai/download" },
    Link { name: "Jan", what: "Free offline ChatGPT-style app with a built-in server (translation).", url: "https://jan.ai/download" },
    Link { name: "VOICEVOX", what: "Free Japanese voice app with many characters (voice).", url: "https://voicevox.hiroshiba.jp/" },
    Link { name: "Kokoro-FastAPI", what: "Small local voice server with Japanese voices (voice).", url: "https://github.com/remsky/Kokoro-FastAPI" },
    Link { name: "Piper", what: "Fast offline voice program; needs a Japanese voice model (voice).", url: "https://github.com/rhasspy/piper" },
    Link { name: "edge-tts", what: "Microsoft's online neural voices, no key needed (voice).", url: "https://github.com/rany2/edge-tts" },
    Link { name: "LibreTranslate", what: "Free translation server you can run yourself (translation).", url: "https://github.com/LibreTranslate/LibreTranslate" },
    Link { name: "Speaches", what: "Whisper speech-recognition server for your computer (speech).", url: "https://github.com/speaches-ai/speaches" },
    Link { name: "Tesseract OCR", what: "Alternative text reader for on-screen text (OCR).", url: "https://github.com/tesseract-ocr/tesseract" },
];

/// Opens a web page in the person's browser.
pub fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    let _ = crate::util::silent_command("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// The models a service offers (for the model picker).
pub fn list_models(backend: TranslationBackend, url: &str, key: &str) -> Result<Vec<String>> {
    let agent = http_agent(20);
    let base = url.trim().trim_end_matches('/');
    let (mut resp, field) = match backend {
        TranslationBackend::Ollama => (agent.get(format!("{base}/api/tags")).call().with_context(|| format!("could not reach Ollama at {base}"))?, "models"),
        TranslationBackend::Anthropic => {
            if key.is_empty() {
                bail!("enter your Anthropic API key first");
            }
            let r = agent
                .get("https://api.anthropic.com/v1/models?limit=100")
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .call()
                .context("could not reach Anthropic")?;
            (r, "data")
        }
        TranslationBackend::OpenAiCompatible => {
            if base.is_empty() {
                bail!("enter the server address first");
            }
            let mut req = agent.get(format!("{base}/models"));
            if !key.is_empty() {
                req = req.header("Authorization", format!("Bearer {key}"));
            }
            (req.call().with_context(|| format!("could not reach {base}"))?, "data")
        }
        _ => bail!("this service has no model list"),
    };
    let status = resp.status();
    let body = read_body_string(&mut resp)?;
    if !status.is_success() {
        bail!("{status}: {}", body.chars().take(300).collect::<String>());
    }
    let v: Value = serde_json::from_str(&body).context("unexpected reply")?;
    let mut names: Vec<String> = v[field]
        .as_array()
        .ok_or_else(|| anyhow!("unexpected reply"))?
        .iter()
        .filter_map(|m| m["id"].as_str().or_else(|| m["name"].as_str()).or_else(|| m["model"].as_str()))
        .map(|s| s.trim_start_matches("models/").to_string())
        .collect();
    names.sort();
    names.dedup();
    if names.is_empty() {
        bail!("the service lists no models (load one in the app first)");
    }
    Ok(names)
}

/// Downloads a model into Ollama (`ollama pull`), reporting progress.
pub fn ollama_pull(url: &str, model: &str, cancel: &CancelToken, progress: &mut dyn FnMut(&str, f32)) -> Result<()> {
    let base = url.trim().trim_end_matches('/');
    let agent: ureq::Agent = crate::util::agent_config().http_status_as_error(false).build().into();
    let resp = agent
        .post(format!("{base}/api/pull"))
        .send_json(json!({ "model": model.trim(), "stream": true }))
        .with_context(|| format!("could not reach Ollama at {base} — is Ollama installed and running?"))?;
    let ok = resp.status().is_success();
    let reader = std::io::BufReader::new(resp.into_body().into_reader());
    for line in reader.lines() {
        if cancel.is_cancelled() {
            bail!(Cancelled);
        }
        let line = line?;
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(e) = v["error"].as_str() {
            bail!("Ollama: {e}");
        }
        let status = v["status"].as_str().unwrap_or("");
        let f = match (v["completed"].as_f64(), v["total"].as_f64()) {
            (Some(c), Some(t)) if t > 0.0 => (c / t) as f32,
            _ => -1.0,
        };
        progress(&format!("Ollama: {model} — {status}"), f);
    }
    if !ok {
        bail!("Ollama could not download {model}");
    }
    Ok(())
}

/// Models already in Ollama (None when Ollama isn't running).
pub fn ollama_models(url: &str) -> Option<Vec<String>> {
    let agent = http_agent(2);
    let mut r = agent.get(format!("{}/api/tags", url.trim().trim_end_matches('/'))).call().ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v: Value = serde_json::from_str(&read_body_string(&mut r).ok()?).ok()?;
    Some(v["models"].as_array()?.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect())
}
