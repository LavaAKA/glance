//! Dragging files out of Glance into Explorer, a chat app, an email... (Windows drag and drop).
use std::path::PathBuf;

/// Starts a system drag of `paths` and returns when they're dropped (or the drag is cancelled).
/// Only offers Copy, so a drop never moves the user's files by surprise.
#[cfg(windows)]
pub fn drag_files(paths: &[PathBuf]) {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
    use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DoDragDrop, IDropSource};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{BHID_DataObject, IShellItemArray, SHCreateShellItemArrayFromIDLists, SHParseDisplayName};
    use windows::core::PCWSTR;
    unsafe {
        let mut pidls: Vec<*const ITEMIDLIST> = vec![];
        for p in paths {
            let wide: Vec<u16> = p.as_os_str().encode_wide().chain([0]).collect();
            let mut pidl = std::ptr::null_mut();
            if SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None).is_ok() {
                pidls.push(pidl);
            }
        }
        let data: Option<IDataObject> = SHCreateShellItemArrayFromIDLists(&pidls)
            .and_then(|a: IShellItemArray| a.BindToHandler(None, &BHID_DataObject))
            .ok();
        for p in pidls {
            CoTaskMemFree(Some(p.cast()));
        }
        let Some(data) = data else { return };
        let source: IDropSource = imp::DropSource.into();
        let mut effect = DROPEFFECT(0);
        let _ = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK};
    use windows::Win32::System::Ole::{DROPEFFECT, IDropSource, IDropSource_Impl};
    use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
    use windows::core::{BOOL, HRESULT, implement};

    #[implement(IDropSource)]
    pub struct DropSource;

    impl IDropSource_Impl for DropSource_Impl {
        fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
            if escape.as_bool() {
                DRAGDROP_S_CANCEL
            } else if keys.0 & MK_LBUTTON.0 == 0 {
                DRAGDROP_S_DROP // mouse released
            } else {
                S_OK
            }
        }

        fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }
}

#[cfg(not(windows))]
pub fn drag_files(_: &[PathBuf]) {}
