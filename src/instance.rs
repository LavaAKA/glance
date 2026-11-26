//! One window at a time: a second launch hands its path to the running viewer and exits.
//! Uses a loopback-only TCP port; anything received is only ever treated as a path to open.
use eframe::egui;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

const ADDR: &str = "127.0.0.1:47811";
const HELLO: &[u8; 7] = b"glance\n";

/// Gives `path` (or just "come to front") to a running viewer. True if one accepted it.
pub fn hand_off(path: Option<&Path>) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&ADDR.parse().unwrap(), Duration::from_millis(300)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    // Make sure it's really a viewer on that port, not some other program.
    let mut hello = [0u8; HELLO.len()];
    if s.read_exact(&mut hello).is_err() || &hello != HELLO {
        return false;
    }
    let msg = path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    s.write_all(msg.as_bytes()).is_ok()
}

/// Receives paths from later launches (None = just focus). Returns None if the port is taken.
pub fn listen(ctx: egui::Context) -> Option<mpsc::Receiver<Option<PathBuf>>> {
    let listener = TcpListener::bind(ADDR).ok()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let _ = s.write_all(HELLO);
            let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
            let mut msg = String::new();
            let _ = s.take(32 * 1024).read_to_string(&mut msg);
            if tx.send((!msg.is_empty()).then(|| PathBuf::from(msg))).is_err() {
                return;
            }
            ctx.request_repaint();
        }
    });
    Some(rx)
}
