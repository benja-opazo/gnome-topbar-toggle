use cairo::{Antialias, Context, Format, ImageSurface};
use dirs;
use gtk::prelude::*;
use gtk::{FileChooserAction, ResponseType};
use notify_rust::Notification;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};
use tray_icon::{
    TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};

#[derive(Serialize, Deserialize, Clone)]
struct PersistentConfig {
    emoji: String,
    script_path: PathBuf,
    #[serde(default)]
    recents: Vec<String>, // New field
}

impl PersistentConfig {
    fn get_path(id: &str) -> PathBuf {
        let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("bash-toggle");
        let _ = fs::create_dir_all(&path);
        path.push(format!("{}.json", id));
        path
    }

    fn load(id: &str) -> Self {
        let path = Self::get_path(id);
        if let Ok(data) = fs::read_to_string(path) {
            if let Ok(config) = serde_json::from_str(&data) {
                return config;
            }
        }
        // Default values if no file exists
        Self {
            emoji: "🚀".to_string(),
            script_path: PathBuf::from("script.sh"),
            recents: Vec::new(),
        }
    }

    fn save(&self, id: &str) {
        let path = Self::get_path(id);
        if let Ok(data) = serde_json::to_string(self) {
            let _ = fs::write(path, data);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Off,
    TurningOn,
    On,
    Error,
}

struct AppContext {
    state: State,
    emoji: String,
    script_path: std::path::PathBuf, // Store the selected script path
}

fn create_emoji_picker(
    app_state: Arc<Mutex<AppContext>>,
    app_id: String,
    tray: Arc<Mutex<tray_icon::TrayIcon>>,
) -> (gtk::Window, gtk::FlowBox) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);

    window.set_title("Emoji Picker");
    window.set_default_size(450, 600);
    window.set_decorated(false);
    window.set_skip_taskbar_hint(true);

    window.set_type_hint(gtk::gdk::WindowTypeHint::Utility);
    // Ensure the window can actually receive focus events
    window.set_focus_on_map(true);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let notebook = gtk::Notebook::new();
    notebook.set_show_tabs(true);
    notebook.set_show_border(false);
    notebook.set_scrollable(true);

    let scrolled = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);

    // Inside create_emoji_picker
    let adj_scroll = scrolled.vadjustment();
    let current_animation: Arc<Mutex<Option<glib::SourceId>>> = Arc::new(Mutex::new(None));
    // Track the target and start time outside the closure to allow "chaining"
    let animation_data = Arc::new(Mutex::new((0.0, 0.0, std::time::Instant::now())));

    let anim_tracker = current_animation.clone();
    let data_tracker = animation_data.clone();

    scrolled.connect_scroll_event(move |_, event| {
        let (_, dy) = event.scroll_deltas().unwrap_or((0.0, 0.0));

        if dy != 0.0 {
            let mut data = data_tracker.lock().unwrap();
            let mut tracker = anim_tracker.lock().unwrap();

            let current_val = adj_scroll.value();

            // If an animation is already running, we build on top of its target
            let base_y = if tracker.is_some() {
                data.1
            } else {
                current_val
            };
            let new_target = (base_y + (dy * 160.0)).clamp(
                adj_scroll.lower(),
                adj_scroll.upper() - adj_scroll.page_size(),
            );

            // Update the shared animation data
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

                        let elapsed = start_time.elapsed().as_millis() as f64;
                        let duration_ms = 150.0;
                        let t = (elapsed / duration_ms).min(1.0);

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
        } else {
            glib::Propagation::Proceed
        }
    });

    // --- ENABLE SMOOTH SCROLLING ---
    // This enables the smooth-scrolling behavior for mouse wheels and touchpads
    scrolled.set_propagate_natural_height(true);
    scrolled.set_kinetic_scrolling(true);

    // Ensure the scroll policy allows for vertical movement
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

    let adj = scrolled.vadjustment();
    adj.set_step_increment(0.1); // Smaller value = smoother but slower wheel scroll

    let content_vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content_vbox.set_margin(10);

    let category_order = vec![
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
        let group_name = format!("{:?}", emoji.group());
        emoji_groups
            .entry(group_name)
            .or_insert_with(Vec::new)
            .push(emoji.as_str().to_string());
    }

    let mut recents_flowbox = gtk::FlowBox::new();
    let mut sections: Vec<gtk::Box> = Vec::new();

    for (group_id, display_name, icon) in category_order {
        let flowbox = gtk::FlowBox::new();
        flowbox.set_max_children_per_line(9);
        flowbox.set_selection_mode(gtk::SelectionMode::None);

        if group_id == "Recents" {
            recents_flowbox = flowbox.clone();
        }

        let config = PersistentConfig::load(&app_id);
        let list = if group_id == "Recents" {
            config.recents
        } else {
            emoji_groups.get(group_id).cloned().unwrap_or_default()
        };

        if list.is_empty() && group_id != "Recents" {
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

        for emoji_str in list {
            let btn = gtk::Button::with_label(&emoji_str);
            btn.set_relief(gtk::ReliefStyle::None);
            btn.set_size_request(42, 42);

            let app_state_c = Arc::clone(&app_state);
            let tray_c = Arc::clone(&tray);
            let app_id_c = app_id.clone();
            let win_c = window.clone();
            let e_str = emoji_str.clone();

            btn.connect_clicked(move |_| {
                let mut app = app_state_c.lock().unwrap();
                app.emoji = e_str.clone();

                let mut cfg = PersistentConfig::load(&app_id_c);
                cfg.emoji = e_str.clone();
                cfg.recents.retain(|x| x != &e_str);
                cfg.recents.insert(0, e_str.clone());
                cfg.recents.truncate(9);
                cfg.save(&app_id_c);

                let _ = tray_c
                    .lock()
                    .unwrap()
                    .set_icon(Some(create_emoji_icon(&app.emoji, app.state)));
                win_c.hide();
            });
            flowbox.add(&btn);
        }

        content_vbox.add(&section_box);
        sections.push(section_box.clone());

        let tab_label = gtk::Label::new(Some(icon));
        let dummy_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        notebook.append_page(&dummy_page, Some(&tab_label));
        section_box.show_all();
    }

    let adj_animate = adj.clone();
    let content_ref = content_vbox.clone();

    notebook.connect_switch_page(move |_, _, page_num| {
        if let Some(target_section) = sections.get(page_num as usize) {
            let adj_timer = adj_animate.clone();
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

                    // FIX: Clone the adjustment handle specifically for the inner FnMut closure
                    let adj_inner = adj_timer.clone();

                    glib::timeout_add_local(std::time::Duration::from_millis(10), move || {
                        let elapsed = start_time.elapsed().as_millis() as f64;
                        let t = elapsed / duration_ms;

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

    window.connect_delete_event(|win, _| {
        win.hide();
        glib::Propagation::Stop
    });

    window.connect_focus_out_event(|win, _| {
        win.hide();
        glib::Propagation::Stop
    });

    (window, recents_flowbox)
}

fn send_notif(title: &str, body: &str) {
    let _ = Notification::new()
        .summary(title)
        .body(body)
        .appname("BashToggle")
        .show();
}

fn create_emoji_icon(emoji: &str, state: State) -> tray_icon::Icon {
    let size = 32;
    let mut surface =
        ImageSurface::create(Format::ARgb32, size, size).expect("Source surface failed");

    {
        let cr = Context::new(&surface).expect("Context failed");
        cr.set_antialias(Antialias::Best);

        // 1. Main Emoji
        cr.select_font_face(
            "Noto Color Emoji",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Normal,
        );
        cr.set_font_size(22.0);
        let extents = cr.text_extents(emoji).unwrap();
        cr.move_to(
            (size as f64 / 2.0) - (extents.width() / 2.0) - extents.x_bearing() - 2.0,
            (size as f64 / 2.0) - (extents.height() / 2.0) - extents.y_bearing(),
        );
        cr.show_text(emoji).expect("Failed to render emoji");

        // 2. Status Indicator Circle
        let (r, g, b) = match state {
            State::Off => (0.5, 0.5, 0.5),
            State::TurningOn => (0.0, 0.5, 1.0),
            State::On => (0.0, 0.9, 0.0),
            State::Error => (1.0, 0.0, 0.0),
        };

        cr.set_source_rgb(r, g, b);
        cr.arc(26.0, 26.0, 5.0, 0.0, 2.0 * std::f64::consts::PI);
        cr.fill().expect("Failed to fill circle");
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_width(1.0);
        cr.arc(26.0, 26.0, 5.0, 0.0, 2.0 * std::f64::consts::PI);
        let _ = cr.stroke();
    }

    let data = surface.data().expect("Failed to get surface data");
    let mut rgba = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(4) {
        rgba.push(chunk[2]);
        rgba.push(chunk[1]);
        rgba.push(chunk[0]);
        rgba.push(chunk[3]);
    }
    tray_icon::Icon::from_rgba(rgba, size as u32, size as u32).expect("Icon creation failed")
}

fn main() {
    tracing_subscriber::fmt::init();

    // 1. Get ID from input arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: gnome-topbar-toggle <unique-id>");
        std::process::exit(1);
    }
    let app_id = args[1].clone();
    info!("Starting BashToggle with ID: {}", app_id);

    gtk::init().expect("Failed to initialize GTK");

    // Initialize with a default script.sh in the current directory
    let config = PersistentConfig::load(&app_id);

    let app_state = Arc::new(Mutex::new(AppContext {
        state: State::Off,
        emoji: config.emoji.clone(),
        script_path: config.script_path.clone(),
    }));

    // --- SETUP MENU ---
    let menu = Menu::new();
    let toggle_item = MenuItem::with_id("toggle", "State: Off", true, None);
    let add_script_item = MenuItem::with_id("add_script", "Add/Change Script", true, None);
    let emoji_picker_item = MenuItem::with_id("select_emoji", "Browse All Emojis...", true, None);
    //let emoji_submenu = Submenu::with_id("emoji_root", "Select Emoji", true);
    let emojis = vec!["🚀", "⚙️", "🔥", "🤖", "⭐"];
    /*for e in emojis {
        emoji_submenu.append(&MenuItem::with_id(format!("emoji_{}", e), e, true, None)).unwrap();
    }*/

    menu.append_items(&[
        &toggle_item,
        &add_script_item, // NEW ITEM
        &PredefinedMenuItem::separator(),
        &emoji_picker_item, // New item
        //&emoji_submenu,
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("quit", "Quit", true, None),
    ])
    .unwrap();

    let tray_icon = Arc::new(Mutex::new(
        TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(create_emoji_icon(&config.emoji, State::Off))
            .build()
            .unwrap(),
    ));
    // Create the picker window (it will be hidden by default)
    let (emoji_picker_window, recents_box) = create_emoji_picker(
        Arc::clone(&app_state),
        app_id.clone(),
        Arc::clone(&tray_icon),
    );

    let (tx, rx) = glib::MainContext::channel::<State>(glib::Priority::default());
    let menu_channel = MenuEvent::receiver();

    // --- HANDLE BACKGROUND RESPONSES ---
    let app_state_rx = Arc::clone(&app_state);
    let tray_rx = Arc::clone(&tray_icon);
    let toggle_item_rx = toggle_item.clone();

    rx.attach(None, move |next_state| {
        debug!("State update received: {:?}", next_state);
        let mut app = app_state_rx.lock().unwrap();
        app.state = next_state;

        toggle_item_rx.set_text(format!("State: {:?}", next_state));
        let _ = tray_rx
            .lock()
            .unwrap()
            .set_icon(Some(create_emoji_icon(&app.emoji, next_state)));

        if next_state == State::Error {
            error!("Execution failed. Reverting icon in 2s...");
            let app_err = Arc::clone(&app_state_rx);
            let tray_err = Arc::clone(&tray_rx);
            let toggle_err = toggle_item_rx.clone();
            glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
                if let Ok(mut a) = app_err.lock() {
                    a.state = State::Off;
                    toggle_err.set_text("State: Off");
                    let _ = tray_err
                        .lock()
                        .unwrap()
                        .set_icon(Some(create_emoji_icon(&a.emoji, State::Off)));
                }
                glib::ControlFlow::Break
            });
        }
        glib::ControlFlow::Continue
    });

    // --- MAIN EVENT LOOP ---
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if let Ok(event) = menu_channel.try_recv() {
            debug!("Menu interaction: {:?}", event.id);
            if event.id == "quit" {
                info!("Exiting application");
                gtk::main_quit();
            } else if event.id == "add_script" {
                info!("Opening GTK Direct File Chooser...");

                // Create a standard GTK File Chooser Dialog
                let file_chooser = gtk::FileChooserDialog::new(
                    Some("Select Bash Script"),
                    None::<&gtk::Window>,
                    gtk::FileChooserAction::Open,
                );

                // Add the necessary buttons manually for this version
                file_chooser.add_button("_Cancel", gtk::ResponseType::Cancel);
                file_chooser.add_button("_Open", gtk::ResponseType::Accept);

                let app_state_file = Arc::clone(&app_state);

                // For FileChooserDialog, we use a simple run() loop
                // which is safe inside this glib timeout
                let response = file_chooser.run();

                if response == gtk::ResponseType::Accept {
                    if let Some(path) = file_chooser.filename() {
                        let mut app = app_state_file.lock().unwrap();
                        app.script_path = path.clone();

                        // Load existing config to preserve the recents list
                        let current_cfg = PersistentConfig::load(&app_id);

                        PersistentConfig {
                            emoji: app.emoji.clone(),
                            script_path: path,
                            recents: current_cfg.recents, // Fixes E0063
                        }
                        .save(&app_id);

                        send_notif("Script Updated", "Configuration saved.");
                    }
                } else {
                    debug!("File selection cancelled.");
                }

                // Explicitly hide and destroy to clean up the dock icon
                file_chooser.close();
                // unsafe
                //file_chooser.destroy();
            } else if event.id == "select_emoji" {
                if emoji_picker_window.is_visible() {
                    emoji_picker_window.hide();
                    return glib::ControlFlow::Continue;
                }
                let config = PersistentConfig::load(&app_id);

                // 2. Clear the old recent buttons
                for child in recents_box.children() {
                    recents_box.remove(&child);
                }

                // 3. Repopulate with new recents
                for e_str in config.recents {
                    let btn = gtk::Button::with_label(&e_str);
                    btn.set_relief(gtk::ReliefStyle::None);
                    btn.set_size_request(42, 42);

                    // Reuse your existing click logic here to update app_state
                    let app_state_c = Arc::clone(&app_state);
                    let tray_c = Arc::clone(&tray_icon);
                    let app_id_c = app_id.clone();
                    let win_c = emoji_picker_window.clone();
                    let e_val = e_str.clone();

                    btn.connect_clicked(move |_| {
                        let mut app = app_state_c.lock().unwrap();
                        app.emoji = e_val.clone();

                        let mut cfg = PersistentConfig::load(&app_id_c);
                        cfg.recents.retain(|x| x != &e_val);
                        cfg.recents.insert(0, e_val.clone());
                        cfg.recents.truncate(9);
                        cfg.save(&app_id_c);

                        let _ = tray_c
                            .lock()
                            .unwrap()
                            .set_icon(Some(create_emoji_icon(&app.emoji, app.state)));
                        win_c.hide();
                    });

                    recents_box.add(&btn);
                }

                // --- POSITION WINDOW AT MOUSE ---
                // 1. Get the default display and the pointer position
                let display = gtk::gdk::Display::default().expect("Could not get default display");
                let seat = display.default_seat().expect("Could not get default seat");
                let device = seat.pointer().expect("Could not get pointer device");

                // 2. Get the screen and coordinates
                let (_, x, y) = device.position();

                // 3. Move the window before showing it
                // We subtract a small offset (e.g., 10px) so the window doesn't appear
                // exactly under the cursor, which could trigger an immediate click
                emoji_picker_window.move_(x - 225, y - 100);

                emoji_picker_window.show_all();
                let win_clone = emoji_picker_window.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                    win_clone.present();
                    glib::ControlFlow::Break
                });
            } else if event.id == "toggle" {
                let mut app = app_state.lock().unwrap();
                if app.state == State::Off || app.state == State::Error {
                    info!("Executing: {:?}", app.script_path);
                    app.state = State::TurningOn;

                    let _ = tray_icon
                        .lock()
                        .unwrap()
                        .set_icon(Some(create_emoji_icon(&app.emoji, State::TurningOn)));
                    toggle_item.set_text("State: Turning on...");

                    let tx_clone = tx.clone();
                    let script_to_run = app.script_path.clone(); // Clone path for the thread

                    std::thread::spawn(move || {
                        let res = Command::new("bash").arg(script_to_run).output();
                        info!("Background thread: Executing Script");

                        let final_state = match &res {
                            Ok(output) => {
                                if output.status.success() {
                                    info!(
                                        "Script completed successfully (Exit Code 0). Reverting to Off."
                                    );
                                    debug!(
                                        "Stdout: {}",
                                        String::from_utf8_lossy(&output.stdout).trim()
                                    );

                                    let _ = tx_clone.send(State::On);
                                    std::thread::sleep(std::time::Duration::from_secs_f32(1.5));
                                    // SUCCESS: Go back to Off state
                                    State::Off
                                } else {
                                    let exit_code = output
                                        .status
                                        .code()
                                        .map(|c| c.to_string())
                                        .unwrap_or_else(|| "Signaled".into());
                                    let stderr = String::from_utf8_lossy(&output.stderr);

                                    error!("Script failed!");
                                    error!("  Exit Code: {}", exit_code);
                                    if !stderr.is_empty() {
                                        error!("  Stderr: {}", stderr.trim());
                                    }
                                    // FAILURE: Stay in Error state (which triggers the 2s red icon)
                                    State::Error
                                }
                            }
                            Err(e) => {
                                error!("Failed to launch bash process: {}", e);
                                State::Error
                            }
                        };

                        let _ = tx_clone.send(final_state);
                    });
                } else {
                    info!("Manual reset to Off");
                    app.state = State::Off;
                    toggle_item.set_text("State: Off");
                    let _ = tray_icon
                        .lock()
                        .unwrap()
                        .set_icon(Some(create_emoji_icon(&app.emoji, State::Off)));
                }
            }
        }
        glib::ControlFlow::Continue
    });

    gtk::main();
}
