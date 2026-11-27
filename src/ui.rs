// SPDX-License-Identifier: GPL-3.0-or-later
//! The BabelCraft window: splash, file pickers, settings, multi-stage progress, setup checks,
//! the "AI models & downloads" page and the log.

use crate::ai::{self, PROVIDERS};
use crate::brain;
use crate::checks::{self, Check, Fix, Status};
use crate::config::{self, AsrEngine, AudioMode, Config, OcrEngineKind, OcrSampling, TranslationBackend, TtsBackend};
use crate::i18n::{tr, trf};
use crate::langs::{self, Lang};
use crate::pipeline::{self, Job};
use crate::tools::Component;
use crate::util::{CancelToken, DownloadStats, LogLevel, Msg, Reporter, SetupInfo, Stage, fmt_duration};
use crossbeam_channel::{Receiver, unbounded};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Layout, Margin, RichText,
    Sense, Stroke, TextureHandle, TextureOptions, vec2,
};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

// Palette taken from the icon: warm yellow tile, cream engraving, dark ink.
const BG: Color32 = Color32::from_rgb(23, 21, 15);
const PANEL: Color32 = Color32::from_rgb(31, 28, 20);
const CARD: Color32 = Color32::from_rgb(39, 35, 25);
const CARD_STROKE: Color32 = Color32::from_rgb(62, 56, 39);
const INPUT_BG: Color32 = Color32::from_rgb(18, 17, 12);
const YELLOW: Color32 = Color32::from_rgb(247, 194, 27);
const YELLOW_DIM: Color32 = Color32::from_rgb(120, 94, 20);
const CREAM: Color32 = Color32::from_rgb(242, 234, 211);
const MUTED: Color32 = Color32::from_rgb(163, 155, 132);
const INK: Color32 = Color32::from_rgb(26, 22, 10);
const GREEN: Color32 = Color32::from_rgb(110, 196, 108);
const RED: Color32 = Color32::from_rgb(229, 96, 77);
const ORANGE: Color32 = Color32::from_rgb(240, 160, 48);

const SPLASH_SECS: f32 = 1.8;
const SCREENSHOT_FRAME: u64 = 120;
const SCREENSHOT_SPLASH_FRAMES: u64 = 40;

struct LogLine {
    level: LogLevel,
    at: String,
    text: String,
}

struct RunState {
    rx: Receiver<Msg>,
    cancel: CancelToken,
    started: Instant,
    plan: Vec<Stage>,
    current: Option<Stage>,
    stage_frac: f32,
    stage_started: Instant,
    done: HashSet<Stage>,
    cancelling: bool,
    /// First-use downloads in progress (shown instead of the stage bars).
    setup: Option<SetupInfo>,
    /// When the real work started (after any first-use downloads), for the time-left estimate.
    work_started: Instant,
}

enum Outcome {
    None,
    Finished { path: PathBuf, secs: f64 },
    Failed(String),
    Cancelled,
}

enum DlMsg {
    Label(String),
    Progress(f32),
    Stats(Option<DownloadStats>),
    Done(Result<String, String>),
}

/// Something BabelCraft downloads or sets up in the background.
#[derive(Clone, PartialEq, Debug)]
enum Task {
    /// Everything the current settings still need.
    AutoInstall,
    Component(Component),
    WhisperModel(usize),
    BrainModel(&'static str),
    OllamaPull(String),
    /// Get the interface ready in this language (font + translation); `true` = translate it anew.
    Interface(String, bool),
}

struct Download {
    task: Task,
    what: String,
    rx: Receiver<DlMsg>,
    frac: f32,
    stats: Option<DownloadStats>,
    cancel: CancelToken,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Translate,
    Downloads,
}

/// What is already on this computer (worked out in the background for the Downloads page).
#[derive(Clone, Default)]
struct Inventory {
    ffmpeg: bool,
    ocr: bool,
    font: Option<String>,
    whisper_cli: bool,
    whisper_models: Vec<bool>,
    brain_engine: bool,
    brain_models: Vec<bool>,
    voicevox_bundled: bool,
    voicevox_app: bool,
    voicevox_running: Option<String>,
    ollama: Option<Vec<String>>,
    system_voices: Vec<String>,
}

fn take_inventory(cfg: &Config) -> Inventory {
    Inventory {
        ffmpeg: crate::tools::works(&cfg.ffmpeg(), "-version") && crate::tools::works(&cfg.ffprobe(), "-version"),
        ocr: crate::tools::ocr_models_present(),
        font: crate::fonts::find_cjk_font(&cfg.overlay.font_path).map(|f| f.path.display().to_string()),
        whisper_cli: cfg.uses_builtin_whisper() || crate::tools::works(&cfg.whisper_cli(), "--help"),
        whisper_models: WHISPER_MODELS.iter().map(|(f, _)| config::models_dir().join(f).is_file()).collect(),
        brain_engine: brain::engine_ready(cfg.translation.brain_gpu),
        brain_models: brain::MODELS.iter().map(|m| brain::model_ready(m.id)).collect(),
        voicevox_bundled: crate::voicevox::bundled_engine().is_some(),
        voicevox_app: crate::voicevox::app_engine().is_some(),
        voicevox_running: crate::voicevox::reachable(&cfg.tts.voicevox_url),
        ollama: ai::ollama_models(&cfg.translation.ollama_url),
        system_voices: crate::tts::system_voices(),
    }
}

/// Runs one background download / setup task.
fn run_task(task: &Task, cfg: &Config, cancel: &CancelToken, progress: crate::tools::Progress) -> anyhow::Result<String> {
    match task {
        Task::AutoInstall => {
            let done = crate::tools::install_missing(cfg, cancel, progress)?;
            Ok(if done.is_empty() { "nothing was missing".into() } else { done.iter().map(|c| c.label()).collect::<Vec<_>>().join(", ") })
        }
        Task::Component(c) => {
            crate::tools::install(*c, cfg, cancel, progress)?;
            Ok(c.label().to_string())
        }
        Task::WhisperModel(i) => {
            let file = WHISPER_MODELS[*i].0;
            let dest = config::models_dir().join(file);
            let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{file}");
            crate::tools::fetch(&url, &dest, &format!("Whisper model {file}"), cancel, progress)?;
            Ok(dest.display().to_string())
        }
        Task::BrainModel(id) => {
            if !brain::engine_ready(cfg.translation.brain_gpu) && brain::engine_installable() {
                brain::install_engine(cfg.translation.brain_gpu, cancel, progress)?;
            }
            brain::install_model(id, cancel, progress)?;
            Ok(brain::model(id).name.to_string())
        }
        Task::OllamaPull(name) => {
            ai::ollama_pull(&cfg.translation.ollama_url, name, cancel, progress)?;
            Ok(format!("{name} (in Ollama)"))
        }
        Task::Interface(code, again) => {
            let lang = langs::find(code).ok_or_else(|| anyhow::anyhow!("unknown language {code}"))?;
            crate::interface::prepare(cfg, lang, *again, cancel, progress)
        }
    }
}

fn task_label(task: &Task) -> String {
    match task {
        Task::AutoInstall => "components".into(),
        Task::Component(c) => c.label().into(),
        Task::WhisperModel(i) => format!("Whisper model {}", WHISPER_MODELS[*i].0),
        Task::BrainModel(id) => format!("AI brain model {}", brain::model(id).name),
        Task::OllamaPull(n) => format!("{n} into Ollama"),
        Task::Interface(code, _) => format!("interface language {}", langs::find(code).map(|l| l.english).unwrap_or(code)),
    }
}

struct Screenshot {
    path: PathBuf,
    requested: bool,
    saved: Arc<AtomicBool>,
}

pub struct BabelApp {
    cfg: Config,
    saved_cfg: Config,
    dirty_since: Option<Instant>,
    icon: TextureHandle,
    has_cjk_ui_font: bool,
    /// Interface language the fonts were set up for, and whether its letters can be shown.
    ui_fonts_for: String,
    ui_script_ok: bool,
    /// The language menu (speaking-mouth button in the header).
    lang_menu_open: bool,
    lang_search: String,
    lang_notice: String,
    /// Fonts for every writing system are loaded (so the menu can show each language's own name).
    menu_fonts: bool,
    /// Languages translated earlier by BabelCraft's AI (looked up now and then while the menu is open).
    lang_saved: Option<(Instant, HashSet<String>)>,
    started: Instant,
    frame: u64,
    splash_skipped: bool,
    screenshot: Option<Screenshot>,

    run: Option<RunState>,
    /// Scroll the page so the progress bars and time left are in view (on small screens the
    /// progress card starts below the visible part of the window).
    reveal_progress: bool,
    // Progress kept after a job ends so the bars stay visible.
    last_plan: Vec<Stage>,
    last_done: HashSet<Stage>,
    last_overall: f32,
    outcome: Outcome,

    logs: Vec<LogLine>,
    show_log: bool,
    show_settings: bool,

    checks: Vec<Check>,
    checks_rx: Option<Receiver<Vec<Check>>>,
    download: Option<Download>,
    whisper_choice: usize,
    test_rx: Option<Receiver<String>>,
    test_result: String,

    page: Page,
    inventory: Option<Inventory>,
    inventory_rx: Option<Receiver<Inventory>>,
    /// Model names a service reported (for the model picker), and which service they belong to.
    models_rx: Option<Receiver<Result<Vec<String>, String>>>,
    models_for: String,
    model_list: Vec<String>,
    model_list_err: String,
    ollama_pull_name: String,

    /// Test switches (environment variables, for reproducing problems):
    /// BABELCRAFT_TEST_AUTOSTART=1 starts a job with the saved files right after the splash,
    /// BABELCRAFT_TEST_MINIMIZE=<seconds> minimises the window that long after the start,
    /// BABELCRAFT_TEST_EXIT_WHEN_DONE=1 closes the app when the job ends.
    test_autostart: bool,
    test_minimize_after: Option<f32>,
    test_exit_when_done: bool,
    test_job_started: Option<Instant>,
    test_minimized: bool,
}

const WHISPER_MODELS: [(&str, &str); 5] = [
    ("ggml-tiny.en.bin", "tiny.en · 75 MB · fastest"),
    ("ggml-base.en.bin", "base.en · 142 MB · recommended"),
    ("ggml-small.en.bin", "small.en · 466 MB · better"),
    ("ggml-medium.en.bin", "medium.en · 1.5 GB · best (slow)"),
    ("ggml-large-v3-turbo.bin", "large-v3-turbo · 1.6 GB · GPU"),
];

impl BabelApp {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>) -> Self {
        let ctx = &cc.egui_ctx;
        let cfg = Config::load();
        let fonts = install_fonts(ctx, &cfg, false);
        let has_cjk_ui_font = fonts.japanese;
        apply_theme(ctx);

        let icon_img = image::load_from_memory(crate::ICON_PNG).map(|i| i.to_rgba8()).unwrap_or_default();
        let size = [icon_img.width() as usize, icon_img.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, icon_img.as_raw());
        let icon = ctx.load_texture("babelcraft-icon", color, TextureOptions::LINEAR);

        let whisper_choice = WHISPER_MODELS
            .iter()
            .position(|(f, _)| cfg.whisper_model().file_name().map(|n| n == *f).unwrap_or(false))
            .unwrap_or(1);

        let ui_fonts_for = cfg.ui_lang().to_string();
        let mut app = Self {
            saved_cfg: cfg.clone(),
            cfg,
            dirty_since: None,
            icon,
            has_cjk_ui_font,
            ui_fonts_for,
            ui_script_ok: fonts.script_ok,
            lang_menu_open: false,
            lang_search: String::new(),
            lang_notice: String::new(),
            menu_fonts: false,
            lang_saved: None,
            started: Instant::now(),
            frame: 0,
            splash_skipped: false,
            screenshot: screenshot.map(|path| Screenshot { path, requested: false, saved: Arc::new(AtomicBool::new(false)) }),
            run: None,
            reveal_progress: false,
            last_plan: Vec::new(),
            last_done: HashSet::new(),
            last_overall: 0.0,
            outcome: Outcome::None,
            logs: Vec::new(),
            show_log: true,
            show_settings: true,
            checks: Vec::new(),
            checks_rx: None,
            download: None,
            whisper_choice,
            test_rx: None,
            test_result: String::new(),
            page: if std::env::var("BABELCRAFT_PAGE").map(|p| p.eq_ignore_ascii_case("downloads")).unwrap_or(false) {
                Page::Downloads
            } else {
                Page::Translate
            },
            inventory: None,
            inventory_rx: None,
            models_rx: None,
            models_for: String::new(),
            model_list: Vec::new(),
            model_list_err: String::new(),
            ollama_pull_name: "qwen2.5:7b".into(),
            test_autostart: std::env::var("BABELCRAFT_TEST_AUTOSTART").is_ok_and(|v| v == "1"),
            test_minimize_after: std::env::var("BABELCRAFT_TEST_MINIMIZE").ok().and_then(|v| v.parse().ok()),
            test_exit_when_done: std::env::var("BABELCRAFT_TEST_EXIT_WHEN_DONE").is_ok_and(|v| v == "1"),
            test_job_started: None,
            test_minimized: false,
        };
        app.log(LogLevel::Info, format!("BabelCraft {} ready. Settings: {}", env!("CARGO_PKG_VERSION"), config::config_file().display()));
        app.log(LogLevel::Info, "Silent mode: nothing is ever played through your speakers.".into());
        if !has_cjk_ui_font {
            app.log(LogLevel::Warn, "No Japanese font found yet — see Setup checks.".into());
        }
        app.start_checks(ctx);
        if app.screenshot.is_some() && std::env::var("BABELCRAFT_SHOT_SETUP").is_ok_and(|v| v == "1") {
            app.show_sample_setup();
        }
        // Screenshot mode only: BABELCRAFT_SHOT_LANGMENU=1 photographs the open language menu
        // (any other value is typed into its search box).
        if let (Some(_), Ok(v)) = (&app.screenshot, std::env::var("BABELCRAFT_SHOT_LANGMENU")) {
            app.lang_menu_open = true;
            if v != "1" {
                app.lang_search = v;
            }
        }
        app
    }

    fn log(&mut self, level: LogLevel, text: String) {
        let at = clock_now();
        for line in text.lines() {
            self.logs.push(LogLine { level, at: at.clone(), text: line.to_string() });
        }
        if self.logs.len() > 5000 {
            self.logs.drain(..1000);
        }
        // Also kept in the log file (not for screenshot runs).
        if self.screenshot.is_none() {
            let tag = match level {
                LogLevel::Info => "",
                LogLevel::Ok => "ok: ",
                LogLevel::Warn => "WARNING: ",
                LogLevel::Error => "ERROR: ",
            };
            crate::logfile::append(&format!("{tag}{text}"));
        }
    }

    fn start_checks(&mut self, ctx: &egui::Context) {
        let (tx, rx) = unbounded();
        let cfg = self.cfg.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = checks::run_all(&cfg);
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.checks_rx = Some(rx);
    }

    fn refresh_inventory(&mut self, ctx: &egui::Context) {
        if self.inventory_rx.is_some() {
            return;
        }
        let (tx, rx) = unbounded();
        let cfg = self.cfg.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(take_inventory(&cfg));
            ctx.request_repaint();
        });
        self.inventory_rx = Some(rx);
    }

    /// Chooses a translation service and fills in its address and suggested model.
    fn apply_provider(&mut self, id: &str) {
        let Some(p) = ai::provider(id) else { return };
        let t = &mut self.cfg.translation;
        if t.provider == p.id {
            return;
        }
        t.provider = p.id.into();
        t.backend = p.backend;
        if matches!(p.backend, TranslationBackend::OpenAiCompatible | TranslationBackend::Anthropic) {
            if !p.url.is_empty() {
                t.openai_url = p.url.into();
            }
            t.openai_model = p.model.into();
        }
        self.model_list.clear();
        self.models_for.clear();
        self.model_list_err.clear();
        self.test_result.clear();
    }

    fn start_model_list(&mut self, ctx: &egui::Context) {
        let (tx, rx) = unbounded();
        let t = &self.cfg.translation;
        let backend = t.backend;
        let url = if backend == TranslationBackend::Ollama { t.ollama_url.clone() } else { t.openai_url.clone() };
        let key = self.cfg.chat_key();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(ai::list_models(backend, &url, &key).map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        self.models_for = self.cfg.translation.provider.clone();
        self.model_list_err.clear();
        self.models_rx = Some(rx);
    }

    fn running(&self) -> bool {
        self.run.is_some()
    }

    fn start_job(&mut self, ctx: &egui::Context) {
        if self.cfg.last_input.trim().is_empty() || self.cfg.last_output.trim().is_empty() {
            self.outcome = Outcome::Failed(tr("Choose an input video and an output file first.").into());
            return;
        }
        let input = PathBuf::from(self.cfg.last_input.trim());
        let output = PathBuf::from(self.cfg.last_output.trim());
        let _ = self.cfg.save();
        self.saved_cfg = self.cfg.clone();
        let (tx, rx) = unbounded();
        let c2 = ctx.clone();
        let rep = Reporter::new(tx, Arc::new(move || c2.request_repaint()));
        let cancel = CancelToken::default();
        pipeline::spawn(Job { input, output, cfg: self.cfg.clone() }, rep, cancel.clone());
        self.log(LogLevel::Info, "──────── New job started ────────".into());
        self.run = Some(RunState {
            rx,
            cancel,
            started: Instant::now(),
            plan: Stage::ALL.to_vec(),
            current: None,
            stage_frac: 0.0,
            stage_started: Instant::now(),
            done: HashSet::new(),
            cancelling: false,
            setup: None,
            work_started: Instant::now(),
        });
        self.outcome = Outcome::None;
        self.show_log = true;
        self.reveal_progress = true;
    }

    /// Screenshot mode only (BABELCRAFT_SHOT_SETUP=1): shows how a first-use download looks on the
    /// progress screen, with sample numbers and without downloading anything.
    fn show_sample_setup(&mut self) {
        let (_tx, rx) = unbounded();
        let ago = |s: u64| Instant::now().checked_sub(Duration::from_secs(s)).unwrap_or_else(Instant::now);
        self.run = Some(RunState {
            rx,
            cancel: CancelToken::default(),
            started: ago(492),
            plan: Stage::ALL.to_vec(),
            current: Some(Stage::Extract),
            stage_frac: 0.0,
            stage_started: ago(492),
            done: HashSet::new(),
            cancelling: false,
            setup: Some(SetupInfo {
                index: 2,
                count: 3,
                what: "AI brain model Qwen 3 · 4B".into(),
                frac: Some(0.53),
                download: Some(DownloadStats { done: 1_325_158_635, total: Some(2_497_281_120), per_sec: 2_700_000.0, resumed_from: 0 }),
            }),
            work_started: ago(492),
        });
        self.reveal_progress = true;
    }

    fn poll(&mut self, ctx: &egui::Context) {
        // Job messages
        let mut finished: Option<Outcome> = None;
        let mut new_logs = Vec::new();
        let mut scroll_to_bars = false;
        if let Some(run) = self.run.as_mut() {
            while let Ok(m) = run.rx.try_recv() {
                match m {
                    Msg::Plan(p) => run.plan = p,
                    Msg::StageStart(s) => {
                        run.current = Some(s);
                        run.stage_frac = 0.0;
                        run.stage_started = Instant::now();
                        new_logs.push((LogLevel::Info, format!("▶ {}", s.label())));
                    }
                    Msg::StageProgress(f) => run.stage_frac = f,
                    Msg::StageDone(s) => {
                        run.done.insert(s);
                        new_logs.push((LogLevel::Ok, format!("Done: {} ({})", s.label(), fmt_duration(run.stage_started.elapsed().as_secs_f64()))));
                    }
                    Msg::Log(l, t) => new_logs.push((l, t)),
                    Msg::Setup(s) => {
                        // The download lines make the card taller: bring the bars back into view.
                        scroll_to_bars |= s.is_some() && run.setup.is_none();
                        if s.is_none() && run.setup.is_some() {
                            // Downloads done: the real work (and its time estimate) starts now.
                            let now = Instant::now();
                            run.work_started = now;
                            run.stage_started = now;
                            run.stage_frac = 0.0;
                        }
                        run.setup = s;
                    }
                    Msg::Finished(p) => finished = Some(Outcome::Finished { path: p, secs: run.started.elapsed().as_secs_f64() }),
                    Msg::Failed(e) => {
                        new_logs.push((LogLevel::Error, format!("× {e}")));
                        finished = Some(Outcome::Failed(e));
                    }
                    Msg::Cancelled => {
                        new_logs.push((LogLevel::Warn, "Job cancelled; helper processes stopped and the partial file removed.".into()));
                        if run.setup.is_some() {
                            new_logs.push((LogLevel::Info, tr("What has downloaded so far is kept: next time it carries on where it stopped.").into()));
                        }
                        finished = Some(Outcome::Cancelled);
                    }
                }
            }
        }
        for (l, t) in new_logs {
            self.log(l, t);
        }
        self.reveal_progress |= scroll_to_bars;
        if let Some(o) = finished {
            if let Some(run) = self.run.take() {
                self.last_overall = if matches!(o, Outcome::Finished { .. }) { 1.0 } else { overall_of(&run) };
                self.last_plan = run.plan;
                self.last_done = run.done;
            }
            self.outcome = o;
            ctx.send_viewport_cmd(egui::ViewportCommand::Title("BabelCraft".into()));
        }

        // Setup checks
        if let Some(rx) = &self.checks_rx {
            if let Ok(c) = rx.try_recv() {
                self.checks = c;
                self.checks_rx = None;
            }
        }
        // Downloads
        let mut dl_done = None;
        if let Some(dl) = self.download.as_mut() {
            while let Ok(m) = dl.rx.try_recv() {
                match m {
                    DlMsg::Label(l) => dl.what = l,
                    DlMsg::Progress(f) => dl.frac = f,
                    DlMsg::Stats(s) => dl.stats = s,
                    DlMsg::Done(r) => dl_done = Some((dl.task.clone(), r)),
                }
            }
        }
        if let Some((task, r)) = dl_done {
            self.download = None;
            let what = task_label(&task);
            match r {
                Ok(done) => {
                    self.log(LogLevel::Ok, format!("Done: {what} — {done}"));
                    match &task {
                        // Fonts may have arrived: set them up again.
                        Task::AutoInstall | Task::Component(Component::Font) => self.ui_fonts_for.clear(),
                        Task::Interface(code, _) => {
                            self.cfg.ui_language = code.clone();
                            self.ui_fonts_for.clear();
                            self.lang_menu_open = false;
                            self.lang_notice.clear();
                        }
                        Task::WhisperModel(i) => {
                            let file = WHISPER_MODELS[*i].0;
                            self.cfg.asr.model_path = if file == config::DEFAULT_WHISPER_MODEL {
                                String::new()
                            } else {
                                config::models_dir().join(file).display().to_string()
                            };
                        }
                        Task::BrainModel(id) => self.cfg.translation.brain_model = id.to_string(),
                        Task::OllamaPull(name) => self.cfg.translation.ollama_model = name.clone(),
                        _ => {}
                    }
                }
                Err(e) => {
                    if matches!(task, Task::Interface(..)) {
                        self.lang_notice = format!("{}: {}", tr("Could not translate the buttons and menus"), e.lines().next().unwrap_or(""));
                    }
                    self.log(LogLevel::Error, format!("{what}: {e}"));
                }
            }
            self.start_checks(ctx);
            self.inventory_rx = None;
            self.refresh_inventory(ctx);
        }
        // Translation test
        if let Some(rx) = &self.test_rx {
            if let Ok(r) = rx.try_recv() {
                self.test_result = r;
                self.test_rx = None;
            }
        }
        // Model list of a service
        if let Some(rx) = &self.models_rx {
            if let Ok(r) = rx.try_recv() {
                match r {
                    Ok(list) => {
                        self.model_list = list;
                        self.model_list_err.clear();
                    }
                    Err(e) => {
                        self.model_list.clear();
                        self.model_list_err = e;
                    }
                }
                self.models_rx = None;
            }
        }
        // What's installed (Downloads page)
        if let Some(rx) = &self.inventory_rx {
            if let Ok(inv) = rx.try_recv() {
                self.inventory = Some(inv);
                self.inventory_rx = None;
            }
        }

        // Persist settings shortly after they change.
        if self.cfg != self.saved_cfg && self.screenshot.is_none() {
            let since = *self.dirty_since.get_or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_millis(800) {
                if let Err(e) = self.cfg.save() {
                    self.log(LogLevel::Warn, format!("Could not save settings: {e}"));
                }
                self.saved_cfg = self.cfg.clone();
                self.dirty_since = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(850));
            }
        }

        if let Some(run) = &self.run {
            let title = match &run.setup {
                Some(s) => format!(
                    "BabelCraft — setup: downloading{}",
                    s.frac.map(|f| format!(" {:.0}%", pct_down(f))).unwrap_or_default()
                ),
                None => format!(
                    "BabelCraft — {:.0}% · {}",
                    (overall_of(run) * 100.0).round(),
                    run.current.map(|s| s.short()).unwrap_or("Starting")
                ),
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.checks_rx.is_some() || self.download.is_some() || self.test_rx.is_some() || self.models_rx.is_some() || self.inventory_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    /// Handles a one-click fix offered by Setup checks.
    fn apply_fix(&mut self, fix: Fix, ctx: &egui::Context) {
        match fix {
            Fix::AutoInstall => self.start_task(Task::AutoInstall, ctx),
            Fix::UseBrain => {
                self.apply_provider("brain");
                self.log(LogLevel::Info, "Translation now uses BabelCraft's own offline AI brain.".into());
                self.start_task(Task::AutoInstall, ctx);
            }
        }
    }

    fn start_task(&mut self, task: Task, ctx: &egui::Context) {
        if self.download.is_some() {
            return;
        }
        let (tx, rx) = unbounded();
        let cancel = CancelToken::default();
        let c2 = cancel.clone();
        let ctx2 = ctx.clone();
        let cfg = self.cfg.clone();
        let what = task_label(&task);
        self.log(LogLevel::Info, format!("{} {what}…", tr("Downloading")));
        let t2 = task.clone();
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let ctx3 = ctx2.clone();
            let mut last = Instant::now() - Duration::from_secs(1);
            let mut progress = move |what: &str, f: f32| {
                if last.elapsed() > Duration::from_millis(100) || f == 0.0 {
                    last = Instant::now();
                    let _ = tx2.send(DlMsg::Label(what.to_string()));
                    let _ = tx2.send(DlMsg::Progress(f.max(0.0)));
                    // Sizes, speed and time left of the file downloading on this thread.
                    let _ = tx2.send(DlMsg::Stats(crate::util::current_download()));
                    ctx3.request_repaint();
                }
            };
            let r = run_task(&t2, &cfg, &c2, &mut progress).map_err(|e| format!("{e:#}"));
            let _ = tx.send(DlMsg::Done(r));
            ctx2.request_repaint();
        });
        self.download = Some(Download { task, what, rx, frac: 0.0, stats: None, cancel });
    }

    /// The shared progress line for whatever is downloading.
    fn download_bar(&mut self, ui: &mut egui::Ui) {
        if let Some(dl) = &self.download {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                let mut line = format!("{} {}", tr("Downloading"), crate::i18n::tr_step(&dl.what));
                if let Some(d) = &dl.stats {
                    line.push_str(" — ");
                    line.push_str(&crate::i18n::download_numbers(d));
                    if let Some(eta) = d.eta_secs() {
                        line.push_str(&crate::i18n::time_left(&fmt_duration(eta)));
                    }
                }
                ui.label(RichText::new(line).color(CREAM));
                if ui.small_button(tr("Stop")).clicked() {
                    dl.cancel.cancel();
                }
            });
            big_bar(ui, dl.stats.and_then(|d| d.frac()).unwrap_or(dl.frac), 10.0, YELLOW, "");
        }
    }
}

// ---------------------------------------------------------------------- panels
impl BabelApp {
    fn header(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(Margin::symmetric(16, 10)).stroke(Stroke::new(1.0, CARD_STROKE)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new(egui::load::SizedTexture::new(self.icon.id(), vec2(46.0, 46.0))).corner_radius(9));
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.add_space(2.0);
                        ui.label(RichText::new(tr("BabelCraft")).size(24.0).strong().color(YELLOW));
                        let sub = if self.has_cjk_ui_font {
                            tr("English → 日本語  ·  on-screen text + voice dubbing")
                        } else {
                            tr("English → Japanese  ·  on-screen text + voice dubbing")
                        };
                        ui.label(RichText::new(sub).size(13.0).color(MUTED));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        toggle_chip(ui, &mut self.show_log, tr("Log console"));
                        toggle_chip(ui, &mut self.show_settings, tr("Settings"));
                        // Interface language: one button with the speaking-mouth picture, opening
                        // a list of every language (each shown in its own writing).
                        ui.add_space(6.0);
                        let lang = langs::find(self.cfg.ui_lang()).unwrap_or_else(langs::english);
                        let busy = self.download.as_ref().is_some_and(|d| matches!(d.task, Task::Interface(..)));
                        let resp = language_button(ui, lang, busy, self.lang_menu_open)
                            .on_hover_text(format!("{} · Language", tr("Language of BabelCraft's buttons and menus")));
                        if resp.clicked() {
                            self.lang_menu_open = !self.lang_menu_open;
                        }
                        let mut open = self.lang_menu_open;
                        let close_behavior = if self.screenshot.is_some() {
                            egui::PopupCloseBehavior::IgnoreClicks
                        } else {
                            egui::PopupCloseBehavior::CloseOnClickOutside
                        };
                        egui::Popup::from_response(&resp)
                            .open_bool(&mut open)
                            .close_behavior(close_behavior)
                            .align(egui::RectAlign::BOTTOM_END)
                            .gap(6.0)
                            .width(380.0)
                            .frame(egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, YELLOW_DIM)).corner_radius(10).inner_margin(12))
                            .show(|ui| self.language_menu(ui));
                        // Closed by a click outside, or by choosing a language.
                        self.lang_menu_open = open && self.lang_menu_open;
                        ui.add_space(8.0);
                        let (txt, col) = if self.running() { (tr("● Working in the background"), YELLOW) } else { (tr("● Idle"), MUTED) };
                        ui.label(RichText::new(txt).size(13.0).color(col));
                    });
                });
            });
    }

    /// The list behind the speaking-mouth button: languages ready to use, then every other
    /// language (BabelCraft's AI translates the interface the first time one is chosen).
    fn language_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.cfg.ui_lang().to_string();
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            paint_language_icon(ui.painter(), r);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.add_space(3.0);
                ui.label(RichText::new(tr("Interface language")).size(16.0).strong().color(CREAM));
                ui.label(RichText::new(tr("Language of BabelCraft's buttons and menus")).size(12.0).color(MUTED));
            });
        });
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut self.lang_search).hint_text(tr("Search languages…")).desired_width(f32::INFINITY));

        // A language being set up (font download and/or translation).
        if let Some(dl) = &self.download {
            if let Task::Interface(code, _) = &dl.task {
                let name = langs::find(code).map(|l| l.native).unwrap_or(code);
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(trf("Translating BabelCraft into {0}…", &[name])).color(CREAM));
                    if ui.small_button(tr("Stop")).clicked() {
                        dl.cancel.cancel();
                    }
                });
                hint(ui, &crate::i18n::tr_step(&dl.what));
                big_bar(ui, dl.stats.and_then(|d| d.frac()).unwrap_or(dl.frac), 8.0, YELLOW, "");
            }
        }
        if !self.lang_notice.is_empty() {
            ui.add_space(6.0);
            banner(ui, ORANGE, |ui| {
                ui.label(RichText::new(&self.lang_notice).size(12.5).color(CREAM));
            });
        }

        // Saved AI translations, looked up every couple of seconds.
        if self.lang_saved.as_ref().is_none_or(|(t, _)| t.elapsed() > Duration::from_secs(2)) {
            let dir = config::data_dir().join("i18n");
            let saved = std::fs::read_dir(dir)
                .map(|rd| rd.flatten().filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_string())).collect())
                .unwrap_or_default();
            self.lang_saved = Some((Instant::now(), saved));
        }
        let saved = self.lang_saved.as_ref().map(|(_, s)| s.clone()).unwrap_or_default();
        let built_in: HashSet<&str> = crate::i18n::built_in_codes().chain(["en"]).collect();
        let q = self.lang_search.trim().to_lowercase();
        let wanted = |l: &Lang| q.is_empty() || l.english.to_lowercase().contains(&q) || l.native.to_lowercase().contains(&q) || l.code.to_lowercase() == q;
        let (ready, more): (Vec<&'static Lang>, Vec<&'static Lang>) =
            langs::LANGUAGES.iter().filter(|l| wanted(l)).partition(|l| built_in.contains(l.code) || saved.contains(l.code));

        let mut chosen: Option<(&'static str, bool)> = None;
        ui.add_space(6.0);
        // As tall as the window allows.
        let list_height = (ui.ctx().content_rect().height() - 250.0).clamp(160.0, 560.0);
        egui::ScrollArea::vertical()
            .max_height(list_height)
            .min_scrolled_height(list_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
            if !ready.is_empty() {
                ui.label(RichText::new(tr("Ready to use")).size(12.5).strong().color(YELLOW));
                for l in &ready {
                    let ai = saved.contains(l.code) && !built_in.contains(l.code);
                    let tag = if ai { tr("Translated by AI") } else { "" };
                    let row = language_row(ui, l, l.code == current, tag);
                    if row.clicked() {
                        chosen = Some((l.code, false));
                    }
                    if ai {
                        row.context_menu(|ui| {
                            if ui.button(tr("Translate again")).clicked() {
                                chosen = Some((l.code, true));
                            }
                        });
                    }
                }
            }
            if !more.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(tr("More languages — BabelCraft's AI translates the buttons and menus the first time you choose one")).size(12.5).strong().color(YELLOW));
                hint(ui, tr("Uses the translator chosen in Settings → Translation. The offline AI brain downloads about 2.5 GB the first time."));
                for l in &more {
                    if language_row(ui, l, false, "").clicked() {
                        chosen = Some((l.code, false));
                    }
                }
            }
            if ready.is_empty() && more.is_empty() {
                hint(ui, tr("No language matches."));
            }
        });
        if let Some((code, again)) = chosen {
            self.choose_language(code, again, ui.ctx());
        }
    }

    /// Switches the interface to `code`, getting it ready first if it needs a font or a translation.
    fn choose_language(&mut self, code: &'static str, again: bool, ctx: &egui::Context) {
        self.lang_notice.clear();
        let Some(lang) = langs::find(code) else { return };
        let (font, text) = crate::interface::needs(lang);
        if !font && !text && !again {
            self.cfg.ui_language = lang.code.to_string();
            self.lang_menu_open = false;
            self.lang_search.clear();
            return;
        }
        if self.running() {
            self.lang_notice = tr("Wait until the video has finished translating.").into();
            return;
        }
        if self.download.is_some() {
            self.lang_notice = tr("Wait until the current download has finished.").into();
            return;
        }
        if lang.script.rtl() {
            self.log(LogLevel::Info, tr("Right-to-left writing may show in the wrong order in places.").into());
        }
        self.start_task(Task::Interface(lang.code.to_string(), again), ctx);
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("log")
            .resizable(self.show_log)
            .default_size(150.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(Margin::symmetric(14, 8)).stroke(Stroke::new(1.0, CARD_STROKE)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let arrow = if self.show_log { "▼" } else { "▶" };
                    let title = RichText::new(format!("{arrow}  {}  ({} {})", tr("Log console"), self.logs.len(), tr("lines"))).strong().color(CREAM);
                    if ui.add(egui::Label::new(title).sense(Sense::click())).clicked() {
                        self.show_log = !self.show_log;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button(tr("Clear")).clicked() {
                            self.logs.clear();
                        }
                        if ui.small_button(tr("Copy all")).clicked() {
                            let text = self.logs.iter().map(|l| format!("{} {}", l.at, l.text)).collect::<Vec<_>>().join("\n");
                            ui.ctx().copy_text(text);
                        }
                    });
                });
                if self.show_log {
                    ui.add_space(4.0);
                    egui::Frame::new().fill(INPUT_BG).corner_radius(6).inner_margin(8).show(ui, |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                            for l in &self.logs {
                                let col = match l.level {
                                    LogLevel::Info => Color32::from_rgb(200, 193, 172),
                                    LogLevel::Warn => ORANGE,
                                    LogLevel::Error => RED,
                                    LogLevel::Ok => GREEN,
                                };
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(&l.at).monospace().size(11.5).color(MUTED));
                                    ui.label(RichText::new(&l.text).monospace().size(12.0).color(col));
                                });
                            }
                        });
                    });
                }
            });
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        if !self.show_settings {
            return;
        }
        let running = self.running();
        egui::Panel::left("settings")
            .resizable(true)
            .default_size(370.0)
            .min_size(300.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(Margin::symmetric(14, 12)).stroke(Stroke::new(1.0, CARD_STROKE)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Settings")).size(18.0).strong().color(CREAM));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button(tr("Reset to defaults")).clicked() {
                            // Keeps the chosen files and the interface language.
                            let (i, o, l) = (self.cfg.last_input.clone(), self.cfg.last_output.clone(), self.cfg.ui_language.clone());
                            self.cfg = Config { last_input: i, last_output: o, ui_language: l, ..Default::default() };
                        }
                    });
                });
                ui.label(RichText::new(tr("Saved automatically.")).size(12.0).color(MUTED));
                ui.add_space(6.0);
                // Screenshot mode only: BABELCRAFT_SHOT_ABOUT=1 photographs the About section.
                let shot_about = self.screenshot.is_some() && std::env::var("BABELCRAFT_SHOT_ABOUT").is_ok_and(|v| v == "1");
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_enabled_ui(!running, |ui| {
                        self.settings_translation(ui);
                        self.settings_audio(ui);
                        self.settings_ocr(ui);
                        self.settings_asr(ui);
                        self.settings_tts(ui);
                        self.settings_tools(ui);
                    });
                    // Always usable, even while a job runs.
                    settings_about(ui, shot_about);
                    if shot_about {
                        ui.scroll_to_cursor(Some(Align::BOTTOM));
                    }
                });
            });
    }
}

/// Version, licence and source code (the GPL's "appropriate legal notices").
fn settings_about(ui: &mut egui::Ui, open: bool) {
    section(ui, tr("About BabelCraft"), open, |ui| {
        ui.label(RichText::new(format!("BabelCraft {}", env!("CARGO_PKG_VERSION"))).strong().color(CREAM));
        hint(ui, "Copyright (C) 2026 BabelCraft contributors");
        ui.label(
            RichText::new(tr(
                "Free software: you may share and change it under the GNU General Public License, version 3 or (at your option) any later version. It comes with NO WARRANTY.",
            ))
            .size(12.5)
            .color(CREAM),
        );
        ui.horizontal_wrapped(|ui| {
            if link_button(ui, tr("Licence (GNU GPL v3) ↗")) {
                ai::open_url("https://www.gnu.org/licenses/gpl-3.0.html");
            }
            if link_button(ui, tr("Source code ↗")) {
                ai::open_url("https://github.com/Arrestis/Babel");
            }
        });
    });
}

// -------------------------------------------------------------------- settings
impl BabelApp {
    fn settings_translation(&mut self, ui: &mut egui::Ui) {
        let mut test_clicked = false;
        let mut list_clicked = false;
        let mut picked: Option<&'static str> = None;
        let mut link: Option<&'static str> = None;
        let mut download: Option<Task> = None;
        let mut go_downloads = false;
        let testing = self.test_rx.is_some();
        let test_result = self.test_result.clone();
        let listing = self.models_rx.is_some();
        let same_service = self.models_for == self.cfg.translation.provider;
        let model_list = if same_service { self.model_list.clone() } else { Vec::new() };
        let list_err = if same_service { self.model_list_err.clone() } else { String::new() };
        let can_download = self.download.is_none();
        let Config { translation: t, api_keys: keys, .. } = &mut self.cfg;
        section(ui, tr("Translation"), true, |ui| {
            let current = ai::provider(&t.provider);
            ui.label(RichText::new(tr("Who translates")).color(MUTED));
            egui::ComboBox::from_id_salt("tr_provider")
                .width(ui.available_width() - 4.0)
                .height(520.0)
                .selected_text(tr(current.map(|p| p.name).unwrap_or("Other OpenAI-compatible server")))
                .show_ui(ui, |ui| {
                    ui.label(RichText::new(tr("On this computer — free, private, offline")).size(11.5).color(YELLOW));
                    for p in PROVIDERS.iter().filter(|p| p.local) {
                        if ui.selectable_label(t.provider == p.id, tr(p.name)).clicked() {
                            picked = Some(p.id);
                        }
                    }
                    ui.separator();
                    ui.label(RichText::new(tr("Online services — need an account and API key")).size(11.5).color(YELLOW));
                    for p in PROVIDERS.iter().filter(|p| !p.local) {
                        if ui.selectable_label(t.provider == p.id, tr(p.name)).clicked() {
                            picked = Some(p.id);
                        }
                    }
                });
            ui.add_space(4.0);
            match t.backend {
                TranslationBackend::BuiltinBrain => {
                    let m = brain::model(&t.brain_model);
                    ui.horizontal(|ui| {
                        ui.add_sized(vec2(84.0, 20.0), egui::Label::new(RichText::new(tr("AI model")).color(MUTED)));
                        egui::ComboBox::from_id_salt("brain_model")
                            .width(ui.available_width() - 4.0)
                            .selected_text(format!("{} · {}", tr(m.name), m.size))
                            .show_ui(ui, |ui| {
                                for bm in brain::MODELS {
                                    let mark = if brain::model_ready(bm.id) { format!("  · {}", tr("downloaded")) } else { String::new() };
                                    ui.selectable_value(&mut t.brain_model, bm.id.to_string(), format!("{} · {}{mark}", tr(bm.name), bm.size));
                                }
                            });
                    });
                    hint(ui, tr(m.note));
                    if !cfg!(target_os = "macos") {
                        ui.checkbox(&mut t.brain_gpu, tr("Use the graphics card (faster; Vulkan)"));
                    }
                    let ready = brain::model_ready(m.id) && brain::engine_ready(t.brain_gpu);
                    ui.horizontal(|ui| {
                        if ready {
                            ui.label(RichText::new(tr("Downloaded — ready to translate offline")).size(12.5).color(GREEN));
                        } else {
                            ui.label(RichText::new(tr("Downloads by itself on first use")).size(12.5).color(ORANGE));
                            if ui.add_enabled(can_download, egui::Button::new(tr("Download now"))).clicked() {
                                download = Some(Task::BrainModel(m.id));
                            }
                        }
                    });
                    hint(ui, tr("Runs on your computer: nothing is sent online, no account or key needed."));
                    if ui.small_button(tr("All models & downloads…")).clicked() {
                        go_downloads = true;
                    }
                }
                TranslationBackend::Ollama => {
                    field(ui, tr("Server URL"), &mut t.ollama_url);
                    model_field(ui, &mut t.ollama_model, &model_list, listing, &list_err, &mut list_clicked);
                    hint(ui, tr("Runs fully offline. Try qwen2.5:7b, gemma2:9b or llama3.1:8b."));
                }
                TranslationBackend::OpenAiCompatible => {
                    field(ui, tr("Base URL"), &mut t.openai_url);
                    key_field(ui, keys, &t.provider, tr("API key"));
                    model_field(ui, &mut t.openai_model, &model_list, listing, &list_err, &mut list_clicked);
                    if current.map(|p| p.local).unwrap_or(false) {
                        hint(ui, tr("Start the app's local server first, then press List to pick a loaded model."));
                    } else {
                        hint(ui, tr("Press List to see the models your account can use."));
                    }
                }
                TranslationBackend::Anthropic => {
                    key_field(ui, keys, "anthropic", tr("API key"));
                    model_field(ui, &mut t.openai_model, &model_list, listing, &list_err, &mut list_clicked);
                    hint(ui, tr("claude-opus-5-5 is the most capable; claude-sonnet-5-5 and claude-haiku-5-5 cost less."));
                }
                TranslationBackend::DeepL => {
                    key_field(ui, keys, "deepl", tr("API key"));
                    hint(ui, tr("Free keys end in :fx. Empty key uses DEEPL_AUTH_KEY."));
                }
                TranslationBackend::LibreTranslate => {
                    field(ui, tr("Server URL"), &mut t.libre_url);
                    key_field(ui, keys, "libre", tr("API key"));
                    hint(ui, tr("Your own LibreTranslate server (no key needed) or a hosted one with a key."));
                }
            }
            if let Some(p) = current.filter(|p| !p.link.is_empty() && p.id != "brain") {
                let label = if p.local { tr("Get the app ↗") } else { tr("Get an API key ↗") };
                if ui.small_button(label).on_hover_text(p.link).clicked() {
                    link = Some(p.link);
                }
            }
            ui.horizontal(|ui| {
                ui.label(tr("Lines per request"));
                ui.add(egui::DragValue::new(&mut t.batch_size).range(1..=50));
            });
            ui.label(tr("Style / glossary hint"));
            ui.add(egui::TextEdit::multiline(&mut t.style_hint).desired_rows(2).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                if ui.add_enabled(!testing, egui::Button::new(tr("Test translation"))).clicked() {
                    test_clicked = true;
                }
                if testing {
                    ui.spinner();
                }
            });
            if !test_result.is_empty() {
                let col = if test_result.starts_with('×') { RED } else { CREAM };
                ui.add(egui::Label::new(RichText::new(&test_result).size(12.0).color(col)).wrap());
            }
        });
        let ctx = ui.ctx().clone();
        if let Some(id) = picked {
            self.apply_provider(id);
        }
        if let Some(url) = link {
            ai::open_url(url);
        }
        if let Some(task) = download {
            self.start_task(task, &ctx);
        }
        if go_downloads {
            self.page = Page::Downloads;
            self.refresh_inventory(&ctx);
        }
        if list_clicked {
            self.start_model_list(&ctx);
        }
        if test_clicked {
            let (tx, rx) = unbounded();
            let cfg = self.cfg.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let r = crate::translate::Translator::new(&cfg)
                    .without_memory()
                    .test(&CancelToken::default())
                    .map(|s| format!("→ 「{s}」"))
                    .unwrap_or_else(|e| format!("× {e:#}"));
                let _ = tx.send(r);
                ctx.request_repaint();
            });
            self.test_rx = Some(rx);
            self.test_result = if self.cfg.translation.backend == TranslationBackend::BuiltinBrain {
                tr("Testing… (starting the AI brain)").into()
            } else {
                tr("Testing…").into()
            };
        }
    }

    fn settings_audio(&mut self, ui: &mut egui::Ui) {
        section(ui, tr("Audio mixing"), true, |ui| {
            let a = &mut self.cfg.audio;
            ui.radio_value(&mut a.mode, AudioMode::Replace, tr("Replace the English voice (Japanese only)"));
            ui.radio_value(&mut a.mode, AudioMode::Duck, tr("Duck original audio under the Japanese dub"));
            ui.add_enabled_ui(a.mode == AudioMode::Duck, |ui| {
                ui.add(egui::Slider::new(&mut a.duck_level_percent, 0.0..=60.0).suffix(" %").text(tr("original volume")));
                ui.checkbox(&mut a.duck_whole_track, tr("Duck the whole video (not only while speaking)"));
            });
            ui.add(egui::Slider::new(&mut a.dub_volume_percent, 30.0..=150.0).suffix(" %").text(tr("dub volume")));
            ui.add(egui::Slider::new(&mut a.max_tempo, 1.0..=2.0).fixed_decimals(2).suffix("×").text(tr("max speed-up")));
            hint(ui, tr("Lines that run long are sped up (up to this limit) so they end before the next line."));
            ui.checkbox(&mut a.keep_original_track, tr("Also keep the English audio as a 2nd track"));
        });
    }

    fn settings_ocr(&mut self, ui: &mut egui::Ui) {
        section(ui, tr("On-screen text (OCR)"), false, |ui| {
            let o = &mut self.cfg.ocr;
            ui.checkbox(&mut o.enabled, tr("Translate on-screen English text"));
            ui.add_enabled_ui(o.enabled, |ui| {
                ui.radio_value(&mut o.sampling, OcrSampling::SceneChanges, tr("Only when the picture changes (fast)"));
                ui.radio_value(&mut o.sampling, OcrSampling::EveryInterval, tr("Every sample"));
                ui.horizontal(|ui| {
                    ui.label(tr("Sample every"));
                    ui.add(egui::DragValue::new(&mut o.interval_ms).range(100..=10_000).speed(50).suffix(" ms"));
                });
                ui.add_enabled_ui(o.sampling == OcrSampling::SceneChanges, |ui| {
                    ui.add(egui::Slider::new(&mut o.change_threshold, 0.005..=0.2).fixed_decimals(3).text(tr("change sensitivity")));
                });
                ui.add(egui::Slider::new(&mut o.min_confidence, 0.0..=95.0).text(tr("min confidence")));
                ui.horizontal(|ui| {
                    ui.label(tr("Ignore text shown less than"));
                    ui.add(egui::DragValue::new(&mut o.min_duration_ms).range(0..=10_000).speed(50).suffix(" ms"));
                });
                ui.add(egui::Slider::new(&mut o.min_text_height_percent, 0.0..=8.0).fixed_decimals(1).suffix(" %").text(tr("smallest text size")));
                hint(ui, tr("Smaller lines (menus, buttons and labels in screen recordings) are left out. 0 keeps everything."));
                ui.horizontal(|ui| {
                    ui.label(tr("Layout mode"));
                    egui::ComboBox::from_id_salt("psm").selected_text(tr(psm_label(o.psm))).show_ui(ui, |ui| {
                        for p in [3u32, 6, 11, 12] {
                            ui.selectable_value(&mut o.psm, p, tr(psm_label(p)));
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Engine"));
                    ui.radio_value(&mut o.engine, OcrEngineKind::Builtin, tr("Built-in"));
                    ui.radio_value(&mut o.engine, OcrEngineKind::Tesseract, tr("Tesseract"));
                });
                if o.engine == OcrEngineKind::Tesseract {
                    field(ui, tr("Tesseract"), &mut o.tesseract_path);
                }
                let ov = &mut self.cfg.overlay;
                ui.add(egui::Slider::new(&mut ov.box_opacity, 0.3..=1.0).fixed_decimals(2).text(tr("box opacity")));
                ui.horizontal(|ui| {
                    ui.label(tr("Japanese font"));
                    if ui.small_button(tr("Browse…")).clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("Fonts", &["ttf", "otf", "ttc"]).pick_file() {
                            ov.font_path = p.display().to_string();
                        }
                    }
                    if !ov.font_path.is_empty() && ui.small_button(tr("Auto")).clicked() {
                        ov.font_path.clear();
                    }
                });
                let label = if ov.font_path.is_empty() { tr("auto-detect").to_string() } else { ov.font_path.clone() };
                hint(ui, &label);
            });
        });
    }

    fn settings_asr(&mut self, ui: &mut egui::Ui) {
        let mut download_clicked = false;
        let mut link: Option<&'static str> = None;
        let can_download = self.download.is_none();
        let model_label = self.cfg.whisper_model().display().to_string();
        let whisper_choice = &mut self.whisper_choice;
        let Config { asr: a, api_keys: keys, .. } = &mut self.cfg;
        section(ui, tr("Speech (Whisper)"), false, |ui| {
            ui.checkbox(&mut a.enabled, tr("Transcribe and dub spoken English"));
            ui.add_enabled_ui(a.enabled, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add_enabled_ui(cfg!(feature = "whisper"), |ui| {
                        ui.radio_value(&mut a.engine, AsrEngine::Builtin, tr("Built-in"));
                    });
                    ui.radio_value(&mut a.engine, AsrEngine::Cli, tr("whisper-cli"));
                    ui.radio_value(&mut a.engine, AsrEngine::Api, tr("Online / server API"));
                });
                if a.engine == AsrEngine::Api {
                    let svc = ai::speech_service(&a.api_service);
                    egui::ComboBox::from_id_salt("stt_service").width(ui.available_width() - 4.0).selected_text(tr(svc.name)).show_ui(ui, |ui| {
                        for s in ai::SPEECH_SERVICES {
                            if ui.selectable_label(a.api_service == s.id, tr(s.name)).clicked() && a.api_service != s.id {
                                a.api_service = s.id.into();
                                a.api_url = s.url.into();
                                a.api_model = s.model.into();
                            }
                        }
                    });
                    field(ui, tr("Base URL"), &mut a.api_url);
                    let key_id = a.api_service.clone();
                    key_field(ui, keys, &key_id, tr("API key"));
                    field(ui, tr("Model"), &mut a.api_model);
                    hint(ui, tr("The video's sound is uploaded to this service to be transcribed. OpenAI and Groq need an API key."));
                    let label = if svc.local { tr("Get the server ↗") } else { tr("Get an API key ↗") };
                    if ui.small_button(label).on_hover_text(svc.link).clicked() {
                        link = Some(svc.link);
                    }
                    return;
                }
                if !cfg!(feature = "whisper") {
                    hint(ui, tr("Built-in Whisper needs a build with --features whisper (see README)."));
                }
                if a.engine == AsrEngine::Cli || !cfg!(feature = "whisper") {
                    field(ui, tr("whisper-cli"), &mut a.whisper_cli_path);
                }
                ui.horizontal(|ui| {
                    ui.label(tr("Model"));
                    if ui.small_button(tr("Browse…")).clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("Whisper ggml model", &["bin"]).pick_file() {
                            a.model_path = p.display().to_string();
                        }
                    }
                    if !a.model_path.is_empty() && ui.small_button(tr("Default")).clicked() {
                        a.model_path.clear();
                    }
                });
                hint(ui, &model_label);
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("wmodel")
                        .width(210.0)
                        .selected_text(tr(WHISPER_MODELS[*whisper_choice].1))
                        .show_ui(ui, |ui| {
                            for (i, (_, label)) in WHISPER_MODELS.iter().enumerate() {
                                ui.selectable_value(whisper_choice, i, tr(label));
                            }
                        });
                    if ui.add_enabled(can_download, egui::Button::new(tr("Download"))).clicked() {
                        download_clicked = true;
                    }
                });
                ui.checkbox(&mut a.use_gpu, tr("Use GPU when available"));
                ui.horizontal(|ui| {
                    ui.label(tr("CPU threads (0 = auto)"));
                    ui.add(egui::DragValue::new(&mut a.threads).range(0..=64));
                });
            });
        });
        if download_clicked {
            let ctx = ui.ctx().clone();
            self.start_task(Task::WhisperModel(self.whisper_choice), &ctx);
        }
        if let Some(url) = link {
            ai::open_url(url);
        }
    }

    fn settings_tts(&mut self, ui: &mut egui::Ui) {
        let mut link: Option<&'static str> = None;
        let voices = self.inventory.as_ref().map(|i| i.system_voices.clone()).unwrap_or_default();
        let Config { tts: t, api_keys: keys, .. } = &mut self.cfg;
        section(ui, tr("Japanese voice (TTS)"), false, |ui| {
            egui::ComboBox::from_id_salt("tts_backend").width(ui.available_width() - 4.0).selected_text(tr(t.backend.label())).show_ui(ui, |ui| {
                for b in TtsBackend::ALL {
                    ui.selectable_value(&mut t.backend, b, tr(b.label()));
                }
            });
            match t.backend {
                TtsBackend::Voicevox => {
                    ui.horizontal(|ui| {
                        ui.add_sized(vec2(84.0, 20.0), egui::Label::new(RichText::new(tr("Voice")).color(MUTED)));
                        let current = VOICEVOX_SPEAKERS.iter().find(|(id, _)| *id == t.voicevox_speaker).map(|(_, n)| *n).unwrap_or("custom");
                        egui::ComboBox::from_id_salt("vv_speaker").width(ui.available_width() - 70.0).selected_text(tr(current)).show_ui(ui, |ui| {
                            for (id, name) in VOICEVOX_SPEAKERS {
                                ui.selectable_value(&mut t.voicevox_speaker, *id, tr(name));
                            }
                        });
                        ui.add(egui::DragValue::new(&mut t.voicevox_speaker).range(0..=300)).on_hover_text(tr("Speaker ID"));
                    });
                    ui.checkbox(&mut t.voicevox_auto_start, tr("Start VOICEVOX by itself when needed"));
                    field(ui, tr("Engine URL"), &mut t.voicevox_url);
                    hint(ui, tr("Free, offline, high quality. BabelCraft downloads the VOICEVOX engine (1.9 GB) the first time. Credit the voice when you publish (e.g. \"VOICEVOX:ずんだもん\")."));
                    if ui.small_button(tr("VOICEVOX website and terms ↗")).clicked() {
                        link = Some("https://voicevox.hiroshiba.jp/");
                    }
                }
                TtsBackend::OpenAiCompatible => {
                    ui.horizontal_wrapped(|ui| {
                        for p in ai::VOICE_PRESETS {
                            if ui.small_button(tr(p.name)).on_hover_text(p.url).clicked() {
                                t.openai_url = p.url.into();
                                t.openai_model = p.model.into();
                                t.openai_voice = p.voice.into();
                            }
                        }
                    });
                    if let Some(p) = ai::VOICE_PRESETS.iter().find(|p| p.url == t.openai_url.trim()) {
                        let label = if p.url.contains("localhost") { tr("Get the server ↗") } else { tr("Get an API key ↗") };
                        if ui.small_button(label).on_hover_text(p.link).clicked() {
                            link = Some(p.link);
                        }
                    }
                    field(ui, tr("Base URL"), &mut t.openai_url);
                    if t.openai_url.contains("api.openai.com") {
                        key_field(ui, keys, "openai", tr("API key"));
                    } else {
                        secret(ui, tr("API key"), &mut t.openai_api_key);
                    }
                    field(ui, tr("Model"), &mut t.openai_model);
                    field(ui, tr("Voice"), &mut t.openai_voice);
                    hint(ui, tr("e.g. Kokoro-FastAPI locally (voice jf_alpha) or OpenAI (gpt-4o-mini-tts / tts-1, voices alloy, nova…)."));
                }
                TtsBackend::System => {
                    field(ui, tr("Voice name"), &mut t.system_voice);
                    if voices.is_empty() {
                        hint(ui, tr("Uses a Japanese voice built into Windows or macOS. Leave the name empty to use the first one found."));
                    } else {
                        hint(ui, &format!("{} {}", tr("Found:"), voices.join(", ")));
                    }
                }
                TtsBackend::EdgeTts => {
                    field(ui, tr("edge-tts"), &mut t.edge_tts_path);
                    field(ui, tr("Voice"), &mut t.edge_voice);
                    hint(ui, tr("Needs internet (`pip install edge-tts`). ja-JP-NanamiNeural / ja-JP-KeitaNeural."));
                }
                TtsBackend::Piper => {
                    field(ui, tr("piper"), &mut t.piper_path);
                    ui.horizontal(|ui| {
                        ui.label(tr("Voice model"));
                        if ui.small_button(tr("Browse…")).clicked() {
                            if let Some(p) = rfd::FileDialog::new().add_filter("Piper voice", &["onnx"]).pick_file() {
                                t.piper_model = p.display().to_string();
                            }
                        }
                    });
                    let label = if t.piper_model.is_empty() { tr("no model selected").to_string() } else { t.piper_model.clone() };
                    hint(ui, &label);
                }
            }
        });
        if let Some(url) = link {
            ai::open_url(url);
        }
    }

    fn settings_tools(&mut self, ui: &mut egui::Ui) {
        section(ui, tr("Tools & output"), false, |ui| {
            let t = &mut self.cfg.tools;
            field(ui, tr("ffmpeg"), &mut t.ffmpeg_path);
            field(ui, tr("ffprobe"), &mut t.ffprobe_path);
            ui.add(egui::Slider::new(&mut t.video_crf, 14..=32).text(tr("quality (CRF)")));
            ui.horizontal(|ui| {
                ui.label(tr("Encoder speed"));
                egui::ComboBox::from_id_salt("preset").selected_text(t.video_preset.clone()).show_ui(ui, |ui| {
                    for p in ["ultrafast", "veryfast", "faster", "fast", "medium", "slow"] {
                        ui.selectable_value(&mut t.video_preset, p.to_string(), p);
                    }
                });
            });
            ui.checkbox(&mut t.keep_temp_files, tr("Keep temporary files (for debugging)"));
            hint(ui, &format!("{} {}", tr("Settings file:"), config::config_file().display()));
        });
    }
}

// ---------------------------------------------------------------- main cards
impl BabelApp {
    fn files_card(&mut self, ui: &mut egui::Ui) {
        let running = self.running();
        let mut picked_input: Option<PathBuf> = None;
        card(ui, |ui| {
            ui.label(RichText::new(tr("1  ·  Choose your video")).size(17.0).strong().color(CREAM));
            ui.add_space(6.0);
            ui.add_enabled_ui(!running, |ui| {
                // The label column fits the longer label in the chosen language; the text boxes
                // take what is left beside the Browse buttons.
                let body = egui::TextStyle::Body.resolve(ui.style());
                let label_w = [tr("English video"), tr("Save Japanese video as")]
                    .iter()
                    .map(|t| ui.painter().layout_no_wrap(t.to_string(), body.clone(), MUTED).size().x)
                    .fold(0.0f32, f32::max);
                let label_w = (label_w + 16.0).clamp(150.0, 280.0);
                let browse_w = [tr("Browse…")]
                    .iter()
                    .map(|t| ui.painter().layout_no_wrap(t.to_string(), body.clone(), CREAM).size().x + 24.0)
                    .fold(90.0f32, f32::max);
                let w = (ui.available_width() - label_w - browse_w - 2.0 * ui.spacing().item_spacing.x - 4.0).max(160.0);
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(label_w, 28.0), egui::Label::new(RichText::new(tr("English video")).color(MUTED)));
                    ui.add_sized(
                        vec2(w, 28.0),
                        egui::TextEdit::singleline(&mut self.cfg.last_input)
                            .hint_text(tr("MP4, MKV, MOV or AVI — or drop a file on the window"))
                            .align(Align2::LEFT_CENTER),
                    );
                    if ui.add_sized(vec2(browse_w, 28.0), egui::Button::new(tr("Browse…"))).clicked() {
                        picked_input = rfd::FileDialog::new()
                            .set_title(tr("Choose an English video"))
                            .add_filter("Video", &pipeline::INPUT_EXTENSIONS)
                            .pick_file();
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(label_w, 28.0), egui::Label::new(RichText::new(tr("Save Japanese video as")).color(MUTED)));
                    ui.add_sized(
                        vec2(w, 28.0),
                        egui::TextEdit::singleline(&mut self.cfg.last_output)
                            .hint_text(tr("where the translated video is written (.mp4 / .mkv / .mov)"))
                            .align(Align2::LEFT_CENTER),
                    );
                    if ui.add_sized(vec2(browse_w, 28.0), egui::Button::new(tr("Browse…"))).clicked() {
                        let mut d = rfd::FileDialog::new()
                            .set_title(tr("Save the Japanese video"))
                            .add_filter("MP4 video", &["mp4"])
                            .add_filter("Matroska video", &["mkv"])
                            .add_filter("QuickTime video", &["mov"]);
                        let cur = PathBuf::from(&self.cfg.last_output);
                        if let Some(dir) = cur.parent().filter(|d| d.is_dir()) {
                            d = d.set_directory(dir);
                        }
                        if let Some(name) = cur.file_name() {
                            d = d.set_file_name(name.to_string_lossy());
                        }
                        if let Some(mut p) = d.save_file() {
                            if p.extension().is_none() {
                                p.set_extension("mp4");
                            }
                            self.cfg.last_output = p.display().to_string();
                        }
                    }
                });
                ui.add_space(8.0);
                // The video's language and the language to translate into (any two, both ways).
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(label_w, 28.0), egui::Label::new(RichText::new(tr("Video language → translate into")).color(MUTED)));
                    changed |= language_combo(ui, "src_lang", &mut self.cfg.translation.source_language);
                    if ui.add_sized(vec2(34.0, 28.0), egui::Button::new("⇄")).on_hover_text(tr("Swap the two languages")).clicked() {
                        std::mem::swap(&mut self.cfg.translation.source_language, &mut self.cfg.translation.target_language);
                        changed = true;
                    }
                    changed |= language_combo(ui, "dst_lang", &mut self.cfg.translation.target_language);
                });
                if self.cfg.translation.source_language == self.cfg.translation.target_language {
                    hint(ui, tr("Choose two different languages."));
                }
                if changed {
                    self.refresh_output_name();
                }
            });
        });
        if let Some(p) = picked_input {
            self.set_input(p);
        }
    }

    fn set_input(&mut self, p: PathBuf) {
        self.cfg.last_input = p.display().to_string();
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "video".into());
        let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let out_ext = match ext.as_str() {
            "mkv" => "mkv",
            "mov" => "mov",
            _ => "mp4",
        };
        self.cfg.last_output = p.with_file_name(format!("{stem}.{}.{out_ext}", self.cfg.target_lang().code)).display().to_string();
    }

    /// Renames the output after a language change (`name.ja.mp4` → `name.es.mp4`).
    fn refresh_output_name(&mut self) {
        let out = PathBuf::from(self.cfg.last_output.trim());
        let (Some(stem), Some(ext)) = (out.file_stem().map(|s| s.to_string_lossy().to_string()), out.extension().map(|e| e.to_string_lossy().to_string())) else { return };
        let code = self.cfg.target_lang().code;
        let base = langs::LANGUAGES.iter().find_map(|l| stem.strip_suffix(&format!(".{}", l.code)).map(|b| b.to_string()));
        if let Some(base) = base {
            self.cfg.last_output = out.with_file_name(format!("{base}.{code}.{ext}")).display().to_string();
        }
    }

    fn progress_card(&mut self, ui: &mut egui::Ui) {
        let mut start_clicked = false;
        let ready = !self.cfg.last_input.trim().is_empty() && !self.cfg.last_output.trim().is_empty();
        let (plan, done, current, stage_frac, overall, elapsed, stage_elapsed) = match &self.run {
            Some(r) => (
                r.plan.clone(),
                r.done.clone(),
                r.current,
                r.stage_frac,
                overall_of(r),
                Some(r.started.elapsed().as_secs_f64()),
                Some(r.stage_started.elapsed().as_secs_f64()),
            ),
            None => (
                if self.last_plan.is_empty() { Stage::ALL.to_vec() } else { self.last_plan.clone() },
                self.last_done.clone(),
                None,
                if matches!(self.outcome, Outcome::Finished { .. }) { 1.0 } else { 0.0 },
                self.last_overall,
                None,
                None,
            ),
        };
        let running = self.running();
        // First-use downloads, and how long the real work has been going (for the time left).
        let setup = self.run.as_ref().and_then(|r| r.setup.clone());
        let work_elapsed = self.run.as_ref().map(|r| r.work_started.elapsed().as_secs_f64());
        let scroll_to_bars = std::mem::take(&mut self.reveal_progress);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(trf("2  ·  Translate {0} → {1}", &[self.cfg.source_lang().shown(), self.cfg.target_lang().shown()])).size(17.0).strong().color(CREAM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(run) = self.run.as_mut() {
                        let label = if run.cancelling { tr("Stopping…") } else { tr("■  Cancel") };
                        let b = egui::Button::new(RichText::new(label).size(16.0).strong().color(Color32::WHITE))
                            .fill(RED)
                            .min_size(vec2(130.0, 40.0))
                            .corner_radius(8);
                        if ui.add_enabled(!run.cancelling, b).clicked() {
                            run.cancel.cancel();
                            run.cancelling = true;
                        }
                    } else {
                        let b = egui::Button::new(RichText::new(tr("▶  Start translating")).size(16.0).strong().color(INK))
                            .fill(if ready { YELLOW } else { YELLOW_DIM })
                            .min_size(vec2(210.0, 40.0))
                            .corner_radius(8);
                        if ui.add(b).on_hover_text(tr("Runs in the background, completely silently.")).clicked() {
                            start_clicked = true;
                        }
                    }
                });
            });
            ui.add_space(10.0);

            // Stage chips: one row when there is room, otherwise rows of three (or two).
            let gap = 8.0;
            let avail = ui.available_width();
            let cols = if avail >= 6.0 * 128.0 { 6 } else if avail >= 3.0 * 118.0 { 3 } else { 2 };
            let chip_w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
            for (r, row) in Stage::ALL.chunks(cols).enumerate() {
                if r > 0 {
                    ui.add_space(gap - ui.spacing().item_spacing.y);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (j, s) in row.iter().enumerate() {
                        let state = if !plan.contains(s) {
                            ChipState::Skipped
                        } else if done.contains(s) {
                            ChipState::Done
                        } else if current == Some(*s) {
                            ChipState::Active
                        } else {
                            ChipState::Pending
                        };
                        stage_chip(ui, r * cols + j + 1, *s, state, chip_w, if current == Some(*s) { stage_frac } else { 0.0 });
                    }
                });
            }
            ui.add_space(14.0);

            let headline = match (&setup, &current, &self.outcome) {
                (Some(s), _, _) => crate::i18n::setup_headline(s.index, s.count),
                (None, Some(s), _) => format!("{}…", tr(s.label())),
                (None, None, Outcome::Finished { .. }) => tr("Finished — your Japanese video is ready").into(),
                (None, None, Outcome::Failed(_)) => tr("Stopped with an error").into(),
                (None, None, Outcome::Cancelled) => tr("Cancelled").into(),
                (None, None, Outcome::None) if running => tr("Starting…").into(),
                (None, None, Outcome::None) => tr("Ready when you are").into(),
            };
            ui.label(RichText::new(headline).size(20.0).strong().color(CREAM));
            if let Some(s) = &setup {
                // What is downloading, how much is done and how fast — so a long first download never looks stuck.
                let mut line = crate::i18n::tr_step(&s.what);
                if let Some(d) = &s.download {
                    line.push_str("  —  ");
                    line.push_str(&crate::i18n::download_numbers(d));
                }
                ui.add_space(2.0);
                ui.label(RichText::new(line).size(15.0).color(YELLOW));
                ui.label(
                    RichText::new(tr("Only needed once. If you cancel, what has downloaded is kept and carries on next time."))
                        .size(12.5)
                        .color(MUTED),
                );
            }
            ui.add_space(6.0);

            if let Some(s) = &setup {
                let f = s.frac.unwrap_or(0.0);
                let text = match s.frac {
                    Some(f) => format!("{}  {:.0}%", tr("Download"), pct_down(f)),
                    None => tr("Please wait…").into(),
                };
                big_bar(ui, f, 30.0, YELLOW, &text);
                ui.add_space(6.0);
                // How far through the list of downloads.
                let steps = ((s.index.saturating_sub(1)) as f32 + f) / s.count.max(1) as f32;
                big_bar(ui, steps, 14.0, Color32::from_rgb(214, 196, 140), "");
            } else {
                big_bar(ui, overall, 30.0, YELLOW, &format!("{}  {:.0}%", tr("Overall"), overall * 100.0));
                ui.add_space(6.0);
                big_bar(ui, stage_frac, 14.0, Color32::from_rgb(214, 196, 140), "");
            }
            ui.add_space(8.0);

            let eta = match (&setup, work_elapsed) {
                // During the downloads: their own time left.
                (Some(s), _) => s.download.and_then(|d| d.eta_secs()).map(fmt_duration).unwrap_or_else(|| tr("estimating…").into()),
                // Afterwards: from the work done since, so download time doesn't stretch the estimate.
                (None, Some(w)) if overall > 0.015 => fmt_duration(w * (1.0 - overall as f64) / overall as f64),
                (None, Some(_)) => tr("estimating…").into(),
                (None, None) => "—".into(),
            };
            let elapsed_txt = elapsed.map(fmt_duration).unwrap_or_else(|| match &self.outcome {
                Outcome::Finished { secs, .. } => fmt_duration(*secs),
                _ => "—".into(),
            });
            ui.horizontal(|ui| {
                stat(ui, tr("Overall"), &format!("{:.0}%", overall * 100.0));
                match &setup {
                    Some(s) => stat(ui, tr("Download"), &s.frac.map(|f| format!("{:.0}%", pct_down(f))).unwrap_or_else(|| "—".into())),
                    None => stat(ui, tr("Stage"), &format!("{:.0}%", stage_frac * 100.0)),
                }
                stat(ui, tr("Elapsed"), &elapsed_txt);
                stat(ui, if setup.is_some() { tr("Time left (download)") } else { tr("Time left (ETA)") }, &eta);
                if let (Some(se), None) = (stage_elapsed, &setup) {
                    stat(ui, tr("This stage"), &fmt_duration(se));
                }
            });
            if scroll_to_bars {
                // Just started (or a download began): make sure the bars and time left are on screen.
                ui.scroll_to_cursor(Some(Align::BOTTOM));
            }

            match &self.outcome {
                Outcome::Finished { path, .. } => {
                    ui.add_space(8.0);
                    banner(ui, GREEN, |ui| {
                        ui.label(RichText::new(format!("{} {}", tr("Saved"), path.display())).color(CREAM));
                        if ui.small_button(tr("Show in folder")).clicked() {
                            reveal(path);
                        }
                    });
                }
                Outcome::Failed(e) => {
                    ui.add_space(8.0);
                    banner(ui, RED, |ui| {
                        ui.label(RichText::new(format!("× {}", e.lines().next().unwrap_or(""))).color(CREAM));
                    });
                }
                Outcome::Cancelled => {
                    ui.add_space(8.0);
                    banner(ui, ORANGE, |ui| {
                        ui.label(RichText::new(tr("Cancelled — all helper processes were stopped.")).color(CREAM));
                    });
                }
                Outcome::None => {}
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr("Silent background mode — nothing is played or previewed. Minimise BabelCraft and keep working."))
                    .size(12.0)
                    .color(MUTED),
            );
        });
        if start_clicked {
            let ctx = ui.ctx().clone();
            self.start_job(&ctx);
        }
    }

    fn checks_card(&mut self, ui: &mut egui::Ui) {
        let mut fix_clicked = None;
        let mut recheck = false;
        let checking = self.checks_rx.is_some();
        let can_download = self.download.is_none();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Setup checks")).size(17.0).strong().color(CREAM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(!checking, egui::Button::new(tr("Re-check"))).clicked() {
                        recheck = true;
                    }
                    let needs_setup = self.checks.iter().any(|c| c.fix == Some(Fix::AutoInstall) && c.status != Status::Ok);
                    if needs_setup {
                        let b = egui::Button::new(RichText::new(tr("⬇  Set up automatically")).strong().color(INK)).fill(YELLOW).corner_radius(6);
                        if ui.add_enabled(can_download, b).on_hover_text(tr("Downloads FFmpeg, the offline AI brain, the VOICEVOX voice, models and fonts into BabelCraft's own folder — nothing to install by hand.")).clicked() {
                            fix_clicked = Some(Fix::AutoInstall);
                        }
                    }
                    if checking {
                        ui.spinner();
                        ui.label(RichText::new(tr("checking…")).color(MUTED));
                    }
                });
            });
            ui.add_space(4.0);
            if self.checks.is_empty() {
                ui.label(
                    RichText::new(tr("Looking for FFmpeg, Tesseract, Whisper, a Japanese font, the translator and the voice engine…")).color(MUTED),
                );
            }
            for c in &self.checks {
                ui.horizontal(|ui| {
                    let col = match c.status {
                        Status::Ok => GREEN,
                        Status::Warn => ORANGE,
                        Status::Missing => RED,
                    };
                    ui.label(RichText::new(tr("●")).color(col));
                    ui.add_sized(vec2(120.0, 18.0), egui::Label::new(RichText::new(tr(c.name)).strong().color(CREAM)));
                    ui.label(RichText::new(&c.detail).size(12.5).color(if c.status == Status::Ok { MUTED } else { col }));
                    if let Some(fix) = c.fix {
                        let label = if fix == Fix::UseBrain { tr("Use the offline AI brain") } else { tr("Download") };
                        if c.status != Status::Ok && ui.add_enabled(can_download, egui::Button::new(label)).clicked() {
                            fix_clicked = Some(fix);
                        }
                    }
                });
                if c.status != Status::Ok && !c.hint.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(26.0);
                        ui.label(RichText::new(tr(c.hint)).size(12.0).color(MUTED));
                    });
                }
            }
        });
        self.download_bar_in_card(ui);
        if recheck {
            let ctx = ui.ctx().clone();
            self.start_checks(&ctx);
        }
        if let Some(f) = fix_clicked {
            let ctx = ui.ctx().clone();
            self.apply_fix(f, &ctx);
        }
    }

    fn download_bar_in_card(&mut self, ui: &mut egui::Ui) {
        if self.download.is_some() {
            ui.add_space(8.0);
            card(ui, |ui| self.download_bar(ui));
        }
    }
}

// ------------------------------------------------------ AI models & downloads page
impl BabelApp {
    fn page_tabs(&mut self, ui: &mut egui::Ui) {
        let mut go = None;
        ui.horizontal(|ui| {
            for (page, label) in [(Page::Translate, tr("▶  Translate a video")), (Page::Downloads, tr("⬇  AI models & downloads"))] {
                let on = self.page == page;
                let b = egui::Button::new(RichText::new(label).size(15.0).strong().color(if on { INK } else { CREAM }))
                    .fill(if on { YELLOW } else { CARD })
                    .stroke(Stroke::new(1.0, if on { YELLOW } else { CARD_STROKE }))
                    .corner_radius(8)
                    .min_size(vec2(0.0, 34.0));
                if ui.add(b).clicked() {
                    go = Some(page);
                }
            }
        });
        if let Some(p) = go {
            if p == Page::Downloads && self.page != Page::Downloads {
                let ctx = ui.ctx().clone();
                self.inventory_rx = None;
                self.refresh_inventory(&ctx);
            }
            self.page = p;
        }
        ui.add_space(10.0);
    }

    fn downloads_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.inventory.is_none() && self.inventory_rx.is_none() {
            self.refresh_inventory(&ctx);
        }
        let inv = self.inventory.clone().unwrap_or_default();
        let have = self.inventory.is_some();
        let loading = self.inventory_rx.is_some();
        let can = self.download.is_none() && self.run.is_none();
        let brain_in_use = self.cfg.translation.backend == TranslationBackend::BuiltinBrain;
        let ollama_in_use = self.cfg.translation.backend == TranslationBackend::Ollama;
        let current_brain = self.cfg.translation.brain_model.clone();
        let mut wchoice = self.whisper_choice;
        let mut task: Option<Task> = None;
        let mut link: Option<&'static str> = None;
        let mut use_brain: Option<&'static str> = None;
        let mut use_ollama = false;
        let mut remove_model: Option<&'static str> = None;
        let mut remove_voicevox = false;
        let mut refresh = false;
        let state = |ok: bool| {
            if !have {
                (ItemState::Info, tr("checking…"))
            } else if ok {
                (ItemState::Ready, tr("ready"))
            } else {
                (ItemState::Missing, tr("not downloaded yet"))
            }
        };

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("AI models & downloads")).size(18.0).strong().color(CREAM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(!loading, egui::Button::new(tr("Refresh"))).clicked() {
                        refresh = true;
                    }
                    if loading {
                        ui.spinner();
                    }
                });
            });
            ui.label(
                RichText::new(tr("Everything BabelCraft uses, where it comes from, and what is already on this computer. Downloads go into BabelCraft's own folder — nothing is installed into your system."))
                    .size(12.5)
                    .color(MUTED),
            );
            hint(ui, &format!("{} {}", tr("Folder:"), config::data_dir().display()));
        });
        self.download_bar_in_card(ui);
        ui.add_space(10.0);

        // ---- needed for every video
        card(ui, |ui| {
            heading(ui, tr("Needed for every video (downloaded automatically)"));
            let (s, l) = state(inv.ffmpeg);
            item_row(ui, s, l, "FFmpeg", "~100 MB", tr("Reads the video and writes the finished Japanese version."), |ui| {
                if link_button(ui, tr("Website ↗")) {
                    link = Some("https://ffmpeg.org/download.html");
                }
                if have && !inv.ffmpeg && action_button(ui, tr("Download"), true, can) {
                    task = Some(Task::Component(Component::Ffmpeg));
                }
            });
            let (s, l) = state(inv.ocr);
            item_row(ui, s, l, tr("Text-reading models (OCR)"), "12 MB", tr("Find and read the English text shown in the picture."), |ui| {
                if link_button(ui, tr("Website ↗")) {
                    link = Some("https://github.com/robertknight/ocrs");
                }
                if have && !inv.ocr && action_button(ui, tr("Download"), true, can) {
                    task = Some(Task::Component(Component::OcrModels));
                }
            });
            let (s, l) = state(inv.font.is_some());
            item_row(ui, s, l, tr("Japanese font (Noto Sans JP)"), "5 MB", tr("Draws the Japanese text over the video, so it never shows as □□□."), |ui| {
                if link_button(ui, tr("Website ↗")) {
                    link = Some("https://fonts.google.com/noto/specimen/Noto+Sans+JP");
                }
                if have && inv.font.is_none() && action_button(ui, tr("Download"), true, can) {
                    task = Some(Task::Component(Component::Font));
                }
            });
            let installable = crate::tools::whisper_cli_installable();
            let (s, l) = if have && !inv.whisper_cli && !installable { (ItemState::Unavailable, tr("use Online API or the Whisper-built-in download")) } else { state(inv.whisper_cli) };
            item_row(ui, s, l, tr("Whisper speech tool (whisper.cpp)"), "~10 MB", tr("Turns the English speech into text with timings."), |ui| {
                if link_button(ui, tr("Website ↗")) {
                    link = Some("https://github.com/ggml-org/whisper.cpp");
                }
                if have && !inv.whisper_cli && installable && action_button(ui, tr("Download"), true, can) {
                    task = Some(Task::Component(Component::WhisperCli));
                }
            });
            let ready = inv.whisper_models.get(wchoice).copied().unwrap_or(false);
            let (s, l) = state(ready);
            item_row(ui, s, l, tr("Whisper speech model"), "", tr("Bigger models hear more accurately but take longer."), |ui| {
                if link_button(ui, tr("Website ↗")) {
                    link = Some("https://huggingface.co/ggerganov/whisper.cpp");
                }
                if have && !ready && action_button(ui, tr("Download"), true, can) {
                    task = Some(Task::WhisperModel(wchoice));
                }
                egui::ComboBox::from_id_salt("dl_wmodel").width(200.0).selected_text(tr(WHISPER_MODELS[wchoice].1)).show_ui(ui, |ui| {
                    for (i, (_, label)) in WHISPER_MODELS.iter().enumerate() {
                        let mark = if inv.whisper_models.get(i).copied().unwrap_or(false) { format!("  · {}", tr("downloaded")) } else { String::new() };
                        ui.selectable_value(&mut wchoice, i, format!("{}{mark}", tr(label)));
                    }
                });
            });
        });
        ui.add_space(10.0);

        // ---- offline AI brain
        card(ui, |ui| {
            heading(ui, tr("Offline AI brain — translation on your own computer"));
            ui.label(
                RichText::new(tr("No account, no API key, and nothing leaves your computer. BabelCraft downloads the llama.cpp engine and the model you pick, runs it hidden only while translating, then closes it."))
                    .size(12.5)
                    .color(MUTED),
            );
            ui.add_space(6.0);
            let (s, l) = if !brain::engine_installable() { (ItemState::Unavailable, tr("not available for this computer")) } else { state(inv.brain_engine) };
            item_row(ui, s, l, tr("AI brain engine (llama.cpp)"), "~25 MB", tr("The program that runs the AI model."), |ui| {
                if link_button(ui, tr("Website ↗")) {
                    link = Some("https://github.com/ggml-org/llama.cpp");
                }
                if have && !inv.brain_engine && brain::engine_installable() && action_button(ui, tr("Download"), true, can) {
                    task = Some(Task::Component(Component::BrainEngine));
                }
            });
            for (i, m) in brain::MODELS.iter().enumerate() {
                let ready = inv.brain_models.get(i).copied().unwrap_or(false);
                let in_use = brain_in_use && current_brain == m.id;
                let (s, l) = match (have, ready, in_use) {
                    (false, _, _) => (ItemState::Info, tr("checking…")),
                    (true, true, true) => (ItemState::Ready, tr("in use")),
                    (true, true, false) => (ItemState::Ready, tr("downloaded")),
                    (true, false, _) => (ItemState::Missing, tr("not downloaded yet")),
                };
                let what = format!("{}  ·  {} {}", tr(m.note), tr("Licence:"), m.licence);
                item_row(ui, s, l, tr(m.name), m.size, &what, |ui| {
                    if link_button(ui, tr("Model page ↗")) {
                        link = Some(m.page);
                    }
                    if ready {
                        if action_button(ui, tr("Remove"), false, can && !in_use) {
                            remove_model = Some(m.id);
                        }
                        if !in_use && action_button(ui, tr("Use for translation"), true, true) {
                            use_brain = Some(m.id);
                        }
                    } else if have && action_button(ui, tr("Download"), true, can) {
                        task = Some(Task::BrainModel(m.id));
                    }
                });
            }
            ui.add_space(4.0);
            heading(ui, tr("Or use the free Ollama app"));
            match &inv.ollama {
                Some(models) => {
                    let what = if models.is_empty() { tr("No models yet — download one below.").to_string() } else { models.join(", ") };
                    let status = format!("{} · {} {}", tr("running"), models.len(), tr("models"));
                    item_row(ui, ItemState::Ready, &status, "Ollama", "", &what, |ui| {
                        if !ollama_in_use && action_button(ui, tr("Use Ollama for translation"), true, true) {
                            use_ollama = true;
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(tr("Download a model into Ollama:")).color(MUTED));
                        ui.add(egui::TextEdit::singleline(&mut self.ollama_pull_name).desired_width(160.0));
                        if action_button(ui, tr("Download"), true, can && !self.ollama_pull_name.trim().is_empty()) {
                            task = Some(Task::OllamaPull(self.ollama_pull_name.trim().to_string()));
                        }
                        if link_button(ui, tr("Model library ↗")) {
                            link = Some("https://ollama.com/library");
                        }
                    });
                }
                None => {
                    let (s, l) = if have { (ItemState::Info, tr("not running")) } else { (ItemState::Info, tr("checking…")) };
                    item_row(ui, s, l, "Ollama", "", tr("Free app that runs many open AI models. Install it, and BabelCraft can use it and download models into it."), |ui| {
                        if link_button(ui, tr("Get Ollama ↗")) {
                            link = Some("https://ollama.com/download");
                        }
                    });
                }
            }
        });
        ui.add_space(10.0);

        // ---- Japanese voice
        card(ui, |ui| {
            heading(ui, tr("Japanese voice"));
            let vv_installable = crate::voicevox::installable();
            let (s, l): (ItemState, String) = if !have {
                (ItemState::Info, tr("checking…").into())
            } else if let Some(v) = &inv.voicevox_running {
                (ItemState::Ready, format!("{} {v}", tr("running ·")))
            } else if inv.voicevox_bundled {
                (ItemState::Ready, tr("downloaded · starts by itself").into())
            } else if inv.voicevox_app {
                (ItemState::Ready, tr("VOICEVOX app found · starts by itself").into())
            } else if vv_installable {
                (ItemState::Missing, tr("not downloaded yet").into())
            } else {
                (ItemState::Unavailable, tr("not available for this computer").into())
            };
            item_row(ui, s, &l, tr("VOICEVOX voice engine"), crate::voicevox::DOWNLOAD_SIZE, tr("Free, offline, natural Japanese voices. Credit the voice when you publish, e.g. \"VOICEVOX:ずんだもん\"."), |ui| {
                if link_button(ui, tr("Terms ↗")) {
                    link = Some("https://voicevox.hiroshiba.jp/term/");
                }
                if link_button(ui, tr("Website ↗")) {
                    link = Some("https://voicevox.hiroshiba.jp/");
                }
                if inv.voicevox_bundled {
                    if action_button(ui, tr("Remove"), false, can) {
                        remove_voicevox = true;
                    }
                } else if have && vv_installable && !inv.voicevox_app && action_button(ui, tr("Download"), true, can) {
                    task = Some(Task::Component(Component::VoiceEngine));
                }
            });
            let (s, l): (ItemState, String) = if !have {
                (ItemState::Info, tr("checking…").into())
            } else if let Some(v) = inv.system_voices.first() {
                (ItemState::Ready, v.clone())
            } else if cfg!(any(windows, target_os = "macos")) {
                (ItemState::Info, tr("no Japanese voice installed").into())
            } else {
                (ItemState::Unavailable, tr("not available on Linux").into())
            };
            item_row(ui, s, &l, tr("Computer's own Japanese voice"), "", tr("Windows and macOS can add a Japanese voice for free in their speech settings."), |ui| {
                #[cfg(windows)]
                if link_button(ui, tr("Speech settings ↗")) {
                    link = Some("ms-settings:speech");
                }
                #[cfg(target_os = "macos")]
                if link_button(ui, tr("Speech settings ↗")) {
                    link = Some("x-apple.systempreferences:com.apple.preference.universalaccess");
                }
                let _ = ui;
            });
        });
        ui.add_space(10.0);

        // ---- other free apps
        card(ui, |ui| {
            heading(ui, tr("Other free apps BabelCraft can work with"));
            for a in ai::OPTIONAL_APPS {
                item_row(ui, ItemState::Info, "", a.name, "", tr(a.what), |ui| {
                    if link_button(ui, tr("Website ↗")) {
                        link = Some(a.url);
                    }
                });
            }
        });
        ui.add_space(10.0);

        // ---- online services
        let keys = &mut self.cfg.api_keys;
        card(ui, |ui| {
            heading(ui, tr("Online AI services — paste your API key"));
            ui.label(
                RichText::new(tr("Keys stay in BabelCraft's settings file on this computer. Online services charge for what you use; pick one in Settings → Translation or Speech."))
                    .size(12.5)
                    .color(MUTED),
            );
            ui.add_space(6.0);
            for p in PROVIDERS.iter().filter(|p| !p.local && p.id != "custom") {
                let saved = keys.get(p.id).map(|k| !k.trim().is_empty()).unwrap_or(false);
                let from_env = p.env.iter().any(|v| std::env::var(v).map(|s| !s.trim().is_empty()).unwrap_or(false));
                let (s, l) = if saved {
                    (ItemState::Ready, tr("key saved"))
                } else if from_env {
                    (ItemState::Ready, tr("key from environment"))
                } else {
                    (ItemState::Info, "")
                };
                item_row(ui, s, l, tr(p.name), "", tr(p.uses), |ui| {
                    if link_button(ui, tr("Get a key ↗")) {
                        link = Some(p.link);
                    }
                    let mut k = keys.get(p.id).cloned().unwrap_or_default();
                    let r = ui.add(egui::TextEdit::singleline(&mut k).password(true).hint_text(tr("paste API key")).desired_width(200.0));
                    if r.changed() {
                        if k.trim().is_empty() {
                            keys.remove(p.id);
                        } else {
                            keys.insert(p.id.to_string(), k);
                        }
                    }
                });
            }
        });

        self.whisper_choice = wchoice;
        if refresh {
            self.inventory_rx = None;
            self.refresh_inventory(&ctx);
            self.start_checks(&ctx);
        }
        if let Some(url) = link {
            ai::open_url(url);
        }
        if let Some(t) = task {
            self.start_task(t, &ctx);
        }
        if let Some(id) = use_brain {
            self.apply_provider("brain");
            self.cfg.translation.brain_model = id.to_string();
            self.log(LogLevel::Ok, format!("Translation now uses the offline AI brain ({}).", brain::model(id).name));
            self.start_checks(&ctx);
        }
        if use_ollama {
            self.apply_provider("ollama");
            self.log(LogLevel::Ok, "Translation now uses Ollama.".into());
            self.start_checks(&ctx);
        }
        if let Some(id) = remove_model {
            match brain::remove_model(id) {
                Ok(()) => self.log(LogLevel::Info, format!("Removed {}.", brain::model(id).name)),
                Err(e) => self.log(LogLevel::Error, format!("{e:#}")),
            }
            self.inventory_rx = None;
            self.refresh_inventory(&ctx);
        }
        if remove_voicevox {
            match crate::voicevox::remove() {
                Ok(()) => self.log(LogLevel::Info, "Removed the VOICEVOX engine.".into()),
                Err(e) => self.log(LogLevel::Error, format!("{e:#}")),
            }
            self.inventory_rx = None;
            self.refresh_inventory(&ctx);
            self.start_checks(&ctx);
        }
    }
}

// ------------------------------------------------------ splash and screenshot
impl BabelApp {
    fn splash(&mut self, ui: &mut egui::Ui, t: f32) {
        let fade = if t > SPLASH_SECS - 0.4 { ((SPLASH_SECS - t) / 0.4).clamp(0.0, 1.0) } else { (t / 0.25).clamp(0.0, 1.0) };
        let a = (fade * 255.0) as u8;
        let icon_id = self.icon.id();
        let sub = if self.has_cjk_ui_font { tr("English → 日本語 video translator") } else { tr("English → Japanese video translator") };
        let mut clicked = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ui, |ui| {
            let rect = ui.max_rect();
            let center = rect.center() - vec2(0.0, 50.0);
            // Soft yellow glow behind the icon.
            for i in 0..10 {
                let r = 150.0 + i as f32 * 16.0;
                let alpha = ((10 - i) as f32 * 1.6 * fade) as u8;
                ui.painter().circle_filled(center, r, Color32::from_rgba_unmultiplied(247, 194, 27, alpha));
            }
            let size = 260.0 * (0.94 + 0.06 * fade);
            let icon_rect = egui::Rect::from_center_size(center, vec2(size, size));
            egui::Image::new(egui::load::SizedTexture::new(icon_id, vec2(size, size)))
                .tint(Color32::from_white_alpha(a))
                .paint_at(ui, icon_rect);
            let painter = ui.painter();
            let below = icon_rect.center_bottom();
            painter.text(below + vec2(0.0, 44.0), Align2::CENTER_CENTER, "BabelCraft", FontId::proportional(44.0), YELLOW.gamma_multiply(fade));
            painter.text(below + vec2(0.0, 86.0), Align2::CENTER_CENTER, sub, FontId::proportional(18.0), CREAM.gamma_multiply(fade));
            let dots = ((t * 3.0) as usize % 3) + 1;
            painter.text(
                below + vec2(0.0, 118.0),
                Align2::CENTER_CENTER,
                format!("{}{}", tr("loading"), ".".repeat(dots)),
                FontId::proportional(14.0),
                MUTED.gamma_multiply(fade),
            );
            if ui.interact(rect, ui.scope_id().with("splash"), Sense::click()).clicked() {
                clicked = true;
            }
        });
        if clicked {
            self.splash_skipped = true;
        }
    }

    /// Test switches for reproducing problems in the real window (see the struct fields).
    fn test_switches(&mut self, ctx: &egui::Context) {
        if self.test_autostart && self.test_job_started.is_none() && self.started.elapsed() > Duration::from_millis(800) {
            self.test_job_started = Some(Instant::now());
            self.log(LogLevel::Info, "(test) starting the job automatically".into());
            self.start_job(ctx);
        }
        let Some(t0) = self.test_job_started else { return };
        if let Some(after) = self.test_minimize_after {
            if !self.test_minimized && t0.elapsed().as_secs_f32() >= after {
                self.test_minimized = true;
                self.log(LogLevel::Info, "(test) minimising the window".into());
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
        if self.test_exit_when_done && self.run.is_none() && !matches!(self.outcome, Outcome::None) {
            let summary = match &self.outcome {
                Outcome::Finished { path, secs } => format!("(test) RESULT: finished in {secs:.0} s: {}", path.display()),
                Outcome::Failed(e) => format!("(test) RESULT: failed: {e}"),
                Outcome::Cancelled => "(test) RESULT: cancelled".into(),
                Outcome::None => String::new(),
            };
            crate::logfile::append(&summary);
            self.test_exit_when_done = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(s) = self.screenshot.as_mut() else { return };
        ctx.request_repaint();
        if s.saved.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // Give the background checks a moment to finish so the picture shows their results.
        let busy = self.checks_rx.is_some() || self.inventory_rx.is_some();
        if self.frame >= SCREENSHOT_FRAME && (!busy || self.frame >= SCREENSHOT_FRAME + 360) && !s.requested {
            s.requested = true;
            let path = s.path.clone();
            let saved = s.saved.clone();
            let ctx2 = ctx.clone();
            ctx.request_screenshot(move |img| {
                let [w, h] = img.size;
                let mut buf = image::RgbaImage::new(w as u32, h as u32);
                for (i, p) in img.pixels.iter().enumerate() {
                    buf.put_pixel((i % w) as u32, (i / w) as u32, image::Rgba(p.to_srgba_unmultiplied()));
                }
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match buf.save(&path) {
                    Ok(()) => println!("screenshot saved to {}", path.display()),
                    Err(e) => eprintln!("could not save screenshot: {e}"),
                }
                saved.store(true, Ordering::SeqCst);
                ctx2.request_repaint();
            });
        }
        // Safety net: never hang in screenshot mode.
        if self.frame > SCREENSHOT_FRAME + 1200 {
            std::process::exit(1);
        }
    }
}

impl eframe::App for BabelApp {
    /// Runs before every frame AND while the window is minimised or hidden (when `ui` isn't
    /// called): the job's messages, the taskbar title, the log file and the end of the job are
    /// handled here, so minimising BabelCraft never freezes or loses track of a job.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.test_switches(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        // Interface language: fonts are set up again when it changes; without a font for its
        // letters, or before its translation exists, the interface stays in English.
        let code = self.cfg.ui_lang().to_string();
        // The language menu shows every language's own name, so it needs every writing system.
        if self.lang_menu_open && !self.menu_fonts {
            self.menu_fonts = true;
            self.ui_fonts_for.clear();
        }
        if self.ui_fonts_for != code {
            let fonts = install_fonts(&ctx, &self.cfg, self.menu_fonts);
            self.has_cjk_ui_font = fonts.japanese;
            self.ui_script_ok = fonts.script_ok;
            self.ui_fonts_for = code.clone();
        }
        if !(self.ui_script_ok && crate::i18n::set_language(&code)) {
            crate::i18n::set_language("en");
        }
        self.handle_screenshot(&ctx);

        // Drag & drop a video anywhere on the window.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if let Some(p) = dropped.into_iter().next() {
            if !self.running() {
                self.set_input(p);
            }
        }

        let t = self.started.elapsed().as_secs_f32();
        let splash_on = !self.splash_skipped
            && match &self.screenshot {
                Some(_) => self.frame < SCREENSHOT_SPLASH_FRAMES,
                None => t < SPLASH_SECS,
            };
        if splash_on {
            let st = match &self.screenshot {
                Some(_) => self.frame as f32 / SCREENSHOT_SPLASH_FRAMES as f32 * SPLASH_SECS,
                None => t,
            };
            self.splash(ui, st);
            ctx.request_repaint();
            return;
        }

        self.header(ui);
        self.log_panel(ui);
        self.settings_panel(ui);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(Margin::same(16))).show(ui, |ui| {
            self.page_tabs(ui);
            let page = self.page;
            let mut area = egui::ScrollArea::vertical()
                .id_salt(if page == Page::Translate { "page_translate" } else { "page_downloads" })
                .auto_shrink([false, false]);
            // Screenshot mode only: BABELCRAFT_SHOT_SCROLL=<pixels> photographs further down the page.
            if self.screenshot.is_some() {
                if let Some(px) = std::env::var("BABELCRAFT_SHOT_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
                    area = area.vertical_scroll_offset(px);
                }
            }
            area.show(ui, |ui| match page {
                    Page::Translate => {
                        self.files_card(ui);
                        ui.add_space(12.0);
                        self.progress_card(ui);
                        ui.add_space(12.0);
                        self.checks_card(ui);
                    }
                    Page::Downloads => self.downloads_page(ui),
                });
        });
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if self.screenshot.is_none() && self.cfg != self.saved_cfg {
            let _ = self.cfg.save();
            self.saved_cfg = self.cfg.clone();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Closing the window stops any running job, its child processes and helper servers.
        if let Some(run) = &self.run {
            run.cancel.cancel();
            std::thread::sleep(Duration::from_millis(400));
        }
        if let Some(dl) = &self.download {
            dl.cancel.cancel();
        }
        crate::services::stop_all();
        if self.screenshot.is_none() {
            let _ = self.cfg.save();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BG.to_normalized_gamma_f32()
    }

    /// Screenshot runs leave no trace (no scroll positions or open sections are saved).
    fn persist_egui_memory(&self) -> bool {
        self.screenshot.is_none()
    }
}

// --------------------------------------------------------------------- helpers

/// Whole percent, rounded down, so 100% only shows once a download is really complete.
fn pct_down(f: f32) -> f32 {
    (f * 100.0 + 0.001).floor().clamp(0.0, 100.0)
}

fn overall_of(run: &RunState) -> f32 {
    let total: f32 = run.plan.iter().map(|s| s.weight()).sum::<f32>().max(1.0);
    let done: f32 = run.plan.iter().filter(|s| run.done.contains(s)).map(|s| s.weight()).sum();
    let cur = run
        .current
        .filter(|c| !run.done.contains(c) && run.plan.contains(c))
        .map(|c| c.weight() * run.stage_frac)
        .unwrap_or(0.0);
    ((done + cur) / total).clamp(0.0, 1.0)
}

/// Wall-clock time (UTC) for log lines, without pulling in a date crate.
fn clock_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("[{:02}:{:02}:{:02}]", (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

#[derive(Clone, Copy, PartialEq)]
enum ChipState {
    Pending,
    Active,
    Done,
    Skipped,
}

fn stage_chip(ui: &mut egui::Ui, num: usize, s: Stage, state: ChipState, w: f32, frac: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, 58.0), Sense::hover());
    let p = ui.painter();
    let (fill, stroke, fg, badge) = match state {
        ChipState::Done => (Color32::from_rgb(36, 52, 32), GREEN, CREAM, String::new()),
        ChipState::Active => (Color32::from_rgb(66, 54, 18), YELLOW, CREAM, num.to_string()),
        ChipState::Pending => (CARD, CARD_STROKE, MUTED, num.to_string()),
        ChipState::Skipped => (BG, CARD_STROKE, Color32::from_rgb(90, 85, 70), "–".to_string()),
    };
    p.rect_filled(rect, 8, fill);
    p.rect_stroke(rect, 8, Stroke::new(if state == ChipState::Active { 2.0 } else { 1.0 }, stroke), egui::StrokeKind::Inside);
    if state == ChipState::Active {
        let mut bar = rect.shrink(3.0);
        bar.set_top(bar.bottom() - 4.0);
        bar.set_right(bar.left() + bar.width() * frac.clamp(0.0, 1.0));
        p.rect_filled(bar, 2, YELLOW);
    }
    let c = egui::pos2(rect.left() + 20.0, rect.center().y);
    let badge_fill = match state {
        ChipState::Active => YELLOW,
        ChipState::Done => GREEN.gamma_multiply(0.7),
        _ => Color32::from_rgb(70, 64, 46),
    };
    p.circle_filled(c, 11.0, badge_fill);
    if state == ChipState::Done {
        // A drawn check mark (no font needed).
        let stroke = Stroke::new(2.2, CREAM);
        p.line_segment([c + vec2(-5.0, 0.5), c + vec2(-1.5, 4.0)], stroke);
        p.line_segment([c + vec2(-1.5, 4.0), c + vec2(5.0, -4.0)], stroke);
    } else {
        p.text(c, Align2::CENTER_CENTER, badge, FontId::proportional(12.5), if state == ChipState::Active { INK } else { CREAM });
    }
    let text_rect = egui::Rect::from_min_max(egui::pos2(rect.left() + 38.0, rect.top() + 6.0), egui::pos2(rect.right() - 6.0, rect.bottom() - 6.0));
    let galley = p.layout(tr(s.label()).to_string(), FontId::proportional(12.5), fg, text_rect.width());
    let pos = egui::pos2(text_rect.left(), text_rect.center().y - galley.size().y / 2.0);
    p.galley(pos, galley, fg);
}

fn big_bar(ui: &mut egui::Ui, frac: f32, h: f32, color: Color32, text: &str) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter();
    let r = (h / 2.0).min(10.0) as u8;
    p.rect_filled(rect, r, INPUT_BG);
    p.rect_stroke(rect, r, Stroke::new(1.0, CARD_STROKE), egui::StrokeKind::Inside);
    if frac > 0.0 {
        let mut fill = rect;
        fill.set_right(rect.left() + (rect.width() * frac.clamp(0.0, 1.0)).max(h));
        p.rect_filled(fill, r, color);
    }
    if !text.is_empty() {
        // Dark text only when the whole label sits on the filled part; light text otherwise.
        let font = FontId::proportional(15.0);
        let text_w = p.layout_no_wrap(text.to_string(), font.clone(), CREAM).size().x;
        let filled_right = rect.left() + (rect.width() * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { h } else { 0.0 });
        let col = if filled_right >= rect.left() + 12.0 + text_w + 6.0 { INK } else { CREAM };
        p.text(egui::pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, text, font, col);
    }
}

fn stat(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::new().fill(INPUT_BG).corner_radius(6).inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(label).size(11.5).color(MUTED));
            ui.label(RichText::new(value).size(17.0).strong().color(CREAM));
        });
    });
}

fn banner(ui: &mut egui::Ui, color: Color32, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0, color))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(add);
        });
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_STROKE))
        .corner_radius(10)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_STROKE))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::CollapsingHeader::new(RichText::new(title).strong().color(CREAM)).default_open(open).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                add(ui);
            });
        });
    ui.add_space(6.0);
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(84.0, 20.0), egui::Label::new(RichText::new(label).color(MUTED)));
        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
    });
}

fn secret(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(84.0, 20.0), egui::Label::new(RichText::new(label).color(MUTED)));
        ui.add(egui::TextEdit::singleline(value).password(true).desired_width(f32::INFINITY));
    });
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(MUTED));
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(16.0).strong().color(CREAM));
    ui.add_space(4.0);
}

/// An API key box tied to the shared key list (empty removes the key).
fn key_field(ui: &mut egui::Ui, keys: &mut BTreeMap<String, String>, id: &str, label: &str) {
    let mut k = keys.get(id).cloned().unwrap_or_default();
    let before = k.clone();
    secret(ui, label, &mut k);
    if k != before {
        if k.trim().is_empty() {
            keys.remove(id);
        } else {
            keys.insert(id.to_string(), k);
        }
    }
}

/// Model name box with a "List" button that asks the service which models it has.
fn model_field(ui: &mut egui::Ui, value: &mut String, list: &[String], listing: bool, err: &str, list_clicked: &mut bool) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(84.0, 20.0), egui::Label::new(RichText::new(tr("Model")).color(MUTED)));
        ui.add(egui::TextEdit::singleline(value).desired_width((ui.available_width() - 64.0).max(80.0)));
        if ui.add_enabled(!listing, egui::Button::new(tr("List"))).on_hover_text(tr("Ask the service which models it has")).clicked() {
            *list_clicked = true;
        }
    });
    if listing {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(tr("asking the service…")).size(12.0).color(MUTED));
        });
    }
    if !list.is_empty() {
        egui::ComboBox::from_id_salt("model_pick")
            .width(ui.available_width() - 4.0)
            .height(400.0)
            .selected_text(format!("{} ({})", tr("Pick a model"), list.len()))
            .show_ui(ui, |ui| {
                for m in list {
                    if ui.selectable_label(value == m, m).clicked() {
                        *value = m.clone();
                    }
                }
            });
    }
    if !err.is_empty() {
        ui.label(RichText::new(format!("× {err}")).size(11.5).color(RED));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ItemState {
    Ready,
    Missing,
    Unavailable,
    Info,
}

/// One line of the Downloads page: status dot, name, size, status and description on the
/// left, buttons on the right.
fn item_row(ui: &mut egui::Ui, state: ItemState, status: &str, name: &str, size: &str, what: &str, buttons: impl FnOnce(&mut egui::Ui)) {
    let col = match state {
        ItemState::Ready => GREEN,
        ItemState::Missing => ORANGE,
        ItemState::Unavailable => RED,
        ItemState::Info => MUTED,
    };
    let text = |ui: &mut egui::Ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("●").color(col));
            ui.label(RichText::new(name).strong().size(14.5).color(CREAM));
            if !size.is_empty() {
                ui.label(RichText::new(size).size(12.0).color(MUTED));
            }
            if !status.is_empty() {
                ui.label(RichText::new(status).size(12.0).color(col));
            }
        });
        if !what.is_empty() {
            ui.label(RichText::new(what).size(12.0).color(MUTED));
        }
    };
    egui::Frame::new()
        .fill(INPUT_BG)
        .stroke(Stroke::new(1.0, CARD_STROKE))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if ui.available_width() >= 700.0 {
                // Wide: text on the left, buttons on the right.
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        buttons(ui);
                        ui.with_layout(Layout::top_down(Align::Min), text);
                    });
                });
            } else {
                // Narrow: buttons on their own line under the text.
                text(ui);
                ui.add_space(2.0);
                ui.horizontal_wrapped(buttons);
            }
        });
    ui.add_space(6.0);
}

fn link_button(ui: &mut egui::Ui, label: &str) -> bool {
    ui.add(egui::Button::new(RichText::new(label).size(12.5).color(YELLOW)).fill(CARD).corner_radius(6)).clicked()
}

fn action_button(ui: &mut egui::Ui, label: &str, primary: bool, enabled: bool) -> bool {
    let b = egui::Button::new(RichText::new(label).size(13.0).strong().color(if primary { INK } else { CREAM }))
        .fill(if primary { YELLOW } else { Color32::from_rgb(70, 52, 40) })
        .corner_radius(6);
    ui.add_enabled(enabled, b).clicked()
}

/// Popular VOICEVOX voices (speaker style ids of the standard engine).
const VOICEVOX_SPEAKERS: &[(u32, &str)] = &[
    (3, "Zundamon · normal"),
    (1, "Zundamon · sweet"),
    (2, "Shikoku Metan · normal"),
    (0, "Shikoku Metan · sweet"),
    (8, "Kasukabe Tsumugi"),
    (9, "Namine Ritsu"),
    (10, "Amehare Hau"),
    (11, "Kurono Takehiro"),
    (12, "Shirakami Kotarou"),
    (13, "Aoyama Ryusei"),
    (14, "Meimei Himari"),
    (16, "Kyushu Sora"),
];

/// The picture that stands for "language": a mouth with sound waves coming from it, on a tile
/// in the app icon's yellow.
fn paint_language_icon(p: &egui::Painter, rect: egui::Rect) {
    let s = rect.width().min(rect.height());
    let o = rect.center() - vec2(s, s) * 0.5;
    let at = |x: f32, y: f32| egui::pos2(o.x + x * s, o.y + y * s);
    p.rect_filled(egui::Rect::from_min_size(o, vec2(s, s)), (s * 0.24).round() as u8, YELLOW);
    // Lips: a full lower lip and an upper lip with two curves (the "cupid's bow").
    let lip = Color32::from_rgb(196, 62, 44);
    let lip_shade = Color32::from_rgb(140, 36, 26);
    let lip_light = Color32::from_rgb(232, 120, 96);
    p.add(egui::Shape::ellipse_filled(at(0.34, 0.57), vec2(0.255, 0.16) * s, lip_shade));
    p.add(egui::Shape::ellipse_filled(at(0.34, 0.555), vec2(0.245, 0.15) * s, lip));
    p.add(egui::Shape::ellipse_filled(at(0.255, 0.45), vec2(0.15, 0.105) * s, lip));
    p.add(egui::Shape::ellipse_filled(at(0.425, 0.45), vec2(0.15, 0.105) * s, lip));
    p.add(egui::Shape::ellipse_filled(at(0.34, 0.635), vec2(0.1, 0.028) * s, lip_light));
    // Open mouth with a glimpse of teeth.
    p.add(egui::Shape::ellipse_filled(at(0.34, 0.525), vec2(0.165, 0.068) * s, INK));
    p.add(egui::Shape::ellipse_filled(at(0.34, 0.49), vec2(0.115, 0.026) * s, CREAM));
    // Sound waves coming out of the mouth.
    let c = at(0.575, 0.525);
    for (i, r) in [0.13f32, 0.225, 0.32].into_iter().enumerate() {
        let pts: Vec<egui::Pos2> = (0..=18)
            .map(|k| {
                let a = (-52.0 + 104.0 * k as f32 / 18.0).to_radians();
                c + vec2(a.cos(), a.sin()) * r * s
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new((s * (0.07 - i as f32 * 0.01)).max(1.3), INK)));
    }
}

/// The interface-language button in the header: the speaking-mouth picture and the language's
/// own name.
fn language_button(ui: &mut egui::Ui, lang: &Lang, busy: bool, open: bool) -> egui::Response {
    // A name the fonts can't show yet (before its font is set up) is shown in English.
    let font = FontId::proportional(14.0);
    let shown = if ui.ctx().fonts_mut(|f| f.has_glyphs(&font, lang.native)) { lang.native } else { lang.english };
    let label = if busy { format!("{shown} …") } else { shown.to_string() };
    let galley = ui.painter().layout_no_wrap(label, font, CREAM);
    let icon = 24.0;
    let size = vec2(5.0 + icon + 8.0 + galley.size().x + 12.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let hot = open || resp.hovered();
    let fill = if open { Color32::from_rgb(66, 54, 18) } else if resp.hovered() { Color32::from_rgb(52, 46, 30) } else { CARD };
    p.rect_filled(rect, 16, fill);
    p.rect_stroke(rect, 16, Stroke::new(1.0, if hot { YELLOW } else { CARD_STROKE }), egui::StrokeKind::Inside);
    let icon_rect = egui::Rect::from_min_size(egui::pos2(rect.left() + 5.0, rect.center().y - icon / 2.0), vec2(icon, icon));
    paint_language_icon(p, icon_rect);
    p.galley(egui::pos2(icon_rect.right() + 8.0, rect.center().y - galley.size().y / 2.0), galley, CREAM);
    resp
}

/// One language in the language menu: its own name, its English name and a note on the right.
fn language_row(ui: &mut egui::Ui, l: &Lang, current: bool, tag: &str) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    let p = ui.painter();
    if current {
        p.rect_filled(rect, 6, Color32::from_rgb(66, 54, 18));
    } else if resp.hovered() {
        p.rect_filled(rect, 6, Color32::from_rgb(52, 46, 30));
    }
    let name_font = FontId::proportional(14.0);
    // Its own name when the fonts can show it, otherwise just the English name.
    let own = ui.ctx().fonts_mut(|f| f.has_glyphs(&name_font, l.native));
    let mut x = rect.left() + 8.0;
    let y = rect.center().y;
    if current {
        p.text(egui::pos2(x, y), Align2::LEFT_CENTER, "✓", FontId::proportional(13.0), YELLOW);
    }
    x += 18.0;
    let main = if own { l.native } else { l.english };
    let r = p.text(egui::pos2(x, y), Align2::LEFT_CENTER, main, name_font, if current { YELLOW } else { CREAM });
    if own && l.native != l.english {
        p.text(egui::pos2(r.right() + 8.0, y + 1.0), Align2::LEFT_CENTER, l.english, FontId::proportional(12.0), MUTED);
    }
    if !tag.is_empty() {
        p.text(egui::pos2(rect.right() - 8.0, y + 1.0), Align2::RIGHT_CENTER, tag, FontId::proportional(11.5), MUTED);
    }
    if l.script.rtl() { resp.on_hover_text(tr("Right-to-left writing may show in the wrong order in places.")) } else { resp }
}

/// A drop-down of every language (own name beside the English one when the fonts can show it).
fn language_combo(ui: &mut egui::Ui, id: &str, code: &mut String) -> bool {
    let cur = langs::find(code).unwrap_or_else(langs::english);
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).width(190.0).selected_text(cur.english).show_ui(ui, |ui| {
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for l in langs::LANGUAGES {
                let label = if l.native != l.english && l.script.built_in() { format!("{} · {}", l.english, l.native) } else { l.english.to_string() };
                if ui.selectable_label(cur.code == l.code, label).clicked() {
                    *code = l.code.to_string();
                    changed = true;
                }
            }
        });
    });
    changed
}

fn toggle_chip(ui: &mut egui::Ui, on: &mut bool, label: &str) {
    let b = egui::Button::new(RichText::new(label).color(if *on { INK } else { CREAM }))
        .fill(if *on { YELLOW } else { CARD })
        .corner_radius(14)
        .min_size(vec2(0.0, 28.0));
    if ui.add(b).clicked() {
        *on = !*on;
    }
}

fn psm_label(p: u32) -> &'static str {
    match p {
        3 => "3 · automatic page layout",
        6 => "6 · single text block",
        11 => "11 · sparse text (video)",
        12 => "12 · sparse text + OSD",
        _ => "custom",
    }
}

/// Opens the system file manager at the finished video.
fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}

/// What `install_fonts` found.
struct UiFonts {
    /// A Japanese-capable font (for 日本語 anywhere in the window).
    japanese: bool,
    /// The interface language's letters can be shown.
    script_ok: bool,
}

/// Sets up the window's fonts: egui's own (Latin, Greek, Cyrillic), the symbols font, a font
/// for the interface language's writing system (Chinese, Korean, Hindi, Arabic, Thai…) and a
/// Japanese-capable font. `all_scripts` adds the computer's fonts for every other writing
/// system too (for the language menu, which shows each language's own name).
fn install_fonts(ctx: &egui::Context, cfg: &Config, all_scripts: bool) -> UiFonts {
    let mut defs = FontDefinitions::default();
    // The bundled monospace font has the arrows, dots and squares (▶ ⬇ ● ↗ ■ →) the
    // proportional one lacks; fall back to it so they never show as boxes.
    if defs.font_data.contains_key("Hack") {
        let prop = defs.families.entry(FontFamily::Proportional).or_default();
        if !prop.iter().any(|n| n == "Hack") {
            prop.push("Hack".into());
        }
    }
    let mut add = |name: &str, f: &crate::fonts::FoundFont, cjk: bool| -> bool {
        let Some(data) = crate::fonts::load(f) else { return false };
        let mut fd = FontData::from_owned(data);
        fd.index = f.index;
        if cjk {
            // CJK fonts sit higher and look larger than egui's Latin font; line them up.
            fd.tweak.scale = 0.92;
            fd.tweak.y_offset_factor = 0.12;
        }
        defs.font_data.insert(name.into(), Arc::new(fd));
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            defs.families.entry(fam).or_default().push(name.into());
        }
        true
    };
    // The interface language's own letters come first, so Chinese, for one, gets Chinese shapes.
    let lang = langs::find(cfg.ui_lang()).unwrap_or_else(langs::english);
    let mut script_ok = lang.script.built_in();
    if !script_ok && lang.script != langs::Script::Japanese {
        if let Some(f) = crate::fonts::find_script_font(lang.script) {
            script_ok = add("interface-script", &f, lang.script.cjk());
        }
    }
    let japanese = crate::fonts::find_ui_font(&cfg.overlay.font_path).is_some_and(|f| add("cjk", &f, true));
    if lang.script == langs::Script::Japanese {
        script_ok = japanese;
    }
    if all_scripts {
        // Only fonts already on this computer (nothing is downloaded just for the menu).
        let mut seen = HashSet::new();
        for s in langs::LANGUAGES.iter().map(|l| l.script) {
            if s.built_in() || s == langs::Script::Japanese || s == lang.script || !seen.insert(s) {
                continue;
            }
            if let Some(f) = crate::fonts::find_script_font(s) {
                add(&format!("names-{s:?}"), &f, s.cjk());
            }
        }
    }
    ctx.set_fonts(defs);
    UiFonts { japanese, script_ok }
}

fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = PANEL;
        v.window_fill = CARD;
        v.extreme_bg_color = INPUT_BG;
        v.faint_bg_color = CARD;
        v.override_text_color = Some(CREAM);
        v.selection.bg_fill = YELLOW_DIM;
        v.selection.stroke = Stroke::new(1.0, YELLOW);
        v.hyperlink_color = YELLOW;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, CARD_STROKE);
        v.widgets.inactive.bg_fill = Color32::from_rgb(52, 47, 34);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(52, 47, 34);
        v.widgets.hovered.bg_fill = Color32::from_rgb(70, 62, 40);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(70, 62, 40);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, YELLOW_DIM);
        v.widgets.active.bg_fill = YELLOW_DIM;
        v.widgets.active.weak_bg_fill = YELLOW_DIM;
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::same(6);
        }
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(10.0, 5.0);
        style.spacing.slider_width = 150.0;
        use egui::TextStyle::*;
        style.text_styles.insert(Body, FontId::proportional(14.0));
        style.text_styles.insert(Button, FontId::proportional(14.0));
        style.text_styles.insert(Heading, FontId::proportional(20.0));
        style.text_styles.insert(Small, FontId::proportional(11.5));
        style.text_styles.insert(Monospace, FontId::monospace(12.5));
    });
}
