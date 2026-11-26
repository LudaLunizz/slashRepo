//! The Repo Status window (GTK 4): one notebook tab per tab in the repos file, one card per repo.

use std::cell::{Cell, RefCell};
use std::process::ExitCode;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::config::{self, Sort};
use crate::report::Light;
use crate::view::{self, CardView, Checks, TabView};

fn colour(l: Light) -> String {
    view::hex(view::rgb(l))
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
    checks: RefCell<Checks>,
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
                show_error(&ui, &view::edit_failed(&e));
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
    b.set_child(Some(&label(&format!("<span foreground='{}'>{markup}</span>", view::hex(view::LINK)), false)));
    let (win, url) = (ui.win.downgrade(), url.to_string());
    b.connect_clicked(move |_| {
        if let (true, Some(win)) = (crate::report::openable(&url), win.upgrade()) {
            gtk::UriLauncher::new(&url).launch(Some(&win), gio::Cancellable::NONE, |_| {});
        }
    });
    b
}

fn card(ui: &Rc<Ui>, c: &CardView, tab: &str) -> gtk::Frame {
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
    top.append(&label(&format!("<span foreground='{}' size='large'>●</span>", colour(c.dot)), true));
    top.append(&link(ui, &format!("<b><u>{}</u></b>", esc(&c.repo)), &c.url));
    top.append(&gtk::Box::builder().hexpand(true).build());
    let weak = Rc::downgrade(ui);
    for (text, tip, step, can) in [("▲", "Move up", -1isize, c.can_up), ("▼", "Move down", 1, c.can_down)] {
        let b = gtk::Button::builder().label(text).tooltip_text(tip).has_frame(false).build();
        b.set_sensitive(can);
        let (w, repo, tab) = (weak.clone(), c.repo.clone(), tab.to_string());
        b.connect_clicked(move |_| {
            let (repo, tab) = (repo.clone(), tab.clone());
            later(&w, move |l| config::move_repo(l, &repo, &tab, step));
        });
        top.append(&b);
    }
    let rm = gtk::Button::builder().label("✕").tooltip_text("Remove from this tab").has_frame(false).build();
    let (w, repo_s, tab_s) = (weak.clone(), c.repo.clone(), tab.to_string());
    rm.connect_clicked(move |_| {
        let (repo, tab) = (repo_s.clone(), tab_s.clone());
        later(&w, move |l| config::remove_repo(l, &repo, Some(&tab)));
    });
    top.append(&rm);
    inner.append(&top);
    let Some(status) = &c.status else {
        inner.append(&label(&format!("<small>{}</small>", view::CHECKING), true));
        return frame;
    };
    // the lines on the left, live/private in the bottom-right corner beside them (no extra line)
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).hexpand(true).build();
    body.append(&text);
    inner.append(&body);
    text.append(&label(&format!("<small>{}</small>", esc(status)), true));
    for line in &c.lines {
        match &line.url {
            Some(u) => text.append(&link(ui, &esc(&line.text), u)),
            None => text.append(&label(&esc(&line.text), true)),
        }
    }
    for t in &c.todo {
        text.append(&label(&format!("<b>{}</b>", esc(t)), true));
    }
    if let Some((word, light)) = c.visibility {
        frame.add_css_class(word);
        let tag = gtk::Label::builder().halign(gtk::Align::End).valign(gtk::Align::End).build();
        tag.set_markup(&format!("<span foreground='{}'><small><b>{word}</b></small></span>", colour(light)));
        body.append(&tag);
    }
    frame
}

fn page(ui: &Rc<Ui>, tab: &TabView, repo_count: usize) -> (gtk::Box, gtk::Label) {
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
    let entry = gtk::Entry::builder().placeholder_text(view::ADD_HINT).hexpand(true).build();
    let add = gtk::Button::with_label("Add repo");
    let sort = gtk::DropDown::from_strings(&view::SORT_LABELS);
    sort.set_selected(tab.sort as u32);
    sort.set_tooltip_text(Some("Order of this tab"));
    let drop_tab = gtk::Button::with_label("Remove tab");
    bar.append(&entry);
    bar.append(&add);
    bar.append(&sort);
    bar.append(&drop_tab);
    bx.append(&bar);
    let msg = gtk::Label::builder().xalign(0.0).visible(false).build();
    bx.append(&msg);
    bx.append(&label(&format!("<small>{}</small>", esc(&tab.totals)), true));

    let on_add = {
        let (w, entry, msg, name) = (weak.clone(), entry.clone(), msg.clone(), tab.name.clone());
        move || match config::repo_name(&entry.text()) {
            Some(repo) => {
                let name = name.clone();
                later(&w, move |l| config::add_repo(l, &repo, &name));
            }
            None => {
                msg.set_visible(true);
                msg.set_markup(&format!("<span foreground='{}'>{}</span>", colour(Light::Red), esc(view::BAD_REPO)));
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
        let (w, name) = (weak.clone(), tab.name.clone());
        drop_tab.connect_clicked(move |_| {
            let Some(ui) = w.upgrade() else { return };
            let (question, detail) = view::remove_tab_question(&name, repo_count);
            let ask = gtk::AlertDialog::builder()
                .message(question)
                .detail(detail)
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
    for c in &tab.cards {
        cards.append(&card(ui, c, &tab.name));
    }
    if tab.cards.is_empty() {
        cards.append(&label(view::NO_REPOS, true));
    }
    bx.append(&gtk::ScrolledWindow::builder().child(&cards).vexpand(true).build());
    let tab_label = gtk::Label::new(None);
    tab_label.set_markup(&format!("<span foreground='{}'>●</span> {}", colour(tab.dot), esc(&tab.name)));
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
        let v = ui.checks.borrow().tab(tab);
        let (child, lab) = page(ui, &v, tab.repos.len());
        ui.book.append_page(&child, Some(&lab));
    }
    ui.book.set_current_page(Some(current.min(ui.book.n_pages().saturating_sub(1))));
    ui.building.set(false);

    let (line, checking) = {
        let ch = ui.checks.borrow();
        (ch.status_line(&tabs), ch.checking(&tabs))
    };
    ui.status.set_markup(&format!("<small>{}</small>", esc(&line)));
    ui.refresh.set_sensitive(!checking);
    if !checking {
        snapshot_if_asked(ui);
    }
    let todo = ui.checks.borrow_mut().start(&tabs);
    if todo.is_empty() {
        return;
    }
    let weak = Rc::downgrade(ui);
    glib::MainContext::default().spawn_local(async move {
        let asked = todo.clone();
        // a crashed worker still answers: Checks::apply gives its repos error cards
        let done = gio::spawn_blocking(move || view::run_checks(todo))
            .await
            .unwrap_or_else(|_| view::Done { asked, found: Default::default() });
        let Some(ui) = weak.upgrade() else { return };
        let stamp =
            glib::DateTime::now_local().and_then(|t| t.format("%H:%M")).map_or_else(|_| String::new(), String::from);
        ui.checks.borrow_mut().apply(done, stamp);
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
            && !ui.checks.borrow().busy()
        {
            ui.checks.borrow_mut().clear();
            render(&ui);
        }
        return;
    }
    // link buttons as tight as text lines; a theme button is ~2 lines tall
    let css = gtk::CssProvider::new();
    css.load_from_string(
        "button.slink { padding: 0 4px; min-height: 0; border: none; box-shadow: none; background: none; } \
         button.slink:hover label { text-decoration-line: underline; } \
         frame.live { border: 2px solid #57ab5a; } frame.private { border: 2px solid #539bf5; }",
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
        checks: RefCell::default(),
        building: Cell::new(false),
    });
    let w = Rc::downgrade(&ui);
    refresh.connect_clicked(move |_| {
        if let Some(ui) = w.upgrade() {
            ui.checks.borrow_mut().clear();
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
            show_error(&ui, &view::edit_failed(&e));
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
