<p align="center">
  <img src="assets/icon-256.png" width="160" alt="BabelCraft icon">
</p>

<h1 align="center">BabelCraft</h1>
<p align="center"><b>English → 日本語 video translator</b><br>
Translates a video's on-screen English text <i>and</i> spoken English into Japanese, then renders a finished, dubbed video file — silently, in the background.</p>

<p align="center">Windows 10/11 (x64 · x86 · ARM64) · Linux (x86_64 · aarch64, any distro) · macOS 11+ (Apple Silicon + Intel)<br>
Interface in English or 日本語 · works fully offline, or with the online AI service of your choice</p>

---

## What it does

| Stage | What happens | Engine |
|---|---|---|
| 1 · Extracting Media | Reads the video, pulls 16 kHz mono audio for Whisper and 48 kHz audio for mixing | FFmpeg / FFprobe |
| 2 · Scanning Screen Text (OCR) | Samples frames every *N* ms (streamed straight from FFmpeg), skips frames that haven't changed, finds English text boxes | built-in neural OCR (ocrs) · or Tesseract |
| | Merges identical text across frames into `(start, end, bbox, text)` intervals so a static title/slide is translated **once** | built-in |
| 3 · Transcribing Speech (Whisper) | English speech → timestamped segments | whisper.cpp (built in via `whisper-rs`, or `whisper-cli`) · or an online Whisper API |
| 4 · Translating to Japanese | Batched, cached, context-aware translation | **built-in offline AI brain** · Ollama · LM Studio · Jan · llama.cpp · LibreTranslate · OpenAI · Claude · Gemini · DeepL · OpenRouter · Groq · DeepSeek · Mistral · any OpenAI-compatible server |
| 5 · Synthesizing Japanese Voice | One clip per line; long lines are sped up (≤ your limit) and nudged so lines never overlap | **VOICEVOX (downloaded and started automatically)** · OpenAI voices · Kokoro · the computer's own Japanese voice · edge-tts · Piper |
| 6 · Rendering Final Video | Japanese text boxes (semi-transparent, auto-fitted, CJK font) are composited over the English text; the Japanese soundtrack is mixed (replace, or duck the original to e.g. 15 %) and muxed | FFmpeg (x264 + AAC) |

Also written next to the video: `yourvideo.ja.srt` (Japanese subtitles of the speech).

The app never plays audio or shows a preview, so you can minimise it and keep working. **Cancel** stops every helper process gracefully (FFmpeg gets `q`, then is killed after 1.5 s) and removes the partial file.

## Quick start

1. Download the file for your system from **Releases**:
   - Windows: `BabelCraft-x.y.z-windows-x64.zip` (or `x86` / `arm64`) → unzip → `BabelCraft.exe`
     (Windows on ARM: use `arm64`; Windows 11 on ARM also runs the `x64` build, Windows 10 on ARM the `x86` build)
   - Linux: `.AppImage` (any distro: `chmod +x` and run), `.deb` (Debian/Ubuntu/Mint/Pop!_OS), or `.tar.gz` (`./install.sh` installs for your user)
   - macOS: `BabelCraft-x.y.z-macos-universal.dmg` → drag to Applications → first launch: right-click → **Open**
   - `-whisper` / `-metal` variants have Whisper built in (`metal` = GPU on Apple Silicon).
2. Start BabelCraft. Press **Set up automatically** in *Setup checks* — or just start a job: everything missing is downloaded into BabelCraft's own folder on first use (nothing is installed into your system).
   With the offline AI brain and VOICEVOX this first download is almost 5 GB, so the first job can spend a while on *One-time setup*: the progress screen shows each download's size, speed and time left. If you cancel or close the app, what has downloaded is kept and the next job carries on from there.
3. Pick an English video (or drop it on the window), check the output name, press **Start translating**.

Switch the interface between **English** and **日本語** with the buttons at the top right.

## Everything it uses — and where it comes from

The **⬇ AI models & downloads** page inside the app shows each item, whether it is already on your computer, and a **Download** button. All of these are fetched automatically when a job needs them:

| Item | Size | Source |
|---|---|---|
| FFmpeg + FFprobe (portable) | ~100 MB | [BtbN FFmpeg builds](https://github.com/BtbN/FFmpeg-Builds/releases) (Windows/Linux) · [martin-riedl.de](https://ffmpeg.martin-riedl.de/) (macOS) · [ffmpeg.org](https://ffmpeg.org/download.html) |
| Text-reading models (OCR) | 12 MB | [ocrs](https://github.com/robertknight/ocrs) |
| Japanese font Noto Sans JP | 5 MB | [Google Noto CJK](https://github.com/notofonts/noto-cjk) ([fonts.google.com](https://fonts.google.com/noto/specimen/Noto+Sans+JP)) |
| Whisper speech tool | ~10 MB | [whisper.cpp releases](https://github.com/ggml-org/whisper.cpp/releases) (Windows x64/x86/ARM64, Linux x86_64/aarch64; macOS: use the `-metal` build or `brew install whisper-cpp`) |
| Whisper speech model | 75 MB – 1.6 GB | [ggerganov/whisper.cpp on Hugging Face](https://huggingface.co/ggerganov/whisper.cpp) — tiny.en, **base.en** (default), small.en, medium.en, large-v3-turbo |
| Offline AI brain engine | ~25 MB | [llama.cpp](https://github.com/ggml-org/llama.cpp) `llama-server` (CPU or Vulkan GPU build; Metal on Mac) |
| Offline AI brain model | 1.0 – 4.7 GB | see the table below |
| VOICEVOX voice engine | 1.9 GB | [VOICEVOX engine releases](https://github.com/VOICEVOX/voicevox_engine/releases) — [voicevox.hiroshiba.jp](https://voicevox.hiroshiba.jp/) |

### The offline AI brain

Translation can run entirely on your own computer: no account, no API key, nothing sent online. BabelCraft downloads llama.cpp's server and the model you choose, starts it hidden only while translating, and closes it afterwards.

| Model | Download | Licence | Best for |
|---|---|---|---|
| [Qwen 2.5 · 1.5B Instruct](https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct) | 1.0 GB | Apache-2.0 | older computers, 4 GB memory |
| [Qwen 3 · 4B Instruct 2507](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507) — **default** | 2.5 GB | Apache-2.0 | good natural Japanese on most computers (8 GB) |
| [Gemma 3 · 4B](https://huggingface.co/google/gemma-3-4b-it) | 2.5 GB | Gemma Terms of Use | an alternative style |
| [Qwen 2.5 · 7B Instruct](https://huggingface.co/Qwen/Qwen2.5-7B-Instruct) | 4.7 GB | Apache-2.0 | best quality (16 GB memory or a graphics card) |

Files come from [unsloth](https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF) and [bartowski](https://huggingface.co/bartowski) GGUF conversions (Q4_K_M). Tick **Use the graphics card** in Settings → Translation for the Vulkan build on Windows/Linux (Macs always use Metal).

### Other free apps BabelCraft can use (optional)

| App | Used for | Get it |
|---|---|---|
| Ollama | translation (many open models; BabelCraft can download models into it) | https://ollama.com/download |
| LM Studio | translation (start its local server) | https://lmstudio.ai/download |
| Jan | translation (Local API Server) | https://jan.ai/download |
| llama.cpp server | translation | https://github.com/ggml-org/llama.cpp/releases |
| LibreTranslate | translation (self-hosted) | https://github.com/LibreTranslate/LibreTranslate |
| VOICEVOX app | Japanese voice (BabelCraft starts its engine automatically if installed) | https://voicevox.hiroshiba.jp/ |
| Kokoro-FastAPI | Japanese voice (OpenAI-compatible, voice `jf_alpha`) | https://github.com/remsky/Kokoro-FastAPI |
| Piper | Japanese voice (needs a Japanese voice model) | https://github.com/rhasspy/piper |
| edge-tts | Japanese voice (online, no key) | https://github.com/rany2/edge-tts |
| Speaches | speech recognition (local Whisper server) | https://github.com/speaches-ai/speaches |
| Tesseract | alternative on-screen text reader | https://github.com/tesseract-ocr/tesseract |

**Computer's own Japanese voice:** Windows — Settings → Time & language → Speech → Add voices → Japanese. macOS — System Settings → Accessibility → Spoken Content → System voice → Manage voices → Japanese (Kyoko/Otoya).

### Online AI services (need an account and an API key)

Paste keys on the **AI models & downloads** page or in Settings. One key serves every part that uses that service.

| Service | Used for | Get a key | Environment variable |
|---|---|---|---|
| OpenAI | translation · speech recognition (whisper-1) · voice (gpt-4o-mini-tts) | https://platform.openai.com/api-keys | `OPENAI_API_KEY` |
| Anthropic Claude | translation | https://console.anthropic.com/settings/keys | `ANTHROPIC_API_KEY` |
| Google Gemini | translation | https://aistudio.google.com/apikey | `GEMINI_API_KEY` |
| DeepL | translation | https://www.deepl.com/pro-api | `DEEPL_AUTH_KEY` |
| OpenRouter | translation (hundreds of models) | https://openrouter.ai/keys | `OPENROUTER_API_KEY` |
| Groq | translation · very fast speech recognition | https://console.groq.com/keys | `GROQ_API_KEY` |
| DeepSeek | translation | https://platform.deepseek.com/api_keys | `DEEPSEEK_API_KEY` |
| Mistral AI | translation | https://console.mistral.ai/api-keys | `MISTRAL_API_KEY` |

Press **List** next to the model name to see which models your account or local app offers. Any other OpenAI-compatible server (vLLM, LocalAI, a company endpoint…) works with *Other OpenAI-compatible server*.

**Voice credits:** VOICEVOX voices are free, including for commercial use, but each character's terms ask you to credit the voice when you publish, e.g. `VOICEVOX:ずんだもん` — see https://voicevox.hiroshiba.jp/term/.

**Why the font matters:** most system fallback fonts on Linux/Windows draw Japanese as `□□□` inside video overlays. BabelCraft only accepts a font that really contains kana and kanji: your chosen font → downloaded Noto Sans JP → system Japanese fonts (Yu Gothic, Meiryo, Hiragino, Noto Sans CJK, IPAex, Takao, VL Gothic, `fc-match :lang=ja`). If none is found the job stops before doing any work and tells you how to fix it.

## Settings

Saved automatically as TOML:

- Windows `%APPDATA%\BabelCraft\babelcraft\config\config.toml`
- macOS `~/Library/Application Support/org.BabelCraft.babelcraft/config.toml`
- Linux `~/.config/babelcraft/config.toml`

Downloaded tools and models go to the matching data folder (`.../data/tools`, `.../data/models`, `.../data/fonts`).
**Portable mode:** create a folder named `babelcraft-data` next to the program (or set `BABELCRAFT_DATA_DIR`) and everything is stored there instead, settings included — USB-stick friendly. Copies of FFmpeg/whisper-cli placed next to `BabelCraft.exe` (or in a `tools/` subfolder) are used first.

**Log file:** every job is written to `.../data/logs/babelcraft.log` (Windows: `%APPDATA%\BabelCraft\babelcraft\data\logs\babelcraft.log`); if BabelCraft ever hits an internal error, the details are written there before it closes.

**Picking up where a job stopped:** the on-screen text, the transcription and every line already translated are kept in `.../data/cache` for 30 days, so running the same video again (after fixing a translator setting, say) skips the slow steps and never pays twice for the same online translation.

API keys typed into BabelCraft are stored in plain text in the settings file on your computer only.

Key options: translation service + model + style/glossary hint · audio mode *Replace* or *Duck* (level %, whole-track or only under speech, keep English as a 2nd track) · max speed-up for long lines · OCR sampling interval, "only when the picture changes" threshold, confidence, minimum on-screen time, smallest text size (default 3 % of the picture height — leaves out menus and interface labels in screen recordings) · overlay box opacity / font · Whisper engine, model, GPU, threads, or an online Whisper service · voice engine and voice · x264 quality (CRF) and speed · interface language.

## When a job stops

BabelCraft checks the translator before it starts, so most problems are reported at once with what to do:

- *"Claude / OpenAI / … needs an API key"* — paste the key in **Settings → Translation** or on **AI models & downloads**, or choose the built-in offline AI brain (free, no key).
- *"Ollama isn't running" / "Ollama has no model called …"* — start the Ollama app, or press **List** next to the model and pick one you have.
- *"… is very large"* — models over ~24 GB (or ones made for programming) translate slowly and can make Ollama's engine stop when memory runs out; a 4–14B general model, or the offline brain, works better.
- Anything else: the log file above says what happened. Running the job again carries on from where it stopped.

While BabelCraft is minimised the job keeps running and the taskbar title shows its progress.

## Build from source

Requires the latest stable Rust (2024 edition; developed with 1.99): https://rustup.rs

```bash
git clone https://github.com/Arrestis/Babel.git
cd Babel
cargo build --release                      # uses whisper-cli for speech
cargo build --release --features whisper   # Whisper built in (needs CMake + C/C++ compiler + libclang)
```

GPU Whisper: `--features vulkan` (Windows/Linux, Vulkan SDK), `--features cuda` (NVIDIA CUDA toolkit), `--features metal` (macOS). CPU is always the fallback.

### Windows 10 / 11
- Install **Visual Studio Build Tools** (workload "Desktop development with C++"; add the *x86* and *ARM64* build tools for those targets).
- 64-bit: `cargo build --release`
- 32-bit: `rustup target add i686-pc-windows-msvc` then `cargo build --release --target i686-pc-windows-msvc`
- ARM64: `rustup target add aarch64-pc-windows-msvc` then `cargo build --release --target aarch64-pc-windows-msvc`
- Windows builds use Windows' own secure-connection support (SChannel), so no extra C compiler is needed for any of the three.
- For `--features whisper`: `winget install Kitware.CMake LLVM.LLVM`
- The icon is embedded into the `.exe` automatically (`build.rs`).
- 32-bit BabelCraft on 64-bit Windows uses the 64-bit FFmpeg, AI brain and VOICEVOX downloads; on a true 32-bit Windows install FFmpeg yourself and use an online or LAN AI service.

### Linux (any distro)
```bash
# Debian/Ubuntu
sudo apt install build-essential pkg-config libxkbcommon-dev libgl1-mesa-dev libx11-dev libwayland-dev libgtk-3-dev
# Fedora
sudo dnf install gcc pkgconf libxkbcommon-devel mesa-libGL-devel libX11-devel wayland-devel gtk3-devel
# Arch
sudo pacman -S base-devel libxkbcommon mesa libx11 wayland gtk3
cargo build --release
bash packaging/linux/install.sh            # installs to ~/.local with menu entry + icon
bash packaging/linux/make-appimage.sh      # optional portable AppImage in dist/
cargo install cargo-deb && cargo deb       # optional .deb
```
Works on X11 and Wayland. Add `cmake clang` for `--features whisper`.

### macOS 11+
```bash
xcode-select --install
bash packaging/macos/make-app.sh                           # universal BabelCraft.app + .dmg in dist/
bash packaging/macos/make-app.sh --features whisper,metal  # with GPU Whisper (brew install cmake)
```

### Command line
```
babelcraft                          open the app
babelcraft --lang ja                open with the Japanese interface (en = English)
babelcraft --page downloads         open on the AI models & downloads page
babelcraft --screenshot out.png     open without taking focus, render ~120 frames, save a PNG of the window, exit
babelcraft --version
```

## Publishing on GitHub

1. Put these files in your repository folder (e.g. a clone of `https://github.com/Arrestis/Babel`), then:
   ```bash
   git add .
   git commit -m "BabelCraft 1.1.2"
   git push
   ```
2. **Actions** builds every platform on each push (`.github/workflows/release.yml`).
3. To publish downloads: `git tag v1.1.2 && git push origin v1.1.2` → a Release is created with:
   Windows x64 / x86 / ARM64 / x64-whisper `.zip`, Linux x86_64 / aarch64 `.tar.gz` + `.deb` + `.AppImage` (+ x86_64-whisper), macOS universal `.dmg` (+ `-metal`).

## Project layout

```
Cargo.toml              crate `babelcraft` (Rust 2024), features: whisper, vulkan, cuda, metal
build.rs                embeds the Windows icon/version info
assets/                 icon (source picture, 256/1024 PNG, .ico)
examples/make_icons.rs  regenerates the icons from assets/icon-source.png
src/main.rs             window setup, icon, command line, --screenshot
src/ui.rs               egui interface: splash, files, settings, progress, setup checks, AI models & downloads page, log
src/i18n.rs             English / 日本語 interface text
src/pipeline.rs         background job orchestration, progress, cancel, cleanup, SRT
src/video.rs            FFprobe/FFmpeg: probe, audio extraction, frame streaming, final render
src/ocr.rs              built-in OCR + Tesseract, change detection, interval de-duplication
src/overlay.rs          Japanese text boxes (auto-fit, wrapping rules, never clipped) + overlay stream
src/fonts.rs            cross-platform Japanese font detection
src/asr.rs              Whisper (whisper-rs, whisper-cli, or an online Whisper API)
src/translate.rs        offline AI brain / Ollama / OpenAI-compatible / Claude / DeepL / LibreTranslate, batching + cache
src/ai.rs               catalogue of local and online AI services, model lists, Ollama downloads
src/brain.rs            the offline AI brain (llama.cpp + open models)
src/voicevox.rs         VOICEVOX engine download and automatic start
src/services.rs         starts/stops helper servers hidden in the background
src/tts.rs              VOICEVOX / OpenAI-compatible / system voice / edge-tts / Piper, timing + mixing/ducking
src/tools.rs            automatic downloads (FFmpeg, whisper.cpp, models, font)
src/config.rs           persisted settings (serde + TOML)
src/checks.rs           startup checks with one-click fixes
src/util.rs             silent child processes, cancel token, resumable downloads, messages
src/logfile.rs          the log file and the internal-error recorder
packaging/              Linux .desktop/AppImage/install, macOS Info.plist/.app/.dmg
```

Run the tests with `cargo test`. The download tests are opt-in (they fetch gigabytes into a scratch folder):
`BABELCRAFT_DATA_DIR=target/testdata cargo test -- --ignored --nocapture`.

## Notes & limits

- *Replace* mode removes the whole original soundtrack (music included); use *Duck* to keep background sound under the Japanese voice.
- On-screen text: big, high-contrast text (titles, slides, captions) works best; stylised or moving text may be missed. Text the translator keeps in English (names, web addresses, codes) is left as it is, without a box.
- Speech is translated and voiced sentence by sentence (Whisper's pieces are joined into whole sentences first).
- Translation and voice quality depend on the model and voice you pick. The 4B/7B offline models, Claude, GPT, Gemini or DeepL give natural Japanese.
- With an online service, the text (and for online speech recognition, the audio) is sent to that service.
- Third-party licences: Noto Sans JP — SIL Open Font License; Whisper models — MIT; llama.cpp — MIT; models — see the table above; VOICEVOX — see its terms; FFmpeg builds — GPL.

## License

MIT — see [LICENSE](LICENSE).
