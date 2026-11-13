//! 메인 창: 레이아웃, 패널·탭 동작, 입력 처리, 그리기.
//!
//! 재진입 규칙: App은 RefCell 안에 있으므로, 모달 루프를 돌리는 호출(우클릭 메뉴, 드래그, ShellExecute)은
//! 처리 중에 직접 하지 않고 `pending`에 넣어 빌림이 풀린 뒤 `execute`에서 실행한다.

use crate::gfx::{Font, Gfx, R};
pub(crate) use sakadu::ui::glyph::*;
pub(crate) use sakadu::ui::*;
pub(crate) use sakadu::ui::toolbar::{ToolbarGeom, ToolbarHover, ToolbarScene, ToolbarSpec, can_go_up, draw_view_icon, paint_toolbar, toolbar_geom};
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
mod preview_card;
mod settings_ui;
mod view_menu;
mod caption;
mod sidebar;
mod start;
mod workspace_ui;
mod commands;
mod layout;
mod load;
mod nav;
mod tabs;
mod scroll;
mod edits;
mod window;
mod file_ops;
mod dragdrop;
mod messages;
mod paint;
mod modal;
use palette::{G_SEARCH, PalGate, PalStatus, TIMER_SEARCH, TIMER_SEARCH_READY};
use sidebar::{BAR_W, Panel, Side, TIMER_BAR_HOVER, TIMER_SIDE_ANIM, TIMER_SIDE_CLOSE};
use preview::{Preview, TIMER_PREVIEW, thumb_key};
pub use palette::SearchDone;
pub use preview::{PreviewMsg, unload_handler};
pub use modal::{edit_ctl_color, execute};
use modal::*;


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
    /// 도구 막대의 칸 나누기 단추 (누르면 칸 나누기 메뉴)
    LayoutPick,
    /// 도구 막대의 새 폴더 단추 (지금 칸에)
    NewFolder(usize),
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
    /// 도구 막대의 보기 방식 단추 (누르면 보기 메뉴. 지금 칸의 보기를 바꾼다)
    View(usize),
    /// 열린 메뉴(보기 방식·칸 나누기)의 줄
    ViewMenuItem(usize),
    /// 열린 메뉴 안의 빈 곳
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
    /// 왼쪽 패널 안의 줄·단추 (고정한 것·내 PC·즐겨찾기)
    Side(Side),
    /// 작업 공간 레일의 공간 아이콘 (작업 공간 번호)
    Rail(usize),
    /// 레일의 새 공간(+)
    RailAdd,
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
    /// 도구 막대의 검색 단추
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
    /// 경로 오른쪽 끝의 책갈피 (이 공간에 고정)
    Star(usize),
    /// 새 탭 시작 화면의 카드·줄 (칸, 항목)
    Start(usize, usize),
    /// 미리 보기 머리의 핀 (파일 고정·풀기)
    PreviewPin,
    /// 미리 보기 머리의 열기 (기본 앱으로)
    PreviewOpen,
    /// 미리 보기의 고정 칩 (누르면 핀과 같다)
    PreviewChip,
    /// 자동 미리 보기 카드: 크게 보기 단추, 닫기 단추, 카드의 그 밖의 곳 (아래 칸을 누르지 않게 잡는다. 누르면 크게·작게)
    CardExpand,
    CardClose,
    Card,
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
    /// 사이드바의 고정한 줄을 눌렀다 (pins 번호). 조금 움직이면 PinMove, 그대로 떼면 연다
    PinPress { index: usize, y: f32 },
    /// 고정한 줄을 끌어 순서를 바꾸는 중
    PinMove { from: usize, y: f32 },
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
    /// 이 공간에 고정한 줄의 표시 이름 바꾸기 (pins 번호)
    PinRename(usize),
    /// 사이드바 패널 머리의 작업 공간 이름
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
    /// 사이드바의 내 PC 줄 메뉴
    SideMenu { side: Side, pt: POINT },
    /// 사이드바에서 이 공간에 고정한 줄의 메뉴
    PinMenu { index: usize, pt: POINT },
    /// 레일의 작업 공간 아이콘 메뉴 (바꾸기·이름·색·지우기)
    RailMenu { index: usize, pt: POINT },
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
    /// 작업 공간 목록 메뉴 (레일의 + 를 우클릭: 모든 작업 공간·새로 만들기·이름 바꾸기·지우기)
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
    /// 사이드바의 '모든 공간': 놓으면 즐겨찾기에 넣는다
    Favorites,
    /// 사이드바의 고정 목록이나 레일의 지금 공간 아이콘: 놓으면 이 작업 공간에 고정한다
    Pins,
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
    /// 키보드가 즐겨찾기 줄이 아니라 그 위의 줄(고정한 곳, 내 PC)에 있으면 그 줄. None이면 fav_sel의 즐겨찾기 줄
    top_sel: Option<fav_ui::TopSel>,
    /// 이 작업 공간에서 키보드가 사이드바에 들어온 적이 있는가 (처음에는 맨 위 줄에서 시작한다)
    side_kb_seen: bool,
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
    /// 파일을 끌어 사이드바에 놓으려는 자리 (이 공간에 고정하는 쪽이면 Side::PinEmpty, '모든 공간' 쪽이면 Side::FavEmpty)
    side_drop: Option<Side>,
    /// 사이드바의 내 PC·드라이브 (뒤에서 읽어 온다)
    places: sidebar::Places,
    places_busy: bool,
    places_again: bool,
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
    /// 자동 미리 보기 카드 (그림·글 파일을 고르면 저절로 뜬다)
    card: preview_card::Card,
    /// 마지막 누름(두 번 클릭의 첫 누름)이 미리 보기 카드 위였는가. 그랬으면 이어지는 둘째 누름은 버리고, 아니었으면 카드 아래 것이 받는다
    press_on_card: bool,
    /// 자동 미리 보기가 켜져 있는가 (도구 막대의 눈). 세션에 저장
    auto_preview: bool,
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
        // 처음 쓰면(세션 없음) 작업 공간 패널을 고정해 둔다. 예전 세션의 바로가기·즐겨찾기 패널도 이 패널로 연다
        let (side_pinned, side_w) = match &sess {
            Some(s) => (Panel::from_key(&s.side), s.side_w),
            None => (Some(Panel::Main), session::SIDE_W_DEFAULT),
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
        // 강조 색은 지금 작업 공간의 색을 따른다 (정하지 않았으면 설정의 강조 색)
        let workspaces = crate::workspaces::load();
        let accent = workspaces.current().map(|w| w.color.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| look.accent.clone());
        let mut app = App {
            hwnd,
            gfx,
            theme: crate::theme::resolve(&look.theme, &accent),
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
            top_sel: None,
            side_kb_seen: false,
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
            card: preview_card::Card::new(),
            press_on_card: false,
            auto_preview: sess.as_ref().is_none_or(|s| s.auto_preview),
            preview_host,
            tray: None,
            stale: [false; 4],
            marquee_base: Vec::new(),
            new_tab_home: sess.as_ref().map_or_else(|| session::NEW_TAB_START.to_string(), |s| s.new_tab.clone()),
            recent: sess.as_ref().map(|s| s.recent.clone()).unwrap_or_default(),
            start_keys: false,
            workspaces,
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
        self.ensure_workspace();
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
            EditMode::Rename(_) | EditMode::FavRename(_) | EditMode::PinRename(_) | EditMode::WorkspaceRename => self.rename_edit,
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
