//! Extra windows: keyboard shortcuts, Settings, and the picture-in-picture video window.
use super::*;

const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    ("Anywhere", &[
        ("Ctrl+O / Ctrl+Shift+O", "Open file / folder"),
        ("Ctrl+Shift+N", "New folder"),
        ("Ctrl+B", "Sidebar"),
        ("Ctrl+F", "Search (grid)"),
        ("Ctrl+,", "Settings"),
        ("F1 or ?", "This list"),
        ("F / F11", "Fullscreen"),
        ("Esc", "Back"),
    ]),
    ("Grid", &[
        ("Arrows, Home", "Move around"),
        ("Enter", "Open"),
        ("Space / Ctrl+click", "Select"),
        ("Drag on empty space", "Select with a rectangle"),
        ("Drag a tile out", "Copy to another app"),
        ("Ctrl+A", "Select all"),
        ("F2", "Rename (several: by pattern)"),
        ("Ctrl+scroll", "Tile size"),
    ]),
    ("Photos", &[
        ("← → / swipe", "Previous / next"),
        ("PgUp / PgDn", "Back / forward 10"),
        ("Home", "First photo"),
        ("Scroll, + / -", "Zoom"),
        ("Z", "100%"),
        ("Ctrl+0", "Fit"),
        ("Tab", "Photo only"),
        ("I", "Info panel"),
        ("L", "Loupe"),
        ("R / Shift+R", "Rotate"),
        ("H / V", "Flip"),
        ("E", "Crop and edit"),
        ("Space", "Slideshow"),
    ]),
    ("Videos", &[
        ("Space / K", "Play / pause"),
        ("J / L", "Back / forward 10 seconds"),
        ("[ and ]", "Trim start / end"),
        ("S", "Save frame as picture"),
        ("P", "Picture in picture"),
        ("M", "Mute"),
        ("↑ / ↓", "Volume"),
        ("< / >", "Slower / faster"),
        ("Double-click", "Fullscreen"),
    ]),
    ("Files", &[
        ("1-5 / 0", "Stars / clear"),
        ("Shift+F", "Favorite"),
        ("Alt+1-5", "Color label"),
        ("C / M", "Copy / move to folder"),
        ("Del", "Recycle Bin (with Undo)"),
        ("Ctrl+C / Ctrl+Shift+C", "Copy image / path"),
    ]),
];

impl Viewer {
    pub(super) fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_keys {
            return;
        }
        let resp = egui::Modal::new(egui::Id::unique("keys")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(RichText::new(label(I_INFO, "Keyboard shortcuts")).strong());
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.7).show(ui, |ui| {
                ui.columns(2, |cols| {
                    for (i, (group, keys)) in SHORTCUTS.iter().enumerate() {
                        let ui = &mut cols[usize::from(i >= 2)];
                        ui.add_space(6.0);
                        ui.label(RichText::new(*group).weak());
                        egui::Grid::new(*group).num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
                            for (k, what) in keys.iter() {
                                ui.label(RichText::new(*k).monospace().small());
                                ui.label(*what);
                                ui.end_row();
                            }
                        });
                    }
                });
            });
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                self.show_keys = false;
            }
        });
        if resp.should_close() {
            self.show_keys = false;
        }
    }

    pub(super) fn settings_window(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        egui::Window::new(label(I_SETTINGS, "Settings"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                let heading = |ui: &mut egui::Ui, t: &str| {
                    ui.add_space(8.0);
                    ui.label(RichText::new(t).strong());
                };
                heading(ui, "Look");
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    for (t, name) in [(ThemePref::System, "Same as Windows"), (ThemePref::Light, "Light"), (ThemePref::Dark, "Dark")] {
                        if ui.radio(self.s.theme == t, name).clicked() {
                            acts.push(Action::SetTheme(t));
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Accent");
                    for (name, rgb) in ACCENTS {
                        let c = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                        let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                        ui.painter().circle_filled(r.center(), 8.0, c);
                        if self.s.accent == rgb {
                            ui.painter().circle_stroke(r.center(), 10.0, egui::Stroke::new(1.5, ui.visuals().strong_text_color()));
                        }
                        if resp.on_hover_text(name).clicked() {
                            self.s.accent = rgb;
                            set_accent(rgb);
                            apply_theme(ui.ctx(), self.s.theme);
                        }
                    }
                });
                let mut glass = self.s.glass;
                if ui.checkbox(&mut glass, "Glass (see-through, blurs what's behind the window)").changed() {
                    acts.push(Action::Glass);
                }
                ui.horizontal(|ui| {
                    ui.label("Behind photos");
                    for (b, name) in [(Backdrop::Black, "Black"), (Backdrop::Gray, "Gray"), (Backdrop::Blur, "Blurred")] {
                        ui.radio_value(&mut self.s.backdrop, b, name);
                    }
                });

                heading(ui, "Browsing");
                let mut toggle = |ui: &mut egui::Ui, on: bool, text: &str, act: Action| {
                    let mut v = on;
                    if ui.checkbox(&mut v, text).changed() {
                        acts.push(act);
                    }
                };
                toggle(ui, self.s.sidebar, "Show the sidebar", Action::Sidebar);
                toggle(ui, self.s.group_dates, "Group by date", Action::GroupDates);
                toggle(ui, self.s.subfolders, "Include subfolders", Action::Subfolders);
                ui.checkbox(&mut self.s.square_tiles, "Square tiles");
                ui.horizontal(|ui| {
                    ui.label("Tile size");
                    ui.add(egui::Slider::new(&mut self.s.grid_size, 80.0..=320.0).show_value(false));
                });

                heading(ui, "Viewing");
                ui.horizontal(|ui| {
                    ui.label("Open photos at");
                    for (m, name) in [(FitMode::Fit, "Fit"), (FitMode::Fill, "Fill"), (FitMode::Actual, "100%")] {
                        ui.radio_value(&mut self.s.fit_mode, m, name);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Slideshow");
                    ui.add(egui::Slider::new(&mut self.s.slide_secs, 1.0..=15.0).suffix(" s").step_by(1.0));
                });
                ui.checkbox(&mut self.minimap, "Zoom map while zoomed in");

                heading(ui, "Videos");
                ui.checkbox(&mut self.s.resume, "Resume where I left off");
                ui.checkbox(&mut self.s.subtitles, "Show subtitles (.srt next to the video)");
                ui.checkbox(&mut self.s.video_loop, "Loop");

                heading(ui, "Default app");
                ui.label(RichText::new("Open photos and videos in Glance when you double-click them.").weak());
                ui.horizontal(|ui| {
                    if ui.button(label(I_CHECK, "Set up as default app...")).on_hover_text("Adds Glance to Windows' list of photo and video apps, then opens Default apps so you can pick it").clicked() {
                        match defaults::register() {
                            Ok(()) => {
                                defaults::open_settings();
                                self.toast("In Default apps, choose Glance for the file types you want");
                            }
                            Err(e) => self.toast(e),
                        }
                    }
                    if ui.button("Remove from Windows").on_hover_text("Takes Glance off Windows' list of photo and video apps").clicked() {
                        defaults::unregister();
                        self.toast("Glance removed from Windows' app list");
                    }
                });
                ui.label(RichText::new("Glance is registered at its current location. If you move glance.exe, set it up again.").weak().small());

                heading(ui, "Storage");
                let cache = self.thumb_dir();
                let used: u64 = cache.iter().flat_map(std::fs::read_dir).flatten().flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum();
                ui.horizontal(|ui| {
                    ui.label(format!("Thumbnail cache: {}", files::human_size(used)));
                    if ui.add_enabled(used > 0, egui::Button::new("Clear")).clicked() {
                        if let Some(c) = &cache {
                            let _ = std::fs::remove_dir_all(c); // only Glance's own cache folder
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.s.recent.is_empty(), egui::Button::new("Clear recent folders")).clicked() {
                        self.s.recent.clear();
                    }
                    if ui.add_enabled(!self.s.recent_files.is_empty(), egui::Button::new("Clear recently viewed")).clicked() {
                        self.s.recent_files.clear();
                    }
                    if ui.add_enabled(!self.s.video_pos.is_empty(), egui::Button::new("Forget video positions")).clicked() {
                        self.s.video_pos.clear();
                    }
                });
            });
        if !open {
            self.show_settings = false;
        }
    }

    /// The theme picker from the moon/sun in the corner: every colour, each in light and dark.
    pub(super) fn theme_menu(&self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        ui.set_min_width(230.0);
        ui.label(RichText::new("Theme").strong());
        if ui.radio(self.s.theme == ThemePref::System && self.s.tint.is_none(), "Same as Windows").clicked() {
            acts.push(Action::ColorTheme(None, ThemePref::System));
            ui.close();
        }
        ui.add_space(4.0);
        let colours = std::iter::once(("Gray", None)).chain(RAINBOW.iter().map(|(n, c)| (*n, Some(*c))));
        egui::Grid::new("theme_grid").num_columns(3).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
            for (name, c) in colours {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    let dot = c.map_or(Color32::from_gray(140), |[r, g, b]| Color32::from_rgb(r, g, b));
                    ui.painter().circle_filled(r.center(), 5.0, dot);
                    ui.label(name);
                });
                for (pref, base, ink, icon) in [(ThemePref::Light, 243, Color32::from_gray(40), I_SUN), (ThemePref::Dark, 26, Color32::from_gray(220), I_MOON)] {
                    let light = pref == ThemePref::Light;
                    let bg = c.map_or(Color32::from_gray(base), |c| tinted(Color32::from_gray(base), c, if light { 0.13 } else { 0.11 }));
                    let on = self.s.tint == c && self.s.theme == pref;
                    let (r, resp) = ui.allocate_exact_size(vec2(64.0, 24.0), Sense::click());
                    ui.painter().rect_filled(r, 4.0, bg);
                    let edge = if on { egui::Stroke::new(2.0, accent()) } else if resp.hovered() { egui::Stroke::new(1.0, ui.visuals().strong_text_color()) } else { egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color) };
                    ui.painter().rect_stroke(r, 4.0, edge, egui::StrokeKind::Inside);
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, format!("{icon} {}", if light { "Light" } else { "Dark" }), egui::FontId::proportional(12.0), ink);
                    if resp.on_hover_text(format!("{name}, {}", if light { "light" } else { "dark" })).clicked() {
                        acts.push(Action::ColorTheme(c, pref));
                        ui.close();
                    }
                }
                ui.end_row();
            }
        });
    }

    pub(super) fn receive_dir(&self) -> PathBuf {
        self.s.receive_dir.clone().unwrap_or_else(|| {
            let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
            home.join("Pictures").join("From phone")
        })
    }

    /// "Receive from phone": a QR code the phone scans to open an upload page. The server only
    /// runs while this window is open.
    pub(super) fn phone_window(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        // Each QR code only works for 10 seconds; the old picture is thrown away.
        if let Some(Ok(s)) = &mut self.phone {
            if s.made.elapsed() >= receive::QR_LIFE {
                s.rotate();
                self.phone_qr = None;
            }
        }
        let Some(server) = &self.phone else { return };
        ctx.request_repaint_after(Duration::from_secs(1));
        let mut open = true;
        let mut change_dir = false;
        let mut open_dir = None;
        egui::Window::new(label(I_DOWNLOAD, "Receive from phone"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(340.0)
            .show(ctx, |ui| match server {
                Err(e) => {
                    ui.label(format!("Couldn't start receiving: {e}"));
                }
                Ok(s) => {
                    let url = s.url();
                    let qr = self.phone_qr.get_or_insert_with(|| {
                        let img = receive::qr_image(&url).unwrap_or_else(|| egui::ColorImage::new([1, 1], vec![Color32::WHITE]));
                        ctx.load_texture("phone-qr", img, egui::TextureOptions::NEAREST)
                    });
                    ui.label("Scan this with your phone's camera, then pick the photos and videos to send.");
                    ui.add_space(8.0);
                    let code = ui.vertical_centered(|ui| ui.image((qr.id(), vec2(220.0, 220.0)))).inner.rect;
                    // The Glance icon in the middle, on a white square so the code around it stays clean.
                    if let Some(icon) = &self.app_icon {
                        let side = code.width() * 0.2;
                        ui.painter().rect_filled(Rect::from_center_size(code.center(), Vec2::splat(side + 6.0)), 6.0, Color32::WHITE);
                        ui.painter().image(icon.id(), Rect::from_center_size(code.center(), Vec2::splat(side)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    let left = receive::QR_LIFE.saturating_sub(s.made.elapsed()).as_secs() + 1;
                    ui.vertical_centered(|ui| ui.label(RichText::new(format!("New code in {left} s")).small().weak()));
                    if s.visits.load(std::sync::atomic::Ordering::Relaxed) > 0 {
                        ui.vertical_centered(|ui| ui.label(RichText::new(label(I_CHECK, "Phone connected")).color(accent())));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Saving to");
                        ui.label(RichText::new(s.dir.display().to_string()).strong()).on_hover_text(s.dir.display().to_string());
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Change...").clicked() {
                            change_dir = true;
                        }
                        if ui.button(label(I_FOLDER_OPEN, "Open in Glance")).clicked() {
                            open_dir = Some(s.dir.clone());
                        }
                    });
                    if !self.received.is_empty() {
                        ui.add_space(6.0);
                        ui.label(format!("Received {} this time", plural(self.received.len(), "file")));
                        for p in self.received.iter().rev().take(5) {
                            ui.label(RichText::new(file_name(p)).small().weak());
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(
                            "Your phone needs to be on the same Wi-Fi as this PC. The first time, Windows may ask \
                             to let Glance through the firewall: allow it on private networks. Only someone with \
                             this code can send files, and only while this window is open.",
                        )
                        .small()
                        .weak(),
                    );
                }
            });
        if change_dir {
            if let Some(d) = rfd::FileDialog::new().set_title("Save photos from your phone in").pick_folder() {
                self.s.receive_dir = Some(d);
                self.phone = Some(receive::start(self.receive_dir(), ctx.clone()));
                self.phone_qr = None;
            }
        }
        if let Some(d) = open_dir {
            acts.push(Action::Open(d));
        }
        if !open {
            acts.push(Action::Phone);
        }
    }

    /// The small always-on-top video window. Closing it stops the video (and remembers where).
    pub(super) fn pip_window(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let Some((path, player)) = &mut self.pip else { return };
        let mut close = false;
        let mut back = false;
        let title = format!("{} - {APP}", file_name(path));
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([420.0, 236.0])
            .with_min_inner_size([200.0, 112.0])
            .with_always_on_top();
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("pip"), builder, |ui, _| {
            let ctx = ui.ctx().clone();
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(ui, |ui| {
                let rect = ui.max_rect();
                let resp = ui.allocate_rect(rect, Sense::click());
                if let Some(tex) = player.frame(&ctx) {
                    let (paint, _, _) = layout(rect, tex.size_vec2(), 0, 1.0, Vec2::ZERO);
                    paint_texture(ui.painter(), tex, paint, UV, 0, false, false, Color32::WHITE);
                }
                if resp.clicked() {
                    player.toggle();
                }
                if resp.double_clicked() {
                    back = true;
                }
                resp.on_hover_text("Click: pause / play · double-click: back to Glance");
            });
            if player.playing() {
                ctx.request_repaint();
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if back {
            acts.push(Action::Pip); // hands it back to the main window
        } else if close {
            if let Some((p, pl)) = self.pip.take() {
                self.remember_position(p, pl.time(), pl.duration());
            }
        }
    }
}
