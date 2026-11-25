//! On-screen English text detection (Tesseract) and de-duplication into timed intervals.

use crate::config::{Config, OcrSampling};
use crate::util::{CancelToken, run_process, silent_command};
use anyhow::Result;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl BBox {
    pub fn area(&self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }
    pub fn union(&self, o: &BBox) -> BBox {
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = (self.x + self.w).max(o.x + o.w);
        let y1 = (self.y + self.h).max(o.y + o.h);
        BBox { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
    pub fn iou(&self, o: &BBox) -> f32 {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        let inter = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
        let uni = self.area() + o.area() - inter;
        if uni <= 0.0 { 0.0 } else { inter / uni }
    }
}

#[derive(Clone, Debug)]
pub struct Detection {
    /// In source-video pixels.
    pub bbox: BBox,
    pub text: String,
    pub conf: f32,
}

/// A piece of on-screen text that stays visible from `start` to `end`.
#[derive(Clone, Debug)]
pub struct TextInterval {
    pub start: f64,
    pub end: f64,
    pub bbox: BBox,
    pub english: String,
    pub japanese: String,
}

/// Runs Tesseract on one image file and groups words into paragraphs/lines.
/// `scale` converts OCR-frame pixels back to source-video pixels.
pub fn run_tesseract(cfg: &Config, image: &Path, scale: f32, cancel: &CancelToken) -> Result<Vec<Detection>> {
    let mut cmd = silent_command(cfg.tesseract());
    cmd.arg(image)
        .arg("stdout")
        .args(["-l", &cfg.ocr.language, "--psm", &cfg.ocr.psm.to_string(), "-c", "preserve_interword_spaces=1", "tsv"]);
    let out = run_process(cmd, cancel, None, |_, _| {})?.ok_or_err("tesseract")?;
    Ok(parse_tsv(&out.stdout, cfg.ocr.min_confidence, scale))
}

/// OCR that runs inside BabelCraft (ocrs neural network; no external program).
pub struct BuiltinOcr {
    engine: ocrs::OcrEngine,
}

impl BuiltinOcr {
    pub fn load() -> Result<Self> {
        let (det, rec) = crate::tools::ocr_models();
        let detection_model = rten::Model::load_file(&det).map_err(|e| anyhow::anyhow!("could not load {}: {e}", det.display()))?;
        let recognition_model = rten::Model::load_file(&rec).map_err(|e| anyhow::anyhow!("could not load {}: {e}", rec.display()))?;
        let engine = ocrs::OcrEngine::new(ocrs::OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..Default::default()
        })
        .map_err(|e| anyhow::anyhow!("OCR engine: {e}"))?;
        Ok(Self { engine })
    }

    /// Finds English text in a grayscale frame. `scale` maps frame pixels to video pixels.
    pub fn detect(&self, gray: &[u8], w: u32, h: u32, scale: f32) -> Result<Vec<Detection>> {
        use ocrs::TextItem;
        let src = ocrs::ImageSource::from_bytes(gray, (w, h)).map_err(|e| anyhow::anyhow!("OCR input: {e}"))?;
        let input = self.engine.prepare_input(src).map_err(|e| anyhow::anyhow!("OCR: {e}"))?;
        let words = self.engine.detect_words(&input).map_err(|e| anyhow::anyhow!("OCR: {e}"))?;
        let lines = self.engine.find_text_lines(&input, &words);
        let texts = self.engine.recognize_text(&input, &lines).map_err(|e| anyhow::anyhow!("OCR: {e}"))?;

        // One entry per recognised line.
        let mut found: Vec<(BBox, String)> = texts
            .iter()
            .flatten()
            .filter_map(|line| {
                let text = line.to_string().trim().to_string();
                if text.chars().count() < 2 {
                    return None;
                }
                let r = line.bounding_rect();
                let b = BBox { x: r.left() as f32, y: r.top() as f32, w: r.width() as f32, h: r.height() as f32 };
                Some((b, text))
            })
            .collect();
        found.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));

        // Merge lines stacked directly under each other (same paragraph / title block).
        let mut merged: Vec<(BBox, String)> = Vec::new();
        for (b, t) in found {
            if let Some((mb, mt)) = merged.iter_mut().rev().find(|(mb, _)| {
                let gap = b.y - (mb.y + mb.h);
                let overlap = (mb.x + mb.w).min(b.x + b.w) - mb.x.max(b.x);
                gap >= -b.h * 0.3 && gap < b.h * 0.8 && overlap > 0.5 * b.w.min(mb.w)
            }) {
                *mb = mb.union(&b);
                mt.push(' ');
                mt.push_str(&t);
            } else {
                merged.push((b, t));
            }
        }

        Ok(merged
            .into_iter()
            .filter(|(_, t)| looks_like_english(t))
            .map(|(b, text)| Detection { bbox: BBox { x: b.x * scale, y: b.y * scale, w: b.w * scale, h: b.h * scale }, text, conf: 100.0 })
            .collect())
    }
}

#[derive(Default)]
struct LineAcc {
    words: Vec<(f32, String)>, // (left, word)
    bbox: Option<BBox>,
    conf_sum: f32,
    n: usize,
}

pub fn parse_tsv(tsv: &str, min_conf: f32, scale: f32) -> Vec<Detection> {
    // key: (block, par, line)
    let mut lines: BTreeMap<(u32, u32, u32), LineAcc> = BTreeMap::new();
    for row in tsv.lines().skip(1) {
        let cols: Vec<&str> = row.split('\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let num = |i: usize| cols[i].trim().parse::<f32>().unwrap_or(0.0);
        let conf = num(10);
        let word = cols[11].trim();
        if word.is_empty() || conf < min_conf {
            continue;
        }
        let key = (num(2) as u32, num(3) as u32, num(4) as u32);
        let b = BBox { x: num(6), y: num(7), w: num(8), h: num(9) };
        let acc = lines.entry(key).or_default();
        acc.words.push((b.x, word.to_string()));
        acc.bbox = Some(acc.bbox.map(|a| a.union(&b)).unwrap_or(b));
        acc.conf_sum += conf;
        acc.n += 1;
    }

    // Merge lines of the same paragraph that sit directly under each other.
    let mut paras: BTreeMap<(u32, u32), Vec<(BBox, String, f32)>> = BTreeMap::new();
    for ((blk, par, _), mut acc) in lines {
        acc.words.sort_by(|a, b| a.0.total_cmp(&b.0));
        let text = acc.words.iter().map(|w| w.1.as_str()).collect::<Vec<_>>().join(" ");
        if let Some(b) = acc.bbox {
            paras.entry((blk, par)).or_default().push((b, text, acc.conf_sum / acc.n.max(1) as f32));
        }
    }

    let mut out = Vec::new();
    for (_, mut ls) in paras {
        ls.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
        let mut cur: Option<(BBox, String, f32, usize)> = None;
        for (b, t, c) in ls {
            match cur.as_mut() {
                Some((cb, ct, cc, n)) if b.y - (cb.y + cb.h) < b.h * 0.9 => {
                    *cb = cb.union(&b);
                    ct.push(' ');
                    ct.push_str(&t);
                    *cc += c;
                    *n += 1;
                }
                _ => {
                    if let Some(done) = cur.take() {
                        out.push(done);
                    }
                    cur = Some((b, t, c, 1));
                }
            }
        }
        if let Some(done) = cur {
            out.push(done);
        }
    }

    out.into_iter()
        .filter_map(|(b, text, c, n)| {
            let text = text.trim().to_string();
            if !looks_like_english(&text) {
                return None;
            }
            Some(Detection {
                bbox: BBox { x: b.x * scale, y: b.y * scale, w: b.w * scale, h: b.h * scale },
                text,
                conf: c / n as f32,
            })
        })
        .collect()
}

/// Rejects OCR noise: needs real letters and mostly-letter content.
fn looks_like_english(t: &str) -> bool {
    let letters = t.chars().filter(|c| c.is_ascii_alphabetic()).count();
    let visible = t.chars().filter(|c| !c.is_whitespace()).count();
    letters >= 2 && visible > 0 && letters as f32 / visible as f32 >= 0.55 && t.split_whitespace().any(|w| w.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 2)
}

pub fn normalize(t: &str) -> String {
    let mut s = String::with_capacity(t.len());
    let mut space = false;
    for c in t.chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
            space = false;
        } else if !space && !s.is_empty() {
            s.push(' ');
            space = true;
        }
    }
    s.trim_end().to_string()
}

/// Similarity 0..1 based on Levenshtein distance.
pub fn similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    let a: Vec<char> = a.chars().take(400).collect();
    let b: Vec<char> = b.chars().take(400).collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    1.0 - prev[b.len()] as f32 / a.len().max(b.len()) as f32
}

/// Small grayscale thumbnail used to detect "the picture changed".
pub fn thumbnail(pixels: &[u8], w: u32, h: u32) -> Vec<u8> {
    const TW: u32 = 64;
    const TH: u32 = 36;
    let mut out = vec![0u8; (TW * TH) as usize];
    for ty in 0..TH {
        for tx in 0..TW {
            let x0 = tx * w / TW;
            let x1 = ((tx + 1) * w / TW).max(x0 + 1);
            let y0 = ty * h / TH;
            let y1 = ((ty + 1) * h / TH).max(y0 + 1);
            let mut sum = 0u64;
            let mut n = 0u64;
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    sum += pixels[(y * w + x) as usize] as u64;
                    n += 1;
                    x += 2;
                }
                y += 2;
            }
            out[(ty * TW + tx) as usize] = (sum / n.max(1)) as u8;
        }
    }
    out
}

pub fn thumb_diff(a: &[u8], b: &[u8]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let s: u64 = a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64).sum();
    s as f32 / (a.len() as f32 * 255.0)
}

/// Decides per sampled frame whether OCR must run again.
pub struct ChangeDetector {
    last: Option<Vec<u8>>,
    frames_since_ocr: usize,
}

impl ChangeDetector {
    pub fn new() -> Self {
        Self { last: None, frames_since_ocr: 0 }
    }
    pub fn needs_ocr(&mut self, cfg: &Config, thumb: Vec<u8>) -> bool {
        if cfg.ocr.sampling == OcrSampling::EveryInterval {
            return true;
        }
        let changed = match &self.last {
            None => true,
            Some(prev) => thumb_diff(prev, &thumb) >= cfg.ocr.change_threshold,
        };
        // Re-check at least every ~10 samples so slowly fading text is not missed.
        if changed || self.frames_since_ocr >= 10 {
            self.last = Some(thumb);
            self.frames_since_ocr = 0;
            true
        } else {
            self.frames_since_ocr += 1;
            false
        }
    }
}

struct OpenRegion {
    first_seen: f64,
    last_seen: f64,
    bbox: BBox,
    variants: HashMap<String, (usize, f32, String)>, // normalized -> (count, best conf, original)
    key: String,
}

/// Groups detections from consecutive frames into `(start, end, bbox, text)` intervals.
pub struct IntervalTracker {
    open: Vec<OpenRegion>,
    done: Vec<TextInterval>,
    step: f64,
}

impl IntervalTracker {
    pub fn new(step_secs: f64) -> Self {
        Self { open: Vec::new(), done: Vec::new(), step: step_secs }
    }

    pub fn push(&mut self, t: f64, dets: &[Detection]) {
        let mut matched = vec![false; self.open.len()];
        let mut new_regions = Vec::new();
        for d in dets {
            let key = normalize(&d.text);
            if key.is_empty() {
                continue;
            }
            let best = self
                .open
                .iter()
                .enumerate()
                .filter(|(i, _)| !matched[*i])
                .map(|(i, r)| (i, similarity(&r.key, &key), r.bbox.iou(&d.bbox)))
                .filter(|(_, sim, iou)| *sim >= 0.8 && *iou >= 0.3)
                .max_by(|a, b| (a.1 + a.2).total_cmp(&(b.1 + b.2)));
            match best {
                Some((i, _, _)) => {
                    matched[i] = true;
                    let r = &mut self.open[i];
                    r.last_seen = t;
                    r.bbox = r.bbox.union(&d.bbox);
                    let e = r.variants.entry(key).or_insert((0, 0.0, d.text.clone()));
                    e.0 += 1;
                    if d.conf > e.1 {
                        e.1 = d.conf;
                        e.2 = d.text.clone();
                    }
                }
                None => {
                    let mut variants = HashMap::new();
                    variants.insert(key.clone(), (1, d.conf, d.text.clone()));
                    new_regions.push(OpenRegion { first_seen: t, last_seen: t, bbox: d.bbox, variants, key });
                }
            }
        }
        // Close regions not seen for more than ~1.5 samples (tolerates one OCR miss).
        let step = self.step;
        let mut keep = Vec::new();
        for (i, r) in self.open.drain(..).enumerate() {
            if matched[i] || t - r.last_seen <= step * 1.6 {
                keep.push(r);
            } else {
                self.done.push(Self::close(r, step));
            }
        }
        keep.extend(new_regions);
        self.open = keep;
    }

    /// Text seen on a frame that was skipped (unchanged picture) is still visible.
    pub fn extend_all(&mut self, t: f64) {
        for r in &mut self.open {
            r.last_seen = t;
        }
    }

    fn close(r: OpenRegion, step: f64) -> TextInterval {
        let english = r
            .variants
            .values()
            .max_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)))
            .map(|v| v.2.clone())
            .unwrap_or_default();
        TextInterval {
            start: (r.first_seen - step / 2.0).max(0.0),
            end: r.last_seen + step / 2.0,
            bbox: r.bbox,
            english,
            japanese: String::new(),
        }
    }

    pub fn finish(mut self, duration: f64, min_duration: f64) -> Vec<TextInterval> {
        let step = self.step;
        for r in self.open.drain(..) {
            self.done.push(Self::close(r, step));
        }
        let mut v: Vec<TextInterval> = self
            .done
            .into_iter()
            .map(|mut i| {
                if duration > 0.0 {
                    // Text still visible on the last sample stays until the video ends.
                    if i.end + step >= duration {
                        i.end = duration;
                    }
                    i.end = i.end.min(duration);
                }
                i
            })
            .filter(|i| i.end - i.start >= min_duration)
            .collect();
        v.sort_by(|a, b| a.start.total_cmp(&b.start));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_groups_words_into_lines() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t10\t10\t50\t20\t95\tHello\n\
5\t1\t1\t1\t1\t2\t70\t10\t60\t20\t93\tWorld\n\
5\t1\t2\t1\t1\t1\t10\t300\t30\t20\t20\t~~\n";
        let d = parse_tsv(tsv, 60.0, 1.0);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].text, "Hello World");
        assert_eq!(d[0].bbox.w, 120.0);
    }

    #[test]
    fn static_text_becomes_one_interval() {
        let mut t = IntervalTracker::new(1.0);
        let det = Detection { bbox: BBox { x: 0.0, y: 0.0, w: 100.0, h: 20.0 }, text: "Welcome to the course".into(), conf: 90.0 };
        for i in 0..5 {
            t.push(i as f64, std::slice::from_ref(&det));
        }
        t.push(5.0, &[]);
        t.push(6.0, &[]);
        let v = t.finish(10.0, 0.5);
        assert_eq!(v.len(), 1);
        assert!((v[0].start - 0.0).abs() < 1e-6 && (v[0].end - 4.5).abs() < 1e-6);
    }

    #[test]
    fn similarity_works() {
        assert!(similarity("hello world", "hello wor1d") > 0.85);
        assert!(similarity("hello", "goodbye") < 0.5);
    }
}
