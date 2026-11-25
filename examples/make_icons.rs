//! Regenerates the icon files in `assets/` from `assets/icon-source.png`.
//!
//!     cargo run --example make_icons
//!
//! Finds the yellow rounded square in the source picture, crops it, gives it
//! clean transparent rounded corners and writes:
//!   assets/icon.png (512), assets/icon-256.png, assets/icon-1024.png, assets/icon.ico

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::imageops::FilterType;
use image::{ExtendedColorType, Rgba, RgbaImage};

fn is_yellow(p: &Rgba<u8>) -> bool {
    let [r, g, b, a] = p.0;
    a > 128 && r > 170 && g > 120 && b < 120 && r as i32 - b as i32 > 90
}

fn main() {
    let src = image::open("assets/icon-source.png").expect("assets/icon-source.png").to_rgba8();
    let (w, h) = src.dimensions();

    // Bounding box of the yellow square (rows/cols with enough yellow pixels).
    let col_has = |x: u32| (0..h).filter(|&y| is_yellow(src.get_pixel(x, y))).count() > (h as usize / 10);
    let row_has = |y: u32| (0..w).filter(|&x| is_yellow(src.get_pixel(x, y))).count() > (w as usize / 10);
    let x0 = (0..w).find(|&x| col_has(x)).unwrap_or(0);
    let x1 = (0..w).rev().find(|&x| col_has(x)).unwrap_or(w - 1);
    let y0 = (0..h).find(|&y| row_has(y)).unwrap_or(0);
    let y1 = (0..h).rev().find(|&y| row_has(y)).unwrap_or(h - 1);
    let side = (x1 - x0 + 1).max(y1 - y0 + 1);
    println!("yellow square: x {x0}..{x1}, y {y0}..{y1}, side {side}");

    // Crop exactly to the yellow tile and scale it to a true square (tiny stretch, no black bars).
    let crop = image::imageops::crop_imm(&src, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
    let mut sq: RgbaImage = image::imageops::resize(&crop, side, side, FilterType::Lanczos3);

    // Rounded-corner alpha mask (anti-aliased), slightly inset so no black rim remains.
    let inset = side as f32 * 0.006;
    let radius = side as f32 * 0.15;
    let s = side as f32;
    for y in 0..side {
        for x in 0..side {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let lo = inset + radius;
            let hi = s - inset - radius;
            let dx = if px < lo { lo - px } else if px > hi { px - hi } else { 0.0 };
            let dy = if py < lo { lo - py } else if py > hi { py - hi } else { 0.0 };
            let edge_x = px.min(s - px) - inset;
            let edge_y = py.min(s - py) - inset;
            let cover = if dx > 0.0 && dy > 0.0 {
                (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0)
            } else {
                (edge_x.min(edge_y) + 0.5).clamp(0.0, 1.0)
            };
            let p = sq.get_pixel_mut(x, y);
            p.0[3] = (p.0[3] as f32 * cover) as u8;
        }
    }

    let save = |size: u32, path: &str| {
        let img = image::imageops::resize(&sq, size, size, FilterType::Lanczos3);
        img.save(path).expect("save png");
        println!("wrote {path}");
    };
    save(512, "assets/icon.png");
    save(256, "assets/icon-256.png");
    save(1024, "assets/icon-1024.png");

    let frames: Vec<IcoFrame> = [16u32, 24, 32, 48, 64, 128, 256]
        .iter()
        .map(|&s| {
            let img = image::imageops::resize(&sq, s, s, FilterType::Lanczos3);
            IcoFrame::as_png(img.as_raw(), s, s, ExtendedColorType::Rgba8).expect("ico frame")
        })
        .collect();
    let file = std::fs::File::create("assets/icon.ico").expect("create ico");
    IcoEncoder::new(file).encode_images(&frames).expect("write ico");
    println!("wrote assets/icon.ico");
}
