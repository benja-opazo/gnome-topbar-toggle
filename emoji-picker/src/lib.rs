use gtk::prelude::*;
use std::sync::{Arc, Mutex};

pub struct EmojiPicker {
    window: gtk::Window,
    recents_flowbox: gtk::FlowBox,
    on_select: Arc<dyn Fn(String) + 'static>,
    suppress_hide: Arc<Mutex<bool>>,
}

impl EmojiPicker {
    pub fn new(on_select: impl Fn(String) + 'static) -> Self {
        let on_select: Arc<dyn Fn(String)> = Arc::new(on_select);

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("Emoji Picker");
        window.set_default_size(450, 600);
        window.set_decorated(false);
        window.set_skip_taskbar_hint(true);
        window.set_type_hint(gtk::gdk::WindowTypeHint::Utility);
        window.set_focus_on_map(true);

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let notebook = gtk::Notebook::new();
        notebook.set_show_tabs(true);
        notebook.set_show_border(false);
        notebook.set_scrollable(true);

        let scrolled =
            gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);

        let adj_scroll = scrolled.vadjustment();
        let current_animation: Arc<Mutex<Option<glib::SourceId>>> = Arc::new(Mutex::new(None));
        let animation_data = Arc::new(Mutex::new((0.0_f64, 0.0_f64, std::time::Instant::now())));

        let anim_tracker = current_animation.clone();
        let data_tracker = animation_data.clone();

        scrolled.connect_scroll_event(move |_, event| {
            let (_, dy) = event.scroll_deltas().unwrap_or((0.0, 0.0));
            if dy == 0.0 {
                return glib::Propagation::Proceed;
            }

            let mut data = data_tracker.lock().unwrap();
            let mut tracker = anim_tracker.lock().unwrap();

            let current_val = adj_scroll.value();
            let base_y = if tracker.is_some() { data.1 } else { current_val };
            let new_target = (base_y + (dy * 160.0))
                .clamp(adj_scroll.lower(), adj_scroll.upper() - adj_scroll.page_size());

            *data = (current_val, new_target, std::time::Instant::now());

            if tracker.is_none() {
                let adj_inner = adj_scroll.clone();
                let anim_tracker_inner = anim_tracker.clone();
                let data_inner = data_tracker.clone();

                let source_id =
                    glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
                        let (start_y, target_y, start_time) = {
                            let d = data_inner.lock().unwrap();
                            (d.0, d.1, d.2)
                        };
                        let t = (start_time.elapsed().as_millis() as f64 / 150.0).min(1.0);
                        let progress = 1.0 - (1.0 - t).powi(3);
                        adj_inner.set_value(start_y + ((target_y - start_y) * progress));

                        if t < 1.0 {
                            glib::ControlFlow::Continue
                        } else {
                            *anim_tracker_inner.lock().unwrap() = None;
                            glib::ControlFlow::Break
                        }
                    });
                *tracker = Some(source_id);
            }

            glib::Propagation::Stop
        });

        scrolled.set_propagate_natural_height(true);
        scrolled.set_kinetic_scrolling(true);
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scrolled.vadjustment().set_step_increment(0.1);

        let content_vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
        content_vbox.set_margin(10);

        let category_order = [
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

        let mut emoji_groups: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for emoji in emojis::iter() {
            emoji_groups
                .entry(format!("{:?}", emoji.group()))
                .or_default()
                .push(emoji.as_str().to_string());
        }

        let mut recents_flowbox = gtk::FlowBox::new();
        let mut sections: Vec<gtk::Box> = Vec::new();

        for (group_id, display_name, icon) in &category_order {
            let flowbox = gtk::FlowBox::new();
            flowbox.set_max_children_per_line(9);
            flowbox.set_selection_mode(gtk::SelectionMode::None);

            if *group_id == "Recents" {
                recents_flowbox = flowbox.clone();
            }

            let list: Vec<String> = if *group_id == "Recents" {
                Vec::new()
            } else {
                emoji_groups.get(*group_id).cloned().unwrap_or_default()
            };

            if list.is_empty() && *group_id != "Recents" {
                continue;
            }

            let section_box = gtk::Box::new(gtk::Orientation::Vertical, 5);
            let label = gtk::Label::new(None);
            label.set_markup(&format!(
                "<span size='large' weight='bold'>{}</span>",
                display_name
            ));
            label.set_halign(gtk::Align::Start);
            label.set_margin_bottom(5);
            section_box.add(&label);
            section_box.add(&flowbox);

            for emoji_str in &list {
                add_emoji_button(&flowbox, emoji_str, Arc::clone(&on_select), window.clone());
            }

            content_vbox.add(&section_box);
            sections.push(section_box.clone());

            notebook.append_page(
                &gtk::Box::new(gtk::Orientation::Vertical, 0),
                Some(&gtk::Label::new(Some(icon))),
            );
            section_box.show_all();
        }

        let adj = scrolled.vadjustment();
        let content_ref = content_vbox.clone();

        notebook.connect_switch_page(move |_, _, page_num| {
            if let Some(target_section) = sections.get(page_num as usize) {
                let adj_timer = adj.clone();
                let container = content_ref.clone();
                let target = target_section.clone();

                glib::timeout_add_local(std::time::Duration::from_millis(10), move || {
                    if let Some((_, target_y)) = target.translate_coordinates(&container, 0, 0) {
                        let start_y = adj_timer.value();
                        let end_y = target_y as f64;
                        let distance = end_y - start_y;

                        if distance.abs() < 1.0 {
                            return glib::ControlFlow::Break;
                        }

                        let duration_ms = 250.0;
                        let start_time = std::time::Instant::now();
                        let adj_inner = adj_timer.clone();

                        glib::timeout_add_local(std::time::Duration::from_millis(10), move || {
                            let t = start_time.elapsed().as_millis() as f64 / duration_ms;
                            if t >= 1.0 {
                                adj_inner.set_value(end_y);
                                return glib::ControlFlow::Break;
                            }
                            let progress = 1.0 - (1.0 - t).powi(5);
                            adj_inner.set_value(start_y + (distance * progress));
                            glib::ControlFlow::Continue
                        });
                    }
                    glib::ControlFlow::Break
                });
            }
        });

        scrolled.add(&content_vbox);
        vbox.pack_start(&notebook, false, false, 0);
        vbox.pack_start(&scrolled, true, true, 0);
        window.add(&vbox);

        let suppress_hide: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let suppress_hide_fo = Arc::clone(&suppress_hide);

        window.connect_delete_event(|win, _| {
            win.hide();
            glib::Propagation::Stop
        });

        window.connect_focus_out_event(move |win, _| {
            if !*suppress_hide_fo.lock().unwrap() {
                win.hide();
            }
            glib::Propagation::Stop
        });

        Self {
            window,
            recents_flowbox,
            on_select,
            suppress_hide,
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
            );
        }
        self.recents_flowbox.show_all();
    }

    pub fn show_at(&self, x: i32, y: i32) {
        *self.suppress_hide.lock().unwrap() = true;
        self.window.move_(x - 225, y - 100);
        self.window.show_all();
        let win = self.window.clone();
        let suppress = Arc::clone(&self.suppress_hide);
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            win.present();
            *suppress.lock().unwrap() = false;
            glib::ControlFlow::Break
        });
    }

    pub fn hide(&self) {
        self.window.hide();
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }
}

fn add_emoji_button(
    flowbox: &gtk::FlowBox,
    emoji_str: &str,
    on_select: Arc<dyn Fn(String) + 'static>,
    window: gtk::Window,
) {
    let btn = gtk::Button::with_label(emoji_str);
    btn.set_relief(gtk::ReliefStyle::None);
    btn.set_size_request(42, 42);

    let e = emoji_str.to_string();
    btn.connect_clicked(move |_| {
        window.hide();
        on_select(e.clone());
    });

    flowbox.add(&btn);
}
