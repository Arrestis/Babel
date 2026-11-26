//! Shared helpers: cancellation, silent child processes, messages to the UI, downloads.

use anyhow::{Context, Result, anyhow, bail};
use crossbeam_channel::Sender;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Shared "please stop" flag checked by every pipeline step.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() { Err(anyhow!(Cancelled)) } else { Ok(()) }
    }
    /// Raw flag, handed to whisper.cpp's abort callback.
    #[cfg_attr(not(feature = "whisper"), allow(dead_code))]
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

/// Error marker for a user cancel.
#[derive(Debug)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled by user")
    }
}
impl std::error::Error for Cancelled {}

pub fn is_cancel_error(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Cancelled>().is_some()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Ok,
}

/// The pipeline stages shown in the progress UI.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Stage {
    Extract,
    Ocr,
    Transcribe,
    Translate,
    Synthesize,
    Render,
}

impl Stage {
    pub const ALL: [Stage; 6] =
        [Stage::Extract, Stage::Ocr, Stage::Transcribe, Stage::Translate, Stage::Synthesize, Stage::Render];
    pub fn label(self) -> &'static str {
        match self {
            Stage::Extract => "Extracting Media",
            Stage::Ocr => "Scanning Screen Text (OCR)",
            Stage::Transcribe => "Transcribing Speech (Whisper)",
            Stage::Translate => "Translating to Japanese",
            Stage::Synthesize => "Synthesizing Japanese Voice",
            Stage::Render => "Rendering Final Video",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Stage::Extract => "Extract",
            Stage::Ocr => "OCR",
            Stage::Transcribe => "Whisper",
            Stage::Translate => "Translate",
            Stage::Synthesize => "Voice",
            Stage::Render => "Render",
        }
    }
    /// Rough share of total run time, used for the overall bar and the ETA.
    pub fn weight(self) -> f32 {
        match self {
            Stage::Extract => 4.0,
            Stage::Ocr => 26.0,
            Stage::Transcribe => 20.0,
            Stage::Translate => 10.0,
            Stage::Synthesize => 14.0,
            Stage::Render => 26.0,
        }
    }
}

/// Messages from worker threads to the UI.
#[derive(Debug)]
pub enum Msg {
    /// The set of stages that will run for this job.
    Plan(Vec<Stage>),
    StageStart(Stage),
    StageProgress(f32),
    StageDone(Stage),
    Log(LogLevel, String),
    /// First-use downloads before the real work starts (`None` once they are done).
    Setup(Option<SetupInfo>),
    Finished(std::path::PathBuf),
    Failed(String),
    Cancelled,
}

/// What the one-time setup is fetching right now, for the progress screen.
#[derive(Clone, Debug)]
pub struct SetupInfo {
    /// Which download this is (1-based) and how many there are.
    pub index: usize,
    pub count: usize,
    /// What is being set up, e.g. "AI brain model Qwen 3 · 4B" or "Unpacking FFmpeg".
    pub what: String,
    /// Share of this item done (0..1), when known.
    pub frac: Option<f32>,
    /// Size, speed and time left while a file is downloading.
    pub download: Option<DownloadStats>,
}

/// Live numbers for a file being downloaded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DownloadStats {
    /// Bytes on disk so far (including any kept from an earlier, interrupted try).
    pub done: u64,
    pub total: Option<u64>,
    /// Average speed of this try in bytes per second (0 until it can be measured).
    pub per_sec: f64,
    /// Bytes kept from an earlier try that this download carries on from.
    pub resumed_from: u64,
}

impl DownloadStats {
    pub fn frac(&self) -> Option<f32> {
        self.total.filter(|t| *t > 0).map(|t| (self.done as f64 / t as f64).clamp(0.0, 1.0) as f32)
    }
    /// Seconds left at the current speed.
    pub fn eta_secs(&self) -> Option<f64> {
        let total = self.total?;
        (self.per_sec > 0.0).then(|| total.saturating_sub(self.done) as f64 / self.per_sec)
    }
}

thread_local! {
    static CURRENT_DOWNLOAD: std::cell::Cell<Option<DownloadStats>> = const { std::cell::Cell::new(None) };
}

/// Size, speed and time left of the download running on this thread. Progress callbacks are
/// called on the downloading thread, so they can read it to show more than a percentage.
pub fn current_download() -> Option<DownloadStats> {
    CURRENT_DOWNLOAD.with(|c| c.get())
}

fn set_current_download(s: Option<DownloadStats>) {
    CURRENT_DOWNLOAD.with(|c| c.set(s));
}

/// Clears this thread's download numbers however the download ends.
struct DownloadStatsGuard;
impl Drop for DownloadStatsGuard {
    fn drop(&mut self) {
        set_current_download(None);
    }
}

/// Sizes as download pages write them (1 GB = 1000 MB): "850 KB", "2.5 MB", "142 MB", "2.5 GB".
pub fn fmt_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e7 {
        format!("{:.0} MB", b / 1e6)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.0))
    }
}

#[derive(Clone)]
pub struct Reporter {
    tx: Sender<Msg>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

impl Reporter {
    pub fn new(tx: Sender<Msg>, repaint: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self { tx, repaint }
    }
    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        (self.repaint)();
    }
    pub fn log(&self, level: LogLevel, text: impl Into<String>) {
        self.send(Msg::Log(level, text.into()));
    }
    pub fn info(&self, text: impl Into<String>) {
        self.log(LogLevel::Info, text);
    }
    pub fn warn(&self, text: impl Into<String>) {
        self.log(LogLevel::Warn, text);
    }
    pub fn progress(&self, frac: f32) {
        self.send(Msg::StageProgress(frac.clamp(0.0, 1.0)));
    }
}

/// A `Command` that never opens a console window and never touches the speakers/terminal.
pub fn silent_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub struct ProcOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr_tail: Vec<String>,
}

impl ProcOutput {
    pub fn ok_or_err(self, what: &str) -> Result<Self> {
        if self.status.success() {
            Ok(self)
        } else {
            bail!("{what} failed ({}):\n{}", self.status, self.stderr_tail.join("\n"))
        }
    }
}

enum Line {
    Out(String),
    Err(String),
}

/// Runs a child process to completion while watching the cancel flag.
///
/// * every stdout / stderr line is passed to `on_line(is_stderr, line)`;
/// * stdout is also collected and returned;
/// * on cancel the process first gets `q` on stdin (FFmpeg's graceful stop),
///   then is killed if it hasn't exited after a short grace period.
pub fn run_process(
    mut cmd: Command,
    cancel: &CancelToken,
    stdin_data: Option<&str>,
    mut on_line: impl FnMut(bool, &str),
) -> Result<ProcOutput> {
    let program = cmd.get_program().to_string_lossy().to_string();
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().with_context(|| format!("could not start `{program}` (is it installed and on PATH?)"))?;

    let mut stdin = child.stdin.take();
    if let (Some(data), Some(pipe)) = (stdin_data, stdin.as_mut()) {
        let _ = pipe.write_all(data.as_bytes());
        let _ = pipe.flush();
        stdin = None; // close so the program sees EOF
    }

    let (tx, rx) = crossbeam_channel::unbounded::<Line>();
    let out = child.stdout.take().map(|s| {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(s).lines().map_while(|l| l.ok()) {
                let _ = tx.send(Line::Out(line));
            }
        })
    });
    let err = child.stderr.take().map(|s| {
        let tx = tx.clone();
        std::thread::spawn(move || {
            // FFmpeg/whisper use '\r' for progress lines; split on both.
            let mut reader = BufReader::new(s);
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            while let Ok(1) = reader.read(&mut byte) {
                if byte[0] == b'\n' || byte[0] == b'\r' {
                    if !buf.is_empty() {
                        let _ = tx.send(Line::Err(String::from_utf8_lossy(&buf).to_string()));
                        buf.clear();
                    }
                } else {
                    buf.push(byte[0]);
                }
            }
            if !buf.is_empty() {
                let _ = tx.send(Line::Err(String::from_utf8_lossy(&buf).to_string()));
            }
        })
    });
    drop(tx);

    let mut stdout = String::new();
    let mut tail: std::collections::VecDeque<String> = Default::default();
    let mut cancel_started: Option<Instant> = None;

    let status = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Line::Out(l)) => {
                on_line(false, &l);
                stdout.push_str(&l);
                stdout.push('\n');
            }
            Ok(Line::Err(l)) => {
                on_line(true, &l);
                if tail.len() >= 25 {
                    tail.pop_front();
                }
                tail.push_back(l);
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                // Pipes closed: wait for exit (with cancel handling).
                if let Some(s) = child.try_wait()? {
                    break s;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        }
        if cancel.is_cancelled() {
            match cancel_started {
                None => {
                    cancel_started = Some(Instant::now());
                    if let Some(pipe) = stdin.as_mut() {
                        let _ = pipe.write_all(b"q\n");
                        let _ = pipe.flush();
                    }
                }
                Some(t) if t.elapsed() > Duration::from_millis(1500) => {
                    let _ = child.kill();
                }
                _ => {}
            }
        }
        if let Some(s) = child.try_wait()? {
            // Drain what's left.
            while let Ok(l) = rx.recv_timeout(Duration::from_millis(50)) {
                match l {
                    Line::Out(l) => {
                        on_line(false, &l);
                        stdout.push_str(&l);
                        stdout.push('\n');
                    }
                    Line::Err(l) => {
                        on_line(true, &l);
                        tail.push_back(l);
                    }
                }
            }
            break s;
        }
    };
    drop(stdin);
    if let Some(h) = out {
        let _ = h.join();
    }
    if let Some(h) = err {
        let _ = h.join();
    }
    cancel.check()?;
    Ok(ProcOutput { status, stdout, stderr_tail: tail.into_iter().collect() })
}

/// Quick "does this program run?" probe. Returns the first output line.
pub fn probe_program(program: &str, args: &[&str]) -> Option<String> {
    let out = silent_command(program).args(args).stdin(Stdio::null()).output().ok()?;
    let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
    let s = String::from_utf8_lossy(&text);
    Some(s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string())
}

/// Settings shared by every web request. Windows uses its own secure-connection support
/// (SChannel) and certificate store; Linux and macOS use rustls with Mozilla's roots.
pub fn agent_config() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    let builder = ureq::Agent::config_builder();
    #[cfg(windows)]
    let builder = builder.tls_config(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build(),
    );
    builder
}

pub fn http_agent(timeout_secs: u64) -> ureq::Agent {
    agent_config()
        .timeout_global(Some(Duration::from_secs(timeout_secs)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// Reads a ureq response body with a generous size limit.
pub fn read_body_bytes(resp: &mut ureq::http::Response<ureq::Body>) -> Result<Vec<u8>> {
    Ok(resp.body_mut().with_config().limit(512 * 1024 * 1024).read_to_vec()?)
}

pub fn read_body_string(resp: &mut ureq::http::Response<ureq::Body>) -> Result<String> {
    Ok(resp.body_mut().with_config().limit(64 * 1024 * 1024).read_to_string()?)
}

/// Downloads `url` to `dest` (via a .part file), reporting progress 0..1 when the size is known.
/// An interrupted download (closed app, lost connection) carries on where it stopped, and a
/// dropped connection is retried a few times before giving up.
pub fn download(url: &str, dest: &Path, cancel: &CancelToken, mut progress: impl FnMut(f32, u64)) -> Result<()> {
    let _clear_stats = DownloadStatsGuard;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = dest.with_extension("part");
    let mut last_err = None;
    for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(2 * attempt as u64));
        }
        match download_once(url, &part, cancel, &mut progress) {
            Ok(()) => {
                std::fs::rename(&part, dest)?;
                let _ = std::fs::remove_file(size_note(&part));
                return Ok(());
            }
            Err(e) if is_cancel_error(&e) => return Err(e),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("download failed: {url}")))
}

/// Remembers the full size of a partly downloaded file, so a resume only continues the same file.
fn size_note(part: &Path) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.size", part.display()))
}

fn download_once(url: &str, part: &Path, cancel: &CancelToken, progress: &mut impl FnMut(f32, u64)) -> Result<()> {
    let agent: ureq::Agent = agent_config()
        .timeout_connect(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into();
    let note = size_note(part);
    let expected: Option<u64> = std::fs::read_to_string(&note).ok().and_then(|s| s.trim().parse().ok());
    let mut have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if expected.is_none() {
        have = 0; // no record of what this part file belongs to: start over
    }
    let mut req = agent.get(url);
    if have > 0 {
        req = req.header("Range", format!("bytes={have}-"));
    }
    let resp = req.call().with_context(|| format!("download failed: {url}"))?;
    let status = resp.status().as_u16();
    // 416: the part file already holds everything.
    if status == 416 && have > 0 && expected == Some(have) {
        return Ok(());
    }
    if status == 206 && have > 0 {
        // "bytes 100-199/200": the total must match the file we started.
        let total = resp.headers().get("content-range").and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit('/').next()).and_then(|t| t.trim().parse::<u64>().ok());
        if total.is_none() || total != expected {
            drop(resp);
            let _ = std::fs::remove_file(part);
            let _ = std::fs::remove_file(&note);
            return download_once(url, part, cancel, progress);
        }
    }
    if !(200..300).contains(&status) {
        bail!("download failed ({status}): {url}");
    }
    let resumed = status == 206 && have > 0;
    let mut done: u64 = if resumed { have } else { 0 };
    let total = resp.body().content_length().map(|len| len + done);
    if !resumed {
        match total {
            Some(t) => std::fs::write(&note, t.to_string())?,
            None => {
                let _ = std::fs::remove_file(&note);
            }
        }
    }
    let mut reader = resp.into_body().into_reader();
    let mut file = if resumed {
        std::fs::OpenOptions::new().append(true).open(part)?
    } else {
        std::fs::File::create(part)?
    };
    let started = Instant::now();
    let start_bytes = done;
    let resumed_from = if resumed { have } else { 0 };
    let frac_of = |done: u64| total.map(|t| done as f32 / t.max(1) as f32).unwrap_or(0.0);
    set_current_download(Some(DownloadStats { done, total, per_sec: 0.0, resumed_from }));
    progress(frac_of(done), done);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.is_cancelled() {
            // Keep what arrived so far: the next try carries on from here.
            let _ = file.flush();
            bail!(Cancelled);
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        let secs = started.elapsed().as_secs_f64();
        let per_sec = if secs >= 1.0 { (done - start_bytes) as f64 / secs } else { 0.0 };
        set_current_download(Some(DownloadStats { done, total, per_sec, resumed_from }));
        progress(frac_of(done), done);
    }
    file.flush()?;
    if let Some(t) = total {
        if done < t {
            bail!("the connection closed early ({done} of {t} bytes)");
        }
    }
    Ok(())
}

/// Short, stable fingerprint of a text (FNV-1a), for naming saved files.
pub fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

pub fn fmt_duration(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Secure web requests and a redirected download work with this system's TLS setup.
    /// Run with: cargo test https_works -- --ignored --nocapture
    #[test]
    #[ignore]
    fn https_works() {
        let latest = crate::tools::github_json("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest").unwrap();
        println!("llama.cpp latest stable: {}", latest["tag_name"]);
        let (_, url) = crate::tools::release_files(&latest, &|n: &str| n == "nightly-tag.txt").into_iter().next().unwrap();
        let dest = std::path::PathBuf::from("target/e2e/nightly-tag.txt");
        let _ = std::fs::remove_file(&dest);
        download(&url, &dest, &CancelToken::default(), |_, _| {}).unwrap();
        let text = std::fs::read_to_string(&dest).unwrap();
        println!("nightly tag: {}", text.trim());
        assert!(text.trim().starts_with('b'));
    }
}

/// Seconds → "HH:MM:SS.mmm" for logs.
pub fn fmt_ts(secs: f64) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000)
}
