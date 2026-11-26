//! Photo edits that need pixel math: light/color adjustments, straightening, crop shapes.
use eframe::egui::{Color32, ColorImage, Pos2, Rect, pos2};
use image::RgbaImage;

/// Slider values. All 0 = unchanged.
#[derive(Clone, Copy, PartialEq, Default)]
pub struct Adjust {
    pub brightness: f32, // -1..1
    pub contrast: f32,   // -1..1
    pub saturation: f32, // -1..1
    pub angle: f32,      // straighten, degrees, -45..45
}

impl Adjust {
    pub fn colors_changed(&self) -> bool {
        self.brightness != 0.0 || self.contrast != 0.0 || self.saturation != 0.0
    }
}

/// A lookup-free per-pixel adjustment, shared by the live preview and the saved copy.
fn adjust_rgb([r, g, b]: [u8; 3], a: &Adjust) -> [u8; 3] {
    let c = 1.0 + a.contrast; // 0..2
    let s = 1.0 + a.saturation; // 0..2
    let mut v = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
    for x in &mut v {
        *x = (*x + a.brightness * 0.5 - 0.5) * c + 0.5;
    }
    let luma = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
    v.map(|x| ((luma + (x - luma) * s).clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub fn adjust_image(img: &mut RgbaImage, a: &Adjust) {
    for p in img.pixels_mut() {
        let [r, g, b] = adjust_rgb([p[0], p[1], p[2]], a);
        (p[0], p[1], p[2]) = (r, g, b);
    }
}

pub fn adjust_color_image(img: &ColorImage, a: &Adjust) -> ColorImage {
    let pixels = img
        .pixels
        .iter()
        .map(|c| {
            let [r, g, b, al] = c.to_srgba_unmultiplied();
            let [r, g, b] = adjust_rgb([r, g, b], a);
            Color32::from_rgba_unmultiplied(r, g, b, al)
        })
        .collect();
    ColorImage::new(img.size, pixels)
}

/// A copy no bigger than `max` on its longest side (nearest pixel; it's only a preview).
pub fn downscale(img: &ColorImage, max: usize) -> ColorImage {
    let [w, h] = img.size;
    if w.max(h) <= max {
        return img.clone();
    }
    let k = max as f32 / w.max(h) as f32;
    let (tw, th) = (((w as f32 * k) as usize).max(1), ((h as f32 * k) as usize).max(1));
    let pixels = (0..tw * th).map(|i| img.pixels[(i / tw * h / th) * w + (i % tw * w / tw)]).collect();
    ColorImage::new([tw, th], pixels)
}

/// How much a w×h picture turned by `deg` must grow so it still covers its old frame (no empty corners).
pub fn cover_scale(w: f32, h: f32, deg: f32) -> f32 {
    let (s, c) = deg.to_radians().abs().sin_cos();
    ((w * c + h * s) / w).max((w * s + h * c) / h)
}

/// Turns the picture by `deg` (clockwise) and zooms just enough to hide the corners; same size out.
pub fn straighten(img: &RgbaImage, deg: f32) -> RgbaImage {
    if deg == 0.0 {
        return img.clone();
    }
    let (w, h) = (img.width() as f32, img.height() as f32);
    let k = cover_scale(w, h, deg);
    let (sin, cos) = deg.to_radians().sin_cos();
    let (cx, cy) = (w / 2.0, h / 2.0);
    RgbaImage::from_fn(img.width(), img.height(), |x, y| {
        // Inverse map: output pixel -> where it came from in the source.
        let (dx, dy) = ((x as f32 + 0.5 - cx) / k, (y as f32 + 0.5 - cy) / k);
        let (sx, sy) = (dx * cos + dy * sin + cx - 0.5, -dx * sin + dy * cos + cy - 0.5);
        bilinear(img, sx, sy)
    })
}

fn bilinear(img: &RgbaImage, x: f32, y: f32) -> image::Rgba<u8> {
    let (w, h) = (img.width() as i64 - 1, img.height() as i64 - 1);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let px = |xx: i64, yy: i64| img.get_pixel(xx.clamp(0, w) as u32, yy.clamp(0, h) as u32).0.map(|c| c as f32);
    let (a, b, c, d) = (px(x0 as i64, y0 as i64), px(x0 as i64 + 1, y0 as i64), px(x0 as i64, y0 as i64 + 1), px(x0 as i64 + 1, y0 as i64 + 1));
    image::Rgba(std::array::from_fn(|i| {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (d[i] - c[i]) * fx;
        (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8
    }))
}

/// A crop rectangle dragged from `a` to `b` (0..1 on a photo shown `shown_w`×`shown_h`), held to
/// `aspect` (width/height on screen) when given, and kept inside the photo.
pub fn crop_rect(a: Pos2, b: Pos2, aspect: Option<f32>, shown_w: f32, shown_h: f32) -> Rect {
    let Some(ratio) = aspect else { return Rect::from_two_pos(a, b) };
    let r = ratio * shown_h / shown_w; // the ratio in 0..1 units
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let mut w = dx.abs().max(dy.abs() * r);
    let mut h = w / r;
    // Room left from the anchor in the drag direction.
    let room_x = if dx < 0.0 { a.x } else { 1.0 - a.x };
    let room_y = if dy < 0.0 { a.y } else { 1.0 - a.y };
    let fit = (room_x / w).min(room_y / h).min(1.0);
    (w, h) = (w * fit, h * fit);
    Rect::from_two_pos(a, pos2(a.x + w.copysign(dx), a.y + h.copysign(dy)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_adjust_is_identity() {
        for c in [[0, 0, 0], [255, 255, 255], [10, 128, 240]] {
            assert_eq!(adjust_rgb(c, &Adjust::default()), c);
        }
        let grey = Adjust { saturation: -1.0, ..Default::default() };
        let [r, g, b] = adjust_rgb([200, 40, 40], &grey);
        assert!(r == g && g == b, "no saturation is grey");
        assert!(adjust_rgb([100, 100, 100], &Adjust { brightness: 0.5, ..Default::default() })[0] > 100);
    }

    #[test]
    fn straighten_keeps_size_and_zero_is_identity() {
        let img = RgbaImage::from_fn(40, 20, |x, y| image::Rgba([x as u8, y as u8, 0, 255]));
        assert_eq!(straighten(&img, 0.0), img);
        let s = straighten(&img, 10.0);
        assert_eq!(s.dimensions(), (40, 20));
        assert!(cover_scale(40.0, 20.0, 10.0) > 1.0);
        assert_eq!(cover_scale(40.0, 20.0, 0.0), 1.0);
    }

    #[test]
    fn crop_holds_its_shape() {
        // Square crop on a square photo, dragged wide: becomes square and stays inside.
        let r = crop_rect(pos2(0.5, 0.5), pos2(0.9, 0.6), Some(1.0), 100.0, 100.0);
        assert!((r.width() - r.height()).abs() < 1e-5);
        assert!(r.max.x <= 1.0 && r.max.y <= 1.0);
        // 16:9 on a 1600×900 view is a square in 0..1 units.
        let r = crop_rect(pos2(0.0, 0.0), pos2(0.5, 0.1), Some(16.0 / 9.0), 1600.0, 900.0);
        assert!((r.width() - r.height()).abs() < 1e-5);
        assert!((crop_rect(pos2(0.2, 0.2), pos2(0.4, 0.8), None, 1.0, 1.0).height() - 0.6).abs() < 1e-5);
    }
}
