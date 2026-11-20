//! 검색 화면 (⌘F, 도구 막대의 돋보기): 도구 막대에서 입력 칸이 늘어나고, 결과는 사이드바 자리에 보인다 (Windows 검색과 같은 자리).
//! 찾는 것은 자체 색인(search.rs)이 한다. 처음에는 꺼져 있다: 켜면 홈 폴더와 응용 프로그램 폴더의 이름을 훑어 이 맥의 메모리에만 둔다.
//! 입력할 때마다 뒷줄에서 찾고(창 쪽이 take_find_asks로 가져가 돌린다) 결과를 on_found로 받는다. 훑는 동안과 디스크가 바뀐 뒤에는
//! 색인이 알려 줄 때(index_changed) 같은 검색어로 다시 찾는다.
//! 결과를 둔 채 칸에서 일할 수 있다: 칸을 누르면 키보드만 칸으로 가고 결과는 남는다. Esc나 닫기 단추로 닫는다

use super::*;
use crate::search::{self, Finder, Found, Options, Search, Setting, Status};
use sakadu::query;
use sakadu::ui::toolbar::search_field_parts;
use std::time::Duration;

/// 결과 수 (Windows와 같다)
pub const FIND_LIMIT: usize = 200;
/// 늘어난 입력 칸의 폭 (창이 좁으면 공통 도구 막대가 줄인다)
pub(super) const FIELD_W: f32 = 240.0;
/// 범위 고르기 판, 결과 한 줄, 아래 한 줄의 높이
const SCOPE_H: f32 = 28.0;
const FIND_ROW_H: f32 = 28.0;
const FOOT_H: f32 = 28.0;
/// '검색 켜기' 카드의 단추와 고르기 줄의 높이
const BUTTON_H: f32 = 28.0;
/// 시스템 설정의 '파일 및 폴더' (접근을 거절한 폴더를 다시 허용하는 곳)
const PRIVACY_FILES: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders";
/// 검색 안내 글의 줄 사이
const LINE_H: f32 = 20.0;
/// 체크 표시 (Segoe 번호. 맥은 SF Symbols로 그린다)
const G_CHECK: u16 = 0xE73E;

/// 찾아 달라는 요청: 창 쪽이 뒷줄에서 finder로 찾아 on_found로 돌려준다
pub struct FindAsk {
    pub seq: u64,
    /// 이 요청의 검색어와 범위 (query::result_key)
    key: String,
    pub finder: Finder,
    pub text: String,
    pub scope: Option<String>,
}

impl FindAsk {
    /// 찾는다 (뒷줄에서 부른다)
    pub fn run(&self) -> Found {
        self.finder.find(&self.text, self.scope.as_deref(), FIND_LIMIT)
    }
}

#[derive(Default)]
pub struct Find {
    /// 켰는지, iCloud Drive도 찾는지 (설정 폴더의 search.txt)
    pub setting: Setting,
    /// 검색이 열려 있다 (사이드바 자리에 결과)
    open: bool,
    /// 키보드가 입력 칸에 있다 (칸을 누르면 결과를 둔 채 칸으로 간다)
    focus: bool,
    /// 입력 칸의 글 (창 쪽 입력 칸이 바뀔 때마다 알려 준다)
    text: String,
    /// 범위: 지금 폴더(true) 또는 전체
    here: bool,
    /// 결과: (폴더 여부, 경로. 한글은 합친 꼴)
    results: Vec<(bool, String)>,
    total: usize,
    /// 보이는 결과의 검색어와 범위, 마지막으로 찾으라고 보낸 검색어와 범위
    shown: Option<String>,
    asked: Option<String>,
    /// 마지막으로 낸 요청의 번호 (그보다 먼저 낸 요청의 결과는 버린다)
    seq: u64,
    asks: Vec<FindAsk>,
    /// 고른 결과 줄, 결과 목록의 스크롤
    sel: usize,
    scroll: f32,
    /// 결과보다 먼저 누른 Return (true면 ⌘Return)과 그때의 검색어·범위 열쇠. 그 열쇠의 결과가 오면 한 번 처리하고,
    /// 다른 열쇠의 결과가 오면 버린다 (Return 뒤에 범위를 바꾸거나 칸이 다른 폴더로 가면 새 결과를 저절로 열지 않는다)
    enter: Option<(bool, String)>,
    index: Option<Search>,
    /// 색인을 시작해 달라는 요청 (창 쪽이 시작해 set_index로 넘긴다: 색인이 바뀔 때 다시 그려 달라는 알림을 창 쪽이 건다)
    start: Option<Options>,
    /// 색인의 지금 상태 (훑는 중인지, 개수, 읽지 못한 폴더)
    status: Status,
    /// 맥이 접근을 묻는 폴더 (거절했으면 아래 줄에 알린다)
    protected: Vec<String>,
    /// '지금 폴더'가 훑는 범위 밖이다 (마지막 결과를 찾을 때)
    outside: bool,
    /// 시험·그림·창 시험: 훑는 범위를 이것으로 (홈 폴더 대신. 러너의 홈 폴더를 훑으면 권한 창 때문에 멈출 수 있다)
    pub roots: Option<Vec<String>>,
}

/// 검색 패널에서 누를 수 있는 것
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FindHit {
    /// 범위: 지금 폴더(true) 또는 전체
    Scope(bool),
    Row(usize),
    /// '검색 켜기' 카드의 단추와 iCloud Drive 고르기
    Enable,
    ICloud,
    /// 아래 줄의 '설정에서 허용' (시스템 설정을 연다)
    Allow,
    /// 패널의 그 밖의 곳
    Panel,
}

/// '검색 켜기' 카드의 자리: 안내 글(줄마다), iCloud Drive 고르기 줄, 켜기 단추
struct Intro {
    lines: Vec<(String, R)>,
    icloud: R,
    enable: R,
}

impl App {
    // ---------- 열고 닫기 ----------

    /// 검색이 열려 있는가 (사이드바 자리에 결과)
    pub fn find_shown(&self) -> bool {
        self.find.open
    }

    /// ⌘F·돋보기: 검색을 열고 키보드를 입력 칸에 둔다 (이미 열려 있으면 키보드만). 지난 검색어는 그대로 둔다
    pub fn open_find(&mut self) {
        self.edit = Editing::None;
        if !self.find.open {
            let f = &mut self.find;
            f.open = true;
            f.results.clear();
            f.total = 0;
            f.shown = None;
            f.asked = None;
            f.sel = 0;
            f.scroll = 0.0;
            f.enter = None;
        }
        self.find.focus = true;
    }

    /// 검색을 닫는다 (Esc, 닫기 단추). 사이드바가 돌아온다. 검색어는 다음에 열 때 그대로 있다
    pub fn close_find(&mut self) {
        let f = &mut self.find;
        f.open = false;
        f.focus = false;
        f.enter = None;
        f.results.clear();
        f.shown = None;
        f.asked = None;
    }

    /// 입력 칸이 키보드를 잃었다 (칸이나 다른 곳을 눌렀다): 결과는 그대로 둔다
    pub fn find_blur(&mut self) {
        self.find.focus = false;
    }

    // ---------- 입력 칸 (창 쪽의 두 번째 글 상자) ----------

    /// 입력 칸 자리: 도구 막대에서 늘어난 칸의 글자 자리. 닫혀 있으면 None
    pub fn find_field_rect(&self) -> Option<R> {
        field_parts(&self.toolbar()).map(|p| p.0)
    }

    /// 입력 칸이 키보드를 가져야 하는가 (다른 입력 상자로 경로·이름을 고치는 동안은 아니다)
    pub fn find_wants_keys(&self) -> bool {
        self.find.open && self.find.focus && self.edit == Editing::None
    }

    /// 입력 칸에 처음 넣을 글 (열 때마다 지난 검색어)
    pub fn find_text(&self) -> &str {
        &self.find.text
    }

    /// 입력 칸의 글이 바뀌었다 (창 쪽이 글자마다 알려 준다. 한글은 조합 중인 글자까지)
    pub fn find_typed(&mut self, text: &str) {
        if self.find.text == text {
            return;
        }
        self.find.text = text.to_string();
        self.find.enter = None;
        self.find_ask();
    }

    /// 입력 칸에서 누른 키 (창 쪽 입력 칸이 넘겨준다): ↑↓ 결과 고르기, Return 열기, ⌘Return 칸에서 보기, Esc 닫기. 받았으면 true
    pub fn find_key(&mut self, key: Key, cmd: bool) -> bool {
        match key {
            Key::Up | Key::Down | Key::PageUp | Key::PageDown => self.find_move(key),
            Key::Return if self.find.text.trim().is_empty() || !self.find.setting.on => {}
            // 결과가 아직 이 검색어의 것이 아니면, 그 결과가 올 때 연다 (먼저 떠 있던 다른 검색어의 결과를 열지 않는다)
            Key::Return if !self.find_current() => self.find.enter = Some((cmd, self.find_now())),
            Key::Return => self.open_found(self.find.sel, cmd),
            Key::Esc => self.close_find(),
            _ => return false,
        }
        true
    }

    /// 지금 고른 결과 줄을 위아래로
    fn find_move(&mut self, key: Key) {
        let n = self.find.results.len();
        if n == 0 {
            return;
        }
        let page = (self.find_list().h() / FIND_ROW_H).floor().max(1.0) as usize;
        let s = self.find.sel.min(n - 1);
        self.find.sel = match key {
            Key::Up => s.saturating_sub(1),
            Key::Down => (s + 1).min(n - 1),
            Key::PageUp => s.saturating_sub(page),
            Key::PageDown => (s + page).min(n - 1),
            _ => s,
        };
        self.find_reveal();
    }

    // ---------- 찾기 ----------

    /// '지금 폴더'로 좁혀 찾을 때 그 폴더 (지금 칸이 폴더를 보고 있을 때만)
    fn find_scope(&self) -> Option<String> {
        if self.find.here { self.panes[self.active].tab().loc.dir().map(str::to_string) } else { None }
    }

    /// 지금 검색어와 범위의 열쇠 (보이는 결과가 지금 것인지 가른다)
    fn find_now(&self) -> String {
        query::result_key(self.find.text.trim(), self.find_scope().as_deref())
    }

    /// 보이는 결과가 지금 검색어와 범위의 것인가
    fn find_current(&self) -> bool {
        self.find.shown.as_deref() == Some(self.find_now().as_str())
    }

    /// 지금 검색어와 범위로 찾아 달라고 한다. 검색어가 비었으면 결과를 비운다. 색인이 아직 없으면 생긴 뒤에 찾는다 (find_sync)
    fn find_ask(&mut self) {
        let key = self.find_now();
        let text = self.find.text.trim().to_string();
        if text.is_empty() {
            let f = &mut self.find;
            // 아직 찾는 중인 앞 검색어의 결과가 늦게 와서 빈 칸 아래를 다시 채우지 않게 그 요청을 버린다
            f.seq += 1;
            f.results.clear();
            f.total = 0;
            f.sel = 0;
            f.scroll = 0.0;
            f.shown = Some(key.clone());
            f.asked = Some(key);
            return;
        }
        let Some(index) = &self.find.index else { return };
        let finder = index.finder();
        let scope = self.find_scope();
        self.find.seq += 1;
        self.find.asks.push(FindAsk { seq: self.find.seq, key: key.clone(), finder, text, scope });
        self.find.asked = Some(key);
    }

    /// 창 쪽이 가져가 뒷줄에서 찾는다 (FindAsk::run → on_found)
    pub fn take_find_asks(&mut self) -> Vec<FindAsk> {
        std::mem::take(&mut self.find.asks)
    }

    /// 찾은 결과. 그 뒤에 다시 찾으라고 한 것이 있으면 버린다 (늦게 끝난 옛 검색어의 결과가 덮지 않게)
    pub fn on_found(&mut self, ask: &FindAsk, found: Found) {
        if ask.seq != self.find.seq || !self.find.open {
            return;
        }
        // 같은 검색어를 다시 찾은 것(훑는 중, 디스크가 바뀐 뒤)이면 고른 줄과 스크롤을 그 항목에 둔다
        let same = self.find.shown.as_deref() == Some(ask.key.as_str());
        let keep = if same { self.find.results.get(self.find.sel).map(|r| r.1.clone()) } else { None };
        let f = &mut self.find;
        f.results = found.hits;
        f.total = found.total;
        f.shown = Some(ask.key.clone());
        f.outside = ask.scope.as_deref().is_some_and(|d| !ask.finder.covers(d));
        let last = f.results.len().saturating_sub(1);
        f.sel = match keep {
            Some(p) => f.results.iter().position(|r| r.1 == p).unwrap_or(f.sel.min(last)),
            None if same => f.sel.min(last),
            None => 0,
        };
        if !same {
            f.scroll = 0.0;
        }
        self.find_clamp_scroll();
        if let Some((reveal, key)) = self.find.enter.take()
            && key == ask.key
            && !self.find.results.is_empty()
        {
            self.open_found(self.find.sel, reveal);
        }
    }

    /// 창 쪽이 입력과 뒷줄의 결과를 처리한 뒤마다 부른다: 색인이 막 생겼거나 범위('지금 폴더')의 폴더가 바뀌었으면 다시 찾는다
    pub fn find_sync(&mut self) {
        if self.find.open && self.find.index.is_some() && self.find.asked.as_deref() != Some(self.find_now().as_str()) {
            self.find_ask();
        }
    }

    /// 결과 i를 연다: 폴더는 지금 칸에서 그 폴더로, 파일은 macOS '열기'로.
    /// reveal(⌘Return)이면 지금 칸에서 그 위 폴더를 열고 그 항목을 골라 둔다. 칸에서 열었으면 키보드도 칸으로 간다
    fn open_found(&mut self, i: usize, reveal: bool) {
        let Some((is_dir, path)) = self.find.results.get(i).cloned() else { return };
        let p = self.active;
        if reveal {
            let Some(parent) = util::parent(&path) else { return };
            self.navigate(p, Location::Dir(parent), Some(util::file_name(&path).to_string()));
            self.find.focus = false;
        } else if is_dir {
            self.navigate(p, Location::Dir(path), None);
            self.find.focus = false;
        } else {
            self.open_files.push(path);
        }
    }

    // ---------- 색인 ----------

    /// 저장한 설정을 읽는다 (보통 실행). 켜 두었으면 바로 훑기 시작한다 (색인은 저장하지 않아 켤 때마다 훑는다)
    pub fn load_find_setting(&mut self) {
        self.find.setting = Setting::load();
        if self.find.setting.on {
            self.request_index();
        }
    }

    fn save_find_setting(&self) {
        if self.persist
            && let Err(e) = self.find.setting.save()
        {
            sakadu::diag::log(|| format!("search setting: {e}"));
        }
    }

    /// '검색 켜기': 설정에 적고 훑기를 시작한다
    pub fn enable_find(&mut self) {
        self.find.setting.on = true;
        self.save_find_setting();
        self.request_index();
        self.find.focus = true;
    }

    fn request_index(&mut self) {
        if self.find.index.is_none() && self.find.start.is_none() {
            let opts = self.index_options();
            self.find.protected = opts.protected.clone();
            self.find.start = Some(opts);
        }
    }

    /// 훑는 범위: 홈 폴더와 응용 프로그램 폴더 둘 (iCloud Drive는 고른 사람만). 시험에서는 정해 준 폴더만
    fn index_options(&self) -> Options {
        let home = std::env::var("HOME").map(|h| util::normalize(&h)).unwrap_or_else(|_| self.home.clone());
        let (roots, protected) = match &self.find.roots {
            Some(r) => (r.clone(), Vec::new()),
            None => search::default_scope(&home, self.find.setting.icloud),
        };
        Options { roots, home, protected, threads: 3, packages_are_files: true, max_entries: usize::MAX, watch: true }
    }

    /// 색인을 시작해 달라는 요청 (창 쪽이 가져가 시작하고 set_index로 넘긴다)
    pub fn take_index_start(&mut self) -> Option<Options> {
        self.find.start.take()
    }

    pub fn set_index(&mut self, index: Search) {
        self.find.status = index.status();
        self.find.index = Some(index);
        // 색인이 생기기 전에 친 검색어가 있으면 찾는다 (find_sync)
        self.find.asked = None;
    }

    /// 색인이 바뀌었다 (훑는 중이면 0.25초마다, 그 뒤로는 디스크가 바뀔 때. 창 쪽이 메인 줄에서 부른다): 같은 검색어로 다시 찾는다
    pub fn index_changed(&mut self) {
        let Some(index) = &self.find.index else { return };
        self.find.status = index.status();
        if self.find.open && !self.find.text.trim().is_empty() {
            self.find_ask();
        }
    }

    /// 시험·그림: 시작해 달라는 색인을 여기서 시작하고 다 훑을 때까지 기다린 뒤, 쌓인 검색을 바로 찾는다 (settle에서 부른다)
    pub(super) fn settle_find(&mut self) {
        if let Some(opts) = self.find.start.take() {
            let index = Search::start(opts, || {});
            index.settle(Duration::from_secs(30));
            self.set_index(index);
        }
        if let Some(index) = &self.find.index {
            index.settle(Duration::from_secs(30));
            self.find.status = index.status();
        }
        self.find_sync();
        for ask in self.take_find_asks() {
            let found = ask.run();
            self.on_found(&ask, found);
        }
    }

    // ---------- 자리 ----------

    /// 검색 패널 자리: 레일 옆, 사이드바의 공간 이름 줄부터 아래 끝까지
    fn find_panel(&self) -> R {
        let t = self.side_top();
        R::new(BAR_W, t, SIDE_W, (self.h - MARGIN).max(t))
    }

    /// 범위 고르기 판 (전체 · 지금 폴더)
    fn find_scope_track(&self) -> R {
        let p = self.find_panel();
        R::new(p.l + 8.0, p.t + 2.0, p.r - 8.0, p.t + 2.0 + SCOPE_H)
    }

    fn find_scope_rect(&self, here: bool) -> R {
        let inner = self.find_scope_track().inset(2.0, 2.0);
        let w = inner.w() / 2.0;
        let l = inner.l + if here { w } else { 0.0 };
        R::new(l, inner.t, l + w, inner.b)
    }

    /// 결과 목록 자리
    fn find_list(&self) -> R {
        let p = self.find_panel();
        let t = p.t + SIDE_HEADER_H + 4.0;
        R::new(p.l + 8.0, t, p.r - 8.0, (p.b - FOOT_H).max(t))
    }

    /// 아래 한 줄: 훑는 중인지, 읽지 못한 폴더, 아니면 고른 결과의 위치
    fn find_foot(&self) -> R {
        let p = self.find_panel();
        R::new(p.l + 16.0, p.b - FOOT_H, p.r - 12.0, p.b)
    }

    fn find_clamp_scroll(&mut self) {
        let max = (self.find.results.len() as f32 * FIND_ROW_H - self.find_list().h()).max(0.0);
        self.find.scroll = self.find.scroll.clamp(0.0, max);
    }

    /// 고른 결과 줄이 보이게 스크롤한다
    fn find_reveal(&mut self) {
        let h = self.find_list().h();
        let top = self.find.sel as f32 * FIND_ROW_H;
        if top < self.find.scroll {
            self.find.scroll = top;
        } else if top + FIND_ROW_H > self.find.scroll + h {
            self.find.scroll = top + FIND_ROW_H - h;
        }
        self.find_clamp_scroll();
    }

    /// 마우스 아래가 검색 패널이면 결과 목록을 스크롤한다. 그랬으면 true
    pub(super) fn find_scroll(&mut self, x: f32, dy: f32) -> bool {
        if !self.find.open || !(BAR_W..SIDE_W).contains(&x) {
            return false;
        }
        self.find.scroll += dy;
        self.find_clamp_scroll();
        true
    }

    /// '검색 켜기' 카드의 자리. 안내 글은 패널 폭에 맞춰 줄을 나눈다
    fn find_intro(&self, g: &Gfx) -> Intro {
        let list = self.find_list();
        let text = [
            "홈 폴더와 응용 프로그램의 이름을 읽어 이 맥의 메모리에만 둬요",
            "파일 내용은 읽지 않아요. 데스크탑·문서·다운로드는 맥이 한 번씩 물어요",
        ];
        let w = list.w() - 16.0;
        let mut y = list.t + 12.0;
        let mut lines = Vec::new();
        for (k, para) in text.iter().enumerate() {
            if k > 0 {
                y += 8.0;
            }
            for line in wrap(g, para, Font::Small, w) {
                lines.push((line, R::new(list.l + 8.0, y, list.r - 8.0, y + LINE_H)));
                y += LINE_H;
            }
        }
        let icloud = R::new(list.l + 4.0, y + 12.0, list.r - 4.0, y + 12.0 + BUTTON_H);
        let cx = (list.l + list.r) / 2.0;
        let enable = R::new(cx - 64.0, icloud.b + 12.0, cx + 64.0, icloud.b + 12.0 + BUTTON_H);
        Intro { lines, icloud, enable }
    }

    /// 아래 줄의 '설정에서 허용' 자리 (맥이 접근을 묻는 폴더를 거절했을 때만)
    fn find_allow(&self, g: &Gfx) -> Option<R> {
        if !self.find_denied() || !self.find.status.ready {
            return None;
        }
        let foot = self.find_foot();
        let w = g.measure(ALLOW, Font::Small) + 4.0;
        Some(R::new(foot.r - w, foot.t, foot.r, foot.b))
    }

    /// 맥이 접근을 묻는 폴더 가운데 읽지 못한 것이 있다 (거절했거나 아직 답하지 않았다)
    fn find_denied(&self) -> bool {
        self.find.status.denied.iter().any(|d| self.find.protected.iter().any(|p| util::eq_path(p, d)))
    }

    // ---------- 누르기 ----------

    /// 검색 패널 안에서 누를 수 있는 것 (패널 자리 안이라고 확인한 뒤 부른다)
    pub(super) fn find_hit(&self, g: &Gfx, x: f32, y: f32) -> FindHit {
        if !self.find.setting.on {
            let intro = self.find_intro(g);
            return if intro.enable.contains(x, y) {
                FindHit::Enable
            } else if intro.icloud.contains(x, y) {
                FindHit::ICloud
            } else {
                FindHit::Panel
            };
        }
        for here in [false, true] {
            if self.find_scope_rect(here).contains(x, y) {
                return FindHit::Scope(here);
            }
        }
        if self.find_allow(g).is_some_and(|r| r.contains(x, y)) {
            return FindHit::Allow;
        }
        let list = self.find_list();
        if list.contains(x, y) {
            let i = ((y - list.t + self.find.scroll) / FIND_ROW_H).floor();
            if i >= 0.0 && (i as usize) < self.find.results.len() {
                return FindHit::Row(i as usize);
            }
        }
        FindHit::Panel
    }

    /// 누름이 검색에 닿았으면 처리하고 true. 검색 밖을 누르면 키보드만 입력 칸에서 떠난다 (결과는 남는다)
    pub(super) fn find_press(&mut self, hit: Hit, clicks: usize) -> bool {
        match hit {
            Hit::FindOpen | Hit::FindField => self.open_find(),
            Hit::FindClose => self.close_find(),
            Hit::Find(h) => {
                self.edit = Editing::None;
                self.find.focus = true;
                match h {
                    FindHit::Scope(here) => {
                        // 칸이 폴더를 보고 있지 않으면 지금 폴더로 좁힐 수 없다
                        if !here || self.panes[self.active].tab().loc.dir().is_some() {
                            self.find.here = here;
                            self.find_ask();
                        }
                    }
                    FindHit::Row(i) => {
                        self.find.sel = i;
                        if clicks >= 2 {
                            self.open_found(i, false);
                        }
                    }
                    FindHit::Enable => self.enable_find(),
                    FindHit::ICloud => {
                        self.find.setting.icloud = !self.find.setting.icloud;
                        self.save_find_setting();
                    }
                    FindHit::Allow => self.open_url = Some(PRIVACY_FILES.to_string()),
                    FindHit::Panel => {}
                }
            }
            _ => {
                self.find.focus = false;
                return false;
            }
        }
        true
    }

    // ---------- 그리기 ----------

    /// 검색 패널 (사이드바 자리, 레일 옆): 범위 고르기 판, 결과 목록, 아래 한 줄. 꺼져 있으면 '검색 켜기' 카드
    pub(super) fn paint_find(&self, g: &Gfx, t: &Theme) {
        let list = self.find_list();
        let f = &self.find;
        let hover = match self.hover {
            Hit::Find(h) => Some(h),
            _ => None,
        };
        if !f.setting.on {
            return self.paint_find_intro(g, t, hover);
        }

        // 범위 고르기: 전체 · 지금 폴더 (칸에 폴더가 없으면 지금 폴더는 흐리게, 전체로 찾는다)
        let here_ok = self.panes[self.active].tab().loc.dir().is_some();
        let track = self.find_scope_track();
        g.fill_round(track, palette::R_CTRL, seg_track(t));
        for here in [false, true] {
            let r = self.find_scope_rect(here);
            let enabled = !here || here_ok;
            let on = if here_ok { here == f.here } else { !here };
            if on {
                seg_on(g, r, palette::R_CTRL - 2.0, t);
            } else if hover == Some(FindHit::Scope(here)) && enabled {
                g.fill_round(r, palette::R_CTRL - 2.0, hover_fill(t));
            }
            let fg = if on { t.text } else if enabled { t.dim } else { t.faint };
            g.text(if here { "지금 폴더" } else { "전체" }, r, Font::Hint, fg);
        }

        g.clip(list);
        let q = f.text.trim();
        let current = self.find_current();
        let message = if q.is_empty() {
            Some("파일·폴더 이름을 입력하세요".to_string())
        } else if !current || !f.results.is_empty() {
            None
        } else if f.outside {
            Some("이 폴더는 검색 범위 밖이에요".to_string())
        } else if !f.status.ready {
            Some(format!("아직 폴더를 훑는 중이에요\n다 훑으면 ‘{q}’ 결과를 다시 채워요"))
        } else if f.here && here_ok {
            Some(format!("지금 폴더 아래에는 ‘{q}’ 검색 결과가 없어요"))
        } else {
            Some(format!("‘{q}’ 검색 결과가 없어요"))
        };
        if let Some(m) = message {
            let mut y = list.t + 14.0;
            for para in m.lines() {
                for line in wrap(g, para, Font::Hint, list.w() - 20.0) {
                    g.text(&line, R::new(list.l + 10.0, y, list.r - 10.0, y + LINE_H), Font::Hint, t.faint);
                    y += LINE_H;
                }
            }
        }
        let first = (f.scroll / FIND_ROW_H).floor().max(0.0) as usize;
        let count = (list.h() / FIND_ROW_H).ceil() as usize + 1;
        let focus = self.find_wants_keys();
        for i in first..(first + count).min(f.results.len()) {
            let (_, path) = &f.results[i];
            let y = list.t + i as f32 * FIND_ROW_H - f.scroll;
            let row = R::new(list.l, y, list.r, y + FIND_ROW_H);
            let sel = i == f.sel;
            if sel {
                g.fill_round(row, R_ROW, if focus { t.sel } else { t.sel_inactive });
            } else if hover == Some(FindHit::Row(i)) {
                g.fill_round(row, R_ROW, hover_fill(t));
            }
            g.icon_for_path(path, R::new(row.l + 8.0, y + 6.0, row.l + 24.0, y + 22.0));
            // 이름, 그 뒤에 자리가 남으면 그 항목이 든 폴더의 이름을 흐리게 (전체 위치는 고른 줄 것만 아래 줄에)
            let name = util::file_name(path);
            let room = R::new(row.l + 32.0, y, row.r - 8.0, y + FIND_ROW_H);
            let (fg, dim) = if sel && focus && !t.high_contrast { (t.sel_text, alpha(t.sel_text, 0.7)) } else { (t.text, t.faint) };
            let nw = g.measure(name, Font::Normal).min(room.w());
            g.text(name, room, Font::Normal, fg);
            let folder = util::parent(path).map(|d| util::file_name(&d).to_string()).unwrap_or_default();
            let fl = room.l + nw + 8.0;
            if !folder.is_empty() && room.r - fl >= 40.0 {
                g.text(&folder, R::new(fl, y, room.r, y + FIND_ROW_H), Font::Small, dim);
            }
        }
        g.unclip();
        // 결과가 목록보다 길면 오른쪽 여백에 지금 위치 (보기만, 누르지는 않는다)
        let content = f.results.len() as f32 * FIND_ROW_H;
        if content > list.h() + 0.5 {
            let h = (list.h() * list.h() / content).max(24.0);
            let top = list.t + (f.scroll / (content - list.h())).clamp(0.0, 1.0) * (list.h() - h);
            g.fill_round(R::new(list.r + 2.0, top, list.r + 6.0, top + h), 2.0, t.thumb);
        }

        // 아래 한 줄: 훑는 중이면 그 개수, 맥이 접근을 묻는 폴더를 읽지 못했으면 허용하는 곳, 아니면 고른 결과의 위치
        let foot = self.find_foot();
        let p = self.find_panel();
        g.fill(R::new(p.l + 8.0, foot.t, p.r - 8.0, foot.t + 1.0), alpha(t.text, 0.08));
        if !f.status.ready {
            g.text(&format!("훑는 중 · {}개", util::grouped(f.status.entries as u64)), foot, Font::Small, t.faint);
        } else if let Some(allow) = self.find_allow(g) {
            g.text("읽지 못한 폴더가 있어요", R::new(foot.l, foot.t, allow.l - 4.0, foot.b), Font::Small, t.faint);
            g.text(ALLOW, allow, Font::Small, if hover == Some(FindHit::Allow) { t.text } else { t.chip_text });
        } else if let Some((_, path)) = f.results.get(f.sel).filter(|_| current) {
            g.text(&self.place_of(path), foot, Font::Path, t.dim);
        }
    }

    /// '검색 켜기' 카드: 무엇을 읽는지, iCloud Drive도 찾을지, 켜기 단추
    fn paint_find_intro(&self, g: &Gfx, t: &Theme, hover: Option<FindHit>) {
        let intro = self.find_intro(g);
        for (line, r) in &intro.lines {
            g.text(line, *r, Font::Small, t.dim);
        }
        // iCloud Drive 고르기: 네모 칸과 이름
        let r = intro.icloud;
        if hover == Some(FindHit::ICloud) {
            g.fill_round(r, R_SIDE_ROW, hover_fill(t));
        }
        let bx = R::new(r.l + 6.0, r.t + 6.0, r.l + 22.0, r.t + 22.0);
        if self.find.setting.icloud {
            g.fill_round(bx, 4.0, t.accent_soft);
            g.glyph(G_CHECK, bx, true, t.chip_text);
        } else {
            g.stroke_round(bx.inset(0.5, 0.5), 4.0, t.dim, 1.0);
        }
        g.text("iCloud Drive도 찾기", R::new(bx.r + 8.0, r.t, r.r - 4.0, r.b), Font::Normal, t.text);
        // 켜기 단추: 진하게 채우지 않고 옅은 강조 색 바탕에 강조 색 글자
        g.fill_round(intro.enable, palette::R_CTRL, t.accent_soft);
        if hover == Some(FindHit::Enable) {
            g.fill_round(intro.enable, palette::R_CTRL, alpha(t.accent, 0.14));
        }
        g.text("검색 켜기", intro.enable, Font::Hint, t.chip_text);
    }

    /// 결과가 든 곳을 짧게: 홈 아래는 ~, iCloud Drive 아래는 그 이름으로
    fn place_of(&self, path: &str) -> String {
        let dir = util::parent(path).unwrap_or_default();
        let home = std::env::var("HOME").map(|h| util::normalize(&h)).unwrap_or_else(|_| self.home.clone());
        let drive = search::icloud_drive(&home);
        let rest = |base: &str| dir.strip_prefix(base).filter(|r| r.is_empty() || r.starts_with('/')).map(str::to_string);
        if let Some(r) = rest(&drive) {
            format!("iCloud Drive{r}")
        } else if let Some(r) = rest(&home) {
            format!("~{r}")
        } else {
            dir
        }
    }

    /// 창 없이 그림만 그릴 때(그림 시험) 입력 칸의 글자도 그린다 (창에서는 창 쪽 입력 칸이 그린다)
    pub(super) fn paint_find_text(&self, g: &Gfx, t: &Theme) {
        if let (true, Some(r)) = (self.standalone, self.find_field_rect()) {
            g.text(&self.find.text, r, Font::Normal, t.text);
        }
    }

    /// 창 시험: 검색이 보이는 것 (열렸는지, 키보드가 입력 칸에 있는지, 결과의 이름, 고른 줄, 훑기를 마쳤는지)
    pub fn find_seen(&self) -> (bool, bool, Vec<String>, usize, bool) {
        let names = self.find.results.iter().map(|(_, p)| util::file_name(p).to_string()).collect();
        (self.find.open, self.find_wants_keys(), names, self.find.sel, self.find.index.is_some() && self.find.status.ready)
    }
}

/// 아래 줄의 시스템 설정을 여는 글
const ALLOW: &str = "설정에서 허용";

/// 열린 입력 칸의 글자 자리와 닫기 자리 (닫혀 있으면 None)
pub(super) fn field_parts(m: &ToolbarGeom) -> Option<(R, R)> {
    search_field_parts(m).map(|(_, text, close)| (text, close))
}

/// 글을 폭 w에 맞춰 줄로 나눈다 (띄어쓰기에서. 한 낱말이 폭보다 길면 그 줄은 그리기가 끝을 줄인다)
fn wrap(g: &Gfx, text: &str, f: Font, w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let next = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && g.measure(&next, f) > w {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 주간보고 (맥이 저장하는 나뉜 꼴)
    const WEEKLY_NFD: &str = "\u{110C}\u{116E}\u{1100}\u{1161}\u{11AB}\u{1107}\u{1169}\u{1100}\u{1169}";

    /// 시험 폴더 (그 안만 훑는다): 업무/2026 4분기/보고서.docx, 업무/회의록.md, 다운로드/사진.jpg, 다운로드/주간보고.txt(나뉜 꼴)
    fn demo(tag: &str) -> (std::path::PathBuf, String) {
        let root = std::env::temp_dir().join(format!("sakadu-find-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("업무/2026 4분기")).unwrap();
        std::fs::create_dir_all(root.join("다운로드")).unwrap();
        for f in ["업무/2026 4분기/보고서.docx", "업무/회의록.md", "다운로드/사진.jpg"] {
            std::fs::write(root.join(f), b"x").unwrap();
        }
        std::fs::write(root.join(format!("다운로드/{WEEKLY_NFD}.txt")), b"x").unwrap();
        let path = util::normalize(&std::fs::canonicalize(&root).unwrap().to_string_lossy());
        (root, path)
    }

    fn app(home: &str) -> (App, Gfx) {
        let mut a = App::new(1280.0, 800.0, Theme::LIGHT, home, Favorites::default(), Vec::new());
        a.set_find_roots(vec![home.to_string()]);
        a.settle();
        (a, Gfx::bitmap(8.0, 8.0, 1.0).unwrap())
    }

    fn mid(r: R) -> (f32, f32) {
        ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0)
    }

    fn click(a: &mut App, g: &Gfx, r: R) {
        let (x, y) = mid(r);
        a.mouse_down(g, x, y, Mods::default(), 1);
        a.mouse_up();
    }

    fn found(a: &App) -> Vec<String> {
        a.find_seen().2
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    /// 돋보기로 열면 처음에는 꺼져 있어 켜기 카드가 보이고, 켜면 정해 준 폴더만 훑어 찾는다
    #[test]
    fn search_is_off_until_turned_on_and_then_finds_names() {
        let (root, home) = demo("on");
        let (mut a, g) = app(&home);
        let (x, y) = mid(a.toolbar().search);
        assert_eq!(a.hit(&g, x, y), Hit::FindOpen);
        let search = a.toolbar().search;
        click(&mut a, &g, search);
        assert!(a.find_shown() && a.find_wants_keys());
        assert!(a.find_field_rect().is_some_and(|r| r.w() > 100.0), "the toolbar button grows into a field");
        a.find_typed("보고");
        a.settle();
        assert!(found(&a).is_empty() && a.find.index.is_none(), "nothing is read before search is turned on");
        // 켜기 카드: iCloud Drive 고르기는 껐다 켰다, 켜기 단추는 훑기를 시작한다 (시험에서는 설정을 저장하지 않는다)
        let intro = a.find_intro(&g);
        let (x, y) = mid(intro.icloud);
        assert_eq!(a.hit(&g, x, y), Hit::Find(FindHit::ICloud));
        click(&mut a, &g, intro.icloud);
        assert!(a.find.setting.icloud);
        click(&mut a, &g, intro.icloud);
        assert!(!a.find.setting.icloud);
        let (x, y) = mid(intro.enable);
        assert_eq!(a.hit(&g, x, y), Hit::Find(FindHit::Enable));
        click(&mut a, &g, intro.enable);
        a.settle();
        assert!(a.find.setting.on && a.find.status.ready);
        // 나뉜 꼴로 저장된 이름도 합친 꼴로 찾는다
        assert_eq!(sorted(found(&a)), ["보고서.docx", "주간보고.txt"]);
        // 또는(|)
        a.find_typed("회의록 | 사진");
        a.settle();
        assert_eq!(sorted(found(&a)), ["사진.jpg", "회의록.md"]);
        // 빈 검색어는 결과가 없다
        a.find_typed("  ");
        a.settle();
        assert!(found(&a).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// ↑↓로 고르고, Return은 폴더면 지금 칸에서 열고 파일이면 macOS '열기'로, ⌘Return은 그 위 폴더를 열어 그 항목을 골라 둔다.
    /// 결과보다 먼저 누른 Return은 그 결과가 오면 연다
    #[test]
    fn search_results_open_in_the_pane_or_with_the_default_app() {
        let (root, home) = demo("open");
        let (mut a, _g) = app(&home);
        a.open_find();
        a.enable_find();
        a.find_typed("보고");
        a.settle();
        let weekly = found(&a).iter().position(|n| n == "주간보고.txt").expect("found");
        a.find.sel = 0;
        for _ in 0..weekly {
            assert!(a.find_key(Key::Down, false));
        }
        assert_eq!(a.find.sel, weekly);
        // ⌘Return: 지금 칸에서 다운로드를 열고 그 파일을 골라 둔다 (디스크에는 나뉜 꼴의 이름). 키보드는 칸으로 가고 결과는 남는다
        assert!(a.find_key(Key::Return, true));
        a.settle();
        let tab = a.panes[a.active].tab();
        assert_eq!(tab.loc.dir(), Some(util::join(&home, "다운로드").as_str()));
        assert_eq!(tab.cursor_entry().map(Entry::name_string), Some(format!("{WEEKLY_NFD}.txt")));
        assert!(a.find_shown() && !a.find_wants_keys());
        // 폴더는 지금 칸에서 연다
        a.open_find();
        a.find_typed("2026");
        a.settle();
        assert_eq!(found(&a), ["2026 4분기"]);
        assert!(a.find_key(Key::Return, false));
        a.settle();
        assert_eq!(a.panes[a.active].tab().loc.dir(), Some(util::join(&home, "업무/2026 4분기").as_str()));
        // 파일은 macOS '열기'로. 결과가 오기 전에 누른 Return은 그 결과가 오면 연다
        a.open_find();
        a.find_typed("회의록");
        assert!(a.find_key(Key::Return, false));
        assert!(a.open_files.is_empty(), "waits for the results of the new query");
        a.settle();
        assert_eq!(a.open_files, [util::join(&home, "업무/회의록.md")]);
        // Esc는 닫는다: 사이드바가 돌아온다
        assert!(a.find_key(Key::Esc, false));
        assert!(!a.find_shown());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 찾는 중에 검색어를 지우면, 늦게 온 앞 검색어의 결과는 버린다 (빈 칸 아래에 결과가 남아 열리지 않게)
    #[test]
    fn search_drops_late_results_after_the_query_is_cleared() {
        let (root, home) = demo("clear");
        let (mut a, _g) = app(&home);
        a.open_find();
        a.enable_find();
        a.settle();
        a.find_typed("보고");
        let asks = a.take_find_asks();
        assert_eq!(asks.len(), 1);
        a.find_typed("");
        a.settle();
        for ask in asks {
            let late = ask.run();
            a.on_found(&ask, late);
        }
        assert!(found(&a).is_empty());
        a.find_sync();
        assert!(a.take_find_asks().is_empty() && found(&a).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 결과보다 먼저 누른 Return은 그때의 검색어와 범위의 결과만 연다: 그 뒤에 범위를 바꾸면 새 결과를 저절로 열지 않는다
    #[test]
    fn search_return_waits_only_for_its_own_results() {
        let (root, home) = demo("enter");
        let (mut a, g) = app(&home);
        a.open_find();
        a.enable_find();
        a.navigate(a.active, Location::Dir(util::join(&home, "업무")), None);
        a.settle();
        a.find_typed("회의록");
        assert!(a.find_key(Key::Return, false));
        assert!(a.open_files.is_empty());
        let here = a.find_scope_rect(true);
        click(&mut a, &g, here);
        a.settle();
        assert_eq!(found(&a), ["회의록.md"]);
        assert!(a.open_files.is_empty(), "the Return was for the other scope");
        // 칸이 다른 폴더로 가도 같다
        a.find_typed("보고");
        assert!(a.find_key(Key::Return, false));
        a.navigate(a.active, Location::Dir(util::join(&home, "업무/2026 4분기")), None);
        a.settle();
        assert_eq!(found(&a), ["보고서.docx"]);
        assert!(a.open_files.is_empty(), "the Return was for the folder before");
        // 그 결과에 다시 누르면 연다
        assert!(a.find_key(Key::Return, false));
        assert_eq!(a.open_files, [util::join(&home, "업무/2026 4분기/보고서.docx")]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// '지금 폴더'는 지금 칸의 폴더 아래만 찾는다. 훑는 범위 밖 폴더면 그렇다고 알린다
    #[test]
    fn search_scope_follows_the_active_pane() {
        let (root, home) = demo("scope");
        let outside = std::env::temp_dir().join(format!("sakadu-find-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("밖의 보고서.docx"), b"x").unwrap();
        let (mut a, g) = app(&home);
        a.open_find();
        a.enable_find();
        a.navigate(a.active, Location::Dir(util::join(&home, "업무")), None);
        a.find_typed("docx | md | jpg");
        a.settle();
        assert_eq!(found(&a).len(), 3);
        let here = a.find_scope_rect(true);
        let (x, y) = mid(here);
        assert_eq!(a.hit(&g, x, y), Hit::Find(FindHit::Scope(true)));
        click(&mut a, &g, here);
        a.settle();
        assert_eq!(sorted(found(&a)), ["보고서.docx", "회의록.md"]);
        assert!(a.find_wants_keys(), "the scope buttons keep the keyboard in the field");
        // 칸이 다른 폴더로 가면 그 폴더로 다시 찾는다
        a.navigate(a.active, Location::Dir(util::join(&home, "업무/2026 4분기")), None);
        a.settle();
        assert_eq!(found(&a), ["보고서.docx"]);
        // 훑는 범위 밖
        let out = util::normalize(&std::fs::canonicalize(&outside).unwrap().to_string_lossy());
        a.navigate(a.active, Location::Dir(out), None);
        a.settle();
        assert!(found(&a).is_empty() && a.find.outside);
        let all = a.find_scope_rect(false);
        click(&mut a, &g, all);
        a.settle();
        assert_eq!(found(&a).len(), 3);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    /// 결과를 둔 채 칸에서 일한다: 칸을 누르면 키보드만 떠나고, 돋보기 칸을 누르면 돌아온다. 닫기 단추는 닫는다.
    /// 디스크가 바뀌면 색인이 알려 줄 때 같은 검색어로 다시 찾는다
    #[test]
    fn search_stays_while_working_in_panes_and_follows_the_disk() {
        let (root, home) = demo("stay");
        let (mut a, g) = app(&home);
        a.open_find();
        a.enable_find();
        a.find_typed("보고");
        a.settle();
        let list = a.list_rect(a.active).unwrap();
        a.mouse_down(&g, list.l + 60.0, list.t + ROW_H / 2.0, Mods::default(), 1);
        a.mouse_up();
        assert!(a.find_shown() && !a.find_wants_keys());
        assert_eq!(found(&a).len(), 2);
        let (field, close) = field_parts(&a.toolbar()).unwrap();
        click(&mut a, &g, field);
        assert!(a.find_wants_keys());
        // 경로를 고치기 시작하면 키보드는 그 상자로 간다 (고치기를 마쳐도 검색 칸으로 돌아오지 않는다)
        a.begin_address(a.active);
        a.sync_edit();
        assert!(!a.find_wants_keys());
        a.cancel_address();
        assert!(!a.find_wants_keys());
        // 디스크가 바뀐다
        std::fs::write(std::path::Path::new(&home).join("새 보고 메모.txt"), b"x").unwrap();
        let began = std::time::Instant::now();
        while found(&a).len() < 3 && began.elapsed() < Duration::from_secs(20) {
            a.index_changed();
            a.settle();
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(found(&a).iter().any(|n| n == "새 보고 메모.txt"), "{:?}", found(&a));
        let (x, y) = mid(close);
        assert_eq!(a.hit(&g, x, y), Hit::FindClose);
        click(&mut a, &g, close);
        assert!(!a.find_shown());
        // 닫으면 사이드바가 돌아온다
        let (x, y) = mid(a.side_title());
        assert_eq!(a.hit(&g, x, y), Hit::SideTitle);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn search_places_are_short() {
        let (root, home) = demo("place");
        let (a, _g) = app(&home);
        let env = std::env::var("HOME").map(|h| util::normalize(&h)).unwrap();
        assert_eq!(a.place_of(&util::join(&env, "문서/보고서.docx")), "~/문서");
        assert_eq!(a.place_of(&util::join(&search::icloud_drive(&env), "메모/a.txt")), "iCloud Drive/메모");
        assert_eq!(a.place_of("/Applications/Safari.app"), "/Applications");
        std::fs::remove_dir_all(root).unwrap();
    }
}
