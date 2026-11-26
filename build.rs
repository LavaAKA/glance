// Embeds the app icon into the Windows .exe (what Explorer and the taskbar show).
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.rc");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        // GNU windres breaks on spaces in include paths ("Photo Viewer"), so run it from the
        // project folder with relative input paths only.
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("icon.o");
        let status = Command::new("windres")
            .current_dir(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .args(["assets/icon.rc", "-O", "coff", "-o"])
            .arg(&out)
            .status()
            .expect("windres (from the GNU toolchain) should be on PATH");
        assert!(status.success(), "windres failed to compile assets/icon.rc");
        println!("cargo:rustc-link-arg-bins={}", out.display());
    } else {
        winresource::WindowsResource::new().set_icon("assets/icon.ico").compile().expect("embed icon");
    }
}
