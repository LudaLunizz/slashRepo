//! The Repo Status window on Windows (egui, drawn with wgpu): the same window as the GTK 4 one on
//! Linux. One tab per tab in the repos file, one card per repo; checks run on a worker thread.

use std::collections::{HashMap, HashSet};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Sense, Stroke};

use crate::config::{self, Sort, Tab};
use crate::report::{self, Light};
use crate::view::{self, CardView, Checks, Done, Group, Row, TabView, palette};

const ICON_PNG: &[u8] = include_bytes!("../data/slashrepo-256.png");

const BODY: f32 = 14.0;
const SMALL: f32 = 12.0;
const NAME: f32 = 16.0;

fn rgb((r, g, b): view::Rgb) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn colour(l: Light) -> Color32 {
    rgb(view::rgb(l))
}

/// A tone at `alpha` on a card: pill and to-do backgrounds.
fn tint(l: Light, alpha: f32) -> Color32 {
    rgb(view::mix(view::rgb(l), palette::CARD, alpha))
}

/// The bold face (Segoe UI Bold when Windows has it; see fonts()).
fn bold() -> egui::FontFamily {
    egui::FontFamily::Name("bold".into())
}

fn strong(text: impl Into<String>) -> RichText {
    RichText::new(text).family(bold())
}

fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(rgb(palette::MUTED))
}

fn format(size: f32, color: Color32, family: egui::FontFamily) -> TextFormat {
    TextFormat { font_id: egui::FontId::new(size, family), color, ..Default::default() }
}

/// The round status mark.
fn dot(ui: &mut egui::Ui, l: Light) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, colour(l));
}

/// A rounded word on a tinted ground: the counts at the top, live/private on a card.
fn pill(ui: &mut egui::Ui, words: &str, l: Light, size: f32) {
    egui::Frame::new()
        .fill(tint(l, 0.16))
        .corner_radius(CornerRadius::same(99))
        .inner_margin(egui::Margin::symmetric(10, 2))
        .show(ui, |ui| ui.add(egui::Label::new(strong(words).size(size).color(colour(l))).selectable(false)));
}

/// An issue label.
fn tag(ui: &mut egui::Ui, words: &str) {
    egui::Frame::new()
        .stroke(Stroke::new(1.0, rgb(palette::BORDER)))
        .corner_radius(CornerRadius::same(99))
        .inner_margin(egui::Margin::symmetric(7, 0))
        .show(ui, |ui| ui.add(egui::Label::new(muted(words).size(SMALL)).selectable(false)));
}

/// A frameless button for ▲ ▼ ✕ and "Show 3 more".
fn quiet_button(ui: &mut egui::Ui, text: RichText, enabled: bool) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(text).frame(false))
}

type Change = Box<dyn FnOnce(&[String]) -> Vec<String>>;

/// What a click asks for; applied after the frame is drawn (the drawing borrows the state).
enum Action {
    Edit(Change),
    Refresh,
    NewTab(String),
    /// open or close a folded list ("repo\nPull requests")
    Fold(String),
}

struct App {
    tabs: Vec<Tab>,
    current: usize,
    checks: Checks,
    done: mpsc::Receiver<Done>,
    send: mpsc::Sender<Done>,
    /// the add-repo field of each tab
    entry: HashMap<String, String>,
    /// tab whose add field holds something that is not a repo
    bad_entry: Option<String>,
    new_tab: String,
    /// (tab, its repo count) waiting for "Remove the tab?"
    confirm: Option<(String, usize)>,
    error: Option<String>,
    /// folded lists the user opened; they stay open over a refresh
    unfolded: HashSet<String>,
    /// SLASHREPO_SNAPSHOT: save the window there once every repo is checked, then quit
    snapshot: Option<std::path::PathBuf>,
    idle_since: Option<Instant>,
    shot_asked: bool,
}

impl App {
    fn new() -> Self {
        // a repos file that cannot be read shows why and starts with no tabs; nothing is rebuilt from it
        let (tabs, error) = match config::load_tabs() {
            Ok(tabs) => (tabs, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let snapshot = std::env::var_os("SLASHREPO_SNAPSHOT").map(Into::into);
        // SLASHREPO_SNAPSHOT_TAB=1: the picture shows the second tab
        let current = std::env::var("SLASHREPO_SNAPSHOT_TAB").ok().and_then(|t| t.parse().ok()).unwrap_or(0);
        App { error, snapshot, current, ..App::with_tabs(tabs, Checks::default()) }
    }

    /// A window over given tabs: no repos file read, no snapshot (the tests start here).
    fn with_tabs(tabs: Vec<Tab>, checks: Checks) -> Self {
        let (send, done) = mpsc::channel();
        App {
            tabs,
            current: 0,
            checks,
            done,
            send,
            entry: HashMap::new(),
            bad_entry: None,
            new_tab: String::new(),
            confirm: None,
            error: None,
            unfolded: HashSet::new(),
            snapshot: None,
            idle_since: None,
            shot_asked: false,
        }
    }

    fn checking(&self) -> bool {
        self.checks.checking(&self.tabs)
    }

    /// Collect finished checks, start the ones not asked yet.
    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.done.try_recv() {
            self.checks.apply(done, crate::win::local_hhmm());
        }
        let todo = self.checks.start(&self.tabs);
        if todo.is_empty() {
            return;
        }
        let (send, ctx) = (self.send.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = send.send(view::run_checks(todo));
            ctx.request_repaint();
        });
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Refresh => {
                if !self.checks.busy() {
                    self.checks.clear();
                    self.error = None;
                }
            }
            Action::Fold(key) => {
                if !self.unfolded.remove(&key) {
                    self.unfolded.insert(key);
                }
                return;
            }
            Action::Edit(change) => {
                if let Err(e) = config::edit(change) {
                    self.error = Some(view::edit_failed(&e));
                    return;
                }
                self.error = None;
            }
            Action::NewTab(name) => {
                let n = name.clone();
                if let Err(e) = config::edit(move |l| config::add_tab(l, &n)) {
                    self.error = Some(view::edit_failed(&e));
                    return;
                }
                self.reload_tabs();
                self.current = self.tabs.iter().position(|t| t.name == name).unwrap_or(self.current);
                return;
            }
        }
        self.reload_tabs();
        self.current = self.current.min(self.tabs.len().saturating_sub(1));
    }

    /// Read the repos file again; if it cannot be read the old tabs stay and the error shows.
    fn reload_tabs(&mut self) {
        match config::load_tabs() {
            Ok(tabs) => self.tabs = tabs,
            Err(e) => self.error = Some(e),
        }
    }

    /// The count pills, the "checked 09:30 · …" line and Refresh.
    fn header(&mut self, ui: &mut egui::Ui, act: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            for (words, light) in self.checks.summary(&self.tabs).pills() {
                pill(ui, &words, light, BODY);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let refresh = ui
                    .add_enabled(!self.checking(), egui::Button::new("Refresh"))
                    .on_hover_text("Check every repo again (F5 or Ctrl+R)");
                if refresh.clicked() {
                    act.push(Action::Refresh);
                }
                ui.add(egui::Label::new(muted(self.checks.status_line(&self.tabs))).truncate());
            });
        });
        if crate::demo::on() {
            ui.colored_label(colour(Light::Orange), view::DEMO_NOTE);
        }
        if let Some(e) = &self.error {
            ui.colored_label(colour(Light::Red), e);
        }
    }

    /// One button per tab (dot, name, repo count), then "+ New tab".
    fn tab_strip(&mut self, ui: &mut egui::Ui, act: &mut Vec<Action>) {
        ui.horizontal_wrapped(|ui| {
            for i in 0..self.tabs.len() {
                let v = self.checks.tab(&self.tabs[i]);
                let open = i == self.current;
                let (ink, face) = if open {
                    (rgb(palette::TEXT), bold())
                } else {
                    (rgb(palette::MUTED), egui::FontFamily::Proportional)
                };
                let mut job = LayoutJob::default();
                job.append("●  ", 0.0, format(SMALL, colour(v.dot), egui::FontFamily::Proportional));
                job.append(&v.name, 0.0, format(BODY, ink, face));
                job.append(
                    &format!("   {}", v.cards.len()),
                    0.0,
                    format(SMALL, rgb(palette::MUTED), egui::FontFamily::Proportional),
                );
                if ui.add(egui::Button::selectable(open, job)).clicked() {
                    self.current = i;
                }
            }
            ui.menu_button(muted("+ New tab"), |ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut self.new_tab).hint_text("Tab name, then Enter"));
                if ui.memory(|m| m.focused().is_none()) {
                    field.request_focus();
                }
                if field.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Some(name) = config::tab_name(&self.new_tab)
                {
                    act.push(Action::NewTab(name));
                    self.new_tab.clear();
                    ui.close();
                }
            });
        });
    }

    fn add_field() -> egui::Id {
        egui::Id::unique("add-repo")
    }

    fn bar(&mut self, ui: &mut egui::Ui, tab: &TabView, repo_count: usize, act: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            let text = self.entry.entry(tab.name.clone()).or_default();
            let width = (ui.available_width() - 330.0).max(160.0);
            let field = ui.add(
                egui::TextEdit::singleline(text)
                    .id(Self::add_field())
                    .hint_text(view::ADD_HINT)
                    .desired_width(width)
                    .margin(egui::Margin::symmetric(8, 5)),
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Add repo").clicked() || enter {
                match config::repo_name(text) {
                    Some(repo) => {
                        let name = tab.name.clone();
                        act.push(Action::Edit(Box::new(move |l| config::add_repo(l, &repo, &name))));
                        text.clear();
                        self.bad_entry = None;
                    }
                    None => self.bad_entry = Some(tab.name.clone()),
                }
            }
            let mut sel = tab.sort;
            egui::ComboBox::from_id_salt(("sort", &tab.name))
                .selected_text(view::SORT_LABELS[sel])
                .show_ui(ui, |ui| {
                    for (k, l) in view::SORT_LABELS.iter().enumerate() {
                        ui.selectable_value(&mut sel, k, *l);
                    }
                })
                .response
                .on_hover_text("Order of this tab");
            if sel != tab.sort {
                let (name, s) = (tab.name.clone(), Sort::ALL[sel.min(2)]);
                act.push(Action::Edit(Box::new(move |l| config::set_sort(l, &name, s))));
            }
            let remove = ui
                .add(egui::Button::new(muted("Remove tab")).fill(Color32::TRANSPARENT))
                .on_hover_text("Take this tab and its repos off the list");
            if remove.clicked() {
                self.confirm = Some((tab.name.clone(), repo_count));
            }
        });
        if self.bad_entry.as_deref() == Some(&tab.name) {
            ui.colored_label(colour(Light::Red), view::BAD_REPO);
        }
    }

    /// A label that opens `url` on a click: pointing hand, underlined under the pointer.
    fn open_link(ui: &mut egui::Ui, label: egui::Label, url: &str, tip: &str) {
        let r = ui.add(label.sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
        if r.hovered() {
            ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, rgb(view::LINK)));
        }
        if r.clicked() && report::openable(url) {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
    }

    /// One PR or issue: mark, number, title (cut with … when the window is narrow), labels, details.
    fn row(ui: &mut egui::Ui, r: &Row) {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            if let Some(l) = r.tone {
                dot(ui, l);
            }
            ui.label(muted(&r.id));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(6.0);
                ui.label(muted(&r.meta));
                for l in r.labels.iter().rev() {
                    tag(ui, l);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let title = egui::Label::new(&r.title).truncate();
                    match &r.url {
                        Some(u) => Self::open_link(ui, title, u, &format!("{} {}\nOpen {u}", r.id, r.title)),
                        None => {
                            ui.add(title);
                        }
                    }
                });
            });
        });
    }

    /// "PULL REQUESTS · 6", the first rows, and the rest behind "Show 3 more".
    fn group(&self, ui: &mut egui::Ui, repo: &str, g: &Group, act: &mut Vec<Action>) {
        ui.add_space(2.0);
        ui.label(strong(g.title.to_uppercase()).size(SMALL - 0.5).color(rgb(palette::MUTED)));
        let key = format!("{repo}\n{}", g.title.split(" · ").next().unwrap_or_default());
        let open = self.unfolded.contains(&key);
        let shown = if open { g.rows.len() } else { g.rows.len().min(view::FOLD) };
        for r in &g.rows[..shown] {
            Self::row(ui, r);
        }
        if g.rows.len() > view::FOLD {
            let words = view::fold_text(g.rows.len() - view::FOLD, open);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                if quiet_button(ui, RichText::new(words).color(rgb(view::LINK)), true).clicked() {
                    act.push(Action::Fold(key));
                }
            });
        }
        if let Some(m) = &g.more {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(muted(m));
            });
        }
    }

    /// The to-do on the left, live/private in the bottom-right corner beside it. All in one row: a
    /// right-to-left layout on its own takes all the height left in the scroll area (#68).
    fn foot(ui: &mut egui::Ui, c: &CardView) {
        ui.horizontal(|ui| {
            if c.clear {
                ui.label(muted(format!("✓ {}", view::NOTHING_TO_DO)));
            } else if !c.todo.is_empty() {
                let room = if c.visibility.is_some() { 84.0 } else { 0.0 };
                let width = (ui.available_width() - room - 23.0).max(120.0);
                let box_ = egui::Frame::new()
                    .fill(tint(c.dot, 0.09))
                    .corner_radius(CornerRadius { nw: 0, sw: 0, ne: 6, se: 6 })
                    .inner_margin(egui::Margin { left: 13, right: 10, top: 6, bottom: 6 })
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            for t in &c.todo {
                                ui.label(strong(format!("→ {t}")));
                            }
                        });
                    });
                let edge = box_.response.rect;
                let bar = egui::Rect::from_min_max(edge.min, egui::pos2(edge.min.x + 3.0, edge.max.y));
                ui.painter().rect_filled(bar, CornerRadius::ZERO, colour(c.dot));
            }
            if let Some((word, light)) = c.visibility {
                ui.with_layout(Layout::right_to_left(Align::Max), |ui| pill(ui, word, light, SMALL));
            }
        });
    }

    fn card(&self, ui: &mut egui::Ui, c: &CardView, tab: &str, act: &mut Vec<Action>) {
        let frame = egui::Frame::new()
            .fill(rgb(palette::CARD))
            .stroke(Stroke::new(1.0, rgb(view::border(c.visibility.map(|(_, light)| light)))))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(egui::Margin::symmetric(14, 12));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                dot(ui, c.dot);
                let (owner, name) = view::split_name(&c.repo);
                let mut job = LayoutJob::default();
                job.append(owner, 0.0, format(NAME, rgb(palette::MUTED), egui::FontFamily::Proportional));
                job.append(name, 0.0, format(NAME, rgb(palette::TEXT), bold()));
                Self::open_link(ui, egui::Label::new(job), &c.url, &format!("Open {}", c.url));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let grey = |glyph: &str| RichText::new(glyph).color(colour(Light::Grey));
                    if quiet_button(ui, grey("✕"), true).on_hover_text("Remove from this tab").clicked() {
                        let (repo, tab) = (c.repo.clone(), tab.to_string());
                        act.push(Action::Edit(Box::new(move |l| config::remove_repo(l, &repo, Some(&tab)))));
                    }
                    for (glyph, tip, step, can) in
                        [("▼", "Move down", 1isize, c.can_down), ("▲", "Move up", -1, c.can_up)]
                    {
                        if quiet_button(ui, grey(glyph), can).on_hover_text(tip).clicked() {
                            let (repo, tab) = (c.repo.clone(), tab.to_string());
                            act.push(Action::Edit(Box::new(move |l| config::move_repo(l, &repo, &tab, step))));
                        }
                    }
                });
            });
            if c.checking {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(muted(view::CHECKING));
                });
                return;
            }
            if let Some(e) = &c.error {
                ui.colored_label(colour(Light::Red), e);
            }
            if !c.facts.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    for f in &c.facts {
                        let plain = egui::FontFamily::Proportional;
                        let mut job = LayoutJob::default();
                        job.append(&f.label, 0.0, format(BODY, rgb(palette::MUTED), plain.clone()));
                        match f.tone {
                            Light::Grey => job.append(&f.value, 5.0, format(BODY, rgb(palette::TEXT), plain)),
                            l => {
                                job.append("●", 5.0, format(SMALL, colour(l), plain.clone()));
                                job.append(&f.value, 4.0, format(BODY, colour(l), plain));
                            }
                        }
                        // one fact never breaks in two: the row wraps between facts
                        ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
                    }
                });
            }
            for g in &c.groups {
                self.group(ui, &c.repo, g, act);
            }
            ui.add_space(2.0);
            Self::foot(ui, c);
        });
    }

    fn page(&mut self, ui: &mut egui::Ui, act: &mut Vec<Action>) {
        let Some(tab) = self.tabs.get(self.current) else {
            ui.add_space(24.0);
            ui.label(muted(view::NO_TABS));
            return;
        };
        let (v, count) = (self.checks.tab(tab), tab.repos.len());
        self.bar(ui, &v, count, act);
        ui.label(muted(&v.totals));
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for c in &v.cards {
                self.card(ui, c, &v.name, act);
                ui.add_space(6.0);
            }
            if v.cards.is_empty() {
                ui.add_space(24.0);
                ui.label(muted(view::NO_REPOS));
            }
        });
    }

    /// F5 / Ctrl+R refresh, Ctrl+L the add field, Ctrl+PageUp/PageDown the tabs, Ctrl+W close.
    fn keys(&mut self, ctx: &egui::Context, act: &mut Vec<Action>) {
        use egui::{Key, Modifiers};
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if pressed(Modifiers::NONE, Key::F5) || pressed(Modifiers::CTRL, Key::R) {
            act.push(Action::Refresh);
        }
        if pressed(Modifiers::CTRL, Key::L) {
            ctx.memory_mut(|m| m.request_focus(Self::add_field()));
        }
        if pressed(Modifiers::CTRL, Key::PageDown) && self.current + 1 < self.tabs.len() {
            self.current += 1;
        }
        if pressed(Modifiers::CTRL, Key::PageUp) {
            self.current = self.current.saturating_sub(1);
        }
        if pressed(Modifiers::CTRL, Key::W) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context, act: &mut Vec<Action>) {
        let Some((name, n)) = self.confirm.clone() else { return };
        let (question, detail) = view::remove_tab_question(&name, n);
        let modal = egui::Modal::new(egui::Id::unique("remove-tab")).show(ctx, |ui| {
            ui.heading(question);
            ui.label(detail);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    self.confirm = None;
                }
                if ui.button("Remove").clicked() {
                    let tab = name.clone();
                    act.push(Action::Edit(Box::new(move |l| config::remove_tab(l, &tab))));
                    self.confirm = None;
                }
            });
        });
        if modal.should_close() {
            self.confirm = None;
        }
    }

    /// Developer aid (README pictures, CI): with SLASHREPO_SNAPSHOT=FILE.png the window saves itself
    /// to FILE once every repo is checked, then quits.
    fn snapshot_if_asked(&mut self, ctx: &egui::Context) {
        let Some(file) = self.snapshot.clone() else { return };
        if self.checking() || self.shot_asked {
            self.idle_since = None;
            return;
        }
        let since = *self.idle_since.get_or_insert_with(Instant::now);
        if since.elapsed() < Duration::from_millis(800) {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        self.shot_asked = true;
        let c = ctx.clone();
        ctx.request_screenshot(move |img| {
            if let Err(e) = save_png(&file, &img) {
                eprintln!("snapshot failed: {e}");
            }
            c.send_viewport_cmd(egui::ViewportCommand::Close);
            c.request_repaint();
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump(&ctx);
        let mut act = Vec::new();
        self.keys(&ctx, &mut act);
        let plain =
            |x: i8, y: i8| egui::Frame::new().fill(rgb(palette::BG)).inner_margin(egui::Margin::symmetric(x, y));
        egui::Panel::top("header").frame(plain(16, 10)).show_separator_line(false).show(ui, |ui| {
            self.header(ui, &mut act);
            ui.add_space(4.0);
            self.tab_strip(ui, &mut act);
        });
        egui::CentralPanel::default().frame(plain(16, 4)).show(ui, |ui| self.page(ui, &mut act));
        self.confirm_dialog(&ctx, &mut act);
        for a in act {
            self.apply(a);
        }
        self.snapshot_if_asked(&ctx);
        if self.checking() {
            // keep "checking…" honest while a worker runs (it also wakes us when done)
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}

fn save_png(file: &std::path::Path, img: &egui::ColorImage) -> Result<(), String> {
    let f = std::fs::File::create(file).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(img.as_raw()).map_err(|e| e.to_string())
}

fn icon() -> Option<egui::IconData> {
    let mut r = png::Decoder::new(std::io::Cursor::new(ICON_PNG)).read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    (info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight).then(|| {
        buf.truncate(info.buffer_size());
        egui::IconData { rgba: buf, width: info.width, height: info.height }
    })
}

/// Segoe UI for text, Segoe UI Bold for the "bold" family and Segoe UI Symbol for ● ▲ ▼ ✕ ★ → ✓,
/// from the Windows fonts folder; egui's own fonts stay as the fallback (then bold is not bolder).
fn fonts(ctx: &egui::Context) {
    let dir = std::env::var_os("WINDIR").map_or_else(|| "C:\\Windows".into(), std::path::PathBuf::from).join("Fonts");
    let mut defs = egui::FontDefinitions::default();
    let mut load = |name: &str, file: &str| {
        let data = std::fs::read(dir.join(file)).ok()?;
        defs.font_data.insert(name.into(), egui::FontData::from_owned(data).into());
        Some(name.to_string())
    };
    let text = load("segoe-ui", "segoeui.ttf");
    let heavy = load("segoe-ui-bold", "segoeuib.ttf");
    let symbol = load("segoe-ui-symbol", "seguisym.ttf");
    let front: Vec<String> = text.iter().chain(&symbol).cloned().collect();
    let mut prop = defs.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    prop.splice(0..0, front);
    let mut bold_list = prop.clone();
    bold_list.splice(0..0, heavy);
    defs.families.insert(egui::FontFamily::Proportional, prop);
    defs.families.insert(bold(), bold_list);
    if let (Some(sym), Some(mono)) = (symbol, defs.families.get_mut(&egui::FontFamily::Monospace)) {
        mono.push(sym);
    }
    ctx.set_fonts(defs);
}

/// The window's own look, from view::palette (the GTK window uses the same numbers).
fn style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        use egui::{FontId, TextStyle};
        for (style, size) in
            [(TextStyle::Body, BODY), (TextStyle::Button, BODY), (TextStyle::Small, SMALL), (TextStyle::Heading, 18.0)]
        {
            s.text_styles.insert(style, FontId::proportional(size));
        }
        s.spacing.item_spacing = egui::vec2(8.0, 7.0);
        s.spacing.button_padding = egui::vec2(11.0, 5.0);
        s.spacing.interact_size.y = 28.0;
        let (text, border) = (rgb(palette::TEXT), Stroke::new(1.0, rgb(palette::BORDER)));
        let hover = rgb(view::mix(palette::TEXT, palette::RAISED, 0.08));
        let v = &mut s.visuals;
        v.panel_fill = rgb(palette::BG);
        v.window_fill = rgb(palette::CARD);
        v.window_stroke = border;
        v.extreme_bg_color = rgb(palette::FIELD);
        v.text_edit_bg_color = Some(rgb(palette::FIELD));
        v.faint_bg_color = rgb(palette::CARD);
        v.hyperlink_color = rgb(view::LINK);
        v.selection.bg_fill = rgb(view::mix(view::LINK, palette::BG, 0.4));
        v.selection.stroke = Stroke::new(1.0, text);
        v.widgets.noninteractive.fg_stroke.color = text;
        v.widgets.noninteractive.bg_stroke = border;
        let w = &mut v.widgets;
        for (state, fill, edge) in [
            (&mut w.inactive, rgb(palette::RAISED), border),
            (&mut w.hovered, hover, Stroke::new(1.0, rgb(palette::MUTED))),
            (&mut w.active, hover, Stroke::new(1.0, rgb(view::LINK))),
            (&mut w.open, hover, border),
        ] {
            state.fg_stroke.color = text;
            (state.weak_bg_fill, state.bg_fill, state.bg_stroke) = (fill, fill, edge);
            state.corner_radius = CornerRadius::same(8);
            state.expansion = 0.0;
        }
    });
}

fn title() -> &'static str {
    if crate::demo::on() { "Repo Status (demo)" } else { "Repo Status" }
}

pub fn run() -> ExitCode {
    // the demo opens beside the real window instead of bringing that one to the front
    if !crate::demo::on() && crate::win::show_open_window(title()) {
        return ExitCode::SUCCESS;
    }
    let (width, height) = view::snapshot_size().unwrap_or((940, 800));
    let mut viewport =
        egui::ViewportBuilder::default().with_title(title()).with_inner_size([width as f32, height as f32]);
    if let Some(i) = icon() {
        viewport = viewport.with_icon(i);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let ran = eframe::run_native(
        title(),
        options,
        Box::new(|cc| {
            look(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    );
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("could not open the window: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Fonts and colours: what the window and the tests' harness both start with.
fn look(ctx: &egui::Context) {
    fonts(ctx);
    style(ctx);
}

/// The real window, driven through its accessibility tree (egui_kittest): no GPU, no network. Every
/// repo already has its card, so the window starts no check, and no test clicks anything that
/// writes the repos file.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Card, CheckState, Fetched};
    use egui_kittest::{Harness, kittest::Queryable};

    fn tab(name: &str, repos: &[&str]) -> Tab {
        Tab { name: name.into(), sort: Sort::Mine, repos: repos.iter().map(|r| r.to_string()).collect() }
    }

    fn card(repo: &str, private: bool) -> Card {
        let d = Fetched {
            url: format!("https://github.com/{repo}"),
            pushed: String::new(),
            stars: 1,
            forks: 0,
            branch: "main".into(),
            pr: "PR #",
            unreleased: None,
            ci: CheckState::Passed,
            release: None,
            prs: vec![],
            issues: vec![],
            issue_total: 0,
            pr_total: 0,
            alerts: 0,
            private,
        };
        Card::from_result(repo, Ok(d))
    }

    fn app() -> App {
        let tabs = vec![tab("Mine", &["me/a", "me/b"]), tab("Theirs", &["you/c"])];
        let mut checks = Checks::default();
        let asked = checks.start(&tabs);
        let found = [card("me/a", false), card("me/b", true), card("you/c", false)]
            .into_iter()
            .map(|c| (c.repo.clone(), c))
            .collect();
        checks.apply(Done { asked, found }, "09:30".into());
        App::with_tabs(tabs, checks)
    }

    fn harness() -> Harness<'static, App> {
        // the window's own fonts and colours (Segoe UI + Segoe UI Symbol for ● ▲ ★ → ✓): the harness stops on a
        // glyph no font has, so this also checks that the real font setup covers them
        let mut h =
            Harness::builder().with_size(egui::vec2(940.0, 800.0)).with_context_setup(look).build_eframe(|_| app());
        h.run();
        h
    }

    #[test]
    fn cards_show_their_lines_and_border_words() {
        let h = harness();
        h.get_by_label("me/a");
        h.get_by_label("me/b");
        assert_eq!(h.query_all_by_label("✓ Nothing to do").count(), 2);
        h.get_by_label("live");
        h.get_by_label("private");
        h.get_by_label_contains("checked 09:30 · 3 repos in 2 tabs");
        h.get_by_label("3 clear");
        assert!(h.query_by_label("you/c").is_none(), "only the open tab's cards are drawn");
    }

    /// #68: the live/private word took all the height left, so the first card filled the window.
    #[test]
    fn a_card_is_as_tall_as_its_lines() {
        let h = harness();
        let name = h.get_by_label("me/a").rect();
        let word = h.get_by_label("live").rect();
        let next = h.get_by_label("me/b").rect();
        assert!(word.min.y - name.min.y < 120.0, "live word {word:?} far below the name {name:?}");
        assert!(next.min.y - name.min.y < 200.0, "second card {next:?} pushed down by the first {name:?}");
    }

    #[test]
    fn remove_tab_asks_first_and_cancel_keeps_it() {
        let mut h = harness();
        h.get_by_label("Remove tab").click();
        h.run();
        h.get_by_label("Remove the tab “Mine”?");
        h.get_by_label("Its 2 repos leave the list. Nothing changes on the forge.");
        h.get_by_label("Cancel").click();
        h.run();
        assert!(h.query_by_label("Remove the tab “Mine”?").is_none());
        assert_eq!(h.state().tabs.len(), 2);
    }

    #[test]
    fn a_click_on_a_tab_shows_its_cards() {
        let mut h = harness();
        h.get_by_label_contains("Theirs").click();
        h.run();
        assert_eq!(h.state().current, 1);
        h.get_by_label("you/c");
        assert!(h.query_by_label("me/a").is_none());
    }
}
