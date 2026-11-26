//! The home page: what you see with nothing open.
use super::*;

/// Lays out `add` as a row centered in the available width. egui lays out in one pass, so the
/// row's width from the previous frame is used to center it (one extra frame on first show).
fn centered_row(ui: &mut egui::Ui, salt: &str, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.scope_id().with(salt);
    let width = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0);
    let pad = ((ui.available_width() - width) / 2.0).max(0.0);
    let row = ui.horizontal(|ui| {
        ui.add_space(pad);
        let inner = ui.horizontal(add);
        inner.response.rect.width()
    });
    if (row.inner - width).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, row.inner));
        ui.ctx().request_discard("center home row");
    }
}

/// A clickable card: a rounded panel that lights up under the mouse.
fn card(ui: &mut egui::Ui, size: Vec2) -> (Rect, egui::Response) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let v = ui.visuals();
    let fill = if resp.hovered() { v.widgets.hovered.weak_bg_fill } else { v.faint_bg_color };
    ui.painter().rect_filled(rect, 6.0, fill);
    if resp.hovered() {
        ui.painter().rect_stroke(rect, 6.0, egui::Stroke::new(1.0, accent().gamma_multiply(0.7)), egui::StrokeKind::Inside);
    }
    (rect, resp)
}

fn section_title(ui: &mut egui::Ui, title: &str, clear: Option<&str>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong());
        if let Some(tip) = clear {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = egui::Button::new(RichText::new("Clear").small().weak()).frame(false);
                clicked = ui.add(b).on_hover_text(tip).clicked();
            });
        }
    });
    ui.add_space(4.0);
    clicked
}

impl Viewer {
    pub(super) fn home(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
        let ctx = ui.ctx().clone();
        let recent: Vec<PathBuf> = self.s.recent.iter().filter(|p| p.is_dir()).take(5).cloned().collect();
        let viewed: Vec<PathBuf> = self.s.recent_files.iter().filter(|p| p.is_file()).take(6).cloned().collect();
        // Thumbnails for Recently viewed (usually straight from the cache on disk).
        let missing: Vec<PathBuf> = viewed.iter().filter(|p| !self.thumbs.contains_key(*p)).cloned().collect();
        if !missing.is_empty() && self.thumb_jobs.is_empty() {
            self.thumb_jobs.push(decode::spawn_thumbs(&ctx, missing, self.thumb_dir()));
        }
        if self.app_icon.is_none() {
            let img = image::load_from_memory(include_bytes!("../assets/icon.png")).map(|i| i.into_rgba8()).ok();
            self.app_icon = img.map(|i| {
                let ci = egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw());
                ctx.load_texture("app_icon", ci, egui::TextureOptions::LINEAR)
            });
        }

        let width = (ui.available_width() - 48.0).clamp(280.0, 600.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // Center the column vertically using its height from the last frame.
            let id = ui.scope_id().with("home_height");
            let last = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0);
            let top = ((ui.available_height() - last) / 2.0).max(24.0);
            let left = ((ui.available_width() - width) / 2.0).max(0.0);
            let col = Rect::from_min_size(ui.min_rect().min + vec2(left, top), vec2(width, f32::INFINITY));
            let inner = ui.scope_builder(egui::UiBuilder::new().max_rect(col).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                self.home_column(ui, acts, &recent, &viewed);
            });
            let h = inner.response.rect.height();
            ui.allocate_space(vec2(0.0, top + h + 24.0 - ui.min_rect().height()).max(Vec2::ZERO));
            if (h - last).abs() > 0.5 {
                ui.data_mut(|d| d.insert_temp(id, h));
                ui.ctx().request_discard("center home");
            }
        });
    }

    fn home_column(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Action>, recent: &[PathBuf], viewed: &[PathBuf]) {
        let width = ui.available_width();
        let v = ui.visuals().clone();

        // Header: icon and name.
        ui.horizontal(|ui| {
            if let Some(icon) = &self.app_icon {
                ui.image((icon.id(), vec2(44.0, 44.0)));
            }
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.add_space(3.0);
                ui.label(RichText::new("Glance").size(22.0).strong());
                ui.label(RichText::new("Photo and Video Viewer").weak());
            });
        });
        ui.add_space(18.0);

        // Drop zone with the two open buttons.
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let (zone, _) = ui.allocate_exact_size(vec2(width, 168.0), Sense::hover());
        ui.painter().rect_filled(zone, 8.0, v.faint_bg_color);
        let edge = if hovering { accent() } else { v.widgets.noninteractive.bg_stroke.color };
        let r = zone.shrink(1.0);
        let dash = |a: Pos2, b: Pos2| egui::Shape::dashed_line(&[a, b], egui::Stroke::new(1.0, edge), 6.0, 4.0);
        for (a, b) in [(r.left_top(), r.right_top()), (r.right_top(), r.right_bottom()), (r.right_bottom(), r.left_bottom()), (r.left_bottom(), r.left_top())] {
            ui.painter().extend(dash(a + (b - a).normalized() * 8.0, b - (b - a).normalized() * 8.0));
        }
        let mut zone_ui = ui.new_child(egui::UiBuilder::new().max_rect(zone.shrink(16.0)).layout(egui::Layout::top_down(egui::Align::Center)));
        zone_ui.add_space(4.0);
        zone_ui.label(RichText::new(I_IMAGES).size(28.0).color(if hovering { accent() } else { v.weak_text_color() }));
        zone_ui.add_space(4.0);
        zone_ui.label(if hovering { "Drop to open" } else { "Drop a folder or photo here" });
        zone_ui.add_space(10.0);
        centered_row(&mut zone_ui, "open_buttons", |ui| {
            let primary = egui::Button::new(RichText::new(label(I_FOLDER_OPEN, "Open folder")).color(Color32::WHITE))
                .fill(accent())
                .min_size(vec2(0.0, 28.0));
            if ui.add(primary).on_hover_text("Ctrl+Shift+O").clicked() {
                acts.push(Action::OpenFolder);
            }
            if ui.add(egui::Button::new(label(I_PICTURE, "Open file")).min_size(vec2(0.0, 28.0))).on_hover_text("Ctrl+O").clicked() {
                acts.push(Action::OpenFile);
            }
        });
        zone_ui.add_space(6.0);
        zone_ui.label(RichText::new("Ctrl+Shift+O  ·  Ctrl+O").small().weak());
        // Bottom-right corner: send photos from a phone.
        let corner = Rect::from_min_max(zone.right_bottom() - vec2(300.0, 34.0), zone.right_bottom() - vec2(12.0, 6.0));
        let mut phone_ui = ui.new_child(egui::UiBuilder::new().max_rect(corner).layout(egui::Layout::right_to_left(egui::Align::Center)));
        let phone = egui::Button::new(RichText::new(label(I_DOWNLOAD, "Upload from your phone")).small().color(accent())).frame(false);
        if phone_ui.add(phone).on_hover_text("Shows a QR code to scan with your phone").clicked() {
            acts.push(Action::Phone);
        }
        phone_ui.label(RichText::new("Photos on your phone?").small().weak());
        ui.add_space(22.0);

        // Places: one click into the usual folders.
        let places = sidebar::places();
        if !places.is_empty() {
            section_title(ui, "Places", None);
            let gap = 8.0;
            let n = places.len() as f32;
            let w = (width - gap * (n - 1.0)) / n;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (icon, name, p) in places {
                    let (rect, resp) = card(ui, vec2(w, 62.0));
                    let painter = ui.painter();
                    painter.text(rect.center() - vec2(0.0, 9.0), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(18.0), accent());
                    painter.text(rect.center() + vec2(0.0, 14.0), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(12.5), v.text_color());
                    if resp.on_hover_text(p.display().to_string()).clicked() {
                        acts.push(Action::Open(p));
                    }
                }
            });
            ui.add_space(22.0);
        }

        // Recent folders: one row each, with a remove button on hover.
        if !recent.is_empty() {
            if section_title(ui, "Recent folders", Some("Only clears this list; the folders aren't touched")) {
                self.s.recent.clear();
            }
            for p in recent {
                let (rect, resp) = card(ui, vec2(width, 40.0));
                let painter = ui.painter_at(rect);
                painter.text(rect.left_center() + vec2(14.0, 0.0), egui::Align2::LEFT_CENTER, I_FOLDER, egui::FontId::proportional(16.0), accent());
                let name = truncated(&painter, &file_name(p), width * 0.35, 13.5, v.strong_text_color());
                let name_w = name.size().x;
                painter.galley(rect.left_center() + vec2(40.0, -name.size().y / 2.0), name, v.strong_text_color());
                let parent = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
                let path = truncated(&painter, &parent, rect.width() - name_w - 100.0, 12.0, v.weak_text_color());
                painter.galley(rect.left_center() + vec2(52.0 + name_w, -path.size().y / 2.0), path, v.weak_text_color());
                // Remove from the list (shown on hover).
                let x = Rect::from_center_size(rect.right_center() - vec2(22.0, 0.0), vec2(24.0, 24.0));
                let x_resp = ui.interact(x, ui.scope_id().with(("rm", p)), Sense::click());
                if resp.hovered() || x_resp.hovered() {
                    if x_resp.hovered() {
                        ui.painter().rect_filled(x, 4.0, v.widgets.active.weak_bg_fill);
                    }
                    ui.painter().text(x.center(), egui::Align2::CENTER_CENTER, I_CROSS, egui::FontId::proportional(10.0), v.text_color());
                }
                if x_resp.on_hover_text("Remove from list").clicked() {
                    self.s.recent.retain(|r| r != p);
                } else if resp.clicked() {
                    acts.push(Action::Open(p.clone()));
                }
                ui.add_space(2.0);
            }
            ui.add_space(20.0);
        }

        // Recently viewed: small thumbnails.
        if !viewed.is_empty() {
            if section_title(ui, "Recently viewed", Some("Only clears this list")) {
                self.s.recent_files.clear();
            }
            let gap = 8.0;
            let n = 6.0;
            let w = (width - gap * (n - 1.0)) / n;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for p in viewed {
                    let info = TileInfo { video: video::is_video(p), cover: true, ..Default::default() };
                    let resp = thumb_tile(ui, vec2(w, w * 0.75), self.thumbs.get(p), info);
                    if resp.on_hover_text(file_name(p)).clicked() {
                        acts.push(Action::Open(p.clone()));
                    }
                }
            });
        }
    }
}
