//! The Repo Status window (GTK 4): one notebook tab per tab in the repos file, one card per repo.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::process::ExitCode;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::config::{self, Sort, Tab};
use crate::report::{self, Card, Light};

const LINK: &str = "#539bf5";
const SORT_LABELS: [&str; 3] = ["My order", "Stars", "Forks"];

fn colour(l: Light) -> &'static str {
    match l {
        Light::Red => "#e5534b",
        Light::Orange => "#d29922",
        Light::Green => "#57ab5a",
        Light::Grey => "#768390",
    }
}

fn esc(t: &str) -> String {
    glib::markup_escape_text(t).to_string()
}

fn label(markup: &str, selectable: bool) -> gtk::Label {
    let lb = gtk::Label::builder().xalign(0.0).wrap(true).selectable(selectable).build();
    lb.set_markup(markup);
    lb
}

struct Ui {
    win: gtk::ApplicationWindow,
    book: gtk::Notebook,
    status: gtk::Label,
    refresh: gtk::Button,
    /// repo -> its card; cleared by Refresh
    cards: RefCell<HashMap<String, Card>>,
    /// repos being checked right now
    busy: RefCell<HashSet<String>>,
    stamp: RefCell<String>,
    /// render() is building pages: the sort dropdown's notify must not write the file then
    building: Cell<bool>,
}

/// A red line in the status bar.
fn show_error(ui: &Ui, msg: &str) {
    ui.status.set_markup(&format!("<span foreground='{}'>{}</span>", colour(Light::Red), esc(msg)));
}

/// Run an edit, then redraw, outside the signal of the widget the redraw destroys.
fn later(ui: &Weak<Ui>, change: impl FnOnce(&[String]) -> Vec<String> + 'static) {
    let ui = ui.clone();
    glib::idle_add_local_once(move || {
        if let Some(ui) = ui.upgrade() {
            if let Err(e) = config::edit(change) {
                show_error(&ui, &format!("Could not change {}: {e}", config::path().display()));
                return;
            }
            render(&ui);
        }
    });
}

/// A flat button that opens url: a link inside a selectable label starts a selection on click.
fn link(ui: &Rc<Ui>, markup: &str, url: &str) -> gtk::Button {
    let b = gtk::Button::builder()
        .has_frame(false)
        .tooltip_text(format!("Open {url}"))
        .halign(gtk::Align::Start)
        .css_classes(["slink"])
        .build();
    b.set_child(Some(&label(&format!("<span foreground='{LINK}'>{markup}</span>"), false)));
    let (win, url) = (ui.win.downgrade(), url.to_string());
    b.connect_clicked(move |_| {
        if let (true, Some(win)) = (report::openable(&url), win.upgrade()) {
            gtk::UriLauncher::new(&url).launch(Some(&win), gio::Cancellable::NONE, |_| {});
        }
    });
    b
}

fn card(ui: &Rc<Ui>, repo: &str, tab: &Tab, pos: usize, count: usize) -> gtk::Frame {
    let cards = ui.cards.borrow();
    let c = cards.get(repo);
    let frame = gtk::Frame::new(None);
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    frame.set_child(Some(&inner));
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let dot = colour(c.map_or(Light::Grey, Card::light));
    top.append(&label(&format!("<span foreground='{dot}' size='large'>●</span>"), true));
    let url = c.and_then(|c| c.ok()).map(|d| d.url.clone()).filter(|u| !u.is_empty());
    top.append(&link(ui, &format!("<b><u>{}</u></b>", esc(repo)), &url.unwrap_or_else(|| report::web_url(repo))));
    top.append(&gtk::Box::builder().hexpand(true).build());
    let weak = Rc::downgrade(ui);
    for (text, tip, step) in [("▲", "Move up", -1isize), ("▼", "Move down", 1)] {
        let b = gtk::Button::builder().label(text).tooltip_text(tip).has_frame(false).build();
        // own order only: under stars/forks the position comes from the numbers
        b.set_sensitive(tab.sort == Sort::Mine && pos.checked_add_signed(step).is_some_and(|p| p < count));
        let (w, repo, tab) = (weak.clone(), repo.to_string(), tab.name.clone());
        b.connect_clicked(move |_| {
            let (repo, tab) = (repo.clone(), tab.clone());
            later(&w, move |l| config::move_repo(l, &repo, &tab, step));
        });
        top.append(&b);
    }
    let rm = gtk::Button::builder().label("✕").tooltip_text("Remove from this tab").has_frame(false).build();
    let (w, repo_s, tab_s) = (weak.clone(), repo.to_string(), tab.name.clone());
    rm.connect_clicked(move |_| {
        let (repo, tab) = (repo_s.clone(), tab_s.clone());
        later(&w, move |l| config::remove_repo(l, &repo, Some(&tab)));
    });
    top.append(&rm);
    inner.append(&top);
    let Some(c) = c else {
        inner.append(&label("<small>checking…</small>", true));
        return frame;
    };
    let lines = c.summary_lines();
    inner.append(&label(&format!("<small>{}</small>", esc(&lines[0].0)), true));
    for (line, url) in &lines[1..] {
        match url {
            Some(u) => inner.append(&link(ui, &esc(line), u)),
            None => inner.append(&label(&format!("   {}", esc(line)), true)),
        }
    }
    for t in &c.todo {
        inner.append(&label(&format!("<b>→ {}</b>", esc(t)), true));
    }
    frame
}

fn page(ui: &Rc<Ui>, tab: &Tab) -> (gtk::Box, gtk::Label) {
    let weak = Rc::downgrade(ui);
    let bx = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(16)
        .margin_end(16)
        .build();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let entry = gtk::Entry::builder()
        .placeholder_text("owner/name, or a GitHub, GitLab or Codeberg link")
        .hexpand(true)
        .build();
    let add = gtk::Button::with_label("Add repo");
    let sort = gtk::DropDown::from_strings(&SORT_LABELS);
    sort.set_selected(Sort::ALL.iter().position(|s| *s == tab.sort).unwrap_or(0) as u32);
    sort.set_tooltip_text(Some("Order of this tab"));
    let drop_tab = gtk::Button::with_label("Remove tab");
    bar.append(&entry);
    bar.append(&add);
    bar.append(&sort);
    bar.append(&drop_tab);
    bx.append(&bar);
    let msg = gtk::Label::builder().xalign(0.0).visible(false).build();
    bx.append(&msg);

    let on_add = {
        let (w, entry, msg, name) = (weak.clone(), entry.clone(), msg.clone(), tab.name.clone());
        move || match config::repo_name(&entry.text()) {
            Some(repo) => {
                let name = name.clone();
                later(&w, move |l| config::add_repo(l, &repo, &name));
            }
            None => {
                msg.set_visible(true);
                msg.set_markup(&format!(
                    "<span foreground='{}'>Type owner/name (GitHub), gitlab.com/group/project, \
                     codeberg.org/owner/repo, or paste a link</span>",
                    colour(Light::Red)
                ));
            }
        }
    };
    let on_add2 = on_add.clone();
    entry.connect_activate(move |_| on_add());
    add.connect_clicked(move |_| on_add2());
    {
        let (w, name) = (weak.clone(), tab.name.clone());
        sort.connect_selected_notify(move |d| {
            if w.upgrade().is_none_or(|ui| ui.building.get()) {
                return;
            }
            let (name, s) = (name.clone(), Sort::ALL[(d.selected() as usize).min(2)]);
            later(&w, move |l| config::set_sort(l, &name, s));
        });
    }
    {
        let (w, name, n) = (weak.clone(), tab.name.clone(), tab.repos.len());
        drop_tab.connect_clicked(move |_| {
            let Some(ui) = w.upgrade() else { return };
            let ask = gtk::AlertDialog::builder()
                .message(format!("Remove the tab “{name}”?"))
                .detail(format!("Its {n} repo(s) leave the list. Nothing changes on GitHub."))
                .buttons(["Cancel", "Remove"])
                .cancel_button(0)
                .default_button(0)
                .build();
            let (w, name) = (w.clone(), name.clone());
            ask.choose(Some(&ui.win), gio::Cancellable::NONE, move |res| {
                if res == Ok(1) {
                    later(&w, move |l| config::remove_tab(l, &name));
                }
            });
        });
    }

    let cards = gtk::Box::new(gtk::Orientation::Vertical, 14);
    let repos = report::ordered(&tab.repos, tab.sort, &ui.cards.borrow());
    for (pos, repo) in repos.iter().enumerate() {
        cards.append(&card(ui, repo, tab, pos, repos.len()));
    }
    if repos.is_empty() {
        cards.append(&label("No repos in this tab yet. Add one above.", true));
    }
    bx.append(&gtk::ScrolledWindow::builder().child(&cards).vexpand(true).build());
    let worst = {
        let all = ui.cards.borrow();
        repos.iter().map(|r| all.get(r).map_or(Light::Grey, Card::light)).max().unwrap_or(Light::Grey)
    };
    let tab_label = gtk::Label::new(None);
    tab_label.set_markup(&format!("<span foreground='{}'>●</span> {}", colour(worst), esc(&tab.name)));
    (bx, tab_label)
}

fn render(ui: &Rc<Ui>) {
    // a file that cannot be read keeps the old view and says why; nothing is rebuilt from it
    let tabs = match config::load_tabs() {
        Ok(tabs) => tabs,
        Err(e) => {
            show_error(ui, &e);
            ui.refresh.set_sensitive(true);
            return;
        }
    };
    ui.building.set(true);
    let current = ui.book.current_page().unwrap_or(0);
    while ui.book.n_pages() > 0 {
        ui.book.remove_page(None);
    }
    for tab in &tabs {
        let (child, lab) = page(ui, tab);
        ui.book.append_page(&child, Some(&lab));
    }
    ui.book.set_current_page(Some(current.min(ui.book.n_pages().saturating_sub(1))));
    ui.building.set(false);

    let repos = config::all_repos(&tabs);
    let todo: Vec<String> = {
        let (cards, busy) = (ui.cards.borrow(), ui.busy.borrow());
        repos.iter().filter(|r| !cards.contains_key(*r) && !busy.contains(*r)).cloned().collect()
    };
    let checking = !ui.busy.borrow().is_empty() || !todo.is_empty();
    let stamp = ui.stamp.borrow();
    ui.status.set_markup(&format!(
        "<small>{}{}{} repos in {} tab(s)</small>",
        if checking { "checking… " } else { "" },
        if stamp.is_empty() { String::new() } else { format!("checked {stamp} — ") },
        repos.len(),
        tabs.len()
    ));
    drop(stamp);
    ui.refresh.set_sensitive(!checking);
    if !checking {
        snapshot_if_asked(ui);
    }
    if todo.is_empty() {
        return;
    }
    ui.busy.borrow_mut().extend(todo.iter().cloned());
    let weak = Rc::downgrade(ui);
    let asked = todo.clone();
    glib::MainContext::default().spawn_local(async move {
        let mut found = gio::spawn_blocking(move || report::check_all(&todo)).await.unwrap_or_default();
        let Some(ui) = weak.upgrade() else { return };
        for r in asked {
            ui.busy.borrow_mut().remove(&r);
            // a crashed check must not leave the card "checking…" forever
            found.entry(r.clone()).or_insert_with(|| Card {
                repo: r,
                result: Err("the check crashed".into()),
                todo: vec!["press Refresh to try again".into()],
            });
        }
        ui.cards.borrow_mut().extend(found);
        *ui.stamp.borrow_mut() =
            glib::DateTime::now_local().and_then(|t| t.format("%H:%M")).map_or_else(|_| String::new(), String::from);
        render(&ui);
    });
}

/// Developer aid (README pictures, checks without a desktop): with SLASHREPO_SNAPSHOT=FILE.png the
/// window saves itself to FILE once every repo is checked, then quits.
fn snapshot_if_asked(ui: &Rc<Ui>) {
    let Some(file) = std::env::var_os("SLASHREPO_SNAPSHOT") else { return };
    let win = ui.win.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
        // the window's own paintable draws nothing; its content does
        let Some(body) = win.child() else { return };
        let (w, h) = (body.width(), body.height());
        let snap = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&body)).snapshot(&snap, f64::from(w), f64::from(h));
        let saved = snap.to_node().zip(win.native().and_then(|n| n.renderer())).map(|(node, r)| {
            r.render_texture(node, Some(&gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32))).save_to_png(&file)
        });
        if !matches!(saved, Some(Ok(()))) {
            eprintln!("snapshot failed (on broadway a browser must be connected so the window is drawn): {saved:?}");
        }
        if let Some(app) = win.application() {
            app.quit();
        }
    });
}

/// `shown` points at the open window's state. Every `/repo` activates the app again; a second
/// activation (GApplication hands it to the running process) shows the window that is there,
/// and refreshes it unless a check is still running, as the Refresh button would.
fn activate(app: &gtk::Application, shown: &RefCell<Weak<Ui>>) {
    if let Some(open) = app.windows().first() {
        open.present();
        if let Some(ui) = shown.borrow().upgrade()
            && ui.busy.borrow().is_empty()
        {
            ui.cards.borrow_mut().clear();
            render(&ui);
        }
        return;
    }
    // link buttons as tight as text lines; a theme button is ~2 lines tall
    let css = gtk::CssProvider::new();
    css.load_from_string(
        "button.slink { padding: 0 4px; min-height: 0; border: none; box-shadow: none; background: none; } \
         button.slink:hover label { text-decoration-line: underline; }",
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Repo Status")
        .icon_name(crate::APP_ID)
        .default_width(900)
        .default_height(760)
        .build();
    let header = gtk::HeaderBar::new();
    let refresh = gtk::Button::with_label("Refresh");
    header.pack_start(&refresh);
    let new_tab = gtk::Entry::builder().placeholder_text("Tab name").build();
    let new_tab_btn =
        gtk::MenuButton::builder().label("New tab").popover(&gtk::Popover::builder().child(&new_tab).build()).build();
    header.pack_end(&new_tab_btn);
    win.set_titlebar(Some(&header));
    let status = gtk::Label::builder().xalign(0.0).margin_start(16).margin_top(8).build();
    let book = gtk::Notebook::builder().scrollable(true).vexpand(true).build();
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 6);
    outer.append(&status);
    outer.append(&book);
    win.set_child(Some(&outer));

    let ui = Rc::new(Ui {
        win: win.clone(),
        book,
        status,
        refresh: refresh.clone(),
        cards: RefCell::default(),
        busy: RefCell::default(),
        stamp: RefCell::default(),
        building: Cell::new(false),
    });
    let w = Rc::downgrade(&ui);
    refresh.connect_clicked(move |_| {
        if let Some(ui) = w.upgrade() {
            ui.cards.borrow_mut().clear();
            render(&ui);
        }
    });
    let w = Rc::downgrade(&ui);
    new_tab.connect_activate(move |e| {
        let (Some(ui), Some(name)) = (w.upgrade(), config::tab_name(&e.text())) else { return };
        e.set_text("");
        new_tab_btn.popdown();
        let n = name.clone();
        if let Err(e) = config::edit(move |l| config::add_tab(l, &n)) {
            show_error(&ui, &format!("Could not change {}: {e}", config::path().display()));
            return;
        }
        render(&ui);
        if let Some(i) = config::load_tabs().unwrap_or_default().iter().position(|t| t.name == name) {
            ui.book.set_current_page(Some(i as u32));
        }
    });
    *shown.borrow_mut() = Rc::downgrade(&ui);
    render(&ui);
    // the window owns the state from here; it lives as long as the window
    win.connect_destroy(move |_| drop(ui.clone()));
    win.present();
}

pub fn run() -> ExitCode {
    let app = gtk::Application::builder().application_id(crate::APP_ID).build();
    let shown = RefCell::new(Weak::new());
    app.connect_activate(move |app| activate(app, &shown));
    let code = app.run_with_args::<&str>(&[]);
    if code == glib::ExitCode::SUCCESS { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
