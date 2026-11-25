//! VOICEVOX — free, offline Japanese voices. BabelCraft can download the VOICEVOX engine into
//! its own folder (or use an installed VOICEVOX app) and starts it hidden when a job needs a
//! voice. Nothing is ever played through the speakers; audio only goes to files.

use crate::config::Config;
use crate::services::{self, StopOnDrop};
use crate::tools::{self, Progress};
use crate::util::{CancelToken, Cancelled, http_agent, read_body_string};
use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

pub const SERVICE: &str = "VOICEVOX engine";
pub const DOWNLOAD_SIZE: &str = "1.9 GB";

pub fn engine_dir() -> PathBuf {
    tools::tools_dir().join("voicevox")
}

fn run_name() -> &'static str {
    if cfg!(windows) { "run.exe" } else { "run" }
}

/// The engine BabelCraft downloaded itself.
pub fn bundled_engine() -> Option<PathBuf> {
    let p = engine_dir().join("engine").join(run_name());
    p.is_file().then_some(p)
}

/// The engine inside an installed VOICEVOX app.
pub fn app_engine() -> Option<PathBuf> {
    #[allow(unused_mut)] // nothing to add on Linux
    let mut v: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local).join("Programs").join("VOICEVOX");
        v.push(base.join("vv-engine").join("run.exe"));
        v.push(base.join("run.exe"));
    }
    #[cfg(target_os = "macos")]
    {
        let app = PathBuf::from("/Applications/VOICEVOX.app/Contents");
        v.push(app.join("Resources/vv-engine/run"));
        v.push(app.join("MacOS/run"));
        if let Some(home) = std::env::var_os("HOME") {
            v.push(PathBuf::from(home).join("Applications/VOICEVOX.app/Contents/Resources/vv-engine/run"));
        }
    }
    v.into_iter().find(|p| p.is_file())
}

pub fn engine_exe() -> Option<PathBuf> {
    bundled_engine().or_else(app_engine)
}

pub fn reachable(url: &str) -> Option<String> {
    let mut r = http_agent(2).get(format!("{}/version", url.trim_end_matches('/'))).call().ok()?;
    if !r.status().is_success() {
        return None;
    }
    Some(read_body_string(&mut r).unwrap_or_default().trim().trim_matches('"').to_string())
}

/// Which VOICEVOX engine download fits this computer.
fn asset_kind() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        // Windows on ARM runs the x64 engine through Windows' built-in emulation.
        ("windows", "x86_64" | "aarch64") => "windows-cpu",
        ("windows", "x86") if std::env::var("PROCESSOR_ARCHITEW6432").is_ok() => "windows-cpu",
        ("linux", "x86_64") => "linux-cpu-x64",
        ("linux", "aarch64") => "linux-cpu-arm64",
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        _ => return None,
    })
}

pub fn installable() -> bool {
    asset_kind().is_some()
}

/// Downloads the VOICEVOX engine (a zip package, sometimes split in parts) and unpacks it.
pub fn install(cancel: &CancelToken, progress: Progress) -> Result<()> {
    let kind = asset_kind().context("no ready-made VOICEVOX engine for this computer — use another voice in Settings")?;
    progress("Looking up the newest VOICEVOX engine", -1.0);
    let prefix = format!("voicevox_engine-{kind}-");
    let assets = tools::github_assets("VOICEVOX/voicevox_engine", false, |n| {
        n.starts_with(&prefix) && n[prefix.len()..].starts_with(|c: char| c.is_ascii_digit()) && (n.ends_with(".vvpp") || n.ends_with(".vvppp"))
    })?;
    // One whole package if there is one, otherwise all its parts in order.
    let mut parts: Vec<(String, String)> = match assets.iter().find(|(n, _)| n.ends_with(".vvpp")) {
        Some(single) => vec![single.clone()],
        None => assets.into_iter().filter(|(n, _)| n.ends_with(".vvppp")).collect(),
    };
    parts.sort();
    if parts.is_empty() {
        bail!("VOICEVOX engine download not found");
    }
    let tmp = tools::tools_dir().join("_download_voicevox");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let mut files = Vec::new();
    for (i, (name, url)) in parts.iter().enumerate() {
        let label = if parts.len() > 1 { format!("VOICEVOX voice engine (part {} of {})", i + 1, parts.len()) } else { "VOICEVOX voice engine".into() };
        let f = tmp.join(name);
        tools::fetch(url, &f, &label, cancel, progress)?;
        files.push(f);
    }
    let package = if files.len() == 1 {
        files.remove(0)
    } else {
        progress("Joining the VOICEVOX parts", -1.0);
        let joined = tmp.join("engine.vvpp");
        let mut out = std::io::BufWriter::new(std::fs::File::create(&joined)?);
        let mut buf = vec![0u8; 1 << 20];
        for f in &files {
            let mut r = std::fs::File::open(f)?;
            loop {
                if cancel.is_cancelled() {
                    bail!(Cancelled);
                }
                let n = r.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n])?;
            }
            drop(r);
            let _ = std::fs::remove_file(f);
        }
        out.flush()?;
        joined
    };
    progress("Unpacking VOICEVOX (about a minute)", -1.0);
    let unpack = engine_dir().join("_unpack");
    let _ = std::fs::remove_dir_all(&unpack);
    // A .vvpp file is a zip archive.
    let zip_path = package.with_extension("zip");
    std::fs::rename(&package, &zip_path)?;
    tools::extract(&zip_path, &unpack)?;
    let run = tools::find_file(&unpack, run_name()).context("the VOICEVOX package has no engine program")?;
    let folder = run.parent().unwrap_or(&unpack).to_path_buf();
    let dest = engine_dir().join("engine");
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&folder, &dest).or_else(|_| tools::copy_dir(&folder, &dest))?;
    tools::make_executable(&dest.join(run_name()));
    let _ = std::fs::remove_dir_all(&unpack);
    let _ = std::fs::remove_dir_all(&tmp);
    if bundled_engine().is_none() {
        bail!("VOICEVOX could not be set up");
    }
    Ok(())
}

pub fn remove() -> Result<()> {
    services::stop(SERVICE);
    let d = engine_dir();
    if d.is_dir() {
        std::fs::remove_dir_all(&d).with_context(|| format!("could not delete {}", d.display()))?;
    }
    Ok(())
}

/// Makes sure a VOICEVOX engine answers at the configured address, starting BabelCraft's own
/// (or the installed app's) engine when needed. The returned guard stops what was started.
pub fn ensure_running(cfg: &Config, cancel: &CancelToken, status: &mut dyn FnMut(String)) -> Result<Option<StopOnDrop>> {
    let url = cfg.tts.voicevox_url.trim().trim_end_matches('/').to_string();
    if let Some(v) = reachable(&url) {
        status(format!("VOICEVOX engine {v} is running."));
        return Ok(None);
    }
    if !cfg.tts.voicevox_auto_start {
        bail!("VOICEVOX is not running at {url} — start the VOICEVOX app, or turn on \"Start VOICEVOX automatically\" in Settings");
    }
    let exe = engine_exe().context(
        "VOICEVOX is not set up yet — press \"Set up automatically\" (downloads the free VOICEVOX engine) or install the VOICEVOX app",
    )?;
    let port = url.rsplit(':').next().and_then(|p| p.trim_matches('/').parse::<u16>().ok()).unwrap_or(50021);
    let local = url.contains("127.0.0.1") || url.contains("localhost");
    if !local {
        bail!("VOICEVOX is not reachable at {url}");
    }
    status("Starting the VOICEVOX voice engine (the first start can take a minute)…".into());
    let args: Vec<String> = vec!["--host".into(), "127.0.0.1".into(), "--port".into(), port.to_string()];
    services::start(SERVICE, &exe, &args, &format!("http://127.0.0.1:{port}"), "/version", Duration::from_secs(300), cancel)?;
    status("VOICEVOX voice engine is ready.".into());
    Ok(Some(StopOnDrop(SERVICE)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloads the VOICEVOX engine into a scratch folder, starts it hidden, writes one Japanese
    /// line to a WAV file (nothing is played) and stops it.
    /// Run with: BABELCRAFT_DATA_DIR=target/testdata cargo test voicevox_speaks -- --ignored --nocapture
    #[test]
    #[ignore]
    fn voicevox_speaks() {
        let data = std::env::var("BABELCRAFT_DATA_DIR").expect("set BABELCRAFT_DATA_DIR to a scratch folder");
        assert!(data.contains("target"), "use a scratch folder inside target/");
        let mut cfg = Config::default();
        // A port of its own, so an installed VOICEVOX app can't answer instead.
        cfg.tts.voicevox_url = "http://127.0.0.1:50121".into();
        let cancel = CancelToken::default();
        let mut progress = |what: &str, f: f32| {
            if f == 0.0 || f < 0.0 {
                println!("setup: {what}");
            }
        };
        if bundled_engine().is_none() {
            install(&cancel, &mut progress).unwrap();
        }
        assert!(bundled_engine().is_some());
        let tts = crate::tts::Tts::new(&cfg);
        let t0 = std::time::Instant::now();
        let guard = tts.prepare(&cancel, &mut |m| println!("{m}")).unwrap();
        assert!(guard.is_some(), "BabelCraft should have started its own engine");
        println!("engine ready after {:.1} s", t0.elapsed().as_secs_f32());
        let dir = PathBuf::from("target/e2e");
        std::fs::create_dir_all(&dir).unwrap();
        let out = tts.synthesize("こんにちは。BabelCraft のテストです。", &dir.join("voicevox_hello"), &cancel).unwrap();
        let secs = crate::tts::wav_duration(&out);
        println!("wrote {} ({secs:.2} s)", out.display());
        assert!(secs > 0.5);
        drop(guard);
        assert!(reachable(&cfg.tts.voicevox_url).is_none(), "the engine should be stopped again");
    }
}
