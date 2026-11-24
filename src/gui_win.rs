//! The Repo Status window on Windows (egui, drawn with wgpu): the same window as the GTK 4 one on
//! Linux. One tab per tab in the repos file, one card per repo; checks run on a worker thread.

use std::collections::{HashMap, HashSet};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::config::{self, Sort, Tab};
use crate::report::{self, Card, Light};

const LINK: Color32 = Color32::from_rgb(0x53, 0x9b, 0xf5);
const SORT_LABELS: [&str; 3] = ["My order", "Stars", "Forks"];
const ICON_PNG: &[u8] = include_bytes!("../data/slashrepo-256.png");

fn colour(l: Light) -> Color32 {
    match l {
        Light::Red => Color32::from_rgb(0xe5, 0x53, 0x4b),
        Light::Orange => Color32::from_rgb(0xd2, 0x99, 0x22),
        Light::Green => Color32::from_rgb(0x57, 0xab, 0x5a),
        Light::Grey => Color32::from_rgb(0x76, 0x83, 0x90),
    }
}

type Change = Box<dyn FnOnce(&[String]) -> Vec<String>>;

/// What a click asks for; applied after the frame is drawn (the drawing borrows the state).
enum Action {
    Edit(Change),
    Refresh,
    NewTab(String),
}

struct App {
    tabs: Vec<Tab>,
    current: usize,
    /// repo -> its card; cleared by Refresh
    cards: HashMap<String, Card>,
    /// repos being checked right now
    busy: HashSet<String>,
    stamp: String,
    done: mpsc::Receiver<(Vec<String>, HashMap<String, Card>)>,
    send: mpsc::Sender<(Vec<String>, HashMap<String, Card>)>,
    /// the add-repo field of each tab
    entry: HashMap<String, String>,
    /// tab whose add field holds something that is not a repo
    bad_entry: Option<String>,
    new_tab: String,
    /// (tab, its repo count) waiting for "Remove the tab?"
    confirm: Option<(String, usize)>,
    error: Option<String>,
    /// SLASHREPO_SNAPSHOT: save the window there once every repo is checked, then quit
    snapshot: Option<std::path::PathBuf>,
    idle_since: Option<Instant>,
    shot_asked: bool,
}

impl App {
    fn new() -> Self {
        let (send, done) = mpsc::channel();
        // a repos file that cannot be read shows why and starts with no tabs; nothing is rebuilt from it
        let (tabs, error) = match config::load_tabs() {
            Ok(tabs) => (tabs, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        App {
            tabs,
            current: 0,
            cards: HashMap::new(),
            busy: HashSet::new(),
            stamp: String::new(),
            done,
            send,
            entry: HashMap::new(),
            bad_entry: None,
            new_tab: String::new(),
            confirm: None,
            error,
            snapshot: std::env::var_os("SLASHREPO_SNAPSHOT").map(Into::into),
            idle_since: None,
            shot_asked: false,
        }
    }

    fn checking(&self) -> bool {
        !self.busy.is_empty() || config::all_repos(&self.tabs).iter().any(|r| !self.cards.contains_key(r))
    }

    /// Collect finished checks, start the ones not asked yet.
    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok((asked, mut found)) = self.done.try_recv() {
            for r in asked {
                self.busy.remove(&r);
                // a crashed check must not leave the card "checking…" forever
                found.entry(r.clone()).or_insert_with(|| Card {
                    repo: r,
                    result: Err("the check crashed".into()),
                    todo: vec!["press Refresh to try again".into()],
                });
            }
            self.cards.extend(found);
            self.stamp = crate::win::local_hhmm();
        }
        let todo: Vec<String> = config::all_repos(&self.tabs)
            .into_iter()
            .filter(|r| !self.cards.contains_key(r) && !self.busy.contains(r))
            .collect();
        if todo.is_empty() {
            return;
        }
        self.busy.extend(todo.iter().cloned());
        let (send, ctx) = (self.send.clone(), ctx.clone());
        std::thread::spawn(move || {
            let found = std::panic::catch_unwind(|| report::check_all(&todo)).unwrap_or_default();
            let _ = send.send((todo, found));
            ctx.request_repaint();
        });
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Refresh => self.cards.clear(),
            Action::Edit(change) => {
                if let Err(e) = config::edit(change) {
                    self.error = Some(format!("Could not change {}: {e}", config::path().display()));
                    return;
                }
                self.error = None;
            }
            Action::NewTab(name) => {
                let n = name.clone();
                if let Err(e) = config::edit(move |l| config::add_tab(l, &n)) {
                    self.error = Some(format!("Could not change {}: {e}", config::path().display()));
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

    fn worst(&self, tab: &Tab) -> Light {
        tab.repos.iter().map(|r| self.cards.get(r).map_or(Light::Grey, Card::light)).max().unwrap_or(Light::Grey)
    }

    fn header(&mut self, ui: &mut egui::Ui, act: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.checking(), egui::Button::new("Refresh")).clicked() {
                act.push(Action::Refresh);
            }
            let repos = config::all_repos(&self.tabs).len();
            let mut status = String::new();
            if self.checking() {
                status += "checking… ";
            }
            if !self.stamp.is_empty() {
                status += &format!("checked {} — ", self.stamp);
            }
            status += &format!("{repos} repos in {} tab(s)", self.tabs.len());
            ui.label(RichText::new(status).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("New tab", |ui| {
                    let field = ui.add(egui::TextEdit::singleline(&mut self.new_tab).hint_text("Tab name"));
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
        });
        if let Some(e) = &self.error {
            ui.colored_label(colour(Light::Red), e);
        }
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for i in 0..self.tabs.len() {
                let tab = &self.tabs[i];
                ui.label(RichText::new("●").color(colour(self.worst(tab))));
                if ui.selectable_label(i == self.current, &tab.name).clicked() {
                    self.current = i;
                }
                ui.add_space(8.0);
            }
        });
    }

    fn bar(&mut self, ui: &mut egui::Ui, tab: &Tab, act: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            let text = self.entry.entry(tab.name.clone()).or_default();
            let width = (ui.available_width() - 290.0).max(160.0);
            let field = ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text("owner/name, or a GitHub, GitLab or Codeberg link")
                    .desired_width(width),
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
            let mut sel = Sort::ALL.iter().position(|s| *s == tab.sort).unwrap_or(0);
            let before = sel;
            egui::ComboBox::from_id_salt(("sort", &tab.name))
                .selected_text(SORT_LABELS[sel])
                .show_ui(ui, |ui| {
                    for (k, l) in SORT_LABELS.iter().enumerate() {
                        ui.selectable_value(&mut sel, k, *l);
                    }
                })
                .response
                .on_hover_text("Order of this tab");
            if sel != before {
                let (name, s) = (tab.name.clone(), Sort::ALL[sel.min(2)]);
                act.push(Action::Edit(Box::new(move |l| config::set_sort(l, &name, s))));
            }
            if ui.button("Remove tab").clicked() {
                self.confirm = Some((tab.name.clone(), tab.repos.len()));
            }
        });
        if self.bad_entry.as_deref() == Some(&tab.name) {
            ui.colored_label(
                colour(Light::Red),
                "Type owner/name (GitHub), gitlab.com/group/project, codeberg.org/owner/repo, or paste a link",
            );
        }
    }

    fn card(&self, ui: &mut egui::Ui, repo: &str, tab: &Tab, pos: usize, count: usize, act: &mut Vec<Action>) {
        let c = self.cards.get(repo);
        egui::Frame::group(ui.style()).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").size(18.0).color(colour(c.map_or(Light::Grey, Card::light))));
                let url = c.and_then(|c| c.ok()).map(|d| d.url.clone()).filter(|u| !u.is_empty());
                let url = url.unwrap_or_else(|| report::web_url(repo));
                let name = ui.link(RichText::new(repo).strong().underline().color(LINK));
                if name.on_hover_text(format!("Open {url}")).clicked() && report::openable(&url) {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").on_hover_text("Remove from this tab").clicked() {
                        let (repo, tab) = (repo.to_string(), tab.name.clone());
                        act.push(Action::Edit(Box::new(move |l| config::remove_repo(l, &repo, Some(&tab)))));
                    }
                    // own order only: under stars/forks the position comes from the numbers
                    for (text, tip, step) in [("▼", "Move down", 1isize), ("▲", "Move up", -1)] {
                        let can = tab.sort == Sort::Mine && pos.checked_add_signed(step).is_some_and(|p| p < count);
                        if ui.add_enabled(can, egui::Button::new(text).small()).on_hover_text(tip).clicked() {
                            let (repo, tab) = (repo.to_string(), tab.name.clone());
                            act.push(Action::Edit(Box::new(move |l| config::move_repo(l, &repo, &tab, step))));
                        }
                    }
                });
            });
            let Some(c) = c else {
                ui.label(RichText::new("checking…").small());
                return;
            };
            let lines = c.summary_lines();
            ui.label(RichText::new(&lines[0].0).small());
            for (line, url) in &lines[1..] {
                match url {
                    Some(u) => {
                        if ui.link(RichText::new(line).color(LINK)).on_hover_text(format!("Open {u}")).clicked()
                            && report::openable(u)
                        {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(u));
                        }
                    }
                    None => {
                        ui.label(format!("   {line}"));
                    }
                }
            }
            for t in &c.todo {
                ui.label(RichText::new(format!("→ {t}")).strong());
            }
        });
    }

    fn page(&mut self, ui: &mut egui::Ui, act: &mut Vec<Action>) {
        let Some(tab) = self.tabs.get(self.current).cloned() else {
            ui.label("No tabs yet: make one with “New tab”.");
            return;
        };
        self.bar(ui, &tab, act);
        ui.add_space(6.0);
        let repos = report::ordered(&tab.repos, tab.sort, &self.cards);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (pos, repo) in repos.iter().enumerate() {
                self.card(ui, repo, &tab, pos, repos.len(), act);
                ui.add_space(10.0);
            }
            if repos.is_empty() {
                ui.label("No repos in this tab yet. Add one above.");
            }
        });
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context, act: &mut Vec<Action>) {
        let Some((name, n)) = self.confirm.clone() else { return };
        let modal = egui::Modal::new(egui::Id::unique("remove-tab")).show(ctx, |ui| {
            ui.heading(format!("Remove the tab “{name}”?"));
            ui.label(format!("Its {n} repo(s) leave the list. Nothing changes on GitHub."));
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
        egui::Panel::top("header").show(ui, |ui| {
            ui.add_space(6.0);
            self.header(ui, &mut act);
            ui.add_space(4.0);
            self.tab_strip(ui);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ui, |ui| self.page(ui, &mut act));
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

/// Segoe UI for text and Segoe UI Symbol for ● ▲ ▼ ✕ ★ →, from the Windows fonts folder; egui's
/// own fonts stay as the fallback.
fn fonts(ctx: &egui::Context) {
    let dir = std::env::var_os("WINDIR").map_or_else(|| "C:\\Windows".into(), std::path::PathBuf::from).join("Fonts");
    let mut defs = egui::FontDefinitions::default();
    let mut front = Vec::new();
    for (name, file) in [("segoe-ui", "segoeui.ttf"), ("segoe-ui-symbol", "seguisym.ttf")] {
        if let Ok(data) = std::fs::read(dir.join(file)) {
            defs.font_data.insert(name.into(), egui::FontData::from_owned(data).into());
            front.push(name.to_string());
        }
    }
    if let Some(prop) = defs.families.get_mut(&egui::FontFamily::Proportional) {
        prop.splice(0..0, front.iter().cloned());
    }
    if let (Some(sym), Some(mono)) =
        (front.iter().find(|n| n.contains("symbol")), defs.families.get_mut(&egui::FontFamily::Monospace))
    {
        mono.push(sym.clone());
    }
    ctx.set_fonts(defs);
}

const TITLE: &str = "Repo Status";

pub fn run() -> ExitCode {
    if crate::win::show_open_window(TITLE) {
        return ExitCode::SUCCESS;
    }
    let mut viewport = egui::ViewportBuilder::default().with_title(TITLE).with_inner_size([900.0, 760.0]);
    if let Some(i) = icon() {
        viewport = viewport.with_icon(i);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let ran = eframe::run_native(
        TITLE,
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            fonts(&cc.egui_ctx);
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
