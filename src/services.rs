// SPDX-License-Identifier: GPL-3.0-or-later
//! Helper servers BabelCraft runs by itself — the offline AI brain (llama.cpp) and the
//! VOICEVOX voice engine. They start hidden (no window, no sound), BabelCraft waits until
//! they answer, and stops them again when the job is done or the app closes.

use crate::util::{Cancelled, CancelToken, http_agent, silent_command};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct Running {
    name: &'static str,
    child: Child,
    base: String,
}

static RUNNING: Mutex<Vec<Running>> = Mutex::new(Vec::new());

/// A free TCP port on this computer.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(18080)
}

/// Where a server's own messages are written (handy when it refuses to start).
pub fn log_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("babelcraft-{name}.log"))
}

fn last_lines(path: &Path, n: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Starts `program` (unless this server already runs) and waits until `base + health` answers.
/// Returns the server's base address.
pub fn start(
    name: &'static str,
    program: &Path,
    args: &[String],
    base: &str,
    health: &str,
    timeout: Duration,
    cancel: &CancelToken,
) -> Result<String> {
    {
        let mut list = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        list.retain_mut(|r| matches!(r.child.try_wait(), Ok(None)));
        if let Some(r) = list.iter().find(|r| r.name == name) {
            return Ok(r.base.clone());
        }
    }
    let log = log_file(name);
    let out = std::fs::File::create(&log).context("could not create a log file")?;
    let err = out.try_clone()?;
    let mut cmd = silent_command(program);
    cmd.args(args).stdin(Stdio::null()).stdout(out).stderr(err);
    if let Some(dir) = program.parent() {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().with_context(|| format!("could not start {}", program.display()))?;
    let agent = http_agent(3);
    let url = format!("{}{health}", base.trim_end_matches('/'));
    let t0 = Instant::now();
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            bail!(Cancelled);
        }
        if let Some(status) = child.try_wait()? {
            bail!("{name} stopped right after starting ({status}). Its last messages:\n{}", last_lines(&log, 12));
        }
        if let Ok(r) = agent.get(&url).call() {
            if r.status().is_success() {
                break;
            }
        }
        if t0.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{name} did not get ready within {} s. Its last messages:\n{}", timeout.as_secs(), last_lines(&log, 12));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    RUNNING.lock().unwrap_or_else(|e| e.into_inner()).push(Running { name, child, base: base.to_string() });
    Ok(base.to_string())
}

pub fn stop(name: &str) {
    let mut list = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    for r in list.iter_mut().filter(|r| r.name == name) {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
    list.retain(|r| r.name != name);
}

pub fn stop_all() {
    let mut list = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    for r in list.iter_mut() {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
    list.clear();
}

/// Stops a server when dropped, so it is stopped even when a job fails or is cancelled.
pub struct StopOnDrop(pub &'static str);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        stop(self.0);
    }
}
