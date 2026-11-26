// Glance: Photo and Video Viewer. Browse folders, view photos and videos big, and do light file management.
// Usage: glance [file-or-folder]. No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod decode;
mod defaults;
mod dragout;
mod edit;
mod files;
mod home;
mod instance;
mod sidebar;
mod receive;
mod style;
mod video;
mod windows_ui;

use decode::{Decoded, Meta};
use edit::Adjust;
use eframe::egui::{self, Color32, Key, Modifiers, Pos2, Rect, RichText, Sense, TextureHandle, Vec2, pos2, vec2};
use files::{Entry, SortKey, file_name};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::f32::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime};
use style::*;

const APP: &str = "Glance";
const STAR: Color32 = Color32::from_rgb(232, 180, 60);
const HEART: Color32 = Color32::from_rgb(220, 80, 90);
const CONVERT_FORMATS: [(&str, &str); 3] = [("JPEG", "jpg"), ("PNG", "png"), ("WebP (lossless)", "webp")];
const CONVERT_SIZES: [(&str, u32); 5] = [("Original size", 0), ("3840 px (4K)", 3840), ("2560 px", 2560), ("1920 px (HD)", 1920), ("1280 px", 1280)];
const STRIP_H: f32 = 60.0; // filmstrip height in the big view
const UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Rating {
    stars: u8,
    fav: bool,
    label: u8, // color label: 0 none, else an index into LABELS + 1
}

const LABELS: [(&str, Color32); 5] = [
    ("Red", Color32::from_rgb(220, 70, 70)),
    ("Orange", Color32::from_rgb(230, 140, 50)),
    ("Yellow", Color32::from_rgb(225, 200, 60)),
    ("Green", Color32::from_rgb(80, 170, 90)),
    ("Blue", Color32::from_rgb(70, 130, 220)),
];

#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
enum Filter {
    #[default]
    All,
    Photos,
    Videos,
    Favorites,
    ThreeStars,
    Labeled,
    Located,
    Duplicates,
}

impl Filter {
    const ALL: [(Filter, &str); 8] = [
        (Filter::All, "All files"),
        (Filter::Photos, "Photos"),
        (Filter::Videos, "Videos"),
        (Filter::Favorites, "Favorites"),
        (Filter::ThreeStars, "3 stars and up"),
        (Filter::Labeled, "Color labeled"),
        (Filter::Located, "Has location"),
        (Filter::Duplicates, "Look-alikes"),
    ];

    fn name(self) -> &'static str {
        Self::ALL.iter().find(|(f, _)| *f == self).map_or("", |(_, n)| n)
    }
}

/// How a photo is sized when it opens.
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
enum FitMode {
    #[default]
    Fit,
    Fill,
    Actual,
}

/// What shows around a photo that doesn't fill the window.
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
enum Backdrop {
    #[default]
    Black,
    Gray,
    Blur,
}

/// Light, dark, or whatever Windows is set to.
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

/// Installs our look for both themes and picks one; egui follows Windows when it's System.
fn apply_theme(ctx: &egui::Context, pref: ThemePref) {
    ctx.set_visuals_of(egui::Theme::Dark, visuals(false));
    ctx.set_visuals_of(egui::Theme::Light, visuals(true));
    ctx.set_theme(match pref {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Light => egui::ThemePreference::Light,
        ThemePref::Dark => egui::ThemePreference::Dark,
    });
}

/// What the Undo button on a toast puts back.
enum Undo {
    Restore(Vec<PathBuf>),           // from the Recycle Bin
    MoveBack(Vec<(PathBuf, PathBuf)>), // (original, where it went)
}

struct Toast {
    msg: String,
    at: Instant,
    undo: Option<Undo>,
}

/// Remembered between runs (eframe saves the window size and position itself).
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    recent: Vec<PathBuf>,
    theme: ThemePref,
    grid_size: f32,
    sort: SortKey,
    sort_desc: bool,
    filter: Filter,
    slide_secs: f32,
    ratings: HashMap<PathBuf, Rating>,
    last_target: Option<PathBuf>,
    volume: f32,
    muted: bool,
    video_loop: bool,
    accent: [u8; 3],
    pinned: Vec<PathBuf>,
    sidebar: bool,
    group_dates: bool,
    square_tiles: bool,
    seen_select_hint: bool,
    fit_mode: FitMode,
    backdrop: Backdrop,
    tags: HashMap<PathBuf, Vec<String>>,
    recent_files: Vec<PathBuf>, // recently viewed photos and videos
    subfolders: bool,           // grid shows files from subfolders too
    video_pos: HashMap<PathBuf, f64>, // where each unfinished video was left
    scroll: HashMap<PathBuf, f32>,    // grid scroll position per folder
    subtitles: bool,
    resume: bool,
    receive_dir: Option<PathBuf>, // where photos sent from a phone go
    glass: bool,                  // see-through glass look
    tint: Option<[u8; 3]>,        // colour theme (None = plain gray)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recent: vec![],
            theme: ThemePref::System,
            grid_size: 168.0,
            sort: SortKey::Name,
            sort_desc: false,
            filter: Filter::All,
            slide_secs: 3.0,
            ratings: HashMap::new(),
            last_target: None,
            volume: 0.8,
            muted: false,
            video_loop: false,
            accent: ACCENTS[0].1,
            pinned: vec![],
            sidebar: true,
            group_dates: false,
            square_tiles: false,
            seen_select_hint: false,
            fit_mode: FitMode::Fit,
            backdrop: Backdrop::Black,
            tags: HashMap::new(),
            recent_files: vec![],
            subfolders: false,
            video_pos: HashMap::new(),
            scroll: HashMap::new(),
            subtitles: true,
            resume: true,
            receive_dir: None,
            glass: false,
            tint: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Home,
    Grid,
    Single,
    Compare,
}

enum Slot {
    Loading,
    Ready(Arc<Decoded>),
    Failed(String),
}

enum Modal {
    Delete(Vec<PathBuf>),
    Rename { path: PathBuf, name: String, error: String, focus: bool },
    NewFolder { parent: PathBuf, name: String, error: String, focus: bool },
    BatchRename { paths: Vec<PathBuf>, pattern: String, error: String, focus: bool },
    Convert { paths: Vec<PathBuf>, format: usize, max_side: usize },
}

/// Results from background jobs.
enum Event {
    Toast(String, Option<Undo>),
    Moved(Vec<(PathBuf, PathBuf)>),
    Relist,
    Previews(Vec<(PathBuf, Vec<PathBuf>)>), // folder tile previews
}

/// Everything a menu item, key or button can ask for.
#[derive(Clone)]
enum Action {
    OpenFile,
    OpenFolder,
    Open(PathBuf),
    Back,
    Go(usize),
    Rename,
    Transfer { mv: bool },
    Delete,
    Reveal,
    OpenWith,
    Wallpaper,
    CopyImage,
    CopyPath,
    Rotate(i32),
    FlipH,
    FlipV,
    ToggleEdit,
    SaveCopy,
    SelectAll,
    ClearSelection,
    Fullscreen,
    Info,
    Loupe,
    ActualSize,
    Fit,
    Slideshow,
    Compare,
    Rate(u8),
    Fav,
    PlayPause,
    Seek(f64), // seconds relative to now
    ZoomBy(f32),
    SetFit(FitMode),
    Pin,
    Sidebar,
    GroupDates,
    About,
    Clean,
    Label(u8),
    BatchRename,
    Convert,
    Subfolders,
    SaveFrame,
    TrimMark(bool), // true = start
    TrimSave,
    Pip,
    Map,
    ShowOnMap,
    Keys,
    Settings,
    Phone,
    Glass,
    NewFolder(PathBuf), // inside this folder
    DeleteFolder(PathBuf),
    SetTheme(ThemePref),
    ColorTheme(Option<[u8; 3]>, ThemePref),
    Mute,
    Volume(f32),
    Speed(i32),
    FocusSearch,
}

type DecodeResult = (PathBuf, Result<Decoded, String>);

struct Viewer {
    s: Settings,
    mode: Mode,
    folder: Option<PathBuf>,
    all: Vec<Entry>,     // every image in the folder
    dirs: Vec<PathBuf>,  // subfolders
    files: Vec<PathBuf>, // filtered + sorted: what the grid shows and arrows step through
    times: Vec<SystemTime>, // modified time of each entry in `files` (for date grouping)
    index: usize,

    // How the current photo is shown. zoom/pan are targets; shown_* ease toward them.
    zoom: f32,      // 1.0 = fit to window
    pan: Vec2,
    shown_zoom: f32,
    shown_pan: Vec2,
    swipe: f32,     // horizontal drag offset while swiping to the next photo
    apply_fit: bool, // size the next photo by s.fit_mode once it has loaded
    fade_from: Option<(Arc<Decoded>, Instant)>, // previous photo, cross-fading out
    px_scale: f32,  // screen pixels per original pixel at zoom 1 (for the % readout)
    rot: i32,       // quarter turns clockwise
    flip_h: bool,
    flip_v: bool,
    editing: bool,
    crop: Option<Rect>, // normalized to the displayed photo
    anim_start: Instant,
    readout: Option<String>,

    filter_text: String,
    selected: HashSet<PathBuf>,
    anchor: Option<usize>,
    compare: Option<[PathBuf; 2]>,
    scroll_to_current: bool,

    grid_cols: usize,
    band: Option<Pos2>, // rubber-band selection start
    jump_to: Option<usize>, // date section to scroll to
    hover: Option<(PathBuf, Instant)>, // video tile under the mouse, for the hover preview
    hover_player: Option<(PathBuf, video::Player)>,
    blur: Option<(PathBuf, TextureHandle)>, // blurred backdrop of the photo on screen
    meta: HashMap<PathBuf, Meta>,
    live: HashMap<PathBuf, PathBuf>, // photo -> its Live Photo video
    previews: HashMap<PathBuf, Vec<PathBuf>>, // folder -> files shown on its tile
    restore_scroll: Option<f32>,
    meta_dirty: bool, // new metadata arrived; re-filter
    drag_out: Option<Vec<PathBuf>>,
    clean: bool, // nothing but the photo
    minimap: bool,
    swipe_acc: f32, // touchpad sideways scroll, toward the next photo
    adjust: Adjust,
    aspect: Option<f32>, // crop shape
    edit_base: Option<(PathBuf, egui::ColorImage, Option<TextureHandle>, Adjust)>, // preview source, adjusted texture, for
    trim: (Option<f64>, Option<f64>),
    subs: Option<(PathBuf, Vec<video::Cue>)>,
    pending_resume: Option<f64>,
    pip: Option<(PathBuf, video::Player)>,
    scrub: Option<(PathBuf, video::Player, f64, Instant)>, // seek-bar preview: path, player, last seek, when
    tag_edit: Option<(PathBuf, String)>,
    show_keys: bool,
    phone: Option<Result<receive::Server, String>>, // the Receive from phone window, while open
    phone_qr: Option<TextureHandle>,
    received: Vec<PathBuf>,
    view_key: (Option<PathBuf>, bool), // (folder, big view): a change fades the new screen in
    view_since: Instant,
    folder_rects: Vec<(PathBuf, Rect)>, // folder tiles on screen, for dropping photos on
    own_drop: Option<(Vec<PathBuf>, Pos2, Instant)>, // our last drag-out: what, where it was let go, when
    hwnd: isize,                         // this window, for the Windows blur
    backdrop: Option<(bool, bool)>,      // (glass, dark) last given to Windows
    show_settings: bool,
    app_icon: Option<TextureHandle>,
    strip_lift: f32, // how far the filmstrip covers the bottom of the view right now
    focus_search: bool,

    thumbs: HashMap<PathBuf, TextureHandle>,
    durations: HashMap<PathBuf, f64>,
    thumb_jobs: Vec<decode::ThumbJob>,
    tree: HashMap<PathBuf, Vec<PathBuf>>, // sidebar: subfolders of expanded folders
    cache: HashMap<PathBuf, Slot>,
    player: Option<(PathBuf, Result<video::Player, String>)>, // only while a video is on screen
    decode_tx: mpsc::Sender<DecodeResult>,
    decode_rx: mpsc::Receiver<DecodeResult>,
    events_tx: mpsc::Sender<Event>,
    events_rx: mpsc::Receiver<Event>,
    instance_rx: Option<mpsc::Receiver<Option<PathBuf>>>,
    data_dir: Option<PathBuf>,

    fullscreen: bool,
    show_info: bool,
    loupe: bool,
    slideshow: Option<Instant>,
    modal: Option<Modal>,
    toast: Option<Toast>,
    show_about: bool,
    last_activity: Instant, // mouse/keyboard, for auto-hiding the bars
    speed: f64,
    last_poll: Instant,
    title: String,
}

fn visuals(light: bool) -> egui::Visuals {
    let mut v = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
    if light {
        // Soft gray chrome with near-white cards, like Windows' own light apps.
        v.panel_fill = Color32::from_gray(243);
        v.window_fill = Color32::from_gray(250);
        v.faint_bg_color = Color32::from_gray(251);
        v.extreme_bg_color = Color32::from_gray(226);
        v.widgets.noninteractive.bg_stroke.color = Color32::from_gray(214);
        v.widgets.inactive.weak_bg_fill = Color32::from_gray(229);
        v.widgets.hovered.weak_bg_fill = Color32::from_gray(218);
        v.widgets.active.weak_bg_fill = Color32::from_gray(206);
    } else {
        v.panel_fill = Color32::from_gray(26);
        v.faint_bg_color = Color32::from_gray(32);
        v.extreme_bg_color = Color32::from_gray(14);
        v.widgets.inactive.weak_bg_fill = Color32::from_gray(48);
        v.widgets.hovered.weak_bg_fill = Color32::from_gray(62);
    }
    if glass() {
        // Frosted glass: see-through fills over the Windows blur, with a thin bright rim on controls.
        let (tint, ink): (fn(u8) -> Color32, fn(u8) -> Color32) =
            if light { (|a| Color32::from_rgba_unmultiplied(248, 248, 252, a), Color32::from_black_alpha) } else { (|a| Color32::from_rgba_unmultiplied(22, 22, 28, a), Color32::from_white_alpha) };
        v.panel_fill = tint(if light { 150 } else { 120 });
        v.window_fill = tint(if light { 215 } else { 200 });
        v.faint_bg_color = ink(10);
        v.extreme_bg_color = ink(16);
        v.code_bg_color = ink(12);
        let rim = if light { Color32::from_white_alpha(170) } else { Color32::from_white_alpha(30) };
        v.widgets.noninteractive.bg_stroke.color = ink(if light { 30 } else { 26 });
        for (w, a) in [(&mut v.widgets.inactive, 18), (&mut v.widgets.hovered, 32), (&mut v.widgets.active, 46), (&mut v.widgets.open, 32)] {
            w.weak_bg_fill = ink(a);
            w.bg_fill = ink(a);
            w.bg_stroke = egui::Stroke::new(1.0, rim);
        }
        v.window_stroke = egui::Stroke::new(1.0, rim);
        v.window_corner_radius = 14.into();
        v.menu_corner_radius = 10.into();
        v.window_shadow = egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(70) };
    }
    if let Some(c) = tint() {
        let t = if light { 0.13 } else { 0.11 };
        let w = &mut v.widgets;
        for col in [
            &mut v.panel_fill,
            &mut v.window_fill,
            &mut v.faint_bg_color,
            &mut v.extreme_bg_color,
            &mut v.code_bg_color,
            &mut v.window_stroke.color,
            &mut w.noninteractive.bg_stroke.color,
            &mut w.inactive.weak_bg_fill,
            &mut w.inactive.bg_fill,
            &mut w.hovered.weak_bg_fill,
            &mut w.hovered.bg_fill,
            &mut w.active.weak_bg_fill,
            &mut w.active.bg_fill,
            &mut w.open.weak_bg_fill,
        ] {
            *col = tinted(*col, c, t);
        }
    }
    v.selection.bg_fill = accent();
    v.hyperlink_color = accent();
    let w = &mut v.widgets;
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = 3.into();
    }
    v
}

/// Where a photo of texture size `tex` lands in `rect`: (unrotated paint rect, on-screen rect, fit scale).
fn layout(rect: Rect, tex: Vec2, rot: i32, zoom: f32, pan: Vec2) -> (Rect, Rect, f32) {
    let shown = if rot % 2 == 1 { vec2(tex.y, tex.x) } else { tex };
    let fit = (rect.width() / shown.x).min(rect.height() / shown.y).min(1.0);
    let center = rect.center() + pan;
    let s = fit * zoom;
    (Rect::from_center_size(center, tex * s), Rect::from_center_size(center, shown * s), fit)
}

/// Maps a 0..1 position on the displayed (rotated/flipped) photo back to 0..1 texture coordinates.
fn display_to_texture(mut u: f32, mut v: f32, rot: i32, flip_h: bool, flip_v: bool) -> (f32, f32) {
    for _ in 0..rot.rem_euclid(4) {
        (u, v) = (v, 1.0 - u); // undo one clockwise quarter turn
    }
    (if flip_h { 1.0 - u } else { u }, if flip_v { 1.0 - v } else { v })
}

/// Like paint_texture, but turned by any angle (radians, clockwise).
#[allow(clippy::too_many_arguments)]
fn paint_turned(painter: &egui::Painter, tex: &TextureHandle, rect: Rect, mut uv: Rect, turn: f32, flip_h: bool, flip_v: bool, tint: Color32) {
    if flip_h {
        std::mem::swap(&mut uv.min.x, &mut uv.max.x);
    }
    if flip_v {
        std::mem::swap(&mut uv.min.y, &mut uv.max.y);
    }
    let mut mesh = egui::Mesh::with_texture(tex.id());
    mesh.add_rect_with_uv(rect, uv, tint);
    if turn != 0.0 {
        mesh.rotate(egui::emath::Rot2::from_angle(turn), rect.center());
    }
    painter.add(mesh);
}

/// Draws `tex` into `rect`, flipped (in texture space) and then rotated clockwise about the center.
#[allow(clippy::too_many_arguments)]
fn paint_texture(
    painter: &egui::Painter,
    tex: &TextureHandle,
    rect: Rect,
    uv: Rect,
    rot: i32,
    flip_h: bool,
    flip_v: bool,
    tint: Color32,
) {
    paint_turned(painter, tex, rect, uv, rot as f32 * FRAC_PI_2, flip_h, flip_v, tint);
}

/// Takes `paths` back out of the Recycle Bin (the newest copy of each). Returns how many came back.
#[cfg(windows)]
fn restore_from_bin(paths: &[PathBuf]) -> Result<usize, String> {
    let mut items = trash::os_limited::list().map_err(|e| e.to_string())?;
    items.sort_by_key(|i| std::cmp::Reverse(i.time_deleted));
    let mut picked: Vec<trash::TrashItem> = vec![];
    for p in paths {
        if let Some(i) = items.iter().find(|i| i.original_path() == *p && !picked.iter().any(|x| x.id == i.id)) {
            picked.push(i.clone());
        }
    }
    let n = picked.len();
    trash::os_limited::restore_all(picked).map_err(|e| e.to_string())?;
    Ok(n)
}

#[cfg(not(windows))]
fn restore_from_bin(_: &[PathBuf]) -> Result<usize, String> {
    Err("Restore from the trash isn't supported here".into())
}

/// Groups fingerprints that differ in at most a few bits. Returns each one's group (the index of
/// its first member), or None when nothing else looks like it.
// shortcut: compares every pair; fine for a few thousand photos, use a BK-tree beyond ~20k.
fn group_alike(hashes: &[u64]) -> Vec<Option<usize>> {
    let mut group: Vec<Option<usize>> = vec![None; hashes.len()];
    for i in 0..hashes.len() {
        for j in i + 1..hashes.len() {
            if group[j].is_none() && (hashes[i] ^ hashes[j]).count_ones() <= 5 {
                let g = *group[i].get_or_insert(i);
                group[j] = Some(g);
            }
        }
    }
    group
}

/// Apple Live Photos: IMG_1234.HEIC with IMG_1234.MOV beside it. Photo -> its video.
fn live_pairs(all: &[Entry]) -> HashMap<PathBuf, PathBuf> {
    let key = |p: &Path| p.with_extension("").to_string_lossy().to_lowercase();
    let videos: HashMap<String, &PathBuf> = all.iter().filter(|e| video::is_video(&e.path)).map(|e| (key(&e.path), &e.path)).collect();
    all.iter()
        .filter(|e| !video::is_video(&e.path))
        .filter_map(|e| Some((e.path.clone(), (*videos.get(&key(&e.path))?).clone())))
        .collect()
}

/// A `width`-pixel-wide copy where each pixel averages its whole cell: stretched back up, it looks blurred.
fn tiny_copy(img: &egui::ColorImage, width: usize) -> egui::ColorImage {
    let [w, h] = img.size;
    let (tw, th) = (width.min(w).max(1), (width * h / w.max(1)).clamp(1, h.max(1)));
    let mut out = Vec::with_capacity(tw * th);
    for ty in 0..th {
        for tx in 0..tw {
            // 6×6 samples per cell is plenty and stays fast on 8K photos.
            let mut sum = [0u32; 3];
            for sy in 0..6 {
                for sx in 0..6 {
                    let x = ((tx * 6 + sx) * w / (tw * 6)).min(w - 1);
                    let y = ((ty * 6 + sy) * h / (th * 6)).min(h - 1);
                    let c = img.pixels[y * w + x];
                    sum[0] += c.r() as u32;
                    sum[1] += c.g() as u32;
                    sum[2] += c.b() as u32;
                }
            }
            out.push(Color32::from_rgb((sum[0] / 36) as u8, (sum[1] / 36) as u8, (sum[2] / 36) as u8));
        }
    }
    egui::ColorImage::new([tw, th], out)
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

/// "9 photos, 2 videos" (or just one of them).
fn media_count(files: &[PathBuf]) -> String {
    let videos = files.iter().filter(|p| video::is_video(p)).count();
    let photos = files.len() - videos;
    match (photos, videos) {
        (_, 0) => plural(photos, "photo"),
        (0, _) => plural(videos, "video"),
        _ => format!("{}, {}", plural(photos, "photo"), plural(videos, "video")),
    }
}

/// "Today", "Yesterday", or "October 2026": the date section a file falls in.
fn date_section(t: SystemTime) -> String {
    let t: chrono::DateTime<chrono::Local> = t.into();
    let days = (chrono::Local::now().date_naive() - t.date_naive()).num_days();
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => t.format("%B %Y").to_string(),
    }
}

/// Single-line text cut short with "…" to fit `width`.
fn truncated(painter: &egui::Painter, text: &str, width: f32, size: f32, color: Color32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), egui::FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
}

/// A menu entry with an icon that queues `act` when clicked.
fn item(ui: &mut egui::Ui, acts: &mut Vec<Action>, enabled: bool, icon: &str, text: &str, keys: &str, act: Action) {
    if ui.add_enabled(enabled, egui::Button::new(label(icon, text)).shortcut_text(keys)).clicked() {
        acts.push(act);
    }
}

/// A menu entry with an on/off state: the icon is replaced by a check mark while on.
#[allow(clippy::too_many_arguments)]
fn toggle(ui: &mut egui::Ui, acts: &mut Vec<Action>, enabled: bool, on: bool, icon: &str, text: &str, keys: &str, act: Action) {
    let b = egui::Button::new(label(if on { I_CHECK } else { icon }, text)).shortcut_text(keys).selected(on);
    if ui.add_enabled(enabled, b).clicked() {
        acts.push(act);
    }
}

/// The main button of a dialog, in the accent color.
fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(accent())
}

/// Minimize, maximize/restore and close, drawn by Glance since the window has no system frame.
fn window_buttons(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ui.spacing_mut().item_spacing.x = 0.0;
    // Right to left: close first.
    for kind in 0..3 {
        let (rect, resp) = ui.allocate_exact_size(vec2(46.0, 32.0), Sense::click());
        let close = kind == 0;
        let hot = resp.hovered();
        if hot {
            let bg = if close { Color32::from_rgb(196, 43, 28) } else { ui.visuals().widgets.hovered.weak_bg_fill };
            ui.painter().rect_filled(rect, 0.0, bg);
        }
        let color = if close && hot { Color32::WHITE } else { ui.visuals().text_color() };
        let stroke = egui::Stroke::new(1.0, color);
        let c = rect.center();
        let p = ui.painter();
        match kind {
            0 => {
                p.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], stroke);
                p.line_segment([c + vec2(5.0, -5.0), c + vec2(-5.0, 5.0)], stroke);
            }
            1 if maxed => {
                // Restore: two overlapping squares.
                p.rect_stroke(Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(8.0, 8.0)), 1.0, stroke, egui::StrokeKind::Middle);
                p.line_segment([c + vec2(-3.0, -4.5), c + vec2(4.5, -4.5)], stroke);
                p.line_segment([c + vec2(4.5, -4.5), c + vec2(4.5, 3.0)], stroke);
            }
            1 => {
                p.rect_stroke(Rect::from_center_size(c, vec2(10.0, 10.0)), 1.0, stroke, egui::StrokeKind::Middle);
            }
            _ => {
                p.line_segment([c + vec2(-5.0, 0.0), c + vec2(5.0, 0.0)], stroke);
            }
        }
        let tip = match kind {
            0 => "Close",
            1 if maxed => "Restore down",
            1 => "Maximize",
            _ => "Minimize",
        };
        if resp.on_hover_text(tip).clicked() {
            ctx.send_viewport_cmd(match kind {
                0 => egui::ViewportCommand::Close,
                1 => egui::ViewportCommand::Maximized(!maxed),
                _ => egui::ViewportCommand::Minimized(true),
            });
        }
    }
}

/// Without a system frame the window needs its own resize grips: a thin band along each edge
/// and corner, plus a hairline border so the window edge shows against the desktop.
fn resize_edges(ui: &mut egui::Ui, full: Rect, fullscreen: bool) {
    let ctx = ui.ctx().clone();
    if fullscreen || ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    use egui::ResizeDirection as D;
    let (w, c) = (5.0, 12.0); // edge thickness, corner size
    let r = full;
    let grips = [
        (Rect::from_min_max(r.min, pos2(r.min.x + c, r.min.y + c)), D::NorthWest, egui::CursorIcon::ResizeNorthWest),
        (Rect::from_min_max(pos2(r.max.x - c, r.min.y), pos2(r.max.x, r.min.y + c)), D::NorthEast, egui::CursorIcon::ResizeNorthEast),
        (Rect::from_min_max(pos2(r.min.x, r.max.y - c), pos2(r.min.x + c, r.max.y)), D::SouthWest, egui::CursorIcon::ResizeSouthWest),
        (Rect::from_min_max(r.max - vec2(c, c), r.max), D::SouthEast, egui::CursorIcon::ResizeSouthEast),
        (Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + w)), D::North, egui::CursorIcon::ResizeNorth),
        (Rect::from_min_max(pos2(r.min.x, r.max.y - w), r.max), D::South, egui::CursorIcon::ResizeSouth),
        (Rect::from_min_max(r.min, pos2(r.min.x + w, r.max.y)), D::West, egui::CursorIcon::ResizeWest),
        (Rect::from_min_max(pos2(r.max.x - w, r.min.y), r.max), D::East, egui::CursorIcon::ResizeEast),
    ];
    for (i, (rect, dir, cursor)) in grips.into_iter().enumerate() {
        let resp = ui.interact(rect, egui::Id::unique(("resize", i)), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ctx.set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }
    let border = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("window_border")));
    let edge = if ui.visuals().dark_mode { Color32::from_gray(70) } else { Color32::from_gray(175) };
    border.rect_stroke(full, 0.0, egui::Stroke::new(1.0, edge), egui::StrokeKind::Inside);
}

/// Color label entries (Rate menu and right-click).
fn label_items(ui: &mut egui::Ui, acts: &mut Vec<Action>, enabled: bool) {
    for (i, (name, color)) in LABELS.iter().enumerate() {
        let text = RichText::new(format!("●  {name}")).color(*color);
        let b = egui::Button::new(text).shortcut_text(format!("Alt+{}", i + 1));
        if ui.add_enabled(enabled, b).clicked() {
            acts.push(Action::Label(i as u8 + 1));
        }
    }
}

/// A small icon-only button with a tooltip.
fn icon_button(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(icon).size(15.0))).on_hover_text(tip)
}

/// What a thumbnail tile shows besides the picture.
#[derive(Default)]
struct TileInfo<'a> {
    label: Option<&'a str>, // file name under the tile (grid only)
    rating: Rating,
    current: bool,
    selected: bool,
    video: bool,
    cover: bool,               // crop to fill the tile instead of letterboxing
    duration: Option<f64>,     // video length badge
    live: bool,                // Live Photo: has a short video beside it
    draggable: bool,           // can be dragged out to other apps
}

/// One clickable thumbnail.
fn thumb_tile(ui: &mut egui::Ui, size: Vec2, tex: Option<&TextureHandle>, info: TileInfo) -> egui::Response {
    let TileInfo { label, rating, current, selected, video, cover, duration, live, draggable } = info;
    let (rect, resp) = ui.allocate_exact_size(size, if draggable { Sense::click_and_drag() } else { Sense::click() });
    // Only paint what's on screen; big folders have thousands of these.
    if ui.is_rect_visible(rect) {
        let vis = ui.visuals();
        let painter = ui.painter_at(rect);
        let img = if label.is_some() { rect.with_max_y(rect.max.y - 20.0) } else { rect };
        painter.rect_filled(img, 2.0, vis.extreme_bg_color);
        // Thumbnails that finish loading while on screen fade in rather than pop.
        let shown = ui.ctx().animate_bool_with_time(resp.id.with("loaded"), tex.is_some(), anim(0.25));
        let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), anim(0.12));
        if let Some(t) = tex {
            let s = t.size_vec2();
            let tint = Color32::WHITE.gamma_multiply(shown);
            if cover {
                // Show the middle part that fills the tile, like a phone gallery.
                let k = (img.width() / s.x).max(img.height() / s.y);
                let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(img.width() / (s.x * k), img.height() / (s.y * k)));
                painter.image(t.id(), img, uv, tint);
            } else {
                let fit = Rect::from_center_size(img.center(), s * ((img.width() - 2.0) / s.x).min((img.height() - 2.0) / s.y));
                painter.image(t.id(), fit, UV, tint);
            }
        }
        if let Some(d) = duration.filter(|_| label.is_some()) {
            let g = painter.layout_no_wrap(video::clock(d), egui::FontId::proportional(11.0), Color32::WHITE);
            let r = Rect::from_min_size(img.left_bottom() + vec2(5.0, -g.size().y - 9.0), g.size() + vec2(8.0, 4.0));
            painter.rect_filled(r, 3.0, Color32::from_black_alpha(170));
            painter.galley(r.min + vec2(4.0, 2.0), g, Color32::WHITE);
        }
        if video {
            // Play badge so videos stand out from photos.
            let r = (img.height() * 0.18).clamp(7.0, 22.0);
            let c = img.center();
            painter.circle_filled(c, r, Color32::from_black_alpha(160));
            let tri = vec![c + vec2(-r * 0.35, -r * 0.5), c + vec2(r * 0.55, 0.0), c + vec2(-r * 0.35, r * 0.5)];
            painter.add(egui::Shape::convex_polygon(tri, Color32::WHITE, egui::Stroke::NONE));
        }
        if live && label.is_some() {
            let g = painter.layout_no_wrap("LIVE".into(), egui::FontId::proportional(10.0), Color32::WHITE);
            let r = Rect::from_min_size(img.left_bottom() + vec2(5.0, -g.size().y - 9.0), g.size() + vec2(8.0, 4.0));
            painter.rect_filled(r, 3.0, Color32::from_black_alpha(170));
            painter.galley(r.min + vec2(4.0, 2.0), g, Color32::WHITE);
        }
        if rating.label > 0 && label.is_some() {
            let c = LABELS[(rating.label as usize - 1).min(4)].1;
            painter.circle_filled(img.right_top() + vec2(if rating.fav { -26.0 } else { -11.0 }, 11.0), 5.0, c);
        }
        if let Some(name) = label {
            let color = if current || selected { vis.strong_text_color() } else { vis.weak_text_color() };
            let g = truncated(&painter, name, rect.width() - 2.0, 12.0, color);
            painter.galley(pos2(rect.min.x + 1.0, rect.max.y - 3.0 - g.size().y), g, color);
            let font = egui::FontId::proportional(13.0);
            if rating.stars > 0 {
                let stars = "★".repeat(rating.stars as usize);
                let pos = img.right_bottom() + vec2(-5.0, -4.0);
                painter.text(pos + vec2(1.0, 1.0), egui::Align2::RIGHT_BOTTOM, &stars, font.clone(), Color32::BLACK);
                painter.text(pos, egui::Align2::RIGHT_BOTTOM, &stars, font.clone(), STAR);
            }
            if rating.fav {
                let pos = img.right_top() + vec2(-5.0, 4.0);
                painter.text(pos + vec2(1.0, 1.0), egui::Align2::RIGHT_TOP, "♥", font.clone(), Color32::BLACK);
                painter.text(pos, egui::Align2::RIGHT_TOP, "♥", font, HEART);
            }
        }
        if selected {
            painter.rect_filled(img, 2.0, accent().gamma_multiply(0.25));
            painter.rect_stroke(img, 2.0, egui::Stroke::new(2.0, accent()), egui::StrokeKind::Inside);
            let badge = Rect::from_min_size(img.min + vec2(5.0, 5.0), vec2(18.0, 18.0));
            painter.rect_filled(badge, 9.0, accent());
            painter.text(badge.center(), egui::Align2::CENTER_CENTER, I_CHECK, egui::FontId::proportional(11.0), Color32::WHITE);
        } else if current {
            painter.rect_stroke(img, 2.0, egui::Stroke::new(2.0, accent().gamma_multiply(0.7)), egui::StrokeKind::Inside);
        } else if hover > 0.0 {
            painter.rect_stroke(img, 2.0, egui::Stroke::new(1.0, Color32::from_gray(128).gamma_multiply(hover)), egui::StrokeKind::Inside);
        }
    }
    resp
}

/// A folder in the grid: a simple folder shape with its name underneath.
fn folder_tile(ui: &mut egui::Ui, size: Vec2, name: &str, previews: &[Option<&TextureHandle>]) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let vis = ui.visuals();
        let painter = ui.painter_at(rect);
        let area = rect.with_max_y(rect.max.y - 20.0);
        painter.rect_filled(area, 2.0, vis.extreme_bg_color);
        let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), anim(0.12));
        if !previews.is_empty() {
            // A mosaic of what's inside, with a folder badge so it doesn't pass for a photo.
            let cells: Vec<Rect> = match previews.len() {
                1 => vec![area],
                2 | 3 => vec![area.with_max_x(area.center().x - 1.0), area.with_min_x(area.center().x + 1.0)],
                _ => {
                    let c = area.center();
                    vec![
                        Rect::from_min_max(area.min, c - vec2(1.0, 1.0)),
                        Rect::from_min_max(pos2(c.x + 1.0, area.min.y), pos2(area.max.x, c.y - 1.0)),
                        Rect::from_min_max(pos2(area.min.x, c.y + 1.0), pos2(c.x - 1.0, area.max.y)),
                        Rect::from_min_max(c + vec2(1.0, 1.0), area.max),
                    ]
                }
            };
            for (cell, tex) in cells.iter().zip(previews) {
                if let Some(t) = tex {
                    let s = t.size_vec2();
                    let k = (cell.width() / s.x).max(cell.height() / s.y);
                    let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(cell.width() / (s.x * k), cell.height() / (s.y * k)));
                    painter.image(t.id(), *cell, uv, Color32::from_gray(215 + (40.0 * hover) as u8));
                }
            }
            let badge = Rect::from_min_size(area.left_bottom() + vec2(5.0, -27.0), vec2(22.0, 22.0));
            painter.rect_filled(badge, 3.0, Color32::from_black_alpha(180));
            painter.text(badge.center(), egui::Align2::CENTER_CENTER, I_FOLDER, egui::FontId::proportional(13.0), Color32::WHITE);
            let g = truncated(&painter, name, rect.width() - 2.0, 12.0, vis.text_color());
            painter.galley(pos2(rect.min.x + 1.0, rect.max.y - 3.0 - g.size().y), g, vis.text_color());
            if hover > 0.0 {
                painter.rect_stroke(area, 2.0, egui::Stroke::new(1.0, accent().gamma_multiply(hover)), egui::StrokeKind::Inside);
            }
            return resp;
        }
        let body = Rect::from_center_size(area.center() + vec2(0.0, 4.0), vec2(area.width() * 0.42, area.height() * 0.42));
        let tab = Rect::from_min_size(body.min - vec2(0.0, body.height() * 0.16), vec2(body.width() * 0.4, body.height() * 0.2));
        let color = Color32::from_gray(120).lerp_to_gamma(accent(), hover);
        painter.rect_filled(tab, 2.0, color.gamma_multiply(0.8));
        painter.rect_filled(body, 2.0, color);
        let g = truncated(&painter, name, rect.width() - 2.0, 12.0, vis.text_color());
        painter.galley(pos2(rect.min.x + 1.0, rect.max.y - 3.0 - g.size().y), g, vis.text_color());
    }
    resp
}

impl Viewer {
    fn new(cc: &eframe::CreationContext) -> Self {
        let s: Settings = cc.storage.and_then(|st| eframe::get_value(st, "settings")).unwrap_or_default();
        install_fonts(&cc.egui_ctx);
        set_motion(windows_animations());
        cc.egui_ctx.all_styles_mut(|st| st.animation_time = anim(0.15));
        set_accent(s.accent);
        set_glass(s.glass);
        set_tint(s.tint);
        apply_theme(&cc.egui_ctx, s.theme);
        let (decode_tx, decode_rx) = mpsc::channel();
        let (events_tx, events_rx) = mpsc::channel();
        Self {
            s,
            mode: Mode::Home,
            folder: None,
            all: vec![],
            dirs: vec![],
            files: vec![],
            times: vec![],
            index: 0,
            zoom: 1.0,
            pan: Vec2::ZERO,
            shown_zoom: 1.0,
            shown_pan: Vec2::ZERO,
            swipe: 0.0,
            apply_fit: false,
            fade_from: None,
            px_scale: 1.0,
            rot: 0,
            flip_h: false,
            flip_v: false,
            editing: false,
            crop: None,
            anim_start: Instant::now(),
            readout: None,
            filter_text: String::new(),
            selected: HashSet::new(),
            anchor: None,
            compare: None,
            scroll_to_current: false,
            grid_cols: 1,
            band: None,
            jump_to: None,
            hover: None,
            hover_player: None,
            blur: None,
            meta: HashMap::new(),
            live: HashMap::new(),
            previews: HashMap::new(),
            restore_scroll: None,
            meta_dirty: false,
            drag_out: None,
            clean: false,
            minimap: true,
            swipe_acc: 0.0,
            adjust: Adjust::default(),
            aspect: None,
            edit_base: None,
            trim: (None, None),
            subs: None,
            pending_resume: None,
            pip: None,
            scrub: None,
            tag_edit: None,
            show_keys: false,
            phone: None,
            phone_qr: None,
            received: vec![],
            view_key: (None, false),
            view_since: Instant::now(),
            folder_rects: vec![],
            own_drop: None,
            hwnd: window_handle(cc),
            backdrop: None,
            show_settings: false,
            app_icon: None,
            strip_lift: 0.0,
            focus_search: false,
            thumbs: HashMap::new(),
            durations: HashMap::new(),
            thumb_jobs: vec![],
            tree: HashMap::new(),
            cache: HashMap::new(),
            player: None,
            decode_tx,
            decode_rx,
            events_tx,
            events_rx,
            instance_rx: instance::listen(cc.egui_ctx.clone()),
            data_dir: eframe::storage_dir(APP),
            fullscreen: false,
            show_info: false,
            loupe: false,
            slideshow: None,
            modal: None,
            toast: None,
            show_about: false,
            last_activity: Instant::now(),
            speed: 1.0,
            last_poll: Instant::now(),
            title: String::new(),
        }
    }

    fn current(&self) -> Option<&PathBuf> {
        self.files.get(self.index)
    }

    fn rating(&self, p: &Path) -> Rating {
        self.s.ratings.get(p).copied().unwrap_or_default()
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast { msg: msg.into(), at: Instant::now(), undo: None });
    }

    fn toast_undo(&mut self, msg: impl Into<String>, undo: Option<Undo>) {
        self.toast = Some(Toast { msg: msg.into(), at: Instant::now(), undo });
    }

    /// Puts back what the last delete or move did.
    fn undo(&mut self, ctx: &egui::Context, undo: Undo) {
        match undo {
            Undo::Restore(paths) => self.toast(match restore_from_bin(&paths) {
                Ok(n) => format!("Restored {}", plural(n, "file")),
                Err(e) => format!("Couldn't restore: {e}"),
            }),
            Undo::MoveBack(pairs) => {
                let mut failed = 0;
                for (orig, now) in &pairs {
                    // Back to where it was; never overwrite something that has appeared there since.
                    if orig.exists() || std::fs::rename(now, orig).is_err() {
                        failed += 1;
                    }
                }
                self.toast(if failed == 0 {
                    format!("Moved {} back", plural(pairs.len(), "file"))
                } else {
                    format!("{failed} couldn't be moved back")
                });
            }
        }
        self.relist(ctx);
    }

    /// What file actions apply to: the grid selection, or the photo on screen.
    fn targets(&self) -> Vec<PathBuf> {
        match self.mode {
            Mode::Grid => self.files.iter().filter(|p| self.selected.contains(*p)).cloned().collect(),
            Mode::Single => self.current().cloned().into_iter().collect(),
            _ => vec![],
        }
    }

    fn open(&mut self, ctx: &egui::Context, path: &Path) {
        let Ok(path) = std::path::absolute(path) else { return };
        let (folder, select) = if path.is_dir() {
            (path, None)
        } else {
            match path.parent() {
                Some(dir) if path.is_file() => (dir.to_path_buf(), Some(path)),
                _ => return self.toast(format!("Can't find {}", path.display())),
            }
        };
        self.load_folder(ctx, &folder);
        self.s.recent.retain(|p| *p != folder);
        self.s.recent.insert(0, folder);
        self.s.recent.truncate(10);
        self.mode = Mode::Grid;
        if let Some(p) = select {
            match self.files.iter().position(|f| *f == p) {
                Some(i) => {
                    self.go(i);
                    self.mode = Mode::Single;
                    self.viewed();
                }
                None => self.toast(format!("{} isn't a supported image", file_name(&p))),
            }
        }
    }

    /// The folder's files (and its subfolders' too, when that's on) and its subfolders.
    fn list(&self, dir: &Path) -> (Vec<Entry>, Vec<PathBuf>) {
        let (files, dirs) = files::list_dir(dir);
        if self.s.subfolders { (files::list_deep(dir), dirs) } else { (files, dirs) }
    }

    /// Finds what each folder tile shows, off the UI thread (it reads every subfolder).
    fn load_previews(&self, ctx: &egui::Context) {
        let (dirs, tx, ctx) = (self.dirs.clone(), self.events_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let found = dirs.into_iter().map(|d| {
                let f = files::preview_files(&d);
                (d, f)
            });
            let _ = tx.send(Event::Previews(found.filter(|(_, f)| !f.is_empty()).collect()));
            ctx.request_repaint();
        });
    }

    fn load_folder(&mut self, ctx: &egui::Context, dir: &Path) {
        let (all, dirs) = self.list(dir);
        self.folder = Some(dir.to_path_buf());
        self.thumbs.clear();
        self.meta.clear();
        self.previews.clear();
        let paths = all.iter().map(|e| e.path.clone()).collect();
        self.thumb_jobs = vec![decode::spawn_thumbs(ctx, paths, self.thumb_dir())];
        self.all = all;
        self.dirs = dirs;
        self.selected.clear();
        self.anchor = None;
        self.filter_text.clear();
        self.index = 0;
        self.refresh_view(None);
        self.go(0);
        self.load_previews(ctx);
        // Back where you were the last time you looked at this folder.
        self.restore_scroll = self.s.scroll.get(dir).copied();
        if self.restore_scroll.is_some() {
            self.scroll_to_current = false;
        }
        self.last_poll = Instant::now();
    }

    /// Adds the file on screen to Recently viewed.
    fn viewed(&mut self) {
        if let Some(p) = self.current().cloned() {
            self.s.recent_files.retain(|r| *r != p);
            self.s.recent_files.insert(0, p);
            self.s.recent_files.truncate(12);
        }
    }

    fn thumb_dir(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join("thumbs"))
    }

    /// Re-reads the folder (picks up files added/removed by other programs or our own jobs).
    // shortcut: compares names only; a file edited in place keeps its old thumbnail until reopened.
    fn relist(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.folder.clone().filter(|d| d.is_dir()) else { return };
        let (all, dirs) = self.list(&dir);
        let old: HashSet<&PathBuf> = self.all.iter().map(|e| &e.path).collect();
        let new: Vec<PathBuf> = all.iter().filter(|e| !old.contains(&e.path)).map(|e| e.path.clone()).collect();
        if new.is_empty() && all.len() == old.len() && dirs == self.dirs {
            return;
        }
        if !new.is_empty() {
            self.thumb_jobs.push(decode::spawn_thumbs(ctx, new, self.thumb_dir()));
        }
        let dirs_changed = dirs != self.dirs;
        if dirs_changed {
            self.tree.remove(&dir); // the sidebar re-lists this folder's subfolders
        }
        self.all = all;
        self.dirs = dirs;
        if dirs_changed {
            self.load_previews(ctx);
        }
        let present: HashSet<PathBuf> = self.all.iter().map(|e| e.path.clone()).collect();
        self.selected.retain(|p| present.contains(p));
        self.refresh_view(None);
    }

    fn passes(&self, e: &Entry, dupes: &HashMap<PathBuf, usize>) -> bool {
        let r = self.rating(&e.path);
        let meta = self.meta.get(&e.path);
        let kept = match self.s.filter {
            Filter::All => true,
            Filter::Photos => !video::is_video(&e.path),
            Filter::Videos => video::is_video(&e.path),
            Filter::Favorites => r.fav,
            Filter::ThreeStars => r.stars >= 3,
            Filter::Labeled => r.label > 0,
            Filter::Located => meta.is_some_and(|m| m.gps.is_some()),
            Filter::Duplicates => dupes.contains_key(&e.path),
        };
        kept && (self.filter_text.is_empty() || self.matches_search(e))
    }

    /// Every word must appear in the name, camera, date (taken or modified), file type or a tag.
    fn matches_search(&self, e: &Entry) -> bool {
        let meta = self.meta.get(&e.path);
        let modified: chrono::DateTime<chrono::Local> = e.modified.into();
        let mut hay = file_name(&e.path).to_lowercase();
        for extra in [meta.and_then(|m| m.camera.clone()), meta.and_then(|m| m.taken.clone())].into_iter().flatten() {
            hay += &format!(" {}", extra.to_lowercase());
        }
        hay += &modified.format(" %Y-%m-%d %B").to_string().to_lowercase();
        hay += if video::is_video(&e.path) { " video" } else { " photo" };
        for tag in self.s.tags.get(&e.path).into_iter().flatten() {
            hay += &format!(" {}", tag.to_lowercase());
        }
        self.filter_text.to_lowercase().split_whitespace().all(|w| hay.contains(w))
    }

    /// Files that look like another file in the folder -> a group number shared by look-alikes.
    fn look_alikes(&self) -> HashMap<PathBuf, usize> {
        let hashes: Vec<(PathBuf, u64)> =
            self.all.iter().filter_map(|e| Some((e.path.clone(), self.meta.get(&e.path)?.hash?))).collect();
        let groups = group_alike(&hashes.iter().map(|h| h.1).collect::<Vec<_>>());
        hashes.into_iter().zip(groups).filter_map(|((p, _), g)| Some((p, g?))).collect()
    }

    /// When a file was taken (EXIF), or else last modified.
    fn when(&self, e: &Entry) -> SystemTime {
        let taken = self.meta.get(&e.path).and_then(|m| m.taken.as_deref()).and_then(|t| {
            let t = t.replacen(':', "-", 2); // EXIF writes the date as 2024:07:14
            chrono::NaiveDateTime::parse_from_str(&t, "%Y-%m-%d %H:%M").ok()
        });
        taken.and_then(|t| t.and_local_timezone(chrono::Local).single()).map_or(e.modified, SystemTime::from)
    }

    /// Rebuilds the visible list after filtering/sorting changes, keeping `keep` (or the current photo) selected.
    fn refresh_view(&mut self, keep: Option<PathBuf>) {
        let keep = keep.or_else(|| self.current().cloned());
        let dupes = if self.s.filter == Filter::Duplicates { self.look_alikes() } else { HashMap::new() };
        let mut view: Vec<Entry> = self.all.iter().filter(|e| self.passes(e, &dupes)).cloned().collect();
        files::sort(&mut view, self.s.sort, self.s.sort_desc);
        if self.s.group_dates {
            // Date headings only make sense in date order: newest first, by date taken.
            view.sort_by_cached_key(|e| std::cmp::Reverse(self.when(e)));
        }
        if !dupes.is_empty() {
            view.sort_by_key(|e| dupes[&e.path]); // look-alikes next to each other
        }
        self.times = view.iter().map(|e| self.when(e)).collect();
        self.files = view.into_iter().map(|e| e.path).collect();
        self.live = live_pairs(&self.all);
        self.index = keep
            .and_then(|k| self.files.iter().position(|p| *p == k))
            .unwrap_or(self.index.min(self.files.len().saturating_sub(1)));
        if self.files.is_empty() && self.mode == Mode::Single {
            self.mode = Mode::Grid;
        }
    }

    fn go(&mut self, i: usize) {
        let i = i.min(self.files.len().saturating_sub(1));
        // Cross-fade from the photo on screen to the next one.
        if i != self.index && self.mode == Mode::Single {
            self.fade_from = self.decoded().map(|d| (d, Instant::now()));
        }
        self.index = i;
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.shown_zoom = 1.0;
        self.shown_pan = Vec2::ZERO;
        self.swipe = 0.0;
        self.apply_fit = true;
        self.last_activity = Instant::now(); // show the controls for a moment on each new photo
        self.rot = 0;
        self.flip_h = false;
        self.flip_v = false;
        self.editing = false;
        self.crop = None;
        self.adjust = Adjust::default();
        self.edit_base = None;
        self.trim = (None, None);
        self.anim_start = Instant::now();
        self.scroll_to_current = true;
        if self.slideshow.is_some() {
            self.slideshow = Some(Instant::now());
        }
    }

    /// Esc / Back: leave fullscreen, then edit -> photo -> folder -> empty screen.
    fn back(&mut self, ctx: &egui::Context) {
        if self.fullscreen {
            return self.run(ctx, Action::Fullscreen);
        }
        if self.clean {
            self.clean = false;
            return;
        }
        match self.mode {
            Mode::Single if self.editing => {
                self.editing = false;
                self.crop = None;
                self.adjust = Adjust::default();
                self.edit_base = None;
            }
            Mode::Single | Mode::Compare => {
                self.mode = Mode::Grid;
                self.slideshow = None;
                self.scroll_to_current = true;
            }
            Mode::Grid if !self.selected.is_empty() => self.selected.clear(),
            Mode::Grid => self.go_home(),
            Mode::Home => {}
        }
    }

    /// The home page, with nothing open.
    fn go_home(&mut self) {
        self.mode = Mode::Home;
        self.folder = None;
        self.all.clear();
        self.dirs.clear();
        self.files.clear();
        self.selected.clear();
        self.thumbs.clear();
        self.thumb_jobs.clear();
        self.cache.clear();
        (self.editing, self.clean, self.slideshow, self.modal) = (false, false, None, None);
    }

    fn rekey(&mut self, old: &Path, new: &Path) {
        if let Some(r) = self.s.ratings.remove(old) {
            self.s.ratings.insert(new.to_path_buf(), r);
        }
        if let Some(t) = self.thumbs.remove(old) {
            self.thumbs.insert(new.to_path_buf(), t);
        }
        if self.selected.remove(old) {
            self.selected.insert(new.to_path_buf());
        }
        if let Some(t) = self.s.tags.remove(old) {
            self.s.tags.insert(new.to_path_buf(), t);
        }
        if let Some(m) = self.meta.remove(old) {
            self.meta.insert(new.to_path_buf(), m);
        }
        self.cache.remove(old);
    }

    /// Sends `paths` to the Recycle Bin, or erases them for good when `permanent`.
    fn delete(&mut self, paths: Vec<PathBuf>, permanent: bool) {
        // A video that's playing keeps its file open; let go of it first.
        if self.player.as_ref().is_some_and(|(p, _)| paths.iter().any(|d| p.starts_with(d))) {
            self.player = None;
        }
        let had_folders = paths.iter().any(|p| p.is_dir());
        let mut failed = None;
        let paths: Vec<PathBuf> = if permanent {
            let erase = |p: &PathBuf| if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
            paths.into_iter().filter(|p| erase(p).map_err(|e| failed = Some(e.to_string())).is_ok()).collect()
        } else if let Err(e) = trash::delete_all(&paths) {
            return self.toast(format!("Couldn't delete: {e}"));
        } else {
            paths
        };
        if let Some(e) = &failed {
            self.toast(format!("Couldn't delete some files: {e}"));
        }
        if paths.is_empty() {
            return;
        }
        if had_folders {
            self.tree.clear(); // the sidebar lists folders afresh
            let _ = self.events_tx.send(Event::Relist);
        }
        let gone: HashSet<&PathBuf> = paths.iter().collect();
        // Land on the next photo that survived (or the previous one at the end).
        let (after, before) = self.files.split_at(self.index.min(self.files.len()));
        let keep = after.iter().chain(before.iter().rev()).find(|p| !gone.contains(p)).cloned();
        self.all.retain(|e| !gone.contains(&e.path));
        for p in &paths {
            self.s.ratings.remove(p);
            self.thumbs.remove(p);
            self.cache.remove(p);
            self.selected.remove(p);
        }
        self.refresh_view(keep);
        self.go(self.index);
        if permanent {
            if failed.is_none() {
                self.toast(format!("Deleted {} permanently", plural(paths.len(), "file")));
            }
        } else {
            let msg = format!("Moved {} to the Recycle Bin", plural(paths.len(), "file"));
            self.toast_undo(msg, Some(Undo::Restore(paths)));
        }
    }

    fn rename(&mut self, path: &Path, name: &str) -> Result<(), String> {
        let new = files::rename_file(path, name)?;
        self.rekey(path, &new);
        for e in &mut self.all {
            if e.path == path {
                e.path = new.clone();
            }
        }
        self.refresh_view(Some(new));
        Ok(())
    }

    fn decoded(&self) -> Option<Arc<Decoded>> {
        match self.cache.get(self.current()?)? {
            Slot::Ready(d) => Some(d.clone()),
            _ => None,
        }
    }

    fn run(&mut self, ctx: &egui::Context, action: Action) {
        let single = self.mode == Mode::Single;
        match action {
            Action::OpenFile => {
                let exts: Vec<&str> = files::EXTS.iter().chain(video::EXTS).copied().collect();
                if let Some(p) = rfd::FileDialog::new().add_filter("Photos and videos", &exts).pick_file() {
                    self.open(ctx, &p);
                }
            }
            Action::OpenFolder => {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.open(ctx, &p);
                }
            }
            Action::Open(p) => self.open(ctx, &p),
            Action::Back => self.back(ctx),
            Action::Go(i) => {
                self.go(i);
                self.mode = Mode::Single;
                self.viewed();
            }
            Action::DeleteFolder(d) => self.modal = Some(Modal::Delete(vec![d])),
            Action::NewFolder(parent) => {
                self.modal = Some(Modal::NewFolder { parent, name: "New folder".into(), error: String::new(), focus: true });
            }
            Action::Rename => {
                if let [p] = self.targets().as_slice() {
                    let name = file_name(p);
                    self.modal = Some(Modal::Rename { path: p.clone(), name, error: String::new(), focus: true });
                }
            }
            Action::Delete => {
                let t = self.targets();
                if !t.is_empty() {
                    self.modal = Some(Modal::Delete(t));
                }
            }
            Action::Transfer { mv } => self.transfer(ctx, mv),
            Action::Reveal => self.targets().first().into_iter().for_each(|p| files::reveal(p)),
            Action::OpenWith => self.targets().first().into_iter().for_each(|p| files::open_with(p)),
            Action::Wallpaper => self.wallpaper(ctx),
            Action::CopyImage => self.copy_image(),
            Action::CopyPath => {
                let t = self.targets();
                if !t.is_empty() {
                    ctx.copy_text(t.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
                    self.toast(if t.len() == 1 { "Copied path".into() } else { format!("Copied {} paths", t.len()) });
                }
            }
            Action::Rotate(d) if single => self.rot = (self.rot + d).rem_euclid(4),
            Action::FlipH if single => self.flip_h = !self.flip_h,
            Action::FlipV if single => self.flip_v = !self.flip_v,
            Action::PlayPause => {
                if let Some((_, Ok(p))) = &self.player {
                    p.toggle();
                }
            }
            Action::ToggleEdit if single && !self.current().is_some_and(|p| video::is_video(p)) => {
                self.editing = !self.editing;
                self.crop = None;
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
                self.slideshow = None;
            }
            Action::SaveCopy if single => self.save_copy(ctx),
            Action::SelectAll if self.mode == Mode::Grid => self.selected = self.files.iter().cloned().collect(),
            Action::ClearSelection => self.selected.clear(),
            Action::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
            Action::Info => self.show_info = !self.show_info,
            Action::Loupe => self.loupe = !self.loupe,
            Action::ActualSize if single => {
                // Toggle between 100% (one photo pixel per screen pixel) and fit.
                let at_100 = (self.px_scale * self.zoom - 1.0).abs() < 0.01;
                self.zoom = if at_100 { 1.0 } else { (1.0 / self.px_scale).max(1.0) };
                self.pan = Vec2::ZERO;
            }
            Action::Fit => {
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
                self.s.fit_mode = FitMode::Fit;
            }
            Action::Slideshow if single => {
                self.slideshow = if self.slideshow.is_some() { None } else { Some(Instant::now()) };
            }
            Action::Mute => {
                self.s.muted = !self.s.muted;
                if let Some((_, Ok(p))) = &self.player {
                    p.set_muted(self.s.muted);
                }
                self.toast(if self.s.muted { "Muted" } else { "Sound on" });
            }
            Action::Volume(by) => {
                self.s.volume = (self.s.volume + by).clamp(0.0, 1.0);
                self.s.muted = false;
                if let Some((_, Ok(p))) = &self.player {
                    p.set_volume(self.s.volume as f64);
                    p.set_muted(false);
                }
                self.toast(format!("Volume {:.0}%", self.s.volume * 100.0));
            }
            Action::Speed(step) => {
                const SPEEDS: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
                let i = SPEEDS.iter().position(|s| *s == self.speed).unwrap_or(2) as i32;
                self.speed = SPEEDS[(i + step).clamp(0, 5) as usize];
                if let Some((_, Ok(p))) = &self.player {
                    p.set_rate(self.speed);
                }
                self.toast(format!("Speed {}×", self.speed));
            }
            Action::FocusSearch => self.focus_search = true,
            Action::ColorTheme(c, t) => {
                self.s.tint = c;
                set_tint(c);
                self.s.accent = c.unwrap_or(ACCENTS[0].1); // buttons and highlights match the colour
                set_accent(self.s.accent);
                self.s.theme = t;
                apply_theme(ctx, t);
            }
            Action::SetTheme(t) => {
                self.s.theme = t;
                apply_theme(ctx, t);
            }
            Action::Compare => {
                let picked: Vec<PathBuf> = self.files.iter().filter(|p| self.selected.contains(*p)).cloned().collect();
                if let [a, b] = picked.as_slice() {
                    self.compare = Some([a.clone(), b.clone()]);
                    self.mode = Mode::Compare;
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                }
            }
            Action::Rate(stars) => {
                for p in self.targets() {
                    self.s.ratings.entry(p).or_default().stars = stars;
                }
                self.after_rating();
            }
            Action::Fav => {
                let t = self.targets();
                let all_fav = t.iter().all(|p| self.rating(p).fav);
                for p in t {
                    self.s.ratings.entry(p).or_default().fav = !all_fav;
                }
                self.after_rating();
            }
            Action::Seek(by) => {
                if let Some((_, Ok(p))) = &self.player {
                    p.seek((p.time() + by).clamp(0.0, p.duration()));
                }
            }
            Action::ZoomBy(f) if single => {
                self.zoom = (self.zoom * f).clamp(1.0, 40.0);
                if self.zoom == 1.0 {
                    self.pan = Vec2::ZERO;
                }
            }
            Action::SetFit(m) => {
                self.s.fit_mode = m;
                self.apply_fit = true;
            }
            Action::Pin => {
                if let Some(f) = self.folder.clone() {
                    if self.s.pinned.contains(&f) {
                        self.s.pinned.retain(|p| *p != f);
                    } else {
                        self.s.pinned.push(f);
                    }
                }
            }
            Action::Sidebar => self.s.sidebar = !self.s.sidebar,
            Action::GroupDates => {
                self.s.group_dates = !self.s.group_dates;
                self.refresh_view(None);
            }
            Action::About => self.show_about = true,
            Action::Clean => self.clean = !self.clean,
            Action::Label(l) => {
                let t = self.targets();
                let same = t.iter().all(|p| self.rating(p).label == l);
                for p in t {
                    self.s.ratings.entry(p).or_default().label = if same { 0 } else { l };
                }
                self.after_rating();
            }
            Action::BatchRename => {
                let paths = self.targets();
                if !paths.is_empty() {
                    self.modal = Some(Modal::BatchRename { paths, pattern: "*_###".into(), error: String::new(), focus: true });
                }
            }
            Action::Convert => {
                let paths: Vec<PathBuf> = self.targets().into_iter().filter(|p| !video::is_video(p)).collect();
                if !paths.is_empty() {
                    self.modal = Some(Modal::Convert { paths, format: 0, max_side: 0 });
                }
            }
            Action::Subfolders => {
                self.s.subfolders = !self.s.subfolders;
                if let Some(f) = self.folder.clone() {
                    self.load_folder(ctx, &f);
                }
            }
            Action::SaveFrame => self.save_frame(ctx),
            Action::TrimMark(start) => {
                if let Some((_, Ok(p))) = &self.player {
                    let t = p.time();
                    if start { self.trim.0 = Some(t) } else { self.trim.1 = Some(t) }
                }
            }
            Action::TrimSave => self.save_trim(ctx),
            Action::Pip => match self.pip.take() {
                // Back to the main window, at the same moment.
                Some((p, pl)) => self.remember_position(p, pl.time(), pl.duration()),
                None => {
                    if let Some((p, Ok(pl))) = self.player.take() {
                        self.pip = Some((p, pl));
                    }
                }
            },
            Action::Map => self.open_map(ctx),
            Action::ShowOnMap => {
                if let Some((lat, lon)) = self.current().and_then(|p| self.meta.get(p)?.gps) {
                    let url = format!("https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=15/{lat:.6}/{lon:.6}");
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
            }
            Action::Keys => self.show_keys = !self.show_keys,
            Action::Settings => self.show_settings = !self.show_settings,
            Action::Glass => {
                self.s.glass = !self.s.glass;
                set_glass(self.s.glass);
                apply_theme(ctx, self.s.theme);
            }
            Action::Phone => {
                self.phone = match self.phone.take() {
                    Some(_) => None, // closing stops the server
                    None => {
                        self.received.clear();
                        Some(receive::start(self.receive_dir(), ctx.clone()))
                    }
                };
                self.phone_qr = None;
            }
            _ => {} // single-photo actions pressed while not viewing a photo
        }
    }

    fn after_rating(&mut self) {
        self.s.ratings.retain(|_, r| *r != Rating::default());
        if self.s.filter != Filter::All {
            self.refresh_view(None);
        }
    }

    fn transfer(&mut self, ctx: &egui::Context, mv: bool) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        let mut dialog = rfd::FileDialog::new().set_title(if mv { "Move to folder" } else { "Copy to folder" });
        if let Some(d) = self.s.last_target.clone().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(d);
        }
        let Some(dest) = dialog.pick_folder() else { return };
        self.s.last_target = Some(dest.clone());
        self.transfer_to(ctx, targets, dest, mv);
    }

    /// Moves (or copies) `targets` into `dest` in the background, with Undo for moves.
    fn transfer_to(&mut self, ctx: &egui::Context, targets: Vec<PathBuf>, dest: PathBuf, mv: bool) {
        let (tx, ctx) = (self.events_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let (mut done, mut errors) = (vec![], vec![]);
            for p in &targets {
                match files::transfer(p, &dest, mv) {
                    Ok(n) => done.push((p.clone(), n)),
                    Err(e) => errors.push(e),
                }
            }
            let verb = if mv { "Moved" } else { "Copied" };
            let mut msg = format!("{verb} {} to {}", plural(done.len(), "file"), file_name(&dest));
            if let Some(e) = errors.first() {
                msg += &format!(". {} failed: {e}", errors.len());
            }
            let undo = (mv && !done.is_empty()).then(|| Undo::MoveBack(done.clone()));
            if mv {
                let _ = tx.send(Event::Moved(done));
            }
            let _ = tx.send(Event::Toast(msg, undo));
            let _ = tx.send(Event::Relist);
            ctx.request_repaint();
        });
    }

    fn wallpaper(&mut self, ctx: &egui::Context) {
        let (Some(p), Some(dir)) = (self.current().cloned(), self.data_dir.clone()) else { return };
        let (tx, ctx) = (self.events_tx.clone(), ctx.clone());
        self.toast("Setting wallpaper...");
        std::thread::spawn(move || {
            // Save an upright PNG copy so any format (and EXIF rotation) works as a wallpaper.
            let result = (|| {
                let img = decode::read(&p, None, false)?.frames.swap_remove(0).0;
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let out = dir.join("wallpaper.png");
                img.save(&out).map_err(|e| e.to_string())?;
                files::set_wallpaper(&out)
            })();
            let msg = match result {
                Ok(()) => format!("Wallpaper set to {}", file_name(&p)),
                Err(e) => format!("Couldn't set wallpaper: {e}"),
            };
            let _ = tx.send(Event::Toast(msg, None));
            ctx.request_repaint();
        });
    }

    fn copy_image(&mut self) {
        let Some(d) = self.decoded() else { return };
        let [w, h] = d.pixels.size;
        let bytes: Vec<u8> = d.pixels.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        let result = arboard::Clipboard::new()
            .and_then(|mut c| c.set_image(arboard::ImageData { width: w, height: h, bytes: bytes.into() }));
        self.toast(match result {
            Ok(()) => "Copied image".to_string(),
            Err(e) => format!("Couldn't copy: {e}"),
        });
    }

    /// Writes `name-edited.ext` next to the photo with the current rotation, flips, straightening,
    /// crop and color adjustments.
    fn save_copy(&mut self, ctx: &egui::Context) {
        let Some(p) = self.current().cloned() else { return };
        let (rot, fh, fv, crop, adjust) = (self.rot, self.flip_h, self.flip_v, self.crop, self.adjust);
        let (tx, ctx) = (self.events_tx.clone(), ctx.clone());
        self.editing = false;
        self.toast("Saving copy...");
        std::thread::spawn(move || {
            let result = (|| -> Result<PathBuf, String> {
                let first = decode::read(&p, None, false)?.frames.swap_remove(0).0;
                let mut img = image::DynamicImage::ImageRgba8(first);
                if fh {
                    img = img.fliph();
                }
                if fv {
                    img = img.flipv();
                }
                img = match rot {
                    1 => img.rotate90(),
                    2 => img.rotate180(),
                    3 => img.rotate270(),
                    _ => img,
                };
                if adjust.angle != 0.0 {
                    img = image::DynamicImage::ImageRgba8(edit::straighten(&img.to_rgba8(), adjust.angle));
                }
                if let Some(c) = crop {
                    let (w, h) = (img.width() as f32, img.height() as f32);
                    let (x, y) = ((c.min.x * w) as u32, (c.min.y * h) as u32);
                    let (cw, ch) = ((c.width() * w).round().max(1.0) as u32, (c.height() * h).round().max(1.0) as u32);
                    img = img.crop_imm(x, y, cw, ch);
                }
                if adjust.colors_changed() {
                    let mut rgba = img.to_rgba8();
                    edit::adjust_image(&mut rgba, &adjust);
                    img = image::DynamicImage::ImageRgba8(rgba);
                }
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
                // Keep the format when we can write it; otherwise (HEIC, SVG, RAW...) save PNG.
                let (fmt, ext) = match image::ImageFormat::from_extension(&ext).filter(|f| f.writing_enabled()) {
                    Some(f) => (f, ext),
                    None => (image::ImageFormat::Png, "png".into()),
                };
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let dest = files::unique_dest(p.parent().unwrap_or(Path::new(".")), &format!("{stem}-edited.{ext}"));
                if fmt == image::ImageFormat::Jpeg {
                    img = image::DynamicImage::ImageRgb8(img.to_rgb8()); // JPEG has no alpha
                }
                img.save_with_format(&dest, fmt).map_err(|e| e.to_string())?;
                Ok(dest)
            })();
            let msg = match result {
                Ok(d) => format!("Saved {}", file_name(&d)),
                Err(e) => format!("Couldn't save: {e}"),
            };
            let _ = tx.send(Event::Toast(msg, None));
            let _ = tx.send(Event::Relist);
            ctx.request_repaint();
        });
    }

    /// Saves the video frame on screen as name-frame-0m12s.png next to the video.
    fn save_frame(&mut self, ctx: &egui::Context) {
        let Some((path, Ok(p))) = &self.player else { return };
        let Some(img) = p.snapshot() else { return };
        let t = p.time();
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name = format!("{stem}-frame-{}m{:02}s.png", t as u64 / 60, t as u64 % 60);
        let dest = files::unique_dest(path.parent().unwrap_or(Path::new(".")), &name);
        let (tx, ctx) = (self.events_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let [w, h] = img.size;
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            let result = image::RgbaImage::from_raw(w as u32, h as u32, bytes).ok_or("bad frame".to_string()).and_then(|i| i.save(&dest).map_err(|e| e.to_string()));
            let msg = match result {
                Ok(()) => format!("Saved {}", file_name(&dest)),
                Err(e) => format!("Couldn't save the frame: {e}"),
            };
            let _ = tx.send(Event::Toast(msg, None));
            let _ = tx.send(Event::Relist);
            ctx.request_repaint();
        });
    }

    /// Saves the part between the trim marks as name-trimmed.ext (needs ffmpeg).
    fn save_trim(&mut self, ctx: &egui::Context) {
        let Some((path, Ok(p))) = &self.player else { return };
        let (start, end) = (self.trim.0.unwrap_or(0.0), self.trim.1.unwrap_or(p.duration()));
        if end - start < 0.5 {
            return self.toast("Set the end after the start");
        }
        let (path, tx, ctx) = (path.clone(), self.events_tx.clone(), ctx.clone());
        self.toast("Saving trimmed copy...");
        std::thread::spawn(move || {
            let msg = match files::trim_video(&path, start, end) {
                Ok(d) => format!("Saved {}", file_name(&d)),
                Err(e) => format!("Couldn't trim: {e}"),
            };
            let _ = tx.send(Event::Toast(msg, None));
            let _ = tx.send(Event::Relist);
            ctx.request_repaint();
        });
    }

    /// Writes a map page with a pin for every photo that has a location, and opens it.
    fn open_map(&mut self, ctx: &egui::Context) {
        let points: Vec<(String, f64, f64)> =
            self.files.iter().filter_map(|p| self.meta.get(p)?.gps.map(|(la, lo)| (file_name(p), la, lo))).collect();
        if points.is_empty() {
            return self.toast("No photos here have a location");
        }
        let Some(dir) = self.data_dir.clone() else { return };
        let out = dir.join("map.html");
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&out, files::map_html(&points))) {
            Ok(()) => ctx.open_url(egui::OpenUrl::new_tab(format!("file:///{}", out.display().to_string().replace('\\', "/")))),
            Err(e) => self.toast(format!("Couldn't make the map: {e}")),
        }
    }

    /// Writes resized/converted copies into a "converted" folder next to the originals.
    fn convert(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, format: usize, max_side: usize) {
        let (tx, ctx) = (self.events_tx.clone(), ctx.clone());
        self.toast(format!("Converting {}...", plural(paths.len(), "photo")));
        std::thread::spawn(move || {
            let (ext, max) = (CONVERT_FORMATS[format].1, CONVERT_SIZES[max_side].1);
            let (mut done, mut errors) = (0, vec![]);
            for p in &paths {
                let result = (|| -> Result<(), String> {
                    let mut img = image::DynamicImage::ImageRgba8(decode::read(p, None, false)?.frames.swap_remove(0).0);
                    if max > 0 && img.width().max(img.height()) > max {
                        img = img.resize(max, max, image::imageops::FilterType::Lanczos3);
                    }
                    let dir = p.parent().unwrap_or(Path::new(".")).join("converted");
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let dest = files::unique_dest(&dir, &format!("{stem}.{ext}"));
                    let file = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
                    let w = std::io::BufWriter::new(file);
                    match ext {
                        "jpg" => img.to_rgb8().write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(w, 90)),
                        "webp" => img.to_rgba8().write_with_encoder(image::codecs::webp::WebPEncoder::new_lossless(w)),
                        _ => img.write_with_encoder(image::codecs::png::PngEncoder::new(w)),
                    }
                    .map_err(|e| e.to_string())
                })();
                match result {
                    Ok(()) => done += 1,
                    Err(e) => errors.push(format!("{}: {e}", file_name(p))),
                }
            }
            let mut msg = format!("Saved {} in \"converted\"", plural(done, "photo"));
            if let Some(e) = errors.first() {
                msg += &format!(". {} failed: {e}", errors.len());
            }
            let _ = tx.send(Event::Toast(msg, None));
            let _ = tx.send(Event::Relist);
            ctx.request_repaint();
        });
    }

    /// Collects finished background work.
    fn poll(&mut self, ctx: &egui::Context) {
        for job in &self.thumb_jobs {
            for t in job.rx.try_iter() {
                if let Some(d) = t.duration {
                    self.durations.insert(t.path.clone(), d);
                }
                self.meta.insert(t.path.clone(), t.meta);
                self.meta_dirty = true;
                if let Some(tex) = t.tex {
                    self.thumbs.insert(t.path, tex);
                }
            }
        }
        while let Ok((p, r)) = self.decode_rx.try_recv() {
            if matches!(self.cache.get(&p), Some(Slot::Loading)) {
                self.cache.insert(p, r.map_or_else(Slot::Failed, |d| Slot::Ready(Arc::new(d))));
            }
        }
        while let Ok(ev) = self.events_rx.try_recv() {
            match ev {
                Event::Toast(m, undo) => self.toast_undo(m, undo),
                Event::Moved(pairs) => pairs.iter().for_each(|(o, n)| self.rekey(o, n)),
                Event::Relist => self.relist(ctx),
                Event::Previews(found) => {
                    let new: Vec<PathBuf> = found.iter().flat_map(|(_, f)| f.clone()).filter(|p| !self.thumbs.contains_key(p)).collect();
                    if !new.is_empty() {
                        self.thumb_jobs.push(decode::spawn_thumbs(ctx, new, self.thumb_dir()));
                    }
                    self.previews = found.into_iter().collect();
                }
            }
        }
        // Search, date headings and look-alikes use metadata that trickles in: re-sort now and then.
        let needs_meta = !self.filter_text.is_empty() || self.s.group_dates || matches!(self.s.filter, Filter::Duplicates | Filter::Located);
        if self.meta_dirty && needs_meta && self.mode == Mode::Grid && self.last_poll.elapsed() > Duration::from_millis(400) {
            self.meta_dirty = false;
            self.refresh_view(None);
        }
        let got: Vec<_> = self.phone.iter().flatten().flat_map(|s| s.rx.try_iter()).collect();
        for r in got {
            match r {
                Ok(p) => {
                    self.toast(format!("Received {} from your phone", file_name(&p)));
                    if self.folder.as_deref() == p.parent() {
                        self.relist(ctx);
                    }
                    self.received.push(p);
                }
                Err(e) => self.toast(format!("Couldn't receive {e}")),
            }
        }
        let launches: Vec<_> = self.instance_rx.iter().flat_map(|rx| rx.try_iter()).collect();
        for msg in launches {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            // Opened again from the Start menu (no file): start fresh on the home page,
            // as if the app had just been launched.
            match msg {
                Some(p) => self.open(ctx, &p),
                None => self.go_home(),
            }
        }
        // Watch the folder for changes by re-reading it every couple of seconds.
        if self.folder.is_some() {
            if self.last_poll.elapsed() > Duration::from_secs(2) {
                self.last_poll = Instant::now();
                self.relist(ctx);
            }
            ctx.request_repaint_after(Duration::from_secs(2));
        }
    }

    /// Keeps the current photo and its neighbours decoded (preloading), and drops the rest.
    fn sync_cache(&mut self, ctx: &egui::Context) {
        let n = self.files.len();
        let want: Vec<PathBuf> = match self.mode {
            Mode::Single if n > 0 => {
                // The photo on screen, then two ahead and two behind, so flipping through never waits.
                let mut v: Vec<PathBuf> = vec![];
                for off in [0, 1, 2] {
                    for i in [(self.index + off) % n, (self.index + 2 * n - off) % n] {
                        if !v.contains(&self.files[i]) {
                            v.push(self.files[i].clone());
                        }
                    }
                }
                v
            }
            Mode::Compare => self.compare.clone().map(Vec::from).unwrap_or_default(),
            _ => vec![],
        };
        let want: Vec<PathBuf> = want.into_iter().filter(|p| !video::is_video(p)).collect(); // videos use the player
        self.cache.retain(|p, _| want.contains(p));
        for p in want {
            if !self.cache.contains_key(&p) {
                self.cache.insert(p.clone(), Slot::Loading);
                let (tx, ctx) = (self.decode_tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let r = decode::load(&ctx, &p);
                    let _ = tx.send((p, r));
                    ctx.request_repaint();
                });
            }
        }
    }

    fn keys(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let (single, grid, n) = (self.mode == Mode::Single, self.mode == Mode::Grid, self.files.len());
        let (index, editing) = (self.index, self.editing);
        let is_video = self.player.is_some();
        let playing = matches!(&self.player, Some((_, Ok(p))) if p.playing());
        let cols = self.grid_cols.max(1);
        let many = self.mode == Mode::Grid && self.selected.len() > 1;
        ctx.input_mut(|i| {
            let (none, ctrl, shift) = (Modifiers::NONE, Modifiers::COMMAND, Modifiers::SHIFT);
            let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
            // More specific combos first: consume_key ignores extra Shift for plain bindings.
            let mut bind = |m: Modifiers, key: Key, a: Action| {
                if i.consume_key(m, key) {
                    acts.push(a);
                }
            };
            bind(ctrl_shift, Key::O, Action::OpenFolder);
            if let Some(f) = &self.folder {
                bind(ctrl_shift, Key::N, Action::NewFolder(f.clone()));
            }
            bind(ctrl, Key::O, Action::OpenFile);
            bind(none, Key::Escape, Action::Back);
            bind(none, Key::F11, Action::Fullscreen);
            if single || grid {
                bind(shift, Key::F, Action::Fav); // before plain F, which would also match Shift+F
            }
            bind(none, Key::F, Action::Fullscreen);
            if grid {
                bind(ctrl, Key::F, Action::FocusSearch);
            }
            bind(ctrl_shift, Key::C, Action::CopyPath);
            bind(ctrl, Key::C, Action::CopyImage);
            bind(ctrl, Key::Num0, Action::Fit);
            bind(ctrl, Key::B, Action::Sidebar);
            bind(ctrl, Key::Comma, Action::Settings);
            bind(none, Key::F1, Action::Keys);
            bind(shift, Key::Questionmark, Action::Keys);
            bind(none, Key::Questionmark, Action::Keys);
            if single {
                bind(none, Key::Tab, Action::Clean);
            }
            if single || grid {
                let alt = Modifiers::ALT;
                for (k, l) in [(Key::Num1, 1), (Key::Num2, 2), (Key::Num3, 3), (Key::Num4, 4), (Key::Num5, 5)] {
                    bind(alt, k, Action::Label(l));
                }
                if many {
                    bind(none, Key::F2, Action::BatchRename);
                }
            }            if grid {
                bind(ctrl, Key::A, Action::SelectAll);
            }
            if single && is_video {
                // J/K/L like most video players. L is the loupe for photos.
                bind(none, Key::J, Action::Seek(-10.0));
                bind(none, Key::K, Action::PlayPause);
                bind(none, Key::M, Action::Mute); // M is Move to folder for photos
                bind(none, Key::ArrowUp, Action::Volume(0.1));
                bind(none, Key::ArrowDown, Action::Volume(-0.1));
                bind(shift, Key::Comma, Action::Speed(-1));
                bind(shift, Key::Period, Action::Speed(1));
                bind(none, Key::S, Action::SaveFrame);
                bind(none, Key::P, Action::Pip);
                bind(none, Key::OpenBracket, Action::TrimMark(true));
                bind(none, Key::CloseBracket, Action::TrimMark(false));
                if playing {
                    bind(none, Key::L, Action::Seek(10.0));
                }
            }
            if single {
                bind(none, Key::Plus, Action::ZoomBy(1.25));
                bind(none, Key::Equals, Action::ZoomBy(1.25));
                bind(none, Key::Minus, Action::ZoomBy(0.8));
            }
            if single || grid {
                bind(none, Key::Delete, Action::Delete);
                bind(none, Key::F2, Action::Rename);
                bind(none, Key::C, Action::Transfer { mv: false });
                bind(none, Key::M, Action::Transfer { mv: true });
                for (k, s) in [(Key::Num0, 0), (Key::Num1, 1), (Key::Num2, 2), (Key::Num3, 3), (Key::Num4, 4), (Key::Num5, 5)] {
                    bind(none, k, Action::Rate(s));
                }
            }
            if single {
                bind(shift, Key::R, Action::Rotate(-1));
                bind(none, Key::R, Action::Rotate(1));
                bind(none, Key::H, Action::FlipH);
                bind(none, Key::V, Action::FlipV);
                bind(none, Key::E, Action::ToggleEdit);
                bind(none, Key::I, Action::Info);
                bind(none, Key::L, Action::Loupe);
                bind(none, Key::Z, Action::ActualSize);
                bind(none, Key::Space, if is_video { Action::PlayPause } else { Action::Slideshow });
            }
            if grid {
                // Arrow keys move the highlight; Enter opens it, Space selects it.
                if n > 0 {
                    let step = |k: Key, i: &mut egui::InputState| i.consume_key(none, k);
                    let to = if step(Key::ArrowRight, i) {
                        Some((index + 1).min(n - 1))
                    } else if step(Key::ArrowLeft, i) {
                        Some(index.saturating_sub(1))
                    } else if step(Key::ArrowDown, i) {
                        Some((index + cols).min(n - 1))
                    } else if step(Key::ArrowUp, i) {
                        Some(index.saturating_sub(cols))
                    } else if step(Key::Home, i) {
                        Some(0)
                    } else {
                        None
                    };
                    if let Some(t) = to {
                        self.index = t;
                        self.scroll_to_current = true;
                    }
                    if i.consume_key(none, Key::Enter) {
                        acts.push(Action::Go(self.index));
                    }
                    if i.consume_key(none, Key::Space) {
                        let p = self.files[self.index].clone();
                        if !self.selected.remove(&p) {
                            self.selected.insert(p);
                        }
                        self.anchor = Some(self.index);
                    }
                }
            }
            if single {
                // No End key: on this machine it's a system screenshot shortcut.
                if n > 0 && !editing {
                    let last = n - 1;
                    let target = if i.consume_key(none, Key::ArrowRight) {
                        Some(if index == last { 0 } else { index + 1 })
                    } else if i.consume_key(none, Key::ArrowLeft) {
                        Some(if index == 0 { last } else { index - 1 })
                    } else if i.consume_key(none, Key::Home) {
                        Some(0)
                    } else if i.consume_key(none, Key::PageDown) {
                        Some((index + 10).min(last))
                    } else if i.consume_key(none, Key::PageUp) {
                        Some(index.saturating_sub(10))
                    } else {
                        None
                    };
                    acts.extend(target.map(Action::Go));
                }
            }
        });
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        let single = self.mode == Mode::Single;
        let grid = self.mode == Mode::Grid;
        let targets = self.targets().len();
        let transformed = self.rot != 0 || self.flip_h || self.flip_v || self.crop.is_some() || self.adjust != Adjust::default();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                item(ui, acts, true, I_PICTURE, "Open file...", "Ctrl+O", Action::OpenFile);
                item(ui, acts, true, I_FOLDER_OPEN, "Open folder...", "Ctrl+Shift+O", Action::OpenFolder);
                item(ui, acts, self.folder.is_some(), I_FOLDER, "New folder...", "Ctrl+Shift+N", Action::NewFolder(self.folder.clone().unwrap_or_default()));
                item(ui, acts, true, I_DOWNLOAD, "Receive from phone...", "", Action::Phone);
                ui.add_enabled_ui(!self.s.recent.is_empty(), |ui| {
                    ui.menu_button(label(I_CLOCK, "Recent folders"), |ui| {
                        for p in &self.s.recent {
                            item(ui, acts, p.is_dir(), I_FOLDER, &p.display().to_string(), "", Action::Open(p.clone()));
                        }
                        ui.separator();
                        if ui.button(label(I_TRASH, "Clear recent folders")).clicked() {
                            self.s.recent.clear();
                        }
                    });
                });
                ui.separator();
                item(ui, acts, single && transformed, I_DOWNLOAD, "Save edited copy", "", Action::SaveCopy);
                ui.separator();
                item(ui, acts, targets == 1, I_PENCIL, "Rename...", "F2", Action::Rename);
                item(ui, acts, targets > 1, I_PENCIL, "Rename selected...", "F2", Action::BatchRename);
                item(ui, acts, targets > 0, I_IMAGES, "Resize or convert...", "", Action::Convert);
                item(ui, acts, targets > 0, I_COPY_TO, "Copy to folder...", "C", Action::Transfer { mv: false });
                item(ui, acts, targets > 0, I_MOVE, "Move to folder...", "M", Action::Transfer { mv: true });
                item(ui, acts, targets > 0, I_TRASH, "Delete", "Del", Action::Delete);
                ui.separator();
                item(ui, acts, targets == 1, I_FOLDER, "Show in Explorer", "", Action::Reveal);
                item(ui, acts, targets == 1, I_EXTERNAL, "Open with...", "", Action::OpenWith);
                item(ui, acts, single, I_PICTURE, "Set as wallpaper", "", Action::Wallpaper);
                ui.separator();
                item(ui, acts, true, I_SETTINGS, "Settings...", "Ctrl+,", Action::Settings);
                ui.separator();
                if ui.button(label(I_EXIT, "Exit")).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                item(ui, acts, single, I_COPY, "Copy image", "Ctrl+C", Action::CopyImage);
                item(ui, acts, targets > 0, I_COPY, "Copy path", "Ctrl+Shift+C", Action::CopyPath);
                ui.separator();
                item(ui, acts, single, I_ROTATE_RIGHT, "Rotate right", "R", Action::Rotate(1));
                item(ui, acts, single, I_ROTATE_LEFT, "Rotate left", "Shift+R", Action::Rotate(-1));
                item(ui, acts, single, I_FLIP_H, "Flip horizontal", "H", Action::FlipH);
                item(ui, acts, single, I_FLIP_V, "Flip vertical", "V", Action::FlipV);
                toggle(ui, acts, single, self.editing, I_CROP, "Crop and edit", "E", Action::ToggleEdit);
                ui.separator();
                item(ui, acts, grid, I_APPS, "Select all", "Ctrl+A", Action::SelectAll);
                item(ui, acts, grid && !self.selected.is_empty(), I_CROSS, "Clear selection", "Esc", Action::ClearSelection);
            });
            ui.menu_button("View", |ui| {
                toggle(ui, acts, true, self.s.sidebar, I_SIDEBAR, "Sidebar", "Ctrl+B", Action::Sidebar);
                toggle(ui, acts, true, self.fullscreen, I_EXPAND, "Fullscreen", "F", Action::Fullscreen);
                toggle(ui, acts, true, self.s.glass, I_IMAGES, "Glass look", "", Action::Glass);
                toggle(ui, acts, single, self.clean, I_EYE, "Photo only", "Tab", Action::Clean);
                toggle(ui, acts, true, self.show_info, I_INFO, "Info panel", "I", Action::Info);
                toggle(ui, acts, true, self.loupe, I_SEARCH, "Loupe", "L", Action::Loupe);
                ui.separator();
                toggle(ui, acts, true, self.s.group_dates, I_CALENDAR, "Group by date", "", Action::GroupDates);
                toggle(ui, acts, true, self.s.subfolders, I_FOLDER, "Include subfolders", "", Action::Subfolders);
                let mut square = self.s.square_tiles;
                if ui.add(egui::Button::new(label(if square { I_CHECK } else { I_SQUARE }, "Square tiles")).selected(square)).clicked() {
                    square = !square;
                }
                self.s.square_tiles = square;
                ui.separator();
                item(ui, acts, single, I_ZOOM_IN, "Zoom in", "+", Action::ZoomBy(1.25));
                item(ui, acts, single, I_ZOOM_OUT, "Zoom out", "-", Action::ZoomBy(0.8));
                item(ui, acts, single, I_EYE, "Actual size (100%)", "Z", Action::ActualSize);
                if ui.add(egui::Button::new(label(if self.minimap { I_CHECK } else { I_GRID }, "Zoom map")).selected(self.minimap)).clicked() {
                    self.minimap = !self.minimap;
                }
                ui.menu_button(label(I_EXPAND, "Open photos at"), |ui| {
                    for (m, name, keys) in [(FitMode::Fit, "Fit to window", "Ctrl+0"), (FitMode::Fill, "Fill window", ""), (FitMode::Actual, "Actual size", "")] {
                        toggle(ui, acts, true, self.s.fit_mode == m, I_EXPAND, name, keys, Action::SetFit(m));
                    }
                });
                ui.menu_button(label(I_BRUSH, "Background"), |ui| {
                    for (b, name) in [(Backdrop::Black, "Black"), (Backdrop::Gray, "Gray"), (Backdrop::Blur, "Blurred photo")] {
                        ui.radio_value(&mut self.s.backdrop, b, name);
                    }
                });
                ui.separator();
                toggle(ui, acts, single, self.slideshow.is_some(), I_PLAY, "Slideshow", "Space", Action::Slideshow);
                ui.menu_button(label(I_CLOCK, "Slideshow delay"), |ui| {
                    for secs in [2.0, 3.0, 5.0, 10.0] {
                        ui.radio_value(&mut self.s.slide_secs, secs, format!("{secs} seconds"));
                    }
                });
                ui.separator();
                ui.menu_button(label(I_SUN, "Theme"), |ui| {
                    for (t, icon, name) in [(ThemePref::System, I_PC, "Same as Windows"), (ThemePref::Light, I_SUN, "Light"), (ThemePref::Dark, I_MOON, "Dark")] {
                        toggle(ui, acts, true, self.s.theme == t, icon, name, "", Action::SetTheme(t));
                    }
                });
                ui.menu_button(label(I_PALETTE, "Accent color"), |ui| {
                    for (name, rgb) in ACCENTS {
                        let swatch = RichText::new("■  ").color(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                        let mut job = egui::text::LayoutJob::default();
                        swatch.append_to(&mut job, ui.style(), egui::FontSelection::Default, egui::Align::Center);
                        RichText::new(name).append_to(&mut job, ui.style(), egui::FontSelection::Default, egui::Align::Center);
                        if ui.add(egui::Button::new(job).selected(self.s.accent == rgb)).clicked() {
                            self.s.accent = rgb;
                            set_accent(rgb);
                            apply_theme(ui.ctx(), self.s.theme);
                        }
                    }
                });
            });
            ui.menu_button("Go", |ui| {
                let n = self.files.len();
                let i = self.index;
                item(ui, acts, self.mode != Mode::Home, I_BACK, "Back", "Esc", Action::Back);
                ui.separator();
                item(ui, acts, single && n > 1, I_RIGHT, "Next", "→", Action::Go((i + 1) % n.max(1)));
                item(ui, acts, single && n > 1, I_LEFT, "Previous", "←", Action::Go((i + n.max(1) - 1) % n.max(1)));
                item(ui, acts, single, I_NEXT, "First", "Home", Action::Go(0));
                item(ui, acts, single, I_RIGHT, "Forward 10", "PgDn", Action::Go((i + 10).min(n.saturating_sub(1))));
                item(ui, acts, single, I_LEFT, "Back 10", "PgUp", Action::Go(i.saturating_sub(10)));
                ui.separator();
                item(ui, acts, grid && self.selected.len() == 2, I_GRID, "Compare selected", "", Action::Compare);
                item(ui, acts, self.folder.is_some(), I_PIN, "Map of photos", "", Action::Map);
                let pinned = self.folder.as_ref().is_some_and(|f| self.s.pinned.contains(f));
                toggle(ui, acts, self.folder.is_some(), pinned, I_PIN, "Pin this folder", "", Action::Pin);
            });
            ui.menu_button("Rate", |ui| {
                let all_fav = targets > 0 && self.targets().iter().all(|p| self.rating(p).fav);
                toggle(ui, acts, targets > 0, all_fav, I_HEART, "Favorite", "Shift+F", Action::Fav);
                ui.separator();
                for s in 1..=5u8 {
                    item(ui, acts, targets > 0, I_STAR, &"★".repeat(s as usize), &s.to_string(), Action::Rate(s));
                }
                item(ui, acts, targets > 0, I_CROSS, "Clear stars", "0", Action::Rate(0));
                ui.separator();
                label_items(ui, acts, targets > 0);
            });
            ui.menu_button("Help", |ui| {
                item(ui, acts, true, I_INFO, "Keyboard shortcuts", "F1", Action::Keys);
                item(ui, acts, true, I_HELP, "About Glance", "", Action::About);
            });

            ui.separator();
            if icon_button(ui, I_SIDEBAR, "Sidebar (Ctrl+B)").clicked() {
                acts.push(Action::Sidebar);
            }
            if self.mode != Mode::Home && ui.button(label(I_BACK, "Back")).on_hover_text("Esc").clicked() {
                acts.push(Action::Back);
            }
            self.breadcrumb(ui, acts);
            if self.folder.is_none() {
                ui.label(RichText::new("Glance").weak());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), window_buttons);
        });
    }

    /// Clickable path of the current folder: Pictures › 2024 › Trip.
    fn breadcrumb(&self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        let Some(dir) = &self.folder else { return };
        let mut parts: Vec<&Path> = dir.ancestors().collect();
        parts.reverse();
        let skip = parts.len().saturating_sub(4);
        if skip > 0 {
            ui.label(RichText::new("…").weak());
        }
        for (i, p) in parts.iter().enumerate().skip(skip) {
            if i > 0 {
                ui.label(RichText::new("›").weak());
            }
            if ui.add(egui::Button::new(file_name(p)).frame(false)).clicked() {
                acts.push(Action::Open(p.to_path_buf()));
            }
        }
        if let (Mode::Single, Some(p)) = (self.mode, self.current()) {
            ui.label(RichText::new("›").weak());
            ui.label(RichText::new(file_name(p)).strong());
        }
    }

    /// Second toolbar row: grid tools, or edit tools while cropping.
    fn tool_row(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if self.mode == Mode::Single {
                ui.label(RichText::new("Edit").strong());
                ui.separator();
                for (icon, tip, a) in [
                    (I_ROTATE_LEFT, "Rotate left (Shift+R)", Action::Rotate(-1)),
                    (I_ROTATE_RIGHT, "Rotate right (R)", Action::Rotate(1)),
                    (I_FLIP_H, "Flip horizontal (H)", Action::FlipH),
                    (I_FLIP_V, "Flip vertical (V)", Action::FlipV),
                ] {
                    if icon_button(ui, icon, tip).clicked() {
                        acts.push(a);
                    }
                }
                ui.separator();
                ui.label(RichText::new(I_CROP).weak()).on_hover_text("Crop shape");
                let shapes: [(&str, Option<f32>); 6] = [("Free", None), ("1:1", Some(1.0)), ("4:3", Some(4.0 / 3.0)), ("3:2", Some(1.5)), ("16:9", Some(16.0 / 9.0)), ("3:4", Some(0.75))];
                let current = shapes.iter().find(|s| s.1 == self.aspect).map_or("Free", |s| s.0);
                egui::ComboBox::from_id_salt("aspect").width(64.0).selected_text(current).show_ui(ui, |ui| {
                    for (name, a) in shapes {
                        if ui.selectable_label(self.aspect == a, name).clicked() {
                            self.aspect = a;
                            self.crop = None;
                        }
                    }
                });
                if ui.add_enabled(self.crop.is_some(), egui::Button::new(label(I_CROSS, "Clear crop"))).clicked() {
                    self.crop = None;
                }
                ui.separator();
                let mut a = self.adjust;
                ui.spacing_mut().slider_width = 70.0;
                for (icon, tip, v, range) in [
                    (I_SUN, "Brightness", &mut a.brightness, -1.0..=1.0),
                    (I_PALETTE, "Saturation", &mut a.saturation, -1.0..=1.0),
                    (I_BRUSH, "Contrast", &mut a.contrast, -1.0..=1.0),
                    (I_ROTATE_RIGHT, "Straighten (degrees)", &mut a.angle, -45.0..=45.0),
                ] {
                    ui.label(RichText::new(icon).weak()).on_hover_text(tip);
                    let s = ui.add(egui::Slider::new(v, range).show_value(false)).on_hover_text(format!("{tip} · double-click to reset"));
                    if s.double_clicked() {
                        *v = 0.0;
                    }
                }
                self.adjust = a;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        acts.push(Action::Back);
                    }
                    let save = egui::Button::new(RichText::new(label(I_DOWNLOAD, "Save copy")).color(Color32::WHITE)).fill(accent());
                    if ui.add(save).on_hover_text("Saves name-edited next to the original").clicked() {
                        acts.push(Action::SaveCopy);
                    }
                });
                return;
            }
            if let Some(f) = &self.folder {
                if icon_button(ui, I_FOLDER, "New folder (Ctrl+Shift+N)").clicked() {
                    acts.push(Action::NewFolder(f.clone()));
                }
                ui.separator();
            }
            let before = (self.filter_text.clone(), self.s.filter, self.s.sort, self.s.sort_desc);
            ui.label(RichText::new(I_SEARCH).weak());
            let search = ui.add(egui::TextEdit::singleline(&mut self.filter_text).hint_text("Search name, date, camera, tag").desired_width(190.0));
            if std::mem::take(&mut self.focus_search) {
                search.request_focus();
            }
            search.on_hover_text("Ctrl+F · every word must match: try 2024 july canon");
            ui.label(RichText::new(I_FILTER).weak());
            egui::ComboBox::from_id_salt("filter").selected_text(self.s.filter.name()).show_ui(ui, |ui| {
                for (f, name) in Filter::ALL {
                    ui.selectable_value(&mut self.s.filter, f, name);
                }
            });
            ui.separator();
            ui.label(RichText::new(I_SORT).weak()).on_hover_text("Sort");
            egui::ComboBox::from_id_salt("sort")
                .selected_text(match self.s.sort {
                    SortKey::Name => "Name",
                    SortKey::Modified => "Date modified",
                    SortKey::Size => "Size",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.s.sort, SortKey::Name, "Name");
                    ui.selectable_value(&mut self.s.sort, SortKey::Modified, "Date modified");
                    ui.selectable_value(&mut self.s.sort, SortKey::Size, "Size");
                });
            let dir = if self.s.sort_desc { "Descending" } else { "Ascending" };
            if icon_button(ui, if self.s.sort_desc { "↓" } else { "↑" }, dir).clicked() {
                self.s.sort_desc = !self.s.sort_desc;
            }
            if ui.add(egui::Button::new(I_CALENDAR).selected(self.s.group_dates)).on_hover_text("Group by date").clicked() {
                acts.push(Action::GroupDates);
            }
            let sections = self.sections();
            if sections.len() > 1 {
                ui.menu_button("Jump to ▾", |ui| {
                    for (k, (name, _)) in sections.iter().enumerate() {
                        if ui.button(name).clicked() {
                            self.jump_to = Some(k);
                        }
                    }
                });
            }
            if ui.add(egui::Button::new(I_SQUARE).selected(self.s.square_tiles)).on_hover_text("Square tiles").clicked() {
                self.s.square_tiles = !self.s.square_tiles;
            }
            ui.separator();
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(I_GRID).size(11.0).weak());
            ui.spacing_mut().slider_width = 90.0;
            ui.add(egui::Slider::new(&mut self.s.grid_size, 80.0..=320.0).show_value(false))
                .on_hover_text("Thumbnail size (Ctrl+scroll)");
            ui.label(RichText::new(I_APPS).size(16.0).weak());
            ui.spacing_mut().item_spacing.x = 8.0;
            if before != (self.filter_text.clone(), self.s.filter, self.s.sort, self.s.sort_desc) {
                self.refresh_view(None);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = self.selected.len();
                if n == 0 {
                    if !self.s.seen_select_hint {
                        ui.label(RichText::new("Ctrl+click or drag to select").weak());
                    }
                    return;
                }
                self.s.seen_select_hint = true;
                if icon_button(ui, I_CROSS, "Clear selection (Esc)").clicked() {
                    acts.push(Action::ClearSelection);
                }
                if ui.button(label(I_TRASH, "Delete")).clicked() {
                    acts.push(Action::Delete);
                }
                if ui.button(label(I_MOVE, "Move to...")).clicked() {
                    acts.push(Action::Transfer { mv: true });
                }
                if ui.button(label(I_COPY_TO, "Copy to...")).clicked() {
                    acts.push(Action::Transfer { mv: false });
                }
                if ui.add_enabled(n == 2, egui::Button::new(label(I_GRID, "Compare"))).on_disabled_hover_text("Select exactly 2").clicked() {
                    acts.push(Action::Compare);
                }
                ui.label(format!("{n} selected"));
            });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        let right = ui.max_rect().right();
        let row = ui
            .horizontal(|ui| {
                ui.set_min_height(22.0); // same height on every screen, even with nothing on the left
            let weak = |s: String| RichText::new(s).weak();
            match self.mode {
                Mode::Grid => {
                    let mut s = media_count(&self.files);
                    if self.files.len() != self.all.len() {
                        s += &format!(" (of {})", self.all.len());
                    }
                    if !self.dirs.is_empty() {
                        s += &format!(", {}", plural(self.dirs.len(), "folder"));
                    }
                    ui.label(weak(s));
                }
                Mode::Single => {
                    let Some(p) = self.current() else { return };
                    let mut parts = vec![format!("{} / {}", self.index + 1, self.files.len())];
                    if let Some(Slot::Ready(d)) = self.cache.get(p) {
                        parts.push(format!("{} × {}", d.orig[0], d.orig[1]));
                        parts.push(d.format.clone());
                    }
                    if let Some(e) = self.all.iter().find(|e| e.path == *p) {
                        parts.push(files::human_size(e.size));
                    }
                    ui.label(weak(parts.join("   ·   ")));
                    let r = self.rating(p);
                    if r.stars > 0 {
                        ui.label(RichText::new("★".repeat(r.stars as usize)).color(STAR));
                    }
                    if r.fav {
                        ui.label(RichText::new("♥").color(HEART));
                    }
                    if self.slideshow.is_some() {
                        ui.label(RichText::new(label(I_PLAY, "Slideshow")).color(accent()));
                    }
                    if self.player.is_some() {
                        return; // no zoom or pixel readout for videos
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(28.0); // the theme toggle's corner
                        let small = |icon: &str| egui::Button::new(RichText::new(icon).size(12.0)).frame(false);
                        if ui.add(small(I_PLUS)).on_hover_text("Zoom in (+)").clicked() {
                            acts.push(Action::ZoomBy(1.25));
                        }
                        let pct = ui.add(egui::Button::new(weak(format!("{:.0}%", self.px_scale * self.zoom * 100.0))).frame(false));
                        if pct.on_hover_text("Click for 100% · scroll to zoom · double-click the photo to fit").clicked() {
                            acts.push(Action::ActualSize);
                        }
                        if ui.add(small(I_MINUS)).on_hover_text("Zoom out (-)").clicked() {
                            acts.push(Action::ZoomBy(0.8));
                        }
                        if let Some(r) = &self.readout {
                            ui.separator();
                            ui.label(weak(r.clone()));
                        }
                    });
                }
                Mode::Compare => {
                    ui.label(weak("Comparing 2 photos · scroll to zoom both · Esc to go back".into()));
                }
                Mode::Home => {}
            }
        })
            .response
            .rect;
        // Light/dark toggle in the bottom-right corner, on every screen. It shows the theme in use.
        let light = ui.ctx().theme() == egui::Theme::Light;
        let spot = Rect::from_center_size(pos2(right - 12.0, row.center().y), vec2(22.0, 20.0));
        let resp = ui.interact(spot, ui.scope_id().with("theme_toggle"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(spot, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let color = if resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
        ui.painter().text(spot.center(), egui::Align2::CENTER_CENTER, if light { I_SUN } else { I_MOON }, egui::FontId::proportional(13.0), color);
        let resp = resp.on_hover_text("Theme and colours");
        // Opens upward from the corner (there's no room below the status bar).
        egui::Popup::menu(&resp).align(egui::RectAlign::TOP_END).align_alternatives(&[]).show(|ui| self.theme_menu(ui, acts));
    }

    fn info_panel(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        let Some(p) = self.current().cloned() else { return };
        let p = &p;
        ui.add_space(8.0);
        ui.label(RichText::new(file_name(p)).strong());
        ui.add_space(6.0);
        let dec = match self.cache.get(p) {
            Some(Slot::Ready(d)) => Some(d.clone()),
            _ => None,
        };
        let entry = self.all.iter().find(|e| e.path == *p);
        egui::Grid::new("info").num_columns(2).spacing([12.0, 5.0]).show(ui, |ui| {
            let mut row = |k: &str, v: String| {
                ui.label(RichText::new(k).weak());
                ui.label(v);
                ui.end_row();
            };
            row("Folder", p.parent().map(file_name).unwrap_or_default());
            if let Some(d) = &dec {
                let mp = d.orig[0] as f64 * d.orig[1] as f64 / 1e6;
                row("Dimensions", format!("{} × {} ({mp:.1} MP)", d.orig[0], d.orig[1]));
                row("Format", d.format.clone());
                if d.frames.len() > 1 {
                    row("Frames", d.frames.len().to_string());
                }
            }
            if let Some(e) = entry {
                row("File size", files::human_size(e.size));
                let t: chrono::DateTime<chrono::Local> = e.modified.into();
                row("Modified", t.format("%Y-%m-%d %H:%M").to_string());
            }
            let r = self.rating(p);
            let stars = if r.stars > 0 { "★".repeat(r.stars as usize) } else { "None".into() };
            row("Rating", format!("{stars}{}", if r.fav { "  ♥" } else { "" }));
            if r.label > 0 {
                row("Label", LABELS[(r.label as usize - 1).min(4)].0.to_string());
            }
            for (k, v) in dec.iter().flat_map(|d| d.exif.iter()) {
                row(k, v.clone());
            }
        });
        if self.meta.get(p).is_some_and(|m| m.gps.is_some()) && ui.button(label(I_PIN, "Show on map")).on_hover_text("Opens OpenStreetMap in your browser").clicked() {
            acts.push(Action::ShowOnMap);
        }
        // Tags: typed as a comma-separated list, saved when you leave the box.
        ui.add_space(10.0);
        ui.label(RichText::new("Tags").weak());
        if self.tag_edit.as_ref().is_none_or(|(tp, _)| tp != p) {
            self.tag_edit = Some((p.clone(), self.s.tags.get(p).map(|t| t.join(", ")).unwrap_or_default()));
        }
        if let Some((_, text)) = &mut self.tag_edit {
            let edit = ui.add(egui::TextEdit::singleline(text).hint_text("beach, family").desired_width(f32::INFINITY));
            if edit.lost_focus() {
                let tags: Vec<String> = text.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
                if tags.is_empty() {
                    self.s.tags.remove(p);
                } else {
                    self.s.tags.insert(p.clone(), tags);
                }
            }
        }
        let Some(d) = dec else { return };
        ui.add_space(12.0);
        ui.label(RichText::new("Histogram").weak());
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 90.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
        // Scale to the tallest bin ignoring pure black/white, which often spike.
        let max = d.hist.iter().flat_map(|c| c[1..255].iter()).copied().max().unwrap_or(1).max(1) as f32;
        let colors = [Color32::from_rgb(230, 70, 70), Color32::from_rgb(70, 200, 90), Color32::from_rgb(80, 130, 240)];
        let w = rect.width() / 256.0;
        for (c, color) in d.hist.iter().zip(colors) {
            for (i, &v) in c.iter().enumerate() {
                let h = (v as f32 / max).min(1.0) * rect.height();
                let x = rect.left() + (i as f32 + 0.5) * w;
                painter.line_segment([pos2(x, rect.bottom()), pos2(x, rect.bottom() - h)], egui::Stroke::new(w.max(1.0), color.gamma_multiply(0.45)));
            }
        }
    }

    /// The menu for right-clicking empty space in a folder.
    fn background_menu(&self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        let Some(f) = &self.folder else { return };
        if ui.button(label(I_FOLDER, "New folder...")).clicked() {
            acts.push(Action::NewFolder(f.clone()));
        }
        if ui.button(label(I_FOLDER_OPEN, "Show in Explorer")).clicked() {
            files::reveal(f);
        }
    }

    fn grid_view(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        // Ctrl+scroll (or pinch) resizes the tiles.
        if ui.rect_contains_pointer(ui.max_rect()) {
            let z = ui.input(|i| i.zoom_delta());
            if z != 1.0 {
                self.s.grid_size = (self.s.grid_size * z).clamp(80.0, 320.0);
            }
        }
        if self.files.is_empty() && self.dirs.is_empty() {
            let bg = ui.interact(ui.max_rect(), ui.scope_id().with("bg_menu"), Sense::click());
            bg.context_menu(|ui| self.background_menu(ui, acts));
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 2.0 - 60.0).max(0.0));
                ui.label(RichText::new(I_IMAGES).size(32.0).weak());
                ui.add_space(6.0);
                let msg = if self.all.is_empty() { "No photos or videos in this folder" } else { "Nothing matches the filter" };
                ui.label(RichText::new(msg).strong());
                ui.add_space(10.0);
                if ui.button(label(I_FOLDER_OPEN, "Open another folder")).clicked() {
                    acts.push(Action::OpenFolder);
                }
            });
            return;
        }
        let ctx = ui.ctx().clone();
        let gs = self.s.grid_size;
        let size = vec2(gs, if self.s.square_tiles { gs } else { gs * 0.75 } + 20.0);
        let spacing = vec2(10.0, 14.0);
        self.grid_cols = (((ui.available_width() + spacing.x) / (gs + spacing.x)).floor() as usize).max(1);
        let scroll = std::mem::take(&mut self.scroll_to_current);
        let jump = self.jump_to.take();
        // One unnamed section, or one per date heading.
        let mut sections: Vec<(Option<String>, usize)> = self.sections().into_iter().map(|(s, i)| (Some(s), i)).collect();
        if sections.is_empty() {
            sections.push((None, 0));
        }

        // Dragging on empty space draws a selection rectangle.
        let view = ui.max_rect();
        // The same layer takes right-clicks on empty space (tiles sit on top and keep their own menus).
        let band_resp = ui.interact(view, ui.scope_id().with("band"), Sense::click_and_drag());
        band_resp.context_menu(|ui| self.background_menu(ui, acts));
        if band_resp.drag_started() {
            self.band = ui.input(|i| i.pointer.press_origin());
        }
        let band = self.band.zip(band_resp.interact_pointer_pos()).map(|(a, b)| Rect::from_two_pos(a, b));
        if band_resp.drag_stopped() {
            self.band = None;
        }

        enum Op {
            Open(usize),
            Toggle(usize),
            Range(usize),
            Ensure(usize),
            DragOut(usize),
        }
        let mut ops = vec![];
        let mut tiles: Vec<(usize, Rect)> = vec![];
        let mut on_screen_missing: Vec<PathBuf> = vec![];
        let mut hovered_video: Option<(PathBuf, Rect)> = None;
        let mut folder_rects = vec![];
        let mut area = egui::ScrollArea::vertical()
            .auto_shrink(false)
            .scroll_source(egui::scroll_area::ScrollSource::MOUSE_WHEEL | egui::scroll_area::ScrollSource::SCROLL_BAR);
        if let Some(y) = self.restore_scroll.take() {
            area = area.vertical_scroll_offset(y);
        }
        let out = area.show(ui, |ui| {
            ui.spacing_mut().item_spacing = spacing;
            if !self.dirs.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for d in &self.dirs {
                        let previews: Vec<Option<&TextureHandle>> =
                            self.previews.get(d).into_iter().flatten().map(|p| self.thumbs.get(p)).collect();
                        let resp = folder_tile(ui, size, &file_name(d), &previews);
                        folder_rects.push((d.clone(), resp.rect));
                        if resp.clicked() {
                            acts.push(Action::Open(d.clone()));
                        }
                        resp.context_menu(|ui| {
                            if ui.button(label(I_FOLDER_OPEN, "Open")).clicked() {
                                acts.push(Action::Open(d.clone()));
                            }
                            if ui.button(label(I_FOLDER, "Show in Explorer")).clicked() {
                                files::reveal(d);
                            }
                            ui.separator();
                            if ui.button(label(I_TRASH, "Delete")).clicked() {
                                acts.push(Action::DeleteFolder(d.clone()));
                            }
                        });
                    }
                });
            }
            for (k, (name, start)) in sections.iter().enumerate() {
                let end = sections.get(k + 1).map_or(self.files.len(), |s| s.1);
                if let Some(name) = name {
                    ui.add_space(2.0);
                    let h = ui.label(RichText::new(name).size(15.0).strong());
                    if jump == Some(k) {
                        h.scroll_to_me(Some(egui::Align::TOP));
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    for i in *start..end {
                        let p = &self.files[i];
                        let name = file_name(p);
                        let is_video = video::is_video(p);
                        let info = TileInfo {
                            label: Some(&name),
                            rating: self.rating(p),
                            current: i == self.index,
                            selected: self.selected.contains(p),
                            video: is_video,
                            cover: self.s.square_tiles,
                            duration: self.durations.get(p).copied(),
                            live: self.live.contains_key(p),
                            draggable: true,
                        };
                        let resp = thumb_tile(ui, size, self.thumbs.get(p), info);
                        tiles.push((i, resp.rect));
                        if !self.thumbs.contains_key(p) && ui.is_rect_visible(resp.rect) {
                            on_screen_missing.push(p.clone());
                        }
                        // Hovering a video (or a Live Photo) plays it in the tile.
                        let clip = if is_video { Some(p) } else { self.live.get(p) };
                        if let (Some(clip), true) = (clip, resp.hovered()) {
                            hovered_video = Some((clip.clone(), resp.rect.with_max_y(resp.rect.max.y - 20.0)));
                        }
                        if resp.drag_started() {
                            ops.push(Op::DragOut(i));
                        }
                        if i == self.index && scroll {
                            resp.scroll_to_me(Some(egui::Align::Center));
                        }
                        if resp.clicked() {
                            let m = ui.input(|i| i.modifiers);
                            ops.push(if m.command { Op::Toggle(i) } else if m.shift { Op::Range(i) } else { Op::Open(i) });
                        }
                        resp.on_hover_text(&name).context_menu(|ui| {
                            ops.push(Op::Ensure(i));
                            if ui.button(label(I_EYE, "Open")).clicked() {
                                acts.push(Action::Go(i));
                            }
                            ui.separator();
                            self.file_menu(ui, acts);
                        });
                    }
                });
            }
        });
        self.folder_rects = folder_rects;
        if let Some(f) = &self.folder {
            self.s.scroll.insert(f.clone(), out.state.clamped_offset().y);
        }
        // Thumbnails on screen load before the rest of the folder.
        if !on_screen_missing.is_empty() {
            for job in &self.thumb_jobs {
                job.prioritize(on_screen_missing.clone());
            }
        }
        if let Some(r) = band {
            let add = ui.input(|i| i.modifiers.command);
            let hit = tiles.iter().filter(|(_, t)| t.intersects(r)).map(|(i, _)| self.files[*i].clone());
            if add {
                self.selected.extend(hit);
            } else {
                self.selected = hit.collect();
            }
            let painter = ui.painter().with_clip_rect(view);
            painter.rect_filled(r, 0.0, accent().gamma_multiply(0.15));
            painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, accent()), egui::StrokeKind::Inside);
        }
        self.hover_preview(&ctx, ui, view, hovered_video);
        for op in ops {
            match op {
                Op::Open(i) => {
                    self.selected.clear();
                    self.anchor = Some(i);
                    acts.push(Action::Go(i));
                }
                Op::Toggle(i) => {
                    let p = &self.files[i];
                    if !self.selected.remove(p) {
                        self.selected.insert(p.clone());
                    }
                    self.anchor = Some(i);
                }
                Op::Range(i) => {
                    let a = self.anchor.unwrap_or(i);
                    self.selected.extend(self.files[a.min(i)..=a.max(i)].iter().cloned());
                }
                Op::Ensure(i) => {
                    if !self.selected.contains(&self.files[i]) {
                        self.selected = HashSet::from([self.files[i].clone()]);
                    }
                }
                Op::DragOut(i) => {
                    let p = &self.files[i];
                    self.drag_out = Some(if self.selected.contains(p) {
                        self.files.iter().filter(|f| self.selected.contains(*f)).cloned().collect()
                    } else {
                        vec![p.clone()]
                    });
                }
            }
        }
    }

    /// Date headings for the grid: (name, index of the first file under it). Empty when not grouping.
    // shortcut: recomputed every frame (formats one date per file); cache it if folders of 50k+ files feel slow.
    fn sections(&self) -> Vec<(String, usize)> {
        if !self.s.group_dates {
            return vec![];
        }
        let mut out: Vec<(String, usize)> = vec![];
        for (i, t) in self.times.iter().enumerate() {
            let s = date_section(*t);
            if out.last().is_none_or(|l| l.0 != s) {
                out.push((s, i));
            }
        }
        out
    }

    /// Plays a hovered video tile muted, after the mouse rests on it for a moment.
    fn hover_preview(&mut self, ctx: &egui::Context, ui: &egui::Ui, view: Rect, hovered: Option<(PathBuf, Rect)>) {
        let Some((p, img)) = hovered else {
            self.hover = None;
            self.hover_player = None;
            return;
        };
        let since = match &self.hover {
            Some((hp, t)) if *hp == p => *t,
            _ => {
                self.hover = Some((p.clone(), Instant::now()));
                Instant::now()
            }
        };
        let wait = Duration::from_millis(500);
        if since.elapsed() < wait {
            ctx.request_repaint_after(wait - since.elapsed());
            return;
        }
        if self.hover_player.as_ref().map(|(hp, _)| hp) != Some(&p) {
            self.hover_player = None;
            self.hover_player = video::Player::open(ctx, &p, 0.0).ok().map(|pl| {
                pl.set_muted(true);
                pl.set_loop(true);
                (p.clone(), pl)
            });
        }
        if let Some((_, pl)) = &mut self.hover_player {
            if let Some(tex) = pl.frame(ctx) {
                let painter = ui.painter().with_clip_rect(img.intersect(view));
                painter.rect_filled(img, 2.0, Color32::BLACK);
                let (paint, _, _) = layout(img, tex.size_vec2(), 0, 1.0, Vec2::ZERO);
                paint_texture(&painter, tex, paint, UV, 0, false, false, Color32::WHITE);
            }
            ctx.request_repaint();
        }
    }

    /// Right-click menu for photos (grid tiles and the big view).
    fn file_menu(&self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        let one = self.mode == Mode::Single || self.selected.len() <= 1;
        item(ui, acts, true, I_FOLDER, "Show in Explorer", "", Action::Reveal);
        item(ui, acts, one, I_EXTERNAL, "Open with...", "", Action::OpenWith);
        item(ui, acts, self.mode == Mode::Single, I_PICTURE, "Set as wallpaper", "", Action::Wallpaper);
        ui.separator();
        item(ui, acts, self.mode == Mode::Single, I_COPY, "Copy image", "Ctrl+C", Action::CopyImage);
        item(ui, acts, true, I_COPY, "Copy path", "Ctrl+Shift+C", Action::CopyPath);
        ui.separator();
        item(ui, acts, one, I_PENCIL, "Rename...", "F2", Action::Rename);
        item(ui, acts, true, I_COPY_TO, "Copy to folder...", "C", Action::Transfer { mv: false });
        item(ui, acts, true, I_MOVE, "Move to folder...", "M", Action::Transfer { mv: true });
        item(ui, acts, true, I_TRASH, "Delete", "Del", Action::Delete);
        ui.separator();
        item(ui, acts, true, I_HEART, "Favorite", "Shift+F", Action::Fav);
        ui.menu_button(label(I_STAR, "Rate"), |ui| {
            for s in 1..=5u8 {
                item(ui, acts, true, I_STAR, &"★".repeat(s as usize), &s.to_string(), Action::Rate(s));
            }
            item(ui, acts, true, I_CROSS, "Clear stars", "0", Action::Rate(0));
        });
        ui.menu_button(label(I_BRUSH, "Color label"), |ui| label_items(ui, acts, true));
        ui.separator();
        item(ui, acts, self.mode == Mode::Grid && self.selected.len() > 1, I_PENCIL, "Rename selected...", "F2", Action::BatchRename);
        item(ui, acts, true, I_IMAGES, "Resize or convert...", "", Action::Convert);
    }

    fn zoom_pan_input(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: Rect) {
        if let Some(m) = resp.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = pinch * (scroll * 0.002).exp();
            if factor != 1.0 {
                // Zoom toward the cursor: keep the pixel under the mouse in place.
                let new = (self.zoom * factor).clamp(1.0, 40.0);
                let c = rect.center() + self.pan;
                self.pan = (m + (c - m) * (new / self.zoom)) - rect.center();
                self.zoom = new;
                if new == 1.0 {
                    self.pan = Vec2::ZERO;
                }
            }
        }
        if resp.dragged() && self.zoom > 1.0 {
            self.pan += resp.drag_delta();
        }
        if resp.double_clicked() {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
        if resp.hovered() && self.zoom > 1.0 {
            let icon = if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab };
            ui.ctx().set_cursor_icon(icon);
        }
    }

    /// Drag sideways (when not zoomed in) to go to the next or previous photo.
    fn swipe_input(&mut self, resp: &egui::Response, acts: &mut Vec<Action>) {
        let n = self.files.len();
        if self.zoom > 1.0 || n < 2 {
            return;
        }
        if resp.dragged() {
            self.swipe += resp.drag_delta().x;
        }
        if resp.drag_stopped() && self.swipe.abs() > 90.0 {
            let i = if self.swipe < 0.0 { (self.index + 1) % n } else { (self.index + n - 1) % n };
            acts.push(Action::Go(i));
        }
    }

    /// What shows around a photo that doesn't fill the view.
    fn paint_backdrop(&self, painter: &egui::Painter, rect: Rect, path: &Path) {
        match self.s.backdrop {
            Backdrop::Black => {
                painter.rect_filled(rect, 0.0, Color32::from_gray(8));
            }
            Backdrop::Gray => {
                painter.rect_filled(rect, 0.0, Color32::from_gray(60));
            }
            Backdrop::Blur => {
                painter.rect_filled(rect, 0.0, Color32::BLACK);
                // A tiny averaged copy stretched to cover the view, darkened so the photo stands out.
                if let Some((_, t)) = self.blur.as_ref().filter(|(p, _)| p == path) {
                    let s = t.size_vec2();
                    let k = (rect.width() / s.x).max(rect.height() / s.y);
                    let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(rect.width() / (s.x * k), rect.height() / (s.y * k)));
                    let mut mesh = egui::Mesh::with_texture(t.id());
                    mesh.add_rect_with_uv(rect, uv, Color32::from_gray(90));
                    painter.add(mesh);
                }
            }
        }
    }

    /// Two-finger sideways swipe on a touchpad (or Shift+wheel) goes to the next/previous photo.
    fn touchpad_swipe(&mut self, ui: &egui::Ui, resp: &egui::Response, acts: &mut Vec<Action>) {
        let n = self.files.len();
        if self.zoom > 1.0 || n < 2 || !resp.hovered() {
            return;
        }
        let dx = ui.input(|i| i.smooth_scroll_delta.x);
        if dx == 0.0 {
            self.swipe_acc = 0.0; // gesture over: ready for the next one
        } else if self.swipe_acc.is_finite() {
            self.swipe_acc += dx;
            if self.swipe_acc.abs() > 140.0 {
                let i = if self.swipe_acc < 0.0 { (self.index + 1) % n } else { (self.index + n - 1) % n };
                acts.push(Action::Go(i));
                self.swipe_acc = f32::INFINITY; // one photo per swipe
            }
        }
    }

    /// The photo with the color sliders applied, while editing. Works on a smaller copy so dragging
    /// a slider stays smooth; the saved copy is made from the full photo.
    fn edit_preview(&mut self, ctx: &egui::Context, path: &Path, d: &Decoded) -> Option<TextureHandle> {
        if !self.editing || !self.adjust.colors_changed() {
            return None;
        }
        if self.edit_base.as_ref().is_none_or(|b| b.0 != path) {
            self.edit_base = Some((path.to_path_buf(), edit::downscale(&d.pixels, 1600), None, Adjust::default()));
        }
        let base = self.edit_base.as_mut()?;
        let colors = |a: &Adjust| (a.brightness, a.contrast, a.saturation);
        if base.2.is_none() || colors(&base.3) != colors(&self.adjust) {
            let img = edit::adjust_color_image(&base.1, &self.adjust);
            match &mut base.2 {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => base.2 = Some(ctx.load_texture("adjusted", img, egui::TextureOptions::LINEAR)),
            }
            base.3 = self.adjust;
        }
        base.2.clone()
    }

    /// Small overview in the corner while zoomed in, with the part on screen outlined.
    fn paint_minimap(&self, painter: &egui::Painter, view: Rect, shown: Rect, tex: &TextureHandle, size: Vec2) {
        let shown_size = if self.rot % 2 == 1 { vec2(size.y, size.x) } else { size };
        let k = (160.0 / shown_size.x).min(120.0 / shown_size.y);
        let box_size = shown_size * k;
        let mm = Rect::from_min_size(view.right_bottom() - box_size - vec2(14.0, 14.0), box_size);
        painter.rect_filled(mm.expand(3.0), 3.0, Color32::from_black_alpha(190));
        let paint = Rect::from_center_size(mm.center(), size * k);
        paint_texture(painter, tex, paint, UV, self.rot, self.flip_h, self.flip_v, Color32::from_gray(200));
        let vis = view.intersect(shown);
        let to_mm = |p: Pos2| mm.min + ((p - shown.min) / shown.size()) * mm.size();
        painter.rect_stroke(Rect::from_min_max(to_mm(vis.min), to_mm(vis.max)), 1.0, egui::Stroke::new(1.5, accent()), egui::StrokeKind::Inside);
    }

    fn crop_input(&mut self, ui: &egui::Ui, resp: &egui::Response, shown: Rect) {
        let norm = |p: Pos2| ((p - shown.min) / shown.size()).to_pos2().clamp(Pos2::ZERO, pos2(1.0, 1.0));
        if resp.dragged() {
            if let (Some(a), Some(p)) = (ui.input(|i| i.pointer.press_origin()), resp.interact_pointer_pos()) {
                let r = edit::crop_rect(norm(a), norm(p), self.aspect, shown.width(), shown.height());
                self.crop = (r.width() > 0.01 && r.height() > 0.01).then_some(r);
            }
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }

    fn single_view(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        let rect = ui.max_rect();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let Some(path) = self.current().cloned() else { return };
        if video::is_video(&path) {
            return self.video_view(ui, rect, &resp, acts);
        }
        let d = match self.cache.get(&path) {
            Some(Slot::Ready(d)) => d.clone(),
            Some(Slot::Failed(e)) => {
                let msg = format!("Can't open {}\n{e}", file_name(&path));
                ui.put(rect, egui::Label::new(RichText::new(msg).weak()));
                self.arrows(ui, rect, acts);
                return;
            }
            _ => {
                ui.put(rect, egui::Spinner::new().size(28.0));
                return;
            }
        };
        let tex = d.size();
        let (_, shown, fit) = layout(rect, tex, self.rot, self.zoom, self.pan);
        self.px_scale = fit * tex.x / d.orig[0].max(1) as f32;
        if std::mem::take(&mut self.apply_fit) {
            self.zoom = match self.s.fit_mode {
                FitMode::Fit => 1.0,
                FitMode::Fill => ((rect.width() / shown.width()).max(rect.height() / shown.height())).max(1.0),
                FitMode::Actual => (1.0 / self.px_scale).max(1.0),
            };
            self.pan = Vec2::ZERO;
            (self.shown_zoom, self.shown_pan) = (self.zoom, self.pan);
        }
        if self.editing {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
            self.crop_input(ui, &resp, shown);
        } else {
            self.zoom_pan_input(ui, &resp, rect);
            self.swipe_input(&resp, acts);
            self.touchpad_swipe(ui, &resp, acts);
        }
        // Ease the shown zoom/pan toward the targets so zooming glides instead of jumping.
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let k = 1.0 - (-dt * 18.0).exp();
        self.shown_zoom += (self.zoom - self.shown_zoom) * k;
        self.shown_pan += (self.pan - self.shown_pan) * k;
        if !resp.dragged() {
            self.swipe *= 1.0 - k; // spring back after a short swipe
        }
        let settled = (self.zoom - self.shown_zoom).abs() < 0.001 && (self.pan - self.shown_pan).length() < 0.5;
        if settled {
            (self.shown_zoom, self.shown_pan) = (self.zoom, self.pan);
        }
        if !settled || self.swipe.abs() > 0.5 {
            ui.ctx().request_repaint();
        } else {
            self.swipe = 0.0;
        }
        let offset = self.shown_pan + vec2(self.swipe, 0.0);
        let (paint, shown, _) = layout(rect, tex, self.rot, self.shown_zoom, offset);
        let (frame, next) = d.frame_at(self.anim_start.elapsed());
        if let Some(n) = next {
            ui.ctx().request_repaint_after(n);
        }
        if self.s.backdrop == Backdrop::Blur && self.blur.as_ref().is_none_or(|(p, _)| *p != path) {
            let small = tiny_copy(&d.pixels, 24);
            self.blur = Some((path.clone(), ui.ctx().load_texture("blur", small, egui::TextureOptions::LINEAR)));
        }
        let painter = ui.painter().with_clip_rect(rect);
        self.paint_backdrop(&painter, rect, &path);
        // Cross-fade from the previous photo.
        let fade = Duration::from_millis(180);
        let mut alpha = 1.0;
        if let Some((old, at)) = &self.fade_from {
            let t = at.elapsed().as_secs_f32() / fade.as_secs_f32();
            if t < 1.0 {
                alpha = t;
                let (op, _, _) = layout(rect, old.size(), 0, 1.0, Vec2::ZERO);
                paint_texture(&painter, &old.frames[0].0, op, UV, 0, false, false, Color32::WHITE.gamma_multiply(1.0 - t));
                ui.ctx().request_repaint();
            }
        }
        if alpha >= 1.0 {
            self.fade_from = None;
        }
        let adjusted = self.edit_preview(ui.ctx(), &path, &d);
        let shown_tex = adjusted.as_ref().unwrap_or(frame);
        if self.editing && self.adjust.angle != 0.0 {
            // Straightening: turn the photo and zoom in so the frame stays filled, with a grid to line things up.
            let k = edit::cover_scale(shown.width(), shown.height(), self.adjust.angle);
            let turned = Rect::from_center_size(paint.center(), paint.size() * k);
            let turn = self.rot as f32 * FRAC_PI_2 + self.adjust.angle.to_radians();
            paint_turned(&painter.with_clip_rect(shown.intersect(rect)), shown_tex, turned, UV, turn, self.flip_h, self.flip_v, Color32::WHITE);
            for i in 1..3 {
                let f = i as f32 / 3.0;
                let line = egui::Stroke::new(1.0, Color32::from_white_alpha(90));
                painter.vline(shown.left() + shown.width() * f, shown.y_range(), line);
                painter.hline(shown.x_range(), shown.top() + shown.height() * f, line);
            }
        } else {
            paint_texture(&painter, shown_tex, paint, UV, self.rot, self.flip_h, self.flip_v, Color32::WHITE.gamma_multiply(alpha));
        }
        if !self.editing && self.minimap && self.shown_zoom > 1.01 {
            self.paint_minimap(&painter, rect, shown, frame, tex);
        }

        if let (true, Some(c)) = (self.editing, self.crop) {
            // Shade everything outside the crop.
            let s = Rect::from_min_max(shown.min + c.min.to_vec2() * shown.size(), shown.min + c.max.to_vec2() * shown.size());
            let shade = Color32::from_black_alpha(150);
            painter.rect_filled(Rect::from_min_max(shown.min, pos2(shown.max.x, s.min.y)), 0.0, shade);
            painter.rect_filled(Rect::from_min_max(pos2(shown.min.x, s.max.y), shown.max), 0.0, shade);
            painter.rect_filled(Rect::from_min_max(pos2(shown.min.x, s.min.y), pos2(s.min.x, s.max.y)), 0.0, shade);
            painter.rect_filled(Rect::from_min_max(pos2(s.max.x, s.min.y), pos2(shown.max.x, s.max.y)), 0.0, shade);
            painter.rect_stroke(s, 0.0, egui::Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Outside);
        }

        // Pixel position + color under the mouse, and the loupe.
        if let Some(m) = resp.hover_pos().filter(|m| shown.contains(*m)) {
            let u = (m - shown.min) / shown.size();
            let (x, y) = display_to_texture(u.x, u.y, self.rot, self.flip_h, self.flip_v);
            let [w, h] = d.pixels.size;
            let (px, py) = (((x * w as f32) as usize).min(w - 1), ((y * h as f32) as usize).min(h - 1));
            let [r, g, b, _] = d.pixels[(px, py)].to_srgba_unmultiplied();
            let k = d.orig[0] as f32 / w as f32;
            self.readout = Some(format!("{}, {}   #{r:02X}{g:02X}{b:02X}", (px as f32 * k) as u32, (py as f32 * k) as u32));
            if self.loupe && !self.editing {
                let side = 180.0;
                let mut lr = Rect::from_min_size(m + vec2(24.0, 24.0), Vec2::splat(side));
                // Keep the loupe inside the view.
                lr = lr.translate(vec2((rect.right() - lr.right()).min(0.0), (rect.bottom() - lr.bottom()).min(0.0)));
                let span = 30.0; // texture pixels across the loupe
                let uv = Rect::from_center_size(pos2(x, y), vec2(span / tex.x, span / tex.y));
                let lp = ui.painter().with_clip_rect(lr);
                lp.rect_filled(lr, 0.0, Color32::BLACK);
                paint_texture(&lp, frame, lr, uv, self.rot, self.flip_h, self.flip_v, Color32::WHITE);
                ui.painter().rect_stroke(lr, 2.0, egui::Stroke::new(1.5, Color32::from_gray(200)), egui::StrokeKind::Outside);
            }
        }
        if !self.editing && !self.idle() {
            self.arrows(ui, rect, acts);
        }
        resp.context_menu(|ui| self.file_menu(ui, acts));
    }

    /// A video with a control bar: play/pause, time, seek bar and volume. Click the video to pause.
    fn video_view(&mut self, ui: &mut egui::Ui, rect: Rect, resp: &egui::Response, acts: &mut Vec<Action>) {
        let ctx = ui.ctx().clone();
        // Sits just above the filmstrip while that's showing.
        let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 44.0 - self.strip_lift), pos2(rect.right(), rect.bottom() - self.strip_lift));
        let over_bar = ui.rect_contains_pointer(bar);
        let idle = self.idle();
        if self.pip.as_ref().is_some_and(|(p, _)| self.current() == Some(p)) {
            ui.put(rect.shrink(40.0), egui::Label::new(RichText::new(label(I_EXTERNAL, "Playing in the small window")).weak()));
            let back = Rect::from_center_size(rect.center() + vec2(0.0, 30.0), vec2(160.0, 26.0));
            if ui.put(back, egui::Button::new("Play here instead")).clicked() {
                acts.push(Action::Pip);
            }
            return self.arrows(ui, rect, acts);
        }
        let mut failed = None;
        let mut resumed = None;
        let (mut volume, mut muted, mut looping, mut speed) = (self.s.volume, self.s.muted, self.s.video_loop, self.speed);
        let mut subs_on = self.s.subtitles;
        let trim = self.trim;
        match &mut self.player {
            Some((_, Err(e))) => failed = Some(e.clone()),
            Some((path, Ok(p))) => {
                // Pick up where this video was left last time, once it knows its length.
                if let Some(at) = self.pending_resume.filter(|_| p.duration() > 0.0) {
                    p.seek(at);
                    self.pending_resume = None;
                    resumed = Some(at);
                }
                if let Some(e) = p.error() {
                    failed = Some(e);
                } else {
                    match p.frame(&ctx) {
                        Some(tex) => {
                            // Videos always fill the view (photos stop at 100%), so fullscreen really is full.
                            let s = tex.size_vec2();
                            let paint = Rect::from_center_size(rect.center(), s * (rect.width() / s.x).min(rect.height() / s.y));
                            paint_texture(&ui.painter().with_clip_rect(rect), tex, paint, UV, 0, false, false, Color32::WHITE);
                        }
                        None => {
                            ui.put(rect, egui::Spinner::new().size(28.0));
                        }
                    }
                    // Subtitles from a .srt next to the video.
                    let cue = self.subs.as_ref().filter(|(sp, _)| sp == path && subs_on).and_then(|(_, cues)| {
                        let t = p.time();
                        cues.iter().find(|c| c.start <= t && t < c.end)
                    });
                    if let Some(c) = cue {
                        let font = egui::FontId::proportional((rect.height() / 22.0).clamp(15.0, 30.0));
                        let g = ui.painter().layout(c.text.clone(), font, Color32::WHITE, rect.width() * 0.8);
                        let r = Rect::from_center_size(pos2(rect.center().x, bar.min.y - 18.0 - g.size().y / 2.0), g.size() + vec2(16.0, 8.0));
                        ui.painter().rect_filled(r, 3.0, Color32::from_black_alpha(170));
                        ui.painter().galley(r.min + vec2(8.0, 4.0), g, Color32::WHITE);
                    }
                    let playing = p.playing();
                    if resp.clicked() && !over_bar {
                        p.toggle();
                    }
                    if resp.double_clicked() && !over_bar {
                        acts.push(Action::Fullscreen);
                        p.toggle(); // undo the pause from the double-click's first click
                    }
                    if playing {
                        ctx.request_repaint(); // keep pulling frames
                    }
                    // Controls float over the video and fade out while it plays untouched.
                    let shown = ctx.animate_bool_with_time(ui.scope_id().with("vbar"), !idle || !playing || over_bar, anim(0.25));
                    if shown > 0.0 {
                        let mut bar_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(bar.shrink2(vec2(10.0, 6.0)))
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        bar_ui.multiply_opacity(shown);
                        bar_ui.painter().rect_filled(bar, 0.0, Color32::from_black_alpha(170));
                        let ui = &mut bar_ui;
                        if icon_button(ui, I_BACK10, "Back 10 seconds (J)").clicked() {
                            acts.push(Action::Seek(-10.0));
                        }
                        let (icon, tip) = if playing { (I_PAUSE, "Pause (Space / K)") } else { (I_PLAY, "Play (Space / K)") };
                        if icon_button(ui, icon, tip).clicked() {
                            p.toggle();
                        }
                        if icon_button(ui, I_FWD10, "Forward 10 seconds (L)").clicked() {
                            acts.push(Action::Seek(10.0));
                        }
                        let (t, d) = (p.time(), p.duration());
                        ui.label(RichText::new(format!("{} / {}", video::clock(t), video::clock(d))).monospace().color(Color32::from_gray(210)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().slider_width = 80.0;
                            if ui.add(egui::Button::new(RichText::new(I_EXPAND).size(15.0)).selected(self.fullscreen)).on_hover_text(if self.fullscreen { "Exit fullscreen (F / Esc)" } else { "Fullscreen (F)" }).clicked() {
                                acts.push(Action::Fullscreen);
                            }
                            if ui.add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false)).on_hover_text("Volume").changed() {
                                p.set_volume(volume as f64);
                                if muted {
                                    muted = false;
                                    p.set_muted(false);
                                }
                            }
                            let vol_icon = if muted || volume == 0.0 { I_MUTE } else { I_VOLUME };
                            if icon_button(ui, vol_icon, if muted { "Unmute (M)" } else { "Mute (M)" }).clicked() {
                                muted = !muted;
                                p.set_muted(muted);
                            }
                            if ui.add(egui::Button::new(RichText::new(I_LOOP).size(15.0)).selected(looping)).on_hover_text("Loop").clicked() {
                                looping = !looping;
                                p.set_loop(looping);
                            }
                            ui.menu_button(format!("{speed}×"), |ui| {
                                for r in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0] {
                                    if ui.add(egui::Button::new(format!("{r}×")).selected(speed == r)).clicked() {
                                        speed = r;
                                        p.set_rate(r);
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Playback speed");
                            if self.subs.as_ref().is_some_and(|(sp, c)| sp == path && !c.is_empty()) {
                                if ui.add(egui::Button::new(RichText::new("CC").size(12.0).strong()).selected(subs_on)).on_hover_text("Subtitles").clicked() {
                                    subs_on = !subs_on;
                                }
                            }
                            if icon_button(ui, I_EXTERNAL, "Picture in picture (P)").clicked() {
                                acts.push(Action::Pip);
                            }
                            if icon_button(ui, I_PICTURE, "Save this frame as a picture (S)").clicked() {
                                acts.push(Action::SaveFrame);
                            }
                            ui.menu_button(RichText::new(I_CROP).size(15.0), |ui| {
                                item(ui, acts, true, I_LEFT, "Start here", "[", Action::TrimMark(true));
                                item(ui, acts, true, I_RIGHT, "End here", "]", Action::TrimMark(false));
                                ui.separator();
                                let ready = trim.0.is_some() || trim.1.is_some();
                                item(ui, acts, ready, I_DOWNLOAD, "Save trimmed copy", "", Action::TrimSave);
                                if ui.add_enabled(ready, egui::Button::new(label(I_CROSS, "Clear marks"))).clicked() {
                                    self.trim = (None, None);
                                }
                            })
                            .response
                            .on_hover_text("Trim");
                            ui.separator();
                            ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(40.0);
                            let mut pos = t;
                            let seek = ui.add(egui::Slider::new(&mut pos, 0.0..=d.max(0.01)).show_value(false));
                            if seek.changed() {
                                p.seek(pos);
                            }
                            // Trim marks on the seek bar.
                            let x_at = |s: f64| seek.rect.left() + (s / d.max(0.01)) as f32 * seek.rect.width();
                            for m in [trim.0, trim.1].into_iter().flatten() {
                                let x = x_at(m);
                                ui.painter().line_segment([pos2(x, seek.rect.top() - 2.0), pos2(x, seek.rect.bottom() + 2.0)], egui::Stroke::new(3.0, accent()));
                            }
                            // Hovering the seek bar shows that moment: a frame and the time.
                            if let Some(m) = seek.hover_pos() {
                                let at = ((m.x - seek.rect.left()) / seek.rect.width()).clamp(0.0, 1.0) as f64 * d;
                                if self.scrub.as_ref().is_none_or(|s| s.0 != *path) {
                                    self.scrub = video::Player::open(&ctx, path, 0.0).ok().map(|sp| {
                                        sp.set_muted(true);
                                        sp.pause();
                                        (path.clone(), sp, -1.0, Instant::now())
                                    });
                                }
                                let mut still = None;
                                if let Some((_, sp, last, when)) = &mut self.scrub {
                                    if (at - *last).abs() > 0.2 && when.elapsed() > Duration::from_millis(100) {
                                        sp.seek(at);
                                        (*last, *when) = (at, Instant::now());
                                    }
                                    still = sp.frame(&ctx).cloned();
                                    ctx.request_repaint_after(Duration::from_millis(50));
                                }
                                seek.on_hover_ui_at_pointer(|ui| {
                                    if let Some(t) = still {
                                        let s = t.size_vec2();
                                        ui.image((t.id(), s * (176.0 / s.x)));
                                    }
                                    ui.label(video::clock(at));
                                });
                            }
                        });
                    }
                }
            }
            None => {}
        }
        (self.s.volume, self.s.muted, self.s.video_loop, self.speed) = (volume, muted, looping, speed);
        self.s.subtitles = subs_on;
        if let Some(at) = resumed {
            self.toast(format!("Resumed at {}", video::clock(at)));
        }
        if let Some(e) = failed {
            ui.put(rect, egui::Label::new(RichText::new(format!("Can't play this video\n{e}")).weak()));
        }
        if !idle {
            self.arrows(ui, rect.with_max_y(bar.min.y), acts);
        }
    }

    /// Remembers where an unfinished video was stopped (not the first or last few seconds).
    fn remember_position(&mut self, path: PathBuf, t: f64, d: f64) {
        if t > 10.0 && t < d - 10.0 {
            self.s.video_pos.insert(path, t);
        } else {
            self.s.video_pos.remove(&path);
        }
    }

    /// The row of thumbnails along the bottom of the big view, drawn over the photo at `alpha`.
    fn filmstrip(&mut self, ctx: &egui::Context, view: Rect, alpha: f32, acts: &mut Vec<Action>) {
        let rect = Rect::from_min_max(pos2(view.left(), view.bottom() - STRIP_H), view.right_bottom());
        egui::Area::new(egui::Id::unique("filmstrip"))
            .fixed_pos(rect.min)
            .interactable(alpha > 0.5)
            .show(ctx, |ui| {
                ui.multiply_opacity(alpha);
                ui.set_width(rect.width());
                egui::Frame::NONE.fill(ui.visuals().panel_fill.gamma_multiply(0.9)).inner_margin(egui::Margin::symmetric(6, 6)).show(ui, |ui| {
                    ui.set_width(rect.width() - 12.0);
                    let scroll = std::mem::take(&mut self.scroll_to_current);
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (i, p) in self.files.iter().enumerate() {
                                let info = TileInfo { current: i == self.index, video: video::is_video(p), cover: true, ..Default::default() };
                                let resp = thumb_tile(ui, vec2(64.0, STRIP_H - 12.0), self.thumbs.get(p), info);
                                if i == self.index && scroll {
                                    resp.scroll_to_me(Some(egui::Align::Center));
                                }
                                if resp.on_hover_text(file_name(p)).clicked() {
                                    acts.push(Action::Go(i));
                                }
                            }
                        });
                    });
                });
            });
    }

    /// No mouse or keyboard for a few seconds: bars and controls in the big view step aside.
    fn idle(&self) -> bool {
        self.last_activity.elapsed() > Duration::from_millis(2500)
    }

    /// Prev/next tabs at the edges, only while the mouse is over the photo.
    fn arrows(&self, ui: &mut egui::Ui, rect: Rect, acts: &mut Vec<Action>) {
        let n = self.files.len();
        if n < 2 || !ui.rect_contains_pointer(rect) {
            return;
        }
        let arrow = |t: &str| {
            egui::Button::new(RichText::new(t).size(20.0).color(Color32::from_gray(220)))
                .corner_radius(3.0)
                .fill(Color32::from_rgba_unmultiplied(20, 20, 20, 170))
                .stroke(egui::Stroke::NONE)
        };
        let at = |x: f32| Rect::from_center_size(pos2(x, rect.center().y), vec2(34.0, 60.0));
        if ui.put(at(rect.left() + 26.0), arrow(I_LEFT)).clicked() {
            acts.push(Action::Go((self.index + n - 1) % n));
        }
        if ui.put(at(rect.right() - 26.0), arrow(I_RIGHT)).clicked() {
            acts.push(Action::Go((self.index + 1) % n));
        }
    }

    fn compare_view(&mut self, ui: &mut egui::Ui) {
        let Some(pair) = self.compare.clone() else { return };
        let rect = ui.max_rect();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let mid = rect.center().x;
        let halves = [rect.with_max_x(mid - 1.0), rect.with_min_x(mid + 1.0)];
        // Both sides share zoom and pan, so the same spot lines up.
        let hovered = resp.hover_pos().map_or(halves[0], |m| if m.x < mid { halves[0] } else { halves[1] });
        self.zoom_pan_input(ui, &resp, hovered);
        ui.painter().vline(mid, rect.y_range(), egui::Stroke::new(2.0, ui.visuals().panel_fill));
        for (half, p) in halves.into_iter().zip(&pair) {
            match self.cache.get(p) {
                Some(Slot::Ready(d)) => {
                    let (paint, _, _) = layout(half, d.size(), 0, self.zoom, self.pan);
                    paint_texture(&ui.painter().with_clip_rect(half), &d.frames[0].0, paint, UV, 0, false, false, Color32::WHITE);
                }
                Some(Slot::Failed(e)) => {
                    ui.put(half, egui::Label::new(RichText::new(e).weak()));
                }
                _ => {
                    ui.put(half, egui::Spinner::new());
                }
            }
            let font = egui::FontId::proportional(13.0);
            let label = ui.painter().layout_no_wrap(file_name(p), font, Color32::WHITE);
            let bg = Rect::from_min_size(half.min + vec2(8.0, 8.0), label.size() + vec2(12.0, 6.0));
            ui.painter().rect_filled(bg, 3.0, Color32::from_black_alpha(160));
            ui.painter().galley(bg.min + vec2(6.0, 3.0), label, Color32::WHITE);
        }
    }

    fn modals(&mut self, ctx: &egui::Context) {
        let Some(modal) = &mut self.modal else { return };
        let (mut confirm, mut close) = (false, false);
        let mut permanent = false;
        let response = match modal {
            Modal::Delete(paths) => egui::Modal::new(egui::Id::unique("delete")).show(ctx, |ui| {
                ui.set_width(340.0);
                let folders = paths.iter().filter(|p| p.is_dir()).count();
                let noun = if folders == paths.len() { "folder" } else if folders > 0 { "item" } else { "file" };
                ui.label(RichText::new(label(I_TRASH, &format!("Delete {}?", plural(paths.len(), noun)))).strong());
                if folders > 0 {
                    ui.label(RichText::new("Everything inside goes with it.").color(Color32::from_rgb(220, 110, 100)));
                }
                ui.add_space(6.0);
                for p in paths.iter().take(5) {
                    ui.label(RichText::new(file_name(p)).weak());
                }
                if paths.len() > 5 {
                    ui.label(RichText::new(format!("and {} more", paths.len() - 5)).weak());
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    confirm = ui.add(primary_button("Move to Recycle Bin")).on_hover_text("You can get them back from the Recycle Bin, or with Undo").clicked()
                        || ui.input(|i| i.key_pressed(Key::Enter));
                    let red = Color32::from_rgb(196, 52, 52);
                    let erase = egui::Button::new(RichText::new("Delete Permanently").color(Color32::WHITE)).fill(red);
                    if ui.add(erase).on_hover_text("Erases them right away. This can't be undone.").clicked() {
                        (confirm, permanent) = (true, true);
                    }
                    close = ui.button("Cancel").clicked();
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Enter moves to the Recycle Bin · Esc cancels · Delete Permanently can't be undone").weak().small());
            }),
            Modal::Rename { name, error, focus, .. } => egui::Modal::new(egui::Id::unique("rename")).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(RichText::new(label(I_PENCIL, "Rename")).strong());
                ui.add_space(6.0);
                let edit = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                if std::mem::take(focus) {
                    edit.request_focus();
                }
                if !error.is_empty() {
                    ui.label(RichText::new(error.as_str()).color(Color32::from_rgb(220, 110, 100)));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    confirm = ui.add(primary_button("Rename")).clicked() || enter;
                    close = ui.button("Cancel").clicked();
                    ui.label(RichText::new("Enter · Esc").weak().small());
                });
            }),
            Modal::NewFolder { parent, name, error, focus } => egui::Modal::new(egui::Id::unique("newfolder")).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(RichText::new(label(I_FOLDER, "New folder")).strong());
                ui.label(RichText::new(format!("In {}", parent.display())).weak().small());
                ui.add_space(6.0);
                let edit = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                if std::mem::take(focus) {
                    edit.request_focus();
                    // Select the whole name so typing replaces it.
                    if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), edit.id) {
                        st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(name.chars().count()))));
                        st.store(ui.ctx(), edit.id);
                    }
                }
                if !error.is_empty() {
                    ui.label(RichText::new(error.as_str()).color(Color32::from_rgb(220, 110, 100)));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    confirm = ui.add(primary_button("Create")).clicked() || enter;
                    close = ui.button("Cancel").clicked();
                    ui.label(RichText::new("Enter · Esc").weak().small());
                });
            }),
            Modal::BatchRename { paths, pattern, error, focus } => egui::Modal::new(egui::Id::unique("batch")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new(label(I_PENCIL, &format!("Rename {}", plural(paths.len(), "file")))).strong());
                ui.add_space(4.0);
                ui.label(RichText::new("# becomes a number (### = 001), * the old name").weak().small());
                ui.add_space(6.0);
                let edit = ui.add(egui::TextEdit::singleline(pattern).desired_width(f32::INFINITY));
                if std::mem::take(focus) {
                    edit.request_focus();
                }
                // Preview the first few new names.
                for (i, p) in paths.iter().take(3).enumerate() {
                    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let ext = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(RichText::new(format!("{}  →  {}", file_name(p), files::pattern_name(pattern.trim(), &stem, i + 1, &ext))).weak());
                }
                if paths.len() > 3 {
                    ui.label(RichText::new("…").weak());
                }
                if !error.is_empty() {
                    ui.label(RichText::new(error.as_str()).color(Color32::from_rgb(220, 110, 100)));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    confirm = ui.add(primary_button("Rename")).clicked() || enter;
                    close = ui.button("Cancel").clicked();
                    ui.label(RichText::new("Enter · Esc").weak().small());
                });
            }),
            Modal::Convert { paths, format, max_side } => egui::Modal::new(egui::Id::unique("convert")).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(RichText::new(label(I_IMAGES, &format!("Resize or convert {}", plural(paths.len(), "photo")))).strong());
                ui.add_space(8.0);
                egui::Grid::new("convert_opts").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Format");
                    egui::ComboBox::from_id_salt("fmt").selected_text(CONVERT_FORMATS[*format].0).show_ui(ui, |ui| {
                        for (i, (name, _)) in CONVERT_FORMATS.iter().enumerate() {
                            ui.selectable_value(format, i, *name);
                        }
                    });
                    ui.end_row();
                    ui.label("Longest side");
                    egui::ComboBox::from_id_salt("size").selected_text(CONVERT_SIZES[*max_side].0).show_ui(ui, |ui| {
                        for (i, (name, _)) in CONVERT_SIZES.iter().enumerate() {
                            ui.selectable_value(max_side, i, *name);
                        }
                    });
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Copies go in a \"converted\" folder; the originals stay as they are.").weak().small());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    confirm = ui.add(primary_button("Convert")).clicked() || ui.input(|i| i.key_pressed(Key::Enter));
                    close = ui.button("Cancel").clicked();
                    ui.label(RichText::new("Enter · Esc").weak().small());
                });
            }),
        };
        if response.should_close() {
            close = true;
        }
        if confirm {
            match self.modal.take() {
                Some(Modal::Delete(paths)) => self.delete(paths, permanent),
                Some(Modal::Rename { path, name, focus, .. }) => {
                    if let Err(error) = self.rename(&path, &name) {
                        self.modal = Some(Modal::Rename { path, name, error, focus });
                    }
                }
                Some(Modal::NewFolder { parent, name, focus, .. }) => match files::new_folder(&parent, &name) {
                    Ok(dir) => {
                        self.tree.remove(&parent); // the sidebar lists it next time
                        if self.folder.as_ref() == Some(&parent) {
                            self.relist(ctx);
                        }
                        self.toast(format!("Created {}", file_name(&dir)));
                    }
                    Err(error) => self.modal = Some(Modal::NewFolder { parent, name, error, focus }),
                },
                Some(Modal::BatchRename { paths, pattern, focus, .. }) => match files::batch_rename(&paths, &pattern) {
                    Ok(pairs) => {
                        for (o, n) in &pairs {
                            self.rekey(o, n);
                        }
                        self.relist(ctx);
                        let msg = format!("Renamed {}", plural(pairs.len(), "file"));
                        self.toast_undo(msg, Some(Undo::MoveBack(pairs)));
                    }
                    Err(error) => self.modal = Some(Modal::BatchRename { paths, pattern, error, focus }),
                },
                Some(Modal::Convert { paths, format, max_side }) => self.convert(ctx, paths, format, max_side),
                None => {}
            }
        } else if close {
            self.modal = None;
        }
    }

    /// Drop highlight and the toast. Returns an Undo when its button was clicked.
    fn overlays(&mut self, ctx: &egui::Context, full: Rect) -> Option<Undo> {
        // Dim the window with a thin outline while files are dragged over it.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            painter.rect_filled(full, 0.0, Color32::from_black_alpha(150));
            painter.rect_stroke(full.shrink(6.0), 3.0, egui::Stroke::new(1.5, accent()), egui::StrokeKind::Inside);
            let font = egui::FontId::proportional(16.0);
            painter.text(full.center(), egui::Align2::CENTER_CENTER, label(I_DOWNLOAD, "Drop to open"), font, Color32::from_gray(220));
        }
        let toast = self.toast.as_ref()?;
        // Toasts with Undo stay longer, and while the mouse is on them.
        let life = Duration::from_millis(if toast.undo.is_some() { 8000 } else { 3500 });
        let left = life.saturating_sub(toast.at.elapsed());
        if left.is_zero() {
            self.toast = None;
            return None;
        }
        let mut undo_clicked = false;
        let mut dismiss = false;
        let ease = |secs: f32, d: Duration| if anim(secs) == 0.0 { 1.0 } else { (d.as_secs_f32() / secs).min(1.0) };
        let (fade_in, fade_out) = (ease(0.18, toast.at.elapsed()), ease(0.3, left));
        let area = egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -56.0 + 8.0 * (1.0 - fade_in)))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.multiply_opacity(fade_in.min(fade_out));
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(&toast.msg);
                        if toast.undo.is_some() {
                            ui.add_space(8.0);
                            undo_clicked = ui.button(label(I_UNDO, "Undo")).clicked();
                        }
                        dismiss = ui.add(egui::Button::new(RichText::new(I_CROSS).size(10.0)).frame(false)).clicked();
                    });
                });
            });
        if ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| area.response.rect.contains(p))) {
            if let Some(t) = &mut self.toast {
                t.at = Instant::now() - Duration::from_millis(500); // hold it while the mouse is on it
            }
        }
        if fade_in < 1.0 || fade_out < 1.0 {
            ctx.request_repaint();
        }
        ctx.request_repaint_after(left.min(Duration::from_millis(500)));
        if undo_clicked || dismiss {
            return self.toast.take().and_then(|t| t.undo.filter(|_| undo_clicked));
        }
        None
    }

    fn about(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let resp = egui::Modal::new(egui::Id::unique("about")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(I_PICTURE).size(28.0).color(accent()));
                ui.vertical(|ui| {
                    ui.label(RichText::new("Glance").size(17.0).strong());
                    ui.label(RichText::new(format!("Photo and Video Viewer · version {}", env!("CARGO_PKG_VERSION"))).weak());
                });
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label("Icons: ");
                ui.hyperlink_to("Uicons by Flaticon", "https://www.flaticon.com/uicons");
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Close").clicked() {
                    self.show_about = false;
                }
                ui.label(RichText::new("Esc").weak().small());
            });
        });
        if resp.should_close() {
            self.show_about = false;
        }
    }
}

impl eframe::App for Viewer {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some((p, Ok(pl))) = &self.player {
            self.remember_position(p.clone(), pl.time(), pl.duration());
        }
        eframe::set_value(storage, "settings", &self.s);
    }

    fn clear_color(&self, v: &egui::Visuals) -> [f32; 4] {
        if self.s.glass { [0.0; 4] } else { v.panel_fill.to_normalized_gamma_f32() }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let want = (self.s.glass, ctx.theme() == egui::Theme::Dark);
        if self.backdrop != Some(want) {
            set_backdrop(self.hwnd, want.0, want.1);
            self.backdrop = Some(want);
        }
        self.poll(&ctx);
        self.readout = None;
        if ctx.input(|i| !i.events.is_empty() || i.pointer.is_moving()) {
            self.last_activity = Instant::now();
        }
        // Keep a player only while a video is on screen; dropping it stops playback.
        let in_pip = |p: &PathBuf| self.pip.as_ref().is_some_and(|(pp, _)| pp == p);
        let want = self.current().filter(|p| self.mode == Mode::Single && video::is_video(p) && !in_pip(p)).cloned();
        if self.player.as_ref().map(|(p, _)| p) != want.as_ref() {
            if let Some((p, Ok(pl))) = &self.player {
                self.remember_position(p.clone(), pl.time(), pl.duration());
            }
            self.player = None; // shut the old one down before opening the next
            self.scrub = None;
            self.pending_resume = want.as_ref().and_then(|p| self.s.video_pos.get(p).copied()).filter(|_| self.s.resume);
            if let Some(p) = &want {
                if self.subs.as_ref().is_none_or(|(sp, _)| sp != p) {
                    self.subs = Some((p.clone(), video::find_subtitles(p)));
                }
            }
            self.player = want.map(|p| {
                let player = video::Player::open(&ctx, &p, self.s.volume as f64);
                if let Ok(pl) = &player {
                    pl.set_muted(self.s.muted);
                    pl.set_loop(self.s.video_loop);
                    pl.set_rate(self.speed);
                }
                (p, player)
            });
        }
        if self.mode != Mode::Grid {
            self.hover = None;
            self.hover_player = None;
        }
        let mut acts = vec![];
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            // Our own drag landing back on Glance: onto a folder tile moves the photos in;
            // anywhere else does nothing (they're already here).
            let own = self.own_drop.take().filter(|(paths, _, at)| {
                let same: HashSet<&PathBuf> = paths.iter().collect();
                same == dropped.iter().collect::<HashSet<_>>() && at.elapsed() < Duration::from_secs(3)
            });
            if let Some((paths, pos, _)) = own {
                if let Some((dest, _)) = self.folder_rects.iter().find(|(_, r)| r.contains(pos)).cloned() {
                    self.selected.clear();
                    self.transfer_to(&ctx, paths, dest, true);
                }
            } else {
                acts.push(Action::Open(dropped[0].clone()));
            }
        }
        if self.modal.is_none() && !self.show_about && !ctx.egui_wants_keyboard_input() {
            self.keys(&ctx, &mut acts);
        }

        let full = ui.max_rect();
        let single = self.mode == Mode::Single;
        // In the big view the bottom bars slide away while the mouse rests (not while editing or paused on a menu).
        let paused_video = matches!(&self.player, Some((_, Ok(p))) if !p.playing());
        // Resting the mouse on the bottom controls keeps them up.
        let over_bottom = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| p.y > full.bottom() - 120.0);
        let idle = single && !self.editing && !paused_video && !over_bottom && self.modal.is_none() && self.idle();
        // The filmstrip floats over the bottom of the photo and fades out while you're just looking.
        let strip_alpha = ctx.animate_bool_with_time(egui::Id::unique("strip_fade"), single && !idle && !self.clean, anim(0.35));
        self.strip_lift = STRIP_H * strip_alpha;
        if single && !idle {
            ctx.request_repaint_after(Duration::from_millis(2600).saturating_sub(self.last_activity.elapsed()));
        }
        if idle && ui.rect_contains_pointer(full) {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
        if !self.fullscreen && !(self.clean && single) {
            egui::Panel::top("menu")
                .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(egui::Margin { left: 8, ..Default::default() }))
                .show(ui, |ui| {
                    // The bar doubles as the title bar: drag it to move the window, double-click to maximize.
                    // Created first so the menus and buttons on it still get their clicks.
                    let grip = ui.interact(ui.max_rect(), ui.scope_id().with("titlebar"), Sense::click_and_drag());
                    if grip.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if grip.double_clicked() {
                        let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
                    }
                    self.menu_bar(ui, &mut acts)
                });
            if self.mode == Mode::Grid || (single && self.editing) {
                egui::Panel::top("tools")
                    .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(8, 5)))
                    .show(ui, |ui| self.tool_row(ui, &mut acts));
            }
            egui::Panel::bottom("status")
                .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(8, 3)))
                .show(ui, |ui| self.status_bar(ui, &mut acts));
            if self.s.sidebar {
                egui::Panel::left("sidebar")
                    .default_size(210.0)
                    .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(6, 6)))
                    .show(ui, |ui| self.sidebar(ui, &mut acts));
            }
            if single && self.show_info {
                egui::Panel::right("info").default_size(270.0).show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.info_panel(ui, &mut acts));
                });
            }
        }

        let photo_bg = matches!(self.mode, Mode::Single | Mode::Compare);
        let bg = if photo_bg { ui.visuals().extreme_bg_color } else { ui.visuals().panel_fill };
        let margin = if self.mode == Mode::Grid { 14.0 } else { 0.0 };
        let central = egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(bg).inner_margin(margin))
            .show(ui, |ui| match self.mode {
                Mode::Home => self.home(ui, &mut acts),
                Mode::Grid => self.grid_view(ui, &mut acts),
                Mode::Single => self.single_view(ui, &mut acts),
                Mode::Compare => self.compare_view(ui),
            })
            .response
            .rect;
        // A new screen (folder, photo, home) fades in quickly from the background colour.
        let key = (self.folder.clone().filter(|_| self.mode != Mode::Home), matches!(self.mode, Mode::Single | Mode::Compare));
        if key != self.view_key {
            self.view_key = key;
            self.view_since = Instant::now();
        }
        let secs = anim(0.16);
        let t = if secs == 0.0 { 1.0 } else { self.view_since.elapsed().as_secs_f32() / secs };
        if t < 1.0 {
            ui.painter().rect_filled(central, 0.0, bg.gamma_multiply((1.0 - t) * (1.0 - t)));
            ctx.request_repaint();
        }
        if strip_alpha > 0.0 {
            self.filmstrip(&ctx, central, strip_alpha, &mut acts);
        }

        self.modals(&ctx);
        self.about(&ctx);
        self.shortcuts_window(&ctx);
        self.settings_window(&ctx, &mut acts);
        self.phone_window(&ctx, &mut acts);
        self.pip_window(&ctx, &mut acts);
        if let Some(u) = self.overlays(&ctx, full) {
            self.undo(&ctx, u);
        }
        for a in acts {
            self.run(&ctx, a);
        }

        if let Some(t) = self.slideshow {
            let per = Duration::from_secs_f32(self.s.slide_secs);
            let n = self.files.len();
            if self.mode != Mode::Single || n < 2 {
                self.slideshow = None;
            } else if t.elapsed() >= per {
                self.go((self.index + 1) % n);
            } else {
                ctx.request_repaint_after(per - t.elapsed());
            }
        }
        self.sync_cache(&ctx);
        resize_edges(ui, full, self.fullscreen);
        // Dragging tiles out to other apps: Windows runs the drag until the drop.
        if let Some(paths) = self.drag_out.take() {
            dragout::drag_files(&paths);
            // Windows runs the whole drag inside that call; note where it was let go in case it
            // was dropped back on one of our folder tiles.
            if let Some(pos) = cursor_in_window(self.hwnd, ctx.pixels_per_point()) {
                self.own_drop = Some((paths, pos, Instant::now()));
            }
        }

        let title = match (self.mode, &self.folder, self.current()) {
            (Mode::Single, _, Some(p)) => format!("{} ({}/{}) - {APP}", file_name(p), self.index + 1, self.files.len()),
            (Mode::Compare, ..) => format!("Compare - {APP}"),
            (_, Some(f), _) => format!("{} - {APP}", file_name(f)),
            _ => format!("{APP}: Photo and Video Viewer"),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

fn window_handle(cc: &eframe::CreationContext) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

/// Turns the Windows 11 acrylic blur behind the window on or off (older Windows just ignores it).
#[cfg(windows)]
fn set_backdrop(hwnd: isize, glass: bool, dark: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::*;
    if hwnd == 0 {
        return;
    }
    let h = HWND(hwnd as _);
    let kind = if glass { DWMSBT_TRANSIENTWINDOW } else { DWMSBT_NONE };
    // SAFETY: plain attribute calls on our own window, with correctly sized values.
    unsafe {
        let dark = windows_core::BOOL::from(dark);
        let _ = DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        let _ = DwmSetWindowAttribute(h, DWMWA_SYSTEMBACKDROP_TYPE, &kind as *const _ as _, 4);
    }
}

/// Windows' "Animation effects" setting (Settings > Accessibility > Visual effects).
#[cfg(windows)]
fn windows_animations() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::*;
    let mut on = windows_core::BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL into `on`.
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    ok.is_err() || on.as_bool()
}

#[cfg(not(windows))]
fn windows_animations() -> bool {
    true
}

/// Where the mouse is now, in this window's coordinates.
#[cfg(windows)]
fn cursor_in_window(hwnd: isize, ppp: f32) -> Option<Pos2> {
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT::default();
    // SAFETY: both calls only write into `pt`.
    unsafe {
        GetCursorPos(&mut pt).ok()?;
        ScreenToClient(HWND(hwnd as _), &mut pt).ok().ok()?;
    }
    Some(pos2(pt.x as f32 / ppp, pt.y as f32 / ppp))
}

#[cfg(not(windows))]
fn cursor_in_window(_: isize, _: f32) -> Option<Pos2> {
    None
}

#[cfg(not(windows))]
fn set_backdrop(_: isize, _: bool, _: bool) {}

fn main() -> eframe::Result {
    // Used by the installer: add Glance to (or take it off) Windows' photo and video apps, then quit.
    match std::env::args().nth(1).as_deref() {
        Some("--register") => return defaults::register().map_err(|e| eframe::Error::AppCreation(e.into())),
        Some("--unregister") => {
            defaults::unregister();
            return Ok(());
        }
        _ => {}
    }
    let arg = std::env::args_os().nth(1).and_then(|p| std::path::absolute(p).ok());
    // A viewer is already open: hand it the path (or just bring it forward) and quit.
    if instance::hand_off(arg.as_deref()) {
        return Ok(());
    }
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("bundled icon is a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([480.0, 320.0])
            .with_icon(icon)
            .with_decorations(false) // Glance draws its own title bar (see window_buttons)
            .with_transparent(true), // lets the glass look show the Windows blur
        renderer: eframe::Renderer::Glow, // wgpu can't draw see-through windows on Windows
        ..Default::default()
    };
    // Settings, ratings and the thumbnail cache used to live under the old name; carry them over once.
    if let (Some(old), Some(new)) = (eframe::storage_dir("Photo Viewer"), eframe::storage_dir(APP)) {
        if old.exists() && !new.join("app.ron").exists() {
            let _ = std::fs::remove_dir(&new); // only succeeds on an empty leftover folder
            let _ = new.parent().map(std::fs::create_dir_all);
            let _ = std::fs::rename(&old, &new);
        }
    }
    eframe::run_native(
        APP,
        options,
        Box::new(move |cc| {
            let mut v = Viewer::new(cc);
            if let Some(p) = arg {
                v.open(&cc.egui_ctx, &p);
            }
            Ok(Box::new(v))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_maps_back_to_texture() {
        let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
        // No transform: identity.
        assert!(close(display_to_texture(0.25, 0.75, 0, false, false), (0.25, 0.75)));
        // Rotated 90° clockwise, the texture's top-left corner shows at the top-right.
        assert!(close(display_to_texture(1.0, 0.0, 1, false, false), (0.0, 0.0)));
        // A full turn is the identity again.
        assert!(close(display_to_texture(0.3, 0.6, 4, false, false), (0.3, 0.6)));
        // Mirrored: the left edge shows the texture's right edge.
        assert!(close(display_to_texture(0.0, 0.5, 0, true, false), (1.0, 0.5)));
        assert!(close(display_to_texture(0.5, 0.0, 0, false, true), (0.5, 1.0)));
    }

    #[test]
    fn look_alikes_group_together() {
        let g = group_alike(&[0b1111, 0xFFFF_0000, 0b1101, 0xFFFF_0001, 0x0F0F_0F0F_0F0F_0F0F]);
        assert_eq!(g, [Some(0), Some(1), Some(0), Some(1), None]);
    }

    #[test]
    fn live_photos_pair_by_name() {
        let e = |p: &str| Entry { path: PathBuf::from(p), modified: SystemTime::UNIX_EPOCH, size: 0 };
        let pairs = live_pairs(&[e("a/IMG_1.HEIC"), e("a/IMG_1.MOV"), e("a/IMG_2.jpg"), e("b/IMG_2.mov")]);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[&PathBuf::from("a/IMG_1.HEIC")], PathBuf::from("a/IMG_1.MOV"));
    }

    #[test]
    fn tiny_copy_averages_cells() {
        // Left half black, right half white: two cells, one of each.
        let px: Vec<Color32> = (0..40 * 20).map(|i| if i % 40 < 20 { Color32::BLACK } else { Color32::WHITE }).collect();
        let small = tiny_copy(&egui::ColorImage::new([40, 20], px), 2);
        assert_eq!(small.size, [2, 1]);
        assert_eq!(small.pixels, [Color32::BLACK, Color32::WHITE]);
    }

    #[test]
    fn layout_swaps_sides_when_rotated() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 1000.0));
        let (_, shown, fit) = layout(rect, vec2(2000.0, 1000.0), 1, 1.0, Vec2::ZERO);
        assert_eq!(fit, 0.5);
        assert_eq!(shown.size(), vec2(500.0, 1000.0));
    }
}
