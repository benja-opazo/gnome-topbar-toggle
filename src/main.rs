use cairo::{Antialias, Context, Format, ImageSurface};
use dirs;
use emoji_picker::EmojiPicker;
use gtk::prelude::*;
use notify_rust::Notification;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info};
use tray_icon::{
    TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[derive(Serialize, Deserialize, Clone)]
struct PersistentConfig {
    emoji: String,
    script_path: PathBuf,
    #[serde(default)]
    recents: Vec<String>,
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
    Executing,
    Finished,
    Error,
}

struct AppContext {
    state: State,
    emoji: String,
    script_path: PathBuf,
    cancel_tx: Option<std::sync::mpsc::Sender<()>>,
    child_pgid: Option<u32>,
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

        let (r, g, b) = match state {
            State::Off => (0.5, 0.5, 0.5),
            State::Executing => (0.0, 0.5, 1.0),
            State::Finished => (0.0, 0.9, 0.0),
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

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: gnome-topbar-toggle <unique-id>");
        std::process::exit(1);
    }
    let app_id = args[1].clone();
    info!("Starting BashToggle with ID: {}", app_id);

    gtk::init().expect("Failed to initialize GTK");

    let config = PersistentConfig::load(&app_id);

    let app_state = Arc::new(Mutex::new(AppContext {
        state: State::Off,
        emoji: config.emoji.clone(),
        script_path: config.script_path.clone(),
        cancel_tx: None,
        child_pgid: None,
    }));

    let menu = Menu::new();
    let toggle_item = MenuItem::with_id("toggle", "State: Off", true, None);
    let add_script_item = MenuItem::with_id("add_script", "Add/Change Script", true, None);
    let emoji_picker_item = MenuItem::with_id("select_emoji", "Browse All Emojis...", true, None);

    menu.append_items(&[
        &toggle_item,
        &add_script_item,
        &PredefinedMenuItem::separator(),
        &emoji_picker_item,
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("quit", "Quit", true, None),
    ])
    .unwrap();

    let tray_icon = Arc::new(Mutex::new(
        TrayIconBuilder::new()
            .with_temp_dir_path(std::env::temp_dir().join(format!("tray-icon-{}", app_id)))
            .with_menu(Box::new(menu))
            .with_icon(create_emoji_icon(&config.emoji, State::Off))
            .build()
            .unwrap(),
    ));

    let app_state_cb = Arc::clone(&app_state);
    let tray_cb = Arc::clone(&tray_icon);
    let app_id_cb = app_id.clone();

    let emoji_picker = EmojiPicker::new(move |emoji| {
        let mut app = app_state_cb.lock().unwrap();
        app.emoji = emoji.clone();

        let mut cfg = PersistentConfig::load(&app_id_cb);
        cfg.emoji = emoji.clone();
        cfg.recents.retain(|x| x != &emoji);
        cfg.recents.insert(0, emoji.clone());
        cfg.recents.truncate(9);
        cfg.save(&app_id_cb);

        let _ = tray_cb
            .lock()
            .unwrap()
            .set_icon(Some(create_emoji_icon(&app.emoji, app.state)));
    });

    let (tx, rx) = glib::MainContext::channel::<State>(glib::Priority::default());
    let menu_channel = MenuEvent::receiver();

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

    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if let Ok(event) = menu_channel.try_recv() {
            debug!("Menu interaction: {:?}", event.id);
            if event.id == "quit" {
                if let Some(pgid) = app_state.lock().unwrap().child_pgid {
                    info!("Killing process group {} before exit", pgid);
                    unsafe {
                        libc::kill(-(pgid as libc::pid_t), libc::SIGTERM);
                    }
                }
                info!("Exiting application");
                gtk::main_quit();
            } else if event.id == "add_script" {
                info!("Opening file chooser...");

                let file_chooser = gtk::FileChooserDialog::new(
                    Some("Select Bash Script"),
                    None::<&gtk::Window>,
                    gtk::FileChooserAction::Open,
                );
                file_chooser.add_button("_Cancel", gtk::ResponseType::Cancel);
                file_chooser.add_button("_Open", gtk::ResponseType::Accept);

                let app_state_file = Arc::clone(&app_state);
                let response = file_chooser.run();

                if response == gtk::ResponseType::Accept {
                    if let Some(path) = file_chooser.filename() {
                        let mut app = app_state_file.lock().unwrap();
                        app.script_path = path.clone();

                        let current_cfg = PersistentConfig::load(&app_id);
                        PersistentConfig {
                            emoji: app.emoji.clone(),
                            script_path: path,
                            recents: current_cfg.recents,
                        }
                        .save(&app_id);

                        send_notif("Script Updated", "Configuration saved.");
                    }
                } else {
                    debug!("File selection cancelled.");
                }

                file_chooser.close();
            } else if event.id == "select_emoji" {
                if emoji_picker.is_visible() {
                    emoji_picker.hide();
                    return glib::ControlFlow::Continue;
                }

                let config = PersistentConfig::load(&app_id);
                emoji_picker.refresh_recents(&config.recents);

                let display =
                    gtk::gdk::Display::default().expect("Could not get default display");
                let seat = display.default_seat().expect("Could not get default seat");
                let device = seat.pointer().expect("Could not get pointer device");
                let (_, x, y) = device.position();

                emoji_picker.show_at(x, y);
            } else if event.id == "toggle" {
                let mut app = app_state.lock().unwrap();
                if app.state == State::Off || app.state == State::Error {
                    info!("Executing: {:?}", app.script_path);
                    app.state = State::Executing;

                    let _ = tray_icon
                        .lock()
                        .unwrap()
                        .set_icon(Some(create_emoji_icon(&app.emoji, State::Executing)));
                    toggle_item.set_text("State: Executing...");

                    let tx_clone = tx.clone();
                    let script_to_run = app.script_path.clone();
                    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
                    app.cancel_tx = Some(cancel_tx);
                    drop(app);

                    let app_state_thread = Arc::clone(&app_state);

                    std::thread::spawn(move || {
                        use std::io::Read;

                        let child = Command::new("bash")
                            .arg(script_to_run)
                            .stdout(std::process::Stdio::piped())
                            .stderr(std::process::Stdio::piped())
                            .process_group(0)
                            .spawn();

                        let mut child = match child {
                            Err(e) => {
                                error!("Failed to launch bash process: {}", e);
                                let _ = tx_clone.send(State::Error);
                                return;
                            }
                            Ok(c) => c,
                        };

                        app_state_thread.lock().unwrap().child_pgid = Some(child.id());
                        info!("Script started");

                        loop {
                            if cancel_rx.try_recv().is_ok() {
                                info!("Script cancelled, killing process group");
                                unsafe {
                                    libc::kill(-(child.id() as libc::pid_t), libc::SIGTERM);
                                }
                                let _ = child.wait();
                                app_state_thread.lock().unwrap().child_pgid = None;
                                return;
                            }
                            match child.try_wait() {
                                Ok(Some(status)) => {
                                    app_state_thread.lock().unwrap().child_pgid = None;
                                    let final_state = if status.success() {
                                        let mut stdout = String::new();
                                        if let Some(mut out) = child.stdout.take() {
                                            let _ = out.read_to_string(&mut stdout);
                                        }
                                        info!("Script completed successfully.");
                                        debug!("Stdout: {}", stdout.trim());
                                        let _ = tx_clone.send(State::Finished);
                                        std::thread::sleep(std::time::Duration::from_secs_f32(1.5));
                                        State::Off
                                    } else {
                                        let exit_code = status
                                            .code()
                                            .map(|c| c.to_string())
                                            .unwrap_or_else(|| "Signaled".into());
                                        let mut stderr = String::new();
                                        if let Some(mut err) = child.stderr.take() {
                                            let _ = err.read_to_string(&mut stderr);
                                        }
                                        error!("Script failed! Exit Code: {}", exit_code);
                                        if !stderr.is_empty() {
                                            error!("Stderr: {}", stderr.trim());
                                        }
                                        let body = if stderr.trim().is_empty() {
                                            format!("Exit code: {}", exit_code)
                                        } else {
                                            format!("Exit code: {}\n{}", exit_code, stderr.trim())
                                        };
                                        send_notif("Script Failed", &body);
                                        State::Error
                                    };
                                    let _ = tx_clone.send(final_state);
                                    return;
                                }
                                Ok(None) => {
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                }
                                Err(e) => {
                                    error!("Failed to wait for process: {}", e);
                                    app_state_thread.lock().unwrap().child_pgid = None;
                                    let _ = tx_clone.send(State::Error);
                                    return;
                                }
                            }
                        }
                    });
                } else if app.state == State::Executing {
                    info!("Cancelling script execution");
                    if let Some(tx) = app.cancel_tx.take() {
                        let _ = tx.send(());
                    }
                    app.state = State::Off;
                    toggle_item.set_text("State: Off");
                    let _ = tray_icon
                        .lock()
                        .unwrap()
                        .set_icon(Some(create_emoji_icon(&app.emoji, State::Off)));
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
