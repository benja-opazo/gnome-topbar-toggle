use gtk::prelude::*;
use gtk::{FileChooserAction, FileChooserNative, ResponseType};
use std::path::PathBuf;
use notify_rust::Notification;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    TrayIconBuilder,
};
use cairo::{Context, ImageSurface, Format, Antialias};
use tracing::{info, debug, error, warn};


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

fn send_notif(title: &str, body: &str) {
    let _ = Notification::new().summary(title).body(body).appname("BashToggle").show();
}

fn create_emoji_icon(emoji: &str, state: State) -> tray_icon::Icon {
    let size = 32;
    let mut surface = ImageSurface::create(Format::ARgb32, size, size).expect("Source surface failed");
    
    {
        let cr = Context::new(&surface).expect("Context failed");
        cr.set_antialias(Antialias::Best);

        // 1. Main Emoji
        cr.select_font_face("Noto Color Emoji", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
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
        rgba.push(chunk[2]); rgba.push(chunk[1]); rgba.push(chunk[0]); rgba.push(chunk[3]);
    }
    tray_icon::Icon::from_rgba(rgba, size as u32, size as u32).expect("Icon creation failed")
}

fn main() {
    tracing_subscriber::fmt::init();
    info!("Initializing gnome-topbar-toggle...");

    gtk::init().expect("Failed to initialize GTK");

    // Initialize with a default script.sh in the current directory
    let app_state = Arc::new(Mutex::new(AppContext {
        state: State::Off,
        emoji: "🚀".to_string(),
        script_path: PathBuf::from("script.sh"), 
    }));

    // --- SETUP MENU ---
    let menu = Menu::new();
    let toggle_item = MenuItem::with_id("toggle", "State: Off", true, None);
    let add_script_item = MenuItem::with_id("add_script", "Add/Change Script", true, None);
    let emoji_submenu = Submenu::with_id("emoji_root", "Select Emoji", true);
    let emojis = vec!["🚀", "⚙️", "🔥", "🤖", "⭐"];
    for e in emojis {
        emoji_submenu.append(&MenuItem::with_id(format!("emoji_{}", e), e, true, None)).unwrap();
    }

    menu.append_items(&[
        &toggle_item,
        &add_script_item, // NEW ITEM
        &PredefinedMenuItem::separator(),
        &emoji_submenu,
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("quit", "Quit", true, None),
    ]).unwrap();

    let tray_icon = Arc::new(Mutex::new(
        TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(create_emoji_icon("🚀", State::Off))
            .build()
            .unwrap()
    ));

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
        let _ = tray_rx.lock().unwrap().set_icon(Some(create_emoji_icon(&app.emoji, next_state)));

        if next_state == State::Error {
            error!("Execution failed. Reverting icon in 2s...");
            let app_err = Arc::clone(&app_state_rx);
            let tray_err = Arc::clone(&tray_rx);
            let toggle_err = toggle_item_rx.clone();
            glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
                if let Ok(mut a) = app_err.lock() {
                    a.state = State::Off;
                    toggle_err.set_text("State: Off");
                    let _ = tray_err.lock().unwrap().set_icon(Some(create_emoji_icon(&a.emoji, State::Off)));
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
                        info!("New script selected: {:?}", path);
                        let mut app = app_state_file.lock().unwrap();
                        app.script_path = path;
                        send_notif("Script Updated", "Ready to toggle.");
                    }
                } else {
                    debug!("File selection cancelled.");
                }

                // Explicitly hide and destroy to clean up the dock icon
                file_chooser.close();
                // unsafe
                //file_chooser.destroy();
                
            } else if event.id.as_ref().starts_with("emoji_") {
                let new_emoji = event.id.as_ref().replace("emoji_", "");
                info!("Emoji updated to: {}", new_emoji);
                let mut app = app_state.lock().unwrap();
                app.emoji = new_emoji;
                let _ = tray_icon.lock().unwrap().set_icon(Some(create_emoji_icon(&app.emoji, app.state)));
            } else if event.id == "toggle" {
                let mut app = app_state.lock().unwrap();
                if app.state == State::Off || app.state == State::Error {
                    info!("Executing: {:?}", app.script_path);
                    app.state = State::TurningOn;

                    let _ = tray_icon.lock().unwrap().set_icon(Some(create_emoji_icon(&app.emoji, State::TurningOn)));
                    toggle_item.set_text("State: Turning on...");
                    
                    let tx_clone = tx.clone();
                    let script_to_run = app.script_path.clone(); // Clone path for the thread

                    std::thread::spawn(move || {
                        let res = Command::new("bash").arg(script_to_run).output();
                        info!("Background thread: Executing Script");
                        
                        let final_state = match &res {
                            Ok(output) => {
                                if output.status.success() {
                                    info!("Script completed successfully (Exit Code 0). Reverting to Off.");
                                    debug!("Stdout: {}", String::from_utf8_lossy(&output.stdout).trim());

                                    let _ = tx_clone.send(State::On);
                                    std::thread::sleep(std::time::Duration::from_secs_f32(1.5));
                                    // SUCCESS: Go back to Off state
                                    State::Off
                                } else {
                                    let exit_code = output.status.code().map(|c| c.to_string()).unwrap_or_else(|| "Signaled".into());
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
                    let _ = tray_icon.lock().unwrap().set_icon(Some(create_emoji_icon(&app.emoji, State::Off)));
                }
            }
        }
        glib::ControlFlow::Continue
    });

    gtk::main();
}