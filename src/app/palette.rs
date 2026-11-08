//! 검색 패널(Ctrl+F, 막대의 검색 아이콘): 검색 서비스(인덱스)와 검색에 넣은 폴더에서 이름으로 바로 찾는다.
//! 아이콘 막대 옆에 고정해 열리는 패널이라 결과를 둔 채 칸에서 일할 수 있다. 위치 열기(Ctrl+Enter)는 지금 칸에서 그 폴더를 열고
//! 파일을 골라 두며, 폴더가 바뀌거나 파일 작업이 끝나면 같은 검색어로 다시 찾는다

use super::*;
use crate::service::{self, SearchError, SearchResponse};
use windows::Win32::UI::Controls::EM_GETSEL;
use windows::Win32::UI::Shell::ShellExecuteW;

/// 패널 머리(40) 아래 입력 줄
const S_INPUT_T: f32 = 48.0;
const S_INPUT_H: f32 = 32.0;
/// 범위 칩 줄 (PC 전체 · 현재 폴더)
const S_CHIP_T: f32 = 88.0;
const S_CHIP_H: f32 = 24.0;
const S_CHIP_W: f32 = 78.0;
/// 결과 목록 시작
const S_LIST_T: f32 = 120.0;
const S_FOOT_H: f32 = 30.0;
/// 결과 한 줄: 이름, 그 아래 상위 폴더
const PAL_ROW_H: f32 = 46.0;
const G_CHECK: u16 = 0xE73E;
const PAL_LIMIT: usize = 200;
pub(super) const G_SEARCH: u16 = 0xE721;
pub(super) const TIMER_SEARCH: usize = 20;
/// 검색 서비스가 준비 중일 때 준비됐는지 다시 묻는 간격
pub(super) const TIMER_SEARCH_READY: usize = 21;
/// 폴더가 바뀌거나 파일 작업이 끝난 뒤 같은 검색어로 다시 찾기 (입력 디바운스와 따로)
pub(super) const TIMER_SEARCH_AGAIN: usize = 22;
/// "현재 폴더" 범위로 찾을 때 서비스에 더 넉넉히 묻는다 (폴더 밖 결과를 걸러낸 뒤 남는 것이 있도록)
const SCOPE_LIMIT: usize = PAL_LIMIT * 10;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PalStatus {
    Idle,
    Searching,
    /// ready: 서비스가 모든 디스크 목록을 다 만들었는지, failed: 읽지 못한 드라이브 글자
    Done { total: usize, micros: u64, ready: bool, failed: String },
    NotInstalled,
    Indexing,
    Ready(usize),
    Error(String),
}

pub enum SearchReply {
    Status(Result<(bool, usize), SearchError>),
    Results(Result<SearchResponse, SearchError>),
}

pub struct SearchDone {
    seq: u64,
    /// 결과를 찾은 검색어 (상태 확인이면 None)
    query: Option<String>,
    /// 찾은 범위(현재 폴더). PC 전체면 None
    scope: Option<String>,
    reply: SearchReply,
}

/// 보이는 결과가 지금 입력한 검색어의 것인지, 결과보다 먼저 누른 Enter를 기억한다.
/// 입력이 바뀌면 이전 결과로는 열거나 복사하지 않고, 먼저 누른 Enter는 그 검색어의 결과가 왔을 때 한 번만 처리한다
#[derive(Default)]
pub(super) struct PalGate {
    /// 보이는 결과의 검색어
    shown: Option<String>,
    /// 마지막으로 찾으라고 보낸 검색어
    asked: Option<String>,
    enter: Option<OpenMode>,
    /// 마지막으로 입력한 때 (진단 로그: 입력부터 결과가 화면에 보일 때까지)
    typed: Option<std::time::Instant>,
}

impl PalGate {
    pub(super) fn current(&self, text: &str) -> bool {
        self.shown.as_deref() == Some(text)
    }

    /// 입력이 바뀜: 먼저 눌러 둔 Enter는 버린다
    pub(super) fn edited(&mut self) {
        self.enter = None;
        self.typed = Some(std::time::Instant::now());
    }

    /// 지금 검색어의 결과를 처음 그릴 때 입력부터 걸린 시간 (한 번만)
    fn shown_after_typing(&mut self, text: &str) -> Option<std::time::Duration> {
        if self.current(text) { self.typed.take().map(|t| t.elapsed()) } else { None }
    }

    pub(super) fn cancel(&mut self) {
        self.enter = None;
    }

    fn asked(&mut self, q: &str) {
        self.asked = Some(q.to_string());
    }

    /// Enter. 보이는 결과가 지금 검색어의 것이면 바로 열 방식을, 아니면 결과가 올 때까지 미루고 None
    pub(super) fn enter(&mut self, text: &str, mode: OpenMode) -> Option<OpenMode> {
        if self.current(text) {
            return Some(mode);
        }
        self.enter = Some(mode);
        None
    }

    /// 결과가 왔다. 미뤄 둔 Enter를 지금 처리해야 하면 그 방식을 돌려준다 (한 번만, 결과가 있을 때만)
    pub(super) fn arrived(&mut self, query: &str, text: &str, count: usize) -> Option<OpenMode> {
        self.shown = Some(query.to_string());
        let enter = self.enter.take();
        enter.filter(|_| count > 0 && query == text)
    }
}

/// scope가 있으면 그 폴더 아래만 남긴다. 범위는 검색어 조각마다 경로 조건으로 덧붙여(scoped_query) 서비스와
/// 넣은 폴더가 순위를 매기고 자르기 전에 거르게 하고, 받은 뒤 정확한 범위로 한 번 더 거른다 (결과의 검색어는 입력한 그대로 돌려준다)
fn spawn_search(hwnd: HWND, seq: u64, query: Option<String>, scope: Option<String>, folders: crate::folders::Folders) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let reply = match &query {
            Some(q) => {
                let limit = if scope.is_some() { SCOPE_LIMIT } else { PAL_LIMIT };
                let sent = scope.as_deref().map_or_else(|| q.clone(), |d| scoped_query(q, d));
                SearchReply::Results(search_all(&sent, &folders, limit).map(|mut r| {
                    if let Some(dir) = &scope {
                        r.hits.retain(|(_, p)| under(p, dir));
                        r.total = r.hits.len();
                        r.hits.truncate(PAL_LIMIT);
                    }
                    r
                }))
            }
            None => SearchReply::Status(service::status()),
        };
        post(owner.get(), WM_APP_SEARCH, Box::new(SearchDone { seq, query, scope, reply }));
    });
}

/// 현재 폴더 범위를 검색어 조각마다 경로 조건으로 덧붙인다. 서비스와 넣은 폴더는 '\'가 든 낱말을 전체 경로에서 찾으며
/// 순위를 매기고 자르기 전에 거르므로, 다른 곳의 일치 항목이 많아도 이 폴더 아래 항목을 놓치지 않는다.
/// 경로를 띄어쓰기로 나눈 조각 가운데 '\'가 든 것만 쓴다('\'가 없으면 이름 조건이 된다). 이 폴더 아래 경로는 이 조각들을
/// 늘 모두 품으므로 범위 안의 결과를 잃지 않고, 정확한 범위는 받은 뒤 under()로 거른다
fn scoped_query(q: &str, dir: &str) -> String {
    let dir = format!("{}\\", dir.trim_end_matches('\\'));
    let terms = dir.split_whitespace().filter(|w| w.contains('\\')).collect::<Vec<_>>().join(" ");
    query_pieces(q).iter().map(|p| format!("{p} {terms}")).collect::<Vec<_>>().join(" | ")
}

/// 보이는 결과가 지금 것인지 가르는 열쇠: 검색어와 범위. 범위를 바꾸면 같은 검색어여도 이전 결과로는 열지 않는다
fn pal_key(text: &str, scope: Option<&str>) -> String {
    match scope {
        Some(d) => format!("{text}\u{1F}{}", d.to_lowercase()),
        None => text.to_string(),
    }
}

/// path가 dir 안에 있는가 (대소문자 무시)
fn under(path: &str, dir: &str) -> bool {
    let (p, d) = (path.to_lowercase(), dir.trim_end_matches('\\').to_lowercase());
    p.len() > d.len() && p.starts_with(&d) && p.as_bytes()[d.len()] == b'\\'
}

/// 드라이브 글자들을 "D:, E:"처럼
fn drive_list(letters: &str) -> String {
    letters.chars().map(|c| format!("{c}:")).collect::<Vec<_>>().join(", ")
}

/// 검색어를 `|`로 나눈 조각들(또는). 조각 안의 띄어 쓴 낱말은 모두 포함해야 한다
pub(super) fn query_pieces(q: &str) -> Vec<&str> {
    q.split('|').map(str::trim).filter(|p| !p.is_empty()).collect()
}

/// 여러 줄 텍스트(엑셀 열 등)를 한 줄 검색어로: 줄마다 `|`로 잇는다
pub(super) fn lines_to_query(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" | ")
}

/// 한 조각의 결과: (점수, 폴더 여부, 경로), 전체 개수, 걸린 시간, 서비스 준비 여부, 읽지 못한 드라이브
struct Found {
    hits: Vec<(i32, bool, String)>,
    total: usize,
    micros: u64,
    ready: bool,
    failed: String,
}

/// 같은 검색어 조각은 한 번만 찾는다 (엑셀 열에는 같은 값이 자주 되풀이된다).
/// 대소문자·띄어쓰기 차이만 있는 조각은 찾는 결과가 같으므로 같은 것으로 본다
pub(super) fn unique_pieces(q: &str) -> Vec<&str> {
    let mut seen = HashSet::new();
    query_pieces(q).into_iter().filter(|p| seen.insert(p.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase())).collect()
}

/// 한 조각을 디스크 인덱스(검색 서비스 결과 local)와 검색에 넣은 폴더에서 찾는다
fn search_piece(q: &str, local: Result<SearchResponse, SearchError>, folders: &crate::folders::Folders, limit: usize) -> Result<Found, SearchError> {
    let started = std::time::Instant::now();
    let query = crate::index::Query::parse(q);
    let (extra, extra_total) = folders.search(&query, limit);
    // 서비스가 없거나 실패해도 폴더 쪽 결과는 보여 준다
    let r = match local {
        Ok(r) => r,
        Err(e) if extra_total > 0 => {
            if let SearchError::Other(e) = e {
                crate::diag::log(|| format!("search: {e}"));
            }
            SearchResponse { total: 0, micros: 0, hits: Vec::new(), ready: true, failed: String::new() }
        }
        Err(e) => return Err(e),
    };
    // 서비스 결과에도 같은 기준으로 점수를 매겨 폴더 결과·다른 조각 결과와 섞는다
    let hits = r
        .hits
        .into_iter()
        .map(|(is_dir, path)| (query.name_score(util::file_name(&path), is_dir).and_then(|s| query.path_score(s, &path)).unwrap_or(i32::MIN), is_dir, path))
        .chain(extra.into_iter().map(|h| (h.score, h.is_dir, h.path)))
        .collect();
    Ok(Found { hits, total: r.total + extra_total, micros: r.micros + started.elapsed().as_micros() as u64, ready: r.ready, failed: r.failed })
}

/// 조각마다 찾아 합친다. 여러 조각에 걸린 항목은 가장 높은 점수로 한 번만 남는다.
/// 전체 개수는 조각별 개수의 합이라 겹치는 항목이 있으면 실제보다 조금 클 수 있다
fn search_all(q: &str, folders: &crate::folders::Folders, limit: usize) -> Result<SearchResponse, SearchError> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut all: Vec<(i32, bool, String)> = Vec::new();
    let (mut total, mut micros, mut error) = (0, 0, None);
    let (mut ready, mut failed) = (true, String::new());
    let pieces = unique_pieces(q);
    // 서비스에는 조각들을 묶어 한 번에 묻는다 (줄마다 파이프를 오가지 않게)
    let locals = service::query_many(&pieces, limit);
    for (piece, local) in pieces.into_iter().zip(locals) {
        match search_piece(piece, local, folders, limit) {
            Ok(Found { hits, total: t, micros: m, ready: r, failed: f }) => {
                total += t;
                micros += m;
                ready &= r;
                for c in f.chars() {
                    if !failed.contains(c) {
                        failed.push(c);
                    }
                }
                for hit in hits {
                    match seen.get(&hit.2.to_lowercase()) {
                        Some(&i) => all[i].0 = all[i].0.max(hit.0),
                        None => {
                            seen.insert(hit.2.to_lowercase(), all.len());
                            all.push(hit);
                        }
                    }
                }
            }
            Err(e) => error = Some(e),
        }
    }
    if let (true, Some(e)) = (all.is_empty(), error) {
        return Err(e);
    }
    all.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.len().cmp(&b.2.len())).then_with(|| crate::favorites::natural(&a.2, &b.2)));
    all.truncate(limit);
    Ok(SearchResponse { total, micros, hits: all.into_iter().map(|(_, d, p)| (d, p)).collect(), ready, failed })
}

const PAL_MENU_OPEN: u32 = 1;
const PAL_MENU_LOCATE: u32 = 2;
const PAL_MENU_COPY: u32 = 3;
const PAL_MENU_COPY_PATH: u32 = 4;
const PAL_MENU_PICK_ALL: u32 = 5;
const PAL_MENU_CLEAR: u32 = 6;

/// 검색 결과 오른쪽 버튼 메뉴. 고른 결과(없으면 누른 줄)를 한꺼번에 다룬다
pub(super) fn show_pal_menu(hwnd: HWND, pt: POINT) {
    let Some((n, picks)) = with_app(|a| (a.pal_targets().len(), a.pal_pick_count())) else { return };
    if n == 0 {
        return;
    }
    let open = if n > 1 { format!("{n}개 열기\tEnter") } else { "열기\tEnter".to_string() };
    let mut items: Vec<(u32, String)> = vec![
        (PAL_MENU_OPEN, open),
        (PAL_MENU_LOCATE, "지금 칸에서 위치 열기\tCtrl+Enter".to_string()),
        (0, String::new()),
        (PAL_MENU_COPY, "복사\tCtrl+C".to_string()),
        (PAL_MENU_COPY_PATH, "경로 복사".to_string()),
        (0, String::new()),
        (PAL_MENU_PICK_ALL, "결과 모두 고르기".to_string()),
    ];
    if picks > 0 {
        items.push((PAL_MENU_CLEAR, "고르기 풀기\tEsc".to_string()));
    }
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        for (id, label) in &items {
            let _ = if *id == 0 {
                AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())
            } else {
                AppendMenuW(menu, MF_STRING, *id as usize, &HSTRING::from(label.as_str()))
            };
        }
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;
        let _ = DestroyMenu(menu);
        cmd
    };
    if cmd != 0 {
        with_app(|a| a.pal_menu_command(cmd));
    }
}

/// 입력 상자에 선택한 글자가 있는지
fn edit_has_selection(edit: HWND) -> bool {
    let (mut start, mut end) = (0u32, 0u32);
    unsafe {
        SendMessageW(edit, EM_GETSEL, Some(WPARAM(&mut start as *mut u32 as usize)), Some(LPARAM(&mut end as *mut u32 as isize)));
    }
    start != end
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OpenMode {
    Open,
    Locate,
    NewTab,
}

impl App {
    /// 검색 패널이 열려 있는가 (고정돼 있으면 다른 패널이 잠깐 떠서 가려도 열린 것: 그 사이 온 결과를 버리지 않는다)
    pub(super) fn pal_shown(&self) -> bool {
        self.side_pinned == Some(Panel::Search) || self.side_peek == Some(Panel::Search)
    }

    /// 검색 패널이 지금 화면에 보이는가 (그릴 때·누를 때·입력 상자를 둘 때)
    pub(super) fn pal_visible(&self) -> bool {
        self.side_panel() == Some(Panel::Search)
    }

    /// 검색 패널 자리 (다른 패널이 잠깐 가려도 같은 자리). 열려 있지 않으면 빈 사각형
    pub(super) fn pal_rect(&self) -> R {
        if self.pal_shown() { self.side_panel_rect().unwrap_or(R::new(0.0, 0.0, 0.0, 0.0)) } else { R::new(0.0, 0.0, 0.0, 0.0) }
    }

    pub(super) fn pal_list(&self) -> R {
        let r = self.pal_rect();
        R::new(r.l + 8.0, r.t + S_LIST_T, r.r - 8.0, (r.b - S_FOOT_H).max(r.t + S_LIST_T))
    }

    /// 입력 줄의 들어간 상자 (실제 입력은 그 위에 놓인 EDIT)
    fn pal_input_box(&self) -> R {
        let r = self.pal_rect();
        R::new(r.l + 8.0, r.t + S_INPUT_T, r.r - 8.0, r.t + S_INPUT_T + S_INPUT_H)
    }

    pub(super) fn pal_input_rect(&self) -> R {
        let b = self.pal_input_box();
        R::new(b.l + 30.0, b.t + 8.0, b.r - 8.0, b.b - 4.0)
    }

    /// 범위 칩: PC 전체(false), 현재 폴더(true)
    fn pal_scope_rect(&self, here: bool) -> R {
        let r = self.pal_rect();
        let x = r.l + 8.0 + if here { S_CHIP_W + 6.0 } else { 0.0 };
        R::new(x, r.t + S_CHIP_T, x + S_CHIP_W, r.t + S_CHIP_T + S_CHIP_H)
    }

    /// 머리 오른쪽 닫기 (다른 패널의 핀 자리)
    pub(super) fn pal_close_rect(&self) -> R {
        let r = self.pal_rect();
        R::new(r.r - 8.0 - 26.0, r.t + 9.0, r.r - 8.0, r.t + 35.0)
    }

    fn pal_install_button(&self) -> R {
        let list = self.pal_list();
        let cx = (list.l + list.r) / 2.0;
        R::new(cx - 80.0, list.t + 150.0, cx + 80.0, list.t + 182.0)
    }

    /// 찾은 게 없고 보이는 칸에 검색 범위 밖 폴더가 열려 있으면 "이 폴더도 검색하기" 버튼을 보인다
    fn pal_shows_add_folder(&self) -> bool {
        matches!(self.pal_status, PalStatus::Done { .. }) && self.pal_results.is_empty() && self.pal_outside.is_some()
    }

    /// 검색 패널 안에서 누를 수 있는 것 (패널 자리 안이라고 확인한 뒤 부른다). 빈 곳은 SidePanel
    pub(super) fn pal_hit(&self, x: f32, y: f32) -> Hit {
        if self.pal_close_rect().contains(x, y) {
            return Hit::PalClose;
        }
        for here in [false, true] {
            if self.pal_scope_rect(here).contains(x, y) {
                return Hit::PalScope(here);
            }
        }
        if self.pal_status == PalStatus::NotInstalled && self.pal_install_button().contains(x, y) {
            return Hit::PalInstall;
        }
        if self.pal_shows_add_folder() && self.pal_install_button().contains(x, y) {
            return Hit::PalAddFolder;
        }
        let list = self.pal_list();
        if list.contains(x, y) {
            let i = ((y - list.t + self.pal_scroll) / PAL_ROW_H).floor();
            if i >= 0.0 && (i as usize) < self.pal_results.len() {
                return Hit::PalRow(i as usize);
            }
        }
        Hit::SidePanel
    }

    /// Ctrl+F: 닫혀 있으면 열고, 열려 있는데 키보드가 다른 곳이면 검색 상자로, 검색 상자에 있으면 닫는다
    pub(super) fn toggle_palette(&mut self) {
        if !self.pal_shown() {
            self.open_palette();
        } else if self.pal_focus {
            self.close_palette();
        } else {
            self.pal_focus_input();
        }
    }

    /// 검색 패널을 고정해 열고 키보드를 검색 상자에 둔다
    pub(super) fn open_palette(&mut self) {
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        // 설정 화면이 열려 있으면 닫는다 (그 뒤에 가려 입력할 수 없게 되지 않게)
        self.close_settings();
        if !self.pal_shown() {
            // 검색을 닫으면 전에 고정해 둔 패널로 돌아간다
            self.side_before_search = self.side_pinned.filter(|p| *p != Panel::Search);
            self.show_side(Panel::Search, true);
            self.pal_reset();
            self.pal_check_status();
        }
        self.pal_focus_input();
    }

    fn pal_reset(&mut self) {
        self.pal_results.clear();
        self.pal_gate = PalGate::default();
        self.pal_picked.clear();
        self.pal_sel = 0;
        self.pal_anchor = 0;
        self.pal_scroll = 0.0;
        self.pal_status = PalStatus::Idle;
    }

    /// 검색 상자 글자를 바꾼다 (세션·작업 공간을 되살릴 때). 앱을 다루는 동안 생긴 글자 바뀜 알림은 나중에야 처리되니
    /// (main.rs가 다시 넣는다) 지금 찾으려면 부르는 쪽이 찾는다
    pub(super) fn set_search_text(&self, q: &str) {
        if edit_text(self.search_edit) != q {
            unsafe {
                let _ = SetWindowTextW(self.search_edit, &HSTRING::from(q));
            }
        }
    }

    /// 작업 공간을 되살릴 때: 검색 패널을 고정해 열고 이 검색어·범위로 찾는다 (키보드는 옮기지 않는다)
    pub(super) fn restore_search(&mut self, query: &str, here: bool) {
        self.set_search_text(query);
        self.pal_here = here;
        if !self.pal_shown() {
            self.side_before_search = None;
            self.show_side(Panel::Search, true);
            self.pal_reset();
        }
        // 칸이 바뀌었으니 이미 열려 있었어도 "이 폴더도 검색하기" 대상을 다시 본다
        self.pal_check_status();
        self.pal_run_query();
    }

    /// 키보드를 검색 상자로. 지난 검색어는 통째로 선택한다: 새로 치면 바뀌고, 이어 쓰려면 End. 결과가 없으면 다시 찾아 채운다
    fn pal_focus_input(&mut self) {
        self.fav_blur();
        self.pal_focus = true;
        self.hover = Hit::None;
        self.layout_edits();
        self.focus();
        if !edit_text(self.search_edit).trim().is_empty() {
            unsafe {
                SendMessageW(self.search_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            }
            if !self.pal_current() {
                self.pal_run_query();
            }
        }
        self.invalidate();
    }

    /// 전역 단축키: 이미 열려 있으면 입력 상자에 키보드만
    pub fn open_palette_global(&mut self) {
        if self.pal_shown() {
            self.pal_focus_input();
        } else {
            self.open_palette();
        }
    }

    /// 검색 패널을 닫는다 (전에 고정해 둔 패널이 있으면 그 패널로 돌아간다). 정리는 side_changed → pal_hidden
    pub(super) fn close_palette(&mut self) {
        if !self.pal_shown() {
            return;
        }
        match self.side_before_search.take() {
            Some(p) => self.show_side(p, true),
            None => self.close_side(),
        }
    }

    /// 검색 패널이 닫히거나 다른 패널로 바뀌었다: 진행 중인 검색을 버리고 키보드를 칸으로
    pub(super) fn pal_hidden(&mut self) {
        self.pal_focus = false;
        self.side_before_search = None;
        self.pal_seq += 1;
        self.pal_gate.cancel();
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SEARCH);
            let _ = KillTimer(Some(self.hwnd), TIMER_SEARCH_READY);
        }
        self.focus();
    }

    /// Esc·Tab, 패널 밖 클릭: 키보드를 칸으로 돌려준다 (패널과 결과는 그대로)
    pub(super) fn pal_blur(&mut self) {
        self.pal_focus = false;
        self.focus();
        self.invalidate();
    }

    /// 서비스가 있는지·인덱스가 준비됐는지 확인하고, 보이는 칸 중 검색 범위 밖 폴더를 적어 둔다
    /// (패널을 열 때, 다시 켰는데 열려 있었을 때)
    pub(super) fn pal_check_status(&mut self) {
        if self.pal_shown() {
            self.pal_outside = self.visible_dirs().into_iter().find(|d| self.folders.covering(d).is_none() && crate::folders::outside_disk_index(d));
            self.pal_seq += 1;
            spawn_search(self.hwnd, self.pal_seq, None, None, self.folders.clone());
        }
    }

    /// 폴더가 바뀌거나 파일 작업이 끝났다: 같은 검색어로 잠시 뒤 다시 찾는다 (고른 줄은 경로로 이어 간다)
    pub(super) fn pal_requery(&mut self) {
        if self.pal_shown() && !edit_text(self.search_edit).trim().is_empty() {
            unsafe { SetTimer(Some(self.hwnd), TIMER_SEARCH_AGAIN, 700, None) };
        }
    }

    /// 창이나 패널 크기가 바뀌었다: 스크롤이 목록 끝을 넘지 않게
    pub(super) fn pal_clamp_scroll(&mut self) {
        let max = (self.pal_results.len() as f32 * PAL_ROW_H - self.pal_list().h()).max(0.0);
        self.pal_scroll = self.pal_scroll.clamp(0.0, max);
    }

    /// 입력이 바뀌면 잠깐 모았다가 검색 (빠르게 칠 때 질의를 줄인다).
    /// 바뀌는 즉시 보이는 결과는 이전 검색어의 것이 되므로 열기·복사를 막는다
    pub(super) fn pal_query_changed(&mut self) {
        if self.pal_shown() {
            self.pal_gate.edited();
            unsafe { SetTimer(Some(self.hwnd), TIMER_SEARCH, 40, None) };
            self.invalidate();
        }
    }

    /// 보이는 결과가 지금 입력한 검색어와 범위의 것인지
    fn pal_current(&self) -> bool {
        self.pal_gate.current(&self.pal_key_now())
    }

    /// "현재 폴더" 범위로 찾을 폴더 (범위가 PC 전체이거나 지금 칸에 폴더가 없으면 None)
    fn pal_scope(&self) -> Option<String> {
        if self.pal_here { self.panes[self.active].tab().loc.dir().map(str::to_string) } else { None }
    }

    /// 지금 입력과 범위의 열쇠 (pal_key)
    fn pal_key_now(&self) -> String {
        pal_key(&edit_text(self.search_edit), self.pal_scope().as_deref())
    }

    /// "현재 폴더" 범위로 찾는 중에 지금 칸의 폴더가 바뀌었으면(이동·탭·칸·칸 나누기) 그 폴더로 바로 다시 찾는다
    pub(super) fn pal_follow_scope(&mut self) {
        if !self.pal_here || !self.pal_shown() || edit_text(self.search_edit).trim().is_empty() {
            return;
        }
        if self.pal_gate.asked.as_deref() != Some(self.pal_key_now().as_str()) {
            self.pal_run_query();
        }
    }

    pub(super) fn pal_run_query(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SEARCH);
        }
        let q = edit_text(self.search_edit);
        self.pal_seq += 1;
        // "현재 폴더" 범위: 지금 칸의 폴더 아래만 (내 PC처럼 폴더가 없으면 전체)
        let scope = self.pal_scope();
        let key = pal_key(&q, scope.as_deref());
        self.pal_gate.asked(&key);
        if q.trim().is_empty() {
            self.pal_results.clear();
            self.pal_gate.arrived(&key, &key, 0);
            // 서비스 상태(미설치·인덱싱 중)는 그대로 보여 준다
            if matches!(self.pal_status, PalStatus::Searching | PalStatus::Done { .. }) {
                self.pal_status = PalStatus::Idle;
            }
            self.invalidate();
            return;
        }
        if self.pal_status != PalStatus::NotInstalled {
            self.pal_status = PalStatus::Searching;
        }
        spawn_search(self.hwnd, self.pal_seq, Some(q), scope, self.folders.clone());
        // 넣은 폴더를 아직 훑는 중이면, 다 훑을 때까지 잠깐마다 다시 찾아 결과를 채운다
        if self.folders.list().iter().any(|(_, s)| *s == crate::folders::State::Scanning) {
            unsafe { SetTimer(Some(self.hwnd), TIMER_SEARCH, 300, None) };
        }
        self.invalidate();
    }

    /// 검색 창의 "이 폴더도 검색하기": 넣고 다시 찾는다
    fn pal_add_folder(&mut self) {
        let Some(dir) = self.pal_outside.take() else { return };
        if let Err(e) = self.folders.add(&dir) {
            let p = self.active;
            self.flash(p, format!("검색 폴더 목록을 저장하지 못했습니다: {e}"));
        }
        self.pal_run_query();
    }

    pub(super) fn on_search_done(&mut self, done: SearchDone) {
        // 상태 확인 응답은 그 사이 입력이 있었어도 쓸모 있으므로 순번을 따지지 않는다
        let stale = !matches!(done.reply, SearchReply::Status(_)) && done.seq != self.pal_seq;
        if !self.pal_shown() || stale {
            return;
        }
        let waiting = matches!(self.pal_status, PalStatus::Done { ready: false, .. } | PalStatus::Indexing);
        match done.reply {
            // 준비 중에 보여 준 결과는 준비가 끝나면 지금 검색어로 다시 찾아 채운다
            SearchReply::Status(Ok((true, _))) if matches!(self.pal_status, PalStatus::Done { ready: false, .. }) => self.pal_run_query(),
            SearchReply::Status(Ok((true, n))) => {
                if matches!(self.pal_status, PalStatus::Idle | PalStatus::NotInstalled | PalStatus::Indexing) {
                    self.pal_status = PalStatus::Ready(n);
                }
                if waiting && !edit_text(self.search_edit).trim().is_empty() {
                    self.pal_run_query();
                }
            }
            SearchReply::Status(Ok((false, _))) => {
                if !matches!(self.pal_status, PalStatus::Done { .. } | PalStatus::Searching) {
                    self.pal_status = PalStatus::Indexing;
                }
                self.pal_poll_ready();
            }
            SearchReply::Results(Ok(r)) => {
                // 훑는 중 다시 찾아 결과가 늘어날 때도 골라 둔 항목은 경로로 이어 간다
                let kept: HashSet<String> = self.pal_picked_paths().into_iter().map(|(_, p)| p.to_lowercase()).collect();
                self.pal_picked = r.hits.iter().map(|(_, p)| kept.contains(&p.to_lowercase())).collect();
                let key = pal_key(done.query.as_deref().unwrap_or_default(), done.scope.as_deref());
                let enter = self.pal_gate.arrived(&key, &self.pal_key_now(), r.hits.len());
                // 다시 찾은 결과(파일 작업 뒤 등)에 지금 줄이 남아 있으면 그 줄과 스크롤을 지킨다
                let sel_path = self.pal_results.get(self.pal_sel).map(|(_, p)| p.to_lowercase());
                self.pal_results = r.hits;
                self.pal_status = PalStatus::Done { total: r.total, micros: r.micros, ready: r.ready, failed: r.failed };
                self.pal_sel = sel_path.and_then(|s| self.pal_results.iter().position(|(_, p)| p.to_lowercase() == s)).unwrap_or(0);
                self.pal_anchor = self.pal_sel;
                if self.pal_sel == 0 {
                    self.pal_scroll = 0.0;
                }
                self.pal_ensure_visible();
                if !r.ready {
                    self.pal_poll_ready();
                }
                // 결과보다 먼저 누른 Enter (같은 검색어의 결과가 있을 때 한 번만)
                if let Some(mode) = enter {
                    self.pal_open_paths(self.pal_targets(), mode);
                }
            }
            SearchReply::Status(Err(SearchError::NotRunning)) | SearchReply::Results(Err(SearchError::NotRunning)) => {
                self.pal_gate.cancel();
                self.pal_results.clear();
                self.pal_status = PalStatus::NotInstalled;
            }
            SearchReply::Status(Err(SearchError::Other(e))) | SearchReply::Results(Err(SearchError::Other(e))) => {
                crate::diag::log(|| format!("search: {e}"));
                self.pal_gate.cancel();
                self.pal_status = PalStatus::Error(e);
            }
        }
        self.invalidate();
    }

    /// 서비스가 준비 중이면 잠시 뒤 준비됐는지 다시 묻는다
    fn pal_poll_ready(&mut self) {
        unsafe { SetTimer(Some(self.hwnd), TIMER_SEARCH_READY, 1000, None) };
    }

    pub(super) fn pal_ready_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SEARCH_READY);
        }
        if self.pal_shown() {
            spawn_search(self.hwnd, self.pal_seq, None, None, self.folders.clone());
        }
    }

    fn pal_ensure_visible(&mut self) {
        let list = self.pal_list();
        let y = self.pal_sel as f32 * PAL_ROW_H;
        if y < self.pal_scroll {
            self.pal_scroll = y;
        } else if y + PAL_ROW_H > self.pal_scroll + list.h() {
            self.pal_scroll = y + PAL_ROW_H - list.h();
        }
        let max = (self.pal_results.len() as f32 * PAL_ROW_H - list.h()).max(0.0);
        self.pal_scroll = self.pal_scroll.clamp(0.0, max);
    }

    pub(super) fn pal_wheel(&mut self, dy: f32) {
        let list = self.pal_list();
        let max = (self.pal_results.len() as f32 * PAL_ROW_H - list.h()).max(0.0);
        self.pal_scroll = (self.pal_scroll + dy).clamp(0.0, max);
        self.invalidate();
    }

    // ---------- 여러 결과 고르기 ----------

    /// 골라 둔 결과 (결과 순서)
    fn pal_picked_paths(&self) -> Vec<(bool, String)> {
        self.pal_results.iter().zip(&self.pal_picked).filter(|(_, picked)| **picked).map(|(r, _)| r.clone()).collect()
    }

    /// 다룰 결과: 골라 둔 것이 있으면 그것들, 없으면 지금 줄 하나.
    /// 입력을 바꾼 뒤 아직 새 결과가 오지 않았으면 없음 (이전 검색어의 파일을 열거나 복사하지 않게)
    fn pal_targets(&self) -> Vec<(bool, String)> {
        if !self.pal_current() {
            return Vec::new();
        }
        let picked = self.pal_picked_paths();
        if !picked.is_empty() {
            return picked;
        }
        self.pal_results.get(self.pal_sel).cloned().into_iter().collect()
    }

    fn pal_pick_count(&self) -> usize {
        self.pal_picked.iter().filter(|p| **p).count()
    }

    fn pal_pick_range(&mut self, a: usize, b: usize) {
        self.pal_picked.resize(self.pal_results.len(), false);
        for i in a.min(b)..=a.max(b).min(self.pal_results.len().saturating_sub(1)) {
            self.pal_picked[i] = true;
        }
    }

    fn pal_clear_picks(&mut self) {
        self.pal_picked.iter_mut().for_each(|p| *p = false);
    }

    pub(super) fn pal_activate(&mut self, i: usize, mode: OpenMode) {
        if !self.pal_current() {
            return;
        }
        let Some(target) = self.pal_results.get(i).cloned() else { return };
        self.pal_open_paths(vec![target], mode);
    }

    /// 결과를 연다. 패널과 결과는 그대로 둔다.
    /// 위치 열기(Ctrl+Enter)는 지금 칸에서 그 폴더를 열고 항목을 골라 두고(폴더가 여럿이면 둘째부터 새 탭),
    /// Shift+Enter는 새 탭에서 연다. 같은 폴더의 항목은 한 탭에 모은다
    fn pal_open_paths(&mut self, targets: Vec<(bool, String)>, mode: OpenMode) {
        if targets.is_empty() {
            return;
        }
        let p = self.active;
        let many = targets.len() > 1;
        let files = targets.iter().filter(|(d, _)| !*d).count();
        if mode == OpenMode::Open && files > OPEN_ASK_OVER {
            let paths: Vec<String> = targets.iter().filter(|(d, _)| !*d).map(|(_, p)| p.clone()).collect();
            self.pending.push(Deferred::ConfirmOpen { pane: p, paths });
            for (_, path) in targets.into_iter().filter(|(d, _)| *d) {
                self.new_tab(p, Some(Location::Dir(path)), true);
            }
            return;
        }
        // (폴더, 고를 이름들)
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for (is_dir, path) in targets {
            let parent = util::parent(&path);
            match (is_dir, mode) {
                (true, OpenMode::Open) if !many => self.navigate(p, Location::Dir(path), None),
                (true, OpenMode::Open | OpenMode::NewTab) => self.new_tab(p, Some(Location::Dir(path)), true),
                (false, OpenMode::Open) => self.pending.push(Deferred::Open { pane: p, paths: vec![path], dir: parent }),
                _ => {
                    let Some(dir) = parent else { continue };
                    let name = util::file_name(&path).to_lowercase();
                    match groups.iter_mut().find(|(d, _)| util::eq_path(d, &dir)) {
                        Some((_, names)) => names.push(name),
                        None => groups.push((dir, vec![name])),
                    }
                }
            }
        }
        let mut in_place = mode == OpenMode::Locate;
        for (dir, names) in groups {
            if std::mem::take(&mut in_place) {
                self.navigate(p, Location::Dir(dir), None);
            } else {
                self.new_tab(p, Some(Location::Dir(dir)), true);
            }
            self.panes[p].tab_mut().restore = Some(Restore { cursor: names.first().cloned(), selected: names, reveal: true, ..Default::default() });
        }
        // 위치를 열었으면 바로 그 칸에서 일할 수 있게 키보드를 칸으로
        if mode != OpenMode::Open {
            self.pal_blur();
        }
    }

    /// 고른 결과(없으면 지금 줄)를 파일째 클립보드에 복사한다. 탐색기나 다른 칸에 붙여넣으면 복사된다
    fn pal_copy_files(&mut self) {
        let paths: Vec<String> = self.pal_targets().into_iter().map(|(_, p)| p).collect();
        if paths.is_empty() {
            return;
        }
        let n = paths.len();
        let p = self.active;
        match shell::set_clipboard_files(self.hwnd, &paths, false) {
            Ok(()) => self.flash(p, format!("검색 결과 {n}개를 복사했습니다. 붙여넣을 폴더에서 Ctrl+V")),
            Err(e) => self.flash(p, format!("클립보드에 복사하지 못했습니다: {}", e.message())),
        }
    }

    fn pal_copy_paths(&mut self) {
        let text = self.pal_targets().into_iter().map(|(_, p)| p).collect::<Vec<_>>().join("\r\n");
        if text.is_empty() {
            return;
        }
        let p = self.active;
        if let Err(e) = shell::set_clipboard_text(self.hwnd, &text) {
            self.flash(p, format!("클립보드에 복사하지 못했습니다: {}", e.message()));
        }
    }

    /// 검색 결과 오른쪽 버튼 메뉴에서 고른 것
    pub fn pal_menu_command(&mut self, cmd: u32) {
        match cmd {
            PAL_MENU_OPEN => self.pal_open_paths(self.pal_targets(), OpenMode::Open),
            PAL_MENU_LOCATE => self.pal_open_paths(self.pal_targets(), OpenMode::Locate),
            PAL_MENU_COPY => self.pal_copy_files(),
            PAL_MENU_COPY_PATH => self.pal_copy_paths(),
            PAL_MENU_PICK_ALL => {
                let n = self.pal_results.len();
                self.pal_pick_range(0, n.saturating_sub(1));
            }
            PAL_MENU_CLEAR => self.pal_clear_picks(),
            _ => {}
        }
        self.invalidate();
    }

    /// 오른쪽 버튼을 누른 줄: 골라 둔 것 밖이면 그 줄만 고른다 (탐색기와 같음)
    pub(super) fn pal_right_press(&mut self, i: usize) {
        if !self.pal_picked.get(i).copied().unwrap_or(false) {
            self.pal_clear_picks();
        }
        self.pal_sel = i;
        self.pal_anchor = i;
        self.invalidate();
    }

    fn pal_install(&mut self) {
        // 관리자 권한(UAC)으로 같은 실행 파일을 설치 모드로 띄운다
        if let Ok(me) = std::env::current_exe() {
            unsafe {
                ShellExecuteW(Some(self.hwnd), w!("runas"), &HSTRING::from(me.as_os_str()), w!("--install-service"), PCWSTR::null(), SW_SHOWNORMAL);
            }
        }
        // 설치가 끝나면 다시 확인할 수 있게 상태를 비운다
        self.pal_status = PalStatus::Idle;
        self.pal_seq += 1;
        spawn_search(self.hwnd, self.pal_seq, None, None, self.folders.clone());
        self.invalidate();
    }

    /// 검색 상자에서 누른 키. 처리했으면 true
    pub(super) fn pal_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let n = self.pal_results.len();
        let page = ((self.pal_list().h() / PAL_ROW_H).floor() as usize).max(1);
        let picks = self.pal_pick_count();
        match vk {
            // 골라 둔 게 있으면 먼저 그걸 풀고, 아니면 키보드를 칸으로 (패널은 그대로)
            VK_ESCAPE if picks > 0 => self.pal_clear_picks(),
            VK_ESCAPE | VK_TAB => self.pal_blur(),
            VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT => {
                let s = self.pal_sel;
                self.pal_sel = match vk {
                    VK_UP => s.saturating_sub(1),
                    VK_DOWN => (s + 1).min(n.saturating_sub(1)),
                    VK_PRIOR => s.saturating_sub(page),
                    _ => (s + page).min(n.saturating_sub(1)),
                };
                // Shift를 누르고 움직이면 시작 줄부터 지금 줄까지 고른다
                if shift && n > 0 {
                    self.pal_clear_picks();
                    self.pal_pick_range(self.pal_anchor, self.pal_sel);
                } else {
                    self.pal_anchor = self.pal_sel;
                }
            }
            VK_RETURN => {
                let mode = if ctrl { OpenMode::Locate } else if shift { OpenMode::NewTab } else { OpenMode::Open };
                let key = self.pal_key_now();
                match self.pal_gate.enter(&key, mode) {
                    Some(mode) => self.pal_open_paths(self.pal_targets(), mode),
                    // 결과가 아직 이전 검색어(또는 범위)의 것: 지금 것으로 바로 찾고, 결과가 오면 그때 연다
                    None if self.pal_gate.asked.as_deref() != Some(key.as_str()) => self.pal_run_query(),
                    None => {}
                }
                return true;
            }
            // 검색 단축키를 한 번 더 누르면 패널을 닫고, 설정·도움말·즐겨찾기 단축키는 그쪽으로
            _ if self.keys.find(crate::keys::Combo::now(vk.0)).is_some_and(|a| {
                use crate::keys::Action;
                matches!(a, Action::Search | Action::Settings | Action::Help | Action::Favorites)
            }) =>
            {
                match self.keys.find(crate::keys::Combo::now(vk.0)) {
                    Some(crate::keys::Action::Search) => self.close_palette(),
                    Some(a) => self.run_action(a),
                    None => {}
                }
            }
            // 고른 결과가 있거나 검색어에 선택한 글자가 없으면 파일을 복사한다 (그 밖에는 글자 복사)
            _ if ctrl && vk.0 == b'C' as u16 && n > 0 && (picks > 0 || !edit_has_selection(self.search_edit)) => self.pal_copy_files(),
            _ => return false,
        }
        if self.pal_shown() {
            self.pal_ensure_visible();
        }
        self.invalidate();
        true
    }

    /// 검색 패널 안을 왼쪽 버튼으로 눌렀다
    pub(super) fn pal_mouse_down(&mut self, hit: Hit) {
        match hit {
            // Ctrl+클릭은 하나씩, Shift+클릭은 범위로 고르고, 그냥 클릭은 바로 연다
            Hit::PalRow(i) if key_down(VK_CONTROL) => {
                self.pal_picked.resize(self.pal_results.len(), false);
                if let Some(p) = self.pal_picked.get_mut(i) {
                    *p = !*p;
                }
                self.pal_sel = i;
                self.pal_anchor = i;
            }
            Hit::PalRow(i) if key_down(VK_SHIFT) => {
                self.pal_clear_picks();
                self.pal_pick_range(self.pal_anchor, i);
                self.pal_sel = i;
            }
            Hit::PalRow(i) => self.pal_activate(i, OpenMode::Open),
            Hit::PalInstall => self.pal_install(),
            Hit::PalAddFolder => self.pal_add_folder(),
            // 범위를 바꾸면 같은 검색어로 다시 찾는다 (칸에 폴더가 없으면 현재 폴더는 고를 수 없다)
            Hit::PalScope(here) if here != self.pal_here && (!here || self.panes[self.active].tab().loc.dir().is_some()) => {
                self.pal_here = here;
                self.pal_run_query();
            }
            _ => {}
        }
        // 결과를 누르면 키보드도 검색 쪽으로 (↑↓로 이어서 고르게)
        if matches!(hit, Hit::PalRow(_)) && !self.pal_focus {
            self.pal_focus = true;
            self.focus();
        }
    }

    pub(super) fn paint_search_panel(&mut self) {
        let t = self.theme;
        let r = self.pal_rect();
        let list = self.pal_list();
        let input = self.pal_input_box();
        let install = self.pal_install_button();
        let close = self.pal_close_rect();
        let scopes = [(false, self.pal_scope_rect(false)), (true, self.pal_scope_rect(true))];
        let here = self.pal_here;
        let here_ok = self.panes[self.active].tab().loc.dir().is_some();
        let (hover, sel, scroll, focus) = (self.hover, self.pal_sel, self.pal_scroll, self.pal_focus);
        let status = self.pal_status.clone();
        let query = edit_text(self.search_edit);
        let add_folder = if self.pal_shows_add_folder() { self.pal_outside.clone() } else { None };
        let folder_entries = self.folders.entry_count();
        let picks = self.pal_pick_count();
        let help = self.keys.get(crate::keys::Action::Help).map(crate::keys::Combo::label);
        // 입력을 바꿨는데 새 결과가 아직이면 이전 결과는 흐리게 (열거나 복사할 수 없음)
        let stale = !self.pal_current() && !self.pal_results.is_empty();
        if crate::diag::enabled() && matches!(status, PalStatus::Done { .. }) {
            let key = self.pal_key_now();
            if let Some(d) = self.pal_gate.shown_after_typing(&key) {
                let pieces = query_pieces(&query).len();
                crate::diag::log(|| format!("search: {pieces} piece(s), {} hit(s) shown {:.1}ms after typing", self.pal_results.len(), d.as_secs_f64() * 1000.0));
            }
        }
        let App { gfx, pal_results, pal_picked, types, .. } = self;

        // 머리: 제목, 닫기
        gfx.text("검색", R::new(r.l + 18.0, r.t, close.l - 4.0, r.t + 44.0), Font::Strong, t.text);
        if hover == Hit::PalClose {
            gfx.fill_round(close, 6.0, t.hover);
        }
        gfx.glyph(G_CLOSE, close, true, if hover == Hit::PalClose { t.text } else { t.dim });

        // 입력 줄: 들어간 상자 (실제 입력은 위에 놓인 EDIT). 키보드가 여기 있으면 강조 테두리
        gfx.fill_round(input, 8.0, if t.high_contrast { t.window } else { t.field });
        gfx.stroke_round(input.inset(0.75, 0.75), 8.0, if focus { t.accent } else { t.border }, if focus { 1.5 } else { 1.0 });
        gfx.glyph_font(G_SEARCH, R::new(input.l + 8.0, input.t, input.l + 28.0, input.b), Font::GlyphMid, if focus { t.accent } else { t.dim });

        // 범위 칩: PC 전체 · 현재 폴더 (칸에 폴더가 없으면 현재 폴더는 흐리게, 전체로 찾는다)
        for (h, cr) in scopes {
            let enabled = !h || here_ok;
            let on = if here_ok { h == here } else { !h };
            if on {
                gfx.fill_round(cr, 12.0, t.accent_soft);
            } else if hover == Hit::PalScope(h) && enabled {
                gfx.fill_round(cr, 12.0, t.hover);
            } else {
                gfx.stroke_round(cr.inset(0.5, 0.5), 12.0, t.border, 1.0);
            }
            let label = if h { "현재 폴더" } else { "PC 전체" };
            let w = gfx.measure(label, Font::Small);
            let fg = if on { t.chip_text } else if enabled { t.dim } else { t.faint };
            gfx.text(label, R::new(cr.l + (cr.w() - w) / 2.0, cr.t, cr.r, cr.b), Font::Small, fg);
        }

        gfx.clip(list);
        let message: Option<String> = match &status {
            PalStatus::NotInstalled => Some("검색 서비스가 설치되어 있지 않습니다\n디스크 전체를 바로 찾으려면 관리자 권한으로 한 번 설치합니다".into()),
            PalStatus::Indexing => Some("검색 서비스가 디스크 목록을 만드는 중입니다\n준비되면 입력한 검색어로 결과를 채웁니다".into()),
            PalStatus::Error(e) => Some(format!("검색 서비스와 통신하지 못했습니다\n{e}")),
            // 준비 중·읽지 못한 드라이브 때문에 비어 있는 것은 "일치하는 항목 없음"과 다르게 알린다
            PalStatus::Done { ready: false, .. } if pal_results.is_empty() => Some(format!(
                "검색 서비스가 디스크 목록을 아직 만드는 중입니다\n준비되면 ‘{}’ 결과를 다시 채웁니다",
                query.trim()
            )),
            PalStatus::Done { failed, .. } if pal_results.is_empty() && !failed.is_empty() => Some(format!(
                "‘{}’와(과) 일치하는 항목이 없습니다\n{} 드라이브는 읽지 못해 찾지 못했습니다",
                query.trim(),
                drive_list(failed)
            )),
            PalStatus::Done { .. } if pal_results.is_empty() && add_folder.is_some() => Some(format!(
                "‘{}’와(과) 일치하는 항목이 없습니다\n열어 둔 {}은(는) 아직 검색 범위 밖입니다. 넣으면 그 안도 함께 찾습니다",
                query.trim(),
                add_folder.as_deref().unwrap_or_default()
            )),
            PalStatus::Done { .. } if pal_results.is_empty() && here && here_ok => Some(format!("지금 폴더 아래에 ‘{}’와(과) 일치하는 항목이 없습니다", query.trim())),
            PalStatus::Done { .. } if pal_results.is_empty() => Some(format!("‘{}’와(과) 일치하는 항목이 없습니다", query.trim())),
            PalStatus::Idle | PalStatus::Ready(_) if query.trim().is_empty() => Some(format!(
                "파일·폴더 이름을 입력하세요\n띄어 쓰면 모두 포함 · | 는 또는 · 끝에 \\ 는 경로{}",
                help.as_deref().map(|h| format!(" · 자세히는 {h}")).unwrap_or_default()
            )),
            _ => None,
        };
        if let Some(m) = message {
            gfx.text(&m, R::new(list.l + 10.0, list.t + 14.0, list.r - 10.0, list.t + 142.0), Font::Hint, t.faint);
        }
        let button = match (&status, &add_folder) {
            (PalStatus::NotInstalled, _) => Some(("검색 서비스 설치", Hit::PalInstall)),
            (_, Some(_)) => Some(("이 폴더도 검색하기", Hit::PalAddFolder)),
            _ => None,
        };
        if let Some((label, h)) = button {
            gfx.fill_round(install, 8.0, if hover == h { t.accent } else { alpha(t.accent, 0.85) });
            gfx.text(label, install, Font::Hint, Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 });
        }
        let first = (scroll / PAL_ROW_H).floor().max(0.0) as usize;
        let count = (list.h() / PAL_ROW_H).ceil() as usize + 1;
        for i in first..(first + count).min(pal_results.len()) {
            let (is_dir, path) = &pal_results[i];
            let y = list.t + i as f32 * PAL_ROW_H - scroll;
            let row = R::new(list.l, y, list.r, y + PAL_ROW_H);
            // 지금 줄은 옅은 강조 색, 골라 둔 줄은 더 옅은 강조 색과 왼쪽 체크
            let picked = pal_picked.get(i).copied().unwrap_or(false);
            let current = i == sel;
            if current {
                gfx.fill_round(row, 6.0, t.accent_soft);
            } else if picked {
                gfx.fill_round(row, 6.0, alpha(t.accent, if t.dark { 0.12 } else { 0.07 }));
            } else if hover == Hit::PalRow(i) {
                gfx.fill_round(row, 6.0, t.hover);
            }
            if t.high_contrast && (current || picked) {
                gfx.stroke_round(row.inset(0.75, 0.75), 6.0, if current { t.focus } else { t.border }, 1.5);
            }
            let text_r = if picked { row.r - 26.0 } else { row.r - 8.0 };
            if picked {
                gfx.glyph(G_CHECK, R::new(row.r - 22.0, y, row.r - 8.0, y + PAL_ROW_H), true, t.accent);
            }
            let name = util::file_name(path);
            let ext = if *is_dir { String::new() } else { name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default() };
            let key = if *is_dir { "\\dir".to_string() } else { ext.clone() };
            let icon = types.entry(key).or_insert_with(|| shell::type_info(&ext, *is_dir)).1;
            gfx.icon(icon, R::new(row.l + 10.0, y + 7.0, row.l + 26.0, y + 23.0));
            let (font, fg) = if current && !t.high_contrast { (Font::Strong, t.chip_text) } else { (Font::Normal, t.text) };
            gfx.text(name, R::new(row.l + 36.0, y + 5.0, text_r, y + 24.0), font, fg);
            // 같은 이름의 파일을 폴더로 구별할 수 있게 경로는 흐린 색이 아닌 보조 글자색으로 (넘치면 가운데를 줄인다)
            let parent = util::parent(path).unwrap_or_default();
            gfx.text(&parent, R::new(row.l + 36.0, y + 24.0, text_r, y + 42.0), Font::Path, t.dim);
        }
        if stale {
            gfx.fill(list, alpha(t.pane, 0.6));
        }
        gfx.unclip();

        // 아래: 결과 수 (왼쪽), 키 안내 (오른쪽, 자리가 있을 때)
        let foot = R::new(r.l + 18.0, r.b - S_FOOT_H, r.r - 12.0, r.b);
        gfx.fill(R::new(r.l + 8.0, foot.t, r.r - 8.0, foot.t + 1.0), t.border);
        let left = match &status {
            _ if stale => "찾는 중…".to_string(),
            PalStatus::Done { ready: false, .. } => format!("준비 중 · {}개 먼저", pal_results.len()),
            PalStatus::Done { total, failed, .. } => {
                let shown = pal_results.len();
                let skipped = if failed.is_empty() { String::new() } else { format!(" · {} 빠짐", drive_list(failed)) };
                if *total > shown { format!("{total}개 중 {shown}개{skipped}") } else { format!("{shown}개 결과{skipped}") }
            }
            PalStatus::Searching => "찾는 중…".to_string(),
            PalStatus::Ready(n) if folder_entries > 0 => format!("인덱스 {n}개 · 넣은 폴더 {folder_entries}개"),
            PalStatus::Ready(n) => format!("인덱스 {n}개"),
            _ => String::new(),
        };
        let left = if picks > 0 { format!("{picks}개 고름") } else { left };
        let lw = gfx.measure(&left, Font::Small);
        gfx.text(&left, R::new(foot.l, foot.t, foot.l + lw + 2.0, foot.b), Font::Small, if picks > 0 { t.chip_text } else { t.faint });
        // 키 안내: 고른 게 있으면 Enter 모두 열기, 아니면 Ctrl+Enter 위치 열기
        let (keys, label): (Vec<&str>, &str) = if picks > 0 { (vec!["Enter"], "모두 열기") } else { (vec!["Ctrl+Enter"], "위치 열기") };
        let need = keys.iter().map(|k| gfx.measure(k, Font::Small) + 14.0).sum::<f32>() + gfx.measure(label, Font::Small) + 6.0;
        if foot.l + lw + 16.0 + need <= foot.r {
            let cy = (foot.t + foot.b) / 2.0;
            let mut x = foot.r - need;
            for k in &keys {
                x = kbd(gfx, &t, x, cy, k) + 4.0;
            }
            gfx.text(label, R::new(x + 2.0, foot.t, foot.r + 2.0, foot.b), Font::Small, t.dim);
        }
    }
}

/// 키 모양 작은 칩. 오른쪽 끝 x를 돌려준다
fn kbd(gfx: &Gfx, t: &Theme, x: f32, cy: f32, label: &str) -> f32 {
    let w = gfx.measure(label, Font::Small) + 10.0;
    let r = R::new(x, cy - 10.0, x + w, cy + 10.0);
    gfx.fill_round(r, 4.0, super::sidebar::well(t));
    gfx.text(label, R::new(r.l + 5.0, r.t, r.r, r.b), Font::Small, t.text);
    r.r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 현재 폴더 범위는 검색어 조각마다 경로 조건('\'가 든 낱말)으로 붙는다. 띄어쓰기 뒤 '\'가 없는 조각은 빼고(이름 조건이 되므로)
    /// 정확한 범위는 받은 뒤 거른다
    #[test]
    fn scope_becomes_path_terms() {
        assert_eq!(scoped_query("보고서", r"C:\업무\월간"), r"보고서 C:\업무\월간\");
        assert_eq!(scoped_query("보고서", r"C:\업무\월간\"), r"보고서 C:\업무\월간\");
        assert_eq!(scoped_query("a | b c", r"C:\Sakadu 데모\사진"), r"a C:\Sakadu 데모\사진\ | b c C:\Sakadu 데모\사진\");
        assert_eq!(scoped_query("x", r"C:\My Big Folder\월간"), r"x C:\My Folder\월간\");
        assert_eq!(scoped_query("x", r"\\nas\공유\팀"), r"x \\nas\공유\팀\");
        assert_eq!(scoped_query("x", r"C:\"), r"x C:\");
        // 범위가 다르면 같은 검색어여도 다른 결과로 본다
        assert_ne!(pal_key("보고", Some(r"C:\a")), pal_key("보고", None));
        assert_ne!(pal_key("보고", Some(r"C:\a")), pal_key("보고", Some(r"C:\b")));
        assert_eq!(pal_key("보고", Some(r"C:\A")), pal_key("보고", Some(r"c:\a")));
    }

    #[test]
    fn pieces_split_on_bar() {
        assert_eq!(query_pieces("견적 2026 | 계약서 |  | 0420"), vec!["견적 2026", "계약서", "0420"]);
        assert_eq!(query_pieces("업무\\ 보고"), vec!["업무\\ 보고"]);
        assert!(query_pieces(" | ").is_empty());
    }

    /// 실제 검색 서비스로 "또는" 검색: 조각별 결과가 합쳐지고 겹치는 항목은 한 번만 (cargo test or_search -- --ignored --nocapture)
    #[test]
    #[ignore]
    fn or_search_against_service() {
        let folders = crate::folders::Folders::default();
        let count = |q: &str| search_all(q, &folders, PAL_LIMIT).map(|r| (r.hits.len(), r.total)).unwrap_or((0, 0));
        let (a, b, both) = (count("main.rs sakadu\\src\\"), count("tray.rs sakadu\\src\\"), count("main.rs sakadu\\src\\ | tray.rs sakadu\\src\\"));
        println!("main.rs {a:?}, tray.rs {b:?}, either {both:?}");
        assert_eq!(both.0, a.0 + b.0);
        let same = count("tray.rs sakadu\\src\\ | tray.rs sakadu\\src\\");
        assert_eq!(same.0, b.0, "same piece twice should not duplicate hits");
    }

    /// 회귀: ‘견적’ 결과가 떠 있을 때 ‘계약’으로 바꾸고 바로 Enter를 눌러도 이전 파일을 열지 않는다
    #[test]
    fn enter_right_after_query_change_waits_for_new_results() {
        let mut g = PalGate::default();
        assert_eq!(g.arrived("견적", "견적", 3), None);
        assert!(g.current("견적"));
        assert_eq!(g.enter("견적", OpenMode::Open), Some(OpenMode::Open), "results match: open now");

        // 입력이 바뀌는 즉시 이전 결과는 지금 검색어의 것이 아니다 (열기·복사 막힘)
        g.edited();
        assert!(!g.current("계약"));
        assert_eq!(g.enter("계약", OpenMode::Locate), None, "wait for 계약 results");
        // 늦게 도착한 같은 검색어 결과에서 한 번만 처리
        assert_eq!(g.arrived("계약", "계약", 2), Some(OpenMode::Locate));
        assert_eq!(g.arrived("계약", "계약", 2), None, "only once");

        // Enter 뒤 다시 입력했으면 버린다
        g.edited();
        assert_eq!(g.enter("계약서", OpenMode::Open), None);
        g.edited();
        assert_eq!(g.arrived("계약서", "계약서 2026", 4), None);
        // 결과가 0건이면 실행하지 않는다
        assert_eq!(g.enter("없는이름", OpenMode::Open), None);
        assert_eq!(g.arrived("없는이름", "없는이름", 0), None);
        // 창을 닫았으면 버린다
        g.edited();
        assert_eq!(g.enter("보고서", OpenMode::Open), None);
        g.cancel();
        assert_eq!(g.arrived("보고서", "보고서", 5), None);
    }

    #[test]
    fn repeated_pieces_are_searched_once() {
        assert_eq!(unique_pieces("견적 | 계약 | 견적 |  견적  | 계약서 | 견적"), vec!["견적", "계약", "계약서"]);
        assert_eq!(unique_pieces("견적  2026 | 견적 2026 | ABC | abc"), vec!["견적  2026", "ABC"]);
    }

    /// 실제 검색 서비스: 같은 줄이 되풀이된 여러 줄 검색도 결과·순서가 한 번씩 찾은 것과 같다 (cargo test dedup_keeps -- --ignored)
    #[test]
    #[ignore]
    fn dedup_keeps_results_and_order() {
        let folders = crate::folders::Folders::default();
        let hits = |q: &str| search_all(q, &folders, PAL_LIMIT).map(|r| r.hits).unwrap_or_default();
        let once = hits("main.rs sakadu\\src\\ | tray.rs sakadu\\src\\ | keys.rs sakadu\\src\\");
        let repeated = hits("main.rs sakadu\\src\\ | tray.rs sakadu\\src\\ | MAIN.RS sakadu\\src\\ | keys.rs sakadu\\src\\ | tray.rs  sakadu\\src\\");
        println!("{} hit(s)", once.len());
        assert!(!once.is_empty());
        assert_eq!(once, repeated);
    }

    #[test]
    fn scope_keeps_only_paths_under_the_folder() {
        assert!(under("C:\\업무\\견적.xlsx", "c:\\업무"));
        assert!(under("C:\\OneDrive - Personal\\a b\\x.txt", "C:\\OneDrive - Personal\\a b"));
        assert!(under("C:\\업무\\2026\\견적.xlsx", "C:\\업무\\"));
        assert!(!under("C:\\업무2\\견적.xlsx", "C:\\업무"));
        assert!(!under("C:\\업무", "C:\\업무"));
    }

    #[test]
    fn pasted_lines_become_alternatives() {
        let excel = "20260420_160559\r\n20260407_091344\r\n\r\n  견적서 \r\n";
        assert_eq!(lines_to_query(excel), "20260420_160559 | 20260407_091344 | 견적서");
    }
}
