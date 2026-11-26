//! Left sidebar: pinned folders, Places (Home, Pictures...), and drives, each a folder tree
//! that lists its subfolders only when expanded.
use super::{Action, Viewer};
use crate::files::file_name;
use crate::style::*;
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};

/// Subfolders of `dir`, skipping hidden and system ones ($Recycle.Bin, AppData...).
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(meta) = e.metadata() else { return false };
            #[cfg(windows)]
            let hidden = {
                use std::os::windows::fs::MetadataExt;
                meta.file_attributes() & 0x6 != 0 // HIDDEN | SYSTEM
            };
            #[cfg(not(windows))]
            let hidden = false;
            meta.is_dir() && !hidden && !name.starts_with('.') && !name.starts_with('$')
        })
        .map(|e| e.path())
        .collect();
    out.sort_by_cached_key(|p| file_name(p).to_lowercase());
    out
}

pub(crate) fn places() -> Vec<(&'static str, &'static str, PathBuf)> {
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from) else {
        return vec![];
    };
    [(I_HOME, "Home", home.clone()), (I_PC, "Desktop", home.join("Desktop")), (I_IMAGES, "Pictures", home.join("Pictures")), (I_VIDEO, "Videos", home.join("Videos")), (I_DOWNLOAD, "Downloads", home.join("Downloads"))]
        .into_iter()
        .filter(|(_, _, p)| p.is_dir())
        .collect()
}

#[cfg(windows)]
fn drives() -> Vec<PathBuf> {
    (b'A'..=b'Z').map(|c| PathBuf::from(format!("{}:\\", c as char))).filter(|p| p.is_dir()).collect()
}

#[cfg(not(windows))]
fn drives() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

impl Viewer {
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let heading = |ui: &mut egui::Ui, t: &str| {
                ui.add_space(6.0);
                ui.label(RichText::new(t).small().weak());
            };
            if !self.s.pinned.is_empty() {
                heading(ui, "PINNED");
                for p in self.s.pinned.clone() {
                    self.node(ui, acts, &p, I_PIN, &file_name(&p));
                }
            }
            heading(ui, "PLACES");
            for (icon, name, p) in places() {
                self.node(ui, acts, &p, icon, name);
            }
            heading(ui, "THIS PC");
            for d in drives() {
                let name = d.display().to_string().trim_end_matches('\\').to_owned();
                self.node(ui, acts, &d, I_DRIVE, &name);
            }
        });
    }

    /// One folder row; expanding it lists (and caches) its subfolders.
    fn node(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>, path: &Path, icon: &str, name: &str) {
        let id = ui.make_persistent_id(path);
        let current = self.folder.as_deref() == Some(path);
        let pinned = self.s.pinned.iter().any(|p| p == path);
        let mut pin_toggle = false;
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                let resp = ui.selectable_label(current, label(icon, name)).on_hover_text(path.display().to_string());
                if resp.clicked() {
                    acts.push(Action::Open(path.to_path_buf()));
                }
                resp.context_menu(|ui| {
                    if ui.button(label(I_FOLDER_OPEN, "Open")).clicked() {
                        acts.push(Action::Open(path.to_path_buf()));
                    }
                    if ui.button(label(I_FOLDER, "New folder here...")).clicked() {
                        acts.push(Action::NewFolder(path.to_path_buf()));
                    }
                    if ui.button(label(I_PIN, if pinned { "Unpin" } else { "Pin" })).clicked() {
                        pin_toggle = true;
                    }
                    if ui.button(label(I_FOLDER, "Show in Explorer")).clicked() {
                        crate::files::reveal(path);
                    }
                });
            })
            .body(|ui| {
                let kids = self.tree.entry(path.to_path_buf()).or_insert_with(|| subdirs(path)).clone();
                if kids.is_empty() {
                    ui.label(RichText::new("No folders").small().weak());
                }
                for k in kids {
                    self.node(ui, acts, &k, I_FOLDER, &file_name(&k));
                }
            });
        if pin_toggle {
            if pinned {
                self.s.pinned.retain(|p| p != path);
            } else {
                self.s.pinned.push(path.to_path_buf());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn subdirs_skips_hidden_and_files() {
        let dir = std::env::temp_dir().join("glance-sidebar-test");
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["b", "A", ".git", "$junk"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("file.jpg"), b"x").unwrap();
        assert_eq!(super::subdirs(&dir), [dir.join("A"), dir.join("b")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
