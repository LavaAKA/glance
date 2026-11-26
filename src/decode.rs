//! Turning files into pixels: stills, animated GIFs, SVG, and (on Windows) anything a system
//! codec handles (HEIC, AVIF, camera RAW). Also EXIF info, histograms and the thumbnail cache.
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use image::{AnimationDecoder, DynamicImage, RgbaImage};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

// shortcut: fixed limit instead of asking the GPU; 8192 works on every desktop GPU of the last decade.
/// Largest texture side we upload; bigger photos are downscaled for display only.
pub const MAX_SIDE: u32 = 8192;
/// Thumbnails fit in this square: the largest grid tile (320) stays sharp on high-DPI screens.
pub const THUMB: u32 = 512;

pub type Hist = [[u32; 256]; 3];

/// Decoded pixels before they go to the GPU.
pub struct Raw {
    pub frames: Vec<(RgbaImage, Duration)>,
    pub orig: [u32; 2], // size after EXIF rotation, before any downscale
    pub format: String,
    pub exif: Vec<(&'static str, String)>,
}

/// Reads any supported image. `max_side` downscales; `all_frames` keeps every GIF frame.
pub fn read(path: &Path, max_side: Option<u32>, all_frames: bool) -> Result<Raw, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_uppercase();
    let (exif, orientation) = read_exif(&bytes);
    let (mut frames, format) = if ext == "SVG" {
        (vec![(render_svg(&bytes)?, Duration::ZERO)], ext)
    } else {
        let guessed = image::guess_format(&bytes).ok();
        match guessed {
            Some(image::ImageFormat::Gif) if all_frames => (gif_frames(&bytes)?, "GIF".into()),
            _ => match image::load_from_memory(&bytes) {
                Ok(img) => {
                    let name = guessed.map(|f| format!("{f:?}").to_uppercase()).unwrap_or(ext);
                    (vec![(img.into_rgba8(), Duration::ZERO)], name)
                }
                // Not something the image crate reads: let Windows try (HEIC, AVIF, RAW...).
                // For camera RAW the embedded JPEG preview is far faster, and plenty for viewing.
                Err(e) => {
                    let preview = (max_side.is_some() && RAW_EXTS.contains(&ext.as_str())).then(|| wic_preview(path)).and_then(Result::ok);
                    let img = match preview {
                        Some(p) => p,
                        None => wic(path).map_err(|w| format!("{e} / {w}"))?,
                    };
                    (vec![(img, Duration::ZERO)], ext)
                }
            },
        }
    };
    if let Some(o) = orientation {
        for (f, _) in &mut frames {
            let mut img = DynamicImage::ImageRgba8(std::mem::take(f));
            img.apply_orientation(o);
            *f = img.into_rgba8();
        }
    }
    let orig = [frames[0].0.width(), frames[0].0.height()];
    if let Some(max) = max_side {
        for (f, _) in &mut frames {
            if f.width() > max || f.height() > max {
                let img = DynamicImage::ImageRgba8(std::mem::take(f));
                *f = img.resize(max, max, image::imageops::FilterType::Triangle).into_rgba8();
            }
        }
    }
    Ok(Raw { frames, orig, format, exif })
}

fn read_exif(bytes: &[u8]) -> (Vec<(&'static str, String)>, Option<image::metadata::Orientation>) {
    let Ok(ex) = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)) else {
        return (vec![], None);
    };
    use exif::Tag;
    let fields = [
        ("Camera", Tag::Model),
        ("Make", Tag::Make),
        ("Lens", Tag::LensModel),
        ("Taken", Tag::DateTimeOriginal),
        ("Exposure", Tag::ExposureTime),
        ("Aperture", Tag::FNumber),
        ("ISO", Tag::PhotographicSensitivity),
        ("Focal length", Tag::FocalLength),
    ];
    let mut info: Vec<(&'static str, String)> = fields
        .iter()
        .filter_map(|&(name, tag)| {
            let f = ex.get_field(tag, exif::In::PRIMARY)?;
            Some((name, f.display_value().with_unit(&ex).to_string().trim_matches('"').to_string()))
        })
        .collect();
    if let Some((lat, lon)) = gps(&ex) {
        info.push(("Location", format!("{lat:.5}, {lon:.5}")));
    }
    let orientation = ex
        .get_field(Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .and_then(|v| image::metadata::Orientation::from_exif(v as u8));
    (info, orientation)
}

/// Camera RAW extensions (uppercase), which have an embedded JPEG preview.
const RAW_EXTS: &[&str] = &["DNG", "CR2", "CR3", "NEF", "ARW", "ORF", "RW2", "RAF"];

/// Latitude and longitude in degrees from the GPS EXIF tags.
fn gps(ex: &exif::Exif) -> Option<(f64, f64)> {
    use exif::{Tag, Value};
    let deg = |tag, reference| -> Option<f64> {
        let Value::Rational(r) = &ex.get_field(tag, exif::In::PRIMARY)?.value else { return None };
        let v = r.first()?.to_f64() + r.get(1).map_or(0.0, |m| m.to_f64() / 60.0) + r.get(2).map_or(0.0, |s| s.to_f64() / 3600.0);
        let neg = ex.get_field(reference, exif::In::PRIMARY).is_some_and(|f| matches!(f.display_value().to_string().as_str(), "S" | "W"));
        Some(if neg { -v } else { v })
    };
    let (lat, lon) = (deg(Tag::GPSLatitude, Tag::GPSLatitudeRef)?, deg(Tag::GPSLongitude, Tag::GPSLongitudeRef)?);
    (lat != 0.0 || lon != 0.0).then_some((lat, lon))
}

/// What search, the date headings, duplicates and the map need to know about a file.
#[derive(Clone, Default)]
pub struct Meta {
    pub taken: Option<String>, // "2024-07-14 18:03"
    pub camera: Option<String>,
    pub gps: Option<(f64, f64)>,
    pub hash: Option<u64>, // looks-alike fingerprint (dHash) of the thumbnail
}

/// EXIF date, camera and location without decoding the picture.
pub fn read_meta(path: &Path) -> Meta {
    let Ok(file) = std::fs::File::open(path) else { return Meta::default() };
    let Ok(ex) = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)) else {
        return Meta::default();
    };
    let text = |tag| {
        let f = ex.get_field(tag, exif::In::PRIMARY)?;
        Some(f.display_value().to_string().trim_matches('"').trim().to_string()).filter(|s| !s.is_empty())
    };
    let taken = text(exif::Tag::DateTimeOriginal).map(|d| d.chars().take(16).collect());
    let camera = match (text(exif::Tag::Make), text(exif::Tag::Model)) {
        (Some(make), Some(model)) if !model.to_lowercase().starts_with(&make.to_lowercase()) => Some(format!("{make} {model}")),
        (_, model) => model,
    };
    Meta { taken, camera, gps: gps(&ex), hash: None }
}

/// 64-bit difference hash: photos that look alike (resized, recompressed) differ in only a few bits.
pub fn dhash(img: &RgbaImage) -> u64 {
    let small = image::imageops::resize(&DynamicImage::ImageRgba8(img.clone()).to_luma8(), 9, 8, image::imageops::FilterType::Triangle);
    let mut h = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            h = (h << 1) | (small.get_pixel(x, y)[0] < small.get_pixel(x + 1, y)[0]) as u64;
        }
    }
    h
}

/// The full-size preview a RAW file carries (or its thumbnail), via Windows.
#[cfg(windows)]
fn wic_preview(path: &Path) -> Result<RgbaImage, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::*;
    use windows::core::PCWSTR;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let err = |e: windows::core::Error| e.message().to_string();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(err)?;
        let decoder = factory
            .CreateDecoderFromFilename(PCWSTR(wide.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .map_err(err)?;
        let source: IWICBitmapSource = match decoder.GetPreview() {
            Ok(p) => p,
            Err(_) => decoder.GetFrame(0).map_err(err)?.GetThumbnail().map_err(err)?,
        };
        let conv = factory.CreateFormatConverter().map_err(err)?;
        conv.Initialize(&source, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .map_err(err)?;
        let (mut w, mut h) = (0, 0);
        conv.GetSize(&mut w, &mut h).map_err(err)?;
        let mut buf = vec![0u8; w as usize * h as usize * 4];
        conv.CopyPixels(std::ptr::null(), w * 4, &mut buf).map_err(err)?;
        RgbaImage::from_raw(w, h, buf).ok_or_else(|| "bad preview buffer".into())
    }
}

#[cfg(not(windows))]
fn wic_preview(_: &Path) -> Result<RgbaImage, String> {
    Err("unsupported".into())
}

fn gif_frames(bytes: &[u8]) -> Result<Vec<(RgbaImage, Duration)>, String> {
    let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut out = vec![];
    let mut total = 0usize;
    for frame in decoder.into_frames() {
        let frame = frame.map_err(|e| e.to_string())?;
        let (n, d) = frame.delay().numer_denom_ms();
        let ms = if d == 0 { 0 } else { n / d };
        // Browsers play 0-10 ms delays at 100 ms; match them so those GIFs don't race.
        let ms = if ms <= 10 { 100 } else { ms };
        let buf = frame.into_buffer();
        total += buf.len();
        out.push((buf, Duration::from_millis(ms as u64)));
        // shortcut: cap at ~512 MB of frames; very long GIFs only play their first part.
        if total > 512 << 20 {
            break;
        }
    }
    if out.is_empty() {
        return Err("GIF has no frames".into());
    }
    Ok(out)
}

fn render_svg(bytes: &[u8]) -> Result<RgbaImage, String> {
    use resvg::{tiny_skia, usvg};
    // shortcut: no system fonts loaded, so <text> in SVGs isn't drawn; load fontdb if that matters.
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = 2048.0 / size.width().max(size.height()); // vector: render sharp at a useful size
    let (w, h) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or("SVG is too large")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut img = RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixmap.take()).ok_or("bad SVG buffer")?;
    // tiny-skia hands out premultiplied alpha; everything else here expects straight alpha.
    for p in img.pixels_mut() {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            for c in 0..3 {
                p[c] = (p[c] as u32 * 255 / a).min(255) as u8;
            }
        }
    }
    Ok(img)
}

/// Decodes with Windows Imaging Component, which uses whatever codecs are installed
/// (HEIF/AVIF/Raw Image Extensions from the Store).
#[cfg(windows)]
fn wic(path: &Path) -> Result<RgbaImage, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::*;
    use windows::core::PCWSTR;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let err = |e: windows::core::Error| format!("no Windows codec for this file ({})", e.message());
    unsafe {
        // Decode threads need COM; "already initialized" is fine to ignore.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(err)?;
        let decoder = factory
            .CreateDecoderFromFilename(PCWSTR(wide.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .map_err(err)?;
        let frame = decoder.GetFrame(0).map_err(err)?;
        let conv = factory.CreateFormatConverter().map_err(err)?;
        conv.Initialize(&frame, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .map_err(err)?;
        let (mut w, mut h) = (0, 0);
        conv.GetSize(&mut w, &mut h).map_err(err)?;
        let mut buf = vec![0u8; w as usize * h as usize * 4];
        conv.CopyPixels(std::ptr::null(), w * 4, &mut buf).map_err(err)?;
        RgbaImage::from_raw(w, h, buf).ok_or_else(|| "bad WIC buffer".into())
    }
}

#[cfg(not(windows))]
fn wic(_: &Path) -> Result<RgbaImage, String> {
    Err("unsupported format".into())
}

/// The thumbnail Explorer would show (used for videos). None if Windows has no preview for the file.
#[cfg(windows)]
fn shell_thumbnail(path: &Path) -> Option<RgbaImage> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Shell::*;
    use windows::core::PCWSTR;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // "already initialized" is fine
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
        // THUMBNAILONLY: a real frame, never the generic file-type icon.
        let side = THUMB as i32;
        let hbmp = factory.GetImage(SIZE { cx: side, cy: side }, SIIGBF_BIGGERSIZEOK | SIIGBF_THUMBNAILONLY).ok()?;
        let mut bm = BITMAP::default();
        GetObjectW(hbmp.into(), size_of::<BITMAP>() as i32, Some((&mut bm as *mut BITMAP).cast()));
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // negative = rows top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w.max(0) * h.max(0) * 4) as usize];
        let dc = GetDC(None);
        let rows = GetDIBits(dc, hbmp, 0, h as u32, Some(buf.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        let _ = DeleteObject(hbmp.into());
        if rows <= 0 {
            return None;
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2); // BGRA -> RGBA
            px[3] = 255; // video frames are opaque; the shell often leaves alpha at 0
        }
        RgbaImage::from_raw(w as u32, h as u32, buf)
    }
}

#[cfg(not(windows))]
fn shell_thumbnail(_: &Path) -> Option<RgbaImage> {
    None
}

fn to_color_image(img: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

fn histogram(img: &RgbaImage) -> Box<Hist> {
    let mut h = Box::new([[0u32; 256]; 3]);
    // ~1M samples is plenty for the shape of a histogram.
    let step = (img.pixels().len() / 1_000_000).max(1);
    for p in img.pixels().step_by(step) {
        for c in 0..3 {
            h[c][p[c] as usize] += 1;
        }
    }
    h
}

/// A photo ready to draw.
pub struct Decoded {
    pub frames: Vec<(TextureHandle, Duration)>,
    pub pixels: Arc<ColorImage>, // first frame on the CPU, for the color readout and clipboard
    pub orig: [u32; 2],
    pub format: String,
    pub exif: Vec<(&'static str, String)>,
    pub hist: Box<Hist>,
}

impl Decoded {
    pub fn size(&self) -> egui::Vec2 {
        self.frames[0].0.size_vec2()
    }

    /// The frame to show `t` after the animation started, and how long until the next one.
    pub fn frame_at(&self, t: Duration) -> (&TextureHandle, Option<Duration>) {
        let total: Duration = self.frames.iter().map(|f| f.1).sum();
        if self.frames.len() < 2 || total.is_zero() {
            return (&self.frames[0].0, None);
        }
        let mut t = Duration::from_nanos((t.as_nanos() % total.as_nanos()) as u64);
        for (tex, d) in &self.frames {
            if t < *d {
                return (tex, Some(*d - t));
            }
            t -= *d;
        }
        (&self.frames[0].0, Some(self.frames[0].1))
    }
}

/// Decodes `path` and uploads it to the GPU (egui allows that from any thread).
pub fn load(ctx: &egui::Context, path: &Path) -> Result<Decoded, String> {
    let raw = read(path, Some(MAX_SIDE), true)?;
    let hist = histogram(&raw.frames[0].0);
    let name = path.to_string_lossy();
    let mut pixels = None;
    let mut frames = vec![];
    for (i, (img, delay)) in raw.frames.iter().enumerate() {
        let ci = Arc::new(to_color_image(img));
        frames.push((ctx.load_texture(format!("{name}#{i}"), ci.clone(), TextureOptions::LINEAR), *delay));
        pixels.get_or_insert(ci);
    }
    Ok(Decoded { frames, pixels: pixels.unwrap(), orig: raw.orig, format: raw.format, exif: raw.exif, hist })
}

/// A small copy of `path`, read from / written to the disk cache in `cache` when given.
pub fn thumbnail(path: &Path, cache: Option<&Path>) -> Option<RgbaImage> {
    let key = cache.and_then(|dir| {
        let meta = std::fs::metadata(path).ok()?;
        let mut h = DefaultHasher::new();
        (path, meta.len(), meta.modified().ok(), THUMB).hash(&mut h);
        Some(dir.join(format!("{:016x}.png", h.finish())))
    });
    if let Some(Ok(img)) = key.as_ref().map(image::open) {
        return Some(img.into_rgba8());
    }
    let first = if crate::video::is_video(path) {
        shell_thumbnail(path)?
    } else {
        read(path, Some(MAX_SIDE), false).ok()?.frames.into_iter().next()?.0
    };
    let thumb = DynamicImage::ImageRgba8(first).thumbnail(THUMB, THUMB).into_rgba8();
    if let Some(k) = &key {
        let _ = std::fs::create_dir_all(k.parent()?);
        let _ = thumb.save(k);
    }
    Some(thumb)
}

/// Length of a video in seconds, from the same metadata Explorer shows.
#[cfg(windows)]
pub fn video_duration(path: &Path) -> Option<f64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::EnhancedStorage::PKEY_Media_Duration;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Shell::{IShellItem2, SHCreateItemFromParsingName};
    use windows::core::PCWSTR;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let item: IShellItem2 = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
        let ticks = item.GetUInt64(&PKEY_Media_Duration).ok()?; // 100 ns units
        Some(ticks as f64 / 1e7)
    }
}

#[cfg(not(windows))]
pub fn video_duration(_: &Path) -> Option<f64> {
    None
}

pub struct Thumb {
    pub path: PathBuf,
    pub tex: Option<TextureHandle>,
    pub duration: Option<f64>,
    pub meta: Meta,
}

struct Queue {
    members: std::collections::HashSet<PathBuf>,
    rest: std::collections::VecDeque<PathBuf>,
    urgent: Vec<PathBuf>,
    taken: std::collections::HashSet<PathBuf>,
}

impl Queue {
    fn next(&mut self) -> Option<PathBuf> {
        let mut pick = |p: PathBuf| self.taken.insert(p.clone()).then_some(p);
        while let Some(p) = self.urgent.pop() {
            if let Some(p) = pick(p) {
                return Some(p);
            }
        }
        while let Some(p) = self.rest.pop_front() {
            if let Some(p) = pick(p) {
                return Some(p);
            }
        }
        None
    }
}

/// Thumbnails being made in the background. Workers stop when this is dropped.
pub struct ThumbJob {
    pub rx: mpsc::Receiver<Thumb>,
    queue: Arc<Mutex<Queue>>,
}

impl ThumbJob {
    /// Do these next (the tiles on screen right now), so big folders fill in where you're looking.
    pub fn prioritize(&self, mut paths: Vec<PathBuf>) {
        let mut q = self.queue.lock().unwrap();
        paths.retain(|p| q.members.contains(p) && !q.taken.contains(p));
        paths.reverse(); // urgent is popped from the end: keep top-of-screen first
        q.urgent = paths;
    }
}

/// Makes thumbnails for `files` on a few worker threads.
pub fn spawn_thumbs(ctx: &egui::Context, files: Vec<PathBuf>, cache: Option<PathBuf>) -> ThumbJob {
    let (tx, rx) = mpsc::channel();
    let queue = Arc::new(Mutex::new(Queue {
        members: files.iter().cloned().collect(),
        rest: files.into(),
        urgent: vec![],
        taken: Default::default(),
    }));
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(1, 6));
    for _ in 0..workers {
        let (tx, queue, ctx, cache) = (tx.clone(), queue.clone(), ctx.clone(), cache.clone());
        std::thread::spawn(move || {
            loop {
                let Some(path) = queue.lock().unwrap().next() else { return };
                let duration = crate::video::is_video(&path).then(|| video_duration(&path)).flatten();
                let img = thumbnail(&path, cache.as_deref());
                let mut meta = if duration.is_some() { Meta::default() } else { read_meta(&path) };
                meta.hash = img.as_ref().map(dhash);
                let tex = img.map(|img| {
                    ctx.load_texture(format!("thumb:{}", path.display()), to_color_image(&img), TextureOptions::LINEAR)
                });
                if tx.send(Thumb { path, tex, duration, meta }).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
    }
    ThumbJob { rx, queue }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_sniff_format_and_use_cache() {
        let dir = std::env::temp_dir().join("photo-viewer-thumb-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // PNG bytes behind a .jpg name must still load.
        let p = dir.join("misnamed.jpg");
        image::RgbImage::new(800, 400).save_with_format(&p, image::ImageFormat::Png).unwrap();
        let cache = dir.join("cache");
        let t = thumbnail(&p, Some(&cache)).unwrap();
        assert_eq!(t.dimensions(), (512, 256));
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1, "thumbnail written to cache");
        assert_eq!(thumbnail(&p, Some(&cache)).unwrap().dimensions(), (512, 256));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dhash_matches_look_alikes() {
        let img = RgbaImage::from_fn(200, 100, |x, y| image::Rgba([(x + y) as u8, x as u8, 255 - y as u8, 255]));
        let small = DynamicImage::ImageRgba8(img.clone()).thumbnail(60, 30).into_rgba8();
        let other = RgbaImage::from_fn(200, 100, |x, _| image::Rgba([255 - x as u8, 0, 0, 255]));
        assert!((dhash(&img) ^ dhash(&small)).count_ones() <= 4, "resized copy looks alike");
        assert!((dhash(&img) ^ dhash(&other)).count_ones() > 10, "different picture doesn't");
    }

    #[test]
    fn animation_timing_wraps() {
        // frame_at only needs durations; check the arithmetic on a fake 3-frame loop.
        let ctx = egui::Context::default();
        let tex = |n| ctx.load_texture(n, ColorImage::filled([1, 1], egui::Color32::RED), TextureOptions::LINEAR);
        let ms = Duration::from_millis;
        let d = Decoded {
            frames: vec![(tex("a"), ms(100)), (tex("b"), ms(200)), (tex("c"), ms(100))],
            pixels: Arc::new(ColorImage::filled([1, 1], egui::Color32::RED)),
            orig: [1, 1],
            format: String::new(),
            exif: vec![],
            hist: Box::new([[0; 256]; 3]),
        };
        assert_eq!(d.frame_at(ms(150)).0.name(), "b");
        assert_eq!(d.frame_at(ms(450)).0.name(), "a"); // 450 % 400 = 50
        assert_eq!(d.frame_at(ms(150)).1, Some(ms(150)));
    }
}
