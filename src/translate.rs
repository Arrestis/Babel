//! English → Japanese translation: BabelCraft's offline AI brain, Ollama, any OpenAI-compatible
//! service (OpenAI, Gemini, OpenRouter, Groq, DeepSeek, Mistral, LM Studio, Jan...), Anthropic
//! Claude, DeepL or LibreTranslate.

use crate::config::{Config, TranslationBackend};
use crate::services::StopOnDrop;
use crate::util::{CancelToken, http_agent, read_body_string};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub enum TextKind {
    /// Titles, labels, slides shown on screen (short, may be fragments).
    OnScreen,
    /// Spoken lines; will be read aloud by the Japanese voice.
    Speech,
}

pub struct Translator {
    cfg: Config,
    agent: ureq::Agent,
    cache: HashMap<String, String>,
    /// Address of the running offline AI brain.
    brain_url: Option<String>,
}

impl Translator {
    pub fn new(cfg: &Config) -> Self {
        Self { cfg: cfg.clone(), agent: http_agent(600), cache: HashMap::new(), brain_url: None }
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
        let todo: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            texts
                .iter()
                .filter(|t| !t.trim().is_empty() && !self.cache.contains_key(*t) && seen.insert((*t).clone()))
                .cloned()
                .collect()
        };
        let batch = self.cfg.translation.batch_size.clamp(1, 50);
        let mut done = texts.len() - todo.len();
        on_done(done);
        for chunk in todo.chunks(batch) {
            cancel.check()?;
            let result = match self.translate_batch(chunk, kind) {
                Ok(v) if v.len() == chunk.len() && v.iter().all(|s| !s.trim().is_empty()) => v,
                Ok(_) | Err(_) if chunk.len() > 1 => {
                    on_warn(format!("Batch of {} lines came back incomplete; translating them one by one.", chunk.len()));
                    let mut v = Vec::new();
                    for t in chunk {
                        cancel.check()?;
                        v.push(self.translate_batch(std::slice::from_ref(t), kind)?.into_iter().next().unwrap_or_default());
                    }
                    v
                }
                Ok(v) => v,
                Err(e) => return Err(e),
            };
            for (src, ja) in chunk.iter().zip(result) {
                self.cache.insert(src.clone(), clean_output(&ja));
            }
            done += chunk.len();
            on_done(done);
        }
        Ok(texts.iter().map(|t| self.cache.get(t).cloned().unwrap_or_default()).collect())
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
    fn openai_chat(&self, url: &str, key: &str, model: &str, system: &str, user: &str, json_mode: bool) -> Result<String> {
        let endpoint = format!("{}/chat/completions", url.trim_end_matches('/'));
        let mut body = json!({
            "model": if model.trim().is_empty() { "local-model" } else { model.trim() },
            "temperature": self.cfg.translation.temperature,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });
        if json_mode {
            body["response_format"] = json!({ "type": "json_object" });
            body["max_tokens"] = json!(2048);
        }
        for attempt in 0..2 {
            let mut req = self.agent.post(&endpoint).header("X-Title", "BabelCraft");
            if !key.is_empty() {
                req = req.header("Authorization", format!("Bearer {key}"));
            }
            let mut resp = req.send_json(&body).with_context(|| format!("could not reach {endpoint}"))?;
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
                bail!("the service refused the API key ({status}) — check it in Settings → Translation");
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
             Reply ONLY with JSON of the form {{\"translations\": [\"...\", ...]}} containing exactly one Japanese string per input item, in the same order. No romaji, no notes.",
            self.cfg.translation.style_hint
        )
    }

    fn llm(&self, texts: &[String], kind: TextKind) -> Result<Vec<String>> {
        let user = json!({ "items": texts }).to_string();
        let t = &self.cfg.translation;
        let content = match t.backend {
            TranslationBackend::BuiltinBrain => {
                let url = self.brain_url.as_deref().context("the offline AI brain is not running")?;
                self.openai_chat(url, "", "babelcraft-brain", &self.system_prompt(kind), &user, true)?
            }
            TranslationBackend::OpenAiCompatible => {
                if t.openai_url.trim().is_empty() {
                    bail!("enter the service address in Settings → Translation");
                }
                let key = self.cfg.chat_key();
                let needs_key = crate::ai::provider(&t.provider).map(|p| p.needs_key).unwrap_or(false);
                if needs_key && key.is_empty() {
                    bail!("this service needs an API key — paste it in Settings → Translation (or on the AI models & downloads page)");
                }
                self.openai_chat(&t.openai_url, &key, &t.openai_model, &self.system_prompt(kind), &user, false)?
            }
            TranslationBackend::Anthropic => self.anthropic(&self.system_prompt(kind), &user)?,
            TranslationBackend::Ollama | TranslationBackend::DeepL | TranslationBackend::LibreTranslate => {
                let url = format!("{}/api/chat", self.cfg.translation.ollama_url.trim_end_matches('/'));
                let body = json!({
                    "model": self.cfg.translation.ollama_model,
                    "stream": false,
                    "format": "json",
                    "options": { "temperature": self.cfg.translation.temperature },
                    "messages": [
                        { "role": "system", "content": self.system_prompt(kind) },
                        { "role": "user", "content": user },
                    ],
                });
                let mut resp = self.agent.post(&url).send_json(&body).with_context(|| format!("could not reach Ollama at {url}"))?;
                let status = resp.status();
                let text = read_body_string(&mut resp)?;
                if !status.is_success() {
                    bail!("Ollama error {status}: {text}");
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
            bail!("Claude needs an Anthropic API key — paste it in Settings → Translation");
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
            .context("could not reach Anthropic")?;
        let status = resp.status();
        let text = read_body_string(&mut resp)?;
        if !status.is_success() {
            bail!("Claude error {status}: {}", text.chars().take(600).collect::<String>());
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
        let mut resp = self.agent.post(format!("{base}/translate")).send_json(&body).with_context(|| format!("could not reach LibreTranslate at {base}"))?;
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
            bail!("DeepL needs an API key (Settings → Translation)");
        }
        let host = if key.ends_with(":fx") { "https://api-free.deepl.com" } else { "https://api.deepl.com" };
        let body = json!({ "text": texts, "source_lang": "EN", "target_lang": "JA" });
        let mut resp = self
            .agent
            .post(format!("{host}/v2/translate"))
            .header("Authorization", format!("DeepL-Auth-Key {key}"))
            .send_json(&body)
            .context("could not reach DeepL")?;
        let status = resp.status();
        let text = read_body_string(&mut resp)?;
        if !status.is_success() {
            bail!("DeepL error {status}: {text}");
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
    #[test]
    fn parses_fenced_json() {
        let r = parse_json_list("```json\n{\"translations\": [\"こんにちは\", \"世界\"]}\n```").unwrap();
        assert_eq!(r, vec!["こんにちは", "世界"]);
    }
}
