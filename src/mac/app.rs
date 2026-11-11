//! 맥 시제품 앱: 칸·탭·사이드바 상태, 그리기(paint), 누르기 판정(hit), 동작.
//! 화면 모양은 '모양 2(맥 관례)': 사이드바가 창 위까지 올라가고 신호등은 그 안, 아이콘 막대 대신 고르기 단추,
//! 설정은 작업 줄 오른쪽 끝. 화면 계산·그리기 도우미·치수와 칸 그리기는 Windows와 함께 쓰는 sakadu::ui를 쓴다
//! (칸은 이 앱의 상태로 장면을 채워 sakadu::ui::paint가 그린다).
//! 창(NSView)과 상관없이 돌아가서, 시험과 그림(--render)은 창 없이 같은 코드를 쓴다

use crate::gfx::Gfx;
use sakadu::draw::{Color, Font, R};
use sakadu::favorites::{self, Favorite, Favorites, Row};
use sakadu::listing::{self, Entry, Kind};
use sakadu::palette::{R_ROW, Theme, alpha};
use sakadu::pane::{DEFAULT_COLS, LoadState, Location, Restore, Sort, Tab, ViewMode, child_toward};
use sakadu::session::{self, Look, PaneState, Session};
use sakadu::ui::glyph::*;
use sakadu::ui::paint::{EntryArt, PaneHover, PaneScene, paint_pane_border, paint_pane_head, paint_pane_list};
use sakadu::ui::toolbar::{ToolbarGeom, ToolbarHover, ToolbarScene, ToolbarSpec, can_go_up, paint_toolbar, toolbar_geom};
use sakadu::ui::*;
use sakadu::util;
use std::cell::RefCell;
use std::collections::HashMap;

/// 모양 2의 사이드바 폭: 아이콘 막대와 패널(기본 220)을 합친 폭
const SIDE_W: f32 = BAR_W + 220.0;
/// 맥 창의 신호등 아래 끝 (창에서 읽기 전 기본값, 그림 시험에서도 쓴다)
pub const LIGHTS_BOTTOM: f32 = 28.0;

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
    /// 도구 막대의 칸 나누기·보기 방식 단추 (누르면 메뉴가 열린다)
    LayoutPick,
    ViewPick,
    Tab(usize, usize),
    TabClose(usize, usize),
    TabAdd(usize),
    /// 도구 막대의 뒤로·앞으로 (지금 칸)
    Back(usize),
    Fwd(usize),
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

/// 도구 막대 단추가 여는 메뉴
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuKind {
    /// 칸 나누기 다섯 가지
    Layout,
    /// 보기 방식 여섯 가지와 맨 아래 '숨김 항목 보기'
    View,
}

/// 맥 키 (글자 키는 소문자 그대로)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
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
    pub view: ViewMode,
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
    /// 확인을 시작할 때 그 탭의 세대 번호 (그 사이 다른 곳으로 옮기면 바뀐다)
    pub generation: u64,
    /// 그 칸의 경로 확인 요청 번호 (나중에 낸 요청이 이긴다)
    pub seq: u64,
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
    /// 칸마다 마지막으로 낸 경로 확인 요청 번호
    probe_seq: [u64; 4],
    /// 저장한 세션에서 연 탭과 그 폴더. 그 폴더의 첫 읽기가 실패하면 home으로 바꾼다
    /// (그 사이 다른 곳으로 옮겼으면 그곳의 오류는 그대로 보인다)
    restored: std::collections::HashMap<u64, String>,
    /// 홈 폴더 (세션에서 연 폴더가 없을 때)
    home: String,
    /// 사이드바 목록 스크롤 (줄이 창 아래로 넘칠 때)
    side_scroll: f32,
    /// 열어 달라는 파일 (창 쪽이 macOS '열기'로 연다)
    pub open_files: Vec<String>,
    /// 즐겨찾기를 파일에 저장할지 (시험·그림에서는 저장하지 않는다)
    pub persist: bool,
    /// 열어 달라는 메뉴 (창 쪽이 맥 기본 메뉴로 연다)
    menu: Option<MenuKind>,
    /// Finder '종류' 이름: 확장자별로 한 번만 묻는다 (폴더는 앞에 '/')
    kinds: RefCell<HashMap<String, String>>,
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
            probe_seq: [0; 4],
            restored: std::collections::HashMap::new(),
            home: home.to_string(),
            side_scroll: 0.0,
            open_files: Vec::new(),
            persist: false,
            menu: None,
            kinds: RefCell::new(HashMap::new()),
        };
        for _ in 0..4 {
            let tab = app.new_tab_at(Location::Dir(home.to_string()));
            app.panes.push(Pane { tabs: vec![tab], active: 0, view: ViewMode::Details });
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
        let (list, view) = (self.list_rect(p), self.panes[p].view);
        let current = self.panes[p].tabs.iter().any(|t| t.id == tab_id && t.generation == generation);
        if !current {
            return;
        }
        // 세션에서 연 탭인지는 그 탭의 첫 읽기 결과에서만, 그리고 아직 저장한 그 폴더에 있을 때만 본다
        let saved = self.restored.remove(&tab_id);
        let home = self.home.clone();
        let Some(tab) = self.panes[p].tabs.iter_mut().find(|t| t.id == tab_id) else { return };
        let restored = saved.is_some_and(|d| tab.loc.dir() == Some(d.as_str()));
        match result {
            Ok(entries) => {
                let restore = tab.set_entries(entries, sort, hidden);
                // 스크롤은 다 읽은 그 탭에 맞춘다 (그 사이 다른 탭으로 옮겼어도 보고 있는 탭은 그대로)
                if let Some(list) = list {
                    clamp_tab(tab, list, view);
                    if restore.is_some_and(|r| r.reveal) {
                        reveal(tab, list, view);
                    }
                }
            }
            // 세션에서 연 폴더가 없어졌거나 닿지 않으면 home을 연다 (기록은 남기지 않는다)
            Err(_) if restored && tab.loc != Location::Dir(home.clone()) => {
                tab.loc = Location::Dir(home.clone());
                tab.generation += 1;
                tab.state = LoadState::Loading;
                self.requests.push(Request { tab: tab.id, generation: tab.generation, dir: home });
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
                self.on_probed(&p, result);
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
        let (tab, generation) = (self.panes[p].tab().id, self.panes[p].tab().generation);
        self.probe_seq[p] += 1;
        self.probes.push(Probe { pane: p, tab, generation, seq: self.probe_seq[p], text: text.to_string() });
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

    /// 경로 확인 결과. 그 사이 그 칸의 지금 탭이 바뀌었거나, 그 탭이 다른 곳으로 옮겼거나, 더 나중에 낸 경로 입력이
    /// 있으면 버린다 (확인이 늦게 끝난 순서대로 옮겨 다니지 않게)
    pub fn on_probed(&mut self, probe: &Probe, result: Result<(String, Option<String>), String>) {
        let p = probe.pane;
        let tab = self.panes[p].tab();
        if tab.id != probe.tab || tab.generation != probe.generation || self.probe_seq[p] != probe.seq {
            return;
        }
        match result {
            Ok((dir, select)) => {
                // 대소문자를 가리는 볼륨에서는 Work와 work가 다른 폴더라 글자 그대로 비교한다
                let here = self.panes[p].tab().loc.dir() == Some(dir.as_str());
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
        Some(addr_rect(&geom(r)))
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
                    view: pane.view.code(),
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
        self.home = home.to_string();
        for (p, ps) in s.panes.iter().take(self.panes.len()).enumerate() {
            let mut tabs = Vec::new();
            for t in &ps.tabs {
                // 폴더가 아직 있는지는 여기서 보지 않는다 (꺼진 네트워크 폴더면 창이 뜨기 전에 멈춘다).
                // 처음 읽기가 실패하면 on_listed가 home으로 바꾼다
                let dir = match Location::from_session(t) {
                    Location::Dir(d) => d,
                    _ => home.to_string(),
                };
                let tab = self.new_tab_at(Location::Dir(dir.clone()));
                self.restored.insert(tab.id, dir);
                tabs.push(tab);
            }
            if !tabs.is_empty() {
                let active = ps.active.min(tabs.len() - 1);
                self.panes[p] = Pane { tabs, active, view: ViewMode::from_code(ps.view) };
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

    /// 칸 p의 보기 방식을 바꾼다. 커서 줄이 계속 보이게 스크롤을 맞춘다
    pub fn set_view(&mut self, p: usize, view: ViewMode) {
        self.panes[p].view = view;
        self.clamp_scroll(p);
        self.ensure_visible(p);
    }

    /// 열어 달라는 메뉴와 그 단추 자리 (창 쪽이 가져가서 맥 기본 메뉴로 연다)
    pub fn take_menu(&mut self) -> Option<(MenuKind, R)> {
        let kind = self.menu.take()?;
        let m = self.toolbar();
        Some((kind, if kind == MenuKind::Layout { m.layout } else { m.view }))
    }

    /// 메뉴의 줄: (이름, 지금 것인지). 보기 방식 메뉴의 맨 아래 줄은 숨김 항목 보기
    pub fn menu_items(&self, kind: MenuKind) -> Vec<(String, bool)> {
        use sakadu::keys::{Action, name};
        match kind {
            MenuKind::Layout => {
                let names = [Action::Layout1, Action::Layout2, Action::Layout3, Action::Layout4, Action::Layout5];
                Layout::ALL.iter().zip(names).map(|(l, a)| (name(a).to_string(), *l == self.layout)).collect()
            }
            MenuKind::View => {
                let current = self.panes[self.active].view;
                let mut v: Vec<_> = ViewMode::ALL.iter().map(|v| (v.label().to_string(), *v == current)).collect();
                v.push(("숨김 항목 보기".to_string(), self.show_hidden));
                v
            }
        }
    }

    /// 메뉴에서 i번째 줄을 골랐다
    pub fn pick_menu(&mut self, kind: MenuKind, i: usize) {
        match kind {
            MenuKind::Layout => {
                if let Some(l) = Layout::ALL.get(i) {
                    self.set_layout(*l);
                }
            }
            MenuKind::View => match ViewMode::ALL.get(i) {
                Some(v) => self.set_view(self.active, *v),
                None => self.toggle_hidden(),
            },
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

    /// 커서 이동 (화살표·Home·End·Page). 아이콘 보기에서는 칸 배치를 따라 움직인다 (Windows와 같은 규칙). shift는 범위 고르기
    pub fn move_cursor(&mut self, key: Key, shift: bool) {
        let p = self.active;
        let Some(list) = self.list_rect(p) else { return };
        let grid = Grid::of(list, self.panes[p].view);
        let tab = self.panes[p].tab_mut();
        let n = tab.view.len();
        if n == 0 {
            return;
        }
        let (cur, last) = (tab.cursor as isize, n as isize - 1);
        // 한 줄(목록 보기는 한 열)에 든 개수와 한 화면에 든 개수
        let (line, page) = if grid.horiz {
            (grid.rows as isize, ((list.w() / grid.cell_w).floor() as isize).max(1) * grid.rows as isize)
        } else {
            (grid.cols as isize, ((list.h() / grid.cell_h).floor() as isize).max(1) * grid.cols as isize)
        };
        // 줄을 건너는 이동: 끝 줄이 덜 찼으면 아래 칸 대신 마지막 항목으로
        let jump = |fwd: bool| match fwd {
            false if cur < line => cur,
            false => cur - line,
            true if cur + line > last && cur / line < last / line => last,
            true if cur + line > last => cur,
            true => cur + line,
        };
        // 목록 보기는 위아래가 한 칸, 좌우가 옆 열. 나머지는 좌우가 한 칸, 위아래가 옆 줄
        let (prev, next, back, fwd) = if grid.horiz { (Key::Up, Key::Down, Key::Left, Key::Right) } else { (Key::Left, Key::Right, Key::Up, Key::Down) };
        let to = match key {
            k if k == prev => cur - 1,
            k if k == next => cur + 1,
            k if k == back => jump(false),
            k if k == fwd => jump(true),
            Key::Home => 0,
            Key::End => last,
            Key::PageUp => cur - page,
            Key::PageDown => cur + page,
            _ => return,
        };
        tab.move_cursor(to.clamp(0, last) as usize, shift, false);
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

    fn pane_rects(&self) -> Vec<(usize, R)> {
        pane_rects(self.area(), self.split, self.layout)
    }

    fn splitters(&self) -> (Option<R>, Option<R>) {
        splitters(self.area(), self.split, self.layout)
    }

    fn list_rect(&self, p: usize) -> Option<R> {
        self.pane_rects().into_iter().find(|(i, _)| *i == p).map(|(_, r)| geom(r).list)
    }

    fn clamp_scroll(&mut self, p: usize) {
        let Some(list) = self.list_rect(p) else { return };
        let view = self.panes[p].view;
        for tab in &mut self.panes[p].tabs {
            clamp_tab(tab, list, view);
        }
    }

    fn ensure_visible(&mut self, p: usize) {
        let Some(list) = self.list_rect(p) else { return };
        let view = self.panes[p].view;
        reveal(self.panes[p].tab_mut(), list, view);
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

    /// 도구 막대 단추 자리: 칸 카드의 왼쪽·오른쪽 가장자리에 맞춘다.
    /// 새 폴더·미리 보기는 그 기능이 생길 때 넣는다. 위로는 ⌘↑와 경로 조각으로 한다 (Finder와 같다)
    fn toolbar(&self) -> ToolbarGeom {
        let a = self.area();
        toolbar_geom(&ToolbarSpec { left: a.l, right: a.r, up: false, new_folder: false, preview: false, more: false })
    }

    /// 탭 칩 자리와 새 탭(+) 자리 (app.rs chips와 같다: 오른쪽 항목 수 자리 앞까지, 넘치면 지금 탭을 지킨다)
    fn chips(&self, g: &Gfx, p: usize, gm: &PaneGeom) -> (Vec<R>, R) {
        let pane = &self.panes[p];
        let n = pane.tabs.len();
        let strip = chip_strip(gm, g.measure(&count_text(pane.tab()), Font::Small));
        let natural: Vec<f32> = pane.tabs.iter().map(|t| (g.measure(&t.loc.title(), Font::Normal) + 22.0 + if n > 1 { 20.0 } else { 0.0 }).clamp(52.0, 180.0)).collect();
        let trs = tab_rects(strip, &natural, pane.active);
        let add = add_rect(strip, &trs);
        (trs, add)
    }

    /// 경로 조각 (공통 crumbs, 맥에는 '내 PC'가 없다): (자리, 그 조각의 폴더, 이름). 앞쪽을 접은 "…"도 그 상위 폴더로 간다
    fn crumbs(&self, g: &Gfx, area: R, dir: &str) -> Vec<(R, Option<String>, String)> {
        crumbs(g, area, &Location::Dir(dir.to_string()), None)
            .into_iter()
            .map(|c| (R::new(c.x0, area.t, c.x1, area.b), c.target.dir().map(str::to_string), c.label))
            .collect()
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
            // 도구 막대: 뒤로·앞으로는 지금 칸에 대해 움직인다
            let m = self.toolbar();
            let p = self.active;
            let hits = [(m.back, Hit::Back(p)), (m.fwd, Hit::Fwd(p)), (m.layout, Hit::LayoutPick), (m.view, Hit::ViewPick)];
            return hits.into_iter().find(|(r, _)| r.contains(x, y)).map_or(Hit::None, |(_, h)| h);
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
                let (trs, add) = self.chips(g, p, &gm);
                for (i, tr) in trs.iter().enumerate() {
                    if tr.contains(x, y) {
                        let closable = self.panes[p].tabs.len() > 1 && (i == self.panes[p].active || self.hover == Hit::Tab(p, i) || self.hover == Hit::TabClose(p, i));
                        return if closable && close_rect(*tr).contains(x, y) { Hit::TabClose(p, i) } else { Hit::Tab(p, i) };
                    }
                }
                return if add.contains(x, y) { Hit::TabAdd(p) } else { Hit::Pane(p) };
            }
            if gm.star.contains(x, y) {
                return Hit::Star(p);
            }
            if gm.crumb.contains(x, y) {
                if let Some(dir) = tab.loc.dir()
                    && let Some(i) = self.crumbs(g, gm.crumbs, dir).iter().position(|(cr, path, _)| path.is_some() && cr.contains(x, y))
                {
                    return Hit::Crumb(p, i);
                }
                return Hit::CrumbEmpty(p);
            }
            if gm.list.contains(x, y) {
                let grid = Grid::of(gm.list, self.panes[p].view);
                return grid.at(x, y, tab.scroll, tab.view.len()).map_or(Hit::List(p), |vi| Hit::Row(p, vi));
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
            Hit::Tab(p, _) | Hit::TabClose(p, _) | Hit::TabAdd(p) | Hit::Back(p) | Hit::Fwd(p) | Hit::Crumb(p, _) | Hit::CrumbEmpty(p) | Hit::Star(p) | Hit::Row(p, _) | Hit::List(p) | Hit::Pane(p) => {
                self.activate_pane(p)
            }
            _ => {}
        }
        match hit {
            Hit::CrumbEmpty(p) => self.begin_address(p),
            Hit::Seg(side) => self.set_side(side),
            Hit::SideRow(i) => self.open_side_row(i),
            Hit::LayoutPick => self.menu = Some(MenuKind::Layout),
            Hit::ViewPick => self.menu = Some(MenuKind::View),
            Hit::Tab(p, i) => self.activate_tab(p, i),
            Hit::TabClose(p, i) => self.close_tab(p, i),
            Hit::TabAdd(p) => self.new_tab(p),
            Hit::Back(p) => self.go_back(p),
            Hit::Fwd(p) => self.go_forward(p),
            Hit::Star(p) => self.toggle_star(p),
            Hit::Crumb(p, i) => {
                let Some(r) = self.pane_rects().into_iter().find(|(q, _)| *q == p).map(|(_, r)| geom(r).crumbs) else { return };
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
            (Key::Up | Key::Down | Key::Left | Key::Right, false, false, false) | (Key::Home | Key::End | Key::PageUp | Key::PageDown, false, false, false) => self.move_cursor(key, m.shift),
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

    pub fn paint(&self, g: &mut Gfx) {
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

    /// 칸 영역 위 도구 막대: 뒤로·앞으로, 지금 폴더 이름, 칸 나누기·보기 방식 단추 (공통 그리기)
    fn paint_workbar(&self, g: &Gfx, t: &Theme) {
        let m = self.toolbar();
        let p = self.active;
        let tab = self.panes[p].tab();
        let title = tab.loc.title();
        let hover = match self.hover {
            Hit::Back(_) => ToolbarHover::Back,
            Hit::Fwd(_) => ToolbarHover::Fwd,
            Hit::LayoutPick => ToolbarHover::Layout,
            Hit::ViewPick => ToolbarHover::View,
            _ => ToolbarHover::None,
        };
        paint_toolbar(
            g,
            t,
            &ToolbarScene {
                geom: &m,
                title: &title,
                layout: self.layout,
                view: self.panes[p].view,
                can_back: !tab.back.is_empty(),
                can_fwd: !tab.fwd.is_empty(),
                can_up: can_go_up(&tab.loc),
                new_folder_ok: false,
                view_ok: true,
                preview_on: false,
                layout_open: self.menu == Some(MenuKind::Layout),
                view_open: self.menu == Some(MenuKind::View),
                hover,
            },
        );
    }

    /// 칸 하나: 이 앱의 상태로 장면을 채워 공통 칸 그리기에 넘긴다 (맥에 아직 없는 필터·이름 바꾸기·끌어 놓기는 비워 둔다)
    fn paint_pane(&self, g: &mut Gfx, t: &Theme, p: usize, r: R) {
        let pane = &self.panes[p];
        let tab = pane.tab();
        let active = p == self.active;
        let gm = geom(r);
        let (tabs, add) = self.chips(g, p, &gm);
        // 칩 줄 오른쪽: 항목 수·고른 개수, 잠깐 뜨는 알림이면 그 글을 진하게
        let flash = self.flash.as_ref().filter(|(fp, _)| *fp == p).map(|(_, m)| m.clone());
        let count_hot = flash.is_some();
        let count_text = flash.unwrap_or_else(|| count_text(tab));
        let cw = g.measure(&count_text, Font::Small).min(gm.head.w() * 0.6);
        let count_rect = R::new(gm.head.r - 8.0 - cw, gm.head.t + 8.0, gm.head.r - 8.0, gm.head.t + 36.0);
        let editing = self.editing == Some(p);
        let dir = tab.loc.dir();
        let scene = PaneScene {
            r,
            tabs: &pane.tabs,
            current: pane.active,
            sort: self.sort,
            cols: DEFAULT_COLS,
            view: pane.view,
            active,
            multi: self.layout.count() > 1,
            keyboard_here: active,
            hover: pane_hover(self.hover, p),
            chips: tabs,
            add,
            count_text,
            count_rect,
            count_hot,
            filter: None,
            starred: dir.map(|d| self.favs.find(d).is_some()),
            root: None,
            address: editing,
            // 창에서는 입력 상자가 글자를 그린다. 그림 시험에서만 직접 그린다
            address_text: dir.filter(|_| editing && self.standalone),
            renaming: false,
            dragged_tab: None,
            tab_drop: None,
            drop_on_pane: false,
            drop_row: None,
            marquee: None,
            scroll_drag: false,
            hint: list_hint(tab),
            net_button: None,
            loading: None,
            short_dates: true,
            now: listing::now_filetime(),
        };
        paint_pane_head(g, t, &scene);
        paint_pane_list(g, &mut MacArt { kinds: &self.kinds }, t, &scene);
        paint_pane_border(g, t, &scene);
    }
}

/// 칸 p 안에서 마우스가 올라간 곳 (공통 칸 그리기에 넘긴다)
fn pane_hover(h: Hit, p: usize) -> PaneHover {
    match h {
        Hit::Tab(q, i) if q == p => PaneHover::Tab(i),
        Hit::TabClose(q, i) if q == p => PaneHover::TabClose(i),
        Hit::TabAdd(q) if q == p => PaneHover::TabAdd,
        Hit::Star(q) if q == p => PaneHover::Star,
        Hit::Crumb(q, i) if q == p => PaneHover::Crumb(i),
        Hit::CrumbEmpty(q) if q == p => PaneHover::CrumbEmpty,
        Hit::Row(q, vi) if q == p => PaneHover::Row(vi),
        _ => PaneHover::None,
    }
}

/// 빈 목록 안내 (Windows와 같은 글, 다시 시도는 ⌘R)
fn list_hint(tab: &Tab) -> Option<String> {
    match &tab.state {
        LoadState::Loading if tab.entries.is_empty() => Some(format!("불러오는 중…\n{}", tab.loc.title())),
        LoadState::Error(msg) => Some(format!("폴더를 열 수 없어요\n{msg}\n⌘R을 누르면 다시 시도해요")),
        LoadState::Ready if tab.entries.is_empty() => Some("빈 폴더예요".to_string()),
        LoadState::Ready if tab.view.is_empty() => Some("숨김 항목만 있어요".to_string()),
        _ => None,
    }
}

/// 칸 그리기의 맥 갈고리: Finder 아이콘과 '종류' 이름 (썸네일은 아직 없다)
struct MacArt<'a> {
    kinds: &'a RefCell<HashMap<String, String>>,
}

impl EntryArt<Gfx> for MacArt<'_> {
    fn icon(&mut self, g: &mut Gfx, tab: &Tab, e: &Entry, r: R) {
        if let Some(path) = tab.path_of(e) {
            g.icon_for_path(&path, r);
        }
    }

    fn thumbnail(&mut self, _: &mut Gfx, _: &Tab, _: &Entry, _: R, _: f32, _: f32) -> bool {
        false
    }

    /// 확장자가 같으면 같은 종류로 보고 한 번만 묻는다 (확장자 없는 파일은 파일마다 다를 수 있어 매번 묻는다)
    fn type_name(&mut self, tab: &Tab, e: &Entry) -> String {
        let name = e.name_string();
        let ext = name.rsplit_once('.').map(|(_, x)| x.to_lowercase()).filter(|x| !x.is_empty());
        let key = ext.as_ref().map(|x| if e.kind == Kind::Dir { format!("/{x}") } else { x.clone() });
        let key = key.or_else(|| (e.kind == Kind::Dir).then(|| "/".to_string()));
        if let Some(k) = &key
            && let Some(v) = self.kinds.borrow().get(k)
        {
            return v.clone();
        }
        let v = tab.path_of(e).and_then(|p| Gfx::type_description(&p)).unwrap_or_default();
        if let Some(k) = key {
            self.kinds.borrow_mut().insert(k, v.clone());
        }
        v
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

/// 탭의 스크롤을 내용 길이 안으로 (보기 방식의 칸 배치에 맞춰)
fn clamp_tab(tab: &mut Tab, list: R, view: ViewMode) {
    let grid = Grid::of(list, view);
    let max = (grid.content(tab.view.len()) - grid.page(list)).max(0.0);
    tab.scroll = tab.scroll.clamp(0.0, max);
}

/// 커서 항목이 보이게 탭을 스크롤한다
fn reveal(tab: &mut Tab, list: R, view: ViewMode) {
    let grid = Grid::of(list, view);
    // 스크롤 0 기준 위치 (스크롤 방향으로)
    let it = grid.item(tab.cursor, 0.0);
    let (top, bottom) = if grid.horiz { (it.l - list.l, it.r - list.l + 4.0) } else { (it.t - list.t, it.b - list.t) };
    let page = grid.page(list);
    if top < tab.scroll {
        tab.scroll = top;
    } else if bottom > tab.scroll + page {
        tab.scroll = bottom - page;
    }
    clamp_tab(tab, list, view);
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
        let gm = geom(a.pane_rects()[0].1);
        let head = gm.head;
        let (trs, add) = a.chips(&g, 0, &gm);
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
        a.resize(3600.0, 800.0);
        let gm = geom(a.pane_rects()[1].1);
        let last = a.crumbs(&g, gm.crumbs, &home).last().unwrap().0;
        let (x, y) = (last.r + CHEV_W + 8.0, gm.crumb.t + 10.0);
        assert!(x < gm.crumb.r, "an empty spot after the last crumb");
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
        // 늦게 끝난 확인은 버린다: 경로 입력 두 번이면 나중 것이, 확인 중에 다른 곳으로 옮기면 그 곳이 이긴다
        a.begin_address(1);
        a.commit_address(&util::join(&home, "다운로드"));
        let first = a.take_probes().pop().unwrap();
        a.begin_address(1);
        a.commit_address(&home);
        let second = a.take_probes().pop().unwrap();
        a.on_probed(&second, resolve_input(&second.text, &home));
        a.on_probed(&first, resolve_input(&first.text, &home));
        a.settle();
        assert_eq!(a.pane(1).tab().loc, Location::Dir(home.clone()), "the later address wins");
        a.begin_address(1);
        a.commit_address(&util::join(&home, "다운로드"));
        let slow = a.take_probes().pop().unwrap();
        a.navigate(1, Location::Dir(util::join(&home, "업무")), None);
        a.on_probed(&slow, resolve_input(&slow.text, &home));
        a.settle();
        assert_eq!(a.pane(1).tab().loc, Location::Dir(util::join(&home, "업무")), "navigation after the address wins");
        // 대소문자만 다른 경로는 같은 폴더로 보지 않고 그 경로로 간다 (대소문자를 가리는 볼륨에서는 다른 폴더)
        let work = util::join(&home, "업무");
        let other_case: String = work.chars().map(|c| if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c }).collect();
        if other_case != work && resolve_input(&other_case, &home).is_ok() {
            a.begin_address(1);
            a.commit_address(&other_case);
            a.settle();
            assert_eq!(a.pane(1).tab().loc, Location::Dir(other_case.clone()));
            assert_eq!(a.pane(1).tab().back.last(), Some(&Location::Dir(work)));
        }
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
        s.panes[2].tabs = vec![util::join(&home, "지운 폴더")];
        let (mut b, _g) = app(&home);
        b.restore(&s, &home);
        b.settle();
        assert_eq!((b.layout(), b.active(), b.side, b.show_hidden, b.split), (Layout::Three, 2, Side::Places, true, (0.3, 0.6)));
        assert_eq!(b.pane(0).tab().loc, Location::Dir(util::join(&home, "업무")));
        assert_eq!(b.pane(1).tabs.len(), 2);
        assert_eq!(b.pane(1).active, 1);
        assert_eq!(b.pane(1).tab().loc, Location::Dir(util::join(&home, "다운로드")));
        assert_eq!(b.pane(2).tab().loc, Location::Dir(home.clone()), "a missing folder opens home after its listing fails");
        assert!(b.pane(2).tab().back.is_empty(), "the fallback leaves no history");
        // 첫 읽기 전에 다른 곳으로 옮겼으면 그곳의 오류는 홈으로 바꾸지 않고 보인다
        let mut s2 = s.clone();
        s2.panes[0].tabs = vec![util::join(&home, "지운 폴더")];
        let (mut c, _g) = app(&home);
        c.restore(&s2, &home);
        let gone = util::join(&home, "없는 곳");
        c.navigate(0, Location::Dir(gone.clone()), None);
        c.settle();
        assert_eq!(c.pane(0).tab().loc, Location::Dir(gone));
        assert!(matches!(c.pane(0).tab().state, LoadState::Error(_)));
        assert!(names(&b, 0).contains(&"예산.csv".to_string()), "visible panes are loaded");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn crumbs_walk_up_and_select_the_child() {
        let (root, home) = demo();
        let (mut a, g) = app(&util::join(&util::join(&home, "업무"), "2026 4분기"));
        let gm = geom(a.pane_rects()[0].1);
        let cs = a.crumbs(&g, gm.crumbs, a.pane(0).tab().loc.dir().unwrap());
        let i = cs.iter().position(|(_, _, l)| l == "업무").unwrap();
        let (cr, _, _) = &cs[i];
        a.mouse_down(&g, cr.l + 4.0, cr.t + 8.0, Mods::default(), 1);
        a.settle();
        assert_eq!(a.pane(0).tab().cursor_entry().unwrap().name_string(), "2026 4분기");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn toolbar_buttons_work_on_the_active_pane_and_ask_for_menus() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let m = a.toolbar();
        let mid = |r: R| ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0);
        // 뒤로·앞으로는 지금 칸의 것
        let (x, y) = mid(m.back);
        assert_eq!(a.hit(&g, x, y), Hit::Back(a.active()));
        // 칸 나누기 단추: 메뉴를 열어 달라고 하고, 단추 자리를 알려 준다
        let (x, y) = mid(m.layout);
        assert_eq!(a.hit(&g, x, y), Hit::LayoutPick);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!(a.take_menu().map(|(k, r)| (k, r.l)), Some((MenuKind::Layout, m.layout.l)));
        assert!(a.take_menu().is_none(), "the request is handed over once");
        assert_eq!(a.menu_items(MenuKind::Layout).iter().filter(|(_, on)| *on).count(), 1);
        a.pick_menu(MenuKind::Layout, 4);
        assert_eq!(a.layout(), Layout::Four);
        // 보기 방식 메뉴: 여섯 가지와 맨 아래 숨김 항목 보기
        let (x, y) = mid(a.toolbar().view);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!(a.take_menu().map(|(k, _)| k), Some(MenuKind::View));
        assert_eq!(a.menu_items(MenuKind::View).len(), ViewMode::ALL.len() + 1);
        let p = a.active();
        a.pick_menu(MenuKind::View, 2);
        assert_eq!(a.pane(p).view, ViewMode::Medium);
        let hidden = a.menu_items(MenuKind::View).last().unwrap().1;
        a.pick_menu(MenuKind::View, ViewMode::ALL.len());
        assert_ne!(a.menu_items(MenuKind::View).last().unwrap().1, hidden);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn icon_view_moves_along_the_grid_and_is_saved_in_the_session() {
        let (root, home) = demo();
        let (mut a, _g) = app(&util::join(&home, "업무"));
        a.set_view(0, ViewMode::Medium);
        let n = a.pane(0).tab().view.len();
        assert_eq!(n, 3);
        // 아이콘 보기: 좌우가 한 칸. 한 줄에 다 들어가 있으면 위아래로는 움직이지 않는다
        a.key(Key::Home, Mods::default());
        a.key(Key::Right, Mods::default());
        assert_eq!(a.pane(0).tab().cursor, 1);
        a.key(Key::Down, Mods::default());
        assert_eq!(a.pane(0).tab().cursor, 1);
        a.key(Key::End, Mods::default());
        assert_eq!(a.pane(0).tab().cursor, n - 1);
        // 목록 보기: 위아래가 한 칸
        a.set_view(0, ViewMode::List);
        a.key(Key::Up, Mods::default());
        assert_eq!(a.pane(0).tab().cursor, n - 2);
        // 보기 방식은 세션에 남는다
        let s = a.to_session();
        let (mut b, _g) = app(&home);
        b.restore(&s, &home);
        assert_eq!(b.pane(0).view, ViewMode::List);
        std::fs::remove_dir_all(root).unwrap();
    }
}
