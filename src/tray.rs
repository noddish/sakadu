//! 트레이 대기: 창을 닫아도 끝내지 않고 알림 영역 아이콘으로 남아, 전역 단축키로 언제든 검색할 수 있게 한다.
//! 켜면 로그인할 때 창 없이 시작하도록 현재 사용자의 시작 프로그램(HKCU Run)에 등록한다.

use crate::defaultfm::{delete_value, get, set};
use std::path::Path;
use std::sync::OnceLock;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{LIM_LARGE, LIM_SMALL, LoadIconMetric};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

pub const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Sakadu";
/// 창을 띄우지 않고 트레이에서 시작 (로그인할 때 쓰는 명령줄)
pub const ARG: &str = "--background";
/// build.rs가 실행 파일에 넣는 아이콘 리소스 번호
pub const ICON_APP: u16 = 1;
const ICON_BLACK: u16 = 2;
const ICON_WHITE: u16 = 3;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
const MENU_OPEN: usize = 1;
const MENU_QUIT: usize = 2;

pub fn is_enabled(run_key: &str) -> bool {
    get(run_key, RUN_VALUE).is_some()
}

/// 시작할 때 트레이 대기를 켤지: 시작 프로그램 등록(RUN_KEY)을 따른다.
/// SAKADU_TRAY=0|1이 있으면 그것을 따른다 (시험용: 레지스트리를 건드리지 않고 창을 닫을 때 숨기기·끝내기 두 길을 시험한다)
pub fn wait_at_start() -> bool {
    match std::env::var("SAKADU_TRAY").ok().as_deref() {
        Some("0") => false,
        Some("1") => true,
        _ => is_enabled(RUN_KEY),
    }
}

/// run_key는 보통 RUN_KEY. 테스트에서는 다른 경로를 넘겨 실제 시작 프로그램을 건드리지 않는다
pub fn enable(run_key: &str, exe: &Path) -> Result<(), String> {
    set(run_key, RUN_VALUE, &format!("\"{}\" {ARG}", exe.display()))
}

pub fn disable(run_key: &str) {
    delete_value(run_key, RUN_VALUE);
}

/// 실행 파일에 든 아이콘을 시스템 DPI에 맞는 큰·작은 아이콘 크기로
pub fn load_icon(id: u16, large: bool) -> HICON {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return HICON::default() };
        LoadIconMetric(Some(module.into()), PCWSTR(id as usize as *const u16), if large { LIM_LARGE } else { LIM_SMALL }).unwrap_or_default()
    }
}

/// 탐색기가 다시 시작해 작업 표시줄을 새로 만들면 오는 메시지. 이때 아이콘을 다시 넣어야 한다
pub fn taskbar_created() -> u32 {
    static MSG: OnceLock<u32> = OnceLock::new();
    *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

/// 알림 영역 아이콘. 놓으면(Drop) 알림 영역에서도 지운다
pub struct TrayIcon {
    hwnd: HWND,
    icon: HICON,
}

impl TrayIcon {
    pub fn add(hwnd: HWND, tip: &str) -> TrayIcon {
        // 관리자 권한으로 실행 중이어도 탐색기가 보내는 TaskbarCreated를 받는다
        let _ = unsafe { ChangeWindowMessageFilterEx(hwnd, taskbar_created(), MSGFLT_ALLOW, None) };
        let mut t = TrayIcon { hwnd, icon: HICON::default() };
        t.update(true, tip);
        t
    }

    /// 작업 표시줄 색(밝게·어둡게)에 맞는 아이콘으로 바꾼다. add는 작업 표시줄이 새로 만들어졌을 때
    pub fn update(&mut self, add: bool, tip: &str) {
        let icon = load_icon(if crate::theme::taskbar_is_dark() { ICON_WHITE } else { ICON_BLACK }, false);
        let mut nid = self.data();
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        nid.uCallbackMessage = crate::WM_APP_TRAY;
        nid.hIcon = icon;
        for (d, s) in nid.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
            *d = s;
        }
        unsafe {
            let _ = Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &nid);
            if add {
                nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
            }
            if !self.icon.is_invalid() {
                let _ = DestroyIcon(self.icon);
            }
        }
        self.icon = icon;
    }

    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: 1, ..Default::default() }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
            if !self.icon.is_invalid() {
                let _ = DestroyIcon(self.icon);
            }
        }
    }
}

pub enum Command {
    Open,
    Quit,
}

/// WM_APP_TRAY 처리. 아이콘을 누르면(Enter 포함) Open, 오른쪽 버튼·메뉴 키는 메뉴에서 고른 것.
/// 메뉴는 모달이므로 App 빌림 밖에서 부른다
pub fn on_message(hwnd: HWND, wp: WPARAM, lp: LPARAM) -> Option<Command> {
    // NOTIFYICON_VERSION_4: lParam 아래 16비트가 이벤트, wParam이 메뉴를 띄울 화면 좌표
    match (lp.0 & 0xFFFF) as u32 {
        NIN_SELECT | NIN_KEYSELECT => Some(Command::Open),
        WM_CONTEXTMENU => {
            let pt = POINT { x: (wp.0 & 0xFFFF) as i16 as i32, y: ((wp.0 >> 16) & 0xFFFF) as i16 as i32 };
            show_menu(hwnd, pt)
        }
        _ => None,
    }
}

fn show_menu(hwnd: HWND, pt: POINT) -> Option<Command> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN, w!("Sakadu 열기"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("끝내기"));
        let _ = SetMenuDefaultItem(menu, MENU_OPEN as u32, 0);
        // 알림 영역 메뉴는 먼저 창을 앞으로 가져와야 바깥을 눌렀을 때 닫힌다
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0, pt.x, pt.y, hwnd, None).0 as usize;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match cmd {
            MENU_OPEN => Some(Command::Open),
            MENU_QUIT => Some(Command::Quit),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실제 시작 프로그램 대신 HKCU\Software\SakaduTest 아래에서 등록·해제를 왕복한다
    #[test]
    #[ignore]
    fn enable_and_disable_in_test_key() {
        let key = "Software\\SakaduTest\\Run";
        disable(key);
        assert!(!is_enabled(key));
        enable(key, Path::new("C:\\Tools\\sakadu.exe")).unwrap();
        assert!(is_enabled(key));
        assert_eq!(get(key, RUN_VALUE).as_deref(), Some("\"C:\\Tools\\sakadu.exe\" --background"));
        disable(key);
        assert!(!is_enabled(key));
        let _ = unsafe { windows::Win32::System::Registry::RegDeleteTreeW(windows::Win32::System::Registry::HKEY_CURRENT_USER, w!("Software\\SakaduTest")) };
    }
}
