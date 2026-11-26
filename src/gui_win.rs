//! The Repo Status window on Windows (egui, drawn with wgpu): the same window as the GTK 4 one on
//! Linux. One tab per tab in the repos file, one card per repo; checks run on a worker thread.

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::config::{self, Sort, Tab};
use crate::report::{self, Light};
use crate::view::{self, CardView, Checks, Done, TabView};

const ICON_PNG: &[u8] = include_bytes!("../data/slashrepo-256.png");

fn rgb((r, g, b): (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn colour(l: Light) -> Color32 {
    rgb(view::rgb(l))
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
        App { error, snapshot, ..App::with_tabs(tabs, Checks::default()) }
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
            Action::Refresh => self.checks.clear(),
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

    fn header(&mut self, ui: &mut egui::Ui, act: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.checking(), egui::Button::new("Refresh")).clicked() {
                act.push(Action::Refresh);
            }
            ui.label(RichText::new(self.checks.status_line(&self.tabs)).small());
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
                let v = self.checks.tab(&self.tabs[i]);
                ui.label(RichText::new("●").color(colour(v.dot)));
                if ui.selectable_label(i == self.current, &v.name).clicked() {
                    self.current = i;
                }
                ui.add_space(8.0);
            }
        });
    }

    fn bar(&mut self, ui: &mut egui::Ui, tab: &TabView, repo_count: usize, act: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            let text = self.entry.entry(tab.name.clone()).or_default();
            let width = (ui.available_width() - 290.0).max(160.0);
            let field = ui.add(egui::TextEdit::singleline(text).hint_text(view::ADD_HINT).desired_width(width));
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
            if ui.button("Remove tab").clicked() {
                self.confirm = Some((tab.name.clone(), repo_count));
            }
        });
        if self.bad_entry.as_deref() == Some(&tab.name) {
            ui.colored_label(colour(Light::Red), view::BAD_REPO);
        }
    }

    fn open_link(ui: &mut egui::Ui, text: RichText, url: &str) {
        if ui.link(text).on_hover_text(format!("Open {url}")).clicked() && report::openable(url) {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
    }

    fn card(ui: &mut egui::Ui, c: &CardView, tab: &str, act: &mut Vec<Action>) {
        let mut frame = egui::Frame::group(ui.style()).inner_margin(egui::Margin::symmetric(10, 8));
        if let Some((_, light)) = c.visibility {
            frame = frame.stroke(egui::Stroke::new(2.0, colour(light)));
        }
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").size(18.0).color(colour(c.dot)));
                Self::open_link(ui, RichText::new(&c.repo).strong().underline().color(rgb(view::LINK)), &c.url);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").on_hover_text("Remove from this tab").clicked() {
                        let (repo, tab) = (c.repo.clone(), tab.to_string());
                        act.push(Action::Edit(Box::new(move |l| config::remove_repo(l, &repo, Some(&tab)))));
                    }
                    for (text, tip, step, can) in
                        [("▼", "Move down", 1isize, c.can_down), ("▲", "Move up", -1, c.can_up)]
                    {
                        if ui.add_enabled(can, egui::Button::new(text).small()).on_hover_text(tip).clicked() {
                            let (repo, tab) = (c.repo.clone(), tab.to_string());
                            act.push(Action::Edit(Box::new(move |l| config::move_repo(l, &repo, &tab, step))));
                        }
                    }
                });
            });
            let Some(status) = &c.status else {
                ui.label(RichText::new(view::CHECKING).small());
                return;
            };
            ui.label(RichText::new(status).small());
            for line in &c.lines {
                match &line.url {
                    Some(u) => Self::open_link(ui, RichText::new(&line.text).color(rgb(view::LINK)), u),
                    None => {
                        ui.label(&line.text);
                    }
                }
            }
            for t in &c.todo {
                ui.label(RichText::new(t).strong());
            }
            // live/private in the bottom-right corner, under the arrows and ✕. Inside a row: a
            // right-to-left layout on its own takes all the height left in the scroll area (#68)
            if let Some((word, light)) = c.visibility {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(word).small().strong().color(colour(light)));
                    });
                });
            }
        });
    }

    fn page(&mut self, ui: &mut egui::Ui, act: &mut Vec<Action>) {
        let Some(tab) = self.tabs.get(self.current) else {
            ui.label(view::NO_TABS);
            return;
        };
        let (v, count) = (self.checks.tab(tab), tab.repos.len());
        self.bar(ui, &v, count, act);
        ui.label(RichText::new(&v.totals).small());
        ui.add_space(6.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for c in &v.cards {
                Self::card(ui, c, &v.name, act);
                ui.add_space(10.0);
            }
            if v.cards.is_empty() {
                ui.label(view::NO_REPOS);
            }
        });
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
            alerts: 0,
            private,
        };
        Card { repo: repo.into(), todo: vec!["nothing to do".into()], result: Ok(d) }
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
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 760.0)).build_eframe(|_| app());
        h.run();
        h
    }

    #[test]
    fn cards_show_their_lines_and_border_words() {
        let h = harness();
        h.get_by_label("me/a");
        h.get_by_label("me/b");
        assert_eq!(h.query_all_by_label("→ nothing to do").count(), 2);
        h.get_by_label("live");
        h.get_by_label("private");
        h.get_by_label_contains("checked 09:30 — 3 repos in 2 tab(s)");
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
        h.get_by_label("Its 2 repo(s) leave the list. Nothing changes on GitHub.");
        h.get_by_label("Cancel").click();
        h.run();
        assert!(h.query_by_label("Remove the tab “Mine”?").is_none());
        assert_eq!(h.state().tabs.len(), 2);
    }

    #[test]
    fn a_click_on_a_tab_shows_its_cards() {
        let mut h = harness();
        h.get_by_label("Theirs").click();
        h.run();
        assert_eq!(h.state().current, 1);
        h.get_by_label("you/c");
        assert!(h.query_by_label("me/a").is_none());
    }
}
