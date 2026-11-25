//! Persisted settings (TOML in the platform config directory).
//!
//! Windows: %APPDATA%\BabelCraft\babelcraft\config\config.toml
//! Linux:   ~/.config/babelcraft/config.toml
//! macOS:   ~/Library/Application Support/org.BabelCraft.babelcraft/config.toml

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TranslationBackend {
    /// BabelCraft's own offline AI brain (llama.cpp + a downloaded open model).
    BuiltinBrain,
    /// Local Ollama server (offline).
    Ollama,
    /// Any OpenAI-compatible chat endpoint: OpenAI, Gemini, OpenRouter, Groq, DeepSeek, Mistral,
    /// or local apps (LM Studio, Jan, llama.cpp server, vLLM...).
    OpenAiCompatible,
    /// Anthropic Claude (its own API).
    Anthropic,
    /// DeepL API (free or pro key).
    DeepL,
    /// LibreTranslate server (run it yourself, or a hosted one).
    LibreTranslate,
}

impl TranslationBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::BuiltinBrain => "Offline AI brain",
            Self::Ollama => "Ollama (local, offline)",
            Self::OpenAiCompatible => "OpenAI-compatible API",
            Self::Anthropic => "Anthropic Claude",
            Self::DeepL => "DeepL API",
            Self::LibreTranslate => "LibreTranslate",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct TranslationConfig {
    pub backend: TranslationBackend,
    /// Which service preset is chosen (see `ai::PROVIDERS`), e.g. "brain", "openai", "lmstudio".
    pub provider: String,
    /// Offline AI brain model (see `brain::MODELS`).
    pub brain_model: String,
    /// Run the offline AI brain on the graphics card (Windows/Linux: Vulkan build; macOS always does).
    pub brain_gpu: bool,
    pub ollama_url: String,
    pub ollama_model: String,
    /// Address and model of the chosen OpenAI-compatible or Anthropic service.
    pub openai_url: String,
    /// Older versions kept one key here; keys now live in `Config::api_keys`.
    pub openai_api_key: String,
    pub openai_model: String,
    pub deepl_api_key: String,
    pub libre_url: String,
    /// How many lines are sent to the model in one request.
    pub batch_size: usize,
    pub temperature: f32,
    /// Extra instruction for the translator (tone, glossary...).
    pub style_hint: String,
}

impl Default for TranslationConfig {
    fn default() -> Self {
        Self {
            backend: TranslationBackend::BuiltinBrain,
            provider: "brain".into(),
            brain_model: crate::brain::DEFAULT_MODEL.into(),
            brain_gpu: false,
            ollama_url: "http://localhost:11434".into(),
            ollama_model: "qwen2.5:7b".into(),
            openai_url: "https://api.openai.com/v1".into(),
            openai_api_key: String::new(),
            openai_model: "gpt-4o-mini".into(),
            deepl_api_key: String::new(),
            libre_url: "http://localhost:5000".into(),
            batch_size: 12,
            temperature: 0.2,
            style_hint: "Natural, polite spoken Japanese (です・ます調). Keep names and numbers.".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum AudioMode {
    /// Remove the original audio track; only the Japanese dub is heard.
    Replace,
    /// Keep the original audio, lowered under the Japanese voice.
    Duck,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct AudioConfig {
    pub mode: AudioMode,
    /// Original audio volume while the Japanese voice speaks (percent).
    pub duck_level_percent: f32,
    /// Apply ducking for the whole video instead of only while the dub speaks.
    pub duck_whole_track: bool,
    pub dub_volume_percent: f32,
    /// Fastest allowed speed-up of a dubbed line so it fits its time slot.
    pub max_tempo: f32,
    /// Also keep the untouched English audio as a second (selectable) audio track.
    pub keep_original_track: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            mode: AudioMode::Duck,
            duck_level_percent: 15.0,
            duck_whole_track: false,
            dub_volume_percent: 100.0,
            max_tempo: 1.5,
            keep_original_track: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum OcrSampling {
    /// Run OCR on every sampled frame.
    EveryInterval,
    /// Sample every N ms but only run OCR when the picture changed (scene/slide changes).
    SceneChanges,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum OcrEngineKind {
    /// Built into BabelCraft (ocrs neural OCR, models downloaded automatically).
    Builtin,
    /// External Tesseract program.
    Tesseract,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct OcrConfig {
    pub enabled: bool,
    pub engine: OcrEngineKind,
    pub sampling: OcrSampling,
    pub interval_ms: u32,
    /// 0..1 mean pixel difference that counts as "the picture changed".
    pub change_threshold: f32,
    /// Tesseract word confidence (0-100) below which words are ignored.
    pub min_confidence: f32,
    /// Text visible for less than this is ignored (flicker / motion blur).
    pub min_duration_ms: u32,
    pub tesseract_path: String,
    /// Tesseract page segmentation mode (3 = auto, 11 = sparse text).
    pub psm: u32,
    pub language: String,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            engine: OcrEngineKind::Builtin,
            sampling: OcrSampling::SceneChanges,
            interval_ms: 1000,
            change_threshold: 0.03,
            min_confidence: 60.0,
            min_duration_ms: 700,
            tesseract_path: "tesseract".into(),
            psm: 11,
            language: "eng".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum AsrEngine {
    /// whisper.cpp linked into BabelCraft (build with `--features whisper`).
    Builtin,
    /// whisper.cpp command-line tool (`whisper-cli`), downloaded automatically.
    Cli,
    /// An online or local Whisper API (OpenAI, Groq, a Speaches server...).
    Api,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct AsrConfig {
    pub enabled: bool,
    pub engine: AsrEngine,
    /// Path to a ggml Whisper model, e.g. ggml-base.en.bin. Empty = models folder default.
    pub model_path: String,
    pub whisper_cli_path: String,
    pub use_gpu: bool,
    /// 0 = automatic.
    pub threads: u32,
    /// Online / server speech recognition (see `ai::SPEECH_SERVICES`).
    pub api_service: String,
    pub api_url: String,
    pub api_model: String,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            engine: if cfg!(feature = "whisper") { AsrEngine::Builtin } else { AsrEngine::Cli },
            model_path: String::new(),
            whisper_cli_path: "whisper-cli".into(),
            use_gpu: true,
            threads: 0,
            api_service: "openai".into(),
            api_url: "https://api.openai.com/v1".into(),
            api_model: "whisper-1".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TtsBackend {
    Voicevox,
    OpenAiCompatible,
    EdgeTts,
    Piper,
    /// The Japanese voice built into Windows or macOS.
    System,
}

impl TtsBackend {
    pub const ALL: [Self; 5] = [Self::Voicevox, Self::OpenAiCompatible, Self::System, Self::EdgeTts, Self::Piper];
    pub fn label(self) -> &'static str {
        match self {
            Self::Voicevox => "VOICEVOX (free, offline)",
            Self::OpenAiCompatible => "OpenAI voices / Kokoro (API)",
            Self::EdgeTts => "edge-tts (online)",
            Self::Piper => "Piper (local CLI)",
            Self::System => "Computer's own Japanese voice",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct TtsConfig {
    pub backend: TtsBackend,
    pub voicevox_url: String,
    pub voicevox_speaker: u32,
    /// Start BabelCraft's own (or the installed app's) VOICEVOX engine when it isn't running.
    pub voicevox_auto_start: bool,
    pub openai_url: String,
    pub openai_api_key: String,
    pub openai_model: String,
    pub openai_voice: String,
    pub edge_tts_path: String,
    pub edge_voice: String,
    pub piper_path: String,
    pub piper_model: String,
    /// Part of the system voice's name (empty = first Japanese voice found).
    pub system_voice: String,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            backend: TtsBackend::Voicevox,
            voicevox_url: "http://127.0.0.1:50021".into(),
            voicevox_speaker: 3,
            voicevox_auto_start: true,
            openai_url: "http://localhost:8880/v1".into(),
            openai_api_key: String::new(),
            openai_model: "kokoro".into(),
            openai_voice: "jf_alpha".into(),
            edge_tts_path: "edge-tts".into(),
            edge_voice: "ja-JP-NanamiNeural".into(),
            piper_path: "piper".into(),
            piper_model: String::new(),
            system_voice: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct OverlayConfig {
    /// CJK font file. Empty = auto-detect (bundled / downloaded / system Japanese font).
    pub font_path: String,
    /// Opacity of the box drawn behind the Japanese text (0..1).
    pub box_opacity: f32,
    pub text_color: [u8; 3],
    pub box_color: [u8; 3],
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self { font_path: String::new(), box_opacity: 0.92, text_color: [255, 255, 255], box_color: [16, 16, 20] }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct ToolsConfig {
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    /// x264 quality (lower = better, 18-28 typical).
    pub video_crf: u32,
    pub video_preset: String,
    pub keep_temp_files: bool,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            ffmpeg_path: "ffmpeg".into(),
            ffprobe_path: "ffprobe".into(),
            video_crf: 20,
            video_preset: "medium".into(),
            keep_temp_files: false,
        }
    }
}

/// Language of BabelCraft's own buttons and menus.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum UiLanguage {
    #[default]
    English,
    Japanese,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct Config {
    pub ui_language: UiLanguage,
    pub last_input: String,
    pub last_output: String,
    /// API keys of online services, by service id ("openai", "anthropic", "gemini", "deepl"...).
    /// One key serves every part that uses that service (translation, speech, voice).
    pub api_keys: BTreeMap<String, String>,
    pub translation: TranslationConfig,
    pub audio: AudioConfig,
    pub ocr: OcrConfig,
    pub asr: AsrConfig,
    pub tts: TtsConfig,
    pub overlay: OverlayConfig,
    pub tools: ToolsConfig,
}

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("org", "BabelCraft", "babelcraft")
}

/// The settings file. `BABELCRAFT_CONFIG` names another file; in portable mode it sits in
/// the `babelcraft-data` folder next to the program.
pub fn config_file() -> PathBuf {
    if let Some(p) = std::env::var_os("BABELCRAFT_CONFIG").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    if let Some(dir) = portable_dir() {
        return dir.join("config.toml");
    }
    project_dirs()
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("babelcraft.toml"))
}

/// A `babelcraft-data` folder next to the program switches on portable mode.
fn portable_dir() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.join("babelcraft-data");
    dir.is_dir().then_some(dir)
}

/// Where downloaded tools, models and fonts live.
/// Portable mode: a `babelcraft-data` folder next to the program (or the
/// BABELCRAFT_DATA_DIR variable) keeps everything beside the app, e.g. on a USB stick.
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("BABELCRAFT_DATA_DIR").filter(|d| !d.is_empty()) {
        // Always a full path: helper programs run from their own folders.
        let d = PathBuf::from(d);
        return std::path::absolute(&d).unwrap_or(d);
    }
    if let Some(dir) = portable_dir() {
        return dir;
    }
    project_dirs().map(|d| d.data_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from("babelcraft-data"))
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn fonts_dir() -> PathBuf {
    data_dir().join("fonts")
}

pub const DEFAULT_WHISPER_MODEL: &str = "ggml-base.en.bin";

impl Config {
    pub fn load() -> Self {
        let mut cfg = Self::load_file();
        cfg.normalize();
        match std::env::var("BABELCRAFT_LANG").ok().as_deref().map(str::to_ascii_lowercase).as_deref() {
            Some("ja" | "jp" | "japanese") => cfg.ui_language = UiLanguage::Japanese,
            Some("en" | "english") => cfg.ui_language = UiLanguage::English,
            _ => {}
        }
        cfg
    }

    fn load_file() -> Self {
        let path = config_file();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                eprintln!("BabelCraft: could not read {}: {e}; using defaults", path.display());
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    /// Brings settings written by older versions up to date (service preset, keys).
    pub fn normalize(&mut self) {
        let t = &mut self.translation;
        let preset_ok = crate::ai::provider(&t.provider).map(|p| p.backend == t.backend).unwrap_or(false);
        if !preset_ok {
            t.provider = crate::ai::guess_provider(t.backend, &t.openai_url).to_string();
        }
        // Keys typed into older versions move to the shared key list.
        let old_chat_key = std::mem::take(&mut t.openai_api_key);
        if !old_chat_key.trim().is_empty() {
            let id = if t.backend == TranslationBackend::OpenAiCompatible { t.provider.clone() } else { "openai".to_string() };
            self.api_keys.entry(id).or_insert(old_chat_key);
        }
        let old_deepl = std::mem::take(&mut self.translation.deepl_api_key);
        if !old_deepl.trim().is_empty() {
            self.api_keys.entry("deepl".into()).or_insert(old_deepl);
        }
        if crate::brain::MODELS.iter().all(|m| m.id != self.translation.brain_model) {
            self.translation.brain_model = crate::brain::DEFAULT_MODEL.into();
        }
        self.api_keys.retain(|_, v| !v.trim().is_empty());
    }

    /// API key for a service: typed in BabelCraft, else the service's usual environment variable.
    pub fn key_for(&self, service: &str) -> String {
        if let Some(k) = self.api_keys.get(service).map(|k| k.trim()).filter(|k| !k.is_empty()) {
            return k.to_string();
        }
        let vars: &[&str] = crate::ai::provider(service).map(|p| p.env).unwrap_or(&[]);
        vars.iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.trim().is_empty())).unwrap_or_default()
    }

    /// Key for the chosen translation service.
    pub fn chat_key(&self) -> String {
        self.key_for(&self.translation.provider)
    }

    /// Key for the chosen online speech-recognition service.
    pub fn speech_key(&self) -> String {
        self.key_for(&self.asr.api_service)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_file();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Whisper model file actually used.
    pub fn whisper_model(&self) -> PathBuf {
        if self.asr.model_path.trim().is_empty() {
            models_dir().join(DEFAULT_WHISPER_MODEL)
        } else {
            PathBuf::from(self.asr.model_path.trim())
        }
    }

    /// Helper programs: an explicit path from Settings, else a bundled/downloaded copy, else PATH.
    pub fn ffmpeg(&self) -> PathBuf {
        crate::tools::resolve(&self.tools.ffmpeg_path, "ffmpeg")
    }
    pub fn ffprobe(&self) -> PathBuf {
        crate::tools::resolve(&self.tools.ffprobe_path, "ffprobe")
    }
    pub fn whisper_cli(&self) -> PathBuf {
        crate::tools::resolve(&self.asr.whisper_cli_path, "whisper-cli")
    }
    pub fn tesseract(&self) -> PathBuf {
        crate::tools::resolve(&self.ocr.tesseract_path, "tesseract")
    }
    pub fn uses_builtin_whisper(&self) -> bool {
        cfg!(feature = "whisper") && self.asr.engine == AsrEngine::Builtin
    }

    /// Key for the OpenAI-compatible voice server (OpenAI's own key when it is OpenAI).
    pub fn tts_openai_key(&self) -> String {
        let k = non_empty_or_env(&self.tts.openai_api_key, &["BABELCRAFT_TTS_API_KEY"]);
        if k.is_empty() && self.tts.openai_url.contains("api.openai.com") { self.key_for("openai") } else { k }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_are_upgraded() {
        let old = "[translation]\nbackend = \"Ollama\"\nopenai_api_key = \"sk-old\"\ndeepl_api_key = \"abc:fx\"\n";
        let mut cfg: Config = toml::from_str(old).unwrap();
        cfg.normalize();
        assert_eq!(cfg.translation.provider, "ollama");
        assert_eq!(cfg.api_keys.get("openai").map(String::as_str), Some("sk-old"));
        assert_eq!(cfg.api_keys.get("deepl").map(String::as_str), Some("abc:fx"));
        assert!(cfg.translation.openai_api_key.is_empty());

        let custom = "[translation]\nbackend = \"OpenAiCompatible\"\nopenai_url = \"https://api.groq.com/openai/v1/\"\n";
        let mut cfg: Config = toml::from_str(custom).unwrap();
        cfg.normalize();
        assert_eq!(cfg.translation.provider, "groq");
    }

    #[test]
    fn new_settings_use_the_offline_brain() {
        let cfg = Config::default();
        assert_eq!(cfg.translation.backend, TranslationBackend::BuiltinBrain);
        assert_eq!(crate::ai::provider(&cfg.translation.provider).map(|p| p.backend), Some(TranslationBackend::BuiltinBrain));
        assert!(cfg.tts.voicevox_auto_start);
    }
}

fn non_empty_or_env(value: &str, vars: &[&str]) -> String {
    if !value.trim().is_empty() {
        return value.trim().to_string();
    }
    vars.iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty())).unwrap_or_default()
}
