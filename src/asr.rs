// SPDX-License-Identifier: GPL-3.0-or-later
//! English speech recognition with Whisper: whisper-rs in-process, the whisper.cpp CLI, or an
//! online / local Whisper API (OpenAI, Groq, Speaches... — the /audio/transcriptions endpoint).

use crate::config::{AsrEngine, Config};
use crate::util::{CancelToken, run_process, silent_command};
use anyhow::{Context, Result, bail};
use std::path::Path;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub english: String,
    pub japanese: String,
}

pub fn transcribe(
    cfg: &Config,
    wav16k: &Path,
    tmp: &Path,
    cancel: &CancelToken,
    on_frac: impl FnMut(f32) + 'static,
    on_log: impl FnMut(String),
) -> Result<Vec<Segment>> {
    if cfg.asr.engine == AsrEngine::Api {
        return Ok(clean_segments(api(cfg, wav16k, tmp, cancel, on_frac)?));
    }
    let model = cfg.whisper_model();
    if !model.exists() {
        bail!(
            "Whisper model not found: {}\nOpen Setup checks and press \"Download Whisper model\", or set a model path in Settings → Speech.",
            model.display()
        );
    }
    let raw = match cfg.asr.engine {
        AsrEngine::Builtin if cfg!(feature = "whisper") => builtin(cfg, &model, wav16k, cancel, on_frac)?,
        AsrEngine::Builtin | AsrEngine::Api => {
            let mut on_log = on_log;
            on_log("This build has no built-in Whisper (built without `--features whisper`); using whisper-cli instead.".into());
            cli(cfg, &model, wav16k, tmp, cancel, on_frac)?
        }
        AsrEngine::Cli => cli(cfg, &model, wav16k, tmp, cancel, on_frac)?,
    };
    Ok(clean_segments(raw))
}

/// Online or local Whisper API. The audio goes up in pieces of at most 10 minutes
/// (about 19 MB each), under the usual 25 MB upload limit.
fn api(cfg: &Config, wav: &Path, tmp: &Path, cancel: &CancelToken, mut on_frac: impl FnMut(f32)) -> Result<Vec<Segment>> {
    let a = &cfg.asr;
    let base = a.api_url.trim().trim_end_matches('/');
    if base.is_empty() {
        bail!("enter the speech service address in Settings → Speech");
    }
    let svc = crate::ai::speech_service(&a.api_service);
    let key = cfg.speech_key();
    if !svc.local && key.is_empty() {
        bail!("{} needs an API key — paste it in Settings → Speech", svc.name);
    }
    let mut reader = hound::WavReader::open(wav)?;
    let spec = reader.spec();
    let per_sec = spec.sample_rate as usize * spec.channels.max(1) as usize;
    let samples: Vec<i16> = reader.samples::<i16>().collect::<Result<_, _>>()?;
    let chunk = (per_sec * 600).max(1);
    let pieces = samples.len().div_ceil(chunk).max(1);
    let agent = crate::util::http_agent(900);
    let mut out = Vec::new();
    for (i, part) in samples.chunks(chunk).enumerate() {
        cancel.check()?;
        on_frac(i as f32 / pieces as f32);
        let offset = (i * chunk) as f64 / per_sec as f64;
        let duration = part.len() as f64 / per_sec as f64;
        let path = tmp.join(format!("speech_part_{i:03}.wav"));
        {
            let mut w = hound::WavWriter::create(&path, spec)?;
            for s in part {
                w.write_sample(*s)?;
            }
            w.finalize()?;
        }
        let bytes = std::fs::read(&path)?;
        let _ = std::fs::remove_file(&path);
        let v = post_transcription(&agent, base, &key, &a.api_model, cfg.source_lang().whisper_code(), bytes)?;
        match v["segments"].as_array() {
            Some(segs) if !segs.is_empty() => {
                for s in segs {
                    out.push(Segment {
                        start: offset + s["start"].as_f64().unwrap_or(0.0),
                        end: offset + s["end"].as_f64().unwrap_or(0.0),
                        english: s["text"].as_str().unwrap_or_default().trim().to_string(),
                        japanese: String::new(),
                    });
                }
            }
            _ => {
                // Services without timestamps: the whole piece becomes one line.
                let text = v["text"].as_str().unwrap_or_default().trim().to_string();
                if !text.is_empty() {
                    out.push(Segment { start: offset, end: offset + duration, english: text, japanese: String::new() });
                }
            }
        }
    }
    on_frac(1.0);
    Ok(out)
}

/// POST /audio/transcriptions as multipart/form-data.
fn post_transcription(agent: &ureq::Agent, base: &str, key: &str, model: &str, cfg_lang: Option<&str>, wav: Vec<u8>) -> Result<serde_json::Value> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let boundary = format!("----BabelCraft{stamp:x}");
    let mut body: Vec<u8> = Vec::with_capacity(wav.len() + 1024);
    let model = if model.trim().is_empty() { "whisper-1" } else { model.trim() };
    let lang = cfg_lang.unwrap_or("");
    for (name, value) in [("model", model), ("language", lang), ("response_format", "verbose_json"), ("temperature", "0")] {
        if value.is_empty() { continue; }
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(&wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let url = format!("{base}/audio/transcriptions");
    let mut req = agent.post(&url).header("Content-Type", format!("multipart/form-data; boundary={boundary}"));
    if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let mut resp = req.send(body).with_context(|| format!("could not reach {url}"))?;
    let status = resp.status();
    let text = crate::util::read_body_string(&mut resp)?;
    if !status.is_success() {
        bail!("speech service error {status}: {}", text.chars().take(500).collect::<String>());
    }
    serde_json::from_str(&text).context("unexpected reply from the speech service")
}

/// Joins Whisper's pieces (which often stop mid-sentence) into whole sentences, with start and
/// end times estimated from where each sentence falls inside the pieces. Each line given to the
/// translator and the voice is then a complete thought: better Japanese, and the dub stays in
/// step. A pause of more than 1.5 s, or a run-on longer than 20 s, also ends a line.
pub fn to_sentences(segs: &[Segment]) -> Vec<Segment> {
    const MAX_LEN_SECS: f64 = 20.0;
    const PAUSE_SECS: f64 = 1.5;
    let mut out: Vec<Segment> = Vec::new();
    let mut text = String::new();
    let mut start: Option<f64> = None;
    let mut end = 0.0f64;
    let flush = |text: &mut String, start: &mut Option<f64>, end: f64, out: &mut Vec<Segment>| {
        let t = text.trim().to_string();
        if let (Some(s), false) = (*start, t.is_empty()) {
            out.push(Segment { start: s, end: end.max(s + 0.2), english: t, japanese: String::new() });
        }
        text.clear();
        *start = None;
    };
    for seg in segs {
        let chars: Vec<char> = seg.english.trim().chars().collect();
        if chars.is_empty() {
            continue;
        }
        // A long pause or a very long run-on closes the open line at the piece boundary.
        if start.is_some() && (seg.start - end > PAUSE_SECS || seg.end - start.unwrap_or(seg.start) > MAX_LEN_SECS) {
            flush(&mut text, &mut start, end, &mut out);
        }
        let n = chars.len() as f64;
        let at = |i: usize| seg.start + (seg.end - seg.start) * (i as f64 / n);
        let mut piece_start = 0usize;
        for i in 0..chars.len() {
            let ends_sentence = matches!(chars[i], '.' | '!' | '?')
                && chars.get(i + 1).is_none_or(|c| c.is_whitespace() || *c == '"')
                && !is_abbreviation(&chars[..=i]);
            if ends_sentence {
                let piece: String = chars[piece_start..=i].iter().collect();
                if !piece.trim().is_empty() {
                    if start.is_none() {
                        start = Some(at(piece_start));
                    }
                    text.push(' ');
                    text.push_str(piece.trim());
                    end = at(i + 1);
                    flush(&mut text, &mut start, end, &mut out);
                }
                piece_start = i + 1;
            }
        }
        let rest: String = chars[piece_start.min(chars.len())..].iter().collect();
        if !rest.trim().is_empty() {
            if start.is_none() {
                start = Some(at(piece_start));
            }
            text.push(' ');
            text.push_str(rest.trim());
            end = seg.end;
        }
    }
    flush(&mut text, &mut start, end, &mut out);
    out
}

/// "Mr." "Dr." "e.g." … don't end a sentence.
fn is_abbreviation(upto: &[char]) -> bool {
    let word: String = upto.iter().rev().take_while(|c| !c.is_whitespace()).collect::<Vec<_>>().into_iter().rev().collect();
    let w = word.trim_start_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase();
    matches!(w.as_str(), "mr." | "mrs." | "ms." | "dr." | "st." | "vs." | "etc." | "e.g." | "i.e." | "no." | "jr." | "sr.")
        || (w.len() == 2 && w.ends_with('.') && w != "i." && w.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
}

/// Drops empty/non-speech segments like "[MUSIC]" and trims text.
pub fn clean_segments(raw: Vec<Segment>) -> Vec<Segment> {
    raw.into_iter()
        .filter_map(|mut s| {
            let t = s.english.trim().to_string();
            let inner = t.trim_matches(|c: char| c == '[' || c == ']' || c == '(' || c == ')' || c == '*' || c == '♪' || c.is_whitespace());
            let bracketed = (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')'));
            if inner.is_empty() || bracketed || !t.chars().any(|c| c.is_alphabetic()) || s.end <= s.start {
                return None;
            }
            s.english = t;
            Some(s)
        })
        .collect()
}

fn cli(cfg: &Config, model: &Path, wav: &Path, tmp: &Path, cancel: &CancelToken, mut on_frac: impl FnMut(f32)) -> Result<Vec<Segment>> {
    let out_base = tmp.join("whisper_out");
    let mut cmd = silent_command(cfg.whisper_cli());
    cmd.arg("-m").arg(model).arg("-f").arg(wav).args(["-l", cfg.source_lang().whisper_code().unwrap_or("auto"), "-oj", "-pp"]).arg("-of").arg(&out_base);
    if cfg.asr.threads > 0 {
        cmd.args(["-t", &cfg.asr.threads.to_string()]);
    }
    if !cfg.asr.use_gpu {
        cmd.arg("-ng");
    }
    run_process(cmd, cancel, None, |_, line| {
        // "whisper_print_progress_callback: progress =  45%"
        if let Some(p) = line.split("progress =").nth(1) {
            if let Ok(v) = p.trim().trim_end_matches('%').trim().parse::<f32>() {
                on_frac(v / 100.0);
            }
        }
    })?
    .ok_or_err("whisper-cli")?;
    let json_path = out_base.with_extension("json");
    let text = std::fs::read_to_string(&json_path).with_context(|| format!("whisper-cli wrote no {}", json_path.display()))?;
    parse_cli_json(&text)
}

pub fn parse_cli_json(text: &str) -> Result<Vec<Segment>> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    let segs = v["transcription"].as_array().context("unexpected whisper JSON")?;
    Ok(segs
        .iter()
        .map(|s| Segment {
            start: s["offsets"]["from"].as_f64().unwrap_or(0.0) / 1000.0,
            end: s["offsets"]["to"].as_f64().unwrap_or(0.0) / 1000.0,
            english: s["text"].as_str().unwrap_or_default().trim().to_string(),
            japanese: String::new(),
        })
        .collect())
}

#[cfg(feature = "whisper")]
fn builtin(cfg: &Config, model: &Path, wav: &Path, cancel: &CancelToken, on_frac: impl FnMut(f32) + 'static) -> Result<Vec<Segment>> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let mut reader = hound::WavReader::open(wav)?;
    let samples: Vec<f32> = reader.samples::<i16>().map(|s| s.map(|v| v as f32 / 32768.0)).collect::<Result<_, _>>()?;

    let mut cparams = WhisperContextParameters::default();
    cparams.use_gpu(cfg.asr.use_gpu);
    let ctx = WhisperContext::new_with_params(model, cparams).context("could not load the Whisper model")?;
    let mut state = ctx.create_state().context("could not create Whisper state")?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(cfg.source_lang().whisper_code().unwrap_or("auto")));
    let threads = if cfg.asr.threads > 0 {
        cfg.asr.threads as i32
    } else {
        std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(4).min(8)
    };
    params.set_n_threads(threads);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    let mut on_frac = on_frac;
    params.set_progress_callback_safe(move |p: i32| on_frac(p as f32 / 100.0));
    let flag = cancel.flag();
    params.set_abort_callback_safe(move || flag.load(std::sync::atomic::Ordering::SeqCst));

    state.full(params, &samples).context("Whisper transcription failed")?;
    cancel.check()?;
    let mut out = Vec::new();
    for seg in state.as_iter() {
        out.push(Segment {
            start: seg.start_timestamp() as f64 / 100.0,
            end: seg.end_timestamp() as f64 / 100.0,
            english: seg.to_str_lossy().map(|s| s.to_string()).unwrap_or_default(),
            japanese: String::new(),
        });
    }
    Ok(out)
}

#[cfg(not(feature = "whisper"))]
fn builtin(_: &Config, _: &Path, _: &Path, _: &CancelToken, _: impl FnMut(f32) + 'static) -> Result<Vec<Segment>> {
    unreachable!("guarded by cfg!(feature = \"whisper\")")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seg(start: f64, end: f64, text: &str) -> Segment {
        Segment { start, end, english: text.into(), japanese: String::new() }
    }

    #[test]
    fn pieces_become_whole_sentences() {
        let pieces = vec![
            seg(0.0, 10.0, "First I want to add chapters. So I'm going to look"),
            seg(10.0, 16.0, "through it and say yeah, a chapter there."),
            seg(16.0, 20.0, "Click on Mr. Smith's button"),
            // a long pause closes the open line
            seg(25.0, 28.0, "and that's it."),
        ];
        let s = to_sentences(&pieces);
        let texts: Vec<&str> = s.iter().map(|x| x.english.as_str()).collect();
        assert_eq!(
            texts,
            vec!["First I want to add chapters.", "So I'm going to look through it and say yeah, a chapter there.", "Click on Mr. Smith's button", "and that's it."]
        );
        // "First I want to add chapters." is 29 of the 50 characters of the first piece.
        assert!((s[0].end - 5.8).abs() < 0.3, "end {}", s[0].end);
        assert!((s[1].start - s[0].end).abs() < 0.3);
        assert!((s[1].end - 16.0).abs() < 1e-9);
        assert!((s[3].start - 25.0).abs() < 1e-9);
    }

    #[test]
    fn parses_whisper_cli_json() {
        let j = r#"{"transcription":[{"offsets":{"from":0,"to":2500},"text":" Hello there."},{"offsets":{"from":2500,"to":4000},"text":" [MUSIC]"}]}"#;
        let s = clean_segments(parse_cli_json(j).unwrap());
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].english, "Hello there.");
        assert!((s[0].end - 2.5).abs() < 1e-9);
    }
}
