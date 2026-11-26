//! Folder listing, sorting and file operations (rename, copy/move, shell helpers).
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

pub const EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico", "svg", // built in
    "heic", "heif", "avif", "jxr", "dng", "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf", // via Windows codecs
];

pub fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

pub fn file_name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub size: u64,
}

/// Images and subfolders directly inside `dir`, skipping hidden (dot) entries.
pub fn list_dir(dir: &Path) -> (Vec<Entry>, Vec<PathBuf>) {
    let (mut images, mut dirs) = (vec![], vec![]);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let (path, Ok(meta)) = (e.path(), e.metadata()) else { continue };
        if meta.is_dir() {
            dirs.push(path);
        } else if is_image(&path) || crate::video::is_video(&path) {
            images.push(Entry { path, modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), size: meta.len() });
        }
    }
    dirs.sort_by_cached_key(|d| file_name(d).to_lowercase());
    (images, dirs)
}

fn hidden_name(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(['.', '$']))
}

/// Every photo and video under `dir`, subfolders included.
// shortcut: stops at 50,000 files so a whole drive can't freeze the app; add paging if people need more.
pub fn list_deep(dir: &Path) -> Vec<Entry> {
    let (mut out, mut todo) = (vec![], vec![dir.to_path_buf()]);
    while let Some(d) = todo.pop() {
        let (files, dirs) = list_dir(&d);
        out.extend(files);
        todo.extend(dirs.into_iter().filter(|d| !hidden_name(d)));
        if out.len() > 50_000 {
            break;
        }
    }
    out
}

/// Up to 4 photos or videos inside `dir`, for the folder tile's preview. Reads names only (fast).
pub fn preview_files(dir: &Path) -> Vec<PathBuf> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| !hidden_name(p) && (is_image(p) || crate::video::is_video(p)))
        .collect();
    names.sort_by_cached_key(|p| natural_key(&file_name(p)));
    names.truncate(4);
    names
}

/// The name for file number `n` from a rename pattern: `#` runs become the zero-padded number,
/// `*` the old name. "Trip_###" -> "Trip_007.jpg".
pub fn pattern_name(pattern: &str, old_stem: &str, n: usize, ext: &str) -> String {
    let mut out = String::new();
    let mut hashes = 0;
    let flush = |out: &mut String, hashes: &mut usize| {
        if *hashes > 0 {
            out.push_str(&format!("{n:0width$}", width = *hashes));
            *hashes = 0;
        }
    };
    for c in pattern.chars() {
        if c == '#' {
            hashes += 1;
            continue;
        }
        flush(&mut out, &mut hashes);
        if c == '*' {
            out.push_str(old_stem);
        } else {
            out.push(c);
        }
    }
    flush(&mut out, &mut hashes);
    if !pattern.contains('#') && !pattern.contains('*') {
        out.push_str(&format!(" ({n})")); // keep names unique without a counter in the pattern
    }
    if ext.is_empty() { out } else { format!("{out}.{ext}") }
}

/// Renames `paths` (in order, numbered from 1) by `pattern`. All or nothing: checks every
/// new name first. Returns (old, new) pairs.
pub fn batch_rename(paths: &[PathBuf], pattern: &str) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let bad = ['/', '\\', ':', '?', '"', '<', '>', '|'];
    if pattern.trim().is_empty() || pattern.contains(bad) {
        return Err("That pattern isn't allowed. Avoid / \\ : ? \" < > |".into());
    }
    let pairs: Vec<(PathBuf, PathBuf)> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
            (p.clone(), p.with_file_name(pattern_name(pattern.trim(), &stem, i + 1, &ext)))
        })
        .collect();
    let new: std::collections::HashSet<&PathBuf> = pairs.iter().map(|(_, n)| n).collect();
    if new.len() != pairs.len() {
        return Err("The pattern gives two files the same name.".into());
    }
    let olds: std::collections::HashSet<&PathBuf> = paths.iter().collect();
    if let Some((_, n)) = pairs.iter().find(|(_, n)| n.exists() && !olds.contains(n)) {
        return Err(format!("{} already exists.", file_name(n)));
    }
    // Two steps through temporary names, so swapping names within the set can't collide.
    let tmp: Vec<PathBuf> = pairs.iter().enumerate().map(|(i, (o, _))| o.with_file_name(format!(".glance-rename-{i}.tmp"))).collect();
    for ((o, _), t) in pairs.iter().zip(&tmp) {
        std::fs::rename(o, t).map_err(|e| format!("{}: {e}", file_name(o)))?;
    }
    for ((_, n), t) in pairs.iter().zip(&tmp) {
        std::fs::rename(t, n).map_err(|e| format!("{}: {e}", file_name(n)))?;
    }
    Ok(pairs)
}

/// A page with an OpenStreetMap map and a pin per photo, opened in the browser.
pub fn map_html(points: &[(String, f64, f64)]) -> String {
    // Names go into a JS string: escape anything that could end it or open a tag.
    let js = |s: &str| s.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c.to_string() } else { format!("\\u{:04x}", c as u32) }).collect::<String>();
    let pins: Vec<String> = points.iter().map(|(n, lat, lon)| format!("[{lat},{lon},\"{}\"]", js(n))).collect();
    format!(
        r#"<!doctype html><meta charset="utf-8"><title>Photo map - Glance</title>
<link rel="stylesheet" href="https://unpkg.com/leaflet@1.9.4/dist/leaflet.css">
<script src="https://unpkg.com/leaflet@1.9.4/dist/leaflet.js"></script>
<style>html,body,#m{{height:100%;margin:0}}</style><div id="m"></div>
<script>
const pins=[{}];
const map=L.map('m');
L.tileLayer('https://tile.openstreetmap.org/{{z}}/{{x}}/{{y}}.png',{{maxZoom:19,attribution:'&copy; OpenStreetMap contributors'}}).addTo(map);
const b=[];for(const [la,lo,n] of pins){{L.marker([la,lo]).addTo(map).bindPopup(n.replace(/&/g,'&amp;').replace(/</g,'&lt;'));b.push([la,lo]);}}
map.fitBounds(b,{{maxZoom:14,padding:[30,30]}});
</script>"#,
        pins.join(",")
    )
}

/// Cuts `src` from `start` to `end` seconds without re-encoding, using ffmpeg if it's installed.
/// Cuts land on the nearest keyframes. Returns the new file.
pub fn trim_video(src: &Path, start: f64, end: f64) -> Result<PathBuf, String> {
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = src.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
    let dest = unique_dest(src.parent().unwrap_or(Path::new(".")), &format!("{stem}-trimmed.{ext}"));
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{start:.3}"), "-i"])
        .arg(src)
        .args(["-t", &format!("{:.3}", end - start), "-c", "copy", "-map", "0", "-avoid_negative_ts", "make_zero"])
        .arg(&dest);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash
    }
    let out = cmd.output().map_err(|_| "Trimming needs ffmpeg (winget install ffmpeg)".to_string())?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&dest);
        return Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("ffmpeg failed").to_string());
    }
    Ok(dest)
}

#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum SortKey {
    #[default]
    Name,
    Modified,
    Size,
}

/// Sort key that orders numbers by value, like Explorer: "img2" before "img10".
fn natural_key(name: &str) -> Vec<(u8, String)> {
    let mut parts: Vec<(u8, String)> = vec![];
    for c in name.to_lowercase().chars() {
        let digit = c.is_ascii_digit() as u8;
        match parts.last_mut() {
            Some((d, s)) if *d == digit => s.push(c),
            _ => parts.push((digit, c.to_string())),
        }
    }
    // Pad digit runs so they compare by value as strings.
    for (d, s) in &mut parts {
        if *d == 1 {
            *s = format!("{:0>20}", s.trim_start_matches('0'));
        }
    }
    parts
}

pub fn sort(entries: &mut [Entry], key: SortKey, desc: bool) {
    match key {
        SortKey::Name => entries.sort_by_cached_key(|e| natural_key(&file_name(&e.path))),
        SortKey::Modified => entries.sort_by_key(|e| e.modified),
        SortKey::Size => entries.sort_by_key(|e| e.size),
    }
    if desc {
        entries.reverse();
    }
}

/// `dir/name`, or `dir/stem (1).ext`, `(2)`, ... if that's taken.
pub fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut dest = dir.join(name);
    for i in 1.. {
        if !dest.exists() {
            break;
        }
        dest = dir.join(format!("{stem} ({i}){ext}"));
    }
    dest
}

/// Copies or moves `src` into `dir` without overwriting anything. Returns the new path.
pub fn transfer(src: &Path, dir: &Path, mv: bool) -> Result<PathBuf, String> {
    if mv && src.parent() == Some(dir) {
        return Err(format!("{} is already in that folder", file_name(src)));
    }
    let dest = unique_dest(dir, &file_name(src));
    if mv && std::fs::rename(src, &dest).is_ok() {
        return Ok(dest);
    }
    // Copy (or move across drives, where rename fails): copy, then remove the original.
    std::fs::copy(src, &dest).map_err(|e| format!("{}: {e}", file_name(src)))?;
    if mv {
        std::fs::remove_file(src).map_err(|e| format!("{}: {e}", file_name(src)))?;
    }
    Ok(dest)
}

/// Renames `src` within its folder. Refuses names Windows can't store or that already exist.
/// A file or folder name Windows accepts, trimmed.
fn check_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.ends_with('.') || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err("That name isn't allowed. Avoid / \\ : * ? \" < > |".into());
    }
    Ok(name)
}

/// Makes a new, empty folder called `name` inside `parent`.
pub fn new_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let name = check_name(name)?;
    let dest = parent.join(name);
    if dest.exists() {
        return Err(format!("{name} already exists."));
    }
    std::fs::create_dir(&dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

pub fn rename_file(src: &Path, new_name: &str) -> Result<PathBuf, String> {
    let name = check_name(new_name)?;
    let dest = src.with_file_name(name);
    if dest == src {
        return Ok(dest);
    }
    // A case-only change (photo.jpg -> Photo.jpg) "exists" on Windows but is fine.
    let case_only = file_name(src).eq_ignore_ascii_case(name);
    if dest.exists() && !case_only {
        return Err(format!("{name} already exists."));
    }
    std::fs::rename(src, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

/// Opens Explorer with the file selected.
pub fn reveal(p: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn();
    }
    #[cfg(not(windows))]
    let _ = Command::new("xdg-open").arg(p.parent().unwrap_or(p)).spawn();
}

/// Shows the system "Open with" chooser.
pub fn open_with(p: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("rundll32.exe").raw_arg(format!("shell32.dll,OpenAs_RunDLL {}", p.display())).spawn();
    }
    #[cfg(not(windows))]
    let _ = Command::new("xdg-open").arg(p).spawn();
}

#[cfg(windows)]
pub fn set_wallpaper(p: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::WindowsAndMessaging::*;
    let mut wide: Vec<u16> = p.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(wide.as_mut_ptr().cast()),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    }
    .map_err(|e| e.message().to_string())
}

#[cfg(not(windows))]
pub fn set_wallpaper(_: &Path) -> Result<(), String> {
    Err("Setting the wallpaper is only supported on Windows".into())
}

pub fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn new_folder_checks_the_name() {
        let dir = std::env::temp_dir().join("glance-newfolder-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let made = super::new_folder(&dir, "  Trips 2026 ").unwrap();
        assert_eq!(made, dir.join("Trips 2026"));
        assert!(made.is_dir());
        assert!(super::new_folder(&dir, "Trips 2026").unwrap_err().contains("already exists"));
        assert!(super::new_folder(&dir, "a/b").is_err());
        assert!(super::new_folder(&dir, "   ").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    use super::*;

    #[test]
    fn lists_sorts_and_names_safely() {
        let dir = std::env::temp_dir().join("photo-viewer-files-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["b.JPG", "a.png", "notes.txt", ".hidden.png", "img10.png", "img2.png"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let (mut images, dirs) = list_dir(&dir);
        sort(&mut images, SortKey::Name, false);
        let names: Vec<_> = images.iter().map(|e| file_name(&e.path)).collect();
        assert_eq!(names, ["a.png", "b.JPG", "img2.png", "img10.png"]);
        assert_eq!(dirs, [dir.join("sub")]);

        assert_eq!(unique_dest(&dir, "a.png"), dir.join("a (1).png"));
        assert!(rename_file(&dir.join("a.png"), "bad:name.png").is_err());
        assert!(rename_file(&dir.join("a.png"), "b.JPG").is_err(), "won't overwrite");
        assert_eq!(rename_file(&dir.join("a.png"), "c.png").unwrap(), dir.join("c.png"));

        let copied = transfer(&dir.join("c.png"), &dir.join("sub"), false).unwrap();
        assert!(copied.exists() && dir.join("c.png").exists());
        let again = transfer(&dir.join("c.png"), &dir.join("sub"), true).unwrap();
        assert_eq!(again, dir.join("sub").join("c (1).png"));
        assert!(!dir.join("c.png").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn batch_rename_numbers_and_swaps() {
        assert_eq!(pattern_name("Trip_###", "x", 7, "jpg"), "Trip_007.jpg");
        assert_eq!(pattern_name("*-#", "beach", 12, "png"), "beach-12.png");
        assert_eq!(pattern_name("Holiday", "x", 2, "jpg"), "Holiday (2).jpg");
        let dir = std::env::temp_dir().join("glance-batch-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["a.jpg", "b.jpg"] {
            std::fs::write(dir.join(f), f).unwrap();
        }
        // Reversed order with names that already exist in the set: must not collide.
        let paths = [dir.join("b.jpg"), dir.join("a.jpg")];
        assert!(batch_rename(&paths, "bad:name").is_err());
        let done = batch_rename(&paths, "#").unwrap();
        assert_eq!(done[0].1, dir.join("1.jpg"));
        assert_eq!(std::fs::read_to_string(dir.join("1.jpg")).unwrap(), "b.jpg");
        assert_eq!(std::fs::read_to_string(dir.join("2.jpg")).unwrap(), "a.jpg");
        std::fs::write(dir.join("Taken (1).jpg"), "x").unwrap();
        assert!(batch_rename(&[dir.join("1.jpg")], "Taken").is_err(), "won't overwrite a file outside the set");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn map_page_escapes_names() {
        let html = map_html(&[("a\"</script>.jpg".into(), 1.5, -2.0)]);
        assert!(html.contains("[1.5,-2,\"a\\u0022\\u003c\\u002fscript\\u003e.jpg\"]"));
    }
}
