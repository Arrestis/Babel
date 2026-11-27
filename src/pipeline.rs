// SPDX-License-Identifier: GPL-3.0-or-later
//! The background job: Extract → OCR → Whisper → Translate → Voice → Render.
//! Runs on its own thread; talks to the UI only through `Reporter` messages.

use crate::asr::{self, Segment};
use crate::config::{AudioMode, Config};
use crate::ocr::{self, ChangeDetector, IntervalTracker, TextInterval};
use crate::overlay::{self, OverlayRenderer};
use crate::translate::{TextKind, Translator};
use crate::tts::{self, PlacedClip, Tts};
use crate::util::{CancelToken, LogLevel, Msg, Reporter, SetupInfo, Stage, fmt_bytes, fmt_ts, is_cancel_error};
use crate::video::{self, MediaInfo};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct Job {
    pub input: PathBuf,
    pub output: PathBuf,
    pub cfg: Config,
}

pub const INPUT_EXTENSIONS: [&str; 4] = ["mp4", "mkv", "mov", "avi"];
pub const OUTPUT_EXTENSIONS: [&str; 3] = ["mp4", "mkv", "mov"];

pub fn spawn(job: Job, rep: Reporter, cancel: CancelToken) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("babelcraft-pipeline".into())
        .spawn(move || {
            let tmp = temp_dir();
            // An internal error must end the job with a message, never leave the window waiting.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&job, &tmp, &rep, &cancel))).unwrap_or_else(|p| {
                Err(anyhow::anyhow!(
                    "internal error: {} (details in {})",
                    crate::logfile::panic_message(p.as_ref()),
                    crate::logfile::path().display()
                ))
            });
            if !job.cfg.tools.keep_temp_files {
                let _ = std::fs::remove_dir_all(&tmp);
            } else {
                rep.info(format!("Temporary files kept in {}", tmp.display()));
            }
            match result {
                Ok(path) => rep.send(Msg::Finished(path)),
                Err(e) if is_cancel_error(&e) || cancel.is_cancelled() => {
                    let _ = std::fs::remove_file(&job.output);
                    rep.send(Msg::Cancelled)
                }
                Err(e) => rep.send(Msg::Failed(format!("{e:#}"))),
            }
        })
        .expect("spawn pipeline thread")
}

fn temp_dir() -> PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    std::env::temp_dir().join(format!("babelcraft-{}-{stamp}", std::process::id()))
}

// ------------------------------------------------------------- saved results
// The slow steps' results are kept per video and settings, so running a job again (after
// fixing the translator, say) skips straight to the part that failed.

fn input_identity(input: &Path) -> String {
    let meta = std::fs::metadata(input).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let modified = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}|{size}|{modified}", input.display())
}

fn ocr_cache_key(cfg: &Config, input: &Path) -> String {
    let o = &cfg.ocr;
    format!(
        "screen-v2|{}|{:?}|{:?}|{}|{}|{}|{}|{}|{}|{}",
        input_identity(input),
        o.engine,
        o.sampling,
        o.interval_ms,
        o.change_threshold,
        o.min_confidence,
        o.min_duration_ms,
        o.min_text_height_percent,
        o.psm,
        ocr::ocr_language(cfg)
    )
}

fn asr_cache_key(cfg: &Config, input: &Path) -> String {
    let a = &cfg.asr;
    let engine = if a.engine == crate::config::AsrEngine::Api {
        format!("api|{}|{}|{}", a.api_service, a.api_url, a.api_model)
    } else {
        format!("{:?}|{}", a.engine, cfg.whisper_model().display())
    };
    format!("speech-v2|{}|{engine}|{}", input_identity(input), cfg.source_lang().code)
}

fn cache_file(kind: &str, key: &str) -> PathBuf {
    crate::config::data_dir().join("cache").join(format!("{kind}-{}.json", crate::util::short_hash(key)))
}

fn load_cache<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn save_cache<T: serde::Serialize>(path: &Path, value: &T) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(value) {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// Saved results older than 30 days are removed.
fn prune_cache() {
    let dir = crate::config::data_dir().join("cache");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let limit = std::time::Duration::from_secs(30 * 24 * 3600);
    for e in entries.flatten() {
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map(|age| age > limit).unwrap_or(false);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

const SAVED_NOTE: &str = "The on-screen text, the transcription and every line already translated are saved: \
    run the job again after fixing this and it carries on from here.";

fn stage_done(rep: &Reporter, s: Stage) {
    rep.progress(1.0);
    rep.send(Msg::StageDone(s));
}

fn validate(job: &Job) -> Result<()> {
    if !job.input.is_file() {
        bail!("Input video not found: {}", job.input.display());
    }
    let ext = job.input.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !INPUT_EXTENSIONS.contains(&ext.as_str()) {
        bail!("Unsupported input type .{ext} (use MP4, MKV, MOV or AVI)");
    }
    let oext = job.output.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !OUTPUT_EXTENSIONS.contains(&oext.as_str()) {
        bail!("Output must end in .mp4, .mkv or .mov");
    }
    if job.output == job.input {
        bail!("Output file must be different from the input file");
    }
    if let Some(dir) = job.output.parent() {
        if !dir.as_os_str().is_empty() && !dir.is_dir() {
            bail!("Output folder does not exist: {}", dir.display());
        }
    }
    Ok(())
}

/// First run: fetches anything missing (FFmpeg, models, font, Whisper tool, offline AI brain,
/// voice engine) into BabelCraft's own folder. The progress screen shows it as a one-time setup
/// with what is downloading, how much is done, the speed and the time left. A cancelled download
/// keeps what arrived and carries on from there next time.
fn first_use_setup(cfg: &Config, rep: &Reporter, cancel: &CancelToken) -> Result<()> {
    let missing = crate::tools::missing(cfg);
    if missing.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = missing.iter().map(|c| c.label()).collect();
    rep.info(format!("Setting up for first use: {} (one-time download)…", names.join(", ")));
    let count = missing.len();
    for (i, c) in missing.iter().enumerate() {
        cancel.check()?;
        let mut last_what = String::new();
        let mut last_sent: Option<Instant> = None;
        let mut resume_noted = false;
        let mut progress = |what: &str, f: f32| {
            let new_item = what != last_what;
            if new_item {
                rep.info(format!("  ↓ {what}"));
                last_what = what.to_string();
                resume_noted = false;
            }
            let download = crate::util::current_download();
            let mut news = new_item;
            if let Some(d) = download.filter(|d| d.resumed_from > 0 && !resume_noted) {
                resume_noted = true;
                news = true;
                let of = d.total.map(|t| format!(" of {}", fmt_bytes(t))).unwrap_or_default();
                rep.info(format!("    carrying on from {}{of} (kept from last time)", fmt_bytes(d.resumed_from)));
            }
            // A few updates a second are plenty for the screen.
            if news || last_sent.is_none_or(|t| t.elapsed() >= Duration::from_millis(200)) {
                last_sent = Some(Instant::now());
                let frac = download.and_then(|d| d.frac()).or((f >= 0.0).then_some(f));
                rep.send(Msg::Setup(Some(SetupInfo { index: i + 1, count, what: what.to_string(), frac, download })));
            }
        };
        crate::tools::install(*c, cfg, cancel, &mut progress).with_context(|| format!("automatic setup of {} failed", c.label()))?;
    }
    rep.send(Msg::Setup(None));
    rep.log(LogLevel::Ok, "Setup complete — everything BabelCraft needs is in place.");
    Ok(())
}

fn run(job: &Job, tmp: &Path, rep: &Reporter, cancel: &CancelToken) -> Result<PathBuf> {
    let mut adjusted = job.cfg.clone();
    let (src, dst) = (adjusted.source_lang(), adjusted.target_lang());
    ocr::set_source_english(src.code == "en");
    if src.code == dst.code {
        bail!("The video's language and the language to translate into are both {} — choose two different languages.", src.english);
    }
    // The built-in text reader only knows English; other languages are read with Tesseract.
    if src.code != "en" && adjusted.ocr.enabled {
        if adjusted.ocr.engine == crate::config::OcrEngineKind::Builtin {
            adjusted.ocr.engine = crate::config::OcrEngineKind::Tesseract;
        }
        let langs = crate::tools::tesseract_languages(&adjusted);
        let want = ocr::ocr_language(&adjusted);
        match langs {
            None => {
                rep.warn(format!("On-screen text in {} needs Tesseract (not found) — skipping on-screen text. Install Tesseract with the {} language data to translate it.", src.english, src.english));
                adjusted.ocr.enabled = false;
            }
            Some(l) if !l.is_empty() && !l.iter().any(|x| *x == want) => {
                rep.warn(format!("Tesseract has no '{want}' language data — skipping on-screen text. Install it to translate on-screen {} text.", src.english));
                adjusted.ocr.enabled = false;
            }
            _ => {}
        }
    }
    let cfg = &adjusted;
    let t0 = Instant::now();
    validate(job)?;
    std::fs::create_dir_all(tmp)?;

    let mut plan = vec![Stage::Extract];
    if cfg.ocr.enabled {
        plan.push(Stage::Ocr);
    }
    if cfg.asr.enabled {
        plan.push(Stage::Transcribe);
    }
    if cfg.ocr.enabled || cfg.asr.enabled {
        plan.push(Stage::Translate);
    }
    if cfg.asr.enabled {
        plan.push(Stage::Synthesize);
    }
    plan.push(Stage::Render);
    rep.send(Msg::Plan(plan.clone()));

    // ---------------------------------------------------------------- Extract
    rep.send(Msg::StageStart(Stage::Extract));
    rep.info(format!("Input:  {}", job.input.display()));
    rep.info(format!("Output: {}", job.output.display()));

    // Check the translator first: a missing key or a stopped server is reported now,
    // not after minutes of text reading and transcription.
    if cfg.ocr.enabled || cfg.asr.enabled {
        let name = crate::ai::provider(&cfg.translation.provider).map(|p| p.name).unwrap_or(cfg.translation.backend.label());
        rep.info(format!("Checking the translator ({name})…"));
        for w in crate::translate::preflight(cfg)? {
            rep.warn(w);
        }
    }
    prune_cache();

    first_use_setup(cfg, rep, cancel)?;
    let info = video::probe(cfg, &job.input, cancel)?;
    rep.info(format!(
        "Video {}x{}, {:.2} fps, {} long, audio: {}",
        info.width,
        info.height,
        info.fps,
        crate::util::fmt_duration(info.duration),
        if info.has_audio { "yes" } else { "none" }
    ));

    let font = if cfg.ocr.enabled {
        let mut found = crate::fonts::find_overlay_font(dst, &cfg.overlay.font_path);
        if found.is_none() {
            if let Some((file, url)) = crate::fonts::overlay_font_download(dst) {
                rep.info(format!("Downloading a font for {}…", dst.english));
                let mut quiet = |_: &str, _: f32| {};
                crate::tools::fetch(url, &crate::config::fonts_dir().join(file), "Font", cancel, &mut quiet)?;
                found = crate::fonts::find_overlay_font(dst, &cfg.overlay.font_path);
            }
        }
        let f = found.with_context(|| format!("No font with {} letters found — on-screen text would render as □□□. Choose a font in Settings → On-screen text.", dst.english))?;
        rep.info(format!("Font for {}: {}", dst.english, f.path.display()));
        Some(f)
    } else {
        None
    };
    let do_asr = cfg.asr.enabled && info.has_audio;
    if cfg.asr.enabled && !info.has_audio {
        rep.warn("The video has no audio track — skipping speech transcription and dubbing.");
        plan.retain(|s| *s != Stage::Transcribe && *s != Stage::Synthesize);
        if !cfg.ocr.enabled {
            plan.retain(|s| *s != Stage::Translate);
        }
        rep.send(Msg::Plan(plan.clone()));
    }

    let wav16 = tmp.join("speech_16k.wav");
    let wav48 = tmp.join("original_48k.wav");
    let need48 = do_asr && cfg.audio.mode == AudioMode::Duck;
    if do_asr {
        rep.info("Extracting 16 kHz mono audio for Whisper…");
        let span = if need48 { 0.5 } else { 1.0 };
        video::extract_audio_16k(cfg, &job.input, &wav16, &info, cancel, |f| rep.progress(f * span))?;
    }
    if need48 {
        rep.info("Extracting original soundtrack for mixing…");
        video::extract_audio_48k(cfg, &job.input, &wav48, &info, cancel, |f| rep.progress(0.5 + f * 0.5))?;
    }
    stage_done(rep, Stage::Extract);

    // ---------------------------------------------------------------- OCR
    let mut intervals: Vec<TextInterval> = Vec::new();
    if cfg.ocr.enabled {
        rep.send(Msg::StageStart(Stage::Ocr));
        let saved = cache_file("screen-text", &ocr_cache_key(cfg, &job.input));
        match load_cache::<Vec<TextInterval>>(&saved) {
            Some(v) => {
                rep.info(format!("Reusing the on-screen text read in an earlier run of this video ({} regions).", v.len()));
                intervals = v;
            }
            None => {
                intervals = scan_text(cfg, &job.input, &info, tmp, rep, cancel)?;
                save_cache(&saved, &intervals);
            }
        }
        rep.log(LogLevel::Ok, format!("Found {} distinct on-screen text regions.", intervals.len()));
        stage_done(rep, Stage::Ocr);
    }

    // ---------------------------------------------------------------- Whisper
    let mut segments: Vec<Segment> = Vec::new();
    if do_asr {
        rep.send(Msg::StageStart(Stage::Transcribe));
        if cfg.asr.engine == crate::config::AsrEngine::Api {
            let svc = crate::ai::speech_service(&cfg.asr.api_service);
            rep.info(format!("Transcribing with {} ({}) — the audio is uploaded to that service.", svc.name, cfg.asr.api_model));
        } else {
            rep.info(format!("Transcribing with Whisper model {}", cfg.whisper_model().display()));
        }
        let saved = cache_file("speech", &asr_cache_key(cfg, &job.input));
        match load_cache::<Vec<Segment>>(&saved) {
            Some(v) => {
                rep.info(format!("Reusing the transcription made in an earlier run of this video ({} lines).", v.len()));
                segments = v;
            }
            None => {
                let r2 = rep.clone();
                let r3 = rep.clone();
                segments = asr::transcribe(cfg, &wav16, tmp, cancel, move |f| r2.progress(f), move |m| r3.warn(m))?;
                save_cache(&saved, &segments);
            }
        }
        // Whisper's pieces often stop mid-sentence: translate and voice whole sentences.
        let pieces = segments.len();
        segments = asr::to_sentences(&segments);
        rep.log(LogLevel::Ok, format!("Transcribed {pieces} speech segments ({} sentences).", segments.len()));
        for s in segments.iter().take(5) {
            rep.info(format!("  [{} → {}] {}", fmt_ts(s.start), fmt_ts(s.end), s.english));
        }
        stage_done(rep, Stage::Transcribe);
    }

    // ---------------------------------------------------------------- Translate
    if !intervals.is_empty() || !segments.is_empty() {
        rep.send(Msg::StageStart(Stage::Translate));
        let mut tr = Translator::new(cfg);
        rep.info(format!("Translator: {}", crate::ai::provider(&cfg.translation.provider).map(|p| p.name).unwrap_or(cfg.translation.backend.label())));
        // If translating fails, say that nothing done so far is lost.
        let keep_note = |e: anyhow::Error| {
            if !is_cancel_error(&e) {
                rep.warn(SAVED_NOTE);
            }
            e
        };
        // The offline AI brain runs only while translating (stopped again when `brain` is dropped).
        let brain = tr.prepare(cancel, &mut |m| rep.info(m)).map_err(keep_note)?;
        let total = (intervals.len() + segments.len()).max(1);
        let screen_texts: Vec<String> = intervals.iter().map(|i| i.english.clone()).collect();
        let speech_texts: Vec<String> = segments.iter().map(|s| s.english.clone()).collect();
        if !screen_texts.is_empty() {
            rep.info(format!("Translating {} on-screen texts…", screen_texts.len()));
            let ja = tr
                .translate_all(&screen_texts, TextKind::OnScreen, cancel, |n| rep.progress(n as f32 / total as f32), |w| rep.warn(w))
                .map_err(keep_note)?;
            for (i, j) in intervals.iter_mut().zip(ja) {
                i.japanese = j;
            }
        }
        if !speech_texts.is_empty() {
            rep.info(format!("Translating {} speech segments…", speech_texts.len()));
            let base = intervals.len();
            let ja = tr
                .translate_all(&speech_texts, TextKind::Speech, cancel, |n| rep.progress((base + n) as f32 / total as f32), |w| rep.warn(w))
                .map_err(keep_note)?;
            for (s, j) in segments.iter_mut().zip(ja) {
                s.japanese = j;
            }
        }
        for i in intervals.iter().take(3) {
            rep.info(format!("  \"{}\" → 「{}」", i.english, i.japanese));
        }
        for s in segments.iter().take(3) {
            rep.info(format!("  \"{}\" → 「{}」", s.english, s.japanese));
        }
        // Text the translator kept in English (names, web addresses, codes, misread bits)
        // gets no box: a box with the same English would only hide the original.
        let before = intervals.len();
        intervals.retain(|i| is_translated(&i.english, &i.japanese, dst));
        if intervals.len() < before {
            rep.info(format!(
                "Left {} on-screen texts as they are (names, addresses or codes the translator kept in English).",
                before - intervals.len()
            ));
        }
        if brain.is_some() {
            drop(brain);
            rep.info("Offline AI brain stopped (memory freed for the next steps).");
        }
        stage_done(rep, Stage::Translate);
    } else if plan.contains(&Stage::Translate) {
        rep.send(Msg::StageStart(Stage::Translate));
        rep.warn("Nothing to translate (no on-screen text or speech found).");
        stage_done(rep, Stage::Translate);
    }

    // Japanese subtitle file next to the video (handy for checking the translation).
    if !segments.is_empty() {
        let srt = subtitle_path_for(&job.output, dst.code);
        if let Err(e) = write_srt(&srt, &segments) {
            rep.warn(format!("Could not write subtitles: {e}"));
        } else {
            rep.info(format!("{} subtitles saved: {}", dst.english, srt.display()));
        }
    }

    // ---------------------------------------------------------------- Voice
    let mut dub_audio: Option<PathBuf> = None;
    if do_asr {
        rep.send(Msg::StageStart(Stage::Synthesize));
        let speak: Vec<&Segment> = segments.iter().filter(|s| !s.japanese.trim().is_empty()).collect();
        if speak.is_empty() {
            rep.warn("No speech to dub — keeping the original audio.");
        } else {
            let clips = synthesize(cfg, &speak, &info, tmp, rep, cancel)?;
            let out = tmp.join("japanese_mix_48k.wav");
            let original = (cfg.audio.mode == AudioMode::Duck).then_some(wav48.as_path());
            rep.info(match cfg.audio.mode {
                AudioMode::Duck => format!("Mixing dub over original audio (ducked to {:.0}%)…", cfg.audio.duck_level_percent),
                AudioMode::Replace => "Building Japanese-only soundtrack (English voice replaced)…".into(),
            });
            tts::mix_soundtrack(cfg, original, &clips, info.duration, &out, cancel, |f| rep.progress(0.85 + f * 0.15))?;
            dub_audio = Some(out);
        }
        stage_done(rep, Stage::Synthesize);
    }

    // ---------------------------------------------------------------- Render
    rep.send(Msg::StageStart(Stage::Render));
    let mut overlay_list = None;
    if let (Some(font), false) = (font.as_ref(), intervals.is_empty()) {
        rep.info(format!("Drawing {} text overlays…", dst.english));
        let renderer = OverlayRenderer::new(cfg, font, dst)?;
        overlay_list = overlay::build_overlay_stream(
            &renderer,
            &intervals,
            info.width,
            info.height,
            info.duration,
            &tmp.join("overlays"),
            cancel,
            |f| rep.progress(f * 0.1),
        )?;
    }
    rep.info(if overlay_list.is_some() { "Encoding video with overlays (x264)…" } else { "Muxing video (stream copy, no re-encode)…" });
    let rjob = video::RenderJob { input: &job.input, output: &job.output, overlay_list: overlay_list.as_deref(), dub_audio: dub_audio.as_deref() };
    video::render_final(cfg, &rjob, &info, cancel, |f| rep.progress(0.1 + f * 0.9))?;
    stage_done(rep, Stage::Render);

    rep.log(
        LogLevel::Ok,
        format!("Done in {} — saved {}", crate::util::fmt_duration(t0.elapsed().as_secs_f64()), job.output.display()),
    );
    Ok(job.output.clone())
}

fn scan_text(cfg: &Config, input: &Path, info: &MediaInfo, tmp: &Path, rep: &Reporter, cancel: &CancelToken) -> Result<Vec<TextInterval>> {
    let interval = cfg.ocr.interval_ms.clamp(100, 10_000);
    let step = interval as f64 / 1000.0;
    let expected = ((info.duration / step).ceil() as usize).max(1);
    let (w, h) = video::ocr_frame_size(info);
    let scale = info.width as f32 / w as f32;
    rep.info(format!(
        "Sampling a frame every {interval} ms ({} frames, {}); OCR at {w}x{h}.",
        expected,
        match cfg.ocr.sampling {
            crate::config::OcrSampling::EveryInterval => "OCR on every sample",
            crate::config::OcrSampling::SceneChanges => "OCR only when the picture changes",
        }
    ));
    let frame_png = tmp.join("ocr_frame.png");
    let builtin = match cfg.ocr.engine {
        crate::config::OcrEngineKind::Builtin => {
            rep.info("Using the built-in text reader.");
            Some(ocr::BuiltinOcr::load()?)
        }
        crate::config::OcrEngineKind::Tesseract => None,
    };
    let mut detector = ChangeDetector::new();
    let mut tracker = IntervalTracker::new(step);
    let mut ocr_runs = 0usize;
    let mut skipped = 0usize;
    let mut warned = false;
    // Lines smaller than this (frame pixels) are menus and interface labels, not titles.
    let min_line_h = cfg.ocr.min_text_height_percent.clamp(0.0, 20.0) / 100.0 * h as f32;
    video::stream_gray_frames(cfg, input, info, interval, cancel, |idx, t, pixels| {
        rep.progress(idx as f32 / expected as f32);
        let thumb = ocr::thumbnail(pixels, w, h);
        if detector.needs_ocr(cfg, thumb) {
            let result = match &builtin {
                Some(engine) => engine.detect(pixels, w, h, scale, min_line_h),
                None => {
                    image::GrayImage::from_raw(w, h, pixels.to_vec()).context("frame size mismatch")?.save(&frame_png)?;
                    ocr::run_tesseract(cfg, &frame_png, scale, min_line_h, cancel)
                }
            };
            match result {
                Ok(dets) => tracker.push(t, &dets),
                Err(e) if crate::util::is_cancel_error(&e) => return Err(e),
                Err(e) => {
                    if !warned {
                        rep.warn(format!("Text reader problem (frame at {}): {e:#}", fmt_ts(t)));
                        warned = true;
                    }
                    if ocr_runs == 0 {
                        return Err(e.context("OCR could not run — see Setup checks"));
                    }
                }
            }
            ocr_runs += 1;
        } else {
            tracker.extend_all(t);
            skipped += 1;
        }
        Ok(())
    })?;
    rep.info(format!("OCR ran on {ocr_runs} frames; {skipped} unchanged frames skipped."));
    if let Some(n) = builtin.as_ref().map(|b| b.skipped_small.get()).filter(|n| *n > 0) {
        rep.info(format!(
            "Left out {n} lines of small text (under {:.1}% of the picture height: menus, buttons, labels). Settings → On-screen text changes this.",
            cfg.ocr.min_text_height_percent
        ));
    }
    let v = tracker.finish(info.duration, cfg.ocr.min_duration_ms as f64 / 1000.0);
    for i in v.iter().take(5) {
        rep.info(format!("  [{} → {}] \"{}\"", fmt_ts(i.start), fmt_ts(i.end), i.english));
    }
    Ok(v)
}

fn synthesize(cfg: &Config, speak: &[&Segment], info: &MediaInfo, tmp: &Path, rep: &Reporter, cancel: &CancelToken) -> Result<Vec<PlacedClip>> {
    let tts = Tts::new(cfg);
    // VOICEVOX is started (hidden) if needed and stopped again when `_voice` is dropped.
    let _voice = tts.prepare(cancel, &mut |m| rep.info(m))?;
    let dir = tmp.join("voice");
    std::fs::create_dir_all(&dir)?;
    let mut clips = Vec::new();
    let mut cursor = 0.0f64; // end of the previously placed clip (seconds)
    let max_tempo = cfg.audio.max_tempo.clamp(1.0, 2.5);
    let mut sped_up = 0;
    let mut shifted = 0;
    for (i, seg) in speak.iter().enumerate() {
        cancel.check()?;
        rep.progress(i as f32 / speak.len() as f32 * 0.85);
        let raw = tts.synthesize(&seg.japanese, &dir.join(format!("raw_{i:05}")), cancel).with_context(|| format!("voice synthesis failed for line {}", i + 1))?;
        let clip = dir.join(format!("clip_{i:05}.wav"));
        video::normalize_clip(cfg, &raw, &clip, 1.0, cancel)?;
        let len = tts::wav_duration(&clip);

        // Time available: until the next line starts (or the video ends).
        let next_start = speak.get(i + 1).map(|s| s.start).unwrap_or(info.duration.max(seg.end));
        let start = seg.start.max(cursor);
        let available = (next_start - start).max(seg.end - seg.start).max(0.2);
        let mut final_len = len;
        if len > available * 1.01 {
            let tempo = ((len / available) as f32).min(max_tempo);
            video::normalize_clip(cfg, &raw, &clip, tempo, cancel)?;
            final_len = tts::wav_duration(&clip);
            sped_up += 1;
        }
        if start > seg.start + 0.01 {
            shifted += 1;
        }
        let samples = tts::read_mono_48k(&clip)?;
        clips.push(PlacedClip { start_sample: (start * tts::RATE as f64) as usize, samples });
        cursor = start + final_len + 0.05;
    }
    rep.info(format!(
        "Synthesized {} lines ({} sped up to fit, {} nudged later to avoid overlap).",
        clips.len(),
        sped_up,
        shifted
    ));
    Ok(clips)
}

/// Contains Japanese writing (kana or kanji).
fn has_japanese(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{ff66}'..='\u{ff9f}'))
}

/// `video.ja.mp4` → `video.ja.srt`; `video.mp4` → `video.ja.srt`.
#[allow(dead_code)]
pub fn subtitle_path(output: &Path) -> PathBuf {
    subtitle_path_for(output, "ja")
}

pub fn subtitle_path_for(output: &Path, code: &str) -> PathBuf {
    let tag = format!(".{}", code.to_ascii_lowercase());
    let already = output.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase().ends_with(&tag)).unwrap_or(false);
    if already { output.with_extension("srt") } else { output.with_extension(format!("{code}.srt")) }
}

/// The translation is real text in the target language (not the original left as it was).
/// Japanese and Chinese are told by their characters; other languages by differing from the original.
fn is_translated(original: &str, translated: &str, dst: &crate::langs::Lang) -> bool {
    let t = translated.trim();
    if t.is_empty() {
        return false;
    }
    match dst.script {
        crate::langs::Script::Japanese => has_japanese(t),
        _ => !t.eq_ignore_ascii_case(original.trim()),
    }
}

fn write_srt(path: &Path, segs: &[Segment]) -> Result<()> {
    let ts = |s: f64| {
        let ms = (s.max(0.0) * 1000.0).round() as u64;
        format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000)
    };
    let mut out = String::new();
    for (i, s) in segs.iter().filter(|s| !s.japanese.is_empty()).enumerate() {
        out.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, ts(s.start), ts(s.end), s.japanese));
    }
    std::fs::write(path, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One job on any video with the person's own saved settings (translation switched to the
    /// free offline brain unless BABELCRAFT_TEST_KEEP_TRANSLATOR=1), output in target/e2e/file.
    /// Run with: BABELCRAFT_DATA_DIR=target/testdata BABELCRAFT_TEST_INPUT=<video>
    ///           cargo test job_on_file -- --ignored --nocapture
    /// (BABELCRAFT_TEST_WHISPER_CLI / BABELCRAFT_TEST_WHISPER_MODEL pick a whisper-cli and model.)
    #[test]
    #[ignore]
    fn job_on_file() {
        let input = std::path::absolute(std::env::var("BABELCRAFT_TEST_INPUT").expect("set BABELCRAFT_TEST_INPUT")).unwrap();
        let mut cfg = Config::load();
        if std::env::var("BABELCRAFT_TEST_KEEP_TRANSLATOR").is_err() {
            cfg.translation.backend = crate::config::TranslationBackend::BuiltinBrain;
            cfg.translation.provider = "brain".into();
        }
        cfg.tts.voicevox_url = "http://127.0.0.1:50121".into();
        if let Ok(p) = std::env::var("BABELCRAFT_TEST_WHISPER_CLI") {
            cfg.asr.whisper_cli_path = p;
        }
        if let Ok(p) = std::env::var("BABELCRAFT_TEST_WHISPER_MODEL") {
            cfg.asr.model_path = p;
        }
        cfg.tools.keep_temp_files = true;
        let dir = std::path::absolute("target/e2e/file").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let stem = input.file_stem().unwrap().to_string_lossy().to_string();
        let output = dir.join(format!("{stem}.ja.mp4"));
        let _ = std::fs::remove_file(&output);
        println!("input {}\noutput {}\nsettings: ocr={} asr={:?} translator={:?} voice={:?}", input.display(), output.display(), cfg.ocr.enabled, cfg.asr.engine, cfg.translation.backend, cfg.tts.backend);
        let (tx, rx) = crossbeam_channel::unbounded();
        let rep = Reporter::new(tx, std::sync::Arc::new(|| {}));
        let t0 = Instant::now();
        let handle = spawn(Job { input, output: output.clone(), cfg }, rep, CancelToken::default());
        let mut result = None;
        let mut last_tick = Instant::now();
        loop {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(Msg::Log(_, t)) => println!("[{:6.1}s]   {t}", t0.elapsed().as_secs_f32()),
                Ok(Msg::StageStart(s)) => println!("[{:6.1}s] ▶ {}", t0.elapsed().as_secs_f32(), s.label()),
                Ok(Msg::StageProgress(f)) => {
                    if last_tick.elapsed().as_secs() >= 10 {
                        println!("[{:6.1}s]     {:.0}%", t0.elapsed().as_secs_f32(), f * 100.0);
                        last_tick = Instant::now();
                    }
                }
                Ok(Msg::Finished(p)) => {
                    result = Some(Ok(p));
                    break;
                }
                Ok(Msg::Failed(e)) => {
                    result = Some(Err(e));
                    break;
                }
                Ok(Msg::Cancelled) => break,
                Ok(_) => {}
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if handle.is_finished() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let joined = handle.join();
        println!("job ended after {:.1} s; worker thread {}", t0.elapsed().as_secs_f32(), if joined.is_ok() { "ended normally" } else { "PANICKED" });
        match result {
            Some(Ok(p)) => println!("FINISHED: {}", p.display()),
            Some(Err(e)) => panic!("JOB FAILED: {e}"),
            None => panic!("the job ended without reporting a result (worker thread panicked: {})", joined.is_err()),
        }
    }

    /// Writes every on-screen text interval the OCR finds in a video to target/e2e/file/ocr.tsv.
    /// Run with: BABELCRAFT_DATA_DIR=target/testdata BABELCRAFT_TEST_INPUT=<video>
    ///           cargo test ocr_dump -- --ignored --nocapture
    #[test]
    #[ignore]
    fn ocr_dump() {
        let input = std::path::absolute(std::env::var("BABELCRAFT_TEST_INPUT").expect("set BABELCRAFT_TEST_INPUT")).unwrap();
        let cfg = Config::default();
        let cancel = CancelToken::default();
        let info = video::probe(&cfg, &input, &cancel).unwrap();
        let dir = std::path::absolute("target/e2e/file").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let rep = Reporter::new(tx, std::sync::Arc::new(|| {}));
        let t0 = Instant::now();
        let v = scan_text(&cfg, &input, &info, &dir, &rep, &cancel).unwrap();
        let mut out = String::from("start\tend\tx\ty\tw\th\ttext\n");
        for i in &v {
            out.push_str(&format!("{:.1}\t{:.1}\t{:.0}\t{:.0}\t{:.0}\t{:.0}\t{}\n", i.start, i.end, i.bbox.x, i.bbox.y, i.bbox.w, i.bbox.h, i.english));
        }
        std::fs::write(dir.join("ocr.tsv"), out).unwrap();
        println!("{} intervals in {:.1} s → target/e2e/file/ocr.tsv", v.len(), t0.elapsed().as_secs_f32());
    }

    #[test]
    fn boxes_only_for_japanese() {
        assert!(has_japanese("章やメニューを追加する"));
        assert!(has_japanese("ｱｲｳ"));
        assert!(!has_japanese("EDVC.UK.Com"));
        assert!(!has_japanese("Bride Submenu"));
        assert!(!has_japanese(""));
    }

    #[test]
    fn subtitle_names() {
        assert_eq!(subtitle_path(Path::new("v/lesson.ja.mp4")), PathBuf::from("v/lesson.ja.srt"));
        assert_eq!(subtitle_path(Path::new("v/lesson.mp4")), PathBuf::from("v/lesson.ja.srt"));
    }

    /// One whole job, fully offline, on a generated clip: an English title card plus English
    /// speech (Windows' own English voice) → OCR, Whisper, the offline AI brain, VOICEVOX,
    /// mixing and the final render. Downloads anything missing into the scratch folder.
    /// Run with: BABELCRAFT_DATA_DIR=target/testdata cargo test full_job_offline -- --ignored --nocapture
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn full_job_offline() {
        let data = std::env::var("BABELCRAFT_DATA_DIR").expect("set BABELCRAFT_DATA_DIR to a scratch folder");
        assert!(data.contains("target"), "use a scratch folder inside target/");
        let dir = std::path::PathBuf::from("target/e2e/full");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default();
        cfg.tts.voicevox_url = "http://127.0.0.1:50121".into();
        cfg.audio.mode = AudioMode::Duck;

        // English speech from the computer's English voice (written to a file, never played).
        let speech = std::path::absolute(dir.join("speech.wav")).unwrap();
        let script = "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
            $s.Rate = -1; $s.SetOutputToWaveFile($env:BC_OUT); \
            $s.Speak('Welcome to the course. Today we will learn how to cook perfect rice. Thank you for watching.'); $s.Dispose()";
        let st = crate::util::silent_command("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("BC_OUT", &speech)
            .status()
            .unwrap();
        assert!(st.success() && speech.is_file(), "Windows English voice not available");

        // Make sure FFmpeg is there to build the test clip.
        let cancel = CancelToken::default();
        if !crate::tools::works(&cfg.ffmpeg(), "-version") {
            crate::tools::install_ffmpeg(&cancel, &mut |_: &str, _: f32| {}).unwrap();
        }
        // Title card with dark English text on a light background.
        let font = crate::fonts::find_cjk_font("").expect("font");
        let r = OverlayRenderer::new(&cfg, &font, crate::langs::japanese()).unwrap();
        let mut card = image::RgbaImage::from_pixel(1280, 720, image::Rgba([250, 250, 245, 255]));
        let mut title = r.render_box("Cooking Basics: Perfect Rice", &ocr::BBox { x: 190.0, y: 300.0, w: 900.0, h: 100.0 }, 1280, 720).unwrap();
        for p in title.img.pixels_mut() {
            p.0 = [255 - p.0[0], 255 - p.0[1], 255 - p.0[2], p.0[3]];
        }
        for (x, y, p) in title.img.enumerate_pixels() {
            let (tx, ty) = (title.x as u32 + x, title.y as u32 + y);
            if p.0[3] > 0 && tx < 1280 && ty < 720 {
                let a = p.0[3] as f32 / 255.0;
                let d = card.get_pixel_mut(tx, ty);
                for i in 0..3 {
                    d.0[i] = (p.0[i] as f32 * a + d.0[i] as f32 * (1.0 - a)) as u8;
                }
            }
        }
        card.save(dir.join("card.png")).unwrap();
        let input = std::path::absolute(dir.join("lesson.mp4")).unwrap();
        let st = crate::util::silent_command(cfg.ffmpeg())
            .args(["-y", "-loglevel", "error", "-loop", "1", "-i"])
            .arg(dir.join("card.png"))
            .arg("-i")
            .arg(&speech)
            .args(["-af", "apad=pad_dur=1.5", "-shortest", "-r", "25", "-pix_fmt", "yuv420p", "-c:v", "libx264", "-tune", "stillimage", "-c:a", "aac"])
            .arg(&input)
            .status()
            .unwrap();
        assert!(st.success());

        // The real job, exactly as the app runs it.
        let output = std::path::absolute(dir.join("lesson.ja.mp4")).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        let rep = Reporter::new(tx, std::sync::Arc::new(|| {}));
        let t0 = Instant::now();
        let handle = spawn(Job { input: input.clone(), output: output.clone(), cfg: cfg.clone() }, rep, cancel);
        let mut finished = None;
        loop {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(Msg::Log(_, t)) => println!("  {t}"),
                Ok(Msg::StageStart(s)) => println!("▶ {}", s.label()),
                Ok(Msg::Finished(p)) => {
                    finished = Some(Ok(p));
                    break;
                }
                Ok(Msg::Failed(e)) => {
                    finished = Some(Err(e));
                    break;
                }
                Ok(Msg::Cancelled) => break,
                Ok(_) => {}
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
        }
        handle.join().unwrap();
        println!("job took {:.1} s", t0.elapsed().as_secs_f32());
        let out = finished.expect("job ended without a result").unwrap_or_else(|e| panic!("job failed: {e}"));
        let info = video::probe(&cfg, &out, &CancelToken::default()).unwrap();
        println!("output: {info:?}");
        assert!(info.has_audio && info.width == 1280);
        let srt_path = subtitle_path(&output);
        let srt = std::fs::read_to_string(&srt_path)
            .or_else(|_| std::fs::read_to_string(output.with_extension("ja.srt")))
            .unwrap();
        println!("subtitles:\n{srt}");
        assert!(srt.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{4e00}'..='\u{9fff}').contains(&c)));
        // A frame and the soundtrack to look at / listen to later.
        crate::util::silent_command(cfg.ffmpeg())
            .args(["-y", "-loglevel", "error", "-ss", "2", "-i"])
            .arg(&out)
            .args(["-frames:v", "1"])
            .arg(dir.join("output_frame.png"))
            .status()
            .unwrap();
    }

    /// The one-time setup reports sizes and speed to the progress screen, and a download that was
    /// cancelled part-way carries on from where it stopped. Downloads the OCR models (~12 MB) into
    /// a scratch folder (never the real app folder).
    /// Run with: BABELCRAFT_DATA_DIR=target/setuptest BABELCRAFT_TEST_FFMPEG=<path to ffmpeg>
    ///           cargo test setup_shows_and_resumes -- --ignored --nocapture
    #[test]
    #[ignore]
    fn setup_shows_and_resumes() {
        use crate::util::SetupInfo;
        let data = std::env::var("BABELCRAFT_DATA_DIR").expect("set BABELCRAFT_DATA_DIR to a scratch folder");
        assert!(data.contains("target"), "use a scratch folder inside target/");
        let _ = std::fs::remove_dir_all(crate::config::models_dir().join("ocrs"));
        let mut cfg = Config::default();
        cfg.asr.enabled = false;
        cfg.translation.backend = crate::config::TranslationBackend::DeepL;
        if let Ok(f) = std::env::var("BABELCRAFT_TEST_FFMPEG") {
            let probe = Path::new(&f).with_file_name(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" });
            cfg.tools.ffmpeg_path = f;
            cfg.tools.ffprobe_path = probe.display().to_string();
        }
        let missing = crate::tools::missing(&cfg);
        assert!(matches!(missing.as_slice(), [crate::tools::Component::OcrModels]), "only the OCR models should be missing: {missing:?}");

        let collect = |rx: crossbeam_channel::Receiver<Msg>| -> (Vec<SetupInfo>, Vec<String>, bool) {
            let (mut setups, mut logs, mut ended) = (Vec::new(), Vec::new(), false);
            for m in rx.try_iter() {
                match m {
                    Msg::Setup(Some(s)) => setups.push(s),
                    Msg::Setup(None) => ended = true,
                    Msg::Log(_, t) => logs.push(t),
                    _ => {}
                }
            }
            (setups, logs, ended)
        };

        // 1) Cancel once half a megabyte has arrived.
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = CancelToken::default();
        let (seen_tx, seen_rx) = crossbeam_channel::unbounded::<Msg>();
        let c2 = cancel.clone();
        let watcher = std::thread::spawn(move || {
            for m in rx.iter() {
                if let Msg::Setup(Some(s)) = &m {
                    if s.download.is_some_and(|d| d.done > 500_000) {
                        c2.cancel();
                    }
                }
                let _ = seen_tx.send(m);
            }
        });
        let rep = Reporter::new(tx, std::sync::Arc::new(|| {}));
        let r = first_use_setup(&cfg, &rep, &cancel);
        drop(rep);
        watcher.join().unwrap();
        let (setups, logs, ended) = collect(seen_rx);
        println!("first try: {:?}", r.as_ref().map_err(|e| format!("{e:#}")));
        for l in &logs {
            println!("  log: {l}");
        }
        assert!(r.as_ref().is_err_and(is_cancel_error), "the first try should stop when cancelled");
        assert!(!ended, "setup must not report completion after a cancel");
        let with_size = setups.iter().find(|s| s.download.is_some_and(|d| d.total.is_some())).expect("progress with sizes");
        println!("  e.g. {} · {:?} · {:?}", with_size.what, with_size.frac, with_size.download);
        assert_eq!((with_size.index, with_size.count), (1, 1));

        // 2) Start again: it carries on and finishes.
        let (tx, rx) = crossbeam_channel::unbounded();
        let rep = Reporter::new(tx, std::sync::Arc::new(|| {}));
        let t0 = Instant::now();
        first_use_setup(&cfg, &rep, &CancelToken::default()).unwrap();
        drop(rep);
        let (setups, logs, ended) = collect(rx);
        println!("second try ({:.1} s):", t0.elapsed().as_secs_f64());
        for l in &logs {
            println!("  log: {l}");
        }
        for s in setups.iter().filter(|s| s.download.is_some()) {
            let d = s.download.unwrap();
            println!(
                "  {} · {} · {:.0}% · {:.1} MB/s · left {:?}",
                s.what,
                crate::i18n::download_numbers(&d),
                s.frac.unwrap_or(0.0) * 100.0,
                d.per_sec / 1e6,
                d.eta_secs().map(|e| e.round())
            );
        }
        assert!(logs.iter().any(|l| l.contains("carrying on from")), "the second try should carry on from the kept part");
        assert!(ended, "setup should report completion");
        assert!(crate::tools::ocr_models_present());
        assert!(crate::util::current_download().is_none(), "download numbers are cleared afterwards");
    }

    /// Downloads FFmpeg + OCR models (into target/testdata, never the real app folder), makes a
    /// short video with English on-screen text, reads the text with the built-in OCR, and renders
    /// a Japanese overlay with the downloaded FFmpeg.
    /// Run with: BABELCRAFT_DATA_DIR=target/testdata cargo test --release -- --ignored --nocapture
    #[test]
    #[ignore]
    fn integrated_setup_ocr_and_render() {
        let data = std::env::var("BABELCRAFT_DATA_DIR").expect("set BABELCRAFT_DATA_DIR to a scratch folder");
        assert!(data.contains("target"), "use a scratch folder inside target/");
        let mut cfg = Config::default();
        cfg.asr.enabled = false;
        let cancel = CancelToken::default();
        let mut progress = |what: &str, f: f32| {
            if f == 0.0 {
                println!("setup: {what}");
            }
        };
        let installed = crate::tools::install_missing(&cfg, &cancel, &mut progress).unwrap();
        println!("installed: {installed:?}");
        assert!(crate::tools::missing(&cfg).is_empty(), "still missing: {:?}", crate::tools::missing(&cfg));
        println!("ffmpeg = {}", cfg.ffmpeg().display());

        // A 4 s 1280x720 clip with a title card.
        let dir = std::path::PathBuf::from("target/e2e");
        let _ = std::fs::create_dir_all(&dir);
        let font = crate::fonts::find_cjk_font("").expect("font");
        let r = OverlayRenderer::new(&cfg, &font, crate::langs::japanese()).unwrap();
        let mut card = image::RgbaImage::from_pixel(1280, 720, image::Rgba([250, 250, 245, 255]));
        let mut title = r.render_box("Welcome to the course", &ocr::BBox { x: 240.0, y: 260.0, w: 800.0, h: 90.0 }, 1280, 720).unwrap();
        for p in title.img.pixels_mut() {
            // dark text on light background: invert the box colours
            p.0 = [255 - p.0[0], 255 - p.0[1], 255 - p.0[2], p.0[3]];
        }
        for (x, y, p) in title.img.enumerate_pixels() {
            let (tx, ty) = (title.x as u32 + x, title.y as u32 + y);
            if p.0[3] > 0 && tx < 1280 && ty < 720 {
                let a = p.0[3] as f32 / 255.0;
                let d = card.get_pixel_mut(tx, ty);
                for i in 0..3 {
                    d.0[i] = (p.0[i] as f32 * a + d.0[i] as f32 * (1.0 - a)) as u8;
                }
            }
        }
        card.save(dir.join("card.png")).unwrap();
        let input = dir.join("input.mp4");
        let st = crate::util::silent_command(cfg.ffmpeg())
            .args(["-y", "-loglevel", "error", "-loop", "1", "-i"])
            .arg(dir.join("card.png"))
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-t", "4", "-r", "25", "-pix_fmt", "yuv420p", "-c:v", "libx264", "-c:a", "aac", "-shortest"])
            .arg(&input)
            .status()
            .unwrap();
        assert!(st.success());

        let info = video::probe(&cfg, &input, &cancel).unwrap();
        println!("probe: {info:?}");
        let (tx, _rx) = crossbeam_channel::unbounded();
        let rep = Reporter::new(tx, std::sync::Arc::new(|| {}));
        let mut intervals = scan_text(&cfg, &input, &info, &dir, &rep, &cancel).unwrap();
        println!("intervals: {intervals:?}");
        assert!(intervals.iter().any(|i| i.english.to_lowercase().contains("welcome")), "OCR should read the title");

        for i in &mut intervals {
            i.japanese = "コースへようこそ".into();
        }
        let list = overlay::build_overlay_stream(&r, &intervals, info.width, info.height, info.duration, &dir.join("ov"), &cancel, |_| {})
            .unwrap()
            .unwrap();
        let output = dir.join("output.ja.mp4");
        let job = video::RenderJob { input: &input, output: &output, overlay_list: Some(&list), dub_audio: None };
        video::render_final(&cfg, &job, &info, &cancel, |_| {}).unwrap();
        let out_info = video::probe(&cfg, &output, &cancel).unwrap();
        assert!((out_info.duration - info.duration).abs() < 0.5);
        // Grab a frame to look at.
        crate::util::silent_command(cfg.ffmpeg())
            .args(["-y", "-loglevel", "error", "-ss", "2", "-i"])
            .arg(&output)
            .args(["-frames:v", "1"])
            .arg(dir.join("output_frame.png"))
            .status()
            .unwrap();
    }
}
