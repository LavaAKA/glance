//! Glance-Setup.exe: installs Glance into Program Files like a normal Windows app.
//! It carries glance.exe inside it, so it's a single download. Windows asks for administrator
//! approval when it starts (see setup.manifest).
#![windows_subsystem = "windows"]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::HSTRING;

const GLANCE: &[u8] = include_bytes!("../../target/release/glance.exe");
const UNINSTALL: &str = include_str!("../../installer/uninstall.ps1");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const NO_WINDOW: u32 = 0x0800_0000; // CREATE_NO_WINDOW
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Glance";

fn message(text: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    // SAFETY: plain message box with owned strings.
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Glance Setup"), style) }
}

fn ask(text: &str) -> bool {
    message(text, MB_YESNO | MB_ICONQUESTION) == IDYES
}

fn main() {
    let question = format!(
        "Install Glance {VERSION}?\n\nGlance goes in C:\\Program Files\\Glance, with a Start menu shortcut, \
         and Windows will list it as a photo and video app.\n\nIf Glance is already installed, this updates it."
    );
    if !ask(&question) {
        return;
    }
    match install() {
        Ok(exe) => {
            if ask("Glance is installed.\n\nChoose Glance as your default app for photos and videos now?") {
                open_unelevated("ms-settings:defaultapps?registeredAppUser=Glance");
            }
            open_unelevated(&exe.display().to_string());
        }
        Err(e) => {
            message(&format!("Glance couldn't be installed.\n\n{e}"), MB_OK | MB_ICONERROR);
        }
    }
}

fn install() -> Result<PathBuf, String> {
    let dir = PathBuf::from(std::env::var_os("ProgramFiles").ok_or("Program Files wasn't found")?).join("Glance");
    let exe = dir.join("glance.exe");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    write_app(&exe)?;
    std::fs::write(dir.join("uninstall.ps1"), UNINSTALL).map_err(|e| e.to_string())?;
    shortcut(&exe, &dir)?;
    installed_apps_entry(&exe, &dir)?;
    // List Glance as a photo and video app (file types and "Open with").
    let _ = Command::new(&exe).arg("--register").creation_flags(NO_WINDOW).status();
    Ok(exe)
}

/// Copies glance.exe in, closing a running Glance first (politely, so it saves its settings).
fn write_app(exe: &Path) -> Result<(), String> {
    for force in [false, false, true] {
        match std::fs::write(exe, GLANCE) {
            Ok(()) => return Ok(()),
            Err(_) => {
                let mut kill = Command::new("taskkill");
                kill.args(["/IM", "glance.exe"]);
                if force {
                    kill.arg("/F");
                }
                let _ = kill.creation_flags(NO_WINDOW).status();
                std::thread::sleep(Duration::from_millis(1500));
            }
        }
    }
    std::fs::write(exe, GLANCE).map_err(|e| format!("Couldn't write {}: {e}\nClose Glance and run setup again.", exe.display()))
}

/// Start menu shortcut for everyone on this PC.
fn shortcut(exe: &Path, dir: &Path) -> Result<(), String> {
    let data = std::env::var_os("ProgramData").ok_or("ProgramData wasn't found")?;
    let lnk = Path::new(&data).join(r"Microsoft\Windows\Start Menu\Programs\Glance.lnk");
    let script = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath = '{}'; $s.WorkingDirectory = '{}'; $s.Description = 'Photo and Video Viewer'; $s.Save()",
        lnk.display(),
        exe.display(),
        dir.display()
    );
    let ok = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(()).ok_or_else(|| "Couldn't create the Start menu shortcut".into())
}

/// Settings > Apps > Installed apps, with a working Uninstall button.
fn installed_apps_entry(exe: &Path, dir: &Path) -> Result<(), String> {
    let exe_s = exe.display().to_string();
    let text = [
        ("DisplayName", "Glance".to_string()),
        ("DisplayIcon", format!("\"{exe_s}\",0")),
        ("DisplayVersion", VERSION.to_string()),
        ("Publisher", "Glance".to_string()),
        ("InstallLocation", dir.display().to_string()),
        ("UninstallString", format!("powershell.exe -NoProfile -ExecutionPolicy Bypass -File \"{}\"", dir.join("uninstall.ps1").display())),
        ("URLInfoAbout", "https://lavaaka.github.io/glance/".to_string()),
    ];
    for (name, value) in text {
        let wide: Vec<u16> = value.encode_utf16().chain([0]).collect();
        set(name, REG_SZ, wide.as_ptr() as _, wide.len() * 2)?;
    }
    for (name, value) in [("NoModify", 1u32), ("NoRepair", 1), ("EstimatedSize", (GLANCE.len() / 1024) as u32)] {
        set(name, REG_DWORD, &value as *const u32 as _, 4)?;
    }
    Ok(())
}

fn set(name: &str, kind: REG_VALUE_TYPE, data: *const std::ffi::c_void, len: usize) -> Result<(), String> {
    // SAFETY: `data` points at `len` valid bytes for the duration of the call.
    let err = unsafe { RegSetKeyValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(UNINSTALL_KEY), &HSTRING::from(name), kind.0, Some(data), len as u32) };
    err.ok().map_err(|e| format!("Couldn't add Glance to Installed apps: {e}"))
}

/// Opens a program or link as the signed-in user rather than as administrator (setup runs
/// elevated; an elevated Glance couldn't accept files dragged in from Explorer).
fn open_unelevated(target: &str) {
    let _ = Command::new("explorer.exe").arg(target).spawn();
}
