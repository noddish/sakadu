//! Ctrl+K 검색 팔레트: 검색 서비스(인덱스)에 이름으로 질의해 디스크 전체에서 바로 찾는다

use super::*;
use crate::service::{self, SearchError, SearchResponse};
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

pub(super) fn search_button() -> R {
    let l = fav_button().r + 6.0;
    R::new(l, 8.0, l + 150.0, 38.0)
}

fn spawn_search(hwnd: HWND, seq: u64, query: Option<String>) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let reply = match query {
            Some(q) => SearchReply::Results(service::query(&q, PAL_LIMIT)),
            None => SearchReply::Status(service::status()),
        };
        post(owner.get(), WM_APP_SEARCH, Box::new(SearchDone { seq, reply }));
    });
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
        // 설치 안내에는 버튼 자리까지 필요하다
        let min_rows = if self.pal_status == PalStatus::NotInstalled { 5 } else { 3 };
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

    pub(super) fn pal_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let r = self.pal_rect();
        if !r.contains(x, y) {
            return None;
        }
        if self.pal_status == PalStatus::NotInstalled && self.pal_install_button().contains(x, y) {
            return Some(Hit::PalInstall);
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
        self.pal_sel = 0;
        self.pal_scroll = 0.0;
        self.pal_status = PalStatus::Idle;
        self.pal_seq += 1;
        if !edit_text(self.search_edit).is_empty() {
            unsafe {
                let _ = SetWindowTextW(self.search_edit, w!(""));
            }
        }
        // 서비스가 있는지·인덱스가 준비됐는지 먼저 확인
        spawn_search(self.hwnd, self.pal_seq, None);
        self.hover = Hit::None;
        self.layout_edits();
        self.focus();
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
        spawn_search(self.hwnd, self.pal_seq, Some(q));
        self.invalidate();
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
                self.pal_results = r.hits;
                self.pal_status = PalStatus::Done { total: r.total, micros: r.micros };
                self.pal_sel = 0;
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

    pub(super) fn pal_activate(&mut self, i: usize, mode: OpenMode) {
        let Some((is_dir, path)) = self.pal_results.get(i).cloned() else { return };
        self.close_palette();
        let p = self.active;
        let parent = util::parent(&path);
        match (is_dir, mode) {
            (true, OpenMode::NewTab) => self.new_tab(p, Some(Location::Dir(path)), true),
            (true, _) => self.navigate(p, Location::Dir(path), None),
            (false, OpenMode::Locate | OpenMode::NewTab) => {
                if let Some(dir) = parent {
                    let name = util::file_name(&path).to_string();
                    self.navigate(p, Location::Dir(dir), Some(name));
                }
            }
            (false, OpenMode::Open) => self.pending.push(Deferred::Open { pane: p, paths: vec![path], dir: parent }),
        }
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
        spawn_search(self.hwnd, self.pal_seq, None);
        self.invalidate();
    }

    /// 팔레트가 열려 있을 때 검색 상자의 키. 처리했으면 true
    pub(super) fn pal_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let n = self.pal_results.len();
        let page = ((self.pal_list().h() / PAL_ROW_H).floor() as usize).max(1);
        match vk {
            VK_ESCAPE => self.close_palette(),
            VK_UP => self.pal_sel = self.pal_sel.saturating_sub(1),
            VK_DOWN => self.pal_sel = (self.pal_sel + 1).min(n.saturating_sub(1)),
            VK_PRIOR => self.pal_sel = self.pal_sel.saturating_sub(page),
            VK_NEXT => self.pal_sel = (self.pal_sel + page).min(n.saturating_sub(1)),
            VK_RETURN => {
                let mode = if ctrl { OpenMode::Locate } else if shift { OpenMode::NewTab } else { OpenMode::Open };
                let i = self.pal_sel;
                self.pal_activate(i, mode);
                return true;
            }
            VK_TAB => {}
            _ if ctrl && vk.0 == b'K' as u16 => self.close_palette(),
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
            Hit::PalRow(i) => {
                let mode = if key_down(VK_CONTROL) { OpenMode::Locate } else if key_down(VK_SHIFT) { OpenMode::NewTab } else { OpenMode::Open };
                self.pal_activate(i, mode);
            }
            Hit::PalInstall => self.pal_install(),
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
        let App { gfx, pal_results, types, .. } = self;

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
            PalStatus::Done { .. } if pal_results.is_empty() => Some(format!("‘{}’와(과) 일치하는 항목이 없습니다", query.trim())),
            PalStatus::Idle | PalStatus::Ready(_) if query.trim().is_empty() => {
                Some("파일·폴더 이름을 입력하세요\n여러 낱말은 모두 포함 · ‘업무\\’처럼 \\를 붙이면 경로로 찾기".into())
            }
            _ => None,
        };
        if let Some(m) = message {
            gfx.text(&m, R::new(list.l + 16.0, list.t + 10.0, list.r - 16.0, list.t + 90.0), Font::Hint, t.faint);
        }
        if status == PalStatus::NotInstalled {
            gfx.fill_round(install, 8.0, if hover == Hit::PalInstall { t.accent } else { alpha(t.accent, 0.85) });
            gfx.text("검색 서비스 설치", R::new(install.l, install.t, install.r, install.b), Font::Hint, Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 });
        }
        let first = (scroll / PAL_ROW_H).floor().max(0.0) as usize;
        let count = (list.h() / PAL_ROW_H).ceil() as usize + 1;
        for i in first..(first + count).min(pal_results.len()) {
            let (is_dir, path) = &pal_results[i];
            let y = list.t + i as f32 * PAL_ROW_H - scroll;
            let row = R::new(list.l, y, list.r, y + PAL_ROW_H);
            if i == sel {
                gfx.fill_round(row.inset(2.0, 1.0), 6.0, t.sel);
            } else if hover == Hit::PalRow(i) {
                gfx.fill_round(row.inset(2.0, 1.0), 6.0, t.hover);
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
        gfx.text("Enter 열기 · Ctrl+Enter 위치 열기 · Shift+Enter 새 탭 · Esc 닫기", foot, Font::Normal, t.faint);
        let right = match &status {
            PalStatus::Done { total, micros } => format!("{total}개 중 {}개 · {:.1}ms", pal_results.len(), *micros as f64 / 1000.0),
            PalStatus::Searching => "찾는 중…".to_string(),
            PalStatus::Ready(n) => format!("인덱스 {n}개"),
            _ => String::new(),
        };
        gfx.text(&right, foot, Font::Right, t.faint);
    }
}
