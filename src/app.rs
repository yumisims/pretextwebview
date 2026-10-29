//! egui shell: contact map, file loading, and the edit controls.

use std::sync::mpsc;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use web_time::{Duration, Instant};

use crate::colormap::NAMES;
use crate::model::{ensure_agp_suffix, Mode, Session, BACKGROUNDS};
use crate::pretext::{MapLoader, Pump};
use crate::suggest::{self, SuggestId, Suggestion};
use crate::yumi::SignalMethod;

#[derive(Clone, Copy, PartialEq, Eq)]
struct TexKey {
    revision: u64,
    colormap: usize,
    gamma: [u32; 3],
    signal: u8,
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
    show_view: bool,
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
    /// Suggested AGP download name. Filled from the map, and editable.
    agp_name: String,
    map_url: String,
    menu_theme: MenuTheme,
    suggestions: Vec<Suggestion>,
    suggestion_skips: Vec<SuggestId>,
    suggestions_ready: bool,
    scanned_stamp: u64,
    events: mpsc::Receiver<Msg>,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    events_tx: mpsc::Sender<Msg>,
    dialog_open: bool,
}

impl PretextApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        Self::initial_state()
    }

    fn initial_state() -> Self {
        let (events_tx, events) = mpsc::channel();
        let app = Self {
            session: Session::empty(),
            status: "Load a .pretext file or an https link, or drop a file on the view. Esc hides the panel.".to_string(),
            show_panel: true,
            show_view: false,
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
            agp_name: Session::empty().agp_file_name(),
            map_url: String::new(),
            menu_theme: MenuTheme::Dark,
            suggestions: Vec::new(),
            suggestion_skips: Vec::new(),
            suggestions_ready: false,
            scanned_stamp: 0,
            events,
            events_tx,
            dialog_open: false,
        };
        app
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
        if lower.ends_with(".pretext") {
            self.begin_pretext(name.to_string(), bytes);
        } else {
            self.status = "Only .pretext map files are supported in this viewer.".to_string();
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
        self.session.mode = Mode::Normal;
        self.agp_name = self.session.agp_file_name();
        self.selection.clear();
        self.press = None;
        self.grab_armed = false;
        self.grab_dest = None;
        self.detail = None;
        self.overview_key = None;
        self.saw_tile_preview = false;
        self.loader = Some(loader);
        self.clear_suggestions();
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
                self.clear_suggestions();
                self.status = "Loaded JSON state.".to_string();
            }
            Err(err) => self.status = err,
        }
    }

    fn load_agp_text(&mut self, text: &str) {
        match self.session.load_agp(text) {
            Ok(n) => {
                self.clear_suggestions();
                self.status = format!("AGP placed {n} parts.");
            }
            Err(err) => self.status = err,
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.show_panel = !self.show_panel;
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        let (tooltip, names) = ctx.input(|input| {
            let plain = !input.modifiers.command && !input.modifiers.alt;
            let key = |key| plain && input.key_pressed(key);
            (key(egui::Key::T), key(egui::Key::N))
        });
        if tooltip {
            self.session.tooltip_on = !self.session.tooltip_on;
        }
        if names {
            self.session.labels_on = !self.session.labels_on;
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
            "Edit mode. Drag a block to move it. V cuts the contig under the pointer. I or Space inverts it.".to_string()
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

    /// `from_pointer` inverts the contig under the cursor. A selection is used
    /// when the cursor is inside it, when a block is already picked up, or when
    /// the Invert button is used with the pointer away from the map.
    fn invert_current(&mut self, from_pointer: bool) {
        let uids = self.invert_target(from_pointer);
        if uids.is_empty() {
            self.status = "Point at a contig, or select one, then invert.".to_string();
            return;
        }
        let label = self.describe_uids(&uids);
        self.selection = uids.clone();
        match self.session.invert_uids(&uids) {
            Ok(()) => self.status = format!("Inverted {label}."),
            Err(err) => self.status = err.to_string(),
        }
    }

    fn invert_target(&self, from_pointer: bool) -> Vec<u64> {
        if self.grab_armed && !self.selection.is_empty() {
            return self.selection.clone();
        }
        if from_pointer {
            if let Some(pixel) = self.hover_pixel {
                if let Some((start, end)) = self.selection_span() {
                    if !self.selection.is_empty() && pixel >= start && pixel < end {
                        return self.selection.clone();
                    }
                }
                if let Some(index) = self.session.fragment_at(pixel) {
                    return vec![self.session.fragments[index].uid];
                }
            }
        }
        if !self.selection.is_empty() {
            return self.selection.clone();
        }
        self.hovered_uid().into_iter().collect()
    }

    fn hovered_uid(&self) -> Option<u64> {
        let pixel = self.hover_pixel?;
        let index = self.session.fragment_at(pixel)?;
        Some(self.session.fragments[index].uid)
    }

    fn describe_uids(&self, uids: &[u64]) -> String {
        if uids.len() == 1 {
            self.session
                .fragments
                .iter()
                .find(|frag| frag.uid == uids[0])
                .map(|frag| frag.name.clone())
                .unwrap_or_else(|| "1 contig".to_string())
        } else {
            format!("{} contigs", uids.len())
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

    fn clear_suggestions(&mut self) {
        self.suggestions.clear();
        self.suggestion_skips.clear();
        self.suggestions_ready = false;
        self.scanned_stamp = 0;
    }

    fn scan_suggestions(&mut self, fresh: bool) {
        if self.session.fragments.is_empty() {
            self.suggestions.clear();
            self.suggestions_ready = true;
            self.scanned_stamp = suggest::layout_stamp(&self.session);
            self.status = "Load a map, then scan.".to_string();
            return;
        }
        if fresh {
            self.suggestion_skips.clear();
        }
        self.suggestions = suggest::suggest(&self.session, &self.suggestion_skips);
        self.suggestions_ready = true;
        self.scanned_stamp = suggest::layout_stamp(&self.session);
        let n = self.suggestions.len();
        self.status = if n == 0 {
            "No edits suggested.".to_string()
        } else {
            format!("{n} suggestions. Show jumps there. Accept applies one step.")
        };
    }

    fn suggestions_ui(&mut self, ui: &mut egui::Ui) {
        let row_h = 30.0;
        if menu_button(ui, "Scan", row_h).clicked() {
            self.scan_suggestions(true);
        }
        if self.session.fragments.is_empty() {
            ui.label("Load a map, then scan.");
            return;
        }
        if !self.suggestions_ready {
            ui.label("Scan proposes splits, inversions, and moves.");
            return;
        }
        let stale = suggest::layout_stamp(&self.session) != self.scanned_stamp;
        if stale {
            ui.label("The map changed. Scan again before accepting.");
        }
        if self.suggestions.is_empty() {
            ui.label("No edits suggested.");
            return;
        }
        let mut action: Option<(usize, SuggestAct)> = None;
        for (index, suggestion) in self.suggestions.iter().enumerate() {
            ui.label(suggestion.reason.as_str());
            ui.columns(3, |cols| {
                if menu_button(&mut cols[0], "Show", row_h).clicked() {
                    action = Some((index, SuggestAct::Show));
                }
                if menu_button(&mut cols[1], "Accept", row_h).clicked() && !stale {
                    action = Some((index, SuggestAct::Accept));
                }
                if menu_button(&mut cols[2], "Skip", row_h).clicked() {
                    action = Some((index, SuggestAct::Skip));
                }
            });
        }
        let Some((index, act)) = action else {
            return;
        };
        let Some(suggestion) = self.suggestions.get(index).cloned() else {
            return;
        };
        match act {
            SuggestAct::Show => {
                suggest::focus(&mut self.session, &suggestion);
                self.status = suggestion.reason;
            }
            SuggestAct::Accept => {
                if suggest::layout_stamp(&self.session) != self.scanned_stamp {
                    self.status = "The map changed. Scan again before accepting.".to_string();
                    return;
                }
                match suggest::apply(&mut self.session, &suggestion) {
                    Ok(()) => {
                        self.status = format!("Applied. {}", suggestion.reason);
                        self.scan_suggestions(false);
                    }
                    Err(err) => self.status = err,
                }
            }
            SuggestAct::Skip => {
                self.suggestion_skips.push(suggestion.id);
                self.suggestions.remove(index);
                self.status = "Skipped that suggestion.".to_string();
            }
        }
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

        {
            let load = egui::Button::new("Load Map").min_size(Vec2::new(ui.available_width(), row_h));
            let load_response = ui.add_enabled(!self.dialog_open, load);
            let popup_id = ui.make_persistent_id("load-map-popup");
            if load_response.clicked() {
                ui.memory_mut(|mem| mem.toggle_popup(popup_id));
            }
            egui::popup::popup_below_widget(
                ui,
                popup_id,
                &load_response,
                egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                |ui| {
                    ui.set_min_width(320.0);
                    ui.label("Map link");
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.map_url)
                            .hint_text("https://…")
                            .desired_width(320.0),
                    );
                    if load_response.clicked() {
                        edit.request_focus();
                    }
                    let open = ui.add_sized(Vec2::new(ui.available_width(), row_h), egui::Button::new("Open link"));
                    let enter = edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if open.clicked() || enter {
                        let url = self.map_url.clone();
                        self.fetch_map_url(ui.ctx().clone(), &url);
                        ui.memory_mut(|mem| mem.close_popup());
                    }
                    if ui
                        .add_sized(
                            Vec2::new(ui.available_width(), row_h),
                            egui::Button::new("Choose .pretext file"),
                        )
                        .clicked()
                    {
                        self.pick_pretext(ui.ctx().clone());
                        ui.memory_mut(|mem| mem.close_popup());
                    }
                },
            );
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
            self.session.gamma[0] = self.session.gamma[0].min(self.session.gamma[2]);
            self.session.gamma[1] = self.session.gamma[1].clamp(self.session.gamma[0], self.session.gamma[2]);
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

    fn fetch_map_url(&mut self, ctx: egui::Context, raw: &str) {
        let url = match accept_map_url(raw) {
            Ok(url) => url,
            Err(err) => {
                self.status = err;
                return;
            }
        };
        let name = map_name_from_url(&url);
        self.status = format!("Fetching {name}…");
        self.dialog_open = true;
        let tx = self.events_tx.clone();
        ehttp::fetch(ehttp::Request::get(&url), move |result| {
            match result {
                Ok(response) if response.ok => {
                    let _ = tx.send(Msg::Pretext {
                        name,
                        bytes: response.bytes,
                    });
                }
                Ok(response) => {
                    let _ = tx.send(Msg::Note(format!("The link returned HTTP {}.", response.status)));
                }
                Err(err) => {
                    let _ = tx.send(Msg::Note(format!(
                        "Could not fetch the map ({err}). The server must allow this page to read the file."
                    )));
                }
            }
            let _ = tx.send(Msg::Done);
            ctx.request_repaint();
        });
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

    fn chosen_agp_name(&self) -> String {
        let typed = self.agp_name.trim();
        if typed.is_empty() {
            self.session.agp_file_name()
        } else {
            ensure_agp_suffix(typed)
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
        if self.session.fragments.is_empty() && self.loader.is_none() {
            self.empty_map_prompt(ui.ctx(), rect);
        }
    }

    fn empty_map_prompt(&mut self, ctx: &egui::Context, rect: Rect) {
        let hovering_file = ctx.input(|input| !input.raw.hovered_files.is_empty());
        egui::Area::new(egui::Id::new("empty-map-prompt"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.center())
            .pivot(egui::Align2::CENTER_CENTER)
            .show(ctx, |ui| {
                let fill = if hovering_file {
                    Color32::from_rgb(44, 67, 82)
                } else {
                    Color32::from_rgb(36, 39, 43)
                };
                egui::Frame::new()
                    .fill(fill)
                    .stroke(Stroke::new(
                        1.0_f32,
                        if hovering_file {
                            Color32::from_rgb(117, 190, 230)
                        } else {
                            Color32::from_rgb(88, 96, 105)
                        },
                    ))
                    .inner_margin(egui::Margin::same(24))
                    .corner_radius(8)
                    .show(ui, |ui| {
                        ui.set_width(320.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new("Open a contact map")
                                    .strong()
                                    .size(22.0),
                            );
                            ui.add_space(8.0);
                            ui.label("Choose a .pretext file, or drag and drop it here.");
                            ui.add_space(16.0);
                            if ui
                                .add_sized(
                                    Vec2::new(260.0, 44.0),
                                    egui::Button::new("Choose .pretext file"),
                                )
                                .clicked()
                            {
                                self.pick_pretext(ctx.clone());
                            }
                            ui.add_space(8.0);
                            ui.label("You can also open an HTTPS link from Load Map.");
                        });
                    });
            });
    }

    fn pointer(&mut self, ui: &mut egui::Ui, view: &View, response: &egui::Response) {
        let pan_button = if self.session.invert_mouse {
            egui::PointerButton::Primary
        } else {
            egui::PointerButton::Secondary
        };
        let (hover, scroll, delta, pan_down, pan_pressed) = ui.input(|input| {
            (
                input.pointer.hover_pos(),
                input.smooth_scroll_delta.y,
                input.pointer.delta(),
                input.pointer.button_down(pan_button),
                input.pointer.button_pressed(pan_button),
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

        if pan_pressed && response.hovered() {
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
        self.paint_contig_labels(painter, view);
        self.paint_waypoints(painter, view);
        if self.session.scale_on {
            self.paint_scale(painter, view);
        }
        if self.session.tooltip_on {
            self.paint_tooltip(painter, view);
        }
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

fn paint_mode_help(ctx: &egui::Context, mode: Mode, open: &mut bool) {
    if !*open {
        return;
    }
    let (title, lines) = mode_help(mode);
    let ink = Color32::from_rgb(255, 255, 255);
    let paper = Color32::from_rgb(12, 14, 18);
    let mut visuals = ctx.style().visuals.clone();
    visuals.override_text_color = Some(ink);
    visuals.window_fill = paper;
    ctx.set_visuals(visuals);

    let corner = ctx.available_rect().right_bottom() + Vec2::new(-16.0, -16.0);
    egui::Window::new(egui::RichText::new(title).strong().size(18.0).color(ink))
        .id(egui::Id::new("mode-help"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .default_pos(corner)
        .open(open)
        .movable(true)
        .collapsible(false)
        .resizable(false)
        .fade_in(false)
        .frame(
            egui::Frame::new()
                .fill(paper)
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(230, 230, 230)))
                .inner_margin(egui::Margin::same(12))
                .corner_radius(6),
        )
        .show(ctx, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.style_mut().spacing.item_spacing.y = 4.0;
            ui.set_min_width(240.0);
            ui.set_max_width(360.0);
            for line in lines {
                ui.label(egui::RichText::new(*line).size(16.0).color(ink));
            }
        });
}

fn mode_help(mode: Mode) -> (&'static str, &'static [&'static str]) {
    match mode {
        Mode::Normal => (
            "View",
            &[
                "E: Edit mode",
                "I: invert the contig under the pointer",
                "X: Extension mode",
                "T: tooltip",
                "N: contig name labels",
                "3 / 5: telomere tracks",
                "Scroll: zoom    right drag: pan",
                "Click: focus a contig",
                "H: hide this view",
                "Esc: menu",
            ],
        ),
        Mode::Edit => (
            "Edit Mode",
            &[
                "E: exit",
                "Left drag: move a block",
                "Click, then click outside: place",
                "V: cut the contig under the pointer",
                "I or Space: invert the contig under the pointer",
                "Middle click: invert that contig",
                "Cmd-Z: undo    Cmd-Shift-Z: redo",
                "N: contig name labels",
                "Esc: menu",
            ],
        ),
        Mode::Waypoint => (
            "Waypoint Edit Mode",
            &[
                "Click: add a waypoint",
                "N: contig name labels",
                "T: tooltip",
                "Scroll: zoom    right drag: pan",
                "Esc: menu",
            ],
        ),
        Mode::Scaffold => (
            "Scaffold Edit Mode",
            &[
                "Click: paint the current scaffold",
                "Drag: paint a range",
                "N: contig name labels",
                "Esc: menu",
            ],
        ),
        Mode::Meta => (
            "MetaData Tag Mode",
            &[
                "Click: toggle the active tag",
                "E: Edit mode",
                "N: contig name labels",
                "Esc: menu",
            ],
        ),
        Mode::Extension => (
            "Extension Mode",
            &[
                "X: exit",
                "C: coverage",
                "G: gap",
                "R: repeat density",
                "T: telomere",
                "3 / 5: 3p / 5p telomere",
                "N: contig name labels",
                "Esc: menu",
            ],
        ),
        Mode::SelectSort => (
            "Select sort area",
            &[
                "Click: mark a contig",
                "Shift-click: add or remove",
                "Pixel Sort uses the marked contigs",
                "E: Edit mode",
                "N: contig name labels",
                "Esc: menu",
            ],
        ),
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

enum SuggestAct {
    Show,
    Accept,
    Skip,
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
        signal: match session.signal {
            SignalMethod::Pretext => 0,
            SignalMethod::Yumi => 1,
        },
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

fn accept_map_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if url.is_empty() {
        return Err("Enter an https link.".to_string());
    }
    if !url.to_ascii_lowercase().starts_with("https://") {
        return Err("The link has to start with https://.".to_string());
    }
    if url.len() <= "https://".len() {
        return Err("That link has no host.".to_string());
    }
    Ok(url.to_string())
}

fn map_name_from_url(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("").trim();
    if name.is_empty() || name.contains(':') {
        "map.pretext".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod viewer_tests {
    use super::*;

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 1800.0))),
            events,
            ..Default::default()
        }
    }

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn pointer_frame(app: &mut PretextApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run(input(events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(rect, Sense::click_and_drag());
                let view = View::new(rect, app.session.camera.zoom);
                app.pointer(ui, &view, &response);
            });
        });
    }

    fn pointer_button(button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: Pos2::new(200.0, 200.0),
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn menu_contains_only_viewing_controls() {
        fn collect_text(shape: &egui::epaint::Shape, labels: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(text) => labels.push(text.galley.job.text.clone()),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect_text(shape, labels);
                    }
                }
                _ => {}
            }
        }

        let ctx = egui::Context::default();
        let mut app = PretextApp::initial_state();
        let output = ctx.run(input(Vec::new()), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.options(ui));
        });
        let mut labels = Vec::new();
        for shape in output.shapes {
            collect_text(&shape.shape, &mut labels);
        }
        let mut expected = vec![
            "Esc", "Dark", "Black", "White", "Red", "Blue", "Load Map",
            "Waypoints Always Visible", "Scaffolds Always Visible", "Contig Name Labels",
            "Scale Bars", "Grid", "ID Bars", "Tool Tip", "Invert Mouse Buttons",
            "Gamma Min", "Gamma Mid", "Gamma Max", "Default Gamma", "Colour Maps", "Background",
        ];
        expected.extend(NAMES.iter().copied());
        expected.extend(BACKGROUNDS.iter().map(|(name, _)| *name));
        for label in &expected {
            assert!(labels.iter().any(|text| text == label), "missing control: {label}");
        }
        for label in &labels {
            assert!(
                label.is_empty() || expected.contains(&label.as_str()) || label.parse::<f32>().is_ok(),
                "unexpected menu text: {label}"
            );
        }
    }

    #[test]
    fn escape_hides_and_restores_menu_even_with_text_focus() {
        let ctx = egui::Context::default();
        let mut app = PretextApp::initial_state();
        let _ = ctx.run(input(Vec::new()), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.text_edit_singleline(&mut app.map_url).request_focus();
            });
        });
        for visible in [false, true] {
            let _ = ctx.run(input(vec![key_event(egui::Key::Escape, egui::Modifiers::NONE)]), |ctx| {
                app.keys(ctx);
            });
            assert_eq!(app.show_panel, visible);
        }
    }

    #[test]
    fn removed_shortcuts_do_not_change_map_or_mode() {
        let ctx = egui::Context::default();
        let mut app = PretextApp::initial_state();
        app.session = Session::demo();
        let before = app.session.save_json();
        let mut events: Vec<_> = [
            egui::Key::E, egui::Key::I, egui::Key::X, egui::Key::V, egui::Key::Space,
            egui::Key::H, egui::Key::Num3, egui::Key::Num5,
            egui::Key::C, egui::Key::G, egui::Key::R,
        ].into_iter().map(|key| key_event(key, egui::Modifiers::NONE)).collect();
        events.push(key_event(egui::Key::Z, egui::Modifiers::COMMAND));
        let _ = ctx.run(input(events), |ctx| app.keys(ctx));
        assert_eq!(app.session.mode, Mode::Normal);
        assert_eq!(app.session.save_json(), before);
        assert!(!app.show_view);
    }

    #[test]
    fn retained_label_and_tooltip_shortcuts_work() {
        let ctx = egui::Context::default();
        let mut app = PretextApp::initial_state();
        let _ = ctx.run(input(vec![
            key_event(egui::Key::N, egui::Modifiers::NONE),
            key_event(egui::Key::T, egui::Modifiers::NONE),
        ]), |ctx| app.keys(ctx));
        assert!(!app.session.labels_on);
        assert!(!app.session.tooltip_on);
    }

    #[test]
    fn panning_respects_inverted_mouse_buttons_without_editing() {
        for invert in [false, true] {
            let ctx = egui::Context::default();
            let mut app = PretextApp::initial_state();
            app.session = Session::demo();
            app.session.invert_mouse = invert;
            let before = app.session.to_agp();
            let camera = app.session.camera;
            let pan = if invert { egui::PointerButton::Primary } else { egui::PointerButton::Secondary };
            pointer_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(200.0, 200.0))]);
            pointer_frame(&mut app, &ctx, vec![pointer_button(pan, true)]);
            pointer_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(250.0, 230.0))]);
            pointer_frame(&mut app, &ctx, vec![pointer_button(pan, false)]);
            assert_ne!(app.session.camera.x, camera.x);
            assert_ne!(app.session.camera.y, camera.y);
            assert_eq!(app.session.to_agp(), before);
            assert!(app.selection.is_empty());
            assert!(app.session.edits.is_empty());
        }
    }

    #[test]
    fn clicking_dragging_and_middle_click_cannot_edit_or_select() {
        let ctx = egui::Context::default();
        let mut app = PretextApp::initial_state();
        app.session = Session::demo();
        app.session.mode = Mode::Edit;
        app.hover_pixel = Some(10);
        let before = app.session.save_json();
        for button in [egui::PointerButton::Primary, egui::PointerButton::Middle] {
            pointer_frame(&mut app, &ctx, vec![
                egui::Event::PointerMoved(Pos2::new(200.0, 200.0)),
                pointer_button(button, true),
            ]);
            pointer_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(250.0, 230.0))]);
            pointer_frame(&mut app, &ctx, vec![pointer_button(button, false)]);
        }
        assert_eq!(app.session.save_json(), before);
        assert!(app.selection.is_empty());
        assert!(app.press.is_none());
    }

    #[test]
    fn dropped_state_and_agp_files_are_rejected() {
        let mut app = PretextApp::initial_state();
        let before = app.session.save_json();
        for name in ["state.json", "assembly.agp"] {
            app.ingest(name, Vec::new());
            assert!(app.status.contains("Only .pretext"));
            assert_eq!(app.session.save_json(), before);
            assert!(app.loader.is_none());
        }
    }

    #[test]
    fn loading_a_map_stays_in_viewing_mode() {
        use std::io::Write;

        fn deflate(bytes: &[u8]) -> Vec<u8> {
            let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(bytes).unwrap();
            encoder.finish().unwrap()
        }

        let mut header = Vec::new();
        header.extend_from_slice(&1000u64.to_le_bytes());
        header.extend_from_slice(&1u32.to_le_bytes());
        header.extend_from_slice(&1.0f32.to_le_bytes());
        let mut name = [0u8; 64];
        name[..4].copy_from_slice(b"chr1");
        header.extend_from_slice(&name);
        header.extend_from_slice(&[2, 0, 1]);
        let compressed = deflate(&header);
        let tile = deflate(&[200, 0, 0, 0, 0, 0, 0, 0]);
        let mut bytes = b"pstm".to_vec();
        bytes.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(header.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&compressed);
        bytes.extend_from_slice(&(tile.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&tile);

        let mut app = PretextApp::initial_state();
        app.ingest("tiny.PRETEXT", bytes);
        assert!(app.loader.is_some(), "{}", app.status);
        assert_eq!(app.session.mode, Mode::Normal);
        let ctx = egui::Context::default();
        for _ in 0..10 {
            app.pump_loader(&ctx);
            if app.loader.is_none() {
                break;
            }
        }
        assert!(app.loader.is_none(), "map loading did not finish");
        assert_eq!(app.session.mode, Mode::Normal);
        assert_eq!(app.session.fragments.len(), 1);
        assert_eq!(app.session.fragments[0].name, "chr1");
        assert_eq!(app.session.map_pixels, 4);
        assert!(app.status.contains("Loaded tiny.PRETEXT"), "{}", app.status);
        assert!(!app.status.contains("pick it up"));
    }

    #[test]
    fn scroll_still_zooms_the_map() {
        let ctx = egui::Context::default();
        let mut app = PretextApp::initial_state();
        let zoom = app.session.camera.zoom;
        pointer_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(200.0, 200.0))]);
        pointer_frame(&mut app, &ctx, vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 100.0),
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(app.session.camera.zoom > zoom);
    }
}

#[cfg(test)]
mod url_tests {
    use super::{accept_map_url, map_name_from_url};

    #[test]
    fn https_link_keeps_the_file_name() {
        let url = accept_map_url(
            "  https://bucket.s3.amazonaws.com/maps/gf.pretext?X-Amz-Signature=abc  ",
        )
        .unwrap();
        assert_eq!(map_name_from_url(&url), "gf.pretext");
    }

    #[test]
    fn other_schemes_are_refused() {
        assert!(accept_map_url("ftp://files.example/map.pretext").is_err());
        assert!(accept_map_url("http://example.com/map.pretext").is_err());
        assert!(accept_map_url("s3://bucket/map.pretext").is_err());
        assert!(accept_map_url("   ").is_err());
    }
}
