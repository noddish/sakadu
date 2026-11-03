#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Sakadu: 가볍고 빠른 Windows 멀티뷰 파일 관리자

mod app;
mod defaultfm;
mod diag;
mod droptarget;
mod favorites;
mod gfx;
mod index;
mod listing;
mod pane;
mod service;
mod session;
mod shell;
mod theme;
mod thumbs;
mod undo;
mod util;

use std::cell::RefCell;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Result, w};

pub const WM_APP_LISTED: u32 = WM_APP + 1;
pub const WM_APP_ICONS: u32 = WM_APP + 2;
pub const WM_APP_SEARCH: u32 = WM_APP + 3;
pub const WM_APP_UNDO: u32 = WM_APP + 4;
pub const WM_APP_THUMB: u32 = WM_APP + 5;
pub const WM_APP_PREVIEW: u32 = WM_APP + 6;
/// 패널별 폴더 변경 알림 (WM_APP_NOTIFY + 패널 번호)
pub const WM_APP_NOTIFY: u32 = WM_APP + 0x10;

thread_local! {
    static APP: RefCell<Option<Box<app::App>>> = const { RefCell::new(None) };
}

/// UI 스레드의 App에 접근. 이미 처리 중이면(재진입) None
pub fn with_app<R>(f: impl FnOnce(&mut app::App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        let app = guard.as_mut()?;
        Some(f(app))
    })
}

fn app_busy() -> bool {
    APP.with(|c| c.try_borrow_mut().is_err())
}

/// 메시지 처리 중 미뤄 둔 모달 작업(메뉴, 드래그, 열기)을 App 빌림 없이 실행
pub fn run_deferred(hwnd: HWND) {
    loop {
        let actions = with_app(|a| std::mem::take(&mut a.pending)).unwrap_or_default();
        if actions.is_empty() {
            return;
        }
        for act in actions {
            app::execute(hwnd, act);
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if let Some(r) = shell::forward_menu_msg(msg, wp, lp) {
            return r;
        }
        match msg {
            WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
                if let Some(r) = app::edit_ctl_color(wp, lp) {
                    return r;
                }
            }
            WM_DPICHANGED => {
                with_app(|a| a.on_dpi((wp.0 & 0xFFFF) as f32));
                let rc = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOZORDER | SWP_NOACTIVATE);
                return LRESULT(0);
            }
            WM_GETMINMAXINFO => {
                let mmi = &mut *(lp.0 as *mut MINMAXINFO);
                let s = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                mmi.ptMinTrackSize = POINT { x: (760.0 * s) as i32, y: (420.0 * s) as i32 };
                return LRESULT(0);
            }
            WM_COPYDATA => {
                // 두 번째로 실행된 Sakadu(폴더 열기·Win+E)가 넘긴 경로
                let cds = &*(lp.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT);
                if cds.dwData == app::COPYDATA_OPEN {
                    let units = if cds.lpData.is_null() { &[][..] } else { std::slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2) };
                    let path = String::from_utf16_lossy(units);
                    if IsIconic(hwnd).as_bool() {
                        let _ = ShowWindow(hwnd, SW_RESTORE);
                    }
                    let _ = SetForegroundWindow(hwnd);
                    with_app(|a| a.open_external((!path.is_empty()).then_some(path)));
                    run_deferred(hwnd);
                    return LRESULT(1);
                }
            }
            WM_HOTKEY if wp.0 as i32 == app::HOTKEY_SEARCH => {
                // 다른 앱 위에서 눌러도 창을 앞으로 가져와 바로 검색 (App 빌림 밖에서 창 표시)
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
                with_app(|a| a.open_palette_global());
                run_deferred(hwnd);
                return LRESULT(0);
            }
            WM_DESTROY => {
                with_app(|a| a.shutdown());
                // 다른 프로세스에 있는 미리 보기 처리기는 App 빌림 밖에서 닫는다
                app::unload_handler();
                PostQuitMessage(0);
                return LRESULT(0);
            }
            _ => {}
        }
        match with_app(|a| a.handle(msg, wp, lp)) {
            Some(Some(r)) => {
                run_deferred(hwnd);
                r
            }
            Some(None) => {
                run_deferred(hwnd);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            None => {
                // 처리 도중 다시 들어온 알림·완료 메시지는 잃지 않도록 큐에 다시 넣는다
                if app_busy() && (msg == WM_COMMAND || (WM_APP..0xC000).contains(&msg)) {
                    let _ = PostMessageW(Some(hwnd), msg, wp, lp);
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
    }
}

/// 검색 서비스 관련 실행 옵션. 처리했으면 true
fn run_service_command() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a.eq_ignore_ascii_case(flag));
    let quiet = has("--quiet");
    let report = |title: &str, r: std::result::Result<(), String>| {
        if quiet {
            std::process::exit(if r.is_ok() { 0 } else { 1 });
        }
        let text = match r {
            Ok(()) => format!("{title} 완료"),
            Err(e) => format!("{title} 실패\n{e}"),
        };
        unsafe { MessageBoxW(None, &windows::core::HSTRING::from(text), w!("Sakadu"), MB_OK) };
    };
    if has("--service") {
        let _ = service::run_as_service();
    } else if has("--serve") {
        service::run_foreground();
    } else if has("--install-service") {
        report("검색 서비스 설치", service::install());
    } else if has("--uninstall-service") {
        report("검색 서비스 제거", service::uninstall());
    } else if has("--register-default") {
        report("기본 파일 관리자 등록", defaultfm::copy_self().and_then(|exe| defaultfm::register(defaultfm::CLASSES, &exe)));
    } else if has("--unregister-default") {
        defaultfm::unregister(defaultfm::CLASSES);
        report("기본 파일 관리자 해제", Ok(()));
    } else {
        return false;
    }
    true
}

/// 명령줄로 받은 열 경로 (옵션이 아닌 첫 인자). 폴더 열기·Win+E 연결에서 넘어온다
fn path_arg() -> Option<String> {
    std::env::args().skip(1).find(|a| !a.starts_with("--")).map(|a| a.trim_matches('"').to_string()).filter(|a| !a.is_empty())
}

/// 이미 떠 있는 Sakadu가 있으면 경로를 넘기고 true.
/// 그 창이 응답하지 않으면 폴더가 안 열리는 일이 없도록 탐색기로 대신 연다
fn forward_to_running(path: &Option<String>) -> bool {
    unsafe {
        let Ok(existing) = FindWindowW(w!("SakaduWindow"), windows::core::PCWSTR::null()) else { return false };
        if existing.is_invalid() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(existing, Some(&mut pid));
        // 사용자가 방금 실행했으므로 이 프로세스에 앞으로 가져올 권한이 있다. 그 권한을 기존 창에 넘긴다
        let _ = AllowSetForegroundWindow(pid);
        let data: Vec<u16> = path.as_deref().unwrap_or("").encode_utf16().collect();
        let cds = windows::Win32::System::DataExchange::COPYDATASTRUCT {
            dwData: app::COPYDATA_OPEN,
            cbData: (data.len() * 2) as u32,
            lpData: data.as_ptr() as *mut _,
        };
        let r = SendMessageTimeoutW(existing, WM_COPYDATA, WPARAM(0), LPARAM(&cds as *const _ as isize), SMTO_ABORTIFHUNG | SMTO_BLOCK, 5000, None);
        if r.0 == 0 {
            if let Some(p) = path {
                windows::Win32::UI::Shell::ShellExecuteW(None, w!("open"), w!("explorer.exe"), &windows::core::HSTRING::from(format!("\"{p}\"")), windows::core::PCWSTR::null(), SW_SHOWNORMAL);
            }
        }
        true
    }
}

fn started_without_activation() -> bool {
    use windows::Win32::System::Threading::{GetStartupInfoW, STARTF_USESHOWWINDOW, STARTUPINFOW};
    let mut si = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    unsafe { GetStartupInfoW(&mut si) };
    let cmd = si.wShowWindow as i32;
    si.dwFlags.contains(STARTF_USESHOWWINDOW) && [SW_SHOWNOACTIVATE.0, SW_SHOWMINNOACTIVE.0, SW_SHOWNA.0].contains(&cmd)
}

fn main() -> Result<()> {
    if run_service_command() {
        return Ok(());
    }
    let open_path = path_arg();
    if forward_to_running(&open_path) {
        return Ok(());
    }
    unsafe {
        // 매니페스트로 이미 설정돼 있으면 실패하지만 무해하다
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        OleInitialize(None)?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("SakaduWindow");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let sess = session::load();
        let s = GetDpiForSystem().max(96) as f32 / 96.0;
        let (mut x, mut y, mut w, mut h) = (CW_USEDEFAULT, CW_USEDEFAULT, (1280.0 * s) as i32, (820.0 * s) as i32);
        let mut maximized = false;
        if let Some(sess) = &sess {
            if let Some([sx, sy, sw, sh]) = sess.window {
                let rc = RECT { left: sx, top: sy, right: sx + sw, bottom: sy + sh };
                // 모니터 구성이 바뀌어 화면 밖이 되면 기본 위치로
                if sw > 200 && sh > 150 && !MonitorFromRect(&rc, MONITOR_DEFAULTTONULL).is_invalid() {
                    (x, y, w, h) = (sx, sy, sw, sh);
                }
            }
            maximized = sess.maximized;
        }

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("Sakadu"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let app = app::App::new(hwnd, sess)?;
        APP.with(|c| *c.borrow_mut() = Some(Box::new(app)));
        // 창부터 띄우고 목록은 그 뒤에 읽어, 시작하자마자 화면이 보이게 한다
        let _ = ShowWindow(hwnd, if maximized { SW_SHOWMAXIMIZED } else { SW_SHOWDEFAULT });
        with_app(|a| a.init());
        if open_path.is_some() {
            with_app(|a| a.open_external(open_path));
        }
        // 다른 프로그램이 "활성화하지 말고" 띄운 경우에는 포커스를 가져오지 않는다(SetFocus는 창을 활성화한다).
        // 사용자가 창을 누르면 WM_SETFOCUS에서 필터 상자로 옮긴다
        if !started_without_activation() {
            with_app(|a| a.focus());
        }
        run_deferred(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // COM 객체(렌더 타깃, 셸 객체)를 OLE 해제 전에 놓는다
        APP.with(|c| c.borrow_mut().take());
        OleUninitialize();
    }
    Ok(())
}
