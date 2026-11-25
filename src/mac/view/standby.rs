//! 메뉴 막대 대기, 어디서든 찾기, 로그인할 때 열기 (Windows의 트레이 대기·전역 단축키·시작 프로그램과 같은 자리).
//! - 메뉴 막대 아이콘(NSStatusItem): 대기를 켜면 생긴다. 메뉴는 창 열기·찾기·끝내기. 대기 중에 창을 닫으면 창만 숨는다 (view.rs)
//! - 어디서든 찾기: Carbon의 RegisterEventHotKey로 다른 앱을 쓰는 중에도 키를 받아, 창을 앞으로 가져와 검색을 연다.
//!   Windows처럼 앱이 도는 동안 늘 등록한다 (대기를 켜야 창을 닫은 뒤에도 쓸 수 있다). 보통 실행에서만 한다:
//!   시험·그림·창 시험은 사용자의 키를 가져가지 않는다. Carbon은 실행 파일에 링크하지 않고 처음 쓸 때 연다
//! - 로그인할 때 열기: SMAppService.mainAppService (macOS 13+. ServiceManagement도 처음 쓸 때 연다). 시험에서는 등록하지 않는다
//! - 로그인으로 열렸는지: 앱을 연 Apple 이벤트에 '로그인 항목으로 열림'이 적혀 있는지 본다. 대기를 켰으면 그때는 창 없이 시작한다

use super::*;
use objc2_app_kit::{NSImage, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSError, NSNumber};
use sakadu::keys::Combo;
use std::cell::Cell;
use std::ffi::{CStr, c_void};

/// Carbon 이벤트: 키보드 갈래('keyb')의 전역 단축키 눌림(kEventHotKeyPressed), 이 앱의 단축키 서명,
/// 다른 앱이 이미 가진 키면 등록하지 않는 선택(kEventHotKeyExclusive)
const EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const EVENT_HOT_KEY_PRESSED: u32 = 5;
const HOT_KEY_SIGNATURE: u32 = u32::from_be_bytes(*b"SKDU");
const HOT_KEY_EXCLUSIVE: u32 = 1;
/// Carbon 수정 키 (cmdKey, shiftKey, optionKey, controlKey)
const CMD_KEY: u32 = 0x100;
const SHIFT_KEY: u32 = 0x200;
const OPTION_KEY: u32 = 0x800;
const CONTROL_KEY: u32 = 0x1000;

/// EventHotKeyID, EventTypeSpec (Carbon Events.h)
#[repr(C)]
#[derive(Clone, Copy)]
struct HotKeyId {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

/// EventHandlerUPP: (EventHandlerCallRef, EventRef, 넘긴 값) → OSStatus
type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

/// 쓰는 Carbon 함수들 (처음 쓸 때 프레임워크를 열어 찾는다)
#[derive(Clone, Copy)]
struct Carbon {
    /// GetApplicationEventTarget
    target: unsafe extern "C" fn() -> *mut c_void,
    /// InstallEventHandler(대상, 받는 함수, 종류 수, 종류들, 넘길 값, 받은 것)
    install: unsafe extern "C" fn(*mut c_void, Handler, usize, *const EventTypeSpec, *mut c_void, *mut *mut c_void) -> i32,
    /// RegisterEventHotKey(키 번호, 수정 키, 단축키 이름, 대상, 선택, 받은 것)
    register: unsafe extern "C" fn(u32, u32, HotKeyId, *mut c_void, u32, *mut *mut c_void) -> i32,
    /// UnregisterEventHotKey
    unregister: unsafe extern "C" fn(*mut c_void) -> i32,
    /// CopySymbolicHotKeys: 맥의 키보드 단축키 목록 (찾지 못하면 None: 그때는 맥이 쓰는 키를 가리지 않는다)
    symbolic: Option<unsafe extern "C" fn(*mut *const c_void) -> i32>,
}

thread_local! {
    /// 메뉴 막대 아이콘 (대기 중일 때만)
    static STATUS: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
    /// 등록한 어디서든 찾기 키 (EventHotKeyRef. 없으면 null)
    static HOTKEY: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
    /// 키를 받는 함수를 Carbon에 걸어 두었다 (한 번만 건다)
    static HANDLER: Cell<bool> = const { Cell::new(false) };
    static CARBON: OnceCell<Option<Carbon>> = const { OnceCell::new() };
    /// SMAppService 클래스 (ServiceManagement를 처음 쓸 때 연다. 찾지 못하면 None)
    static SERVICE: OnceCell<Option<&'static AnyClass>> = const { OnceCell::new() };
}

// ---------- 창 ----------

/// 본 창 (숨어 있어도). 아직 없으면 None
pub(super) fn main_window() -> Option<Retained<NSWindow>> {
    VIEW.with(|v| v.get().and_then(|v| v.window()))
}

/// 본 창을 앞으로: 숨겨 두었거나(대기 중에 닫았다) 최소화했거나 앱을 가렸으면 다시 띄우고, 앱을 앞으로 가져온다.
/// 창이 앞에 오면 그사이 바뀐 폴더를 다시 읽는다 (windowDidBecomeKey:)
pub(super) fn show_window(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    app.unhide(None);
    if let Some(w) = main_window() {
        if w.isMiniaturized() {
            w.deminiaturize(None);
        }
        w.makeKeyAndOrderFront(None);
    }
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

/// 어디서든 찾기 (전역 단축키, 메뉴 막대 아이콘의 '찾기'): 창을 앞으로 가져와 검색을 열고 키보드를 입력 칸에 둔다
pub(super) fn find_anywhere(mtm: MainThreadMarker) {
    show_window(mtm);
    with_app(App::open_find);
    after_menu();
}

// ---------- 메뉴 막대 아이콘 ----------

/// 메뉴 막대 아이콘을 대기 설정에 맞춘다: 켜져 있으면 두고(없으면 만든다), 꺼져 있으면 뗀다.
/// 끌 때 본 창이 숨어 있으면 다시 띄운다 (아이콘도 창도 없이 남지 않게)
pub(super) fn sync(mtm: MainThreadMarker) {
    let on = with_app(|a| a.standby()).unwrap_or(false);
    let shown = STATUS.with(|s| s.borrow().is_some());
    if on && !shown {
        let item = make_status_item(mtm);
        STATUS.with(|s| *s.borrow_mut() = Some(item));
        sync_tip(mtm);
    } else if !on && shown {
        if let Some(item) = STATUS.with(|s| s.borrow_mut().take()) {
            NSStatusBar::systemStatusBar().removeStatusItem(&item);
        }
        if main_window().is_some_and(|w| !w.isVisible()) {
            show_window(mtm);
        }
    }
}

/// 메뉴 막대 아이콘 (창 시험이 본다). 대기 중이 아니면 None
pub(super) fn status_item() -> Option<Retained<NSStatusItem>> {
    STATUS.with(|s| s.borrow().clone())
}

/// 메뉴 막대 아이콘을 만든다: 폴더 모양(SF Symbols, 메뉴 막대 색을 따르는 틀 그림)과 메뉴 (창 열기, 찾기, 끝내기)
fn make_status_item(mtm: MainThreadMarker) -> Retained<NSStatusItem> {
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("folder"), Some(&NSString::from_str("Sakadu")));
        match image {
            Some(img) => {
                img.setTemplate(true);
                button.setImage(Some(&img));
            }
            None => button.setTitle(&NSString::from_str("Sakadu")),
        }
    }
    let app = NSApplication::sharedApplication(mtm);
    let delegate = app.delegate();
    let menu = NSMenu::new(mtm);
    let add = |title: &str, action: Sel, target: Option<&AnyObject>| {
        // SAFETY: 받는 쪽(앱 위임, 앱)이 그 동작을 받는다 (앱 위임의 것은 define_class에 있다)
        unsafe {
            let it = menu.addItemWithTitle_action_keyEquivalent(&NSString::from_str(title), Some(action), &NSString::from_str(""));
            it.setTarget(target);
        }
    };
    let delegate: Option<&AnyObject> = delegate.as_deref().map(|d| d.as_ref());
    let app_object: &AnyObject = &app;
    add("창 열기", sel!(sakaduShowWindow:), delegate);
    add("찾기", sel!(sakaduFindAnywhere:), delegate);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add("끝내기", sel!(terminate:), Some(app_object));
    item.setMenu(Some(&menu));
    item
}

/// 메뉴 막대 아이콘의 풍선 도움말을 어디서든 찾기 키에 맞춘다
fn sync_tip(mtm: MainThreadMarker) {
    let Some(item) = status_item() else { return };
    let tip = with_app(|a| a.standby_tip()).unwrap_or_default();
    if let Some(button) = item.button(mtm) {
        button.setToolTip(Some(&NSString::from_str(&tip)));
    }
}

// ---------- 어디서든 찾기 ----------

/// 단축키 표의 키(Windows 가상 키 코드)를 맥의 키 번호(kVK_*)로 (뷰가 키를 읽는 표 KEY_CHARS·F_KEYS를 거꾸로 쓴다).
/// 맥 키로 바꿀 수 없으면 None
pub(super) fn carbon_key(vk: u16) -> Option<u32> {
    let code = match vk {
        0x20 => 0x31,
        0x0D => 0x24,
        0x09 => 0x30,
        0x1B => 0x35,
        0x08 => 0x33,
        // 앞으로 지우기(⌦)
        0x2E => 0x75,
        0x25 => 0x7B,
        0x26 => 0x7E,
        0x27 => 0x7C,
        0x28 => 0x7D,
        0x24 => 0x73,
        0x23 => 0x77,
        0x21 => 0x74,
        0x22 => 0x79,
        0x70..=0x83 => F_KEYS[(vk - 0x70) as usize],
        _ => {
            let name = sakadu::keys::key_name(vk)?;
            let mut chars = name.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else { return None };
            let c = c.to_ascii_lowercase();
            KEY_CHARS.iter().find(|k| k.1 == c)?.0
        }
    };
    Some(code as u32)
}

/// 수정 키를 Carbon의 값으로
pub(super) fn carbon_mods(c: Combo) -> u32 {
    let mut mods = 0;
    for (on, m) in [(c.cmd, CMD_KEY), (c.shift, SHIFT_KEY), (c.alt, OPTION_KEY), (c.ctrl, CONTROL_KEY)] {
        if on {
            mods |= m;
        }
    }
    mods
}

/// Carbon을 열어 쓰는 함수들을 찾는다 (처음 한 번). 실행 파일에 링크하지 않는 것은 열지 못하더라도 앱은 뜨게 하려는 것이다
fn carbon() -> Option<Carbon> {
    CARBON.with(|cell| {
        *cell.get_or_init(|| {
            // SAFETY: NUL로 끝나는 고정 경로다. 받은 핸들은 닫지 않는다 (앱이 끝날 때까지 쓴다)
            let lib = unsafe { libc::dlopen(c"/System/Library/Frameworks/Carbon.framework/Carbon".as_ptr(), libc::RTLD_LAZY) };
            if lib.is_null() {
                sakadu::diag::log(|| "hotkey: Carbon did not load".to_string());
                return None;
            }
            // SAFETY: 연 프레임워크에서 이름으로 찾는다
            let sym = |name: &CStr| Some(unsafe { libc::dlsym(lib, name.as_ptr()) }).filter(|p| !p.is_null());
            let found = || -> Option<Carbon> {
                // SAFETY: Carbon Events.h의 선언과 같은 꼴의 함수 포인터로 바꾼다
                unsafe {
                    Some(Carbon {
                        target: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *mut c_void>(sym(c"GetApplicationEventTarget")?),
                        install: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void, Handler, usize, *const EventTypeSpec, *mut c_void, *mut *mut c_void) -> i32>(sym(
                            c"InstallEventHandler",
                        )?),
                        register: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(u32, u32, HotKeyId, *mut c_void, u32, *mut *mut c_void) -> i32>(sym(c"RegisterEventHotKey")?),
                        unregister: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void) -> i32>(sym(c"UnregisterEventHotKey")?),
                        symbolic: sym(c"CopySymbolicHotKeys").map(|p| std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut *const c_void) -> i32>(p)),
                    })
                }
            };
            let carbon = found();
            if carbon.is_none() {
                sakadu::diag::log(|| "hotkey: Carbon functions missing".to_string());
            }
            carbon
        })
    })
}

/// 어디서든 찾기 키를 눌렀다 (Carbon이 메인 줄에서 부른다). 이벤트를 돌리는 도중이라 할 일은 메인 줄의 다음 차례로 미룬다
extern "C" fn hotkey_pressed(_call: *mut c_void, _event: *mut c_void, _data: *mut c_void) -> i32 {
    DispatchQueue::main().exec_async(|| {
        if let Some(mtm) = MainThreadMarker::new() {
            find_anywhere(mtm);
        }
    });
    0
}

/// 어디서든 찾기 키를 (다시) 등록한다: 앞의 것을 풀고, App이 고른 차례대로(정한 키, 아니면 자동 후보) 맥의 키보드 단축키와
/// 겹치지 않고 다른 앱이 먼저 갖지 않은 첫 키를 등록한다. 결과는 App에 넣는다 (설정 창과 메뉴 막대 아이콘에 보인다).
/// 시험·그림·창 시험(persist가 아닌 실행)에서는 등록하지 않는다
pub(super) fn register_hotkey() {
    unregister_hotkey();
    let choices = with_app(|a| if a.persist { a.global_choices() } else { Vec::new() }).unwrap_or_default();
    let got = if choices.is_empty() { None } else { carbon().and_then(|c| register_first(c, &choices)) };
    with_app(|a| a.hotkey = got);
    sakadu::diag::log(|| format!("hotkey -> {:?}", got.map(Combo::glyphs)));
    if let Some(mtm) = MainThreadMarker::new() {
        sync_tip(mtm);
    }
}

fn register_first(c: Carbon, choices: &[Combo]) -> Option<Combo> {
    // SAFETY: Carbon의 함수들이다. 받는 함수(hotkey_pressed)는 앱이 끝날 때까지 있다
    unsafe {
        let target = (c.target)();
        if !HANDLER.get() {
            let spec = EventTypeSpec { class: EVENT_CLASS_KEYBOARD, kind: EVENT_HOT_KEY_PRESSED };
            let mut handler = std::ptr::null_mut();
            let status = (c.install)(target, hotkey_pressed, 1, &spec, std::ptr::null_mut(), &mut handler);
            if status != 0 {
                sakadu::diag::log(|| format!("hotkey: InstallEventHandler {status}"));
                return None;
            }
            HANDLER.set(true);
        }
        let system = system_shortcuts(c);
        for &combo in choices {
            let Some(code) = carbon_key(combo.vk) else { continue };
            let mods = carbon_mods(combo);
            if system.contains(&(code, mods)) {
                continue;
            }
            let mut hotkey = std::ptr::null_mut();
            let status = (c.register)(code, mods, HotKeyId { signature: HOT_KEY_SIGNATURE, id: 1 }, target, HOT_KEY_EXCLUSIVE, &mut hotkey);
            if status == 0 && !hotkey.is_null() {
                HOTKEY.set(hotkey);
                return Some(combo);
            }
            sakadu::diag::log(|| format!("hotkey: {} taken ({status})", combo.glyphs()));
        }
    }
    None
}

/// 등록한 어디서든 찾기 키를 푼다 (끝낼 때, 다시 등록하기 전에)
pub(super) fn unregister_hotkey() {
    let hotkey = HOTKEY.replace(std::ptr::null_mut());
    if !hotkey.is_null()
        && let Some(c) = carbon()
    {
        // SAFETY: RegisterEventHotKey가 준 것을 한 번만 돌려준다
        unsafe { (c.unregister)(hotkey) };
    }
}

/// 맥의 키보드 단축키(시스템 설정 → 키보드 → 키보드 단축키)에서 켜 둔 것: (키 번호, 수정 키). Carbon에 등록해도 이것을 맥이
/// 먼저 받으므로 후보에서 뺀다 (⌥⌘Space는 보통 'Finder 검색 윈도우 보기'다). 읽지 못하면 비어 있다
fn system_shortcuts(c: Carbon) -> Vec<(u32, u32)> {
    let Some(copy) = c.symbolic else { return Vec::new() };
    let mut array: *const c_void = std::ptr::null();
    // SAFETY: CopySymbolicHotKeys는 새 배열을 준다 (Copy 규칙: 놓는 것은 받은 쪽)
    if unsafe { copy(&mut array) } != 0 {
        return Vec::new();
    }
    // SAFETY: CFArray는 NSArray와 같은 것이다 (toll-free bridged). 받은 몫을 Retained가 맡아 다 쓰면 놓는다
    let Some(list) = (unsafe { Retained::from_raw(array as *mut NSArray) }) else { return Vec::new() };
    let key = |name: &str| NSString::from_str(name);
    let (code_key, mods_key, on_key) = (key("kHISymbolicHotKeyCode"), key("kHISymbolicHotKeyModifiers"), key("kHISymbolicHotKeyEnabled"));
    list.iter()
        .filter_map(|item| {
            let d = item.downcast::<NSDictionary>().ok()?;
            let num = |k: &NSString| d.objectForKey(k)?.downcast::<NSNumber>().ok();
            if !num(&on_key)?.boolValue() {
                return None;
            }
            Some((num(&code_key)?.integerValue() as u32, num(&mods_key)?.integerValue() as u32))
        })
        .collect()
}

// ---------- 로그인할 때 열기 ----------

/// 로그인할 때 열기의 상태 (SMAppService.status)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Login {
    Off,
    On,
    /// 등록했지만 시스템 설정의 로그인 항목에서 허용을 기다린다
    NeedsApproval,
    /// 쓸 수 없다 (맥이 이 앱을 찾지 못했다: 앱 묶음 밖에서 실행 파일만 돌렸을 때 따위)
    Unavailable,
}

/// SMAppService.status의 값: 0 등록 안 됨, 1 켜짐, 2 허용을 기다림, 3 찾지 못함
fn login_of(status: NSInteger) -> Login {
    match status {
        0 => Login::Off,
        1 => Login::On,
        2 => Login::NeedsApproval,
        _ => Login::Unavailable,
    }
}

/// SMAppService 클래스. ServiceManagement를 처음 한 번 열어 찾는다
fn service_class() -> Option<&'static AnyClass> {
    SERVICE.with(|cell| {
        *cell.get_or_init(|| {
            // SAFETY: NUL로 끝나는 고정 경로다. 받은 핸들은 닫지 않는다
            let lib = unsafe { libc::dlopen(c"/System/Library/Frameworks/ServiceManagement.framework/ServiceManagement".as_ptr(), libc::RTLD_LAZY) };
            if lib.is_null() {
                sakadu::diag::log(|| "login item: ServiceManagement did not load".to_string());
                return None;
            }
            AnyClass::get(c"SMAppService")
        })
    })
}

/// 이 앱의 로그인 항목 (SMAppService.mainAppService)
fn main_service() -> Option<Retained<AnyObject>> {
    let cls = service_class()?;
    // SAFETY: +[SMAppService mainAppService] (macOS 13부터)
    unsafe { msg_send![cls, mainAppService] }
}

/// 로그인할 때 열기의 지금 상태 (설정 창을 열 때 묻는다)
pub(super) fn login_status() -> Login {
    let Some(service) = main_service() else { return Login::Unavailable };
    // SAFETY: -[SMAppService status]
    let status: NSInteger = unsafe { msg_send![&*service, status] };
    login_of(status)
}

/// 로그인할 때 열기를 켜거나 끄고, 그 뒤의 상태를 돌려준다. 보통 실행에서만 한다 (시험·창 시험은 이 맥의 로그인 항목을 건드리지 않는다)
pub(super) fn set_login(on: bool) -> Login {
    if !with_app(|a| a.persist).unwrap_or(false) {
        return login_status();
    }
    let Some(service) = main_service() else { return Login::Unavailable };
    // SAFETY: -[SMAppService registerAndReturnError:], -[SMAppService unregisterAndReturnError:]
    let done: Result<(), Retained<NSError>> = unsafe {
        if on {
            msg_send![&*service, registerAndReturnError: _]
        } else {
            msg_send![&*service, unregisterAndReturnError: _]
        }
    };
    if let Err(e) = done {
        sakadu::diag::log(|| format!("login item: {}", e.localizedDescription()));
    }
    login_status()
}

/// 시스템 설정의 로그인 항목을 연다 (허용을 기다릴 때)
pub(super) fn open_login_settings() {
    if let Some(cls) = service_class() {
        // SAFETY: +[SMAppService openSystemSettingsLoginItems] (macOS 13부터)
        let _: () = unsafe { msg_send![cls, openSystemSettingsLoginItems] };
    }
}

/// 로그인 항목으로 저절로 열렸는가: 앱을 연 Apple 이벤트('oapp')에 '로그인 항목으로 열림'('lgit')이 적혀 있다.
/// 앱이 뜨는 동안(applicationDidFinishLaunching:)에만 읽을 수 있다
pub(super) fn launched_at_login() -> bool {
    const OPEN_APPLICATION: u32 = u32::from_be_bytes(*b"oapp");
    const PROP_DATA: u32 = u32::from_be_bytes(*b"prdt");
    const LOGIN_ITEM: u32 = u32::from_be_bytes(*b"lgit");
    let Some(event) = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent() else { return false };
    // SAFETY: -[NSAppleEventDescriptor eventID]와 -paramDescriptorForKeyword: (AEEventID·AEKeyword는 4글자 코드)
    let id: u32 = unsafe { msg_send![&*event, eventID] };
    if id != OPEN_APPLICATION {
        return false;
    }
    let param: Option<Retained<NSAppleEventDescriptor>> = unsafe { msg_send![&*event, paramDescriptorForKeyword: PROP_DATA] };
    param.is_some_and(|p| p.enumCodeValue() == LOGIN_ITEM)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 뷰가 읽는 키(맥 키 번호 → 키 → 단축키 표의 키)를 Carbon 키 번호로 되돌리면 처음 번호가 나온다 (숫자판 Enter만 Return으로)
    #[test]
    fn carbon_keys_invert_what_the_view_reads() {
        let codes = KEY_CHARS.iter().map(|k| k.0).chain(F_KEYS).chain([0x7E, 0x7D, 0x7B, 0x7C, 0x73, 0x77, 0x74, 0x79, 0x30, 0x35, 0x24, 0x33, 0x31]);
        for code in codes {
            let combo = key_of(code).and_then(|k| k.combo(Mods { cmd: true, ..Mods::default() })).unwrap_or_else(|| panic!("key {code:#x}"));
            assert_eq!(carbon_key(combo.vk), Some(code as u32), "{}", combo.glyphs());
        }
        assert_eq!(carbon_key(0x0D), Some(0x24), "the keypad Enter reads as Return");
        // 맥 자판에 없는 키
        assert_eq!(carbon_key(0x2D), None, "Insert");
        assert_eq!(carbon_key(0x13), None, "Pause");
        let combo = |s: &str| Combo::parse(s).unwrap();
        assert_eq!(carbon_mods(combo("Alt+Cmd+Space")), OPTION_KEY | CMD_KEY);
        assert_eq!(carbon_mods(combo("Ctrl+Alt+Space")), CONTROL_KEY | OPTION_KEY);
        assert_eq!(carbon_mods(combo("Ctrl+Shift+Alt+Cmd+F")), 0x1B00);
        assert_eq!(carbon_key(combo("Alt+Cmd+F").vk), Some(0x03));
        assert_eq!(carbon_key(combo("Ctrl+Alt+Space").vk), Some(0x31));
    }

    /// 쓰는 Carbon 함수를 모두 찾는다 (하나라도 없으면 어디서든 찾기가 조용히 없어진다). 키를 등록하지는 않는다
    #[test]
    fn carbon_functions_are_found() {
        let c = carbon().expect("Carbon and its hot key functions load");
        assert!(c.symbolic.is_some(), "CopySymbolicHotKeys");
        // 맥의 키보드 단축키 목록을 읽을 수 있다 (러너에서 무엇이 켜져 있는지는 보지 않는다)
        let _ = system_shortcuts(c);
    }

    /// 로그인할 때 열기: SMAppService와 쓰는 메서드 이름이 맥이 아는 이름과 맞는다. 상태만 읽고 등록하지는 않는다
    #[test]
    fn login_item_names_match_the_system() {
        let cls = service_class().expect("SMAppService is found after opening ServiceManagement");
        for sel in [sel!(mainAppService), sel!(openSystemSettingsLoginItems)] {
            assert!(cls.metaclass().instance_method(sel).is_some(), "class method {sel:?}");
        }
        for sel in [sel!(status), sel!(registerAndReturnError:), sel!(unregisterAndReturnError:)] {
            assert!(cls.instance_method(sel).is_some(), "method {sel:?}");
        }
        assert_eq!([0, 1, 2, 3].map(login_of), [Login::Off, Login::On, Login::NeedsApproval, Login::Unavailable]);
        // 시험 실행 파일은 앱 묶음 밖이라 무엇이 나와도 좋다: 묻다가 멈추지 않으면 된다
        let _ = login_status();
    }
}
