//! Japanese text-to-speech: VOICEVOX (downloaded and started automatically), OpenAI-compatible
//! (/audio/speech: OpenAI voices, Kokoro-FastAPI...), the computer's own Japanese voice,
//! edge-tts and Piper. Nothing is ever played back — audio only goes to files.

use crate::config::{Config, TtsBackend};
use crate::services::StopOnDrop;
use crate::util::{CancelToken, http_agent, read_body_bytes, read_body_string, run_process, silent_command};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct Tts {
    cfg: Config,
    agent: ureq::Agent,
}

/// Lists (Windows) or uses (macOS) the Japanese voices built into the computer.
#[cfg(windows)]
const WINDOWS_VOICES: &str = "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
    $s.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -eq 'ja-JP' } | ForEach-Object { $_.VoiceInfo.Name }";

#[cfg(windows)]
const WINDOWS_SPEAK: &str = "$ErrorActionPreference = 'Stop'; Add-Type -AssemblyName System.Speech; \
    $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; $want = $env:BC_VOICE; \
    $v = $s.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -eq 'ja-JP' -and ($want -eq '' -or $_.VoiceInfo.Name -like \"*$want*\") } | Select-Object -First 1; \
    if (-not $v) { [Console]::Error.WriteLine('No Japanese voice is installed in Windows'); exit 3 }; \
    $s.SelectVoice($v.VoiceInfo.Name); $s.SetOutputToWaveFile($env:BC_OUT); \
    $s.Speak([IO.File]::ReadAllText($env:BC_TXT, [Text.Encoding]::UTF8)); $s.Dispose()";

/// Names of the computer's own Japanese voices.
pub fn system_voices() -> Vec<String> {
    #[cfg(windows)]
    let voices: Vec<String> = silent_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", WINDOWS_VOICES])
        .stdin(std::process::Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    let voices: Vec<String> = silent_command("say")
        .args(["-v", "?"])
        .stdin(std::process::Stdio::null())
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| l.contains("ja_JP"))
                .filter_map(|l| l.split("ja_JP").next().map(|n| n.trim().to_string()))
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default();
    #[cfg(not(any(windows, target_os = "macos")))]
    let voices: Vec<String> = Vec::new();
    voices
}

impl Tts {
    pub fn new(cfg: &Config) -> Self {
        Self { cfg: cfg.clone(), agent: http_agent(180) }
    }

    /// Starts what the chosen voice needs on this computer (the VOICEVOX engine).
    /// Keep the returned guard while synthesizing; dropping it stops what was started.
    pub fn prepare(&self, cancel: &CancelToken, status: &mut dyn FnMut(String)) -> Result<Option<StopOnDrop>> {
        match self.cfg.tts.backend {
            TtsBackend::Voicevox => crate::voicevox::ensure_running(&self.cfg, cancel, status),
            _ => Ok(None),
        }
    }

    /// Synthesizes `text` and returns the written audio file (WAV or MP3).
    pub fn synthesize(&self, text: &str, out_base: &Path, cancel: &CancelToken) -> Result<PathBuf> {
        cancel.check()?;
        let t = &self.cfg.tts;
        match t.backend {
            TtsBackend::Voicevox => {
                let base = t.voicevox_url.trim_end_matches('/');
                let mut resp = self
                    .agent
                    .post(format!("{base}/audio_query"))
                    .query("text", text)
                    .query("speaker", t.voicevox_speaker.to_string())
                    .send_empty()
                    .with_context(|| format!("could not reach VOICEVOX at {base} (is the VOICEVOX engine running?)"))?;
                let status = resp.status();
                let body = read_body_string(&mut resp)?;
                if !status.is_success() {
                    bail!("VOICEVOX audio_query error {status}: {body}");
                }
                let mut query: Value = serde_json::from_str(&body)?;
                query["outputSamplingRate"] = json!(48000);
                query["outputStereo"] = json!(false);
                let mut resp = self
                    .agent
                    .post(format!("{base}/synthesis"))
                    .query("speaker", t.voicevox_speaker.to_string())
                    .send_json(&query)?;
                let status = resp.status();
                let bytes = read_body_bytes(&mut resp)?;
                if !status.is_success() {
                    bail!("VOICEVOX synthesis error {status}: {}", String::from_utf8_lossy(&bytes));
                }
                let out = out_base.with_extension("wav");
                std::fs::write(&out, bytes)?;
                Ok(out)
            }
            TtsBackend::OpenAiCompatible => {
                let url = format!("{}/audio/speech", t.openai_url.trim_end_matches('/'));
                let mut req = self.agent.post(&url);
                let key = self.cfg.tts_openai_key();
                if !key.is_empty() {
                    req = req.header("Authorization", format!("Bearer {key}"));
                }
                let mut resp = req
                    .send_json(json!({ "model": t.openai_model, "voice": t.openai_voice, "input": text, "response_format": "wav" }))
                    .with_context(|| format!("could not reach TTS server at {url}"))?;
                let status = resp.status();
                let bytes = read_body_bytes(&mut resp)?;
                if !status.is_success() {
                    bail!("TTS error {status}: {}", String::from_utf8_lossy(&bytes));
                }
                // Some servers ignore response_format; sniff the header.
                let ext = if bytes.starts_with(b"RIFF") { "wav" } else { "mp3" };
                let out = out_base.with_extension(ext);
                std::fs::write(&out, bytes)?;
                Ok(out)
            }
            TtsBackend::EdgeTts => {
                let out = out_base.with_extension("mp3");
                let mut cmd = silent_command(&t.edge_tts_path);
                cmd.args(["--voice", &t.edge_voice, "--text", text, "--write-media"]).arg(&out);
                run_process(cmd, cancel, None, |_, _| {})?.ok_or_err("edge-tts")?;
                Ok(out)
            }
            TtsBackend::Piper => {
                if t.piper_model.trim().is_empty() {
                    bail!("Piper needs a Japanese voice model (.onnx) — set it in Settings → Japanese voice");
                }
                let out = out_base.with_extension("wav");
                let mut cmd = silent_command(&t.piper_path);
                cmd.arg("--model").arg(t.piper_model.trim()).arg("--output_file").arg(&out);
                run_process(cmd, cancel, Some(text), |_, _| {})?.ok_or_err("piper")?;
                Ok(out)
            }
            TtsBackend::System => system_say(text, out_base, t.system_voice.trim(), cancel),
        }
    }
}

/// The computer's own Japanese voice, written straight to a file (never to the speakers).
fn system_say(text: &str, out_base: &Path, voice: &str, cancel: &CancelToken) -> Result<PathBuf> {
    let txt = out_base.with_extension("txt");
    std::fs::write(&txt, text)?;
    #[cfg(windows)]
    let result = {
        let out = out_base.with_extension("wav");
        let mut cmd = silent_command("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", WINDOWS_SPEAK])
            .env("BC_OUT", &out)
            .env("BC_TXT", &txt)
            .env("BC_VOICE", voice);
        run_process(cmd, cancel, Some(""), |_, _| {})?.ok_or_err("Windows speech")?;
        Ok(out)
    };
    #[cfg(target_os = "macos")]
    let result = {
        let out = out_base.with_extension("aiff");
        let mut cmd = silent_command("say");
        cmd.args(["-v", if voice.is_empty() { "Kyoko" } else { voice }, "-o"]).arg(&out).arg("-f").arg(&txt);
        run_process(cmd, cancel, Some(""), |_, _| {})?.ok_or_err("macOS speech (say)")?;
        Ok(out)
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let result = {
        let _ = (voice, cancel);
        Err(anyhow::anyhow!(
            "Linux has no built-in Japanese voice — choose VOICEVOX (downloaded automatically) in Settings → Japanese voice"
        ))
    };
    result
}

/// A dubbed line already converted to 48 kHz mono and placed on the timeline.
pub struct PlacedClip {
    pub start_sample: usize,
    pub samples: Vec<i16>,
}

pub const RATE: usize = 48_000;

pub fn read_mono_48k(path: &Path) -> Result<Vec<i16>> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    if spec.sample_rate as usize != RATE || spec.channels != 1 {
        bail!("internal: clip is not 48 kHz mono");
    }
    Ok(r.samples::<i16>().collect::<Result<_, _>>()?)
}

/// Seconds of audio in a WAV file (0 for non-WAV/unknown).
pub fn wav_duration(path: &Path) -> f64 {
    hound::WavReader::open(path).map(|r| r.duration() as f64 / r.spec().sample_rate as f64).unwrap_or(0.0)
}

/// Mixes the dub clips with the original soundtrack (ducked) — or alone — into a 48 kHz stereo WAV.
/// Streams the original so long videos don't need it in memory.
pub fn mix_soundtrack(
    cfg: &Config,
    original: Option<&Path>,
    clips: &[PlacedClip],
    duration_secs: f64,
    out: &Path,
    cancel: &CancelToken,
    mut on_frac: impl FnMut(f32),
) -> Result<()> {
    let duck = (cfg.audio.duck_level_percent / 100.0).clamp(0.0, 1.0);
    let dub_gain = (cfg.audio.dub_volume_percent / 100.0).clamp(0.0, 2.0);
    let ramp = (0.25 * RATE as f64) as usize;

    // Speech regions (with a little padding) for the ducking envelope.
    let regions: Vec<(usize, usize)> = clips.iter().map(|c| (c.start_sample, c.start_sample + c.samples.len())).collect();
    let clips_end = regions.iter().map(|r| r.1).max().unwrap_or(0);
    let total = ((duration_secs * RATE as f64) as usize).max(clips_end);

    let mut orig_reader = match original {
        Some(p) => Some(hound::WavReader::open(p)?),
        None => None,
    };
    let mut orig_samples = orig_reader.as_mut().map(|r| r.samples::<i16>());

    let spec = hound::WavSpec { channels: 2, sample_rate: RATE as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(out, spec)?;

    let mut clip_idx = 0usize; // first clip that may still be active
    let mut reg_idx = 0usize;
    for i in 0..total {
        if i % (RATE * 5) == 0 {
            cancel.check()?;
            on_frac(i as f32 / total.max(1) as f32);
        }
        // Ducking gain for the original audio.
        let g = if cfg.audio.duck_whole_track {
            duck
        } else {
            while reg_idx < regions.len() && regions[reg_idx].1 + ramp < i {
                reg_idx += 1;
            }
            let mut dist = usize::MAX;
            for r in regions.iter().skip(reg_idx).take(3) {
                let d = if i < r.0 { r.0 - i } else if i > r.1 { i - r.1 } else { 0 };
                dist = dist.min(d);
            }
            if dist >= ramp { 1.0 } else { duck + (1.0 - duck) * (dist as f32 / ramp as f32) }
        };

        let (mut l, mut r) = match orig_samples.as_mut() {
            Some(it) => {
                let a = it.next().transpose()?.unwrap_or(0) as f32;
                let b = it.next().transpose()?.unwrap_or(0) as f32;
                (a * g, b * g)
            }
            None => (0.0, 0.0),
        };

        while clip_idx < clips.len() && clips[clip_idx].start_sample + clips[clip_idx].samples.len() <= i {
            clip_idx += 1;
        }
        for c in clips.iter().skip(clip_idx) {
            if c.start_sample > i {
                break;
            }
            if let Some(s) = c.samples.get(i - c.start_sample) {
                let v = *s as f32 * dub_gain;
                l += v;
                r += v;
            }
        }
        w.write_sample(l.clamp(-32768.0, 32767.0) as i16)?;
        w.write_sample(r.clamp(-32768.0, 32767.0) as i16)?;
    }
    w.finalize()?;
    on_frac(1.0);
    Ok(())
}

/// Quick reachability check used by Setup checks.
pub fn check_backend(cfg: &Config) -> (bool, String) {
    let t = &cfg.tts;
    match t.backend {
        TtsBackend::Voicevox => {
            if let Some(v) = crate::voicevox::reachable(&t.voicevox_url) {
                return (true, format!("VOICEVOX engine {v} is running"));
            }
            match (t.voicevox_auto_start, crate::voicevox::bundled_engine(), crate::voicevox::app_engine()) {
                (true, Some(_), _) => (true, "VOICEVOX engine set up by BabelCraft · starts by itself when needed".into()),
                (true, None, Some(_)) => (true, "VOICEVOX app found · its engine starts by itself when needed".into()),
                (true, None, None) if crate::voicevox::installable() => {
                    (false, format!("VOICEVOX voice engine not downloaded yet (free, {})", crate::voicevox::DOWNLOAD_SIZE))
                }
                _ => (false, format!("VOICEVOX engine not reachable at {} — start the VOICEVOX app", t.voicevox_url)),
            }
        }
        TtsBackend::System => {
            let voices = system_voices();
            match voices.first() {
                Some(v) => (true, format!("computer voice: {v}")),
                None if cfg!(any(windows, target_os = "macos")) => (false, "no Japanese voice installed on this computer".into()),
                None => (false, "Linux has no built-in Japanese voice — choose VOICEVOX".into()),
            }
        }
        TtsBackend::OpenAiCompatible => {
            if t.openai_url.contains("api.openai.com") && cfg.tts_openai_key().is_empty() {
                (false, "OpenAI voices need your OpenAI API key".into())
            } else {
                (true, format!("will use {} (voice {})", t.openai_url, t.openai_voice))
            }
        }
        TtsBackend::EdgeTts => match crate::util::probe_program(&t.edge_tts_path, &["--version"]) {
            Some(v) => (true, format!("edge-tts {v}")),
            None => (false, "edge-tts not found — install with `pip install edge-tts`".into()),
        },
        TtsBackend::Piper => match crate::util::probe_program(&t.piper_path, &["--help"]) {
            Some(_) if !t.piper_model.is_empty() => (true, "piper found".into()),
            Some(_) => (false, "piper found, but no Japanese voice model selected".into()),
            None => (false, "piper not found".into()),
        },
    }
}
