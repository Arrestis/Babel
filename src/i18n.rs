//! Interface language (English / 日本語) for buttons, menus and labels.
//! Every piece of interface text is written in English and passed through `tr`,
//! which returns the Japanese version when Japanese is selected.

use std::sync::atomic::{AtomicU8, Ordering};

static JAPANESE: AtomicU8 = AtomicU8::new(0);

pub fn set_japanese(on: bool) {
    JAPANESE.store(on as u8, Ordering::Relaxed);
}

pub fn is_japanese() -> bool {
    JAPANESE.load(Ordering::Relaxed) == 1
}

/// Interface text in the chosen language (falls back to English when a line has no translation).
pub fn tr(en: &'static str) -> &'static str {
    if !is_japanese() {
        return en;
    }
    ja(en).unwrap_or(en)
}

fn ja(en: &str) -> Option<&'static str> {
    Some(match en {
        // Header / general
        "BabelCraft" => "BabelCraft",
        "English → 日本語  ·  on-screen text + voice dubbing" => "英語 → 日本語 ・ 画面テキスト翻訳 ＋ 音声吹き替え",
        "English → Japanese  ·  on-screen text + voice dubbing" => "英語 → 日本語 ・ 画面テキスト翻訳 ＋ 音声吹き替え",
        "English → 日本語 video translator" => "英語 → 日本語 動画翻訳ツール",
        "English → Japanese video translator" => "英語 → 日本語 動画翻訳ツール",
        "loading" => "読み込み中",
        "● Working in the background" => "● バックグラウンドで処理中",
        "● Idle" => "● 待機中",
        "Settings" => "設定",
        "Log console" => "ログ",
        "Language" => "表示言語",
        "Interface language" => "表示言語",
        "Saved automatically." => "自動的に保存されます。",
        "Reset to defaults" => "初期設定に戻す",
        "Clear" => "消去",
        "Copy all" => "すべてコピー",
        "lines" => "行",
        "●" => "●",

        // Files card
        "1  ·  Choose your video" => "1  ·  動画を選ぶ",
        "English video" => "英語の動画",
        "Save Japanese video as" => "日本語版の保存先",
        "MP4, MKV, MOV or AVI — or drop a file on the window" => "MP4・MKV・MOV・AVI（ウィンドウにドロップも可）",
        "where the translated video is written (.mp4 / .mkv / .mov)" => "翻訳済み動画の保存先 (.mp4 / .mkv / .mov)",
        "Browse…" => "参照…",
        "Choose an English video" => "英語の動画を選択",
        "Save the Japanese video" => "日本語版の動画を保存",

        // Progress card
        "2  ·  Translate to Japanese" => "2  ·  日本語に翻訳",
        "▶  Start translating" => "▶  翻訳を開始",
        "■  Cancel" => "■  キャンセル",
        "Stopping…" => "停止中…",
        "Runs in the background, completely silently." => "バックグラウンドで無音のまま処理します。",
        "Finished — your Japanese video is ready" => "完了 — 日本語版の動画ができました",
        "Stopped with an error" => "エラーで停止しました",
        "Cancelled" => "キャンセルしました",
        "Starting…" => "開始しています…",
        "Ready when you are" => "準備完了",
        "Overall" => "全体",
        "Stage" => "段階",
        "Elapsed" => "経過時間",
        "Time left (ETA)" => "残り時間",
        "This stage" => "この段階",
        "estimating…" => "計算中…",
        "Saved" => "保存しました",
        "Show in folder" => "フォルダーを開く",
        "Cancelled — all helper processes were stopped." => "キャンセルしました — 補助プロセスはすべて停止しました。",
        "Silent background mode — nothing is played or previewed. Minimise BabelCraft and keep working." => {
            "無音バックグラウンドモード — 再生もプレビューもしません。最小化して他の作業を続けられます。"
        }
        "Choose an input video and an output file first." => "先に入力動画と保存先を選んでください。",

        // Stages
        "Extracting Media" => "メディア抽出",
        "Scanning Screen Text (OCR)" => "画面テキスト読み取り (OCR)",
        "Transcribing Speech (Whisper)" => "音声の文字起こし (Whisper)",
        "Translating to Japanese" => "日本語に翻訳",
        "Synthesizing Japanese Voice" => "日本語音声の合成",
        "Rendering Final Video" => "最終動画の書き出し",

        // Setup checks
        "Setup checks" => "セットアップ確認",
        "Re-check" => "再確認",
        "checking…" => "確認中…",
        "⬇  Set up automatically" => "⬇  自動セットアップ",
        "Downloads FFmpeg, the offline AI brain, the VOICEVOX voice, models and fonts into BabelCraft's own folder — nothing to install by hand." => {
            "FFmpeg・オフライン AI・VOICEVOX 音声・モデル・フォントを BabelCraft 専用フォルダーにダウンロードします。手動インストールは不要です。"
        }
        "Looking for FFmpeg, Tesseract, Whisper, a Japanese font, the translator and the voice engine…" => {
            "FFmpeg・文字認識・Whisper・日本語フォント・翻訳エンジン・音声エンジンを確認しています…"
        }
        "Download" => "ダウンロード",
        "Stop" => "停止",
        "Setting up:" => "セットアップ中：",
        "Downloading" => "ダウンロード中",
        "FFmpeg" => "FFmpeg",
        "Text reader (OCR)" => "文字認識 (OCR)",
        "Japanese font" => "日本語フォント",
        "Whisper model" => "Whisper モデル",
        "Whisper engine" => "Whisper エンジン",
        "Japanese voice" => "日本語音声",
        "Translator" => "翻訳エンジン",
        "BabelCraft sets this up by itself: press \"Set up automatically\", or just start a job (it is fetched on first use)." => {
            "BabelCraft が自動で用意します。「自動セットアップ」を押すか、そのまま翻訳を開始してください（初回に取得します）。"
        }
        "Switch Settings → On-screen text → engine to \"Built-in\" (nothing to install), or install Tesseract." => {
            "設定 → 画面テキスト → エンジンを「内蔵」にする（インストール不要）か、Tesseract をインストールしてください。"
        }
        "Install the English data (tesseract-ocr-eng), or switch to the Built-in engine." => {
            "英語データ (tesseract-ocr-eng) をインストールするか、内蔵エンジンに切り替えてください。"
        }
        "Use the BabelCraft download with Whisper built in (\"-metal\" on Mac), or `brew install whisper-cpp`." => {
            "Whisper 内蔵版の BabelCraft（Mac は「-metal」版）を使うか、`brew install whisper-cpp` を実行してください。"
        }
        "VOICEVOX: install from voicevox.hiroshiba.jp and leave it running. Or pick edge-tts / Piper / an OpenAI-compatible TTS in Settings → Japanese voice." => {
            "VOICEVOX を voicevox.hiroshiba.jp から入手して起動しておくか、設定 → 日本語音声 で edge-tts / Piper / OpenAI 互換 TTS を選んでください。"
        }
        "Install Ollama (ollama.com), run `ollama pull qwen2.5:7b`, or switch to DeepL / an OpenAI-compatible API in Settings." => {
            "Ollama (ollama.com) をインストールして `ollama pull qwen2.5:7b` を実行するか、設定で DeepL / OpenAI 互換 API に切り替えてください。"
        }
        "Run `ollama pull qwen2.5:7b` (or set another model in Settings → Translation)." => {
            "`ollama pull qwen2.5:7b` を実行してください（または 設定 → 翻訳 で別のモデルを指定）。"
        }
        "Enter an API key in Settings → Translation (or set OPENAI_API_KEY)." => "設定 → 翻訳 で API キーを入力してください（または OPENAI_API_KEY を設定）。",
        "Enter your DeepL API key in Settings → Translation (or set DEEPL_AUTH_KEY)." => {
            "設定 → 翻訳 で DeepL API キーを入力してください（または DEEPL_AUTH_KEY を設定）。"
        }

        // Settings: translation
        "Translation" => "翻訳",
        "Ollama (local, offline)" => "Ollama（ローカル・オフライン）",
        "OpenAI-compatible API" => "OpenAI 互換 API",
        "DeepL API" => "DeepL API",
        "Server URL" => "サーバー URL",
        "Base URL" => "ベース URL",
        "Model" => "モデル",
        "API key" => "API キー",
        "Runs fully offline. Try qwen2.5:7b, gemma2:9b or llama3.1:8b." => "完全オフラインで動作します。qwen2.5:7b、gemma2:9b、llama3.1:8b などがおすすめです。",
        "OpenAI, LM Studio, llama.cpp server, vLLM… Empty key uses OPENAI_API_KEY." => {
            "OpenAI・LM Studio・llama.cpp server・vLLM など。空欄なら OPENAI_API_KEY を使います。"
        }
        "Free keys end in :fx. Empty key uses DEEPL_AUTH_KEY." => "無料キーは :fx で終わります。空欄なら DEEPL_AUTH_KEY を使います。",
        "Lines per request" => "1回の送信行数",
        "Style / glossary hint" => "文体・用語の指示",
        "Test translation" => "翻訳テスト",
        "Testing…" => "テスト中…",

        // Settings: audio
        "Audio mixing" => "音声ミックス",
        "Replace the English voice (Japanese only)" => "英語音声を置き換える（日本語のみ）",
        "Duck original audio under the Japanese dub" => "元の音声を下げて日本語音声を重ねる",
        "original volume" => "元の音量",
        "Duck the whole video (not only while speaking)" => "動画全体で音量を下げる（話している間だけでなく）",
        "dub volume" => "吹き替え音量",
        "max speed-up" => "最大速度",
        "Lines that run long are sped up (up to this limit) so they end before the next line." => {
            "長いセリフは次のセリフの前に終わるよう、この上限まで速くします。"
        }
        "Also keep the English audio as a 2nd track" => "英語音声も第2トラックとして残す",

        // Settings: OCR
        "On-screen text (OCR)" => "画面テキスト (OCR)",
        "Translate on-screen English text" => "画面上の英語テキストを翻訳する",
        "Only when the picture changes (fast)" => "画面が変わったときだけ（高速）",
        "Every sample" => "毎回",
        "Sample every" => "読み取り間隔",
        "change sensitivity" => "変化の感度",
        "min confidence" => "最低信頼度",
        "Ignore text shown less than" => "表示時間がこれより短い文字は無視",
        "Layout mode" => "レイアウト方式",
        "Engine" => "エンジン",
        "Built-in" => "内蔵",
        "Tesseract" => "Tesseract",
        "box opacity" => "背景の不透明度",
        "Auto" => "自動",
        "auto-detect" => "自動検出",
        "3 · automatic page layout" => "3 · 自動レイアウト",
        "6 · single text block" => "6 · 単一テキストブロック",
        "11 · sparse text (video)" => "11 · まばらな文字（動画向け）",
        "12 · sparse text + OSD" => "12 · まばらな文字 + 向き検出",
        "custom" => "カスタム",

        // Settings: speech
        "Speech (Whisper)" => "音声認識 (Whisper)",
        "Transcribe and dub spoken English" => "英語の音声を文字起こしして吹き替える",
        "Built-in Whisper needs a build with --features whisper (see README)." => "内蔵 Whisper には --features whisper 付きのビルドが必要です（README 参照）。",
        "Default" => "既定",
        "Use GPU when available" => "利用できる場合は GPU を使う",
        "CPU threads (0 = auto)" => "CPU スレッド数（0 = 自動）",
        "tiny.en · 75 MB · fastest" => "tiny.en · 75 MB · 最速",
        "base.en · 142 MB · recommended" => "base.en · 142 MB · おすすめ",
        "small.en · 466 MB · better" => "small.en · 466 MB · 高精度",
        "medium.en · 1.5 GB · best (slow)" => "medium.en · 1.5 GB · 最高精度（低速）",
        "large-v3-turbo · 1.6 GB · GPU" => "large-v3-turbo · 1.6 GB · GPU 向け",

        // Settings: voice
        "Japanese voice (TTS)" => "日本語音声 (TTS)",
        "VOICEVOX (local HTTP)" => "VOICEVOX（ローカル）",
        "OpenAI-compatible / Kokoro" => "OpenAI 互換 / Kokoro",
        "edge-tts (online)" => "edge-tts（オンライン）",
        "Piper (local CLI)" => "Piper（ローカル）",
        "Engine URL" => "エンジン URL",
        "Speaker ID" => "話者 ID",
        "Free, offline, high quality. Start the VOICEVOX app first." => "無料・オフライン・高品質。先に VOICEVOX を起動してください。",
        "Voice" => "声",
        "e.g. Kokoro-FastAPI locally (voice jf_alpha) or OpenAI (tts-1)." => "例：ローカルの Kokoro-FastAPI（声 jf_alpha）や OpenAI (tts-1)。",
        "Needs internet (`pip install edge-tts`). ja-JP-NanamiNeural / ja-JP-KeitaNeural." => {
            "インターネット接続が必要です（`pip install edge-tts`）。ja-JP-NanamiNeural / ja-JP-KeitaNeural。"
        }
        "Voice model" => "音声モデル",
        "no model selected" => "モデル未選択",

        // Settings: tools
        "Tools & output" => "ツールと出力",
        "quality (CRF)" => "画質 (CRF)",
        "Encoder speed" => "エンコード速度",
        "Keep temporary files (for debugging)" => "一時ファイルを残す（デバッグ用）",
        "Settings file:" => "設定ファイル：",

        // Services (translation, speech, voice)
        "BabelCraft offline AI brain (built in)" => "BabelCraft 内蔵のオフライン AI",
        "llama.cpp server" => "llama.cpp サーバー",
        "OpenRouter (hundreds of models)" => "OpenRouter（数百のモデル）",
        "Other OpenAI-compatible server" => "その他の OpenAI 互換サーバー",
        "translation" => "翻訳",
        "translation · speech recognition · voice" => "翻訳 · 音声認識 · 音声合成",
        "translation · speech recognition" => "翻訳 · 音声認識",
        "Groq (very fast Whisper)" => "Groq（超高速 Whisper）",
        "Whisper server on this computer (Speaches…)" => "このパソコンの Whisper サーバー（Speaches など）",
        "Kokoro-FastAPI (on this computer)" => "Kokoro-FastAPI（このパソコン）",
        "OpenAI voices (online)" => "OpenAI の音声（オンライン）",
        "Runs many open AI models on your computer (translation)." => "多くのオープン AI モデルをパソコンで動かせます（翻訳）。",
        "Desktop app for downloading and running AI models (translation)." => "AI モデルをダウンロードして動かすデスクトップアプリ（翻訳）。",
        "Free offline ChatGPT-style app with a built-in server (translation)." => "サーバー機能付きの無料オフライン ChatGPT 風アプリ（翻訳）。",
        "Free Japanese voice app with many characters (voice)." => "キャラクターが多数いる無料の日本語音声アプリ（音声）。",
        "Small local voice server with Japanese voices (voice)." => "日本語の声を持つ小型ローカル音声サーバー（音声）。",
        "Fast offline voice program; needs a Japanese voice model (voice)." => "高速なオフライン音声合成。日本語の音声モデルが必要です（音声）。",
        "Microsoft's online neural voices, no key needed (voice)." => "Microsoft のオンライン音声。キー不要（音声）。",
        "Free translation server you can run yourself (translation)." => "自分で動かせる無料の翻訳サーバー（翻訳）。",
        "Whisper speech-recognition server for your computer (speech)." => "パソコン用の Whisper 音声認識サーバー（音声認識）。",
        "Alternative text reader for on-screen text (OCR)." => "画面テキスト用の別の文字認識エンジン（OCR）。",

        // Offline AI brain models
        "Qwen 2.5 · 1.5B (small and fast)" => "Qwen 2.5 · 1.5B（小型・高速）",
        "Qwen 3 · 4B (recommended)" => "Qwen 3 · 4B（おすすめ）",
        "Gemma 3 · 4B (alternative)" => "Gemma 3 · 4B（別の選択肢）",
        "Qwen 2.5 · 7B (best quality)" => "Qwen 2.5 · 7B（最高品質）",
        "For older computers or 4 GB of memory. Simpler Japanese." => "古いパソコンやメモリ 4 GB 向け。日本語はやや簡単になります。",
        "Good natural Japanese; runs on most computers with 8 GB of memory." => "自然な日本語。メモリ 8 GB のほとんどのパソコンで動きます。",
        "Google's open model; a different style of Japanese to compare with Qwen." => "Google のオープンモデル。Qwen と違う文体の日本語を比べられます。",
        "Best translations; wants 16 GB of memory or a graphics card." => "最も良い翻訳。メモリ 16 GB かグラフィックカードが必要です。",

        // Settings: translation (services)
        "Offline AI brain" => "オフライン AI",
        "Who translates" => "翻訳を担当する AI",
        "On this computer — free, private, offline" => "このパソコンで — 無料・プライベート・オフライン",
        "Online services — need an account and API key" => "オンラインサービス — アカウントと API キーが必要",
        "AI model" => "AI モデル",
        "Use the graphics card (faster; Vulkan)" => "グラフィックカードを使う（高速・Vulkan）",
        "Downloaded — ready to translate offline" => "ダウンロード済み — オフラインで翻訳できます",
        "Downloads by itself on first use" => "初回使用時に自動でダウンロードします",
        "Download now" => "今すぐダウンロード",
        "Runs on your computer: nothing is sent online, no account or key needed." => {
            "パソコン内で動作します。データは送信されず、アカウントもキーも不要です。"
        }
        "All models & downloads…" => "すべてのモデルとダウンロード…",
        "Start the app's local server first, then press List to pick a loaded model." => {
            "先にアプリのローカルサーバーを起動し、「一覧」で読み込み済みのモデルを選んでください。"
        }
        "Press List to see the models your account can use." => "「一覧」を押すと、利用できるモデルが表示されます。",
        "claude-opus-5-5 is the most capable; claude-sonnet-5-5 and claude-haiku-5-5 cost less." => {
            "claude-opus-5-5 が最も高性能です。claude-sonnet-5-5 と claude-haiku-5-5 は安価です。"
        }
        "Your own LibreTranslate server (no key needed) or a hosted one with a key." => {
            "自分の LibreTranslate サーバー（キー不要）か、キー付きのホスト型サービス。"
        }
        "Get the app ↗" => "アプリを入手 ↗",
        "Get an API key ↗" => "API キーを入手 ↗",
        "Testing… (starting the AI brain)" => "テスト中…（AI を起動しています）",
        "List" => "一覧",
        "Ask the service which models it has" => "使えるモデルをサービスに問い合わせます",
        "asking the service…" => "問い合わせ中…",
        "Pick a model" => "モデルを選ぶ",

        // Settings: speech (online)
        "Online / server API" => "オンライン / サーバー API",
        "The video's sound is uploaded to this service to be transcribed. OpenAI and Groq need an API key." => {
            "動画の音声はこのサービスにアップロードされて文字起こしされます。OpenAI と Groq には API キーが必要です。"
        }
        "Get the server ↗" => "サーバーを入手 ↗",

        // Settings: voice
        "VOICEVOX (free, offline)" => "VOICEVOX（無料・オフライン）",
        "OpenAI voices / Kokoro (API)" => "OpenAI の音声 / Kokoro（API）",
        "Computer's own Japanese voice" => "パソコン内蔵の日本語音声",
        "Start VOICEVOX by itself when needed" => "必要なときに VOICEVOX を自動で起動する",
        "Free, offline, high quality. BabelCraft downloads the VOICEVOX engine (1.9 GB) the first time. Credit the voice when you publish (e.g. \"VOICEVOX:ずんだもん\")." => {
            "無料・オフライン・高品質。初回に BabelCraft が VOICEVOX エンジン（1.9 GB）をダウンロードします。公開時はクレジットを記載してください（例：「VOICEVOX:ずんだもん」）。"
        }
        "VOICEVOX website and terms ↗" => "VOICEVOX のサイトと利用規約 ↗",
        "e.g. Kokoro-FastAPI locally (voice jf_alpha) or OpenAI (gpt-4o-mini-tts / tts-1, voices alloy, nova…)." => {
            "例：ローカルの Kokoro-FastAPI（声 jf_alpha）や OpenAI（gpt-4o-mini-tts / tts-1、声 alloy・nova など）。"
        }
        "Voice name" => "声の名前",
        "Uses a Japanese voice built into Windows or macOS. Leave the name empty to use the first one found." => {
            "Windows や macOS に内蔵の日本語音声を使います。名前が空欄なら最初に見つかった声を使います。"
        }
        "Found:" => "見つかった声：",
        "Zundamon · normal" => "ずんだもん · ノーマル",
        "Zundamon · sweet" => "ずんだもん · あまあま",
        "Shikoku Metan · normal" => "四国めたん · ノーマル",
        "Shikoku Metan · sweet" => "四国めたん · あまあま",
        "Kasukabe Tsumugi" => "春日部つむぎ",
        "Namine Ritsu" => "波音リツ",
        "Amehare Hau" => "雨晴はう",
        "Kurono Takehiro" => "玄野武宏",
        "Shirakami Kotarou" => "白上虎太郎",
        "Aoyama Ryusei" => "青山龍星",
        "Meimei Himari" => "冥鳴ひまり",
        "Kyushu Sora" => "九州そら",

        // Setup checks (new)
        "Use the offline AI brain" => "オフライン AI を使う",
        "Paste the service's API key in Settings → Speech (or on the AI models & downloads page), or switch back to whisper-cli (free, offline)." => {
            "設定 → 音声認識（または「AI モデルとダウンロード」）に API キーを貼り付けるか、whisper-cli（無料・オフライン）に戻してください。"
        }
        "BabelCraft downloads the free VOICEVOX engine by itself (press \"Set up automatically\" or just start a job). When you publish a video, credit the voice, e.g. \"VOICEVOX:ずんだもん\"." => {
            "BabelCraft が無料の VOICEVOX エンジンを自動でダウンロードします（「自動セットアップ」を押すか、そのまま翻訳を開始）。動画を公開するときはクレジットを記載してください（例：「VOICEVOX:ずんだもん」）。"
        }
        "Windows: Settings → Time & language → Speech → Add voices → Japanese. Mac: System Settings → Accessibility → Spoken Content → System voice → Manage voices → Japanese. Or choose VOICEVOX (free, downloaded automatically)." => {
            "Windows：設定 → 時刻と言語 → 音声認識 → 音声の追加 → 日本語。Mac：システム設定 → アクセシビリティ → 読み上げコンテンツ → システムの声 → 声を管理 → 日本語。または VOICEVOX（無料・自動ダウンロード）を選んでください。"
        }
        "Pick VOICEVOX in Settings → Japanese voice (free, downloaded automatically), or start the voice app/server you chose." => {
            "設定 → 日本語音声 で VOICEVOX（無料・自動ダウンロード）を選ぶか、選んだ音声アプリ / サーバーを起動してください。"
        }
        "Install Ollama or LM Studio (see AI models & downloads), or use an online service in Settings → Translation." => {
            "Ollama か LM Studio をインストールする（「AI モデルとダウンロード」参照）か、設定 → 翻訳 でオンラインサービスを使ってください。"
        }
        "Paste your Anthropic API key in Settings → Translation, or use the free offline AI brain." => {
            "設定 → 翻訳 に Anthropic の API キーを貼り付けるか、無料のオフライン AI を使ってください。"
        }
        "Start your LibreTranslate server, or use the free offline AI brain." => "LibreTranslate サーバーを起動するか、無料のオフライン AI を使ってください。",
        "Install Ollama (ollama.com) and run `ollama pull qwen2.5:7b` — or press the button to use BabelCraft's own offline AI brain instead (no install)." => {
            "Ollama (ollama.com) をインストールして `ollama pull qwen2.5:7b` を実行するか、ボタンを押して BabelCraft 内蔵のオフライン AI を使ってください（インストール不要）。"
        }
        "Open the app and start its local server (LM Studio: Developer → Start server; Jan: Settings → Local API Server) — or use the built-in offline AI brain." => {
            "アプリを開いてローカルサーバーを起動してください（LM Studio：Developer → Start server、Jan：Settings → Local API Server）。または内蔵のオフライン AI を使ってください。"
        }
        "Paste your API key in Settings → Translation (or on the AI models & downloads page), or use the free offline AI brain." => {
            "設定 → 翻訳（または「AI モデルとダウンロード」）に API キーを貼り付けるか、無料のオフライン AI を使ってください。"
        }

        // AI models & downloads page
        "▶  Translate a video" => "▶  動画を翻訳",
        "⬇  AI models & downloads" => "⬇  AI モデルとダウンロード",
        "AI models & downloads" => "AI モデルとダウンロード",
        "Refresh" => "更新",
        "Everything BabelCraft uses, where it comes from, and what is already on this computer. Downloads go into BabelCraft's own folder — nothing is installed into your system." => {
            "BabelCraft が使うもの、その入手元、このパソコンにすでにあるものの一覧です。ダウンロードは BabelCraft 専用フォルダーに入り、システムには何もインストールしません。"
        }
        "Folder:" => "フォルダー：",
        "Needed for every video (downloaded automatically)" => "すべての動画で必要（自動でダウンロード）",
        "ready" => "準備完了",
        "not downloaded yet" => "未ダウンロード",
        "Website ↗" => "公式サイト ↗",
        "Reads the video and writes the finished Japanese version." => "動画を読み込み、完成した日本語版を書き出します。",
        "Text-reading models (OCR)" => "文字認識モデル (OCR)",
        "Find and read the English text shown in the picture." => "画面に映る英語の文字を見つけて読み取ります。",
        "Japanese font (Noto Sans JP)" => "日本語フォント (Noto Sans JP)",
        "Draws the Japanese text over the video, so it never shows as □□□." => "動画の上に日本語を描きます（□□□ になりません）。",
        "use Online API or the Whisper-built-in download" => "オンライン API か Whisper 内蔵版を使ってください",
        "Whisper speech tool (whisper.cpp)" => "Whisper 音声認識ツール (whisper.cpp)",
        "Turns the English speech into text with timings." => "英語の音声をタイミング付きの文字にします。",
        "Whisper speech model" => "Whisper 音声モデル",
        "Bigger models hear more accurately but take longer." => "大きいモデルほど正確ですが、時間がかかります。",
        "Offline AI brain — translation on your own computer" => "オフライン AI — 自分のパソコンで翻訳",
        "No account, no API key, and nothing leaves your computer. BabelCraft downloads the llama.cpp engine and the model you pick, runs it hidden only while translating, then closes it." => {
            "アカウントも API キーも不要で、データは外に出ません。BabelCraft が llama.cpp エンジンと選んだモデルをダウンロードし、翻訳中だけ裏で動かして、終われば閉じます。"
        }
        "not available for this computer" => "このパソコンでは使えません",
        "AI brain engine (llama.cpp)" => "AI エンジン (llama.cpp)",
        "The program that runs the AI model." => "AI モデルを動かすプログラムです。",
        "in use" => "使用中",
        "downloaded" => "ダウンロード済み",
        "Licence:" => "ライセンス：",
        "Model page ↗" => "モデルのページ ↗",
        "Remove" => "削除",
        "Use for translation" => "翻訳に使う",
        "Or use the free Ollama app" => "または無料アプリ Ollama を使う",
        "No models yet — download one below." => "まだモデルがありません — 下でダウンロードしてください。",
        "running" => "動作中",
        "models" => "モデル",
        "Use Ollama for translation" => "Ollama で翻訳する",
        "Download a model into Ollama:" => "Ollama にモデルをダウンロード：",
        "Model library ↗" => "モデル一覧 ↗",
        "not running" => "停止中",
        "Free app that runs many open AI models. Install it, and BabelCraft can use it and download models into it." => {
            "多くのオープン AI モデルを動かせる無料アプリです。インストールすれば BabelCraft から使えて、モデルもダウンロードできます。"
        }
        "Get Ollama ↗" => "Ollama を入手 ↗",
        "running ·" => "動作中 ·",
        "downloaded · starts by itself" => "ダウンロード済み · 自動で起動",
        "VOICEVOX app found · starts by itself" => "VOICEVOX アプリあり · 自動で起動",
        "VOICEVOX voice engine" => "VOICEVOX 音声エンジン",
        "Free, offline, natural Japanese voices. Credit the voice when you publish, e.g. \"VOICEVOX:ずんだもん\"." => {
            "無料・オフラインの自然な日本語音声。公開時はクレジットを記載してください（例：「VOICEVOX:ずんだもん」）。"
        }
        "Terms ↗" => "利用規約 ↗",
        "no Japanese voice installed" => "日本語音声がありません",
        "not available on Linux" => "Linux では使えません",
        "Windows and macOS can add a Japanese voice for free in their speech settings." => "Windows と macOS は音声の設定から日本語音声を無料で追加できます。",
        "Speech settings ↗" => "音声の設定 ↗",
        "Other free apps BabelCraft can work with" => "BabelCraft と一緒に使える無料アプリ",
        "Online AI services — paste your API key" => "オンライン AI サービス — API キーを貼り付け",
        "Keys stay in BabelCraft's settings file on this computer. Online services charge for what you use; pick one in Settings → Translation or Speech." => {
            "キーはこのパソコンの BabelCraft 設定ファイルにだけ保存されます。オンラインサービスは使った分だけ料金がかかります。設定 → 翻訳 / 音声認識 で選んでください。"
        }
        "key saved" => "キー保存済み",
        "key from environment" => "環境変数のキー",
        "Get a key ↗" => "キーを入手 ↗",
        "paste API key" => "API キーを貼り付け",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn switches_language() {
        set_japanese(true);
        assert_eq!(tr("Settings"), "設定");
        assert_eq!(tr("not in the table"), "not in the table");
        set_japanese(false);
        assert_eq!(tr("Settings"), "Settings");
    }
}
