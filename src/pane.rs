//! 패널·탭 상태: 위치와 방문 기록, 정렬, 필터, 선택

use crate::listing::Entry;
use crate::util;
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq)]
pub enum Location {
    Computer,
    /// 새 탭 시작 화면 ("어디로 이동할까요?": 즐겨찾기 폴더·최근 위치·드라이브)
    Start,
    Dir(String),
}

impl Location {
    pub fn title(&self) -> String {
        match self {
            Location::Computer => "내 PC".to_string(),
            Location::Start => "새 탭".to_string(),
            Location::Dir(p) => util::file_name(p).to_string(),
        }
    }

    pub fn dir(&self) -> Option<&str> {
        match self {
            Location::Dir(p) => Some(p),
            Location::Computer | Location::Start => None,
        }
    }

    pub fn to_session(&self) -> String {
        match self {
            Location::Computer => "::computer".to_string(),
            Location::Start => "::start".to_string(),
            Location::Dir(p) => p.clone(),
        }
    }

    pub fn from_session(s: &str) -> Location {
        match s {
            "" | "::computer" => Location::Computer,
            "::start" => Location::Start,
            _ => Location::Dir(util::normalize(s)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Modified,
    Type,
    Size,
}

impl SortKey {
    pub fn from_u8(v: u8) -> SortKey {
        match v {
            1 => SortKey::Modified,
            2 => SortKey::Type,
            3 => SortKey::Size,
            _ => SortKey::Name,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sort {
    pub key: SortKey,
    pub desc: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort { key: SortKey::Name, desc: false }
    }
}

#[derive(Debug, PartialEq)]
pub enum LoadState {
    Loading,
    Ready,
    Error(String),
}

/// 다시 불러온 뒤 되살릴 선택·커서·스크롤. 이름은 소문자(내 PC 항목은 경로)로 비교한다
#[derive(Default, Debug, Clone)]
pub struct Restore {
    pub selected: Vec<String>,
    pub cursor: Option<String>,
    pub cursor_index: usize,
    pub scroll: f32,
    /// 커서가 보이도록 스크롤
    pub reveal: bool,
    /// 불러온 뒤 커서 항목 이름 바꾸기 시작
    pub rename: bool,
}

impl Restore {
    pub fn select(name: &str) -> Restore {
        let key = name.to_lowercase();
        Restore { selected: vec![key.clone()], cursor: Some(key), reveal: true, ..Default::default() }
    }
}

pub struct Tab {
    pub id: u64,
    pub loc: Location,
    pub back: Vec<Location>,
    pub fwd: Vec<Location>,
    pub entries: Vec<Entry>,
    /// entries의 인덱스를 정렬·필터 적용 순서로 담는다 (entries 자체는 열거 순서 유지)
    pub view: Vec<u32>,
    pub cursor: usize,
    pub anchor: usize,
    pub scroll: f32,
    pub filter: String,
    pub state: LoadState,
    /// 0이면 아직 한 번도 불러오지 않음
    pub generation: u64,
    pub restore: Option<Restore>,
    pub message: Option<String>,
    /// 읽기(처음 열기·새로 고침)를 시작한 때. 다 읽으면 None. 오래 걸리면 목록 자리에 기다린다는 글을 보인다
    pub loading_since: Option<std::time::Instant>,
    /// 네트워크 폴더를 열지 못했을 때 고칠 방법 (로그인·다시 연결 단추)
    pub net_fix: Option<crate::listing::NetFix>,
    /// 연결이 끊겨 이번 실행에서 마지막으로 본 목록을 보여 주는 중 (흐리게, 다시 연결되면 저절로 새로 고침)
    pub offline: bool,
}

fn key_of(e: &Entry) -> String {
    match &e.target {
        Some(t) => t.to_lowercase(),
        None => e.name_string().to_lowercase(),
    }
}

impl Tab {
    pub fn new(id: u64, loc: Location) -> Tab {
        Tab {
            id,
            loc,
            back: Vec::new(),
            fwd: Vec::new(),
            entries: Vec::new(),
            view: Vec::new(),
            cursor: 0,
            anchor: 0,
            scroll: 0.0,
            filter: String::new(),
            state: LoadState::Loading,
            generation: 0,
            restore: None,
            message: None,
            loading_since: None,
            net_fix: None,
            offline: false,
        }
    }

    pub fn entry_at(&self, vi: usize) -> Option<&Entry> {
        self.view.get(vi).map(|&i| &self.entries[i as usize])
    }

    pub fn cursor_entry(&self) -> Option<&Entry> {
        self.entry_at(self.cursor)
    }

    pub fn path_of(&self, e: &Entry) -> Option<String> {
        if let Some(t) = &e.target {
            return Some(t.to_string());
        }
        self.loc.dir().map(|d| util::join(d, &e.name_string()))
    }

    pub fn is_selected(&self, vi: usize) -> bool {
        self.entry_at(vi).is_some_and(|e| e.selected)
    }

    fn selected_iter(&self) -> impl Iterator<Item = &Entry> {
        self.view.iter().map(|&i| &self.entries[i as usize]).filter(|e| e.selected)
    }

    /// 선택된 항목의 전체 경로(보이는 순서). 선택이 없으면 커서 항목
    pub fn selected_paths(&self) -> Vec<String> {
        let mut v = self.marked_paths();
        if v.is_empty() {
            if let Some(p) = self.cursor_entry().and_then(|e| self.path_of(e)) {
                v.push(p);
            }
        }
        v
    }

    /// 실제로 고른 항목만(커서로 대신하지 않음). 지우기·잘라내기처럼 대상이 분명해야 하는 작업에 쓴다.
    /// 선택을 푼 뒤에도 커서는 남아 있어, 커서로 대신하면 방금 풀어 둔 파일이 지워질 수 있다
    pub fn marked_paths(&self) -> Vec<String> {
        self.selected_iter().filter_map(|e| self.path_of(e)).collect()
    }

    /// (선택 개수, 선택한 파일 크기 합)
    pub fn selection_stats(&self) -> (usize, u64) {
        self.selected_iter().fold((0, 0), |(n, s), e| (n + 1, if e.is_dir() { s } else { s + e.size }))
    }

    pub fn rebuild_view(&mut self, sort: Sort, show_hidden: bool) {
        let keep = self.view.get(self.cursor).copied();
        let needle = self.filter.to_lowercase();
        let entries = &self.entries;
        let mut view: Vec<u32> = (0..entries.len() as u32)
            .filter(|&i| {
                let e = &entries[i as usize];
                (show_hidden || !e.is_hidden())
                    && (needle.is_empty() || e.name_string().to_lowercase().contains(&needle))
            })
            .collect();
        // "내 PC"는 사용자 폴더 → 드라이브 순서를 그대로 둔다
        if self.loc != Location::Computer {
            view.sort_by(|&a, &b| compare(&entries[a as usize], &entries[b as usize], sort));
        }
        // 화면에서 사라진 항목은 선택 해제해, 보이지 않는 파일이 지워지거나 옮겨지는 일을 막는다
        let mut visible = vec![false; entries.len()];
        for &i in &view {
            visible[i as usize] = true;
        }
        for (e, v) in self.entries.iter_mut().zip(visible) {
            if !v {
                e.selected = false;
            }
        }
        self.view = view;
        self.cursor = keep.and_then(|k| self.view.iter().position(|&i| i == k)).unwrap_or(0);
        self.anchor = self.anchor.min(self.view.len().saturating_sub(1));
    }

    /// 새 목록을 적용하고 restore를 반영한다. 반영한 restore를 돌려준다
    pub fn set_entries(&mut self, entries: Vec<Entry>, sort: Sort, show_hidden: bool) -> Option<Restore> {
        self.entries = entries;
        self.state = LoadState::Ready;
        self.view.clear();
        self.cursor = 0;
        self.anchor = 0;
        let restore = self.restore.take();
        if let Some(r) = restore.as_ref().filter(|r| !r.selected.is_empty()) {
            // 모두 고른 채 다시 읽어도(F5) 항목마다 목록 전체를 훑지 않게 한 번만 모아 둔다
            let keep: std::collections::HashSet<&str> = r.selected.iter().map(String::as_str).collect();
            for e in self.entries.iter_mut() {
                if keep.contains(key_of(e).as_str()) {
                    e.selected = true;
                }
            }
        }
        self.rebuild_view(sort, show_hidden);
        if let Some(r) = &restore {
            let found = r.cursor.as_ref().and_then(|c| self.view.iter().position(|&i| key_of(&self.entries[i as usize]) == *c));
            self.cursor = found.unwrap_or(r.cursor_index.min(self.view.len().saturating_sub(1)));
            self.anchor = self.cursor;
            self.scroll = r.scroll;
        } else {
            self.scroll = 0.0;
        }
        restore
    }

    pub fn snapshot(&self) -> Restore {
        Restore {
            selected: self.entries.iter().filter(|e| e.selected).map(key_of).collect(),
            cursor: self.cursor_entry().map(key_of),
            cursor_index: self.cursor,
            scroll: self.scroll,
            reveal: false,
            rename: false,
        }
    }

    pub fn clear_selection(&mut self) {
        for e in &mut self.entries {
            e.selected = false;
        }
    }

    fn set_selected(&mut self, vi: usize, v: bool) {
        if let Some(&i) = self.view.get(vi) {
            self.entries[i as usize].selected = v;
        }
    }

    pub fn select_only(&mut self, vi: usize) {
        self.clear_selection();
        self.set_selected(vi, true);
        self.cursor = vi;
        self.anchor = vi;
    }

    pub fn toggle(&mut self, vi: usize) {
        let v = !self.is_selected(vi);
        self.set_selected(vi, v);
        self.cursor = vi;
        self.anchor = vi;
    }

    pub fn select_range(&mut self, to: usize, additive: bool) {
        if !additive {
            self.clear_selection();
        }
        let (a, b) = if self.anchor <= to { (self.anchor, to) } else { (to, self.anchor) };
        for vi in a..=b.min(self.view.len().saturating_sub(1)) {
            self.set_selected(vi, true);
        }
        self.cursor = to;
    }

    /// 보이는 순서대로 선택 여부
    pub fn selection_mask(&self) -> Vec<bool> {
        (0..self.view.len()).map(|vi| self.is_selected(vi)).collect()
    }

    /// 끌어서 고르기: 끌기 전 선택(base)에 네모 상자 안 항목(inside)을 더한다
    pub fn select_marquee(&mut self, base: &[bool], inside: impl Fn(usize) -> bool) {
        for vi in 0..self.view.len() {
            let v = base.get(vi).copied().unwrap_or(false) || inside(vi);
            self.set_selected(vi, v);
        }
    }

    pub fn select_all(&mut self) {
        for k in 0..self.view.len() {
            self.set_selected(k, true);
        }
    }

    /// 키보드 커서 이동: Shift는 범위 선택, Ctrl은 선택을 유지한 채 커서만 이동
    pub fn move_cursor(&mut self, to: usize, shift: bool, ctrl: bool) {
        if self.view.is_empty() {
            return;
        }
        let to = to.min(self.view.len() - 1);
        if shift {
            self.select_range(to, ctrl);
        } else if ctrl {
            self.cursor = to;
        } else {
            self.select_only(to);
        }
    }

    pub fn nav_push(&mut self, loc: Location) {
        if self.loc != loc {
            let old = std::mem::replace(&mut self.loc, loc);
            self.back.push(old);
            if self.back.len() > 100 {
                self.back.remove(0);
            }
            self.fwd.clear();
        }
    }

    pub fn nav_back(&mut self) -> bool {
        match self.back.pop() {
            Some(prev) => {
                let cur = std::mem::replace(&mut self.loc, prev);
                self.fwd.push(cur);
                true
            }
            None => false,
        }
    }

    pub fn nav_forward(&mut self) -> bool {
        match self.fwd.pop() {
            Some(next) => {
                let cur = std::mem::replace(&mut self.loc, next);
                self.back.push(cur);
                true
            }
            None => false,
        }
    }

    /// 다른 위치로 이동할 때 목록 관련 상태를 비운다
    pub fn reset_view_state(&mut self) {
        self.entries.clear();
        self.view.clear();
        self.cursor = 0;
        self.anchor = 0;
        self.scroll = 0.0;
        self.filter.clear();
        self.message = None;
        self.restore = None;
    }
}

pub fn compare(a: &Entry, b: &Entry, sort: Sort) -> Ordering {
    // 폴더는 정렬 방향과 관계없이 항상 위
    match (a.is_dir(), b.is_dir()) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    let ord = match sort.key {
        SortKey::Name => natural(a, b),
        SortKey::Modified => a.modified.cmp(&b.modified).then_with(|| natural(a, b)),
        SortKey::Size => a.size.cmp(&b.size).then_with(|| natural(a, b)),
        SortKey::Type => cmp_ext(a.ext(), b.ext()).then_with(|| natural(a, b)),
    };
    if sort.desc { ord.reverse() } else { ord }
}

/// 탐색기와 같은 자연 정렬 ("file2" < "file10")
fn natural(a: &Entry, b: &Entry) -> Ordering {
    crate::natural::cmp_z(a.name_z(), b.name_z())
}

fn cmp_ext(a: &[u16], b: &[u16]) -> Ordering {
    let lower = |c: &u16| if (b'A' as u16..=b'Z' as u16).contains(c) { c + 32 } else { *c };
    a.iter().map(lower).cmp(b.iter().map(lower))
}

/// 파일 보기 방식 (탐색기의 보기 메뉴와 같은 순서)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ViewMode {
    Huge,
    Large,
    Medium,
    Small,
    List,
    #[default]
    Details,
}

impl ViewMode {
    pub const ALL: [ViewMode; 6] = [ViewMode::Huge, ViewMode::Large, ViewMode::Medium, ViewMode::Small, ViewMode::List, ViewMode::Details];

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Huge => "아주 큰 아이콘",
            ViewMode::Large => "큰 아이콘",
            ViewMode::Medium => "보통 아이콘",
            ViewMode::Small => "작은 아이콘",
            ViewMode::List => "목록",
            ViewMode::Details => "자세히",
        }
    }

    /// 세션 파일 값. 0(자세히)과 1(큰 아이콘, 예전 썸네일)은 예전 파일과 같다
    pub fn code(self) -> u8 {
        match self {
            ViewMode::Details => 0,
            ViewMode::Large => 1,
            ViewMode::Huge => 2,
            ViewMode::Medium => 3,
            ViewMode::Small => 4,
            ViewMode::List => 5,
        }
    }

    pub fn from_code(c: u8) -> ViewMode {
        ViewMode::ALL.into_iter().find(|v| v.code() == c).unwrap_or_default()
    }
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub sort: Sort,
    /// 수정한 날짜, 유형, 크기 열 너비(DIP)
    pub cols: [f32; 3],
    pub view: ViewMode,
}

/// 유형 열은 "Microsoft Excel 워크시트"가 잘리지 않는 너비
pub const DEFAULT_COLS: [f32; 3] = [140.0, 180.0, 84.0];

impl Pane {
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::{ATTR_HIDDEN, Kind};

    fn tab_with(names: &[(&str, Kind)]) -> Tab {
        let mut t = Tab::new(1, Location::Dir("C:\\t".into()));
        let entries = names.iter().map(|(n, k)| Entry::from_str(n, *k)).collect();
        t.set_entries(entries, Sort::default(), false);
        t
    }

    fn names(t: &Tab) -> Vec<String> {
        t.view.iter().map(|&i| t.entries[i as usize].name_string()).collect()
    }

    #[test]
    fn sorts_dirs_first_and_naturally() {
        let t = tab_with(&[("file10.txt", Kind::File), ("b", Kind::Dir), ("file2.txt", Kind::File), ("A", Kind::Dir)]);
        assert_eq!(names(&t), ["A", "b", "file2.txt", "file10.txt"]);
    }

    #[test]
    fn descending_keeps_dirs_first() {
        let mut t = tab_with(&[("a.txt", Kind::File), ("z", Kind::Dir), ("b.txt", Kind::File)]);
        t.rebuild_view(Sort { key: SortKey::Name, desc: true }, false);
        assert_eq!(names(&t), ["z", "b.txt", "a.txt"]);
    }

    #[test]
    fn filter_hides_and_deselects() {
        let mut t = tab_with(&[("보고서.docx", Kind::File), ("사진.jpg", Kind::File), ("Report.pdf", Kind::File)]);
        t.select_all();
        t.filter = "보고".into();
        t.rebuild_view(Sort::default(), false);
        assert_eq!(names(&t), ["보고서.docx"]);
        assert_eq!(t.selection_stats().0, 1);
        assert_eq!(t.selected_paths(), ["C:\\t\\보고서.docx"]);
    }

    #[test]
    fn hidden_items_follow_toggle() {
        let mut t = Tab::new(1, Location::Dir("C:\\t".into()));
        let mut h = Entry::from_str(".secret", Kind::File);
        h.attrs = ATTR_HIDDEN;
        t.set_entries(vec![h, Entry::from_str("open.txt", Kind::File)], Sort::default(), false);
        assert_eq!(names(&t), ["open.txt"]);
        t.rebuild_view(Sort::default(), true);
        assert_eq!(t.view.len(), 2);
    }

    #[test]
    fn range_selection_with_shift() {
        let mut t = tab_with(&[("1", Kind::File), ("2", Kind::File), ("3", Kind::File), ("4", Kind::File)]);
        t.select_only(1);
        t.move_cursor(3, true, false);
        assert_eq!(t.selection_stats().0, 3);
        t.move_cursor(0, false, false);
        assert_eq!(t.selection_stats().0, 1);
    }

    /// 회귀: 파일을 고른 뒤 빈 곳 클릭·Esc로 선택을 풀면 지우기·잘라내기 대상이 없어야 한다
    #[test]
    fn cleared_selection_leaves_nothing_to_delete() {
        let mut t = tab_with(&[("a.txt", Kind::File), ("b.txt", Kind::File), ("c.txt", Kind::File)]);
        t.select_only(1);
        assert_eq!(t.marked_paths(), ["C:\\t\\b.txt"]);
        t.clear_selection();
        assert!(t.marked_paths().is_empty());
        // 열기·복사처럼 되돌릴 수 있는 작업은 지금처럼 커서 항목을 쓴다
        assert_eq!(t.selected_paths(), ["C:\\t\\b.txt"]);
        // 방향키로 고르면 다시 대상이 된다
        t.move_cursor(2, false, false);
        assert_eq!(t.marked_paths(), ["C:\\t\\c.txt"]);
        // 폴더를 막 연 상태(커서만 첫 줄)도 대상이 없다
        assert!(tab_with(&[("x.txt", Kind::File)]).marked_paths().is_empty());
    }

    #[test]
    fn restore_reselects_by_name_after_reload() {
        let mut t = tab_with(&[("a", Kind::File), ("b", Kind::File), ("c", Kind::File)]);
        t.select_only(1);
        t.restore = Some(t.snapshot());
        let fresh = vec![Entry::from_str("c", Kind::File), Entry::from_str("new", Kind::File), Entry::from_str("b", Kind::File), Entry::from_str("a", Kind::File)];
        t.set_entries(fresh, Sort::default(), false);
        assert_eq!(t.cursor_entry().unwrap().name_string(), "b");
        assert_eq!(t.selection_stats().0, 1);
    }

    /// 성능 측정: QP_BENCH_DIR 폴더를 열거·정렬·필터하는 시간 (cargo test --release -- --ignored --nocapture)
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn bench_list_sort_filter() {
        let Ok(dir) = std::env::var("QP_BENCH_DIR") else { return };
        let t0 = std::time::Instant::now();
        let entries = crate::listing::list_dir(&dir).unwrap();
        let t1 = std::time::Instant::now();
        let n = entries.len();
        let mut t = Tab::new(1, Location::Dir(dir));
        t.set_entries(entries, Sort::default(), false);
        let t2 = std::time::Instant::now();
        t.filter = "_1999".into();
        t.rebuild_view(Sort::default(), false);
        let t3 = std::time::Instant::now();
        println!("{n} entries: list {:?}, sort {:?}, filter {:?} ({} matches)", t1 - t0, t2 - t1, t3 - t2, t.view.len());
    }

    /// 성능: 1만 개를 모두 고른 채 다시 읽을 때(F5) 고른 항목을 이름으로 되살리는 시간
    /// (cargo test --release bench_reload_all_selected -- --ignored --nocapture)
    #[test]
    #[ignore]
    fn bench_reload_all_selected() {
        let names: Vec<String> = (0..10_000).map(|i| format!("파일_{i:05}.txt")).collect();
        let make = || names.iter().map(|n| Entry::from_str(n, Kind::File)).collect::<Vec<_>>();
        let mut t = Tab::new(1, Location::Dir("C:\\t".into()));
        t.set_entries(make(), Sort::default(), false);
        t.select_all();
        for round in 1..=3 {
            t.restore = Some(t.snapshot());
            let fresh = make();
            let t0 = std::time::Instant::now();
            t.set_entries(fresh, Sort::default(), false);
            let took = t0.elapsed();
            assert_eq!(t.selection_stats().0, 10_000);
            println!("round {round}: reload with 10000 selected: {took:?}");
        }
    }

    #[test]
    fn history_back_and_forward() {
        let mut t = Tab::new(1, Location::Computer);
        t.nav_push(Location::Dir("C:\\".into()));
        t.nav_push(Location::Dir("C:\\Users".into()));
        assert!(t.nav_back());
        assert_eq!(t.loc, Location::Dir("C:\\".into()));
        assert!(t.nav_forward());
        assert_eq!(t.loc, Location::Dir("C:\\Users".into()));
        assert!(!t.nav_forward());
    }

    /// 탭 위치는 세션 글자로 저장·복원된다 (시작 화면·내 PC·폴더)
    #[test]
    fn locations_round_trip_through_the_session() {
        for loc in [Location::Start, Location::Computer, Location::Dir("C:\\작업\\a=b".into())] {
            assert_eq!(Location::from_session(&loc.to_session()), loc);
        }
        assert_eq!(Location::from_session(""), Location::Computer);
        assert_eq!(Location::Start.dir(), None);
    }

    /// 세션 파일의 보기 값: 예전 파일(0 자세히, 1 썸네일)은 그대로 읽히고, 모든 보기가 저장·복원된다
    #[test]
    fn view_codes_round_trip_and_keep_old_values() {
        assert_eq!(ViewMode::from_code(0), ViewMode::Details);
        assert_eq!(ViewMode::from_code(1), ViewMode::Large);
        assert_eq!(ViewMode::from_code(99), ViewMode::Details);
        for v in ViewMode::ALL {
            assert_eq!(ViewMode::from_code(v.code()), v);
        }
    }
}
