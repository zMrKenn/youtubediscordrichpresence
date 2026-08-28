#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod discord;
mod ui;

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use obfstr::obfstr;
use serde_json::{Map, Value};

use discord::Discord;

const STALE_SECS: u64 = 30;

fn run_key() -> String {
    obfstr!(r"Software\Microsoft\Windows\CurrentVersion\Run").to_string()
}
fn run_name() -> String {
    obfstr!("YouTubeRPC").to_string()
}

#[derive(Clone)]
pub struct Config {
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

pub enum Msg {
    State(Box<Incoming>),
    Clear,
    SetActivityType(i64),
    #[allow(dead_code)]
    SetSmallIcon(Option<String>),
}

#[derive(Clone, Default)]
pub struct UiSnapshot {
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

fn run_http(
    tx: Sender<Msg>,
    ui: Arc<Mutex<UiSnapshot>>,
    port: u16,
    reveal_flag: Arc<AtomicBool>,
    notify_hwnd: Arc<AtomicIsize>,
) {
    let addr = format!("127.0.0.1:{port}");
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => s,
        Err(_) => return,
    };

    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();

        if url.as_str() == obfstr!("/ping") {
            let mut resp = tiny_http::Response::from_string(obfstr!("ok").to_string());
            add_cors(&mut resp);
            let _ = request.respond(resp);
            continue;
        }

        if url.as_str() == obfstr!("/reveal") {
            reveal_flag.store(true, Ordering::SeqCst);
            ui::notify_reveal(&notify_hwnd);
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
                ui::notify_refresh(&notify_hwnd);
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

pub fn is_autostart_enabled() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(run_key())
        .and_then(|k| k.get_value::<String, _>(run_name()))
        .is_ok()
}

pub fn set_autostart(enable: bool) {
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

pub fn get_flag(name: &str) -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(app_key())
        .and_then(|k| k.get_value::<u32, _>(name))
        .map(|v| v != 0)
        .unwrap_or(false)
}

pub fn set_flag(name: &str, val: bool) {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(app_key()) {
        let _ = k.set_value(name, &(val as u32));
    }
}

pub fn message_box(title: &str, text: &str) {
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

pub fn icon_rgba(size: u32) -> Vec<u8> {
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

pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn main() {
    let cfg = load_config();

    if instance_running(cfg.port) {
        return;
    }

    if is_autostart_enabled() {
        set_autostart(true);
    }
    let startup = std::env::args().any(|a| a == obfstr!("--startup"));

    let ui = Arc::new(Mutex::new(UiSnapshot::default()));
    let reveal_flag = Arc::new(AtomicBool::new(false));
    let notify_hwnd = Arc::new(AtomicIsize::new(0));
    let (tx, rx) = mpsc::channel::<Msg>();

    {
        let tx = tx.clone();
        let ui = ui.clone();
        let port = cfg.port;
        let rf = reveal_flag.clone();
        let nh = notify_hwnd.clone();
        thread::spawn(move || run_http(tx, ui, port, rf, nh));
    }
    {
        let worker_cfg = cfg.clone();
        let ui = ui.clone();
        thread::spawn(move || run_discord(worker_cfg, rx, ui));
    }

    ui::run(cfg, tx, ui, startup, reveal_flag, notify_hwnd);
}

fn instance_running(port: u16) -> bool {
    ureq::post(&format!("http://127.0.0.1:{port}/reveal")).call().is_ok()
}
