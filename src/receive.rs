//! "Receive from phone": a small web server on your home network. The phone scans a QR code,
//! gets an upload page, and sends photos and videos straight into a folder on this PC.
//!
//! Safety: it only runs while the Receive window is open. The QR code holds a one-time code that
//! is replaced every 10 seconds; a phone that opens the page with it gets its own private pass for
//! uploading, so a photo of an old QR code is useless. Only photo and video file types are
//! accepted, nothing is ever overwritten, and names from the phone can't reach outside the folder.
use eframe::egui;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

const PAGE: &str = include_str!("../assets/upload.html");
const PORT: u16 = 47812;
/// Largest single file accepted (long 4K phone videos fit).
const MAX_FILE: u64 = 8 << 30;
/// How long each QR code works.
pub const QR_LIFE: Duration = Duration::from_secs(10);
/// A code scanned just before it was replaced still opens the page for a few seconds.
const GRACE: Duration = Duration::from_secs(5);

struct Codes {
    qr: String,
    old: Option<(String, Instant)>, // the code just replaced, and when
    passes: HashSet<String>,        // phones that opened the page
}

pub struct Server {
    pub dir: PathBuf,
    pub rx: mpsc::Receiver<Result<PathBuf, String>>,
    /// How many times a phone has opened the page.
    pub visits: Arc<AtomicUsize>,
    pub made: Instant,
    base: String,
    codes: Arc<Mutex<Codes>>,
    stop: Arc<AtomicBool>,
}

impl Server {
    pub fn url(&self) -> String {
        format!("{}/?t={}", self.base, self.codes.lock().unwrap().qr)
    }

    /// Replaces the QR code. Phones already on the page keep working.
    pub fn rotate(&mut self) {
        let mut c = self.codes.lock().unwrap();
        let old = std::mem::replace(&mut c.qr, random_code());
        c.old = Some((old, Instant::now()));
        self.made = Instant::now();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A 128-bit random code from the OS-seeded hasher keys (no extra crate needed).
fn random_code() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

/// This PC's address on the home network: a wired or Wi-Fi card that is up and has a router,
/// so a VPN tunnel's address is never picked.
#[cfg(windows)]
fn lan_ip() -> Option<IpAddr> {
    use windows::Win32::NetworkManagement::IpHelper::*;
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    let mut size = 32_000u32;
    let mut buf = vec![0u64; size as usize / 8];
    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let first = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
    // SAFETY: the buffer is 8-byte aligned and `size` says how big it is; Windows fills a linked
    // list inside it, which is only read while `buf` lives.
    unsafe {
        if GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(first), &mut size) != 0 {
            return udp_guess();
        }
        let mut a = first;
        while let Some(ad) = a.as_ref() {
            let wired_or_wifi = ad.IfType == 6 || ad.IfType == 71;
            if wired_or_wifi && ad.OperStatus == IfOperStatusUp && !ad.FirstGatewayAddress.is_null() {
                let mut u = ad.FirstUnicastAddress;
                while let Some(ua) = u.as_ref() {
                    let sa = ua.Address.lpSockaddr;
                    if !sa.is_null() && (*sa).sa_family == AF_INET {
                        let ip = std::net::Ipv4Addr::from(u32::from_be((*(sa as *const SOCKADDR_IN)).sin_addr.S_un.S_addr));
                        if !ip.is_link_local() && !ip.is_loopback() {
                            return Some(IpAddr::V4(ip));
                        }
                    }
                    u = ua.Next;
                }
            }
            a = ad.Next;
        }
    }
    udp_guess()
}

#[cfg(not(windows))]
fn lan_ip() -> Option<IpAddr> {
    udp_guess()
}

/// Fallback: connecting a UDP socket sends nothing; it only asks which card would be used.
fn udp_guess() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.168.0.1:9").ok()?;
    Some(s.local_addr().ok()?.ip()).filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

/// Starts receiving into `dir`. Stops when the returned Server is dropped.
pub fn start(dir: PathBuf, ctx: egui::Context) -> Result<Server, String> {
    std::fs::create_dir_all(&dir).map_err(|e| format!("Can't use {}: {e}", dir.display()))?;
    let listener = TcpListener::bind(("0.0.0.0", PORT))
        .or_else(|_| TcpListener::bind(("0.0.0.0", 0)))
        .map_err(|e| format!("Couldn't start: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let ip = lan_ip().ok_or("This PC doesn't seem to be on a network")?;
    let codes = Arc::new(Mutex::new(Codes { qr: random_code(), old: None, passes: HashSet::new() }));
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let visits = Arc::new(AtomicUsize::new(0));
    let (stop2, dir2, codes2, visits2) = (stop.clone(), dir.clone(), codes.clone(), visits.clone());
    std::thread::spawn(move || {
        while !stop2.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let (codes, dir, tx, ctx, stop, visits) = (codes2.clone(), dir2.clone(), tx.clone(), ctx.clone(), stop2.clone(), visits2.clone());
                    std::thread::spawn(move || {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        let result = handle(stream, &codes, &dir, &visits);
                        if let Some(r) = result {
                            let _ = tx.send(r);
                        }
                        ctx.request_repaint();
                    });
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    });
    Ok(Server { dir, rx, visits, made: Instant::now(), base: format!("http://{ip}:{port}"), codes, stop })
}

fn respond(s: &mut TcpStream, status: &str, kind: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(body));
}

const EXPIRED: &str = "<!doctype html><meta name=viewport content='width=device-width,initial-scale=1'><title>Glance</title>\
<body style='font:16px -apple-system,Segoe UI,sans-serif;padding:24px;background:#1e1e1e;color:#ddd'>\
<h2 style='font-size:20px'>This code has expired</h2><p>Scan the QR code in Glance again. It changes every 10 seconds.</p>";

/// Serves one request. Returns Some for an upload (saved file or error), None otherwise.
fn handle(mut stream: TcpStream, codes: &Mutex<Codes>, dir: &Path, visits: &AtomicUsize) -> Option<Result<PathBuf, String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    // Phone browsers often try a secure (https) connection first. Hang up at once so they fall
    // back to the plain page instead of sitting on a blank screen.
    let mut first = [0u8; 1];
    if stream.peek(&mut first).ok()? == 0 || !first[0].is_ascii_uppercase() {
        return None;
    }
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.by_ref().take(8192).read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next()?.to_string(), parts.next()?.to_string());
    let mut length: Option<u64> = None;
    loop {
        let mut h = String::new();
        if reader.by_ref().take(8192).read_line(&mut h).ok()? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().ok();
            }
        }
    }
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let param = |name: &str| query.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| percent_decode(v));
    let t = param("t").unwrap_or_default();
    match (method.as_str(), path) {
        ("GET", "/") => {
            // A fresh QR code gets this phone its own pass; a pass reloads the page.
            let pass = {
                let mut c = codes.lock().unwrap();
                let fresh = !t.is_empty() && (t == c.qr || c.old.as_ref().is_some_and(|(o, at)| *o == t && at.elapsed() < GRACE));
                if fresh {
                    let p = random_code();
                    c.passes.insert(p.clone());
                    Some(p)
                } else {
                    c.passes.contains(&t).then(|| t.clone())
                }
            };
            match pass {
                Some(p) => {
                    visits.fetch_add(1, Ordering::Relaxed);
                    respond(&mut stream, "200 OK", "text/html; charset=utf-8", PAGE.replace("__PASS__", &p).as_bytes());
                }
                None => respond(&mut stream, "403 Forbidden", "text/html; charset=utf-8", EXPIRED.as_bytes()),
            }
            None
        }
        ("POST", "/upload") => {
            if !codes.lock().unwrap().passes.contains(&t) {
                respond(&mut stream, "403 Forbidden", "text/plain", b"expired");
                return None;
            }
            let result = save_upload(&mut reader, dir, param("name").as_deref().unwrap_or(""), length);
            match &result {
                Ok(_) => respond(&mut stream, "200 OK", "text/plain", b"ok"),
                Err(e) => respond(&mut stream, "400 Bad Request", "text/plain; charset=utf-8", e.as_bytes()),
            }
            Some(result)
        }
        _ => {
            respond(&mut stream, "404 Not Found", "text/plain", b"not found");
            None
        }
    }
}

fn save_upload(body: &mut impl Read, dir: &Path, name: &str, length: Option<u64>) -> Result<PathBuf, String> {
    let name = safe_name(name).ok_or_else(|| format!("{name}: not a photo or video"))?;
    let len = length.ok_or("missing size")?;
    if len > MAX_FILE {
        return Err(format!("{name}: too big"));
    }
    // Write to a hidden partial file first, so Glance never shows half a photo.
    let partial = dir.join(format!(".{}.partial", random_code()));
    let result = (|| {
        let mut f = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
        let got = std::io::copy(&mut body.take(len), &mut f).map_err(|e| e.to_string())?;
        if got != len {
            return Err(format!("{name}: the upload was cut off"));
        }
        let dest = crate::files::unique_dest(dir, &name);
        std::fs::rename(&partial, &dest).map_err(|e| e.to_string())?;
        Ok(dest)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

/// Just the file name, without any folder parts or characters Windows can't store, and only
/// for photo and video types.
fn safe_name(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next()?;
    let clean: String = base.chars().filter(|c| !c.is_control() && !"<>:\"|?*".contains(*c)).collect();
    let clean = clean.trim().trim_start_matches('.').trim_end_matches(['.', ' ']).chars().take(150).collect::<String>();
    let p = Path::new(&clean);
    (!clean.is_empty() && (crate::files::is_image(p) || crate::video::is_video(p))).then_some(clean)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => {
                    out.push(v);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The link as a QR code image (black on white, with the quiet border scanners need).
pub fn qr_image(text: &str) -> Option<egui::ColorImage> {
    // Highest error correction, so the app icon covering the middle doesn't stop phones reading it.
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::H).ok()?;
    let colors = code.to_colors();
    let w = (colors.len() as f64).sqrt() as usize;
    let border = 4;
    let side = w + border * 2;
    let mut px = vec![egui::Color32::WHITE; side * side];
    for (i, c) in colors.iter().enumerate() {
        if *c == qrcode::Color::Dark {
            px[(i / w + border) * side + i % w + border] = egui::Color32::BLACK;
        }
    }
    Some(egui::ColorImage::new([side, side], px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_a_home_network_address() {
        let ip = lan_ip().expect("on a network");
        eprintln!("phone link would use {ip}");
        assert!(matches!(ip, IpAddr::V4(v) if v.is_private()));
    }

    #[test]
    fn names_stay_inside_the_folder() {
        assert_eq!(safe_name("IMG_0001.HEIC").as_deref(), Some("IMG_0001.HEIC"));
        assert_eq!(safe_name("../../Windows/evil.jpg").as_deref(), Some("evil.jpg"));
        assert_eq!(safe_name("C:\\x\\y.png").as_deref(), Some("y.png"));
        assert_eq!(safe_name("..jpg").as_deref(), None, "nothing left but an extension");
        assert_eq!(safe_name("run.exe"), None);
        assert_eq!(safe_name("a<b>.mp4").as_deref(), Some("ab.mp4"));
        assert_eq!(percent_decode("My%20Photo+1.jpg"), "My Photo 1.jpg");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn upload_needs_a_pass_from_a_fresh_qr_code() {
        let dir = std::env::temp_dir().join("glance-receive-test");
        let _ = std::fs::remove_dir_all(&dir);
        let mut server = start(dir.clone(), egui::Context::default()).unwrap();
        let addr = server.base.trim_start_matches("http://").to_string();
        let send = |req: String, body: &[u8]| {
            let mut s = TcpStream::connect(&addr).unwrap();
            s.write_all(req.as_bytes()).unwrap();
            s.write_all(body).unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
            out
        };
        // An https attempt is hung up on straight away instead of hanging.
        let started = Instant::now();
        assert_eq!(send(String::new(), &[0x16, 3, 1, 0, 5]), "");
        assert!(started.elapsed() < Duration::from_secs(2));

        let qr = server.url().split("t=").nth(1).unwrap().to_string();
        let page = send(format!("GET /?t={qr} HTTP/1.1\r\n\r\n"), b"");
        assert!(page.starts_with("HTTP/1.1 200") && !page.contains("__PASS__"));
        let pass = page.split("const pass = \"").nth(1).unwrap().split('"').next().unwrap().to_string();
        assert_ne!(pass, qr);
        assert_eq!(server.visits.load(Ordering::Relaxed), 1);

        let body = b"fake jpeg bytes";
        let upload = |t: &str| send(format!("POST /upload?t={t}&name=My%20Photo.jpg HTTP/1.1\r\nContent-Length: {}\r\n\r\n", body.len()), body);
        assert!(upload(&qr).starts_with("HTTP/1.1 403"), "the QR code alone can't upload");
        assert!(upload("wrong").starts_with("HTTP/1.1 403"));
        let ok = upload(&pass);
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        let saved = server.rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(saved, dir.join("My Photo.jpg"));
        assert_eq!(std::fs::read(&saved).unwrap(), body);

        // A new QR code: the phone already on the page keeps working, and a never-used code is refused.
        server.rotate();
        assert!(upload(&pass).starts_with("HTTP/1.1 200"));
        assert!(send("GET /?t=0123456789abcdef0123456789abcdef HTTP/1.1\r\n\r\n".into(), b"").contains("expired"));
        drop(server);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(qr_image("http://192.168.1.2:47812/?t=abc").is_some());
    }
}