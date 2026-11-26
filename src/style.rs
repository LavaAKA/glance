//! Icons and colors. Icons are Flaticon UIcons (regular rounded), trimmed to the glyphs Glance
//! uses (assets/uicons-rr.ttf). Free to use with credit: "Uicons by Flaticon" (Help > About).
use eframe::egui::{self, Color32};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

pub const I_FOLDER: &str = "\u{f6ba}";
pub const I_FOLDER_OPEN: &str = "\u{f6b5}";
pub const I_PICTURE: &str = "\u{fa78}";
pub const I_IMAGES: &str = "\u{f7fc}";
pub const I_PLAY: &str = "\u{fa9e}";
pub const I_PAUSE: &str = "\u{fa0c}";
pub const I_EXPAND: &str = "\u{f5f0}";
pub const I_INFO: &str = "\u{f80b}";
pub const I_SEARCH: &str = "\u{fbba}";
pub const I_ZOOM_IN: &str = "\u{ff2d}";
pub const I_ZOOM_OUT: &str = "\u{ff2e}";
pub const I_ROTATE_LEFT: &str = "\u{fb68}";
pub const I_ROTATE_RIGHT: &str = "\u{fb6a}";
pub const I_TRASH: &str = "\u{fe17}";
pub const I_COPY: &str = "\u{f4dd}";
pub const I_STAR: &str = "\u{fce9}";
pub const I_HEART: &str = "\u{f7a0}";
pub const I_SETTINGS: &str = "\u{fbd2}";
pub const I_LEFT: &str = "\u{f151}";
pub const I_RIGHT: &str = "\u{f152}";
pub const I_BACK: &str = "\u{f197}";
pub const I_SIDEBAR: &str = "\u{fc08}";
pub const I_GRID: &str = "\u{f71c}";
pub const I_APPS: &str = "\u{f16c}";
pub const I_SORT: &str = "\u{fc78}";
pub const I_FILTER: &str = "\u{f67e}";
pub const I_UNDO: &str = "\u{fe6b}";
pub const I_CROSS: &str = "\u{f4f7}";
pub const I_CHECK: &str = "\u{f3c8}";
pub const I_LOOP: &str = "\u{f1c4}";
pub const I_HOME: &str = "\u{f7b8}";
pub const I_DOWNLOAD: &str = "\u{f591}";
pub const I_VIDEO: &str = "\u{febe}";
pub const I_PIN: &str = "\u{fd95}";
pub const I_EYE: &str = "\u{f5f8}";
pub const I_PLUS: &str = "\u{faac}";
pub const I_MINUS: &str = "\u{f94e}";
pub const I_PENCIL: &str = "\u{fa29}";
pub const I_VOLUME: &str = "\u{fed0}";
pub const I_MUTE: &str = "\u{fecd}";
pub const I_FWD10: &str = "\u{fda6}";
pub const I_BACK10: &str = "\u{fdaa}";
pub const I_SQUARE: &str = "\u{fcd4}";
pub const I_CALENDAR: &str = "\u{f339}";
pub const I_PALETTE: &str = "\u{f9ed}";
pub const I_MOON: &str = "\u{f972}";
pub const I_SUN: &str = "\u{fd17}";
pub const I_MOVE: &str = "\u{f6b2}";
pub const I_COPY_TO: &str = "\u{f6b8}";
pub const I_EXIT: &str = "\u{f5ed}";
pub const I_HELP: &str = "\u{f81e}";
pub const I_PC: &str = "\u{f4cc}";
pub const I_DRIVE: &str = "\u{f786}";
pub const I_FLIP_H: &str = "\u{fb31}";
pub const I_FLIP_V: &str = "\u{fb32}";
pub const I_NEXT: &str = "\u{f19b}";
pub const I_CROP: &str = "\u{fba2}";
pub const I_CLOCK: &str = "\u{f45a}";
pub const I_EXTERNAL: &str = "\u{f1b3}";
pub const I_BRUSH: &str = "\u{f2f7}";

/// Adds the icon font as a fallback, so icons can sit inside any text: format!("{I_TRASH}  Delete").
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let data = egui::FontData::from_static(include_bytes!("../assets/uicons-rr.ttf")).tweak(egui::FontTweak {
        y_offset_factor: 0.12, // the glyphs sit a little high next to text
        ..Default::default()
    });
    fonts.font_data.insert("uicons".into(), Arc::new(data));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("uicons".into());
    }
    ctx.set_fonts(fonts);
}

/// The accent color is a user setting (View > Accent color), read by free drawing functions too.
static ACCENT: AtomicU32 = AtomicU32::new(0x608CC4);

pub fn accent() -> Color32 {
    let v = ACCENT.load(Ordering::Relaxed);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

static GLASS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Glass look: see-through panels over the Windows blur.
pub fn glass() -> bool {
    GLASS.load(Ordering::Relaxed)
}

pub fn set_glass(on: bool) {
    GLASS.store(on, Ordering::Relaxed);
}

/// Colour themes: the whole app takes on a hint of one of these, in light or dark.
pub const RAINBOW: [(&str, [u8; 3]); 7] = [
    ("Red", [214, 64, 64]),
    ("Orange", [232, 128, 40]),
    ("Yellow", [222, 186, 40]),
    ("Green", [64, 168, 88]),
    ("Blue", [56, 128, 220]),
    ("Indigo", [88, 88, 204]),
    ("Violet", [152, 88, 212]),
];

static TINT: AtomicU32 = AtomicU32::new(0); // 0 = none, else 0x01RRGGBB

pub fn tint() -> Option<[u8; 3]> {
    let v = TINT.load(Ordering::Relaxed);
    (v != 0).then(|| [(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn set_tint(c: Option<[u8; 3]>) {
    TINT.store(c.map_or(0, |[r, g, b]| u32::from_be_bytes([1, r, g, b])), Ordering::Relaxed);
}

/// `col` moved `t` of the way toward colour `c` (alpha kept).
pub fn tinted(col: Color32, c: [u8; 3], t: f32) -> Color32 {
    let [r, g, b, a] = col.to_srgba_unmultiplied();
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(mix(r, c[0]), mix(g, c[1]), mix(b, c[2]), a)
}

static MOTION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// How long an animation of `secs` should take: zero when Windows' "Animation effects" is off.
pub fn anim(secs: f32) -> f32 {
    if MOTION.load(Ordering::Relaxed) { secs } else { 0.0 }
}

pub fn set_motion(on: bool) {
    MOTION.store(on, Ordering::Relaxed);
}

pub fn set_accent([r, g, b]: [u8; 3]) {
    ACCENT.store(u32::from_be_bytes([0, r, g, b]), Ordering::Relaxed);
}

pub const ACCENTS: [(&str, [u8; 3]); 6] = [
    ("Blue", [96, 140, 196]),
    ("Teal", [56, 158, 150]),
    ("Green", [92, 164, 96]),
    ("Orange", [214, 128, 56]),
    ("Pink", [204, 92, 148]),
    ("Purple", [140, 108, 204]),
];

/// "icon  text" for buttons and menu items.
pub fn label(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn accent_round_trips() {
        super::set_accent([1, 2, 3]);
        assert_eq!(super::accent(), eframe::egui::Color32::from_rgb(1, 2, 3));
        super::set_accent(super::ACCENTS[0].1);
    }
}
