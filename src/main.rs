//! BabelCraft — translate English videos (on-screen text + speech) into Japanese.
//!
//! Usage:
//!   babelcraft                       open the app
//!   babelcraft --screenshot out.png  open without taking focus, draw ~120 frames,
//!                                    save the window picture and exit

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod ai;
mod asr;
mod brain;
mod checks;
mod config;
mod fonts;
mod i18n;
mod logfile;
mod ocr;
mod overlay;
mod pipeline;
mod services;
mod tools;
mod translate;
mod tts;
mod ui;
mod util;
mod video;
mod voicevox;

use std::path::PathBuf;

pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

fn main() -> eframe::Result<()> {
    // Internal errors are written to the log file before the app closes.
    logfile::install_panic_hook();
    let mut screenshot: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            // Interface language for this session: --lang ja | en
            "--lang" => {
                if let Some(l) = args.next() {
                    // SAFETY: set before any other thread is started.
                    unsafe { std::env::set_var("BABELCRAFT_LANG", l) };
                }
            }
            // Page to open on: --page translate | downloads
            "--page" => {
                if let Some(p) = args.next() {
                    // SAFETY: set before any other thread is started.
                    unsafe { std::env::set_var("BABELCRAFT_PAGE", p) };
                }
            }
            "--version" | "-V" => {
                println!("babelcraft {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "BabelCraft {} — English → Japanese video translator\n\n\
                     Usage: babelcraft [--lang en|ja] [--page translate|downloads] [--screenshot <file.png>]",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            _ => {}
        }
    }

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("BabelCraft")
        .with_app_id("babelcraft")
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([900.0, 600.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }

    let mut options = eframe::NativeOptions { viewport, ..Default::default() };

    let test_run = std::env::var("BABELCRAFT_TEST_AUTOSTART").is_ok_and(|v| v == "1");
    if screenshot.is_some() || test_run {
        // Never steal focus from what the person is doing.
        options.viewport = options.viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            options.viewport = options.viewport.with_window_level(eframe::egui::WindowLevel::AlwaysOnBottom);
        }
        #[cfg(target_os = "macos")]
        {
            options.event_loop_builder = Some(Box::new(|builder| {
                use winit::platform::macos::EventLoopBuilderExtMacOS;
                builder.with_activate_ignoring_other_apps(false);
            }));
        }
        // Fresh state so the picture is reproducible.
        options.persist_window = false;
    }

    eframe::run_native(
        "BabelCraft",
        options,
        Box::new(move |cc| Ok(Box::new(ui::BabelApp::new(cc, screenshot)))),
    )
}
