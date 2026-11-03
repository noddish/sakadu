//! 패널·탭 상태: 위치와 방문 기록, 정렬, 필터, 선택

use crate::listing::Entry;
use crate::util;
use std::cmp::Ordering;
use windows::Win32::UI::Shell::StrCmpLogicalW;
use windows::core::PCWSTR;

#[derive(Clone, Debug, PartialEq)]
pub enum Location {
    Computer,
    Dir(String),
}

impl Location {
    pub fn title(&self) -> String {
        match self {
            Location::Computer => "내 PC".to_string(),
            Location::Dir(p) => util::file_name(p).to_string(),
        }
    }

    pub fn dir(&self) -> Option<&str> {
        match self {
            Location::Dir(p) => Some(p),
            Location::Computer => None,
        }
    }

    pub fn to_session(&self) -> String {
        match self {
            Location::Computer => "::computer".to_string(),
            Location::Dir(p) => p.clone(),
        }
    }

    pub fn from_session(s: &str) -> Location {
        if s.is_empty() || s == "::computer" { Location::Computer } else { Location::Dir(util::normalize(s)) }
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
        let mut v: Vec<String> = self.selected_iter().filter_map(|e| self.path_of(e)).collect();
        if v.is_empty() {
            if let Some(p) = self.cursor_entry().and_then(|e| self.path_of(e)) {
                v.push(p);
            }
        }
        v
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
        if let Some(r) = &restore {
            for e in self.entries.iter_mut() {
                if !r.selected.is_empty() && r.selected.contains(&key_of(e)) {
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
    let r = unsafe { StrCmpLogicalW(PCWSTR(a.name_z().as_ptr()), PCWSTR(b.name_z().as_ptr())) };
    r.cmp(&0)
}

fn cmp_ext(a: &[u16], b: &[u16]) -> Ordering {
    let lower = |c: &u16| if (b'A' as u16..=b'Z' as u16).contains(c) { c + 32 } else { *c };
    a.iter().map(lower).cmp(b.iter().map(lower))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ViewMode {
    #[default]
    Details,
    Thumbs,
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub sort: Sort,
    /// 수정한 날짜, 유형, 크기 열 너비(DIP)
    pub cols: [f32; 3],
    pub view: ViewMode,
}

pub const DEFAULT_COLS: [f32; 3] = [140.0, 130.0, 84.0];

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
}
