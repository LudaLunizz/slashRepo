//! The Repo Status window (GTK 4): a row of tabs, one card per repo. Draws what src/view.rs says,
//! in the window's own dark palette (the same on every desktop theme).

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::process::ExitCode;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};

use crate::config::{self, Sort, Tab};
use crate::report::Light;
use crate::view::{self, CardView, Checks, Group, Row, TabView, palette};

fn colour(l: Light) -> String {
    view::hex(view::rgb(l))
}

/// css class of a tone: t-red, t-green, ...
fn tone(l: Light) -> String {
    format!("t-{}", l.as_str())
}

fn esc(t: &str) -> String {
    glib::markup_escape_text(t).to_string()
}

fn muted(t: &str) -> String {
    format!("<span foreground='{}'>{}</span>", view::hex(palette::MUTED), esc(t))
}

fn label(markup: &str, selectable: bool) -> gtk::Label {
    let lb = gtk::Label::builder().xalign(0.0).wrap(true).selectable(selectable).build();
    lb.set_markup(markup);
    lb
}

fn text(t: &str, class: &str) -> gtk::Label {
    gtk::Label::builder().label(t).xalign(0.0).css_classes([class]).build()
}

fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}

fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}

/// The round status mark.
fn dot(l: Light) -> gtk::Box {
    gtk::Box::builder().css_classes(["sr-dot".to_string(), tone(l)]).valign(gtk::Align::Center).build()
}

fn clear(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

struct Ui {
    win: gtk::ApplicationWindow,
    pills: gtk::Box,
    status: gtk::Label,
    error: gtk::Label,
    refresh: gtk::Button,
    tab_strip: gtk::Box,
    /// the add field, the order and "Remove tab": they work on the open tab
    bar: gtk::Box,
    entry: gtk::Entry,
    entry_msg: gtk::Label,
    sort: gtk::DropDown,
    totals: gtk::Label,
    scroll: gtk::ScrolledWindow,
    checks: RefCell<Checks>,
    tabs: RefCell<Vec<Tab>>,
    current: Cell<usize>,
    /// folded lists the user opened ("repo\nPull requests"); they stay open over a refresh
    unfolded: RefCell<HashSet<String>>,
    /// render() is setting the order dropdown: its notify must not write the file then
    building: Cell<bool>,
}

impl Ui {
    fn tab(&self) -> Option<Tab> {
        self.tabs.borrow().get(self.current.get()).cloned()
    }

    fn show_tab(self: &Rc<Self>, i: usize) {
        if i != self.current.get() && i < self.tabs.borrow().len() {
            self.current.set(i);
            self.scroll.vadjustment().set_value(0.0);
            self.entry_msg.set_visible(false);
            render(self);
        }
    }

    fn refresh(self: &Rc<Self>) {
        if !self.checks.borrow().busy() {
            self.checks.borrow_mut().clear();
            self.error.set_visible(false);
            render(self);
        }
    }
}

/// A red line under the top bar; it stays until the next change that works.
fn show_error(ui: &Ui, msg: &str) {
    ui.error.set_text(msg);
    ui.error.set_visible(true);
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
            ui.error.set_visible(false);
            render(&ui);
        }
    });
}

/// A flat button that opens url: a link inside a selectable label starts a selection on click.
fn link(ui: &Rc<Ui>, child: &impl IsA<gtk::Widget>, url: &str, tip: &str, class: &str) -> gtk::Button {
    let b = gtk::Button::builder().has_frame(false).tooltip_text(tip).css_classes([class]).child(child).build();
    let (win, url) = (ui.win.downgrade(), url.to_string());
    b.connect_clicked(move |_| {
        if let (true, Some(win)) = (crate::report::openable(&url), win.upgrade()) {
            gtk::UriLauncher::new(&url).launch(Some(&win), gio::Cancellable::NONE, |_| {});
        }
    });
    b
}

/// ▲ ▼ ✕: quiet until the pointer is on them.
fn icon_button(glyph: &str, tip: &str) -> gtk::Button {
    gtk::Button::builder().label(glyph).tooltip_text(tip).has_frame(false).css_classes(["sr-icon"]).build()
}

/// One PR or issue: mark, number, title (cut with … when the window is narrow), labels, details.
fn row(ui: &Rc<Ui>, r: &Row) -> gtk::Widget {
    let line = hbox(8);
    if let Some(l) = r.tone {
        line.append(&dot(l));
    }
    line.append(&text(&r.id, "sr-muted"));
    let title = gtk::Label::builder()
        .label(&r.title)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .width_chars(8)
        .build();
    line.append(&title);
    for l in &r.labels {
        line.append(&text(l, "sr-tag"));
    }
    line.append(&text(&r.meta, "sr-muted"));
    match &r.url {
        Some(u) => link(ui, &line, u, &format!("{} {}\nOpen {u}", r.id, r.title), "sr-row").upcast(),
        None => {
            line.add_css_class("sr-row-plain");
            line.set_tooltip_text(Some(&format!("{} {}", r.id, r.title)));
            line.upcast()
        }
    }
}

/// "PULL REQUESTS · 6", the first rows, and the rest behind "Show 3 more".
fn group(ui: &Rc<Ui>, repo: &str, g: &Group) -> gtk::Box {
    let bx = vbox(0);
    bx.append(&text(&g.title.to_uppercase(), "sr-group"));
    let (shown, folded) = g.rows.split_at(g.rows.len().min(view::FOLD));
    for r in shown {
        bx.append(&row(ui, r));
    }
    if !folded.is_empty() {
        let key = format!("{repo}\n{}", g.title.split(" · ").next().unwrap_or_default());
        let open = ui.unfolded.borrow().contains(&key);
        let rest = vbox(0);
        for r in folded {
            rest.append(&row(ui, r));
        }
        let more = gtk::Revealer::builder().child(&rest).reveal_child(open).build();
        let toggle = gtk::Button::builder()
            .label(view::fold_text(folded.len(), open))
            .has_frame(false)
            .halign(gtk::Align::Start)
            .css_classes(["sr-more"])
            .build();
        let (weak, more2, hidden) = (Rc::downgrade(ui), more.clone(), folded.len());
        toggle.connect_clicked(move |b| {
            let Some(ui) = weak.upgrade() else { return };
            let open = !more2.reveals_child();
            more2.set_reveal_child(open);
            b.set_label(&view::fold_text(hidden, open));
            let mut set = ui.unfolded.borrow_mut();
            if open {
                set.insert(key.clone());
            } else {
                set.remove(&key);
            }
        });
        bx.append(&more);
        bx.append(&toggle);
    }
    if let Some(m) = &g.more {
        bx.append(&text(m, "sr-beyond"));
    }
    bx
}

/// "release v1.4.2 · 3d ago   CI on main ● passing   …" as one line that wraps between facts.
fn facts_markup(c: &CardView) -> String {
    let nb = |t: &str| esc(t).replace(' ', "\u{a0}");
    let one = |f: &view::Fact| {
        let value = match f.tone {
            Light::Grey => nb(&f.value),
            l => format!("<span foreground='{}'>●\u{a0}{}</span>", colour(l), nb(&f.value)),
        };
        format!("<span foreground='{}'>{}</span>\u{a0}{value}", view::hex(palette::MUTED), nb(&f.label))
    };
    c.facts.iter().map(one).collect::<Vec<_>>().join("\u{2003} ")
}

fn card(ui: &Rc<Ui>, c: &CardView, tab: &str) -> gtk::Box {
    let bx = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["sr-card"]).build();
    let top = hbox(8);
    top.append(&dot(c.dot));
    let (owner, name) = view::split_name(&c.repo);
    let title = label(&format!("{}<b>{}</b>", muted(owner), esc(name)), false);
    title.set_ellipsize(pango::EllipsizeMode::Middle);
    title.set_wrap(false);
    top.append(&link(ui, &title, &c.url, &format!("Open {}", c.url), "sr-name"));
    top.append(&gtk::Box::builder().hexpand(true).build());
    let weak = Rc::downgrade(ui);
    for (glyph, tip, step, can) in [("▲", "Move up", -1isize, c.can_up), ("▼", "Move down", 1, c.can_down)] {
        let b = icon_button(glyph, tip);
        b.set_sensitive(can);
        let (w, repo, tab) = (weak.clone(), c.repo.clone(), tab.to_string());
        b.connect_clicked(move |_| {
            let (repo, tab) = (repo.clone(), tab.clone());
            later(&w, move |l| config::move_repo(l, &repo, &tab, step));
        });
        top.append(&b);
    }
    let rm = icon_button("✕", "Remove from this tab");
    let (w, repo_s, tab_s) = (weak.clone(), c.repo.clone(), tab.to_string());
    rm.connect_clicked(move |_| {
        let (repo, tab) = (repo_s.clone(), tab_s.clone());
        later(&w, move |l| config::remove_repo(l, &repo, Some(&tab)));
    });
    top.append(&rm);
    bx.append(&top);

    if c.checking {
        let wait = hbox(8);
        wait.append(&gtk::Spinner::builder().spinning(true).build());
        wait.append(&text(view::CHECKING, "sr-muted"));
        bx.append(&wait);
        return bx;
    }
    if let Some(e) = &c.error {
        let lb = label(&esc(e), true);
        lb.add_css_class("sr-error");
        bx.append(&lb);
    }
    if !c.facts.is_empty() {
        bx.append(&label(&facts_markup(c), true));
    }
    for g in &c.groups {
        bx.append(&group(ui, &c.repo, g));
    }
    // the to-do on the left, live/private in the bottom-right corner beside it (no extra line)
    let foot = hbox(8);
    if c.clear {
        let ok = label(&format!("✓ {}", view::NOTHING_TO_DO), true);
        ok.add_css_class("sr-clear");
        ok.set_hexpand(true);
        ok.set_valign(gtk::Align::Center);
        foot.append(&ok);
    } else {
        let todo = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(3)
            .hexpand(true)
            .css_classes(["sr-todo".to_string(), tone(c.dot)])
            .build();
        for t in &c.todo {
            todo.append(&label(&format!("→ {}", esc(t)), true));
        }
        foot.append(&todo);
    }
    if let Some((word, light)) = c.visibility {
        bx.add_css_class(word);
        let tag = text(word, "sr-vis");
        tag.add_css_class(&tone(light));
        tag.set_valign(gtk::Align::End);
        tag.set_tooltip_text(Some(if word == "live" { "Public: anyone can see it" } else { "Private" }));
        foot.append(&tag);
    }
    bx.append(&foot);
    bx
}

fn tab_button(ui: &Rc<Ui>, i: usize, v: &TabView, current: bool) -> gtk::Button {
    let inner = hbox(7);
    inner.append(&dot(v.dot));
    inner.append(&gtk::Label::new(Some(&v.name)));
    inner.append(&text(&v.cards.len().to_string(), "sr-count"));
    let b = gtk::Button::builder().child(&inner).has_frame(false).css_classes(["sr-tab"]).build();
    if current {
        b.add_css_class("current");
    }
    let weak = Rc::downgrade(ui);
    b.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.show_tab(i);
        }
    });
    b
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
    let current = ui.current.get().min(tabs.len().saturating_sub(1));
    ui.current.set(current);
    let views: Vec<TabView> = tabs.iter().map(|t| ui.checks.borrow().tab(t)).collect();

    clear(&ui.tab_strip);
    for (i, v) in views.iter().enumerate() {
        ui.tab_strip.append(&tab_button(ui, i, v, i == current));
    }
    let cards = vbox(12);
    cards.add_css_class("sr-cards");
    ui.bar.set_visible(!tabs.is_empty());
    ui.totals.set_visible(!tabs.is_empty());
    match views.get(current) {
        Some(v) => {
            ui.building.set(true);
            ui.sort.set_selected(v.sort as u32);
            ui.building.set(false);
            ui.totals.set_text(&v.totals);
            for c in &v.cards {
                cards.append(&card(ui, c, &v.name));
            }
            if v.cards.is_empty() {
                cards.append(&text(view::NO_REPOS, "sr-empty"));
            }
        }
        None => cards.append(&text(view::NO_TABS, "sr-empty")),
    }
    // the scrolled window stays, so a refresh or a moved card keeps the place you scrolled to
    ui.scroll.set_child(Some(&cards));

    let (line, pills, checking) = {
        let ch = ui.checks.borrow();
        (ch.status_line(&tabs), ch.summary(&tabs).pills(), ch.checking(&tabs))
    };
    clear(&ui.pills);
    for (words, light) in pills {
        let p = text(&words, "sr-pill");
        p.add_css_class(&tone(light));
        ui.pills.append(&p);
    }
    ui.status.set_text(&line);
    ui.status.set_tooltip_text(Some(&line));
    ui.refresh.set_sensitive(!checking);
    *ui.tabs.borrow_mut() = tabs;
    if !checking {
        snapshot_if_asked(ui);
    }
    let todo = {
        let tabs = ui.tabs.borrow();
        ui.checks.borrow_mut().start(&tabs)
    };
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
/// window saves itself to FILE once every repo is checked, then quits. SLASHREPO_SNAPSHOT_SCALE=2
/// draws it twice as large (sharp on a high-density screen).
fn snapshot_if_asked(ui: &Rc<Ui>) {
    if std::env::var_os("SLASHREPO_SNAPSHOT").is_some() {
        snapshot_soon(ui.win.clone(), 20);
    }
}

fn snapshot_soon(win: gtk::ApplicationWindow, tries: u32) {
    glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
        let Some(file) = std::env::var_os("SLASHREPO_SNAPSHOT") else { return };
        let scale = std::env::var("SLASHREPO_SNAPSHOT_SCALE").ok().and_then(|s| s.parse::<f32>().ok());
        let scale = scale.filter(|s| (1.0..=4.0).contains(s)).unwrap_or(1.0);
        // the window's own paintable draws nothing; its content does
        let Some(body) = win.child() else { return };
        // SLASHREPO_SNAPSHOT_SIZE: lay the content out at that size even when the (off-screen)
        // display is smaller than the picture asked for
        let (w, h) = view::snapshot_size().unwrap_or((body.width(), body.height()));
        if (w, h) != (body.width(), body.height()) {
            body.allocate(w, h, -1, None);
        }
        let snap = gtk::Snapshot::new();
        snap.scale(scale, scale);
        gtk::WidgetPaintable::new(Some(&body)).snapshot(&snap, f64::from(w), f64::from(h));
        let area = gtk::graphene::Rect::new(0.0, 0.0, w as f32 * scale, h as f32 * scale);
        let saved = snap
            .to_node()
            .zip(win.native().and_then(|n| n.renderer()))
            .map(|(node, r)| r.render_texture(node, Some(&area)).save_to_png(&file));
        match saved {
            Some(Ok(())) => {}
            // not on screen yet (the made-up repos of --demo are "checked" before the window is)
            None if tries > 0 => return snapshot_soon(win, tries - 1),
            _ => {
                eprintln!("snapshot failed (on broadway a browser must be connected so the window is drawn): {saved:?}")
            }
        }
        if let Some(app) = win.application() {
            app.quit();
        }
    });
}

/// The window's own look. Colours come from view::palette, so the Windows window matches.
fn css() -> String {
    let h = view::hex;
    let (bg, card, raised, field) = (h(palette::BG), h(palette::CARD), h(palette::RAISED), h(palette::FIELD));
    let (border, text, muted, link) = (h(palette::BORDER), h(palette::TEXT), h(palette::MUTED), h(view::LINK));
    let mut css = format!(
        "window.sr, window.sr .sr-body {{ background: {bg}; color: {text}; }}
         .sr-top {{ padding: 14px 16px 4px 16px; }}
         .sr-tabs {{ padding: 4px 12px 0 12px; }}
         .sr-bar {{ padding: 6px 16px 0 16px; }}
         .sr-totals {{ padding: 2px 18px 4px 18px; color: {muted}; }}
         .sr-cards {{ padding: 6px 16px 18px 16px; }}
         .sr-muted, .sr-status, .sr-beyond {{ color: {muted}; }}
         .sr-beyond {{ padding: 2px 8px; }}
         .sr-empty {{ color: {muted}; padding: 28px 8px; }}
         .sr-error {{ color: {red}; }}
         .sr-demo {{ color: {orange}; padding: 0 16px; }}
         .sr-top .sr-error {{ padding: 0 16px; }}
         .sr-dot {{ min-width: 10px; min-height: 10px; border-radius: 5px; background: {grey}; }}
         .sr-pill {{ border-radius: 99px; padding: 3px 11px; font-weight: 600; }}
         .sr-vis {{ border-radius: 99px; padding: 1px 9px; font-size: 0.85em; font-weight: 700; }}
         .sr-count {{ border-radius: 99px; padding: 0 7px; font-size: 0.85em; color: {muted}; background: {bg}; }}
         .sr-tag {{ border-radius: 99px; padding: 0 7px; font-size: 0.85em; color: {muted}; border: 1px solid {border}; }}
         .sr-group {{ color: {muted}; font-size: 0.8em; font-weight: 700; letter-spacing: 0.06em; padding: 2px 0 2px 0; }}
         .sr-card {{ background: {card}; border: 1px solid {border}; border-radius: 12px; padding: 12px 14px; }}
         .sr-clear {{ color: {muted}; }}
         .sr-todo {{ border-left: 3px solid {grey}; border-radius: 0 6px 6px 0; padding: 6px 10px; font-weight: 600; }}
         window.sr button {{ background-image: none; box-shadow: none; text-shadow: none; -gtk-icon-shadow: none; }}
         button.sr-btn, dropdown.sr-btn > button {{ background-color: {raised}; color: {text}; border: 1px solid {border};
            border-radius: 8px; padding: 4px 12px; min-height: 24px; }}
         button.sr-btn:hover, dropdown.sr-btn > button:hover {{ background-color: {hover}; }}
         button.sr-btn:disabled {{ color: {grey}; }}
         button.sr-quiet {{ background-color: transparent; color: {muted}; }}
         button.sr-quiet:hover {{ color: {text}; }}
         button.sr-tab {{ background-color: transparent; color: {muted}; border: 1px solid transparent; border-radius: 8px;
            padding: 5px 10px; min-height: 0; }}
         button.sr-tab:hover {{ background-color: {raised}; color: {text}; }}
         button.sr-tab.current {{ background-color: {raised}; border-color: {border}; color: {text}; font-weight: 600; }}
         button.sr-name {{ background-color: transparent; border: none; padding: 0 2px; min-height: 0; color: {text};
            font-size: 1.12em; font-weight: normal; }}
         button.sr-name:hover label {{ text-decoration-line: underline; }}
         button.sr-icon {{ background-color: transparent; border: none; border-radius: 6px; padding: 0; min-width: 28px;
            min-height: 26px; color: {grey}; }}
         button.sr-icon:hover {{ background-color: {raised}; color: {text}; }}
         button.sr-icon:disabled {{ opacity: 0.35; }}
         button.sr-row {{ background-color: transparent; border: none; border-radius: 6px; padding: 4px 8px; min-height: 0;
            color: {text}; font-weight: normal; }}
         button.sr-row:hover {{ background-color: {raised}; }}
         button.sr-row:hover > box > label:nth-child(3) {{ color: {link}; }}
         .sr-row-plain {{ padding: 4px 8px; }}
         button.sr-more {{ background-color: transparent; border: none; padding: 2px 8px; min-height: 0; color: {link};
            font-weight: normal; }}
         button.sr-more:hover label {{ text-decoration-line: underline; }}
         entry.sr-entry {{ background-color: {field}; color: {text}; border: 1px solid {border}; border-radius: 8px;
            box-shadow: none; outline-color: {link}; min-height: 32px; caret-color: {text}; }}
         entry.sr-entry > text > placeholder {{ color: {grey}; }}
         window.sr scrollbar {{ background-color: transparent; border: none; }}
         window.sr *:focus-visible {{ outline-color: {link}; }}",
        red = colour(Light::Red),
        orange = colour(Light::Orange),
        grey = colour(Light::Grey),
        hover = h(view::mix(palette::TEXT, palette::RAISED, 0.08)),
    );
    for l in [Light::Red, Light::Orange, Light::Green, Light::Grey, Light::Blue] {
        let (c, t) = (colour(l), tone(l));
        let tint = h(view::mix(view::rgb(l), palette::CARD, 0.16));
        css += &format!(
            " .sr-dot.{t} {{ background: {c}; }} .sr-pill.{t}, .sr-vis.{t} {{ color: {c}; background: {tint}; }}
              .sr-todo.{t} {{ border-left-color: {c}; background: {faint}; }}",
            faint = h(view::mix(view::rgb(l), palette::CARD, 0.09)),
        );
    }
    css += &format!(
        " .sr-card.live {{ border-color: {}; }} .sr-card.private {{ border-color: {}; }}",
        h(view::border(Some(Light::Green))),
        h(view::border(Some(Light::Blue)))
    );
    css
}

/// `shown` points at the open window's state. Every `/repo` activates the app again; a second
/// activation (GApplication hands it to the running process) shows the window that is there,
/// and refreshes it unless a check is still running, as the Refresh button would.
fn activate(app: &gtk::Application, shown: &RefCell<Weak<Ui>>) {
    if let Some(open) = app.windows().first() {
        open.present();
        if let Some(ui) = shown.borrow().upgrade() {
            ui.refresh();
        }
        return;
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css());
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    // popovers, the dropdown list and the "Remove the tab?" dialog come from the theme: ask for
    // its dark side so they fit the window
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    let demo = crate::demo::on();
    let (width, height) = crate::view::snapshot_size().unwrap_or((940, 800));
    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .title(if demo { "Repo Status (demo)" } else { "Repo Status" })
        .icon_name(crate::APP_ID)
        .default_width(width)
        .default_height(height)
        .css_classes(["sr"])
        .build();

    // top: the count pills, the "checked 09:30 · …" line, Refresh
    let pills = hbox(8);
    let status = gtk::Label::builder()
        .xalign(1.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::Start)
        .css_classes(["sr-status"])
        .build();
    let refresh = gtk::Button::builder()
        .label("Refresh")
        .tooltip_text("Check every repo again (F5 or Ctrl+R)")
        .css_classes(["sr-btn"])
        .build();
    let top = hbox(12);
    top.add_css_class("sr-top");
    top.append(&pills);
    top.append(&status);
    top.append(&refresh);
    let error = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).css_classes(["sr-error"]).build();
    error.set_margin_start(16);
    error.set_margin_end(16);

    // tabs, then "+ New tab"
    let tab_strip = hbox(4);
    let strip_scroll = gtk::ScrolledWindow::builder()
        .child(&tab_strip)
        .hexpand(true)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hscrollbar_policy(gtk::PolicyType::External)
        .build();
    let new_tab = gtk::Entry::builder().placeholder_text("Tab name, then Enter").build();
    let new_tab_btn = gtk::MenuButton::builder()
        .label("+ New tab")
        .has_frame(false)
        .popover(&gtk::Popover::builder().child(&new_tab).build())
        .build();
    if let Some(b) = new_tab_btn.first_child() {
        b.add_css_class("sr-btn");
        b.add_css_class("sr-quiet");
    }
    let tabs_row = hbox(6);
    tabs_row.add_css_class("sr-tabs");
    tabs_row.append(&strip_scroll);
    tabs_row.append(&new_tab_btn);

    // the open tab's bar
    let entry = gtk::Entry::builder().placeholder_text(view::ADD_HINT).hexpand(true).css_classes(["sr-entry"]).build();
    let add = gtk::Button::builder().label("Add repo").css_classes(["sr-btn"]).build();
    let sort = gtk::DropDown::from_strings(&view::SORT_LABELS);
    sort.set_tooltip_text(Some("Order of this tab"));
    sort.add_css_class("sr-btn");
    let drop_tab = gtk::Button::builder()
        .label("Remove tab")
        .tooltip_text("Take this tab and its repos off the list")
        .css_classes(["sr-btn", "sr-quiet"])
        .build();
    let bar = hbox(8);
    bar.add_css_class("sr-bar");
    bar.append(&entry);
    bar.append(&add);
    bar.append(&sort);
    bar.append(&drop_tab);
    let entry_msg = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).css_classes(["sr-error"]).build();
    entry_msg.set_margin_start(18);
    let totals = gtk::Label::builder().xalign(0.0).selectable(true).css_classes(["sr-totals"]).build();
    let scroll = gtk::ScrolledWindow::builder().vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

    let body = vbox(4);
    body.add_css_class("sr-body");
    body.append(&top);
    if demo {
        body.append(&text(view::DEMO_NOTE, "sr-demo"));
    }
    body.append(&error);
    body.append(&tabs_row);
    body.append(&bar);
    body.append(&entry_msg);
    body.append(&totals);
    body.append(&scroll);
    win.set_child(Some(&body));

    let ui = Rc::new(Ui {
        win: win.clone(),
        pills,
        status,
        error,
        refresh: refresh.clone(),
        tab_strip,
        bar,
        entry: entry.clone(),
        entry_msg,
        sort: sort.clone(),
        totals,
        scroll,
        checks: RefCell::default(),
        tabs: RefCell::default(),
        current: Cell::new(0),
        unfolded: RefCell::default(),
        building: Cell::new(false),
    });

    let w = Rc::downgrade(&ui);
    refresh.connect_clicked(move |_| {
        if let Some(ui) = w.upgrade() {
            ui.refresh();
        }
    });

    let on_add = {
        let w = Rc::downgrade(&ui);
        move || {
            let Some(ui) = w.upgrade() else { return };
            let Some(tab) = ui.tab() else { return };
            match config::repo_name(&ui.entry.text()) {
                Some(repo) => {
                    ui.entry.set_text("");
                    ui.entry_msg.set_visible(false);
                    later(&w, move |l| config::add_repo(l, &repo, &tab.name));
                }
                None => {
                    ui.entry_msg.set_text(view::BAD_REPO);
                    ui.entry_msg.set_visible(true);
                }
            }
        }
    };
    let on_add2 = on_add.clone();
    entry.connect_activate(move |_| on_add());
    add.connect_clicked(move |_| on_add2());

    let w = Rc::downgrade(&ui);
    sort.connect_selected_notify(move |d| {
        let Some(ui) = w.upgrade().filter(|ui| !ui.building.get()) else { return };
        let Some(tab) = ui.tab() else { return };
        let s = Sort::ALL[(d.selected() as usize).min(2)];
        later(&w, move |l| config::set_sort(l, &tab.name, s));
    });

    let w = Rc::downgrade(&ui);
    drop_tab.connect_clicked(move |_| {
        let Some(ui) = w.upgrade() else { return };
        let Some(tab) = ui.tab() else { return };
        let (question, detail) = view::remove_tab_question(&tab.name, tab.repos.len());
        let ask = gtk::AlertDialog::builder()
            .message(question)
            .detail(detail)
            .buttons(["Cancel", "Remove"])
            .cancel_button(0)
            .default_button(0)
            .build();
        let w = w.clone();
        ask.choose(Some(&ui.win), gio::Cancellable::NONE, move |res| {
            if res == Ok(1) {
                later(&w, move |l| config::remove_tab(l, &tab.name));
            }
        });
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
        if let Some(i) = config::load_tabs().unwrap_or_default().iter().position(|t| t.name == name) {
            ui.current.set(i);
            ui.scroll.vadjustment().set_value(0.0);
        }
        render(&ui);
    });

    // keys: F5 / Ctrl+R refresh, Ctrl+L the add field, Ctrl+PageUp/PageDown the tabs, Ctrl+W close
    let keys = gtk::ShortcutController::new();
    keys.set_scope(gtk::ShortcutScope::Global);
    type Key = Box<dyn Fn(&Rc<Ui>)>;
    let key = |trigger: &str, run: Key| {
        let w = Rc::downgrade(&ui);
        let action = gtk::CallbackAction::new(move |_, _| {
            if let Some(ui) = w.upgrade() {
                run(&ui);
            }
            glib::Propagation::Stop
        });
        keys.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string(trigger), Some(action)));
    };
    key("F5|<Control>r", Box::new(|ui| ui.refresh()));
    key(
        "<Control>l",
        Box::new(|ui| {
            ui.entry.grab_focus();
        }),
    );
    key("<Control>Page_Down", Box::new(|ui| ui.show_tab(ui.current.get() + 1)));
    key("<Control>Page_Up", Box::new(|ui| ui.show_tab(ui.current.get().saturating_sub(1))));
    key("<Control>w", Box::new(|ui| ui.win.close()));
    win.add_controller(keys);

    *shown.borrow_mut() = Rc::downgrade(&ui);
    // SLASHREPO_SNAPSHOT_TAB=1: the picture shows the second tab
    if let Some(i) = std::env::var("SLASHREPO_SNAPSHOT_TAB").ok().and_then(|t| t.parse().ok()) {
        ui.current.set(i);
    }
    render(&ui);
    // the window owns the state from here; it lives as long as the window
    win.connect_destroy(move |_| drop(ui.clone()));
    win.present();
    // no field has the keyboard at the start: nothing glows until you click or press Ctrl+L
    gtk::prelude::GtkWindowExt::set_focus(&win, None::<&gtk::Widget>);
}

pub fn run() -> ExitCode {
    // the demo is its own application: it never takes over (or is taken over by) the real window
    let id = if crate::demo::on() { format!("{}.Demo", crate::APP_ID) } else { crate::APP_ID.to_string() };
    let app = gtk::Application::builder().application_id(id).build();
    let shown = RefCell::new(Weak::new());
    app.connect_activate(move |app| activate(app, &shown));
    let code = app.run_with_args::<&str>(&[]);
    if code == glib::ExitCode::SUCCESS { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
