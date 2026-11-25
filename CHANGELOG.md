# Changelog

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
