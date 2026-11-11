//! 메인 창: 레이아웃, 패널·탭 동작, 입력 처리, 그리기.
//!
//! 재진입 규칙: App은 RefCell 안에 있으므로, 모달 루프를 돌리는 호출(우클릭 메뉴, 드래그, ShellExecute)은
//! 처리 중에 직접 하지 않고 `pending`에 넣어 빌림이 풀린 뒤 `execute`에서 실행한다.

use crate::gfx::{Font, Gfx, R};
pub(crate) use sakadu::ui::glyph::*;
pub(crate) use sakadu::ui::*;
use crate::listing::{Entry, Kind, drive_type_label, format_size, format_time, list_dir};
use crate::pane::*;
use crate::shell::{self, FileOp, key_down, list_computer};
use crate::theme::{Color, FOCUS_W, R_CTRL, R_PANE, R_ROW, Theme, alpha, colorref};
use crate::{WM_APP_ICONS, WM_APP_LISTED, WM_APP_NOTIFY, WM_APP_SEARCH, session, util, with_app};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Ole::{IDropTarget, RegisterDragDrop, RevokeDragDrop};
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETCUEBANNER, EM_SETMARGINS, EM_SETSEL, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{
    DefSubclassProc, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Profile, IsUserAnAdmin, SHACF_AUTOAPPEND_FORCE_OFF,
    SHACF_AUTOSUGGEST_FORCE_ON, SHACF_FILESYS_DIRS, SHAutoComplete, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, PCWSTR, w};

mod fav_ui;
mod palette;
mod preview;
mod settings_ui;
mod view_menu;
mod caption;
mod sidebar;
mod start;
mod workspace_ui;
mod commands;
use fav_ui::G_STAR;
use palette::{G_SEARCH, PalGate, PalStatus, TIMER_SEARCH, TIMER_SEARCH_READY};
use sidebar::{BAR_W, Fold, Panel, Side, TIMER_BAR_HOVER, TIMER_SIDE_ANIM, TIMER_SIDE_CLOSE};
use preview::{G_PREVIEW, Preview, TIMER_PREVIEW, thumb_key};
pub use palette::SearchDone;
pub use preview::{PreviewMsg, unload_handler};


pub const ID_FILTER: i32 = 100;
pub const ID_RENAME: i32 = 101;
pub const ID_ADDR: i32 = 102;
pub const ID_SEARCH: i32 = 103;
/// 명령 메뉴(Ctrl+K)의 입력 상자
pub const ID_CMD: i32 = 104;
/// 전역 단축키 (Ctrl+Alt+Space): 어디서든 검색 패널
pub const HOTKEY_SEARCH: i32 = 1;
/// 다른 Sakadu 프로세스가 WM_COPYDATA로 넘기는 "이 경로를 열어 달라" 요청
pub const COPYDATA_OPEN: usize = 0x5150_0001;
/// 다른 버전의 Sakadu가 이어받으려고 보내는 "끝내 달라" 요청. 0.5.1까지는 모르는 요청이라 0을 돌려준다
pub const COPYDATA_QUIT: usize = 0x5150_0002;
/// COPYDATA_QUIT 응답: 끝내는 중 / 작업 중이라 거절
pub const QUIT_OK: isize = 1;
pub const QUIT_BUSY: isize = 2;
const ID_OPEN_TAB: u32 = shell::MENU_CUSTOM;
const ID_ADD_FAV: u32 = shell::MENU_CUSTOM + 1;
const TIMER_REFRESH: usize = 10;
/// 짧은 날짜(방금·오늘·어제)를 시간이 흐르는 대로 다시 그린다
const TIMER_DATES: usize = 49;
/// 작업 줄·사이드바 단추 말풍선을 띄우기까지
const TIMER_TIP: usize = 42;
/// 칸 아래 알림(flash)을 지우기까지
const TIMER_FLASH: usize = 44;
const FLASH_MS: u32 = 5000;
/// 되돌리기 단추가 달린 알림은 조금 더 오래 (누를 시간)
const UNDO_TOAST_MS: u32 = 7000;
const TOAST_UNDO: &str = "되돌리기";
/// 이미 있는 즐겨찾기를 다시 넣으려 할 때 그 줄을 잠깐 강조하는 시간
const TIMER_FAV_FLASH: usize = 45;



#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    None,
    Layout(usize),
    /// 경로 줄의 새 폴더 단추
    NewFolder(usize),
    Hidden,
    SplitX,
    SplitY,
    Tab(usize, usize),
    TabClose(usize, usize),
    TabAdd(usize),
    TabStrip(usize),
    Back(usize),
    Fwd(usize),
    Up(usize),
    Crumb(usize, usize),
    CrumbEmpty(usize),
    /// 보기 방식 버튼 (누르면 보기 메뉴)
    View(usize),
    /// 보기 메뉴의 줄 (ViewMode::ALL 순서)
    ViewMenuItem(usize),
    /// 보기 메뉴 안의 빈 곳
    ViewMenuPanel,
    /// 창 조절 버튼 (최소화·최대화·닫기). 비클라이언트 메시지로만 정해진다
    Caption(caption::CaptionButton),
    Header(usize, SortKey),
    HeaderResize(usize, usize),
    Row(usize, usize),
    ListEmpty(usize),
    Thumb(usize),
    Track(usize),
    /// 칩 줄 필터 칩의 × (필터 지우기)
    FilterClear(usize),
    /// 칸 아래쪽 알림 (누르면 아무 일도 없다: 그 아래 줄을 누르지 않게)
    Toast(usize),
    /// 열지 못한 네트워크 폴더의 로그인·다시 연결 단추
    NetFix(usize),
    /// 명령 메뉴의 줄, 메뉴 안 빈 곳, 메뉴 밖(누르면 닫는다)
    CmdRow(usize),
    CmdPanel,
    CmdScrim,
    /// 알림의 되돌리기 단추
    ToastUndo(usize),
    Pane(usize),
    /// 왼쪽 패널 안의 줄·단추 (즐겨찾기·드라이브·검색 폴더)
    Side(Side),
    /// 아이콘 막대의 패널 아이콘
    Bar(Panel),
    /// 패널 머리의 핀 (고정·풀기)
    SidePin,
    /// 패널 안의 빈 곳 (떠 있는 패널 아래 칸을 누르지 않게)
    SidePanel,
    /// 고정한 패널의 오른쪽 가장자리 (너비 조절)
    SideResize,
    /// 작업 줄 오른쪽 `…` (도움말·설정·정보 메뉴)
    More,
    /// 검색 패널의 "이 폴더도 검색하기" 버튼
    PalAddFolder,
    SearchButton,
    PalRow(usize),
    PalInstall,
    /// 검색 패널 머리의 닫기
    PalClose,
    /// 검색 범위 칩 (false: PC 전체, true: 현재 폴더)
    PalScope(bool),
    PreviewButton,
    SettingsButton,
    /// 설정 창 안
    Set(settings_ui::SetHit),
    /// 경로 오른쪽 끝의 즐겨찾기 별
    Star(usize),
    /// 새 탭 시작 화면의 카드·줄 (칸, 항목)
    Start(usize, usize),
    /// 미리 보기 머리의 핀 (파일 고정·풀기)
    PreviewPin,
    /// 미리 보기 머리의 열기 (기본 앱으로)
    PreviewOpen,
    /// 미리 보기의 고정 칩 (누르면 핀과 같다)
    PreviewChip,
    /// 작업 줄 왼쪽의 작업 공간 단추 (메뉴)
    Workspace,
}

impl Hit {
    fn pane(self) -> Option<usize> {
        use Hit::*;
        match self {
            Tab(p, _) | TabClose(p, _) | TabAdd(p) | TabStrip(p) | Back(p) | Fwd(p) | Up(p) | Crumb(p, _) | CrumbEmpty(p) | Star(p) | View(p)
            | Header(p, _) | HeaderResize(p, _) | Row(p, _) | ListEmpty(p) | Thumb(p) | Track(p) | FilterClear(p) | NewFolder(p) | Pane(p)
            | Start(p, _) => Some(p),
            _ => Option::None,
        }
    }
}

enum Drag {
    None,
    SplitX,
    SplitY,
    Col { pane: usize, col: usize, start_x: f32, start_w: f32 },
    Thumb { pane: usize, grab: f32 },
    Press { pane: usize, vi: usize, x: f32, y: f32, select_on_up: bool },
    /// 사이드바 즐겨찾기 줄을 눌렀다. 조금 움직이면 FavMove
    FavPress { row: crate::favorites::Row, y: f32 },
    FavMove { from: crate::favorites::Row, y: f32 },
    /// 고정한 패널의 오른쪽 가장자리를 끌어 너비를 바꾸는 중
    SideW { start_x: f32, start_w: f32 },
    /// 탭을 눌렀다. 조금 움직이면 TabMove
    TabPress { pane: usize, tab: usize, x: f32, y: f32 },
    /// 탭을 끄는 중. (x, y)는 지금 포인터 위치
    TabMove { pane: usize, tab: usize, x: f32, y: f32 },
    /// 빈 곳이나 자세히 보기의 이름 칸 밖을 눌렀다. 조금 움직이면 Marquee. additive는 Ctrl
    MarqueeArm { pane: usize, x: f32, y: f32, additive: bool },
    /// 네모 상자로 고르는 중. (x0, y0)는 시작점(스크롤 방향 좌표는 스크롤 0 기준), (x, y)는 지금 포인터
    Marquee { pane: usize, x0: f32, y0: f32, x: f32, y: f32 },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum EditMode {
    None,
    Address(usize),
    Rename(usize),
    /// 즐겨찾기 표시 이름 바꾸기
    FavRename(crate::favorites::Row),
    /// 작업 줄의 작업 공간 이름
    WorkspaceRename,
}

struct RenameCtx {
    pane: usize,
    tab: u64,
    path: String,
    old: String,
    /// 폴더면 확장자 경고를 하지 않는다
    dir: bool,
}

/// 진행 중인 잘라내기 붙여넣기. 작업마다 번호를 매겨, 먼저 끝난 작업이 다른 작업의 표시까지 지우지 않게 한다
#[derive(Default)]
struct Moving {
    next: u64,
    /// (작업 번호, 옮기는 파일)
    ops: Vec<(u64, Vec<String>)>,
}

impl Moving {
    /// paths 가운데 하나라도 이미 옮기는 중인가
    fn busy(&self, paths: &[String]) -> bool {
        self.ops.iter().any(|(_, moving)| moving.iter().any(|m| paths.iter().any(|p| util::eq_path(m, p))))
    }

    fn begin(&mut self, paths: &[String]) -> u64 {
        self.next += 1;
        self.ops.push((self.next, paths.to_vec()));
        self.next
    }

    fn end(&mut self, id: u64) {
        self.ops.retain(|(i, _)| *i != id);
    }
}

/// 화면 설정에서 바뀐 부분 (apply_look이 그만큼만 다시 만든다)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LookPart {
    /// 테마·강조 색
    Colors,
    Font,
    Zoom,
    /// 글자 표시 방식
    Text,
}

pub enum Deferred {
    Close,
    Open { pane: usize, paths: Vec<String>, dir: Option<String> },
    Menu { pane: usize, paths: Vec<String>, dir: Option<String>, pt: POINT },
    PrepareMenu { paths: Vec<String>, dir: Option<String> },
    FavMenu { row: crate::favorites::Row, pt: POINT },
    FavSortMenu { pt: POINT },
    /// 사이드바의 내 PC·드라이브·검색 폴더 줄 메뉴
    SideMenu { side: Side, pt: POINT },
    DragOut(Vec<String>),
    Verb { paths: Vec<String>, verb: &'static str },
    Paste { pane: usize, dest: String },
    /// 트레이 대기 중에 창을 닫으면 끝내지 않고 숨긴다
    HideToTray,
    /// 미리 보기 처리기를 지금 상태에 맞춘다 (열기·닫기·크기)
    PreviewSync,
    /// 설정 창에서 메뉴·확인 창·다른 프로그램을 띄우는 조작
    Settings { ctl: settings_ui::Ctl, index: usize, pt: POINT },
    /// 검색 결과 오른쪽 버튼 메뉴
    PalMenu { pt: POINT },
    /// 작업 줄 `…` 메뉴 (도움말·설정·정보)
    MoreMenu { pt: POINT },
    /// 새 탭 시작 화면 항목의 메뉴. 위치는 메뉴를 열 때 정해 둔다
    StartMenu { pane: usize, item: start::StartItem, loc: Location, pt: POINT },
    /// 작업 공간 메뉴 (저장한 작업 공간·새로 저장·이름 바꾸기·지우기)
    WorkspaceMenu { pt: POINT },
    /// 네트워크 공유에 Windows 로그인 창으로 연결한 뒤 그 탭을 다시 읽는다
    NetFix { tab: u64, share: String, ask: bool },
    /// 확장자를 바꾸는 이름 바꾸기: 묻고 나서 바꾸거나, 친 이름 그대로 상자를 다시 연다
    ConfirmRename { tab: u64, path: String, old: String, new: String },
    /// 파일을 많이 고르고 열 때: 묻고 나서 연다 (작업 폴더는 파일마다 그 폴더)
    ConfirmOpen { pane: usize, paths: Vec<String> },
}

/// 끌어 놓을 곳 (drop_dir_at)
pub enum DropAt {
    None,
    Dir(String),
    /// 사이드바 즐겨찾기: 놓으면 즐겨찾기에 넣는다
    Favorites,
}

pub struct Listed {
    tab: u64,
    generation: u64,
    result: Result<Vec<Entry>, String>,
    /// 열지 못한 폴더가 지워졌거나 옮겨진 것 (그 위 폴더는 있다). 네트워크가 끊겼거나 USB를 뽑아 잠깐 못 연 것과 가른다
    gone: bool,
    /// 네트워크 폴더를 열지 못한 까닭에 맞는 고칠 방법
    net: Option<crate::listing::NetFix>,
}

pub struct IconsReady {
    tab: u64,
    generation: u64,
    icons: Vec<(u32, i32)>,
}

pub struct App {
    hwnd: HWND,
    gfx: Gfx,
    theme: Theme,
    panes: Vec<Pane>,
    layout: Layout,
    split: (f32, f32),
    active: usize,
    show_hidden: bool,
    w: f32,
    h: f32,
    hover: Hit,
    drag: Drag,
    tracking: bool,
    /// 창 조절 버튼 위에서 비클라이언트 마우스 떠남을 기다리는 중
    nc_tracking: bool,
    /// 누르고 있는 창 조절 버튼 (떼면 실행)
    caption_pressed: Option<caption::CaptionButton>,
    /// 창이 활성인가 (아니면 창 조절 버튼을 흐리게)
    window_active: bool,
    filter_edit: HWND,
    rename_edit: HWND,
    addr_edit: HWND,
    edit_mode: EditMode,
    rename: Option<RenameCtx>,
    edit_font: HFONT,
    pub pending: Vec<Deferred>,
    watches: [Option<(u32, String)>; 4],
    drop_hover: Option<(usize, Option<usize>)>,
    next_id: u64,
    types: HashMap<String, (String, i32)>,
    favs: crate::favorites::Favorites,
    /// 열린 보기 방식 메뉴
    view_menu: Option<view_menu::ViewMenu>,
    /// 키보드가 사이드바 즐겨찾기에 있는가 (Ctrl+B)
    fav_focus: bool,
    /// 사이드바 즐겨찾기에서 고른 줄 (favs.rows() 번호)
    fav_sel: usize,
    /// 보이는 패널 가운데 목록의 스크롤
    side_scroll: f32,
    /// 패널마다 기억해 둔 스크롤 (Panel 순서). 다른 패널이 떴다 닫혀도 자리를 잃지 않게
    side_scroll_memo: [f32; Panel::ALL.len()],
    /// 고정한 패널 (칸을 밀어낸다). 세션에 저장
    side_pinned: Option<Panel>,
    /// 아이콘에 머물러 칸 위에 떠 있는 패널 (마우스가 떠나거나 위치를 고르면 닫힌다)
    side_peek: Option<Panel>,
    /// 패널 너비 (DIP)
    side_w: f32,
    /// 머무르면 패널을 띄울 아이콘 (TIMER_BAR_HOVER를 기다리는 중)
    bar_arm: Option<Panel>,
    /// 떠 있는 패널 밖으로 나가 닫기(TIMER_SIDE_CLOSE)를 기다리는 중
    peek_leaving: bool,
    /// 패널·작업 줄에서 연 팝업 메뉴가 떠 있는 동안 (그 위로 간 마우스를 패널 밖으로 보지 않게)
    menu_open: bool,
    /// 파일을 끌어 사이드바에 놓으려는 자리 (즐겨찾기 쪽이면 Side::FavEmpty)
    side_drop: Option<Side>,
    /// 사이드바의 내 PC·드라이브 (뒤에서 읽어 온다)
    places: sidebar::Places,
    places_busy: bool,
    places_again: bool,
    /// (폴더, 디스크 검색 밖인가): 드라이브 종류는 폴더가 바뀔 때만 묻는다
    outside_cache: RefCell<(String, bool)>,
    /// 앞 누름이 팝업을 열거나 닫았다: 이어지는 두 번 클릭 누름은 무시한다
    swallow_dbl: bool,
    /// 필터 상자를 마지막으로 둔 자리 (픽셀 x, y, 너비, 높이)
    filter_placed: Cell<(i32, i32, i32, i32)>,
    /// 잠깐 강조할 즐겨찾기 줄 (이미 있는 것을 다시 넣으려 할 때. 키보드는 옮기지 않는다)
    fav_flash: Option<crate::favorites::Row>,
    search_edit: HWND,
    /// 명령 메뉴 (Ctrl+K)
    cmd_edit: HWND,
    cmd: Option<commands::CmdMenu>,
    cmd_at: Option<std::time::Instant>,
    /// 키보드가 검색 상자에 있는가 (검색 패널이 보일 때)
    pal_focus: bool,
    /// 검색 범위: 지금 칸의 폴더 아래만
    pal_here: bool,
    /// 검색 패널을 열기 전에 고정해 두었던 패널 (검색을 닫으면 돌아간다)
    side_before_search: Option<Panel>,
    pal_results: Vec<(bool, String)>,
    pal_sel: usize,
    /// 검색 결과 중 골라 둔 것 (Ctrl·Shift+클릭, Shift+↑↓). 결과와 같은 순서
    pal_picked: Vec<bool>,
    /// Shift로 범위를 고를 때의 시작 줄
    pal_anchor: usize,
    pal_scroll: f32,
    pal_status: PalStatus,
    /// 검색 결과 경로 → (수정한 때, 크기). 읽지 못했으면 None. 보이는 줄만 뒤에서 읽는다
    pal_meta: HashMap<String, Option<(u64, u64)>>,
    /// 지금 결과가 온 뒤에 읽은 경로 (여기 없는 보이는 줄은 다시 읽는다)
    pal_meta_fresh: HashSet<String>,
    pal_meta_busy: bool,
    /// 늦게 도착한 이전 검색 결과를 버리기 위한 순번
    pal_seq: u64,
    /// 보이는 결과가 지금 검색어의 것인지, 결과보다 먼저 누른 Enter
    pal_gate: PalGate,
    /// Ctrl+Z로 되돌릴 작업 (마지막이 가장 최근)
    undo: Vec<crate::undo::Undo>,
    /// 진행 중인 잘라내기 붙여넣기 (끝나기 전에 같은 파일을 또 붙여넣지 않게)
    moving: Moving,
    /// 실제로 등록된 전역 검색 단축키
    hotkey: Option<String>,
    /// 화면 설정 (테마·강조 색·글꼴·크기)
    look: session::Look,
    /// 바꿀 수 있는 단축키
    keys: crate::keys::Keymap,
    /// 요청했지만 아직 오지 않은 썸네일 키
    thumb_wanted: HashSet<String>,
    /// 셸이 그림을 주지 못한 키 (다시 요청하지 않는다)
    thumb_failed: HashSet<String>,
    /// 미리 보기 창. 닫혀 있으면 None
    preview: Option<Preview>,
    /// 미리 보기 처리기(PDF 등)가 그려 넣을 자식 창
    preview_host: HWND,
    /// 트레이 대기 중이면 알림 영역 아이콘. 이때 창을 닫으면 끝내지 않고 숨긴다
    tray: Option<crate::tray::TrayIcon>,
    /// 창을 숨긴 동안 폴더가 바뀐 패널. 다시 보일 때 한 번만 읽는다
    stale: [bool; 4],
    /// 네모 상자로 고르기 시작할 때의 선택 (보이는 순서)
    marquee_base: Vec<bool>,
    /// 새 탭을 열 위치 (세션 형식, session::NEW_TAB_SAME이면 지금 탭과 같은 폴더)
    new_tab_home: String,
    /// 시작 화면의 최근 위치 (최근 것이 앞, 세션에 저장)
    recent: Vec<String>,
    /// 시작 화면에서 키보드를 쓰기 시작했는지 (초점 테두리를 보일지)
    start_keys: bool,
    /// 이름 붙여 저장해 둔 화면들과 지금 작업 공간
    workspaces: crate::workspaces::Workspaces,
    /// 바로가기 패널에서 접어 둔 구역 (Fold 순서)
    folded: [bool; 2],
    /// 시작할 때 세션 대신 지금 작업 공간의 화면으로 열지 (세션이 다른 작업 공간·예전 빌드의 것이면)
    ws_restore: bool,
    /// 닫은 탭 (칸, 위치, 뒤로 기록, 앞으로 기록). 마지막 것부터 Ctrl+Shift+T로 다시 연다
    closed_tabs: Vec<(usize, Location, Vec<Location>, Vec<Location>)>,
    /// 뒤에서 확인 중인 경로 (번호, 칸, 경로)와 다음 번호
    probes: Vec<(u64, usize, String)>,
    probe_next: u64,
    /// 칸마다 요청 번호 (경로 입력·이름 바꾸기·경로 확인을 시작하면 오른다)
    probe_seq: [u64; 4],
    /// "여는 중…"으로 띄운 알림 (탭 번호, 글): 확인이 끝나면 이것만 지운다
    probe_shown: Option<(u64, String)>,
    /// 휠 한 칸의 줄 수 (시스템 설정, 바뀌면 다시 읽는다)
    wheel_lines: f32,
    /// 지금 처리하는 키가 누르고 있어 반복된 것인지 (Enter·Delete·Backspace는 반복을 무시한다)
    key_repeat: bool,
    /// 패널이 펼쳐지기 시작한 때 (움직이는 중일 때만)
    side_anim: Option<std::time::Instant>,
    /// 되돌리기 단추가 달린 알림: (칸, 알릴 때의 기록 번호). 그 뒤로 기록이 바뀌면 단추를 감춘다
    toast_undo: Option<(usize, u64)>,
    /// 되돌리기 기록이 바뀔 때마다(넣기·꺼내기) 하나씩 오른다
    undo_rev: u64,
    wheel: Option<Wheel>,
    /// 네트워크 폴더(소문자 경로) → 이번 실행에서 마지막으로 읽은 목록. 연결이 끊기면 흐리게 보여 준다
    net_cache: HashMap<String, Vec<Entry>>,
    /// 페이드가 시작된 때: 말풍선·정보 카드, 알림, 보기 메뉴
    tip_at: Option<std::time::Instant>,
    toast_at: Option<std::time::Instant>,
    view_menu_at: Option<std::time::Instant>,
    /// 패널 머리의 작은 단추를 마지막으로 그렸는지. 그려지지 않은 단추는 누르지 않는다 (터치로 탭하면 보이기 전에 눌리므로)
    side_minis_shown: bool,
    /// 단추 말풍선이 떠 있는지
    tip_shown: bool,
    /// 설정 창 (열려 있으면 창 전체를 덮는다)
    settings: Option<settings_ui::Settings>,
    /// Ctrl+F 검색에 넣은 폴더(공유폴더 등 디스크 인덱스 밖)
    folders: crate::folders::Folders,
    /// 검색 패널을 열 때 보이는 칸에 열려 있던, 아직 검색에 넣지 않은 공유폴더·USB 폴더 (결과가 없으면 넣기 버튼)
    pal_outside: Option<String>,
}

// ---------- 기하 (공통 코드 sakadu::ui) ----------

/// 말풍선 "이름 (단축키)"를 이름과 단축키로. 경로(‘\’가 든 글)나 단축키가 아닌 괄호는 그대로 둔다
fn split_tip_key(text: &str) -> (&str, Option<&str>) {
    if text.contains('\\') || !text.ends_with(')') {
        return (text, None);
    }
    let Some(i) = text.rfind(" (") else { return (text, None) };
    let key = &text[i + 2..text.len() - 1];
    let f_key = key.strip_prefix('F').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    if key.contains('+') || f_key || matches!(key, "Del" | "Esc" | "Enter") { (&text[..i], Some(key)) } else { (text, None) }
}



fn type_info(types: &mut HashMap<String, (String, i32)>, e: &Entry) -> (String, i32) {
    let key = if e.is_dir() { "\\dir".to_string() } else { e.ext_lower() };
    types.entry(key).or_insert_with_key(|k| shell::type_info(k, e.is_dir())).clone()
}

fn loword(v: usize) -> u16 {
    (v & 0xFFFF) as u16
}

fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

fn edit_text(h: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(h).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let n = GetWindowTextW(h, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

fn expand_env(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let mut buf = vec![0u16; 4096];
    let n = unsafe { ExpandEnvironmentStringsW(&HSTRING::from(s), Some(&mut buf)) } as usize;
    if n == 0 || n > buf.len() { s.to_string() } else { util::from_wide(&buf) }
}

/// 주소창 입력 해석: 경로, 환경 변수, "shell:" 이름, 파일 경로(→ 상위 폴더에서 선택)
fn resolve_input(text: &str) -> Result<(Location, Option<String>), String> {
    let t = text.trim();
    if t.is_empty() || t == "::computer" || t.eq_ignore_ascii_case("this pc") || t.eq_ignore_ascii_case("내 pc") {
        return Ok((Location::Computer, None));
    }
    let path = util::normalize(&expand_env(t));
    if shell::is_dir(&path) {
        return Ok((Location::Dir(path), None));
    }
    if shell::exists(&path) {
        if let Some(parent) = util::parent(&path) {
            return Ok((Location::Dir(parent), Some(util::file_name(&path).to_string())));
        }
    }
    if let Some(p) = shell::resolve_shell_path(t) {
        if shell::is_dir(&p) {
            return Ok((Location::Dir(util::normalize(&p)), None));
        }
    }
    Err(format!("경로를 찾을 수 없어요: {t}"))
}

/// 뒤 스레드에서 경로를 확인한 뒤 할 일. 꺼진 NAS를 확인하는 동안(수십 초) 창이 멈추지 않게 한다
enum ProbeThen {
    /// 경로 입력: 기다리는 동안 그 탭이 다른 곳으로 옮기지 않았으면 연다
    Address { tab: u64, from: Location },
    /// 다른 앱에서 넘어온 경로: 새 탭에서 연다 (지금 탭이 이미 그 폴더면 그대로)
    External,
    /// 즐겨찾기: 폴더는 열고 파일은 연다 (새 탭이면 그 폴더를 열어 파일을 고른다).
    /// is_dir: 즐겨찾기·최근 위치가 아는 종류 (모르면 None, 폴더로 본다). 확인하지 못한 네트워크 경로를 열 때 쓴다
    Favorite { tab: u64, from: Location, new_tab: bool, is_dir: Option<bool> },
}

pub struct Probed {
    /// 기다리는 목록(App::probes)의 번호
    id: u64,
    /// 그 칸의 요청 번호. 그사이 칸에서 경로 입력·이름 바꾸기·다른 확인을 시작했으면 이 결과는 버린다
    seq: u64,
    pane: usize,
    then: ProbeThen,
    shown: String,
    result: Result<(Location, Option<String>), String>,
}

fn spawn_probe(hwnd: HWND, id: u64, seq: u64, pane: usize, text: String, then: ProbeThen) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let _com = shell::ComInit::sta();
        let result = resolve_input(&text);
        post(owner.get(), crate::WM_APP_PROBED, Box::new(Probed { id, seq, pane, then, shown: text, result }));
    });
}

/// 공유(\\서버\…)나 연결 드라이브의 경로로 보이는 입력
fn net_text(text: &str) -> bool {
    let s = text.trim();
    s.starts_with("\\\\") || (s.len() >= 2 && s.as_bytes()[1] == b':' && s.as_bytes()[0].is_ascii_alphabetic() && !is_local_path(s))
}

/// 네트워크 드라이브·공유가 아닌 경로 (감시 등록이 네트워크를 기다리지 않는다)
fn is_local_path(path: &str) -> bool {
    if path.starts_with("\\\\") {
        return false;
    }
    let root: String = path.chars().take(3).collect();
    const DRIVE_REMOTE: u32 = 4;
    unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(&HSTRING::from(root)) != DRIVE_REMOTE }
}

/// 경로 확인이 이보다 오래 걸리면 "여는 중…"을 알린다
const TIMER_PROBE: usize = 50;
/// 오래 걸리는 폴더 읽기: 목록 자리의 "불러오는 중…"을 띄우고 지운다
const TIMER_LOADING: usize = 51;
/// 폴더 읽기가 이보다 오래 걸리면 목록 자리에 기다린다는 글을 보인다
const LOADING_SHOW_MS: u128 = 300;
/// 잠시 뒤 한 번 다시 그린다 (빨리 끝나면 보이지 않을 '불러오는 중…'·흐림을 늦게 보일 때)
const TIMER_REDRAW: usize = 52;
/// 휠 스크롤을 목표 자리까지 부드럽게 옮긴다
const TIMER_WHEEL: usize = 53;
/// 페이드가 도는 동안 다시 그린다
const TIMER_ANIM: usize = 54;
/// 연결이 끊긴 네트워크 폴더를 10초마다 다시 읽어 본다
const TIMER_OFFLINE: usize = 55;
const TIP_FADE_MS: f32 = 140.0;
const TOAST_FADE_MS: f32 = 200.0;
const MENU_FADE_MS: f32 = 160.0;

/// 휠로 움직이는 중인 목록: 칸, 탭, 목표 스크롤, 마지막으로 놓은 스크롤 (다른 곳에서 스크롤을 바꾸면 멈춘다)
#[derive(Clone, Copy)]
struct Wheel {
    pane: usize,
    tab: u64,
    target: f32,
    last: f32,
}

/// 페이드 진행 (0→1, 빠르게 시작해 천천히 멈춘다). 시작하지 않았거나 끝났으면 None
fn anim_progress(at: Option<std::time::Instant>, ms: f32) -> Option<f32> {
    let p = at?.elapsed().as_secs_f32() * 1000.0 / ms;
    (p < 1.0).then(|| 1.0 - (1.0 - p).powi(3))
}

fn default_locations() -> [Location; 4] {
    let known = |id| shell::known_folder(id).map(Location::Dir).unwrap_or(Location::Computer);
    [known(&FOLDERID_Profile), Location::Computer, known(&FOLDERID_Downloads), known(&FOLDERID_Documents)]
}

fn post<T>(hwnd: HWND, msg: u32, payload: Box<T>) {
    let raw = Box::into_raw(payload);
    if unsafe { PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(raw as isize)) }.is_err() {
        drop(unsafe { Box::from_raw(raw) });
    }
}

/// 파일마다 아이콘이 다른 형식. 나머지는 확장자별 아이콘을 쓴다
fn needs_own_icon(e: &Entry) -> bool {
    e.kind == Kind::File && matches!(e.ext_lower().as_str(), "exe" | "lnk" | "ico" | "url" | "cur" | "ani" | "scr" | "cpl" | "msc" | "appref-ms")
}

/// 입력 상자에서 고른 글자 범위 (시작, 끝). 고른 게 없으면 둘이 같다(캐럿 위치)
fn edit_sel(edit: HWND) -> (u32, u32) {
    let (mut start, mut end) = (0u32, 0u32);
    unsafe {
        SendMessageW(edit, EM_GETSEL, Some(WPARAM(&mut start as *mut u32 as usize)), Some(LPARAM(&mut end as *mut u32 as isize)));
    }
    (start, end)
}

/// 이름의 확장자 (소문자). 점으로 시작하는 이름이나 확장자가 없으면 빈 문자열
fn ext_of_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_lowercase(),
        _ => String::new(),
    }
}

/// 이름 상자에 넣지 않는 문자
fn blocked_name_char(c: char) -> bool {
    matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

/// 휠 한 칸에 움직일 줄 수 (마우스 설정의 "한 번에 스크롤할 줄 수", 한 화면씩이면 10줄)
fn wheel_lines() -> f32 {
    let mut n = 3u32;
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, Some(&mut n as *mut u32 as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    match n {
        0 => 3.0,
        u32::MAX => 10.0,
        n => n.min(20) as f32,
    }
}

/// 다시 열 수 있게 기억하는 닫은 탭 수
const CLOSED_TABS_KEEP: usize = 20;

/// 한꺼번에 열 때 묻는 파일 수
const OPEN_ASK_OVER: usize = 15;

/// 백그라운드에서 폴더를 읽고 결과를 창 메시지로 보낸다. 개별 아이콘은 목록을 먼저 보낸 뒤 채운다
fn spawn_listing(hwnd: HWND, tab: u64, generation: u64, loc: Location) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let _com = shell::ComInit::sta();
        let result = match &loc {
            Location::Computer => Ok(list_computer()),
            // 시작 화면은 읽을 것이 없다 (즐겨찾기·최근 위치·드라이브는 앱이 이미 들고 있다)
            Location::Start => Ok(Vec::new()),
            Location::Dir(d) => list_dir(d),
        };
        let special: Vec<(u32, String)> = match (&result, &loc) {
            (Ok(entries), Location::Dir(d)) => entries
                .iter()
                .enumerate()
                .filter(|(_, e)| needs_own_icon(e))
                .map(|(i, e)| (i as u32, util::join(d, &e.name_string())))
                .collect(),
            _ => Vec::new(),
        };
        let net = match (&result, &loc) {
            (Err(_), Location::Dir(d)) if !is_local_path(d) => crate::listing::net_fix(d, crate::listing::last_list_error()),
            _ => None,
        };
        let gone = match (&result, &loc) {
            (Err(_), Location::Dir(d)) => !shell::exists(d) && util::parent(d).is_some_and(|p| shell::is_dir(&p)),
            _ => false,
        };
        post(owner.get(), WM_APP_LISTED, Box::new(Listed { tab, generation, result, gone, net }));
        if !special.is_empty() {
            let icons = special.into_iter().map(|(i, p)| (i, shell::icon_for_path(&p))).collect();
            post(owner.get(), WM_APP_ICONS, Box::new(IconsReady { tab, generation, icons }));
        }
    });
}

unsafe fn create_edit(parent: HWND, id: i32) -> HWND {
    unsafe {
        let h = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            PCWSTR::null(),
            WS_CHILD | WS_CLIPSIBLINGS | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as isize as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default();
        let _ = SetWindowSubclass(h, Some(edit_proc), id as usize, 0);
        h
    }
}

/// 입력 상자 글꼴. family가 비어 있으면 Segoe UI
fn make_edit_font(dpi: f32, family: &str) -> HFONT {
    let face = HSTRING::from(if family.is_empty() { "Segoe UI" } else { family });
    unsafe {
        CreateFontW(
            -((13.0 * dpi / 96.0).round() as i32),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            &face,
        )
    }
}

impl App {
    pub fn new(hwnd: HWND, sess: Option<session::Session>) -> windows::core::Result<App> {
        let look = sess.as_ref().map(|s| s.look.clone()).unwrap_or_default();
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 * look.zoom as f32 / 100.0;
        let mut gfx = Gfx::new(dpi, shell::system_imagelist(), &look.font)?;
        gfx.set_text_look(crate::gfx::TextLook::from_key(&look.text));
        let mut next_id = 1u64;
        let defaults = default_locations();
        let mut panes = Vec::with_capacity(4);
        for (i, default) in defaults.into_iter().enumerate() {
            let ps = sess.as_ref().and_then(|s| s.panes.get(i));
            let mut tabs: Vec<Tab> = Vec::new();
            for loc in ps.map(|p| p.tabs.iter().map(|t| Location::from_session(t)).collect()).unwrap_or_else(|| vec![default]) {
                tabs.push(Tab::new(next_id, loc));
                next_id += 1;
            }
            if tabs.is_empty() {
                tabs.push(Tab::new(next_id, Location::Computer));
                next_id += 1;
            }
            let active = ps.map(|p| p.active.min(tabs.len() - 1)).unwrap_or(0);
            let sort = ps.map(|p| Sort { key: SortKey::from_u8(p.sort_key), desc: p.sort_desc }).unwrap_or_default();
            let cols = ps.map(|p| p.cols.map(|c| c.clamp(60.0, 400.0))).unwrap_or(DEFAULT_COLS);
            let view = ps.map(|p| ViewMode::from_code(p.view)).unwrap_or_default();
            panes.push(Pane { tabs, active, sort, cols, view });
        }
        let layout = sess.as_ref().map(|s| Layout::ALL[(s.layout as usize).min(4)]).unwrap_or(Layout::TwoV);
        // 처음 쓰면(세션 없음) 드라이브가 보이는 바로가기 패널을 고정해 둔다. 예전 세션은 session::parse가 즐겨찾기로 둔다
        let (side_pinned, side_w) = match &sess {
            Some(s) => (Panel::from_key(&s.side), s.side_w),
            None => (Some(Panel::Places), session::SIDE_W_DEFAULT),
        };
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        let (filter_edit, rename_edit, addr_edit, search_edit) =
            unsafe { (create_edit(hwnd, ID_FILTER), create_edit(hwnd, ID_RENAME), create_edit(hwnd, ID_ADDR), create_edit(hwnd, ID_SEARCH)) };
        let cmd_edit = unsafe { create_edit(hwnd, ID_CMD) };
        let preview_host = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                PCWSTR::null(),
                WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(hwnd),
                None,
                None,
                None,
            )
            .unwrap_or_default()
        };
        let mut app = App {
            hwnd,
            gfx,
            theme: crate::theme::resolve(&look.theme, &look.accent),
            panes,
            layout,
            split: sess.as_ref().map(|s| (s.split.0.clamp(0.15, 0.85), s.split.1.clamp(0.15, 0.85))).unwrap_or((0.5, 0.5)),
            active: sess.as_ref().map(|s| s.active.min(layout.count() - 1)).unwrap_or(0),
            show_hidden: sess.as_ref().is_some_and(|s| s.show_hidden),
            w: rc.right as f32 * 96.0 / dpi,
            h: rc.bottom as f32 * 96.0 / dpi,
            hover: Hit::None,
            drag: Drag::None,
            tracking: false,
            nc_tracking: false,
            caption_pressed: None,
            window_active: true,
            filter_edit,
            rename_edit,
            addr_edit,
            edit_mode: EditMode::None,
            rename: None,
            edit_font: HFONT::default(),
            pending: Vec::new(),
            watches: Default::default(),
            drop_hover: None,
            next_id,
            types: HashMap::new(),
            favs: crate::favorites::load(),
            view_menu: None,
            fav_focus: false,
            fav_sel: 0,
            side_scroll: 0.0,
            side_scroll_memo: [0.0; Panel::ALL.len()],
            side_pinned,
            side_peek: None,
            side_w,
            bar_arm: None,
            peek_leaving: false,
            menu_open: false,
            side_drop: None,
            places: sidebar::Places::default(),
            places_busy: false,
            places_again: false,
            outside_cache: RefCell::new((String::new(), false)),
            swallow_dbl: false,
            filter_placed: Cell::new((0, 0, 0, 0)),
            fav_flash: None,
            search_edit,
            cmd_edit,
            cmd: None,
            cmd_at: None,
            pal_focus: false,
            pal_here: false,
            side_before_search: None,
            pal_results: Vec::new(),
            pal_sel: 0,
            pal_picked: Vec::new(),
            pal_anchor: 0,
            pal_scroll: 0.0,
            pal_status: PalStatus::Idle,
            pal_meta: HashMap::new(),
            pal_meta_fresh: HashSet::new(),
            pal_meta_busy: false,
            pal_seq: 0,
            pal_gate: PalGate::default(),
            undo: Vec::new(),
            moving: Moving::default(),
            hotkey: None,
            look,
            keys: crate::keys::Keymap::load(),
            thumb_wanted: HashSet::new(),
            thumb_failed: HashSet::new(),
            preview: sess.as_ref().is_some_and(|s| s.preview).then(Preview::new),
            preview_host,
            tray: None,
            stale: [false; 4],
            marquee_base: Vec::new(),
            new_tab_home: sess.as_ref().map_or_else(|| session::NEW_TAB_START.to_string(), |s| s.new_tab.clone()),
            recent: sess.as_ref().map(|s| s.recent.clone()).unwrap_or_default(),
            start_keys: false,
            workspaces: crate::workspaces::load(),
            folded: Fold::ALL.map(|f| sess.as_ref().is_some_and(|s| s.folded.iter().any(|k| k == f.key()))),
            side_anim: None,
            toast_undo: None,
            undo_rev: 0,
            wheel: None,
            net_cache: HashMap::new(),
            tip_at: None,
            toast_at: None,
            view_menu_at: None,
            side_minis_shown: false,
            ws_restore: false,
            closed_tabs: Vec::new(),
            probes: Vec::new(),
            probe_next: 0,
            probe_seq: [0; 4],
            probe_shown: None,
            wheel_lines: wheel_lines(),
            key_repeat: false,
            tip_shown: false,
            settings: None,
            folders: crate::folders::Folders::start(),
            pal_outside: None,
        };
        unsafe {
            let cue = HSTRING::from("입력하면 바로 필터");
            SendMessageW(filter_edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
            let cue = HSTRING::from("파일·폴더 이름");
            SendMessageW(search_edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
            let cue = HSTRING::from("폴더·탭·명령 이름");
            SendMessageW(cmd_edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
        }
        app.apply_edit_font();
        app.apply_window_theme();
        app.refresh_places();
        // 지난번 검색어·범위와 미리 보기에 고정해 둔 파일
        if let Some(s) = &sess {
            app.set_search_text(&s.query);
            app.pal_here = s.scope_here;
            app.preview_restore_pin(s.pin.as_deref(), s.pin_id);
        }
        app.ws_restore = app.workspaces.current.is_some() && sess.as_ref().and_then(|s| s.ws.as_deref()) != app.workspaces.current_name();
        Ok(app)
    }

    /// 메인 창을 표시한 뒤에 할 초기화: 자동 완성, 드롭 대상, 첫 목록 불러오기, 변경 알림
    pub fn init(&mut self) {
        self.restore_workspace_if_needed();
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_DATES, 60_000, None);
        }
        unsafe {
            let _ = SHAutoComplete(self.addr_edit, SHACF_FILESYS_DIRS | SHACF_AUTOSUGGEST_FORCE_ON | SHACF_AUTOAPPEND_FORCE_OFF);
            let target: IDropTarget = crate::droptarget::DropTarget::new(self.hwnd).into();
            let r = RegisterDragDrop(self.hwnd, &target);
            crate::diag::log(|| format!("RegisterDragDrop -> {r:?}, elevated={}", IsUserAnAdmin().as_bool()));
            shell::warm_up_menus();
        }
        // 검색 서비스에 상태를 한 번 물어, 이 계정이 볼 수 있는 폴더 판정을 미리 채우게 한다 (로그인 직후 첫 검색이 느리지 않게)
        std::thread::spawn(|| {
            let _ = crate::service::status();
        });
        self.register_global_hotkey();
        for p in 0..self.layout.count() {
            self.load(p, false);
        }
        self.sync_watches();
        let filter = self.panes[self.active].tab().filter.clone();
        self.set_filter_edit_text(&filter);
        self.layout_edits();
        self.update_title();
        // 지난번에 검색 패널을 열어 둔 채 끝냈으면 서비스 상태부터 확인하고, 남은 검색어로 다시 찾는다
        self.pal_check_status();
        if self.pal_shown() && !edit_text(self.search_edit).trim().is_empty() {
            self.pal_run_query();
        }
    }

    /// 어디서든 검색(전역 단축키)을 등록한다. 설정에서 정한 키가 없으면 비어 있는 첫 후보를 쓴다.
    /// 정한 키를 다른 프로그램이 쓰고 있으면 false
    pub(super) fn register_global_hotkey(&mut self) -> bool {
        // 시험용으로 따로 띄운 창은 사용자가 쓰는 Sakadu의 단축키를 가져가지 않는다
        if crate::isolated() {
            self.hotkey = None;
            return false;
        }
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_SEARCH);
            let mods = |c: crate::keys::Combo| {
                let mut m = MOD_NOREPEAT;
                if c.ctrl {
                    m |= MOD_CONTROL;
                }
                if c.shift {
                    m |= MOD_SHIFT;
                }
                if c.alt {
                    m |= MOD_ALT;
                }
                m
            };
            self.hotkey = match self.keys.global {
                Some(c) => RegisterHotKey(Some(self.hwnd), HOTKEY_SEARCH, mods(c), c.vk as u32).is_ok().then(|| c.label()),
                None => {
                    let candidates = [
                        (MOD_CONTROL | MOD_ALT, VK_SPACE.0 as u32, "Ctrl+Alt+Space"),
                        (MOD_WIN | MOD_SHIFT, b'F' as u32, "Win+Shift+F"),
                        (MOD_CONTROL | MOD_ALT, b'F' as u32, "Ctrl+Alt+F"),
                    ];
                    // Sakadu 안 단축키로 정해 둔 키는 고르지 않는다 (어디서든 키가 먼저 가로채 그 동작을 키로 부를 수 없게 된다)
                    let taken = |m: HOT_KEY_MODIFIERS, vk: u32| {
                        let c = crate::keys::Combo { ctrl: m.contains(MOD_CONTROL), shift: m.contains(MOD_SHIFT), alt: m.contains(MOD_ALT), vk: vk as u16 };
                        !m.contains(MOD_WIN) && self.keys.find(c).is_some()
                    };
                    candidates
                        .iter()
                        .filter(|(m, vk, _)| !taken(*m, *vk))
                        .find(|(m, vk, _)| RegisterHotKey(Some(self.hwnd), HOTKEY_SEARCH, *m | MOD_NOREPEAT, *vk).is_ok())
                        .map(|c| c.2.to_string())
                }
            };
        }
        crate::diag::log(|| format!("hotkey -> {:?}", self.hotkey));
        let tip = self.tray_tip();
        if let Some(t) = &mut self.tray {
            t.update(false, &tip);
        }
        self.hotkey.is_some()
    }

    pub fn focus(&self) {
        unsafe {
            // 비활성 창에 SetFocus를 하면 창이 활성화돼 다른 앱에서 포커스를 빼앗는다.
            // 나중에 창이 활성화되면 WM_SETFOCUS에서 다시 불린다
            if GetActiveWindow() != self.hwnd {
                return;
            }
            let _ = SetFocus(Some(self.focus_target()));
        }
    }

    fn focus_target(&self) -> HWND {
        // 설정 창에서는 글자가 필터 상자로 들어가지 않게 메인 창이 키를 받는다
        if self.settings.is_some() {
            return self.hwnd;
        }
        if self.cmd.is_some() {
            return self.cmd_edit;
        }
        match self.edit_mode {
            EditMode::Address(_) => self.addr_edit,
            EditMode::Rename(_) | EditMode::FavRename(_) | EditMode::WorkspaceRename => self.rename_edit,
            EditMode::None if self.pal_focus && self.pal_shown() => self.search_edit,
            // 즐겨찾기에 키보드가 있으면 메인 창이 키를 받는다 (필터 상자에 커서가 깜박이지 않게)
            EditMode::None if self.fav_focus => self.hwnd,
            EditMode::None => self.filter_edit,
        }
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn next_tab_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn scale(&self) -> f32 {
        self.gfx.dpi / 96.0
    }

    fn dip(&self, lp: LPARAM) -> (f32, f32) {
        let x = loword(lp.0 as usize) as i16 as f32;
        let y = hiword(lp.0 as usize) as i16 as f32;
        (x / self.scale(), y / self.scale())
    }

    fn to_screen(&self, x: f32, y: f32) -> POINT {
        let mut pt = POINT { x: (x * self.scale()) as i32, y: (y * self.scale()) as i32 };
        unsafe {
            let _ = ClientToScreen(self.hwnd, &mut pt);
        }
        pt
    }

    // ---------- 레이아웃 ----------

    /// 칸이 놓이는 곳: 아이콘 막대(고정한 패널이 있으면 그 패널) 오른쪽, 작업 줄 아래 (미리 보기가 열려 있으면 그만큼 비운다)
    fn area(&self) -> R {
        let l = self.panes_left();
        let right = self.w - MARGIN - if self.preview.is_some() { self.preview_w() + GAP } else { 0.0 };
        R::new(l, WORKBAR_H, right.max(l + 100.0), (self.h - MARGIN).max(WORKBAR_H + 100.0))
    }

    /// 칸 나누기 버튼 5개를 한 덩어리로 담는 판. 작업 줄에서 칸과 같은 왼쪽 선
    fn layout_pill(&self) -> R {
        let l = self.workspace_button().r + 8.0;
        R::new(l, 6.0, l + 6.0 + 5.0 * 32.0, 38.0)
    }

    fn layout_buttons(&self) -> [R; 5] {
        let l0 = self.layout_pill().l + 3.0;
        std::array::from_fn(|i| {
            let l = l0 + i as f32 * 32.0;
            R::new(l, 9.0, l + 32.0, 35.0)
        })
    }

    /// 작업 줄 오른쪽: 창 단추 바로 앞에 미리 보기, 그 앞에 숨김 항목, 그 앞에 `…`
    fn preview_button(&self) -> R {
        let r = self.caption_left() - 8.0;
        R::new(r - 32.0, 6.0, r, 38.0)
    }

    fn hidden_button(&self) -> R {
        let p = self.preview_button();
        R::new(p.l - 36.0, 6.0, p.l - 4.0, 38.0)
    }

    fn more_button(&self) -> R {
        let h = self.hidden_button();
        R::new(h.l - 36.0, 6.0, h.l - 4.0, 38.0)
    }

    fn pane_rects(&self) -> Vec<(usize, R)> {
        pane_rects(self.area(), self.split, self.layout)
    }

    fn pane_rect(&self, p: usize) -> Option<R> {
        self.pane_rects().into_iter().find(|(i, _)| *i == p).map(|(_, r)| r)
    }

    fn splitters(&self) -> (Option<R>, Option<R>) {
        splitters(self.area(), self.split, self.layout)
    }

    fn list_rect(&self, p: usize) -> Option<R> {
        self.pane_rect(p).map(|r| geom(r).list)
    }

    /// 칸 p의 칩 줄 오른쪽 글자: 불러오는 중이거나, 항목 수와 고른 개수·크기
    fn head_count_text(&self, p: usize) -> String {
        let tab = self.panes[p].tab();
        if tab.loc == Location::Start {
            return String::new();
        }
        match &tab.state {
            LoadState::Loading => "불러오는 중…".to_string(),
            LoadState::Error(_) => "열 수 없음".to_string(),
            LoadState::Ready if tab.offline => "연결 끊김 · 마지막으로 본 목록".to_string(),
            LoadState::Ready if tab.loading_since.is_some_and(|s| s.elapsed().as_millis() >= LOADING_SHOW_MS * 2) => "새로 고치는 중…".to_string(),
            LoadState::Ready => {
                let (sel, bytes) = tab.selection_stats();
                let n = tab.view.len();
                match (sel, bytes) {
                    (0, _) => format!("{}개", util::grouped(n as u64)),
                    (s, 0) => format!("{}개 중 {}개 선택", util::grouped(n as u64), util::grouped(s as u64)),
                    (s, b) => format!("{}개 중 {}개 선택 · {}", util::grouped(n as u64), util::grouped(s as u64), format_size(b)),
                }
            }
        }
    }

    /// 칩 줄 오른쪽 자리의 폭: 필터 칩, 아니면 항목 수 글자 (너무 길면 칩에 자리를 남기고 줄인다)
    fn head_right_w(&self, p: usize, g: &PaneGeom) -> f32 {
        if !self.panes[p].tab().filter.is_empty() {
            return filter_chip(g).w();
        }
        self.gfx.measure(&self.head_count_text(p), Font::Small).min(g.head.w() * 0.35)
    }

    /// 칩 줄 오른쪽 자리: 필터 칩, 아니면 항목 수 글자. 좁은 칸에서 탭 칩과 겹치면 이쪽이 위에 그려지고 먼저 눌린다
    fn head_right_rect(&self, p: usize, g: &PaneGeom) -> R {
        if !self.panes[p].tab().filter.is_empty() {
            return filter_chip(g);
        }
        let w = self.head_right_w(p, g);
        R::new(g.head.r - 8.0 - w, g.head.t + 8.0, g.head.r - 8.0, g.head.t + 36.0)
    }

    /// 칸 p의 탭 칩 자리와 새 탭(+) 자리
    fn chips(&self, p: usize, g: &PaneGeom) -> (Vec<R>, R) {
        let pane = &self.panes[p];
        let n = pane.tabs.len();
        let natural: Vec<f32> = pane
            .tabs
            .iter()
            .enumerate()
            .map(|(_, tab)| {
                // 지금 탭만 굵게 그리지만 폭은 모두 굵은 글자로 잰다: 탭을 누를 때마다 옆 탭이 밀리지 않게
                (self.gfx.measure(&tab.loc.title(), Font::Normal) + 22.0 + if n > 1 { 20.0 } else { 0.0 }).clamp(52.0, 180.0)
            })
            .collect();
        let strip = chip_strip(g, self.head_right_w(p, g));
        let trs = tab_rects(strip, &natural, pane.active);
        let add = add_rect(strip, &trs);
        (trs, add)
    }

    /// 필터 칩 안의 개수: 고른 게 있으면 "고른/전체개"
    fn filter_count_text(&self, p: usize) -> String {
        if let Some(n) = self.start_count(p) {
            return format!("{n}개");
        }
        let tab = self.panes[p].tab();
        match tab.selection_stats().0 {
            0 => format!("{}개", util::grouped(tab.view.len() as u64)),
            s => format!("{}/{}개", util::grouped(s as u64), util::grouped(tab.view.len() as u64)),
        }
    }

    /// 필터 칩 안: (입력 상자 자리, 개수 글자 자리, × 단추).
    /// 개수 자리는 "0,000개"만큼은 늘 잡아 둬, 개수가 바뀌어도 입력 상자(자식 창)가 개수를 덮지 않게 한다
    fn filter_parts(&self, p: usize, chip: R) -> (R, R, R) {
        let n = self.filter_count_text(p);
        let nw = self.gfx.measure(&n, Font::Small).max(self.gfx.measure("0,000개", Font::Small));
        let clear = R::new(chip.r - 24.0, chip.t + 4.0, chip.r - 4.0, chip.b - 4.0);
        let count = R::new(clear.l - 4.0 - nw, chip.t, clear.l - 4.0, chip.b);
        let edit = R::new(chip.l + 28.0, chip.t, (count.l - 6.0).max(chip.l + 60.0), chip.b);
        (edit, count, clear)
    }

    fn hit(&self, x: f32, y: f32) -> Hit {
        if self.settings.is_some() {
            return Hit::Set(self.settings_hit(x, y));
        }
        // 명령 메뉴는 모든 것 위에 떠 있다 (밖을 누르면 닫힌다)
        if let Some(h) = self.cmd_hit(x, y) {
            return h;
        }
        if x < BAR_W {
            return self.bar_hit(x, y);
        }
        if let Some(h) = self.view_menu_hit(x, y) {
            return h;
        }
        for (p, _) in self.pane_rects() {
            if let Some((pill, btn)) = self.toast_rects(p) {
                if btn.is_some_and(|b| b.contains(x, y)) {
                    return Hit::ToastUndo(p);
                }
                if pill.contains(x, y) {
                    return Hit::Toast(p);
                }
            }
        }
        // 패널은 칸보다 위에 있다 (떠 있을 때 그 아래 칸을 누르지 않게)
        if self.side_panel_rect().is_some_and(|r| r.contains(x, y)) {
            return self.sidebar_hit(x, y);
        }
        if self.side_resize_rect().is_some_and(|r| r.contains(x, y)) {
            return Hit::SideResize;
        }
        for (p, _) in self.pane_rects() {
            if self.net_button(p).is_some_and(|(b, _)| b.contains(x, y)) {
                return Hit::NetFix(p);
            }
        }
        if y < WORKBAR_H {
            if self.workspace_button().contains(x, y) {
                return Hit::Workspace;
            }
            if let Some(i) = self.layout_buttons().iter().position(|r| r.contains(x, y)) {
                return Hit::Layout(i);
            }
            if self.hidden_button().contains(x, y) {
                return Hit::Hidden;
            }
            if self.preview_button().contains(x, y) {
                return Hit::PreviewButton;
            }
            if self.more_button().contains(x, y) {
                return Hit::More;
            }
            return Hit::None;
        }
        if let Some(h) = self.preview_hit(x, y) {
            return h;
        }
        let (sx, sy) = self.splitters();
        if sx.is_some_and(|r| r.contains(x, y)) {
            return Hit::SplitX;
        }
        if sy.is_some_and(|r| r.contains(x, y)) {
            return Hit::SplitY;
        }
        for (p, r) in self.pane_rects() {
            if !r.contains(x, y) {
                continue;
            }
            let g = geom(r);
            let pane = &self.panes[p];
            let tab = pane.tab();
            if g.head.contains(x, y) {
                // 오른쪽 자리(필터 칩·개수)는 탭 칩 위에 그려지므로 먼저 본다
                let right = self.head_right_rect(p, &g);
                if !tab.filter.is_empty() && right.contains(x, y) {
                    if self.filter_parts(p, right).2.contains(x, y) {
                        return Hit::FilterClear(p);
                    }
                    return Hit::Pane(p);
                }
                if x >= right.l - 4.0 {
                    return Hit::TabStrip(p);
                }
                let (trs, add) = self.chips(p, &g);
                for (i, tr) in trs.iter().enumerate() {
                    if tr.contains(x, y) {
                        if pane.tabs.len() > 1 && close_rect(*tr).contains(x, y) {
                            return Hit::TabClose(p, i);
                        }
                        return Hit::Tab(p, i);
                    }
                }
                if add.contains(x, y) {
                    return Hit::TabAdd(p);
                }
                return Hit::TabStrip(p);
            }
            if g.back.contains(x, y) {
                return Hit::Back(p);
            }
            if g.fwd.contains(x, y) {
                return Hit::Fwd(p);
            }
            if g.up.contains(x, y) {
                return Hit::Up(p);
            }
            // 시작 화면에는 보기·새 폴더 단추가 없다
            let start = tab.loc == Location::Start;
            if !start && g.view.contains(x, y) {
                return Hit::View(p);
            }
            if !start && g.new_folder.contains(x, y) {
                return Hit::NewFolder(p);
            }
            if tab.loc.dir().is_some() && self.edit_mode != EditMode::Address(p) && g.star.contains(x, y) {
                return Hit::Star(p);
            }
            if g.crumb.contains(x, y) {
                let cs = crumbs(&self.gfx, g.crumbs, &tab.loc, Some("내 PC"));
                return match cs.iter().position(|c| x >= c.x0 && x < c.x1) {
                    Some(i) => Hit::Crumb(p, i),
                    None => Hit::CrumbEmpty(p),
                };
            }
            if start {
                return self.start_hit(p, x, y);
            }
            if g.header.contains(x, y) {
                if pane.view != ViewMode::Details {
                    return match sort_chips(&self.gfx, g.header, pane.sort.key).into_iter().find(|(_, _, r)| r.contains(x, y)) {
                        Some((key, _, _)) => Hit::Header(p, key),
                        None => Hit::Pane(p),
                    };
                }
                let cols = columns(g.list, pane.cols);
                for (ci, c) in cols.resizable().iter().enumerate() {
                    if let Some((l, _)) = c {
                        if (x - l).abs() <= 4.0 {
                            return Hit::HeaderResize(p, ci);
                        }
                    }
                }
                let key = if cols.size.is_some_and(|(l, _)| x >= l) {
                    SortKey::Size
                } else if cols.kind.is_some_and(|(l, _)| x >= l) {
                    SortKey::Type
                } else if cols.modified.is_some_and(|(l, _)| x >= l) {
                    SortKey::Modified
                } else {
                    SortKey::Name
                };
                return Hit::Header(p, key);
            }
            if g.list.contains(x, y) {
                let grid = Grid::of(g.list, pane.view);
                if let Some((track, thumb)) = grid.bar(g.list, tab.view.len(), tab.scroll) {
                    if if grid.horiz { y >= track.t } else { x >= track.l } {
                        return if thumb.contains(x, y) { Hit::Thumb(p) } else { Hit::Track(p) };
                    }
                }
                return match grid.at(x, y, tab.scroll, tab.view.len()) {
                    Some(vi) => Hit::Row(p, vi),
                    None => Hit::ListEmpty(p),
                };
            }
            return Hit::Pane(p);
        }
        Hit::None
    }

    // ---------- 목록 불러오기 ----------

    fn load(&mut self, p: usize, keep: bool) {
        let t = self.panes[p].active;
        self.load_at(p, t, keep);
    }

    /// keep이면 지금 목록을 보여 둔 채 새로 읽고, 선택·스크롤을 되살린다
    fn load_at(&mut self, p: usize, t: usize, keep: bool) {
        let tab = &mut self.panes[p].tabs[t];
        tab.generation += 1;
        if keep {
            if tab.restore.is_none() {
                tab.restore = Some(tab.snapshot());
            }
            // 열지 못했던 폴더를 다시 읽을 때는 다시 "불러오는 중"으로 (오류 글이 그대로 남아 아무 일도 없는 것처럼 보이지 않게)
            if !matches!(tab.state, LoadState::Ready) {
                tab.state = LoadState::Loading;
            }
        } else {
            tab.state = LoadState::Loading;
        }
        if tab.loc != Location::Start {
            tab.loading_since = Some(std::time::Instant::now());
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_LOADING, LOADING_SHOW_MS as u32 + 20, None);
            }
        }
        spawn_listing(self.hwnd, tab.id, tab.generation, tab.loc.clone());
    }

    fn find_tab(&self, id: u64) -> Option<(usize, usize)> {
        self.panes.iter().enumerate().find_map(|(p, pane)| pane.tabs.iter().position(|t| t.id == id).map(|t| (p, t)))
    }

    fn on_listed(&mut self, l: Listed) {
        let Some((p, t)) = self.find_tab(l.tab) else { return };
        let (sort, hidden) = (self.panes[p].sort, self.show_hidden);
        let is_active = t == self.panes[p].active;
        let tab = &mut self.panes[p].tabs[t];
        if tab.generation != l.generation {
            return;
        }
        let waited = tab.loading_since.take().is_some();
        let ok = l.result.is_ok();
        let mut went_offline = false;
        let restore = match l.result {
            Ok(entries) => {
                if let Some(dir) = tab.loc.dir() {
                    self.folders.observe(dir, &entries);
                    // 네트워크 폴더는 마지막으로 본 목록을 기억해 둔다 (연결이 끊기면 흐리게 보여 준다)
                    if !is_local_path(dir) && entries.len() <= 20_000 {
                        if self.net_cache.len() >= 64 {
                            self.net_cache.clear();
                        }
                        self.net_cache.insert(dir.to_lowercase(), entries.clone());
                    }
                }
                tab.offline = false;
                tab.net_fix = None;
                tab.set_entries(entries, sort, hidden)
            }
            Err(msg) => {
                // 로그인이 필요하면 단추를 보이고, 닿지 못한 것이면 기억한 목록을 흐리게
                let cached = match (&l.net, tab.loc.dir()) {
                    (Some(f), _) if f.ask => None,
                    (_, Some(dir)) if !is_local_path(dir) => self.net_cache.get(&dir.to_lowercase()).cloned(),
                    _ => None,
                };
                tab.net_fix = l.net.clone();
                match cached {
                    Some(entries) => {
                        went_offline = !tab.offline && is_active;
                        tab.offline = true;
                        tab.set_entries(entries, sort, hidden)
                    }
                    None => {
                        tab.offline = false;
                        tab.entries.clear();
                        tab.view.clear();
                        tab.state = LoadState::Error(msg);
                        None
                    }
                }
            }
        };
        // 지워졌거나 옮겨진 폴더는 시작 화면의 최근 위치에서 뺀다 (잠깐 못 연 것은 그대로)
        if l.gone {
            if let Some(d) = self.panes[p].tabs[t].loc.dir().map(str::to_string) {
                self.forget_recent(&d);
            }
        }
        if self.panes[p].active == t {
            self.clamp_scroll(p);
            if p == self.active {
                self.layout_edits();
            }
            if let Some(r) = restore {
                if r.reveal {
                    self.ensure_visible(p);
                }
                // 고르려던 항목(새 폴더)을 실제로 찾았을 때만: 못 찾았는데 열면 엉뚱한 항목의 이름이 바뀐다
                let found = self.panes[p].tab().cursor_entry().map(|e| e.name_string().to_lowercase()) == r.cursor;
                if r.rename && p == self.active && found && self.edit_mode == EditMode::None {
                    self.begin_rename(p);
                }
            }
            // 이름을 바꾸는 중에 목록이 다시 오면(폴더 감시, 앞선 작업의 고르기) 상자를 바꾸던 항목에 그대로 둔다
            if self.edit_mode == EditMode::Rename(p) {
                if let Some(key) = self.rename.as_ref().filter(|c| c.tab == l.tab).map(|c| c.old.to_lowercase()) {
                    let tab = self.panes[p].tab_mut();
                    match tab.view.iter().position(|&i| tab.entries[i as usize].name_string().to_lowercase() == key) {
                        Some(vi) => {
                            tab.cursor = vi;
                            tab.anchor = vi;
                            let i = tab.view[vi] as usize;
                            tab.entries[i].selected = true;
                        }
                        None => {
                            self.cancel_edit();
                            self.flash(p, "바꾸던 항목이 다른 곳에서 지워졌거나 옮겨졌어요");
                        }
                    }
                }
            }
            if self.edit_mode == EditMode::Rename(p) {
                self.layout_edits();
            }
            // 다 읽은 폴더를 이제 감시한다 (읽기 전에는 꺼진 NAS를 기다리며 멈출 수 있어 미뤄 두었다)
            if ok {
                self.sync_watches();
            }
            if waited {
                self.refresh_cursor();
            }
        }
        if self.panes[p].tabs.get(t).is_some_and(|tab| tab.offline) {
            if went_offline {
                self.flash(p, "연결이 끊겨 마지막으로 본 목록을 보여 줘요 · 다시 연결되면 저절로 새로 고쳐요");
            }
            unsafe { SetTimer(Some(self.hwnd), TIMER_OFFLINE, 10_000, None) };
        }
        self.invalidate();
    }

    /// 네트워크 연결(로그인 창)이 끝났다: 되면 그 탭을 다시 읽고, 안 됐으면 까닭을 알린다 (취소는 조용히)
    pub(super) fn after_net_fix(&mut self, tab: u64, r: u32) {
        let Some((p, t)) = self.find_tab(tab) else { return };
        match r {
            0 => self.load_at(p, t, true),
            1223 => {}
            e => {
                let msg = windows::core::Error::from_hresult(windows::Win32::Foundation::WIN32_ERROR(e).to_hresult()).message();
                self.flash(p, format!("연결하지 못했어요 · {msg}"));
            }
        }
        self.invalidate();
    }

    /// 열지 못한 네트워크 폴더의 로그인·다시 연결 단추 (목록 자리 안내 글 아래)
    fn net_button(&self, p: usize) -> Option<(R, &'static str)> {
        let tab = self.panes[p].tab();
        let f = tab.net_fix.as_ref().filter(|_| matches!(tab.state, LoadState::Error(_)))?;
        let list = geom(self.pane_rect(p)?).list;
        let label = if f.ask { "로그인" } else { "다시 연결" };
        let w = self.gfx.measure(label, Font::Normal) + 32.0;
        Some((R::new(list.l + 24.0, list.t + 120.0, list.l + 24.0 + w, list.t + 152.0), label))
    }

    fn net_fix_click(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        if let Some(f) = tab.net_fix.clone() {
            self.pending.push(Deferred::NetFix { tab: tab.id, share: f.share, ask: f.ask });
        }
    }

    fn on_icons(&mut self, ic: IconsReady) {
        let Some((p, t)) = self.find_tab(ic.tab) else { return };
        let tab = &mut self.panes[p].tabs[t];
        if tab.generation != ic.generation {
            return;
        }
        for (i, icon) in ic.icons {
            if let Some(e) = tab.entries.get_mut(i as usize) {
                e.icon = icon;
            }
        }
        self.invalidate();
    }

    fn sync_watches(&mut self) {
        for p in 0..4 {
            // 다 읽은 폴더만 감시한다. 읽는 중이거나 열지 못한 폴더(꺼진 NAS)는 등록하는 동안 네트워크를 기다리며 창이 멈출 수 있다
            let want = (p < self.layout.count())
                .then(|| {
                    let tab = self.panes[p].tab();
                    tab.loc
                        .dir()
                        .filter(|d| matches!(tab.state, LoadState::Ready) || (matches!(tab.state, LoadState::Loading) && is_local_path(d)))
                        .map(String::from)
                })
                .flatten();
            if want.as_deref() == self.watches[p].as_ref().map(|(_, d)| d.as_str()) {
                continue;
            }
            if let Some((id, _)) = self.watches[p].take() {
                if id != 0 {
                    shell::unwatch(id);
                }
            }
            if let Some(d) = want {
                // 등록에 실패해도(id 0) 기억해 두어, 같은 폴더를 다른 칸·탭을 오갈 때마다 다시 시도하지 않는다 (F5면 다시)
                let id = shell::watch(self.hwnd, &d, WM_APP_NOTIFY + p as u32);
                self.watches[p] = Some((id, d));
            }
        }
    }

    // ---------- 이동 ----------

    pub fn navigate(&mut self, p: usize, loc: Location, select: Option<String>) {
        if matches!(self.edit_mode, EditMode::Rename(e) | EditMode::Address(e) if e == p) {
            self.cancel_edit();
        }
        let tab = self.panes[p].tab_mut();
        tab.nav_push(loc);
        tab.reset_view_state();
        tab.restore = select.map(|s| Restore::select(&s));
        self.load(p, false);
        self.after_location_change(p);
    }

    /// 바깥(폴더 열기·Win+E·명령줄)에서 넘어온 경로. 폴더는 활성 패널의 새 탭, 파일은 위치를 열고 선택.
    /// 경로가 없으면(Win+E) 창만 앞으로
    pub fn open_external(&mut self, path: Option<String>) {
        let Some(raw) = path else { return };
        // 설정 화면 뒤에 새 탭이 가려지지 않게 닫는다
        self.close_settings();
        self.fav_blur();
        let p = self.active;
        // 꺼진 NAS 경로여도 보낸 쪽(다른 Sakadu·탐색기 대신 연 창)을 기다리게 하지 않게, 확인은 뒤에서 한다
        self.probe(p, raw, ProbeThen::External);
    }

    fn after_location_change(&mut self, p: usize) {
        let loc = self.panes[p].tab().loc.clone();
        self.note_recent(&loc);
        self.start_keys = false;
        if p == self.active {
            self.set_filter_edit_text("");
            self.layout_edits();
            // "현재 폴더" 범위로 찾는 중이면 바뀐 폴더로 다시 찾는다
            self.pal_follow_scope();
        }
        self.sync_watches();
        self.update_title();
        self.invalidate();
    }

    fn go_back(&mut self, p: usize) {
        let tab = self.panes[p].tab_mut();
        let from = tab.loc.clone();
        if tab.nav_back() {
            let select = child_toward(&tab.loc, &from);
            tab.reset_view_state();
            tab.restore = select.map(|s| Restore::select(&s));
            self.load(p, false);
            self.after_location_change(p);
        }
    }

    fn go_forward(&mut self, p: usize) {
        let tab = self.panes[p].tab_mut();
        if tab.nav_forward() {
            tab.reset_view_state();
            self.load(p, false);
            self.after_location_change(p);
        }
    }

    fn go_up(&mut self, p: usize) {
        if let Location::Dir(d) = self.panes[p].tab().loc.clone() {
            match util::parent(&d) {
                Some(parent) => self.navigate(p, Location::Dir(parent), Some(util::file_name(&d).to_string())),
                None => self.navigate(p, Location::Computer, Some(d)),
            }
        }
    }

    fn refresh(&mut self, p: usize) {
        // 감시 등록에 실패했던 폴더는 F5로 다시 시도한다
        if self.watches[p].as_ref().is_some_and(|(id, _)| *id == 0) {
            self.watches[p] = None;
        }
        self.load(p, true);
    }

    fn open_items(&mut self, p: usize, new_tab: bool) {
        let tab = self.panes[p].tab();
        let Some(cur) = tab.cursor_entry() else { return };
        if cur.is_dir() {
            let Some(path) = tab.path_of(cur) else { return };
            if new_tab {
                self.new_tab(p, Some(Location::Dir(path)), false);
            } else {
                self.navigate(p, Location::Dir(path), None);
            }
            return;
        }
        let mut paths: Vec<String> = tab
            .view
            .iter()
            .map(|&i| &tab.entries[i as usize])
            .filter(|e| e.selected && !e.is_dir())
            .filter_map(|e| tab.path_of(e))
            .collect();
        if paths.is_empty() {
            paths.extend(tab.path_of(cur));
        }
        let dir = tab.loc.dir().map(String::from);
        if paths.len() > OPEN_ASK_OVER {
            self.pending.push(Deferred::ConfirmOpen { pane: p, paths });
        } else {
            self.pending.push(Deferred::Open { pane: p, paths, dir });
        }
    }

    // ---------- 탭·패널 ----------

    /// 위치 없이 여는 새 탭(Ctrl+T·+·탭 줄 더블클릭)의 위치: 설정에 따라 내 PC, 정해 둔 폴더, 지금 탭과 같은 폴더
    fn new_tab_location(&self, p: usize) -> Location {
        if self.new_tab_home == session::NEW_TAB_SAME { self.panes[p].tab().loc.clone() } else { Location::from_session(&self.new_tab_home) }
    }

    pub fn new_tab(&mut self, p: usize, loc: Option<Location>, activate: bool) {
        let loc = loc.unwrap_or_else(|| self.new_tab_location(p));
        // 뒤에 여는 탭(가운데 클릭)은 넣지 않는다: 그대로 보이는 시작 화면의 줄이 마우스 아래에서 밀리지 않게
        if activate {
            self.note_recent(&loc);
        }
        let id = self.next_tab_id();
        let pane = &mut self.panes[p];
        let idx = pane.active + 1;
        pane.tabs.insert(idx, Tab::new(id, loc));
        if activate {
            pane.active = idx;
            self.after_tab_switch(p);
        } else {
            self.load_at(p, idx, false);
        }
        self.invalidate();
    }

    fn close_tab(&mut self, p: usize, t: usize) {
        let pane = &mut self.panes[p];
        if pane.tabs.len() <= 1 || t >= pane.tabs.len() {
            return;
        }
        let was_active = pane.active == t;
        let closed = pane.tabs.remove(t);
        // Ctrl+Shift+T로 다시 열 수 있게 위치와 뒤로·앞으로 기록을 남긴다
        self.closed_tabs.push((p, closed.loc, closed.back, closed.fwd));
        if self.closed_tabs.len() > CLOSED_TABS_KEEP {
            self.closed_tabs.remove(0);
        }
        let pane = &mut self.panes[p];
        if t < pane.active || pane.active >= pane.tabs.len() {
            pane.active = pane.active.saturating_sub(1);
        }
        if was_active {
            self.after_tab_switch(p);
        }
        self.invalidate();
    }

    /// 마지막에 닫은 탭을 닫았던 칸(이제 없으면 지금 칸)에 다시 연다
    fn reopen_closed_tab(&mut self) {
        let Some((p, loc, back, fwd)) = self.closed_tabs.pop() else {
            self.flash(self.active, "다시 열 탭이 없어요");
            return;
        };
        let p = if p < self.layout.count() { p } else { self.active };
        self.activate_pane(p);
        let id = self.next_tab_id();
        let pane = &mut self.panes[p];
        let idx = pane.active + 1;
        let mut tab = Tab::new(id, loc);
        tab.back = back;
        tab.fwd = fwd;
        pane.tabs.insert(idx, tab);
        pane.active = idx;
        self.after_tab_switch(p);
        self.invalidate();
    }

    /// 끌던 탭을 놓을 곳: (칸, 들어갈 위치). 탭 줄 위면 놓은 자리, 칸 본문이면 그 칸의 맨 끝. 칸 밖이면 None
    fn tab_drop_target(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let (q, r) = self.pane_rects().into_iter().find(|(_, r)| r.contains(x, y))?;
        let g = geom(r);
        let n = self.panes[q].tabs.len();
        let at = if g.head.contains(x, y) { drop_index(&self.chips(q, &g).0, x) } else { n };
        Some((q, at))
    }

    /// 탭을 같은 칸 안에서 옮기거나 다른 칸으로 보낸다.
    /// 칸에 하나뿐인 탭은 그 칸을 비울 수 없으니 남겨 두고, 대상 칸에 같은 위치를 새 탭으로 연다
    fn move_tab(&mut self, p: usize, t: usize, q: usize, at: usize) {
        if t >= self.panes[p].tabs.len() {
            return;
        }
        if p == q {
            let pane = &mut self.panes[p];
            let to = (if at > t { at - 1 } else { at }).min(pane.tabs.len() - 1);
            if to != t {
                let tab = pane.tabs.remove(t);
                pane.tabs.insert(to, tab);
                pane.active = to;
            }
            self.invalidate();
            return;
        }
        let tab = if self.panes[p].tabs.len() > 1 {
            let src = &mut self.panes[p];
            let tab = src.tabs.remove(t);
            if t < src.active || src.active >= src.tabs.len() {
                src.active = src.active.saturating_sub(1);
            }
            self.after_tab_switch(p);
            tab
        } else {
            let loc = self.panes[p].tab().loc.clone();
            Tab::new(self.next_tab_id(), loc)
        };
        let dst = &mut self.panes[q];
        let at = at.min(dst.tabs.len());
        dst.tabs.insert(at, tab);
        dst.active = at;
        self.activate_pane(q);
        self.after_tab_switch(q);
    }

    fn activate_tab(&mut self, p: usize, t: usize) {
        if t < self.panes[p].tabs.len() && t != self.panes[p].active {
            self.panes[p].active = t;
            self.after_tab_switch(p);
        }
    }

    fn cycle_tab(&mut self, p: usize, back: bool) {
        let n = self.panes[p].tabs.len();
        let a = self.panes[p].active;
        self.activate_tab(p, if back { (a + n - 1) % n } else { (a + 1) % n });
    }

    /// 탭을 바꾸면 그 탭을 다시 읽는다(보이지 않는 동안 바뀐 내용 반영)
    fn after_tab_switch(&mut self, p: usize) {
        if matches!(self.edit_mode, EditMode::Rename(e) | EditMode::Address(e) if e == p) {
            self.cancel_edit();
        }
        self.start_keys = false;
        let t = self.panes[p].active;
        let fresh = self.panes[p].tab().generation == 0;
        self.load_at(p, t, !fresh);
        if p == self.active {
            let f = self.panes[p].tab().filter.clone();
            self.set_filter_edit_text(&f);
            // "현재 폴더" 범위로 찾는 중이면 이 탭의 폴더로 다시 찾는다
            self.pal_follow_scope();
        }
        self.sync_watches();
        self.update_title();
        self.layout_edits();
        self.invalidate();
    }

    fn activate_pane(&mut self, p: usize) {
        if p == self.active || p >= self.layout.count() {
            return;
        }
        match self.edit_mode {
            EditMode::None => {}
            // 이름(파일·즐겨찾기·작업 공간)은 확정하고, 경로 입력은 취소한다 (탐색기와 같음)
            EditMode::Rename(_) | EditMode::FavRename(_) | EditMode::WorkspaceRename => self.commit_rename(),
            EditMode::Address(_) => self.cancel_edit(),
        }
        self.active = p;
        self.start_keys = false;
        let f = self.panes[p].tab().filter.clone();
        self.set_filter_edit_text(&f);
        // "현재 폴더" 범위로 찾는 중이면 이 칸의 폴더로 다시 찾는다
        self.pal_follow_scope();
        self.layout_edits();
        self.update_title();
        self.invalidate();
    }

    fn cycle_pane(&mut self, back: bool) {
        let n = self.layout.count();
        let next = if back { (self.active + n - 1) % n } else { (self.active + 1) % n };
        self.activate_pane(next);
    }

    fn set_layout(&mut self, l: Layout) {
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        self.close_view_menu();
        let before = self.layout.count();
        self.layout = l;
        if self.active >= l.count() {
            self.active = 0;
            let f = self.panes[0].tab().filter.clone();
            self.set_filter_edit_text(&f);
        }
        for p in 0..l.count() {
            if self.panes[p].tab().generation == 0 {
                self.load(p, false);
            } else if p >= before {
                // 숨어 있는 동안에는 감시하지 않았으니, 다시 보일 때 선택·스크롤을 지키며 새로 읽는다
                self.load(p, true);
            }
            self.clamp_scroll(p);
        }
        // 지금 칸이 바뀌었으면(칸을 줄여 지금 칸이 사라짐) "현재 폴더" 범위 검색도 따라간다
        self.pal_follow_scope();
        self.sync_watches();
        self.layout_edits();
        self.update_title();
        self.invalidate();
    }

    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        let hidden = self.show_hidden;
        for pane in &mut self.panes {
            let sort = pane.sort;
            for tab in &mut pane.tabs {
                tab.rebuild_view(sort, hidden);
            }
        }
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.layout_edits();
        self.invalidate();
    }

    fn set_view(&mut self, p: usize, v: ViewMode) {
        if self.panes[p].view == v {
            return;
        }
        if self.edit_mode == EditMode::Rename(p) {
            self.cancel_edit();
        }
        self.panes[p].view = v;
        self.ensure_visible(p);
        self.invalidate();
    }

    fn on_thumb(&mut self, img: crate::thumbs::Image) {
        self.thumb_wanted.remove(&img.key);
        if img.pixels.is_empty() {
            if self.thumb_failed.len() > 4096 {
                self.thumb_failed.clear();
            }
            self.thumb_failed.insert(img.key);
            // 미리 보기가 이 그림을 기다리고 있으면 "미리 볼 수 없습니다"로 바꿔 그린다
            self.invalidate();
            return;
        }
        if self.gfx.add_thumb(img.key, img.w, img.h, &img.pixels) {
            self.invalidate();
        }
    }

    /// 화면에 필요한데 없는 그림만 요청한다. 같은 목록을 다시 보내지 않는다
    fn request_thumbs(&mut self, missing: Vec<crate::thumbs::Request>) {
        let keys: HashSet<String> = missing.iter().map(|r| r.key.clone()).collect();
        if keys != self.thumb_wanted {
            self.thumb_wanted = keys;
            crate::thumbs::request(self.hwnd, missing);
        }
    }

    fn toggle_sort(&mut self, p: usize, key: SortKey) {
        let pane = &mut self.panes[p];
        if pane.sort.key == key {
            pane.sort.desc = !pane.sort.desc;
        } else {
            // 날짜·크기는 큰 값(최신·큰 파일)부터 보는 경우가 많다
            pane.sort = Sort { key, desc: matches!(key, SortKey::Modified | SortKey::Size) };
        }
        let sort = pane.sort;
        for tab in &mut pane.tabs {
            tab.rebuild_view(sort, self.show_hidden);
        }
        self.ensure_visible(p);
    }

    // ---------- 스크롤 ----------

    fn grid(&self, p: usize) -> Option<(R, Grid)> {
        self.list_rect(p).map(|l| (l, Grid::of(l, self.panes[p].view)))
    }

    fn clamp_scroll(&mut self, p: usize) {
        if let Some(max) = self.start_max_scroll(p) {
            let tab = self.panes[p].tab_mut();
            tab.scroll = tab.scroll.clamp(0.0, max);
            return;
        }
        let Some((list, grid)) = self.grid(p) else { return };
        let tab = self.panes[p].tab_mut();
        let max = (grid.content(tab.view.len()) - grid.page(list)).max(0.0);
        tab.scroll = tab.scroll.clamp(0.0, max);
    }

    fn ensure_visible(&mut self, p: usize) {
        if self.panes[p].tab().loc == Location::Start {
            self.clamp_scroll(p);
            return;
        }
        let Some((list, grid)) = self.grid(p) else { return };
        let tab = self.panes[p].tab_mut();
        // 스크롤 0 기준 위치 (스크롤 방향으로)
        let it = grid.item(tab.cursor, 0.0);
        let (top, bottom) = if grid.horiz { (it.l - list.l, it.r - list.l + 4.0) } else { (it.t - list.t, it.b - list.t) };
        let page = grid.page(list);
        if top < tab.scroll {
            tab.scroll = top;
        } else if bottom > tab.scroll + page {
            tab.scroll = bottom - page;
        }
        self.clamp_scroll(p);
    }

    fn scroll_by(&mut self, p: usize, dy: f32) {
        self.panes[p].tab_mut().scroll += dy;
        self.clamp_scroll(p);
        if self.edit_mode == EditMode::Rename(p) {
            self.layout_edits();
        }
        self.invalidate();
    }

    /// 한 화면에 들어가는 항목 수 (Page Up/Down 이동량)
    fn page_rows(&self, p: usize) -> isize {
        self.grid(p)
            .map(|(l, g)| {
                if g.horiz {
                    ((l.w() / g.cell_w).floor() as isize).max(1) * g.rows as isize
                } else {
                    ((l.h() / g.cell_h).floor() as isize).max(1) * g.cols as isize
                }
            })
            .unwrap_or(10)
    }

    // ---------- 입력 상자 ----------

    fn apply_edit_font(&mut self) {
        let font = make_edit_font(self.gfx.dpi, &self.look.font);
        for h in [self.filter_edit, self.rename_edit, self.addr_edit, self.search_edit, self.cmd_edit] {
            unsafe {
                SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
                // EC_LEFTMARGIN | EC_RIGHTMARGIN, 좌우 2px
                SendMessageW(h, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(2 | (2 << 16))));
            }
        }
        if !self.edit_font.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.edit_font.into());
            }
        }
        self.edit_font = font;
    }

    fn rename_rect(&self, p: usize) -> Option<R> {
        let (list, grid) = self.grid(p)?;
        let pane = &self.panes[p];
        let tab = pane.tab();
        let it = grid.item(tab.cursor, tab.scroll);
        if it.t < list.t - 1.0 || it.b > list.b + 1.0 || it.l < list.l - 1.0 || it.r > list.r + 1.0 {
            return None;
        }
        match grid.cell {
            CellKind::Tile(thumb) => {
                // 그림 아래 이름 자리
                let y = it.t + 8.0 + thumb + 6.0;
                return Some(R::new(it.l + 4.0, y, it.r - 8.0, y + 24.0));
            }
            CellKind::Icon => return Some(R::new(it.l + 24.0, it.t + 2.0, it.r - 6.0, it.b - 2.0)),
            CellKind::Row => {}
        }
        let cols = columns(list, pane.cols);
        Some(R::new(cols.name.0 + 24.0, it.t + 2.0, cols.name.1 - 4.0, it.b - 2.0))
    }

    /// 실제 Win32 입력 상자(한글 IME 지원)를 그려 둔 자리 위에 맞춘다.
    /// 이름을 바꾸는 줄(파일·즐겨찾기)이 보이는 곳 밖으로 밀려났으면(휠·스크롤 막대·창 크기·칸 나누기 등) 입력을 확정한다:
    /// 입력 상자를 숨기기만 하면 키보드가 메인 창으로 가, 이어서 누른 Enter·Delete가 고른 파일에 먹는다
    fn layout_edits(&mut self) {
        let lost = match self.edit_mode {
            EditMode::Rename(p) => self.rename_rect(p).is_none(),
            EditMode::FavRename(row) => self.fav_rename_rect(row).is_none(),
            _ => false,
        };
        if lost {
            self.commit_rename();
        }
        let s = self.scale();
        let place = |h: HWND, r: Option<R>| unsafe {
            match r {
                Some(r) => {
                    let _ = SetWindowPos(
                        h,
                        None,
                        (r.l * s).round() as i32,
                        (r.t * s).round() as i32,
                        (r.w() * s).round() as i32,
                        (r.h() * s).round() as i32,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
                None => {
                    let _ = ShowWindow(h, SW_HIDE);
                }
            }
        };
        let edit_h = 20.0;
        let center = |b: R, lp: f32, rp: f32| R::new(b.l + lp, b.t + (b.h() - edit_h) / 2.0, b.r - rp, b.t + (b.h() + edit_h) / 2.0);
        self.place_filter_edit(true);
        // 다른 패널이 잠깐 가리는 동안 검색 상자는 숨기지 않고 화면 밖에 둔다 (숨기면 키보드 포커스가 메인 창으로 가 버린다)
        let search_at = if self.pal_visible() {
            Some(center(self.pal_input_rect(), 4.0, 4.0))
        } else if self.pal_shown() {
            Some(R::new(-4000.0, -4000.0, -3800.0, -3980.0))
        } else {
            None
        };
        place(self.search_edit, search_at);
        place(self.cmd_edit, self.cmd_input_rect().map(|r| center(r, 0.0, 0.0)));
        let addr = match self.edit_mode {
            EditMode::Address(p) => self.pane_rect(p).map(|r| center(addr_rect(&geom(r)), 8.0, 8.0)),
            _ => None,
        };
        place(self.addr_edit, addr);
        let ren = match self.edit_mode {
            EditMode::Rename(p) => self.rename_rect(p).map(|r| center(r, 4.0, 4.0)),
            EditMode::FavRename(row) => self.fav_rename_rect(row).map(|r| center(r, 4.0, 4.0)),
            EditMode::WorkspaceRename => Some(center(self.workspace_rename_rect(), 0.0, 0.0)),
            _ => None,
        };
        place(self.rename_edit, ren);
    }

    /// 필터 상자 자리 (DIP). 활성 칸의 필터 칩 안에 둔다. 필터가 비어 있거나 팝업이 열려 있으면 화면 밖에 두되
    /// 포커스는 그대로 둬, 목록에서 바로 글자를 치면 걸러진다 (첫 글자가 들어오면 칩이 생겨 그 안으로 옮긴다)
    fn filter_edit_rect(&self) -> R {
        let popup = self.view_menu.is_some() || self.settings.is_some();
        match self.pane_rect(self.active) {
            Some(r) if !popup && !self.panes[self.active].tab().filter.is_empty() => {
                let b = self.filter_parts(self.active, filter_chip(&geom(r))).0;
                // 떠 있는 패널이 그 자리를 덮으면 입력 상자(자식 창은 늘 그림 위에 보인다)도 숨긴다
                if self.side_floating() && self.side_panel_rect().is_some_and(|p| p.l < b.r && p.r > b.l && p.t < b.b && p.b > b.t) {
                    return R::new(-4000.0, -4000.0, -3800.0, -3980.0);
                }
                // 입력 상자 글자는 위쪽에 붙어 그려지므로 칩 안 다른 글자(개수)와 줄이 맞게 2px 내린다
                let t = b.t + (b.h() - 20.0) / 2.0 + 2.0;
                R::new(b.l, t, b.r, t + 20.0)
            }
            _ => R::new(-4000.0, -4000.0, -3800.0, -3980.0),
        }
    }

    /// 필터 상자를 제자리로. force가 아니면 자리가 바뀌었을 때만 옮긴다.
    /// 칩 안 개수 글자(고른 개수 포함)는 고르거나 목록이 바뀔 때마다 폭이 달라지므로 그릴 때마다 맞춘다
    fn place_filter_edit(&self, force: bool) {
        let s = self.scale();
        let r = self.filter_edit_rect();
        let px = ((r.l * s).round() as i32, (r.t * s).round() as i32, (r.w() * s).round() as i32, (r.h() * s).round() as i32);
        if !force && self.filter_placed.get() == px {
            return;
        }
        self.filter_placed.set(px);
        unsafe {
            let _ = SetWindowPos(self.filter_edit, None, px.0, px.1, px.2, px.3, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    }

    fn set_filter_edit_text(&self, s: &str) {
        if edit_text(self.filter_edit) != s {
            let n = s.encode_utf16().count();
            unsafe {
                let _ = SetWindowTextW(self.filter_edit, &HSTRING::from(s));
                // 캐럿은 친 직후처럼 끝에 둔다 (칸·탭을 오간 뒤에도 Delete가 같은 뜻이 되게)
                SendMessageW(self.filter_edit, EM_SETSEL, Some(WPARAM(n)), Some(LPARAM(n as isize)));
            }
        }
    }

    fn sync_filter_from_edit(&mut self) {
        let text = edit_text(self.filter_edit);
        let p = self.active;
        let (sort, hidden) = (self.panes[p].sort, self.show_hidden);
        let tab = self.panes[p].tab_mut();
        if tab.filter == text {
            // 목록 쪽에서 필터를 이미 비웠어도(다른 위치로 이동 등) 입력 상자 자리는 맞춘다
            self.layout_edits();
            return;
        }
        tab.filter = text;
        tab.rebuild_view(sort, hidden);
        // 입력하는 동안 첫 일치 항목을 선택해 Enter로 바로 열 수 있게 한다
        if !tab.filter.is_empty() && !tab.view.is_empty() {
            tab.select_only(0);
        }
        tab.scroll = 0.0;
        // 시작 화면도 글자를 치면 첫 일치 항목에 초점을 두어 Enter로 바로 연다
        if tab.loc == Location::Start {
            tab.cursor = 0;
            self.start_keys = !tab.filter.is_empty();
        }
        self.ensure_visible(p);
        // 필터 칩이 생기거나 없어지고, 개수 글자 폭이 바뀌니 입력 상자 자리를 다시 잡는다
        self.layout_edits();
        self.invalidate();
    }

    fn clear_filter(&mut self) {
        self.set_filter_edit_text("");
        self.sync_filter_from_edit();
    }

    pub fn begin_rename(&mut self, p: usize) {
        // 열려 있는 다른 입력을 먼저 끝낸다 (이름은 확정, 경로는 취소). 칸을 바꾸기 전에 해야 앞 이름이 새 항목에 붙지 않는다
        self.end_edits_for_click();
        self.activate_pane(p);
        // 이 칸에서 기다리던 경로 확인은 버린다 (도착해서 이름 상자를 닫지 않게)
        self.bump_probe_seq(p);
        let tab = self.panes[p].tab();
        if tab.loc == Location::Computer {
            return;
        }
        let Some(e) = tab.cursor_entry() else { return };
        // 고른 것 없이 커서만 남아 있으면 탐색기처럼 아무것도 하지 않는다
        if !e.selected {
            return;
        }
        let Some(path) = tab.path_of(e) else { return };
        let name = e.name_string();
        let dir = e.is_dir();
        // 파일은 확장자를 뺀 부분만 선택 (탐색기와 같음)
        let stem = match name.rfind('.') {
            Some(i) if i > 0 && e.kind == Kind::File => name[..i].encode_utf16().count(),
            _ => name.encode_utf16().count(),
        };
        self.rename = Some(RenameCtx { pane: p, tab: tab.id, path, old: name.clone(), dir });
        self.edit_mode = EditMode::Rename(p);
        self.ensure_visible(p);
        unsafe {
            let _ = SetWindowTextW(self.rename_edit, &HSTRING::from(name));
            SendMessageW(self.rename_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(stem as isize)));
        }
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    fn commit_rename(&mut self) {
        self.commit_rename_with(false);
    }

    /// 이름 바꾸기 확정. enter: Enter로 확정했으면 쓸 수 없는 이름일 때 상자를 친 그대로 열어 둔 채 알린다
    fn commit_rename_with(&mut self, enter: bool) {
        if let EditMode::FavRename(row) = self.edit_mode {
            self.commit_fav_rename(row);
            return;
        }
        if self.edit_mode == EditMode::WorkspaceRename {
            self.commit_workspace_rename();
            return;
        }
        let Some(ctx) = self.rename.take() else { return };
        // 끝의 공백·점은 Windows가 떼어 내므로 미리 뗀다 (탐색기와 같음)
        let new_name = edit_text(self.rename_edit).trim().trim_end_matches(['.', ' ']).to_string();
        if new_name.is_empty() || new_name == ctx.old {
            self.close_edit();
            return;
        }
        if let Some(msg) = util::name_problem(&new_name) {
            self.flash(ctx.pane, msg);
            if enter {
                self.rename = Some(ctx);
                return;
            }
            self.close_edit();
            return;
        }
        self.close_edit();
        // 확장자를 바꾸거나 지우면 탐색기처럼 한 번 묻는다 (아니요면 친 이름 그대로 상자를 다시 연다)
        if !ctx.dir && ext_of_name(&ctx.old) != ext_of_name(&new_name) {
            self.pending.push(Deferred::ConfirmRename { tab: ctx.tab, path: ctx.path, old: ctx.old, new: new_name });
            return;
        }
        self.start_rename(ctx.tab, ctx.path, new_name);
    }

    /// 이름 바꾸기 작업을 시작한다. 바뀐 이름을 고르고(스크롤은 그대로), 필터에 걸려 사라질 이름이면 필터를 푼다
    fn start_rename(&mut self, tab_id: u64, path: String, new_name: String) {
        // 확인 창을 기다리는 동안 다른 폴더로 옮긴 탭이면 고를 것도 알릴 것도 없다
        if let Some((p, t)) = self.tab_showing(tab_id, &path) {
            let tab = &mut self.panes[p].tabs[t];
            tab.restore = Some(Restore { scroll: tab.scroll, cursor_index: tab.cursor, ..Restore::select(&new_name) });
            // 필터에 걸리지 않는 이름이면 목록에서 숨는다. 필터는 그대로 두고(클릭 도중에 목록을 바꾸지 않게) 알린다
            let filter = tab.filter.clone();
            if !filter.is_empty() && !new_name.to_lowercase().contains(&filter.to_lowercase()) {
                self.flash(p, format!("‘{new_name}’ · 필터 ‘{filter}’에 맞지 않아 목록에서 숨겨져요 · Esc로 필터 풀기"));
            }
        }
        shell::spawn_file_op(self.hwnd, FileOp::Rename { path, name: new_name });
    }

    /// 탭 tab_id가 아직 path가 있는 폴더를 보고 있으면 (칸, 탭)
    fn tab_showing(&self, tab_id: u64, path: &str) -> Option<(usize, usize)> {
        let parent = util::parent(path)?;
        self.find_tab(tab_id).filter(|&(p, t)| self.panes[p].tabs[t].loc.dir().is_some_and(|d| util::eq_path(d, &parent)))
    }

    /// 확장자 경고에서 '아니요': 친 이름을 그대로 둔 채 그 항목의 이름 상자를 다시 연다
    fn reopen_rename(&mut self, tab_id: u64, path: &str, old: &str, typed: &str) {
        let Some((p, t)) = self.tab_showing(tab_id, path) else { return };
        if self.panes[p].active != t {
            return;
        }
        let key = old.to_lowercase();
        let tab = self.panes[p].tab_mut();
        let Some(vi) = tab.view.iter().position(|&i| tab.entries[i as usize].name_string().to_lowercase() == key) else { return };
        tab.select_only(vi);
        self.begin_rename(p);
        if self.edit_mode == EditMode::Rename(p) {
            let stem = match typed.rfind('.') {
                Some(i) if i > 0 => typed[..i].encode_utf16().count(),
                _ => typed.encode_utf16().count(),
            };
            unsafe {
                let _ = SetWindowTextW(self.rename_edit, &HSTRING::from(typed));
                SendMessageW(self.rename_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(stem as isize)));
            }
        }
    }

    /// 이름 상자에 쓸 수 없는 글자를 치면: 넣지 않고 알린다. 파일·폴더 이름 바꾸기일 때만 (즐겨찾기·작업 공간 이름은 표시용)
    pub fn rename_char_blocked(&mut self) -> bool {
        let EditMode::Rename(p) = self.edit_mode else { return false };
        self.flash(p, "이름에는 \\ / : * ? \" < > | 를 쓸 수 없어요");
        true
    }

    fn begin_address(&mut self, p: usize) {
        // 이름 상자가 열려 있으면 친 이름을 확정한 뒤 경로 입력으로 넘어간다
        self.end_edits_for_click();
        self.bump_probe_seq(p);
        self.activate_pane(p);
        let text = self.panes[p].tab().loc.dir().unwrap_or("").to_string();
        self.edit_mode = EditMode::Address(p);
        unsafe {
            let _ = SetWindowTextW(self.addr_edit, &HSTRING::from(text));
            SendMessageW(self.addr_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    fn commit_address(&mut self) {
        let EditMode::Address(p) = self.edit_mode else { return };
        let text = edit_text(self.addr_edit);
        self.close_edit();
        let tab = self.panes[p].tab();
        let then = ProbeThen::Address { tab: tab.id, from: tab.loc.clone() };
        self.probe(p, text, then);
    }

    /// 경로를 뒤 스레드에서 확인하고 나서 연다 (on_probed). 꺼진 NAS를 확인하는 동안에도 창은 그대로 쓸 수 있다
    fn probe(&mut self, p: usize, text: String, then: ProbeThen) {
        let seq = self.bump_probe_seq(p);
        self.probe_next += 1;
        let id = self.probe_next;
        self.probes.push((id, p, text.trim().to_string()));
        spawn_probe(self.hwnd, id, seq, p, text, then);
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_PROBE, 400, None);
        }
    }

    /// 칸 p의 요청 번호를 올린다: 그 칸에서 아직 기다리는 경로 확인의 결과는 버려진다
    fn bump_probe_seq(&mut self, p: usize) -> u64 {
        let p = p.min(3);
        self.probe_seq[p] += 1;
        self.probe_seq[p]
    }

    /// 경로 확인이 오래 걸리면 기다리는 중임을 알린다. 기다리는 동안 몇 초마다 다시 띄워 알림이 사라지지 않게 한다
    fn on_probe_timer(&mut self) {
        let Some((_, p, text)) = self.probes.first().cloned() else {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_PROBE);
            }
            return;
        };
        let p = p.min(self.layout.count().saturating_sub(1));
        let msg = format!("여는 중… {text}");
        self.flash(p, msg.clone());
        self.probe_shown = Some((self.panes[p].tab().id, msg));
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_PROBE, 2500, None);
        }
    }

    /// 경로 확인 중이라고 띄운 알림만 지운다 (다른 알림은 그대로)
    fn clear_probe_note(&mut self) {
        let Some((tab, msg)) = self.probe_shown.take() else { return };
        if let Some((p, t)) = self.find_tab(tab) {
            let tab = &mut self.panes[p].tabs[t];
            if tab.message.as_deref() == Some(msg.as_str()) {
                tab.message = None;
                self.invalidate();
            }
        }
    }

    fn on_probed(&mut self, pr: Probed) {
        self.probes.retain(|(id, _, _)| *id != pr.id);
        self.clear_probe_note();
        if self.probes.is_empty() {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_PROBE);
            }
        }
        self.refresh_cursor();
        let p = pr.pane.min(self.layout.count().saturating_sub(1));
        // 기다리는 동안 그 칸에서 다른 일을 시작했거나(경로 입력·이름 바꾸기·다른 확인), 탭을 닫거나 옮겼으면 버린다
        let newer = pr.seq != self.probe_seq[pr.pane.min(3)];
        let moved = |a: &App, tab: u64, from: &Location| a.panes[p].tab().id != tab || &a.panes[p].tab().loc != from;
        match pr.then {
            ProbeThen::Address { tab, from } => {
                if newer || moved(self, tab, &from) {
                    return;
                }
                match pr.result {
                    Ok((loc, select)) => self.navigate(p, loc, select),
                    Err(_) if net_text(&pr.shown) => self.navigate(p, Location::Dir(pr.shown.trim().to_string()), None),
                    Err(msg) => self.flash(p, msg),
                }
            }
            ProbeThen::External => match pr.result {
                Ok((loc, select)) => {
                    // 지금 탭이 이미 그 폴더면 그대로 (대소문자는 가리지 않는다)
                    let same = match (&self.panes[p].tab().loc, &loc) {
                        (Location::Dir(a), Location::Dir(b)) => util::eq_path(a, b),
                        (a, b) => a == b,
                    };
                    if select.is_none() && same {
                        return;
                    }
                    self.new_tab(p, Some(loc), true);
                    if let Some(name) = select {
                        self.panes[p].tab_mut().restore = Some(Restore::select(&name));
                    }
                }
                Err(_) => self.flash(p, format!("경로를 찾을 수 없어요: {}", pr.shown)),
            },
            ProbeThen::Favorite { tab, from, new_tab, is_dir } => {
                if !new_tab && (newer || moved(self, tab, &from)) {
                    return;
                }
                match pr.result {
                    Ok((loc, None)) => {
                        if new_tab {
                            self.new_tab(p, Some(loc), true);
                        } else {
                            self.navigate(p, loc, None);
                        }
                    }
                    Ok((Location::Dir(dir), Some(name))) => {
                        if new_tab {
                            self.new_tab(p, Some(Location::Dir(dir)), true);
                            self.panes[p].tab_mut().restore = Some(Restore::select(&name));
                        } else {
                            let path = util::join(&dir, &name);
                            self.pending.push(Deferred::Open { pane: p, paths: vec![path], dir: Some(dir) });
                        }
                    }
                    Ok(_) => {}
                    // 꺼졌거나 로그인이 필요한 공유의 즐겨찾기: 그 폴더(파일이면 담은 폴더)를 열어 단추·기억한 목록을 보인다
                    Err(_) if net_text(&pr.shown) => {
                        let path = pr.shown.trim().to_string();
                        let dir = if is_dir == Some(false) { util::parent(&path).unwrap_or(path.clone()) } else { path.clone() };
                        if new_tab {
                            self.new_tab(p, Some(Location::Dir(dir)), true);
                        } else {
                            self.navigate(p, Location::Dir(dir), None);
                        }
                    }
                    Err(_) => self.flash(p, format!("즐겨찾기 항목을 찾을 수 없어요: {}", pr.shown)),
                }
            }
        }
    }

    fn close_edit(&mut self) {
        self.edit_mode = EditMode::None;
        unsafe {
            let _ = ShowWindow(self.rename_edit, SW_HIDE);
            let _ = ShowWindow(self.addr_edit, SW_HIDE);
        }
        self.focus();
        self.invalidate();
    }

    fn cancel_edit(&mut self) {
        self.rename = None;
        self.close_edit();
    }

    /// 목록을 클릭하면 이름 바꾸기는 확정, 주소 입력은 취소 (탐색기와 같음)
    fn end_edits_for_click(&mut self) {
        match self.edit_mode {
            EditMode::Rename(_) | EditMode::FavRename(_) | EditMode::WorkspaceRename => self.commit_rename(),
            EditMode::Address(_) => self.cancel_edit(),
            EditMode::None => {}
        }
    }

    /// 칸 아래쪽에 잠깐 알림을 띄운다 (몇 초 뒤 사라진다)
    fn flash(&mut self, p: usize, msg: impl Into<String>) {
        self.toast_undo = None;
        self.panes[p].tab_mut().message = Some(msg.into());
        self.start_anim(|a| &mut a.toast_at);
        unsafe { SetTimer(Some(self.hwnd), TIMER_FLASH, FLASH_MS, None) };
        self.invalidate();
    }

    /// 페이드를 시작한다 (Windows에서 애니메이션을 끄면 하지 않는다)
    fn start_anim(&mut self, at: fn(&mut App) -> &mut Option<std::time::Instant>) {
        *at(self) = sidebar::motion_on().then(std::time::Instant::now);
        if at(self).is_some() {
            unsafe { SetTimer(Some(self.hwnd), TIMER_ANIM, 15, None) };
        }
    }

    fn anims_running(&self) -> bool {
        anim_progress(self.tip_at, TIP_FADE_MS).is_some()
            || anim_progress(self.toast_at, TOAST_FADE_MS).is_some()
            || anim_progress(self.view_menu_at, MENU_FADE_MS).is_some()
            || anim_progress(self.cmd_at, commands::CMD_FADE_MS).is_some()
    }

    /// 휠 한 걸음: 목표 자리 쪽으로 남은 거리의 35%씩 (0.2초쯤에 닿는다)
    fn on_wheel_timer(&mut self) {
        let Some(w) = self.wheel else {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_WHEEL);
            }
            return;
        };
        let alive = w.pane < self.layout.count() && self.panes[w.pane].tab().id == w.tab && (self.panes[w.pane].tab().scroll - w.last).abs() < 0.5;
        let mut done = !alive;
        if alive {
            let cur = self.panes[w.pane].tab().scroll;
            let next = if (w.target - cur).abs() < 1.0 { w.target } else { cur + (w.target - cur) * 0.35 };
            self.panes[w.pane].tab_mut().scroll = next;
            self.clamp_scroll(w.pane);
            let now = self.panes[w.pane].tab().scroll;
            done = (now - w.target).abs() < 0.5 || (now - cur).abs() < 0.01;
            self.wheel = Some(Wheel { last: now, ..w });
            self.invalidate();
        }
        if done {
            self.wheel = None;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_WHEEL);
            }
        }
    }

    fn clear_flash(&mut self) {
        self.toast_undo = None;
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                tab.message = None;
            }
        }
        self.invalidate();
    }

    /// 보이는 칸들의 지금 폴더 (활성 칸부터, 같은 폴더는 한 번)
    fn visible_dirs(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in std::iter::once(self.active).chain(0..self.layout.count()) {
            if let Some(d) = self.panes[p].tab().loc.dir() {
                if !out.iter().any(|x| util::eq_path(x, d)) {
                    out.push(d.to_string());
                }
            }
        }
        out
    }

    // ---------- 트레이 대기 ----------

    /// 켜 두면 창을 닫아도 끝내지 않고 알림 영역에 남아, 전역 단축키로 다시 띄운다
    pub fn set_tray(&mut self, on: bool) {
        if !on {
            self.tray = None;
        } else if self.tray.is_none() {
            self.tray = Some(crate::tray::TrayIcon::add(self.hwnd, &self.tray_tip()));
        }
    }

    fn tray_tip(&self) -> String {
        match &self.hotkey {
            Some(key) => format!("Sakadu · {key}로 검색"),
            None => "Sakadu".to_string(),
        }
    }

    /// 탐색기가 다시 시작돼 알림 영역 아이콘이 사라졌을 때
    pub fn on_taskbar_created(&mut self) {
        let tip = self.tray_tip();
        if let Some(t) = &mut self.tray {
            t.update(true, &tip);
        }
    }

    /// 숨겨 둔 창이 다시 보일 때: 그사이 바뀐 폴더를 읽는다
    pub fn on_shown(&mut self) {
        for p in 0..self.layout.count() {
            if std::mem::take(&mut self.stale[p]) {
                self.load(p, true);
            }
        }
        self.invalidate();
    }

    /// 트레이 메뉴의 끝내기
    pub fn quit(&mut self) {
        if unsafe { IsWindowVisible(self.hwnd) }.as_bool() {
            self.save_session();
        }
        self.pending.push(Deferred::Close);
    }

    // ---------- 파일 작업 ----------

    fn delete(&mut self, p: usize, permanent: bool) {
        let tab = self.panes[p].tab();
        if tab.loc == Location::Computer {
            return;
        }
        // 선택을 풀어 둔 상태에서는 커서 항목으로 대신하지 않는다
        let paths = tab.marked_paths();
        if paths.is_empty() {
            self.flash(p, "지울 항목을 먼저 고르세요");
            return;
        }
        shell::spawn_file_op(self.hwnd, FileOp::Delete { paths, permanent });
    }

    /// Ctrl+Shift+C: 고른 항목의 경로를 글자로 복사한다 (한 줄에 하나, 따옴표 없이). 고른 게 없으면 지금 폴더
    fn copy_paths(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        let mut paths = tab.marked_paths();
        if paths.is_empty() {
            paths.extend(tab.loc.dir().map(String::from));
        }
        if paths.is_empty() {
            return;
        }
        let n = paths.len();
        if shell::set_clipboard_text(self.hwnd, &paths.join("\r\n")).is_ok() {
            self.flash(p, if n == 1 { format!("경로를 복사했어요: {}", paths[0]) } else { format!("경로 {n}개를 복사했어요") });
        }
    }

    fn copy(&mut self, p: usize, cut: bool) {
        let tab = self.panes[p].tab();
        // 고른 것이 없으면(커서만 남은 상태) 아무것도 올리지 않는다 (탐색기와 같음)
        let paths = tab.marked_paths();
        if paths.is_empty() {
            self.flash(p, if cut { "잘라낼 항목을 먼저 고르세요" } else { "복사할 항목을 먼저 고르세요" });
            return;
        }
        let n = paths.len();
        self.pending.push(Deferred::Verb { paths, verb: if cut { "cut" } else { "copy" } });
        self.flash(p, format!("항목 {n}개를 {}", if cut { "잘라냈어요" } else { "복사했어요" }));
    }

    fn paste(&mut self, p: usize) {
        if let Some(dest) = self.panes[p].tab().loc.dir().map(String::from) {
            self.pending.push(Deferred::Paste { pane: p, dest });
        }
    }

    fn new_folder(&mut self, p: usize) {
        let Some(dir) = self.panes[p].tab().loc.dir().map(String::from) else { return };
        let mut name = "새 폴더".to_string();
        let mut n = 2;
        while shell::exists(&util::join(&dir, &name)) {
            name = format!("새 폴더 ({n})");
            n += 1;
        }
        let path = util::join(&dir, &name);
        match std::fs::create_dir(&path) {
            Ok(()) => {
                // 만든 직후 표식을 남겨, 되돌릴 때 그사이 자료가 들어왔으면 지우지 않게 한다
                if let Some(s) = crate::undo::stamp(&path, true) {
                    self.push_undo(crate::undo::Undo::Create(path, s));
                }
                // 필터에 걸려 새 폴더가 보이지 않으면 다른 항목에 이름 상자가 열리므로 필터를 먼저 푼다
                if !self.panes[p].tab().filter.is_empty() {
                    if p == self.active {
                        self.clear_filter();
                    } else {
                        self.panes[p].tab_mut().filter.clear();
                    }
                }
                let tab = self.panes[p].tab_mut();
                tab.restore = Some(Restore { rename: true, scroll: tab.scroll, cursor_index: tab.cursor, ..Restore::select(&name) });
                self.load(p, true);
            }
            Err(e) => self.flash(p, format!("폴더를 만들 수 없어요: {e}")),
        }
    }

    fn push_undo(&mut self, u: crate::undo::Undo) {
        crate::diag::log(|| format!("undo: recorded ({})", u.describe()));
        self.undo.push(u);
        self.undo_rev += 1;
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
    }

    /// Ctrl+Z: 이 앱에서 한 마지막 파일 작업을 되돌린다
    fn undo_last(&mut self) {
        let p = self.active;
        // 진행 중인 작업은 끝나야 기록이 생긴다: 지금 되돌리면 그 앞 작업이 되돌려진다
        if shell::file_ops_running() {
            self.flash(p, "진행 중인 작업이 끝난 뒤 되돌릴 수 있어요");
            return;
        }
        match self.undo.pop() {
            Some(u) => {
                self.undo_rev += 1;
                self.flash(p, u.describe());
                shell::spawn_file_op(self.hwnd, FileOp::Undo(u));
            }
            None => self.flash(p, "되돌릴 작업이 없어요"),
        }
    }

    fn on_file_op(&mut self, r: shell::OpReport) {
        if let Some(u) = r.undo {
            // 옮기기·복사·휴지통: 무엇을 했는지와 되돌리기 단추를 알림으로 (Ctrl+Z와 같은 기록을 쓴다)
            let text = u.done_text();
            self.push_undo(u);
            if let Some(text) = text {
                let p = self.active;
                self.flash(p, text);
                self.toast_undo = Some((p, self.undo_rev));
                unsafe { SetTimer(Some(self.hwnd), TIMER_FLASH, UNDO_TOAST_MS, None) };
            }
        }
        if let Some(o) = r.undone {
            self.on_undone(o);
        }
        if let Some(cut) = r.cut {
            self.after_cut_paste(cut);
        }
        // 복사·이동·삭제로 남은 용량이 바뀌었다
        self.refresh_places();
        // 고정한 미리 보기 파일의 이름이 바뀌었거나 옮겨졌거나 지워졌을 수 있다
        self.preview_pin_check();
    }

    /// 되돌리기가 일부만 됐으면 알리고, 실패한 항목은 기록으로 되돌려 놓아 다시 시도할 수 있게 한다
    fn on_undone(&mut self, o: crate::undo::Undone) {
        let failed = o.retry.as_ref().map_or(0, |r| r.len());
        if let Some(r) = o.retry {
            self.push_undo(r);
        }
        let key = self.keys.get(crate::keys::Action::Undo).map_or_else(|| "되돌리기".to_string(), |c| c.label());
        let changed = match o.changed.as_slice() {
            [] => String::new(),
            [one] => format!("‘{}’ 항목은 그사이 바뀌었거나 없어져 그대로 두었어요", util::file_name(one)),
            many => format!("{}개는 그사이 바뀌었거나 없어져 그대로 두었어요", many.len()),
        };
        let msg = match (failed, changed.is_empty()) {
            (0, true) => return,
            (0, false) => changed,
            (n, true) => format!("{n}개를 되돌리지 못했어요 · {key}로 다시 시도"),
            (n, false) => format!("{n}개를 되돌리지 못했어요({key}로 다시 시도) · {changed}"),
        };
        self.flash(self.active, msg);
    }

    /// 알림의 되돌리기: 알린 뒤로 다른 작업·되돌리기가 없었을 때만 (그 작업이 기록 맨 위에 있다)
    fn toast_undo_click(&mut self, p: usize) {
        let ours = self.toast_undo.is_some_and(|(tp, rev)| tp == p && rev == self.undo_rev);
        self.panes[p].tab_mut().message = None;
        self.toast_undo = None;
        if ours {
            self.undo_last();
        }
        self.invalidate();
    }

    /// 칸 p의 알림 자리: (알약, 되돌리기 단추)
    fn toast_rects(&self, p: usize) -> Option<(R, Option<R>)> {
        let r = self.pane_rect(p)?;
        let m = self.panes[p].tab().message.as_ref()?;
        let g = &self.gfx;
        let list = geom(r).list;
        let undo = self.toast_undo.is_some_and(|(tp, rev)| tp == p && rev == self.undo_rev);
        let bw = if undo { g.measure(TOAST_UNDO, Font::Normal) + 24.0 } else { 0.0 };
        let extra = if undo { bw + 8.0 } else { 0.0 };
        let tw = g.measure(m, Font::Normal).min((r.w() - 80.0 - extra).max(40.0));
        let cx = (r.l + r.r) / 2.0;
        let w = tw + 30.0 + extra;
        let pill = R::new(cx - w / 2.0, list.b - 52.0, cx + w / 2.0, list.b - 16.0);
        let btn = undo.then(|| R::new(pill.r - 6.0 - bw, pill.t + 5.0, pill.r - 6.0, pill.b - 5.0));
        Some((pill, btn))
    }

    /// 잘라내기 붙여넣기가 끝나면 클립보드를 정리한다. 다 옮겼으면 비우고, 남은 파일이 있으면 그것만 다시 잘라 둔다.
    /// 그사이 사용자가 다른 것을 복사했으면 건드리지 않는다
    fn after_cut_paste(&mut self, cut: shell::CutLeft) {
        self.moving.end(cut.id);
        let untouched = shell::clipboard_seq() == cut.seq || shell::clipboard_files().is_some_and(|(paths, is_move)| is_move && paths == cut.clip);
        if !untouched || cut.left == cut.clip {
            return;
        }
        if cut.left.is_empty() {
            shell::clear_clipboard();
        } else if let Err(e) = shell::set_clipboard_files(self.hwnd, &cut.left, true) {
            crate::diag::log(|| format!("cut: could not keep remaining files on clipboard: {e}"));
        }
    }

    fn properties(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        let any_selected = tab.selection_stats().0 > 0;
        let paths = if any_selected {
            tab.selected_paths()
        } else {
            match tab.loc.dir() {
                Some(d) => vec![d.to_string()],
                None => return,
            }
        };
        self.pending.push(Deferred::Verb { paths, verb: "properties" });
    }

    fn context_menu_at_cursor(&mut self, p: usize) {
        let Some((list, grid)) = self.grid(p) else { return };
        let tab = self.panes[p].tab();
        let it = grid.item(tab.cursor, tab.scroll);
        let pt = self.to_screen(it.l + 36.0, it.b.clamp(list.t, list.b));
        // 고른 것이 없으면 빈 곳 메뉴 (커서만 남은 항목의 메뉴를 띄우지 않는다). 빈 곳 메뉴가 없는 내 PC는 커서 항목
        let dir = tab.loc.dir().map(String::from);
        let paths = if dir.is_none() { tab.selected_paths() } else { tab.marked_paths() };
        self.pending.push(Deferred::Menu { pane: p, paths, dir, pt });
    }

    // ---------- 창·세션 ----------

    fn update_title(&self) {
        // 관리자 권한으로 실행되면 일반 권한 탐색기와의 끌어서 놓기가 Windows 보안(UIPI)으로 막히므로 표시해 둔다
        let admin = if unsafe { IsUserAnAdmin() }.as_bool() { " (관리자)" } else { "" };
        let title = format!("{} - Sakadu{admin}", self.panes[self.active].tab().loc.title());
        unsafe {
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(title));
        }
    }

    fn apply_window_theme(&mut self) {
        unsafe {
            let dark = BOOL::from(self.theme.dark);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const c_void, 4);
            // 제목 표시줄을 창 바탕색과 맞춰 한 덩어리처럼 보이게 (Windows 11)
            let caption = colorref(self.theme.window);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_CAPTION_COLOR, &caption as *const _ as *const c_void, 4);
            let text = colorref(self.theme.text);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_TEXT_COLOR, &text as *const _ as *const c_void, 4);
        }
        let t = self.theme;
        let colors = unsafe {
            EditColors {
                text: colorref(t.text),
                field: colorref(t.field),
                field_brush: CreateSolidBrush(colorref(t.field)),
                pane: colorref(t.pane),
                pane_brush: CreateSolidBrush(colorref(t.pane)),
                chip: colorref(t.accent_soft),
                chip_brush: CreateSolidBrush(colorref(t.accent_soft)),
                rename: self.rename_edit,
                preview_host: self.preview_host,
                filter: self.filter_edit,
                cmd: self.cmd_edit,
            }
        };
        if let Some(old) = EDIT_COLORS.replace(Some(colors)) {
            unsafe {
                let _ = DeleteObject(old.field_brush.into());
                let _ = DeleteObject(old.pane_brush.into());
                let _ = DeleteObject(old.chip_brush.into());
            }
        }
        for h in [self.filter_edit, self.rename_edit, self.addr_edit, self.search_edit, self.cmd_edit] {
            unsafe {
                let _ = InvalidateRect(Some(h), None, true);
            }
        }
        self.invalidate();
    }

    /// 세션을 저장한다. 작업 공간이 있으면 지금 작업 공간에도 지금 화면을 적어 둔다
    fn save_session(&mut self) {
        let s = self.screen_state();
        let _ = session::save(&s);
        if let Some(w) = self.workspaces.current.and_then(|c| self.workspaces.list.get_mut(c)) {
            w.state = s;
            if let Err(e) = crate::workspaces::save(&self.workspaces) {
                crate::diag::log(|| format!("workspaces: save failed: {e}"));
            }
        }
    }

    pub fn shutdown(&mut self) {
        for w in self.watches.iter_mut() {
            if let Some((id, _)) = w.take() {
                shell::unwatch(id);
            }
        }
        unsafe {
            let _ = RevokeDragDrop(self.hwnd);
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_SEARCH);
        }
        self.tray = None;
    }

    /// 창 DPI가 바뀌었을 때(모니터 이동 등). 설정의 크기(%)를 곱해 그린다
    pub fn on_dpi(&mut self, dpi: f32) {
        self.gfx.set_dpi(dpi * self.zoom());
        self.apply_edit_font();
    }

    /// 설정의 크기 (1.0 = 100%)
    pub fn zoom(&self) -> f32 {
        self.look.zoom as f32 / 100.0
    }

    /// 화면 설정을 바꾼 뒤: 바뀐 것만 다시 적용하고 저장한다.
    /// 색(테마·강조 색)만 바뀌면 글꼴을 다시 만들거나 아이콘·썸네일을 비우지 않는다
    pub(super) fn apply_look(&mut self, changed: LookPart) {
        self.theme = crate::theme::resolve(&self.look.theme, &self.look.accent);
        if changed == LookPart::Font {
            if let Err(e) = self.gfx.set_font(&self.look.font) {
                crate::diag::log(|| format!("font: {e}"));
            }
            self.apply_edit_font();
        }
        if changed == LookPart::Text {
            self.gfx.set_text_look(crate::gfx::TextLook::from_key(&self.look.text));
        }
        if changed == LookPart::Zoom {
            // 크기가 바뀌면 아이콘·썸네일 비트맵 크기도 달라지므로 다시 만든다 (on_dpi)
            let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96) as f32;
            self.on_dpi(dpi);
            let mut rc = RECT::default();
            unsafe {
                let _ = GetClientRect(self.hwnd, &mut rc);
            }
            self.on_size(rc.right as u32, rc.bottom as u32);
        } else if matches!(changed, LookPart::Font | LookPart::Text) {
            // 글자 폭이 달라지므로 입력 상자 자리를 다시 잡는다
            self.layout_edits();
        }
        self.apply_window_theme();
        self.save_session();
    }

    fn on_size(&mut self, wpx: u32, hpx: u32) {
        self.w = wpx as f32 / self.scale();
        self.h = hpx as f32 / self.scale();
        self.gfx.resize(wpx, hpx);
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.side_clamp_scroll();
        self.pal_clamp_scroll();
        self.layout_edits();
        self.invalidate();
    }

    // ---------- 드래그 앤 드롭 ----------

    /// 드롭 위치의 대상. 폴더 행 위면 그 폴더, 패널 안 다른 곳이면 패널의 현재 폴더.
    /// 사이드바는 즐겨찾기 쪽이면 즐겨찾기에 넣기, 드라이브·검색 폴더 줄이면 그 폴더
    pub fn drop_dir_at(&mut self, pt: POINTL) -> DropAt {
        let mut cp = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut cp);
        }
        let (x, y) = (cp.x as f32 / self.scale(), cp.y as f32 / self.scale());
        let over_side = self.settings.is_none() && (x < BAR_W || self.side_panel_rect().is_some_and(|r| r.contains(x, y)));
        if over_side && matches!(self.hover, Hit::Crumb(..)) {
            self.hover = Hit::None;
            self.invalidate();
        }
        // 막대의 별 위에 놓으면 즐겨찾기에 넣는다 (패널이 닫혀 있어도)
        if x < BAR_W && self.settings.is_none() {
            let side = matches!(self.bar_hit(x, y), Hit::Bar(Panel::Favorites)).then_some(Side::FavEmpty);
            if self.drop_hover.take().is_some() || side != self.side_drop {
                self.side_drop = side;
                self.invalidate();
            }
            return if side.is_some() { DropAt::Favorites } else { DropAt::None };
        }
        if self.settings.is_none() && self.side_panel_rect().is_some_and(|r| r.contains(x, y)) {
            let (side, dir) = self.side_drop_at(x, y).map_or((None, None), |(s, d)| (Some(s), d));
            let side = side.filter(|s| *s == Side::FavEmpty || dir.is_some());
            if self.drop_hover.take().is_some() || side != self.side_drop {
                self.side_drop = side;
                self.invalidate();
            }
            return match (side, dir) {
                (Some(Side::FavEmpty), _) => DropAt::Favorites,
                (_, Some(d)) => DropAt::Dir(d),
                _ => DropAt::None,
            };
        }
        if self.side_drop.take().is_some() {
            self.invalidate();
        }
        let (hover, dir) = match self.hit(x, y) {
            Hit::Row(p, vi) => {
                let tab = self.panes[p].tab();
                match tab.entry_at(vi) {
                    Some(e) if e.is_dir() => (Some((p, Some(vi))), tab.path_of(e)),
                    _ => (Some((p, None)), tab.loc.dir().map(String::from)),
                }
            }
            Hit::Tab(p, t) => (Some((p, None)), self.panes[p].tabs[t].loc.dir().map(String::from)),
            // 경로 조각: 그 단계의 폴더로 놓는다. 칸 테두리 대신 조각을 강조한다 (아래 crumb_drop)
            Hit::Crumb(p, i) => {
                let target = self
                    .pane_rect(p)
                    .and_then(|r| crumbs(&self.gfx, geom(r).crumbs, &self.panes[p].tab().loc, Some("내 PC")).into_iter().nth(i))
                    .and_then(|c| c.target.dir().map(String::from));
                (None, target)
            }
            h => match h.pane() {
                Some(p) => (Some((p, None)), self.panes[p].tab().loc.dir().map(String::from)),
                None => (None, None),
            },
        };
        let hover = if dir.is_some() { hover } else { None };
        // 놓을 경로 조각은 마우스를 올린 것처럼 강조한다 (끄는 동안에는 마우스 이동 메시지가 오지 않는다)
        let crumb = match self.hit(x, y) {
            Hit::Crumb(p, i) if dir.is_some() => Some(Hit::Crumb(p, i)),
            _ => None,
        };
        if let Some(h) = crumb {
            if self.hover != h {
                self.hover = h;
                self.invalidate();
            }
        } else if matches!(self.hover, Hit::Crumb(..)) {
            self.hover = Hit::None;
            self.invalidate();
        }
        if hover != self.drop_hover {
            self.drop_hover = hover;
            self.invalidate();
        }
        dir.map_or(DropAt::None, DropAt::Dir)
    }

    pub fn clear_drop_hover(&mut self) {
        if self.drop_hover.take().is_some() | self.side_drop.take().is_some() {
            self.invalidate();
        }
        if matches!(self.hover, Hit::Crumb(..)) {
            self.hover = Hit::None;
            self.invalidate();
        }
    }

    // ---------- 메시지 처리 ----------

    pub fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let hwnd = self.hwnd;
        unsafe {
            match msg {
                WM_PAINT => {
                    let mut ps = PAINTSTRUCT::default();
                    BeginPaint(hwnd, &mut ps);
                    self.paint();
                    let _ = EndPaint(hwnd, &ps);
                }
                WM_ERASEBKGND => return Some(LRESULT(1)),
                WM_SIZE => self.on_size(loword(lp.0 as usize) as u32, hiword(lp.0 as usize) as u32),
                WM_MOUSEMOVE => {
                    let (x, y) = self.dip(lp);
                    self.on_mouse_move(x, y);
                }
                WM_MOUSELEAVE => {
                    self.tracking = false;
                    self.side_mouse_left();
                    if self.hover != Hit::None {
                        self.hover = Hit::None;
                        self.update_tip();
                        self.invalidate();
                    }
                }
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                    let (x, y) = self.dip(lp);
                    self.on_lbutton_down(x, y, msg == WM_LBUTTONDBLCLK);
                }
                WM_LBUTTONUP => self.on_lbutton_up(),
                WM_RBUTTONDOWN => {
                    let (x, y) = self.dip(lp);
                    self.on_rbutton_down(x, y);
                }
                WM_RBUTTONUP => {
                    let (x, y) = self.dip(lp);
                    if !self.on_rbutton_up(x, y) {
                        return None;
                    }
                }
                WM_MBUTTONUP => {
                    let (x, y) = self.dip(lp);
                    self.on_mbutton_up(x, y);
                }
                WM_XBUTTONUP => {
                    let (x, y) = self.dip(lp);
                    let p = self.hit(x, y).pane().unwrap_or(self.active);
                    match hiword(wp.0) {
                        XBUTTON1 => self.go_back(p),
                        XBUTTON2 => self.go_forward(p),
                        _ => {}
                    }
                    return Some(LRESULT(1));
                }
                WM_MOUSEWHEEL => self.on_wheel(wp, lp),
                WM_CAPTURECHANGED => {
                    if !matches!(self.drag, Drag::None) {
                        self.drag = Drag::None;
                        self.invalidate();
                    }
                }
                WM_SETCURSOR => {
                    if loword(lp.0 as usize) as u32 == HTCLIENT && self.on_set_cursor() {
                        return Some(LRESULT(1));
                    }
                    return None;
                }
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    self.key_repeat = lp.0 & (1 << 30) != 0;
                    if !self.on_key(VIRTUAL_KEY(wp.0 as u16)) {
                        return None;
                    }
                }
                WM_SETFOCUS => self.focus(),
                WM_ACTIVATE => {
                    // 다른 창으로 전환하면 보기 메뉴를 닫는다 (WA_INACTIVE).
                    // 검색 패널은 다른 앱에서 무언가 복사해 오는 동안에도 적어 둔 검색어가 그대로 있도록 열어 둔다
                    self.window_active = loword(wp.0) != 0;
                    self.invalidate();
                    if loword(wp.0) == 0 {
                        self.close_view_menu();
                        self.close_peek();
                    } else {
                        // 관리자 승인(검색 서비스 설치) 등을 마치고 돌아오면 설정 창의 상태를 다시 읽는다.
                        // 다른 곳에서 파일을 옮겼을 수 있으니 드라이브 남은 용량도 다시 읽는다
                        self.refresh_settings();
                        self.refresh_places();
                        // 다른 앱에서 고정한 파일의 이름을 바꾸거나 옮겼을 수 있다
                        self.preview_pin_check();
                        // 비활성인 동안 남겨 둔 알림은 이제부터 잠시 뒤 지운다
                        if self.panes.iter().any(|pn| pn.tabs.iter().any(|t| t.message.is_some())) {
                            SetTimer(Some(hwnd), TIMER_FLASH, FLASH_MS, None);
                        }
                    }
                    return None;
                }
                WM_COMMAND => {
                    if hiword(wp.0) as u32 == EN_CHANGE {
                        match loword(wp.0) as i32 {
                            ID_FILTER => self.sync_filter_from_edit(),
                            ID_SEARCH => self.pal_query_changed(),
                            ID_CMD => self.cmd_rebuild(),
                            _ => {}
                        }
                    }
                }
                WM_TIMER => {
                    let id = wp.0;
                    if id == TIMER_SEARCH_READY {
                        self.pal_ready_timer();
                    } else if id == TIMER_SEARCH || id == palette::TIMER_SEARCH_AGAIN {
                        let _ = KillTimer(Some(hwnd), id);
                        self.pal_run_query();
                    } else if id == TIMER_PREVIEW {
                        self.start_preview();
                    } else if id == TIMER_PROBE {
                        self.on_probe_timer();
                    } else if id == TIMER_REDRAW {
                        let _ = KillTimer(Some(hwnd), id);
                        self.invalidate();
                    } else if id == TIMER_LOADING {
                        // 읽는 중인 탭이 남아 있는 동안만 다시 그려 "불러오는 중…"을 띄운다
                        let busy = (0..self.layout.count()).any(|p| self.panes[p].tab().loading_since.is_some());
                        if !busy {
                            let _ = KillTimer(Some(hwnd), id);
                        } else if sidebar::motion_on() {
                            // 로딩 막대가 움직이도록 자주 다시 그린다
                            SetTimer(Some(hwnd), id, 32, None);
                        }
                        self.refresh_cursor();
                        self.invalidate();
                    } else if id == TIMER_FAV_FLASH {
                        let _ = KillTimer(Some(hwnd), id);
                        if self.fav_flash.take().is_some() {
                            self.invalidate();
                        }
                    } else if id == TIMER_FLASH {
                        let _ = KillTimer(Some(hwnd), id);
                        // 다른 앱을 쓰거나 트레이에 숨어 있는 동안 끝난 작업의 알림은 돌아왔을 때 보이게 남겨 둔다
                        if self.window_active && IsWindowVisible(hwnd).as_bool() {
                            self.clear_flash();
                        }
                    } else if id == TIMER_TIP {
                        let _ = KillTimer(Some(hwnd), id);
                        self.start_anim(|a| &mut a.tip_at);
                        self.tip_shown = true;
                        self.invalidate();
                    } else if id == TIMER_OFFLINE {
                        let mut any = false;
                        for p in 0..self.layout.count() {
                            let tab = self.panes[p].tab();
                            if tab.offline {
                                any = true;
                                if tab.loading_since.is_none() {
                                    self.load(p, true);
                                }
                            }
                        }
                        if !any {
                            let _ = KillTimer(Some(hwnd), id);
                        }
                    } else if id == TIMER_WHEEL {
                        self.on_wheel_timer();
                    } else if id == TIMER_ANIM {
                        if !self.anims_running() {
                            let _ = KillTimer(Some(hwnd), id);
                        }
                        self.invalidate();
                    } else if id == TIMER_BAR_HOVER {
                        self.on_bar_hover_timer();
                    } else if id == TIMER_SIDE_CLOSE {
                        self.on_side_close_timer();
                    } else if id == TIMER_SIDE_ANIM {
                        self.on_side_anim_timer();
                    } else if id == TIMER_DATES {
                        if self.look.dates != "full" && IsWindowVisible(hwnd).as_bool() {
                            self.invalidate();
                        }
                    } else if (TIMER_REFRESH..TIMER_REFRESH + 4).contains(&id) {
                        let _ = KillTimer(Some(hwnd), id);
                        let p = id - TIMER_REFRESH;
                        if p < self.layout.count() {
                            // 트레이에 숨어 있는 동안에는 읽지 않고, 다시 보일 때 읽는다
                            if IsWindowVisible(hwnd).as_bool() {
                                self.load(p, true);
                                // 파일 작업·이름 바꾸기로 바뀐 것을 검색 결과와 고정한 미리 보기에도 반영한다
                                self.pal_requery();
                                self.preview_pin_check();
                            } else {
                                self.stale[p] = true;
                            }
                        }
                    }
                }
                // 고대비 모드를 켜고 끄면 시스템 색이 바뀐다
                WM_SYSCOLORCHANGE => {
                    self.theme = crate::theme::resolve(&self.look.theme, &self.look.accent);
                    self.apply_window_theme();
                    return None;
                }
                WM_SETTINGCHANGE => {
                    if wp.0 == SPI_SETWHEELSCROLLLINES.0 as usize {
                        self.wheel_lines = wheel_lines();
                    }
                    let colors = wp.0 == SPI_SETHIGHCONTRAST.0 as usize;
                    if colors || lp.0 != 0 && PCWSTR(lp.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                        self.theme = crate::theme::resolve(&self.look.theme, &self.look.accent);
                        self.apply_window_theme();
                        let tip = self.tray_tip();
                        if let Some(t) = &mut self.tray {
                            t.update(false, &tip);
                        }
                    }
                    return None;
                }
                // 시계를 바꾸면 짧은 날짜(오늘·어제)를 다시 그린다
                WM_TIMECHANGE => self.invalidate(),
                // Windows를 끄거나 로그아웃할 때: 트레이에 숨어 있어도 지금 화면을 저장한다
                WM_QUERYENDSESSION => {
                    self.save_session();
                    return Some(LRESULT(1));
                }
                WM_CLOSE => {
                    self.save_session();
                    let to_tray = self.tray.is_some();
                    crate::diag::log(|| format!("close: {}", if to_tray { "hide to tray" } else { "destroy window" }));
                    self.pending.push(if to_tray { Deferred::HideToTray } else { Deferred::Close });
                }
                // 즐겨찾기에 키보드가 있을 때의 글자: 그 글자로 시작하는 줄로
                WM_CHAR if self.fav_focus => {
                    self.fav_type_char(wp.0 as u16);
                }
                // USB·네트워크 드라이브를 꽂거나 빼면 사이드바 드라이브 목록을 다시 읽는다
                // (DBT_DEVICEARRIVAL 0x8000, DBT_DEVICEREMOVECOMPLETE 0x8004)
                WM_DEVICECHANGE => {
                    if matches!(wp.0, 0x8000 | 0x8004) {
                        self.refresh_places();
                    }
                    return None;
                }
                crate::WM_APP_PLACES => self.on_places(*Box::from_raw(lp.0 as *mut sidebar::Places)),
                WM_APP_LISTED => self.on_listed(*Box::from_raw(lp.0 as *mut Listed)),
                crate::WM_APP_PROBED => self.on_probed(*Box::from_raw(lp.0 as *mut Probed)),
                crate::WM_APP_PALMETA => self.on_pal_meta(*Box::from_raw(lp.0 as *mut palette::PalMeta)),
                WM_APP_ICONS => self.on_icons(*Box::from_raw(lp.0 as *mut IconsReady)),
                WM_APP_SEARCH => self.on_search_done(*Box::from_raw(lp.0 as *mut SearchDone)),
                crate::WM_APP_FILEOP => self.on_file_op(*Box::from_raw(lp.0 as *mut shell::OpReport)),
                crate::WM_APP_THUMB => self.on_thumb(*Box::from_raw(lp.0 as *mut crate::thumbs::Image)),
                crate::WM_APP_PREVIEW => self.on_preview_msg(*Box::from_raw(lp.0 as *mut PreviewMsg)),
                m if (WM_APP_NOTIFY..WM_APP_NOTIFY + 4).contains(&m) => {
                    shell::ack_notify(wp, lp);
                    // 연속된 변경(복사 중 등)을 한 번의 새로 고침으로 묶는다
                    SetTimer(Some(hwnd), TIMER_REFRESH + (m - WM_APP_NOTIFY) as usize, 250, None);
                }
                _ => return None,
            }
        }
        Some(LRESULT(0))
    }

    fn on_mouse_move(&mut self, x: f32, y: f32) {
        match self.drag {
            Drag::SplitX => {
                let a = self.area();
                self.split.0 = ((x - a.l) / (a.w() - GAP)).clamp(0.15, 0.85);
                self.after_resize_drag();
            }
            Drag::SplitY => {
                let a = self.area();
                self.split.1 = ((y - a.t) / (a.h() - GAP)).clamp(0.15, 0.85);
                self.after_resize_drag();
            }
            Drag::Col { pane, col, start_x, start_w } => {
                self.panes[pane].cols[col] = (start_w + (start_x - x)).clamp(60.0, 400.0);
                self.invalidate();
            }
            Drag::Thumb { pane, grab } => {
                let Some((list, grid)) = self.grid(pane) else { return };
                let tab = self.panes[pane].tab();
                let content = grid.content(tab.view.len());
                if let Some((track, thumb)) = grid.bar(list, tab.view.len(), tab.scroll) {
                    let max = content - grid.page(list);
                    let (_, thumb_len, track_start, track_len) = grid.bar_span(track, thumb);
                    let ratio = (grid.along(x, y) - grab - track_start) / (track_len - thumb_len).max(1.0);
                    self.panes[pane].tab_mut().scroll = ratio * max;
                    self.scroll_by(pane, 0.0);
                }
            }
            Drag::Press { pane, x: x0, y: y0, .. } => {
                if (x - x0).abs() > 4.0 || (y - y0).abs() > 4.0 {
                    let paths = self.panes[pane].tab().selected_paths();
                    self.drag = Drag::None;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    crate::diag::log(|| format!("drag: start with {} item(s)", paths.len()));
                    if !paths.is_empty() {
                        self.pending.push(Deferred::DragOut(paths));
                    }
                }
            }
            Drag::FavPress { .. } | Drag::FavMove { .. } => self.fav_drag_move(y),
            Drag::SideW { start_x, start_w } => {
                let lo = session::SIDE_W_RANGE.0;
                self.side_w = (start_w + x - start_x).clamp(lo, self.side_w_max());
                self.side_clamp_scroll();
                self.after_resize_drag();
            }
            Drag::TabPress { pane, tab, x: x0, y: y0 } => {
                if (x - x0).abs() > 6.0 || (y - y0).abs() > 6.0 {
                    self.drag = Drag::TabMove { pane, tab, x, y };
                    self.invalidate();
                }
            }
            Drag::TabMove { pane, tab, .. } => {
                self.drag = Drag::TabMove { pane, tab, x, y };
                self.invalidate();
            }
            Drag::MarqueeArm { pane, x: x0, y: y0, additive } => {
                if (x - x0).abs() > 4.0 || (y - y0).abs() > 4.0 {
                    let tab = self.panes[pane].tab_mut();
                    if !additive {
                        tab.clear_selection();
                    }
                    self.marquee_base = tab.selection_mask();
                    // 시작점은 스크롤 0 기준으로 둔다 (목록 보기는 가로 스크롤)
                    let scroll = tab.scroll;
                    let horiz = self.grid(pane).is_some_and(|(_, g)| g.horiz);
                    let (x0, y0) = if horiz { (x0 + scroll, y0) } else { (x0, y0 + scroll) };
                    self.update_marquee(pane, x0, y0, x, y);
                }
            }
            Drag::Marquee { pane, x0, y0, .. } => self.update_marquee(pane, x0, y0, x, y),
            Drag::None => {
                let h = self.hit(x, y);
                if h != self.hover {
                    let on_toast = |h: Hit| matches!(h, Hit::Toast(_) | Hit::ToastUndo(_));
                    match (on_toast(self.hover), on_toast(h)) {
                        (false, true) => unsafe {
                            let _ = KillTimer(Some(self.hwnd), TIMER_FLASH);
                        },
                        (true, false) => unsafe {
                            SetTimer(Some(self.hwnd), TIMER_FLASH, 3000, None);
                        },
                        _ => {}
                    }
                    self.hover = h;
                    self.cmd_hover(h);
                    self.view_menu_hover(h);
                    self.update_tip();
                    self.invalidate();
                }
                self.side_track(x, y);
                if !self.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    self.tracking = unsafe { TrackMouseEvent(&mut tme) }.is_ok();
                }
            }
        }
    }

    /// 자세히 보기에서 이름 칸 밖(날짜·유형·크기)은 끌면 네모 상자로 고른다. 이름 칸은 파일 끌기
    fn marquee_column(&self, p: usize, x: f32) -> bool {
        self.panes[p].view == ViewMode::Details && self.list_rect(p).is_some_and(|l| x >= columns(l, self.panes[p].cols).name.1)
    }

    fn update_marquee(&mut self, p: usize, x0: f32, y0: f32, x: f32, y: f32) {
        self.drag = Drag::Marquee { pane: p, x0, y0, x, y };
        let Some((list, grid)) = self.grid(p) else { return };
        // 목록 밖으로 벗어나면 그쪽으로 스크롤 (목록 보기는 왼쪽·오른쪽, 나머지는 위·아래)
        let (pos, lo, hi) = if grid.horiz { (x, list.l, list.r) } else { (y, list.t, list.b) };
        if pos < lo {
            self.scroll_by(p, -(lo - pos).min(40.0));
        } else if pos > hi {
            self.scroll_by(p, (pos - hi).min(40.0));
        }
        let tab = self.panes[p].tab_mut();
        let (sx, sy) = if grid.horiz { (tab.scroll, 0.0) } else { (0.0, tab.scroll) };
        let (t, b) = (y0.min(y + sy), y0.max(y + sy));
        let (l, r) = (x0.min(x + sx), x0.max(x + sx));
        tab.select_marquee(&self.marquee_base, |vi| {
            let it = grid.item(vi, 0.0);
            it.l < r && it.r > l && it.t < b && it.b > t
        });
        self.invalidate();
    }

    fn after_resize_drag(&mut self) {
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.pal_clamp_scroll();
        self.layout_edits();
        self.invalidate();
    }

    fn on_lbutton_down(&mut self, x: f32, y: f32, dbl: bool) {
        // 앞 누름이 검색 패널·보기 메뉴·설정을 열거나 닫았으면, 두 번 클릭의 두 번째 누름은 그 아래 다른 것을 누르지 않게 버린다
        if dbl && std::mem::take(&mut self.swallow_dbl) {
            return;
        }
        let popups = |a: &App| (a.pal_shown(), a.view_menu.is_some(), a.settings.is_some());
        let before = popups(self);
        // 시작 화면 카드를 눌러 옮겨 갔으면 두 번째 누름이 새 목록의 항목을 열지 않게 버린다
        let start = matches!(self.hit(x, y), Hit::Start(..));
        self.press(x, y, dbl);
        self.swallow_dbl = popups(self) != before || (start && !dbl);
    }

    fn press(&mut self, x: f32, y: f32, dbl: bool) {
        let hit = self.hit(x, y);
        if self.cmd.is_some() {
            return self.cmd_click(hit);
        }
        // 시작 화면 항목은 누른 순간의 위치를 연다 (아래의 이름 확정 등으로 목록이 바뀌어도)
        let start_target = match hit {
            Hit::Start(p, i) => self.start_target(p, i),
            _ => None,
        };
        // 누르면 말풍선을 거두고 기다리던 것도 지운다 (누른 뒤에 말풍선·정보 카드가 뒤늦게 뜨지 않게)
        self.tip_shown = false;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_TIP);
        }
        // 마우스를 쓰면 시작 화면의 키보드 초점 테두리는 감춘다
        self.start_keys = false;
        if let Hit::Set(h) = hit {
            self.settings_click(h, x, y);
            return;
        }
        // 사이드바 단추·드라이브·폴더 줄의 두 번째 누름(두 번 클릭)은 아무것도 하지 않는다.
        // 첫 누름이 시작한 일(새 그룹·추가 뒤 이름 입력 등)을 끝내 버리지 않게
        if dbl && matches!(hit, Hit::Side(s) if !matches!(s, Side::Fav(_))) {
            return;
        }
        // 막대 아이콘·핀·`…`·검색 패널 단추도 두 번째 누름은 무시한다 (첫 누름이 연 것을 바로 닫지 않게)
        if dbl
            && matches!(
                hit,
                Hit::Bar(_) | Hit::SidePin | Hit::More | Hit::SearchButton | Hit::PalClose | Hit::PalScope(_) | Hit::PalInstall | Hit::PalAddFolder
                    | Hit::PreviewPin | Hit::PreviewChip | Hit::PreviewOpen | Hit::Workspace
            )
        {
            return;
        }
        // 떠 있는 패널 밖을 누르면 패널은 닫고, 누른 것은 그대로 처리한다
        if self.side_floating() && !matches!(hit, Hit::Side(_) | Hit::SidePin | Hit::SidePanel | Hit::Bar(_) | Hit::SearchButton | Hit::SettingsButton) {
            self.close_peek();
        }
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        // 검색 패널 밖을 누르면 키보드는 검색 상자에서 칸으로 (패널과 결과는 그대로)
        if self.pal_focus && !matches!(hit, Hit::PalRow(_) | Hit::PalScope(_) | Hit::PalInstall | Hit::PalAddFolder | Hit::PalClose | Hit::SidePanel | Hit::SearchButton) {
            self.pal_blur();
        }
        // 보기 메뉴가 열려 있으면 메뉴 밖 클릭은 메뉴를 닫기만 한다 (보기 버튼은 아래에서 닫기·다른 칸 메뉴 열기)
        if self.view_menu.is_some() && !matches!(hit, Hit::View(_)) {
            self.view_menu_mouse_down(hit);
            self.invalidate();
            return;
        }
        // 즐겨찾기 줄 밖을 누르면 키보드는 칸으로 돌아간다
        if self.fav_focus && !matches!(hit, Hit::Side(Side::Fav(_))) {
            self.fav_blur();
        }
        if let Some(p) = hit.pane() {
            self.activate_pane(p);
        }
        if !matches!(hit, Hit::CrumbEmpty(_)) {
            self.end_edits_for_click();
        }
        let capture = |app: &mut App, d: Drag| {
            app.drag = d;
            unsafe { SetCapture(app.hwnd) };
        };
        match hit {
            Hit::Layout(i) => self.set_layout(Layout::ALL[i]),
            Hit::Side(s) => self.side_mouse_down(s, y, dbl),
            // 막대의 검색 아이콘: 패널이 보이면 닫고, 아니면 연다
            Hit::SearchButton => {
                if self.pal_shown() {
                    self.close_palette();
                } else {
                    self.open_palette();
                }
            }
            Hit::PalRow(_) | Hit::PalInstall | Hit::PalAddFolder | Hit::PalScope(_) => self.pal_mouse_down(hit, dbl),
            Hit::PalClose => self.close_palette(),
            Hit::NewFolder(p) => self.new_folder(p),
            Hit::Hidden => self.toggle_hidden(),
            Hit::PreviewButton => self.toggle_preview(),
            Hit::PreviewPin | Hit::PreviewChip => self.toggle_preview_pin(),
            Hit::PreviewOpen => self.preview_open(),
            Hit::Workspace => {
                let r = self.workspace_button();
                let pt = self.to_screen(r.l, r.b + 2.0);
                self.pending.push(Deferred::WorkspaceMenu { pt });
            }
            Hit::SettingsButton => self.toggle_settings(settings_ui::Page::General),
            Hit::Bar(panel) => self.bar_click(panel),
            Hit::SidePin => self.side_toggle_pin(),
            Hit::SidePanel => {}
            Hit::SideResize => capture(self, Drag::SideW { start_x: x, start_w: self.side_w }),
            Hit::More => {
                let r = self.more_button();
                let pt = self.to_screen(r.l, r.b + 2.0);
                self.pending.push(Deferred::MoreMenu { pt });
            }
            Hit::View(p) => self.toggle_view_menu(p),
            Hit::SplitX => capture(self, Drag::SplitX),
            Hit::SplitY => capture(self, Drag::SplitY),
            Hit::Tab(p, t) => {
                self.activate_tab(p, t);
                capture(self, Drag::TabPress { pane: p, tab: t, x, y });
            }
            Hit::TabClose(p, t) => self.close_tab(p, t),
            Hit::TabAdd(p) => self.new_tab(p, None, true),
            Hit::TabStrip(p) if dbl => self.new_tab(p, None, true),
            Hit::Back(p) => self.go_back(p),
            Hit::Fwd(p) => self.go_forward(p),
            Hit::Up(p) => self.go_up(p),
            Hit::Crumb(p, i) => {
                let Some(r) = self.pane_rect(p) else { return };
                let cur = self.panes[p].tab().loc.clone();
                if let Some(c) = crumbs(&self.gfx, geom(r).crumbs, &cur, Some("내 PC")).into_iter().nth(i) {
                    if c.target == cur {
                        self.refresh(p);
                    } else {
                        let select = child_toward(&c.target, &cur);
                        self.navigate(p, c.target, select);
                    }
                }
            }
            Hit::CrumbEmpty(p) => self.begin_address(p),
            Hit::Star(p) => self.toggle_star(p),
            Hit::Header(p, key) => self.toggle_sort(p, key),
            Hit::HeaderResize(p, col) => {
                let start_w = self.panes[p].cols[col];
                capture(self, Drag::Col { pane: p, col, start_x: x, start_w });
            }
            Hit::Thumb(p) => {
                let Some((list, grid)) = self.grid(p) else { return };
                let tab = self.panes[p].tab();
                if let Some((track, thumb)) = grid.bar(list, tab.view.len(), tab.scroll) {
                    let (start, ..) = grid.bar_span(track, thumb);
                    capture(self, Drag::Thumb { pane: p, grab: grid.along(x, y) - start });
                }
            }
            Hit::Track(p) => {
                let Some((list, grid)) = self.grid(p) else { return };
                let tab = self.panes[p].tab();
                if let Some((track, thumb)) = grid.bar(list, tab.view.len(), tab.scroll) {
                    let (start, ..) = grid.bar_span(track, thumb);
                    let page = grid.page(list);
                    self.scroll_by(p, if grid.along(x, y) < start { -page } else { page });
                }
            }
            Hit::Row(p, vi) if !dbl && self.marquee_column(p, x) => {
                // 누르기만 하면 그 줄을 고르고, 끌면 네모 상자 고르기
                let tab = self.panes[p].tab_mut();
                if ctrl {
                    tab.toggle(vi);
                } else if shift {
                    tab.select_range(vi, false);
                } else {
                    tab.select_only(vi);
                }
                capture(self, Drag::MarqueeArm { pane: p, x, y, additive: ctrl });
            }
            Hit::Row(p, vi) => {
                if dbl {
                    self.drag = Drag::None;
                    self.open_items(p, false);
                    return;
                }
                let tab = self.panes[p].tab_mut();
                let was_selected = tab.is_selected(vi);
                if ctrl && shift {
                    tab.select_range(vi, true);
                } else if ctrl {
                    tab.toggle(vi);
                } else if shift {
                    tab.select_range(vi, false);
                } else if !was_selected {
                    tab.select_only(vi);
                } else {
                    tab.cursor = vi;
                }
                // 이미 선택된 여러 항목을 끌 수 있도록, 단일 선택 전환은 버튼을 뗄 때 한다
                capture(self, Drag::Press { pane: p, vi, x, y, select_on_up: !ctrl && !shift && was_selected });
            }
            Hit::ListEmpty(p) => {
                if !ctrl {
                    self.panes[p].tab_mut().clear_selection();
                }
                capture(self, Drag::MarqueeArm { pane: p, x, y, additive: ctrl });
            }
            Hit::FilterClear(_) => self.clear_filter(),
            Hit::ToastUndo(p) => self.toast_undo_click(p),
            Hit::NetFix(p) => self.net_fix_click(p),
            Hit::Toast(_) => {}
            // 한 번 누르면 연다 (Ctrl이면 새 탭에서)
            Hit::Start(p, _) if !dbl => {
                if let Some(loc) = start_target {
                    self.start_go(p, loc, ctrl, true);
                }
            }
            _ => {}
        }
        self.invalidate();
    }

    fn on_lbutton_up(&mut self) {
        if self.fav_mouse_up(key_down(VK_CONTROL)) {
            return;
        }
        if let Drag::Press { pane, vi, select_on_up: true, .. } = self.drag {
            self.panes[pane].tab_mut().select_only(vi);
        }
        if let Drag::TabMove { pane, tab, x, y } = self.drag {
            if let Some((q, at)) = self.tab_drop_target(x, y) {
                self.move_tab(pane, tab, q, at);
            }
        }
        if !matches!(self.drag, Drag::None) {
            self.drag = Drag::None;
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        self.invalidate();
    }

    fn on_rbutton_down(&mut self, x: f32, y: f32) {
        let hit = self.hit(x, y);
        if self.settings.is_some() || self.cmd.is_some() {
            return;
        }
        // 즐겨찾기 줄 밖을 누르면 키보드는 칸으로 (그 뒤 Delete가 즐겨찾기를 지우지 않게). 검색 상자도 마찬가지
        if self.fav_focus && !matches!(hit, Hit::Side(Side::Fav(_))) {
            self.fav_blur();
        }
        if self.pal_focus && !matches!(hit, Hit::PalRow(_) | Hit::PalScope(_) | Hit::PalInstall | Hit::PalAddFolder | Hit::PalClose | Hit::SidePanel) {
            self.pal_blur();
        }
        if let Hit::PalRow(i) = hit {
            self.pal_right_press(i);
            return;
        }
        if matches!(hit, Hit::PalInstall | Hit::PalAddFolder | Hit::PalScope(_) | Hit::PalClose) {
            return;
        }
        if let Some(p) = hit.pane() {
            self.activate_pane(p);
        }
        self.end_edits_for_click();
        if let Hit::Side(_) = hit {
            self.invalidate();
            return;
        }
        match hit {
            Hit::Row(p, vi) => {
                let tab = self.panes[p].tab_mut();
                if tab.is_selected(vi) {
                    tab.cursor = vi;
                } else {
                    tab.select_only(vi);
                }
            }
            Hit::ListEmpty(p) => self.panes[p].tab_mut().clear_selection(),
            _ => {}
        }
        // 셸 메뉴는 확장을 불러오느라 수십~수백 ms가 걸리므로 버튼을 누르는 순간 만들기 시작한다
        if let Some((_, paths, dir)) = self.menu_target(x, y) {
            self.pending.push(Deferred::PrepareMenu { paths, dir });
        }
        self.invalidate();
    }

    /// 우클릭 위치의 메뉴 대상: (패널, 선택 항목 경로 — 빈 곳이면 비어 있음, 현재 폴더)
    fn menu_target(&self, x: f32, y: f32) -> Option<(usize, Vec<String>, Option<String>)> {
        let (p, items) = match self.hit(x, y) {
            Hit::Row(p, _) => (p, true),
            Hit::ListEmpty(p) | Hit::Pane(p) | Hit::Header(p, _) => (p, false),
            _ => return None,
        };
        let tab = self.panes[p].tab();
        // 시작 화면에는 셸 메뉴를 띄울 폴더가 없다 (항목 메뉴는 따로)
        if tab.loc == Location::Start {
            return None;
        }
        let paths = if items { tab.selected_paths() } else { Vec::new() };
        Some((p, paths, tab.loc.dir().map(String::from)))
    }

    fn on_rbutton_up(&mut self, x: f32, y: f32) -> bool {
        if self.settings.is_some() || self.cmd.is_some() {
            return true;
        }
        if let Hit::PalRow(_) = self.hit(x, y) {
            let pt = self.to_screen(x, y);
            self.pending.push(Deferred::PalMenu { pt });
            return true;
        }
        if self.view_menu.is_some() {
            self.close_view_menu();
            return true;
        }
        if let Hit::Side(s) = self.hit(x, y) {
            self.side_context_menu(s, x, y);
            return true;
        }
        if let Hit::Start(p, i) = self.hit(x, y) {
            self.start_context_menu(p, i, x, y);
            return true;
        }
        let Some((pane, paths, dir)) = self.menu_target(x, y) else { return false };
        let pt = self.to_screen(x, y);
        self.pending.push(Deferred::Menu { pane, paths, dir, pt });
        true
    }

    fn on_mbutton_up(&mut self, x: f32, y: f32) {
        if self.settings.is_some() || self.cmd.is_some() {
            return;
        }
        let start_target = match self.hit(x, y) {
            Hit::Start(p, i) => self.start_target(p, i).map(|loc| (p, loc)),
            _ => None,
        };
        if self.fav_focus && !matches!(self.hit(x, y), Hit::Side(Side::Fav(_))) {
            self.fav_blur();
        }
        if let Hit::PalRow(i) = self.hit(x, y) {
            self.pal_activate(i, palette::OpenMode::NewTab);
            return;
        }
        if self.view_menu.is_some() {
            self.close_view_menu();
            return;
        }
        match self.hit(x, y) {
            Hit::Side(s @ (Side::Fav(crate::favorites::Row::Item { .. }) | Side::Computer | Side::Drive(_) | Side::Folder(_))) => self.side_open(s, true),
            Hit::Tab(p, t) | Hit::TabClose(p, t) => self.close_tab(p, t),
            Hit::Start(..) => {
                if let Some((p, loc)) = start_target {
                    self.start_go(p, loc, true, false);
                }
            }
            Hit::Row(p, vi) => {
                let tab = self.panes[p].tab();
                if let Some(path) = tab.entry_at(vi).filter(|e| e.is_dir()).and_then(|e| tab.path_of(e)) {
                    self.new_tab(p, Some(Location::Dir(path)), false);
                }
            }
            _ => {}
        }
    }

    fn on_wheel(&mut self, wp: WPARAM, lp: LPARAM) {
        let delta = hiword(wp.0) as i16 as f32;
        let mut pt = POINT { x: loword(lp.0 as usize) as i16 as i32, y: hiword(lp.0 as usize) as i16 as i32 };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }
        let (x, y) = (pt.x as f32 / self.scale(), pt.y as f32 / self.scale());
        if self.settings.is_some() {
            self.settings_wheel(-delta / 120.0 * self.wheel_lines * ROW_H);
            return;
        }
        // 명령 메뉴가 열려 있으면 그 목록만 움직인다
        if self.cmd.is_some() {
            if self.cmd_rect().is_some_and(|r| r.contains(x, y)) {
                self.cmd_wheel(-delta / 120.0 * self.wheel_lines * ROW_H);
            }
            return;
        }
        if self.pal_visible() && self.pal_list().contains(x, y) {
            self.pal_wheel(-delta / 120.0 * self.wheel_lines * ROW_H);
            return;
        }
        // 보기 메뉴가 열려 있으면 휠로 뒤쪽 목록이 움직이지 않게
        if self.view_menu.is_some() {
            return;
        }
        if self.side_panel_rect().is_some_and(|r| r.contains(x, y)) {
            self.side_wheel(-delta / 120.0 * self.wheel_lines * ROW_H);
            return;
        }
        if self.preview_rect().is_some_and(|r| r.contains(x, y)) {
            self.preview_wheel(-delta / 120.0 * self.wheel_lines * ROW_H);
            return;
        }
        let hit = self.hit(x, y);
        // 탭 줄에서 휠: 옆 탭으로 (탭이 많아 가려진 탭도 닿게)
        if let Hit::Tab(p, _) | Hit::TabClose(p, _) | Hit::TabStrip(p) | Hit::TabAdd(p) = hit {
            self.cycle_tab(p, delta > 0.0);
            self.invalidate();
            return;
        }
        let p = hit.pane().unwrap_or(self.active);
        // 아이콘 보기는 한 번에 한 줄, 목록 보기는 한 열(가로), 나머지는 세 줄
        let step = match self.grid(p) {
            _ if self.panes[p].tab().loc == Location::Start => self.wheel_lines * ROW_H,
            Some((_, g)) if g.horiz => g.cell_w,
            Some((_, g)) if matches!(g.cell, CellKind::Tile(_)) => g.cell_h,
            _ => self.wheel_lines * ROW_H,
        };
        let dy = -delta / 120.0 * step;
        if !sidebar::motion_on() || self.edit_mode == EditMode::Rename(p) {
            self.wheel = None;
            return self.scroll_by(p, dy);
        }
        let (id, cur) = (self.panes[p].tab().id, self.panes[p].tab().scroll);
        let base = match self.wheel {
            Some(w) if w.pane == p && w.tab == id && (w.last - cur).abs() < 0.5 => w.target,
            _ => cur,
        };
        // 목표도 목록 끝을 넘지 않게 (clamp_scroll로 잰다)
        self.panes[p].tab_mut().scroll = base + dy;
        self.clamp_scroll(p);
        let target = self.panes[p].tab().scroll;
        self.panes[p].tab_mut().scroll = cur;
        self.wheel = Some(Wheel { pane: p, tab: id, target, last: cur });
        unsafe { SetTimer(Some(self.hwnd), TIMER_WHEEL, 15, None) };
        self.on_wheel_timer();
    }

    /// 마우스가 칸 위에 그대로 있을 때도 커서 모양(기다림 표시)을 지금 상태로 맞춘다
    fn refresh_cursor(&self) {
        if !matches!(self.drag, Drag::None) || !unsafe { GetCapture() }.is_invalid() {
            return;
        }
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() || unsafe { WindowFromPoint(pt) } != self.hwnd {
            return;
        }
        let _ = unsafe { ScreenToClient(self.hwnd, &mut pt) };
        let (x, y) = (pt.x as f32 / self.scale(), pt.y as f32 / self.scale());
        if self.pane_rects().iter().any(|(_, r)| r.contains(x, y)) {
            self.on_set_cursor();
        }
    }

    fn on_set_cursor(&self) -> bool {
        let id = match self.drag {
            Drag::SplitX | Drag::Col { .. } | Drag::SideW { .. } => IDC_SIZEWE,
            Drag::SplitY => IDC_SIZENS,
            _ => {
                let mut pt = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut pt);
                    let _ = ScreenToClient(self.hwnd, &mut pt);
                }
                let (x, y) = (pt.x as f32 / self.scale(), pt.y as f32 / self.scale());
                let hit = self.hit(x, y);
                let under = self.pane_rects().into_iter().find(|(_, r)| r.contains(x, y)).map(|(q, _)| q);
                let waiting = !self.probes.is_empty()
                    || under.is_some_and(|q| !self.panes[q].tab().offline && self.panes[q].tab().loading_since.is_some_and(|s| s.elapsed().as_millis() >= LOADING_SHOW_MS));
                match hit {
                    Hit::SplitX | Hit::HeaderResize(..) | Hit::SideResize => IDC_SIZEWE,
                    Hit::SplitY => IDC_SIZENS,
                    Hit::CrumbEmpty(_) => IDC_IBEAM,
                    _ if waiting => IDC_APPSTARTING,
                    _ => IDC_ARROW,
                }
            }
        };
        unsafe {
            SetCursor(LoadCursorW(None, id).ok());
        }
        true
    }

    fn move_cursor_key(&mut self, p: usize, vk: VIRTUAL_KEY, shift: bool, ctrl: bool) {
        let page = self.page_rows(p);
        let grid = self.grid(p).map(|(_, g)| g);
        let cols = grid.map(|g| g.cols as isize).unwrap_or(1);
        let tab = self.panes[p].tab_mut();
        let n = tab.view.len();
        if n == 0 {
            return;
        }
        let cur = tab.cursor as isize;
        let last = n as isize - 1;
        // 목록 보기: 위·아래는 한 칸, 왼쪽·오른쪽은 옆 열로 (오른쪽 끝 열이 짧으면 마지막 항목으로)
        if let Some(g) = grid.filter(|g| g.horiz) {
            let rows = g.rows as isize;
            let to = match vk {
                VK_UP => cur - 1,
                VK_DOWN => cur + 1,
                VK_LEFT if cur < rows => cur,
                VK_LEFT => cur - rows,
                VK_RIGHT if cur + rows > last && cur / rows < last / rows => last,
                VK_RIGHT if cur + rows > last => cur,
                VK_RIGHT => cur + rows,
                VK_PRIOR => cur - page,
                VK_NEXT => cur + page,
                VK_HOME => 0,
                _ => last,
            };
            tab.move_cursor(to.clamp(0, last) as usize, shift, ctrl);
            self.ensure_visible(p);
            return;
        }
        let to = match vk {
            VK_UP if cur < cols => cur,
            VK_UP => cur - cols,
            // 마지막 줄이 덜 찼으면 아래 칸 대신 마지막 항목으로
            VK_DOWN if cur + cols > last && cur / cols < last / cols => last,
            VK_DOWN if cur + cols > last => cur,
            VK_DOWN => cur + cols,
            VK_LEFT => cur - 1,
            VK_RIGHT => cur + 1,
            VK_PRIOR => cur - page,
            VK_NEXT => cur + page,
            VK_HOME => 0,
            _ => last,
        };
        tab.move_cursor(to.clamp(0, last) as usize, shift, ctrl);
        self.ensure_visible(p);
    }

    fn escape(&mut self, p: usize) {
        if !self.panes[p].tab().filter.is_empty() {
            self.clear_filter();
        } else {
            self.panes[p].tab_mut().clear_selection();
        }
    }

    /// 단축키. 처리했으면 true
    /// 바꿀 수 있는 단축키에 매인 동작
    pub(super) fn run_action(&mut self, a: crate::keys::Action) {
        use crate::keys::Action;
        let p = self.active;
        match a {
            Action::Search => self.toggle_palette(),
            Action::Commands => self.toggle_commands(),
            Action::Favorites => self.toggle_fav_focus(),
            Action::AddFavorite => self.add_favorite(),
            Action::NewTab => self.new_tab(p, None, true),
            Action::CloseTab => {
                let t = self.panes[p].active;
                self.close_tab(p, t);
            }
            Action::ReopenTab => self.reopen_closed_tab(),
            Action::NewFolder => self.new_folder(p),
            Action::Hidden => self.toggle_hidden(),
            Action::Preview => self.toggle_preview(),
            Action::Undo => self.undo_last(),
            Action::Layout1 => self.set_layout(Layout::ALL[0]),
            Action::Layout2 => self.set_layout(Layout::ALL[1]),
            Action::Layout3 => self.set_layout(Layout::ALL[2]),
            Action::Layout4 => self.set_layout(Layout::ALL[3]),
            Action::Layout5 => self.set_layout(Layout::ALL[4]),
            Action::Thumbs => self.set_view(p, ViewMode::Large),
            Action::Huge => self.set_view(p, ViewMode::Huge),
            Action::Medium => self.set_view(p, ViewMode::Medium),
            Action::SmallIcons => self.set_view(p, ViewMode::Small),
            Action::List => self.set_view(p, ViewMode::List),
            Action::Details => self.set_view(p, ViewMode::Details),
            Action::Address => self.begin_address(p),
            Action::Rename => self.begin_rename(p),
            Action::Refresh => self.refresh(p),
            Action::Settings => self.toggle_settings(settings_ui::Page::General),
            Action::Help => self.toggle_settings(settings_ui::Page::Help),
        }
    }

    fn on_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        // 누르고 있어 반복되는 Enter·Delete·Backspace·F2는 한 번만 (폴더를 열자마자 그 안의 첫 항목을 열거나, 여러 단계 위로 가지 않게)
        if self.key_repeat && matches!(vk, VK_RETURN | VK_DELETE | VK_BACK | VK_F2) {
            return true;
        }
        if self.settings.is_some() {
            return self.settings_key(vk);
        }
        if self.view_menu.is_some() {
            return self.view_menu_key(vk);
        }
        if self.fav_focus {
            return self.fav_key(vk);
        }
        if let Some(a) = self.keys.find(crate::keys::Combo::now(vk.0)) {
            self.run_action(a);
            self.invalidate();
            return true;
        }
        // 시작 화면: 화살표는 카드·줄 사이를 옮기고, Enter는 고른 곳을 연다
        if self.panes[self.active].tab().loc == Location::Start && self.start_key(self.active, vk) {
            return true;
        }
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let alt = key_down(VK_MENU);
        let p = self.active;
        match vk {
            VK_TAB if ctrl => self.cycle_tab(p, shift),
            VK_TAB if !alt => self.cycle_pane(shift),
            VK_LEFT if alt => self.go_back(p),
            VK_RIGHT if alt => self.go_forward(p),
            VK_UP if alt => self.go_up(p),
            VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_HOME | VK_END if !alt => self.move_cursor_key(p, vk, shift, ctrl),
            VK_LEFT | VK_RIGHT if !alt && self.panes[p].view != ViewMode::Details => self.move_cursor_key(p, vk, shift, ctrl),
            VK_RETURN if alt => self.properties(p),
            VK_RETURN => self.open_items(p, ctrl),
            VK_BACK if !ctrl && !alt => self.go_up(p),
            // 경로 입력은 바꿀 수 있는 단축키 말고도 탐색기와 같은 F4·Alt+D로 늘 된다
            VK_F4 if !alt && !ctrl => self.begin_address(p),
            // Ctrl+F4는 탭 닫기 (탐색기·브라우저와 같음). Alt+F4는 여기서 처리하지 않아야 창이 닫힌다
            VK_F4 if ctrl && !alt => {
                let t = self.panes[p].active;
                self.close_tab(p, t);
            }
            VK_DELETE if !ctrl && !alt => self.delete(p, shift),
            VK_ESCAPE => self.escape(p),
            VK_APPS => self.context_menu_at_cursor(p),
            VK_F10 if shift => self.context_menu_at_cursor(p),
            _ if ctrl && !alt => {
                let c = vk.0 as u8;
                if vk.0 > 0xFF {
                    return false;
                }
                // 바꿀 수 없는 기본 조작 (나머지 Ctrl 단축키는 keys.rs의 키 맵에서 처리)
                match c {
                    b'A' => self.panes[p].tab_mut().select_all(),
                    b'C' if shift => self.copy_paths(p),
                    b'C' => self.copy(p, false),
                    b'X' => self.copy(p, true),
                    b'V' => self.paste(p),
                    _ => return false,
                }
            }
            _ if alt && vk.0 == b'D' as u16 => self.begin_address(p),
            _ => return false,
        }
        self.invalidate();
        true
    }

    /// 입력 상자에서 누른 키. 필터 상자는 글자 입력만 받고 탐색 키는 목록으로 넘긴다
    pub fn on_edit_key(&mut self, id: i32, vk: VIRTUAL_KEY) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let alt = key_down(VK_MENU);
        match id {
            ID_SEARCH => self.pal_key(vk),
            ID_CMD => self.cmd_key(vk),
            // 보기 메뉴가 열려 있거나 키보드가 즐겨찾기에 있으면 필터 상자의 키는 그쪽 조작에 쓴다
            ID_FILTER if self.view_menu.is_some() => self.view_menu_key(vk),
            ID_FILTER if self.fav_focus => self.fav_key(vk),
            ID_FILTER => {
                let len = unsafe { GetWindowTextLengthW(self.filter_edit) };
                let empty = len == 0;
                let (sel_start, sel_end) = edit_sel(self.filter_edit);
                let text_sel = sel_start != sel_end;
                // 필터 글자가 있어도 목록을 다루는 키: 처음·끝, 아이콘 보기의 좌우, 글자 끝에서 누른 Delete(파일 지우기)
                let list_key = match vk {
                    VK_HOME | VK_END => true,
                    VK_LEFT | VK_RIGHT => self.panes[self.active].view != ViewMode::Details,
                    VK_DELETE => !text_sel && sel_end as i32 >= len,
                    _ => false,
                };
                let text_key = !list_key && matches!(vk, VK_BACK | VK_DELETE | VK_HOME | VK_END | VK_LEFT | VK_RIGHT);
                let forward = match vk {
                    VK_ESCAPE if !empty => {
                        self.clear_filter();
                        return true;
                    }
                    // 빈 필터를 공백으로 시작하지 않는다
                    VK_SPACE if empty && !ctrl && !alt => return true,
                    _ if list_key && !alt => true,
                    _ if text_key && !empty && !alt => false,
                    // 필터를 건 채로도 Ctrl+A·Z는 목록(전체 고르기·되돌리기)에 쓴다. Ctrl+C·X는 필터 글자를 골랐을 때만 글자에,
                    // Ctrl+V는 클립보드에 파일이 있으면 목록(붙여넣기)에 쓴다
                    _ if ctrl && !empty && matches!(vk.0 as u8, b'C' | b'X') => !text_sel,
                    _ if ctrl && !empty && vk.0 as u8 == b'V' => shell::clipboard_has_files(),
                    VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_RETURN | VK_TAB | VK_ESCAPE | VK_APPS => true,
                    _ if (VK_F1.0..=VK_F24.0).contains(&vk.0) => true,
                    _ if text_key || ctrl || alt => true,
                    _ => false,
                };
                forward && self.on_key(vk)
            }
            ID_RENAME => match vk {
                VK_RETURN => {
                    self.commit_rename_with(true);
                    true
                }
                VK_TAB => {
                    self.commit_rename();
                    true
                }
                VK_ESCAPE => {
                    self.cancel_edit();
                    true
                }
                _ => false,
            },
            ID_ADDR => match vk {
                VK_RETURN => {
                    self.commit_address();
                    true
                }
                VK_ESCAPE | VK_TAB => {
                    self.cancel_edit();
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// 입력 상자가 포커스를 잃으면: 이름 바꾸기는 확정, 주소 입력은 취소
    pub fn on_edit_blur(&mut self, id: i32) {
        match (id, self.edit_mode) {
            (ID_RENAME, EditMode::Rename(_) | EditMode::FavRename(_) | EditMode::WorkspaceRename) => self.commit_rename(),
            (ID_ADDR, EditMode::Address(_)) => self.cancel_edit(),
            _ => {}
        }
    }

    // ---------- 그리기 ----------

    fn paint(&mut self) {
        // 목록이 마우스 아래에서 밀리면(휠·키보드 스크롤, 새로 고침) 강조된 줄이 엉뚱한 곳에 남지 않게 지금 자리로 맞춘다
        if matches!(self.drag, Drag::None) && matches!(self.hover, Hit::Row(..)) {
            let mut pt = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(self.hwnd, &mut pt);
            }
            let h = self.hit(pt.x as f32 / self.scale(), pt.y as f32 / self.scale());
            if h != self.hover {
                self.hover = h;
            }
        }
        self.sync_preview();
        self.place_filter_edit(false);
        if !self.gfx.begin(self.hwnd) {
            return;
        }
        self.gfx.clear(self.theme.window);
        // 설정 창은 창 전체를 덮으니 그 아래는 그리지 않는다
        if self.settings.is_some() {
            self.paint_settings();
            self.paint_caption();
            self.gfx.end();
            return;
        }
        self.paint_bar();
        self.paint_workbar();
        self.paint_caption();
        let mut missing = Vec::new();
        for (p, r) in self.pane_rects() {
            self.paint_pane(p, r, &mut missing);
        }
        self.paint_preview(&mut missing);
        // 패널은 칸 다음에: 떠 있을 때 칸 위에 그려진다
        self.paint_sidebar();
        let t = self.theme;
        let (sx, sy) = self.splitters();
        let hot_x = matches!(self.drag, Drag::SplitX) || self.hover == Hit::SplitX;
        let hot_y = matches!(self.drag, Drag::SplitY) || self.hover == Hit::SplitY;
        if let (Some(r), true) = (sx, hot_x) {
            let cx = (r.l + r.r) / 2.0;
            self.gfx.fill_round(R::new(cx - 1.5, r.t + 8.0, cx + 1.5, r.b - 8.0), 1.5, t.focus);
        }
        if let (Some(r), true) = (sy, hot_y) {
            let cy = (r.t + r.b) / 2.0;
            self.gfx.fill_round(R::new(r.l + 8.0, cy - 1.5, r.r - 8.0, cy + 1.5), 1.5, t.focus);
        }
        let hot_side = matches!(self.drag, Drag::SideW { .. }) || self.hover == Hit::SideResize;
        if let (Some(r), true) = (self.side_resize_rect(), hot_side) {
            let cx = (r.l + r.r) / 2.0;
            self.gfx.fill_round(R::new(cx - 1.5, r.t + 8.0, cx + 1.5, r.b - 8.0), 1.5, t.focus);
        }
        if self.view_menu.is_some() {
            self.paint_view_menu();
        }
        self.paint_toasts();
        self.paint_commands();
        self.paint_tip();
        self.paint_fav_card(&mut missing);
        self.gfx.end();
        self.request_thumbs(missing);
    }

    /// 알림: 칸 아래쪽 가운데 어두운 알약 (몇 초 뒤 사라진다).
    /// 검색 패널·보기 메뉴보다 위에 그려, 그 위에서 한 일(검색 결과 복사 등)의 결과도 보이게 한다
    fn paint_toasts(&self) {
        let t = self.theme;
        let g = &self.gfx;
        for (p, _) in self.pane_rects() {
            let (Some(m), Some((tr, btn))) = (&self.panes[p].tab().message, self.toast_rects(p)) else { continue };
            let pushed = anim_progress(self.toast_at, TOAST_FADE_MS).is_some_and(|f| g.push_fade(f, 0.0, (1.0 - f) * 8.0));
            let (bg, fg) = if t.high_contrast {
                (t.sel, t.sel_text)
            } else if t.dark {
                (Color { r: 0.92, g: 0.93, b: 0.94, a: 1.0 }, Color { r: 0.12, g: 0.14, b: 0.16, a: 1.0 })
            } else {
                (Color { r: 0.12, g: 0.14, b: 0.16, a: 1.0 }, Color { r: 0.95, g: 0.95, b: 0.96, a: 1.0 })
            };
            let radius = tr.h() / 2.0;
            if !t.high_contrast {
                soft_shadow(g, tr, radius, 10.0, 0.18, t.dark);
            }
            g.fill_round(tr, radius, bg);
            if t.high_contrast {
                g.stroke_round(tr.inset(0.5, 0.5), radius, t.border, 1.0);
            }
            g.text(m, R::new(tr.l + 16.0, tr.t, btn.map_or(tr.r - 14.0, |b| b.l - 8.0), tr.b), Font::Normal, fg);
            if let Some(b) = btn {
                let hot = self.hover == Hit::ToastUndo(p);
                g.fill_round(b, b.h() / 2.0, alpha(fg, if hot { 0.26 } else { 0.14 }));
                let w = g.measure(TOAST_UNDO, Font::Normal);
                g.text(TOAST_UNDO, R::new(b.l + (b.w() - w) / 2.0, b.t, b.r, b.b), Font::Normal, fg);
            }
            if pushed {
                g.pop_fade();
            }
        }
    }

    /// 칸 영역 위 작업 줄: 칸 나누기, 숨김 항목, 미리 보기. 창 단추는 paint_caption
    fn paint_workbar(&self) {
        self.paint_workspace_button();
        let t = self.theme;
        let g = &self.gfx;
        // 판(well)과 같은 색이면 올려도 보이지 않으니 반투명으로 겹친다
        let hover_bg = sidebar::hover_fill(&t);
        // 칸 나누기: 한 덩어리 판 위에 지금 고른 것만 도드라지게
        let pill = self.layout_pill();
        g.fill_round(pill, pill.h() / 2.0, sidebar::seg_track(&t));
        for (i, r) in self.layout_buttons().iter().enumerate() {
            let layout = Layout::ALL[i];
            let current = layout == self.layout;
            if current {
                sidebar::seg_on(g, *r, r.h() / 2.0, &t);
            } else if self.hover == Hit::Layout(i) {
                g.fill_round(*r, r.h() / 2.0, hover_bg);
            }
            draw_layout_icon(g, *r, layout, if current || self.hover == Hit::Layout(i) { t.text } else { t.dim });
        }
        // 글자 없는 버튼: 올려 두면 이름과 단축키를 말풍선으로 보여 준다.
        // 쉴 때는 칸 나누기 그림과 같은 회색이라 아이콘마다 진하기가 달라 보이지 않는다
        let icon_button = |r: R, glyph: u16, on: bool, hovered: bool| {
            if on {
                sidebar::seg_on(g, r, R_CTRL, &t);
            } else if hovered {
                g.fill_round(r, R_CTRL, hover_bg);
            }
            g.glyph(glyph, r, false, if on || hovered { t.text } else { t.dim });
        };
        icon_button(self.hidden_button(), G_HIDDEN, self.show_hidden, self.hover == Hit::Hidden);
        icon_button(self.preview_button(), G_PREVIEW, self.preview.is_some(), self.hover == Hit::PreviewButton);
        icon_button(self.more_button(), G_MORE, false, self.hover == Hit::More);
    }

    /// 단추에 잠시 올려 두면 보여 줄 (버튼 자리, 이름과 단축키)
    fn tip(&self) -> Option<(R, String)> {
        if self.view_menu.is_some() || self.settings.is_some() || self.cmd.is_some() || !matches!(self.drag, Drag::None) {
            return None;
        }
        use crate::keys::Action;
        const LAYOUTS: [Action; 5] = [Action::Layout1, Action::Layout2, Action::Layout3, Action::Layout4, Action::Layout5];
        // "이름 (단축키)". 단축키를 비워 두었으면 이름만
        let named = |a: Action, name: &str| match self.keys.get(a) {
            Some(c) => format!("{name} ({})", c.label()),
            None => name.to_string(),
        };
        Some(match self.hover {
            Hit::Layout(i) => (self.layout_buttons()[i], named(LAYOUTS[i], crate::keys::name(LAYOUTS[i]))),
            Hit::NewFolder(p) => (geom(self.pane_rect(p)?).new_folder, named(Action::NewFolder, "새 폴더")),
            Hit::Back(p) => (geom(self.pane_rect(p)?).back, "뒤로 (Alt+←)".to_string()),
            Hit::Fwd(p) => (geom(self.pane_rect(p)?).fwd, "앞으로 (Alt+→)".to_string()),
            Hit::Up(p) => (geom(self.pane_rect(p)?).up, "위로 (Alt+↑)".to_string()),
            Hit::Hidden => (self.hidden_button(), named(Action::Hidden, if self.show_hidden { "숨김 항목 감추기" } else { "숨김 항목 보기" })),
            Hit::Side(s) => return self.side_tip(s),
            Hit::SearchButton => (
                self.search_button(),
                match &self.hotkey {
                    Some(hk) => format!("{} · 다른 앱에서는 {hk}", named(Action::Search, "검색")),
                    None => named(Action::Search, "검색"),
                },
            ),
            Hit::PreviewButton => (self.preview_button(), named(Action::Preview, "미리 보기 창")),
            Hit::SettingsButton => (self.settings_button(), named(Action::Settings, "설정")),
            Hit::Bar(p) if self.side_panel() == Some(p) => return None,
            Hit::Bar(Panel::Favorites) => (self.bar_rect(Panel::Favorites), named(Action::Favorites, "즐겨찾기")),
            Hit::Bar(p) => (self.bar_rect(p), p.title().to_string()),
            Hit::SidePin => (self.side_pin_rect()?, (if self.side_floating() { "고정해 두기" } else { "고정 풀기" }).to_string()),
            Hit::More => (self.more_button(), "도움말·설정".to_string()),
            Hit::PalClose => (self.pal_close_rect(), "검색 닫기".to_string()),
            Hit::Start(p, i) => return self.start_tip(p, i),
            Hit::PreviewPin | Hit::PreviewOpen => return self.preview_tip(),
            Hit::Workspace => (self.workspace_button(), "작업 공간 · 지금 화면을 저장하고 바꾸기".to_string()),
            _ => return None,
        })
    }

    /// 버튼 위에 마우스가 바뀌면 말풍선을 숨기고, 잠시 머무르면 보이게 한다
    fn update_tip(&mut self) {
        // 말풍선이 떠 있던 채로 옆 단추로 옮기면 곧바로 보인다 (Windows 말풍선의 다시 보이기와 같다)
        let follow = std::mem::replace(&mut self.tip_shown, false);
        unsafe {
            if self.tip().is_some() {
                SetTimer(Some(self.hwnd), TIMER_TIP, if follow { 100 } else { 600 }, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_TIP);
            }
        }
    }

    fn paint_tip(&self) {
        // 즐겨찾기 항목은 말풍선 대신 정보 카드 (sidebar::paint_fav_card)
        if matches!(self.hover, Hit::Side(Side::Fav(crate::favorites::Row::Item { .. }))) {
            return;
        }
        let Some((b, text)) = self.tip().filter(|_| self.tip_shown) else { return };
        let t = self.theme;
        let (label, key) = split_tip_key(&text);
        let key_w = key.map_or(0.0, |k| 8.0 + self.gfx.measure(k, Font::Small) + 10.0);
        let w = (self.gfx.measure(label, Font::Normal) + 20.0 + key_w).min((self.w - MARGIN * 2.0).max(40.0));
        // 막대 아이콘은 오른쪽에(아래쪽 설정이 창 밖으로 나가지 않게), 작업 줄 단추는 아래에.
        // 창 아래쪽 가까이(시작 화면 카드, 패널 마지막 줄)에서는 단추 위로 올린다
        let r = if b.r <= BAR_W {
            let cy = ((b.t + b.b) / 2.0).clamp(MARGIN + 14.0, (self.h - MARGIN - 14.0).max(MARGIN + 14.0));
            R::new(b.r + 8.0, cy - 14.0, b.r + 8.0 + w, cy + 14.0)
        } else {
            let l = ((b.l + b.r) / 2.0 - w / 2.0).clamp(MARGIN, (self.w - MARGIN - w).max(MARGIN));
            if b.b + 36.0 > self.h - MARGIN { R::new(l, b.t - 36.0, l + w, b.t - 8.0) } else { R::new(l, b.b + 8.0, l + w, b.b + 36.0) }
        };
        let (bg, fg) = match () {
            _ if t.high_contrast => (t.pane, t.text),
            _ if t.dark => (Color { r: 0.24, g: 0.25, b: 0.27, a: 1.0 }, t.text),
            _ => (t.text, Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }),
        };
        let fade = anim_progress(self.tip_at, TIP_FADE_MS);
        let pushed = fade.is_some_and(|f| {
            let off = (1.0 - f) * 4.0;
            if b.r <= BAR_W { self.gfx.push_fade(f, -off, 0.0) } else { self.gfx.push_fade(f, 0.0, -off) }
        });
        if !t.high_contrast {
            soft_shadow(&self.gfx, r, R_CTRL, 8.0, 0.14, t.dark);
        }
        self.gfx.fill_round(r, R_CTRL, bg);
        if t.high_contrast {
            self.gfx.stroke_round(r.inset(0.5, 0.5), R_CTRL, t.border, 1.0);
        }
        self.gfx.text(label, R::new(r.l + 10.0, r.t, r.r - 6.0 - key_w, r.b), Font::Normal, fg);
        if let Some(k) = key {
            let kr = R::new(r.r - 10.0 - (key_w - 8.0), r.t + 4.0, r.r - 10.0, r.b - 4.0);
            self.gfx.fill_round(kr, 4.0, alpha(fg, 0.16));
            self.gfx.text(k, R::new(kr.l + 5.0, kr.t, kr.r, kr.b), Font::Small, fg);
        }
        if pushed {
            self.gfx.pop_fade();
        }
    }

    fn paint_pane(&mut self, p: usize, r: R, missing: &mut Vec<crate::thumbs::Request>) {
        let t = self.theme;
        let active = p == self.active;
        let multi = self.layout.count() > 1;
        let hover = self.hover;
        let drop_hover = self.drop_hover;
        let edit_mode = self.edit_mode;
        // 키보드가 사이드바 즐겨찾기에 있으면 칸의 포커스 테두리는 그리지 않는다 (포커스 표시는 한 곳에만)
        let keyboard_here = active && !self.fav_focus;
        let thumb_drag = matches!(self.drag, Drag::Thumb { pane, .. } if pane == p);
        // 끌고 있는 탭: (그 탭이 있던 칸, 탭), 놓을 곳 (칸, 위치)
        let (dragged_tab, tab_drop) = match self.drag {
            Drag::TabMove { pane, tab, x, y } => (Some((pane, tab)), self.tab_drop_target(x, y)),
            _ => (None, None),
        };
        // 경로 오른쪽 즐겨찾기 별: 폴더일 때만 (켜짐 여부)
        let view_menu_open = self.view_menu.is_some_and(|m| m.pane == p);
        let starred = self.panes[p].tab().loc.dir().map(|d| self.favs.find(&util::normalize(d)).is_some());
        // 네모 상자 고르기 (화면 좌표)
        let marquee = match self.drag {
            Drag::Marquee { pane, x0, y0, x, y } if pane == p => {
                let scroll = self.panes[p].tab().scroll;
                let (x0, y0) = if self.grid(p).is_some_and(|(_, g)| g.horiz) { (x0 - scroll, y0) } else { (x0, y0 - scroll) };
                Some(R::new(x0.min(x), y0.min(y), x0.max(x), y0.max(y)))
            }
            _ => None,
        };
        let g = geom(r);
        let (trs, add) = self.chips(p, &g);
        let count_text = self.head_count_text(p);
        let right = self.head_right_rect(p, &g);
        let fchip = filter_chip(&g);
        let (fedit, fcount, fclear) = self.filter_parts(p, fchip);
        let fcount_text = self.filter_count_text(p);
        // 팝업이 열려 있으면 입력 상자(자식 창)는 화면 밖에 있으니 필터 글자를 직접 그린다
        let edit_in_chip = active && !(self.view_menu.is_some() || self.settings.is_some());
        // 시작 화면이면 머리글·목록 대신 그릴 카드와 줄
        let start = self.start_view(p);
        let folder_icon = if start.is_some() { self.type_icon("", true) } else { -1 };
        let start_focus = (keyboard_here && self.start_keys).then(|| self.panes[p].tab().cursor);
        // 목록의 수정한 날짜를 짧게(오늘·어제) 쓸지와 그 기준 시각
        let (short_dates, now) = (self.look.dates != "full", crate::listing::now_filetime());
        let motion = sidebar::motion_on();
        let net_btn = self.net_button(p);
        let App { gfx, panes, types, thumb_failed, places, .. } = self;
        let pane = &panes[p];
        let tab = pane.tab();

        // 카드: 밝은 테마는 테두리 없이 옅은 그림자로 떠 있고, 지금 칸은 조금 더 높이 뜬다 (고대비는 테두리로)
        if active && multi && !t.high_contrast {
            soft_shadow(gfx, r, RADIUS, 12.0, 0.10, t.dark);
        } else if !t.high_contrast && !t.dark {
            soft_shadow(gfx, r, RADIUS, 6.0, 0.06, false);
        }
        gfx.fill_round(r, RADIUS, t.pane);
        // 지금 칸: 위쪽 가장자리 안쪽에 얇은 강조 선 (칸이 여럿일 때 어느 칸에서 일하는지 한눈에)
        if active && multi {
            gfx.fill_round(R::new(r.l + RADIUS, r.t + 1.0, r.r - RADIUS, r.t + 3.0), 1.0, if t.high_contrast { t.focus } else { t.accent });
        }

        // 칩 줄: 탭 칩. 지금 탭은 바탕이 있는 칩(지금 칸이면 옅은 강조 색), 나머지는 글자만이고 올리면 옅은 바탕.
        // 오른쪽 자리(필터 칩·개수) 앞까지만 그린다 (좁은 칸에서 겹치면 누르기도 오른쪽 자리가 먼저)
        gfx.clip(R::new(r.l + 1.0, g.head.t, (right.l - 4.0).max(r.l + 1.0), g.head.b));
        let hovered_tab = match hover {
            Hit::Tab(hp, ht) | Hit::TabClose(hp, ht) if hp == p => Some(ht),
            _ => None,
        };
        for (i, tr) in trs.iter().enumerate() {
            // 칸이 좁아 가려진 탭 (휠·Ctrl+Tab으로 닿는다)
            if tr.w() <= 0.0 {
                continue;
            }
            let current = i == pane.active;
            let hovered = hovered_tab == Some(i);
            if current {
                gfx.fill_round(*tr, tr.h() / 2.0, sidebar::well(&t));
                // 고대비에서는 바탕색만으로 구분이 어려우니 테두리를 더한다
                if t.high_contrast {
                    gfx.stroke_round(tr.inset(0.5, 0.5), tr.h() / 2.0, if active { t.focus } else { t.pane_border }, 1.0);
                }
            } else if hovered {
                gfx.fill_round(*tr, tr.h() / 2.0, t.hover);
            }
            let closable = pane.tabs.len() > 1 && (current || hovered);
            let label = R::new(tr.l + 12.0, tr.t, tr.r - if closable { 26.0 } else { 10.0 }, tr.b);
            let fg = t.text;
            let color = if dragged_tab == Some((p, i)) { alpha(fg, 0.5) } else { fg };
            gfx.text(&pane.tabs[i].loc.title(), label, Font::Normal, color);
            if closable {
                let cr = close_rect(*tr);
                if hover == Hit::TabClose(p, i) {
                    gfx.fill_round(cr, cr.h() / 2.0, alpha(t.text, 0.12));
                }
                gfx.glyph(G_CLOSE, cr, true, if hover == Hit::TabClose(p, i) { t.text } else { t.dim });
            }
        }
        // 새 탭(+): 평소엔 흐리게, 올리면 옅은 바탕
        if hover == Hit::TabAdd(p) {
            gfx.fill_round(add, add.h() / 2.0, t.hover);
        }
        gfx.glyph_font(G_ADD, add, Font::GlyphMid, if hover == Hit::TabAdd(p) { t.text } else { t.dim });
        // 끌어 놓을 자리
        if let Some((_, at)) = tab_drop.filter(|(q, _)| *q == p) {
            let x = trs.get(at).map(|tr| tr.l - 2.0).or(trs.last().map(|tr| tr.r + 2.0)).unwrap_or(g.head.l + 8.0);
            gfx.fill_round(R::new(x - 1.5, add.t, x + 1.5, add.b), 1.5, t.accent);
        }
        gfx.unclip();

        // 칩 줄 오른쪽: 필터 칩(글자를 치면), 아니면 항목 수·고른 개수
        if tab.filter.is_empty() {
            gfx.text(&count_text, right, Font::Small, t.faint);
        } else {
            gfx.fill_round(fchip, fchip.h() / 2.0, t.accent_soft);
            if t.high_contrast {
                gfx.stroke_round(fchip.inset(0.5, 0.5), fchip.h() / 2.0, t.border, 1.0);
            }
            gfx.glyph(G_FILTER, R::new(fchip.l + 8.0, fchip.t, fchip.l + 26.0, fchip.b), true, t.chip_text);
            // 지금 칸에서는 입력 상자(자식 창)가 이 자리에 글자를 그린다
            if !edit_in_chip {
                gfx.text(&tab.filter, fedit, Font::Normal, t.text);
            }
            let cw = gfx.measure(&fcount_text, Font::Small);
            gfx.text(&fcount_text, R::new(fcount.r - cw - 1.0, fcount.t, fcount.r, fcount.b), Font::Small, t.dim);
            if hover == Hit::FilterClear(p) {
                gfx.fill_round(fclear, fclear.h() / 2.0, alpha(t.text, 0.12));
            }
            gfx.glyph(G_CLOSE, fclear, true, t.dim);
        }

        // 경로 줄: 탐색 단추
        let off = if t.high_contrast { t.faint } else { alpha(t.dim, 0.45) };
        let nav_btn = |gfx: &Gfx, rr: R, glyph: u16, font: Font, enabled: bool, hovered: bool| {
            if rr.w() <= 0.0 {
                return;
            }
            if hovered && enabled {
                gfx.fill_round(rr, R_CTRL, t.hover);
            }
            gfx.glyph_font(glyph, rr, font, if !enabled { off } else if hovered { t.text } else { t.dim });
        };
        // 이동 화살표는 가운데 크기(12px), 새 폴더·별·보기는 도구 아이콘 크기(16px)
        nav_btn(gfx, g.back, G_BACK, Font::GlyphMid, !tab.back.is_empty(), hover == Hit::Back(p));
        nav_btn(gfx, g.fwd, G_FWD, Font::GlyphMid, !tab.fwd.is_empty(), hover == Hit::Fwd(p));
        nav_btn(gfx, g.up, G_UP, Font::GlyphMid, matches!(tab.loc, Location::Dir(_)), hover == Hit::Up(p));
        // 시작 화면에는 새 폴더·보기 단추가 없다. 좁은 칸의 경로 입력 상자가 그 자리를 덮으면 그리지 않는다
        if start.is_none() && !(edit_mode == EditMode::Address(p) && g.crumb.w() < ADDR_MIN_W) {
            nav_btn(gfx, g.new_folder, G_NEW_FOLDER, Font::Glyph, tab.loc.dir().is_some(), hover == Hit::NewFolder(p));
            // 보기 방식: 지금 보기 그림과 아래 화살표 (누르면 보기 메뉴)
            if hover == Hit::View(p) || view_menu_open {
                gfx.fill_round(g.view, R_CTRL, t.hover);
            }
            let view_glyph = if matches!(pane.view, ViewMode::Huge | ViewMode::Large | ViewMode::Medium) { G_GRID } else { G_LIST };
            let view_hot = hover == Hit::View(p) || view_menu_open;
            gfx.glyph(view_glyph, R::new(g.view.l + 2.0, g.view.t, g.view.l + 26.0, g.view.b), false, if view_hot { t.text } else { t.dim });
            gfx.glyph(G_CHEVRON_DOWN, R::new(g.view.r - 16.0, g.view.t + 1.0, g.view.r - 4.0, g.view.b + 1.0), true, t.faint);
        }

        // 경로: 상자 없이 글자만. 경로 위에 올리면 옅은 바탕(빈 곳을 누르면 경로 입력), 조각에 올리면 그 조각이 한 단계 진하게
        if edit_mode == EditMode::Address(p) {
            let ar = addr_rect(&g);
            gfx.fill_round(ar, R_CTRL, t.field);
            gfx.stroke_round(ar.inset(0.75, 0.75), R_CTRL, t.accent, 1.5);
        } else {
            let over_path = matches!(hover, Hit::CrumbEmpty(hp) | Hit::Crumb(hp, _) if hp == p);
            if over_path {
                gfx.fill_round(g.crumb, R_CTRL, t.hover);
            }
            let cs = crumbs(gfx, g.crumbs, &tab.loc, Some("내 PC"));
            let n = cs.len();
            // 별 단추: 이 폴더가 즐겨찾기에 있으면 채운 별(강조 색), 없으면 빈 별
            if let Some(on) = starred.filter(|_| g.star.w() > 0.0) {
                let hot = hover == Hit::Star(p);
                if hot {
                    gfx.fill_round(g.star, R_CTRL, t.hover);
                }
                gfx.glyph(if on { G_STAR_FILL } else { G_STAR }, g.star, false, if on || hot { t.text } else { t.dim });
            }
            gfx.clip(g.crumbs);
            for (i, c) in cs.iter().enumerate() {
                let last = i + 1 == n;
                if hover == Hit::Crumb(p, i) {
                    gfx.fill_round(R::new(c.x0, g.crumb.t + 2.0, c.x1, g.crumb.b - 2.0), R_ROW, t.sel_inactive);
                }
                if c.label == "…" && !last {
                    gfx.glyph(G_MORE, R::new(c.x0, g.crumb.t, c.x1, g.crumb.b), true, t.dim);
                } else {
                    gfx.text(
                        &c.label,
                        R::new(c.x0 + CRUMB_PAD, g.crumb.t, c.x1 - CRUMB_PAD / 2.0, g.crumb.b),
                        Font::Normal,
                        if last { t.text } else { t.dim },
                    );
                }
                if !last {
                    gfx.glyph(G_CHEVRON, R::new(c.x1, g.crumb.t, c.x1 + CHEV_W, g.crumb.b), true, t.faint);
                }
            }
            gfx.unclip();
        }

        // 테두리: 모든 칸이 같은 옅은 선. 끌어 놓을 대상(파일, 다른 칸에서 끌어 온 탭)일 때만 강조 색으로.
        // 고대비에서는 모든 칸을 2px로 두고 활성 칸만 포커스 색으로 바꿔 두께는 그대로 둔다
        let tab_target = tab_drop.is_some_and(|(q, _)| q == p) && dragged_tab.is_some_and(|(from, _)| from != p);
        let dropping = tab_target || matches!(drop_hover, Some((dp, None)) if dp == p);
        let border_w = if t.high_contrast || dropping { 2.0 } else if t.dark { 1.0 } else { 0.0 };
        let border_c = match () {
            _ if dropping => t.accent,
            _ if t.high_contrast && active && multi => t.focus,
            _ => t.pane_border,
        };

        // 시작 화면: 머리글·목록 자리에 카드와 줄
        if let Some(sv) = &start {
            let hovered = match hover {
                Hit::Start(hp, i) if hp == p => Some(i),
                _ => None,
            };
            sv.paint(gfx, &t, places, folder_icon, hovered, start_focus);
            if border_w > 0.0 {
                gfx.stroke_round(r.inset(border_w / 2.0, border_w / 2.0), RADIUS, border_c, border_w);
            }
            return;
        }

        // 열 머리글
        let cols = columns(g.list, pane.cols);
        let header = |gfx: &Gfx, (l, rr): (f32, f32), label: &str, key: SortKey, right: bool| {
            let sorted = pane.sort.key == key;
            let color = if sorted || hover == Hit::Header(p, key) { t.text } else { t.dim };
            let tr = R::new(l + 8.0, g.header.t, rr - 8.0, g.header.b);
            gfx.text(label, tr, if right { Font::Right } else { Font::Normal }, color);
            if sorted {
                let glyph = if pane.sort.desc { G_SORT_DOWN } else { G_SORT_UP };
                let gx = if right { l } else { (tr.l + gfx.measure(label, Font::Normal) + 4.0).min(rr - 18.0) };
                gfx.glyph(glyph, R::new(gx, g.header.t, gx + 16.0, g.header.b), true, t.dim);
            }
        };
        if pane.view == ViewMode::Details {
            header(gfx, (cols.name.0 - 4.0, cols.name.1), "이름", SortKey::Name, false);
            if let Some(c) = cols.modified {
                header(gfx, c, "수정한 날짜", SortKey::Modified, false);
            }
            if let Some(c) = cols.kind {
                header(gfx, c, "유형", SortKey::Type, false);
            }
            if let Some(c) = cols.size {
                header(gfx, c, "크기", SortKey::Size, true);
            }
            gfx.fill(R::new(g.list.l + 8.0, g.header.b - 1.0, g.list.r - SCROLL_W - 6.0, g.header.b), t.border);
        } else {
            // 아이콘·목록 보기: 열 대신 정렬 버튼. 지금 기준은 바탕이 있는 버튼으로 보인다
            for (key, label, cr) in sort_chips(gfx, g.header, pane.sort.key) {
                let sorted = pane.sort.key == key;
                let hot = hover == Hit::Header(p, key);
                if sorted {
                    gfx.fill_round(cr, cr.h() / 2.0, sidebar::well(&t));
                    if t.high_contrast {
                        gfx.stroke_round(cr.inset(0.5, 0.5), cr.h() / 2.0, t.border, 1.0);
                    }
                } else if hot {
                    gfx.fill_round(cr, cr.h() / 2.0, t.hover);
                }
                let tr = R::new(cr.l + 12.0, cr.t, cr.r - if sorted { 28.0 } else { 12.0 }, cr.b);
                gfx.text(label, tr, Font::Normal, if sorted || hot { t.text } else { t.dim });
                if sorted {
                    let glyph = if pane.sort.desc { G_SORT_DOWN } else { G_SORT_UP };
                    gfx.glyph(glyph, R::new(cr.r - 26.0, cr.t, cr.r - 10.0, cr.b), true, t.dim);
                }
            }
        }

        // 행·칸: 보이는 부분만 그린다
        gfx.clip(g.list);
        let grid = Grid::of(g.list, pane.view);
        let thumb_size = if let CellKind::Tile(s) = grid.cell { s } else { THUMB };
        let thumb_px = (thumb_size * gfx.dpi / 96.0).round() as u32;
        // 여러 개를 골랐으면 그중 포커스가 어느 것인지도 보인다
        let many_selected = tab.selection_stats().0 > 1;
        for vi in grid.visible(g.list, tab.scroll, tab.view.len()) {
            let e = &tab.entries[tab.view[vi] as usize];
            let it = grid.item(vi, tab.scroll);
            // 선택(바탕)과 키보드 포커스(테두리)는 따로 보인다. 포커스는 활성 칸에만, 선택한 하나와 겹치면 바탕만으로
            let focused = keyboard_here && vi == tab.cursor && (!e.selected || many_selected);
            let fg = if e.selected && active { t.sel_text } else { t.text };
            if grid.cell == CellKind::Icon {
                // 작은 아이콘·목록: 칸 하나에 작은 아이콘과 이름
                let cell = R::new(it.l + 1.0, it.t + 1.0, it.r - 3.0, it.b - 1.0);
                if e.selected {
                    gfx.fill_round(cell, R_ROW, if active { t.sel } else { t.sel_inactive });
                } else if hover == Hit::Row(p, vi) {
                    gfx.fill_round(cell, R_ROW, t.hover);
                }
                if drop_hover == Some((p, Some(vi))) {
                    gfx.stroke_round(cell.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                } else if focused {
                    gfx.stroke_round(cell.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
                }
                let (_, type_icon) = type_info(types, e);
                let iy = it.t + (ROW_H - 16.0) / 2.0;
                gfx.icon(if e.icon >= 0 { e.icon } else { type_icon }, R::new(it.l + 4.0, iy, it.l + 20.0, iy + 16.0));
                if edit_mode == EditMode::Rename(p) && vi == tab.cursor {
                    let er = R::new(it.l + 24.0, it.t + 2.0, it.r - 6.0, it.b - 2.0);
                    gfx.fill_round(er, R_ROW, t.pane);
                    gfx.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                } else {
                    let fg = if e.is_hidden() && !(e.selected && active) { t.faint } else { fg };
                    gfx.text_w(e.name(), R::new(it.l + 28.0, it.t, it.r - 8.0, it.b), Font::Normal, fg);
                }
                continue;
            }
            if let CellKind::Tile(thumb) = grid.cell {
                let tile = R::new(it.l + 4.0, it.t + 4.0, it.r - 4.0, it.b - 4.0);
                if e.selected {
                    gfx.fill_round(tile, R_ROW, if active { t.sel } else { t.sel_inactive });
                } else if hover == Hit::Row(p, vi) {
                    gfx.fill_round(tile, R_ROW, t.hover);
                }
                if drop_hover == Some((p, Some(vi))) {
                    gfx.stroke_round(tile.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                } else if focused {
                    gfx.stroke_round(tile.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
                }
                let cx = (it.l + it.r) / 2.0;
                let bx = R::new(cx - thumb / 2.0, it.t + 8.0, cx + thumb / 2.0, it.t + 8.0 + thumb);
                let opacity = if e.is_hidden() { 0.55 } else { 1.0 };
                let drawn = match tab.path_of(e) {
                    Some(path) => {
                        let key = thumb_key(&path, e.modified, thumb_px);
                        let ok = gfx.thumb(&key, bx, opacity);
                        if !ok && !thumb_failed.contains(&key) {
                            missing.push(crate::thumbs::Request { key, path, px: thumb_px });
                        }
                        ok
                    }
                    None => false,
                };
                if !drawn {
                    // 썸네일이 오기 전까지는 작은 아이콘
                    let (_, type_icon) = type_info(types, e);
                    let my = (bx.t + bx.b) / 2.0;
                    gfx.icon(if e.icon >= 0 { e.icon } else { type_icon }, R::new(cx - 16.0, my - 16.0, cx + 16.0, my + 16.0));
                }
                let cap = R::new(tile.l + 4.0, bx.b + 6.0, tile.r - 4.0, tile.b - 2.0);
                if edit_mode == EditMode::Rename(p) && vi == tab.cursor {
                    let er = R::new(it.l + 4.0, bx.b + 6.0, it.r - 8.0, bx.b + 30.0);
                    gfx.fill_round(er, R_ROW, t.pane);
                    gfx.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                } else {
                    gfx.text_w(e.name(), cap, Font::Caption, if e.is_hidden() { t.faint } else { fg });
                }
                continue;
            }
            let y = it.t;
            let row = R::new(g.list.l + 8.0, y + 1.0, g.list.r - SCROLL_W - 6.0, y + ROW_H - 1.0);
            if e.selected {
                gfx.fill_round(row, R_ROW, if active { t.sel } else { t.sel_inactive });
            } else if hover == Hit::Row(p, vi) {
                gfx.fill_round(row, R_ROW, t.hover);
            }
            if drop_hover == Some((p, Some(vi))) {
                gfx.stroke_round(row.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
            } else if focused {
                gfx.stroke_round(row.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
            }
            // 고대비에서는 선택 바탕(Highlight) 위 글자를 모두 HighlightText로
            let dim = if e.selected && active && t.high_contrast { t.sel_text } else { t.dim };
            let (type_name, type_icon) = type_info(types, e);
            gfx.icon(if e.icon >= 0 { e.icon } else { type_icon }, R::new(cols.name.0 + 4.0, y + (ROW_H - 16.0) / 2.0, cols.name.0 + 20.0, y + (ROW_H + 16.0) / 2.0));
            let fg = if e.is_hidden() && !(e.selected && active) { t.faint } else { fg };
            gfx.text_w(e.name(), R::new(cols.name.0 + 28.0, y, cols.name.1 - 8.0, y + ROW_H), Font::Normal, fg);
            if let Some((l, rr)) = cols.modified {
                let s = match e.kind {
                    Kind::Drive if e.size > 0 => format!("{} 사용 가능", format_size(e.free)),
                    Kind::File | Kind::Dir if short_dates => crate::listing::format_time_short(e.modified, now),
                    Kind::File | Kind::Dir => format_time(e.modified),
                    _ => String::new(),
                };
                gfx.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Normal, dim);
            }
            if let Some((l, rr)) = cols.kind {
                let s = if e.kind == Kind::Drive { drive_type_label(e.drive_type) } else { type_name.as_str() };
                gfx.text(s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Normal, dim);
            }
            if let Some((l, rr)) = cols.size {
                let s = match e.kind {
                    Kind::File => format_size(e.size),
                    Kind::Drive if e.size > 0 => format_size(e.size),
                    _ => String::new(),
                };
                gfx.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Right, dim);
            }
            if edit_mode == EditMode::Rename(p) && vi == tab.cursor {
                let er = R::new(cols.name.0 + 24.0, y + 2.0, cols.name.1 - 4.0, y + ROW_H - 2.0);
                gfx.fill_round(er, R_ROW, t.pane);
                gfx.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
            }
        }
        gfx.unclip();

        let hint = match &tab.state {
            LoadState::Loading if tab.loading_since.is_some_and(|s| s.elapsed().as_millis() >= LOADING_SHOW_MS) => {
                let slow = tab.loading_since.is_some_and(|s| s.elapsed().as_secs() >= 3);
                let place = tab.loc.dir().unwrap_or_default();
                Some(if slow { format!("불러오는 중… 응답을 기다리고 있어요\n{place}\n다른 칸·탭은 그대로 쓸 수 있어요") } else { format!("불러오는 중…\n{place}") })
            }
            LoadState::Error(msg) => Some(match &tab.net_fix {
                Some(f) if f.ask => format!("로그인이 필요해요\n{}\n로그인하면 이 폴더를 열어요", f.share),
                Some(f) => format!("네트워크 드라이브의 연결이 끊겨 있어요\n{}\n다시 연결하거나 F5로 다시 시도해요", f.share),
                None => format!("폴더를 열 수 없어요\n{msg}\nF5를 누르면 다시 시도해요"),
            }),
            LoadState::Ready if tab.entries.is_empty() => Some("빈 폴더예요".to_string()),
            LoadState::Ready if tab.view.is_empty() && !tab.filter.is_empty() => Some(format!("필터 ‘{}’에 맞는 항목이 없어요", tab.filter)),
            LoadState::Ready if tab.view.is_empty() => Some("숨김 항목만 있어요".to_string()),
            _ => None,
        };
        if let Some(h) = hint {
            gfx.text(&h, R::new(g.list.l + 24.0, g.list.t + 24.0, g.list.r - 24.0, g.list.t + 110.0), Font::Hint, t.faint);
        }
        // 로그인·다시 연결: 이 칸의 할 일 하나뿐이라 강조 색 단추
        if let Some((b, label)) = net_btn {
            let k = if hover == Hit::NetFix(p) { 0.88 } else { 1.0 };
            gfx.fill_round(b, R_CTRL, Color { r: t.accent.r * k, g: t.accent.g * k, b: t.accent.b * k, a: 1.0 });
            let lw = gfx.measure(label, Font::Normal);
            gfx.text(label, R::new(b.l + (b.w() - lw) / 2.0, b.t, b.r, b.b), Font::Normal, if t.dark { t.window } else { Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 } });
        }
        // 연결이 끊겨 기억한 목록을 보이는 중이면 흐리게
        if tab.offline {
            gfx.fill(g.list, alpha(t.pane, 0.45));
        }

        if let Some((_, thumb)) = grid.bar(g.list, tab.view.len(), tab.scroll) {
            let hot = thumb_drag || matches!(hover, Hit::Thumb(hp) | Hit::Track(hp) if hp == p);
            let thumb = if hot { thumb } else { thumb.inset(1.0, 0.0) };
            gfx.fill_round(thumb, thumb.w() / 2.0, if hot { t.dim } else { t.thumb });
        }

        if let Some(m) = marquee {
            gfx.clip(g.list);
            gfx.fill(m, alpha(t.accent, 0.15));
            gfx.stroke_round(m.inset(0.5, 0.5), 1.0, t.accent, 1.0);
            gfx.unclip();
        }

        // 늦게 읽히는 폴더: 머리글 아래를 오가는 가는 막대 (0.3초 안에 끝나면 보이지 않는다, 애니메이션을 끄면 글자만)
        if let Some(ms) = tab.loading_since.filter(|_| !tab.offline).map(|s| s.elapsed().as_millis()).filter(|ms| *ms >= LOADING_SHOW_MS && motion) {
            let track = R::new(g.list.l + 8.0, g.header.b - 2.0, g.list.r - SCROLL_W - 6.0, g.header.b);
            let seg = track.w() * 0.3;
            let x = track.l - seg + (track.w() + seg) * ((ms % 1400) as f32 / 1400.0);
            gfx.clip(track);
            gfx.fill_round(R::new(x, track.t, x + seg, track.b), 1.0, t.accent);
            gfx.unclip();
        }

        if border_w > 0.0 {
            gfx.stroke_round(r.inset(border_w / 2.0, border_w / 2.0), RADIUS, border_c, border_w);
        }
    }
}

// ---------- 모달 작업 실행 (App 빌림이 풀린 상태) ----------

pub fn execute(hwnd: HWND, act: Deferred) {
    match act {
        Deferred::Close => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        Deferred::HideToTray => unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
            // 숨어 있는 동안 쓰지 않는 메모리를 돌려준다. 다시 띄우면 필요한 만큼 돌아온다
            let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
        },
        Deferred::Open { pane, paths, dir } => {
            for path in paths {
                open_one(hwnd, pane, &path, dir.as_deref());
            }
        }
        Deferred::ConfirmRename { tab, path, old, new } => {
            let text = format!("확장자를 바꾸면 파일을 열지 못할 수도 있어요.\n\n‘{old}’ → ‘{new}’\n\n이대로 바꿀까요?");
            let yes = unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("이름 바꾸기"), MB_YESNO | MB_ICONWARNING) } == IDYES;
            with_app(|a| if yes { a.start_rename(tab, path, new) } else { a.reopen_rename(tab, &path, &old, &new) });
        }
        Deferred::ConfirmOpen { pane, paths } => {
            let text = format!("파일 {}개를 한꺼번에 열까요?", paths.len());
            if unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Sakadu"), MB_YESNO | MB_ICONQUESTION) } == IDYES {
                for path in paths {
                    let dir = util::parent(&path);
                    open_one(hwnd, pane, &path, dir.as_deref());
                }
            }
        }
        Deferred::DragOut(paths) => {
            let _ = shell::drag_out(hwnd, &paths);
        }
        Deferred::Verb { paths, verb } => {
            let _ = shell::invoke_verb(hwnd, &paths, verb);
        }
        Deferred::Paste { pane, dest } => match shell::clipboard_files() {
            Some((paths, false)) if !paths.is_empty() => shell::spawn_file_op(hwnd, FileOp::Copy { paths, dest }),
            Some((clip, true)) if !clip.is_empty() => {
                // 클립보드는 옮기기가 끝난 뒤에 정리한다 (같은 폴더·취소·실패면 다시 붙여넣을 수 있게)
                let seq = shell::clipboard_seq();
                let paths: Vec<String> = clip.iter().filter(|p| !util::parent(p).is_some_and(|d| util::eq_path(&d, &dest))).cloned().collect();
                let busy = with_app(|a| a.moving.busy(&paths)).unwrap_or(false);
                if busy {
                    with_app(|a| a.flash(pane, "잘라낸 파일을 옮기는 중이에요"));
                } else if paths.is_empty() {
                    with_app(|a| a.flash(pane, "이미 이 폴더에 있어요 · 다른 폴더에 붙여넣으세요"));
                } else if let Some(id) = with_app(|a| a.moving.begin(&paths)) {
                    shell::spawn_cut_paste(hwnd, paths, dest, seq, clip, id);
                }
            }
            _ => {
                with_app(|a| a.flash(pane, "클립보드에 붙여넣을 파일이 없어요"));
            }
        },
        Deferred::Menu { pane, paths, dir, pt } => show_menu(hwnd, MenuRequest { pane, paths, dir, pt }),
        Deferred::PrepareMenu { paths, dir } => prepare_menu(hwnd, paths, dir),
        Deferred::FavMenu { row, pt } => with_menu(|| fav_ui::show_fav_menu(hwnd, row, pt)),
        Deferred::FavSortMenu { pt } => with_menu(|| fav_ui::show_fav_sort_menu(hwnd, pt)),
        Deferred::SideMenu { side, pt } => with_menu(|| sidebar::show_side_menu(hwnd, side, pt)),
        Deferred::PreviewSync => preview::sync_handler(),
        Deferred::Settings { ctl, index, pt } => settings_ui::run_deferred(hwnd, ctl, index, pt),
        Deferred::PalMenu { pt } => palette::show_pal_menu(hwnd, pt),
        Deferred::MoreMenu { pt } => with_menu(|| sidebar::show_more_menu(hwnd, pt)),
        Deferred::StartMenu { pane, item, loc, pt } => with_menu(|| start::show_start_menu(hwnd, pane, item, loc, pt)),
        Deferred::WorkspaceMenu { pt } => with_menu(|| workspace_ui::show_workspace_menu(hwnd, pt)),
        Deferred::NetFix { tab, share, ask } => {
            let r = crate::listing::net_connect(hwnd, &share, ask);
            with_app(|a| a.after_net_fix(tab, r));
        }
    }
}

/// 팝업 메뉴를 띄우는 동안 표시해 둔다: 메뉴 위로 간 마우스 때문에 떠 있는 패널이 닫히지 않게
/// 파일 하나 열기. 폴더를 가리키는 바로 가기(.lnk)는 Sakadu 안에서 그 폴더로 간다
fn open_one(hwnd: HWND, pane: usize, path: &str, dir: Option<&str>) {
    if path.to_lowercase().ends_with(".lnk") {
        if let Some(target) = shell::shortcut_target_dir(path) {
            with_app(|a| a.navigate(pane, Location::Dir(target), None));
            return;
        }
    }
    shell::open_path(hwnd, path, dir);
}

fn with_menu(f: impl FnOnce()) {
    with_app(|a| a.menu_open = true);
    f();
    with_app(|a| a.menu_open = false);
}

fn remove_search_folder(hwnd: HWND, folders: &crate::folders::Folders, root: &str) {
    let text = format!("이 폴더를 검색에서 뺄까요?\n{root}");
    if unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Sakadu"), MB_YESNO | MB_ICONQUESTION) } != IDYES {
        return;
    }
    let msg = match folders.remove(root) {
        Ok(()) => format!("검색에서 뺐어요: {root}"),
        Err(e) => format!("검색 폴더 목록을 저장하지 못했어요: {e}"),
    };
    with_app(|a| a.flash(a.active, msg));
}

struct MenuRequest {
    pane: usize,
    paths: Vec<String>,
    dir: Option<String>,
    pt: POINT,
}

struct PreparedMenu {
    paths: Vec<String>,
    dir: Option<String>,
    extended: bool,
    menu: shell::ShellMenu,
    single_dir: bool,
}

thread_local! {
    /// 오른쪽 버튼을 누를 때 미리 만든 셸 메뉴
    static PREPARED_MENU: RefCell<Option<PreparedMenu>> = const { RefCell::new(None) };
    static PREPARING_MENU: Cell<bool> = const { Cell::new(false) };
    /// 준비 도중(셸 확장이 메시지를 돌리는 사이) 버튼을 떼면, 준비가 끝나는 즉시 띄운다
    static SHOW_AFTER_PREPARE: RefCell<Option<MenuRequest>> = const { RefCell::new(None) };
}

fn build_menu(hwnd: HWND, paths: &[String], dir: Option<&str>, extended: bool) -> Option<(shell::ShellMenu, bool)> {
    let cm = if paths.is_empty() { shell::background_menu(hwnd, dir?) } else { shell::ui_object(hwnd, paths) }.ok()?;
    let single_dir = paths.len() == 1 && shell::is_dir(&paths[0]);
    let custom: Vec<(u32, &str)> = if single_dir {
        vec![(ID_OPEN_TAB, "새 탭에서 열기"), (ID_ADD_FAV, "즐겨찾기에 추가")]
    } else if paths.len() == 1 {
        vec![(ID_ADD_FAV, "즐겨찾기에 추가")]
    } else if paths.is_empty() {
        vec![(ID_ADD_FAV, "이 폴더를 즐겨찾기에 추가")]
    } else {
        Vec::new()
    };
    let menu = shell::ShellMenu::new(cm, extended, &custom).ok()?;
    Some((menu, single_dir))
}

fn prepare_menu(hwnd: HWND, paths: Vec<String>, dir: Option<String>) {
    let extended = key_down(VK_SHIFT);
    let t = std::time::Instant::now();
    PREPARING_MENU.set(true);
    let built = build_menu(hwnd, &paths, dir.as_deref(), extended);
    PREPARING_MENU.set(false);
    crate::diag::log(|| format!("menu: prepared in {:?}", t.elapsed()));
    PREPARED_MENU.with(|m| *m.borrow_mut() = built.map(|(menu, single_dir)| PreparedMenu { paths, dir, extended, menu, single_dir }));
    if let Some(req) = SHOW_AFTER_PREPARE.with(|s| s.borrow_mut().take()) {
        show_menu(hwnd, req);
    }
}

fn show_menu(hwnd: HWND, req: MenuRequest) {
    if PREPARING_MENU.get() {
        SHOW_AFTER_PREPARE.with(|s| *s.borrow_mut() = Some(req));
        return;
    }
    let extended = key_down(VK_SHIFT);
    let prepared = PREPARED_MENU
        .with(|m| m.borrow_mut().take())
        .filter(|p| p.paths == req.paths && p.dir == req.dir && p.extended == extended);
    let reused = prepared.is_some();
    let (menu, single_dir) = match prepared {
        Some(p) => (p.menu, p.single_dir),
        None => match build_menu(hwnd, &req.paths, req.dir.as_deref(), extended) {
            Some(b) => b,
            None => return,
        },
    };
    crate::diag::log(|| format!("menu: show (prepared={reused})"));
    let MenuRequest { pane, paths, dir, pt } = req;
    let Some(picked) = menu.track(hwnd, pt) else { return };
    if picked.id == ID_OPEN_TAB {
        with_app(|a| a.new_tab(pane, Some(Location::Dir(paths[0].clone())), false));
        return;
    }
    if picked.id == ID_ADD_FAV {
        if let Some(target) = paths.first().cloned().or(dir) {
            with_app(|a| a.add_favorite_path(&target, None));
        }
        return;
    }
    // 폴더 열기·이름 바꾸기는 탐색기 창 대신 이 앱 안에서 처리
    match picked.verb.as_str() {
        "rename" => {
            with_app(|a| a.begin_rename(pane));
        }
        "open" | "explore" if single_dir => {
            with_app(|a| a.navigate(pane, Location::Dir(util::normalize(&paths[0])), None));
        }
        "opennewwindow" if single_dir => {
            with_app(|a| a.new_tab(pane, Some(Location::Dir(util::normalize(&paths[0]))), true));
        }
        _ => {
            let _ = menu.invoke(hwnd, picked.id, Some(pt), dir.as_deref());
        }
    }
}

/// 입력 상자 색. 입력 상자는 App이 다른 메시지를 처리하는 도중에도 다시 그려지며 색을 물으므로
/// App 밖에 두고 창 프로시저에서 바로 답한다
#[derive(Clone, Copy)]
struct EditColors {
    text: COLORREF,
    field: COLORREF,
    field_brush: HBRUSH,
    pane: COLORREF,
    pane_brush: HBRUSH,
    /// 필터 칩 바탕 (옅은 강조 색)
    chip: COLORREF,
    chip_brush: HBRUSH,
    rename: HWND,
    preview_host: HWND,
    filter: HWND,
    cmd: HWND,
}

thread_local! {
    /// 처리한 키에 뒤따르는 WM_CHAR를 삼켜 삑 소리나 제어 문자가 들어가지 않게 한다
    static SUPPRESS_CHAR: Cell<bool> = const { Cell::new(false) };
    static EDIT_COLORS: Cell<Option<EditColors>> = const { Cell::new(None) };
}

/// WM_CTLCOLOREDIT 응답
pub fn edit_ctl_color(wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let c = EDIT_COLORS.get()?;
    let hdc = HDC(wp.0 as *mut c_void);
    let target = HWND(lp.0 as *mut c_void);
    let (bg, brush) = if target == c.rename || target == c.preview_host || target == c.cmd {
        (c.pane, c.pane_brush)
    } else if target == c.filter {
        (c.chip, c.chip_brush)
    } else {
        (c.field, c.field_brush)
    };
    unsafe {
        SetTextColor(hdc, c.text);
        SetBkColor(hdc, bg);
    }
    Some(LRESULT(brush.0 as isize))
}

pub unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, id: usize, _data: usize) -> LRESULT {
    unsafe {
        match msg {
            WM_GETDLGCODE => return LRESULT(DefSubclassProc(hwnd, msg, wp, lp).0 | DLGC_WANTALLKEYS as isize),
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                SUPPRESS_CHAR.set(false);
                let repeat = lp.0 & (1 << 30) != 0;
                let handled = with_app(|app| {
                    app.key_repeat = repeat;
                    app.on_edit_key(id as i32, VIRTUAL_KEY(wp.0 as u16))
                })
                .unwrap_or(false);
                // 글자를 만드는 키(Enter·Esc·Tab·Backspace·Space·Ctrl 조합)를 처리했을 때만 뒤따르는 WM_CHAR를 삼킨다.
                // 미뤄 둔 확인 창(이름·열기)의 메시지 루프가 그 글자를 먼저 꺼내 가므로, 창을 띄우기 전에 정해 둔다
                if handled {
                    let vk = VIRTUAL_KEY(wp.0 as u16);
                    SUPPRESS_CHAR.set(matches!(vk, VK_RETURN | VK_ESCAPE | VK_TAB | VK_BACK | VK_SPACE) || key_down(VK_CONTROL));
                }
                if let Ok(parent) = GetParent(hwnd) {
                    crate::run_deferred(parent);
                }
                if handled {
                    return LRESULT(0);
                }
            }
            // 이름 상자에 붙여넣을 때는 첫 줄에서 쓸 수 없는 문자를 뺀다
            WM_PASTE if id as i32 == ID_RENAME && with_app(|app| matches!(app.edit_mode, EditMode::Rename(_))).unwrap_or(false) => {
                if let Some(text) = shell::clipboard_text(hwnd) {
                    let clean: String = text.lines().next().unwrap_or("").chars().filter(|c| !blocked_name_char(*c)).collect();
                    let q = HSTRING::from(clean);
                    SendMessageW(hwnd, EM_REPLACESEL, Some(WPARAM(1)), Some(LPARAM(q.as_ptr() as isize)));
                    return LRESULT(0);
                }
            }
            // 검색 상자에 여러 줄(엑셀 열 등)을 붙여넣으면 줄마다 `|`로 이어 한 번에 찾는다
            WM_PASTE if id as i32 == ID_SEARCH => {
                if let Some(text) = shell::clipboard_text(hwnd).filter(|t| t.trim().contains('\n')) {
                    let q = HSTRING::from(palette::lines_to_query(&text));
                    SendMessageW(hwnd, EM_REPLACESEL, Some(WPARAM(1)), Some(LPARAM(q.as_ptr() as isize)));
                    return LRESULT(0);
                }
            }
            WM_CHAR | WM_SYSCHAR => {
                if SUPPRESS_CHAR.replace(false) || wp.0 == 0x7F {
                    return LRESULT(0);
                }
                if msg == WM_CHAR
                    && id as i32 == ID_RENAME
                    && char::from_u32(wp.0 as u32).is_some_and(blocked_name_char)
                    && with_app(|app| app.rename_char_blocked()).unwrap_or(false)
                {
                    return LRESULT(0);
                }
                // 보기 메뉴가 열려 있으면 글자가 필터 상자로 들어가지 않게 삼킨다
                if id as i32 == ID_FILTER && with_app(|app| app.view_menu.is_some()).unwrap_or(false) {
                    return LRESULT(0);
                }
                // 키보드가 즐겨찾기에 있으면 글자는 필터 대신 줄 찾기에 쓴다
                if msg == WM_CHAR && id as i32 == ID_FILTER && with_app(|app| app.fav_type_char(wp.0 as u16)).unwrap_or(false) {
                    return LRESULT(0);
                }
            }
            // 필터 상자를 직접 누르면 키보드는 칸으로 돌아온다 (즐겨찾기 조작이 필터 글자·Delete를 가로채지 않게).
            // 앱이 스스로 포커스를 줄 때는 App이 빌려져 있어 여기서 아무것도 하지 않는다
            WM_SETFOCUS if id as i32 == ID_FILTER => {
                with_app(|app| {
                    if app.fav_focus {
                        app.fav_focus = false;
                        app.invalidate();
                    }
                });
            }
            // 검색 상자를 직접 누르면 키보드는 검색 쪽으로
            WM_SETFOCUS if id as i32 == ID_SEARCH => {
                with_app(|app| {
                    if !app.pal_focus {
                        app.pal_focus = true;
                        app.invalidate();
                    }
                });
            }
            WM_KILLFOCUS => {
                let r = DefSubclassProc(hwnd, msg, wp, lp);
                // 검색 상자에서 이 창의 다른 곳으로 키보드가 옮겨 갔다 (wParam이 0이면 다른 앱으로: 돌아오면 검색 상자로)
                if id as i32 == ID_SEARCH && wp.0 != 0 {
                    with_app(|app| {
                        if app.pal_focus {
                            app.pal_focus = false;
                            app.invalidate();
                        }
                    });
                }
                with_app(|app| app.on_edit_blur(id as i32));
                if let Ok(parent) = GetParent(hwnd) {
                    crate::run_deferred(parent);
                }
                return r;
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 말풍선은 "이름 (단축키)"의 단축키만 키캡으로 떼고, 폴더 이름의 괄호나 경로는 그대로 둔다
    #[test]
    fn tip_splits_only_shortcuts() {
        assert_eq!(split_tip_key("새 폴더 (Ctrl+Shift+N)"), ("새 폴더", Some("Ctrl+Shift+N")));
        assert_eq!(split_tip_key("이름 바꾸기 (F2)"), ("이름 바꾸기", Some("F2")));
        assert_eq!(split_tip_key("뒤로 (Alt+←)"), ("뒤로", Some("Alt+←")));
        assert_eq!(split_tip_key("사진 (2024)"), ("사진 (2024)", None));
        assert_eq!(split_tip_key("C:\\자료\\보고서 (Ctrl+1)"), ("C:\\자료\\보고서 (Ctrl+1)", None));
        assert_eq!(split_tip_key("검색 (Ctrl+F) · 다른 앱에서는 Win+Alt+F"), ("검색 (Ctrl+F) · 다른 앱에서는 Win+Alt+F", None));
        assert_eq!(split_tip_key("설정"), ("설정", None));
    }

    /// 회귀: 이동 작업이 겹쳐도 먼저 끝난 작업이 다른 작업의 '옮기는 중' 표시를 지우지 않는다
    #[test]
    fn overlapping_moves_keep_each_other_busy() {
        let (a, b) = (vec!["C:\\s\\a.txt".to_string()], vec!["C:\\s\\b.txt".to_string()]);
        let mut m = Moving::default();
        let ida = m.begin(&a);
        let idb = m.begin(&b);
        m.end(ida);
        assert!(!m.busy(&a), "A finished");
        assert!(m.busy(&b), "B still moving");
        assert!(m.busy(&["c:\\S\\B.TXT".to_string(), "C:\\s\\c.txt".to_string()]), "any overlapping file counts");
        m.end(idb);
        assert!(!m.busy(&b));
    }

}
