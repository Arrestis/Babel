// SPDX-License-Identifier: GPL-3.0-or-later
//! Draws the translated text over the original text boxes (semi-transparent box, auto-fitted
//! text in the target language's writing) and writes a timed overlay stream FFmpeg can
//! composite. Lines are shaped like a word processor would (joined Arabic letters, Indian and
//! Thai vowel signs, right-to-left words the right way round).

use crate::config::Config;
use crate::fonts::FoundFont;
use crate::langs::Lang;
use crate::ocr::{BBox, TextInterval};
use crate::util::CancelToken;
use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use anyhow::{Context, Result};
use image::{ImageEncoder, Rgba, RgbaImage};
use std::io::Write;
use std::path::{Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;

pub struct OverlayRenderer {
    /// The font file (for shaping) and the same font for drawing the glyph outlines.
    data: Vec<u8>,
    index: u32,
    shaper_data: harfrust::ShaperData,
    font: FontVec,
    /// The language the text is in (line breaking, writing direction).
    lang: &'static Lang,
    text: [u8; 3],
    box_rgb: [u8; 3],
    opacity: f32,
}

/// One shaped glyph, in font units.
struct ShapedGlyph {
    id: u16,
    advance: f32,
    x_off: f32,
    y_off: f32,
}

/// Right-to-left letters (Hebrew, Arabic, Thaana and their presentation forms).
fn is_rtl_char(c: char) -> bool {
    matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}

/// Splits a line into runs of one writing direction; spaces and punctuation stay with the run
/// they follow. Numbers and Latin words inside Arabic or Hebrew become runs of their own.
fn direction_runs(line: &str) -> Vec<String> {
    if !line.chars().any(is_rtl_char) {
        return vec![line.to_string()];
    }
    let mut runs: Vec<(bool, String)> = Vec::new();
    for c in line.chars() {
        let strong = if is_rtl_char(c) {
            Some(true)
        } else if c.is_alphanumeric() {
            Some(false)
        } else {
            None
        };
        match (strong, runs.last_mut()) {
            (Some(rtl), Some((r, s))) if *r == rtl => s.push(c),
            (None, Some((_, s))) => s.push(c),
            (Some(rtl), _) => runs.push((rtl, c.to_string())),
            (None, None) => runs.push((true, c.to_string())),
        }
    }
    runs.into_iter().map(|(_, s)| s).collect()
}

/// Characters that must not start a line (Japanese line-breaking rule, simplified).
const NO_LINE_START: &str = "、。，．・：；？！ー―…‥」』）】〉》〕］｝ゝゞぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ,.!?:;)]}%";

pub struct Placed {
    pub x: i64,
    pub y: i64,
    pub img: RgbaImage,
}

impl OverlayRenderer {
    pub fn new(cfg: &Config, font: &FoundFont, lang: &'static Lang) -> Result<Self> {
        let data = crate::fonts::load(font).context("could not read the overlay font")?;
        let shaper_data = {
            let fref = harfrust::FontRef::from_index(&data, font.index).map_err(|e| anyhow::anyhow!("invalid font file: {e}"))?;
            harfrust::ShaperData::new(&fref)
        };
        let outlines = FontVec::try_from_vec_and_index(data.clone(), font.index).context("invalid font file")?;
        Ok(Self {
            data,
            index: font.index,
            shaper_data,
            font: outlines,
            lang,
            text: cfg.overlay.text_color,
            box_rgb: cfg.overlay.box_color,
            opacity: cfg.overlay.box_opacity.clamp(0.0, 1.0),
        })
    }

    /// Shapes one line into glyphs in display order.
    fn shape(&self, line: &str) -> Vec<ShapedGlyph> {
        let Ok(fref) = harfrust::FontRef::from_index(&self.data, self.index) else { return Vec::new() };
        let shaper = self.shaper_data.shaper(&fref).build();
        let mut runs: Vec<Vec<ShapedGlyph>> = Vec::new();
        for run in direction_runs(line) {
            let mut buf = harfrust::UnicodeBuffer::new();
            buf.push_str(&run);
            buf.guess_segment_properties();
            let shaped = shaper.shape(buf, harfrust::ShapeOptions::new());
            runs.push(
                shaped
                    .glyph_infos()
                    .iter()
                    .zip(shaped.glyph_positions())
                    .map(|(i, p)| ShapedGlyph { id: i.glyph_id as u16, advance: p.x_advance as f32, x_off: p.x_offset as f32, y_off: p.y_offset as f32 })
                    .collect(),
            );
        }
        // A right-to-left line shows its runs from right to left.
        if self.lang.script.rtl() {
            runs.reverse();
        }
        runs.into_iter().flatten().collect()
    }

    /// Font units → pixels at text size `px`.
    fn unit(&self, px: f32) -> f32 {
        self.font.as_scaled(PxScale::from(px)).h_scale_factor()
    }

    fn text_width(&self, s: &str, px: f32) -> f32 {
        self.shape(s).iter().map(|g| g.advance).sum::<f32>() * self.unit(px)
    }

    /// Greedy wrap. Languages written with spaces break between words; Japanese, Chinese, Thai
    /// and the like may break between any two characters (keeping Latin words and numbers whole,
    /// and respecting NO_LINE_START).
    pub fn wrap(&self, text: &str, px: f32, max_w: f32) -> Vec<String> {
        let per_char = self.lang.no_spaces();
        let mut tokens: Vec<String> = Vec::new();
        for g in text.graphemes(true) {
            if g == "\n" || g == "\r\n" {
                tokens.push("\n".into());
                continue;
            }
            let is_space = g.chars().all(char::is_whitespace);
            let ascii_word = g.chars().all(|c| c.is_ascii_alphanumeric());
            match tokens.last_mut() {
                // A word runs until the next space (the space starts the next token).
                Some(t) if !per_char && !is_space && t != "\n" => t.push_str(g),
                Some(t) if per_char && ascii_word && t.chars().last().is_some_and(|l| l.is_ascii_alphanumeric()) => t.push_str(g),
                _ => tokens.push(g.to_string()),
            }
        }
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        for tok in tokens {
            if tok == "\n" {
                lines.push(std::mem::take(&mut cur));
                continue;
            }
            let candidate = format!("{cur}{tok}");
            let forbidden_start = tok.chars().next().is_some_and(|c| NO_LINE_START.contains(c));
            if cur.is_empty() || self.text_width(&candidate, px) <= max_w || forbidden_start {
                cur = candidate;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = tok.trim_start().to_string();
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines.into_iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
    }

    /// Renders one text box. The box covers the original text and grows if the translation
    /// needs more room, so text is never clipped.
    pub fn render_box(&self, text: &str, bbox: &BBox, frame_w: u32, frame_h: u32) -> Option<Placed> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let fw = frame_w as f32;
        let fh = frame_h as f32;
        // OCR boxes hug the letters, so pad generously to fully cover the English.
        let pad = (bbox.h * 0.22).clamp(6.0, 32.0);
        let mut bx = (bbox.x - pad).max(0.0);
        let mut by = (bbox.y - pad).max(0.0);
        let mut bw = (bbox.w + 2.0 * pad).min(fw - bx);
        let mut bh = (bbox.h + 2.0 * pad).min(fh - by);
        let min_px = (fh / 42.0).max(12.0);
        // Never let a box be narrower than a few characters.
        if bw < min_px * 4.0 + 2.0 * pad {
            let grow = min_px * 4.0 + 2.0 * pad - bw;
            bx = (bx - grow / 2.0).max(0.0);
            bw = (bw + grow).min(fw - bx);
        }
        let avail_w = (bw - 2.0 * pad).max(min_px);
        let avail_h = (bh - 2.0 * pad).max(min_px);

        // Largest size that fits (start near the English cap height, shrink 6% per step).
        let mut px = (bbox.h * 1.0).clamp(min_px, fh / 8.0);
        let mut lines = self.wrap(text, px, avail_w);
        loop {
            let total_h = lines.len() as f32 * px * 1.25;
            let widest = lines.iter().map(|l| self.text_width(l, px)).fold(0.0, f32::max);
            if (total_h <= avail_h && widest <= avail_w + 0.5) || px <= min_px {
                break;
            }
            px = (px * 0.94).max(min_px);
            lines = self.wrap(text, px, avail_w);
        }
        let line_h = px * 1.25;
        // Still too tall at the smallest size: widen the box first (up to 60% of the frame)
        // so the text keeps a readable shape instead of a tall narrow column.
        let mut avail_w = avail_w;
        let max_w = (fw * 0.6).max(avail_w);
        while lines.len() as f32 * line_h > avail_h && avail_w < max_w {
            avail_w = (avail_w * 1.25).min(max_w);
            lines = self.wrap(text, px, avail_w);
        }
        if avail_w + 2.0 * pad > bw {
            let need = avail_w + 2.0 * pad;
            let cx = bx + bw / 2.0;
            bx = (cx - need / 2.0).clamp(0.0, (fw - need).max(0.0));
            bw = need.min(fw - bx);
        }
        let total_h = lines.len() as f32 * line_h;
        let widest = lines.iter().map(|l| self.text_width(l, px)).fold(0.0, f32::max);
        // Grow the box when even the minimum size doesn't fit.
        if total_h > avail_h {
            let need = total_h + 2.0 * pad;
            let cy = by + bh / 2.0;
            by = (cy - need / 2.0).clamp(0.0, (fh - need).max(0.0));
            bh = need.min(fh - by);
        }
        if widest > avail_w {
            let need = widest + 2.0 * pad;
            let cx = bx + bw / 2.0;
            bx = (cx - need / 2.0).clamp(0.0, (fw - need).max(0.0));
            bw = need.min(fw - bx);
        }

        let iw = bw.ceil().max(1.0) as u32;
        let ih = bh.ceil().max(1.0) as u32;
        let mut img = RgbaImage::from_pixel(iw, ih, Rgba([0, 0, 0, 0]));
        let radius = (bh * 0.18).min(10.0);
        let a = (self.opacity * 255.0) as u8;
        for y in 0..ih {
            for x in 0..iw {
                let cover = rounded_cover(x as f32 + 0.5, y as f32 + 0.5, iw as f32, ih as f32, radius);
                if cover > 0.0 {
                    let [r, g, b] = self.box_rgb;
                    img.put_pixel(x, y, Rgba([r, g, b, (a as f32 * cover) as u8]));
                }
            }
        }

        let scaled = self.font.as_scaled(PxScale::from(px));
        let ascent = scaled.ascent();
        let top = (ih as f32 - total_h) / 2.0 + (line_h - (scaled.ascent() - scaled.descent())) / 2.0;
        let shadow = (px / 18.0).clamp(1.0, 3.0);
        let k = self.unit(px);
        for (li, line) in lines.iter().enumerate() {
            let glyphs = self.shape(line);
            let lw = glyphs.iter().map(|g| g.advance).sum::<f32>() * k;
            let x0 = (iw as f32 - lw) / 2.0;
            let baseline = top + ascent + li as f32 * line_h;
            for (offset, color, alpha) in [(shadow, [0u8, 0, 0], 0.65f32), (0.0, self.text, 1.0)] {
                let mut caret = x0 + offset;
                for g in &glyphs {
                    let pos = point(caret + g.x_off * k, baseline + offset - g.y_off * k);
                    caret += g.advance * k;
                    let glyph = ab_glyph::GlyphId(g.id).with_scale_and_position(PxScale::from(px), pos);
                    if let Some(outline) = self.font.outline_glyph(glyph) {
                        let bounds = outline.px_bounds();
                        outline.draw(|gx, gy, cov| {
                            let x = bounds.min.x as i64 + gx as i64;
                            let y = bounds.min.y as i64 + gy as i64;
                            if x >= 0 && y >= 0 && (x as u32) < iw && (y as u32) < ih {
                                blend(img.get_pixel_mut(x as u32, y as u32), color, cov * alpha);
                            }
                        });
                    }
                }
            }
        }
        Some(Placed { x: bx.round() as i64, y: by.round() as i64, img })
    }
}

fn rounded_cover(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let dx = if px < r { r - px } else if px > w - r { px - (w - r) } else { 0.0 };
    let dy = if py < r { r - py } else if py > h - r { py - (h - r) } else { 0.0 };
    if dx > 0.0 && dy > 0.0 { (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0) } else { 1.0 }
}

/// Source-over blend of a solid colour with coverage `a` onto a straight-alpha pixel.
fn blend(dst: &mut Rgba<u8>, rgb: [u8; 3], a: f32) {
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let da = dst.0[3] as f32 / 255.0;
    let out_a = a + da * (1.0 - a);
    for i in 0..3 {
        let c = (rgb[i] as f32 * a + dst.0[i] as f32 * da * (1.0 - a)) / out_a.max(1e-6);
        dst.0[i] = c.round().clamp(0.0, 255.0) as u8;
    }
    dst.0[3] = (out_a * 255.0).round() as u8;
}

fn blit(dst: &mut RgbaImage, p: &Placed) {
    for (x, y, px) in p.img.enumerate_pixels() {
        let tx = p.x + x as i64;
        let ty = p.y + y as i64;
        if tx < 0 || ty < 0 || tx as u32 >= dst.width() || ty as u32 >= dst.height() || px.0[3] == 0 {
            continue;
        }
        blend(dst.get_pixel_mut(tx as u32, ty as u32), [px.0[0], px.0[1], px.0[2]], px.0[3] as f32 / 255.0);
    }
}

fn write_png(img: &RgbaImage, path: &Path) -> Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let enc = image::codecs::png::PngEncoder::new_with_quality(
        file,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );
    enc.write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8)?;
    Ok(())
}

/// Writes one full-frame PNG per distinct "set of visible boxes" plus an ffconcat
/// list with their durations. Returns the list path (None when nothing to draw).
pub fn build_overlay_stream(
    renderer: &OverlayRenderer,
    intervals: &[TextInterval],
    width: u32,
    height: u32,
    duration: f64,
    dir: &Path,
    cancel: &CancelToken,
    mut on_frac: impl FnMut(f32),
) -> Result<Option<PathBuf>> {
    let items: Vec<(f64, f64, Placed)> = intervals
        .iter()
        .filter(|i| !i.japanese.trim().is_empty())
        .filter_map(|i| renderer.render_box(&i.japanese, &i.bbox, width, height).map(|p| (i.start, i.end, p)))
        .collect();
    if items.is_empty() {
        return Ok(None);
    }
    std::fs::create_dir_all(dir)?;

    let mut times: Vec<f64> = vec![0.0, duration];
    for (s, e, _) in &items {
        times.push(s.clamp(0.0, duration));
        times.push(e.clamp(0.0, duration));
    }
    times.sort_by(|a, b| a.total_cmp(b));
    times.dedup_by(|a, b| (*a - *b).abs() < 0.005);

    let blank = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));
    write_png(&blank, &dir.join("blank.png"))?;

    let mut list = String::from("ffconcat version 1.0\n");
    let mut last_file = "blank.png".to_string();
    let mut prev_key: Option<Vec<usize>> = None;
    let mut pending: Option<(String, f64)> = None; // file + accumulated duration
    let n = times.len().saturating_sub(1);
    for k in 0..n {
        cancel.check()?;
        on_frac(k as f32 / n.max(1) as f32);
        let (t0, t1) = (times[k], times[k + 1]);
        if t1 - t0 < 0.001 {
            continue;
        }
        let mid = (t0 + t1) / 2.0;
        let active: Vec<usize> = items.iter().enumerate().filter(|(_, (s, e, _))| *s <= mid && mid < *e).map(|(i, _)| i).collect();
        if prev_key.as_ref() == Some(&active) {
            if let Some(p) = pending.as_mut() {
                p.1 += t1 - t0;
            }
            continue;
        }
        if let Some((f, d)) = pending.take() {
            list.push_str(&format!("file '{f}'\nduration {d:.3}\n"));
        }
        let file = if active.is_empty() {
            "blank.png".to_string()
        } else {
            let name = format!("ov_{k:06}.png");
            let mut frame = blank.clone();
            for &i in &active {
                blit(&mut frame, &items[i].2);
            }
            write_png(&frame, &dir.join(&name))?;
            name
        };
        last_file = file.clone();
        pending = Some((file, t1 - t0));
        prev_key = Some(active);
    }
    if let Some((f, d)) = pending.take() {
        list.push_str(&format!("file '{f}'\nduration {d:.3}\n"));
    }
    // ffconcat needs the last file repeated for its duration to be honoured.
    // (The final entry has no duration line, so it adds no extra time.)
    list.push_str(&format!("file '{last_file}'\n"));
    let list_path = dir.join("overlays.ffconcat");
    let mut f = std::fs::File::create(&list_path)?;
    f.write_all(list.as_bytes())?;
    on_frac(1.0);
    Ok(Some(list_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders sample overlays onto a fake 1280x720 frame (written to target/overlay_preview.png)
    /// and checks that text never leaves its box. Skipped when no Japanese font is installed.
    #[test]
    fn renders_japanese_box_and_stream() {
        let Some(font) = crate::fonts::find_cjk_font("") else { return };
        let cfg = Config::default();
        let r = OverlayRenderer::new(&cfg, &font, crate::langs::japanese()).unwrap();
        let title = BBox { x: 240.0, y: 80.0, w: 800.0, h: 70.0 };
        let p = r.render_box("機械学習入門へようこそ", &title, 1280, 720).unwrap();
        assert!(p.img.width() >= 800 && p.img.height() >= 70);
        let tiny = BBox { x: 1180.0, y: 680.0, w: 60.0, h: 16.0 };
        let p2 = r.render_box("これはとても長い日本語の説明文で、元の英語より長くなります。", &tiny, 1280, 720).unwrap();
        assert!(p2.x >= 0 && p2.y >= 0);
        assert!(p2.x + p2.img.width() as i64 <= 1280 && p2.y + p2.img.height() as i64 <= 720);

        let mut frame = RgbaImage::from_pixel(1280, 720, Rgba([40, 90, 140, 255]));
        blit(&mut frame, &p);
        blit(&mut frame, &p2);
        let p3 = r.render_box("第1章：データの準備", &BBox { x: 80.0, y: 400.0, w: 420.0, h: 44.0 }, 1280, 720).unwrap();
        blit(&mut frame, &p3);
        let _ = std::fs::create_dir_all("target");
        frame.save("target/overlay_preview.png").unwrap();

        let dir = std::env::temp_dir().join("babelcraft-overlay-test");
        let iv = |s: f64, e: f64, ja: &str, b: BBox| TextInterval { start: s, end: e, bbox: b, english: "x".into(), japanese: ja.into() };
        let list = build_overlay_stream(
            &r,
            &[iv(1.0, 4.0, "こんにちは", title), iv(3.0, 6.0, "世界", tiny)],
            1280,
            720,
            8.0,
            &dir,
            &CancelToken::default(),
            |_| {},
        )
        .unwrap()
        .unwrap();
        let text = std::fs::read_to_string(list).unwrap();
        let total: f64 = text.lines().filter_map(|l| l.strip_prefix("duration ")).map(|d| d.parse::<f64>().unwrap()).sum();
        assert!((total - 8.0).abs() < 0.01, "durations add up to the video length: {text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Other languages: words stay whole, joined scripts are shaped, right-to-left runs are
    /// ordered, and every box stays inside the frame (written to target/overlay_languages.png).
    #[test]
    fn renders_other_languages() {
        let cfg = Config::default();
        let samples = [
            ("es", "Bienvenidos al curso de cocina: el arroz perfecto en veinte minutos"),
            ("ru", "Добро пожаловать на курс: идеальный рис"),
            ("ar", "مرحبا بكم في الدورة 2024"),
            ("hi", "पाठ्यक्रम में आपका स्वागत है"),
            ("th", "ยินดีต้อนรับสู่หลักสูตรการทำอาหาร"),
            ("ko", "요리 강좌에 오신 것을 환영합니다"),
            ("zh-Hans", "欢迎来到烹饪课程：完美的米饭"),
            ("en", "Welcome to the course"),
        ];
        let mut frame = RgbaImage::from_pixel(1280, 720, Rgba([40, 90, 140, 255]));
        for (i, (code, text)) in samples.iter().enumerate() {
            let lang = crate::langs::find(code).unwrap();
            let Some(font) = crate::fonts::find_overlay_font(lang, "") else { continue };
            let r = OverlayRenderer::new(&cfg, &font, lang).unwrap();
            let bbox = BBox { x: 40.0 + (i % 2) as f32 * 620.0, y: 30.0 + (i / 2) as f32 * 170.0, w: 560.0, h: 48.0 };
            let p = r.render_box(text, &bbox, 1280, 720).unwrap();
            assert!(p.x >= 0 && p.y >= 0 && p.x + p.img.width() as i64 <= 1280 && p.y + p.img.height() as i64 <= 720, "{code} box leaves the frame");
            blit(&mut frame, &p);
            if lang.script == crate::langs::Script::Latin {
                for line in r.wrap(text, 40.0, 300.0) {
                    assert!(line.split(' ').all(|w| text.contains(w)), "{code}: a word was split: {line}");
                }
            }
        }
        let _ = std::fs::create_dir_all("target");
        frame.save("target/overlay_languages.png").unwrap();
        assert_eq!(direction_runs("مرحبا 2024 abc مرحبا").len(), 3);
        assert_eq!(direction_runs("plain English").len(), 1);
    }
}
