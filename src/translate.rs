// SPDX-License-Identifier: GPL-3.0-or-later
//! English → Japanese translation: BabelCraft's offline AI brain, Ollama, any OpenAI-compatible
//! service (OpenAI, Gemini, OpenRouter, Groq, DeepSeek, Mistral, LM Studio, Jan...), Anthropic
//! Claude, DeepL or LibreTranslate.

use crate::config::{Config, TranslationBackend};
use crate::services::StopOnDrop;
use crate::util::{CancelToken, http_agent, read_body_string};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub enum TextKind {
    /// Titles, labels, slides shown on screen (short, may be fragments).
    OnScreen,
    /// Spoken lines; will be read aloud by the Japanese voice.
    Speech,
}

/// A problem with the translation service itself (no key, not running, refused, crashed):
/// retrying line by line won't help, so it is reported straight away.
#[derive(Debug)]
pub struct ServiceProblem(pub String);

impl std::fmt::Display for ServiceProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ServiceProblem {}

fn service_problem(msg: impl Into<String>) -> anyhow::Error {
    anyhow!(ServiceProblem(msg.into()))
}

fn is_service_problem(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<ServiceProblem>().is_some())
}

const SWITCH_HINT: &str = "Or choose the built-in offline AI brain in Settings → Translation (free, no key needed).";

/// Checks the chosen translator before a job starts, so a missing key or a stopped server is
/// reported at once instead of after minutes of text reading and transcription.
/// Returns warnings worth showing (a very large or programming-only model, …).
pub fn preflight(cfg: &Config) -> Result<Vec<String>> {
    let t = &cfg.translation;
    let mut warnings = Vec::new();
    match t.backend {
        // Its engine and model are downloaded by the automatic setup.
        TranslationBackend::BuiltinBrain => {}
        TranslationBackend::Ollama => {
            let Some(models) = crate::ai::ollama_models_with_size(&t.ollama_url) else {
                return Err(service_problem(format!(
                    "Ollama isn't running at {} — start the Ollama app and try again. {SWITCH_HINT}",
                    t.ollama_url
                )));
            };
            let want = t.ollama_model.trim();
            let found = models.iter().find(|(n, _)| n == want || n.strip_suffix(":latest") == Some(want) || want.strip_suffix(":latest") == Some(n.as_str()));
            let Some((name, size)) = found else {
                let names: Vec<&str> = models.iter().map(|(n, _)| n.as_str()).take(8).collect();
                return Err(service_problem(format!(
                    "Ollama has no model called '{want}'. Your models: {}. Pick one in Settings → Translation (press List), or download one on the AI models & downloads page.",
                    if names.is_empty() { "none yet".to_string() } else { names.join(", ") }
                )));
            };
            let gb = *size as f64 / 1e9;
            if gb > 24.0 {
                warnings.push(format!(
                    "'{name}' is very large ({gb:.0} GB): translating with it can be very slow, and Ollama may stop if the computer runs short of memory. A 4–14B model (for example gemma or qwen) translates well and much faster."
                ));
            }
            if name.contains("coder") || name.contains("code") {
                warnings.push(format!("'{name}' is made for programming; a general chat model gives more natural Japanese."));
            }
        }
        TranslationBackend::OpenAiCompatible => {
            let p = crate::ai::provider(&t.provider);
            let name = p.map(|p| p.name).unwrap_or("the translation service");
            if t.openai_url.trim().is_empty() {
                return Err(service_problem(format!("No address is set for {name}. {SWITCH_HINT}")));
            }
            if p.map(|p| p.needs_key).unwrap_or(false) && cfg.chat_key().is_empty() {
                return Err(service_problem(format!(
                    "{name} needs an API key — paste it in Settings → Translation or on the AI models & downloads page. {SWITCH_HINT}"
                )));
            }
            let local = p.map(|p| p.local).unwrap_or(false) || t.openai_url.contains("localhost") || t.openai_url.contains("127.0.0.1");
            if local && http_agent(5).get(format!("{}/models", t.openai_url.trim_end_matches('/'))).call().is_err() {
                return Err(service_problem(format!("{name} isn't answering at {} — start its local server and try again. {SWITCH_HINT}", t.openai_url)));
            }
        }
        TranslationBackend::Anthropic => {
            if cfg.key_for("anthropic").is_empty() {
                return Err(service_problem(format!(
                    "Claude needs an Anthropic API key — paste it in Settings → Translation or on the AI models & downloads page. {SWITCH_HINT}"
                )));
            }
        }
        TranslationBackend::DeepL => {
            if cfg.key_for("deepl").is_empty() {
                return Err(service_problem(format!("DeepL needs an API key — paste it in Settings → Translation. {SWITCH_HINT}")));
            }
        }
        TranslationBackend::LibreTranslate => {
            if http_agent(5).get(format!("{}/languages", t.libre_url.trim_end_matches('/'))).call().is_err() {
                return Err(service_problem(format!("LibreTranslate isn't answering at {}. {SWITCH_HINT}", t.libre_url)));
            }
        }
    }
    Ok(warnings)
}

impl TextKind {
    fn tag(self) -> &'static str {
        match self {
            TextKind::OnScreen => "screen",
            TextKind::Speech => "speech",
        }
    }
}

pub struct Translator {
    cfg: Config,
    agent: ureq::Agent,
    /// Finished translations ("kind|English" → Japanese), kept on disk per service and model,
    /// so a job run again after a failure doesn't translate (or pay for) the same lines twice.
    cache: HashMap<String, String>,
    cache_file: Option<std::path::PathBuf>,
    /// Address of the running offline AI brain.
    brain_url: Option<String>,
}

/// The model (or service) that produces the translations, for the translation memory.
fn translator_identity(cfg: &Config) -> String {
    let t = &cfg.translation;
    let model = match t.backend {
        TranslationBackend::BuiltinBrain => t.brain_model.clone(),
        TranslationBackend::Ollama => t.ollama_model.clone(),
        TranslationBackend::OpenAiCompatible | TranslationBackend::Anthropic => format!("{} {}", t.openai_url, t.openai_model),
        TranslationBackend::DeepL => "deepl".into(),
        TranslationBackend::LibreTranslate => t.libre_url.clone(),
    };
    format!("v1|{:?}|{}|{}|{}", t.backend, t.provider, model, t.style_hint)
}

impl Translator {
    pub fn new(cfg: &Config) -> Self {
        let cache_file = Some(crate::config::data_dir().join("cache").join(format!("translations-{}.json", crate::util::short_hash(&translator_identity(cfg)))));
        let cache = cache_file
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok())
            .unwrap_or_default();
        Self { cfg: cfg.clone(), agent: http_agent(600), cache, cache_file, brain_url: None }
    }

    /// No translation memory on disk (the "Test translation" button).
    pub fn without_memory(mut self) -> Self {
        self.cache.clear();
        self.cache_file = None;
        self
    }

    fn save_memory(&self) {
        if let Some(p) = &self.cache_file {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(text) = serde_json::to_string(&self.cache) {
                let tmp = p.with_extension("tmp");
                if std::fs::write(&tmp, text).is_ok() {
                    let _ = std::fs::rename(&tmp, p);
                }
            }
        }
    }

    /// Starts what the chosen service needs on this computer (the offline AI brain).
    /// Keep the returned guard while translating; dropping it stops the brain again.
    pub fn prepare(&mut self, cancel: &CancelToken, status: &mut dyn FnMut(String)) -> Result<Option<StopOnDrop>> {
        if self.cfg.translation.backend != TranslationBackend::BuiltinBrain {
            return Ok(None);
        }
        let url = crate::brain::start(&self.cfg, cancel, status)?;
        self.brain_url = Some(url);
        Ok(Some(StopOnDrop(crate::brain::SERVICE)))
    }

    /// Translates every text (cached, batched). `on_done(n)` is called after each batch.
    pub fn translate_all(
        &mut self,
        texts: &[String],
        kind: TextKind,
        cancel: &CancelToken,
        mut on_done: impl FnMut(usize),
        mut on_warn: impl FnMut(String),
    ) -> Result<Vec<String>> {
        let key = |t: &str| format!("{}|{t}", kind.tag());
        let todo: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            texts
                .iter()
                .filter(|t| !t.trim().is_empty() && !self.cache.contains_key(&key(t)) && seen.insert((*t).clone()))
                .cloned()
                .collect()
        };
        let remembered = texts.iter().filter(|t| !t.trim().is_empty()).count() - todo.len();
        if remembered > 0 && self.cache_file.is_some() {
            on_warn(format!("{remembered} lines were already translated in an earlier run; reusing them."));
        }
        let batch = self.cfg.translation.batch_size.clamp(1, 50);
        let mut done = texts.len() - todo.len();
        on_done(done);
        for chunk in todo.chunks(batch) {
            cancel.check()?;
            let result = match self.translate_batch(chunk, kind) {
                Ok(v) if v.len() == chunk.len() && v.iter().all(|s| !s.trim().is_empty()) => v,
                Ok(v) if chunk.len() > 1 => {
                    on_warn(format!("The translator returned {} lines for {}; translating them one by one.", v.len(), chunk.len()));
                    self.one_by_one(chunk, kind, cancel)?
                }
                // A problem with the service itself (no key, not running…) is reported as it is.
                Err(e) if chunk.len() > 1 && !is_service_problem(&e) => {
                    on_warn(format!("A batch of {} lines failed ({e}); translating them one by one.", chunk.len()));
                    self.one_by_one(chunk, kind, cancel)?
                }
                Ok(v) => v,
                Err(e) => return Err(e),
            };
            for (src, ja) in chunk.iter().zip(result) {
                self.cache.insert(key(src), clean_output(&ja));
            }
            self.save_memory();
            done += chunk.len();
            on_done(done);
        }
        Ok(texts.iter().map(|t| self.cache.get(&key(t)).cloned().unwrap_or_default()).collect())
    }

    fn one_by_one(&self, chunk: &[String], kind: TextKind, cancel: &CancelToken) -> Result<Vec<String>> {
        let mut v = Vec::new();
        for t in chunk {
            cancel.check()?;
            v.push(self.translate_batch(std::slice::from_ref(t), kind)?.into_iter().next().unwrap_or_default());
        }
        Ok(v)
    }

    fn translate_batch(&self, texts: &[String], kind: TextKind) -> Result<Vec<String>> {
        match self.cfg.translation.backend {
            TranslationBackend::DeepL => self.deepl(texts),
            TranslationBackend::LibreTranslate => self.libre(texts),
            TranslationBackend::BuiltinBrain
            | TranslationBackend::Ollama
            | TranslationBackend::OpenAiCompatible
            | TranslationBackend::Anthropic => self.llm(texts, kind),
        }
    }

    /// One OpenAI-style chat request. Retries without `temperature` for models that refuse it.
    /// With a `schema`, the answer is held to it (llama.cpp's server supports this).
    fn openai_chat(&self, url: &str, key: &str, model: &str, system: &str, user: &str, schema: Option<Value>) -> Result<String> {
        let endpoint = format!("{}/chat/completions", url.trim_end_matches('/'));
        let mut body = json!({
            "model": if model.trim().is_empty() { "local-model" } else { model.trim() },
            "temperature": self.cfg.translation.temperature,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });
        if let Some(schema) = schema {
            body["response_format"] = json!({ "type": "json_schema", "json_schema": { "name": "translations", "schema": schema, "strict": true } });
            body["max_tokens"] = json!(4096);
        }
        for attempt in 0..2 {
            let mut req = self.agent.post(&endpoint).header("X-Title", "BabelCraft");
            if !key.is_empty() {
                req = req.header("Authorization", format!("Bearer {key}"));
            }
            let mut resp = req.send_json(&body).map_err(|e| service_problem(format!("could not reach {endpoint} ({e}). {SWITCH_HINT}")))?;
            let status = resp.status();
            let text = read_body_string(&mut resp)?;
            if status.is_success() {
                let v: Value = serde_json::from_str(&text)?;
                return Ok(v["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_string());
            }
            if attempt == 0 && status.as_u16() == 400 && text.contains("temperature") {
                body.as_object_mut().map(|o| o.remove("temperature"));
                continue;
            }
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(service_problem(format!("the service refused the API key ({status}) — check it in Settings → Translation")));
            }
            if status.as_u16() == 404 || status.as_u16() == 429 || status.is_server_error() {
                return Err(service_problem(format!(
                    "translation service error {status}: {}",
                    text.chars().take(400).collect::<String>()
                )));
            }
            bail!("translation service error {status}: {}", text.chars().take(600).collect::<String>());
        }
        unreachable!()
    }

    fn system_prompt(&self, kind: TextKind) -> String {
        let what = match kind {
            TextKind::OnScreen => "on-screen text (titles, captions, slide text, labels) from a video. Keep translations short so they fit where the English was",
            TextKind::Speech => "spoken lines from a video, in order. They will be read aloud by a Japanese voice, so make them natural spoken Japanese and about as long as the English",
        };
        format!(
            "You are a professional English-to-Japanese video localizer. You translate {what}. {} \
             Translate the complete meaning of every item: never leave out, shorten or merge sentences, even greetings. \
             The lines are numbered. A line may be an unfinished sentence that continues in the next line: translate each line on its own anyway, never join lines. \
             Reply ONLY with JSON of the form {{\"translations\": {{\"01\": \"...\", \"02\": \"...\"}}}} using exactly the same numbers, one Japanese translation per number. No romaji, no notes.",
            self.cfg.translation.style_hint
        )
    }

    fn llm(&self, texts: &[String], kind: TextKind) -> Result<Vec<String>> {
        // Zero-padded numbers keep the lines in order ("01" … "12") in the JSON object.
        let numbered: serde_json::Map<String, Value> = texts.iter().enumerate().map(|(i, t)| (line_key(i), json!(t))).collect();
        let user = json!({ "lines": numbered }).to_string();
        let t = &self.cfg.translation;
        let content = match t.backend {
            TranslationBackend::BuiltinBrain => {
                let url = self.brain_url.as_deref().ok_or_else(|| service_problem("the offline AI brain is not running"))?;
                self.openai_chat(url, "", "babelcraft-brain", &self.system_prompt(kind), &user, Some(numbered_schema(texts.len())))?
            }
            TranslationBackend::OpenAiCompatible => {
                if t.openai_url.trim().is_empty() {
                    return Err(service_problem(format!("enter the service address in Settings → Translation. {SWITCH_HINT}")));
                }
                let key = self.cfg.chat_key();
                let needs_key = crate::ai::provider(&t.provider).map(|p| p.needs_key).unwrap_or(false);
                if needs_key && key.is_empty() {
                    return Err(service_problem(format!(
                        "this service needs an API key — paste it in Settings → Translation (or on the AI models & downloads page). {SWITCH_HINT}"
                    )));
                }
                self.openai_chat(&t.openai_url, &key, &t.openai_model, &self.system_prompt(kind), &user, None)?
            }
            TranslationBackend::Anthropic => self.anthropic(&self.system_prompt(kind), &user)?,
            TranslationBackend::Ollama | TranslationBackend::DeepL | TranslationBackend::LibreTranslate => {
                let model = self.cfg.translation.ollama_model.trim();
                let url = format!("{}/api/chat", self.cfg.translation.ollama_url.trim_end_matches('/'));
                let body = json!({
                    "model": model,
                    "stream": false,
                    // A schema with every line number makes Ollama answer each line.
                    "format": numbered_schema(texts.len()),
                    "options": { "temperature": self.cfg.translation.temperature },
                    "messages": [
                        { "role": "system", "content": self.system_prompt(kind) },
                        { "role": "user", "content": user },
                    ],
                });
                let mut resp = self
                    .agent
                    .post(&url)
                    .send_json(&body)
                    .map_err(|e| service_problem(format!("could not reach Ollama at {url} ({e}) — is the Ollama app running? {SWITCH_HINT}")))?;
                let status = resp.status();
                let text = read_body_string(&mut resp)?;
                if !status.is_success() {
                    let lower = text.to_lowercase();
                    if lower.contains("terminated") || lower.contains("exit status") || lower.contains("memory") || lower.contains("allocate") {
                        return Err(service_problem(format!(
                            "Ollama's engine stopped while running '{model}' — the model is probably too large for this computer's memory. \
                             Choose a smaller model in Settings → Translation (press List; 4–14B models translate well), or the built-in offline AI brain.\nOllama said: {}",
                            text.chars().take(300).collect::<String>()
                        )));
                    }
                    if status.as_u16() == 404 {
                        return Err(service_problem(format!(
                            "Ollama has no model called '{model}' — pick one in Settings → Translation (press List). {SWITCH_HINT}"
                        )));
                    }
                    bail!("Ollama error {status}: {}", text.chars().take(400).collect::<String>());
                }
                let v: Value = serde_json::from_str(&text)?;
                v["message"]["content"].as_str().unwrap_or_default().to_string()
            }
        };
        let content = strip_thinking(&content);
        if texts.len() == 1 {
            // Accept either JSON or plain text for single items.
            if let Ok(v) = parse_json_list(&content) {
                if let Some(first) = v.into_iter().next() {
                    return Ok(vec![first]);
                }
            }
            return Ok(vec![content.trim().to_string()]);
        }
        parse_json_list(&content)
    }

    /// Anthropic Claude (Messages API).
    fn anthropic(&self, system: &str, user: &str) -> Result<String> {
        let key = self.cfg.key_for("anthropic");
        if key.is_empty() {
            return Err(service_problem(format!("Claude needs an Anthropic API key — paste it in Settings → Translation. {SWITCH_HINT}")));
        }
        let model = self.cfg.translation.openai_model.trim();
        let model = if model.is_empty() || !model.starts_with("claude") { "claude-opus-5-5" } else { model };
        let body = json!({
            "model": model,
            "max_tokens": 4096,
            "temperature": self.cfg.translation.temperature,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
        });
        let mut resp = self
            .agent
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01")
            .send_json(&body)
            .map_err(|e| service_problem(format!("could not reach Anthropic ({e})")))?;
        let status = resp.status();
        let text = read_body_string(&mut resp)?;
        if !status.is_success() {
            let msg = format!("Claude error {status}: {}", text.chars().take(600).collect::<String>());
            if matches!(status.as_u16(), 401 | 403 | 404 | 429) || status.is_server_error() {
                return Err(service_problem(msg));
            }
            bail!(msg);
        }
        let v: Value = serde_json::from_str(&text)?;
        Ok(v["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["type"] == "text")
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join(""))
    }

    /// LibreTranslate (POST /translate with a list of texts).
    fn libre(&self, texts: &[String]) -> Result<Vec<String>> {
        let base = self.cfg.translation.libre_url.trim().trim_end_matches('/');
        if base.is_empty() {
            bail!("enter the LibreTranslate address in Settings → Translation");
        }
        let mut body = json!({ "q": texts, "source": "en", "target": "ja", "format": "text" });
        let key = self.cfg.key_for("libre");
        if !key.is_empty() {
            body["api_key"] = json!(key);
        }
        let mut resp = self
            .agent
            .post(format!("{base}/translate"))
            .send_json(&body)
            .map_err(|e| service_problem(format!("could not reach LibreTranslate at {base} ({e})")))?;
        let status = resp.status();
        let text = read_body_string(&mut resp)?;
        if !status.is_success() {
            bail!("LibreTranslate error {status}: {}", text.chars().take(400).collect::<String>());
        }
        let v: Value = serde_json::from_str(&text)?;
        match &v["translatedText"] {
            Value::Array(a) => Ok(a.iter().map(|x| x.as_str().unwrap_or_default().to_string()).collect()),
            Value::String(s) => Ok(vec![s.clone()]),
            _ => bail!("unexpected LibreTranslate reply"),
        }
    }

    fn deepl(&self, texts: &[String]) -> Result<Vec<String>> {
        let key = self.cfg.key_for("deepl");
        if key.is_empty() {
            return Err(service_problem(format!("DeepL needs an API key (Settings → Translation). {SWITCH_HINT}")));
        }
        let host = if key.ends_with(":fx") { "https://api-free.deepl.com" } else { "https://api.deepl.com" };
        let body = json!({ "text": texts, "source_lang": "EN", "target_lang": "JA" });
        let mut resp = self
            .agent
            .post(format!("{host}/v2/translate"))
            .header("Authorization", format!("DeepL-Auth-Key {key}"))
            .send_json(&body)
            .map_err(|e| service_problem(format!("could not reach DeepL ({e})")))?;
        let status = resp.status();
        let text = read_body_string(&mut resp)?;
        if !status.is_success() {
            return Err(service_problem(format!("DeepL error {status}: {}", text.chars().take(400).collect::<String>())));
        }
        let v: Value = serde_json::from_str(&text)?;
        Ok(v["translations"]
            .as_array()
            .ok_or_else(|| anyhow!("unexpected DeepL reply"))?
            .iter()
            .map(|t| t["text"].as_str().unwrap_or_default().to_string())
            .collect())
    }

    /// Small round-trip used by the "Test" button (starts and stops the offline brain if chosen).
    pub fn test(&mut self, cancel: &CancelToken) -> Result<String> {
        let _brain = self.prepare(cancel, &mut |_| {})?;
        let r = self.translate_batch(&["Hello! Welcome to the video.".to_string()], TextKind::Speech)?;
        Ok(clean_output(&r.into_iter().next().unwrap_or_default()))
    }
}

/// "01", "02" … — numbers that sort in order as text too.
fn line_key(i: usize) -> String {
    format!("{:02}", i + 1)
}

/// JSON schema for `{"translations": {"01": "…", …, "NN": "…"}}` with every number required.
fn numbered_schema(n: usize) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for i in 0..n {
        props.insert(line_key(i), json!({ "type": "string" }));
        required.push(json!(line_key(i)));
    }
    json!({
        "type": "object",
        "properties": {
            "translations": { "type": "object", "properties": props, "required": required, "additionalProperties": false }
        },
        "required": ["translations"],
        "additionalProperties": false
    })
}

/// Removes the "thinking" some reasoning models write before their answer.
fn strip_thinking(s: &str) -> String {
    let mut out = s.to_string();
    while let (Some(a), Some(b)) = (out.find("<think>"), out.find("</think>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</think>".len(), "");
    }
    out
}

/// Pulls a list of strings out of a model reply (tolerates code fences and extra prose).
pub fn parse_json_list(content: &str) -> Result<Vec<String>> {
    let s = content.trim();
    let start = s.find(['{', '[']).ok_or_else(|| anyhow!("no JSON in reply"))?;
    let end = s.rfind(['}', ']']).ok_or_else(|| anyhow!("no JSON in reply"))?;
    let v: Value = serde_json::from_str(&s[start..=end])?;
    // Numbered answers ({"1": "…", "2": "…"}), on their own or under "translations".
    let numbered = |o: &serde_json::Map<String, Value>| -> Option<Vec<String>> {
        let mut items: Vec<(u32, String)> = Vec::new();
        for (k, v) in o {
            items.push((k.trim().parse().ok()?, v.as_str()?.to_string()));
        }
        if items.is_empty() {
            return None;
        }
        items.sort_by_key(|(n, _)| *n);
        Some(items.into_iter().map(|(_, t)| t).collect())
    };
    if let Value::Object(o) = &v {
        if let Some(list) = o.get("translations").and_then(|t| t.as_object()).and_then(|t| numbered(t)).or_else(|| numbered(o)) {
            return Ok(list);
        }
    }
    let arr = match &v {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o
            .get("translations")
            .or_else(|| o.values().find(|x| x.is_array()))
            .and_then(|x| x.as_array().cloned())
            .ok_or_else(|| anyhow!("no list in reply"))?,
        _ => bail!("unexpected reply"),
    };
    Ok(arr
        .into_iter()
        .map(|x| match x {
            Value::String(s) => s,
            Value::Object(o) => o.values().find_map(|v| v.as_str().map(String::from)).unwrap_or_default(),
            other => other.to_string(),
        })
        .collect())
}

fn clean_output(s: &str) -> String {
    s.trim().trim_matches('"').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One short test translation with the person's saved settings (whatever service they chose).
    /// Run with: cargo test saved_translator_works -- --ignored --nocapture
    #[test]
    #[ignore]
    fn saved_translator_works() {
        let cfg = Config::load();
        println!("service: {} · model: {}", cfg.translation.provider, cfg.translation.openai_model);
        let lines = vec!["Welcome to the course. Today we will learn how to add chapters.".to_string(), "Click on the chapter button.".to_string()];
        let mut tr = Translator::new(&cfg);
        let cancel = CancelToken::default();
        let _guard = tr.prepare(&cancel, &mut |m| println!("{m}")).unwrap();
        let out = tr.translate_all(&lines, TextKind::Speech, &cancel, |_| {}, |w| println!("warning: {w}")).unwrap();
        for (e, j) in lines.iter().zip(&out) {
            println!("  {e} → {j}");
        }
        assert!(out.iter().all(|j| !j.trim().is_empty()));
    }
    #[test]
    fn parses_numbered_replies_in_order() {
        let r = parse_json_list("{\"translations\": {\"2\": \"二\", \"1\": \"一\", \"10\": \"十\"}}").unwrap();
        assert_eq!(r, vec!["一", "二", "十"]);
        let r = parse_json_list("{\"1\": \"こんにちは\", \"2\": \"世界\"}").unwrap();
        assert_eq!(r, vec!["こんにちは", "世界"]);
    }

    /// Speech fragments from a real video, translated in batches by the offline brain: every
    /// batch should come back with one line per fragment.
    /// Run with: BABELCRAFT_DATA_DIR=target/testdata cargo test brain_keeps_lines_apart -- --ignored --nocapture
    #[test]
    #[ignore]
    fn brain_keeps_lines_apart() {
        let fragments: Vec<String> = [
            "First thing I want to do, I want to put some chapter points on it. So I'm going to look",
            "through it and say yeah, I want a chapter point just there. Come over to this button,",
            "click on it and it adds in a chapter point. Drag along a bit more, add another chapter.",
            "If you want to get completely accurate, then you can use the one frame forward and one",
            "frame backwards keys to get to exactly the right place. You can play it and then stop.",
            "There we are. That's where I want the chapter to go. Click on the chapter button. You notice",
            "on the menu there, it shows you the buttons or you click on the timeline, it shows you",
            "the chapters, which you can't see when you're in the project tab. Right now, I've brought",
            "in a menu and I want to use it.",
            "So the first thing is to set the end action of the timeline.",
            "Then I link each button to its chapter.",
            "And that's it, the disc is ready to build.",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // As in a job: Whisper's pieces are joined into whole sentences first.
        let pieces: Vec<crate::asr::Segment> = fragments
            .iter()
            .enumerate()
            .map(|(i, t)| crate::asr::Segment { start: i as f64 * 6.0, end: i as f64 * 6.0 + 6.0, english: t.clone(), japanese: String::new() })
            .collect();
        let fragments: Vec<String> = crate::asr::to_sentences(&pieces).into_iter().map(|s| s.english).collect();
        let cfg = Config::default();
        let mut tr = Translator::new(&cfg).without_memory();
        let cancel = CancelToken::default();
        let _brain = tr.prepare(&cancel, &mut |m| println!("{m}")).unwrap();
        let warnings = std::cell::RefCell::new(Vec::new());
        let t0 = std::time::Instant::now();
        let out = tr.translate_all(&fragments, TextKind::Speech, &cancel, |_| {}, |w| warnings.borrow_mut().push(w)).unwrap();
        println!("translated {} lines in {:.1} s", out.len(), t0.elapsed().as_secs_f32());
        for (e, j) in fragments.iter().zip(&out) {
            println!("  {e}\n    → {j}");
        }
        println!("warnings: {:?}", warnings.borrow());
        assert!(out.iter().all(|j| !j.trim().is_empty()));
        assert!(warnings.borrow().is_empty(), "the batch should come back complete");
    }

    #[test]
    fn parses_fenced_json() {
        let r = parse_json_list("```json\n{\"translations\": [\"こんにちは\", \"世界\"]}\n```").unwrap();
        assert_eq!(r, vec!["こんにちは", "世界"]);
    }
}
