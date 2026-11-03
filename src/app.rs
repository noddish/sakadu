//! 메인 창: 레이아웃, 패널·탭 동작, 입력 처리, 그리기.
//!
//! 재진입 규칙: App은 RefCell 안에 있으므로, 모달 루프를 돌리는 호출(우클릭 메뉴, 드래그, ShellExecute)은
//! 처리 중에 직접 하지 않고 `pending`에 넣어 빌림이 풀린 뒤 `execute`에서 실행한다.

use crate::gfx::{Font, Gfx, R};
use crate::listing::{Entry, Kind, drive_type_label, format_size, format_time, list_computer, list_dir};
use crate::pane::*;
use crate::shell::{self, FileOp, key_down};
use crate::theme::{Color, Theme, alpha, colorref};
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
use windows::Win32::UI::Controls::{EM_SETCUEBANNER, EM_SETMARGINS, EM_SETSEL, WM_MOUSELEAVE};
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
use fav_ui::{G_STAR, fav_button};
use palette::{G_SEARCH, PalStatus, TIMER_SEARCH, search_button};
use preview::{G_PREVIEW, Preview, TIMER_PREVIEW, thumb_key};
pub use palette::SearchDone;
pub use preview::{TextReady, unload_handler};

const TOOLBAR_H: f32 = 46.0;
const MARGIN: f32 = 8.0;
const GAP: f32 = 6.0;
const TAB_H: f32 = 36.0;
const NAV_H: f32 = 38.0;
const HEADER_H: f32 = 28.0;
const ROW_H: f32 = 26.0;
const STATUS_H: f32 = 32.0;
const SCROLL_W: f32 = 10.0;
const RADIUS: f32 = 8.0;
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

// Segoe Fluent Icons / Segoe MDL2 Assets 글리프
const G_BACK: u16 = 0xE72B;
const G_FWD: u16 = 0xE72A;
const G_UP: u16 = 0xE74A;
const G_ADD: u16 = 0xE710;
const G_CLOSE: u16 = 0xE8BB;
const G_CHEVRON: u16 = 0xE76C;
const G_SORT_UP: u16 = 0xE70E;
const G_SORT_DOWN: u16 = 0xE70D;
const G_NEW_FOLDER: u16 = 0xE8F4;
const G_HIDDEN: u16 = 0xE7B3;
const G_FILTER: u16 = 0xE71C;
const G_LIST: u16 = 0xE8FD;
const G_GRID: u16 = 0xF0E2;
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
    NewFolder,
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
    View(usize, ViewMode),
    Header(usize, SortKey),
    HeaderResize(usize, usize),
    Row(usize, usize),
    ListEmpty(usize),
    Thumb(usize),
    Track(usize),
    Status(usize),
    Pane(usize),
    FavButton,
    FavRow(usize),
    FavEdit(usize),
    FavRemove(usize),
    FavAdd,
    FavSort,
    FavNewGroup,
    /// 검색 창의 "이 폴더도 검색하기" 버튼
    PalAddFolder,
    FavPanel,
    SearchButton,
    PalRow(usize),
    PalInstall,
    PalPanel,
    PreviewButton,
    SettingsButton,
}

impl Hit {
    fn pane(self) -> Option<usize> {
        use Hit::*;
        match self {
            Tab(p, _) | TabClose(p, _) | TabAdd(p) | TabStrip(p) | Back(p) | Fwd(p) | Up(p) | Crumb(p, _) | CrumbEmpty(p) | View(p, _)
            | Header(p, _) | HeaderResize(p, _) | Row(p, _) | ListEmpty(p) | Thumb(p) | Track(p) | Status(p) | Pane(p) => Some(p),
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
    FavPress { index: usize, y: f32 },
    FavMove { from: usize, y: f32 },
    /// 탭을 눌렀다. 조금 움직이면 TabMove
    TabPress { pane: usize, tab: usize, x: f32, y: f32 },
    /// 탭을 끄는 중. (x, y)는 지금 포인터 위치
    TabMove { pane: usize, tab: usize, x: f32, y: f32 },
    /// 빈 곳이나 자세히 보기의 이름 칸 밖을 눌렀다. 조금 움직이면 Marquee. additive는 Ctrl
    MarqueeArm { pane: usize, x: f32, y: f32, additive: bool },
    /// 네모 상자로 고르는 중. (x0, y0)는 시작점(y0은 스크롤 0 기준), (x, y)는 지금 포인터
    Marquee { pane: usize, x0: f32, y0: f32, x: f32, y: f32 },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum EditMode {
    None,
    Address(usize),
    Rename(usize),
    /// 즐겨찾기 표시 이름 바꾸기
    FavRename(usize),
}

struct RenameCtx {
    pane: usize,
    tab: u64,
    path: String,
    old: String,
}

pub enum Deferred {
    Close,
    Open { pane: usize, paths: Vec<String>, dir: Option<String> },
    Menu { pane: usize, paths: Vec<String>, dir: Option<String>, pt: POINT },
    PrepareMenu { paths: Vec<String>, dir: Option<String> },
    FavMenu { index: usize, pt: POINT },
    FavSortMenu { pt: POINT },
    DragOut(Vec<String>),
    Verb { paths: Vec<String>, verb: &'static str },
    Paste { pane: usize, dest: String },
    /// 트레이 대기 중에 창을 닫으면 끝내지 않고 숨긴다
    HideToTray,
    /// 미리 보기 처리기를 지금 상태에 맞춘다 (열기·닫기·크기)
    PreviewSync,
    SettingsMenu { pt: POINT },
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
    fav_open: bool,
    fav_sel: usize,
    fav_scroll: f32,
    search_edit: HWND,
    pal_open: bool,
    pal_results: Vec<(bool, String)>,
    pal_sel: usize,
    pal_scroll: f32,
    pal_status: PalStatus,
    /// 늦게 도착한 이전 검색 결과를 버리기 위한 순번
    pal_seq: u64,
    /// Ctrl+Z로 되돌릴 작업 (마지막이 가장 최근)
    undo: Vec<crate::undo::Undo>,
    /// 실제로 등록된 전역 검색 단축키
    hotkey: Option<&'static str>,
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
    /// Ctrl+F 검색에 넣은 폴더(공유폴더 등 디스크 인덱스 밖)
    folders: crate::folders::Folders,
    /// 검색 창을 열 때 보이는 칸에 열려 있던, 아직 검색에 넣지 않은 공유폴더·USB 폴더 (결과가 없으면 넣기 버튼)
    pal_outside: Option<String>,
}

// ---------- 기하 ----------

struct PaneGeom {
    body: R,
    tabs: R,
    back: R,
    fwd: R,
    up: R,
    crumb: R,
    /// 보기 전환 (자세히, 썸네일)
    views: [R; 2],
    header: R,
    list: R,
    status: R,
    filter: R,
}

fn geom(r: R) -> PaneGeom {
    let tabs = R::new(r.l, r.t, r.r, r.t + TAB_H);
    let nav = R::new(r.l, tabs.b, r.r, tabs.b + NAV_H);
    let back = R::new(nav.l + 8.0, nav.t + 5.0, nav.l + 36.0, nav.b - 5.0);
    let fwd = R::new(back.r + 2.0, back.t, back.r + 30.0, back.b);
    let up = R::new(fwd.r + 2.0, back.t, fwd.r + 30.0, back.b);
    let thumbs = R::new(nav.r - 8.0 - 28.0, back.t, nav.r - 8.0, back.b);
    let details = R::new(thumbs.l - 30.0, back.t, thumbs.l - 2.0, back.b);
    let crumb = R::new(up.r + 8.0, back.t, (details.l - 6.0).max(up.r + 40.0), back.b);
    let header = R::new(r.l, nav.b, r.r, nav.b + HEADER_H);
    let status = R::new(r.l, r.b - STATUS_H, r.r, r.b);
    let list = R::new(r.l, header.b, r.r, status.t.max(header.b));
    let fw = (status.w() * 0.45).clamp(120.0, 240.0);
    let filter = R::new(status.r - fw - 8.0, status.t + 5.0, status.r - 8.0, status.b - 5.0);
    PaneGeom { body: R::new(r.l, tabs.b, r.r, r.b), tabs, back, fwd, up, crumb, views: [details, thumbs], header, list, status, filter }
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
    let kind = if width >= 540.0 {
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
    Cols { name: (list.l + 8.0, x.max(list.l + 120.0)), modified, kind, size }
}

fn tab_rects(g: &PaneGeom, n: usize) -> Vec<R> {
    let avail = g.tabs.w() - 16.0 - 34.0;
    let w = (avail / n.max(1) as f32).clamp(72.0, 200.0);
    (0..n)
        .map(|i| {
            let l = g.tabs.l + 8.0 + i as f32 * (w + 2.0);
            R::new(l, g.tabs.t + 6.0, l + w, g.tabs.b - 2.0)
        })
        .collect()
}

fn close_rect(tr: R) -> R {
    R::new(tr.r - 26.0, tr.t + 5.0, tr.r - 6.0, tr.b - 5.0)
}

fn add_rect(g: &PaneGeom, tabs: &[R]) -> R {
    let l = tabs.last().map(|r| r.r + 4.0).unwrap_or(g.tabs.l + 8.0);
    R::new(l, g.tabs.t + 8.0, l + 26.0, g.tabs.b - 4.0)
}

const TILE_W: f32 = 124.0;
const TILE_H: f32 = 148.0;
/// 썸네일 칸 안의 그림 영역(DIP)
const THUMB: f32 = 96.0;

/// 목록 배치: 자세히(한 줄에 하나) 또는 썸네일 격자. 그리기·클릭·스크롤·키 이동이 모두 이것을 쓴다
#[derive(Clone, Copy)]
struct Grid {
    ox: f32,
    oy: f32,
    cols: usize,
    cell_w: f32,
    cell_h: f32,
    thumbs: bool,
}

impl Grid {
    fn of(list: R, view: ViewMode) -> Grid {
        match view {
            ViewMode::Details => Grid { ox: list.l + 4.0, oy: list.t, cols: 1, cell_w: list.w() - SCROLL_W - 10.0, cell_h: ROW_H, thumbs: false },
            ViewMode::Thumbs => {
                let usable = (list.w() - SCROLL_W - 14.0).max(TILE_W);
                let cols = ((usable / TILE_W).floor() as usize).max(1);
                // 남는 폭을 칸에 고르게 나눠 오른쪽이 비지 않게
                Grid { ox: list.l + 6.0, oy: list.t + 6.0, cols, cell_w: usable / cols as f32, cell_h: TILE_H, thumbs: true }
            }
        }
    }

    fn content_h(&self, n: usize) -> f32 {
        n.div_ceil(self.cols) as f32 * self.cell_h + if self.thumbs { 12.0 } else { 0.0 }
    }

    fn item(&self, vi: usize, scroll: f32) -> R {
        let x = self.ox + (vi % self.cols) as f32 * self.cell_w;
        let y = self.oy + (vi / self.cols) as f32 * self.cell_h - scroll;
        R::new(x, y, x + self.cell_w, y + self.cell_h)
    }

    fn at(&self, x: f32, y: f32, scroll: f32, n: usize) -> Option<usize> {
        let (fx, fy) = ((x - self.ox) / self.cell_w, (y - self.oy + scroll) / self.cell_h);
        if fx < 0.0 || fy < 0.0 || fx as usize >= self.cols {
            return None;
        }
        let vi = fy as usize * self.cols + fx as usize;
        (vi < n).then_some(vi)
    }

    /// 화면에 보이는 항목 범위
    fn visible(&self, list: R, scroll: f32, n: usize) -> std::ops::Range<usize> {
        let first_row = ((scroll - (self.oy - list.t)) / self.cell_h).floor().max(0.0) as usize;
        let rows = (list.h() / self.cell_h).ceil() as usize + 1;
        (first_row * self.cols).min(n)..((first_row + rows) * self.cols).min(n)
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

fn layout_buttons() -> [R; 5] {
    std::array::from_fn(|i| {
        let l = MARGIN + i as f32 * 36.0;
        R::new(l, 8.0, l + 32.0, 38.0)
    })
}

fn new_folder_button() -> R {
    let l = MARGIN + 5.0 * 36.0 + 14.0;
    R::new(l, 8.0, l + 92.0, 38.0)
}

fn hidden_button() -> R {
    let l = new_folder_button().r + 6.0;
    R::new(l, 8.0, l + 104.0, 38.0)
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

fn make_edit_font(dpi: f32) -> HFONT {
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
            w!("Segoe UI"),
        )
    }
}

impl App {
    pub fn new(hwnd: HWND, sess: Option<session::Session>) -> windows::core::Result<App> {
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32;
        let gfx = Gfx::new(dpi, shell::system_imagelist())?;
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
            let view = if ps.is_some_and(|p| p.view == 1) { ViewMode::Thumbs } else { ViewMode::Details };
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
            theme: Theme::system(),
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
            fav_open: false,
            fav_sel: 0,
            fav_scroll: 0.0,
            search_edit,
            pal_open: false,
            pal_results: Vec::new(),
            pal_sel: 0,
            pal_scroll: 0.0,
            pal_status: PalStatus::Idle,
            pal_seq: 0,
            undo: Vec::new(),
            hotkey: None,
            thumb_wanted: HashSet::new(),
            thumb_failed: HashSet::new(),
            preview: sess.as_ref().is_some_and(|s| s.preview).then(Preview::new),
            preview_host,
            tray: None,
            stale: [false; 4],
            marquee_base: Vec::new(),
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
            // 다른 프로그램이 이미 쓰는 키는 건너뛰고 비어 있는 첫 후보를 쓴다
            let candidates = [
                (MOD_CONTROL | MOD_ALT, VK_SPACE.0 as u32, "Ctrl+Alt+Space"),
                (MOD_WIN | MOD_SHIFT, b'F' as u32, "Win+Shift+F"),
                (MOD_CONTROL | MOD_ALT, b'F' as u32, "Ctrl+Alt+F"),
            ];
            self.hotkey = candidates.iter().find(|(m, vk, _)| RegisterHotKey(Some(self.hwnd), HOTKEY_SEARCH, *m | MOD_NOREPEAT, *vk).is_ok()).map(|c| c.2);
            crate::diag::log(|| format!("hotkey -> {:?}", self.hotkey));
        }
        for p in 0..self.layout.count() {
            self.load(p, false);
        }
        self.sync_watches();
        let filter = self.panes[self.active].tab().filter.clone();
        self.set_filter_edit_text(&filter);
        self.layout_edits();
        self.update_title();
    }

    pub fn focus(&self) {
        unsafe {
            let _ = SetFocus(Some(self.focus_target()));
        }
    }

    fn focus_target(&self) -> HWND {
        match self.edit_mode {
            EditMode::Address(_) => self.addr_edit,
            EditMode::Rename(_) | EditMode::FavRename(_) => self.rename_edit,
            EditMode::None if self.pal_open => self.search_edit,
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

    fn area(&self) -> R {
        let right = self.w - MARGIN - if self.preview.is_some() { self.preview_w() + GAP } else { 0.0 };
        R::new(MARGIN, TOOLBAR_H, right.max(MARGIN + 100.0), (self.h - MARGIN).max(TOOLBAR_H + 100.0))
    }

    /// 도구 막대 오른쪽 끝: 설정
    fn settings_button(&self) -> R {
        R::new(self.w - MARGIN - 32.0, 8.0, self.w - MARGIN, 38.0)
    }

    fn preview_button(&self) -> R {
        let s = self.settings_button();
        R::new(s.l - 36.0, 8.0, s.l - 4.0, 38.0)
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

    fn hit(&self, x: f32, y: f32) -> Hit {
        if self.pal_open {
            if let Some(h) = self.pal_hit(x, y) {
                return h;
            }
        }
        if y < TOOLBAR_H && search_button().contains(x, y) {
            return Hit::SearchButton;
        }
        if self.fav_open {
            if let Some(h) = self.fav_hit(x, y) {
                return h;
            }
        }
        if y < TOOLBAR_H {
            if fav_button().contains(x, y) {
                return Hit::FavButton;
            }
            if let Some(i) = layout_buttons().iter().position(|r| r.contains(x, y)) {
                return Hit::Layout(i);
            }
            if new_folder_button().contains(x, y) {
                return Hit::NewFolder;
            }
            if hidden_button().contains(x, y) {
                return Hit::Hidden;
            }
            if self.preview_button().contains(x, y) {
                return Hit::PreviewButton;
            }
            if self.settings_button().contains(x, y) {
                return Hit::SettingsButton;
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
            if g.tabs.contains(x, y) {
                let trs = tab_rects(&g, pane.tabs.len());
                for (i, tr) in trs.iter().enumerate() {
                    if tr.contains(x, y) {
                        if pane.tabs.len() > 1 && close_rect(*tr).contains(x, y) {
                            return Hit::TabClose(p, i);
                        }
                        return Hit::Tab(p, i);
                    }
                }
                if add_rect(&g, &trs).contains(x, y) {
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
            if g.views[0].contains(x, y) {
                return Hit::View(p, ViewMode::Details);
            }
            if g.views[1].contains(x, y) {
                return Hit::View(p, ViewMode::Thumbs);
            }
            if g.crumb.contains(x, y) {
                let cs = crumbs(&self.gfx, g.crumb, &tab.loc);
                return match cs.iter().position(|c| x >= c.x0 && x < c.x1) {
                    Some(i) => Hit::Crumb(p, i),
                    None => Hit::CrumbEmpty(p),
                };
            }
            if g.header.contains(x, y) {
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
            if g.status.contains(x, y) {
                return Hit::Status(p);
            }
            if g.list.contains(x, y) {
                let grid = Grid::of(g.list, pane.view);
                if let Some((track, thumb)) = scroll_metrics(g.list, grid.content_h(tab.view.len()), tab.scroll) {
                    if x >= track.l {
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
        self.close_palette();
        self.close_favorites();
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

    pub fn new_tab(&mut self, p: usize, loc: Option<Location>, activate: bool) {
        let loc = loc.unwrap_or_else(|| self.panes[p].tab().loc.clone());
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
        let at = if g.tabs.contains(x, y) { tab_rects(&g, n).iter().filter(|tr| (tr.l + tr.r) / 2.0 < x).count() } else { n };
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
        let max = (grid.content_h(tab.view.len()) - list.h()).max(0.0);
        tab.scroll = tab.scroll.clamp(0.0, max);
    }

    fn ensure_visible(&mut self, p: usize) {
        let Some((list, grid)) = self.grid(p) else { return };
        let tab = self.panes[p].tab_mut();
        // 스크롤 0 기준 위치
        let it = grid.item(tab.cursor, 0.0);
        let (top, bottom) = (it.t - list.t, it.b - list.t);
        if top < tab.scroll {
            tab.scroll = top;
        } else if bottom > tab.scroll + list.h() {
            tab.scroll = bottom - list.h();
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
        self.grid(p).map(|(l, g)| ((l.h() / g.cell_h).floor() as isize).max(1) * g.cols as isize).unwrap_or(10)
    }

    // ---------- 입력 상자 ----------

    fn apply_edit_font(&mut self) {
        let font = make_edit_font(self.gfx.dpi);
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
        if it.t < list.t - 1.0 || it.b > list.b + 1.0 {
            return None;
        }
        if grid.thumbs {
            // 썸네일 아래 이름 자리
            let y = it.t + 8.0 + THUMB + 6.0;
            return Some(R::new(it.l + 4.0, y, it.r - 8.0, y + 24.0));
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
        // 즐겨찾기 팝업이 열린 동안은 필터 상자(자식 창)가 팝업을 가리지 않게 화면 밖에 둔다. 포커스는 유지
        let filter = if self.fav_open || self.pal_open {
            Some(R::new(-4000.0, -4000.0, -3800.0, -3980.0))
        } else {
            self.pane_rect(self.active).map(|r| center(geom(r).filter, 26.0, 8.0))
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
            EditMode::FavRename(i) if self.fav_open => self.fav_rename_rect(i).map(|r| center(r, 4.0, 4.0)),
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
        if let EditMode::FavRename(i) = self.edit_mode {
            self.commit_fav_rename(i);
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

    fn flash(&mut self, p: usize, msg: impl Into<String>) {
        self.panes[p].tab_mut().message = Some(msg.into());
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
        match self.hotkey {
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
        let paths = tab.selected_paths();
        if !paths.is_empty() {
            shell::spawn_file_op(self.hwnd, FileOp::Delete { paths, permanent });
        }
    }

    fn copy(&mut self, p: usize, cut: bool) {
        let paths = self.panes[p].tab().selected_paths();
        if paths.is_empty() {
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
        match std::fs::create_dir(util::join(&dir, &name)) {
            Ok(()) => {
                self.push_undo(crate::undo::Undo::Create(util::join(&dir, &name)));
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
                rename: self.rename_edit,
                preview_host: self.preview_host,
            }
        };
        if let Some(old) = EDIT_COLORS.replace(Some(colors)) {
            unsafe {
                let _ = DeleteObject(old.field_brush.into());
                let _ = DeleteObject(old.pane_brush.into());
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
            panes: self
                .panes
                .iter()
                .map(|p| session::PaneState {
                    tabs: p.tabs.iter().map(|t| t.loc.to_session()).collect(),
                    active: p.active,
                    cols: p.cols,
                    sort_key: p.sort.key.to_u8(),
                    sort_desc: p.sort.desc,
                    view: (p.view == ViewMode::Thumbs) as u8,
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

    pub fn on_dpi(&mut self, dpi: f32) {
        self.gfx.set_dpi(dpi);
        self.apply_edit_font();
    }

    fn on_size(&mut self, wpx: u32, hpx: u32) {
        self.w = wpx as f32 / self.scale();
        self.h = hpx as f32 / self.scale();
        self.gfx.resize(wpx, hpx);
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.layout_edits();
        self.invalidate();
    }

    // ---------- 드래그 앤 드롭 ----------

    /// 드롭 위치의 대상 폴더. 폴더 행 위면 그 폴더, 패널 안 다른 곳이면 패널의 현재 폴더
    pub fn drop_dir_at(&mut self, pt: POINTL) -> Option<String> {
        let mut cp = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut cp);
        }
        let (x, y) = (cp.x as f32 / self.scale(), cp.y as f32 / self.scale());
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
        dir
    }

    pub fn clear_drop_hover(&mut self) {
        if self.drop_hover.take().is_some() {
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
                    // 다른 창으로 전환하면 즐겨찾기 팝업을 닫는다 (WA_INACTIVE)
                    if loword(wp.0) == 0 {
                        self.close_favorites();
                        self.close_palette();
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
                    if id == TIMER_SEARCH {
                        self.pal_run_query();
                    } else if id == TIMER_PREVIEW {
                        self.start_preview();
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
                WM_SETTINGCHANGE => {
                    if lp.0 != 0 && PCWSTR(lp.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                        self.theme = Theme::system();
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
                WM_APP_LISTED => self.on_listed(*Box::from_raw(lp.0 as *mut Listed)),
                WM_APP_ICONS => self.on_icons(*Box::from_raw(lp.0 as *mut IconsReady)),
                WM_APP_SEARCH => self.on_search_done(*Box::from_raw(lp.0 as *mut SearchDone)),
                crate::WM_APP_UNDO => self.push_undo(*Box::from_raw(lp.0 as *mut crate::undo::Undo)),
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
                let content = grid.content_h(tab.view.len());
                if let Some((track, thumb)) = scroll_metrics(list, content, tab.scroll) {
                    let max = content - list.h();
                    let ratio = (y - grab - track.t) / (track.h() - thumb.h()).max(1.0);
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
                    let y0 = y0 + tab.scroll;
                    self.update_marquee(pane, x0, y0, x, y);
                }
            }
            Drag::Marquee { pane, x0, y0, .. } => self.update_marquee(pane, x0, y0, x, y),
            Drag::None => {
                let h = self.hit(x, y);
                if h != self.hover {
                    self.hover = h;
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
        // 목록 위·아래로 벗어나면 그쪽으로 스크롤
        if y < list.t {
            self.scroll_by(p, -(list.t - y).min(40.0));
        } else if y > list.b {
            self.scroll_by(p, (y - list.b).min(40.0));
        }
        let tab = self.panes[p].tab_mut();
        let (t, b) = (y0.min(y + tab.scroll), y0.max(y + tab.scroll));
        let (l, r) = (x0.min(x), x0.max(x));
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
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        if self.pal_open && hit != Hit::SearchButton {
            self.pal_mouse_down(hit);
            self.invalidate();
            return;
        }
        if self.fav_open && hit != Hit::FavButton {
            self.end_edits_for_click();
            self.fav_mouse_down(hit, y);
            self.invalidate();
            return;
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
            Hit::FavButton => self.toggle_favorites(),
            Hit::SearchButton => self.toggle_palette(),
            Hit::NewFolder => self.new_folder(self.active),
            Hit::Hidden => self.toggle_hidden(),
            Hit::PreviewButton => self.toggle_preview(),
            Hit::SettingsButton => {
                let r = self.settings_button();
                let pt = self.to_screen(r.r, r.b + 2.0);
                self.pending.push(Deferred::SettingsMenu { pt });
            }
            Hit::View(p, v) => self.set_view(p, v),
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
                if let Some(c) = crumbs(&self.gfx, geom(r).crumb, &cur).into_iter().nth(i) {
                    if c.target == cur {
                        self.refresh(p);
                    } else {
                        let select = child_toward(&c.target, &cur);
                        self.navigate(p, c.target, select);
                    }
                }
            }
            Hit::CrumbEmpty(p) => self.begin_address(p),
            Hit::Header(p, key) => self.toggle_sort(p, key),
            Hit::HeaderResize(p, col) => {
                let start_w = self.panes[p].cols[col];
                capture(self, Drag::Col { pane: p, col, start_x: x, start_w });
            }
            Hit::Thumb(p) => {
                let Some((list, grid)) = self.grid(p) else { return };
                let tab = self.panes[p].tab();
                if let Some((_, thumb)) = scroll_metrics(list, grid.content_h(tab.view.len()), tab.scroll) {
                    capture(self, Drag::Thumb { pane: p, grab: y - thumb.t });
                }
            }
            Hit::Track(p) => {
                let Some((list, grid)) = self.grid(p) else { return };
                let tab = self.panes[p].tab();
                if let Some((_, thumb)) = scroll_metrics(list, grid.content_h(tab.view.len()), tab.scroll) {
                    self.scroll_by(p, if y < thumb.t { -list.h() } else { list.h() });
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
            Hit::Status(_) => self.focus(),
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
        if self.pal_open {
            if !matches!(hit, Hit::PalRow(_) | Hit::PalPanel | Hit::PalInstall | Hit::PalAddFolder) {
                self.close_palette();
            }
            self.invalidate();
            return;
        }
        if self.fav_open {
            match hit {
                Hit::FavRow(i) | Hit::FavEdit(i) | Hit::FavRemove(i) => {
                    self.end_edits_for_click();
                    self.fav_sel = i;
                }
                Hit::FavAdd | Hit::FavSort | Hit::FavNewGroup | Hit::FavPanel | Hit::FavButton => {}
                _ => self.close_favorites(),
            }
            self.invalidate();
            return;
        }
        if let Some(p) = hit.pane() {
            self.activate_pane(p);
        }
        self.end_edits_for_click();
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
            Hit::ListEmpty(p) | Hit::Pane(p) | Hit::Header(p, _) | Hit::Status(p) => (p, false),
            _ => return None,
        };
        let tab = self.panes[p].tab();
        let paths = if items { tab.selected_paths() } else { Vec::new() };
        Some((p, paths, tab.loc.dir().map(String::from)))
    }

    fn on_rbutton_up(&mut self, x: f32, y: f32) -> bool {
        if self.pal_open {
            return true;
        }
        if self.fav_open {
            if let Hit::FavRow(i) | Hit::FavEdit(i) | Hit::FavRemove(i) = self.hit(x, y) {
                let pt = self.to_screen(x, y);
                self.pending.push(Deferred::FavMenu { index: i, pt });
            }
            return true;
        }
        let Some((pane, paths, dir)) = self.menu_target(x, y) else { return false };
        let pt = self.to_screen(x, y);
        self.pending.push(Deferred::Menu { pane, paths, dir, pt });
        true
    }

    fn on_mbutton_up(&mut self, x: f32, y: f32) {
        if self.pal_open {
            if let Hit::PalRow(i) = self.hit(x, y) {
                self.pal_activate(i, palette::OpenMode::NewTab);
            }
            return;
        }
        if self.fav_open {
            if let Hit::FavRow(i) = self.hit(x, y) {
                self.fav_open_item(i, true);
            }
            return;
        }
        match self.hit(x, y) {
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
        if self.pal_open && self.pal_rect().contains(x, y) {
            self.pal_wheel(-delta / 120.0 * 3.0 * ROW_H);
            return;
        }
        if self.fav_open && self.fav_rect().contains(x, y) {
            self.fav_wheel(-delta / 120.0 * 3.0 * ROW_H);
            return;
        }
        if self.preview_rect().is_some_and(|r| r.contains(x, y)) {
            self.preview_wheel(-delta / 120.0 * 3.0 * ROW_H);
            return;
        }
        let p = self.hit(x, y).pane().unwrap_or(self.active);
        // 썸네일 보기에서는 한 칸에 한 줄씩
        let step = if self.panes[p].view == ViewMode::Thumbs { TILE_H } else { 3.0 * ROW_H };
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
        let cols = self.grid(p).map(|(_, g)| g.cols as isize).unwrap_or(1);
        let tab = self.panes[p].tab_mut();
        let n = tab.view.len();
        if n == 0 {
            return;
        }
        let cur = tab.cursor as isize;
        let last = n as isize - 1;
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
    fn on_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        if self.pal_open {
            return self.pal_key(vk);
        }
        if self.fav_open {
            return self.fav_key(vk);
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
            VK_LEFT | VK_RIGHT if !alt && self.panes[p].view == ViewMode::Thumbs => self.move_cursor_key(p, vk, shift, ctrl),
            VK_RETURN if alt => self.properties(p),
            VK_RETURN => self.open_items(p, ctrl),
            VK_BACK if !ctrl && !alt => self.go_up(p),
            VK_F2 => self.begin_rename(p),
            VK_F4 => self.begin_address(p),
            VK_F5 => self.refresh(p),
            VK_DELETE if !ctrl && !alt => self.delete(p, shift),
            VK_ESCAPE => self.escape(p),
            VK_APPS => self.context_menu_at_cursor(p),
            VK_F10 if shift => self.context_menu_at_cursor(p),
            _ if ctrl && !alt => {
                let c = vk.0 as u8;
                if vk.0 > 0xFF {
                    return false;
                }
                match c {
                    b'A' => self.panes[p].tab_mut().select_all(),
                    b'C' => self.copy(p, false),
                    b'X' => self.copy(p, true),
                    b'V' => self.paste(p),
                    b'T' => self.new_tab(p, None, true),
                    b'W' => {
                        let t = self.panes[p].active;
                        self.close_tab(p, t);
                    }
                    b'H' => self.toggle_hidden(),
                    b'B' => self.toggle_favorites(),
                    b'F' => self.toggle_palette(),
                    b'Z' => self.undo_last(),
                    b'D' => self.add_favorite(),
                    b'N' if shift => self.new_folder(p),
                    b'L' => self.begin_address(p),
                    // 탐색기와 같은 보기 단축키
                    b'2' if shift => self.set_view(p, ViewMode::Thumbs),
                    b'6' if shift => self.set_view(p, ViewMode::Details),
                    b'1'..=b'5' if !shift => self.set_layout(Layout::ALL[(c - b'1') as usize]),
                    _ => return false,
                }
            }
            _ if alt && vk.0 == b'D' as u16 => self.begin_address(p),
            _ if alt && vk.0 == b'P' as u16 => self.toggle_preview(),
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
            // 즐겨찾기 팝업이 열려 있으면 필터 상자의 키는 팝업 조작에 쓴다
            ID_FILTER if self.fav_open => self.fav_key(vk),
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
        self.paint_toolbar();
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
        if self.fav_open {
            self.paint_favorites();
        }
        if self.pal_open {
            self.paint_palette();
        }
        self.gfx.end();
        self.request_thumbs(missing);
    }

    fn paint_toolbar(&self) {
        let t = self.theme;
        let g = &self.gfx;
        let hover_bg = t.sel_inactive;
        for (i, r) in layout_buttons().iter().enumerate() {
            let layout = Layout::ALL[i];
            let current = layout == self.layout;
            if current {
                g.fill_round(*r, 6.0, t.accent_soft);
            } else if self.hover == Hit::Layout(i) {
                g.fill_round(*r, 6.0, hover_bg);
            }
            draw_layout_icon(g, *r, layout, if current { t.accent } else { t.dim });
        }
        let button = |r: R, glyph: u16, label: &str, on: bool, hovered: bool| {
            if on {
                g.fill_round(r, 6.0, t.accent_soft);
            } else if hovered {
                g.fill_round(r, 6.0, hover_bg);
            }
            let fg = if on { t.accent } else { t.text };
            g.glyph(glyph, R::new(r.l + 8.0, r.t, r.l + 28.0, r.b), false, fg);
            g.text(label, R::new(r.l + 32.0, r.t, r.r - 6.0, r.b), Font::Normal, fg);
        };
        button(new_folder_button(), G_NEW_FOLDER, "새 폴더", false, self.hover == Hit::NewFolder);
        button(hidden_button(), G_HIDDEN, "숨김 항목", self.show_hidden, self.hover == Hit::Hidden);
        button(fav_button(), G_STAR, "즐겨찾기", self.fav_open, self.hover == Hit::FavButton);
        // 검색 상자처럼 보이는 버튼: 누르면 Ctrl+F 팔레트
        let sb = search_button();
        let on = self.pal_open || self.hover == Hit::SearchButton;
        g.fill_round(sb, 8.0, if on { t.pane } else { t.field });
        g.stroke_round(sb.inset(0.5, 0.5), 8.0, if self.pal_open { t.accent } else { t.border }, 1.0);
        g.glyph(G_SEARCH, R::new(sb.l + 8.0, sb.t, sb.l + 28.0, sb.b), true, t.dim);
        g.text("검색", R::new(sb.l + 30.0, sb.t, sb.r - 8.0, sb.b), Font::Normal, t.dim);
        g.text("Ctrl+F", R::new(sb.l + 30.0, sb.t, sb.r - 10.0, sb.b), Font::Right, t.faint);
        // 글자 없는 버튼: 올려 두면 오른쪽 안내 문구 자리에 설명을 보여 준다
        let icon_button = |r: R, glyph: u16, on: bool, hovered: bool| {
            if on {
                g.fill_round(r, 6.0, t.accent_soft);
            } else if hovered {
                g.fill_round(r, 6.0, hover_bg);
            }
            g.glyph(glyph, r, false, if on { t.accent } else { t.text });
        };
        let (pb, setb) = (self.preview_button(), self.settings_button());
        icon_button(pb, G_PREVIEW, self.preview.is_some(), self.hover == Hit::PreviewButton);
        icon_button(setb, G_SETTINGS, false, self.hover == Hit::SettingsButton);
        let hint = match (self.hover, self.hotkey) {
            (Hit::PreviewButton, _) => "미리 보기 창 (Alt+P)".to_string(),
            (Hit::SettingsButton, _) => "설정: 보기, 기본 파일 관리자, 검색 서비스".to_string(),
            (_, Some(hk)) => format!("{hk} 어디서든 검색 · Ctrl+B 즐겨찾기 · Ctrl+Z 되돌리기 · 입력하면 필터"),
            (_, None) => "Ctrl+B 즐겨찾기 · Ctrl+Z 되돌리기 · Ctrl+1~5 레이아웃 · 입력하면 필터".to_string(),
        };
        let right = pb.l - 10.0;
        let left = sb.r + 24.0;
        if right - left > g.measure(&hint, Font::Normal) {
            g.text(&hint, R::new(left, 8.0, right, 38.0), Font::Right, t.faint);
        }
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
        // 네모 상자 고르기 (화면 좌표)
        let marquee = match self.drag {
            Drag::Marquee { pane, x0, y0, x, y } if pane == p => {
                let y0 = y0 - self.panes[p].tab().scroll;
                Some(R::new(x0.min(x), y0.min(y), x0.max(x), y0.max(y)))
            }
            _ => None,
        };
        let App { gfx, panes, types, thumb_failed, .. } = self;
        let pane = &panes[p];
        let tab = pane.tab();
        let g = geom(r);

        // 바탕: 탭 줄은 조금 어둡게, 본문은 패널 색
        gfx.fill_round(r, RADIUS, t.strip);
        gfx.fill(R::new(r.l, g.body.t, r.r, g.body.t + RADIUS), t.pane);
        gfx.fill_round(g.body, RADIUS, t.pane);

        // 탭
        gfx.clip(R::new(r.l + 1.0, g.tabs.t, r.r - 1.0, g.tabs.b));
        let trs = tab_rects(&g, pane.tabs.len());
        for (i, tr) in trs.iter().enumerate() {
            let is_active = i == pane.active;
            let hovered = matches!(hover, Hit::Tab(hp, ht) | Hit::TabClose(hp, ht) if hp == p && ht == i);
            if is_active {
                gfx.fill_round(*tr, 7.0, t.pane);
                gfx.fill(R::new(tr.l, tr.b - 8.0, tr.r, g.tabs.b + 1.0), t.pane);
            } else if hovered {
                gfx.fill_round(*tr, 7.0, t.sel_inactive);
            }
            let closable = pane.tabs.len() > 1 && (is_active || hovered);
            let label = R::new(tr.l + 12.0, tr.t, tr.r - if closable { 28.0 } else { 10.0 }, tr.b);
            let color = if dragged_tab == Some((p, i)) { t.faint } else if is_active { t.text } else { t.dim };
            gfx.text(&pane.tabs[i].loc.title(), label, if is_active { Font::Strong } else { Font::Normal }, color);
            if closable {
                let cr = close_rect(*tr);
                if hover == Hit::TabClose(p, i) {
                    gfx.fill_round(cr, 4.0, t.sel_inactive);
                }
                gfx.glyph(G_CLOSE, cr, true, t.dim);
            }
        }
        let add = add_rect(&g, &trs);
        if hover == Hit::TabAdd(p) {
            gfx.fill_round(add, 6.0, t.sel_inactive);
        }
        gfx.glyph(G_ADD, add, true, t.dim);
        // 끌어 놓을 자리
        if let Some((_, at)) = tab_drop.filter(|(q, _)| *q == p) {
            let x = trs.get(at).map(|tr| tr.l - 2.0).or(trs.last().map(|tr| tr.r + 2.0)).unwrap_or(g.tabs.l + 8.0);
            gfx.fill_round(R::new(x - 1.5, g.tabs.t + 6.0, x + 1.5, g.tabs.b - 2.0), 1.5, t.accent);
        }
        gfx.unclip();

        // 탐색 버튼
        let nav_btn = |gfx: &Gfx, rr: R, glyph: u16, enabled: bool, hovered: bool| {
            if hovered && enabled {
                gfx.fill_round(rr, 6.0, t.hover);
            }
            gfx.glyph(glyph, rr, false, if enabled { t.text } else { t.faint });
        };
        nav_btn(gfx, g.back, G_BACK, !tab.back.is_empty(), hover == Hit::Back(p));
        nav_btn(gfx, g.fwd, G_FWD, !tab.fwd.is_empty(), hover == Hit::Fwd(p));
        nav_btn(gfx, g.up, G_UP, tab.loc != Location::Computer, hover == Hit::Up(p));
        for (vr, mode, glyph) in [(g.views[0], ViewMode::Details, G_LIST), (g.views[1], ViewMode::Thumbs, G_GRID)] {
            let on = pane.view == mode;
            if on {
                gfx.fill_round(vr, 6.0, t.accent_soft);
            } else if hover == Hit::View(p, mode) {
                gfx.fill_round(vr, 6.0, t.hover);
            }
            gfx.glyph(glyph, vr, true, if on { t.accent } else { t.dim });
        }

        // 경로 표시줄
        gfx.fill_round(g.crumb, 6.0, t.field);
        if edit_mode == EditMode::Address(p) {
            gfx.stroke_round(g.crumb.inset(0.75, 0.75), 6.0, t.accent, 1.5);
        } else {
            let cs = crumbs(gfx, g.crumb, &tab.loc);
            let n = cs.len();
            gfx.clip(g.crumb);
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

        // 행·칸: 보이는 부분만 그린다
        gfx.clip(g.list);
        let grid = Grid::of(g.list, pane.view);
        let thumb_px = (THUMB * gfx.dpi / 96.0).round() as u32;
        for vi in grid.visible(g.list, tab.scroll, tab.view.len()) {
            let e = &tab.entries[tab.view[vi] as usize];
            let it = grid.item(vi, tab.scroll);
            if grid.thumbs {
                let tile = R::new(it.l + 2.0, it.t + 2.0, it.r - 4.0, it.b - 4.0);
                if e.selected {
                    gfx.fill_round(tile, 6.0, if active { t.sel } else { t.sel_inactive });
                } else if hover == Hit::Row(p, vi) {
                    gfx.fill_round(tile, 6.0, t.hover);
                }
                if drop_hover == Some((p, Some(vi))) {
                    gfx.stroke_round(tile.inset(0.75, 0.75), 6.0, t.accent, 1.5);
                } else if active && vi == tab.cursor && !e.selected {
                    gfx.stroke_round(tile.inset(0.5, 0.5), 6.0, alpha(t.accent, 0.45), 1.0);
                }
                let cx = (it.l + it.r) / 2.0;
                let bx = R::new(cx - THUMB / 2.0, it.t + 8.0, cx + THUMB / 2.0, it.t + 8.0 + THUMB);
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
                    gfx.fill_round(er, 4.0, t.pane);
                    gfx.stroke_round(er.inset(0.75, 0.75), 4.0, t.accent, 1.5);
                } else {
                    gfx.text_w(e.name(), cap, Font::Caption, if e.is_hidden() { t.faint } else { t.text });
                }
                continue;
            }
            let y = it.t;
            let row = R::new(g.list.l + 4.0, y + 1.0, g.list.r - SCROLL_W - 6.0, y + ROW_H - 1.0);
            if e.selected {
                gfx.fill_round(row, 5.0, if active { t.sel } else { t.sel_inactive });
            } else if hover == Hit::Row(p, vi) {
                gfx.fill_round(row, 5.0, t.hover);
            }
            if drop_hover == Some((p, Some(vi))) {
                gfx.stroke_round(row.inset(0.75, 0.75), 5.0, t.accent, 1.5);
            } else if active && vi == tab.cursor && !e.selected {
                gfx.stroke_round(row.inset(0.5, 0.5), 5.0, alpha(t.accent, 0.45), 1.0);
            }
            let (type_name, type_icon) = type_info(types, e);
            gfx.icon(if e.icon >= 0 { e.icon } else { type_icon }, R::new(cols.name.0 + 4.0, y + 5.0, cols.name.0 + 20.0, y + 21.0));
            let fg = if e.is_hidden() { t.faint } else { t.text };
            gfx.text_w(e.name(), R::new(cols.name.0 + 28.0, y, cols.name.1 - 8.0, y + ROW_H), Font::Normal, fg);
            if let Some((l, rr)) = cols.modified {
                let s = match e.kind {
                    Kind::Drive if e.size > 0 => format!("{} 사용 가능", format_size(e.free)),
                    Kind::File | Kind::Dir => format_time(e.modified),
                    _ => String::new(),
                };
                gfx.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Normal, t.dim);
            }
            if let Some((l, rr)) = cols.kind {
                let s = if e.kind == Kind::Drive { drive_type_label(e.drive_type) } else { type_name.as_str() };
                gfx.text(s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Normal, t.dim);
            }
            if let Some((l, rr)) = cols.size {
                let s = match e.kind {
                    Kind::File => format_size(e.size),
                    Kind::Drive if e.size > 0 => format_size(e.size),
                    _ => String::new(),
                };
                gfx.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Right, t.dim);
            }
            if edit_mode == EditMode::Rename(p) && vi == tab.cursor {
                let er = R::new(cols.name.0 + 24.0, y + 2.0, cols.name.1 - 4.0, y + ROW_H - 2.0);
                gfx.fill_round(er, 4.0, t.pane);
                gfx.stroke_round(er.inset(0.75, 0.75), 4.0, t.accent, 1.5);
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

        if let Some((_, thumb)) = scroll_metrics(g.list, grid.content_h(tab.view.len()), tab.scroll) {
            let hot = thumb_drag || matches!(hover, Hit::Thumb(hp) | Hit::Track(hp) if hp == p);
            gfx.fill_round(thumb, 3.0, if hot { t.dim } else { t.thumb });
        }

        // 상태 표시줄과 필터
        gfx.fill(R::new(g.status.l + 8.0, g.status.t, g.status.r - 8.0, g.status.t + 1.0), t.border);
        let (sel, bytes) = tab.selection_stats();
        let status = match (&tab.message, &tab.state) {
            (Some(m), _) => m.clone(),
            (None, LoadState::Loading) => "불러오는 중…".to_string(),
            (None, LoadState::Error(_)) => "열 수 없음".to_string(),
            (None, LoadState::Ready) => {
                let mut s = format!("항목 {}개", tab.view.len());
                if sel > 0 {
                    s.push_str(&format!(" · {sel}개 선택"));
                    if bytes > 0 {
                        s.push_str(&format!(" ({})", format_size(bytes)));
                    }
                }
                s
            }
        };
        let status_color = if tab.message.is_some() { t.accent } else { t.dim };
        gfx.text(&status, R::new(g.status.l + 14.0, g.status.t, g.filter.l - 8.0, g.status.b), Font::Normal, status_color);
        if active || !tab.filter.is_empty() {
            gfx.fill_round(g.filter, 6.0, t.field);
            let glyph_color = if tab.filter.is_empty() { t.faint } else { t.accent };
            gfx.glyph(G_FILTER, R::new(g.filter.l + 4.0, g.filter.t, g.filter.l + 24.0, g.filter.b), true, glyph_color);
            if !active {
                gfx.text(&tab.filter, R::new(g.filter.l + 26.0, g.filter.t, g.filter.r - 6.0, g.filter.b), Font::Normal, t.dim);
            }
        }

        if let Some(m) = marquee {
            gfx.clip(g.list);
            gfx.fill(m, alpha(t.accent, 0.15));
            gfx.stroke_round(m.inset(0.5, 0.5), 1.0, t.accent, 1.0);
            gfx.unclip();
        }

        // 테두리: 드롭 대상(파일, 다른 칸에서 끌어 온 탭) > 활성 패널 > 기본
        let tab_target = tab_drop.is_some_and(|(q, _)| q == p) && dragged_tab.is_some_and(|(from, _)| from != p);
        match drop_hover {
            _ if tab_target => gfx.stroke_round(r.inset(1.0, 1.0), RADIUS, t.accent, 2.0),
            Some((dp, None)) if dp == p => gfx.stroke_round(r.inset(1.0, 1.0), RADIUS, t.accent, 2.0),
            _ if active && multi => gfx.stroke_round(r.inset(0.75, 0.75), RADIUS, t.accent, 1.5),
            _ => gfx.stroke_round(r.inset(0.5, 0.5), RADIUS, t.border, 1.0),
        }
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
            Some((paths, is_move)) if !paths.is_empty() => {
                let op = if is_move { FileOp::Move { paths, dest } } else { FileOp::Copy { paths, dest } };
                shell::spawn_file_op(hwnd, op);
                if is_move {
                    shell::clear_clipboard();
                }
            }
            _ => {
                with_app(|a| a.flash(pane, "클립보드에 붙여넣을 파일이 없습니다"));
            }
        },
        Deferred::Menu { pane, paths, dir, pt } => show_menu(hwnd, MenuRequest { pane, paths, dir, pt }),
        Deferred::PrepareMenu { paths, dir } => prepare_menu(hwnd, paths, dir),
        Deferred::FavMenu { index, pt } => fav_ui::show_fav_menu(hwnd, index, pt),
        Deferred::FavSortMenu { pt } => fav_ui::show_fav_sort_menu(hwnd, pt),
        Deferred::PreviewSync => preview::sync_handler(),
        Deferred::SettingsMenu { pt } => show_settings_menu(hwnd, pt),
    }
}

fn show_settings_menu(hwnd: HWND, pt: POINT) {
    use crate::defaultfm;
    const DETAILS: usize = 1;
    const THUMBS: usize = 2;
    const PREVIEW: usize = 3;
    const HIDDEN: usize = 4;
    const DEFAULT_FM: usize = 5;
    const SERVICE: usize = 6;
    const TRAY: usize = 7;
    /// 보이는 칸들의 검색 범위 밖 폴더 (FOLDER_ITEM + 번호)
    const FOLDER_ITEM: usize = 200;
    /// 검색에 넣은 폴더 하위 메뉴 (FOLDER_LIST + 번호)
    const FOLDER_LIST: usize = 100;
    let Some((thumbs, preview, hidden, dirs, folders)) =
        with_app(|a| (a.panes[a.active].view == ViewMode::Thumbs, a.preview.is_some(), a.show_hidden, a.visible_dirs(), a.folders.clone()))
    else {
        return;
    };
    let is_default = defaultfm::is_registered(defaultfm::CLASSES);
    let service = crate::service::is_installed();
    let tray = crate::tray::is_enabled(crate::tray::RUN_KEY);
    // 활성 칸이 아니어도, 보이는 칸에 열린 공유폴더·USB 폴더는 여기서 넣거나 뺄 수 있다: (보여 줄 경로, 이미 넣었는지)
    let mut candidates: Vec<(String, bool)> = Vec::new();
    for d in &dirs {
        let c = match folders.covering(d) {
            Some(root) => (root, true),
            None if crate::folders::outside_disk_index(d) => (d.clone(), false),
            None => continue,
        };
        if !candidates.iter().any(|(p, _)| util::eq_path(p, &c.0)) {
            candidates.push(c);
        }
    }
    let added = folders.list();
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let item = |id: usize, label: &str, checked: bool| {
            let _ = AppendMenuW(menu, MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED }, id, &HSTRING::from(label));
        };
        let sep = || {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        };
        item(DETAILS, "자세히 보기\tCtrl+Shift+6", !thumbs);
        item(THUMBS, "썸네일 보기\tCtrl+Shift+2", thumbs);
        sep();
        item(PREVIEW, "미리 보기 창\tAlt+P", preview);
        item(HIDDEN, "숨김 항목 표시\tCtrl+H", hidden);
        sep();
        item(DEFAULT_FM, "기본 파일 관리자로 사용 (폴더 열기·Win+E)", is_default);
        item(TRAY, "트레이에서 대기 (로그인할 때 시작)", tray);
        item(SERVICE, if service { "디스크 검색 서비스 제거…" } else { "디스크 검색 서비스 설치…" }, false);
        if !candidates.is_empty() || !added.is_empty() {
            sep();
        }
        for (i, (path, on)) in candidates.iter().enumerate() {
            item(FOLDER_ITEM + i, &format!("이 폴더도 검색하기 ({path})"), *on);
        }
        if !added.is_empty() {
            if let Ok(sub) = CreatePopupMenu() {
                for (i, (path, state)) in added.iter().enumerate() {
                    let state = match state {
                        crate::folders::State::Scanning => "훑는 중".to_string(),
                        crate::folders::State::Ready(n) => format!("{n}개"),
                        crate::folders::State::Offline => "연결 안 됨".to_string(),
                    };
                    let _ = AppendMenuW(sub, MF_STRING, FOLDER_LIST + i, &HSTRING::from(format!("{path}\t{state}")));
                }
                let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("검색에 넣은 폴더"));
            }
        }
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_RIGHTALIGN).0, pt.x, pt.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        cmd
    };
    match cmd {
        DETAILS | THUMBS => {
            with_app(|a| {
                let p = a.active;
                a.set_view(p, if cmd == THUMBS { ViewMode::Thumbs } else { ViewMode::Details });
            });
        }
        PREVIEW => {
            with_app(|a| a.toggle_preview());
        }
        HIDDEN => {
            with_app(|a| a.toggle_hidden());
        }
        DEFAULT_FM if is_default => {
            defaultfm::unregister(defaultfm::CLASSES);
            with_app(|a| a.flash(a.active, "기본 파일 관리자 설정을 해제했습니다. 폴더는 다시 탐색기로 열립니다"));
        }
        DEFAULT_FM => {
            let text = "폴더·드라이브를 열 때와 Win+E를 Sakadu로 엽니다.\n\
                        바탕 화면·작업 표시줄은 그대로 탐색기가 맡고, 제어판·휴지통 같은 특수 폴더도 탐색기로 열립니다.\n\
                        현재 사용자에게만 적용되며, 이 메뉴에서 언제든 되돌릴 수 있습니다.\n\
                        Win+E는 탐색기가 이전 설정을 기억하고 있어서, 다시 로그인하거나 PC를 다시 시작한 뒤부터 바뀔 수 있습니다.\n\n계속할까요?";
            let ok = unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Sakadu"), MB_YESNO | MB_ICONQUESTION) } == IDYES;
            if ok {
                let r = defaultfm::copy_self().and_then(|exe| defaultfm::register(defaultfm::CLASSES, &exe));
                crate::diag::log(|| format!("default file manager: register -> {r:?}, folder opens with {:?}", defaultfm::effective_command("Directory")));
                let msg = match r {
                    Ok(()) => "이제 폴더가 Sakadu로 열립니다. Win+E는 다시 로그인한 뒤부터 바뀔 수 있습니다".to_string(),
                    Err(e) => format!("기본 파일 관리자로 등록하지 못했습니다: {e}"),
                };
                with_app(|a| a.flash(a.active, msg));
            }
        }
        _ if cmd >= FOLDER_ITEM && cmd < FOLDER_ITEM + candidates.len() => {
            let (path, on) = &candidates[cmd - FOLDER_ITEM];
            if *on {
                remove_search_folder(hwnd, &folders, path);
            } else {
                let msg = match folders.add(path) {
                    Ok(()) => "이제 Ctrl+F로 이 폴더 안도 찾습니다. 처음 훑는 동안에는 결과가 덜 나올 수 있습니다".to_string(),
                    Err(e) => format!("검색 폴더 목록을 저장하지 못했습니다: {e}"),
                };
                with_app(|a| a.flash(a.active, msg));
            }
        }
        _ if cmd >= FOLDER_LIST && cmd < FOLDER_LIST + added.len() => {
            remove_search_folder(hwnd, &folders, &added[cmd - FOLDER_LIST].0);
        }
        TRAY if tray => {
            crate::tray::disable(crate::tray::RUN_KEY);
            with_app(|a| {
                a.set_tray(false);
                a.flash(a.active, "트레이 대기를 껐습니다. 이제 창을 닫으면 Sakadu가 끝납니다");
            });
        }
        TRAY => {
            // 기본 파일 관리자와 같은 위치에 복사해 두고 그 파일을 로그인할 때 실행한다
            let r = defaultfm::copy_self().and_then(|exe| crate::tray::enable(crate::tray::RUN_KEY, &exe));
            crate::diag::log(|| format!("tray: enable -> {r:?}"));
            with_app(|a| match r {
                Ok(()) => {
                    a.set_tray(true);
                    a.flash(a.active, "이제 창을 닫아도 트레이에서 기다리고, 로그인할 때도 창 없이 시작합니다");
                }
                Err(e) => a.flash(a.active, format!("시작 프로그램에 등록하지 못했습니다: {e}")),
            });
        }
        SERVICE => {
            // 서비스 설치·제거는 관리자 권한이 필요하므로 UAC를 거쳐 따로 실행한다
            let arg = if service { "--uninstall-service" } else { "--install-service" };
            if let Ok(exe) = std::env::current_exe() {
                unsafe {
                    windows::Win32::UI::Shell::ShellExecuteW(Some(hwnd), w!("runas"), &HSTRING::from(exe.as_os_str()), &HSTRING::from(arg), PCWSTR::null(), SW_SHOWNORMAL);
                }
            }
        }
        _ => {}
    }
    with_app(|a| a.invalidate());
}

fn remove_search_folder(hwnd: HWND, folders: &crate::folders::Folders, root: &str) {
    let text = format!("이 폴더를 Ctrl+F 검색에서 뺄까요?\n{root}");
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
    rename: HWND,
    preview_host: HWND,
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
    let (bg, brush) = if target == c.rename || target == c.preview_host { (c.pane, c.pane_brush) } else { (c.field, c.field_brush) };
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
            WM_CHAR | WM_SYSCHAR => {
                if SUPPRESS_CHAR.replace(false) || wp.0 == 0x7F {
                    return LRESULT(0);
                }
                // 즐겨찾기 팝업이 열려 있으면 글자는 필터 대신 항목 찾기에 쓴다
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
