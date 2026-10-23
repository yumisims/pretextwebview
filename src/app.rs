//! egui shell: contact map, file loading, and the edit controls.

use std::sync::mpsc;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use web_time::{Duration, Instant};

use crate::colormap::NAMES;
use crate::model::{Mode, Session, SortMode, BACKGROUNDS};
use crate::pretext::{MapLoader, Pump};

#[derive(Clone, Copy, PartialEq, Eq)]
struct TexKey {
    revision: u64,
    colormap: usize,
    gamma: [u32; 3],
}

struct Detail {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    key: TexKey,
    window: [i32; 4],
    tex: egui::TextureHandle,
}

struct Press {
    pixel: u32,
    origin: Pos2,
    moved: bool,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
enum Msg {
    Pretext { name: String, bytes: Vec<u8> },
    Json(String),
    Agp(String),
    Note(String),
    Done,
}

pub struct PretextApp {
    session: Session,
    status: String,
    show_panel: bool,
    selection: Vec<u64>,
    press: Option<Press>,
    grab_armed: bool,
    grab_dest: Option<u32>,
    panning: bool,
    hover_pixel: Option<u32>,
    hover_world: Option<Vec2>,
    loader: Option<MapLoader>,
    saw_tile_preview: bool,
    preview_at: Instant,
    map_tex: Option<egui::TextureHandle>,
    overview_key: Option<TexKey>,
    detail: Option<Detail>,
    detail_at: Instant,
    seq_query: String,
    menu_theme: MenuTheme,
    events: mpsc::Receiver<Msg>,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    events_tx: mpsc::Sender<Msg>,
    dialog_open: bool,
}

impl PretextApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let (events_tx, events) = mpsc::channel();
        let mut app = Self {
            session: Session::demo(),
            status: "Demo map. Load a .pretext file, or drop one on the view. Esc hides the panel.".to_string(),
            show_panel: true,
            selection: Vec::new(),
            press: None,
            grab_armed: false,
            grab_dest: None,
            panning: false,
            hover_pixel: None,
            hover_world: None,
            loader: None,
            saw_tile_preview: false,
            preview_at: Instant::now(),
            map_tex: None,
            overview_key: None,
            detail: None,
            detail_at: Instant::now(),
            seq_query: String::new(),
            menu_theme: MenuTheme::Dark,
            events,
            events_tx,
            dialog_open: false,
        };
        #[cfg(not(target_arch = "wasm32"))]
        app.open_demo_file();
        #[cfg(target_arch = "wasm32")]
        app.fetch_bundled(cc.egui_ctx.clone());
        app
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_demo_file(&mut self) {
        for path in ["demo.pretext", "src/demo.pretext"] {
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            if bytes.len() >= 4 && &bytes[..4] == b"pstm" {
                self.begin_pretext(path.to_string(), bytes);
                return;
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn fetch_bundled(&mut self, ctx: egui::Context) {
        let tx = self.events_tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some((name, bytes)) = fetch_bundled_bytes().await {
                let _ = tx.send(Msg::Pretext { name, bytes });
                ctx.request_repaint();
            }
        });
    }

    fn poll(&mut self) {
        while let Ok(msg) = self.events.try_recv() {
            self.dialog_open = false;
            match msg {
                Msg::Pretext { name, bytes } => self.begin_pretext(name, bytes),
                Msg::Json(text) => self.load_json_text(&text),
                Msg::Agp(text) => self.load_agp_text(&text),
                Msg::Note(text) => self.status = text,
                Msg::Done => {}
            }
        }
    }

    fn take_drops(&mut self, ctx: &egui::Context) {
        let drops = ctx.input(|input| input.raw.dropped_files.clone());
        for file in drops {
            if let Some(bytes) = file.bytes {
                let name = if file.name.is_empty() {
                    "dropped.pretext".to_string()
                } else {
                    file.name
                };
                self.ingest(&name, bytes.to_vec());
                continue;
            }
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(path) = file.path {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let name = path
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "dropped.pretext".to_string());
                        self.ingest(&name, bytes);
                    }
                    Err(err) => self.status = format!("could not read file: {err}"),
                }
            }
        }
    }

    fn ingest(&mut self, name: &str, bytes: Vec<u8>) {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".json") {
            self.load_json_text(&String::from_utf8_lossy(&bytes));
        } else if lower.ends_with(".agp") {
            self.load_agp_text(&String::from_utf8_lossy(&bytes));
        } else {
            self.begin_pretext(name.to_string(), bytes);
        }
    }

    fn begin_pretext(&mut self, name: String, bytes: Vec<u8>) {
        let loader = match MapLoader::open(name.clone(), bytes) {
            Ok(loader) => loader,
            Err(err) => {
                self.status = err;
                return;
            }
        };
        let preview = loader.preview();
        self.status = self.session.install_map(preview, name);
        self.selection.clear();
        self.press = None;
        self.grab_armed = false;
        self.grab_dest = None;
        self.detail = None;
        self.overview_key = None;
        self.saw_tile_preview = false;
        self.loader = Some(loader);
    }

    fn pump_loader(&mut self, ctx: &egui::Context) {
        let Some(loader) = self.loader.as_mut() else {
            return;
        };
        let pumped = loader.pump();
        let progress = loader.progress();
        match pumped {
            Ok(Pump::Loading) => {
                self.status = progress;
                if !self.saw_tile_preview || self.preview_at.elapsed() >= Duration::from_millis(80) {
                    self.session.bump();
                    self.preview_at = Instant::now();
                    self.saw_tile_preview = true;
                }
                ctx.request_repaint();
            }
            Ok(Pump::Ready) => {
                let loader = self.loader.take().expect("loader still open");
                self.status = match loader.finish() {
                    Ok(loaded) => self.session.complete_map(loaded),
                    Err(err) => err,
                };
            }
            Err(err) => {
                self.loader = None;
                self.status = err;
            }
        }
    }

    fn load_json_text(&mut self, text: &str) {
        match self.session.load_json(text) {
            Ok(()) => {
                self.selection.clear();
                self.detail = None;
                self.overview_key = None;
                self.status = "Loaded JSON state.".to_string();
            }
            Err(err) => self.status = err,
        }
    }

    fn load_agp_text(&mut self, text: &str) {
        match self.session.load_agp(text) {
            Ok(n) => self.status = format!("AGP placed {n} parts."),
            Err(err) => self.status = err,
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let pressed = ctx.input(|input| {
            let plain = !input.modifiers.command && !input.modifiers.alt;
            let key = |key| plain && input.key_pressed(key);
            (
                input.key_pressed(egui::Key::Escape),
                input.modifiers.command && input.key_pressed(egui::Key::Z) && !input.modifiers.shift,
                input.modifiers.command && input.key_pressed(egui::Key::Z) && input.modifiers.shift,
                key(egui::Key::I),
                key(egui::Key::E),
                key(egui::Key::X),
                key(egui::Key::Num3),
                key(egui::Key::Num5),
                key(egui::Key::C),
                key(egui::Key::G),
                key(egui::Key::R),
                key(egui::Key::T),
                key(egui::Key::N),
                key(egui::Key::V),
            )
        });
        let (escape, undo, redo, invert, edit_key, extension, tel3, tel5, coverage, gap, repeat, tooltip_or_tel, names, cut) =
            pressed;
        if escape {
            self.show_panel = !self.show_panel;
        }
        if undo {
            self.session.undo_edit();
        }
        if redo {
            self.session.redo_edit();
        }
        if invert {
            self.invert_selection();
        }
        if edit_key {
            self.toggle_edit_mode();
        }
        if extension && (self.session.mode == Mode::Normal || self.session.mode == Mode::Extension) {
            self.session.mode = if self.session.mode == Mode::Extension {
                Mode::Normal
            } else {
                Mode::Extension
            };
        }
        if tel3 {
            self.toggle_track("3p_telomere");
        }
        if tel5 {
            self.toggle_track("5p_telomere");
        }
        if self.session.mode == Mode::Extension {
            if coverage {
                self.toggle_track("coverage");
            }
            if gap {
                self.toggle_track("gap");
            }
            if repeat {
                self.toggle_track("repeat_density");
            }
            if tooltip_or_tel {
                self.toggle_track("telomere");
            }
        } else if tooltip_or_tel {
            self.session.tooltip_on = !self.session.tooltip_on;
        }
        if names {
            self.session.labels_on = !self.session.labels_on;
        }
        if cut && self.session.mode == Mode::Edit {
            self.cut_at_hover();
        }
    }

    fn cut_at_hover(&mut self) {
        let Some(pixel) = self.hover_pixel else {
            self.status = "Move the pointer onto the contig, then press V.".to_string();
            return;
        };
        if self.session.split_pixel(pixel) {
            self.status = format!("Cut the contig at pixel {pixel}.");
        } else {
            self.status = "V cuts inside a contig. Move the pointer off the boundary.".to_string();
        }
    }

    fn toggle_edit_mode(&mut self) {
        self.session.mode = match self.session.mode {
            Mode::Edit => Mode::Normal,
            Mode::Normal | Mode::SelectSort | Mode::Meta => Mode::Edit,
            other => other,
        };
        self.status = if self.session.mode == Mode::Edit {
            "Edit mode. Drag a block to move it. V cuts the contig under the pointer. I inverts.".to_string()
        } else {
            "Left edit mode.".to_string()
        };
    }

    fn toggle_track(&mut self, name: &str) {
        let Some(track) = self.session.tracks.iter_mut().find(|track| track.name == name) else {
            return;
        };
        track.on = !track.on;
        let state = if track.on { "on" } else { "off" };
        self.status = format!("{name} {state}");
    }

    fn invert_selection(&mut self) {
        match self.session.invert_uids(&self.selection) {
            Ok(()) => self.status = "Inverted the selection.".to_string(),
            Err(err) => self.status = err.to_string(),
        }
    }

    fn uids_covering(&self, mut start: u32, mut end: u32) -> Vec<u64> {
        if end < start {
            std::mem::swap(&mut start, &mut end);
        }
        let end = end.saturating_add(1);
        let mut acc = 0u32;
        let mut uids = Vec::new();
        for frag in &self.session.fragments {
            let next = acc + frag.pixels;
            if next > start && acc < end {
                uids.push(frag.uid);
            }
            acc = next;
        }
        uids
    }

    fn selection_span(&self) -> Option<(u32, u32)> {
        let mut acc = 0u32;
        let mut start = None;
        let mut end = None;
        for frag in &self.session.fragments {
            let next = acc + frag.pixels;
            if self.selection.contains(&frag.uid) {
                if start.is_none() {
                    start = Some(acc);
                }
                end = Some(next);
            }
            acc = next;
        }
        Some((start?, end?))
    }

    fn hover_fragment_span(&self) -> Option<(u32, u32)> {
        if self.session.mode != Mode::Edit {
            return None;
        }
        let pixel = self.hover_pixel?;
        let index = self.session.fragment_at(pixel)?;
        let start = self.session.prefix(index);
        Some((start, start + self.session.fragments[index].pixels))
    }

    fn options(&mut self, ui: &mut egui::Ui) {
        let row_h = 30.0;
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        ui.spacing_mut().button_padding = Vec2::new(6.0, 4.0);

        ui.horizontal(|ui| {
            let esc_w = 56.0;
            let name_w = (ui.available_width() - ui.spacing().item_spacing.x - esc_w).max(40.0);
            ui.add_sized(
                Vec2::new(name_w, 22.0),
                egui::Label::new(egui::RichText::new(self.session.file_name.as_str())).truncate(),
            );
            if ui.add_sized(Vec2::new(esc_w, 22.0), egui::Button::new("Esc")).clicked() {
                self.show_panel = false;
            }
        });

        let themes = [
            MenuTheme::Dark,
            MenuTheme::Black,
            MenuTheme::White,
            MenuTheme::Red,
            MenuTheme::Blue,
        ];
        ui.columns(themes.len(), |cols| {
            for (col, theme) in cols.iter_mut().zip(themes) {
                if menu_choice(col, theme.label(), self.menu_theme == theme, 26.0).clicked() {
                    self.menu_theme = theme;
                }
            }
        });

        ui.columns(3, |cols| {
            let load = egui::Button::new("Load Map").min_size(Vec2::new(cols[0].available_width(), row_h));
            if cols[0].add_enabled(!self.dialog_open, load).clicked() {
                self.pick_pretext(cols[0].ctx().clone());
            }
            if menu_button(&mut cols[1], "Save State", row_h).clicked() {
                self.save_text(cols[1].ctx().clone(), "state.json", "json", self.session.save_json());
            }
            if menu_button(&mut cols[2], "Load State", row_h).clicked() {
                self.pick_text(cols[2].ctx().clone(), "JSON", "json", Kind::Json);
            }
        });

        let (wide, narrow) = split_widths(ui, 2.0, 1.0);
        ui.horizontal(|ui| {
            if menu_sized(ui, "Generate AGP", wide, row_h).clicked() {
                self.save_text(ui.ctx().clone(), "map.agp", "agp", self.session.to_agp());
            }
            if menu_sized(ui, "User Profile", narrow, row_h).clicked() {
                self.session.reset_profile();
                self.status = "Restored the default profile.".to_string();
            }
        });

        ui.columns(3, |cols| {
            if menu_button(&mut cols[0], "About", row_h).clicked() {
                self.status = "pretextwebview".to_string();
            }
            if menu_button(&mut cols[1], "Clipboard Editor", row_h).clicked() {
                cols[1].ctx().copy_text(self.session.edits_text());
                self.status = "Copied the edit list.".to_string();
            }
            if menu_button(&mut cols[2], "Clear Cache", row_h).clicked() {
                self.detail = None;
                self.overview_key = None;
                self.status = "Cleared the image cache.".to_string();
            }
        });

        ui.columns(2, |cols| {
            if self.purple_button(&mut cols[0], "Pixel Cut").clicked() {
                let only = if self.selection.is_empty() {
                    None
                } else {
                    Some(self.selection.as_slice())
                };
                let n = self.session.pixel_cut(only);
                self.status = format!("Pixel cut made {n} cuts.");
            }
            if self.purple_button(&mut cols[1], "Pixel Sort").clicked() {
                self.pixel_sort();
            }
        });

        let modes = [
            (Mode::Waypoint, "Waypoint Edit Mode"),
            (Mode::Scaffold, "Scaffold Edit Mode"),
            (Mode::Edit, "Edit Mode"),
            (Mode::SelectSort, "Select sort area"),
            (Mode::Meta, "MetaData Tag Mode"),
            (Mode::Extension, "Extension Mode"),
        ];
        for pair in modes.chunks(2) {
            ui.columns(2, |cols| {
                for (col, (mode, label)) in cols.iter_mut().zip(pair) {
                    if menu_choice(col, label, self.session.mode == *mode, row_h).clicked() {
                        let entering_edit = *mode == Mode::Edit && self.session.mode != Mode::Edit;
                        self.session.mode = if self.session.mode == *mode { Mode::Normal } else { *mode };
                        if entering_edit {
                            self.status = "Edit mode. Drag a block to move it. V cuts the contig under the pointer. I inverts.".to_string();
                        }
                    }
                }
            });
        }

        ui.columns(2, |cols| {
            menu_check(&mut cols[0], &mut self.session.waypoints_always, "Waypoints Always Visible");
            menu_check(&mut cols[1], &mut self.session.scaffolds_always, "Scaffolds Always Visible");
        });
        ui.columns(2, |cols| {
            menu_check(&mut cols[0], &mut self.session.labels_on, "Contig Name Labels");
            menu_check(&mut cols[1], &mut self.session.scale_on, "Scale Bars");
        });
        ui.columns(2, |cols| {
            menu_check(&mut cols[0], &mut self.session.grid_on, "Grid");
            menu_check(&mut cols[1], &mut self.session.id_bars_on, "ID Bars");
        });
        ui.columns(2, |cols| {
            menu_check(&mut cols[0], &mut self.session.tooltip_on, "Tool Tip");
            menu_check(&mut cols[1], &mut self.session.invert_mouse, "Invert Mouse Buttons");
        });

        let mut gamma_changed = false;
        gamma_changed |= gamma_slider(ui, "Gamma Min", &mut self.session.gamma[0]);
        gamma_changed |= gamma_slider(ui, "Gamma Mid", &mut self.session.gamma[1]);
        gamma_changed |= gamma_slider(ui, "Gamma Max", &mut self.session.gamma[2]);
        if gamma_changed {
            self.session.gamma[1] = self.session.gamma[1].clamp(self.session.gamma[0], self.session.gamma[2]);
            self.session.gamma[0] = self.session.gamma[0].min(self.session.gamma[2]);
        }
        if menu_button(ui, "Default Gamma", row_h).clicked() {
            self.session.gamma = [0.0, 0.5, 1.0];
            self.session.bump();
        }

        ui.add_space(4.0);
        egui::containers::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.make_persistent_id("colour-maps"),
            true,
        )
            .show_header(ui, |ui| {
                ui.label("Colour Maps");
            })
            .body(|ui| {
                for (index, name) in NAMES.iter().enumerate() {
                    ui.radio_value(&mut self.session.colormap, index, *name);
                }
                ui.separator();
                ui.label("Background");
                for (index, (name, _)) in BACKGROUNDS.iter().enumerate() {
                    ui.radio_value(&mut self.session.background, index, *name);
                }
            });

        ui.collapsing("Extensions", |ui| {
            for track in &mut self.session.tracks {
                ui.checkbox(&mut track.on, track.name.as_str());
            }
        });

        ui.collapsing("Sort", |ui| {
            for mode in [SortMode::UnionFind, SortMode::Fuse, SortMode::DeepFuse] {
                ui.radio_value(&mut self.session.sort_mode, mode, mode.label());
            }
            ui.add(egui::Slider::new(&mut self.session.link_threshold, 0.0..=1.0).text("link threshold"));
            ui.add(egui::Slider::new(&mut self.session.cut_threshold, 0.0..=1.0).text("cut threshold"));
        });

        let edit_count = self.session.edits.len();
        ui.collapsing(format!("Edits ({edit_count})"), |ui| {
            ui.columns(3, |cols| {
                if menu_button(&mut cols[0], "Undo", row_h).clicked() {
                    self.session.undo_edit();
                }
                if menu_button(&mut cols[1], "Redo", row_h).clicked() {
                    self.session.redo_edit();
                }
                if menu_button(&mut cols[2], "Erase", row_h).clicked() {
                    self.session.erase_all();
                    self.selection.clear();
                    self.status = "Erased edits.".to_string();
                }
            });
            ui.columns(3, |cols| {
                if menu_button(&mut cols[0], "Invert", row_h).clicked() {
                    self.invert_selection();
                }
                if menu_button(&mut cols[1], "Split", row_h).clicked() {
                    if let Some(pixel) = self.hover_pixel {
                        if self.session.split_pixel(pixel) {
                            self.status = format!("Split at pixel {pixel}.");
                        } else {
                            self.status = "The pointer is on a fragment edge.".to_string();
                        }
                    }
                }
                if menu_button(&mut cols[2], "Glue", row_h).clicked() {
                    let only = if self.selection.is_empty() {
                        None
                    } else {
                        Some(self.selection.as_slice())
                    };
                    let n = self.session.glue_adjacent(only);
                    self.status = format!("Glued {n} joins.");
                }
            });
            if menu_button(ui, "Load AGP", row_h).clicked() {
                self.pick_text(ui.ctx().clone(), "AGP", "agp", Kind::Agp);
            }
            if menu_button(ui, "Consolidate marked", row_h).clicked() {
                match self.session.consolidate(&self.selection) {
                    Ok(ids) => {
                        self.selection = ids;
                        self.status = "Moved the marked contigs together.".to_string();
                    }
                    Err(err) => self.status = err.to_string(),
                }
            }
            if self.session.edits.is_empty() {
                ui.label("No edits.");
            }
            for (index, edit) in self.session.edits.iter().enumerate() {
                ui.label(format!("{}. {}", index + 1, edit.summary));
            }
        });

        ui.collapsing("Waypoints", |ui| {
            if self.session.waypoints.is_empty() {
                ui.label("No waypoints.");
            }
            let mut jump = None;
            let mut remove = None;
            for (index, _) in self.session.waypoints.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.button(format!("Go {}", index + 1)).clicked() {
                        jump = Some(index);
                    }
                    if ui.button("Remove").clicked() {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = jump {
                self.session.jump_waypoint(index);
            }
            if let Some(index) = remove {
                self.session.waypoints.remove(index);
            }
        });

        ui.collapsing("Scaffolds", |ui| {
            if menu_button(ui, "New scaffold", row_h).clicked() {
                let next = self.session.fragments.iter().map(|frag| frag.scaff_id).max().unwrap_or(0) + 1;
                self.session.paint_scaff = next;
                self.status = format!("Scaffold {next}. Paint it in Scaffold Edit Mode.");
            }
            ui.add(egui::DragValue::new(&mut self.session.paint_scaff).range(0..=10_000u32).prefix("scaffold "));
        });

        ui.collapsing("Meta Data Tags", |ui| {
            ui.checkbox(&mut self.session.meta_always, "Meta labels");
            let names = self.session.tags.clone();
            let mut active = self.session.active_tag.min(names.len().saturating_sub(1));
            egui::ComboBox::from_label("Active tag")
                .selected_text(names.get(active).cloned().unwrap_or_default())
                .show_ui(ui, |ui| {
                    for (index, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut active, index, name);
                    }
                });
            self.session.active_tag = active;
            let mut grey_changed = false;
            for (index, name) in names.iter().enumerate() {
                if let Some(on) = self.session.grey_tags.get_mut(index) {
                    grey_changed |= ui.checkbox(on, format!("grey {name}")).changed();
                }
            }
            if grey_changed {
                self.session.bump();
            }
            if ui.button("Move tagged to end").clicked() {
                let bit = 1u64 << self.session.active_tag.min(63);
                let n = self.session.move_tags_to_end(bit);
                self.status = format!("Moved {n} tagged fragments to the end.");
            }
        });

        ui.collapsing("Sequences", |ui| {
            ui.text_edit_singleline(&mut self.seq_query);
            let query = self.seq_query.to_ascii_lowercase();
            let mut shown = 0usize;
            let mut focus = None;
            for (index, frag) in self.session.fragments.iter().enumerate() {
                if !query.is_empty() && !frag.name.to_ascii_lowercase().contains(&query) {
                    continue;
                }
                shown += 1;
                if shown > 200 {
                    ui.label("Showing the first 200 matches.");
                    break;
                }
                let marked = self.selection.contains(&frag.uid);
                if ui.selectable_label(marked, frag.name.as_str()).clicked() {
                    focus = Some(index);
                }
            }
            if shown == 0 {
                ui.label("No sequences.");
            }
            if let Some(index) = focus {
                self.session.focus_fragment(index);
            }
        });
    }

    fn purple_button(&self, ui: &mut egui::Ui, label: &str) -> egui::Response {
        ui.scope(|ui| {
            let widgets = &mut ui.style_mut().visuals.widgets;
            widgets.inactive.weak_bg_fill = Color32::from_rgb(153, 50, 204);
            widgets.inactive.fg_stroke.color = Color32::WHITE;
            widgets.hovered.weak_bg_fill = Color32::from_rgb(186, 85, 211);
            widgets.hovered.fg_stroke.color = Color32::WHITE;
            widgets.active.weak_bg_fill = Color32::from_rgb(128, 0, 128);
            widgets.active.fg_stroke.color = Color32::WHITE;
            ui.add_sized(Vec2::new(ui.available_width(), 32.0), egui::Button::new(label))
        })
        .inner
    }

    fn pixel_sort(&mut self) {
        let slots: Vec<usize> = self
            .session
            .fragments
            .iter()
            .enumerate()
            .filter(|(_, frag)| self.selection.contains(&frag.uid))
            .map(|(index, _)| index)
            .collect();
        let links = if self.session.mode == Mode::SelectSort && !slots.is_empty() {
            self.session.sort_fragments(Some(&slots))
        } else {
            self.session.sort_fragments(None)
        };
        self.status = format!("Pixel sort kept {links} links.");
    }

    fn pick_pretext(&mut self, ctx: egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            let Some(path) = rfd::FileDialog::new().add_filter("Pretext", &["pretext"]).pick_file() else {
                return;
            };
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "map.pretext".to_string());
                    self.begin_pretext(name, bytes);
                }
                Err(err) => self.status = format!("could not read file: {err}"),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.dialog_open = true;
            let tx = self.events_tx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let handle = rfd::AsyncFileDialog::new()
                    .add_filter("Pretext", &["pretext"])
                    .pick_file()
                    .await;
                if let Some(handle) = handle {
                    let name = handle.file_name();
                    let bytes = handle.read().await;
                    let _ = tx.send(Msg::Pretext { name, bytes });
                }
                let _ = tx.send(Msg::Done);
                ctx.request_repaint();
            });
        }
    }

    fn pick_text(&mut self, ctx: egui::Context, title: &str, ext: &str, kind: Kind) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            let Some(path) = rfd::FileDialog::new().add_filter(title, &[ext]).pick_file() else {
                return;
            };
            match std::fs::read_to_string(&path) {
                Ok(text) => match kind {
                    Kind::Json => self.load_json_text(&text),
                    Kind::Agp => self.load_agp_text(&text),
                },
                Err(err) => self.status = format!("could not read file: {err}"),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (title, ext);
            self.dialog_open = true;
            let tx = self.events_tx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let handle = rfd::AsyncFileDialog::new().pick_file().await;
                if let Some(handle) = handle {
                    let bytes = handle.read().await;
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    let msg = match kind {
                        Kind::Json => Msg::Json(text),
                        Kind::Agp => Msg::Agp(text),
                    };
                    let _ = tx.send(msg);
                }
                let _ = tx.send(Msg::Done);
                ctx.request_repaint();
            });
        }
    }

    fn save_text(&mut self, ctx: egui::Context, file_name: &str, ext: &str, text: String) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            let Some(path) = rfd::FileDialog::new()
                .add_filter(ext, &[ext])
                .set_file_name(file_name)
                .save_file()
            else {
                return;
            };
            self.status = match std::fs::write(&path, text) {
                Ok(()) => format!("Saved {}", path.display()),
                Err(err) => format!("could not save file: {err}"),
            };
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ext;
            self.dialog_open = true;
            let tx = self.events_tx.clone();
            let file_name = file_name.to_string();
            wasm_bindgen_futures::spawn_local(async move {
                let handle = rfd::AsyncFileDialog::new().set_file_name(file_name).save_file().await;
                let note = if let Some(handle) = handle {
                    match handle.write(text.as_bytes()).await {
                        Ok(()) => format!("Saved {}", handle.file_name()),
                        Err(err) => format!("could not save file: {err}"),
                    }
                } else {
                    "Save cancelled.".to_string()
                };
                let _ = tx.send(Msg::Note(note));
                ctx.request_repaint();
            });
        }
    }

    fn sync_textures(&mut self, ctx: &egui::Context, view: &View) {
        let key = tex_key(&self.session);
        if self.overview_key != Some(key) {
            let (side, rgba) = self.session.raster_rgba(1024);
            let pixels = (side as usize).saturating_mul(side as usize).saturating_mul(4);
            if side > 0 && rgba.len() == pixels {
                let image = egui::ColorImage::from_rgba_unmultiplied([side as usize, side as usize], &rgba);
                if let Some(tex) = &mut self.map_tex {
                    tex.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.map_tex = Some(ctx.load_texture("contact-map", image, egui::TextureOptions::LINEAR));
                }
                self.overview_key = Some(key);
            }
        }

        let pointer_down = ctx.input(|input| input.pointer.any_down());
        let view_min = view.rect.width().min(view.rect.height());
        if pointer_down || self.loader.is_some() || !self.session.wants_detail(view_min) {
            return;
        }
        let (x0, y0, x1, y1) = view.visible_world(self.session.camera);
        let window = [
            quant(x0),
            quant(y0),
            quant(x1),
            quant(y1),
        ];
        if self.detail.as_ref().is_some_and(|detail| detail.key == key && detail.window == window) {
            return;
        }
        let settled = self.detail.as_ref().map(|detail| detail.key) == Some(key);
        if settled && self.detail_at.elapsed() < Duration::from_millis(150) {
            ctx.request_repaint_after(Duration::from_millis(150));
            return;
        }
        let samples = view.rect.width().min(view.rect.height()).round().clamp(96.0, 320.0) as u32;
        let rgba = self.session.raster_window(x0, y0, x1, y1, samples);
        let pixels = (samples as usize) * (samples as usize) * 4;
        if rgba.len() != pixels {
            return;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([samples as usize, samples as usize], &rgba);
        let tex = if let Some(detail) = &mut self.detail {
            detail.tex.set(image, egui::TextureOptions::NEAREST);
            detail.tex.clone()
        } else {
            ctx.load_texture("contact-detail", image, egui::TextureOptions::NEAREST)
        };
        self.detail = Some(Detail {
            x0,
            y0,
            x1,
            y1,
            key,
            window,
            tex,
        });
        self.detail_at = Instant::now();
    }

    fn map_view(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let view = View::new(rect, self.session.camera.zoom);
        self.pointer(ui, &view, &response);
        let view = View::new(rect, self.session.camera.zoom);
        self.sync_textures(ui.ctx(), &view);
        let painter = ui.painter_at(rect);
        self.paint(&painter, &view);
    }

    fn pointer(&mut self, ui: &mut egui::Ui, view: &View, response: &egui::Response) {
        let (edit_button, pan_button) = if self.session.invert_mouse {
            (egui::PointerButton::Secondary, egui::PointerButton::Primary)
        } else {
            (egui::PointerButton::Primary, egui::PointerButton::Secondary)
        };
        let (hover, scroll, delta, edit_down, edit_pressed, edit_released, pan_down, pan_pressed, shift) = ui.input(|input| {
            (
                input.pointer.hover_pos(),
                input.smooth_scroll_delta.y,
                input.pointer.delta(),
                input.pointer.button_down(edit_button),
                input.pointer.button_pressed(edit_button),
                input.pointer.button_released(edit_button),
                input.pointer.button_down(pan_button),
                input.pointer.button_pressed(pan_button),
                input.modifiers.shift,
            )
        });

        let pointer = response.hover_pos().or(hover).filter(|pos| view.rect.contains(*pos));
        if let Some(pos) = pointer {
            let world = view.screen_to_world(self.session.camera, pos);
            self.hover_world = Some(world);
            self.hover_pixel = pixel_of(&self.session, world.x);
            if delta.length_sq() > 0.0 {
                ui.ctx().request_repaint();
            }
            if scroll.abs() > 0.0 && response.hovered() {
                let before = world;
                let zoom = (self.session.camera.zoom * (scroll * 0.0015).exp()).clamp(0.25, 80.0);
                let scale = view.rect.width().min(view.rect.height()) * zoom;
                let cursor = pos - view.rect.center();
                self.session.camera.zoom = zoom;
                self.session.camera.x = before.x - cursor.x / scale;
                self.session.camera.y = before.y - cursor.y / scale;
            }
        } else {
            self.hover_world = None;
            self.hover_pixel = None;
        }

        if pan_pressed && hover.is_some_and(|pos| view.rect.contains(pos)) {
            self.panning = true;
        }
        if self.panning {
            if pan_down {
                let scale = (view.rect.width().min(view.rect.height()) * self.session.camera.zoom).max(1.0);
                self.session.camera.x -= delta.x / scale;
                self.session.camera.y -= delta.y / scale;
            } else {
                self.panning = false;
            }
        }

        let over_map = hover.filter(|pos| view.map_rect(self.session.camera).contains(*pos));
        if edit_pressed {
            if let Some(pos) = over_map {
                if let Some(pixel) = pixel_of(&self.session, view.screen_to_world(self.session.camera, pos).x) {
                    self.press = Some(Press {
                        pixel,
                        origin: pos,
                        moved: false,
                    });
                }
            }
        }
        if edit_down {
            if let Some(press) = &mut self.press {
                if let Some(pos) = hover {
                    if pos.distance(press.origin) > 4.0 {
                        press.moved = true;
                    }
                }
            }
            if self.session.mode == Mode::Edit && self.press.as_ref().is_some_and(|press| press.moved) {
                let dest = hover.and_then(|pos| pixel_of(&self.session, view.screen_to_world(self.session.camera, pos).x));
                self.drag_edit(dest);
            }
        } else if edit_released {
            if let Some(press) = self.press.take() {
                let end = hover
                    .map(|pos| view.screen_to_world(self.session.camera, pos))
                    .and_then(|world| pixel_of(&self.session, world.x))
                    .unwrap_or(press.pixel);
                if self.grab_armed && self.grab_dest.is_some() {
                    let count = self.selection.len();
                    self.session.note_edit(format!("moved {count} fragment(s)"));
                    self.status = format!("Moved {count} fragment(s).");
                    self.grab_armed = false;
                    self.grab_dest = None;
                } else {
                    if self.grab_armed {
                        self.session.disarm_edit();
                        self.grab_armed = false;
                        self.grab_dest = None;
                    }
                    self.finish_press(press, end, shift);
                }
            }
        }
    }

    fn drag_edit(&mut self, dest: Option<u32>) {
        let Some(dest) = dest else {
            return;
        };
        if self.grab_dest == Some(dest) {
            return;
        }
        let start = self.press.as_ref().map(|press| press.pixel).unwrap_or(dest);
        if !self.grab_armed {
            let uids = if self
                .selection_span()
                .is_some_and(|(from, to)| start >= from && start < to)
                && !self.selection.is_empty()
            {
                self.selection.clone()
            } else {
                let Some(index) = self.session.fragment_at(start) else {
                    return;
                };
                vec![self.session.fragments[index].uid]
            };
            self.selection = uids;
            self.session.arm_edit();
            self.grab_armed = true;
        }
        if self.session.place_armed(&self.selection, dest).is_ok() {
            self.grab_dest = Some(dest);
        }
    }

    fn finish_press(&mut self, press: Press, end: u32, shift: bool) {
        if press.moved {
            self.selection = self.uids_covering(press.pixel, end);
            if self.session.mode == Mode::Scaffold {
                self.paint_scaffolds();
            }
            return;
        }
        let Some(index) = self.session.fragment_at(end) else {
            return;
        };
        let uid = self.session.fragments[index].uid;
        if shift {
            if let Some(pos) = self.selection.iter().position(|id| *id == uid) {
                self.selection.remove(pos);
            } else {
                self.selection.push(uid);
            }
            return;
        }
        match self.session.mode {
            Mode::Edit => self.edit_click(end, uid),
            Mode::Normal | Mode::Extension => self.session.focus_fragment(index),
            Mode::Waypoint => {
                let world = self.session.pixel_to_world_center(end);
                self.session.add_waypoint(world, world);
                self.status = format!("{} waypoints.", self.session.waypoints.len());
            }
            Mode::Scaffold => {
                self.session.set_scaffold(index, self.session.paint_scaff);
                self.selection = vec![uid];
            }
            Mode::Meta => {
                self.session.toggle_tag(index);
                self.selection = vec![uid];
            }
            Mode::SelectSort => self.selection = vec![uid],
        }
    }

    fn edit_click(&mut self, pixel: u32, uid: u64) {
        if let Some((start, end)) = self.selection_span() {
            if pixel < start || pixel >= end {
                match self.session.place_uids(&self.selection, pixel) {
                    Ok(()) => {
                        self.status = "Moved the selection.".to_string();
                        self.selection.clear();
                    }
                    Err(err) => self.status = err.to_string(),
                }
                return;
            }
        }
        self.selection = vec![uid];
    }

    fn paint_scaffolds(&mut self) {
        let scaff = self.session.paint_scaff;
        let indexes: Vec<usize> = self
            .session
            .fragments
            .iter()
            .enumerate()
            .filter(|(_, frag)| self.selection.contains(&frag.uid))
            .map(|(index, _)| index)
            .collect();
        for index in indexes {
            self.session.set_scaffold(index, scaff);
        }
    }

    fn paint(&self, painter: &egui::Painter, view: &View) {
        let bg = rgba_color(self.session.background_rgba());
        painter.rect_filled(view.rect, 0.0, bg);
        let map = view.map_rect(self.session.camera);
        if let Some(tex) = &self.map_tex {
            painter.image(
                tex.id(),
                map,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if let Some(detail) = &self.detail {
            if detail.key == tex_key(&self.session) {
                let a = view.world_to_screen(self.session.camera, Vec2::new(detail.x0, detail.y0));
                let b = view.world_to_screen(self.session.camera, Vec2::new(detail.x1, detail.y1));
                painter.image(
                    detail.tex.id(),
                    Rect::from_two_pos(a, b),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        self.paint_fragments(painter, view, map);
        self.paint_selection(painter, view, map);
        self.paint_contig_labels(painter, view);
        self.paint_waypoints(painter, view);
        self.paint_tracks(painter, view);
        if self.session.scale_on {
            self.paint_scale(painter, view);
        }
        if self.session.tooltip_on {
            self.paint_tooltip(painter, view);
        }
        self.paint_extension_overlay(painter, view);
    }

    fn paint_extension_overlay(&self, painter: &egui::Painter, view: &View) {
        if self.session.mode != Mode::Extension || self.show_panel {
            return;
        }
        let mut lines = vec!["Extensions:".to_string(), "X: exit".to_string()];
        for track in &self.session.tracks {
            let key = match track.name.as_str() {
                "coverage" => "C",
                "gap" => "G",
                "repeat_density" => "R",
                "3p_telomere" => "3",
                "5p_telomere" => "5",
                "telomere" => "T",
                _ => continue,
            };
            lines.push(format!("{key}: Graph: {}", track.name));
        }
        let text = lines.join("\n");
        let color = Color32::from_rgb(240, 185, 15);
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(14.0), color);
        let pad = 8.0;
        let size = galley.size() + Vec2::splat(pad * 2.0);
        let rect = Rect::from_min_size(view.rect.right_top() + Vec2::new(-size.x - 12.0, 12.0), size);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(145, 140, 115));
        painter.galley(rect.min + Vec2::splat(pad), galley, color);
    }

    fn paint_contig_labels(&self, painter: &egui::Painter, view: &View) {
        if !self.session.labels_on {
            return;
        }
        let font = egui::FontId::proportional(self.session.label_size.max(8.0));
        let fg = rgba_color(self.session.label_fg);
        let bg = rgba_color(self.session.label_bg);
        let pixels = self.session.map_pixels.max(1);
        let camera = self.session.camera;
        let map_top = view.world_to_screen(camera, Vec2::new(0.0, -0.5)).y;
        let map_left = view.world_to_screen(camera, Vec2::new(-0.5, 0.0)).x;
        let inset = if self.session.id_bars_on {
            self.session.id_bar_size.max(4.0)
        } else {
            0.0
        };
        let top_y = map_top.max(view.rect.top()) + inset + 4.0;
        let left_x = map_left.max(view.rect.left()) + inset + 4.0;
        let mut acc = 0u32;
        for frag in &self.session.fragments {
            let start = acc;
            acc += frag.pixels;
            if frag.name.is_empty() {
                continue;
            }
            let x0 = view.world_to_screen(camera, Vec2::new(edge_world(start, pixels), 0.0)).x;
            let x1 = view.world_to_screen(camera, Vec2::new(edge_world(acc, pixels), 0.0)).x;
            let y0 = view.world_to_screen(camera, Vec2::new(0.0, edge_world(start, pixels))).y;
            let y1 = view.world_to_screen(camera, Vec2::new(0.0, edge_world(acc, pixels))).y;
            let wide = x1 - x0;
            let tall = y1 - y0;
            if wide.max(tall) < 8.0 {
                continue;
            }
            let on_x = x1 > view.rect.left() && x0 < view.rect.right();
            let on_y = y1 > view.rect.top() && y0 < view.rect.bottom();
            if !on_x && !on_y {
                continue;
            }
            let galley = painter.layout_no_wrap(frag.name.clone(), font.clone(), fg);
            let text_w = galley.size().x;
            let text_h = galley.size().y;
            if on_x && wide > text_w {
                let mid = (x0 + x1) * 0.5;
                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(mid - text_w * 0.5 - 2.0, top_y),
                        Pos2::new(mid + text_w * 0.5 + 2.0, top_y + text_h),
                    ),
                    0.0,
                    bg,
                );
                painter.galley(Pos2::new(mid - text_w * 0.5, top_y), galley.clone(), fg);
            }
            if on_y && tall > text_w {
                let mid = (y0 + y1) * 0.5;
                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(left_x, mid - text_w * 0.5 - 2.0),
                        Pos2::new(left_x + text_h, mid + text_w * 0.5 + 2.0),
                    ),
                    0.0,
                    bg,
                );
                let mut shape = egui::epaint::TextShape::new(Pos2::new(left_x, mid + text_w * 0.5), galley, fg);
                shape.angle = -std::f32::consts::FRAC_PI_2;
                painter.add(shape);
            }
        }
    }

    fn paint_fragments(&self, painter: &egui::Painter, view: &View, map: Rect) {
        let stroke = Stroke::new(1.0_f32, Color32::from_white_alpha(50));
        let mut acc = 0u32;
        for frag in &self.session.fragments {
            let start = acc;
            acc += frag.pixels;
            let width = frag.pixels as f32 / self.session.map_pixels.max(1) as f32 * view.scale;
            let x0 = view.world_to_screen(self.session.camera, Vec2::new(edge_world(start, self.session.map_pixels), 0.0)).x;
            let x1 = view.world_to_screen(self.session.camera, Vec2::new(edge_world(acc, self.session.map_pixels), 0.0)).x;
            let y0 = view.world_to_screen(self.session.camera, Vec2::new(0.0, edge_world(start, self.session.map_pixels))).y;
            let y1 = view.world_to_screen(self.session.camera, Vec2::new(0.0, edge_world(acc, self.session.map_pixels))).y;
            if self.session.grid_on && width >= 1.5 {
                painter.line_segment([Pos2::new(x1, map.top()), Pos2::new(x1, map.bottom())], stroke);
                painter.line_segment([Pos2::new(map.left(), y1), Pos2::new(map.right(), y1)], stroke);
            }
            if self.session.id_bars_on && x1 - x0 >= 1.0 {
                let color = fragment_color(frag.scaff_id, frag.uid);
                let span = (y1 - y0).max(1.0);
                let bar = self.session.id_bar_size.clamp(2.5, 4.5).min(span);
                let inset = 1.5_f32.min((x1 - x0) * 0.12);
                let (x_lo, x_hi) = (x0 + inset, x1 - inset);
                if x_hi > x_lo {
                    // A thin rounded strip on the top of this contig. Inverted contigs keep it on the far edge.
                    let rect = if frag.inverted {
                        Rect::from_min_max(Pos2::new(x_lo, y1 - bar), Pos2::new(x_hi, y1))
                    } else {
                        Rect::from_min_max(Pos2::new(x_lo, y0), Pos2::new(x_hi, y0 + bar))
                    };
                    painter.rect_filled(rect, bar * 0.5, color);
                    painter.rect_stroke(
                        rect,
                        bar * 0.5,
                        Stroke::new(1.0_f32, Color32::from_black_alpha(90)),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            if width < 36.0 {
                continue;
            }
            let center = Pos2::new((x0 + x1) * 0.5, map.top() + self.session.id_bar_size + 8.0);
            if (self.session.scaffolds_always || self.session.mode == Mode::Scaffold) && frag.scaff_id != 0 {
                painter.text(
                    Pos2::new(center.x, center.y + 14.0),
                    egui::Align2::CENTER_TOP,
                    format!("s{}", frag.scaff_id),
                    egui::FontId::proportional(self.session.scaff_size.max(10.0)),
                    rgba_color(self.session.scaff_text),
                );
            }
            if (self.session.meta_always || self.session.mode == Mode::Meta) && frag.meta != 0 {
                let tags = self.session.tag_names(frag.meta).join(" ");
                painter.text(
                    Pos2::new(center.x, map.bottom() - 16.0),
                    egui::Align2::CENTER_BOTTOM,
                    tags,
                    egui::FontId::proportional(self.session.meta_size),
                    rgba_color(self.session.meta_text),
                );
            }
        }
    }

    fn paint_selection(&self, painter: &egui::Painter, view: &View, map: Rect) {
        let span = if self.grab_armed {
            self.selection_span()
        } else if let Some(press) = &self.press {
            if press.moved && self.session.mode != Mode::Edit {
                let end = self.hover_pixel.unwrap_or(press.pixel);
                let (a, b) = if press.pixel <= end {
                    (press.pixel, end.saturating_add(1))
                } else {
                    (end, press.pixel.saturating_add(1))
                };
                Some((a, b))
            } else {
                self.selection_span().or_else(|| self.hover_fragment_span())
            }
        } else {
            self.selection_span().or_else(|| self.hover_fragment_span())
        };
        let Some((start, end)) = span else {
            return;
        };
        let x0 = view.world_to_screen(self.session.camera, Vec2::new(edge_world(start, self.session.map_pixels), 0.0)).x;
        let x1 = view.world_to_screen(self.session.camera, Vec2::new(edge_world(end, self.session.map_pixels), 0.0)).x;
        let y0 = view.world_to_screen(self.session.camera, Vec2::new(0.0, edge_world(start, self.session.map_pixels))).y;
        let y1 = view.world_to_screen(self.session.camera, Vec2::new(0.0, edge_world(end, self.session.map_pixels))).y;
        let color = if self.grab_armed {
            self.session.edit_edit
        } else {
            self.session.edit_select
        };
        let stroke = Stroke::new(2.0_f32, rgba_color(color));
        painter.rect_stroke(
            Rect::from_min_max(Pos2::new(x0, map.top()), Pos2::new(x1, map.bottom())),
            0.0,
            stroke,
            egui::StrokeKind::Inside,
        );
        painter.rect_stroke(
            Rect::from_min_max(Pos2::new(map.left(), y0), Pos2::new(map.right(), y1)),
            0.0,
            stroke,
            egui::StrokeKind::Inside,
        );
    }

    fn paint_waypoints(&self, painter: &egui::Painter, view: &View) {
        if !self.session.waypoints_always && self.session.mode != Mode::Waypoint {
            return;
        }
        for (index, waypoint) in self.session.waypoints.iter().enumerate() {
            let pos = view.world_to_screen(self.session.camera, Vec2::new(waypoint.x, waypoint.y));
            painter.circle_filled(pos, self.session.waypoint_size.max(4.0), rgba_color(self.session.waypoint_base));
            painter.text(
                pos + Vec2::new(8.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{}", index + 1),
                egui::FontId::proportional(12.0),
                rgba_color(self.session.waypoint_text),
            );
        }
    }

    fn paint_tracks(&self, painter: &egui::Painter, view: &View) {
        let tracks: Vec<_> = self.session.tracks.iter().filter(|track| track.on).collect();
        if tracks.is_empty() {
            return;
        }
        let mut acc = 0u32;
        for frag in &self.session.fragments {
            let start = acc;
            acc += frag.pixels;
            let x0 = view.world_to_screen(self.session.camera, Vec2::new(edge_world(start, self.session.map_pixels), 0.0)).x;
            let x1 = view.world_to_screen(self.session.camera, Vec2::new(edge_world(acc, self.session.map_pixels), 0.0)).x;
            for (index, track) in tracks.iter().enumerate() {
                let value = frag.track_value(track.kind).clamp(0.0, 1.0);
                let height = 8.0 * track.height.max(0.4);
                let top = view.rect.top() + index as f32 * (height + 2.0);
                let filled = Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1, top + height * value.max(0.08)));
                painter.rect_filled(filled, 0.0, rgba_color(track.color));
            }
        }
    }

    fn paint_scale(&self, painter: &egui::Painter, view: &View) {
        let bp_per_px = self.session.genome_bp as f32 / view.scale.max(1.0);
        let nice = nice_bp(bp_per_px * 120.0);
        let bar = (nice / bp_per_px).clamp(24.0, view.rect.width() * 0.4);
        let origin = view.rect.left_bottom() + Vec2::new(16.0, -28.0);
        let stroke = Stroke::new(2.0_f32, rgba_color(self.session.scale_fg));
        painter.line_segment([origin, origin + Vec2::new(bar, 0.0)], stroke);
        painter.text(
            origin + Vec2::new(0.0, -4.0),
            egui::Align2::LEFT_BOTTOM,
            format_bp(nice),
            egui::FontId::proportional(self.session.scale_size.max(11.0)),
            rgba_color(self.session.scale_fg),
        );
    }

    fn paint_tooltip(&self, painter: &egui::Painter, view: &View) {
        let Some(world) = self.hover_world else {
            return;
        };
        let x = coord_label(&self.session, world.x);
        let y = coord_label(&self.session, world.y);
        let mut text = format!("{x}\nvs\n{y}");
        if let Some(pixel) = self.hover_pixel {
            if let Some(index) = self.session.fragment_at(pixel) {
                let frag = &self.session.fragments[index];
                if frag.scaff_id != 0 {
                    text.push_str(&format!("\nScaffold: {}", frag.scaff_id));
                }
                let tags = self.session.tag_names(frag.meta);
                if !tags.is_empty() {
                    text.push_str(&format!("\nTags: {}", tags.join(", ")));
                }
            }
        }
        let anchor = view.world_to_screen(self.session.camera, world) + Vec2::new(16.0, 16.0);
        let galley = painter.layout_no_wrap(
            text,
            egui::FontId::proportional(self.session.tooltip_size),
            rgba_color(self.session.tooltip_fg),
        );
        let mut pos = anchor;
        let size = galley.size();
        if pos.x + size.x > view.rect.right() - 8.0 {
            pos.x = anchor.x - size.x - 32.0;
        }
        if pos.y + size.y > view.rect.bottom() - 8.0 {
            pos.y = anchor.y - size.y - 32.0;
        }
        pos = pos.max(view.rect.min + Vec2::splat(8.0));
        let bg = Rect::from_min_size(pos, size).expand(4.0);
        painter.rect_filled(bg, 2.0, rgba_color(self.session.tooltip_bg));
        painter.galley(pos, galley, rgba_color(self.session.tooltip_fg));
    }
}

impl eframe::App for PretextApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_menu_theme(ctx, self.menu_theme);
        self.poll();
        self.take_drops(ctx);
        self.pump_loader(ctx);
        self.keys(ctx);

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.label(self.status.as_str());
        });
        if self.show_panel {
            egui::SidePanel::left("options")
                .resizable(true)
                .min_width(380.0)
                .default_width(440.0)
                .show(ctx, |ui| {
                    egui::TopBottomPanel::bottom("options-foot")
                        .exact_height(36.0)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(8)))
                        .show_inside(ui, |ui| {
                            ui.label(self.session.summary_line());
                        });
                    egui::ScrollArea::vertical().show(ui, |ui| self.options(ui));
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.map_view(ui));
    }
}

fn menu_button(ui: &mut egui::Ui, text: &str, height: f32) -> egui::Response {
    menu_sized(ui, text, ui.available_width(), height)
}

fn menu_sized(ui: &mut egui::Ui, text: &str, width: f32, height: f32) -> egui::Response {
    ui.add_sized(Vec2::new(width, height), egui::Button::new(text))
}

fn menu_choice(ui: &mut egui::Ui, text: &str, on: bool, height: f32) -> egui::Response {
    let mut button = egui::Button::new(text);
    if on {
        button = button.stroke(Stroke::new(2.0_f32, Color32::from_rgb(214, 176, 48)));
    }
    ui.add_sized(Vec2::new(ui.available_width(), height), button)
}

fn split_widths(ui: &egui::Ui, left: f32, right: f32) -> (f32, f32) {
    let gap = ui.spacing().item_spacing.x;
    let avail = (ui.available_width() - gap).max(0.0);
    let sum = (left + right).max(1.0);
    (avail * left / sum, avail * right / sum)
}

fn menu_check(ui: &mut egui::Ui, on: &mut bool, text: &str) {
    ui.scope(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.checkbox(on, text);
    });
}

fn gamma_slider(ui: &mut egui::Ui, label: &str, value: &mut f32) -> bool {
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_min_width(width);
        ui.add_sized(Vec2::new(86.0, 20.0), egui::Label::new(label));
        ui.spacing_mut().slider_width = (ui.available_width() - 4.0).max(48.0);
        ui.add(
            egui::Slider::new(value, 0.0..=1.0)
                .trailing_fill(true)
                .show_value(false),
        )
        .changed()
    })
    .inner
}

enum Kind {
    Json,
    Agp,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuTheme {
    Dark,
    Blue,
    White,
    Red,
    Black,
}

impl MenuTheme {
    fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Blue => "Blue",
            Self::White => "White",
            Self::Red => "Red",
            Self::Black => "Black",
        }
    }
}

struct MenuPalette {
    text: Color32,
    window: Color32,
    header: Color32,
    border: Color32,
    button: Color32,
    button_hover: Color32,
    button_active: Color32,
    widget: Color32,
}

fn menu_palette(theme: MenuTheme) -> MenuPalette {
    let rgb = |r, g, b| Color32::from_rgb(r, g, b);
    match theme {
        MenuTheme::Dark => MenuPalette {
            text: rgb(210, 210, 210),
            window: rgb(57, 67, 71),
            header: rgb(51, 51, 56),
            border: rgb(46, 46, 46),
            button: rgb(48, 83, 111),
            button_hover: rgb(58, 93, 121),
            button_active: rgb(63, 98, 126),
            widget: rgb(50, 58, 61),
        },
        MenuTheme::Blue => MenuPalette {
            text: rgb(20, 20, 20),
            window: rgb(202, 212, 214),
            header: rgb(137, 182, 224),
            border: rgb(140, 159, 173),
            button: rgb(137, 182, 224),
            button_hover: rgb(142, 187, 229),
            button_active: rgb(147, 192, 234),
            widget: rgb(177, 210, 210),
        },
        MenuTheme::White => MenuPalette {
            text: rgb(70, 70, 70),
            window: rgb(175, 175, 175),
            header: rgb(175, 175, 175),
            border: rgb(0, 0, 0),
            button: rgb(185, 185, 185),
            button_hover: rgb(170, 170, 170),
            button_active: rgb(160, 160, 160),
            widget: rgb(150, 150, 150),
        },
        MenuTheme::Red => MenuPalette {
            text: rgb(190, 190, 190),
            window: rgb(30, 33, 40),
            header: rgb(181, 45, 69),
            border: rgb(51, 55, 67),
            button: rgb(181, 45, 69),
            button_hover: rgb(190, 50, 70),
            button_active: rgb(195, 55, 75),
            widget: rgb(51, 55, 67),
        },
        MenuTheme::Black => MenuPalette {
            text: rgb(230, 230, 230),
            window: rgb(18, 18, 18),
            header: rgb(28, 28, 28),
            border: rgb(70, 70, 70),
            button: rgb(42, 42, 42),
            button_hover: rgb(58, 58, 58),
            button_active: rgb(72, 72, 72),
            widget: rgb(32, 32, 32),
        },
    }
}

fn apply_menu_theme(ctx: &egui::Context, theme: MenuTheme) {
    let palette = menu_palette(theme);
    let mut visuals = if matches!(theme, MenuTheme::White | MenuTheme::Blue) {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    visuals.override_text_color = Some(palette.text);
    visuals.window_fill = palette.window;
    visuals.panel_fill = palette.window;
    visuals.extreme_bg_color = palette.widget;
    visuals.faint_bg_color = palette.header;
    visuals.window_stroke = Stroke::new(1.0_f32, palette.border);
    visuals.selection.bg_fill = palette.button_active;
    visuals.selection.stroke = Stroke::new(1.0_f32, palette.text);
    let style_widget = |widget: &mut egui::style::WidgetVisuals, fill: Color32| {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke.color = palette.border;
        widget.fg_stroke.color = palette.text;
    };
    style_widget(&mut visuals.widgets.noninteractive, palette.window);
    style_widget(&mut visuals.widgets.inactive, palette.button);
    style_widget(&mut visuals.widgets.hovered, palette.button_hover);
    style_widget(&mut visuals.widgets.active, palette.button_active);
    style_widget(&mut visuals.widgets.open, palette.header);
    ctx.set_visuals(visuals);
}

struct View {
    rect: Rect,
    scale: f32,
}

impl View {
    fn new(rect: Rect, zoom: f32) -> Self {
        Self {
            rect,
            scale: rect.width().min(rect.height()) * zoom,
        }
    }

    fn world_to_screen(&self, camera: crate::model::Camera, world: Vec2) -> Pos2 {
        self.rect.center() + (world - Vec2::new(camera.x, camera.y)) * self.scale
    }

    fn screen_to_world(&self, camera: crate::model::Camera, pos: Pos2) -> Vec2 {
        Vec2::new(camera.x, camera.y) + (pos - self.rect.center()) / self.scale.max(1e-4)
    }

    fn map_rect(&self, camera: crate::model::Camera) -> Rect {
        Rect::from_center_size(self.world_to_screen(camera, Vec2::ZERO), Vec2::splat(self.scale))
    }

    fn visible_world(&self, camera: crate::model::Camera) -> (f32, f32, f32, f32) {
        let min = self.screen_to_world(camera, self.rect.min);
        let max = self.screen_to_world(camera, self.rect.max);
        (
            min.x.clamp(-0.5, 0.5),
            min.y.clamp(-0.5, 0.5),
            max.x.clamp(-0.5, 0.5),
            max.y.clamp(-0.5, 0.5),
        )
    }
}

fn tex_key(session: &Session) -> TexKey {
    TexKey {
        revision: session.map_revision,
        colormap: session.colormap,
        gamma: session.gamma.map(|value| (value * 1000.0).round() as u32),
    }
}

fn quant(value: f32) -> i32 {
    (value * 10_000.0).round() as i32
}

fn coord_label(session: &Session, world: f32) -> String {
    let map = session.map_pixels.max(1) as f64;
    let exact = ((world as f64 + 0.5) * map).clamp(0.0, map - 1e-6);
    let pixel = exact.floor() as u32;
    let Some(index) = session.fragment_at(pixel) else {
        return format!("pixel {pixel}");
    };
    let frag = &session.fragments[index];
    let local = pixel.saturating_sub(session.prefix(index)).min(frag.pixels.saturating_sub(1));
    let here = frag.ref_at(local).max(0) as f64;
    let next = if local + 1 < frag.pixels {
        frag.ref_at(local + 1).max(0) as f64
    } else {
        here
    };
    let at = here + (next - here) * (exact - pixel as f64);
    let bp = (at + 0.5) / map * session.genome_bp as f64;
    format!("{} {}", frag.name, format_coord(bp))
}

fn format_coord(bp: f64) -> String {
    if bp >= 1_000_000.0 {
        format!("{:.2} Mb", bp / 1_000_000.0)
    } else if bp >= 1_000.0 {
        format!("{:.1} kb", bp / 1_000.0)
    } else {
        format!("{:.0} bp", bp.max(0.0))
    }
}

fn pixel_of(session: &Session, world_x: f32) -> Option<u32> {
    if session.map_pixels == 0 {
        return None;
    }
    let pixel = session.world_to_pixel(world_x);
    if pixel < 0 {
        None
    } else {
        Some((pixel as u32).min(session.map_pixels - 1))
    }
}

fn edge_world(pixel: u32, map_pixels: u32) -> f32 {
    pixel as f32 / map_pixels.max(1) as f32 - 0.5
}

fn rgba_color(color: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3])
}

fn fragment_color(scaff: u32, uid: u64) -> Color32 {
    let hue = if scaff == 0 { uid.wrapping_mul(47) } else { scaff as u64 * 53 };
    Color32::from_rgb(
        70 + (hue % 180) as u8,
        80 + ((hue / 3) % 150) as u8,
        90 + ((hue / 7) % 140) as u8,
    )
}

fn nice_bp(bp: f32) -> f32 {
    if bp <= 1.0 {
        return 1.0;
    }
    let exp = bp.log10().floor();
    let base = 10f32.powf(exp);
    let mant = bp / base;
    let nice = if mant < 1.5 {
        1.0
    } else if mant < 3.5 {
        2.0
    } else if mant < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * base
}

fn format_bp(bp: f32) -> String {
    if bp >= 1_000_000.0 {
        format!("{:.2} Mb", bp / 1_000_000.0)
    } else if bp >= 1_000.0 {
        format!("{:.0} kb", bp / 1_000.0)
    } else {
        format!("{bp:.0} bp")
    }
}

#[cfg(target_arch = "wasm32")]
async fn fetch_bundled_bytes() -> Option<(String, Vec<u8>)> {
    use eframe::wasm_bindgen::JsCast;

    let window = eframe::web_sys::window()?;
    let url = "idOchAnnu3_RC-5104_normal.alg.pretext";
    let value = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url)).await.ok()?;
    let response: eframe::web_sys::Response = value.dyn_into().ok()?;
    if !response.ok() {
        return None;
    }
    let buffer = wasm_bindgen_futures::JsFuture::from(response.array_buffer().ok()?).await.ok()?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    if bytes.len() < 4 || &bytes[..4] != b"pstm" {
        return None;
    }
    Some((url.to_string(), bytes))
}
