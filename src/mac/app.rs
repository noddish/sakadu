//! 맥 시제품 앱: 칸·탭·사이드바 상태, 그리기(paint), 누르기 판정(hit), 동작.
//! 화면 모양은 '모양 2(맥 관례)': 사이드바가 창 위까지 올라가고 신호등은 그 안, 아이콘 막대 대신 고르기 단추,
//! 설정은 작업 줄 오른쪽 끝. 치수·색·규칙은 Windows app.rs와 같다 (3단계에서 공통 화면 코드로 합친다).
//! 창(NSView)과 상관없이 돌아가서, 시험과 그림(--render)은 창 없이 같은 코드를 쓴다

use crate::gfx::Gfx;
use sakadu::draw::{Color, Font, R};
use sakadu::favorites::{self, Favorite, Favorites, Row};
use sakadu::listing::{self, Entry, Kind};
use sakadu::palette::{FOCUS_W, R_CTRL, R_ROW, Theme, alpha};
use sakadu::pane::{DEFAULT_COLS, LoadState, Location, Restore, Sort, Tab};
use sakadu::session::{self, Look, PaneState, Session};
use sakadu::util;

// app.rs와 같은 치수
const WORKBAR_H: f32 = 44.0;
const MARGIN: f32 = 8.0;
const GAP: f32 = 8.0;
const HEAD_H: f32 = 44.0;
const PATH_H: f32 = 36.0;
const HEADER_H: f32 = 28.0;
pub const ROW_H: f32 = 28.0;
const SCROLL_W: f32 = 10.0;
const RADIUS: f32 = 12.0;
const CHEV_W: f32 = 16.0;
const CRUMB_PAD: f32 = 8.0;
/// 경로 입력 상자의 가장 좁은 폭 (app.rs ADDR_MIN_W)
const ADDR_MIN_W: f32 = 80.0;
// app/sidebar.rs와 같은 치수 (모양 2는 아이콘 막대와 패널을 합친 폭)
const SIDE_W: f32 = 44.0 + 220.0;
const SIDE_HEADER_H: f32 = 44.0;
const SIDE_ROW_H: f32 = 32.0;
const INDENT: f32 = 20.0;
/// 맥 창의 신호등 아래 끝 (창에서 읽기 전 기본값, 그림 시험에서도 쓴다)
pub const LIGHTS_BOTTOM: f32 = 28.0;

// Windows 아이콘 글꼴 번호 (gfx가 SF Symbols로 바꾼다)
const G_BACK: u16 = 0xE72B;
const G_FWD: u16 = 0xE72A;
const G_UP: u16 = 0xE74A;
const G_ADD: u16 = 0xE710;
const G_CLOSE: u16 = 0xE8BB;
const G_CHEVRON: u16 = 0xE76C;
const G_CHEVRON_DOWN: u16 = 0xE70D;
const G_SORT_UP: u16 = 0xE70E;
const G_STAR: u16 = 0xE734;
const G_STAR_FILL: u16 = 0xE735;
const G_NEW_FOLDER: u16 = 0xE8F4;
const G_HIDDEN: u16 = 0xE7B3;
const G_LIST: u16 = 0xE8FD;
const G_SETTINGS: u16 = 0xE713;
const G_MORE: u16 = 0xE712;
const G_PREVIEW: u16 = 0xE90D;
const G_SEARCH: u16 = 0xE721;
const G_PLACES: u16 = 0xE8B7;
const G_PIN_FILL: u16 = 0xE841;

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
}

/// 사이드바 고르기: 바로가기, 즐겨찾기, 검색
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Places,
    Favorites,
    Search,
}

impl Side {
    const ALL: [Side; 3] = [Side::Places, Side::Favorites, Side::Search];

    fn title(self) -> &'static str {
        match self {
            Side::Places => "바로가기",
            Side::Favorites => "즐겨찾기",
            Side::Search => "검색",
        }
    }

    fn glyph(self) -> u16 {
        match self {
            Side::Places => G_PLACES,
            Side::Favorites => G_STAR,
            Side::Search => G_SEARCH,
        }
    }
}

/// 마우스가 가리키는 것 (칸 번호 p, 탭·줄 번호 i)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    None,
    Seg(Side),
    SideRow(usize),
    Layout(usize),
    Hidden,
    Tab(usize, usize),
    TabClose(usize, usize),
    TabAdd(usize),
    Back(usize),
    Fwd(usize),
    Up(usize),
    Crumb(usize, usize),
    /// 경로 줄의 빈 곳 (누르면 경로 입력)
    CrumbEmpty(usize),
    Star(usize),
    Row(usize, usize),
    List(usize),
    Pane(usize),
    SplitX,
    SplitY,
}

/// 누른 수정 키
#[derive(Clone, Copy, Default, Debug)]
pub struct Mods {
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// 맥 키 (글자 키는 소문자 그대로)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    Esc,
    Char(char),
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

impl Pane {
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
}

/// 폴더 읽기 요청: 창 쪽이 뒷줄에서 읽고 on_listed로 돌려준다
pub struct Request {
    pub tab: u64,
    pub generation: u64,
    pub dir: String,
}

/// 경로 입력 확인 요청: 창 쪽이 뒷줄에서 resolve_input을 돌려 on_probed로 돌려준다 (꺼진 NAS를 확인하는 동안 멈추지 않게)
pub struct Probe {
    pub pane: usize,
    pub tab: u64,
    pub text: String,
}

/// 주소 입력 해석 (app.rs resolve_input과 같은 규칙): 앞의 ~는 홈, 폴더면 그 폴더, 파일이면 그 폴더에서 그 파일을 고른다
pub fn resolve_input(text: &str, home: &str) -> Result<(String, Option<String>), String> {
    let t = text.trim();
    let missing = || format!("경로를 찾을 수 없어요: {t}");
    let expanded = match t.strip_prefix('~') {
        Some("") => home.to_string(),
        Some(rest) if rest.starts_with('/') => format!("{}{rest}", home.trim_end_matches('/')),
        _ => t.to_string(),
    };
    if !expanded.starts_with('/') {
        return Err(missing());
    }
    let path = util::normalize(&expanded);
    match std::fs::metadata(&path) {
        Ok(m) if m.is_dir() => Ok((path, None)),
        Ok(_) => util::parent(&path).map(|p| (p, Some(util::file_name(&path).to_string()))).ok_or_else(missing),
        Err(_) => Err(missing()),
    }
}

/// 사이드바 한 줄
struct SideRow {
    label: String,
    path: Option<String>,
    is_dir: bool,
    indent: bool,
    /// 즐겨찾기 그룹 머리 (그룹 번호, 접힘)
    group: Option<(usize, bool)>,
}

/// 칸 하나의 자리 (app.rs PaneGeom과 같은 계산)
struct Geom {
    head: R,
    back: R,
    fwd: R,
    up: R,
    crumb: R,
    star: R,
    new_folder: R,
    view: R,
    header: R,
    list: R,
}

fn geom(r: R) -> Geom {
    let head = R::new(r.l, r.t, r.r, r.t + HEAD_H);
    let path = R::new(r.l, head.b, r.r, head.b + PATH_H);
    let (t, b) = (path.t + 4.0, path.b - 4.0);
    let view = R::new(path.r - 8.0 - 40.0, t, path.r - 8.0, b);
    let new_folder = R::new(view.l - 4.0 - 28.0, t, view.l - 4.0, b);
    let star = R::new(new_folder.l - 4.0 - 28.0, t, new_folder.l - 4.0, b);
    let back = R::new(path.l + 8.0, t, path.l + 36.0, b);
    let fwd = R::new(back.r + 2.0, t, back.r + 30.0, b);
    let up = R::new(fwd.r + 2.0, t, fwd.r + 30.0, b);
    let crumb = R::new(up.r + 8.0, path.t + 4.0, (star.l - 4.0).max(up.r + 8.0), path.b - 4.0);
    let header = R::new(r.l, path.b, r.r, path.b + HEADER_H);
    let list = R::new(r.l, header.b, r.r, (r.b - 6.0).max(header.b));
    Geom { head, back, fwd, up, crumb, star, new_folder, view, header, list }
}

pub struct App {
    pub w: f32,
    pub h: f32,
    pub theme: Theme,
    panes: Vec<Pane>,
    layout: Layout,
    active: usize,
    split: (f32, f32),
    side: Side,
    favs: Favorites,
    show_hidden: bool,
    pub hover: Hit,
    next_id: u64,
    sort: Sort,
    /// 끌고 있는 칸 경계
    drag: Option<Hit>,
    /// 바로가기 줄 (이름, 경로)
    places: Vec<(String, String)>,
    requests: Vec<Request>,
    /// 신호등 아래 끝 (창에서 읽는다). 사이드바 고르기 단추를 그 아래에 둔다
    pub lights_bottom: f32,
    /// 창 없이 그림만 그릴 때 (그림 시험): 신호등과 경로 입력 글자도 직접 그린다 (창에서는 시스템·입력 상자가 그린다)
    pub standalone: bool,
    /// 경로를 입력하는 칸 (창 쪽 입력 상자가 그 자리에 뜬다)
    pub editing: Option<usize>,
    /// 칸 p의 칩 줄 오른쪽에 잠깐 보일 알림 (다음 입력 때 지운다)
    flash: Option<(usize, String)>,
    probes: Vec<Probe>,
    /// 사이드바 목록 스크롤 (줄이 창 아래로 넘칠 때)
    side_scroll: f32,
    /// 열어 달라는 파일 (창 쪽이 macOS '열기'로 연다)
    pub open_files: Vec<String>,
    /// 즐겨찾기를 파일에 저장할지 (시험·그림에서는 저장하지 않는다)
    pub persist: bool,
}

impl App {
    /// home: 처음 여는 폴더, places: 바로가기 줄
    pub fn new(w: f32, h: f32, theme: Theme, home: &str, favs: Favorites, places: Vec<(String, String)>) -> App {
        let mut app = App {
            w,
            h,
            theme,
            panes: Vec::new(),
            layout: Layout::TwoV,
            active: 0,
            split: (0.5, 0.5),
            side: Side::Favorites,
            favs,
            show_hidden: false,
            hover: Hit::None,
            next_id: 1,
            sort: Sort::default(),
            drag: None,
            places,
            requests: Vec::new(),
            lights_bottom: LIGHTS_BOTTOM,
            standalone: false,
            editing: None,
            flash: None,
            probes: Vec::new(),
            side_scroll: 0.0,
            open_files: Vec::new(),
            persist: false,
        };
        for _ in 0..4 {
            let tab = app.new_tab_at(Location::Dir(home.to_string()));
            app.panes.push(Pane { tabs: vec![tab], active: 0 });
        }
        for p in 0..app.layout.count() {
            app.load(p);
        }
        app
    }

    fn new_tab_at(&mut self, loc: Location) -> Tab {
        self.next_id += 1;
        Tab::new(self.next_id, loc)
    }

    /// 창 제목: 지금 칸의 지금 탭
    pub fn title(&self) -> String {
        format!("{} - Sakadu", self.panes[self.active].tab().loc.title())
    }

    #[cfg(test)]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    #[cfg(test)]
    pub fn active(&self) -> usize {
        self.active
    }

    #[cfg(test)]
    pub fn pane(&self, p: usize) -> &Pane {
        &self.panes[p]
    }

    /// 칸 p의 지금 탭 (그림 시험에서 고른 줄을 정할 때)
    pub fn tab_mut(&mut self, p: usize) -> &mut Tab {
        self.panes[p].tab_mut()
    }

    #[cfg(test)]
    pub fn favorites(&self) -> &Favorites {
        &self.favs
    }

    // ---------- 폴더 읽기 ----------

    /// 칸 p의 지금 탭을 (다시) 읽는다. 결과는 on_listed로 받는다
    fn load(&mut self, p: usize) {
        let tab = self.panes[p].tab_mut();
        tab.generation += 1;
        if tab.state != LoadState::Ready {
            tab.state = LoadState::Loading;
        }
        if let Some(dir) = tab.loc.dir() {
            self.requests.push(Request { tab: tab.id, generation: tab.generation, dir: dir.to_string() });
        }
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// 뒷줄에서 읽은 결과. 그 사이 다른 곳으로 옮겼으면(세대 번호가 다르면) 버린다
    pub fn on_listed(&mut self, tab_id: u64, generation: u64, result: Result<Vec<Entry>, String>) {
        let (sort, hidden) = (self.sort, self.show_hidden);
        let Some(p) = self.panes.iter().position(|pane| pane.tabs.iter().any(|t| t.id == tab_id)) else { return };
        let list = self.list_rect(p);
        let Some(tab) = self.panes[p].tabs.iter_mut().find(|t| t.id == tab_id) else { return };
        if tab.generation != generation {
            return;
        }
        match result {
            Ok(entries) => {
                let restore = tab.set_entries(entries, sort, hidden);
                // 스크롤은 다 읽은 그 탭에 맞춘다 (그 사이 다른 탭으로 옮겼어도 보고 있는 탭은 그대로)
                if let Some(list) = list {
                    clamp_tab(tab, list);
                    if restore.is_some_and(|r| r.reveal) {
                        reveal(tab, list);
                    }
                }
            }
            Err(msg) => {
                tab.entries.clear();
                tab.view.clear();
                tab.state = LoadState::Error(msg);
            }
        }
    }

    /// 쌓인 읽기 요청을 바로 처리한다 (시험·그림용, 창에서는 뒷줄에서 읽는다)
    pub fn settle(&mut self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        loop {
            let (reqs, probes) = (self.take_requests(), self.take_probes());
            if reqs.is_empty() && probes.is_empty() {
                break;
            }
            for r in reqs {
                let result = listing::list_dir(&r.dir);
                self.on_listed(r.tab, r.generation, result);
            }
            for p in probes {
                let result = resolve_input(&p.text, &home);
                self.on_probed(p.pane, p.tab, result);
            }
        }
    }

    // ---------- 경로 입력 ----------

    /// 칸 p에서 경로 입력을 시작한다 (폴더를 보고 있을 때만)
    pub fn begin_address(&mut self, p: usize) {
        if self.panes[p].tab().loc.dir().is_some() {
            self.activate_pane(p);
            self.editing = Some(p);
        }
    }

    pub fn cancel_address(&mut self) {
        self.editing = None;
    }

    /// 입력한 경로로 간다. 확인은 뒷줄에서 하고 결과는 on_probed로 받는다
    pub fn commit_address(&mut self, text: &str) {
        let Some(p) = self.editing.take() else { return };
        let tab = self.panes[p].tab().id;
        self.probes.push(Probe { pane: p, tab, text: text.to_string() });
    }

    /// 그림 시험용: 칸 p에서 text를 입력해 확정한 결과까지 바로 반영한다
    pub fn commit_address_for_render(&mut self, p: usize, text: &str) {
        self.begin_address(p);
        self.commit_address(text);
        self.settle();
    }

    pub fn take_probes(&mut self) -> Vec<Probe> {
        std::mem::take(&mut self.probes)
    }

    /// 경로 확인 결과. 그 사이 그 칸의 지금 탭이 바뀌었으면 버린다
    pub fn on_probed(&mut self, p: usize, tab: u64, result: Result<(String, Option<String>), String>) {
        if self.panes[p].tab().id != tab {
            return;
        }
        match result {
            Ok((dir, select)) => {
                let here = self.panes[p].tab().loc.dir().is_some_and(|d| util::eq_path(d, &dir));
                if here && select.is_none() {
                    self.refresh(p);
                } else {
                    self.navigate(p, Location::Dir(dir), select);
                }
            }
            Err(msg) => self.flash = Some((p, msg)),
        }
    }

    /// 경로 입력 상자 자리: 칸의 경로 자리, 너무 좁으면 새 폴더·보기 단추 자리까지 (app.rs addr_rect)
    pub fn address_rect(&self) -> Option<R> {
        let p = self.editing?;
        let r = self.pane_rects().into_iter().find(|(q, _)| *q == p)?.1;
        let gm = geom(r);
        Some(if gm.crumb.w() >= ADDR_MIN_W { gm.crumb } else { R::new(gm.up.r + 8.0, gm.crumb.t, gm.view.r, gm.crumb.b) })
    }

    /// 경로 입력 상자에 처음 넣을 글자 (지금 폴더)
    pub fn address_text(&self) -> String {
        self.editing.and_then(|p| self.panes[p].tab().loc.dir().map(str::to_string)).unwrap_or_default()
    }

    // ---------- 세션 ----------

    /// 끝낼 때 저장할 화면 상태 (맥이 쓰지 않는 값은 기본값)
    pub fn to_session(&self) -> Session {
        let layout = Layout::ALL.iter().position(|l| *l == self.layout).unwrap_or(1) as u8;
        let side = match self.side {
            Side::Places => "places",
            Side::Favorites => "favorites",
            Side::Search => "search",
        };
        Session {
            window: None,
            maximized: false,
            layout,
            split: self.split,
            active: self.active,
            show_hidden: self.show_hidden,
            preview: false,
            new_tab: session::NEW_TAB_SAME.to_string(),
            recent: Vec::new(),
            side: side.to_string(),
            side_w: session::SIDE_W_DEFAULT,
            pin: None,
            pin_id: None,
            ws: None,
            query: String::new(),
            scope_here: false,
            folded: Vec::new(),
            look: Look::default(),
            panes: self
                .panes
                .iter()
                .map(|pane| PaneState {
                    tabs: pane.tabs.iter().map(|t| t.loc.to_session()).collect(),
                    active: pane.active,
                    cols: DEFAULT_COLS,
                    sort_key: 0,
                    sort_desc: false,
                    view: 0,
                })
                .collect(),
        }
    }

    /// 저장한 화면 상태로 연다. 없어진 폴더(와 맥에서 쓰지 않는 '내 PC'·시작 화면)는 home으로 바꾼다
    pub fn restore(&mut self, s: &Session, home: &str) {
        self.requests.clear();
        self.layout = Layout::ALL[(s.layout as usize).min(4)];
        self.split = (s.split.0.clamp(0.15, 0.85), s.split.1.clamp(0.15, 0.85));
        self.show_hidden = s.show_hidden;
        self.side = match s.side.as_str() {
            "places" => Side::Places,
            "search" => Side::Search,
            _ => Side::Favorites,
        };
        for (p, ps) in s.panes.iter().take(self.panes.len()).enumerate() {
            let mut tabs = Vec::new();
            for t in &ps.tabs {
                let dir = match Location::from_session(t) {
                    Location::Dir(d) if std::path::Path::new(&d).is_dir() => d,
                    _ => home.to_string(),
                };
                tabs.push(self.new_tab_at(Location::Dir(dir)));
            }
            if !tabs.is_empty() {
                let active = ps.active.min(tabs.len() - 1);
                self.panes[p] = Pane { tabs, active };
            }
        }
        self.active = s.active.min(self.layout.count() - 1);
        for p in 0..self.layout.count() {
            self.load_if_new(p);
        }
    }

    // ---------- 이동 ----------

    /// 칸 p를 loc으로 옮긴다 (기록을 남긴다). select가 있으면 읽은 뒤 그 이름을 고른다
    pub fn navigate(&mut self, p: usize, loc: Location, select: Option<String>) {
        let tab = self.panes[p].tab_mut();
        tab.nav_push(loc);
        tab.reset_view_state();
        tab.state = LoadState::Loading;
        tab.restore = select.as_deref().map(Restore::select);
        self.load(p);
    }

    pub fn go_up(&mut self, p: usize) {
        let Some(dir) = self.panes[p].tab().loc.dir().map(str::to_string) else { return };
        if let Some(parent) = util::parent(&dir) {
            self.navigate(p, Location::Dir(parent), Some(util::file_name(&dir).to_string()));
        }
    }

    pub fn go_back(&mut self, p: usize) {
        let from = self.panes[p].tab().loc.clone();
        if self.panes[p].tab_mut().nav_back() {
            self.after_history(p, from);
        }
    }

    pub fn go_forward(&mut self, p: usize) {
        let from = self.panes[p].tab().loc.clone();
        if self.panes[p].tab_mut().nav_forward() {
            self.after_history(p, from);
        }
    }

    /// 뒤로·앞으로 뒤: 상위 폴더로 돌아왔으면 원래 있던 하위 폴더를 고른다
    fn after_history(&mut self, p: usize, from: Location) {
        let tab = self.panes[p].tab_mut();
        let select = child_toward(&tab.loc, &from);
        tab.reset_view_state();
        tab.state = LoadState::Loading;
        tab.restore = select.as_deref().map(Restore::select);
        self.load(p);
    }

    /// 줄 vi 열기: 폴더면 그리로, 파일이면 macOS '열기'
    pub fn open_row(&mut self, p: usize, vi: usize) {
        let tab = self.panes[p].tab();
        let Some(e) = tab.entry_at(vi) else { return };
        let Some(path) = tab.path_of(e) else { return };
        if e.is_dir() {
            self.navigate(p, Location::Dir(path), None);
        } else {
            self.open_files.push(path);
        }
    }

    /// 열기 키(⌘↓·⌘O): 커서가 폴더면 그리로, 아니면 고른 파일을 연다
    pub fn open_selected(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        let (cursor, folder) = (tab.cursor, tab.cursor_entry().is_some_and(Entry::is_dir));
        if folder {
            return self.open_row(p, cursor);
        }
        let files: Vec<String> = tab.selected_paths();
        self.open_files.extend(files);
    }

    pub fn refresh(&mut self, p: usize) {
        let tab = self.panes[p].tab_mut();
        if tab.restore.is_none() {
            tab.restore = Some(tab.snapshot());
        }
        self.load(p);
    }

    // ---------- 탭·칸 ----------

    /// 지금 폴더를 새 탭으로 (지금 탭 바로 뒤)
    pub fn new_tab(&mut self, p: usize) {
        let loc = self.panes[p].tab().loc.clone();
        let tab = self.new_tab_at(loc);
        let pane = &mut self.panes[p];
        pane.active += 1;
        pane.tabs.insert(pane.active, tab);
        self.load(p);
    }

    /// 마지막 탭은 닫지 않는다
    pub fn close_tab(&mut self, p: usize, i: usize) {
        let pane = &mut self.panes[p];
        if pane.tabs.len() <= 1 || i >= pane.tabs.len() {
            return;
        }
        pane.tabs.remove(i);
        if pane.active > i || pane.active >= pane.tabs.len() {
            pane.active = pane.active.saturating_sub(1);
        }
        self.load_if_new(p);
    }

    pub fn activate_tab(&mut self, p: usize, i: usize) {
        if i < self.panes[p].tabs.len() {
            self.panes[p].active = i;
            self.load_if_new(p);
        }
    }

    pub fn cycle_tab(&mut self, p: usize, back: bool) {
        let n = self.panes[p].tabs.len();
        let i = self.panes[p].active;
        self.activate_tab(p, if back { (i + n - 1) % n } else { (i + 1) % n });
    }

    /// 한 번도 읽지 않은 탭이면 읽는다
    fn load_if_new(&mut self, p: usize) {
        if self.panes[p].tab().generation == 0 {
            self.load(p);
        }
    }

    pub fn activate_pane(&mut self, p: usize) {
        if p < self.layout.count() {
            self.active = p;
        }
    }

    pub fn cycle_pane(&mut self) {
        self.active = (self.active + 1) % self.layout.count();
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
        self.active = self.active.min(layout.count() - 1);
        for p in 0..layout.count() {
            self.load_if_new(p);
            self.clamp_scroll(p);
        }
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        let (sort, hidden) = (self.sort, self.show_hidden);
        for p in 0..self.panes.len() {
            for tab in &mut self.panes[p].tabs {
                tab.rebuild_view(sort, hidden);
            }
            self.clamp_scroll(p);
        }
    }

    pub fn set_side(&mut self, side: Side) {
        if self.side != side {
            self.side = side;
            self.side_scroll = 0.0;
        }
    }

    // ---------- 고르기 ----------

    pub fn select_all(&mut self) {
        self.panes[self.active].tab_mut().select_all();
    }

    pub fn clear_selection(&mut self) {
        self.panes[self.active].tab_mut().clear_selection();
    }

    /// 줄 이동 (↑↓·Home·End·Page). shift는 범위 고르기
    pub fn move_cursor(&mut self, key: Key, shift: bool) {
        let p = self.active;
        let page = (self.list_rect(p).map_or(ROW_H, |l| l.h()) / ROW_H).floor().max(1.0) as usize;
        let tab = self.panes[p].tab_mut();
        let n = tab.view.len();
        if n == 0 {
            return;
        }
        let c = tab.cursor;
        let to = match key {
            Key::Up => c.saturating_sub(1),
            Key::Down => (c + 1).min(n - 1),
            Key::Home => 0,
            Key::End => n - 1,
            Key::PageUp => c.saturating_sub(page),
            Key::PageDown => (c + page).min(n - 1),
            _ => return,
        };
        tab.move_cursor(to, shift, false);
        self.ensure_visible(p);
    }

    // ---------- 즐겨찾기 ----------

    /// 칸 p의 폴더를 즐겨찾기에 넣거나 뺀다 (경로 줄의 별)
    pub fn toggle_star(&mut self, p: usize) {
        let Some(dir) = self.panes[p].tab().loc.dir().map(str::to_string) else { return };
        match self.favs.find(&dir) {
            Some((g, i)) => {
                self.favs.remove_item(g, i);
            }
            None => {
                self.favs.push(None, Favorite { name: favorites::default_name(&dir), path: dir, is_dir: true });
            }
        }
        self.save_favorites();
    }

    /// 사이드바에 넣기(⌃⌘T): 커서 항목, 없으면 지금 폴더
    pub fn add_favorite(&mut self) {
        let p = self.active;
        let tab = self.panes[p].tab();
        let target = tab.cursor_entry().and_then(|e| tab.path_of(e).map(|path| (path, e.is_dir()))).or_else(|| tab.loc.dir().map(|d| (d.to_string(), true)));
        let Some((path, is_dir)) = target else { return };
        if self.favs.find(&path).is_none() {
            self.favs.push(None, Favorite { name: favorites::default_name(&path), path, is_dir });
            self.save_favorites();
        }
        self.side = Side::Favorites;
    }

    fn save_favorites(&mut self) {
        self.clamp_side_scroll();
        if self.persist
            && let Err(e) = favorites::save(&self.favs)
        {
            sakadu::diag::log(|| format!("favorites: {e}"));
        }
    }

    fn side_rows(&self) -> Vec<SideRow> {
        match self.side {
            Side::Places => self.places.iter().map(|(l, p)| SideRow { label: l.clone(), path: Some(p.clone()), is_dir: true, indent: false, group: None }).collect(),
            Side::Favorites => self
                .favs
                .rows()
                .into_iter()
                .filter_map(|row| match row {
                    Row::Group(g) => self.favs.groups.get(g).map(|grp| SideRow { label: grp.name.clone(), path: None, is_dir: false, indent: false, group: Some((g, grp.collapsed)) }),
                    Row::Item { group, index } => self.favs.item(group, index).map(|f| SideRow { label: f.name.clone(), path: Some(f.path.clone()), is_dir: f.is_dir, indent: group.is_some(), group: None }),
                })
                .collect(),
            Side::Search => Vec::new(),
        }
    }

    fn open_side_row(&mut self, i: usize) {
        let rows = self.side_rows();
        let Some(row) = rows.get(i) else { return };
        if let Some((g, collapsed)) = row.group {
            self.favs.set_collapsed(g, !collapsed);
            self.save_favorites();
            return;
        }
        let Some(path) = row.path.clone() else { return };
        if row.is_dir {
            self.navigate(self.active, Location::Dir(path), None);
        } else {
            self.open_files.push(path);
        }
    }

    // ---------- 자리 ----------

    fn area(&self) -> R {
        let l = SIDE_W + MARGIN;
        R::new(l, WORKBAR_H, (self.w - MARGIN).max(l + 100.0), (self.h - MARGIN).max(WORKBAR_H + 100.0))
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
            Layout::Three => vec![(0, R::new(a.l, a.t, xm, a.b)), (1, R::new(xm + GAP, a.t, a.r, ym)), (2, R::new(xm + GAP, ym + GAP, a.r, a.b))],
            Layout::Four => vec![
                (0, R::new(a.l, a.t, xm, ym)),
                (1, R::new(xm + GAP, a.t, a.r, ym)),
                (2, R::new(a.l, ym + GAP, xm, a.b)),
                (3, R::new(xm + GAP, ym + GAP, a.r, a.b)),
            ],
        }
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
        self.pane_rects().into_iter().find(|(i, _)| *i == p).map(|(_, r)| geom(r).list)
    }

    fn clamp_scroll(&mut self, p: usize) {
        let Some(list) = self.list_rect(p) else { return };
        for tab in &mut self.panes[p].tabs {
            clamp_tab(tab, list);
        }
    }

    fn ensure_visible(&mut self, p: usize) {
        let Some(list) = self.list_rect(p) else { return };
        reveal(self.panes[p].tab_mut(), list);
    }

    /// 사이드바 고르기 단추 판
    fn seg_track(&self) -> R {
        let t = WORKBAR_H.max(self.lights_bottom + 8.0) + 4.0;
        R::new(12.0, t, SIDE_W - 12.0, t + 32.0)
    }

    fn seg_rect(&self, i: usize) -> R {
        let track = self.seg_track();
        let w = (track.w() - 6.0) / 3.0;
        R::new(track.l + 3.0 + i as f32 * w, track.t + 3.0, track.l + 3.0 + (i + 1) as f32 * w, track.b - 3.0)
    }

    fn side_panel(&self) -> R {
        R::new(0.0, self.seg_track().b + 8.0, SIDE_W, self.h - MARGIN)
    }

    /// 사이드바 목록이 보이는 곳 (머리 줄 아래)
    fn side_list(&self) -> R {
        let pr = self.side_panel();
        R::new(pr.l, pr.t + SIDE_HEADER_H + 2.0, pr.r, pr.b.max(pr.t + SIDE_HEADER_H + 2.0))
    }

    fn side_row_rect(&self, i: usize) -> R {
        let list = self.side_list();
        let y = list.t + i as f32 * SIDE_ROW_H - self.side_scroll;
        R::new(list.l + 8.0, y, list.r - 8.0, y + SIDE_ROW_H)
    }

    /// 사이드바 스크롤을 줄 수 안으로 (줄이 바뀌거나 창 크기가 바뀐 뒤)
    fn clamp_side_scroll(&mut self) {
        let max = (self.side_rows().len() as f32 * SIDE_ROW_H - self.side_list().h()).max(0.0);
        self.side_scroll = self.side_scroll.clamp(0.0, max);
    }

    fn workspace_button(&self, g: &Gfx) -> R {
        let l = SIDE_W + MARGIN;
        R::new(l, 6.0, l + (g.measure("작업 공간", Font::Normal) + 40.0).clamp(96.0, 220.0), 38.0)
    }

    fn layout_buttons(&self, g: &Gfx) -> [R; 5] {
        let l0 = self.workspace_button(g).r + 8.0 + 3.0;
        std::array::from_fn(|i| {
            let l = l0 + i as f32 * 32.0;
            R::new(l, 9.0, l + 32.0, 35.0)
        })
    }

    /// 작업 줄 오른쪽 단추: 설정, 미리 보기, 숨김, … (오른쪽 끝부터)
    fn right_button(&self, i: usize) -> R {
        let r = self.w - 8.0 - i as f32 * 36.0;
        R::new(r - 32.0, 6.0, r, 38.0)
    }

    /// 탭 칩 자리와 새 탭(+) 자리
    /// app.rs chips·chip_strip·add_rect와 같다: 넘치면 지금 탭은 제 폭을 지키고 나머지를 줄이며, 그래도 넘치는 탭은
    /// 폭 0으로 가린다 (⌃Tab으로 닿는다)
    fn chips(&self, g: &Gfx, p: usize, head: R) -> (Vec<R>, R) {
        let pane = &self.panes[p];
        let n = pane.tabs.len();
        let right = g.measure(&count_text(pane.tab()), Font::Small);
        let l = head.l + 8.0;
        let strip = R::new(l, head.t + 8.0, (head.r - 8.0 - right - 8.0).max(l + 100.0), head.t + 36.0);
        let natural: Vec<f32> = pane.tabs.iter().map(|t| (g.measure(&t.loc.title(), Font::Normal) + 22.0 + if n > 1 { 20.0 } else { 0.0 }).clamp(52.0, 180.0)).collect();
        let trs = tab_rects(strip, &natural, pane.active);
        let add_l = trs.iter().map(|r| r.r + 4.0).fold(strip.l, f32::max);
        (trs, R::new(add_l, strip.t, add_l + 28.0, strip.b))
    }

    /// 경로 조각: (자리, 그 조각의 전체 경로, 이름). 앞쪽이 넘치면 "…" 조각(경로 없음)으로 줄인다
    fn crumbs(&self, g: &Gfx, area: R, dir: &str) -> Vec<(R, Option<String>, String)> {
        let segs = util::segments(dir);
        let width = |s: &str| g.measure(s, Font::Normal) + CRUMB_PAD * 1.5;
        let more_w = 28.0;
        let mut used = 0.0;
        let mut take = 0;
        for (label, _) in segs.iter().rev() {
            let w = width(label) + if take == 0 { 0.0 } else { CHEV_W };
            if used + w + more_w + CHEV_W > area.w() && take > 0 {
                break;
            }
            used += w;
            take += 1;
        }
        let mut out = Vec::new();
        let mut x = area.l;
        if take < segs.len() {
            out.push((R::new(x, area.t, x + more_w, area.b), None, "…".to_string()));
            x += more_w + CHEV_W;
        }
        for (label, path) in &segs[segs.len() - take..] {
            let w = width(label);
            out.push((R::new(x, area.t, x + w, area.b), Some(path.clone()), label.clone()));
            x += w + CHEV_W;
        }
        out
    }

    // ---------- 누르기 판정 ----------

    pub fn hit(&self, g: &Gfx, x: f32, y: f32) -> Hit {
        if x < SIDE_W {
            for (i, side) in Side::ALL.iter().enumerate() {
                if self.seg_rect(i).contains(x, y) {
                    return Hit::Seg(*side);
                }
            }
            let n = self.side_rows().len();
            if !self.side_list().contains(x, y) {
                return Hit::None;
            }
            return (0..n).find(|&i| self.side_row_rect(i).contains(x, y)).map_or(Hit::None, Hit::SideRow);
        }
        if y < WORKBAR_H {
            if let Some(i) = self.layout_buttons(g).iter().position(|r| r.contains(x, y)) {
                return Hit::Layout(i);
            }
            if self.right_button(2).contains(x, y) {
                return Hit::Hidden;
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
            let gm = geom(r);
            let tab = self.panes[p].tab();
            if gm.head.contains(x, y) {
                let (trs, add) = self.chips(g, p, gm.head);
                for (i, tr) in trs.iter().enumerate() {
                    if tr.contains(x, y) {
                        let closable = self.panes[p].tabs.len() > 1 && (i == self.panes[p].active || self.hover == Hit::Tab(p, i) || self.hover == Hit::TabClose(p, i));
                        return if closable && close_rect(*tr).contains(x, y) { Hit::TabClose(p, i) } else { Hit::Tab(p, i) };
                    }
                }
                return if add.contains(x, y) { Hit::TabAdd(p) } else { Hit::Pane(p) };
            }
            for (rr, h) in [(gm.back, Hit::Back(p)), (gm.fwd, Hit::Fwd(p)), (gm.up, Hit::Up(p)), (gm.star, Hit::Star(p))] {
                if rr.contains(x, y) {
                    return h;
                }
            }
            if gm.crumb.contains(x, y) {
                if let Some(dir) = tab.loc.dir()
                    && let Some(i) = self.crumbs(g, gm.crumb, dir).iter().position(|(cr, path, _)| path.is_some() && cr.contains(x, y))
                {
                    return Hit::Crumb(p, i);
                }
                return Hit::CrumbEmpty(p);
            }
            if gm.list.contains(x, y) {
                let vi = ((y - gm.list.t + tab.scroll) / ROW_H).floor() as usize;
                return if vi < tab.view.len() && x < gm.list.r - SCROLL_W - 6.0 { Hit::Row(p, vi) } else { Hit::List(p) };
            }
            return Hit::Pane(p);
        }
        Hit::None
    }

    // ---------- 마우스·키 ----------

    /// 누름. clicks는 연속으로 누른 횟수 (2면 두 번 누르기)
    pub fn mouse_down(&mut self, g: &Gfx, x: f32, y: f32, mods: Mods, clicks: usize) {
        let hit = self.hit(g, x, y);
        // 입력 상자 밖을 누르면 입력을 그만둔다. 알림은 다음 입력 때 지운다
        self.editing = None;
        self.flash = None;
        match hit {
            Hit::Tab(p, _) | Hit::TabClose(p, _) | Hit::TabAdd(p) | Hit::Back(p) | Hit::Fwd(p) | Hit::Up(p) | Hit::Crumb(p, _) | Hit::CrumbEmpty(p) | Hit::Star(p) | Hit::Row(p, _) | Hit::List(p) | Hit::Pane(p) => {
                self.activate_pane(p)
            }
            _ => {}
        }
        match hit {
            Hit::CrumbEmpty(p) => self.begin_address(p),
            Hit::Seg(side) => self.set_side(side),
            Hit::SideRow(i) => self.open_side_row(i),
            Hit::Layout(i) => self.set_layout(Layout::ALL[i]),
            Hit::Hidden => self.toggle_hidden(),
            Hit::Tab(p, i) => self.activate_tab(p, i),
            Hit::TabClose(p, i) => self.close_tab(p, i),
            Hit::TabAdd(p) => self.new_tab(p),
            Hit::Back(p) => self.go_back(p),
            Hit::Fwd(p) => self.go_forward(p),
            Hit::Up(p) => self.go_up(p),
            Hit::Star(p) => self.toggle_star(p),
            Hit::Crumb(p, i) => {
                let Some(r) = self.pane_rects().into_iter().find(|(q, _)| *q == p).map(|(_, r)| geom(r).crumb) else { return };
                let Some(dir) = self.panes[p].tab().loc.dir().map(str::to_string) else { return };
                if let Some((_, Some(path), _)) = self.crumbs(g, r, &dir).get(i).cloned() {
                    if util::eq_path(&path, &dir) {
                        self.refresh(p);
                    } else {
                        let select = child_toward(&Location::Dir(path.clone()), &Location::Dir(dir));
                        self.navigate(p, Location::Dir(path), select);
                    }
                }
            }
            Hit::Row(p, vi) => {
                if clicks >= 2 {
                    return self.open_row(p, vi);
                }
                let tab = self.panes[p].tab_mut();
                match (mods.cmd, mods.shift) {
                    (true, true) => tab.select_range(vi, true),
                    (true, false) => tab.toggle(vi),
                    (false, true) => tab.select_range(vi, false),
                    (false, false) => tab.select_only(vi),
                }
            }
            Hit::List(p) => {
                if !mods.cmd {
                    self.panes[p].tab_mut().clear_selection();
                }
            }
            Hit::SplitX | Hit::SplitY => self.drag = Some(hit),
            Hit::None | Hit::Pane(_) => {}
        }
    }

    pub fn mouse_dragged(&mut self, x: f32, y: f32) {
        let a = self.area();
        match self.drag {
            Some(Hit::SplitX) => self.split.0 = ((x - GAP / 2.0 - a.l) / (a.w() - GAP)).clamp(0.15, 0.85),
            Some(Hit::SplitY) => self.split.1 = ((y - GAP / 2.0 - a.t) / (a.h() - GAP)).clamp(0.15, 0.85),
            _ => return,
        }
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
    }

    pub fn mouse_up(&mut self) {
        self.drag = None;
    }

    /// 마우스 올림이 바뀌었으면 true (다시 그린다)
    pub fn mouse_moved(&mut self, g: &Gfx, x: f32, y: f32) -> bool {
        let h = self.hit(g, x, y);
        let changed = h != self.hover;
        self.hover = h;
        changed
    }

    /// 스크롤 (dy > 0이면 아래 항목이 보이게). 마우스 아래 칸을 움직인다
    pub fn scroll(&mut self, g: &Gfx, x: f32, y: f32, dy: f32) {
        if x < SIDE_W {
            self.side_scroll += dy;
            return self.clamp_side_scroll();
        }
        let p = match self.hit(g, x, y) {
            Hit::Row(p, _) | Hit::List(p) | Hit::Pane(p) => p,
            _ => return,
        };
        self.panes[p].tab_mut().scroll += dy;
        self.clamp_scroll(p);
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.w = w;
        self.h = h;
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.clamp_side_scroll();
    }

    /// Finder 관례 키. 처리했으면 true
    pub fn key(&mut self, key: Key, m: Mods) -> bool {
        let p = self.active;
        self.flash = None;
        match (key, m.cmd, m.ctrl, m.alt) {
            // 경로 입력: ⌘L(Windows Ctrl+L), ⌘⇧G(Finder '폴더로 이동')
            (Key::Char('l'), true, false, false) => self.begin_address(p),
            (Key::Char('g'), true, false, false) if m.shift => self.begin_address(p),
            (Key::Up | Key::Down, false, false, false) | (Key::Home | Key::End | Key::PageUp | Key::PageDown, false, false, false) => self.move_cursor(key, m.shift),
            (Key::Down, true, false, false) | (Key::Char('o'), true, false, false) => self.open_selected(p),
            (Key::Up, true, false, false) => self.go_up(p),
            (Key::Char('['), true, false, false) => self.go_back(p),
            (Key::Char(']'), true, false, false) => self.go_forward(p),
            (Key::Char('t'), true, true, false) => self.add_favorite(),
            (Key::Char('t'), true, false, false) => self.new_tab(p),
            (Key::Char('w'), true, false, false) => {
                let i = self.panes[p].active;
                self.close_tab(p, i)
            }
            (Key::Tab, false, true, false) => self.cycle_tab(p, m.shift),
            (Key::Tab, false, false, false) => self.cycle_pane(),
            (Key::Char('a'), true, false, false) => self.select_all(),
            (Key::Esc, false, false, false) => self.clear_selection(),
            (Key::Char('.'), true, false, false) if m.shift => self.toggle_hidden(),
            (Key::Char('r'), true, false, false) => self.refresh(p),
            (Key::Char(c @ '1'..='5'), true, false, true) => self.set_layout(Layout::ALL[c as usize - '1' as usize]),
            _ => return false,
        }
        true
    }

    // ---------- 그리기 ----------

    pub fn paint(&self, g: &Gfx) {
        let t = &self.theme;
        g.clear(t.window);
        self.paint_sidebar(g, t);
        self.paint_workbar(g, t);
        for (p, r) in self.pane_rects() {
            self.paint_pane(g, t, p, r);
        }
        if matches!(self.hover, Hit::SplitX | Hit::SplitY) || self.drag.is_some() {
            let (sx, sy) = self.splitters();
            let s = if matches!(self.drag.unwrap_or(self.hover), Hit::SplitX) { sx } else { sy };
            if let Some(s) = s {
                let line = if s.w() < s.h() { R::new(s.l + GAP / 2.0 - 1.0, s.t + 12.0, s.l + GAP / 2.0 + 1.0, s.b - 12.0) } else { R::new(s.l + 12.0, s.t + GAP / 2.0 - 1.0, s.r - 12.0, s.t + GAP / 2.0 + 1.0) };
                g.fill_round(line, 1.0, alpha(t.accent, 0.6));
            }
        }
    }

    fn paint_sidebar(&self, g: &Gfx, t: &Theme) {
        // 창 바탕 위에 아주 옅은 면, 오른쪽에 얇은 선
        g.fill(R::new(0.0, 0.0, SIDE_W, self.h), alpha(t.text, if t.dark { 0.03 } else { 0.025 }));
        g.fill(R::new(SIDE_W - 1.0, 0.0, SIDE_W, self.h), alpha(t.text, 0.08));
        if self.standalone {
            traffic_lights(g, WORKBAR_H / 2.0);
        }
        // 바로가기·즐겨찾기·검색 고르기
        let track = self.seg_track();
        g.fill_round(track, track.h() / 2.0, seg_track(t));
        for (i, side) in Side::ALL.iter().enumerate() {
            let r = self.seg_rect(i);
            let on = *side == self.side;
            if on {
                seg_on(g, r, r.h() / 2.0, t);
            } else if self.hover == Hit::Seg(*side) {
                g.fill_round(r, r.h() / 2.0, hover_fill(t));
            }
            g.glyph(side.glyph(), r, false, if on { t.text } else { t.dim });
        }
        let pr = self.side_panel();
        g.text(self.side.title(), R::new(pr.l + 18.0, pr.t, pr.r - 44.0, pr.t + SIDE_HEADER_H), Font::Strong, t.text);
        if self.side == Side::Favorites {
            g.glyph(G_PIN_FILL, R::new(pr.r - 36.0, pr.t + 8.0, pr.r - 8.0, pr.t + 36.0), false, t.dim);
        }
        let rows = self.side_rows();
        let note = |l1: &str, l2: &str| {
            let r = self.side_row_rect(0);
            g.text(l1, R::new(r.l + 10.0, r.t + 4.0, r.r - 6.0, r.t + 22.0), Font::Small, t.faint);
            g.text(l2, R::new(r.l + 10.0, r.t + 22.0, r.r - 6.0, r.t + 40.0), Font::Small, t.faint);
        };
        match self.side {
            Side::Favorites if rows.is_empty() => note("즐겨찾기가 비어 있어요", "경로 오른쪽 별(☆)이나 ⌃⌘T로 넣어 보세요"),
            Side::Search => note("검색은 다음 단계에서 넣어요", "지금은 폴더를 열어 찾아 주세요"),
            _ => {}
        }
        let current = self.panes[self.active].tab().loc.dir().map(str::to_string);
        g.clip(self.side_list());
        for (i, row) in rows.iter().enumerate() {
            let r = self.side_row_rect(i);
            if let Some((_, collapsed)) = row.group {
                if self.hover == Hit::SideRow(i) {
                    g.fill_round(r, R_ROW, hover_fill(t));
                }
                g.glyph(if collapsed { G_CHEVRON } else { G_CHEVRON_DOWN }, R::new(r.l + 8.0, r.t, r.l + 24.0, r.b), true, t.faint);
                g.text(&row.label, R::new(r.l + 36.0, r.t, r.r - 8.0, r.b), Font::Normal, t.text);
                continue;
            }
            let is_current = row.is_dir && row.path.as_deref().zip(current.as_deref()).is_some_and(|(a, b)| util::eq_path(a, b));
            if is_current {
                g.fill_round(r, R_ROW, well(t));
            }
            if self.hover == Hit::SideRow(i) {
                g.fill_round(r, R_ROW, hover_fill(t));
            }
            let lx = r.l + 36.0 + if row.indent { INDENT } else { 0.0 };
            if let Some(path) = &row.path {
                g.icon_for_path(path, R::new(lx - 26.0, r.t + 8.0, lx - 10.0, r.t + 24.0));
            }
            g.text(&row.label, R::new(lx, r.t, r.r - 8.0, r.b), Font::Normal, t.text);
        }
        g.unclip();
    }

    fn paint_workbar(&self, g: &Gfx, t: &Theme) {
        let ws = self.workspace_button(g);
        g.text("작업 공간", R::new(ws.l + 10.0, ws.t, ws.r - 24.0, ws.b), Font::Normal, t.dim);
        g.glyph(G_CHEVRON_DOWN, R::new(ws.r - 22.0, ws.t + 1.0, ws.r - 8.0, ws.b + 1.0), true, t.faint);
        let bs = self.layout_buttons(g);
        let pill = R::new(bs[0].l - 3.0, 6.0, bs[4].r + 3.0, 38.0);
        g.fill_round(pill, pill.h() / 2.0, seg_track(t));
        for (i, r) in bs.iter().enumerate() {
            let current = Layout::ALL[i] == self.layout;
            if current {
                seg_on(g, *r, r.h() / 2.0, t);
            } else if self.hover == Hit::Layout(i) {
                g.fill_round(*r, r.h() / 2.0, hover_fill(t));
            }
            draw_layout_icon(g, *r, i, if current || self.hover == Hit::Layout(i) { t.text } else { t.dim });
        }
        // 오른쪽 끝부터 설정, 미리 보기, 숨김 항목, … (설정·미리 보기·…는 뒤 단계라 흐리게)
        let off = alpha(t.dim, 0.45);
        for (i, glyph) in [G_SETTINGS, G_PREVIEW, G_HIDDEN, G_MORE].into_iter().enumerate() {
            let r = self.right_button(i);
            let hidden_btn = glyph == G_HIDDEN;
            if hidden_btn && self.show_hidden {
                seg_on(g, r, R_CTRL, t);
            } else if hidden_btn && self.hover == Hit::Hidden {
                g.fill_round(r, R_CTRL, hover_fill(t));
            }
            g.glyph(glyph, r, false, if !hidden_btn { off } else if self.show_hidden || self.hover == Hit::Hidden { t.text } else { t.dim });
        }
    }

    fn paint_pane(&self, g: &Gfx, t: &Theme, p: usize, r: R) {
        let pane = &self.panes[p];
        let tab = pane.tab();
        let active = p == self.active;
        let multi = self.layout.count() > 1;
        let hover = self.hover;
        if active && multi {
            soft_shadow(g, r, RADIUS, 12.0, 0.10, t.dark);
        } else if !t.dark {
            soft_shadow(g, r, RADIUS, 6.0, 0.06, false);
        }
        g.fill_round(r, RADIUS, t.pane);
        if active && multi {
            g.fill_round(R::new(r.l + RADIUS, r.t + 1.0, r.r - RADIUS, r.t + 3.0), 1.0, t.accent);
        }
        let gm = geom(r);

        // 칩 줄: 탭 칩, 새 탭(+), 오른쪽에 항목 수·고른 개수
        let flash = self.flash.as_ref().filter(|(fp, _)| *fp == p).map(|(_, m)| m.as_str());
        let count = flash.map_or_else(|| count_text(tab), str::to_string);
        let cw = g.measure(&count, Font::Small).min(gm.head.w() * 0.6);
        g.text(&count, R::new(gm.head.r - 8.0 - cw, gm.head.t + 8.0, gm.head.r - 8.0, gm.head.t + 36.0), Font::Small, if flash.is_some() { t.text } else { t.faint });
        let (trs, add) = self.chips(g, p, gm.head);
        for (i, tr) in trs.iter().enumerate() {
            if tr.w() <= 0.0 {
                continue;
            }
            let current = i == pane.active;
            let hovered = matches!(hover, Hit::Tab(hp, hi) | Hit::TabClose(hp, hi) if hp == p && hi == i);
            if current {
                g.fill_round(*tr, tr.h() / 2.0, well(t));
            } else if hovered {
                g.fill_round(*tr, tr.h() / 2.0, t.hover);
            }
            let closable = pane.tabs.len() > 1 && (current || hovered);
            g.text(&pane.tabs[i].loc.title(), R::new(tr.l + 12.0, tr.t, tr.r - if closable { 26.0 } else { 10.0 }, tr.b), Font::Normal, t.text);
            if closable {
                let cr = close_rect(*tr);
                let hot = hover == Hit::TabClose(p, i);
                if hot {
                    g.fill_round(cr, cr.h() / 2.0, alpha(t.text, 0.12));
                }
                g.glyph(G_CLOSE, cr, true, if hot { t.text } else { t.dim });
            }
        }
        if hover == Hit::TabAdd(p) {
            g.fill_round(add, add.h() / 2.0, t.hover);
        }
        g.glyph_font(G_ADD, add, Font::GlyphMid, if hover == Hit::TabAdd(p) { t.text } else { t.dim });

        // 경로 줄
        let off = alpha(t.dim, 0.45);
        let nav = |rr: R, glyph: u16, font: Font, enabled: bool, h: Hit| {
            let hot = enabled && hover == h;
            if hot {
                g.fill_round(rr, R_CTRL, t.hover);
            }
            g.glyph_font(glyph, rr, font, if !enabled { off } else if hot { t.text } else { t.dim });
        };
        let dir = tab.loc.dir().unwrap_or_default();
        nav(gm.back, G_BACK, Font::GlyphMid, !tab.back.is_empty(), Hit::Back(p));
        nav(gm.fwd, G_FWD, Font::GlyphMid, !tab.fwd.is_empty(), Hit::Fwd(p));
        nav(gm.up, G_UP, Font::GlyphMid, util::parent(dir).is_some(), Hit::Up(p));
        // 경로 입력 중: 경로 자리에 입력 바탕과 강조 색 테두리 (글자는 창의 입력 상자가 그린다)
        if self.editing == Some(p)
            && let Some(ar) = self.address_rect()
        {
            g.fill_round(ar, R_CTRL, t.field);
            g.stroke_round(ar.inset(0.75, 0.75), R_CTRL, t.accent, 1.5);
            if self.standalone {
                g.text(dir, R::new(ar.l + 8.0, ar.t, ar.r - 8.0, ar.b), Font::Path, t.text);
            }
            if gm.crumb.w() < ADDR_MIN_W {
                return self.paint_list(g, t, p, r, &gm);
            }
            g.glyph(G_NEW_FOLDER, gm.new_folder, false, off);
            g.glyph(G_LIST, R::new(gm.view.l + 2.0, gm.view.t, gm.view.l + 26.0, gm.view.b), false, off);
            g.glyph(G_CHEVRON_DOWN, R::new(gm.view.r - 16.0, gm.view.t + 1.0, gm.view.r - 4.0, gm.view.b + 1.0), true, t.faint);
            return self.paint_list(g, t, p, r, &gm);
        }
        if hover == Hit::CrumbEmpty(p) {
            g.fill_round(gm.crumb, R_CTRL, t.hover);
        }
        g.glyph(G_NEW_FOLDER, gm.new_folder, false, off);
        g.glyph(G_LIST, R::new(gm.view.l + 2.0, gm.view.t, gm.view.l + 26.0, gm.view.b), false, off);
        g.glyph(G_CHEVRON_DOWN, R::new(gm.view.r - 16.0, gm.view.t + 1.0, gm.view.r - 4.0, gm.view.b + 1.0), true, t.faint);
        let starred = self.favs.find(dir).is_some();
        if hover == Hit::Star(p) {
            g.fill_round(gm.star, R_CTRL, t.hover);
        }
        g.glyph(if starred { G_STAR_FILL } else { G_STAR }, gm.star, false, if starred || hover == Hit::Star(p) { t.text } else { t.dim });
        let cs = self.crumbs(g, gm.crumb, dir);
        let n = cs.len();
        g.clip(gm.crumb);
        for (i, (cr, path, label)) in cs.iter().enumerate() {
            let last = i + 1 == n;
            if hover == Hit::Crumb(p, i) {
                g.fill_round(R::new(cr.l, gm.crumb.t + 2.0, cr.r, gm.crumb.b - 2.0), R_ROW, t.sel_inactive);
            }
            if path.is_none() {
                g.glyph(G_MORE, *cr, true, t.dim);
            } else {
                g.text(label, R::new(cr.l + CRUMB_PAD, cr.t, cr.r - CRUMB_PAD / 2.0, cr.b), Font::Normal, if last { t.text } else { t.dim });
            }
            if !last {
                g.glyph(G_CHEVRON, R::new(cr.r, cr.t, cr.r + CHEV_W, cr.b), true, t.faint);
            }
        }
        g.unclip();
        self.paint_list(g, t, p, r, &gm);
    }

    /// 칸의 열 머리글·목록·안내·스크롤 막대·테두리
    fn paint_list(&self, g: &Gfx, t: &Theme, p: usize, r: R, gm: &Geom) {
        let pane = &self.panes[p];
        let tab = pane.tab();
        let active = p == self.active;
        let hover = self.hover;
        // 열 머리글
        let cols = columns(gm.list);
        g.text("이름", R::new(cols.name.0 + 4.0, gm.header.t, cols.name.1, gm.header.b), Font::Normal, t.dim);
        let nw = g.measure("이름", Font::Normal);
        g.glyph(G_SORT_UP, R::new(cols.name.0 + 8.0 + nw, gm.header.t, cols.name.0 + 22.0 + nw, gm.header.b), true, t.faint);
        if let Some((l, rr)) = cols.modified {
            g.text("수정한 날짜", R::new(l + 8.0, gm.header.t, rr - 8.0, gm.header.b), Font::Normal, t.dim);
        }
        if let Some((l, rr)) = cols.size {
            g.text("크기", R::new(l + 8.0, gm.header.t, rr - 8.0, gm.header.b), Font::Right, t.dim);
        }
        g.fill(R::new(r.l + 8.0, gm.header.b - 1.0, r.r - 8.0, gm.header.b), alpha(t.text, 0.07));

        // 목록
        let now = listing::now_filetime();
        let list = gm.list;
        g.clip(list);
        let first = (tab.scroll / ROW_H).floor().max(0.0) as usize;
        let last = ((tab.scroll + list.h()) / ROW_H).ceil() as usize;
        for vi in first..last.min(tab.view.len()) {
            let e = &tab.entries[tab.view[vi] as usize];
            let y = list.t + vi as f32 * ROW_H - tab.scroll;
            let row = R::new(list.l + 8.0, y + 1.0, list.r - SCROLL_W - 6.0, y + ROW_H - 1.0);
            if e.selected {
                g.fill_round(row, R_ROW, if active { t.sel } else { t.sel_inactive });
            } else if hover == Hit::Row(p, vi) {
                g.fill_round(row, R_ROW, t.hover);
            }
            if active && vi == tab.cursor {
                g.stroke_round(row.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
            }
            if let Some(path) = tab.path_of(e) {
                g.icon_for_path(&path, R::new(cols.name.0 + 4.0, y + (ROW_H - 16.0) / 2.0, cols.name.0 + 20.0, y + (ROW_H + 16.0) / 2.0));
            }
            let fg = if e.is_hidden() && !(e.selected && active) { t.faint } else { t.text };
            g.text(&e.name_string(), R::new(cols.name.0 + 28.0, y, cols.name.1 - 8.0, y + ROW_H), Font::Normal, fg);
            if let Some((l, rr)) = cols.modified {
                g.text(&listing::format_time_short(e.modified, now), R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Normal, t.dim);
            }
            if let Some((l, rr)) = cols.size {
                let s = if e.kind == Kind::File { listing::format_size(e.size) } else { String::new() };
                g.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Right, t.dim);
            }
        }
        g.unclip();

        // 빈 목록 안내 (Windows와 같은 글)
        let hint = match &tab.state {
            LoadState::Loading if tab.entries.is_empty() => Some(format!("불러오는 중…\n{}", tab.loc.title())),
            LoadState::Error(msg) => Some(format!("폴더를 열 수 없어요\n{msg}\n⌘R을 누르면 다시 시도해요")),
            LoadState::Ready if tab.entries.is_empty() => Some("빈 폴더예요".to_string()),
            LoadState::Ready if tab.view.is_empty() => Some("숨김 항목만 있어요".to_string()),
            _ => None,
        };
        if let Some(h) = hint {
            for (i, line) in h.lines().enumerate() {
                let y = list.t + 24.0 + i as f32 * 20.0;
                g.text(line, R::new(list.l + 24.0, y, list.r - 24.0, y + 20.0), Font::Hint, t.faint);
            }
        }

        // 스크롤 막대: 넘칠 때만
        let content = tab.view.len() as f32 * ROW_H;
        if content > list.h() && list.h() > 0.0 {
            let th = (list.h() * list.h() / content).max(24.0);
            let ty = list.t + (list.h() - th) * (tab.scroll / (content - list.h()));
            g.fill_round(R::new(list.r - SCROLL_W + 1.0, ty, list.r - 4.0, ty + th), 2.5, t.thumb);
        }

        if t.dark {
            g.stroke_round(r.inset(0.5, 0.5), RADIUS, t.pane_border, 1.0);
        }
    }
}

/// 칩 줄 오른쪽 글: 항목 수, 고른 것이 있으면 고른 개수·크기
fn count_text(tab: &Tab) -> String {
    let (n, size) = tab.selection_stats();
    if n > 0 {
        let s = if size > 0 { format!(" · {}", listing::format_size(size)) } else { String::new() };
        format!("{}개 중 {}개 선택{}", tab.view.len(), n, s)
    } else {
        format!("{}개", tab.view.len())
    }
}

/// app.rs child_toward: ancestor 아래로 current가 있으면 ancestor 바로 밑 이름
fn child_toward(ancestor: &Location, current: &Location) -> Option<String> {
    let (Location::Dir(a), Location::Dir(cur)) = (ancestor, current) else { return None };
    // 한글처럼 여러 바이트인 글자 가운데를 자르지 않게 글자 경계에서만 비교한다
    let head = cur.get(..a.len()).filter(|h| cur.len() > a.len() && util::eq_path(h, a))?;
    let sep = if cur.starts_with('/') { '/' } else { '\\' };
    let rest = &cur[head.len()..];
    // "/tmp/a"는 "/tmp/ab"의 상위 폴더가 아니다 (구분자 바로 앞에서 끝나야 한다)
    if !a.ends_with(sep) && !rest.starts_with(sep) {
        return None;
    }
    rest.trim_start_matches(sep).split(sep).next().filter(|s| !s.is_empty()).map(String::from)
}

/// 탭의 스크롤을 목록 길이 안으로
fn clamp_tab(tab: &mut Tab, list: R) {
    let max = (tab.view.len() as f32 * ROW_H - list.h()).max(0.0);
    tab.scroll = tab.scroll.clamp(0.0, max);
}

/// 커서 줄이 보이게 탭을 스크롤한다
fn reveal(tab: &mut Tab, list: R) {
    let top = tab.cursor as f32 * ROW_H;
    if top < tab.scroll {
        tab.scroll = top;
    } else if top + ROW_H > tab.scroll + list.h() {
        tab.scroll = top + ROW_H - list.h();
    }
}

fn close_rect(tr: R) -> R {
    R::new(tr.r - 24.0, tr.t + 4.0, tr.r - 4.0, tr.b - 4.0)
}

/// 줄어든 탭 칩의 가장 좁은 폭
const CHIP_MIN: f32 = 64.0;

/// app.rs tab_rects와 같다. 다 들어가면 칩마다 이름 길이(natural)대로 둔다. 넘치면 지금 탭(active)은 제 폭을 지키고
/// 나머지가 남는 폭을 고르게 나눈다. 그래도 넘치면 지금 탭이 보이는 만큼만 보이고 나머지는 폭 0으로 양 끝에 둔다.
/// 새 탭(+) 자리 32를 늘 남긴다
fn tab_rects(strip: R, natural: &[f32], active: usize) -> Vec<R> {
    const GAP_X: f32 = 2.0;
    let n = natural.len();
    let avail = strip.w() - 32.0;
    let total = natural.iter().sum::<f32>() + GAP_X * n.saturating_sub(1) as f32;
    let place = |widths: &mut dyn Iterator<Item = (usize, f32)>, first: usize, shown: usize| -> Vec<R> {
        let mut out = vec![R::new(strip.l, strip.t, strip.l, strip.b); n];
        let mut x = strip.l;
        for (i, w) in widths {
            out[i] = R::new(x, strip.t, x + w, strip.b);
            x += w + GAP_X;
        }
        let end = (x - GAP_X).max(strip.l);
        for (i, r) in out.iter_mut().enumerate() {
            if i >= first + shown {
                *r = R::new(end, strip.t, end, strip.b);
            }
        }
        out
    };
    if n == 0 {
        return Vec::new();
    }
    if total <= avail {
        return place(&mut natural.iter().copied().enumerate(), 0, n);
    }
    let active = active.min(n - 1);
    let cur_w = natural[active].min((avail - CHIP_MIN - GAP_X).max(CHIP_MIN)).min(avail.max(0.0));
    let rest = avail - cur_w - GAP_X;
    let others = (((rest + GAP_X) / (CHIP_MIN + GAP_X) + 0.001).floor().max(0.0) as usize).min(n - 1);
    let shown = others + 1;
    let first = (active + 1).saturating_sub(shown).min(n - shown);
    let w_other = if others > 0 { ((rest + GAP_X) / others as f32 - GAP_X).min(180.0) } else { 0.0 };
    place(&mut (first..first + shown).map(|i| (i, if i == active { cur_w } else { natural[i].min(w_other) })), first, shown)
}

struct Cols {
    name: (f32, f32),
    modified: Option<(f32, f32)>,
    size: Option<(f32, f32)>,
}

/// app.rs columns와 같은 규칙 (유형 열은 아직 그리지 않지만 자리는 같게 비운다)
fn columns(list: R) -> Cols {
    let w = DEFAULT_COLS;
    let mut x = list.r - SCROLL_W - 6.0;
    let size = (list.w() >= 460.0).then(|| {
        let c = (x - w[2], x);
        x -= w[2];
        c
    });
    if list.w() >= 590.0 {
        x -= w[1];
    }
    let modified = (list.w() >= 300.0).then(|| {
        let c = (x - w[0], x);
        x -= w[0];
        c
    });
    Cols { name: (list.l + 12.0, x.max(list.l + 124.0)), modified, size }
}

// ---------- sidebar.rs와 같은 면 ----------

fn well(t: &Theme) -> Color {
    t.sel_inactive
}

fn seg_track(t: &Theme) -> Color {
    if t.dark { Color { r: 0.0, g: 0.0, b: 0.0, a: 0.28 } } else { t.sel_inactive }
}

fn hover_fill(t: &Theme) -> Color {
    if t.dark { Color { r: 1.0, g: 1.0, b: 1.0, a: 0.06 } } else { Color { r: 0.06, g: 0.09, b: 0.16, a: 0.05 } }
}

fn seg_on(g: &Gfx, r: R, radius: f32, t: &Theme) {
    if t.dark {
        g.fill_round(r, radius, t.sel_inactive);
        g.fill(R::new(r.l + radius, r.t + 0.5, r.r - radius, r.t + 1.5), Color { r: 1.0, g: 1.0, b: 1.0, a: 0.10 });
    } else {
        soft_shadow(g, r, radius, 3.0, 0.12, false);
        g.fill_round(r, radius, t.pane);
    }
}

/// app.rs soft_shadow와 같다
fn soft_shadow(g: &Gfx, r: R, radius: f32, depth: f32, strength: f32, dark: bool) {
    const LAYERS: usize = 8;
    let (base, k) = if dark { ((0.0, 0.0, 0.0), 2.5) } else { ((0.06, 0.09, 0.16), 1.0) };
    let ink = Color { r: base.0, g: base.1, b: base.2, a: (strength * k).min(0.7) / LAYERS as f32 };
    for i in 1..=LAYERS {
        let f = i as f32 / LAYERS as f32;
        let spread = depth * (f * 0.5 - 0.25);
        let dy = depth * f * 0.6;
        g.fill_round(R::new(r.l - spread, r.t + dy - spread, r.r + spread, r.b + dy + spread), radius + spread.max(0.0), ink);
    }
    g.fill_round(R::new(r.l - 0.5, r.t + 0.5, r.r + 0.5, r.b + 1.5), radius + 0.5, Color { a: if dark { 0.25 } else { 0.05 }, ..ink });
}

/// app.rs draw_layout_icon과 같다 (0 한 칸, 1 좌우, 2 위아래, 3 세 칸, 4 네 칸)
fn draw_layout_icon(g: &Gfx, r: R, layout: usize, c: Color) {
    let (cx, cy) = ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0);
    let ic = R::new(cx - 9.0, cy - 7.0, cx + 9.0, cy + 7.0);
    g.stroke_round(ic, 2.0, c, 1.3);
    let v = |x: f32, t: f32, b: f32| g.fill(R::new(x - 0.65, t, x + 0.65, b), c);
    let h = |y: f32, l: f32, rr: f32| g.fill(R::new(l, y - 0.65, rr, y + 0.65), c);
    match layout {
        1 => v(cx, ic.t, ic.b),
        2 => h(cy, ic.l, ic.r),
        3 => {
            v(cx, ic.t, ic.b);
            h(cy, cx, ic.r);
        }
        4 => {
            v(cx, ic.t, ic.b);
            h(cy, ic.l, ic.r);
        }
        _ => {}
    }
}

/// 맥 창의 닫기·최소화·확대 단추 (그림 시험용. 실제 창에서는 시스템이 그린다)
fn traffic_lights(g: &Gfx, cy: f32) {
    for (i, hex) in [0xFF5F57u32, 0xFEBC2E, 0x28C840].into_iter().enumerate() {
        let cx = 20.0 + i as f32 * 20.0;
        let c = Color { r: ((hex >> 16) & 0xFF) as f32 / 255.0, g: ((hex >> 8) & 0xFF) as f32 / 255.0, b: (hex & 0xFF) as f32 / 255.0, a: 1.0 };
        let r = R::new(cx - 6.0, cy - 6.0, cx + 6.0, cy + 6.0);
        g.fill_round(r, 6.0, c);
        g.stroke_round(r.inset(0.25, 0.25), 6.0, Color { r: 0.0, g: 0.0, b: 0.0, a: 0.12 }, 0.5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 시험용 폴더: 하위 폴더 둘, 파일 셋, 숨김 파일 하나
    fn demo() -> (std::path::PathBuf, String) {
        let root = std::env::temp_dir().join(format!("sakadu-mac-app-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("업무").join("2026 4분기")).unwrap();
        std::fs::create_dir_all(root.join("다운로드")).unwrap();
        for f in ["업무/예산.csv", "업무/회의록.md", "업무/2026 4분기/보고서.docx", "다운로드/사진.jpg", ".숨김"] {
            std::fs::write(root.join(f), b"x").unwrap();
        }
        let path = util::normalize(&std::fs::canonicalize(&root).unwrap().to_string_lossy());
        (root, path)
    }

    fn app(home: &str) -> (App, Gfx) {
        let mut a = App::new(1280.0, 800.0, Theme::LIGHT, home, Favorites::default(), Vec::new());
        a.settle();
        (a, Gfx::bitmap(8.0, 8.0, 1.0).unwrap())
    }

    /// 칸 p의 줄 vi 가운데 좌표
    fn row_point(a: &App, p: usize, vi: usize) -> (f32, f32) {
        let l = a.list_rect(p).unwrap();
        (l.l + 60.0, l.t + vi as f32 * ROW_H + ROW_H / 2.0 - a.pane(p).tab().scroll)
    }

    fn names(a: &App, p: usize) -> Vec<String> {
        let t = a.pane(p).tab();
        t.view.iter().map(|&i| t.entries[i as usize].name_string()).collect()
    }

    #[test]
    fn opens_folders_and_goes_back_up_with_the_child_selected() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        assert_eq!(names(&a, 0), ["다운로드", "업무"], "folders first, hidden file not shown");
        let (x, y) = row_point(&a, 0, 1);
        a.mouse_down(&g, x, y, Mods::default(), 2);
        a.settle();
        assert_eq!(names(&a, 0), ["2026 4분기", "예산.csv", "회의록.md"]);
        assert_eq!(a.title(), "업무 - Sakadu");
        a.go_up(0);
        a.settle();
        let t = a.pane(0).tab();
        assert_eq!(t.cursor_entry().unwrap().name_string(), "업무", "the folder we left is selected");
        a.go_back(0);
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(util::join(&home, "업무")));
        a.go_back(0);
        a.settle();
        assert_eq!(a.pane(0).tab().cursor_entry().unwrap().name_string(), "업무");
        // 파일을 두 번 누르면 macOS '열기'로 넘긴다
        a.navigate(0, Location::Dir(util::join(&home, "업무")), None);
        a.settle();
        let (x, y) = row_point(&a, 0, 1);
        a.mouse_down(&g, x, y, Mods::default(), 2);
        assert_eq!(a.open_files, [util::join(&util::join(&home, "업무"), "예산.csv")]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selection_follows_cmd_and_shift() {
        let (root, home) = demo();
        let (mut a, g) = app(&util::join(&home, "업무"));
        let click = |a: &mut App, vi: usize, m: Mods| {
            let (x, y) = row_point(a, 0, vi);
            a.mouse_down(&g, x, y, m, 1);
        };
        click(&mut a, 0, Mods::default());
        click(&mut a, 2, Mods { shift: true, ..Default::default() });
        assert_eq!(a.pane(0).tab().selection_stats().0, 3);
        click(&mut a, 1, Mods { cmd: true, ..Default::default() });
        assert_eq!(a.pane(0).tab().selection_mask(), [true, false, true]);
        assert!(a.key(Key::Esc, Mods::default()));
        assert_eq!(a.pane(0).tab().selection_stats().0, 0);
        assert!(a.key(Key::Char('a'), Mods { cmd: true, ..Default::default() }));
        assert_eq!(a.pane(0).tab().selection_stats().0, 3);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tabs_layouts_hidden_and_stars() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        assert!(a.key(Key::Char('t'), Mods { cmd: true, ..Default::default() }));
        a.settle();
        assert_eq!((a.pane(0).tabs.len(), a.pane(0).active), (2, 1));
        assert!(a.key(Key::Char('w'), Mods { cmd: true, ..Default::default() }));
        assert_eq!(a.pane(0).tabs.len(), 1);
        a.key(Key::Char('w'), Mods { cmd: true, ..Default::default() });
        assert_eq!(a.pane(0).tabs.len(), 1, "the last tab stays");
        a.key(Key::Char('4'), Mods { cmd: true, alt: true, ..Default::default() });
        a.settle();
        assert_eq!(a.layout(), Layout::Three);
        a.key(Key::Char('5'), Mods { cmd: true, alt: true, ..Default::default() });
        a.settle();
        assert_eq!(a.layout(), Layout::Four);
        assert!(names(&a, 3).contains(&"업무".to_string()), "a pane shown for the first time is loaded");
        a.key(Key::Tab, Mods::default());
        assert_eq!(a.active(), 1);
        a.key(Key::Char('.'), Mods { cmd: true, shift: true, ..Default::default() });
        assert!(names(&a, 0).contains(&".숨김".to_string()));
        // 별: 지금 칸 폴더를 즐겨찾기에 넣고, 사이드바 줄을 누르면 그 폴더로
        a.toggle_star(1);
        assert_eq!(a.favorites().items.len(), 1);
        a.navigate(1, Location::Dir(util::join(&home, "다운로드")), None);
        a.settle();
        let r = a.side_row_rect(0);
        a.mouse_down(&g, r.l + 40.0, r.t + 10.0, Mods::default(), 1);
        a.settle();
        assert_eq!(a.pane(1).tab().loc, Location::Dir(home.clone()));
        a.toggle_star(1);
        assert!(a.favorites().items.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn split_drag_and_scroll_stay_in_range() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let (sx, _) = a.splitters();
        let s = sx.unwrap();
        a.mouse_down(&g, s.l + 2.0, s.t + 50.0, Mods::default(), 1);
        a.mouse_dragged(10_000.0, 0.0);
        a.mouse_up();
        assert!((a.split.0 - 0.85).abs() < 1e-4);
        let (x, y) = row_point(&a, 0, 0);
        a.scroll(&g, x, y, 500.0);
        assert_eq!(a.pane(0).tab().scroll, 0.0, "a short list does not scroll");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 회귀: 한글 이름 폴더 사이를 뒤로 갈 때 글자 가운데를 잘라 멈추지 않는다. 이름이 겹치는 형제 폴더는 상위가 아니다
    #[test]
    fn child_toward_is_safe_with_korean_names() {
        let d = |s: &str| Location::Dir(s.to_string());
        assert_eq!(child_toward(&d("/tmp/a"), &d("/tmp/한글")), None);
        assert_eq!(child_toward(&d("/tmp/a"), &d("/tmp/ab/c")), None);
        assert_eq!(child_toward(&d("/tmp"), &d("/tmp/한글/사진")).as_deref(), Some("한글"));
        assert_eq!(child_toward(&d("/"), &d("/Users/hong")).as_deref(), Some("Users"));
    }

    /// 탭이 넘쳐도 지금 탭은 보이고 닫을 수 있다 (나머지는 줄거나 가려지고 ⌃Tab으로 닿는다)
    #[test]
    fn active_tab_stays_visible_when_tabs_overflow() {
        let (root, home) = demo();
        let long = util::join(&home, "업무");
        let (mut a, g) = app(&long);
        for _ in 0..12 {
            a.new_tab(0);
        }
        a.settle();
        let head = geom(a.pane_rects()[0].1).head;
        let (trs, add) = a.chips(&g, 0, head);
        let cur = trs[a.pane(0).active];
        assert!(cur.w() >= 52.0 && cur.r <= add.l, "{cur:?} {add:?}");
        assert!(add.r <= head.r, "the + button stays inside the pane");
        let (x, y) = (close_rect(cur).l + 4.0, cur.t + 8.0);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!(a.pane(0).tabs.len(), 12, "the active tab's close button works");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 즐겨찾기가 사이드바 아래로 넘치면 스크롤해서 마지막 줄까지 누를 수 있다
    #[test]
    fn sidebar_scrolls_to_rows_below_the_window() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let favs: Vec<Favorite> = (0..40).map(|i| Favorite { name: format!("폴더 {i}"), path: util::join(&home, "다운로드"), is_dir: true }).collect();
        a.favs = Favorites { items: favs, groups: Vec::new() };
        a.resize(1280.0, 420.0);
        let last = a.side_row_rect(39);
        assert!(last.t > a.side_list().b, "the last row starts below the window");
        a.scroll(&g, 100.0, 300.0, 100_000.0);
        let last = a.side_row_rect(39);
        assert!(last.b <= a.side_list().b + 0.5 && last.t >= a.side_list().t, "{last:?}");
        a.mouse_down(&g, last.l + 40.0, last.t + 10.0, Mods::default(), 1);
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(util::join(&home, "다운로드")));
        // 헤더 위로 밀려난 줄은 누를 수 없다
        let first = a.side_row_rect(0);
        assert!(first.b < a.side_list().t);
        assert_eq!(a.hit(&g, first.l + 40.0, a.side_list().t - 6.0), Hit::None);
        // 줄이 줄면 스크롤도 줄 수 안으로
        a.favs.items.truncate(3);
        a.resize(1280.0, 420.0);
        assert_eq!(a.side_scroll, 0.0);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 위로 간 뒤 다 읽기 전에 다른 탭으로 옮겨도, 떠난 폴더가 보이게 하는 스크롤은 그 탭에만 적용된다
    #[test]
    fn reveal_scroll_stays_with_the_listed_tab() {
        let (root, home) = demo();
        let long = util::join(&home, "긴 폴더");
        for i in 0..100 {
            std::fs::create_dir_all(util::join(&long, &format!("sub_{i:03}"))).unwrap();
        }
        let (mut a, _g) = app(&long);
        a.new_tab(0);
        a.settle();
        // 보고 있을 탭: 커서는 아래쪽, 스크롤은 맨 위
        a.tab_mut(0).cursor = 80;
        a.tab_mut(0).scroll = 0.0;
        a.activate_tab(0, 0);
        a.navigate(0, Location::Dir(util::join(&long, "sub_090")), None);
        a.settle();
        a.go_up(0);
        a.activate_tab(0, 1);
        a.settle();
        assert_eq!(a.pane(0).tabs[1].scroll, 0.0, "the tab being viewed did not jump");
        let listed = &a.pane(0).tabs[0];
        assert_eq!(listed.cursor_entry().unwrap().name_string(), "sub_090");
        assert!(listed.scroll > 0.0, "the folder we left is scrolled into view in its own tab");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 주소 입력 해석: 폴더, 파일(그 폴더에서 고름), ~, 없는 경로·상대 경로
    #[test]
    fn resolve_input_like_windows() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        assert_eq!(resolve_input(&format!("  {work}/ "), "/x"), Ok((work.clone(), None)));
        assert_eq!(resolve_input(&util::join(&work, "예산.csv"), "/x"), Ok((work.clone(), Some("예산.csv".to_string()))));
        assert_eq!(resolve_input("~", &home), Ok((home.clone(), None)));
        assert_eq!(resolve_input("~/업무", &home), Ok((work.clone(), None)));
        assert!(resolve_input(&util::join(&home, "없는 폴더"), "/x").is_err());
        assert!(resolve_input("업무", &home).is_err(), "relative paths are not guessed");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 경로 줄 빈 곳을 누르면 입력, Return이면 그 경로로, 없는 경로면 칩 줄 알림, 다른 곳을 누르면 취소
    #[test]
    fn address_edit_commits_cancels_and_flashes() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let gm = geom(a.pane_rects()[1].1);
        let (x, y) = (gm.crumb.r - 6.0, gm.crumb.t + 10.0);
        assert_eq!(a.hit(&g, x, y), Hit::CrumbEmpty(1));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!((a.editing, a.active()), (Some(1), 1));
        assert_eq!(a.address_text(), home);
        assert!(a.address_rect().is_some());
        a.commit_address(&util::join(&util::join(&home, "업무"), "회의록.md"));
        a.settle();
        assert_eq!(a.editing, None);
        assert_eq!(a.pane(1).tab().loc, Location::Dir(util::join(&home, "업무")));
        assert_eq!(a.pane(1).tab().cursor_entry().unwrap().name_string(), "회의록.md");
        // 없는 경로: 그대로 두고 알림
        a.key(Key::Char('l'), Mods { cmd: true, ..Default::default() });
        assert_eq!(a.editing, Some(1));
        a.commit_address("/없는/경로");
        a.settle();
        assert!(a.flash.as_ref().is_some_and(|(p, m)| *p == 1 && m.contains("찾을 수 없어요")));
        assert_eq!(a.pane(1).tab().loc, Location::Dir(util::join(&home, "업무")));
        // ⌘⇧G로 열고, 목록을 누르면 취소된다 (알림도 지운다)
        a.key(Key::Char('g'), Mods { cmd: true, shift: true, ..Default::default() });
        assert_eq!(a.editing, Some(1));
        let l = a.list_rect(0).unwrap();
        a.mouse_down(&g, l.l + 60.0, l.b - 10.0, Mods::default(), 1);
        assert_eq!((a.editing, a.flash.is_none()), (None, true));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 세션: 화면 상태를 저장했다가 그대로 연다. 없어진 폴더는 홈으로
    #[test]
    fn session_round_trip() {
        let (root, home) = demo();
        let (mut a, _g) = app(&home);
        a.navigate(0, Location::Dir(util::join(&home, "업무")), None);
        a.new_tab(1);
        a.navigate(1, Location::Dir(util::join(&home, "다운로드")), None);
        a.set_layout(Layout::Three);
        a.activate_pane(2);
        a.set_side(Side::Places);
        a.toggle_hidden();
        a.split = (0.3, 0.6);
        a.settle();
        let mut s = a.to_session();
        // 다시 읽은 세션 글자로 연다 (파일 형식까지)
        s = session::parse(&session::serialize(&s)).expect("session");
        s.panes[3].tabs = vec![util::join(&home, "지운 폴더")];
        let (mut b, _g) = app(&home);
        b.restore(&s, &home);
        b.settle();
        assert_eq!((b.layout(), b.active(), b.side, b.show_hidden, b.split), (Layout::Three, 2, Side::Places, true, (0.3, 0.6)));
        assert_eq!(b.pane(0).tab().loc, Location::Dir(util::join(&home, "업무")));
        assert_eq!(b.pane(1).tabs.len(), 2);
        assert_eq!(b.pane(1).active, 1);
        assert_eq!(b.pane(1).tab().loc, Location::Dir(util::join(&home, "다운로드")));
        assert_eq!(b.pane(3).tab().loc, Location::Dir(home.clone()), "a missing folder opens home");
        assert!(names(&b, 0).contains(&"예산.csv".to_string()), "visible panes are loaded");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn crumbs_walk_up_and_select_the_child() {
        let (root, home) = demo();
        let (mut a, g) = app(&util::join(&util::join(&home, "업무"), "2026 4분기"));
        let gm = geom(a.pane_rects()[0].1);
        let cs = a.crumbs(&g, gm.crumb, a.pane(0).tab().loc.dir().unwrap());
        let i = cs.iter().position(|(_, _, l)| l == "업무").unwrap();
        let (cr, _, _) = &cs[i];
        a.mouse_down(&g, cr.l + 4.0, cr.t + 8.0, Mods::default(), 1);
        a.settle();
        assert_eq!(a.pane(0).tab().cursor_entry().unwrap().name_string(), "2026 4분기");
        std::fs::remove_dir_all(root).unwrap();
    }
}
