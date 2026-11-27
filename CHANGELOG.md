# Changelog

## 1.3.0

### New
- **Translate between any two of 115 languages, both ways** — English → Japanese is still the default, but the video language and the language to translate into are now chosen on the main page (with a swap button): Japanese → English, Spanish → Korean, Arabic → Hindi… Speech is recognised in the video's language (Whisper, multilingual model downloaded automatically), translated, and dubbed.
- **Dubbing in other languages** uses Piper: BabelCraft downloads the Piper program and one natural voice for the language once, and runs it hidden (VOICEVOX is still used for Japanese).
- **On-screen text** in the target language is drawn with properly shaped letters (joined Arabic, Hindi, Thai, Chinese, Korean…), with a matching font found or downloaded. Subtitle files and the dub's audio tag use the target language (ideo.es.srt).
- Text in non-English videos is read with Tesseract (install it with the language's data); English still uses the built-in reader.

## 1.2.0

### New
- **BabelCraft's buttons and menus in any language.** The two language buttons are replaced by one **speaking-mouth button** (a mouth with sound waves) at the top right. It opens a list of 115 languages, each shown in its own writing, with a search box. English stays the default.
- **Translated by hand and built in:** English, 日本語, Español, Français, Deutsch, Português, Italiano, Русский, 简体中文 and 한국어 switch at once.
- **Every other language** is translated by BabelCraft's AI the first time you choose it, using the translator chosen in *Settings → Translation* (the offline AI brain unless you picked another). The menu shows the progress, the result is saved, and from then on the switch is instant. Right-click → **Translate again** redoes it.
- **Fonts for every writing system:** Chinese, Korean, Hindi and other Indian scripts, Arabic, Hebrew, Thai, Amharic, Georgian, Armenian… use the computer's own font, or a free Noto font is downloaded when there is none.
- `--lang <code>` accepts any language code or name (for example `--lang fr`, `--lang zh-Hans`).

### Improved
- *Reset to defaults* keeps the interface language.
- The video file boxes make room for longer labels in other languages.
- Interface texts now live in plain text files (`assets/i18n/`), so anyone can correct or add a translation.

## 1.1.2

### New
- **Windows installer (`.msi`)** for x64, x86, ARM64 and x64 with Whisper built in: a standard setup wizard in BabelCraft's colours (welcome, licence, install folder), installs to *Program Files* with Start menu and desktop shortcuts, uninstalls from *Settings → Apps*, and replaces an older version when you install a newer one. The `.zip` files are still there for running without installing.
- **Licence: GNU General Public License v3 or later** (`GPL-3.0-or-later`); earlier versions were MIT. *Settings → About BabelCraft* shows the version, the licence and links to the licence text and the source code.

### Improved
- **First-time downloads no longer look stuck.** While a job downloads what it needs (almost 5 GB with the offline AI brain and VOICEVOX), the progress screen shows *One-time setup · download 2 of 3*, the file being fetched, how much has arrived, the speed and the time left, instead of an *Extracting Media* bar that barely moved. The taskbar title shows the download percentage too.
- The time left for the translation itself no longer counts the download time.
- If you cancel during a download, the log says that what has downloaded is kept; the next job says it is *carrying on from* that point. This now also works for the VOICEVOX engine, which used to start its download again from the beginning.
- The **AI models & downloads** page shows size, speed and time left for its downloads as well.

## 1.1.1

### Fixed
- **Jobs that stopped after transcription.** The translator is now checked before the job starts: a missing API key, a stopped Ollama/LM Studio server or a missing model is reported at once with what to do (or switch to the built-in offline AI brain), instead of after minutes of text reading and transcription. Very large or programming-only Ollama models get a warning (they can make Ollama's engine stop), and if Ollama's engine stops mid-job the message says why.
- **Minimised window froze.** While BabelCraft was minimised its progress, taskbar title, log and stage times stopped updating (the job kept running unseen). The window now keeps track of the job while minimised.
- **Screen recordings produced hundreds of junk translations.** Text smaller than 3% of the picture height (menus, buttons, interface labels) is left out, one-frame flashes of title animations are ignored (text must stay 1.5 s), and garbled words are recognised as noise. On a 4-minute tutorial this went from 358 to 12 pieces of on-screen text. *Settings → On-screen text → smallest text size* changes it.
- **Choppy, misaligned dubbing.** Whisper's pieces often stop mid-sentence; they are now joined into whole sentences (with estimated times) before translation and voicing, and lines are numbered so every batch comes back complete and in order.

### New
- Re-running a job after a failure carries on where it stopped: the on-screen text, the transcription and every line already translated are kept (in BabelCraft's data folder, for 30 days).
- A log file (`logs/babelcraft.log` in BabelCraft's data folder) records every job, and any internal error is written there before the app closes; an internal error in a job ends it with a message instead of leaving the window waiting.

## 1.1.0

### New
- **Offline AI brain built in.** Translation can run entirely on your computer: BabelCraft downloads llama.cpp's `llama-server` and an open model (Qwen 3 4B by default; Qwen 2.5 1.5B / 7B and Gemma 3 4B to choose from), starts it hidden only while translating, then closes it. No account, no API key, nothing sent online.
- **Local and online AI services** for translation: Ollama (list models, download models into Ollama), LM Studio, Jan, llama.cpp server, LibreTranslate, OpenAI, Anthropic Claude, Google Gemini, DeepL, OpenRouter, Groq, DeepSeek, Mistral, and any OpenAI-compatible server. A **List** button shows the models a service offers.
- **Online speech recognition** (OpenAI Whisper API, Groq, or a local Speaches server) as an alternative to whisper.cpp.
- **VOICEVOX set up automatically**: the free VOICEVOX engine is downloaded and started hidden when a job needs a Japanese voice (an installed VOICEVOX app is used too). New voice picker with popular VOICEVOX characters.
- **The computer's own Japanese voice** (Windows / macOS) as a no-download voice option; OpenAI voices and Kokoro presets.
- **AI models & downloads page**: everything BabelCraft uses, what is already on your computer, Download / Remove buttons, links to every optional app, and boxes to paste API keys for online services.
- One-click **Use the offline AI brain** fix in Setup checks when the chosen translator isn't reachable.
- Interface in **English or 日本語** (buttons at the top right, or `--lang ja`).
- Big downloads **resume** where they stopped after an interruption, and dropped connections are retried.
- Portable mode now keeps the settings file in `babelcraft-data` too; `BABELCRAFT_CONFIG` points to another settings file.

### Fixed
- Symbols that showed as empty boxes (□) on some systems: arrows and dots now come from a bundled font, check marks are drawn.
- Stage boxes wrap into rows on narrow windows instead of squeezing their words.

## 1.0.0

First release: English → Japanese translation of on-screen text (built-in OCR with de-duplicated text intervals and auto-fitted Japanese overlays) and speech (Whisper → translation → Japanese voice, sped up to fit), rendered with FFmpeg; silent background processing with a multi-stage progress bar, ETA, log and Cancel; automatic download of FFmpeg, OCR models, Whisper and a Japanese font.
