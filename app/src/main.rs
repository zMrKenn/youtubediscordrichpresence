#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod discord;

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, Rounding, Sense, Stroke};
use obfstr::obfstr;
use serde_json::{Map, Value};

use discord::Discord;

const STALE_SECS: u64 = 30;

const BG: Color32 = Color32::from_rgb(0x0f, 0x0f, 0x0f);
const CARD: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
const CARD2: Color32 = Color32::from_rgb(0x21, 0x21, 0x21);
const ACCENT: Color32 = Color32::from_rgb(0x8b, 0x5c, 0xf6);
const TEXT: Color32 = Color32::from_rgb(0xf1, 0xf1, 0xf1);
const MUTED: Color32 = Color32::from_rgb(0xaa, 0xaa, 0xaa);
const GREEN: Color32 = Color32::from_rgb(0x2b, 0xa6, 0x40);
const TRACK: Color32 = Color32::from_rgb(0x4d, 0x4d, 0x4d);

fn run_key() -> String {
    obfstr!(r"Software\Microsoft\Windows\CurrentVersion\Run").to_string()
}
fn run_name() -> String {
    obfstr!("YouTubeRPC").to_string()
}

#[derive(Clone)]
struct Config {
    client_id: String,
    activity_type: i64,
    small_icon: Option<String>,
    port: u16,
}

fn load_config() -> Config {
    let mut map = std::collections::HashMap::new();
    for dir in config_dirs() {
        let path = dir.join(obfstr!(".env"));
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    map.entry(k.trim().to_string())
                        .or_insert_with(|| v.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    let get = |key: &str| -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| map.get(key).cloned().filter(|s| !s.is_empty()))
    };

    Config {
        // baked in so the exe runs standalone; .env still overrides
        client_id: get(obfstr!("DISCORD_APP_ID"))
            .unwrap_or_else(|| obfstr!("1542500318561181777").to_string()),
        activity_type: get(obfstr!("ACTIVITY_TYPE")).and_then(|s| s.parse().ok()).unwrap_or(2),
        small_icon: get(obfstr!("SMALL_ICON_URL")),
        port: get(obfstr!("BRIDGE_PORT")).and_then(|s| s.parse().ok()).unwrap_or(41414),
    }
}

fn config_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.to_path_buf());
        }
    }
    dirs
}

// what the extension sends; parsed by hand so field names stay out of the binary
#[derive(Default, Clone)]
struct Incoming {
    clear: bool,
    video_id: String,
    url: String,
    title: String,
    channel: String,
    #[allow(dead_code)]
    album: String,
    live: bool,
    ended: bool,
    current_time: f64,
    duration: f64,
    playing: bool,
    is_music: bool,
    thumbnail: String,
}

impl Incoming {
    fn from_value(v: &Value) -> Self {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let b = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
        let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
        Incoming {
            clear: b(obfstr!("clear")),
            video_id: s(obfstr!("videoId")),
            url: s(obfstr!("url")),
            title: s(obfstr!("title")),
            channel: s(obfstr!("channel")),
            album: s(obfstr!("album")),
            live: b(obfstr!("live")),
            ended: b(obfstr!("ended")),
            current_time: f(obfstr!("currentTime")),
            duration: f(obfstr!("duration")),
            playing: b(obfstr!("playing")),
            is_music: b(obfstr!("isMusic")),
            thumbnail: s(obfstr!("thumbnail")),
        }
    }
}

enum Msg {
    State(Box<Incoming>),
    Clear,
    SetActivityType(i64),
    #[allow(dead_code)]
    SetSmallIcon(Option<String>),
}

// current song state, shared with the UI
#[derive(Clone, Default)]
struct UiSnapshot {
    connected: bool,
    has_song: bool,
    title: String,
    channel: String,
    thumbnail: String,
    playing: bool,
    live: bool,
    current_time: f64,
    duration: f64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

fn pad2(s: &str) -> String {
    if s.chars().count() >= 2 {
        s.to_string()
    } else {
        format!("{}\u{200b}\u{200b}", s).chars().take(2.max(s.chars().count())).collect()
    }
}

fn build_activity(inc: &Incoming, cfg: &Config) -> Value {
    let mut assets = Map::new();
    if !inc.thumbnail.is_empty() {
        assets.insert(obfstr!("large_image").to_string(), Value::from(inc.thumbnail.clone()));
    }
    if let Some(icon) = &cfg.small_icon {
        assets.insert(obfstr!("small_image").to_string(), Value::from(icon.clone()));
    }

    let watch = format!("{}{}", obfstr!("https://www.youtube.com/watch?v="), inc.video_id);
    let listen_url = if inc.url.is_empty() { watch.clone() } else { inc.url.clone() };

    let mut b1 = Map::new();
    b1.insert(obfstr!("label").to_string(), Value::from(obfstr!("Listen Along").to_string()));
    b1.insert(obfstr!("url").to_string(), Value::from(listen_url));
    let mut b2 = Map::new();
    b2.insert(obfstr!("label").to_string(), Value::from(obfstr!("Play on YouTube").to_string()));
    b2.insert(obfstr!("url").to_string(), Value::from(watch));
    let buttons = Value::Array(vec![Value::Object(b1), Value::Object(b2)]);

    let title = if inc.title.is_empty() { obfstr!("YouTube").to_string() } else { inc.title.clone() };
    let default_state = if inc.is_music {
        obfstr!("YouTube Music").to_string()
    } else {
        obfstr!("YouTube").to_string()
    };
    let state_line = if inc.channel.is_empty() { default_state } else { inc.channel.clone() };

    let mut act = Map::new();
    act.insert(obfstr!("type").to_string(), Value::from(cfg.activity_type));
    act.insert(obfstr!("details").to_string(), Value::from(pad2(&truncate(&title, 128))));
    act.insert(obfstr!("state").to_string(), Value::from(pad2(&truncate(&state_line, 128))));
    if !assets.is_empty() {
        act.insert(obfstr!("assets").to_string(), Value::Object(assets));
    }
    act.insert(obfstr!("buttons").to_string(), buttons);

    let now = now_ms();
    let elapsed_ms = (inc.current_time * 1000.0) as u64;
    if inc.live {
        let mut ts = Map::new();
        ts.insert(obfstr!("start").to_string(), Value::from(now.saturating_sub(elapsed_ms)));
        act.insert(obfstr!("timestamps").to_string(), Value::Object(ts));
    } else if inc.playing && inc.duration > 0.0 {
        let start = now.saturating_sub(elapsed_ms);
        let end = start + (inc.duration * 1000.0) as u64;
        let mut ts = Map::new();
        ts.insert(obfstr!("start").to_string(), Value::from(start));
        ts.insert(obfstr!("end").to_string(), Value::from(end));
        act.insert(obfstr!("timestamps").to_string(), Value::Object(ts));
    }

    Value::Object(act)
}

fn run_http(tx: Sender<Msg>, ui: Arc<Mutex<UiSnapshot>>, port: u16, reveal_flag: Arc<AtomicBool>) {
    let addr = format!("127.0.0.1:{port}");
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => s,
        Err(_) => return,
    };

    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();

        // health check for the popup
        if url.as_str() == obfstr!("/ping") {
            let mut resp = tiny_http::Response::from_string(obfstr!("ok").to_string());
            add_cors(&mut resp);
            let _ = request.respond(resp);
            continue;
        }

        // a second launch pings this to reopen the window
        if url.as_str() == obfstr!("/reveal") {
            reveal_flag.store(true, Ordering::SeqCst);
            let mut resp = tiny_http::Response::from_string(obfstr!("ok").to_string());
            add_cors(&mut resp);
            let _ = request.respond(resp);
            continue;
        }

        if method == tiny_http::Method::Options {
            let mut resp = tiny_http::Response::empty(204);
            add_cors(&mut resp);
            resp.add_header(
                tiny_http::Header::from_bytes(
                    obfstr!("Access-Control-Allow-Methods").as_bytes(),
                    obfstr!("POST, OPTIONS").as_bytes(),
                )
                .unwrap(),
            );
            resp.add_header(
                tiny_http::Header::from_bytes(
                    obfstr!("Access-Control-Allow-Headers").as_bytes(),
                    obfstr!("Content-Type").as_bytes(),
                )
                .unwrap(),
            );
            let _ = request.respond(resp);
            continue;
        }

        if method != tiny_http::Method::Post || url.as_str() != obfstr!("/state") {
            let _ = request.respond(tiny_http::Response::empty(404));
            continue;
        }

        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() {
            let _ = request.respond(tiny_http::Response::empty(400));
            continue;
        }

        match serde_json::from_str::<Value>(&body) {
            Ok(v) => {
                let inc = Incoming::from_value(&v);
                update_ui_from_incoming(&ui, &inc);
                let msg = if inc.clear { Msg::Clear } else { Msg::State(Box::new(inc)) };
                let _ = tx.send(msg);
                let mut resp = tiny_http::Response::from_string(obfstr!("ok").to_string());
                add_cors(&mut resp);
                let _ = request.respond(resp);
            }
            Err(_) => {
                let mut resp = tiny_http::Response::from_string(obfstr!("bad json").to_string())
                    .with_status_code(400);
                add_cors(&mut resp);
                let _ = request.respond(resp);
            }
        }
    }
}

fn add_cors<R: std::io::Read>(resp: &mut tiny_http::Response<R>) {
    resp.add_header(
        tiny_http::Header::from_bytes(
            obfstr!("Access-Control-Allow-Origin").as_bytes(),
            obfstr!("*").as_bytes(),
        )
        .unwrap(),
    );
}

fn update_ui_from_incoming(ui: &Arc<Mutex<UiSnapshot>>, inc: &Incoming) {
    if let Ok(mut s) = ui.lock() {
        if inc.clear || inc.ended {
            s.has_song = false;
        } else {
            s.has_song = true;
            s.title = inc.title.clone();
            s.channel = inc.channel.clone();
            s.thumbnail = inc.thumbnail.clone();
            s.playing = inc.playing;
            s.live = inc.live;
            s.current_time = inc.current_time;
            s.duration = inc.duration;
        }
    }
}

fn run_discord(mut cfg: Config, rx: Receiver<Msg>, ui: Arc<Mutex<UiSnapshot>>) {
    let mut discord = Discord::new(cfg.client_id.clone());
    let mut last_incoming: Option<Incoming> = None;
    let mut last_at = Instant::now();

    let set_connected = |ui: &Arc<Mutex<UiSnapshot>>, v: bool| {
        if let Ok(mut s) = ui.lock() {
            s.connected = v;
        }
    };

    loop {
        if !discord.connected() {
            match discord.connect() {
                Ok(()) => {
                    set_connected(&ui, true);
                    if let Some(inc) = &last_incoming {
                        if last_at.elapsed() < Duration::from_secs(STALE_SECS) {
                            let _ = discord.set_activity(build_activity(inc, &cfg));
                        }
                    }
                }
                Err(_) => {
                    set_connected(&ui, false);
                    thread::sleep(Duration::from_secs(3));
                }
            }
        }

        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Msg::State(inc)) => {
                if inc.ended {
                    continue;
                }
                last_at = Instant::now();
                let act = build_activity(&inc, &cfg);
                last_incoming = Some(*inc);
                if discord.connected() && discord.set_activity(act).is_err() {
                    discord.disconnect();
                    set_connected(&ui, false);
                }
            }
            Ok(Msg::Clear) => {
                last_incoming = None;
                if discord.connected() && discord.clear().is_err() {
                    discord.disconnect();
                    set_connected(&ui, false);
                }
            }
            Ok(Msg::SetActivityType(t)) => {
                cfg.activity_type = t;
                if let (Some(inc), true) = (&last_incoming, discord.connected()) {
                    let _ = discord.set_activity(build_activity(inc, &cfg));
                }
            }
            Ok(Msg::SetSmallIcon(icon)) => {
                cfg.small_icon = icon;
                if let (Some(inc), true) = (&last_incoming, discord.connected()) {
                    let _ = discord.set_activity(build_activity(inc, &cfg));
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if last_incoming.is_some() && last_at.elapsed() > Duration::from_secs(STALE_SECS) {
                    last_incoming = None;
                    if let Ok(mut s) = ui.lock() {
                        s.has_song = false;
                    }
                    if discord.connected() && discord.clear().is_err() {
                        discord.disconnect();
                        set_connected(&ui, false);
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn is_autostart_enabled() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(run_key())
        .and_then(|k| k.get_value::<String, _>(run_name()))
        .is_ok()
}

fn set_autostart(enable: bool) {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if enable {
        if let Ok(exe) = std::env::current_exe() {
            if let Ok((key, _)) = hkcu.create_subkey(run_key()) {
                let value = format!("\"{}\" {}", exe.to_string_lossy(), obfstr!("--startup"));
                let _ = key.set_value(run_name(), &value);
            }
        }
    } else if let Ok(key) = hkcu.open_subkey_with_flags(run_key(), winreg::enums::KEY_ALL_ACCESS) {
        let _ = key.delete_value(run_name());
    }
}

fn app_key() -> String {
    obfstr!(r"Software\YouTubeRPC").to_string()
}

fn get_flag(name: &str) -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(app_key())
        .and_then(|k| k.get_value::<u32, _>(name))
        .map(|v| v != 0)
        .unwrap_or(false)
}

fn set_flag(name: &str, val: bool) {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(app_key()) {
        let _ = k.set_value(name, &(val as u32));
    }
}

#[cfg(windows)]
fn message_box(title: &str, text: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let t = wide(text);
    let c = wide(title);
    unsafe {
        let _ = MessageBoxW(HWND::default(), PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OK | MB_ICONINFORMATION);
    }
}

// the app mark: violet rounded square with three white "now playing" bars
fn icon_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let k = s / 32.0;
    let r = 7.0 * k;
    let bottom = 23.0 * k;
    let bars = [
        (7.0 * k, 12.0 * k, 13.0 * k),
        (13.5 * k, 18.5 * k, 8.0 * k),
        (20.0 * k, 25.0 * k, 12.0 * k),
    ];
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let fx = x as f32;
            let fy = y as f32;
            let mut transparent = false;
            let corners = [(r, r), (s - r, r), (r, s - r), (s - r, s - r)];
            for (cxr, cyr) in corners {
                let ox = (fx < r && cxr == r) || (fx > s - r && cxr == s - r);
                let oy = (fy < r && cyr == r) || (fy > s - r && cyr == s - r);
                if ox && oy {
                    let dx = fx + 0.5 - cxr;
                    let dy = fy + 0.5 - cyr;
                    if dx * dx + dy * dy > r * r {
                        transparent = true;
                    }
                }
            }
            if transparent {
                continue;
            }
            let in_bar = bars.iter().any(|&(x0, x1, ytop)| fx >= x0 && fx < x1 && fy >= ytop && fy < bottom);
            if in_bar {
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            } else {
                rgba[idx] = 0x8b;
                rgba[idx + 1] = 0x5c;
                rgba[idx + 2] = 0xf6;
                rgba[idx + 3] = 255;
            }
        }
    }
    rgba
}

fn fetch_image(url: &str) -> Option<egui::ColorImage> {
    // mqdefault is a clean 16:9 frame
    let url = url.replace(obfstr!("hqdefault"), obfstr!("mqdefault"));
    let resp = ureq::get(&url).call().ok()?;
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw()))
}

fn main() {
    let cfg = load_config();

    // one instance only; if it's already up, tell it to show and exit
    if instance_running(cfg.port) {
        return;
    }

    if is_autostart_enabled() {
        set_autostart(true);
    }
    let startup = std::env::args().any(|a| a == obfstr!("--startup"));

    let ui = Arc::new(Mutex::new(UiSnapshot::default()));
    let reveal_flag = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<Msg>();

    {
        let tx = tx.clone();
        let ui = ui.clone();
        let port = cfg.port;
        let rf = reveal_flag.clone();
        thread::spawn(move || run_http(tx, ui, port, rf));
    }
    {
        let worker_cfg = cfg.clone();
        let ui = ui.clone();
        thread::spawn(move || run_discord(worker_cfg, rx, ui));
    }

    run_app(cfg, tx, ui, startup, reveal_flag);
}

fn instance_running(port: u16) -> bool {
    ureq::post(&format!("http://127.0.0.1:{port}/reveal")).call().is_ok()
}

fn run_app(cfg: Config, tx: Sender<Msg>, ui: Arc<Mutex<UiSnapshot>>, startup: bool, reveal_flag: Arc<AtomicBool>) {
    let icon = egui::IconData { rgba: icon_rgba(32), width: 32, height: 32 };
    let mut opts = eframe::NativeOptions::default();
    opts.viewport = egui::ViewportBuilder::default()
        .with_title(obfstr!("YouTube RPC").to_string())
        .with_inner_size([400.0, 660.0])
        .with_min_inner_size([340.0, 460.0])
        .with_visible(!startup)
        .with_icon(icon);

    let _ = eframe::run_native(
        obfstr!("YouTube RPC"),
        opts,
        Box::new(move |cc| {
            setup_style(&cc.egui_ctx);
            let (tray, open_id, quit_id) = build_tray();
            spawn_tray_thread(cc.egui_ctx.clone(), open_id, quit_id, reveal_flag);
            Ok(Box::new(App::new(ui, tx, cfg, tray)))
        }),
    );
}

// eframe stops calling update() while hidden, so the tray runs on its own thread.
// Open forces the window back via Win32 - a queued egui command won't wake it.
fn spawn_tray_thread(
    ctx: egui::Context,
    open_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
    reveal_flag: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let menu_rx = tray_icon::menu::MenuEvent::receiver();
        let tray_rx = tray_icon::TrayIconEvent::receiver();
        loop {
            if let Ok(ev) = menu_rx.recv_timeout(Duration::from_millis(200)) {
                if ev.id == quit_id {
                    std::process::exit(0);
                } else if ev.id == open_id {
                    reveal(&ctx);
                }
            }
            while let Ok(ev) = tray_rx.try_recv() {
                if let tray_icon::TrayIconEvent::DoubleClick { .. } = ev {
                    reveal(&ctx);
                }
            }
            // set by a second launch to raise the window
            if reveal_flag.swap(false, Ordering::SeqCst) {
                reveal(&ctx);
            }
        }
    });
}

fn reveal(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    ctx.request_repaint();
    #[cfg(windows)]
    show_main_window();
}

#[cfg(windows)]
fn show_main_window() {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_SHOW};
    let title: Vec<u16> = obfstr!("YouTube RPC").encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if let Ok(hwnd) = FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

fn setup_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    ctx.set_visuals(visuals);
}

fn build_tray() -> (tray_icon::TrayIcon, tray_icon::menu::MenuId, tray_icon::menu::MenuId) {
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
    let menu = Menu::new();
    let open = MenuItem::new(obfstr!("Open"), true, None);
    let quit = MenuItem::new(obfstr!("Quit"), true, None);
    let _ = menu.append(&open);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&quit);
    let icon = tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32).expect("icon");
    let tray = tray_icon::TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(obfstr!("YouTube RPC"))
        .with_icon(icon)
        .build()
        .expect("tray");
    (tray, open.id().clone(), quit.id().clone())
}

struct App {
    ui: Arc<Mutex<UiSnapshot>>,
    tx: Sender<Msg>,
    port: u16,
    activity_type: i64,
    autostart: bool,
    _tray: tray_icon::TrayIcon,
    thumb_url: String,
    thumb_tex: Option<egui::TextureHandle>,
    thumb_rx: Option<Receiver<Option<egui::ColorImage>>>,
    anchor_key: String,
    anchor_ct: f64,
    anchor_at: Instant,
}

impl App {
    fn new(ui: Arc<Mutex<UiSnapshot>>, tx: Sender<Msg>, cfg: Config, tray: tray_icon::TrayIcon) -> Self {
        App {
            ui,
            tx,
            port: cfg.port,
            activity_type: cfg.activity_type,
            autostart: is_autostart_enabled(),
            _tray: tray,
            thumb_url: String::new(),
            thumb_tex: None,
            thumb_rx: None,
            anchor_key: String::new(),
            anchor_ct: 0.0,
            anchor_at: Instant::now(),
        }
    }

    fn update_thumbnail(&mut self, ctx: &egui::Context, snap: &UiSnapshot) {
        if !snap.has_song {
            self.thumb_tex = None;
            self.thumb_url.clear();
            self.thumb_rx = None;
            return;
        }
        if !snap.thumbnail.is_empty() && snap.thumbnail != self.thumb_url && self.thumb_rx.is_none() {
            self.thumb_url = snap.thumbnail.clone();
            self.thumb_tex = None;
            let (tx, rx) = mpsc::channel();
            let url = snap.thumbnail.clone();
            thread::spawn(move || {
                let _ = tx.send(fetch_image(&url));
            });
            self.thumb_rx = Some(rx);
        }
        if let Some(rx) = &self.thumb_rx {
            if let Ok(res) = rx.try_recv() {
                self.thumb_rx = None;
                if let Some(ci) = res {
                    self.thumb_tex =
                        Some(ctx.load_texture(obfstr!("thumb"), ci, egui::TextureOptions::LINEAR));
                    ctx.request_repaint();
                }
            }
        }
    }

    fn displayed_elapsed(&mut self, snap: &UiSnapshot) -> f64 {
        let key = format!("{}|{}", snap.title, snap.current_time);
        if key != self.anchor_key {
            self.anchor_key = key;
            self.anchor_ct = snap.current_time;
            self.anchor_at = Instant::now();
        }
        if snap.playing && !snap.live {
            (self.anchor_ct + self.anchor_at.elapsed().as_secs_f64()).min(snap.duration)
        } else {
            self.anchor_ct
        }
    }
}

fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        [0.059, 0.059, 0.059, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // X drops to the tray, app keeps running
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            #[cfg(windows)]
            if !get_flag(obfstr!("bgNotice")) {
                set_flag(obfstr!("bgNotice"), true);
                message_box(
                    obfstr!("YouTube RPC"),
                    obfstr!("YouTube RPC is still running in the background and will keep updating your Discord activity.\n\nTo stop it completely, right-click the violet icon in the tray (near the clock) and choose Quit."),
                );
            }
        }

        let snap = self.ui.lock().map(|s| s.clone()).unwrap_or_default();
        self.update_thumbnail(ctx, &snap);
        let elapsed = self.displayed_elapsed(&snap);

        let frame = egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(16.0));
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            draw_header(ui, &snap);
            ui.add_space(14.0);
            draw_now_playing(ui, &snap, elapsed, self.thumb_tex.as_ref());
            ui.add_space(16.0);
            self.draw_settings(ui);
            ui.add_space(10.0);
            draw_footer(ui, self.port);
        });

        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

fn draw_logo(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(7.0), ACCENT);
    let base = rect.min;
    let bar = |x: f32, ytop: f32| {
        egui::Rect::from_min_max(
            egui::pos2(base.x + x, base.y + ytop),
            egui::pos2(base.x + x + 4.0, base.y + 22.0),
        )
    };
    p.rect_filled(bar(6.0, 12.0), Rounding::same(1.5), Color32::WHITE);
    p.rect_filled(bar(13.0, 7.0), Rounding::same(1.5), Color32::WHITE);
    p.rect_filled(bar(20.0, 11.0), Rounding::same(1.5), Color32::WHITE);
}

fn draw_header(ui: &mut egui::Ui, snap: &UiSnapshot) {
    ui.horizontal(|ui| {
        draw_logo(ui);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(obfstr!("Discord Presence")).size(15.0).color(TEXT));
            ui.label(egui::RichText::new(obfstr!("Listening to YouTube")).size(11.0).color(MUTED));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (label, on) = if !snap.connected {
                (obfstr!("Waiting").to_string(), false)
            } else if snap.has_song {
                (
                    if snap.playing { obfstr!("Playing").to_string() } else { obfstr!("Paused").to_string() },
                    true,
                )
            } else {
                (obfstr!("Connected").to_string(), true)
            };
            egui::Frame::none()
                .fill(CARD2)
                .rounding(Rounding::same(20.0))
                .inner_margin(egui::Margin::symmetric(10.0, 5.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(
                            dot.center(),
                            4.0,
                            if on { GREEN } else { Color32::from_rgb(0x66, 0x66, 0x66) },
                        );
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(label).size(11.0).color(MUTED));
                    });
                });
        });
    });
}

fn draw_now_playing(
    ui: &mut egui::Ui,
    snap: &UiSnapshot,
    elapsed: f64,
    thumb: Option<&egui::TextureHandle>,
) {
    egui::Frame::none()
        .fill(CARD)
        .rounding(Rounding::same(12.0))
        .show(ui, |ui| {
            let w = ui.available_width();
            if !snap.has_song {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 132.0), Sense::hover());
                let p = ui.painter();
                p.text(
                    rect.center() - egui::vec2(0.0, 9.0),
                    egui::Align2::CENTER_CENTER,
                    obfstr!("Nothing playing"),
                    egui::FontId::proportional(15.0),
                    TEXT,
                );
                p.text(
                    rect.center() + egui::vec2(0.0, 13.0),
                    egui::Align2::CENTER_CENTER,
                    obfstr!("Play something on YouTube"),
                    egui::FontId::proportional(12.0),
                    MUTED,
                );
                return;
            }

            // thumbnail (16:9)
            let th = w * 9.0 / 16.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, th), Sense::hover());
            let round_top = Rounding { nw: 12.0, ne: 12.0, sw: 0.0, se: 0.0 };
            if let Some(tex) = thumb {
                ui.painter().image(
                    tex.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                ui.painter().rect_filled(rect, round_top, Color32::BLACK);
            }
            if !snap.live {
                let txt = fmt_time(snap.duration);
                let pos = rect.right_bottom() + egui::vec2(-8.0, -8.0);
                let galley = ui.painter().layout_no_wrap(
                    txt,
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
                let bg = egui::Rect::from_min_size(
                    pos - egui::vec2(galley.size().x + 8.0, galley.size().y + 4.0),
                    galley.size() + egui::vec2(8.0, 4.0),
                );
                ui.painter().rect_filled(bg, Rounding::same(4.0), Color32::from_black_alpha(200));
                ui.painter().galley(bg.min + egui::vec2(4.0, 2.0), galley, Color32::WHITE);
            }

            // text block with padding
            egui::Frame::none()
                .inner_margin(egui::Margin { left: 14.0, right: 14.0, top: 12.0, bottom: 14.0 })
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(&snap.title).size(15.0).color(TEXT).strong());
                    ui.add_space(3.0);
                    ui.label(egui::RichText::new(&snap.channel).size(13.0).color(MUTED));
                    ui.add_space(12.0);

                    if !snap.live {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(fmt_time(elapsed)).size(11.0).color(MUTED));
                            ui.add_space(6.0);
                            let bar_w = ui.available_width() - 42.0;
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w.max(10.0), 4.0), Sense::hover());
                            ui.painter().rect_filled(rect, Rounding::same(2.0), TRACK);
                            let frac = if snap.duration > 0.0 { (elapsed / snap.duration).clamp(0.0, 1.0) } else { 0.0 };
                            let mut fill = rect;
                            fill.set_width(rect.width() * frac as f32);
                            ui.painter().rect_filled(fill, Rounding::same(2.0), ACCENT);
                            ui.add_space(6.0);
                            ui.label(egui::RichText::new(fmt_time(snap.duration)).size(11.0).color(MUTED));
                        });
                    } else {
                        ui.label(egui::RichText::new(obfstr!("Live")).size(12.0).color(ACCENT).strong());
                    }
                });
        });
}

impl App {
    fn draw_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(obfstr!("SETTINGS")).size(11.0).color(MUTED));
        ui.add_space(8.0);

        // Show as
        card_row(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(obfstr!("Show as")).size(13.0).color(TEXT));
                ui.label(egui::RichText::new(obfstr!("How it appears on your profile")).size(11.0).color(MUTED));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::Frame::none()
                    .fill(CARD2)
                    .rounding(Rounding::same(8.0))
                    .inner_margin(egui::Margin::same(3.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (val, label) in
                                [(0i64, obfstr!("Playing")), (3, obfstr!("Watching")), (2, obfstr!("Listening"))]
                            {
                                if seg_button(ui, label, self.activity_type == val) {
                                    self.activity_type = val;
                                    let _ = self.tx.send(Msg::SetActivityType(val));
                                }
                            }
                        });
                    });
            });
        });

        ui.add_space(8.0);

        // Autostart
        card_row(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(obfstr!("Start with Windows")).size(13.0).color(TEXT));
                ui.label(egui::RichText::new(obfstr!("Launch automatically at login")).size(11.0).color(MUTED));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if toggle(ui, self.autostart).clicked() {
                    self.autostart = !self.autostart;
                    set_autostart(self.autostart);
                }
            });
        });
    }
}

fn card_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(CARD)
        .rounding(Rounding::same(10.0))
        .inner_margin(egui::Margin::symmetric(12.0, 11.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_width(ui.available_width());
                add(ui);
            });
        });
}

fn seg_button(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    let text = egui::RichText::new(label)
        .size(12.0)
        .color(if active { Color32::WHITE } else { MUTED });
    let btn = egui::Button::new(text)
        .fill(if active { ACCENT } else { Color32::TRANSPARENT })
        .rounding(Rounding::same(6.0))
        .stroke(Stroke::NONE);
    ui.add(btn).clicked()
}

fn toggle(ui: &mut egui::Ui, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 22.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(11.0), if on { ACCENT } else { TRACK });
    let knob_x = if on { rect.right() - 11.0 } else { rect.left() + 11.0 };
    p.circle_filled(egui::pos2(knob_x, rect.center().y), 8.0, Color32::WHITE);
    resp
}

fn draw_footer(ui: &mut egui::Ui, port: u16) {
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(format!("{}{}", obfstr!("Bridge on 127.0.0.1:"), port))
                .size(11.0)
                .color(MUTED),
        );
    });
}
