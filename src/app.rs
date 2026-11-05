//! 메인 창: 레이아웃, 패널·탭 동작, 입력 처리, 그리기.
//!
//! 재진입 규칙: App은 RefCell 안에 있으므로, 모달 루프를 돌리는 호출(우클릭 메뉴, 드래그, ShellExecute)은
//! 처리 중에 직접 하지 않고 `pending`에 넣어 빌림이 풀린 뒤 `execute`에서 실행한다.

use crate::gfx::{Font, Gfx, R};
use crate::listing::{Entry, Kind, drive_type_label, format_size, format_time, list_computer, list_dir};
use crate::pane::*;
use crate::shell::{self, FileOp, key_down};
use crate::theme::{Color, FOCUS_W, R_PANE, R_ROW, Theme, alpha, colorref};
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
use windows::Win32::UI::Controls::{EM_REPLACESEL, EM_SETCUEBANNER, EM_SETMARGINS, EM_SETSEL, WM_MOUSELEAVE};
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
use fav_ui::G_STAR;
use palette::{G_SEARCH, PalGate, PalStatus, TIMER_SEARCH, TIMER_SEARCH_READY};
use sidebar::{SIDEBAR_W, Side};
use preview::{G_PREVIEW, Preview, TIMER_PREVIEW, thumb_key};
pub use palette::SearchDone;
pub use preview::{TextReady, unload_handler};

/// 칸 영역 위 작업 줄 (칸 나누기·숨김·미리 보기·창 단추). 빈 곳은 창을 끄는 제목 줄 역할도 한다
const WORKBAR_H: f32 = 44.0;
const MARGIN: f32 = 8.0;
const GAP: f32 = 8.0;
/// 칸 맨 위 칩 줄 (탭 칩, 새 탭, 오른쪽에 항목 수 또는 필터 칩)
const HEAD_H: f32 = 44.0;
/// 칩 줄 아래 경로 줄 (뒤로·앞으로·위로, 경로, 별, 새 폴더, 보기)
const PATH_H: f32 = 34.0;
const HEADER_H: f32 = 28.0;
const ROW_H: f32 = 26.0;
const SCROLL_W: f32 = 10.0;
/// 칸 카드의 모서리
const RADIUS: f32 = 10.0;
const CHEV_W: f32 = 16.0;
const CRUMB_PAD: f32 = 8.0;

pub const ID_FILTER: i32 = 100;
pub const ID_RENAME: i32 = 101;
pub const ID_ADDR: i32 = 102;
pub const ID_SEARCH: i32 = 103;
/// 전역 단축키 (Ctrl+Alt+Space): 어디서든 검색 팔레트
pub const HOTKEY_SEARCH: i32 = 1;
/// 다른 Sakadu 프로세스가 WM_COPYDATA로 넘기는 "이 경로를 열어 달라" 요청
pub const COPYDATA_OPEN: usize = 0x5150_0001;
const ID_OPEN_TAB: u32 = shell::MENU_CUSTOM;
const ID_ADD_FAV: u32 = shell::MENU_CUSTOM + 1;
const TIMER_REFRESH: usize = 10;
/// 도구 막대 버튼 말풍선을 띄우기까지
const TIMER_TIP: usize = 42;
/// 칸 아래 알림(flash)을 지우기까지
const TIMER_FLASH: usize = 44;
const FLASH_MS: u32 = 5000;

// Segoe Fluent Icons / Segoe MDL2 Assets 글리프
const G_BACK: u16 = 0xE72B;
const G_FWD: u16 = 0xE72A;
const G_UP: u16 = 0xE74A;
const G_ADD: u16 = 0xE710;
const G_CLOSE: u16 = 0xE8BB;
const G_CHEVRON: u16 = 0xE76C;
const G_STAR_FILL: u16 = 0xE735;
const G_SORT_UP: u16 = 0xE70E;
const G_SORT_DOWN: u16 = 0xE70D;
const G_NEW_FOLDER: u16 = 0xE8F4;
const G_HIDDEN: u16 = 0xE7B3;
const G_FILTER: u16 = 0xE71C;
const G_LIST: u16 = 0xE8FD;
const G_GRID: u16 = 0xF0E2;
const G_CHEVRON_DOWN: u16 = 0xE70D;
const G_SETTINGS: u16 = 0xE713;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    One,
    TwoV,
    TwoH,
    Three,
    Four,
}

impl Layout {
    pub const ALL: [Layout; 5] = [Layout::One, Layout::TwoV, Layout::TwoH, Layout::Three, Layout::Four];

    pub fn count(self) -> usize {
        match self {
            Layout::One => 1,
            Layout::TwoV | Layout::TwoH => 2,
            Layout::Three => 3,
            Layout::Four => 4,
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|&l| l == self).unwrap_or(0)
    }
}

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
    Pane(usize),
    /// 사이드바 가운데 목록 (즐겨찾기·드라이브·검색 폴더)
    Side(Side),
    /// 검색 창의 "이 폴더도 검색하기" 버튼
    PalAddFolder,
    SearchButton,
    PalRow(usize),
    PalInstall,
    PalPanel,
    PreviewButton,
    SettingsButton,
    /// 사이드바 아래쪽 도움말
    HelpButton,
    /// 설정 창 안
    Set(settings_ui::SetHit),
    /// 경로 상자 오른쪽 끝의 즐겨찾기 별
    Star(usize),
}

impl Hit {
    fn pane(self) -> Option<usize> {
        use Hit::*;
        match self {
            Tab(p, _) | TabClose(p, _) | TabAdd(p) | TabStrip(p) | Back(p) | Fwd(p) | Up(p) | Crumb(p, _) | CrumbEmpty(p) | Star(p) | View(p)
            | Header(p, _) | HeaderResize(p, _) | Row(p, _) | ListEmpty(p) | Thumb(p) | Track(p) | FilterClear(p) | NewFolder(p) | Pane(p) => Some(p),
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
}

struct RenameCtx {
    pane: usize,
    tab: u64,
    path: String,
    old: String,
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
    /// 사이드바 가운데 목록의 스크롤
    side_scroll: f32,
    /// 파일을 끌어 사이드바에 놓으려는 자리 (즐겨찾기 쪽이면 Side::FavEmpty)
    side_drop: Option<Side>,
    /// 사이드바의 내 PC·드라이브 (뒤에서 읽어 온다)
    places: sidebar::Places,
    places_busy: bool,
    places_again: bool,
    /// (폴더, 디스크 검색 밖인가): 드라이브 종류는 폴더가 바뀔 때만 묻는다
    outside_cache: RefCell<(String, bool)>,
    search_edit: HWND,
    pal_open: bool,
    pal_results: Vec<(bool, String)>,
    pal_sel: usize,
    /// 검색 결과 중 골라 둔 것 (Ctrl·Shift+클릭, Shift+↑↓). 결과와 같은 순서
    pal_picked: Vec<bool>,
    /// Shift로 범위를 고를 때의 시작 줄
    pal_anchor: usize,
    pal_scroll: f32,
    pal_status: PalStatus,
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
    /// 도구 막대 버튼 말풍선이 떠 있는지
    tip_shown: bool,
    /// 설정 창 (열려 있으면 창 전체를 덮는다)
    settings: Option<settings_ui::Settings>,
    /// Ctrl+F 검색에 넣은 폴더(공유폴더 등 디스크 인덱스 밖)
    folders: crate::folders::Folders,
    /// 검색 창을 열 때 보이는 칸에 열려 있던, 아직 검색에 넣지 않은 공유폴더·USB 폴더 (결과가 없으면 넣기 버튼)
    pal_outside: Option<String>,
}

// ---------- 기하 ----------

struct PaneGeom {
    /// 칩 줄 (탭 칩, 새 탭, 오른쪽 항목 수·필터 칩)
    head: R,
    back: R,
    fwd: R,
    up: R,
    /// 경로 글자 자리 (주소 입력 상자도 여기에 놓인다)
    crumb: R,
    /// 경로 조각을 그리는 곳 (오른쪽 끝 별을 뺀 자리)
    crumbs: R,
    /// 경로 오른쪽 끝: 즐겨찾기 별
    star: R,
    new_folder: R,
    /// 보기 방식 버튼 (지금 보기 그림 + 아래 화살표)
    view: R,
    header: R,
    list: R,
}

fn geom(r: R) -> PaneGeom {
    let head = R::new(r.l, r.t, r.r, r.t + HEAD_H);
    let path = R::new(r.l, head.b, r.r, head.b + PATH_H);
    let (t, b) = (path.t + 4.0, path.b - 4.0);
    let back = R::new(path.l + 10.0, t, path.l + 36.0, b);
    let fwd = R::new(back.r + 2.0, t, back.r + 28.0, b);
    let up = R::new(fwd.r + 2.0, t, fwd.r + 28.0, b);
    let view = R::new(path.r - 10.0 - 40.0, t, path.r - 10.0, b);
    let new_folder = R::new(view.l - 4.0 - 26.0, t, view.l - 4.0, b);
    let crumb = R::new(up.r + 8.0, path.t + 3.0, (new_folder.l - 4.0).max(up.r + 48.0), path.b - 3.0);
    let star = R::new(crumb.r - 28.0, crumb.t + 2.0, crumb.r - 4.0, crumb.b - 2.0);
    let crumbs = R::new(crumb.l, crumb.t, star.l, crumb.b);
    let header = R::new(r.l, path.b, r.r, path.b + HEADER_H);
    // 아래쪽은 둥근 모서리에 항목이 닿지 않게 조금 띄운다
    let list = R::new(r.l, header.b, r.r, (r.b - 6.0).max(header.b));
    PaneGeom { head, back, fwd, up, crumb, crumbs, star, new_folder, view, header, list }
}

/// 필터 칩 자리 (칩 줄 오른쪽 끝)
fn filter_chip(g: &PaneGeom) -> R {
    let w = (g.head.w() * 0.4).clamp(150.0, 240.0);
    R::new(g.head.r - 12.0 - w, g.head.t + 9.0, g.head.r - 12.0, g.head.t + 35.0)
}

/// 칩 줄에서 탭 칩과 새 탭(+)이 놓일 곳: 왼쪽 여백 10, 오른쪽은 항목 수·필터 칩 자리(right)를 뺀 곳
fn chip_strip(g: &PaneGeom, right: f32) -> R {
    let l = g.head.l + 10.0;
    R::new(l, g.head.t + 8.0, (g.head.r - 12.0 - right - 8.0).max(l + 100.0), g.head.t + 36.0)
}

struct Cols {
    name: (f32, f32),
    modified: Option<(f32, f32)>,
    kind: Option<(f32, f32)>,
    size: Option<(f32, f32)>,
}

impl Cols {
    /// 너비 조절 가능한 열(수정한 날짜, 유형, 크기)의 범위
    fn resizable(&self) -> [Option<(f32, f32)>; 3] {
        [self.modified, self.kind, self.size]
    }
}

/// 좁은 패널에서는 유형 → 날짜 순으로 열을 숨긴다
fn columns(list: R, w: [f32; 3]) -> Cols {
    let mut x = list.r - SCROLL_W - 6.0;
    let width = list.w();
    let size = Some((x - w[2], x));
    x -= w[2];
    let kind = if width >= 590.0 {
        let c = (x - w[1], x);
        x -= w[1];
        Some(c)
    } else {
        None
    };
    let modified = if width >= 400.0 {
        let c = (x - w[0], x);
        x -= w[0];
        Some(c)
    } else {
        None
    };
    Cols { name: (list.l + 12.0, x.max(list.l + 124.0)), modified, kind, size }
}

/// 자세히 보기가 아닌 보기의 정렬 버튼 (기준, 이름, 자리). 머리글 줄 왼쪽부터 나란히 둔다.
/// 열이 없는 격자 위에 자세히 보기의 열 머리글이 떠 있지 않게 한다. 지금 기준에만 화살표 자리가 붙는다
fn sort_chips(gfx: &Gfx, header: R, current: SortKey) -> Vec<(SortKey, &'static str, R)> {
    const KEYS: [(SortKey, &str); 4] = [(SortKey::Name, "이름"), (SortKey::Modified, "수정한 날짜"), (SortKey::Type, "유형"), (SortKey::Size, "크기")];
    let mut x = header.l + 8.0;
    KEYS.iter()
        .map(|&(key, label)| {
            let w = gfx.measure(label, Font::Normal) + if key == current { 38.0 } else { 20.0 };
            let r = R::new(x, header.t + 4.0, x + w, header.b - 4.0);
            x = r.r + 4.0;
            (key, label, r)
        })
        .collect()
}

/// 탭 칩 자리. 다 들어가면 칩마다 이름 길이(natural)대로, 아니면 같은 폭으로 줄이고, 그래도 넘치면
/// 지금 탭(active)이 보이는 만큼만 보이고 나머지는 폭 0으로 양 끝에 둔다(그리기·누르기에서 빠짐). 새 탭(+)은 늘 보이는 칩 바로 뒤
fn tab_rects(strip: R, natural: &[f32], active: usize) -> Vec<R> {
    let n = natural.len();
    let avail = strip.w() - 32.0;
    let total = natural.iter().sum::<f32>() + 2.0 * n.saturating_sub(1) as f32;
    if total <= avail {
        let mut x = strip.l;
        return natural
            .iter()
            .map(|w| {
                let r = R::new(x, strip.t, x + w, strip.b);
                x = r.r + 2.0;
                r
            })
            .collect();
    }
    let w = ((avail + 2.0) / n.max(1) as f32 - 2.0).clamp(72.0, 180.0);
    // 나눗셈 오차로 하나 덜 들어간다고 보지 않게 조금 여유를 둔다
    let fits = ((avail + 2.0) / (w + 2.0) + 0.001).floor().max(1.0) as usize;
    let first = if n > fits { (active + 1).saturating_sub(fits).min(n - fits) } else { 0 };
    let start = strip.l;
    let end = start + fits.min(n) as f32 * (w + 2.0) - 2.0;
    (0..n)
        .map(|i| {
            if i < first {
                R::new(start, strip.t, start, strip.b)
            } else if i >= first + fits {
                R::new(end, strip.t, end, strip.b)
            } else {
                let l = start + (i - first) as f32 * (w + 2.0);
                R::new(l, strip.t, l + w, strip.b)
            }
        })
        .collect()
}

/// 탭 줄의 x에 놓았을 때 들어갈 자리. 보이는 탭만 보고 정한다:
/// 가려진 탭(폭 0)까지 세면 마지막으로 보이는 탭 바로 뒤에 놓아도 맨 끝으로 가 버린다
fn drop_index(trs: &[R], x: f32) -> usize {
    let visible: Vec<usize> = (0..trs.len()).filter(|&i| trs[i].w() > 0.0).collect();
    match visible.iter().rev().find(|&&i| (trs[i].l + trs[i].r) / 2.0 < x) {
        Some(&i) => i + 1,
        None => visible.first().copied().unwrap_or(0),
    }
}

/// 탭 칩 안 오른쪽 끝의 닫기 단추
fn close_rect(tr: R) -> R {
    R::new(tr.r - 24.0, tr.t + 4.0, tr.r - 4.0, tr.b - 4.0)
}

fn add_rect(strip: R, tabs: &[R]) -> R {
    let l = tabs.iter().map(|r| r.r + 4.0).fold(strip.l, f32::max);
    R::new(l, strip.t, l + 28.0, strip.b)
}

/// 큰 아이콘 보기의 그림 영역(DIP). 보통·아주 큰 아이콘은 VIEW_THUMBS
const THUMB: f32 = 96.0;
/// 작은 아이콘 보기 칸 너비(남는 폭은 칸에 나눈다), 목록 보기 열 너비
const SMALL_W: f32 = 240.0;
const LIST_W: f32 = 260.0;

/// 한 칸에 무엇을 그리나
#[derive(Clone, Copy, PartialEq, Debug)]
enum CellKind {
    /// 자세히: 한 줄에 이름·날짜·유형·크기
    Row,
    /// 아이콘 보기: 그림(이 크기의 정사각형, DIP) 아래 이름 두 줄
    Tile(f32),
    /// 작은 아이콘·목록: 작은 아이콘 옆에 이름
    Icon,
}

/// 목록 배치. 그리기·클릭·스크롤·키 이동·네모 고르기가 모두 이것을 쓴다.
/// 목록 보기(horiz)만 위에서 아래로 채운 뒤 옆 열로 넘어가고 가로로 스크롤한다. 나머지는 왼쪽에서 오른쪽으로 채우고 세로로 스크롤
#[derive(Clone, Copy)]
struct Grid {
    ox: f32,
    oy: f32,
    /// 한 줄의 칸 수 (세로 스크롤)
    cols: usize,
    /// 한 열의 칸 수 (가로 스크롤)
    rows: usize,
    cell_w: f32,
    cell_h: f32,
    cell: CellKind,
    horiz: bool,
}

impl Grid {
    fn of(list: R, view: ViewMode) -> Grid {
        let tile = |thumb: f32, min_w: f32| {
            let usable = (list.w() - SCROLL_W - 14.0).max(min_w);
            let cols = ((usable / min_w).floor() as usize).max(1);
            // 남는 폭을 칸에 고르게 나눠 오른쪽이 비지 않게
            Grid { ox: list.l + 6.0, oy: list.t + 6.0, cols, rows: 0, cell_w: usable / cols as f32, cell_h: thumb + 52.0, cell: CellKind::Tile(thumb), horiz: false }
        };
        match view {
            // 줄 바탕은 머리글·상태 표시줄 구분선과 같은 8px 안쪽에서 시작한다 (활성 칸 테두리에 붙어 보이지 않게)
            ViewMode::Details => Grid { ox: list.l + 8.0, oy: list.t, cols: 1, rows: 0, cell_w: list.w() - SCROLL_W - 14.0, cell_h: ROW_H, cell: CellKind::Row, horiz: false },
            ViewMode::Huge => tile(256.0, 280.0),
            ViewMode::Large => tile(THUMB, 124.0),
            ViewMode::Medium => tile(48.0, 88.0),
            ViewMode::Small => {
                let usable = (list.w() - SCROLL_W - 14.0).max(SMALL_W);
                let cols = ((usable / SMALL_W).floor() as usize).max(1);
                Grid { ox: list.l + 8.0, oy: list.t + 2.0, cols, rows: 0, cell_w: usable / cols as f32, cell_h: ROW_H, cell: CellKind::Icon, horiz: false }
            }
            ViewMode::List => {
                // 아래에 가로 스크롤 막대 자리를 남긴다
                let rows = (((list.h() - 6.0 - SCROLL_W - 4.0) / ROW_H).floor() as usize).max(1);
                Grid { ox: list.l + 8.0, oy: list.t + 2.0, cols: 0, rows, cell_w: LIST_W, cell_h: ROW_H, cell: CellKind::Icon, horiz: true }
            }
        }
    }

    /// 스크롤 방향으로 잰 전체 내용 길이
    fn content(&self, n: usize) -> f32 {
        let pad = if matches!(self.cell, CellKind::Tile(_)) { 12.0 } else if self.cell == CellKind::Icon { 4.0 } else { 0.0 };
        if self.horiz { n.div_ceil(self.rows) as f32 * self.cell_w + pad + 8.0 } else { n.div_ceil(self.cols) as f32 * self.cell_h + pad }
    }

    /// 스크롤 방향으로 잰 보이는 길이
    fn page(&self, list: R) -> f32 {
        if self.horiz { list.w() } else { list.h() }
    }

    /// 칸 위치 (열, 줄)
    fn pos(&self, vi: usize) -> (usize, usize) {
        if self.horiz { (vi / self.rows, vi % self.rows) } else { (vi % self.cols, vi / self.cols) }
    }

    fn item(&self, vi: usize, scroll: f32) -> R {
        let (c, r) = self.pos(vi);
        let (sx, sy) = if self.horiz { (scroll, 0.0) } else { (0.0, scroll) };
        let x = self.ox + c as f32 * self.cell_w - sx;
        let y = self.oy + r as f32 * self.cell_h - sy;
        R::new(x, y, x + self.cell_w, y + self.cell_h)
    }

    fn at(&self, x: f32, y: f32, scroll: f32, n: usize) -> Option<usize> {
        let (sx, sy) = if self.horiz { (scroll, 0.0) } else { (0.0, scroll) };
        let (fx, fy) = ((x - self.ox + sx) / self.cell_w, (y - self.oy + sy) / self.cell_h);
        if fx < 0.0 || fy < 0.0 {
            return None;
        }
        let (c, r) = (fx as usize, fy as usize);
        let vi = if self.horiz {
            if r >= self.rows {
                return None;
            }
            c * self.rows + r
        } else {
            if c >= self.cols {
                return None;
            }
            r * self.cols + c
        };
        (vi < n).then_some(vi)
    }

    /// 화면에 보이는 항목 범위
    fn visible(&self, list: R, scroll: f32, n: usize) -> std::ops::Range<usize> {
        if self.horiz {
            let first = ((scroll - (self.ox - list.l)) / self.cell_w).floor().max(0.0) as usize;
            let cols = (list.w() / self.cell_w).ceil() as usize + 1;
            return (first * self.rows).min(n)..((first + cols) * self.rows).min(n);
        }
        let first_row = ((scroll - (self.oy - list.t)) / self.cell_h).floor().max(0.0) as usize;
        let rows = (list.h() / self.cell_h).ceil() as usize + 1;
        (first_row * self.cols).min(n)..((first_row + rows) * self.cols).min(n)
    }

    /// 스크롤 막대 (트랙, 손잡이). 넘치지 않으면 None
    fn bar(&self, list: R, n: usize, scroll: f32) -> Option<(R, R)> {
        let content = self.content(n);
        if !self.horiz {
            return scroll_metrics(list, content, scroll);
        }
        let view = list.w();
        if content <= view || view <= 0.0 {
            return None;
        }
        let track = R::new(list.l + 2.0, list.b - SCROLL_W - 2.0, list.r - 2.0, list.b - 2.0);
        let tw = (view / content * track.w()).max(24.0).min(track.w());
        let x = track.l + (track.w() - tw) * (scroll / (content - view)).clamp(0.0, 1.0);
        Some((track, R::new(x, track.t + 2.0, x + tw, track.b - 2.0)))
    }

    /// 스크롤 방향 좌표 (가로 스크롤이면 x)
    fn along(&self, x: f32, y: f32) -> f32 {
        if self.horiz { x } else { y }
    }

    /// 손잡이의 스크롤 방향 시작·길이, 트랙의 시작·길이
    fn bar_span(&self, track: R, thumb: R) -> (f32, f32, f32, f32) {
        if self.horiz { (thumb.l, thumb.w(), track.l, track.w()) } else { (thumb.t, thumb.h(), track.t, track.h()) }
    }
}

/// 목록이 넘칠 때 (트랙, 손잡이). content는 전체 내용 높이
fn scroll_metrics(list: R, content: f32, scroll: f32) -> Option<(R, R)> {
    let view = list.h();
    if content <= view || view <= 0.0 {
        return None;
    }
    let track = R::new(list.r - SCROLL_W - 2.0, list.t + 2.0, list.r - 2.0, list.b - 2.0);
    let th = (view / content * track.h()).max(24.0).min(track.h());
    let y = track.t + (track.h() - th) * (scroll / (content - view)).clamp(0.0, 1.0);
    Some((track, R::new(track.l + 2.0, y, track.r - 2.0, y + th)))
}

struct Crumb {
    x0: f32,
    x1: f32,
    label: String,
    target: Location,
}

/// 경로 표시줄 조각. 공간이 모자라면 앞쪽을 "…"로 접는다
fn crumbs(gfx: &Gfx, area: R, loc: &Location) -> Vec<Crumb> {
    let mut items = vec![("내 PC".to_string(), Location::Computer)];
    if let Location::Dir(path) = loc {
        for (label, full) in util::segments(path) {
            items.push((label, Location::Dir(full)));
        }
    }
    let widths: Vec<f32> = items.iter().map(|(l, _)| gfx.measure(l, Font::Strong) + CRUMB_PAD * 2.0).collect();
    let avail = area.w() - 8.0;
    let mut start = items.len() - 1;
    let mut total = widths[start];
    while start > 0 {
        let ellipsis = if start - 1 > 0 { 24.0 + CHEV_W } else { 0.0 };
        if total + CHEV_W + widths[start - 1] + ellipsis > avail {
            break;
        }
        start -= 1;
        total += CHEV_W + widths[start];
    }
    let mut out = Vec::new();
    let mut x = area.l + 4.0;
    if start > 0 {
        out.push(Crumb { x0: x, x1: x + 24.0, label: "…".into(), target: items[start - 1].1.clone() });
        x += 24.0 + CHEV_W;
    }
    for (i, (label, target)) in items.into_iter().enumerate().skip(start) {
        let w = widths[i].min((area.r - 4.0 - x).max(0.0));
        out.push(Crumb { x0: x, x1: x + w, label, target });
        x += w + CHEV_W;
    }
    out
}

/// 상위 위치로 갈 때, 방금 있던 하위 항목을 선택하기 위한 이름(내 PC면 드라이브 경로)
fn child_toward(ancestor: &Location, current: &Location) -> Option<String> {
    let Location::Dir(cur) = current else { return None };
    match ancestor {
        Location::Computer => Some(util::segments(cur).first()?.1.clone()),
        Location::Dir(a) => {
            if cur.len() <= a.len() || !util::eq_path(&cur[..a.len()], a) {
                return None;
            }
            cur[a.len()..].trim_start_matches('\\').split('\\').next().map(String::from)
        }
    }
}

/// 칸 나누기 버튼 5개를 한 덩어리로 담는 판. 작업 줄 왼쪽 끝, 칸과 같은 왼쪽 선
fn layout_pill() -> R {
    let l = SIDEBAR_W + MARGIN;
    R::new(l, 7.0, l + 6.0 + 5.0 * 32.0, 37.0)
}

fn layout_buttons() -> [R; 5] {
    std::array::from_fn(|i| {
        let l = SIDEBAR_W + MARGIN + 3.0 + i as f32 * 32.0;
        R::new(l, 10.0, l + 32.0, 34.0)
    })
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
    Err(format!("경로를 찾을 수 없습니다: {t}"))
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

/// 백그라운드에서 폴더를 읽고 결과를 창 메시지로 보낸다. 개별 아이콘은 목록을 먼저 보낸 뒤 채운다
fn spawn_listing(hwnd: HWND, tab: u64, generation: u64, loc: Location) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let _com = shell::ComInit::sta();
        let result = match &loc {
            Location::Computer => Ok(list_computer()),
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
        post(owner.get(), WM_APP_LISTED, Box::new(Listed { tab, generation, result }));
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
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        let (filter_edit, rename_edit, addr_edit, search_edit) =
            unsafe { (create_edit(hwnd, ID_FILTER), create_edit(hwnd, ID_RENAME), create_edit(hwnd, ID_ADDR), create_edit(hwnd, ID_SEARCH)) };
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
            theme: Theme::resolve(&look.theme, &look.accent),
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
            side_drop: None,
            places: sidebar::Places::default(),
            places_busy: false,
            places_again: false,
            outside_cache: RefCell::new((String::new(), false)),
            search_edit,
            pal_open: false,
            pal_results: Vec::new(),
            pal_sel: 0,
            pal_picked: Vec::new(),
            pal_anchor: 0,
            pal_scroll: 0.0,
            pal_status: PalStatus::Idle,
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
            new_tab_home: sess.as_ref().map_or_else(|| "::computer".to_string(), |s| s.new_tab.clone()),
            tip_shown: false,
            settings: None,
            folders: crate::folders::Folders::start(),
            pal_outside: None,
        };
        unsafe {
            let cue = HSTRING::from("입력하면 바로 필터");
            SendMessageW(filter_edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
            let cue = HSTRING::from("디스크 전체에서 파일·폴더 이름 찾기");
            SendMessageW(search_edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
        }
        app.apply_edit_font();
        app.apply_window_theme();
        app.refresh_places();
        Ok(app)
    }

    /// 메인 창을 표시한 뒤에 할 초기화: 자동 완성, 드롭 대상, 첫 목록 불러오기, 변경 알림
    pub fn init(&mut self) {
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
    }

    /// 어디서든 검색(전역 단축키)을 등록한다. 설정에서 정한 키가 없으면 비어 있는 첫 후보를 쓴다.
    /// 정한 키를 다른 프로그램이 쓰고 있으면 false
    pub(super) fn register_global_hotkey(&mut self) -> bool {
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
        match self.edit_mode {
            EditMode::Address(_) => self.addr_edit,
            EditMode::Rename(_) | EditMode::FavRename(_) => self.rename_edit,
            EditMode::None if self.pal_open => self.search_edit,
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

    /// 칸이 놓이는 곳: 사이드바 오른쪽, 작업 줄 아래 (미리 보기가 열려 있으면 그만큼 비운다)
    fn area(&self) -> R {
        let l = SIDEBAR_W + MARGIN;
        let right = self.w - MARGIN - if self.preview.is_some() { self.preview_w() + GAP } else { 0.0 };
        R::new(l, WORKBAR_H, right.max(l + 100.0), (self.h - MARGIN).max(WORKBAR_H + 100.0))
    }

    /// 작업 줄 오른쪽: 창 단추 바로 앞에 미리 보기, 그 앞에 숨김 항목
    fn preview_button(&self) -> R {
        let r = self.caption_left() - 8.0;
        R::new(r - 32.0, 7.0, r, 37.0)
    }

    fn hidden_button(&self) -> R {
        let p = self.preview_button();
        R::new(p.l - 36.0, 7.0, p.l - 4.0, 37.0)
    }

    fn split_points(&self) -> (f32, f32) {
        let a = self.area();
        (a.l + (a.w() - GAP) * self.split.0, a.t + (a.h() - GAP) * self.split.1)
    }

    fn pane_rects(&self) -> Vec<(usize, R)> {
        let a = self.area();
        let (xm, ym) = self.split_points();
        match self.layout {
            Layout::One => vec![(0, a)],
            Layout::TwoV => vec![(0, R::new(a.l, a.t, xm, a.b)), (1, R::new(xm + GAP, a.t, a.r, a.b))],
            Layout::TwoH => vec![(0, R::new(a.l, a.t, a.r, ym)), (1, R::new(a.l, ym + GAP, a.r, a.b))],
            Layout::Three => vec![
                (0, R::new(a.l, a.t, xm, a.b)),
                (1, R::new(xm + GAP, a.t, a.r, ym)),
                (2, R::new(xm + GAP, ym + GAP, a.r, a.b)),
            ],
            Layout::Four => vec![
                (0, R::new(a.l, a.t, xm, ym)),
                (1, R::new(xm + GAP, a.t, a.r, ym)),
                (2, R::new(a.l, ym + GAP, xm, a.b)),
                (3, R::new(xm + GAP, ym + GAP, a.r, a.b)),
            ],
        }
    }

    fn pane_rect(&self, p: usize) -> Option<R> {
        self.pane_rects().into_iter().find(|(i, _)| *i == p).map(|(_, r)| r)
    }

    fn splitters(&self) -> (Option<R>, Option<R>) {
        let a = self.area();
        let (xm, ym) = self.split_points();
        let sx = matches!(self.layout, Layout::TwoV | Layout::Three | Layout::Four).then(|| R::new(xm, a.t, xm + GAP, a.b));
        let sy = match self.layout {
            Layout::TwoH | Layout::Four => Some(R::new(a.l, ym, a.r, ym + GAP)),
            Layout::Three => Some(R::new(xm + GAP, ym, a.r, ym + GAP)),
            _ => None,
        };
        (sx, sy)
    }

    fn list_rect(&self, p: usize) -> Option<R> {
        self.pane_rect(p).map(|r| geom(r).list)
    }

    /// 칸 p의 칩 줄 오른쪽 글자: 불러오는 중이거나, 항목 수와 고른 개수·크기
    fn head_count_text(&self, p: usize) -> String {
        let tab = self.panes[p].tab();
        match &tab.state {
            LoadState::Loading => "불러오는 중…".to_string(),
            LoadState::Error(_) => "열 수 없음".to_string(),
            LoadState::Ready => {
                let (sel, bytes) = tab.selection_stats();
                let n = tab.view.len();
                match (sel, bytes) {
                    (0, _) => format!("{n}개"),
                    (s, 0) => format!("{n}개 중 {s}개 선택"),
                    (s, b) => format!("{n}개 중 {s}개 선택 · {}", format_size(b)),
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

    /// 칸 p의 탭 칩 자리와 새 탭(+) 자리
    fn chips(&self, p: usize, g: &PaneGeom) -> (Vec<R>, R) {
        let pane = &self.panes[p];
        let n = pane.tabs.len();
        let natural: Vec<f32> = pane
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let font = if i == pane.active { Font::Strong } else { Font::Normal };
                (self.gfx.measure(&tab.loc.title(), font) + 22.0 + if n > 1 { 20.0 } else { 0.0 }).clamp(52.0, 180.0)
            })
            .collect();
        let strip = chip_strip(g, self.head_right_w(p, g));
        let trs = tab_rects(strip, &natural, pane.active);
        let add = add_rect(strip, &trs);
        (trs, add)
    }

    /// 필터 칩 안: (입력 상자 자리, 개수 글자 자리, × 단추)
    fn filter_parts(&self, p: usize, chip: R) -> (R, R, R) {
        let n = format!("{}개", self.panes[p].tab().view.len());
        let nw = self.gfx.measure(&n, Font::Small);
        let clear = R::new(chip.r - 24.0, chip.t + 3.0, chip.r - 4.0, chip.b - 3.0);
        let count = R::new(clear.l - 4.0 - nw, chip.t, clear.l - 4.0, chip.b);
        let edit = R::new(chip.l + 28.0, chip.t, (count.l - 6.0).max(chip.l + 60.0), chip.b);
        (edit, count, clear)
    }

    fn hit(&self, x: f32, y: f32) -> Hit {
        if self.settings.is_some() {
            return Hit::Set(self.settings_hit(x, y));
        }
        if self.pal_open {
            if let Some(h) = self.pal_hit(x, y) {
                return h;
            }
        }
        if self.search_button().contains(x, y) {
            return Hit::SearchButton;
        }
        if let Some(h) = self.view_menu_hit(x, y) {
            return h;
        }
        if x < SIDEBAR_W {
            return self.sidebar_hit(x, y);
        }
        if y < WORKBAR_H {
            if let Some(i) = layout_buttons().iter().position(|r| r.contains(x, y)) {
                return Hit::Layout(i);
            }
            if self.hidden_button().contains(x, y) {
                return Hit::Hidden;
            }
            if self.preview_button().contains(x, y) {
                return Hit::PreviewButton;
            }
            return Hit::None;
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
                if !tab.filter.is_empty() {
                    let chip = filter_chip(&g);
                    if self.filter_parts(p, chip).2.contains(x, y) {
                        return Hit::FilterClear(p);
                    }
                    if chip.contains(x, y) {
                        return Hit::Pane(p);
                    }
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
            if g.view.contains(x, y) {
                return Hit::View(p);
            }
            if g.new_folder.contains(x, y) {
                return Hit::NewFolder(p);
            }
            if tab.loc.dir().is_some() && g.star.contains(x, y) {
                return Hit::Star(p);
            }
            if g.crumb.contains(x, y) {
                let cs = crumbs(&self.gfx, g.crumbs, &tab.loc);
                return match cs.iter().position(|c| x >= c.x0 && x < c.x1) {
                    Some(i) => Hit::Crumb(p, i),
                    None => Hit::CrumbEmpty(p),
                };
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
        } else {
            tab.state = LoadState::Loading;
        }
        spawn_listing(self.hwnd, tab.id, tab.generation, tab.loc.clone());
    }

    fn find_tab(&self, id: u64) -> Option<(usize, usize)> {
        self.panes.iter().enumerate().find_map(|(p, pane)| pane.tabs.iter().position(|t| t.id == id).map(|t| (p, t)))
    }

    fn on_listed(&mut self, l: Listed) {
        let Some((p, t)) = self.find_tab(l.tab) else { return };
        let (sort, hidden) = (self.panes[p].sort, self.show_hidden);
        let tab = &mut self.panes[p].tabs[t];
        if tab.generation != l.generation {
            return;
        }
        let restore = match l.result {
            Ok(entries) => {
                if let Some(dir) = tab.loc.dir() {
                    self.folders.observe(dir, &entries);
                }
                tab.set_entries(entries, sort, hidden)
            }
            Err(msg) => {
                tab.entries.clear();
                tab.view.clear();
                tab.state = LoadState::Error(msg);
                None
            }
        };
        if self.panes[p].active == t {
            self.clamp_scroll(p);
            if let Some(r) = restore {
                if r.reveal {
                    self.ensure_visible(p);
                }
                if r.rename && p == self.active {
                    self.begin_rename(p);
                }
            }
            if self.edit_mode == EditMode::Rename(p) {
                self.layout_edits();
            }
        }
        self.invalidate();
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
            let want = (p < self.layout.count()).then(|| self.panes[p].tab().loc.dir().map(String::from)).flatten();
            if want.as_deref() == self.watches[p].as_ref().map(|(_, d)| d.as_str()) {
                continue;
            }
            if let Some((id, _)) = self.watches[p].take() {
                shell::unwatch(id);
            }
            if let Some(d) = want {
                let id = shell::watch(self.hwnd, &d, WM_APP_NOTIFY + p as u32);
                if id != 0 {
                    self.watches[p] = Some((id, d));
                }
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
        self.close_palette();
        self.fav_blur();
        let p = self.active;
        let path = util::normalize(&raw);
        if shell::is_dir(&path) {
            if self.panes[p].tab().loc.dir().is_some_and(|d| util::eq_path(d, &path)) {
                return;
            }
            self.new_tab(p, Some(Location::Dir(path)), true);
        } else if shell::exists(&path) {
            if let Some(dir) = util::parent(&path) {
                let name = util::file_name(&path).to_string();
                self.new_tab(p, Some(Location::Dir(dir)), true);
                self.panes[p].tab_mut().restore = Some(Restore::select(&name));
            }
        } else {
            self.flash(p, format!("경로를 찾을 수 없습니다: {raw}"));
        }
    }

    fn after_location_change(&mut self, p: usize) {
        if p == self.active {
            self.set_filter_edit_text("");
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
            .take(32)
            .collect();
        if paths.is_empty() {
            paths.extend(tab.path_of(cur));
        }
        let dir = tab.loc.dir().map(String::from);
        self.pending.push(Deferred::Open { pane: p, paths, dir });
    }

    // ---------- 탭·패널 ----------

    /// 위치 없이 여는 새 탭(Ctrl+T·+·탭 줄 더블클릭)의 위치: 설정에 따라 내 PC, 정해 둔 폴더, 지금 탭과 같은 폴더
    fn new_tab_location(&self, p: usize) -> Location {
        if self.new_tab_home == session::NEW_TAB_SAME { self.panes[p].tab().loc.clone() } else { Location::from_session(&self.new_tab_home) }
    }

    pub fn new_tab(&mut self, p: usize, loc: Option<Location>, activate: bool) {
        let loc = loc.unwrap_or_else(|| self.new_tab_location(p));
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
        pane.tabs.remove(t);
        if t < pane.active || pane.active >= pane.tabs.len() {
            pane.active = pane.active.saturating_sub(1);
        }
        if was_active {
            self.after_tab_switch(p);
        }
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
        let t = self.panes[p].active;
        let fresh = self.panes[p].tab().generation == 0;
        self.load_at(p, t, !fresh);
        if p == self.active {
            let f = self.panes[p].tab().filter.clone();
            self.set_filter_edit_text(&f);
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
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        self.active = p;
        let f = self.panes[p].tab().filter.clone();
        self.set_filter_edit_text(&f);
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
        self.layout = l;
        if self.active >= l.count() {
            self.active = 0;
            let f = self.panes[0].tab().filter.clone();
            self.set_filter_edit_text(&f);
        }
        for p in 0..l.count() {
            if self.panes[p].tab().generation == 0 {
                self.load(p, false);
            }
            self.clamp_scroll(p);
        }
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
        let Some((list, grid)) = self.grid(p) else { return };
        let tab = self.panes[p].tab_mut();
        let max = (grid.content(tab.view.len()) - grid.page(list)).max(0.0);
        tab.scroll = tab.scroll.clamp(0.0, max);
    }

    fn ensure_visible(&mut self, p: usize) {
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
        for h in [self.filter_edit, self.rename_edit, self.addr_edit, self.search_edit] {
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

    /// 실제 Win32 입력 상자(한글 IME 지원)를 그려 둔 자리 위에 맞춘다
    fn layout_edits(&self) {
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
        // 필터 상자는 활성 칸의 필터 칩 안에 둔다. 필터가 비어 있거나 팝업이 열려 있으면 화면 밖에 두되 포커스는 그대로 둬,
        // 목록에서 바로 글자를 치면 걸러진다 (첫 글자가 들어오면 칩이 생겨 그 안으로 옮긴다)
        let popup = self.pal_open || self.view_menu.is_some() || self.settings.is_some();
        let filter = match self.pane_rect(self.active) {
            Some(r) if !popup && !self.panes[self.active].tab().filter.is_empty() => {
                Some(center(self.filter_parts(self.active, filter_chip(&geom(r))).0, 0.0, 0.0))
            }
            _ => Some(R::new(-4000.0, -4000.0, -3800.0, -3980.0)),
        };
        place(self.filter_edit, filter);
        place(self.search_edit, self.pal_open.then(|| center(self.pal_input_rect(), 4.0, 4.0)));
        let addr = match self.edit_mode {
            EditMode::Address(p) => self.pane_rect(p).map(|r| center(geom(r).crumb, 8.0, 8.0)),
            _ => None,
        };
        place(self.addr_edit, addr);
        let ren = match self.edit_mode {
            EditMode::Rename(p) => self.rename_rect(p).map(|r| center(r, 4.0, 4.0)),
            EditMode::FavRename(row) => self.fav_rename_rect(row).map(|r| center(r, 4.0, 4.0)),
            _ => None,
        };
        place(self.rename_edit, ren);
    }

    fn set_filter_edit_text(&self, s: &str) {
        if edit_text(self.filter_edit) != s {
            unsafe {
                let _ = SetWindowTextW(self.filter_edit, &HSTRING::from(s));
            }
        }
    }

    fn sync_filter_from_edit(&mut self) {
        let text = edit_text(self.filter_edit);
        let p = self.active;
        let (sort, hidden) = (self.panes[p].sort, self.show_hidden);
        let tab = self.panes[p].tab_mut();
        if tab.filter == text {
            return;
        }
        tab.filter = text;
        tab.rebuild_view(sort, hidden);
        // 입력하는 동안 첫 일치 항목을 선택해 Enter로 바로 열 수 있게 한다
        if !tab.filter.is_empty() && !tab.view.is_empty() {
            tab.select_only(0);
        }
        tab.scroll = 0.0;
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
        let tab = self.panes[p].tab();
        if tab.loc == Location::Computer {
            return;
        }
        let Some(e) = tab.cursor_entry() else { return };
        let Some(path) = tab.path_of(e) else { return };
        let name = e.name_string();
        // 파일은 확장자를 뺀 부분만 선택 (탐색기와 같음)
        let stem = match name.rfind('.') {
            Some(i) if i > 0 && e.kind == Kind::File => name[..i].encode_utf16().count(),
            _ => name.encode_utf16().count(),
        };
        self.rename = Some(RenameCtx { pane: p, tab: tab.id, path, old: name.clone() });
        self.activate_pane(p);
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
        if let EditMode::FavRename(row) = self.edit_mode {
            self.commit_fav_rename(row);
            return;
        }
        let Some(ctx) = self.rename.take() else { return };
        let new_name = edit_text(self.rename_edit).trim().to_string();
        self.close_edit();
        if new_name.is_empty() || new_name == ctx.old {
            return;
        }
        if let Some(c) = util::invalid_name_char(&new_name) {
            self.flash(ctx.pane, format!("파일 이름에는 {c} 문자를 쓸 수 없습니다"));
            return;
        }
        if let Some((p, t)) = self.find_tab(ctx.tab) {
            self.panes[p].tabs[t].restore = Some(Restore::select(&new_name));
        }
        shell::spawn_file_op(self.hwnd, FileOp::Rename { path: ctx.path, name: new_name });
    }

    fn begin_address(&mut self, p: usize) {
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
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
        match resolve_input(&text) {
            Ok((loc, select)) => self.navigate(p, loc, select),
            Err(msg) => self.flash(p, msg),
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
            EditMode::Rename(_) | EditMode::FavRename(_) => self.commit_rename(),
            EditMode::Address(_) => self.cancel_edit(),
            EditMode::None => {}
        }
    }

    /// 칸 아래쪽에 잠깐 알림을 띄운다 (몇 초 뒤 사라진다)
    fn flash(&mut self, p: usize, msg: impl Into<String>) {
        self.panes[p].tab_mut().message = Some(msg.into());
        unsafe { SetTimer(Some(self.hwnd), TIMER_FLASH, FLASH_MS, None) };
        self.invalidate();
    }

    fn clear_flash(&mut self) {
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

    fn copy(&mut self, p: usize, cut: bool) {
        let tab = self.panes[p].tab();
        let paths = if cut { tab.marked_paths() } else { tab.selected_paths() };
        if paths.is_empty() {
            if cut {
                self.flash(p, "잘라낼 항목을 먼저 고르세요");
            }
            return;
        }
        let n = paths.len();
        self.pending.push(Deferred::Verb { paths, verb: if cut { "cut" } else { "copy" } });
        self.flash(p, format!("항목 {n}개를 {}", if cut { "잘라냈습니다" } else { "복사했습니다" }));
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
                self.panes[p].tab_mut().restore = Some(Restore { rename: true, ..Restore::select(&name) });
                self.load(p, true);
            }
            Err(e) => self.flash(p, format!("폴더를 만들 수 없습니다: {e}")),
        }
    }

    fn push_undo(&mut self, u: crate::undo::Undo) {
        crate::diag::log(|| format!("undo: recorded ({})", u.describe()));
        self.undo.push(u);
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
    }

    /// Ctrl+Z: 이 앱에서 한 마지막 파일 작업을 되돌린다
    fn undo_last(&mut self) {
        let p = self.active;
        match self.undo.pop() {
            Some(u) => {
                self.flash(p, u.describe());
                shell::spawn_file_op(self.hwnd, FileOp::Undo(u));
            }
            None => self.flash(p, "되돌릴 작업이 없습니다"),
        }
    }

    fn on_file_op(&mut self, r: shell::OpReport) {
        if let Some(u) = r.undo {
            self.push_undo(u);
        }
        if let Some(o) = r.undone {
            self.on_undone(o);
        }
        if let Some(cut) = r.cut {
            self.after_cut_paste(cut);
        }
        // 복사·이동·삭제로 남은 용량이 바뀌었다
        self.refresh_places();
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
            [one] => format!("‘{}’은(는) 그사이 바뀌었거나 없어져 그대로 두었습니다", util::file_name(one)),
            many => format!("{}개는 그사이 바뀌었거나 없어져 그대로 두었습니다", many.len()),
        };
        let msg = match (failed, changed.is_empty()) {
            (0, true) => return,
            (0, false) => changed,
            (n, true) => format!("{n}개를 되돌리지 못했습니다 · {key}로 다시 시도"),
            (n, false) => format!("{n}개를 되돌리지 못했습니다({key}로 다시 시도) · {changed}"),
        };
        self.flash(self.active, msg);
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
        let paths = if tab.view.is_empty() { Vec::new() } else { tab.selected_paths() };
        let dir = tab.loc.dir().map(String::from);
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
            // 제목 표시줄을 도구 막대 색과 맞춰 한 덩어리처럼 보이게 (Windows 11)
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
                search: self.search_edit,
            }
        };
        if let Some(old) = EDIT_COLORS.replace(Some(colors)) {
            unsafe {
                let _ = DeleteObject(old.field_brush.into());
                let _ = DeleteObject(old.pane_brush.into());
                let _ = DeleteObject(old.chip_brush.into());
            }
        }
        for h in [self.filter_edit, self.rename_edit, self.addr_edit] {
            unsafe {
                let _ = InvalidateRect(Some(h), None, true);
            }
        }
        self.invalidate();
    }

    fn save_session(&self) {
        let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        unsafe {
            let _ = GetWindowPlacement(self.hwnd, &mut wp);
        }
        let r = wp.rcNormalPosition;
        let s = session::Session {
            window: Some([r.left, r.top, r.right - r.left, r.bottom - r.top]),
            maximized: wp.showCmd == SW_SHOWMAXIMIZED.0 as u32,
            layout: self.layout.index() as u8,
            split: self.split,
            active: self.active,
            show_hidden: self.show_hidden,
            preview: self.preview.is_some(),
            new_tab: self.new_tab_home.clone(),
            look: self.look.clone(),
            panes: self
                .panes
                .iter()
                .map(|p| session::PaneState {
                    tabs: p.tabs.iter().map(|t| t.loc.to_session()).collect(),
                    active: p.active,
                    cols: p.cols,
                    sort_key: p.sort.key.to_u8(),
                    sort_desc: p.sort.desc,
                    view: p.view.code(),
                })
                .collect(),
        };
        let _ = session::save(&s);
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
        self.theme = Theme::resolve(&self.look.theme, &self.look.accent);
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
        if x < SIDEBAR_W && self.settings.is_none() {
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
            h => match h.pane() {
                Some(p) => (Some((p, None)), self.panes[p].tab().loc.dir().map(String::from)),
                None => (None, None),
            },
        };
        let hover = if dir.is_some() { hover } else { None };
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
                    if !self.on_key(VIRTUAL_KEY(wp.0 as u16)) {
                        return None;
                    }
                }
                WM_SETFOCUS => self.focus(),
                WM_ACTIVATE => {
                    // 다른 창으로 전환하면 보기 메뉴를 닫는다 (WA_INACTIVE).
                    // 검색 창은 다른 앱에서 무언가 복사해 오는 동안에도 적어 둔 검색어가 그대로 있도록 열어 둔다
                    self.window_active = loword(wp.0) != 0;
                    self.invalidate();
                    if loword(wp.0) == 0 {
                        self.close_view_menu();
                    } else {
                        // 관리자 승인(검색 서비스 설치) 등을 마치고 돌아오면 설정 창의 상태를 다시 읽는다.
                        // 다른 곳에서 파일을 옮겼을 수 있으니 드라이브 남은 용량도 다시 읽는다
                        self.refresh_settings();
                        self.refresh_places();
                    }
                    return None;
                }
                WM_COMMAND => {
                    if hiword(wp.0) as u32 == EN_CHANGE {
                        match loword(wp.0) as i32 {
                            ID_FILTER => self.sync_filter_from_edit(),
                            ID_SEARCH => self.pal_query_changed(),
                            _ => {}
                        }
                    }
                }
                WM_TIMER => {
                    let id = wp.0;
                    if id == TIMER_SEARCH_READY {
                        self.pal_ready_timer();
                    } else if id == TIMER_SEARCH {
                        self.pal_run_query();
                    } else if id == TIMER_PREVIEW {
                        self.start_preview();
                    } else if id == TIMER_FLASH {
                        let _ = KillTimer(Some(hwnd), id);
                        self.clear_flash();
                    } else if id == TIMER_TIP {
                        let _ = KillTimer(Some(hwnd), id);
                        self.tip_shown = true;
                        self.invalidate();
                    } else if (TIMER_REFRESH..TIMER_REFRESH + 4).contains(&id) {
                        let _ = KillTimer(Some(hwnd), id);
                        let p = id - TIMER_REFRESH;
                        if p < self.layout.count() {
                            // 트레이에 숨어 있는 동안에는 읽지 않고, 다시 보일 때 읽는다
                            if IsWindowVisible(hwnd).as_bool() {
                                self.load(p, true);
                            } else {
                                self.stale[p] = true;
                            }
                        }
                    }
                }
                // 고대비 모드를 켜고 끄면 시스템 색이 바뀐다
                WM_SYSCOLORCHANGE => {
                    self.theme = Theme::resolve(&self.look.theme, &self.look.accent);
                    self.apply_window_theme();
                    return None;
                }
                WM_SETTINGCHANGE => {
                    let colors = wp.0 == SPI_SETHIGHCONTRAST.0 as usize;
                    if colors || lp.0 != 0 && PCWSTR(lp.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                        self.theme = Theme::resolve(&self.look.theme, &self.look.accent);
                        self.apply_window_theme();
                        let tip = self.tray_tip();
                        if let Some(t) = &mut self.tray {
                            t.update(false, &tip);
                        }
                    }
                    return None;
                }
                WM_CLOSE => {
                    self.save_session();
                    self.pending.push(if self.tray.is_some() { Deferred::HideToTray } else { Deferred::Close });
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
                WM_APP_ICONS => self.on_icons(*Box::from_raw(lp.0 as *mut IconsReady)),
                WM_APP_SEARCH => self.on_search_done(*Box::from_raw(lp.0 as *mut SearchDone)),
                crate::WM_APP_FILEOP => self.on_file_op(*Box::from_raw(lp.0 as *mut shell::OpReport)),
                crate::WM_APP_THUMB => self.on_thumb(*Box::from_raw(lp.0 as *mut crate::thumbs::Image)),
                crate::WM_APP_PREVIEW => self.on_preview_text(*Box::from_raw(lp.0 as *mut TextReady)),
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
                    self.hover = h;
                    self.view_menu_hover(h);
                    self.update_tip();
                    self.invalidate();
                }
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
        self.layout_edits();
        self.invalidate();
    }

    fn on_lbutton_down(&mut self, x: f32, y: f32, dbl: bool) {
        let hit = self.hit(x, y);
        self.tip_shown = false;
        if let Hit::Set(h) = hit {
            self.settings_click(h, x, y);
            return;
        }
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        if self.pal_open && hit != Hit::SearchButton {
            self.pal_mouse_down(hit);
            self.invalidate();
            return;
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
            Hit::SearchButton => self.toggle_palette(),
            Hit::NewFolder(p) => self.new_folder(p),
            Hit::Hidden => self.toggle_hidden(),
            Hit::PreviewButton => self.toggle_preview(),
            Hit::SettingsButton => self.toggle_settings(settings_ui::Page::General),
            Hit::HelpButton => self.toggle_settings(settings_ui::Page::Help),
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
                if let Some(c) = crumbs(&self.gfx, geom(r).crumbs, &cur).into_iter().nth(i) {
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
        if self.settings.is_some() {
            return;
        }
        if self.pal_open {
            match hit {
                Hit::PalRow(i) => self.pal_right_press(i),
                Hit::PalPanel | Hit::PalInstall | Hit::PalAddFolder => {}
                _ => self.close_palette(),
            }
            self.invalidate();
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
        let paths = if items { tab.selected_paths() } else { Vec::new() };
        Some((p, paths, tab.loc.dir().map(String::from)))
    }

    fn on_rbutton_up(&mut self, x: f32, y: f32) -> bool {
        if self.settings.is_some() {
            return true;
        }
        if self.pal_open {
            if let Hit::PalRow(_) = self.hit(x, y) {
                let pt = self.to_screen(x, y);
                self.pending.push(Deferred::PalMenu { pt });
            }
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
        let Some((pane, paths, dir)) = self.menu_target(x, y) else { return false };
        let pt = self.to_screen(x, y);
        self.pending.push(Deferred::Menu { pane, paths, dir, pt });
        true
    }

    fn on_mbutton_up(&mut self, x: f32, y: f32) {
        if self.settings.is_some() {
            return;
        }
        if self.pal_open {
            if let Hit::PalRow(i) = self.hit(x, y) {
                self.pal_activate(i, palette::OpenMode::NewTab);
            }
            return;
        }
        if self.view_menu.is_some() {
            self.close_view_menu();
            return;
        }
        match self.hit(x, y) {
            Hit::Side(s @ (Side::Fav(crate::favorites::Row::Item { .. }) | Side::Computer | Side::Drive(_) | Side::Folder(_))) => self.side_open(s, true),
            Hit::Tab(p, t) | Hit::TabClose(p, t) => self.close_tab(p, t),
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
            self.settings_wheel(-delta / 120.0 * 3.0 * ROW_H);
            return;
        }
        if self.pal_open && self.pal_rect().contains(x, y) {
            self.pal_wheel(-delta / 120.0 * 3.0 * ROW_H);
            return;
        }
        // 보기 메뉴가 열려 있으면 휠로 뒤쪽 목록이 움직이지 않게
        if self.view_menu.is_some() {
            return;
        }
        if x < SIDEBAR_W {
            self.side_wheel(-delta / 120.0 * 3.0 * ROW_H);
            return;
        }
        if self.preview_rect().is_some_and(|r| r.contains(x, y)) {
            self.preview_wheel(-delta / 120.0 * 3.0 * ROW_H);
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
            Some((_, g)) if g.horiz => g.cell_w,
            Some((_, g)) if matches!(g.cell, CellKind::Tile(_)) => g.cell_h,
            _ => 3.0 * ROW_H,
        };
        self.scroll_by(p, -delta / 120.0 * step);
    }

    fn on_set_cursor(&self) -> bool {
        let id = match self.drag {
            Drag::SplitX | Drag::Col { .. } => IDC_SIZEWE,
            Drag::SplitY => IDC_SIZENS,
            _ => {
                let mut pt = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut pt);
                    let _ = ScreenToClient(self.hwnd, &mut pt);
                }
                match self.hit(pt.x as f32 / self.scale(), pt.y as f32 / self.scale()) {
                    Hit::SplitX | Hit::HeaderResize(..) => IDC_SIZEWE,
                    Hit::SplitY => IDC_SIZENS,
                    Hit::CrumbEmpty(_) => IDC_IBEAM,
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
            Action::Favorites => self.toggle_fav_focus(),
            Action::AddFavorite => self.add_favorite(),
            Action::NewTab => self.new_tab(p, None, true),
            Action::CloseTab => {
                let t = self.panes[p].active;
                self.close_tab(p, t);
            }
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
        if self.settings.is_some() {
            return self.settings_key(vk);
        }
        if self.view_menu.is_some() {
            return self.view_menu_key(vk);
        }
        if self.pal_open {
            return self.pal_key(vk);
        }
        if self.fav_focus {
            return self.fav_key(vk);
        }
        if let Some(a) = self.keys.find(crate::keys::Combo::now(vk.0)) {
            self.run_action(a);
            self.invalidate();
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
            VK_F4 => self.begin_address(p),
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
            // 보기 메뉴가 열려 있거나 키보드가 즐겨찾기에 있으면 필터 상자의 키는 그쪽 조작에 쓴다
            ID_FILTER if self.view_menu.is_some() => self.view_menu_key(vk),
            ID_FILTER if self.fav_focus => self.fav_key(vk),
            ID_FILTER => {
                let empty = unsafe { GetWindowTextLengthW(self.filter_edit) } == 0;
                let text_key = matches!(vk, VK_BACK | VK_DELETE | VK_HOME | VK_END | VK_LEFT | VK_RIGHT);
                let forward = match vk {
                    VK_ESCAPE if !empty => {
                        self.clear_filter();
                        return true;
                    }
                    // 빈 필터를 공백으로 시작하지 않는다
                    VK_SPACE if empty && !ctrl && !alt => return true,
                    _ if text_key && !empty && !alt => false,
                    _ if ctrl && !empty && matches!(vk.0 as u8, b'A' | b'C' | b'X' | b'V' | b'Z') => false,
                    VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_RETURN | VK_TAB | VK_ESCAPE | VK_APPS => true,
                    _ if (VK_F1.0..=VK_F24.0).contains(&vk.0) => true,
                    _ if text_key || ctrl || alt => true,
                    _ => false,
                };
                forward && self.on_key(vk)
            }
            ID_RENAME => match vk {
                VK_RETURN | VK_TAB => {
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
            (ID_RENAME, EditMode::Rename(_) | EditMode::FavRename(_)) => self.commit_rename(),
            (ID_ADDR, EditMode::Address(_)) => self.cancel_edit(),
            _ => {}
        }
    }

    // ---------- 그리기 ----------

    fn paint(&mut self) {
        self.sync_preview();
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
        self.paint_sidebar();
        self.paint_workbar();
        self.paint_caption();
        let mut missing = Vec::new();
        for (p, r) in self.pane_rects() {
            self.paint_pane(p, r, &mut missing);
        }
        self.paint_preview(&mut missing);
        let t = self.theme;
        let (sx, sy) = self.splitters();
        let hot_x = matches!(self.drag, Drag::SplitX) || self.hover == Hit::SplitX;
        let hot_y = matches!(self.drag, Drag::SplitY) || self.hover == Hit::SplitY;
        if let (Some(r), true) = (sx, hot_x) {
            let cx = (r.l + r.r) / 2.0;
            self.gfx.fill_round(R::new(cx - 1.5, r.t + 8.0, cx + 1.5, r.b - 8.0), 1.5, alpha(t.accent, 0.6));
        }
        if let (Some(r), true) = (sy, hot_y) {
            let cy = (r.t + r.b) / 2.0;
            self.gfx.fill_round(R::new(r.l + 8.0, cy - 1.5, r.r - 8.0, cy + 1.5), 1.5, alpha(t.accent, 0.6));
        }
        if self.view_menu.is_some() {
            self.paint_view_menu();
        }
        if self.pal_open {
            self.paint_palette();
        }
        self.paint_tip();
        self.gfx.end();
        self.request_thumbs(missing);
    }

    /// 칸 영역 위 작업 줄: 칸 나누기, 숨김 항목, 미리 보기. 창 단추는 paint_caption
    fn paint_workbar(&self) {
        let t = self.theme;
        let g = &self.gfx;
        let hover_bg = t.sel_inactive;
        // 칸 나누기: 한 덩어리 판 위에 지금 고른 것만 도드라지게
        g.fill_round(layout_pill(), 8.0, sidebar::well(&t));
        for (i, r) in layout_buttons().iter().enumerate() {
            let layout = Layout::ALL[i];
            let current = layout == self.layout;
            if current {
                g.fill_round(*r, 6.0, t.pane);
                g.stroke_round(r.inset(0.5, 0.5), 6.0, t.border, 1.0);
            } else if self.hover == Hit::Layout(i) {
                g.fill_round(*r, 6.0, hover_bg);
            }
            draw_layout_icon(g, *r, layout, if current { t.accent } else { t.dim });
        }
        // 글자 없는 버튼: 올려 두면 이름과 단축키를 말풍선으로 보여 준다.
        // 쉴 때는 칸 나누기 그림과 같은 회색이라 아이콘마다 진하기가 달라 보이지 않는다
        let icon_button = |r: R, glyph: u16, on: bool, hovered: bool| {
            if on {
                g.fill_round(r, 6.0, t.accent_soft);
            } else if hovered {
                g.fill_round(r, 6.0, hover_bg);
            }
            g.glyph(glyph, r, false, if on { t.accent } else if hovered { t.text } else { t.dim });
        };
        icon_button(self.hidden_button(), G_HIDDEN, self.show_hidden, self.hover == Hit::Hidden);
        icon_button(self.preview_button(), G_PREVIEW, self.preview.is_some(), self.hover == Hit::PreviewButton);
    }

    /// 도구 막대 버튼에 잠시 올려 두면 보여 줄 (버튼 자리, 이름과 단축키)
    fn tip(&self) -> Option<(R, String)> {
        if self.pal_open || self.view_menu.is_some() || self.settings.is_some() || !matches!(self.drag, Drag::None) {
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
            Hit::Layout(i) => (layout_buttons()[i], named(LAYOUTS[i], crate::keys::name(LAYOUTS[i]))),
            Hit::NewFolder(p) => (geom(self.pane_rect(p)?).new_folder, named(Action::NewFolder, "새 폴더")),
            Hit::Hidden => (self.hidden_button(), named(Action::Hidden, if self.show_hidden { "숨김 항목 감추기" } else { "숨김 항목 보기" })),
            Hit::Side(s) => return self.side_tip(s),
            Hit::SearchButton => (
                self.search_button(),
                match &self.hotkey {
                    Some(hk) => format!("{} · 다른 앱에서는 {hk}", named(Action::Search, "디스크 전체 검색")),
                    None => named(Action::Search, "디스크 전체 검색"),
                },
            ),
            Hit::PreviewButton => (self.preview_button(), named(Action::Preview, "미리 보기 창")),
            Hit::SettingsButton => (self.settings_button(), named(Action::Settings, "설정")),
            Hit::HelpButton => (self.help_button(), named(Action::Help, "도움말")),
            _ => return None,
        })
    }

    /// 버튼 위에 마우스가 바뀌면 말풍선을 숨기고, 잠시 머무르면 보이게 한다
    fn update_tip(&mut self) {
        self.tip_shown = false;
        unsafe {
            if self.tip().is_some() {
                SetTimer(Some(self.hwnd), TIMER_TIP, 600, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_TIP);
            }
        }
    }

    fn paint_tip(&self) {
        let Some((b, text)) = self.tip().filter(|_| self.tip_shown) else { return };
        let t = self.theme;
        let w = self.gfx.measure(&text, Font::Normal) + 20.0;
        // 사이드바 단추는 오른쪽에(아래쪽 단추가 창 밖으로 나가지 않게), 작업 줄 단추는 아래에
        let r = if b.r <= SIDEBAR_W {
            let cy = (b.t + b.b) / 2.0;
            R::new(b.r + 8.0, cy - 13.0, b.r + 8.0 + w, cy + 13.0)
        } else {
            let l = ((b.l + b.r) / 2.0 - w / 2.0).clamp(MARGIN, (self.w - MARGIN - w).max(MARGIN));
            R::new(l, b.b + 6.0, l + w, b.b + 32.0)
        };
        let shadow = Color { r: 0.0, g: 0.0, b: 0.0, a: if t.dark { 0.4 } else { 0.1 } };
        self.gfx.fill_round(R::new(r.l - 1.0, r.t + 1.0, r.r + 1.0, r.b + 3.0), 7.0, shadow);
        self.gfx.fill_round(r, 6.0, t.pane);
        self.gfx.stroke_round(r.inset(0.5, 0.5), 6.0, t.border, 1.0);
        self.gfx.text(&text, R::new(r.l + 10.0, r.t, r.r - 6.0, r.b), Font::Normal, t.text);
    }

    fn paint_pane(&mut self, p: usize, r: R, missing: &mut Vec<crate::thumbs::Request>) {
        let t = self.theme;
        let active = p == self.active;
        let multi = self.layout.count() > 1;
        let hover = self.hover;
        let drop_hover = self.drop_hover;
        let edit_mode = self.edit_mode;
        let thumb_drag = matches!(self.drag, Drag::Thumb { pane, .. } if pane == p);
        // 끌고 있는 탭: (그 탭이 있던 칸, 탭), 놓을 곳 (칸, 위치)
        let (dragged_tab, tab_drop) = match self.drag {
            Drag::TabMove { pane, tab, x, y } => (Some((pane, tab)), self.tab_drop_target(x, y)),
            _ => (None, None),
        };
        // 경로 상자의 즐겨찾기 별: 폴더일 때만 (켜짐 여부)
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
        let right_w = self.head_right_w(p, &g);
        let fchip = filter_chip(&g);
        let (fedit, fcount, fclear) = self.filter_parts(p, fchip);
        let App { gfx, panes, types, thumb_failed, .. } = self;
        let pane = &panes[p];
        let tab = pane.tab();

        // 카드: 지금 칸은 아래로 옅게 번지는 그림자로 떠 보이고, 다른 칸은 평평하다 (고대비는 테두리로)
        if active && multi && !t.high_contrast {
            let k = if t.dark { 3.0 } else { 1.0 };
            let ink = |a: f32| Color { r: 0.06, g: 0.09, b: 0.16, a: (a * k).min(0.6) };
            gfx.fill_round(R::new(r.l + 10.0, r.t + 14.0, r.r - 10.0, r.b + 10.0), RADIUS + 6.0, ink(0.035));
            gfx.fill_round(R::new(r.l + 4.0, r.t + 8.0, r.r - 4.0, r.b + 6.0), RADIUS + 3.0, ink(0.05));
            gfx.fill_round(R::new(r.l - 0.5, r.t + 1.0, r.r + 0.5, r.b + 2.0), RADIUS + 0.5, ink(0.06));
        }
        gfx.fill_round(r, RADIUS, t.pane);

        // 칩 줄: 탭 칩. 지금 탭은 바탕이 있는 칩(지금 칸이면 옅은 강조 색), 나머지는 글자만이고 올리면 옅은 바탕
        gfx.clip(R::new(r.l + 1.0, g.head.t, r.r - 1.0, g.head.b));
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
                gfx.fill_round(*tr, 7.0, if active { t.accent_soft } else { sidebar::well(&t) });
                // 고대비에서는 바탕색만으로 구분이 어려우니 테두리를 더한다
                if t.high_contrast {
                    gfx.stroke_round(tr.inset(0.5, 0.5), 7.0, if active { t.focus } else { t.pane_border }, 1.0);
                }
            } else if hovered {
                gfx.fill_round(*tr, 7.0, t.hover);
            }
            let closable = pane.tabs.len() > 1 && (current || hovered);
            let label = R::new(tr.l + 12.0, tr.t, tr.r - if closable { 26.0 } else { 10.0 }, tr.b);
            let fg = if current && active { t.chip_text } else { t.text };
            let color = if dragged_tab == Some((p, i)) { alpha(fg, 0.5) } else { fg };
            gfx.text(&pane.tabs[i].loc.title(), label, if current { Font::Strong } else { Font::Normal }, color);
            if closable {
                let cr = close_rect(*tr);
                if hover == Hit::TabClose(p, i) {
                    gfx.fill_round(cr, 5.0, alpha(t.text, 0.12));
                }
                gfx.glyph(G_CLOSE, cr, true, alpha(fg, 0.8));
            }
        }
        // 새 탭(+): 평소엔 흐리게, 올리면 옅은 바탕
        if hover == Hit::TabAdd(p) {
            gfx.fill_round(add, 7.0, t.hover);
        }
        gfx.glyph(G_ADD, add, true, if hover == Hit::TabAdd(p) { t.dim } else { t.faint });
        // 끌어 놓을 자리
        if let Some((_, at)) = tab_drop.filter(|(q, _)| *q == p) {
            let x = trs.get(at).map(|tr| tr.l - 2.0).or(trs.last().map(|tr| tr.r + 2.0)).unwrap_or(g.head.l + 10.0);
            gfx.fill_round(R::new(x - 1.5, add.t, x + 1.5, add.b), 1.5, t.accent);
        }
        gfx.unclip();

        // 칩 줄 오른쪽: 필터 칩(글자를 치면), 아니면 항목 수·고른 개수
        if tab.filter.is_empty() {
            let cr = R::new(g.head.r - 12.0 - right_w, add.t, g.head.r - 12.0, add.b);
            gfx.text(&count_text, cr, Font::Small, t.faint);
        } else {
            gfx.fill_round(fchip, 13.0, t.accent_soft);
            if t.high_contrast {
                gfx.stroke_round(fchip.inset(0.5, 0.5), 13.0, t.border, 1.0);
            }
            gfx.glyph(G_FILTER, R::new(fchip.l + 8.0, fchip.t, fchip.l + 26.0, fchip.b), true, t.accent);
            // 지금 칸에서는 입력 상자(자식 창)가 이 자리에 글자를 그린다
            if !active {
                gfx.text(&tab.filter, fedit, Font::Normal, t.text);
            }
            gfx.text(&format!("{}개", tab.view.len()), fcount, Font::Small, t.dim);
            if hover == Hit::FilterClear(p) {
                gfx.fill_round(fclear, 10.0, alpha(t.text, 0.12));
            }
            gfx.glyph(G_CLOSE, fclear, true, t.dim);
        }

        // 경로 줄: 탐색 단추
        let nav_btn = |gfx: &Gfx, rr: R, glyph: u16, enabled: bool, hovered: bool| {
            if hovered && enabled {
                gfx.fill_round(rr, 6.0, t.hover);
            }
            gfx.glyph(glyph, rr, true, if enabled { t.text } else { t.faint });
        };
        nav_btn(gfx, g.back, G_BACK, !tab.back.is_empty(), hover == Hit::Back(p));
        nav_btn(gfx, g.fwd, G_FWD, !tab.fwd.is_empty(), hover == Hit::Fwd(p));
        nav_btn(gfx, g.up, G_UP, tab.loc != Location::Computer, hover == Hit::Up(p));
        nav_btn(gfx, g.new_folder, G_NEW_FOLDER, tab.loc.dir().is_some(), hover == Hit::NewFolder(p));
        // 보기 방식: 지금 보기 그림과 아래 화살표 (누르면 보기 메뉴)
        if hover == Hit::View(p) || view_menu_open {
            gfx.fill_round(g.view, 6.0, t.hover);
        }
        let view_glyph = if matches!(pane.view, ViewMode::Huge | ViewMode::Large | ViewMode::Medium) { G_GRID } else { G_LIST };
        gfx.glyph(view_glyph, R::new(g.view.l + 2.0, g.view.t, g.view.l + 26.0, g.view.b), true, t.text);
        gfx.glyph(G_CHEVRON_DOWN, R::new(g.view.r - 16.0, g.view.t + 1.0, g.view.r - 4.0, g.view.b + 1.0), true, t.faint);

        // 경로: 상자 없이 글자만. 경로 위에 올리면 옅은 바탕(빈 곳을 누르면 경로 입력), 조각에 올리면 그 조각이 한 단계 진하게
        if edit_mode == EditMode::Address(p) {
            gfx.fill_round(g.crumb, 6.0, t.field);
            gfx.stroke_round(g.crumb.inset(0.75, 0.75), 6.0, t.accent, 1.5);
        } else {
            let over_path = matches!(hover, Hit::CrumbEmpty(hp) | Hit::Crumb(hp, _) | Hit::Star(hp) if hp == p);
            if over_path {
                gfx.fill_round(g.crumb, 6.0, t.hover);
            }
            let cs = crumbs(gfx, g.crumbs, &tab.loc);
            let n = cs.len();
            // 오른쪽 끝 별: 이 폴더가 즐겨찾기에 있으면 노란 별, 없으면 흐린 빈 별
            if let Some(on) = starred {
                if hover == Hit::Star(p) {
                    gfx.fill_round(g.star, 5.0, t.sel_inactive);
                }
                let gold = Color { r: 0.95, g: 0.69, b: 0.0, a: 1.0 };
                gfx.glyph(if on { G_STAR_FILL } else { G_STAR }, g.star, true, if on { gold } else if over_path { t.dim } else { t.faint });
            }
            gfx.clip(g.crumbs);
            for (i, c) in cs.iter().enumerate() {
                let last = i + 1 == n;
                if hover == Hit::Crumb(p, i) {
                    gfx.fill_round(R::new(c.x0, g.crumb.t + 3.0, c.x1, g.crumb.b - 3.0), 4.0, t.sel_inactive);
                }
                gfx.text(
                    &c.label,
                    R::new(c.x0 + CRUMB_PAD, g.crumb.t, c.x1 - CRUMB_PAD / 2.0, g.crumb.b),
                    if last { Font::Strong } else { Font::Normal },
                    if last { t.text } else { t.dim },
                );
                if !last {
                    gfx.glyph(G_CHEVRON, R::new(c.x1, g.crumb.t, c.x1 + CHEV_W, g.crumb.b), true, t.faint);
                }
            }
            gfx.unclip();
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
            gfx.fill(R::new(g.list.l + 8.0, g.header.b - 1.0, g.list.r - 8.0, g.header.b), t.border);
        } else {
            // 아이콘·목록 보기: 열 대신 정렬 버튼. 지금 기준은 바탕이 있는 버튼으로 보인다
            for (key, label, cr) in sort_chips(gfx, g.header, pane.sort.key) {
                let sorted = pane.sort.key == key;
                let hot = hover == Hit::Header(p, key);
                if sorted {
                    gfx.fill_round(cr, 6.0, t.field);
                    if t.high_contrast {
                        gfx.stroke_round(cr.inset(0.5, 0.5), 6.0, t.border, 1.0);
                    }
                } else if hot {
                    gfx.fill_round(cr, 6.0, t.hover);
                }
                let tr = R::new(cr.l + 10.0, cr.t, cr.r - if sorted { 28.0 } else { 10.0 }, cr.b);
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
            let focused = active && vi == tab.cursor && (!e.selected || many_selected);
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
                let tile = R::new(it.l + 2.0, it.t + 2.0, it.r - 4.0, it.b - 4.0);
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
            gfx.icon(if e.icon >= 0 { e.icon } else { type_icon }, R::new(cols.name.0 + 4.0, y + 5.0, cols.name.0 + 20.0, y + 21.0));
            let fg = if e.is_hidden() && !(e.selected && active) { t.faint } else { fg };
            gfx.text_w(e.name(), R::new(cols.name.0 + 28.0, y, cols.name.1 - 8.0, y + ROW_H), Font::Normal, fg);
            if let Some((l, rr)) = cols.modified {
                let s = match e.kind {
                    Kind::Drive if e.size > 0 => format!("{} 사용 가능", format_size(e.free)),
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
            LoadState::Error(msg) => Some(format!("폴더를 열 수 없습니다\n{msg}")),
            LoadState::Ready if tab.entries.is_empty() => Some("빈 폴더입니다".to_string()),
            LoadState::Ready if tab.view.is_empty() && !tab.filter.is_empty() => Some(format!("‘{}’와(과) 일치하는 항목이 없습니다", tab.filter)),
            LoadState::Ready if tab.view.is_empty() => Some("숨김 항목만 있습니다".to_string()),
            _ => None,
        };
        if let Some(h) = hint {
            gfx.text(&h, R::new(g.list.l + 24.0, g.list.t + 24.0, g.list.r - 24.0, g.list.t + 110.0), Font::Hint, t.faint);
        }

        if let Some((_, thumb)) = grid.bar(g.list, tab.view.len(), tab.scroll) {
            let hot = thumb_drag || matches!(hover, Hit::Thumb(hp) | Hit::Track(hp) if hp == p);
            gfx.fill_round(thumb, 3.0, if hot { t.dim } else { t.thumb });
        }

        if let Some(m) = marquee {
            gfx.clip(g.list);
            gfx.fill(m, alpha(t.accent, 0.15));
            gfx.stroke_round(m.inset(0.5, 0.5), 1.0, t.accent, 1.0);
            gfx.unclip();
        }

        // 알림: 칸 아래쪽 가운데 어두운 알약 (몇 초 뒤 사라진다)
        if let Some(m) = &tab.message {
            let tw = gfx.measure(m, Font::Normal).min((r.w() - 80.0).max(40.0));
            let cx = (r.l + r.r) / 2.0;
            let tr = R::new(cx - tw / 2.0 - 16.0, g.list.b - 52.0, cx + tw / 2.0 + 16.0, g.list.b - 16.0);
            let (bg, fg) = if t.high_contrast {
                (t.sel, t.sel_text)
            } else if t.dark {
                (Color { r: 0.92, g: 0.93, b: 0.94, a: 1.0 }, Color { r: 0.12, g: 0.14, b: 0.16, a: 1.0 })
            } else {
                (Color { r: 0.12, g: 0.14, b: 0.16, a: 1.0 }, Color { r: 0.95, g: 0.95, b: 0.96, a: 1.0 })
            };
            gfx.fill_round(R::new(tr.l - 1.0, tr.t + 2.0, tr.r + 1.0, tr.b + 5.0), 12.0, Color { r: 0.0, g: 0.0, b: 0.0, a: if t.dark { 0.35 } else { 0.12 } });
            gfx.fill_round(tr, 10.0, bg);
            gfx.text(m, R::new(tr.l + 16.0, tr.t, tr.r - 14.0, tr.b), Font::Normal, fg);
        }

        // 테두리: 모든 칸이 같은 옅은 선. 끌어 놓을 대상(파일, 다른 칸에서 끌어 온 탭)일 때만 강조 색으로.
        // 고대비에서는 모든 칸을 2px로 두고 활성 칸만 포커스 색으로 바꿔 두께는 그대로 둔다
        let tab_target = tab_drop.is_some_and(|(q, _)| q == p) && dragged_tab.is_some_and(|(from, _)| from != p);
        let dropping = tab_target || matches!(drop_hover, Some((dp, None)) if dp == p);
        let w = if t.high_contrast || dropping { 2.0 } else { 1.0 };
        let color = match () {
            _ if dropping => t.accent,
            _ if t.high_contrast && active && multi => t.focus,
            _ => t.pane_border,
        };
        gfx.stroke_round(r.inset(w / 2.0, w / 2.0), RADIUS, color, w);
    }
}

fn draw_layout_icon(g: &Gfx, r: R, layout: Layout, c: Color) {
    let (cx, cy) = ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0);
    let ic = R::new(cx - 9.0, cy - 7.0, cx + 9.0, cy + 7.0);
    g.stroke_round(ic, 2.0, c, 1.3);
    let v = |x: f32, t: f32, b: f32| g.fill(R::new(x - 0.65, t, x + 0.65, b), c);
    let h = |y: f32, l: f32, rr: f32| g.fill(R::new(l, y - 0.65, rr, y + 0.65), c);
    match layout {
        Layout::One => {}
        Layout::TwoV => v(cx, ic.t, ic.b),
        Layout::TwoH => h(cy, ic.l, ic.r),
        Layout::Three => {
            v(cx, ic.t, ic.b);
            h(cy, cx, ic.r);
        }
        Layout::Four => {
            v(cx, ic.t, ic.b);
            h(cy, ic.l, ic.r);
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
                if path.to_lowercase().ends_with(".lnk") {
                    if let Some(target) = shell::shortcut_target_dir(&path) {
                        with_app(|a| a.navigate(pane, Location::Dir(target), None));
                        continue;
                    }
                }
                shell::open_path(hwnd, &path, dir.as_deref());
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
                    with_app(|a| a.flash(pane, "잘라낸 파일을 옮기는 중입니다"));
                } else if paths.is_empty() {
                    with_app(|a| a.flash(pane, "이미 이 폴더에 있습니다 · 다른 폴더에 붙여넣으세요"));
                } else if let Some(id) = with_app(|a| a.moving.begin(&paths)) {
                    shell::spawn_cut_paste(hwnd, paths, dest, seq, clip, id);
                }
            }
            _ => {
                with_app(|a| a.flash(pane, "클립보드에 붙여넣을 파일이 없습니다"));
            }
        },
        Deferred::Menu { pane, paths, dir, pt } => show_menu(hwnd, MenuRequest { pane, paths, dir, pt }),
        Deferred::PrepareMenu { paths, dir } => prepare_menu(hwnd, paths, dir),
        Deferred::FavMenu { row, pt } => fav_ui::show_fav_menu(hwnd, row, pt),
        Deferred::FavSortMenu { pt } => fav_ui::show_fav_sort_menu(hwnd, pt),
        Deferred::SideMenu { side, pt } => sidebar::show_side_menu(hwnd, side, pt),
        Deferred::PreviewSync => preview::sync_handler(),
        Deferred::Settings { ctl, index, pt } => settings_ui::run_deferred(hwnd, ctl, index, pt),
        Deferred::PalMenu { pt } => palette::show_pal_menu(hwnd, pt),
    }
}

fn remove_search_folder(hwnd: HWND, folders: &crate::folders::Folders, root: &str) {
    let text = format!("이 폴더를 디스크 검색에서 뺄까요?\n{root}");
    if unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Sakadu"), MB_YESNO | MB_ICONQUESTION) } != IDYES {
        return;
    }
    let msg = match folders.remove(root) {
        Ok(()) => format!("검색에서 뺐습니다: {root}"),
        Err(e) => format!("검색 폴더 목록을 저장하지 못했습니다: {e}"),
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
    /// 검색 창 입력 상자 (판과 같은 바탕, 상자 없이)
    search: HWND,
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
    let (bg, brush) = if target == c.rename || target == c.preview_host || target == c.search {
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
                let handled = with_app(|app| app.on_edit_key(id as i32, VIRTUAL_KEY(wp.0 as u16))).unwrap_or(false);
                if let Ok(parent) = GetParent(hwnd) {
                    crate::run_deferred(parent);
                }
                if handled {
                    // 글자를 만드는 키(Enter·Esc·Tab·Backspace·Space·Ctrl 조합)를 처리했을 때만 뒤따르는 WM_CHAR를 삼킨다
                    let vk = VIRTUAL_KEY(wp.0 as u16);
                    let makes_char = matches!(vk, VK_RETURN | VK_ESCAPE | VK_TAB | VK_BACK | VK_SPACE) || key_down(VK_CONTROL);
                    SUPPRESS_CHAR.set(makes_char);
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
                // 보기 메뉴가 열려 있으면 글자가 필터 상자로 들어가지 않게 삼킨다
                if id as i32 == ID_FILTER && with_app(|app| app.view_menu.is_some()).unwrap_or(false) {
                    return LRESULT(0);
                }
                // 키보드가 즐겨찾기에 있으면 글자는 필터 대신 줄 찾기에 쓴다
                if msg == WM_CHAR && id as i32 == ID_FILTER && with_app(|app| app.fav_type_char(wp.0 as u16)).unwrap_or(false) {
                    return LRESULT(0);
                }
            }
            WM_KILLFOCUS => {
                let r = DefSubclassProc(hwnd, msg, wp, lp);
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

    /// 모든 보기에서 칸 가운데를 누르면 그 항목이 잡히고, 보이는 범위에 들어 있고, 경로 상자 오른쪽 보기 버튼과 겹치지 않는다
    #[test]
    fn every_view_maps_items_and_clicks_consistently() {
        let g = geom(R::new(0.0, 0.0, 700.0, 520.0));
        assert!(g.new_folder.l > g.crumb.r && g.view.l > g.new_folder.r && g.view.r <= g.head.r);
        assert!(g.star.r <= g.crumb.r && g.crumbs.r <= g.star.l);
        let n = 137;
        for view in ViewMode::ALL {
            let grid = Grid::of(g.list, view);
            let page = grid.page(g.list);
            let max = (grid.content(n) - page).max(0.0);
            for scroll in [0.0, max / 2.0, max] {
                let shown = grid.visible(g.list, scroll, n);
                for vi in 0..n {
                    let it = grid.item(vi, scroll);
                    let (cx, cy) = ((it.l + it.r) / 2.0, (it.t + it.b) / 2.0);
                    if !g.list.contains(cx, cy) {
                        continue;
                    }
                    assert_eq!(grid.at(cx, cy, scroll, n), Some(vi), "{view:?} scroll={scroll} vi={vi}");
                    assert!(shown.contains(&vi), "{view:?} scroll={scroll} vi={vi} not in {shown:?}");
                }
            }
            // 마지막 항목까지 스크롤하면 끝이 보인다
            let last = grid.item(n - 1, max);
            if grid.horiz {
                assert!(last.r <= g.list.r + 0.5 && last.b <= g.list.b - SCROLL_W, "{view:?} {last:?}");
            } else {
                assert!(last.b <= g.list.b + 0.5, "{view:?} {last:?}");
            }
        }
    }

    /// 목록 보기는 위에서 아래로 채운 뒤 옆 열로 넘어간다 (탐색기와 같다)
    #[test]
    fn list_view_fills_columns_top_to_bottom() {
        let g = geom(R::new(0.0, 0.0, 700.0, 520.0));
        let grid = Grid::of(g.list, ViewMode::List);
        assert!(grid.horiz && grid.rows > 1);
        let (a, b, c) = (grid.item(0, 0.0), grid.item(1, 0.0), grid.item(grid.rows, 0.0));
        assert_eq!(a.l, b.l);
        assert!(b.t > a.t);
        assert_eq!(c.t, a.t);
        assert!(c.l > a.l);
        // 가로로 스크롤하면 왼쪽으로 밀린다
        assert_eq!(grid.item(0, 50.0).l, a.l - 50.0);
    }

    /// 좁은 칸에 탭이 많아도 지금 탭과 새 탭(+)은 칩 줄 안(오른쪽 항목 수 자리 앞)에 보이고 누를 수 있다
    #[test]
    fn many_tabs_keep_active_and_add_reachable() {
        let g = geom(R::new(0.0, 0.0, 380.0, 600.0));
        let strip = chip_strip(&g, 60.0);
        for (n, active) in [(3, 1), (12, 0), (12, 6), (12, 11), (40, 39)] {
            let trs = tab_rects(strip, &vec![120.0; n], active);
            let a = trs[active];
            assert!(a.w() > 0.0 && a.l >= strip.l && a.r <= strip.r, "active tab visible: n={n} active={active} {a:?}");
            let add = add_rect(strip, &trs);
            assert!(add.r <= strip.r, "add button inside strip: n={n} {add:?}");
            let (x, y) = ((a.l + a.r) / 2.0, (a.t + a.b) / 2.0);
            assert_eq!(trs.iter().filter(|r| r.contains(x, y)).count(), 1, "one tab under the cursor");
        }
        // 다 들어가면 칩마다 이름 길이대로 차례로
        let trs = tab_rects(strip, &[60.0, 90.0], 0);
        assert!(trs.windows(2).all(|w| w[0].r < w[1].l));
        assert_eq!((trs[0].w(), trs[1].w()), (60.0, 90.0));
        // 필터 칩은 칩 줄 안, 칩 자리와 겹치지 않는다
        let chip = filter_chip(&g);
        assert!(chip.r <= g.head.r && chip_strip(&g, chip.w()).r <= chip.l);
    }

    /// 회귀: 탭이 가려져 있어도 놓은 자리(보이는 탭 사이)에 들어간다
    #[test]
    fn drop_position_counts_only_visible_tabs() {
        let g = geom(R::new(0.0, 0.0, 380.0, 600.0));
        let strip = chip_strip(&g, 60.0);
        let trs = tab_rects(strip, &[120.0; 12], 6);
        let visible: Vec<usize> = (0..12).filter(|&i| trs[i].w() > 0.0).collect();
        let (first, last) = (visible[0], *visible.last().unwrap());
        assert!(first > 0 && last < 11, "both sides hidden: {visible:?}");
        // 마지막으로 보이는 탭 바로 뒤 → 그 탭 다음 자리 (맨 끝이 아니라)
        assert_eq!(drop_index(&trs, trs[last].r + 1.0), last + 1);
        // 첫 보이는 탭 앞 → 그 탭 자리 (맨 앞이 아니라)
        assert_eq!(drop_index(&trs, trs[first].l + 1.0), first);
        // 보이는 두 탭 사이
        assert_eq!(drop_index(&trs, trs[first + 1].l), first + 1);
        // 다 보일 때는 예전과 같다
        let all = tab_rects(strip, &[60.0; 3], 0);
        assert_eq!((drop_index(&all, 0.0), drop_index(&all, all[2].r + 1.0)), (0, 3));
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
