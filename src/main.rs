#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Sakadu: 가볍고 빠른 Windows 멀티뷰 파일 관리자

mod app;
mod defaultfm;
mod diag;
mod droptarget;
mod favorites;
mod folders;
mod gfx;
mod acl;
mod index;
mod keys;
mod listing;
mod pane;
mod service;
mod session;
mod shell;
mod theme;
mod thumbs;
mod tray;
mod undo;
mod util;
mod workspaces;

use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromRect,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Result, w};

pub const WM_APP_LISTED: u32 = WM_APP + 1;
pub const WM_APP_ICONS: u32 = WM_APP + 2;
pub const WM_APP_SEARCH: u32 = WM_APP + 3;
/// 파일 작업 결과 (shell::OpReport)
pub const WM_APP_FILEOP: u32 = WM_APP + 4;
pub const WM_APP_THUMB: u32 = WM_APP + 5;
pub const WM_APP_PREVIEW: u32 = WM_APP + 6;
/// 알림 영역(트레이) 아이콘을 누르거나 메뉴를 열 때
pub const WM_APP_TRAY: u32 = WM_APP + 7;
/// 사이드바 드라이브 목록 (app::sidebar::Places)
pub const WM_APP_PLACES: u32 = WM_APP + 8;
/// 뒤 스레드에서 경로를 확인한 결과 (app::Probed)
pub const WM_APP_PROBED: u32 = WM_APP + 9;
/// 검색 결과 줄의 수정한 날짜·크기 (app::palette::PalMeta)
pub const WM_APP_PALMETA: u32 = WM_APP + 10;
/// 패널별 폴더 변경 알림 (WM_APP_NOTIFY + 패널 번호)
pub const WM_APP_NOTIFY: u32 = WM_APP + 0x10;

thread_local! {
    static APP: RefCell<Option<Box<app::App>>> = const { RefCell::new(None) };
    /// 트레이에서 창 없이 시작했을 때, 처음 보일 때 쓸 표시 방법(세션의 최대화 여부)
    static FIRST_SHOW: Cell<Option<SHOW_WINDOW_CMD>> = const { Cell::new(None) };
    /// 다른 버전에 이어받기를 허락해 끝내는 중
    static QUITTING: Cell<bool> = const { Cell::new(false) };
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

/// 숨겨 두었거나(트레이) 최소화한 창을 보이고 앞으로 가져온다. App 빌림 밖에서 부른다
fn bring_to_front(hwnd: HWND) {
    unsafe {
        let hidden = !IsWindowVisible(hwnd).as_bool();
        if hidden {
            let _ = ShowWindow(hwnd, FIRST_SHOW.take().unwrap_or(SW_SHOW));
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
        if hidden {
            with_app(|a| a.on_shown());
        }
    }
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
            // 제목 줄 없애기: 위쪽 비클라이언트 영역을 클라이언트로 돌린다 (양옆·아래 크기 조절 테두리는 그대로).
            // 최대화하면 창이 화면 밖으로 테두리만큼 나가므로 위도 그만큼 안으로 들인다.
            // wParam이 FALSE(창을 만들 때)면 lParam은 RECT 하나, TRUE면 NCCALCSIZE_PARAMS. 둘 다 첫 사각형이 창→클라이언트
            WM_NCCALCSIZE => {
                let rect = if wp.0 != 0 { &mut (*(lp.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0] } else { &mut *(lp.0 as *mut RECT) };
                let top = rect.top;
                let r = DefWindowProcW(hwnd, msg, wp, lp);
                rect.top = top + if IsZoomed(hwnd).as_bool() { resize_border(hwnd) } else { 0 };
                return r;
            }
            WM_NCHITTEST => {
                let def = DefWindowProcW(hwnd, msg, wp, lp);
                if def.0 as u32 != HTCLIENT {
                    return def;
                }
                if let Some(Some(code)) = with_app(|a| a.nc_hit(lp)) {
                    return LRESULT(code as isize);
                }
                return def;
            }
            WM_NCMOUSEMOVE | WM_NCMOUSELEAVE | WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK | WM_NCLBUTTONUP => {
                if let Some(Some(r)) = with_app(|a| a.on_nc_mouse(msg, wp)) {
                    return r;
                }
            }
            WM_GETMINMAXINFO => {
                let mmi = &mut *(lp.0 as *mut MINMAXINFO);
                // 설정의 크기(%)만큼 최소 크기도 키운다 (창을 만드는 중이거나 App 처리 중이면 100%)
                let s = GetDpiForWindow(hwnd).max(96) as f32 / 96.0 * with_app(|a| a.zoom()).unwrap_or(1.0);
                mmi.ptMinTrackSize = POINT { x: (760.0 * s) as i32, y: (420.0 * s) as i32 };
                return LRESULT(0);
            }
            WM_COPYDATA => {
                // 두 번째로 실행된 Sakadu(폴더 열기·Win+E)가 넘긴 경로
                let cds = &*(lp.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT);
                if cds.dwData == app::COPYDATA_OPEN {
                    let units = if cds.lpData.is_null() { &[][..] } else { std::slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2) };
                    let path = String::from_utf16_lossy(units);
                    bring_to_front(hwnd);
                    with_app(|a| a.open_external((!path.is_empty()).then_some(path)));
                    run_deferred(hwnd);
                    return LRESULT(1);
                }
                if cds.dwData == app::COPYDATA_QUIT {
                    // 이미 끝내는 중이면(창을 닫으며 정리하는 동안 다시 요청받음) 그대로 둔다
                    if QUITTING.get() {
                        return LRESULT(app::QUIT_OK);
                    }
                    // 다른 버전의 Sakadu가 이어받으려 한다. 지금 끝나면 하던 일을 잃으면 거절한다
                    if quit_would_lose_work(hwnd) || with_app(|a| a.quit()).is_none() {
                        return LRESULT(app::QUIT_BUSY);
                    }
                    QUITTING.set(true);
                    // 보낸 쪽을 먼저 풀어 준다. 다른 프로세스가 보낸 메시지를 처리하는 동안에는
                    // 밖으로 나가는 COM 호출(미리 보기 처리기 정리 등)이 막혀 창을 닫으며 정리하지 못한다
                    let _ = ReplyMessage(LRESULT(app::QUIT_OK));
                    run_deferred(hwnd);
                    return LRESULT(app::QUIT_OK);
                }
            }
            WM_HOTKEY if wp.0 as i32 == app::HOTKEY_SEARCH => {
                // 다른 앱 위에서 눌러도 창을 앞으로 가져와 바로 검색 (App 빌림 밖에서 창 표시)
                bring_to_front(hwnd);
                with_app(|a| a.open_palette_global());
                run_deferred(hwnd);
                return LRESULT(0);
            }
            WM_APP_TRAY => {
                match tray::on_message(hwnd, wp, lp) {
                    Some(tray::Command::Open) => bring_to_front(hwnd),
                    Some(tray::Command::Quit) => {
                        with_app(|a| a.quit());
                        run_deferred(hwnd);
                    }
                    None => {}
                }
                return LRESULT(0);
            }
            m if m == tray::taskbar_created() => {
                with_app(|a| a.on_taskbar_created());
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

/// 지금 끝나면 하던 일을 잃는지: Sakadu가 시작한 파일 작업이 끝나지 않았거나, 확인 창이 떠 있거나(창이 비활성),
/// 이 프로세스에 다른 창이 보인다. 셸이 이 프로세스 안에서 하는 일(우클릭 메뉴의 붙여넣기·압축, 셸에 맡긴 끌어 놓기,
/// 속성 창)은 따로 셀 수 없어, 그 진행 창·속성 창이 보이는지로 대신 본다
fn quit_would_lose_work(main: HWND) -> bool {
    if shell::file_ops_running() || !unsafe { windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(main) }.as_bool() {
        return true;
    }
    struct Find {
        main: HWND,
        pid: u32,
        found: bool,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> windows::core::BOOL {
        let f = unsafe { &mut *(lp.0 as *mut Find) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        if h != f.main && pid == f.pid && unsafe { IsWindowVisible(h) }.as_bool() {
            // 디버그 빌드에 붙은 콘솔 창은 셈하지 않는다
            let mut class = [0u16; 32];
            let n = unsafe { GetClassNameW(h, &mut class) }.max(0) as usize;
            if String::from_utf16_lossy(&class[..n]) != "ConsoleWindowClass" {
                f.found = true;
                return windows::core::BOOL(0);
            }
        }
        windows::core::BOOL(1)
    }
    let mut f = Find { main, pid: unsafe { windows::Win32::System::Threading::GetCurrentProcessId() }, found: false };
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut f as *mut Find as isize)) };
    f.found
}

/// 이미 떠 있는 Sakadu 창
fn running_window() -> Option<HWND> {
    unsafe { FindWindowW(w!("SakaduWindow"), windows::core::PCWSTR::null()) }.ok().filter(|h| !h.is_invalid())
}

/// 시험용으로 따로 띄운 Sakadu인지 (SAKADU_ISOLATED 환경 변수가 있으면).
/// 화면 밖 스크린샷·점검 창이 사용자가 쓰고 있는 Sakadu와 서로 찾지 않게 창 종류 이름을 따로 쓰고,
/// 한 창 실행(시작 잠금·경로 넘기기·이어받기), 설치 사본 갱신, 트레이 대기, 전역 단축키를 쓰지 않는다
pub fn isolated() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SAKADU_ISOLATED").is_some())
}

/// 떠 있는 Sakadu에 경로를 넘긴다.
/// 그 창이 응답하지 않으면 폴더가 안 열리는 일이 없도록 탐색기로 대신 연다
fn forward_to_running(existing: HWND, path: &Option<String>) {
    unsafe {
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
    }
}

/// 떠 있는 다른 Sakadu 프로세스. 실행 파일 위치를 읽고 끝나기를 기다리는 데만 쓴다
struct Process(HANDLE);

impl Process {
    fn of_window(hwnd: HWND) -> Option<Process> {
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 {
            return None;
        }
        // 관리자 권한으로 떠 있어도 이 권한으로는 열린다
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok().map(Process)
    }

    fn exe(&self) -> Option<std::path::PathBuf> {
        use windows::Win32::System::Threading::{PROCESS_NAME_WIN32, QueryFullProcessImageNameW};
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        unsafe { QueryFullProcessImageNameW(self.0, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len) }.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]).into())
    }

    /// ms 안에 끝나면 true. 기다리기(SYNCHRONIZE) 권한이 없을 수 있어 끝남 코드를 읽어 본다
    fn wait_exit(&self, ms: u64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        loop {
            let mut code = 0u32;
            if unsafe { windows::Win32::System::Threading::GetExitCodeProcess(self.0, &mut code) }.is_err() {
                return false;
            }
            if code != STILL_ACTIVE.0 as u32 {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// 관리자 권한(UAC 상승)으로 실행 중인지. 알 수 없으면 None
fn elevated(process: HANDLE) -> Option<bool> {
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    unsafe {
        let mut token = HANDLE::default();
        windows::Win32::System::Threading::OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let r = GetTokenInformation(token, TokenElevation, Some(&mut e as *mut _ as *mut _), size_of::<TOKEN_ELEVATION>() as u32, &mut len);
        let _ = CloseHandle(token);
        r.ok()?;
        Some(e.TokenIsElevated != 0)
    }
}

/// 시작 순서 잠금. 떠 있는 창을 찾는 데서 새 창을 띄울 때까지 한 번에 한 실행만 지나가게 한다.
/// 이어받는 몇 초(이전 프로세스가 끝나기를 기다리고 사본을 바꾸는 동안) 사이에 다른 실행(폴더 열기, 두 번 누름,
/// 로그인할 때 시작)이 끼어들어 창이 둘 뜨거나 사본을 함께 쓰지 않게 한다
struct StartupLock {
    mutex: HANDLE,
    held: bool,
}

impl StartupLock {
    /// 아무것도 잠그지 않는다 (시험용으로 따로 띄운 Sakadu)
    fn none() -> StartupLock {
        StartupLock { mutex: HANDLE::default(), held: false }
    }

    fn acquire() -> StartupLock {
        use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
        use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        use windows::Win32::System::Threading::{CreateMutexW, MUTEX_MODIFY_STATE, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
        let name = w!("Local\\SakaduStartup");
        // 관리자 권한으로 실행된 Sakadu가 먼저 만들어도 일반 권한 실행이 열 수 있게, 로그인한(대화형) 사용자에게도 연다.
        // 그래도 무결성 수준 때문에 전체 권한으로는 못 여니, 기다리고 놓는 권한만으로 다시 연다
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let made = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w!("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;IU)"), SDDL_REVISION_1, &mut sd, None) }.is_ok();
        let sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() };
        let mutex = unsafe { CreateMutexW(made.then_some(&sa as *const _), false, name) }
            .or_else(|_| unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE, false, name) });
        if made {
            let _ = unsafe { LocalFree(Some(HLOCAL(sd.0))) };
        }
        if let Err(e) = &mutex {
            diag::log(|| format!("startup lock unavailable: {e}"));
        }
        let mut lock = StartupLock { mutex: mutex.unwrap_or_default(), held: false };
        lock.wait();
        lock
    }

    /// 다른 실행이 너무 오래 붙잡고 있으면(응답 없음 등) 기다리다 그냥 지나간다
    fn wait(&mut self) {
        if self.held || self.mutex.is_invalid() {
            return;
        }
        let r = unsafe { windows::Win32::System::Threading::WaitForSingleObject(self.mutex, 30_000) };
        self.held = r == WAIT_OBJECT_0 || r == WAIT_ABANDONED;
    }

    fn release(&mut self) {
        if std::mem::take(&mut self.held) {
            let _ = unsafe { windows::Win32::System::Threading::ReleaseMutex(self.mutex) };
        }
    }
}

impl Drop for StartupLock {
    fn drop(&mut self) {
        self.release();
        if !self.mutex.is_invalid() {
            let _ = unsafe { CloseHandle(self.mutex) };
        }
    }
}

/// 설치 사본을 바꾸지 못했다고 알린다
fn warn_sync_failed(owner: Option<HWND>, e: &str) {
    let text = format!(
        "로그인할 때와 폴더를 열 때 쓰는 Sakadu를 이 버전으로 바꾸지 못했어요.\n{e}\n\n\
         지금 쓰는 창은 이 버전이지만, 다음에 로그인하거나 폴더를 열면 이전 버전이 열릴 수 있어요. \
         이 파일을 다시 실행하면 다시 바꿔 봐요."
    );
    unsafe { MessageBoxW(owner, &windows::core::HSTRING::from(text), w!("Sakadu"), MB_OK | MB_ICONWARNING | MB_SETFOREGROUND) };
}

/// 떠 있는 Sakadu에 끝내 달라고 한다 (app::QUIT_OK·QUIT_BUSY).
/// 0이면 이 요청을 모르는 0.5.1까지의 버전이거나, 관리자 권한으로 떠 있어 메시지가 막혔거나, 응답이 없다
fn ask_to_quit(existing: HWND) -> isize {
    let cds = windows::Win32::System::DataExchange::COPYDATASTRUCT { dwData: app::COPYDATA_QUIT, cbData: 0, lpData: std::ptr::null_mut() };
    let mut result = 0usize;
    let r = unsafe { SendMessageTimeoutW(existing, WM_COPYDATA, WPARAM(0), LPARAM(&cds as *const _ as isize), SMTO_ABORTIFHUNG | SMTO_BLOCK, 5000, Some(&mut result)) };
    if r.0 == 0 { 0 } else { result as isize }
}

/// 떠 있는 Sakadu를 두고 어떻게 할지 (take_over의 결과)
#[derive(PartialEq)]
enum TakeOver {
    /// 떠 있는 창에 경로만 넘긴다
    Forward,
    /// 직접 실행한 이 exe와 같은 빌드가 떠 있다. 경로를 넘기기 전에 설치 사본을 맞춰 둔다
    /// (사본을 바꾸지 못한 뒤에 같은 파일을 다시 실행한 경우)
    SameBuild,
    /// 떠 있던 Sakadu가 끝났다. 떠 있는 창을 다시 살핀 뒤 이 버전으로 시작한다
    Gone,
}

/// 떠 있는 Sakadu가 지금 직접 실행한 것과 다른 빌드면 그 프로세스를 끝내게 한다.
/// 이쪽이 로그인할 때·폴더를 열 때 실행된 설치 사본이면 이어받지 않고 경로만 넘긴다.
/// 안내 창이 떠 있는 동안은 lock을 풀어, 그사이 폴더 열기가 기다리지 않고 떠 있는 창으로 가게 한다
fn take_over(existing: HWND, lock: &mut StartupLock) -> TakeOver {
    // 디버그 빌드는 콘솔 창이 붙어 있어, 설치 사본이 되면 로그인할 때와 폴더를 열 때마다 콘솔이 뜬다
    if cfg!(debug_assertions) {
        return TakeOver::Forward;
    }
    let Ok(me) = std::env::current_exe() else { return TakeOver::Forward };
    if defaultfm::is_install_copy(&me) {
        return TakeOver::Forward;
    }
    let Some(process) = Process::of_window(existing) else { return TakeOver::Forward };
    let Some(theirs) = process.exe() else { return TakeOver::Forward };
    if defaultfm::same_build(&me, &theirs) {
        return TakeOver::SameBuild;
    }
    // 관리자 권한으로 실행한 이쪽이 일반 권한 창을 이어받지는 않는다. 일반 권한으로 실행되는 설치 사본(폴더 열기·Win+E)이
    // 관리자 권한 창에는 경로를 넘기지 못한다(UIPI). 일반 권한인 이쪽이 관리자 권한 창을 만나면 요청이 막혀(0) 안내 창으로 간다
    match elevated(unsafe { windows::Win32::System::Threading::GetCurrentProcess() }) {
        Some(false) => {}
        Some(true) if elevated(process.0) == Some(true) => {}
        _ => return TakeOver::Forward,
    }
    diag::log(|| format!("take over: running {} differs from {}", theirs.display(), me.display()));
    loop {
        let answer = ask_to_quit(existing);
        diag::log(|| format!("take over: quit request -> {answer}"));
        // 끝내 달라는 요청을 받아들였거나, 그사이 사용자가 끝냈다.
        // 프로세스가 늦게 끝나 사본이 잠겨 있어도 copy_self가 비켜 두고 바꾼다
        if answer == app::QUIT_OK || !unsafe { IsWindow(Some(existing)) }.as_bool() {
            process.wait_exit(10_000);
            return TakeOver::Gone;
        }
        let first = if answer == app::QUIT_BUSY {
            "실행 중인 Sakadu가 복사·이동 중이거나 진행 창·확인 창이 떠 있어서 바로 바꾸지 못했어요.\n\
             작업을 마친 뒤 [다시 시도]를 누르면 이 버전으로 열려요."
        } else {
            "다른 버전의 Sakadu가 이미 실행 중이에요.\n\
             트레이의 Sakadu 아이콘을 오른쪽 클릭해 [끝내기]를 누른 뒤(트레이 아이콘이 없으면 창을 닫은 뒤) [다시 시도]를 누르면 이 버전으로 열려요."
        };
        let copy = if defaultfm::install_copy_in_use() { "\n로그인할 때와 폴더를 열 때도 이 버전이 열리도록 바꿔요." } else { "" };
        let text = format!("{first}{copy}\n\n실행 중: {}\n지금 실행: {}\n\n[취소]를 누르면 실행 중인 창을 그대로 열어요.", theirs.display(), me.display());
        lock.release();
        let pick = unsafe { MessageBoxW(None, &windows::core::HSTRING::from(text), w!("Sakadu"), MB_RETRYCANCEL | MB_ICONINFORMATION | MB_SETFOREGROUND) };
        lock.wait();
        if pick != IDRETRY {
            // 창을 이미 끝냈으면 넘길 곳이 없으니 이 버전으로 연다
            if unsafe { IsWindow(Some(existing)) }.as_bool() {
                return TakeOver::Forward;
            }
            process.wait_exit(10_000);
            return TakeOver::Gone;
        }
        // 다시 시도: 아직 떠 있으면 곧바로 다시 요청하고, 끝냈으면 위에서 Gone
    }
}

/// 모니터 작업 영역(작업 표시줄 제외, 픽셀)
fn work_area(monitor: HMONITOR) -> RECT {
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut mi);
    }
    mi.rcWork
}

/// 처음 열 때의 창: 마우스가 있는 모니터 작업 영역의 가운데.
/// 고정 크기는 배율이 큰 FHD 노트북(150%)에서 화면보다 커진다
fn default_window_rect(fallback: (i32, i32)) -> RECT {
    let mut pt = POINT::default();
    let wa = unsafe {
        let _ = GetCursorPos(&mut pt);
        work_area(MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY))
    };
    centered_in(wa).unwrap_or(RECT { left: 0, top: 0, right: fallback.0, bottom: fallback.1 })
}

/// 작업 영역의 가로 80%·세로 85%를 가운데에. 작업 영역을 알 수 없으면 None
fn centered_in(wa: RECT) -> Option<RECT> {
    let (aw, ah) = (wa.right - wa.left, wa.bottom - wa.top);
    if aw < 200 || ah < 150 {
        return None;
    }
    let (w, h) = ((aw as f32 * 0.8) as i32, (ah as f32 * 0.85) as i32);
    let (l, t) = (wa.left + (aw - w) / 2, wa.top + (ah - h) / 2);
    Some(RECT { left: l, top: t, right: l + w, bottom: t + h })
}

/// 저장된 창이 지금 모니터 작업 영역보다 크거나 밖으로 나가 있으면 안으로 줄이고 옮긴다
fn fit_to_work_area(rc: RECT) -> RECT {
    fit_in(rc, work_area(unsafe { MonitorFromRect(&rc, MONITOR_DEFAULTTONEAREST) }))
}

fn fit_in(rc: RECT, wa: RECT) -> RECT {
    let (aw, ah) = (wa.right - wa.left, wa.bottom - wa.top);
    if aw < 200 || ah < 150 {
        return rc;
    }
    let w = (rc.right - rc.left).min(aw);
    let h = (rc.bottom - rc.top).min(ah);
    let l = rc.left.clamp(wa.left, wa.right - w);
    let t = rc.top.clamp(wa.top, wa.bottom - h);
    RECT { left: l, top: t, right: l + w, bottom: t + h }
}

/// 창 크기 조절 테두리 두께(px, 창 DPI 기준). 제목 줄을 없앤 창의 위 가장자리 조절과 최대화 때 안쪽 여백에 쓴다
pub fn resize_border(hwnd: HWND) -> i32 {
    use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96);
        GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    }
}

fn started_with_position() -> bool {
    use windows::Win32::System::Threading::{GetStartupInfoW, STARTF_USEPOSITION, STARTUPINFOW};
    let mut si = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    unsafe { GetStartupInfoW(&mut si) };
    si.dwFlags.contains(STARTF_USEPOSITION)
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
    // panic=abort라 창도 없이 꺼지므로, --log로 켠 로그에 어디서 멈췄는지 남긴다
    std::panic::set_hook(Box::new(|info| diag::log(|| format!("panic: {info}"))));
    let background = std::env::args().skip(1).any(|a| a.eq_ignore_ascii_case(tray::ARG));
    let open_path = path_arg();
    let isolated = isolated();
    diag::log(|| format!("isolated: {isolated}"));
    let mut lock = if isolated { StartupLock::none() } else { StartupLock::acquire() };
    // 이어받는 동안 다른 Sakadu가 떴을 수 있어, 창이 하나도 없을 때까지 다시 살핀다
    while let Some(existing) = running_window().filter(|_| !isolated) {
        // 로그인할 때 시작했는데 이미 떠 있으면 그대로 둔다
        if background {
            return Ok(());
        }
        match take_over(existing, &mut lock) {
            TakeOver::Gone => continue,
            how => {
                if how == TakeOver::SameBuild {
                    let sync = defaultfm::sync_install_copy();
                    diag::log(|| format!("install copy sync (same build running) -> {sync:?}"));
                    if let Err(e) = sync {
                        lock.release();
                        warn_sync_failed(None, &e);
                    }
                }
                forward_to_running(existing, &open_path);
                return Ok(());
            }
        }
    }
    // 로그인할 때·폴더를 열 때 실행되는 설치 사본을 지금 실행한 버전으로 맞춘다.
    // 로그인할 때는 그 사본이 실행되고, 디버그 빌드는 콘솔 창이 붙어 있어 사본으로 쓰지 않는다
    let sync = if background || isolated || cfg!(debug_assertions) { Ok(false) } else { defaultfm::sync_install_copy() };
    diag::log(|| format!("install copy sync -> {sync:?}"));
    unsafe {
        // 매니페스트로 이미 설정돼 있으면 실패하지만 무해하다
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        OleInitialize(None)?;
        let instance = GetModuleHandleW(None)?;
        // 따로 띄운 시험 창은 다른 이름을 써서, 폴더 열기·이어받기가 서로를 찾지 않게 한다
        let class = if isolated { w!("SakaduIsolatedWindow") } else { w!("SakaduWindow") };
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hIcon: tray::load_icon(tray::ICON_APP, true),
            hIconSm: tray::load_icon(tray::ICON_APP, false),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let sess = session::load();
        let s = GetDpiForSystem().max(96) as f32 / 96.0;
        // 예전 판의 고정 기본 크기. 이 크기로 저장돼 있으면 사용자가 크기를 바꾼 적이 없다고 본다
        let old_default = ((1280.0 * s) as i32, (820.0 * s) as i32);
        let mut rc = default_window_rect(old_default);
        let mut maximized = false;
        if let Some(sess) = &sess {
            if let Some([sx, sy, sw, sh]) = sess.window {
                let saved = RECT { left: sx, top: sy, right: sx + sw, bottom: sy + sh };
                // 모니터 구성이 바뀌어 화면 밖이 되면 기본 위치로
                if sw > 200 && sh > 150 && (sw, sh) != old_default && !MonitorFromRect(&saved, MONITOR_DEFAULTTONULL).is_invalid() {
                    rc = fit_to_work_area(saved);
                }
            }
            maximized = sess.maximized;
        }
        // 실행한 쪽이 위치를 정해 주면(STARTF_USEPOSITION) 그 위치를 따른다
        let (x, y) = if started_with_position() { (CW_USEDEFAULT, CW_USEDEFAULT) } else { (rc.left, rc.top) };
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);

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
        // 제목 줄을 없앤 틀로 다시 계산하게 한다 (WM_NCCALCSIZE)
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        let app = app::App::new(hwnd, sess)?;
        APP.with(|c| *c.borrow_mut() = Some(Box::new(app)));
        if background {
            FIRST_SHOW.set(Some(if maximized { SW_SHOWMAXIMIZED } else { SW_SHOWNORMAL }));
        } else {
            // 창부터 띄우고 목록은 그 뒤에 읽어, 시작하자마자 화면이 보이게 한다
            let _ = ShowWindow(hwnd, if maximized { SW_SHOWMAXIMIZED } else { SW_SHOWDEFAULT });
        }
        with_app(|a| a.init());
        if open_path.is_some() {
            with_app(|a| a.open_external(open_path));
        }
        // 다른 프로그램이 "활성화하지 말고" 띄운 경우에는 포커스를 가져오지 않는다(SetFocus는 창을 활성화한다).
        // 사용자가 창을 누르면 WM_SETFOCUS에서 필터 상자로 옮긴다
        if !background && !started_without_activation() {
            with_app(|a| a.focus());
        }
        if !isolated && (background || tray::wait_at_start()) {
            with_app(|a| a.set_tray(true));
        }
        run_deferred(hwnd);
        // 창이 준비됐으니 기다리던 실행은 이 창에 경로를 넘긴다
        drop(lock);
        if let Err(e) = sync {
            warn_sync_failed(Some(hwnd), &e);
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn r(l: i32, t: i32, w: i32, h: i32) -> RECT {
        RECT { left: l, top: t, right: l + w, bottom: t + h }
    }

    /// 같은 사용자의 다른 프로세스도 권한 상승 여부를 읽을 수 있어야 이어받는다 (못 읽으면 경로만 넘긴다)
    #[test]
    #[ignore]
    fn reads_elevation_of_other_process() {
        use std::os::windows::io::AsRawHandle;
        let mine = elevated(unsafe { windows::Win32::System::Threading::GetCurrentProcess() });
        assert!(mine.is_some());
        let mut child = std::process::Command::new("ping").args(["-n", "3", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        let theirs = elevated(HANDLE(child.as_raw_handle()));
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(theirs, mine);
        // 창으로 연 프로세스 핸들(PROCESS_QUERY_LIMITED_INFORMATION)로도 읽힌다
        let limited = unsafe { windows::Win32::System::Threading::OpenProcess(windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION, false, std::process::id()) }.map(Process).unwrap();
        assert_eq!(elevated(limited.0), mine);
    }

    #[test]
    fn first_window_fits_fhd_work_area() {
        // FHD, 작업 표시줄 48px 제외. 배율 150%면 예전 기본 크기(1920×1230)는 화면을 넘었다
        let wa = r(0, 0, 1920, 1032);
        assert_eq!(centered_in(wa), Some(r(192, 77, 1536, 877)));
        assert_eq!(centered_in(r(0, 0, 0, 0)), None);
    }

    #[test]
    fn saved_window_is_pulled_inside() {
        let wa = r(0, 0, 1920, 1032);
        // 큰 모니터에서 쓰던 크기: 줄여서 화면 안으로
        assert_eq!(fit_in(r(-50, 100, 2400, 1300), wa), r(0, 0, 1920, 1032));
        // 오른쪽 아래로 삐져나간 창: 크기는 두고 옮긴다
        assert_eq!(fit_in(r(1500, 900, 800, 600), wa), r(1120, 432, 800, 600));
        // 이미 안에 있으면 그대로
        assert_eq!(fit_in(r(100, 100, 800, 600), wa), r(100, 100, 800, 600));
    }
}
