// SPDX-License-Identifier: GPL-3.0-or-later
//! Startup checks: tells the person what is ready and what BabelCraft can set up by itself.

use crate::config::{Config, OcrEngineKind, TranslationBackend};
use crate::util::{http_agent, probe_program};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Ok,
    Warn,
    Missing,
}

#[derive(Clone, Debug)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub hint: &'static str,
    /// Which one-click fix the UI can offer.
    pub fix: Option<Fix>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fix {
    /// Download and set up every missing built-in component.
    AutoInstall,
    /// Switch translation to BabelCraft's own offline AI brain (and download it).
    UseBrain,
}

const AUTO_HINT: &str = "BabelCraft sets this up by itself: press \"Set up automatically\", or just start a job (it is fetched on first use).";

fn where_from(p: &std::path::Path) -> String {
    if p.starts_with(crate::tools::tools_dir()) {
        "set up by BabelCraft".into()
    } else if p.is_absolute() {
        p.display().to_string()
    } else {
        "system install".into()
    }
}

pub fn run_all(cfg: &Config) -> Vec<Check> {
    let mut v = Vec::new();

    // FFmpeg + FFprobe (bundled copy, downloaded copy, or system install)
    let ffmpeg = cfg.ffmpeg();
    let ffprobe = cfg.ffprobe();
    match (probe_program(&ffmpeg.to_string_lossy(), &["-version"]), probe_program(&ffprobe.to_string_lossy(), &["-version"])) {
        (Some(l), Some(_)) => v.push(ok("FFmpeg", format!("{} · {}", short(&l), where_from(&ffmpeg)))),
        _ => v.push(Check {
            name: "FFmpeg",
            status: Status::Missing,
            detail: "not set up yet (needed to read and write video)".into(),
            hint: AUTO_HINT,
            fix: Some(Fix::AutoInstall),
        }),
    }

    // OCR
    if cfg.ocr.enabled {
        match cfg.ocr.engine {
            OcrEngineKind::Builtin => {
                if crate::tools::ocr_models_present() {
                    v.push(ok("Text reader (OCR)", "built into BabelCraft · models ready".into()));
                } else {
                    v.push(Check {
                        name: "Text reader (OCR)",
                        status: Status::Missing,
                        detail: "built-in OCR models not downloaded yet (~12 MB)".into(),
                        hint: AUTO_HINT,
                        fix: Some(Fix::AutoInstall),
                    });
                }
            }
            OcrEngineKind::Tesseract => {
                let t = cfg.tesseract();
                match probe_program(&t.to_string_lossy(), &["--version"]) {
                    Some(l) => {
                        let langs = probe_langs(&t.to_string_lossy());
                        let status = if langs.is_empty() || langs.iter().any(|x| x == &cfg.ocr.language) { Status::Ok } else { Status::Warn };
                        v.push(Check {
                            name: "Text reader (OCR)",
                            status,
                            detail: format!("Tesseract {}", short(&l)),
                            hint: "Install the English data (tesseract-ocr-eng), or switch to the Built-in engine.",
                            fix: None,
                        });
                    }
                    None => v.push(Check {
                        name: "Text reader (OCR)",
                        status: Status::Missing,
                        detail: "Tesseract not found".into(),
                        hint: "Switch Settings → On-screen text → engine to \"Built-in\" (nothing to install), or install Tesseract.",
                        fix: None,
                    }),
                }
            }
        }
        match crate::fonts::find_cjk_font(&cfg.overlay.font_path) {
            Some(f) => v.push(ok("Japanese font", f.path.display().to_string())),
            None => v.push(Check {
                name: "Japanese font",
                status: Status::Missing,
                detail: "No font with Japanese characters found".into(),
                hint: AUTO_HINT,
                fix: Some(Fix::AutoInstall),
            }),
        }
    }

    // Whisper
    if cfg.asr.enabled && cfg.asr.engine == crate::config::AsrEngine::Api {
        let svc = crate::ai::speech_service(&cfg.asr.api_service);
        if !svc.local && cfg.speech_key().is_empty() {
            v.push(Check {
                name: "Whisper engine",
                status: Status::Missing,
                detail: format!("{} · no API key yet", svc.name),
                hint: "Paste the service's API key in Settings → Speech (or on the AI models & downloads page), or switch back to whisper-cli (free, offline).",
                fix: None,
            });
        } else {
            v.push(ok("Whisper engine", format!("{} · {}", svc.name, cfg.asr.api_model)));
        }
    }
    if cfg.asr.enabled && cfg.asr.engine != crate::config::AsrEngine::Api {
        let model = cfg.whisper_model();
        if model.is_file() {
            v.push(ok("Whisper model", model.display().to_string()));
        } else {
            v.push(Check {
                name: "Whisper model",
                status: Status::Missing,
                detail: "speech model not downloaded yet (base.en, ~142 MB)".into(),
                hint: AUTO_HINT,
                fix: Some(Fix::AutoInstall),
            });
        }
        if cfg.uses_builtin_whisper() {
            v.push(ok("Whisper engine", "built into BabelCraft".into()));
        } else {
            let cli = cfg.whisper_cli();
            match probe_program(&cli.to_string_lossy(), &["--help"]) {
                Some(_) => v.push(ok("Whisper engine", format!("whisper.cpp · {}", where_from(&cli)))),
                None if crate::tools::whisper_cli_installable() => v.push(Check {
                    name: "Whisper engine",
                    status: Status::Missing,
                    detail: "whisper.cpp not set up yet".into(),
                    hint: AUTO_HINT,
                    fix: Some(Fix::AutoInstall),
                }),
                None => v.push(Check {
                    name: "Whisper engine",
                    status: Status::Missing,
                    detail: "whisper.cpp not found".into(),
                    hint: "Use the BabelCraft download with Whisper built in (\"-metal\" on Mac), or `brew install whisper-cpp`.",
                    fix: None,
                }),
            }
        }
    }

    if cfg.asr.enabled {
        let (good, detail) = crate::tts::check_backend(cfg);
        let downloadable = cfg.tts.backend == crate::config::TtsBackend::Voicevox
            && cfg.tts.voicevox_auto_start
            && crate::voicevox::engine_exe().is_none()
            && crate::voicevox::installable();
        let hint = match cfg.tts.backend {
            crate::config::TtsBackend::Voicevox if downloadable => {
                "BabelCraft downloads the free VOICEVOX engine by itself (press \"Set up automatically\" or just start a job). When you publish a video, credit the voice, e.g. \"VOICEVOX:ずんだもん\"."
            }
            crate::config::TtsBackend::System => {
                "Windows: Settings → Time & language → Speech → Add voices → Japanese. Mac: System Settings → Accessibility → Spoken Content → System voice → Manage voices → Japanese. Or choose VOICEVOX (free, downloaded automatically)."
            }
            _ => "Pick VOICEVOX in Settings → Japanese voice (free, downloaded automatically), or start the voice app/server you chose.",
        };
        v.push(Check {
            name: "Japanese voice",
            status: if good { Status::Ok } else { Status::Missing },
            detail,
            hint,
            fix: if downloadable { Some(Fix::AutoInstall) } else { None },
        });
    }

    // Translation
    if cfg.ocr.enabled || cfg.asr.enabled {
        v.push(check_translation(cfg));
    }
    v
}

fn check_translation(cfg: &Config) -> Check {
    let t = &cfg.translation;
    let provider = crate::ai::provider(&t.provider);
    let name = provider.map(|p| p.name).unwrap_or("translation service");
    match t.backend {
        TranslationBackend::BuiltinBrain => {
            let m = crate::brain::model(&t.brain_model);
            let engine = crate::brain::engine_ready(t.brain_gpu);
            let model = crate::brain::model_ready(m.id);
            if engine && model {
                ok("Translator", format!("offline AI brain · {} · runs on this computer", m.name))
            } else if !crate::brain::engine_installable() && !engine {
                Check {
                    name: "Translator",
                    status: Status::Missing,
                    detail: "no ready-made AI brain engine for this computer".into(),
                    hint: "Install Ollama or LM Studio (see AI models & downloads), or use an online service in Settings → Translation.",
                    fix: None,
                }
            } else {
                Check {
                    name: "Translator",
                    status: Status::Missing,
                    detail: format!("offline AI brain not downloaded yet ({} · {})", m.name, m.size),
                    hint: AUTO_HINT,
                    fix: Some(Fix::AutoInstall),
                }
            }
        }
        TranslationBackend::Anthropic => {
            if cfg.key_for("anthropic").is_empty() {
                Check {
                    name: "Translator",
                    status: Status::Missing,
                    detail: "Anthropic Claude · no API key yet".into(),
                    hint: "Paste your Anthropic API key in Settings → Translation, or use the free offline AI brain.",
                    fix: Some(Fix::UseBrain),
                }
            } else {
                ok("Translator", format!("Anthropic Claude · {}", t.openai_model))
            }
        }
        TranslationBackend::LibreTranslate => {
            let url = format!("{}/languages", t.libre_url.trim_end_matches('/'));
            match http_agent(3).get(&url).call() {
                Ok(r) if r.status().is_success() => ok("Translator", format!("LibreTranslate · {}", t.libre_url)),
                _ => Check {
                    name: "Translator",
                    status: Status::Missing,
                    detail: format!("LibreTranslate not reachable at {}", t.libre_url),
                    hint: "Start your LibreTranslate server, or use the free offline AI brain.",
                    fix: Some(Fix::UseBrain),
                },
            }
        }
        TranslationBackend::Ollama => {
            let url = format!("{}/api/tags", t.ollama_url.trim_end_matches('/'));
            match http_agent(3).get(&url).call() {
                Ok(mut r) if r.status().is_success() => {
                    let body = crate::util::read_body_string(&mut r).unwrap_or_default();
                    let has = body.contains(&format!("\"{}\"", t.ollama_model))
                        || body.contains(&format!("\"{}:latest\"", t.ollama_model));
                    if has {
                        // Very large or programming-only models translate slowly and can stop Ollama.
                        let warnings = crate::translate::preflight(cfg).unwrap_or_default();
                        if let Some(w) = warnings.first() {
                            Check {
                                name: "Translator",
                                status: Status::Warn,
                                detail: format!("Ollama · {} — {w}", t.ollama_model),
                                hint: "Pick a 4–14B general model in Settings → Translation (press List), or use the built-in offline AI brain.",
                                fix: Some(Fix::UseBrain),
                            }
                        } else {
                            ok("Translator", format!("Ollama · {}", t.ollama_model))
                        }
                    } else {
                        let installed: Vec<String> = serde_json::from_str::<serde_json::Value>(&body)
                            .ok()
                            .and_then(|v| v["models"].as_array().cloned())
                            .unwrap_or_default()
                            .iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect();
                        let detail = if installed.is_empty() {
                            format!("Ollama is running but has no models; '{}' is not downloaded", t.ollama_model)
                        } else {
                            let list: Vec<&str> = installed.iter().map(|s| s.as_str()).take(6).collect();
                            format!("Ollama is running, but '{}' is not one of its models: {}", t.ollama_model, list.join(", "))
                        };
                        Check {
                            name: "Translator",
                            status: Status::Warn,
                            detail,
                            hint: "Pick one of your Ollama models in Settings → Translation (press List), or download one on the AI models & downloads page.",
                            fix: None,
                        }
                    }
                }
                _ => Check {
                    name: "Translator",
                    status: Status::Missing,
                    detail: format!("Ollama not reachable at {}", t.ollama_url),
                    hint: "Install Ollama (ollama.com) and run `ollama pull qwen2.5:7b` — or press the button to use BabelCraft's own offline AI brain instead (no install).",
                    fix: Some(Fix::UseBrain),
                },
            }
        }
        TranslationBackend::OpenAiCompatible => {
            let local = provider.map(|p| p.local).unwrap_or(false) || t.openai_url.contains("localhost") || t.openai_url.contains("127.0.0.1");
            let needs_key = provider.map(|p| p.needs_key).unwrap_or(false);
            if local {
                let url = format!("{}/models", t.openai_url.trim_end_matches('/'));
                match http_agent(3).get(&url).call() {
                    Ok(r) if r.status().is_success() => ok("Translator", format!("{name} · {}", if t.openai_model.is_empty() { "model loaded in the app" } else { t.openai_model.as_str() })),
                    _ => Check {
                        name: "Translator",
                        status: Status::Missing,
                        detail: format!("{name} not reachable at {}", t.openai_url),
                        hint: "Open the app and start its local server (LM Studio: Developer → Start server; Jan: Settings → Local API Server) — or use the built-in offline AI brain.",
                        fix: Some(Fix::UseBrain),
                    },
                }
            } else if needs_key && cfg.chat_key().is_empty() {
                Check {
                    name: "Translator",
                    status: Status::Missing,
                    detail: format!("{name} · no API key yet"),
                    hint: "Paste your API key in Settings → Translation (or on the AI models & downloads page), or use the free offline AI brain.",
                    fix: Some(Fix::UseBrain),
                }
            } else {
                ok("Translator", format!("{name} · {}", t.openai_model))
            }
        }
        TranslationBackend::DeepL => {
            if cfg.key_for("deepl").is_empty() {
                Check {
                    name: "Translator",
                    status: Status::Missing,
                    detail: "No DeepL key set".into(),
                    hint: "Enter your DeepL API key in Settings → Translation (or set DEEPL_AUTH_KEY).",
                    fix: None,
                }
            } else {
                ok("Translator", "DeepL API key set".into())
            }
        }
    }
}

fn probe_langs(tesseract: &str) -> Vec<String> {
    let out = crate::util::silent_command(tesseract).arg("--list-langs").stdin(std::process::Stdio::null()).output();
    match out {
        Ok(o) => {
            let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            text.lines().skip(1).map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
        }
        Err(_) => Vec::new(),
    }
}

fn ok(name: &'static str, detail: String) -> Check {
    Check { name, status: Status::Ok, detail, hint: "", fix: None }
}

fn short(line: &str) -> String {
    line.chars().take(70).collect()
}
