//! Renders assets/icon.svg into assets/icon.png (256 px, window + home page) and assets/icon.ico
//! (16-256 px, the .exe and Start menu). Run: cargo run --example make_icon
use resvg::{tiny_skia, usvg};

fn render(tree: &usvg::Tree, side: u32) -> image::RgbaImage {
    let mut pixmap = tiny_skia::Pixmap::new(side, side).unwrap();
    let k = side as f32 / tree.size().width();
    resvg::render(tree, tiny_skia::Transform::from_scale(k, k), &mut pixmap.as_mut());
    let mut img = image::RgbaImage::from_raw(side, side, pixmap.take()).unwrap();
    // tiny-skia gives premultiplied alpha; PNG/ICO want straight alpha.
    for p in img.pixels_mut() {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            for c in 0..3 {
                p[c] = (p[c] as u32 * 255 / a).min(255) as u8;
            }
        }
    }
    img
}

fn main() {
    let svg = std::fs::read("assets/icon.svg").expect("run from the project folder");
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default()).unwrap();
    render(&tree, 256).save("assets/icon.png").unwrap();
    // Small sizes as classic 32-bit bitmaps (read by every Windows icon loader), 256 as PNG.
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 256];
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let img = render(&tree, s);
            if s == 256 {
                let mut png = vec![];
                img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
                png
            } else {
                dib(&img)
            }
        })
        .collect();
    let mut ico = vec![0, 0, 1, 0];
    ico.extend((sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, data) in sizes.iter().zip(&images) {
        let side = if *s >= 256 { 0 } else { *s as u8 };
        ico.extend([side, side, 0, 0, 1, 0, 32, 0]);
        ico.extend((data.len() as u32).to_le_bytes());
        ico.extend(offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for data in &images {
        ico.extend(data);
    }
    std::fs::write("assets/icon.ico", ico).unwrap();
    println!("wrote assets/icon.png and assets/icon.ico");
}

/// An icon entry as a DIB: header, bottom-up BGRA rows, then an all-clear AND mask.
fn dib(img: &image::RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mask_row = w.div_ceil(32) * 4;
    let mut out = vec![];
    for v in [40u32, w, h * 2] {
        out.extend(v.to_le_bytes());
    }
    out.extend(1u16.to_le_bytes()); // planes
    out.extend(32u16.to_le_bytes()); // bits per pixel
    for v in [0u32, w * h * 4 + mask_row * h, 0, 0, 0, 0] {
        out.extend(v.to_le_bytes());
    }
    for y in (0..h).rev() {
        for x in 0..w {
            let [r, g, b, a] = img.get_pixel(x, y).0;
            out.extend([b, g, r, a]);
        }
    }
    out.extend(std::iter::repeat_n(0u8, (mask_row * h) as usize));
    out
}
