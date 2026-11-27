// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-contained setup: BabelCraft finds or downloads its own helper tools and models
//! (FFmpeg/FFprobe, whisper.cpp CLI, Whisper model, OCR models, Japanese font) into its
//! data folder, so nothing has to be installed by hand.

use crate::config::{self, Config};
use crate::util::{CancelToken, download, probe_program};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub const OCR_DETECTION_URL: &str = "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx";
pub const OCR_RECOGNITION_URL: &str = "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx";

pub fn tools_dir() -> PathBuf {
    config::data_dir().join("tools")
}

pub fn ocr_models_dir() -> PathBuf {
    config::models_dir().join("ocrs")
}

pub fn ocr_models() -> (PathBuf, PathBuf) {
    let d = ocr_models_dir();
    (d.join("text-detection.onnx"), d.join("text-recognition.onnx"))
}

pub fn ocr_models_present() -> bool {
    let (a, b) = ocr_models();
    a.is_file() && b.is_file()
}

pub fn exe_name(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// GET a GitHub API address and parse the JSON reply.
pub fn github_json(url: &str) -> Result<serde_json::Value> {
    let agent = crate::util::http_agent(30);
    let mut resp = agent
        .get(url)
        .header("User-Agent", "BabelCraft")
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("could not reach GitHub")?;
    let status = resp.status();
    let body = crate::util::read_body_string(&mut resp)?;
    if !status.is_success() {
        bail!("GitHub answered {status} (try again in a few minutes)");
    }
    serde_json::from_str(&body).context("unexpected reply from GitHub")
}

/// (name, url) of a release's files that match `wanted`.
pub fn release_files(release: &serde_json::Value, wanted: &dyn Fn(&str) -> bool) -> Vec<(String, String)> {
    release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["name"].as_str()?.to_string(), a["browser_download_url"].as_str()?.to_string())))
        .filter(|(n, _)| wanted(n))
        .collect()
}

/// Download links of the newest GitHub release that has files matching `wanted`
/// (name, url) — every matching file of that one release, so split packages stay together.
pub fn github_assets(repo: &str, allow_prerelease: bool, wanted: impl Fn(&str) -> bool) -> Result<Vec<(String, String)>> {
    let releases = github_json(&format!("https://api.github.com/repos/{repo}/releases?per_page=15"))
        .with_context(|| format!("could not look up {repo}"))?;
    for r in releases.as_array().into_iter().flatten() {
        if r["draft"].as_bool() == Some(true) || (!allow_prerelease && r["prerelease"].as_bool() == Some(true)) {
            continue;
        }
        let found = release_files(r, &wanted);
        if !found.is_empty() {
            return Ok(found);
        }
    }
    bail!("no matching download found in {repo}'s releases")
}

/// Folders searched for bundled helper programs, in order.
fn search_dirs(name: &str) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.to_path_buf());
            v.push(dir.join("tools"));
            v.push(dir.join("../Resources/tools")); // macOS .app bundle
        }
    }
    let t = tools_dir();
    v.push(t.join(name));
    // ffprobe lives beside ffmpeg, etc.
    for sub in ["ffmpeg", "whisper-cli"] {
        v.push(t.join(sub));
    }
    v.push(t.clone());
    v
}

/// Turns a configured program ("ffmpeg" or a full path) into the program to run:
/// an explicit path wins, then a copy shipped next to BabelCraft or downloaded by it,
/// then whatever is on PATH.
pub fn resolve(configured: &str, name: &str) -> PathBuf {
    let c = configured.trim();
    let bare = c.is_empty() || c == name || c == exe_name(name);
    if !bare {
        return PathBuf::from(c);
    }
    for dir in search_dirs(name) {
        let p = dir.join(exe_name(name));
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(name)
}

pub fn works(program: &Path, arg: &str) -> bool {
    probe_program(&program.to_string_lossy(), &[arg]).is_some()
}

// ------------------------------------------------------------------ installers

/// Where a portable FFmpeg build for this computer can be downloaded (archive URL(s)).
fn ffmpeg_sources() -> Result<Vec<&'static str>> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    Ok(match (os, arch) {
        ("windows", "x86_64") => vec!["https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"],
        ("windows", "aarch64") => vec!["https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-winarm64-gpl.zip"],
        // 32-bit Windows: the 64-bit build is used when the OS is 64-bit (most "x86" installs).
        ("windows", "x86") if std::env::var("PROCESSOR_ARCHITEW6432").is_ok() => {
            vec!["https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"]
        }
        ("linux", "x86_64") => vec!["https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"],
        ("linux", "aarch64") => vec!["https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linuxarm64-gpl.tar.xz"],
        ("macos", "aarch64") => vec![
            "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/snapshot/ffmpeg.zip",
            "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/snapshot/ffprobe.zip",
        ],
        ("macos", _) => vec![
            "https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/release/ffmpeg.zip",
            "https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/release/ffprobe.zip",
        ],
        _ => bail!("No ready-made FFmpeg download for {os}/{arch} — please install FFmpeg with your package manager."),
    })
}

/// Name of the whisper.cpp release file for this computer (None = no prebuilt CLI).
fn whisper_asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("whisper-bin-x64.zip"),
        ("windows", "x86") => Some("whisper-bin-Win32.zip"),
        ("windows", "aarch64") => Some("whisper-bin-win-cpu-arm64.zip"),
        ("linux", "x86_64") => Some("whisper-bin-ubuntu-x64.tar.gz"),
        ("linux", "aarch64") => Some("whisper-bin-ubuntu-arm64.tar.gz"),
        _ => None,
    }
}

pub fn whisper_cli_installable() -> bool {
    whisper_asset().is_some()
}

/// Progress reporting for installers: (what, fraction 0..1 or <0 when unknown).
pub type Progress<'a> = &'a mut dyn FnMut(&str, f32);

/// Downloads one file. The label stays `what`; sizes, speed and time left are available to the
/// progress callback through `util::current_download()`.
pub fn fetch(url: &str, dest: &Path, what: &str, cancel: &CancelToken, progress: Progress) -> Result<()> {
    progress(what, 0.0);
    download(url, dest, cancel, |f, _bytes| progress(what, if f > 0.0 { f } else { -1.0 }))
        .with_context(|| format!("could not download {what}"))
}

pub fn install_ffmpeg(cancel: &CancelToken, progress: Progress) -> Result<()> {
    let dest = tools_dir().join("ffmpeg");
    let tmp = tools_dir().join("_download");
    std::fs::create_dir_all(&tmp)?;
    for (i, url) in ffmpeg_sources()?.into_iter().enumerate() {
        let file = tmp.join(format!("ffmpeg-{i}{}", archive_ext(url)));
        fetch(url, &file, "FFmpeg", cancel, progress)?;
        progress("Unpacking FFmpeg", -1.0);
        let unpack = tmp.join(format!("unpacked-{i}"));
        let _ = std::fs::remove_dir_all(&unpack);
        extract(&file, &unpack)?;
        std::fs::create_dir_all(&dest)?;
        for name in ["ffmpeg", "ffprobe"] {
            if let Some(found) = find_file(&unpack, &exe_name(name)) {
                let target = dest.join(exe_name(name));
                let _ = std::fs::remove_file(&target);
                std::fs::copy(&found, &target)?;
                make_executable(&target);
            }
        }
        let _ = std::fs::remove_dir_all(&unpack);
        let _ = std::fs::remove_file(&file);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if !dest.join(exe_name("ffmpeg")).is_file() || !dest.join(exe_name("ffprobe")).is_file() {
        bail!("FFmpeg download did not contain ffmpeg and ffprobe");
    }
    Ok(())
}

pub fn install_whisper_cli(cancel: &CancelToken, progress: Progress) -> Result<()> {
    let asset = whisper_asset().context("no ready-made whisper.cpp tool for this system — use a BabelCraft build with built-in Whisper, or Settings → Speech → Online API")?;
    // Newest release that actually has files attached.
    let (_, url) = github_assets("ggml-org/whisper.cpp", true, |n| n == asset)?
        .into_iter()
        .next()
        .with_context(|| format!("whisper.cpp release file {asset} not found"))?;

    let dest = tools_dir().join("whisper-cli");
    let tmp = tools_dir().join("_download");
    std::fs::create_dir_all(&tmp)?;
    let file = tmp.join(asset);
    fetch(&url, &file, "Whisper tool (whisper.cpp)", cancel, progress)?;
    progress("Unpacking whisper.cpp", -1.0);
    let _ = std::fs::remove_dir_all(&dest);
    let unpack = tmp.join("whisper");
    extract(&file, &unpack)?;
    // Keep the folder holding whisper-cli together with its DLLs / shared libraries.
    let cli = find_file(&unpack, &exe_name("whisper-cli")).context("whisper-cli not found in the download")?;
    let folder = cli.parent().unwrap_or(&unpack).to_path_buf();
    copy_dir(&folder, &dest)?;
    // Shared libraries may sit in a sibling lib folder.
    if let Some(parent) = folder.parent() {
        for lib in ["lib", "lib64"] {
            let l = parent.join(lib);
            if l.is_dir() {
                copy_dir(&l, &dest)?;
            }
        }
    }
    for entry in std::fs::read_dir(&dest)?.flatten() {
        make_executable(&entry.path());
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

pub fn install_ocr_models(cancel: &CancelToken, progress: Progress) -> Result<()> {
    let (det, rec) = ocr_models();
    if !det.is_file() {
        fetch(OCR_DETECTION_URL, &det, "Text-detection model", cancel, progress)?;
    }
    if !rec.is_file() {
        fetch(OCR_RECOGNITION_URL, &rec, "Text-recognition model", cancel, progress)?;
    }
    Ok(())
}

pub fn install_whisper_model(cfg: &Config, cancel: &CancelToken, progress: Progress) -> Result<()> {
    let model = cfg.whisper_model();
    if model.is_file() {
        return Ok(());
    }
    let file = model.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| config::DEFAULT_WHISPER_MODEL.into());
    if !file.starts_with("ggml-") {
        bail!("Whisper model {} not found", model.display());
    }
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{file}");
    fetch(&url, &model, "Whisper speech model", cancel, progress)
}

pub fn install_font(cancel: &CancelToken, progress: Progress) -> Result<()> {
    let dest = config::fonts_dir().join(crate::fonts::NOTO_FILE);
    fetch(crate::fonts::NOTO_URL, &dest, "Japanese font (Noto Sans JP)", cancel, progress)
}

/// The language data Tesseract has (None when Tesseract can't be run).
pub fn tesseract_languages(cfg: &Config) -> Option<Vec<String>> {
    let exe = cfg.tesseract();
    if !works(&exe, "--version") {
        return None;
    }
    let out = crate::util::silent_command(&exe).arg("--list-langs").stdin(std::process::Stdio::null()).output().ok()?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Some(text.lines().skip(1).map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

/// Everything BabelCraft can download and set up by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Ffmpeg,
    OcrModels,
    Font,
    WhisperModel,
    WhisperCli,
    /// llama.cpp server for the offline AI brain.
    BrainEngine,
    /// The AI model the offline brain runs (the one chosen in Settings).
    BrainModel,
    /// The free VOICEVOX Japanese voice engine.
    VoiceEngine,
}

impl Component {
    pub fn label(self) -> &'static str {
        match self {
            Component::Ffmpeg => "FFmpeg",
            Component::OcrModels => "text-reading models",
            Component::Font => "Japanese font",
            Component::WhisperModel => "Whisper speech model",
            Component::WhisperCli => "Whisper tool",
            Component::BrainEngine => "offline AI brain engine",
            Component::BrainModel => "offline AI brain model",
            Component::VoiceEngine => "VOICEVOX voice engine",
        }
    }
}

/// What still has to be fetched for a job with these settings.
pub fn missing(cfg: &Config) -> Vec<Component> {
    use crate::config::{AsrEngine, TranslationBackend, TtsBackend};
    let mut v = Vec::new();
    if !works(&cfg.ffmpeg(), "-version") || !works(&cfg.ffprobe(), "-version") {
        v.push(Component::Ffmpeg);
    }
    if cfg.ocr.enabled {
        if cfg.ocr.engine == crate::config::OcrEngineKind::Builtin && !ocr_models_present() {
            v.push(Component::OcrModels);
        }
        if cfg.target_lang().code == "ja" && crate::fonts::find_cjk_font(&cfg.overlay.font_path).is_none() {
            v.push(Component::Font);
        }
    }
    if cfg.asr.enabled && cfg.asr.engine != AsrEngine::Api {
        if !cfg.whisper_model().is_file() {
            v.push(Component::WhisperModel);
        }
        if !cfg.uses_builtin_whisper() && whisper_cli_installable() && !works(&cfg.whisper_cli(), "--help") {
            v.push(Component::WhisperCli);
        }
    }
    if (cfg.ocr.enabled || cfg.asr.enabled) && cfg.translation.backend == TranslationBackend::BuiltinBrain {
        if !crate::brain::engine_ready(cfg.translation.brain_gpu) && crate::brain::engine_installable() {
            v.push(Component::BrainEngine);
        }
        if !crate::brain::model_ready(&cfg.translation.brain_model) {
            v.push(Component::BrainModel);
        }
    }
    if cfg.asr.enabled
        && cfg.tts.backend == TtsBackend::Voicevox
        && cfg.tts.voicevox_auto_start
        && cfg.target_lang().code == "ja"
        && crate::voicevox::engine_exe().is_none()
        && crate::voicevox::installable()
        && crate::voicevox::reachable(&cfg.tts.voicevox_url).is_none()
    {
        v.push(Component::VoiceEngine);
    }
    v
}

pub fn install(c: Component, cfg: &Config, cancel: &CancelToken, progress: Progress) -> Result<()> {
    match c {
        Component::Ffmpeg => install_ffmpeg(cancel, progress),
        Component::OcrModels => install_ocr_models(cancel, progress),
        Component::Font => install_font(cancel, progress),
        Component::WhisperModel => install_whisper_model(cfg, cancel, progress),
        Component::WhisperCli => install_whisper_cli(cancel, progress),
        Component::BrainEngine => crate::brain::install_engine(cfg.translation.brain_gpu, cancel, progress),
        Component::BrainModel => crate::brain::install_model(&cfg.translation.brain_model, cancel, progress),
        Component::VoiceEngine => crate::voicevox::install(cancel, progress),
    }
}

/// Installs everything missing. Returns the components that were installed.
pub fn install_missing(cfg: &Config, cancel: &CancelToken, progress: Progress) -> Result<Vec<Component>> {
    let todo = missing(cfg);
    for c in &todo {
        cancel.check()?;
        install(*c, cfg, cancel, progress).with_context(|| format!("automatic setup of {} failed", c.label()))?;
    }
    Ok(todo)
}

// ------------------------------------------------------------------ archives

fn archive_ext(url: &str) -> &'static str {
    if url.ends_with(".tar.xz") {
        ".tar.xz"
    } else if url.ends_with(".tar.gz") {
        ".tar.gz"
    } else {
        ".zip"
    }
}

pub fn extract(archive: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let name = archive.to_string_lossy().to_string();
    if name.ends_with(".zip") {
        let f = std::fs::File::open(archive)?;
        let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).context("damaged zip download")?;
        z.extract(dest).context("could not unpack zip")?;
    } else if name.ends_with(".tar.gz") {
        let f = std::fs::File::open(archive)?;
        tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(f))).unpack(dest)?;
    } else if name.ends_with(".tar.xz") {
        // The system `tar` is much faster; fall back to the built-in decoder.
        let ok = crate::util::silent_command("tar")
            .arg("-xJf")
            .arg(archive)
            .arg("-C")
            .arg(dest)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let tar_path = archive.with_extension("");
            {
                let mut input = std::io::BufReader::new(std::fs::File::open(archive)?);
                let mut out = std::io::BufWriter::new(std::fs::File::create(&tar_path)?);
                lzma_rs::xz_decompress(&mut input, &mut out).map_err(|e| anyhow::anyhow!("xz: {e:?}"))?;
            }
            tar::Archive::new(std::fs::File::open(&tar_path)?).unpack(dest)?;
            let _ = std::fs::remove_file(&tar_path);
        }
    } else {
        bail!("unknown archive type: {name}");
    }
    Ok(())
}

pub fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false) {
                return Some(p);
            }
        }
    }
    None
}

pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t)?;
        } else {
            std::fs::copy(&p, &t)?;
        }
    }
    Ok(())
}

pub fn make_executable(_p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(_p) {
            let mut perm = m.permissions();
            perm.set_mode(perm.mode() | 0o755);
            let _ = std::fs::set_permissions(_p, perm);
        }
    }
}
