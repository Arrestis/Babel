// SPDX-License-Identifier: GPL-3.0-or-later
//
// BabelCraft — English to Japanese video translator.
// Copyright (C) 2026 BabelCraft contributors
//
// This program is free software: you can redistribute it and/or modify it under the terms of
// the GNU General Public License as published by the Free Software Foundation, either version 3
// of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
// without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with this program
// (the LICENSE file). If not, see <https://www.gnu.org/licenses/>.

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
mod interface;
mod langs;
mod logfile;
mod ocr;
mod overlay;
mod piper;
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
            // Interface language for this session: --lang <code or name> (en, ja, es, zh-Hans, Korean…)
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
                     Usage: babelcraft [--lang <language code, e.g. en, ja, es, zh-Hans>] [--page translate|downloads] [--screenshot <file.png>]\n\n\
                     Copyright (C) 2026 BabelCraft contributors. Free software under the GNU GPL v3 or later\n\
                     <https://www.gnu.org/licenses/gpl-3.0.html>; there is NO WARRANTY.\n\
                     Source code: https://github.com/Arrestis/Babel",
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
    let mut scratch_state: Option<PathBuf> = None;
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
        // Fresh state so the picture is reproducible: a throwaway file for the window layout
        // (panel sizes, scroll positions), so the person's own layout is neither read nor changed.
        options.persist_window = false;
        let state = std::env::temp_dir().join(format!("babelcraft-ui-state-{}.ron", std::process::id()));
        let _ = std::fs::remove_file(&state);
        options.persistence_path = Some(state.clone());
        scratch_state = Some(state);
    }

    let result = eframe::run_native(
        "BabelCraft",
        options,
        Box::new(move |cc| Ok(Box::new(ui::BabelApp::new(cc, screenshot)))),
    );
    if let Some(state) = scratch_state {
        let _ = std::fs::remove_file(state);
    }
    result
}
