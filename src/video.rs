// SPDX-License-Identifier: GPL-3.0-or-later
//! FFmpeg / FFprobe wrappers: probing, audio extraction, frame streaming and final render.

use crate::config::Config;
use crate::util::{CancelToken, Cancelled, run_process, silent_command};
use anyhow::{Context, Result, bail};
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub duration: f64,
    /// Display size (after rotation metadata is applied).
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
    pub fps: f64,
}

pub fn probe(cfg: &Config, input: &Path, cancel: &CancelToken) -> Result<MediaInfo> {
    let mut cmd = silent_command(cfg.ffprobe());
    cmd.args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"]).arg(input);
    let out = run_process(cmd, cancel, None, |_, _| {})?.ok_or_err("ffprobe")?;
    let v: serde_json::Value = serde_json::from_str(&out.stdout).context("ffprobe returned invalid JSON")?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video").context("the file has no video stream")?;
    let mut info = MediaInfo {
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
        width: video["width"].as_u64().unwrap_or(0) as u32,
        height: video["height"].as_u64().unwrap_or(0) as u32,
        ..Default::default()
    };
    let rotation = video["side_data_list"]
        .as_array()
        .and_then(|l| l.iter().find_map(|d| d["rotation"].as_f64()))
        .or_else(|| video["tags"]["rotate"].as_str().and_then(|s| s.parse::<f64>().ok()))
        .unwrap_or(0.0);
    if (rotation.abs() as i64) % 180 == 90 {
        std::mem::swap(&mut info.width, &mut info.height);
    }
    info.duration = v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| video["duration"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0.0);
    info.fps = video["avg_frame_rate"]
        .as_str()
        .and_then(|r| {
            let (a, b) = r.split_once('/')?;
            let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
            (b > 0.0).then_some(a / b)
        })
        .unwrap_or(30.0);
    if info.width == 0 || info.height == 0 {
        bail!("could not read the video size");
    }
    Ok(info)
}

/// Parses FFmpeg `-progress pipe:1` lines and calls `on_frac` with 0..1.
fn progress_parser(duration: f64, mut on_frac: impl FnMut(f32)) -> impl FnMut(bool, &str) {
    move |is_err, line| {
        if is_err {
            return;
        }
        if let Some(v) = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")) {
            if let Ok(us) = v.trim().parse::<f64>() {
                if duration > 0.0 {
                    on_frac(((us / 1_000_000.0) / duration).clamp(0.0, 1.0) as f32);
                }
            }
        }
    }
}

fn ffmpeg(cfg: &Config) -> std::process::Command {
    // stdin stays open (no -nostdin) so a Cancel can send 'q' for a graceful stop.
    let mut c = silent_command(cfg.ffmpeg());
    c.args(["-hide_banner", "-y", "-loglevel", "error", "-nostats", "-progress", "pipe:1"]);
    c
}

/// Extracts mono 16 kHz PCM WAV (what Whisper expects).
pub fn extract_audio_16k(cfg: &Config, input: &Path, out: &Path, info: &MediaInfo, cancel: &CancelToken, on_frac: impl FnMut(f32)) -> Result<()> {
    let mut cmd = ffmpeg(cfg);
    cmd.arg("-i").arg(input).args(["-vn", "-map", "0:a:0", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"]).arg(out);
    run_process(cmd, cancel, None, progress_parser(info.duration, on_frac))?.ok_or_err("ffmpeg (16 kHz audio)")?;
    Ok(())
}

/// Extracts stereo 48 kHz PCM WAV of the original soundtrack (for mixing).
pub fn extract_audio_48k(cfg: &Config, input: &Path, out: &Path, info: &MediaInfo, cancel: &CancelToken, on_frac: impl FnMut(f32)) -> Result<()> {
    let mut cmd = ffmpeg(cfg);
    cmd.arg("-i").arg(input).args(["-vn", "-map", "0:a:0", "-ac", "2", "-ar", "48000", "-c:a", "pcm_s16le"]).arg(out);
    run_process(cmd, cancel, None, progress_parser(info.duration, on_frac))?.ok_or_err("ffmpeg (48 kHz audio)")?;
    Ok(())
}

/// Converts any audio file to mono 48 kHz s16 WAV, optionally speeding it up (atempo).
pub fn normalize_clip(cfg: &Config, input: &Path, out: &Path, tempo: f32, cancel: &CancelToken) -> Result<()> {
    let mut cmd = ffmpeg(cfg);
    cmd.arg("-i").arg(input);
    let mut filters = Vec::new();
    let mut t = tempo.clamp(0.25, 4.0);
    if (t - 1.0).abs() > 0.01 {
        // atempo accepts 0.5..2.0 per instance; chain for larger factors.
        while t > 2.0 {
            filters.push("atempo=2.0".to_string());
            t /= 2.0;
        }
        filters.push(format!("atempo={t:.4}"));
    }
    if !filters.is_empty() {
        cmd.arg("-af").arg(filters.join(","));
    }
    cmd.args(["-ac", "1", "-ar", "48000", "-c:a", "pcm_s16le"]).arg(out);
    run_process(cmd, cancel, None, |_, _| {})?.ok_or_err("ffmpeg (voice clip)")?;
    Ok(())
}

/// Size of the frames handed to OCR: small videos are upscaled (Tesseract likes
/// bigger glyphs), huge ones are downscaled. Always even.
pub fn ocr_frame_size(info: &MediaInfo) -> (u32, u32) {
    let target_w = info.width.clamp(1280, 1920) as f64;
    let scale = target_w / info.width as f64;
    let w = ((info.width as f64 * scale) as u32) & !1;
    let h = ((info.height as f64 * scale) as u32) & !1;
    (w.max(2), h.max(2))
}

/// Streams grayscale frames sampled every `interval_ms` straight from FFmpeg
/// (no images written to disk). `on_frame(index, time_secs, pixels)`.
pub fn stream_gray_frames(
    cfg: &Config,
    input: &Path,
    info: &MediaInfo,
    interval_ms: u32,
    cancel: &CancelToken,
    mut on_frame: impl FnMut(usize, f64, &[u8]) -> Result<()>,
) -> Result<()> {
    let (w, h) = ocr_frame_size(info);
    let fps = 1000.0 / interval_ms.max(50) as f64;
    let mut cmd = silent_command(cfg.ffmpeg());
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .arg("-i")
        .arg(input)
        .args(["-an", "-sn", "-vf"])
        .arg(format!("fps={fps:.6},scale={w}:{h}:flags=bicubic,format=gray"))
        .args(["-f", "rawvideo", "-pix_fmt", "gray", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("could not start ffmpeg")?;
    let mut stdout = child.stdout.take().context("no ffmpeg stdout")?;
    let stderr = child.stderr.take();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = stderr {
            let _ = e.read_to_string(&mut s);
        }
        s
    });

    let frame_len = (w * h) as usize;
    let mut buf = vec![0u8; frame_len];
    let mut idx = 0usize;
    let result: Result<()> = (|| {
        loop {
            if cancel.is_cancelled() {
                bail!(Cancelled);
            }
            match stdout.read_exact(&mut buf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
            let t = idx as f64 * interval_ms as f64 / 1000.0;
            on_frame(idx, t, &buf)?;
            idx += 1;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let err_text = err_thread.join().unwrap_or_default();
    result?;
    if !status.success() {
        bail!("ffmpeg frame extraction failed: {err_text}");
    }
    Ok(())
}

pub struct RenderJob<'a> {
    pub input: &'a Path,
    pub output: &'a Path,
    /// ffconcat list of full-frame transparent overlay PNGs (None = no on-screen text).
    pub overlay_list: Option<&'a Path>,
    /// Final mixed Japanese soundtrack (None = keep original audio as-is).
    pub dub_audio: Option<&'a Path>,
}

/// Composites overlays + muxes the Japanese audio into the output file.
pub fn render_final(cfg: &Config, job: &RenderJob, info: &MediaInfo, cancel: &CancelToken, on_frac: impl FnMut(f32)) -> Result<()> {
    let ext = job.output.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_ascii_lowercase();
    let mut cmd = ffmpeg(cfg);
    cmd.arg("-i").arg(job.input);
    let mut next_input = 1;
    let overlay_idx = job.overlay_list.map(|list| {
        cmd.args(["-f", "concat", "-safe", "0", "-i"]).arg(list);
        next_input += 1;
        next_input - 1
    });
    let audio_idx = job.dub_audio.map(|a| {
        cmd.arg("-i").arg(a);
        next_input += 1;
        next_input - 1
    });

    // Video
    if let Some(oi) = overlay_idx {
        cmd.arg("-filter_complex").arg(format!(
            "[{oi}:v]format=rgba,scale={w}:{h}[ov];[0:v][ov]overlay=0:0:eof_action=pass:format=auto,format=yuv420p[v]",
            w = info.width,
            h = info.height
        ));
        cmd.args(["-map", "[v]"]);
        cmd.args(["-c:v", "libx264", "-crf"]).arg(cfg.tools.video_crf.to_string());
        cmd.arg("-preset").arg(&cfg.tools.video_preset);
    } else {
        cmd.args(["-map", "0:v:0", "-c:v", "copy"]);
    }

    // Audio
    match audio_idx {
        Some(ai) => {
            cmd.arg("-map").arg(format!("{ai}:a:0"));
            if cfg.audio.keep_original_track && info.has_audio {
                cmd.args(["-map", "0:a:0?"]);
            }
            let dst = cfg.target_lang();
            cmd.args(["-c:a", "aac", "-b:a", "192k", "-metadata:s:a:0"]).arg(format!("language={}", dst.iso639_2())).arg("-metadata:s:a:0").arg(format!("title={} dub", dst.english));
            if cfg.audio.keep_original_track && info.has_audio {
                cmd.args(["-metadata:s:a:1", "language=eng", "-metadata:s:a:1", "title=Original English"]);
            }
        }
        None => {
            if info.has_audio {
                cmd.args(["-map", "0:a?", "-c:a", "copy"]);
            }
        }
    }
    cmd.args(["-map_metadata", "0", "-t"]).arg(format!("{:.3}", info.duration.max(0.1)));
    if ext == "mp4" || ext == "mov" || ext == "m4v" {
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.arg(job.output);
    run_process(cmd, cancel, None, progress_parser(info.duration, on_frac))?.ok_or_err("ffmpeg (final render)")?;
    Ok(())
}
