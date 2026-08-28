use std::io::Read;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use obfstr::obfstr;
use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    AlphaBlend, BeginPaint, BitBlt, CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject, EndPaint, FillRect,
    GetDC, GetTextExtentPoint32W, InvalidateRect, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    StretchBlt, TextOutW, AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER,
    BLENDFUNCTION, DEFAULT_CHARSET, DIB_RGB_COLORS, FW_BOLD, FW_NORMAL, HBITMAP, HBRUSH, HDC,
    HFONT, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetMessageW, GetWindowLongPtrW, IsWindowVisible, KillTimer, LoadCursorW,
    LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW, SetCursor, SetForegroundWindow,
    SetTimer, SetWindowLongPtrW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    GWLP_USERDATA, HICON, ICONINFO, IDC_ARROW, IDC_HAND, MINMAXINFO, MSG, SW_HIDE, SW_SHOW,
    WM_CLOSE, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_LBUTTONDOWN,
    WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_TIMER, WM_USER, WNDCLASSW, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
};

const WM_MOUSELEAVE: u32 = 0x02A3;

use crate::{
    get_flag, icon_rgba, message_box, set_autostart, set_flag, Config, Msg, UiSnapshot,
};

const WM_APP_REVEAL: u32 = WM_USER + 1;
const WM_APP_REFRESH: u32 = WM_USER + 2;
const TIMER_UI: usize = 1;

const WIN_W: i32 = 400;
const WIN_H: i32 = 660;
const PAD: i32 = 16;

const C_BG: COLORREF = rgb(0x0f, 0x0f, 0x0f);
const C_CARD: COLORREF = rgb(0x18, 0x18, 0x18);
const C_CARD2: COLORREF = rgb(0x21, 0x21, 0x21);
const C_ACCENT: COLORREF = rgb(0x8b, 0x5c, 0xf6);
const C_TEXT: COLORREF = rgb(0xf1, 0xf1, 0xf1);
const C_MUTED: COLORREF = rgb(0xaa, 0xaa, 0xaa);
const C_GREEN: COLORREF = rgb(0x2b, 0xa6, 0x40);
const C_TRACK: COLORREF = rgb(0x4d, 0x4d, 0x4d);

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// Build an HICON from raw RGBA bytes so the window title bar and taskbar show
// the app's own violet mark instead of Windows' default icon.
unsafe fn make_hicon(rgba: &[u8], size: i32) -> Option<HICON> {
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let screen = GetDC(None);
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let color_bmp = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    let _ = ReleaseDC(None, screen);
    if bits.is_null() {
        let _ = DeleteObject(color_bmp);
        return None;
    }
    let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
    for i in 0..(size * size) as usize {
        dst[i * 4] = rgba[i * 4 + 2];       // B
        dst[i * 4 + 1] = rgba[i * 4 + 1];   // G
        dst[i * 4 + 2] = rgba[i * 4];       // R
        dst[i * 4 + 3] = rgba[i * 4 + 3];   // A
    }
    let mask_bmp = CreateBitmap(size, size, 1, 1, None);
    if mask_bmp.is_invalid() {
        let _ = DeleteObject(color_bmp);
        return None;
    }
    let info = ICONINFO {
        fIcon: windows::Win32::Foundation::BOOL(1),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask_bmp,
        hbmColor: color_bmp,
    };
    let hicon = CreateIconIndirect(&info).ok();
    let _ = DeleteObject(color_bmp);
    let _ = DeleteObject(mask_bmp);
    hicon
}

struct Hit {
    toggle: RECT,
    segs: [(RECT, i64); 3],
}

#[derive(Copy, Clone, PartialEq)]
enum HoverKind {
    None,
    Toggle,
    Seg(usize),
}

struct WindowState {
    tx: Sender<Msg>,
    snap: Arc<Mutex<UiSnapshot>>,
    port: u16,
    activity_type: i64,
    autostart: bool,
    thumb_url: String,
    thumb_bmp: Option<HBITMAP>,
    thumb_rx: Option<std::sync::mpsc::Receiver<(Vec<u8>, u32, u32)>>,
    anchor_key: String,
    anchor_ct: f64,
    anchor_at: Instant,
    hit: Hit,
    toggle_anim: f32,
    seg_anim: f32,
    thumb_w: i32,
    thumb_h_src: i32,
    thumb_fade: f32,
    hover: HoverKind,
    tracking_mouse: bool,
}

pub fn run(
    cfg: Config,
    tx: Sender<Msg>,
    snap: Arc<Mutex<UiSnapshot>>,
    startup: bool,
    reveal_flag: Arc<AtomicBool>,
    notify_hwnd: Arc<AtomicIsize>,
) {
    let class = wide(obfstr!("YouTubeRPCWindow"));
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap();
        let app_icon = make_hicon(&icon_rgba(32), 32).unwrap_or_else(|| {
            LoadIconW(None, PCWSTR::null()).unwrap_or_default()
        });
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            hIcon: app_icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let title = wide(obfstr!("YouTube RPC"));
        let hwnd = CreateWindowExW(
            Default::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            WIN_W,
            WIN_H,
            None,
            None,
            hinst,
            None,
        )
        .expect("window");

        // dark title bar to match the app
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const core::ffi::c_void,
            size_of::<i32>() as u32,
        );

        let autostart = crate::is_autostart_enabled();
        let seg_target = seg_index_for(cfg.activity_type) as f32;
        let state = Box::new(WindowState {
            tx: tx.clone(),
            snap: snap.clone(),
            port: cfg.port,
            activity_type: cfg.activity_type,
            autostart,
            thumb_url: String::new(),
            thumb_bmp: None,
            thumb_rx: None,
            anchor_key: String::new(),
            anchor_ct: 0.0,
            anchor_at: Instant::now(),
            hit: Hit {
                toggle: RECT::default(),
                segs: [(RECT::default(), 0); 3],
            },
            toggle_anim: if autostart { 1.0 } else { 0.0 },
            seg_anim: seg_target,
            thumb_w: 0,
            thumb_h_src: 0,
            thumb_fade: 1.0,
            hover: HoverKind::None,
            tracking_mouse: false,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

        notify_hwnd.store(hwnd.0 as isize, Ordering::SeqCst);

        let (tray, open_id, quit_id) = build_tray();
        let _tray = tray;
        spawn_tray_thread(hwnd, open_id, quit_id, reveal_flag);

        if startup {
            let _ = ShowWindow(hwnd, SW_HIDE);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn hwnd_from_raw(raw: isize) -> Option<HWND> {
    if raw == 0 {
        None
    } else {
        Some(HWND(raw as *mut _))
    }
}

pub fn notify_reveal(hwnd_slot: &Arc<AtomicIsize>) {
    if let Some(hwnd) = hwnd_from_raw(hwnd_slot.load(Ordering::SeqCst)) {
        unsafe {
            let _ = PostMessageW(hwnd, WM_APP_REVEAL, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn notify_refresh(hwnd_slot: &Arc<AtomicIsize>) {
    if let Some(hwnd) = hwnd_from_raw(hwnd_slot.load(Ordering::SeqCst)) {
        unsafe {
            let _ = PostMessageW(hwnd, WM_APP_REFRESH, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => LRESULT(0),
        WM_GETMINMAXINFO => {
            if let Some(info) = (lparam.0 as *mut MINMAXINFO).as_mut() {
                info.ptMinTrackSize.x = 340;
                info.ptMinTrackSize.y = 460;
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_UI {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_APP_REVEAL => {
            restore_from_tray(hwnd);
            let _ = InvalidateRect(hwnd, None, true);
            update_timer(hwnd);
            LRESULT(0)
        }
        WM_APP_REFRESH => {
            if IsWindowVisible(hwnd).as_bool() {
                let st = state_mut(hwnd);
                st.refresh_thumb();
                let _ = InvalidateRect(hwnd, None, false);
                update_timer(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            let pt = POINT { x, y };
            let st = state_mut(hwnd);
            let mut nk = HoverKind::None;
            if pt_in_rect(&st.hit.toggle, pt) {
                nk = HoverKind::Toggle;
            } else {
                for (i, (rect, _)) in st.hit.segs.iter().enumerate() {
                    if pt_in_rect(rect, pt) {
                        nk = HoverKind::Seg(i);
                        break;
                    }
                }
            }
            if nk != st.hover {
                st.hover = nk;
                let _ = InvalidateRect(hwnd, None, false);
            }
            if !st.tracking_mouse {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                st.tracking_mouse = true;
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let st = state_mut(hwnd);
            st.tracking_mouse = false;
            if st.hover != HoverKind::None {
                st.hover = HoverKind::None;
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let st = state_mut(hwnd);
            if st.hover != HoverKind::None && (wparam.0 as isize) == hwnd.0 as isize {
                let c = LoadCursorW(None, IDC_HAND).unwrap_or_default();
                let _ = SetCursor(c);
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            let pt = windows::Win32::Foundation::POINT { x, y };
            let st = state_mut(hwnd);
            if pt_in_rect(&st.hit.toggle, pt) {
                st.autostart = !st.autostart;
                set_autostart(st.autostart);
                let _ = InvalidateRect(hwnd, None, false);
            } else {
                for (rect, val) in st.hit.segs {
                    if pt_in_rect(&rect, pt) && st.activity_type != val {
                        st.activity_type = val;
                        let _ = st.tx.send(Msg::SetActivityType(val));
                        let _ = InvalidateRect(hwnd, None, false);
                        break;
                    }
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            hide_to_tray(hwnd);
            if !get_flag(obfstr!("bgNotice")) {
                set_flag(obfstr!("bgNotice"), true);
                message_box(
                    obfstr!("YouTube RPC"),
                    obfstr!("YouTube RPC is still running in the background and will keep updating your Discord activity.\n\nTo stop it completely, right-click the violet icon in the tray (near the clock) and choose Quit."),
                );
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let st = state_mut(hwnd);
            if let Some(bmp) = st.thumb_bmp.take() {
                let _ = DeleteObject(bmp);
            }
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;
            if !raw.is_null() {
                drop(Box::from_raw(raw));
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn state_mut<'a>(hwnd: HWND) -> &'a mut WindowState {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;
    &mut *raw
}

fn pt_in_rect(r: &RECT, pt: windows::Win32::Foundation::POINT) -> bool {
    pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
}

// LTR order: Listening (val 2), Watching (val 3), Playing (val 0)
fn seg_index_for(activity_type: i64) -> usize {
    match activity_type {
        2 => 0,
        3 => 1,
        _ => 2,
    }
}

const SEG_VALS: [i64; 3] = [2, 3, 0];

// Move `cur` toward `target` by a fraction. Returns new value + true if animating.
fn ease(cur: f32, target: f32, k: f32) -> (f32, bool) {
    let diff = target - cur;
    if diff.abs() < 0.003 {
        (target, false)
    } else {
        (cur + diff * k, true)
    }
}

fn lerp_color(a: COLORREF, b: COLORREF, t: f32) -> COLORREF {
    let t = t.clamp(0.0, 1.0);
    let ar = (a.0 & 0xff) as f32;
    let ag = ((a.0 >> 8) & 0xff) as f32;
    let ab = ((a.0 >> 16) & 0xff) as f32;
    let br = (b.0 & 0xff) as f32;
    let bg = ((b.0 >> 8) & 0xff) as f32;
    let bb = ((b.0 >> 16) & 0xff) as f32;
    let r = (ar + (br - ar) * t) as u32 & 0xff;
    let g = (ag + (bg - ag) * t) as u32 & 0xff;
    let bl = (ab + (bb - ab) * t) as u32 & 0xff;
    COLORREF(r | (g << 8) | (bl << 16))
}

unsafe fn hide_to_tray(hwnd: HWND) {
    let _ = KillTimer(hwnd, TIMER_UI);
    let style = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
        hwnd,
        windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE,
    );
    let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
        hwnd,
        windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE,
        (style & !WS_EX_APPWINDOW.0 as isize) | WS_EX_TOOLWINDOW.0 as isize,
    );
    let _ = ShowWindow(hwnd, SW_HIDE);
}

unsafe fn restore_from_tray(hwnd: HWND) {
    let style = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
        hwnd,
        windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE,
    );
    let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
        hwnd,
        windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE,
        (style & !WS_EX_TOOLWINDOW.0 as isize) | WS_EX_APPWINDOW.0 as isize,
    );
    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = SetForegroundWindow(hwnd);
}

unsafe fn update_timer(hwnd: HWND) {
    update_timer_ex(hwnd, false);
}

unsafe fn update_timer_ex(hwnd: HWND, animating: bool) {
    let _ = KillTimer(hwnd, TIMER_UI);
    if !IsWindowVisible(hwnd).as_bool() {
        return;
    }
    if animating {
        let _ = SetTimer(hwnd, TIMER_UI, 16, None);
        return;
    }
    let snap = state_mut(hwnd).snap.lock().map(|s| s.clone()).unwrap_or_default();
    if snap.has_song && snap.playing && !snap.live {
        let _ = SetTimer(hwnd, TIMER_UI, 500, None);
    }
}

impl WindowState {
    fn refresh_thumb(&mut self) {
        let snap = self.snap.lock().map(|s| s.clone()).unwrap_or_default();
        if !snap.has_song {
            self.thumb_url.clear();
            if let Some(bmp) = self.thumb_bmp.take() {
                unsafe {
                    let _ = DeleteObject(bmp);
                }
            }
            self.thumb_rx = None;
            return;
        }
        if snap.thumbnail.is_empty() || snap.thumbnail == self.thumb_url || self.thumb_rx.is_some() {
            return;
        }
        self.thumb_url = snap.thumbnail.clone();
        if let Some(bmp) = self.thumb_bmp.take() {
            unsafe {
                let _ = DeleteObject(bmp);
            }
        }
        let (tx, rx) = std::sync::mpsc::channel::<(Vec<u8>, u32, u32)>();
        let url = snap.thumbnail.clone();
        thread::spawn(move || {
            if let Some(data) = fetch_thumb_bytes(&url) {
                let _ = tx.send(data);
            }
        });
        self.thumb_rx = Some(rx);
    }

    fn poll_thumb(&mut self, hwnd: HWND) {
        if let Some(rx) = &self.thumb_rx {
            if let Ok((rgba, w, h)) = rx.try_recv() {
                self.thumb_rx = None;
                if let Some(bmp) = unsafe { rgba_bytes_to_hbitmap(&rgba, w as i32, h as i32) } {
                    if let Some(old) = self.thumb_bmp.replace(bmp) {
                        unsafe {
                            let _ = DeleteObject(old);
                        }
                    }
                    self.thumb_w = w as i32;
                    self.thumb_h_src = h as i32;
                    self.thumb_fade = 0.0;
                    unsafe {
                        let _ = InvalidateRect(hwnd, None, false);
                    }
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

fn fetch_thumb_bytes(url: &str) -> Option<(Vec<u8>, u32, u32)> {
    let url = url.replace(obfstr!("hqdefault"), obfstr!("mqdefault"));
    let resp = ureq::get(&url).call().ok()?;
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let w = img.width();
    let h = img.height();
    Some((img.into_raw(), w, h))
}

unsafe fn rgba_bytes_to_hbitmap(rgba: &[u8], w: i32, h: i32) -> Option<HBITMAP> {
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let screen = GetDC(None);
    let hbmp = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    let _ = ReleaseDC(None, screen);
    if bits.is_null() {
        let _ = DeleteObject(hbmp);
        return None;
    }
    let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
    for (i, chunk) in rgba.chunks_exact(4).enumerate() {
        let o = i * 4;
        dst[o] = chunk[2];
        dst[o + 1] = chunk[1];
        dst[o + 2] = chunk[0];
        dst[o + 3] = chunk[3];
    }
    Some(hbmp)
}

fn build_tray() -> (tray_icon::TrayIcon, tray_icon::menu::MenuId, tray_icon::menu::MenuId) {
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

fn spawn_tray_thread(
    hwnd: HWND,
    open_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
    reveal_flag: Arc<AtomicBool>,
) {
    let hwnd_raw = hwnd.0 as isize;
    thread::spawn(move || {
        let hwnd = HWND(hwnd_raw as *mut _);
        let menu_rx = tray_icon::menu::MenuEvent::receiver();
        let tray_rx = tray_icon::TrayIconEvent::receiver();
        loop {
            if let Ok(ev) = menu_rx.recv_timeout(Duration::from_millis(500)) {
                if ev.id == quit_id {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    std::process::exit(0);
                } else if ev.id == open_id {
                    unsafe {
                        let _ = PostMessageW(hwnd, WM_APP_REVEAL, WPARAM(0), LPARAM(0));
                    }
                }
            }
            while let Ok(ev) = tray_rx.try_recv() {
                if let tray_icon::TrayIconEvent::DoubleClick { .. } = ev {
                    unsafe {
                        let _ = PostMessageW(hwnd, WM_APP_REVEAL, WPARAM(0), LPARAM(0));
                    }
                }
            }
            if reveal_flag.swap(false, Ordering::SeqCst) {
                unsafe {
                    let _ = PostMessageW(hwnd, WM_APP_REVEAL, WPARAM(0), LPARAM(0));
                }
            }
        }
    });
}

// Paint handler wired through WM_PAINT in wnd_proc - add WM_PAINT case
unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;

    let mem = CreateCompatibleDC(hdc);
    let bmp = CreateCompatibleBitmap(hdc, w, h);
    let old = SelectObject(mem, bmp);

    fill(mem, &rc, C_BG);

    let st = state_mut(hwnd);
    st.poll_thumb(hwnd);
    let snap = st.snap.lock().map(|s| s.clone()).unwrap_or_default();
    let elapsed = st.displayed_elapsed(&snap);
    st.refresh_thumb();

    // advance animations toward their targets
    let toggle_target = if st.autostart { 1.0 } else { 0.0 };
    let seg_target = seg_index_for(st.activity_type) as f32;
    let (new_toggle, tog_active) = ease(st.toggle_anim, toggle_target, 0.28);
    let (new_seg, seg_active) = ease(st.seg_anim, seg_target, 0.28);
    let (new_fade, fade_active) = ease(st.thumb_fade, 1.0, 0.22);
    st.toggle_anim = new_toggle;
    st.seg_anim = new_seg;
    st.thumb_fade = new_fade;
    let animating = tog_active || seg_active || fade_active;

    draw_ui(mem, &rc, st, &snap, elapsed);

    let _ = BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
    let _ = SelectObject(mem, old);
    let _ = DeleteObject(bmp);
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);

    update_timer_ex(hwnd, animating);
}

// Segmented control with a sliding pill behind the active button, drawn
// right-aligned in `row`. Uses seg_anim (0..2) for the pill position so a
// switch animates smoothly between segments. Returns the container's left
// edge so the caller knows where its own text has to stop.
unsafe fn draw_segmented(hdc: HDC, st: &mut WindowState, font: HFONT, row: &RECT) -> i32 {
    let labels = [
        obfstr!("Listening").to_string(),
        obfstr!("Watching").to_string(),
        obfstr!("Playing").to_string(),
    ];
    let seg_w: i32 = 66;
    let seg_h: i32 = 24;
    let pad: i32 = 3;
    let container_w = seg_w * 3 + pad * 2;
    let container_h = seg_h + pad * 2;
    let cright = row.right - 12;
    let ctop = row.top + (row.bottom - row.top - container_h) / 2;
    let container = RECT {
        left: cright - container_w,
        top: ctop,
        right: cright,
        bottom: ctop + container_h,
    };
    fill_round(hdc, &container, 8, C_CARD2);

    // sliding pill at seg_anim (0..2)
    let pill_x = container.left + pad + (seg_w as f32 * st.seg_anim) as i32;
    let pill = RECT {
        left: pill_x,
        top: container.top + pad,
        right: pill_x + seg_w,
        bottom: container.bottom - pad,
    };
    fill_round(hdc, &pill, 6, C_ACCENT);

    for i in 0..3 {
        let sx = container.left + pad + (i as i32) * seg_w;
        let seg_rc = RECT {
            left: sx,
            top: container.top + pad,
            right: sx + seg_w,
            bottom: container.bottom - pad,
        };
        st.hit.segs[i] = (seg_rc, SEG_VALS[i]);
        // Interpolate text color from muted -> white based on how close the pill is
        let closeness = 1.0 - ((st.seg_anim - i as f32).abs()).min(1.0);
        let color = lerp_color(C_MUTED, C_TEXT, closeness);
        draw_text_center(hdc, font, color, &seg_rc, 0, &labels[i]);
    }
    container.left
}

unsafe fn fill(hdc: HDC, rc: &RECT, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    let _ = FillRect(hdc, rc, HBRUSH(brush.0));
    let _ = DeleteObject(brush);
}

unsafe fn draw_ui(hdc: HDC, client: &RECT, st: &mut WindowState, snap: &UiSnapshot, elapsed: f64) {
    let font_title = make_font(-15, FW_BOLD.0 as i32);
    let font_body = make_font(-13, FW_NORMAL.0 as i32);
    let font_small = make_font(-11, FW_NORMAL.0 as i32);
    let font_h1 = make_font(-15, FW_NORMAL.0 as i32);

    let old_font = SelectObject(hdc, font_title);
    let _ = SetBkMode(hdc, TRANSPARENT);

    let mut y = PAD;

    // header - logo is 30px tall; center the two text lines vertically inside it
    draw_logo(hdc, PAD, y);
    let header_left = PAD + 38;
    let title_rc = RECT {
        left: header_left,
        top: y,
        right: client.right - PAD - 100,
        bottom: y + 15,
    };
    let sub_rc = RECT {
        left: header_left,
        top: y + 15,
        right: client.right - PAD - 100,
        bottom: y + 30,
    };
    draw_text_center_left(hdc, font_h1, C_TEXT, &title_rc, obfstr!("Discord Presence"));
    draw_text_center_left(hdc, font_small, C_MUTED, &sub_rc, obfstr!("Listening to YouTube"));

    let pill_w = 92;
    let pill = RECT {
        left: client.right - PAD - pill_w,
        top: y + 4,
        right: client.right - PAD,
        bottom: y + 28,
    };
    fill_round(hdc, &pill, 14, C_CARD2);
    let dot_color = if !snap.connected {
        rgb(0x66, 0x66, 0x66)
    } else {
        C_GREEN
    };
    let label = if !snap.connected {
        obfstr!("Waiting").to_string()
    } else if snap.has_song {
        if snap.playing {
            obfstr!("Playing").to_string()
        } else {
            obfstr!("Paused").to_string()
        }
    } else {
        obfstr!("Connected").to_string()
    };
    // center the dot+gap+text group horizontally inside the pill
    let dot_diameter = 8i32;
    let gap = 6i32;
    let text_w = {
        let old = SelectObject(hdc, font_small);
        let w: Vec<u16> = label.encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &w, &mut sz);
        let _ = SelectObject(hdc, old);
        sz.cx
    };
    let content_w = dot_diameter + gap + text_w;
    let content_left = pill.left + (pill.right - pill.left - content_w) / 2;
    let cy = (pill.top + pill.bottom) / 2;
    draw_dot(hdc, content_left + dot_diameter / 2, cy, dot_color);
    let text_rc = RECT {
        left: content_left + dot_diameter + gap,
        top: pill.top,
        right: pill.right,
        bottom: pill.bottom,
    };
    draw_text_center_left(hdc, font_small, C_MUTED, &text_rc, &label);

    y += 46;

    // now playing card - height is computed from actual content
    let card_left = PAD;
    let card_right = client.right - PAD;
    let inner_w = card_right - card_left;
    let text_pad = 14i32;
    let title_lines = if snap.has_song {
        wrap_text(hdc, font_title, &snap.title, inner_w - text_pad * 2, 2)
    } else {
        Vec::new()
    };
    let title_line_h = 21;
    let title_block_h = title_lines.len().max(1) as i32 * title_line_h;
    let thumb_h_calc = inner_w * 9 / 16;
    let card_h = if snap.has_song {
        thumb_h_calc + 12 + title_block_h + 3 + 18 + 12 + 6 + 14
    } else {
        132
    };

    let card = RECT {
        left: card_left,
        top: y,
        right: card_right,
        bottom: y + card_h,
    };
    fill_round(hdc, &card, 12, C_CARD);
    if !snap.has_song {
        draw_text_center(hdc, font_h1, C_TEXT, &card, -10, obfstr!("Nothing playing"));
        draw_text_center(hdc, font_body, C_MUTED, &card, 14, obfstr!("Play something on YouTube"));
    } else {
        let thumb = RECT {
            left: card.left,
            top: card.top,
            right: card.right,
            bottom: card.top + thumb_h_calc,
        };
        fill(hdc, &thumb, rgb(0, 0, 0));
        if let Some(bmp) = st.thumb_bmp {
            blit_thumb(hdc, bmp, &thumb, st.thumb_w, st.thumb_h_src, st.thumb_fade);
        }
        // round the top corners so the thumbnail matches the card's shape
        punch_corners(hdc, &thumb, 12, C_BG, [true, true, false, false]);

        let mut ty = thumb.bottom + 12;
        for line in &title_lines {
            draw_text(hdc, font_title, C_TEXT, card.left + text_pad, ty, line);
            ty += title_line_h;
        }
        ty += 3;
        draw_text(hdc, font_body, C_MUTED, card.left + text_pad, ty, &truncate(&snap.channel, 60));
        ty += 26;
        if snap.live {
            draw_text(hdc, font_body, C_ACCENT, card.left + text_pad, ty, obfstr!("Live"));
        } else {
            let bar_left = card.left + text_pad;
            let bar_right = card.right - text_pad;
            let bar_y = ty + 6;
            let bar = RECT {
                left: bar_left + 36,
                top: bar_y,
                right: bar_right - 36,
                bottom: bar_y + 4,
            };
            fill_round(hdc, &bar, 2, C_TRACK);
            let frac = if snap.duration > 0.0 {
                (elapsed / snap.duration).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if frac > 0.0 {
                let mut fill_r = bar;
                fill_r.right = bar.left + ((bar.right - bar.left) as f64 * frac) as i32;
                fill_round(hdc, &fill_r, 2, C_ACCENT);
            }
            // time labels sit exactly 4px from each end of the bar, both vertically centered
            let time_rc_l = RECT {
                left: bar_left,
                top: bar_y - 10,
                right: bar.left - 4,
                bottom: bar_y + 14,
            };
            let time_rc_r = RECT {
                left: bar.right + 4,
                top: bar_y - 10,
                right: bar_right,
                bottom: bar_y + 14,
            };
            draw_text_center_right(hdc, font_small, C_MUTED, &time_rc_l, &crate::fmt_time(elapsed));
            draw_text_center_left(hdc, font_small, C_MUTED, &time_rc_r, &crate::fmt_time(snap.duration));
        }
    }

    y = card.bottom + 16;

    draw_text(hdc, font_small, C_MUTED, PAD, y, obfstr!("SETTINGS"));
    y += 22;

    // show as row
    let row1 = RECT {
        left: PAD,
        top: y,
        right: client.right - PAD,
        bottom: y + 58,
    };
    fill_round(hdc, &row1, 10, C_CARD);
    let seg_left = draw_segmented(hdc, st, font_small, &row1);
    let text_max_1 = (seg_left - (row1.left + 12) - 10).max(0);
    draw_text_ellipsis(
        hdc,
        font_body,
        C_TEXT,
        row1.left + 12,
        row1.top + 10,
        obfstr!("Show as"),
        text_max_1,
    );
    draw_text_ellipsis(
        hdc,
        font_small,
        C_MUTED,
        row1.left + 12,
        row1.top + 30,
        obfstr!("How it appears on your profile"),
        text_max_1,
    );

    y = row1.bottom + 8;

    // autostart row
    let row2 = RECT {
        left: PAD,
        top: y,
        right: client.right - PAD,
        bottom: y + 58,
    };
    fill_round(hdc, &row2, 10, C_CARD);
    let toggle = RECT {
        left: row2.right - 52,
        top: row2.top + 16,
        right: row2.right - 12,
        bottom: row2.top + 38,
    };
    let text_max_2 = (toggle.left - (row2.left + 12) - 10).max(0);
    draw_text_ellipsis(
        hdc,
        font_body,
        C_TEXT,
        row2.left + 12,
        row2.top + 10,
        obfstr!("Start with Windows"),
        text_max_2,
    );
    draw_text_ellipsis(
        hdc,
        font_small,
        C_MUTED,
        row2.left + 12,
        row2.top + 30,
        obfstr!("Launch automatically at login"),
        text_max_2,
    );
    st.hit.toggle = toggle;
    draw_toggle(hdc, &toggle, st.toggle_anim);

    let footer = format!("{}{}", obfstr!("Bridge on 127.0.0.1:"), st.port);
    // sit at the bottom if there's room, otherwise sit right under row2 with a small gap
    let footer_top = (client.bottom - 28).max(row2.bottom + 14);
    draw_text_center(
        hdc,
        font_small,
        C_MUTED,
        &RECT {
            left: PAD,
            top: footer_top,
            right: client.right - PAD,
            bottom: footer_top + 20,
        },
        0,
        &footer,
    );

    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(font_title);
    let _ = DeleteObject(font_body);
    let _ = DeleteObject(font_small);
    let _ = DeleteObject(font_h1);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

// Wrap a string to at most `max_lines` lines that each fit within `max_w`.
// If the text overflows, the last line ends with an ellipsis.
unsafe fn wrap_text(hdc: HDC, font: HFONT, text: &str, max_w: i32, max_lines: usize) -> Vec<String> {
    let old = SelectObject(hdc, font);
    let measure = |s: &str| -> i32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &w, &mut sz);
        sz.cx
    };
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in &words {
        let candidate = if cur.is_empty() {
            (*word).to_string()
        } else {
            format!("{} {}", cur, word)
        };
        if measure(&candidate) <= max_w {
            cur = candidate;
        } else {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                if lines.len() == max_lines {
                    // no room for more; append rest as ellipsis
                    let rest = &words[lines.len()..].join(" ");
                    let mut last = lines.pop().unwrap();
                    let mut trial = format!("{} {}…", last, rest);
                    while measure(&trial) > max_w && !last.is_empty() {
                        last.pop();
                        trial = format!("{}…", last);
                    }
                    lines.push(trial);
                    let _ = SelectObject(hdc, old);
                    return lines;
                }
            }
            // word itself may still be too long - break by chars
            let mut w = (*word).to_string();
            while measure(&w) > max_w && w.chars().count() > 1 {
                w.pop();
            }
            cur = w;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    let _ = SelectObject(hdc, old);
    lines
}

unsafe fn make_font(height: i32, weight: i32) -> HFONT {
    let face = wide("Segoe UI");
    CreateFontW(
        height,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        0,
        0,
        0,
        PCWSTR(face.as_ptr()),
    )
}

unsafe fn draw_text(hdc: HDC, font: HFONT, color: COLORREF, x: i32, y: i32, text: &str) {
    let old = SelectObject(hdc, font);
    let _ = SetTextColor(hdc, color);
    let w = wide(text);
    let _ = TextOutW(hdc, x, y, &w[..w.len() - 1]);
    let _ = SelectObject(hdc, old);
}

// Draw text at (x, y), truncating with an ellipsis if it can't fit in max_w px.
unsafe fn draw_text_ellipsis(
    hdc: HDC,
    font: HFONT,
    color: COLORREF,
    x: i32,
    y: i32,
    text: &str,
    max_w: i32,
) {
    if max_w <= 0 {
        return;
    }
    let old = SelectObject(hdc, font);
    let measure = |s: &str| -> i32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &w, &mut sz);
        sz.cx
    };
    let mut s = text.to_string();
    if measure(&s) > max_w {
        while s.chars().count() > 1 && measure(&format!("{}…", s)) > max_w {
            s.pop();
        }
        s.push('…');
    }
    let _ = SetTextColor(hdc, color);
    let w: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let _ = TextOutW(hdc, x, y, &w[..w.len() - 1]);
    let _ = SelectObject(hdc, old);
}

// Vertically-centered right-aligned text (last character ends at rc.right).
unsafe fn draw_text_center_right(hdc: HDC, font: HFONT, color: COLORREF, rc: &RECT, text: &str) {
    let old = SelectObject(hdc, font);
    let _ = SetTextColor(hdc, color);
    let w = wide(text);
    let s = &w[..w.len() - 1];
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, s, &mut sz);
    let x = rc.right - sz.cx;
    let y = rc.top + (rc.bottom - rc.top - sz.cy) / 2;
    let _ = TextOutW(hdc, x, y, s);
    let _ = SelectObject(hdc, old);
}

// Vertically-centered left-aligned text (draws inside `rc` at rc.left, y-centered).
unsafe fn draw_text_center_left(hdc: HDC, font: HFONT, color: COLORREF, rc: &RECT, text: &str) {
    let old = SelectObject(hdc, font);
    let _ = SetTextColor(hdc, color);
    let w = wide(text);
    let s = &w[..w.len() - 1];
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, s, &mut sz);
    let y = rc.top + (rc.bottom - rc.top - sz.cy) / 2;
    let _ = TextOutW(hdc, rc.left, y, s);
    let _ = SelectObject(hdc, old);
}

unsafe fn draw_text_center(hdc: HDC, font: HFONT, color: COLORREF, rc: &RECT, yoff: i32, text: &str) {
    let old = SelectObject(hdc, font);
    let _ = SetTextColor(hdc, color);
    let w = wide(text);
    let s = &w[..w.len() - 1];
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, s, &mut sz);
    let x = rc.left + (rc.right - rc.left - sz.cx) / 2;
    let y = rc.top + (rc.bottom - rc.top - sz.cy) / 2 + yoff;
    let _ = TextOutW(hdc, x, y, s);
    let _ = SelectObject(hdc, old);
}

// Paints `bg` over the specified corners of `rc` as if the rect were rounded.
// Used to make the thumbnail follow the card's curved top corners.
// mask: [top_left, top_right, bottom_left, bottom_right]
unsafe fn punch_corners(hdc: HDC, rc: &RECT, r: i32, bg: COLORREF, mask: [bool; 4]) {
    if r <= 0 {
        return;
    }
    let r = r.min((rc.right - rc.left) / 2).min((rc.bottom - rc.top) / 2);
    if r <= 0 {
        return;
    }
    let br_col = (bg.0 & 0xff) as u8;
    let bg_col = ((bg.0 >> 8) & 0xff) as u8;
    let brr_col = ((bg.0 >> 16) & 0xff) as u8;

    for (i, &m) in mask.iter().enumerate() {
        if !m {
            continue;
        }
        let (dst_x, dst_y, cx_f, cy_f) = match i {
            0 => (rc.left, rc.top, r as f32, r as f32),
            1 => (rc.right - r, rc.top, 0.0, r as f32),
            2 => (rc.left, rc.bottom - r, r as f32, 0.0),
            _ => (rc.right - r, rc.bottom - r, 0.0, 0.0),
        };

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: r,
                biHeight: -r,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let mem = CreateCompatibleDC(hdc);
        let hbmp = match CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(_) => {
                let _ = DeleteDC(mem);
                continue;
            }
        };
        if bits.is_null() {
            let _ = DeleteObject(hbmp);
            let _ = DeleteDC(mem);
            continue;
        }
        let old = SelectObject(mem, hbmp);

        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (r * r * 4) as usize);
        let rf = r as f32;
        for y in 0..r {
            for x in 0..r {
                let dx = x as f32 + 0.5 - cx_f;
                let dy = y as f32 + 0.5 - cy_f;
                let dist = (dx * dx + dy * dy).sqrt();
                // alpha for the area OUTSIDE the arc (i.e., where we paint bg)
                let alpha = if dist >= rf + 0.5 {
                    1.0
                } else if dist <= rf - 0.5 {
                    0.0
                } else {
                    dist - rf + 0.5
                };
                let idx = ((y * r + x) * 4) as usize;
                pixels[idx] = (br_col as f32 * alpha) as u8;
                pixels[idx + 1] = (bg_col as f32 * alpha) as u8;
                pixels[idx + 2] = (brr_col as f32 * alpha) as u8;
                pixels[idx + 3] = (255.0 * alpha) as u8;
            }
        }

        let bf = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let _ = AlphaBlend(hdc, dst_x, dst_y, r, r, mem, 0, 0, r, r, bf);

        let _ = SelectObject(mem, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(mem);
    }
}

// Anti-aliased rounded rectangle: fill a 32bpp DIB with per-pixel alpha at the
// corners, then AlphaBlend it onto hdc. Smooth like egui, no jagged edges.
unsafe fn fill_round(hdc: HDC, rc: &RECT, r: i32, color: COLORREF) {
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;
    if w <= 0 || h <= 0 {
        return;
    }
    let r = r.min(w / 2).min(h / 2).max(0);

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let mem = CreateCompatibleDC(hdc);
    let hbmp = match CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
        Ok(b) => b,
        Err(_) => {
            let _ = DeleteDC(mem);
            return;
        }
    };
    if bits.is_null() {
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(mem);
        return;
    }
    let old = SelectObject(mem, hbmp);

    let cr = (color.0 & 0xff) as u8;
    let cg = ((color.0 >> 8) & 0xff) as u8;
    let cb = ((color.0 >> 16) & 0xff) as u8;

    let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
    // Fill everything opaque with the color first
    for chunk in pixels.chunks_exact_mut(4) {
        chunk[0] = cb;
        chunk[1] = cg;
        chunk[2] = cr;
        chunk[3] = 0xff;
    }

    // Then soften the four corners with per-pixel alpha coverage
    if r > 0 {
        let rf = r as f32;
        for y in 0..r {
            for x in 0..r {
                let dx = rf - 0.5 - x as f32;
                let dy = rf - 0.5 - y as f32;
                let dist = (dx * dx + dy * dy).sqrt();
                let alpha = if dist <= rf - 0.5 {
                    1.0
                } else if dist >= rf + 0.5 {
                    0.0
                } else {
                    rf + 0.5 - dist
                };
                let ac = alpha * 255.0;
                let br = (cb as f32 * alpha) as u8;
                let bg_ = (cg as f32 * alpha) as u8;
                let brr = (cr as f32 * alpha) as u8;
                let ba = ac as u8;
                let corners: [(i32, i32); 4] = [
                    (x, y),
                    (w - 1 - x, y),
                    (x, h - 1 - y),
                    (w - 1 - x, h - 1 - y),
                ];
                for (cx, cy) in corners {
                    let idx = ((cy * w + cx) * 4) as usize;
                    pixels[idx] = br;
                    pixels[idx + 1] = bg_;
                    pixels[idx + 2] = brr;
                    pixels[idx + 3] = ba;
                }
            }
        }
    }

    let bf = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = AlphaBlend(hdc, rc.left, rc.top, w, h, mem, 0, 0, w, h, bf);

    let _ = SelectObject(mem, old);
    let _ = DeleteObject(hbmp);
    let _ = DeleteDC(mem);
}

unsafe fn draw_logo(hdc: HDC, x: i32, y: i32) {
    let sq = RECT {
        left: x,
        top: y,
        right: x + 30,
        bottom: y + 30,
    };
    fill_round(hdc, &sq, 7, C_ACCENT);
    let white = CreateSolidBrush(C_TEXT);
    let bars = [(6, 12), (13, 7), (20, 11)];
    for (bx, by) in bars {
        let bar = RECT {
            left: x + bx,
            top: y + by,
            right: x + bx + 4,
            bottom: y + 22,
        };
        let _ = FillRect(hdc, &bar, HBRUSH(white.0));
    }
    let _ = DeleteObject(white);
}

unsafe fn draw_dot(hdc: HDC, cx: i32, cy: i32, color: COLORREF) {
    let r = 4;
    // nudge down 1px so it lines up visually with the text baseline
    let rc = RECT {
        left: cx - r,
        top: cy - r + 1,
        right: cx + r,
        bottom: cy + r + 1,
    };
    fill_round(hdc, &rc, r, color);
}

// Toggle with an animated knob position. `t` is 0..1 (0 = off, 1 = on).
unsafe fn draw_toggle(hdc: HDC, rc: &RECT, t: f32) {
    let t = t.clamp(0.0, 1.0);
    let track_r = (rc.bottom - rc.top) / 2;
    let bg = lerp_color(C_TRACK, C_ACCENT, t);
    fill_round(hdc, rc, track_r, bg);
    let kx_off = (rc.left + 11) as f32;
    let kx_on = (rc.right - 11) as f32;
    let kx = kx_off + (kx_on - kx_off) * t;
    let ky = (rc.top + rc.bottom) / 2;
    let knob = RECT {
        left: kx as i32 - 8,
        top: ky - 8,
        right: kx as i32 + 8,
        bottom: ky + 8,
    };
    fill_round(hdc, &knob, 8, C_TEXT);
}

unsafe fn blit_thumb(hdc: HDC, bmp: HBITMAP, dst: &RECT, src_w: i32, src_h: i32, fade: f32) {
    let mem = CreateCompatibleDC(hdc);
    let old = SelectObject(mem, bmp);
    let w = dst.right - dst.left;
    let h = dst.bottom - dst.top;
    let sw = if src_w > 0 { src_w } else { w };
    let sh = if src_h > 0 { src_h } else { h };
    if fade >= 0.999 {
        let _ = StretchBlt(hdc, dst.left, dst.top, w, h, mem, 0, 0, sw, sh, SRCCOPY);
    } else {
        let bf = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (fade.clamp(0.0, 1.0) * 255.0) as u8,
            AlphaFormat: 0,
        };
        let _ = AlphaBlend(hdc, dst.left, dst.top, w, h, mem, 0, 0, sw, sh, bf);
    }
    let _ = SelectObject(mem, old);
    let _ = DeleteDC(mem);
}
