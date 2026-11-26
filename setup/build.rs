// Embeds the icon, version info and the "run as administrator" manifest into Glance-Setup.exe.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=setup.rc");
    println!("cargo:rerun-if-changed=setup.manifest");
    println!("cargo:rerun-if-changed=../target/release/glance.exe");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("setup.o");
    // Run from this folder with relative paths: GNU windres breaks on spaces in include paths.
    let status = Command::new("windres")
        .current_dir(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .args(["setup.rc", "-O", "coff", "-o"])
        .arg(&out)
        .status()
        .expect("windres (from the GNU toolchain) should be on PATH");
    assert!(status.success(), "windres failed to compile setup.rc");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
