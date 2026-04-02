mod scroll;

use gtk::prelude::*;
use scroll::{SmoothScroller, scroll_to_widget};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub struct EmojiPickerConfig {
    pub window_width: i32,
    pub window_height: i32,
    pub button_size: i32,
    pub columns: u32,
    pub content_padding: i32,
}

impl Default for EmojiPickerConfig {
    fn default() -> Self {
        Self {
            window_width: 450,
            window_height: 600,
            button_size: 42,
            columns: 9,
            content_padding: 10,
        }
    }
}

pub struct EmojiPicker {
    window: gtk::Window,
    recents_flowbox: gtk::FlowBox,
    on_select: Arc<dyn Fn(String) + 'static>,
    suppress_hide: Arc<Mutex<bool>>,
    pending_present: Arc<Mutex<Option<glib::SourceId>>>,
    config: EmojiPickerConfig,
}

impl EmojiPicker {
    pub fn new(config: EmojiPickerConfig, on_select: impl Fn(String) + 'static) -> Self {
        let on_select: Arc<dyn Fn(String)> = Arc::new(on_select);

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("Emoji Picker");
        window.set_default_size(config.window_width, config.window_height);
        window.set_decorated(false);
        window.set_skip_taskbar_hint(true);
        window.set_type_hint(gtk::gdk::WindowTypeHint::Utility);
        window.set_focus_on_map(true);

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let scrolled =
            gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        let scroller = SmoothScroller::new(scrolled.vadjustment());
        scroller.attach(&scrolled);
        scrolled.set_propagate_natural_height(true);
        scrolled.set_kinetic_scrolling(true);
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scrolled.vadjustment().set_step_increment(0.1);

        let EmojiContent {
            notebook,
            content_vbox,
            recents_flowbox,
            sections,
        } = build_emoji_content(&on_select, &window, config);

        let adj = scrolled.vadjustment();
        let content_ref = content_vbox.clone();
        notebook.connect_switch_page(move |_, _, page_num| {
            if let Some(section) = sections.get(page_num as usize) {
                scroll_to_widget(adj.clone(), section.clone(), content_ref.clone());
            }
        });

        scrolled.add(&content_vbox);
        vbox.pack_start(&notebook, false, false, 0);
        vbox.pack_start(&scrolled, true, true, 0);
        window.add(&vbox);

        let suppress_hide: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let pending_present: Arc<Mutex<Option<glib::SourceId>>> = Arc::new(Mutex::new(None));

        let suppress_hide_fi = Arc::clone(&suppress_hide);
        let suppress_hide_fo = Arc::clone(&suppress_hide);
        let pending_fo = Arc::clone(&pending_present);

        window.connect_delete_event(|win, _| {
            win.hide();
            glib::Propagation::Stop
        });

        window.connect_focus_in_event(move |_, _| {
            *suppress_hide_fi.lock().unwrap() = false;
            glib::Propagation::Proceed
        });

        window.connect_focus_out_event(move |win, _| {
            if !*suppress_hide_fo.lock().unwrap() {
                if let Some(id) = pending_fo.lock().unwrap().take() {
                    id.remove();
                }
                win.hide();
            }
            glib::Propagation::Stop
        });

        Self {
            window,
            recents_flowbox,
            on_select,
            suppress_hide,
            pending_present,
            config,
        }
    }

    pub fn refresh_recents(&self, recents: &[String]) {
        for child in self.recents_flowbox.children() {
            self.recents_flowbox.remove(&child);
        }
        for emoji_str in recents {
            add_emoji_button(
                &self.recents_flowbox,
                emoji_str,
                Arc::clone(&self.on_select),
                self.window.clone(),
                self.config.button_size,
            );
        }
        self.recents_flowbox.show_all();
    }

    pub fn show_at(&self, x: i32, y: i32) {
        if let Some(id) = self.pending_present.lock().unwrap().take() {
            id.remove();
        }

        *self.suppress_hide.lock().unwrap() = true;
        self.window.move_(x - self.config.window_width / 2, y - 100);
        self.window.show_all();

        let win = self.window.clone();
        let pending = Arc::clone(&self.pending_present);
        let source_id = glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            *pending.lock().unwrap() = None;
            win.present();
            glib::ControlFlow::Break
        });
        *self.pending_present.lock().unwrap() = Some(source_id);
    }

    pub fn hide(&self) {
        if let Some(id) = self.pending_present.lock().unwrap().take() {
            id.remove();
        }
        *self.suppress_hide.lock().unwrap() = false;
        self.window.hide();
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }
}

// ── Emoji content ─────────────────────────────────────────────────────────────

const CATEGORIES: &[(&str, &str, &str)] = &[
    ("Recents", "Recently Used", "🕒"),
    ("SmileysAndEmotion", "Smileys", "😀"),
    ("PeopleAndBody", "People", "👋"),
    ("AnimalsAndNature", "Nature", "🐶"),
    ("FoodAndDrink", "Food", "🍎"),
    ("TravelAndPlaces", "Travel", "🚗"),
    ("Activities", "Activities", "⚽"),
    ("Objects", "Objects", "💡"),
    ("Symbols", "Symbols", "🔣"),
    ("Flags", "Flags", "🏁"),
];

struct EmojiContent {
    notebook: gtk::Notebook,
    content_vbox: gtk::Box,
    recents_flowbox: gtk::FlowBox,
    sections: Vec<gtk::Box>,
}

fn build_emoji_content(
    on_select: &Arc<dyn Fn(String) + 'static>,
    window: &gtk::Window,
    config: EmojiPickerConfig,
) -> EmojiContent {
    let emoji_groups = grouped_emojis();

    let notebook = gtk::Notebook::new();
    notebook.set_show_tabs(true);
    notebook.set_show_border(false);
    notebook.set_scrollable(true);

    let content_vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content_vbox.set_margin(config.content_padding);

    let mut recents_flowbox = gtk::FlowBox::new();
    let mut sections = Vec::new();

    for &(group_id, display_name, icon) in CATEGORIES {
        let list: Vec<String> = if group_id == "Recents" {
            Vec::new()
        } else {
            emoji_groups.get(group_id).cloned().unwrap_or_default()
        };

        if list.is_empty() && group_id != "Recents" {
            continue;
        }

        let (section_box, flowbox) =
            build_category_section(display_name, &list, on_select, window, config);

        if group_id == "Recents" {
            recents_flowbox = flowbox;
        }

        content_vbox.add(&section_box);
        sections.push(section_box.clone());
        notebook.append_page(
            &gtk::Box::new(gtk::Orientation::Vertical, 0),
            Some(&gtk::Label::new(Some(icon))),
        );
        section_box.show_all();
    }

    EmojiContent {
        notebook,
        content_vbox,
        recents_flowbox,
        sections,
    }
}

fn grouped_emojis() -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for emoji in emojis::iter() {
        map.entry(format!("{:?}", emoji.group()))
            .or_default()
            .push(emoji.as_str().to_string());
    }
    map
}

fn build_category_section(
    display_name: &str,
    list: &[String],
    on_select: &Arc<dyn Fn(String) + 'static>,
    window: &gtk::Window,
    config: EmojiPickerConfig,
) -> (gtk::Box, gtk::FlowBox) {
    let flowbox = gtk::FlowBox::new();
    flowbox.set_max_children_per_line(config.columns);
    flowbox.set_selection_mode(gtk::SelectionMode::None);

    for emoji_str in list {
        add_emoji_button(&flowbox, emoji_str, Arc::clone(on_select), window.clone(), config.button_size);
    }

    let label = gtk::Label::new(None);
    label.set_markup(&format!(
        "<span size='large' weight='bold'>{}</span>",
        display_name
    ));
    label.set_halign(gtk::Align::Start);
    label.set_margin_bottom(5);

    let section_box = gtk::Box::new(gtk::Orientation::Vertical, 5);
    section_box.add(&label);
    section_box.add(&flowbox);

    (section_box, flowbox)
}

fn add_emoji_button(
    flowbox: &gtk::FlowBox,
    emoji_str: &str,
    on_select: Arc<dyn Fn(String) + 'static>,
    window: gtk::Window,
    button_size: i32,
) {
    let btn = gtk::Button::with_label(emoji_str);
    btn.set_relief(gtk::ReliefStyle::None);
    btn.set_size_request(button_size, button_size);

    let e = emoji_str.to_string();
    btn.connect_clicked(move |_| {
        window.hide();
        on_select(e.clone());
    });

    flowbox.add(&btn);
}
