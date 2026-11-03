//! Windows 셸 연동: PIDL, 아이콘·형식 이름, 우클릭 메뉴, 파일 작업, 클립보드, 드래그, 변경 알림.
//! 복사·이동·삭제·이름 바꾸기는 IFileOperation을 써서 탐색기와 같은 진행 창, 충돌 처리, 실행 취소를 얻는다.

use crate::undo::{Done, Sink, Undo};
use crate::util::{self, wide};
use std::cell::RefCell;
use std::ffi::c_void;
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES, GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
};
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_RBUTTON, MK_SHIFT, MODIFIERKEYS_FLAGS};
use windows::Win32::System::Ole::*;
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
use windows::Win32::UI::Controls::HIMAGELIST;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

/// 스레드 사이로 창 핸들을 넘기기 위한 래퍼 (HWND는 프로세스 전역 값이라 안전)
#[derive(Clone, Copy)]
pub struct SendHwnd(pub HWND);
unsafe impl Send for SendHwnd {}
impl SendHwnd {
    pub fn get(&self) -> HWND {
        self.0
    }
}

/// 현재 스레드의 COM(STA) 초기화. drop 시 해제
pub struct ComInit(bool);
impl ComInit {
    pub fn sta() -> ComInit {
        ComInit(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok())
    }
}
impl Drop for ComInit {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

pub fn key_down(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

struct Pidl(*mut ITEMIDLIST);
impl Drop for Pidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CoTaskMemFree(Some(self.0 as *const c_void)) };
        }
    }
}

fn parse_pidl(path: &str) -> Result<Pidl> {
    let mut p = std::ptr::null_mut();
    unsafe { SHParseDisplayName(&HSTRING::from(path), None, &mut p, 0, None)? };
    Ok(Pidl(p))
}

/// 같은 부모 폴더에 있는 항목들의 셸 UI 객체(IContextMenu, IDataObject, IDropTarget 등)
pub fn ui_object<T: Interface>(hwnd: HWND, paths: &[String]) -> Result<T> {
    let pidls = paths.iter().map(|p| parse_pidl(p)).collect::<Result<Vec<_>>>()?;
    let first = pidls.first().ok_or_else(|| Error::from(E_INVALIDARG))?;
    unsafe {
        let folder: IShellFolder = SHBindToParent(first.0, None)?;
        let children: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| ILFindLastID(p.0) as *const _).collect();
        folder.GetUIObjectOf(hwnd, &children, None)
    }
}

/// 폴더 빈 곳의 우클릭 메뉴 (새로 만들기, 붙여넣기, 속성 등)
pub fn background_menu(hwnd: HWND, dir: &str) -> Result<IContextMenu> {
    let pidl = parse_pidl(dir)?;
    unsafe {
        let desktop = SHGetDesktopFolder()?;
        let folder: IShellFolder = desktop.BindToObject(pidl.0, None)?;
        folder.CreateViewObject(hwnd)
    }
}

pub fn folder_drop_target(hwnd: HWND, dir: &str) -> Result<IDropTarget> {
    ui_object(hwnd, &[dir.to_string()])
}

// ---------- 우클릭 메뉴 ----------

pub const MENU_FIRST: u32 = 1;
const MENU_LAST: u32 = 0x6FFF;
pub const MENU_CUSTOM: u32 = 0x7000;
const CMIC_MASK_UNICODE: u32 = 0x0000_4000;

thread_local! {
    /// 메뉴가 열려 있는 동안 "보내기"·"연결 프로그램" 같은 하위 메뉴 메시지를 셸에 전달하기 위해 보관
    static ACTIVE_MENU: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
}

/// 메뉴가 열려 있을 때 하위 메뉴 그리기·초기화 메시지를 셸 메뉴로 넘긴다
pub fn forward_menu_msg(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    if !matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        return None;
    }
    ACTIVE_MENU.with(|m| {
        let m = m.borrow();
        let cm = m.as_ref()?;
        unsafe {
            if let Ok(cm3) = cm.cast::<IContextMenu3>() {
                let mut res = LRESULT(0);
                if cm3.HandleMenuMsg2(msg, wp, lp, Some(&mut res)).is_ok() {
                    return Some(res);
                }
            } else if msg != WM_MENUCHAR {
                if let Ok(cm2) = cm.cast::<IContextMenu2>() {
                    if cm2.HandleMenuMsg(msg, wp, lp).is_ok() {
                        return Some(LRESULT(if msg == WM_INITMENUPOPUP { 0 } else { 1 }));
                    }
                }
            }
        }
        None
    })
}

pub struct Picked {
    pub id: u32,
    pub verb: String,
}

pub struct ShellMenu {
    cm: IContextMenu,
    menu: HMENU,
}

impl ShellMenu {
    pub fn new(cm: IContextMenu, extended: bool, custom: &[(u32, &str)]) -> Result<ShellMenu> {
        unsafe {
            let menu = CreatePopupMenu()?;
            let mut flags = CMF_NORMAL | CMF_CANRENAME;
            if extended {
                flags |= CMF_EXTENDEDVERBS;
            }
            let hr = cm.QueryContextMenu(menu, 0, MENU_FIRST, MENU_LAST, flags);
            if hr.is_err() {
                let _ = DestroyMenu(menu);
                return Err(hr.into());
            }
            for (i, (id, label)) in custom.iter().enumerate() {
                let _ = InsertMenuW(menu, i as u32, MF_BYPOSITION | MF_STRING, *id as usize, &HSTRING::from(*label));
            }
            if !custom.is_empty() {
                let _ = InsertMenuW(menu, custom.len() as u32, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null());
            }
            Ok(ShellMenu { cm, menu })
        }
    }

    pub fn track(&self, hwnd: HWND, pt: POINT) -> Option<Picked> {
        ACTIVE_MENU.with(|m| *m.borrow_mut() = Some(self.cm.clone()));
        let cmd = unsafe { TrackPopupMenuEx(self.menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None) }.0 as u32;
        ACTIVE_MENU.with(|m| *m.borrow_mut() = None);
        if cmd == 0 {
            return None;
        }
        let verb = if cmd >= MENU_CUSTOM { String::new() } else { self.verb(cmd - MENU_FIRST) };
        Some(Picked { id: cmd, verb })
    }

    fn verb(&self, offset: u32) -> String {
        let mut buf = [0u16; 128];
        let ok = unsafe { self.cm.GetCommandString(offset as usize, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32) };
        if ok.is_ok() { util::from_wide(&buf).to_lowercase() } else { String::new() }
    }

    pub fn invoke(&self, hwnd: HWND, id: u32, pt: Option<POINT>, dir: Option<&str>) -> Result<()> {
        invoke(&self.cm, hwnd, Verb::Id(id - MENU_FIRST), pt, dir)
    }

    pub fn invoke_verb(&self, hwnd: HWND, verb: &str, dir: Option<&str>) -> Result<()> {
        invoke(&self.cm, hwnd, Verb::Name(verb), None, dir)
    }
}

impl Drop for ShellMenu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.menu);
        }
    }
}

enum Verb<'a> {
    Id(u32),
    Name(&'a str),
}

fn invoke(cm: &IContextMenu, hwnd: HWND, verb: Verb, pt: Option<POINT>, dir: Option<&str>) -> Result<()> {
    let (verb_a, verb_w) = match verb {
        Verb::Name(v) => (v.bytes().chain(Some(0)).collect::<Vec<u8>>(), wide(v)),
        Verb::Id(_) => (Vec::new(), Vec::new()),
    };
    let dir_w = dir.map(wide);
    let mut info = CMINVOKECOMMANDINFOEX {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: CMIC_MASK_UNICODE,
        hwnd,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    match verb {
        Verb::Id(id) => {
            // MAKEINTRESOURCE: 하위 16비트에 명령 오프셋
            info.lpVerb = PCSTR(id as usize as *const u8);
            info.lpVerbW = PCWSTR(id as usize as *const u16);
        }
        Verb::Name(_) => {
            info.lpVerb = PCSTR(verb_a.as_ptr());
            info.lpVerbW = PCWSTR(verb_w.as_ptr());
        }
    }
    if let Some(d) = &dir_w {
        info.lpDirectoryW = PCWSTR(d.as_ptr());
    }
    if let Some(p) = pt {
        info.fMask |= CMIC_MASK_PTINVOKE;
        info.ptInvoke = p;
    }
    if key_down(VK_SHIFT) {
        info.fMask |= CMIC_MASK_SHIFT_DOWN;
    }
    if key_down(VK_CONTROL) {
        info.fMask |= CMIC_MASK_CONTROL_DOWN;
    }
    unsafe { cm.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO) }
}

/// 셸 확장 DLL을 미리 불러 두어 첫 우클릭 메뉴를 빠르게 한다.
/// 측정상 첫 메뉴 400~500ms → 예열 후 약 150ms. 낮은 우선순위 스레드에서 한 번만 실행
pub fn warm_up_menus() {
    std::thread::spawn(|| {
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
        let _com = ComInit::sta();
        let t = std::time::Instant::now();
        // 폴더용·파일용 확장이 다르므로 둘 다 연다
        let mut items = Vec::new();
        items.extend(known_folder(&FOLDERID_Documents));
        if let Some(w) = std::env::var_os("WINDIR") {
            items.push(format!("{}\\win.ini", w.to_string_lossy()));
        }
        for _ in 0..2 {
            for p in &items {
                if let Ok(cm) = ui_object::<IContextMenu>(HWND::default(), std::slice::from_ref(p)) {
                    let _ = ShellMenu::new(cm, false, &[]);
                }
            }
        }
        if let Some(parent) = items.first().and_then(|d| util::parent(d)) {
            if let Ok(cm) = background_menu(HWND::default(), &parent) {
                let _ = ShellMenu::new(cm, false, &[]);
            }
        }
        crate::diag::log(|| format!("menu: warm-up done in {:?}", t.elapsed()));
    });
}

/// 항목에 셸 동사(copy, cut, properties 등)를 실행
pub fn invoke_verb(hwnd: HWND, paths: &[String], verb: &str) -> Result<()> {
    let cm: IContextMenu = ui_object(hwnd, paths)?;
    ShellMenu::new(cm, false, &[])?.invoke_verb(hwnd, verb, None)
}

// ---------- 열기·경로 ----------

pub fn open_path(hwnd: HWND, path: &str, dir: Option<&str>) {
    let p = HSTRING::from(path);
    let d = HSTRING::from(dir.unwrap_or(""));
    unsafe { ShellExecuteW(Some(hwnd), PCWSTR::null(), &p, PCWSTR::null(), &d, SW_SHOWNORMAL) };
}

/// 바로 가기(.lnk)가 폴더를 가리키면 그 경로
pub fn shortcut_target_dir(path: &str) -> Option<String> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        file.Load(&HSTRING::from(path), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let target = util::from_wide(&buf);
        is_dir(&target).then_some(target)
    }
}

/// "shell:Downloads" 같은 셸 이름을 파일 시스템 경로로
pub fn resolve_shell_path(name: &str) -> Option<String> {
    let pidl = parse_pidl(name).ok()?;
    let mut buf = [0u16; 260];
    unsafe { SHGetPathFromIDListW(pidl.0, &mut buf) }.as_bool().then(|| util::from_wide(&buf))
}

fn attributes(path: &str) -> u32 {
    unsafe { GetFileAttributesW(&HSTRING::from(path)) }
}

pub fn exists(path: &str) -> bool {
    attributes(path) != INVALID_FILE_ATTRIBUTES
}

pub fn is_dir(path: &str) -> bool {
    let a = attributes(path);
    a != INVALID_FILE_ATTRIBUTES && a & FILE_ATTRIBUTE_DIRECTORY.0 != 0
}

pub fn known_folder(id: &GUID) -> Option<String> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const c_void));
        s
    }
}

// ---------- 아이콘·형식 이름 ----------

fn file_info(path: &str, attrs: u32, flags: SHGFI_FLAGS) -> SHFILEINFOW {
    let mut sfi = SHFILEINFOW::default();
    unsafe {
        SHGetFileInfoW(
            &HSTRING::from(path),
            FILE_FLAGS_AND_ATTRIBUTES(attrs),
            Some(&mut sfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    sfi
}

/// 표시 이름(현지화된 "다운로드", "로컬 디스크 (C:)")과 아이콘
pub fn display_info(path: &str) -> (String, i32) {
    let sfi = file_info(path, 0, SHGFI_DISPLAYNAME | SHGFI_SYSICONINDEX | SHGFI_SMALLICON);
    (util::from_wide(&sfi.szDisplayName), sfi.iIcon)
}

/// 확장자별 형식 이름과 아이콘 (디스크 접근 없음)
pub fn type_info(ext: &str, dir: bool) -> (String, i32) {
    let (name, attrs) = if dir { ("folder".to_string(), FILE_ATTRIBUTE_DIRECTORY.0) } else { (format!("file.{ext}"), FILE_ATTRIBUTE_NORMAL.0) };
    let sfi = file_info(&name, attrs, SHGFI_TYPENAME | SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES);
    (util::from_wide(&sfi.szTypeName), sfi.iIcon)
}

/// 파일마다 아이콘이 다른 형식(exe, lnk 등)용. 디스크를 읽으므로 백그라운드에서 호출
pub fn icon_for_path(path: &str) -> i32 {
    file_info(path, 0, SHGFI_SYSICONINDEX | SHGFI_SMALLICON).iIcon
}

pub fn system_imagelist() -> HIMAGELIST {
    let mut sfi = SHFILEINFOW::default();
    let h = unsafe {
        SHGetFileInfoW(
            w!(".txt"),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut sfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
        )
    };
    HIMAGELIST(h as isize)
}

// ---------- 파일 작업 ----------

pub enum FileOp {
    Delete { paths: Vec<String>, permanent: bool },
    Copy { paths: Vec<String>, dest: String },
    Move { paths: Vec<String>, dest: String },
    Rename { path: String, name: String },
    /// Ctrl+Z: 기록해 둔 작업을 반대로 실행
    Undo(Undo),
}

/// 진행 창이 떠 있는 동안에도 UI가 멈추지 않도록 별도 STA 스레드에서 실행.
/// 끝나면 실제 결과로 만든 되돌리기 기록을 창에 보낸다(WM_APP_UNDO)
pub fn spawn_file_op(hwnd: HWND, op: FileOp) {
    let owner = SendHwnd(hwnd);
    std::thread::spawn(move || {
        let _com = ComInit::sta();
        // 취소나 실패는 셸이 직접 안내 창을 띄우므로 여기서는 되돌리기 기록만 남긴다
        match run_file_op(owner.get(), &op) {
            Ok(done) if !matches!(op, FileOp::Undo(_)) => {
                if let Some(u) = Undo::from_done(&done) {
                    post_box(owner.get(), crate::WM_APP_UNDO, Box::new(u));
                }
            }
            Ok(_) => {}
            Err(e) => crate::diag::log(|| format!("file op failed: {e}")),
        }
    });
}

/// 백그라운드 스레드의 결과를 창 메시지로 넘긴다 (창이 없으면 해제)
pub fn post_box<T>(hwnd: HWND, msg: u32, payload: Box<T>) {
    let raw = Box::into_raw(payload);
    if unsafe { PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(raw as isize)) }.is_err() {
        drop(unsafe { Box::from_raw(raw) });
    }
}

/// 휴지통 폴더를 열거해 기록해 둔 항목($Recycle.Bin 안 실제 파일 경로)을 찾고, 그 항목의 "복원" 동사를 실행한다.
/// 셸이 삭제 직후 알려 주는 항목은 실제 파일이라 "복원"이 없으므로 휴지통 화면의 항목으로 다시 찾아야 한다
fn restore_from_recycle_bin(owner: HWND, items: &[(String, String)]) -> Result<()> {
    unsafe {
        let bin: IShellItem = SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)?;
        let all: IEnumShellItems = bin.BindToHandler(None, &BHID_EnumItems)?;
        let mut left = items.len();
        loop {
            let mut one = [None];
            let mut got = 0u32;
            if all.Next(&mut one, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            let Some(it) = one[0].take() else { break };
            let path = it.GetDisplayName(SIGDN_FILESYSPATH).ok().and_then(|p| {
                let s = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as *const c_void));
                s
            });
            if path.is_some_and(|p| items.iter().any(|(b, _)| util::eq_path(b, &p))) {
                let cm: IContextMenu = it.BindToHandler(None, &BHID_SFUIObject)?;
                ShellMenu::new(cm, false, &[])?.invoke_verb(owner, "undelete", None)?;
                left -= 1;
                if left == 0 {
                    break;
                }
            }
        }
        if left > 0 {
            crate::diag::log(|| format!("undo: {left} item(s) not found in recycle bin"));
        }
        Ok(())
    }
}

/// 실행하고 셸이 알려 준 실제 결과를 돌려준다. 중간에 취소돼도 이미 끝난 부분은 기록한다
fn run_file_op(owner: HWND, op: &FileOp) -> Result<Vec<Done>> {
    unsafe {
        if let FileOp::Undo(Undo::Recycle(items)) = op {
            restore_from_recycle_bin(owner, items)?;
            return Ok(Vec::new());
        }
        let fo: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        fo.SetOwnerWindow(owner)?;
        let sink = ComObject::new(Sink::new());
        let cookie = fo.Advise(&sink.to_interface::<IFileOperationProgressSink>())?;
        let item = |p: &str| -> Result<IShellItem> { SHCreateItemFromParsingName(&HSTRING::from(p), None) };
        let to_recycle = FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_WANTNUKEWARNING.0 | FOFX_RECYCLEONDELETE.0;
        match op {
            FileOp::Delete { paths, permanent } => {
                // 휴지통 삭제는 확인 없이(되돌릴 수 있음), 영구 삭제나 휴지통에 못 넣는 경우는 확인을 받는다
                let flags = if *permanent { FOF_WANTNUKEWARNING.0 } else { to_recycle };
                fo.SetOperationFlags(FILEOPERATION_FLAGS(flags))?;
                for p in paths {
                    fo.DeleteItem(&item(p)?, None)?;
                }
            }
            FileOp::Copy { paths, dest } | FileOp::Move { paths, dest } => {
                let copy = matches!(op, FileOp::Copy { .. });
                let same_dir = paths.iter().all(|p| util::parent(p).is_some_and(|d| util::eq_path(&d, dest)));
                if same_dir && !copy {
                    return Ok(Vec::new());
                }
                let mut flags = FOF_ALLOWUNDO.0 | FOF_NOCONFIRMMKDIR.0;
                if same_dir {
                    // 같은 폴더에 붙여넣으면 탐색기처럼 "- 복사본"을 만든다
                    flags |= FOF_RENAMEONCOLLISION.0;
                }
                fo.SetOperationFlags(FILEOPERATION_FLAGS(flags))?;
                let target = item(dest)?;
                for p in paths {
                    let it = item(p)?;
                    if copy {
                        fo.CopyItem(&it, &target, PCWSTR::null(), None)?;
                    } else {
                        fo.MoveItem(&it, &target, PCWSTR::null(), None)?;
                    }
                }
            }
            FileOp::Rename { path, name } => {
                fo.SetOperationFlags(FOF_ALLOWUNDO)?;
                fo.RenameItem(&item(path)?, &HSTRING::from(name.as_str()), None)?;
            }
            FileOp::Undo(u) => match u {
                Undo::Rename { now, name } => {
                    fo.SetOperationFlags(FOF_ALLOWUNDO)?;
                    fo.RenameItem(&item(now)?, &HSTRING::from(name.as_str()), None)?;
                }
                Undo::Move(moves) => {
                    fo.SetOperationFlags(FILEOPERATION_FLAGS(FOF_ALLOWUNDO.0 | FOF_NOCONFIRMMKDIR.0))?;
                    for (now, was) in moves {
                        let parent = util::parent(was).ok_or_else(|| Error::from(E_INVALIDARG))?;
                        fo.MoveItem(&item(now)?, &item(&parent)?, &HSTRING::from(util::file_name(was)), None)?;
                    }
                }
                Undo::Copy(paths) => {
                    fo.SetOperationFlags(FILEOPERATION_FLAGS(to_recycle))?;
                    for p in paths {
                        fo.DeleteItem(&item(p)?, None)?;
                    }
                }
                Undo::Create(p) => {
                    fo.SetOperationFlags(FILEOPERATION_FLAGS(to_recycle))?;
                    fo.DeleteItem(&item(p)?, None)?;
                }
                Undo::Recycle(_) => {}
            },
        }
        let r = fo.PerformOperations();
        let _ = fo.Unadvise(cookie);
        if let Err(e) = r {
            crate::diag::log(|| format!("file op ended early: {e}"));
        }
        Ok(sink.done.take())
    }
}

// ---------- 클립보드·드래그 ----------

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

/// 데이터 객체에 들어 있는 파일 경로 목록(CF_HDROP)
pub fn hdrop_paths(obj: &IDataObject) -> Option<Vec<String>> {
    unsafe {
        let mut med = obj.GetData(&hdrop_format()).ok()?;
        let hdrop = HDROP(med.u.hGlobal.0);
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        let mut paths = Vec::with_capacity(count as usize);
        for i in 0..count {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(hdrop, i, Some(&mut buf));
            paths.push(util::from_wide(&buf));
        }
        ReleaseStgMedium(&mut med);
        Some(paths)
    }
}

/// 끌어서 놓기의 기본 동작을 복사로: 수정 키 없이 놓으면 Ctrl을 더해 셸 드롭 대상에 넘긴다.
/// Shift(이동)·Ctrl·Alt(바로 가기)를 직접 누르거나 오른쪽 버튼으로 끌면(메뉴) 그대로 둔다
pub fn copy_by_default(keys: MODIFIERKEYS_FLAGS) -> MODIFIERKEYS_FLAGS {
    let explicit = MK_SHIFT.0 | MK_CONTROL.0 | MK_ALT | MK_RBUTTON.0;
    if keys.0 & explicit == 0 { MODIFIERKEYS_FLAGS(keys.0 | MK_CONTROL.0) } else { keys }
}

/// 데이터 객체에 "선호 동작"을 적는다. 탐색기 같은 다른 드롭 대상도 수정 키가 없으면 이 동작을 쓴다
pub fn set_preferred_effect(obj: &IDataObject, effect: DROPEFFECT) -> Result<()> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, 4)?;
        let p = GlobalLock(h) as *mut u32;
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err(E_OUTOFMEMORY.into());
        }
        *p = effect.0;
        let _ = GlobalUnlock(h);
        let mut fmt = hdrop_format();
        fmt.cfFormat = RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16;
        let medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: h }, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
        // fRelease=true: 성공하면 메모리는 데이터 객체가 해제한다
        obj.SetData(&fmt, &medium, true).inspect_err(|_| {
            let _ = GlobalFree(Some(h));
        })
    }
}

/// 진단 로그용: 셸 이동·복사는 비동기일 수 있어 잠시 뒤 경로가 있는지 기록한다
pub fn log_exists_later(what: &'static str, paths: Vec<String>) {
    if !crate::diag::enabled() || paths.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let present = paths.iter().filter(|p| std::path::Path::new(p).exists()).count();
        crate::diag::log(|| format!("{what}: {present}/{} after 1.5s (e.g. {})", paths.len(), paths[0]));
    });
}

/// 클립보드의 파일 목록과 "잘라내기"였는지 여부
pub fn clipboard_files() -> Option<(Vec<String>, bool)> {
    unsafe {
        let obj = OleGetClipboard().ok()?;
        let paths = hdrop_paths(&obj)?;
        let mut fmt = hdrop_format();
        fmt.cfFormat = RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16;
        let is_move = match obj.GetData(&fmt) {
            Ok(mut m) => {
                let p = GlobalLock(m.u.hGlobal) as *const u32;
                let v = if p.is_null() { 0 } else { *p };
                let _ = GlobalUnlock(m.u.hGlobal);
                ReleaseStgMedium(&mut m);
                v & DROPEFFECT_MOVE.0 != 0
            }
            Err(_) => false,
        };
        Some((paths, is_move))
    }
}

pub fn clear_clipboard() {
    unsafe {
        let _ = OleSetClipboard(None);
    }
}

/// bytes를 담은 전역 메모리 (클립보드에 넘기면 시스템이 해제한다)
fn global_bytes(bytes: &[u8]) -> Result<HGLOBAL> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err(E_OUTOFMEMORY.into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Ok(h)
    }
}

/// 클립보드를 비우고 (형식, 내용)들을 넣는다
fn set_clipboard(hwnd: HWND, items: Vec<(u32, Vec<u8>)>) -> Result<()> {
    unsafe {
        OpenClipboard(Some(hwnd))?;
        let r = EmptyClipboard().and_then(|_| {
            for (format, bytes) in items {
                let h = global_bytes(&bytes)?;
                if let Err(e) = SetClipboardData(format, Some(HANDLE(h.0))) {
                    let _ = GlobalFree(Some(h));
                    return Err(e);
                }
            }
            Ok(())
        });
        let _ = CloseClipboard();
        r
    }
}

fn utf16_bytes(units: &[u16]) -> Vec<u8> {
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}

/// CF_HDROP 내용: DROPFILES 머리 + 경로들(NUL로 구분, 끝에 NUL 하나 더)
fn hdrop_bytes(paths: &[String]) -> Vec<u8> {
    let header = size_of::<DROPFILES>();
    let df = DROPFILES { pFiles: header as u32, fWide: true.into(), ..Default::default() };
    let mut bytes = unsafe { std::slice::from_raw_parts(&df as *const DROPFILES as *const u8, header) }.to_vec();
    let mut names: Vec<u16> = Vec::new();
    for p in paths {
        names.extend(p.encode_utf16());
        names.push(0);
    }
    names.push(0);
    bytes.extend(utf16_bytes(&names));
    bytes
}

/// 파일 목록을 복사한 상태로 클립보드에 넣는다. 여러 폴더에 흩어진 파일도 한 번에 붙여넣을 수 있다
pub fn set_clipboard_files(hwnd: HWND, paths: &[String]) -> Result<()> {
    let effect = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) };
    set_clipboard(hwnd, vec![(CF_HDROP.0 as u32, hdrop_bytes(paths)), (effect, DROPEFFECT_COPY.0.to_le_bytes().to_vec())])
}

pub fn set_clipboard_text(hwnd: HWND, text: &str) -> Result<()> {
    set_clipboard(hwnd, vec![(CF_UNICODETEXT.0 as u32, utf16_bytes(&wide(text)))])
}

pub fn clipboard_text(hwnd: HWND) -> Option<String> {
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let text = GetClipboardData(CF_UNICODETEXT.0 as u32).ok().and_then(|h| {
            let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
            if p.is_null() {
                return None;
            }
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            let _ = GlobalUnlock(HGLOBAL(h.0));
            Some(s)
        });
        let _ = CloseClipboard();
        text
    }
}

pub fn drag_out(hwnd: HWND, paths: &[String]) -> Result<()> {
    let obj: IDataObject = ui_object(hwnd, paths).inspect_err(|e| crate::diag::log(|| format!("drag: data object failed: {e}")))?;
    // 탐색기 등 다른 앱에 놓을 때도 기본은 복사 (Shift를 누르면 이동)
    if let Err(e) = set_preferred_effect(&obj, DROPEFFECT_COPY) {
        crate::diag::log(|| format!("drag: preferred effect not set: {e}"));
    }
    crate::diag::log(|| format!("drag: SHDoDragDrop begin, lbutton down={}", key_down(windows::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON)));
    let r = unsafe { SHDoDragDrop(Some(hwnd), &obj, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK) };
    crate::diag::log(|| format!("drag: SHDoDragDrop -> {r:?}"));
    log_exists_later("drag: sources still at original place", paths.to_vec());
    r.map(|_| ())
}

// ---------- 변경 알림 ----------

/// 폴더 변경 알림 등록. 실패하면 0
pub fn watch(hwnd: HWND, dir: &str, msg: u32) -> u32 {
    let Ok(pidl) = parse_pidl(dir) else { return 0 };
    let entry = SHChangeNotifyEntry { pidl: pidl.0, fRecursive: false.into() };
    unsafe {
        SHChangeNotifyRegister(
            hwnd,
            SHCNRF_ShellLevel | SHCNRF_InterruptLevel | SHCNRF_NewDelivery,
            SHCNE_ALLEVENTS.0 as i32,
            msg,
            1,
            &entry,
        )
    }
}

pub fn unwatch(id: u32) {
    if id != 0 {
        unsafe {
            let _ = SHChangeNotifyDeregister(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use windows::Win32::System::SystemServices::MK_LBUTTON;

    /// 여러 폴더의 파일을 담은 CF_HDROP 내용을 셸이 그대로 읽는지 (클립보드는 건드리지 않는다)
    #[test]
    fn hdrop_bytes_are_readable_by_the_shell() {
        let paths = vec!["C:\\업무\\견적서.xlsx".to_string(), "\\\\nas\\scan\\20260420_160559.pdf".to_string()];
        let h = global_bytes(&hdrop_bytes(&paths)).unwrap();
        let hdrop = HDROP(h.0);
        let read: Vec<String> = unsafe {
            (0..DragQueryFileW(hdrop, u32::MAX, None))
                .map(|i| {
                    let mut buf = vec![0u16; DragQueryFileW(hdrop, i, None) as usize + 1];
                    DragQueryFileW(hdrop, i, Some(&mut buf));
                    util::from_wide(&buf)
                })
                .collect()
        };
        unsafe {
            let _ = GlobalFree(Some(h));
        }
        assert_eq!(read, paths);
    }

    /// 되돌리기 왕복: 이름 바꾸기·이동·휴지통 삭제를 실제로 하고 기록대로 되돌린다
    #[test]
    #[ignore]
    fn undo_roundtrip() {
        let _com = ComInit::sta();
        let base = std::env::temp_dir().join(format!("qp-undo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (src, dst) = (base.join("src"), base.join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        let a = src.join("sakadu-undo-test.txt");
        std::fs::write(&a, b"hello").unwrap();
        let a_s = a.to_string_lossy().into_owned();
        let owner = HWND::default();
        let run = |op: FileOp| run_file_op(owner, &op).unwrap();

        let done = run(FileOp::Rename { path: a_s.clone(), name: "renamed.txt".into() });
        let u = Undo::from_done(&done).expect("rename recorded");
        println!("rename -> {}", u.describe());
        assert!(src.join("renamed.txt").exists());
        run(FileOp::Undo(u));
        assert!(a.exists(), "rename undone");

        let done = run(FileOp::Move { paths: vec![a_s.clone()], dest: dst.to_string_lossy().into_owned() });
        let u = Undo::from_done(&done).expect("move recorded");
        println!("move -> {}", u.describe());
        assert!(!a.exists());
        run(FileOp::Undo(u));
        assert!(a.exists(), "move undone");

        let done = run(FileOp::Delete { paths: vec![a_s.clone()], permanent: false });
        let u = Undo::from_done(&done).expect("recycle recorded");
        println!("recycle -> {} {:?}", u.describe(), u);
        assert!(!a.exists());
        run(FileOp::Undo(u));
        let t0 = Instant::now();
        while !a.exists() && t0.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let restored = a.exists();
        let _ = std::fs::remove_dir_all(&base);
        assert!(restored, "restored from recycle bin");
    }

    /// 기본 복사 규칙이 실제 셸 드롭 대상에서 그대로 동작하는지 (파일을 실제로 만들고 옮겨 본다)
    #[test]
    #[ignore]
    fn drop_copies_by_default_and_moves_with_shift() {
        unsafe { OleInitialize(None).unwrap() };
        let none = MODIFIERKEYS_FLAGS(0);
        // (설명, 데이터 객체에 선호 동작=복사 표시, 끄는 중 키, 놓을 때 키) -> (원본 남음, 대상 도착)
        let run = |label: &str, prefer_copy: bool, drag_keys: MODIFIERKEYS_FLAGS, drop_keys: MODIFIERKEYS_FLAGS| -> (bool, bool) {
            let base = std::env::temp_dir().join(format!("qp-copy-{}-{}", std::process::id(), label.len()));
            let _ = std::fs::remove_dir_all(&base);
            let (src, dst) = (base.join("src"), base.join("dst"));
            std::fs::create_dir_all(&src).unwrap();
            std::fs::create_dir_all(&dst).unwrap();
            std::fs::write(src.join("a.txt"), b"hello").unwrap();
            let data: IDataObject = ui_object(HWND::default(), &[src.join("a.txt").to_string_lossy().into_owned()]).unwrap();
            if prefer_copy {
                set_preferred_effect(&data, DROPEFFECT_COPY).unwrap();
            }
            let target = folder_drop_target(HWND::default(), dst.to_str().unwrap()).unwrap();
            let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
            let pt = POINTL::default();
            let mut effect = allowed;
            unsafe { target.DragEnter(&data, drag_keys, pt, &mut effect).unwrap() };
            let shown = effect.0;
            let mut effect = allowed;
            unsafe { target.Drop(&data, drop_keys, pt, &mut effect).unwrap() };
            let t0 = Instant::now();
            while !dst.join("a.txt").exists() && t0.elapsed().as_secs() < 5 {
                let mut msg = MSG::default();
                unsafe {
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            let r = (src.join("a.txt").exists(), dst.join("a.txt").exists());
            println!("{label}: cursor effect={shown} -> source kept={}, arrived={}", r.0, r.1);
            let _ = std::fs::remove_dir_all(&base);
            r
        };
        let shift = MODIFIERKEYS_FLAGS(MK_LBUTTON.0 | MK_SHIFT.0);
        let baseline = run("baseline (no change, same drive)", false, MK_LBUTTON, none);
        let into_app = run("into Sakadu: keys get Ctrl", false, copy_by_default(MK_LBUTTON), copy_by_default(none));
        let to_explorer = run("to Explorer: preferred effect copy", true, MK_LBUTTON, none);
        let shift_move = run("into Sakadu with Shift", false, copy_by_default(shift), copy_by_default(MK_SHIFT));
        let shift_explorer = run("to Explorer with Shift", true, shift, MK_SHIFT);
        assert_eq!(baseline, (false, true), "same-drive default is move");
        assert_eq!(into_app, (true, true), "default copy into Sakadu");
        assert_eq!(to_explorer, (true, true), "default copy to Explorer");
        assert_eq!(shift_move, (false, true), "Shift moves into Sakadu");
        assert_eq!(shift_explorer, (false, true), "Shift moves to Explorer");
    }

    /// 셸 드롭 대상에 데이터 객체를 넘기면 실제로 복사되는지 (화면 없이 실행)
    #[test]
    #[ignore]
    fn shell_drop_target_copies_file() {
        unsafe { OleInitialize(None).unwrap() };
        let base = std::env::temp_dir().join(format!("qp-drop-{}", std::process::id()));
        let (src, dst) = (base.join("src"), base.join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();

        let data: IDataObject = ui_object(HWND::default(), &[src.join("a.txt").to_string_lossy().into_owned()]).unwrap();
        let target = folder_drop_target(HWND::default(), dst.to_str().unwrap()).unwrap();
        let mut effect = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
        let pt = POINTL::default();
        unsafe { target.DragEnter(&data, MK_LBUTTON, pt, &mut effect).unwrap() };
        println!("DragEnter effect = {}", effect.0);
        let mut effect = DROPEFFECT_COPY;
        unsafe { target.Drop(&data, MK_LBUTTON, pt, &mut effect).unwrap() };
        println!("Drop effect = {}", effect.0);

        // 셸 복사는 비동기일 수 있으므로 메시지를 돌리며 기다린다
        let t0 = Instant::now();
        while !dst.join("a.txt").exists() && t0.elapsed().as_secs() < 5 {
            let mut msg = MSG::default();
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let copied = dst.join("a.txt").exists();
        let _ = std::fs::remove_dir_all(&base);
        assert!(copied, "drop did not copy the file");
    }

    /// 백그라운드 스레드 예열이 다른 스레드의 첫 메뉴를 빠르게 하는지
    #[test]
    #[ignore]
    fn bench_context_menu_after_warmup() {
        let folder = known_folder(&FOLDERID_Documents).unwrap();
        let f2 = folder.clone();
        let t = Instant::now();
        std::thread::spawn(move || {
            let _com = ComInit::sta();
            for _ in 0..2 {
                let cm: IContextMenu = ui_object(HWND::default(), std::slice::from_ref(&f2)).unwrap();
                drop(ShellMenu::new(cm, false, &[]).unwrap());
            }
        })
        .join()
        .unwrap();
        println!("warm-up thread {:?}", t.elapsed());
        unsafe { OleInitialize(None).unwrap() };
        for round in 1..=3 {
            let t = Instant::now();
            let cm: IContextMenu = ui_object(HWND::default(), std::slice::from_ref(&folder)).unwrap();
            drop(ShellMenu::new(cm, false, &[]).unwrap());
            println!("after warm-up round {round}: {:?}", t.elapsed());
        }
    }

    /// 실제 드래그 순서 재현: 놓기 직전 버튼이 떼어진 상태로 DragOver를 다시 부르면 이동이 일어나는지
    #[test]
    #[ignore]
    fn shell_drop_after_buttonless_dragover() {
        unsafe { OleInitialize(None).unwrap() };
        let run = |extra_dragover: bool| -> (u32, bool) {
            let base = std::env::temp_dir().join(format!("qp-drop2-{}-{extra_dragover}", std::process::id()));
            let (src, dst) = (base.join("src"), base.join("dst"));
            std::fs::create_dir_all(&src).unwrap();
            std::fs::create_dir_all(&dst).unwrap();
            std::fs::write(src.join("a.txt"), b"hello").unwrap();
            let data: IDataObject = ui_object(HWND::default(), &[src.join("a.txt").to_string_lossy().into_owned()]).unwrap();
            let target = folder_drop_target(HWND::default(), dst.to_str().unwrap()).unwrap();
            let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
            let pt = POINTL::default();
            let mut effect = allowed;
            unsafe { target.DragEnter(&data, MK_LBUTTON, pt, &mut effect).unwrap() };
            let mut effect = allowed;
            unsafe { target.DragOver(MK_LBUTTON, pt, &mut effect).unwrap() };
            let mut effect = allowed;
            if extra_dragover {
                // 기존 Drop 구현이 하던 일: 버튼 없는 상태로 DragOver를 한 번 더 부르고 그 결과를 Drop에 넘김
                unsafe { target.DragOver(MODIFIERKEYS_FLAGS(0), pt, &mut effect).unwrap() };
            }
            let before = effect.0;
            unsafe { target.Drop(&data, MODIFIERKEYS_FLAGS(0), pt, &mut effect).unwrap() };
            println!("  Drop returned effect = {}", effect.0);
            let t0 = Instant::now();
            while !dst.join("a.txt").exists() && t0.elapsed().as_secs() < 5 {
                let mut msg = MSG::default();
                unsafe {
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let moved = dst.join("a.txt").exists();
            let _ = std::fs::remove_dir_all(&base);
            (before, moved)
        };
        let (before_old, moved_old) = run(true);
        let (before_new, moved_new) = run(false);
        println!("old order: effect passed to Drop = {before_old}, file arrived = {moved_old}");
        println!("fixed order: effect passed to Drop = {before_new}, file arrived = {moved_new}");
        assert!(moved_new);
    }

    /// 우클릭 메뉴를 만드는 단계별 시간 (처음 / 두 번째 이후)
    #[test]
    #[ignore]
    fn bench_context_menu() {
        unsafe { OleInitialize(None).unwrap() };
        let folder = known_folder(&FOLDERID_Documents).unwrap();
        let parent = util::parent(&folder).unwrap();
        for round in 1..=3 {
            let t0 = Instant::now();
            let cm: IContextMenu = ui_object(HWND::default(), std::slice::from_ref(&folder)).unwrap();
            let t1 = Instant::now();
            let menu = ShellMenu::new(cm, false, &[]).unwrap();
            let t2 = Instant::now();
            drop(menu);
            let bg = background_menu(HWND::default(), &parent).unwrap();
            let t3 = Instant::now();
            let menu = ShellMenu::new(bg, false, &[]).unwrap();
            let t4 = Instant::now();
            drop(menu);
            println!(
                "round {round}: item ui_object {:?}, item QueryContextMenu {:?} | background create {:?}, background QueryContextMenu {:?}",
                t1 - t0,
                t2 - t1,
                t3 - t2,
                t4 - t3
            );
        }
        // 플래그별 비교 (항목 메뉴)
        for (name, flags) in [("normal", CMF_NORMAL | CMF_CANRENAME), ("async", CMF_NORMAL | CMF_CANRENAME | CMF_ASYNCVERBSTATE)] {
            let mut times = Vec::new();
            for _ in 0..4 {
                let cm: IContextMenu = ui_object(HWND::default(), std::slice::from_ref(&folder)).unwrap();
                let menu = unsafe { CreatePopupMenu().unwrap() };
                let t = Instant::now();
                let _ = unsafe { cm.QueryContextMenu(menu, 0, MENU_FIRST, MENU_LAST, flags) };
                times.push(t.elapsed().as_millis());
                let count = unsafe { GetMenuItemCount(Some(menu)) };
                unsafe {
                    let _ = DestroyMenu(menu);
                }
                if times.len() == 1 {
                    println!("{name}: {count} items");
                }
            }
            println!("{name}: QueryContextMenu ms = {times:?}");
        }
    }
}

/// 새 방식(NewDelivery) 알림은 공유 메모리를 잠갔다 풀어야 해제된다
pub fn ack_notify(wp: WPARAM, lp: LPARAM) {
    unsafe {
        let lock = SHChangeNotification_Lock(HANDLE(wp.0 as *mut c_void), lp.0 as u32, None, None);
        if !lock.is_invalid() {
            let _ = SHChangeNotification_Unlock(lock);
        }
    }
}
