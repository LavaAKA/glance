//! Video playback through Windows Media Foundation's Media Engine (what the built-in Windows
//! apps use): decoding, sound, seeking and volume come from Windows, and each new frame is
//! copied into an egui texture.
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::path::Path;
use std::sync::Arc;

pub const EXTS: &[&str] = &["mp4", "m4v", "mov", "wmv", "avi", "mkv", "webm", "3gp", "mts", "m2ts"];

#[cfg(windows)]
pub use imp::Player;

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::*;
    use windows::core::{BSTR, implement};

    /// The engine requires a callback; we only use it to wake the UI (loaded, ended, error...).
    #[implement(IMFMediaEngineNotify)]
    struct Notify(egui::Context);

    impl IMFMediaEngineNotify_Impl for Notify_Impl {
        fn EventNotify(&self, _event: u32, _param1: usize, _param2: u32) -> windows::core::Result<()> {
            self.0.request_repaint();
            Ok(())
        }
    }

    pub struct Player {
        engine: IMFMediaEngine,
        wic: IWICImagingFactory,
        target: Option<(IWICBitmap, u32, u32)>,
        texture: Option<TextureHandle>,
        last: Option<Arc<ColorImage>>, // the newest frame on the CPU, for saving it as a picture
        last_pts: i64,
    }

    fn err(e: windows::core::Error) -> String {
        e.message().to_string()
    }

    impl Player {
        /// Opens `path` and starts playing at `volume` (0..1).
        pub fn open(ctx: &egui::Context, path: &Path, volume: f64) -> Result<Self, String> {
            unsafe {
                // The UI thread already has COM (winit sets it up for drag and drop); ignore "already initialized".
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(err)?;
                let factory: IMFMediaEngineClassFactory =
                    CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_ALL).map_err(err)?;
                let mut attrs = None;
                MFCreateAttributes(&mut attrs, 2).map_err(err)?;
                let attrs = attrs.ok_or("no attributes")?;
                let notify: IMFMediaEngineNotify = Notify(ctx.clone()).into();
                attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify).map_err(err)?;
                // 87 = DXGI_FORMAT_B8G8R8A8_UNORM: frames come out as BGRA.
                attrs.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, 87).map_err(err)?;
                let engine = factory.CreateInstance(0, &attrs).map_err(err)?;
                engine.SetSource(&BSTR::from(path.to_string_lossy().as_ref())).map_err(err)?;
                let _ = engine.SetVolume(volume);
                engine.Play().map_err(err)?;
                let wic = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(err)?;
                Ok(Self { engine, wic, target: None, texture: None, last: None, last_pts: -1 })
            }
        }

        /// Copies the newest frame into the texture (if there's a new one) and returns it.
        pub fn frame(&mut self, ctx: &egui::Context) -> Option<&TextureHandle> {
            unsafe {
                if let Ok(pts) = self.engine.OnVideoStreamTick() {
                    if pts != self.last_pts && self.copy_frame(ctx).is_some() {
                        self.last_pts = pts;
                    }
                }
            }
            self.texture.as_ref()
        }

        unsafe fn copy_frame(&mut self, ctx: &egui::Context) -> Option<()> {
            unsafe {
                let (mut w, mut h) = (0u32, 0u32);
                self.engine.GetNativeVideoSize(Some(&mut w), Some(&mut h)).ok()?;
                if w == 0 || h == 0 {
                    return None;
                }
                if self.target.as_ref().map(|t| (t.1, t.2)) != Some((w, h)) {
                    let bmp = self.wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppBGRA, WICBitmapCacheOnLoad).ok()?;
                    self.target = Some((bmp, w, h));
                }
                let bmp = &self.target.as_ref()?.0;
                let dst = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
                self.engine.TransferVideoFrame(bmp, None, &dst, None).ok()?;
                let lock = bmp.Lock(&WICRect { X: 0, Y: 0, Width: w as i32, Height: h as i32 }, WICBitmapLockRead.0 as u32).ok()?;
                let (mut size, mut ptr) = (0u32, std::ptr::null_mut());
                lock.GetDataPointer(&mut size, &mut ptr).ok()?;
                let stride = lock.GetStride().ok()? as usize;
                let data = std::slice::from_raw_parts(ptr, size as usize);
                let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
                for row in data.chunks(stride).take(h as usize) {
                    for px in row[..w as usize * 4].chunks_exact(4) {
                        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                    }
                }
                let img = Arc::new(ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba));
                match &mut self.texture {
                    Some(t) => t.set(img.clone(), TextureOptions::LINEAR),
                    None => self.texture = Some(ctx.load_texture("video", img.clone(), TextureOptions::LINEAR)),
                }
                self.last = Some(img);
                Some(())
            }
        }

        pub fn playing(&self) -> bool {
            unsafe { !self.engine.IsPaused().as_bool() && !self.engine.IsEnded().as_bool() }
        }

        pub fn toggle(&self) {
            unsafe {
                if self.playing() {
                    let _ = self.engine.Pause();
                } else {
                    if self.engine.IsEnded().as_bool() {
                        let _ = self.engine.SetCurrentTime(0.0);
                    }
                    let _ = self.engine.Play();
                }
            }
        }

        /// Seconds into the video.
        pub fn time(&self) -> f64 {
            unsafe { self.engine.GetCurrentTime() }
        }

        /// Length in seconds (0 until the file has loaded).
        pub fn duration(&self) -> f64 {
            let d = unsafe { self.engine.GetDuration() };
            if d.is_finite() { d } else { 0.0 }
        }

        pub fn seek(&self, secs: f64) {
            unsafe {
                let _ = self.engine.SetCurrentTime(secs);
            }
        }

        pub fn set_volume(&self, v: f64) {
            unsafe {
                let _ = self.engine.SetVolume(v);
            }
        }

        /// The frame on screen, as pixels.
        pub fn snapshot(&self) -> Option<Arc<ColorImage>> {
            self.last.clone()
        }

        pub fn pause(&self) {
            unsafe {
                let _ = self.engine.Pause();
            }
        }

        pub fn set_muted(&self, m: bool) {
            unsafe {
                let _ = self.engine.SetMuted(m);
            }
        }

        pub fn set_rate(&self, r: f64) {
            unsafe {
                let _ = self.engine.SetPlaybackRate(r);
            }
        }

        pub fn set_loop(&self, l: bool) {
            unsafe {
                let _ = self.engine.SetLoop(l);
            }
        }

        /// A readable message if Windows couldn't play the file.
        pub fn error(&self) -> Option<String> {
            let code = unsafe { self.engine.GetError().ok()?.GetErrorCode() };
            Some(match code {
                0 => return None,
                4 => "This video format isn't supported (Windows may need a codec extension from the Microsoft Store)".into(),
                3 => "The video couldn't be decoded".into(),
                c => format!("Couldn't play this video (error {c})"),
            })
        }
    }

    impl Drop for Player {
        fn drop(&mut self) {
            unsafe {
                let _ = self.engine.Shutdown();
            }
        }
    }
}

// shortcut: video playback is Windows-only; other platforms need an ffmpeg/gstreamer backend.
#[cfg(not(windows))]
pub struct Player;

#[cfg(not(windows))]
impl Player {
    pub fn open(_: &egui::Context, _: &Path, _: f64) -> Result<Self, String> {
        Err("Video playback is only supported on Windows".into())
    }
    pub fn frame(&mut self, _: &egui::Context) -> Option<&TextureHandle> {
        None
    }
    pub fn playing(&self) -> bool {
        false
    }
    pub fn toggle(&self) {}
    pub fn time(&self) -> f64 {
        0.0
    }
    pub fn duration(&self) -> f64 {
        0.0
    }
    pub fn seek(&self, _: f64) {}
    pub fn set_volume(&self, _: f64) {}
    pub fn snapshot(&self) -> Option<Arc<ColorImage>> {
        None
    }
    pub fn pause(&self) {}
    pub fn set_muted(&self, _: bool) {}
    pub fn set_rate(&self, _: f64) {}
    pub fn set_loop(&self, _: bool) {}
    pub fn error(&self) -> Option<String> {
        None
    }
}

pub fn is_video(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// 75 -> "1:15", 3725 -> "1:02:05".
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// One subtitle: shown from `start` to `end` seconds.
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Parses SubRip (.srt) text. Malformed blocks are skipped.
pub fn parse_srt(text: &str) -> Vec<Cue> {
    let secs = |t: &str| -> Option<f64> {
        let (hms, ms) = t.trim().split_once([',', '.'])?;
        let mut parts = hms.split(':').map(|n| n.trim().parse::<f64>().ok());
        let (h, m, s) = (parts.next()??, parts.next()??, parts.next()??);
        Some(h * 3600.0 + m * 60.0 + s + ms.trim().parse::<f64>().ok()? / 1000.0)
    };
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    text.split("\n\n")
        .filter_map(|block| {
            let mut lines = block.trim().lines().skip_while(|l| !l.contains("-->"));
            let (a, b) = lines.next()?.split_once("-->")?;
            // Strip simple tags like <i> and {\an8}.
            let body: String = lines.collect::<Vec<_>>().join("\n");
            let mut clean = String::new();
            let mut skip = None;
            for c in body.chars() {
                match (skip, c) {
                    (None, '<') => skip = Some('>'),
                    (None, '{') => skip = Some('}'),
                    (Some(end), c) if c == end => skip = None,
                    (None, c) => clean.push(c),
                    _ => {}
                }
            }
            Some(Cue { start: secs(a)?, end: secs(b.split_whitespace().next()?)?, text: clean })
        })
        .collect()
}

/// Subtitles next to a video: clip.srt, or clip.en.srt and the like.
pub fn find_subtitles(video: &Path) -> Vec<Cue> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().map(|s| s.to_string_lossy().to_lowercase())) else {
        return vec![];
    };
    let srt = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).find(|p| {
        let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        name.ends_with(".srt") && (name == format!("{stem}.srt") || name.starts_with(&format!("{stem}.")))
    });
    srt.and_then(|p| std::fs::read(p).ok()).map(|b| parse_srt(&String::from_utf8_lossy(&b))).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn srt_parses() {
        let cues = super::parse_srt("1\r\n00:00:01,500 --> 00:00:03,000\r\n<i>Hello</i>\r\nthere\r\n\r\n2\r\n00:01:00,000 --> 00:01:02,250\r\nBye\r\n");
        assert_eq!(cues.len(), 2);
        assert_eq!((cues[0].start, cues[0].end), (1.5, 3.0));
        assert_eq!(cues[0].text, "Hello\nthere");
        assert_eq!((cues[1].start, cues[1].end), (60.0, 62.25));
    }

    #[test]
    fn clock_formats() {
        assert_eq!(super::clock(75.4), "1:15");
        assert_eq!(super::clock(3725.0), "1:02:05");
        assert_eq!(super::clock(-1.0), "0:00");
    }
}
