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

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let notebook = gtk::Notebook::new();
    notebook.set_show_tabs(true);
    notebook.set_show_border(false);
    notebook.set_scrollable(true);

    let scrolled = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    let adj = scrolled.vadjustment();

    let content_vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content_vbox.set_margin(10);

    // ID = Internal Library Name, Name = Custom UI Label, Icon = Tab Icon
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

        // CRITICAL FIX: Use group_id for lookup, NOT display_name
        let list = if group_id == "Recents" {
            config.recents
        } else {
            emoji_groups.get(group_id).cloned().unwrap_or_default()
        };

        // Ensure the category shows up if it has emojis or is the Recents tab
        if list.is_empty() && group_id != "Recents" {
            continue;
        }

        let section_box = gtk::Box::new(gtk::Orientation::Vertical, 5);

        // Label using the Custom Display Name
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

        // Ensure all child widgets (including the label) are rendered
        section_box.show_all();
    }

    let adj_animate = adj.clone();
    let content_ref = content_vbox.clone();
    notebook.connect_switch_page(move |_, _, page_num| {
        if let Some(target_section) = sections.get(page_num as usize) {
            let adj_timer = adj_animate.clone();
            let target = target_section.clone();
            let container = content_ref.clone();

            glib::timeout_add_local(std::time::Duration::from_millis(20), move || {
                if let Some((_, target_y)) = target.translate_coordinates(&container, 0, 0) {
                    let target_y = target_y as f64;
                    let start_y = adj_timer.value();
                    let distance = target_y - start_y;
                    let steps = 12;
                    let mut current_step = 0;

                    let adj_frame = adj_timer.clone();
                    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                        current_step += 1;
                        let progress = current_step as f64 / steps as f64;
                        let ease_out = 1.0 - (1.0 - progress).powi(3);

                        adj_frame.set_value(start_y + (distance * ease_out));

                        if current_step < steps {
                            glib::ControlFlow::Continue
                        } else {
                            glib::ControlFlow::Break
                        }
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

                emoji_picker_window.show_all();
                emoji_picker_window.present();
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
