//! Ctrl+F 검색 팔레트: 검색 서비스(인덱스)와 검색에 넣은 폴더에서 이름으로 바로 찾는다

use super::*;
use crate::service::{self, SearchError, SearchResponse};
use windows::Win32::UI::Controls::EM_GETSEL;
use windows::Win32::UI::Shell::ShellExecuteW;

const PAL_W: f32 = 760.0;
const PAL_HEAD_H: f32 = 52.0;
const PAL_FOOT_H: f32 = 30.0;
const PAL_ROW_H: f32 = 34.0;
const PAL_MAX_ROWS: usize = 12;
const PAL_LIMIT: usize = 200;
pub(super) const G_SEARCH: u16 = 0xE721;
pub(super) const TIMER_SEARCH: usize = 20;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PalStatus {
    Idle,
    Searching,
    Done { total: usize, micros: u64 },
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
    reply: SearchReply,
}

/// 도구 막대 검색 상자의 너비 (창이 좁으면 줄어든다)
pub(super) const SEARCH_W: f32 = 300.0;

impl App {
    pub(super) fn search_button(&self) -> R {
        let l = self.fav_button().r + 6.0;
        let r = (l + SEARCH_W).min(self.preview_button().l - 16.0).max(l + 140.0);
        R::new(l, 8.0, r, 38.0)
    }
}

fn spawn_search(hwnd: HWND, seq: u64, query: Option<String>, folders: crate::folders::Folders) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let reply = match query {
            Some(q) => SearchReply::Results(search_all(&q, &folders)),
            None => SearchReply::Status(service::status()),
        };
        post(owner.get(), WM_APP_SEARCH, Box::new(SearchDone { seq, reply }));
    });
}

/// 검색어를 `|`로 나눈 조각들(또는). 조각 안의 띄어 쓴 낱말은 모두 포함해야 한다
pub(super) fn query_pieces(q: &str) -> Vec<&str> {
    q.split('|').map(str::trim).filter(|p| !p.is_empty()).collect()
}

/// 여러 줄 텍스트(엑셀 열 등)를 한 줄 검색어로: 줄마다 `|`로 잇는다
pub(super) fn lines_to_query(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" | ")
}

/// 한 조각을 디스크 인덱스(검색 서비스)와 검색에 넣은 폴더에서 찾아 (점수, 폴더 여부, 경로), 전체 개수, 걸린 시간
fn search_piece(q: &str, folders: &crate::folders::Folders) -> Result<(Vec<(i32, bool, String)>, usize, u64), SearchError> {
    let local = service::query(q, PAL_LIMIT);
    let started = std::time::Instant::now();
    let query = crate::index::Query::parse(q);
    let (extra, extra_total) = folders.search(&query, PAL_LIMIT);
    // 서비스가 없거나 실패해도 폴더 쪽 결과는 보여 준다
    let (hits, total, micros) = match local {
        Ok(r) => (r.hits, r.total, r.micros),
        Err(e) if extra_total > 0 => {
            if let SearchError::Other(e) = e {
                crate::diag::log(|| format!("search: {e}"));
            }
            (Vec::new(), 0, 0)
        }
        Err(e) => return Err(e),
    };
    // 서비스 결과에도 같은 기준으로 점수를 매겨 폴더 결과·다른 조각 결과와 섞는다
    let all = hits
        .into_iter()
        .map(|(is_dir, path)| (query.name_score(util::file_name(&path), is_dir).and_then(|s| query.path_score(s, &path)).unwrap_or(i32::MIN), is_dir, path))
        .chain(extra.into_iter().map(|h| (h.score, h.is_dir, h.path)))
        .collect();
    Ok((all, total + extra_total, micros + started.elapsed().as_micros() as u64))
}

/// 조각마다 찾아 합친다. 여러 조각에 걸린 항목은 가장 높은 점수로 한 번만 남는다.
/// 전체 개수는 조각별 개수의 합이라 겹치는 항목이 있으면 실제보다 조금 클 수 있다
fn search_all(q: &str, folders: &crate::folders::Folders) -> Result<SearchResponse, SearchError> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut all: Vec<(i32, bool, String)> = Vec::new();
    let (mut total, mut micros, mut error) = (0, 0, None);
    for piece in query_pieces(q) {
        match search_piece(piece, folders) {
            Ok((hits, t, m)) => {
                total += t;
                micros += m;
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
    all.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.len().cmp(&b.2.len())));
    all.truncate(PAL_LIMIT);
    Ok(SearchResponse { total, micros, hits: all.into_iter().map(|(_, d, p)| (d, p)).collect() })
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
        (PAL_MENU_LOCATE, "새 탭에서 위치 열기\tCtrl+Enter".to_string()),
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

pub enum OpenMode {
    Open,
    Locate,
    NewTab,
}

impl App {
    pub(super) fn pal_rect(&self) -> R {
        let w = PAL_W.min(self.w - 40.0).max(320.0);
        let l = ((self.w - w) / 2.0).max(MARGIN);
        // 설치 안내·폴더 넣기 버튼 자리까지 필요하다
        let min_rows = if self.pal_status == PalStatus::NotInstalled || self.pal_shows_add_folder() { 5 } else { 3 };
        let rows = self.pal_results.len().clamp(min_rows, PAL_MAX_ROWS) as f32;
        let h = (PAL_HEAD_H + rows * PAL_ROW_H + PAL_FOOT_H + 8.0).min((self.h - TOOLBAR_H - 16.0).max(180.0));
        R::new(l, TOOLBAR_H + 6.0, l + w, TOOLBAR_H + 6.0 + h)
    }

    fn pal_list(&self) -> R {
        let r = self.pal_rect();
        R::new(r.l + 6.0, r.t + PAL_HEAD_H, r.r - 6.0, r.b - PAL_FOOT_H)
    }

    pub(super) fn pal_input_rect(&self) -> R {
        let r = self.pal_rect();
        R::new(r.l + 44.0, r.t + 10.0, r.r - 14.0, r.t + PAL_HEAD_H - 10.0)
    }

    fn pal_install_button(&self) -> R {
        let list = self.pal_list();
        let cx = (list.l + list.r) / 2.0;
        R::new(cx - 90.0, list.t + 96.0, cx + 90.0, list.t + 128.0)
    }

    /// 찾은 게 없고 보이는 칸에 검색 범위 밖 폴더가 열려 있으면 "이 폴더도 검색하기" 버튼을 보인다
    fn pal_shows_add_folder(&self) -> bool {
        matches!(self.pal_status, PalStatus::Done { .. }) && self.pal_results.is_empty() && self.pal_outside.is_some()
    }

    pub(super) fn pal_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let r = self.pal_rect();
        if !r.contains(x, y) {
            return None;
        }
        if self.pal_status == PalStatus::NotInstalled && self.pal_install_button().contains(x, y) {
            return Some(Hit::PalInstall);
        }
        if self.pal_shows_add_folder() && self.pal_install_button().contains(x, y) {
            return Some(Hit::PalAddFolder);
        }
        let list = self.pal_list();
        if list.contains(x, y) {
            let i = ((y - list.t + self.pal_scroll) / PAL_ROW_H).floor();
            if i >= 0.0 && (i as usize) < self.pal_results.len() {
                return Some(Hit::PalRow(i as usize));
            }
        }
        Some(Hit::PalPanel)
    }

    pub(super) fn toggle_palette(&mut self) {
        if self.pal_open {
            self.close_palette();
        } else {
            self.open_palette();
        }
    }

    pub(super) fn open_palette(&mut self) {
        self.close_favorites();
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        self.pal_open = true;
        self.pal_results.clear();
        self.pal_picked.clear();
        self.pal_sel = 0;
        self.pal_anchor = 0;
        self.pal_scroll = 0.0;
        self.pal_status = PalStatus::Idle;
        self.pal_seq += 1;
        self.pal_outside = self.visible_dirs().into_iter().find(|d| self.folders.covering(d).is_none() && crate::folders::outside_disk_index(d));
        // 서비스가 있는지·인덱스가 준비됐는지 먼저 확인
        spawn_search(self.hwnd, self.pal_seq, None, self.folders.clone());
        self.hover = Hit::None;
        self.layout_edits();
        self.focus();
        // 지난 검색어는 남겨 두고 통째로 선택한다: 새로 치면 바뀌고, 이어 쓰려면 End. 결과도 다시 찾아 채운다
        if !edit_text(self.search_edit).trim().is_empty() {
            unsafe {
                SendMessageW(self.search_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            }
            self.pal_run_query();
        }
        self.invalidate();
    }

    /// 전역 단축키: 이미 열려 있으면 입력 상자에 포커스만
    pub fn open_palette_global(&mut self) {
        if self.pal_open {
            self.focus();
        } else {
            self.open_palette();
        }
    }

    pub(super) fn close_palette(&mut self) {
        if !self.pal_open {
            return;
        }
        self.pal_open = false;
        self.pal_seq += 1;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SEARCH);
        }
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    /// 입력이 바뀌면 잠깐 모았다가 검색 (빠르게 칠 때 질의를 줄인다)
    pub(super) fn pal_query_changed(&mut self) {
        if self.pal_open {
            unsafe { SetTimer(Some(self.hwnd), TIMER_SEARCH, 40, None) };
        }
    }

    pub(super) fn pal_run_query(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SEARCH);
        }
        let q = edit_text(self.search_edit);
        self.pal_seq += 1;
        if q.trim().is_empty() {
            self.pal_results.clear();
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
        spawn_search(self.hwnd, self.pal_seq, Some(q), self.folders.clone());
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
        if !self.pal_open || stale {
            return;
        }
        match done.reply {
            SearchReply::Status(Ok((true, n))) => {
                if matches!(self.pal_status, PalStatus::Idle | PalStatus::NotInstalled | PalStatus::Indexing) {
                    self.pal_status = PalStatus::Ready(n);
                }
            }
            SearchReply::Status(Ok((false, _))) => {
                if !matches!(self.pal_status, PalStatus::Done { .. } | PalStatus::Searching) {
                    self.pal_status = PalStatus::Indexing;
                }
            }
            SearchReply::Results(Ok(r)) => {
                // 훑는 중 다시 찾아 결과가 늘어날 때도 골라 둔 항목은 경로로 이어 간다
                let kept: HashSet<String> = self.pal_picked_paths().into_iter().map(|(_, p)| p.to_lowercase()).collect();
                self.pal_picked = r.hits.iter().map(|(_, p)| kept.contains(&p.to_lowercase())).collect();
                self.pal_results = r.hits;
                self.pal_status = PalStatus::Done { total: r.total, micros: r.micros };
                self.pal_sel = 0;
                self.pal_anchor = 0;
                self.pal_scroll = 0.0;
            }
            SearchReply::Status(Err(SearchError::NotRunning)) | SearchReply::Results(Err(SearchError::NotRunning)) => {
                self.pal_results.clear();
                self.pal_status = PalStatus::NotInstalled;
            }
            SearchReply::Status(Err(SearchError::Other(e))) | SearchReply::Results(Err(SearchError::Other(e))) => {
                crate::diag::log(|| format!("search: {e}"));
                self.pal_status = PalStatus::Error(e);
            }
        }
        self.invalidate();
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

    /// 다룰 결과: 골라 둔 것이 있으면 그것들, 없으면 지금 줄 하나
    fn pal_targets(&self) -> Vec<(bool, String)> {
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
        let Some(target) = self.pal_results.get(i).cloned() else { return };
        self.pal_open_paths(vec![target], mode);
    }

    /// 결과를 연다. 위치 열기(Ctrl+Enter)와 Shift+Enter는 지금 탭을 두고 새 탭에서 열고,
    /// 여러 개면 폴더마다 새 탭을 하나씩 열어 그 안의 고른 파일들을 선택해 둔다
    fn pal_open_paths(&mut self, targets: Vec<(bool, String)>, mode: OpenMode) {
        if targets.is_empty() {
            return;
        }
        self.close_palette();
        let p = self.active;
        let many = targets.len() > 1;
        // 위치 열기에서 같은 폴더의 파일은 한 탭에 모은다: (폴더, 고를 이름들)
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for (is_dir, path) in targets {
            let parent = util::parent(&path);
            match (is_dir, &mode) {
                (true, OpenMode::Open) if !many => self.navigate(p, Location::Dir(path), None),
                (true, _) => self.new_tab(p, Some(Location::Dir(path)), true),
                (false, OpenMode::Open) => self.pending.push(Deferred::Open { pane: p, paths: vec![path], dir: parent }),
                (false, _) => {
                    let Some(dir) = parent else { continue };
                    let name = util::file_name(&path).to_lowercase();
                    match groups.iter_mut().find(|(d, _)| util::eq_path(d, &dir)) {
                        Some((_, names)) => names.push(name),
                        None => groups.push((dir, vec![name])),
                    }
                }
            }
        }
        for (dir, names) in groups {
            self.new_tab(p, Some(Location::Dir(dir)), true);
            self.panes[p].tab_mut().restore = Some(Restore { cursor: names.first().cloned(), selected: names, reveal: true, ..Default::default() });
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
        match shell::set_clipboard_files(self.hwnd, &paths) {
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
        spawn_search(self.hwnd, self.pal_seq, None, self.folders.clone());
        self.invalidate();
    }

    /// 팔레트가 열려 있을 때 검색 상자의 키. 처리했으면 true
    pub(super) fn pal_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let n = self.pal_results.len();
        let page = ((self.pal_list().h() / PAL_ROW_H).floor() as usize).max(1);
        let picks = self.pal_pick_count();
        match vk {
            // 골라 둔 게 있으면 먼저 그걸 푼다
            VK_ESCAPE if picks > 0 => self.pal_clear_picks(),
            VK_ESCAPE => self.close_palette(),
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
                self.pal_open_paths(self.pal_targets(), mode);
                return true;
            }
            VK_TAB => {}
            // 검색 단축키를 한 번 더 누르면 닫고, 설정·도움말·즐겨찾기 단축키는 검색 창을 닫고 그쪽으로
            _ if self.keys.find(crate::keys::Combo::now(vk.0)).is_some_and(|a| {
                use crate::keys::Action;
                matches!(a, Action::Search | Action::Settings | Action::Help | Action::Favorites)
            }) =>
            {
                let a = self.keys.find(crate::keys::Combo::now(vk.0));
                self.close_palette();
                if let Some(a) = a.filter(|a| *a != crate::keys::Action::Search) {
                    self.run_action(a);
                }
            }
            // 고른 결과가 있거나 검색어에 선택한 글자가 없으면 파일을 복사한다 (그 밖에는 글자 복사)
            _ if ctrl && vk.0 == b'C' as u16 && n > 0 && (picks > 0 || !edit_has_selection(self.search_edit)) => self.pal_copy_files(),
            _ => return false,
        }
        if self.pal_open {
            self.pal_ensure_visible();
        }
        self.invalidate();
        true
    }

    /// 팔레트가 열려 있을 때 왼쪽 버튼. 밖을 누르면 닫고 클릭은 소비
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
            Hit::PalPanel => {}
            _ => self.close_palette(),
        }
    }

    pub(super) fn paint_palette(&mut self) {
        let t = self.theme;
        let r = self.pal_rect();
        let list = self.pal_list();
        let input = self.pal_input_rect();
        let install = self.pal_install_button();
        let (hover, sel, scroll) = (self.hover, self.pal_sel, self.pal_scroll);
        let status = self.pal_status.clone();
        let query = edit_text(self.search_edit);
        let add_folder = if self.pal_shows_add_folder() { self.pal_outside.clone() } else { None };
        let folder_entries = self.folders.entry_count();
        let picks = self.pal_pick_count();
        let help = self.keys.get(crate::keys::Action::Help).map(crate::keys::Combo::label);
        let App { gfx, pal_results, pal_picked, types, .. } = self;

        let shadow = Color { r: 0.0, g: 0.0, b: 0.0, a: if t.dark { 0.5 } else { 0.16 } };
        gfx.fill_round(R::new(r.l - 3.0, r.t + 2.0, r.r + 3.0, r.b + 6.0), 14.0, shadow);
        gfx.fill_round(r, 12.0, t.pane);
        gfx.stroke_round(r.inset(0.5, 0.5), 12.0, t.accent, 1.2);

        // 입력 줄 (실제 입력은 위에 놓인 EDIT 컨트롤)
        gfx.glyph(G_SEARCH, R::new(r.l + 14.0, r.t, r.l + 40.0, r.t + PAL_HEAD_H), false, t.accent);
        gfx.fill_round(input.inset(-4.0, 0.0), 8.0, t.field);
        gfx.fill(R::new(r.l + 10.0, r.t + PAL_HEAD_H - 1.0, r.r - 10.0, r.t + PAL_HEAD_H), t.border);

        gfx.clip(list);
        let message: Option<String> = match &status {
            PalStatus::NotInstalled => Some("검색 서비스가 설치되어 있지 않습니다\n디스크 전체를 바로 찾으려면 관리자 권한으로 한 번 설치합니다".into()),
            PalStatus::Indexing => Some("인덱스를 만드는 중입니다… 잠시 후 다시 입력해 보세요".into()),
            PalStatus::Error(e) => Some(format!("검색 서비스와 통신하지 못했습니다\n{e}")),
            PalStatus::Done { .. } if pal_results.is_empty() && add_folder.is_some() => Some(format!(
                "‘{}’와(과) 일치하는 항목이 없습니다\n열어 둔 {}은(는) 아직 검색 범위 밖입니다. 넣으면 그 안도 함께 찾습니다",
                query.trim(),
                add_folder.as_deref().unwrap_or_default()
            )),
            PalStatus::Done { .. } if pal_results.is_empty() => Some(format!("‘{}’와(과) 일치하는 항목이 없습니다", query.trim())),
            PalStatus::Idle | PalStatus::Ready(_) if query.trim().is_empty() => {
                Some(format!(
                    "파일·폴더 이름을 입력하세요\n띄어 쓰면 모두 포함 · | 는 또는 · 끝에 \\ 는 경로{}",
                    help.as_deref().map(|h| format!(" · 자세히는 {h}")).unwrap_or_default()
                ))
            }
            _ => None,
        };
        if let Some(m) = message {
            gfx.text(&m, R::new(list.l + 16.0, list.t + 10.0, list.r - 16.0, list.t + 90.0), Font::Hint, t.faint);
        }
        let button = match (&status, &add_folder) {
            (PalStatus::NotInstalled, _) => Some(("검색 서비스 설치", Hit::PalInstall)),
            (_, Some(_)) => Some(("이 폴더도 검색하기", Hit::PalAddFolder)),
            _ => None,
        };
        if let Some((label, h)) = button {
            gfx.fill_round(install, 8.0, if hover == h { t.accent } else { alpha(t.accent, 0.85) });
            gfx.text(label, R::new(install.l, install.t, install.r, install.b), Font::Hint, Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 });
        }
        let first = (scroll / PAL_ROW_H).floor().max(0.0) as usize;
        let count = (list.h() / PAL_ROW_H).ceil() as usize + 1;
        for i in first..(first + count).min(pal_results.len()) {
            let (is_dir, path) = &pal_results[i];
            let y = list.t + i as f32 * PAL_ROW_H - scroll;
            let row = R::new(list.l, y, list.r, y + PAL_ROW_H);
            // 고른 줄은 선택 색, 고른 게 있을 때 지금 줄은 테두리로만 표시
            let picked = pal_picked.get(i).copied().unwrap_or(false);
            if picked || (i == sel && picks == 0) {
                gfx.fill_round(row.inset(2.0, 1.0), 6.0, t.sel);
            } else if hover == Hit::PalRow(i) {
                gfx.fill_round(row.inset(2.0, 1.0), 6.0, t.hover);
            }
            if i == sel && picks > 0 {
                gfx.stroke_round(row.inset(2.75, 1.75), 6.0, t.accent, 1.5);
            }
            let name = util::file_name(path);
            let ext = if *is_dir { String::new() } else { name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default() };
            let key = if *is_dir { "\\dir".to_string() } else { ext.clone() };
            let icon = types.entry(key).or_insert_with(|| shell::type_info(&ext, *is_dir)).1;
            gfx.icon(icon, R::new(row.l + 12.0, y + 9.0, row.l + 28.0, y + 25.0));
            let split = row.l + 38.0 + (row.w() - 38.0) * 0.42;
            gfx.text(name, R::new(row.l + 40.0, y, split - 8.0, y + PAL_ROW_H), Font::Normal, t.text);
            let parent = util::parent(path).unwrap_or_default();
            gfx.text(&parent, R::new(split, y, row.r - 12.0, y + PAL_ROW_H), Font::Path, t.faint);
        }
        gfx.unclip();

        let foot = R::new(r.l + 16.0, r.b - PAL_FOOT_H, r.r - 14.0, r.b - 2.0);
        let foot_text = if picks > 0 {
            format!("{picks}개 고름 · Enter 모두 열기 · Ctrl+C 복사")
        } else {
            format!("Enter 열기 · Ctrl+Enter 위치{}", help.as_deref().map(|h| format!(" · {h} 도움말")).unwrap_or_default())
        };
        gfx.text(&foot_text, foot, Font::Normal, t.faint);
        let right = match &status {
            PalStatus::Done { total, micros } => format!("{total}개 중 {}개 · {:.1}ms", pal_results.len(), *micros as f64 / 1000.0),
            PalStatus::Searching => "찾는 중…".to_string(),
            PalStatus::Ready(n) if folder_entries > 0 => format!("인덱스 {n}개 · 넣은 폴더 {folder_entries}개"),
            PalStatus::Ready(n) => format!("인덱스 {n}개"),
            _ => String::new(),
        };
        gfx.text(&right, foot, Font::Right, t.faint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let count = |q: &str| search_all(q, &folders).map(|r| (r.hits.len(), r.total)).unwrap_or((0, 0));
        let (a, b, both) = (count("main.rs sakadu\\src\\"), count("tray.rs sakadu\\src\\"), count("main.rs sakadu\\src\\ | tray.rs sakadu\\src\\"));
        println!("main.rs {a:?}, tray.rs {b:?}, either {both:?}");
        assert_eq!(both.0, a.0 + b.0);
        let same = count("tray.rs sakadu\\src\\ | tray.rs sakadu\\src\\");
        assert_eq!(same.0, b.0, "same piece twice should not duplicate hits");
    }

    #[test]
    fn pasted_lines_become_alternatives() {
        let excel = "20260420_160559\r\n20260407_091344\r\n\r\n  견적서 \r\n";
        assert_eq!(lines_to_query(excel), "20260420_160559 | 20260407_091344 | 견적서");
    }
}
