//! Lets Windows offer Glance as the app for photos and videos. Windows doesn't allow an app to
//! make itself the default; we register what Glance can open (for this user only, no admin),
//! then open Settings > Default apps on Glance's page so the user picks it.
use std::path::Path;

const APP: &str = "Glance";
const IMAGE: &str = "Glance.Image";
const VIDEO: &str = "Glance.Video";
const CAPS: &str = r"Software\Glance\Capabilities";

/// Every registry value to write, as (key under HKEY_CURRENT_USER, value name or None for the
/// key's default value, data).
fn entries(exe: &Path) -> Vec<(String, Option<String>, String)> {
    let exe = exe.display().to_string();
    let open = format!("\"{exe}\" \"%1\"");
    let icon = format!("\"{exe}\",0");
    let mut out = vec![];
    let mut set = |k: &str, n: Option<&str>, v: &str| out.push((k.to_string(), n.map(str::to_string), v.to_string()));
    for (progid, kind) in [(IMAGE, "Glance photo"), (VIDEO, "Glance video")] {
        let base = format!(r"Software\Classes\{progid}");
        set(&base, None, kind);
        set(&format!(r"{base}\DefaultIcon"), None, &icon);
        set(&format!(r"{base}\shell\open\command"), None, &open);
        set(&format!(r"{base}\Application"), Some("ApplicationName"), APP);
    }
    // "Open with" shows "Glance" rather than "glance.exe".
    set(r"Software\Classes\Applications\glance.exe", Some("FriendlyAppName"), APP);
    set(r"Software\Classes\Applications\glance.exe\shell\open\command", None, &open);
    set(CAPS, Some("ApplicationName"), APP);
    set(CAPS, Some("ApplicationDescription"), "Photo and video viewer");
    set(r"Software\RegisteredApplications", Some(APP), CAPS);
    let types = crate::files::EXTS.iter().map(|e| (e, IMAGE)).chain(crate::video::EXTS.iter().map(|e| (e, VIDEO)));
    for (ext, progid) in types {
        set(&format!(r"{CAPS}\FileAssociations"), Some(&format!(".{ext}")), progid);
        set(&format!(r"Software\Classes\.{ext}\OpenWithProgids"), Some(progid), "");
    }
    out
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Registry::*;
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    use windows::core::HSTRING;

    fn name(n: &Option<String>) -> HSTRING {
        HSTRING::from(n.as_deref().unwrap_or(""))
    }

    pub fn register() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        for (key, value_name, data) in super::entries(&exe) {
            let wide: Vec<u16> = data.encode_utf16().chain([0]).collect();
            // SAFETY: `wide` is a NUL-terminated UTF-16 string of the given byte length.
            let err = unsafe {
                RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key.as_str()), &name(&value_name), REG_SZ.0, Some(wide.as_ptr() as _), (wide.len() * 2) as u32)
            };
            err.ok().map_err(|e| format!("Couldn't register with Windows: {e}"))?;
        }
        notify();
        Ok(())
    }

    /// Removes only what `register` wrote.
    pub fn unregister() {
        let exe = std::env::current_exe().unwrap_or_default();
        // SAFETY: plain registry calls on keys under HKEY_CURRENT_USER; failures (already gone) are fine.
        unsafe {
            for tree in [r"Software\Classes\Glance.Image", r"Software\Classes\Glance.Video", r"Software\Classes\Applications\glance.exe", r"Software\Glance"] {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(tree));
            }
            for (key, value_name, _) in super::entries(&exe) {
                if key.ends_with("OpenWithProgids") || key == r"Software\RegisteredApplications" {
                    let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key.as_str()), &name(&value_name));
                }
            }
        }
        notify();
    }

    /// Tells Explorer the file types changed, so icons and "Open with" update now.
    fn notify() {
        // SAFETY: documented call with no pointers for this event.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }

    /// Settings > Apps > Default apps, on Glance's page.
    pub fn open_settings() {
        let _ = std::process::Command::new("explorer").arg("ms-settings:defaultapps?registeredAppUser=Glance").spawn();
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn register() -> Result<(), String> {
        Err("Only on Windows".into())
    }
    pub fn unregister() {}
    pub fn open_settings() {}
}

pub use imp::{open_settings, register, unregister};

#[cfg(test)]
mod tests {
    #[test]
    fn registers_every_type_with_a_quoted_command() {
        let e = super::entries(std::path::Path::new(r"C:\Apps\Glance\glance.exe"));
        let has = |k: &str, n: Option<&str>, v: &str| e.iter().any(|(ek, en, ev)| ek == k && en.as_deref() == n && ev == v);
        assert!(has(r"Software\Classes\Glance.Image\shell\open\command", None, r#""C:\Apps\Glance\glance.exe" "%1""#));
        assert!(has(r"Software\Glance\Capabilities\FileAssociations", Some(".jpg"), "Glance.Image"));
        assert!(has(r"Software\Glance\Capabilities\FileAssociations", Some(".mp4"), "Glance.Video"));
        assert!(has(r"Software\Classes\.heic\OpenWithProgids", Some("Glance.Image"), ""));
        assert!(has(r"Software\RegisteredApplications", Some("Glance"), r"Software\Glance\Capabilities"));
        // Everything stays under the user's own keys.
        assert!(e.iter().all(|(k, ..)| k.starts_with(r"Software\")));
    }
}
