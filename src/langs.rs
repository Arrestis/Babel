// SPDX-License-Identifier: GPL-3.0-or-later
//! The languages BabelCraft's own buttons and menus can be shown in, with the writing system
//! each one needs a font for.

/// Writing systems, for picking a font that has the right letters.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Script {
    Latin,
    Greek,
    Cyrillic,
    Armenian,
    Georgian,
    Hebrew,
    Arabic,
    Thaana,
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Odia,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Sinhala,
    Thai,
    Lao,
    Khmer,
    Myanmar,
    Ethiopic,
    Tibetan,
    Japanese,
    ChineseSimplified,
    ChineseTraditional,
    Korean,
}

impl Script {
    /// Written right to left.
    pub fn rtl(self) -> bool {
        matches!(self, Script::Hebrew | Script::Arabic | Script::Thaana)
    }

    /// egui's own font already has these letters.
    pub fn built_in(self) -> bool {
        matches!(self, Script::Latin | Script::Greek | Script::Cyrillic)
    }

    /// A few letters a font must have to show this writing system.
    pub fn sample(self) -> &'static [char] {
        match self {
            Script::Latin => &['A', 'é', 'ñ'],
            Script::Greek => &['α', 'Ω'],
            Script::Cyrillic => &['Д', 'ж'],
            Script::Armenian => &['Ա', 'ա'],
            Script::Georgian => &['ა', 'ქ'],
            Script::Hebrew => &['א', 'ש'],
            Script::Arabic => &['ا', 'ب', 'ع'],
            Script::Thaana => &['ދ', 'ވ'],
            Script::Devanagari => &['क', 'ि'],
            Script::Bengali => &['ক', 'া'],
            Script::Gurmukhi => &['ਕ', 'ਾ'],
            Script::Gujarati => &['ક', 'ા'],
            Script::Odia => &['କ', 'ା'],
            Script::Tamil => &['க', 'ா'],
            Script::Telugu => &['క', 'ా'],
            Script::Kannada => &['ಕ', 'ಾ'],
            Script::Malayalam => &['ക', 'ാ'],
            Script::Sinhala => &['ක', 'ා'],
            Script::Thai => &['ก', 'ไ'],
            Script::Lao => &['ກ', 'ລ'],
            Script::Khmer => &['ក', 'ខ'],
            Script::Myanmar => &['က', 'မ'],
            Script::Ethiopic => &['አ', 'ማ'],
            Script::Tibetan => &['ཀ', 'བ'],
            Script::Japanese => &['あ', 'ア', '日', '語'],
            Script::ChineseSimplified => &['简', '们', '这', '语'],
            Script::ChineseTraditional => &['繁', '們', '這', '語'],
            Script::Korean => &['한', '국', '어'],
        }
    }

    /// CJK fonts sit higher and look larger than egui's Latin font; they get nudged into line.
    pub fn cjk(self) -> bool {
        matches!(self, Script::Japanese | Script::ChineseSimplified | Script::ChineseTraditional | Script::Korean)
    }
}

/// One language: its code, its English name and its own name.
#[derive(Debug)]
pub struct Lang {
    pub code: &'static str,
    pub english: &'static str,
    pub native: &'static str,
    pub script: Script,
}

const fn l(code: &'static str, english: &'static str, native: &'static str, script: Script) -> Lang {
    Lang { code, english, native, script }
}

use Script::*;

/// Every language the interface can be shown in (sorted by English name).
pub const LANGUAGES: &[Lang] = &[
    l("af", "Afrikaans", "Afrikaans", Latin),
    l("sq", "Albanian", "Shqip", Latin),
    l("am", "Amharic", "አማርኛ", Ethiopic),
    l("ar", "Arabic", "العربية", Arabic),
    l("hy", "Armenian", "Հայերեն", Armenian),
    l("as", "Assamese", "অসমীয়া", Bengali),
    l("az", "Azerbaijani", "Azərbaycanca", Latin),
    l("eu", "Basque", "Euskara", Latin),
    l("be", "Belarusian", "Беларуская", Cyrillic),
    l("bn", "Bengali", "বাংলা", Bengali),
    l("bs", "Bosnian", "Bosanski", Latin),
    l("bg", "Bulgarian", "Български", Cyrillic),
    l("my", "Burmese", "မြန်မာ", Myanmar),
    l("ca", "Catalan", "Català", Latin),
    l("ceb", "Cebuano", "Cebuano", Latin),
    l("ny", "Chichewa", "Chichewa", Latin),
    l("zh-Hans", "Chinese (Simplified)", "简体中文", ChineseSimplified),
    l("zh-Hant", "Chinese (Traditional)", "繁體中文", ChineseTraditional),
    l("co", "Corsican", "Corsu", Latin),
    l("hr", "Croatian", "Hrvatski", Latin),
    l("cs", "Czech", "Čeština", Latin),
    l("da", "Danish", "Dansk", Latin),
    l("dv", "Dhivehi", "ދިވެހި", Thaana),
    l("nl", "Dutch", "Nederlands", Latin),
    l("en", "English", "English", Latin),
    l("eo", "Esperanto", "Esperanto", Latin),
    l("et", "Estonian", "Eesti", Latin),
    l("fil", "Filipino", "Filipino", Latin),
    l("fi", "Finnish", "Suomi", Latin),
    l("fr", "French", "Français", Latin),
    l("fy", "Frisian", "Frysk", Latin),
    l("gl", "Galician", "Galego", Latin),
    l("ka", "Georgian", "ქართული", Georgian),
    l("de", "German", "Deutsch", Latin),
    l("el", "Greek", "Ελληνικά", Greek),
    l("gn", "Guarani", "Avañe'ẽ", Latin),
    l("gu", "Gujarati", "ગુજરાતી", Gujarati),
    l("ht", "Haitian Creole", "Kreyòl ayisyen", Latin),
    l("ha", "Hausa", "Hausa", Latin),
    l("haw", "Hawaiian", "ʻŌlelo Hawaiʻi", Latin),
    l("he", "Hebrew", "עברית", Hebrew),
    l("hi", "Hindi", "हिन्दी", Devanagari),
    l("hmn", "Hmong", "Hmoob", Latin),
    l("hu", "Hungarian", "Magyar", Latin),
    l("is", "Icelandic", "Íslenska", Latin),
    l("ig", "Igbo", "Igbo", Latin),
    l("id", "Indonesian", "Bahasa Indonesia", Latin),
    l("ga", "Irish", "Gaeilge", Latin),
    l("it", "Italian", "Italiano", Latin),
    l("ja", "Japanese", "日本語", Japanese),
    l("jv", "Javanese", "Basa Jawa", Latin),
    l("kn", "Kannada", "ಕನ್ನಡ", Kannada),
    l("kk", "Kazakh", "Қазақ тілі", Cyrillic),
    l("km", "Khmer", "ខ្មែរ", Khmer),
    l("rw", "Kinyarwanda", "Kinyarwanda", Latin),
    l("ko", "Korean", "한국어", Korean),
    l("ku", "Kurdish (Kurmanji)", "Kurdî", Latin),
    l("ckb", "Kurdish (Sorani)", "کوردی", Arabic),
    l("ky", "Kyrgyz", "Кыргызча", Cyrillic),
    l("lo", "Lao", "ລາວ", Lao),
    l("la", "Latin", "Latina", Latin),
    l("lv", "Latvian", "Latviešu", Latin),
    l("lt", "Lithuanian", "Lietuvių", Latin),
    l("lb", "Luxembourgish", "Lëtzebuergesch", Latin),
    l("mk", "Macedonian", "Македонски", Cyrillic),
    l("mg", "Malagasy", "Malagasy", Latin),
    l("ms", "Malay", "Bahasa Melayu", Latin),
    l("ml", "Malayalam", "മലയാളം", Malayalam),
    l("mt", "Maltese", "Malti", Latin),
    l("mi", "Māori", "Te Reo Māori", Latin),
    l("mr", "Marathi", "मराठी", Devanagari),
    l("mn", "Mongolian", "Монгол", Cyrillic),
    l("ne", "Nepali", "नेपाली", Devanagari),
    l("no", "Norwegian", "Norsk", Latin),
    l("or", "Odia", "ଓଡ଼ିଆ", Odia),
    l("ps", "Pashto", "پښتو", Arabic),
    l("fa", "Persian", "فارسی", Arabic),
    l("pl", "Polish", "Polski", Latin),
    l("pt", "Portuguese", "Português", Latin),
    l("pa", "Punjabi", "ਪੰਜਾਬੀ", Gurmukhi),
    l("qu", "Quechua", "Runasimi", Latin),
    l("ro", "Romanian", "Română", Latin),
    l("ru", "Russian", "Русский", Cyrillic),
    l("sm", "Samoan", "Gagana Sāmoa", Latin),
    l("sa", "Sanskrit", "संस्कृतम्", Devanagari),
    l("gd", "Scottish Gaelic", "Gàidhlig", Latin),
    l("sr", "Serbian", "Српски", Cyrillic),
    l("st", "Sesotho", "Sesotho", Latin),
    l("sn", "Shona", "chiShona", Latin),
    l("sd", "Sindhi", "سنڌي", Arabic),
    l("si", "Sinhala", "සිංහල", Sinhala),
    l("sk", "Slovak", "Slovenčina", Latin),
    l("sl", "Slovenian", "Slovenščina", Latin),
    l("so", "Somali", "Soomaali", Latin),
    l("es", "Spanish", "Español", Latin),
    l("su", "Sundanese", "Basa Sunda", Latin),
    l("sw", "Swahili", "Kiswahili", Latin),
    l("sv", "Swedish", "Svenska", Latin),
    l("tg", "Tajik", "Тоҷикӣ", Cyrillic),
    l("ta", "Tamil", "தமிழ்", Tamil),
    l("tt", "Tatar", "Татарча", Cyrillic),
    l("te", "Telugu", "తెలుగు", Telugu),
    l("th", "Thai", "ไทย", Thai),
    l("bo", "Tibetan", "བོད་སྐད་", Tibetan),
    l("ti", "Tigrinya", "ትግርኛ", Ethiopic),
    l("tr", "Turkish", "Türkçe", Latin),
    l("tk", "Turkmen", "Türkmençe", Latin),
    l("uk", "Ukrainian", "Українська", Cyrillic),
    l("ur", "Urdu", "اردو", Arabic),
    l("ug", "Uyghur", "ئۇيغۇرچە", Arabic),
    l("uz", "Uzbek", "Oʻzbekcha", Latin),
    l("vi", "Vietnamese", "Tiếng Việt", Latin),
    l("cy", "Welsh", "Cymraeg", Latin),
    l("xh", "Xhosa", "isiXhosa", Latin),
    l("yi", "Yiddish", "ייִדיש", Hebrew),
    l("yo", "Yoruba", "Yorùbá", Latin),
    l("zu", "Zulu", "isiZulu", Latin),
];

/// English, the default.
pub fn english() -> &'static Lang {
    find("en").expect("English is in the list")
}

/// Japanese, the default language videos are translated into.
pub fn japanese() -> &'static Lang {
    find("ja").expect("Japanese is in the list")
}

impl Lang {
    /// The name to show in the interface: the English name in the English interface, otherwise
    /// the language's own name (understood whatever the interface language is).
    pub fn shown(&self) -> &'static str {
        if crate::i18n::current() == "en" { self.english } else { self.native }
    }

    /// Whisper's code for this language, or None when Whisper doesn't know it (then it guesses).
    pub fn whisper_code(&self) -> Option<&'static str> {
        const KNOWN: &[&str] = &[
            "af", "am", "ar", "as", "az", "be", "bg", "bn", "bo", "bs", "ca", "cs", "cy", "da", "de", "el", "en", "es", "et", "eu",
            "fa", "fi", "fr", "gl", "gu", "ha", "haw", "he", "hi", "hr", "ht", "hu", "hy", "id", "is", "it", "ja", "ka", "kk", "km",
            "kn", "ko", "la", "lb", "lo", "lt", "lv", "mg", "mi", "mk", "ml", "mn", "mr", "ms", "mt", "my", "ne", "nl", "no", "pa",
            "pl", "ps", "pt", "ro", "ru", "sa", "sd", "si", "sk", "sl", "sn", "so", "sq", "sr", "su", "sv", "sw", "ta", "te", "tg",
            "th", "tk", "tr", "tt", "uk", "ur", "uz", "vi", "yi", "yo",
        ];
        match self.code {
            "zh-Hans" | "zh-Hant" => Some("zh"),
            "fil" => Some("tl"),
            "jv" => Some("jw"),
            c => KNOWN.iter().find(|k| **k == c).copied(),
        }
    }

    /// Tesseract's language data name (traineddata) for on-screen text in this language.
    pub fn tesseract_code(&self) -> Option<&'static str> {
        Some(match self.code {
            "af" => "afr", "am" => "amh", "ar" => "ara", "as" => "asm", "az" => "aze", "be" => "bel", "bg" => "bul",
            "bn" => "ben", "bo" => "bod", "bs" => "bos", "ca" => "cat", "ceb" => "ceb", "co" => "cos", "cs" => "ces",
            "cy" => "cym", "da" => "dan", "de" => "deu", "dv" => "div", "el" => "ell", "en" => "eng", "eo" => "epo",
            "es" => "spa", "et" => "est", "eu" => "eus", "fa" => "fas", "fi" => "fin", "fil" => "fil", "fr" => "fra",
            "fy" => "fry", "ga" => "gle", "gd" => "gla", "gl" => "glg", "gu" => "guj", "he" => "heb", "hi" => "hin",
            "hr" => "hrv", "ht" => "hat", "hu" => "hun", "hy" => "hye", "id" => "ind", "is" => "isl", "it" => "ita",
            "ja" => "jpn", "jv" => "jav", "ka" => "kat", "kk" => "kaz", "km" => "khm", "kn" => "kan", "ko" => "kor",
            "ku" => "kmr", "ky" => "kir", "la" => "lat", "lb" => "ltz", "lo" => "lao", "lt" => "lit", "lv" => "lav",
            "mi" => "mri", "mk" => "mkd", "ml" => "mal", "mn" => "mon", "mr" => "mar", "ms" => "msa", "mt" => "mlt",
            "my" => "mya", "ne" => "nep", "nl" => "nld", "no" => "nor", "or" => "ori", "pa" => "pan", "pl" => "pol",
            "ps" => "pus", "pt" => "por", "qu" => "que", "ro" => "ron", "ru" => "rus", "sa" => "san", "sd" => "snd",
            "si" => "sin", "sk" => "slk", "sl" => "slv", "sq" => "sqi", "sr" => "srp", "su" => "sun", "sv" => "swe",
            "sw" => "swa", "ta" => "tam", "te" => "tel", "tg" => "tgk", "th" => "tha", "ti" => "tir", "tr" => "tur",
            "tt" => "tat", "ug" => "uig", "uk" => "ukr", "ur" => "urd", "uz" => "uzb", "vi" => "vie", "yi" => "yid",
            "yo" => "yor", "zh-Hans" => "chi_sim", "zh-Hant" => "chi_tra",
            _ => return None,
        })
    }

    /// ISO 639-2 code for the dubbed audio track's language tag ("und" when unknown).
    pub fn iso639_2(&self) -> &'static str {
        match self.code {
            "en" => "eng", "ja" => "jpn", "es" => "spa", "fr" => "fre", "de" => "ger", "pt" => "por", "it" => "ita",
            "ru" => "rus", "zh-Hans" | "zh-Hant" => "chi", "ko" => "kor", "ar" => "ara", "hi" => "hin", "id" => "ind",
            "tr" => "tur", "vi" => "vie", "th" => "tha", "pl" => "pol", "nl" => "dut", "sv" => "swe", "uk" => "ukr",
            "el" => "gre", "he" => "heb", "cs" => "cze", "ro" => "rum", "hu" => "hun", "da" => "dan", "fi" => "fin",
            "no" => "nor", "fa" => "per", "bn" => "ben", "ta" => "tam", "te" => "tel", "ms" => "may", "fil" => "fil",
            "sw" => "swa", "ur" => "urd", "bg" => "bul", "hr" => "hrv", "sr" => "srp", "sk" => "slo", "sl" => "slv",
            "ca" => "cat", "lt" => "lit", "lv" => "lav", "et" => "est",
            _ => "und",
        }
    }

    /// Languages written without spaces between words (lines may break between any two letters).
    pub fn no_spaces(&self) -> bool {
        matches!(
            self.script,
            Script::Japanese | Script::ChineseSimplified | Script::ChineseTraditional | Script::Thai | Script::Lao | Script::Khmer | Script::Myanmar | Script::Tibetan
        )
    }
}

/// The language with this code (case doesn't matter).
pub fn find(code: &str) -> Option<&'static Lang> {
    LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code.trim()))
}

/// A language code from a code, an English name or a language's own name
/// ("ja", "Japanese", "日本語", "zh-hans", "chinese (simplified)"…). Settings written by
/// version 1.1 say "English" or "Japanese".
pub fn normalize(s: &str) -> Option<&'static str> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_lowercase();
    let alias = match lower.as_str() {
        "jp" => Some("ja"),
        "zh" | "zh-cn" | "zh_cn" | "zh-sg" | "chinese" => Some("zh-Hans"),
        "zh-tw" | "zh_tw" | "zh-hk" => Some("zh-Hant"),
        "nb" | "nn" => Some("no"),
        "tl" => Some("fil"),
        "pt-br" | "pt_br" | "pt-pt" => Some("pt"),
        "iw" => Some("he"),
        "in" => Some("id"),
        _ => None,
    };
    if let Some(a) = alias {
        return Some(a);
    }
    LANGUAGES
        .iter()
        .find(|l| l.code.eq_ignore_ascii_case(s) || l.english.eq_ignore_ascii_case(s) || l.native.to_lowercase() == lower)
        .map(|l| l.code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_names_resolve() {
        let mut seen = std::collections::HashSet::new();
        for l in LANGUAGES {
            assert!(seen.insert(l.code.to_lowercase()), "duplicate code {}", l.code);
            assert_eq!(normalize(l.code), Some(l.code));
            assert_eq!(normalize(l.english), Some(l.code), "{}", l.english);
            assert_eq!(normalize(l.native), Some(l.code), "{}", l.native);
        }
        assert_eq!(normalize("Japanese"), Some("ja"));
        assert_eq!(normalize("English"), Some("en"));
        assert_eq!(normalize("ZH-CN"), Some("zh-Hans"));
        assert_eq!(normalize("klingon"), None);
        assert!(LANGUAGES.len() >= 100);
    }
}
