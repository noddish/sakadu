//! 창 왼쪽의 작업 공간 레일과 그 옆에 여는 패널.
//! 레일에는 작업 공간마다 아이콘(공간 색, 칸 나누기 모양)이 있고, 그 아래 새 공간(+), 맨 아래 설정이 있다 (자리와 그리기는 공통 코드 ui::rail).
//! 패널은 한 번에 하나만 연다: 작업 공간 패널(이 공간에 고정한 것, 아래쪽 '모든 공간' = 내 PC와 즐겨찾기)과 검색 패널(palette.rs).
//! 검색 패널은 늘 고정으로만 열린다 (떠 있다가 닫히면 입력 중인 검색이 사라지니까).
//! 지금 공간 아이콘에 잠깐 머무르면 패널이 칸 위에 떠서 열리고(마우스가 떠나거나 위치를 고르면 닫힘), 그 아이콘을 누르거나 패널의 핀을 누르면
//! 고정돼 칸을 밀어내며 오른쪽 가장자리를 끌어 너비를 바꿀 수 있다. 다른 공간 아이콘을 누르면 그 공간으로 바꾼다.
//! 패널 가운데 목록의 줄 배치는 `side_slots` 한 곳에서 정해 그리기·누르기·끌어 놓기가 같이 쓴다.
//! 즐겨찾기와 고정한 것의 동작(열기·추가·이름·그룹·키보드·끌어서 옮기기)은 fav_ui.rs, 작업 공간을 바꾸고 만드는 것은 workspace_ui.rs

use super::*;
use crate::favorites::Row;
use sakadu::ui::rail::{RailGeom, RailHover, RailScene, RailSpace, paint_rail, rail_geom};
use sakadu::ui::{SIDE_HEADER_H, SIDE_LABEL_H, side_common_top, side_row_bg};

pub(super) use sakadu::ui::{BAR_W, SIDE_ROW_H, draw_pin, hover_fill, seg_on, seg_track, well};
/// 레일 첫 아이콘의 위쪽 (도구 막대 단추와 같은 높이)
const RAIL_TOP: f32 = 10.0;
const ROW_L: f32 = 8.0;
/// 작은 단추 (머리 줄의 핀, '모든 공간' 줄의 추가·정렬·새 그룹)
const MINI: f32 = 24.0;
/// 패널 머리에서 작업 공간 이름이 놓이는 줄의 높이
const TITLE_H: f32 = 28.0;
/// 고정한 것이 없을 때의 안내 자리
const HINT_H: f32 = 44.0;
/// 고정 목록과 '모든 공간' 구역 사이
const COMMON_GAP: f32 = 12.0;
/// 아이콘에 머무른 뒤 패널이 떠서 열리기까지
const HOVER_OPEN_MS: u32 = 300;
/// 떠 있는 패널 밖으로 나간 뒤 닫히기까지
const PEEK_CLOSE_MS: u32 = 400;
pub(super) const TIMER_BAR_HOVER: usize = 46;
pub(super) const TIMER_SIDE_CLOSE: usize = 47;
/// 패널이 열릴 때 펼쳐지는 움직임 (Windows에서 애니메이션을 끄면 하지 않는다)
pub(super) const TIMER_SIDE_ANIM: usize = 48;
const SIDE_ANIM_SECS: f32 = 0.18;
const G_SORT: u16 = 0xE8CB;
const G_NEW_GROUP: u16 = 0xE8F4;
const G_TWIST_OPEN: u16 = 0xE70D;
const G_TWIST_CLOSED: u16 = 0xE76C;

/// 레일 옆에 여는 패널
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    /// 작업 공간 패널: 이 공간에 고정한 것과 '모든 공간'
    Main,
    Search,
}

impl Panel {
    pub const ALL: [Panel; 2] = [Panel::Main, Panel::Search];

    /// 세션에 저장하는 이름
    pub fn key(self) -> &'static str {
        match self {
            Panel::Main => "main",
            Panel::Search => "search",
        }
    }

    /// 예전 세션의 "places"(바로가기)와 "favorites"(즐겨찾기)는 지금의 작업 공간 패널로 연다
    pub fn from_key(key: &str) -> Option<Panel> {
        match key {
            "main" | "places" | "favorites" => Some(Panel::Main),
            "search" => Some(Panel::Search),
            _ => None,
        }
    }
}

/// 패널 안에서 누를 수 있는 것
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Side {
    /// 이 공간에 고정한 것 (지금 작업 공간의 pins 번호)
    Pin(usize),
    /// 고정한 것이 없을 때의 안내 자리. 끌어 놓을 때는 고정 목록 쪽 (놓으면 이 공간에 고정)
    PinEmpty,
    /// '모든 공간' 구역 이름 줄 (오른쪽에 추가·정렬·새 그룹 단추가 놓인다)
    CommonHead,
    FavNewGroup,
    FavSort,
    FavAdd,
    Fav(Row),
    /// 끌어 놓을 때의 '모든 공간' 쪽 (놓으면 즐겨찾기에 넣는다)
    FavEmpty,
    Computer,
}

/// 가운데 목록의 한 자리 (스크롤 전, 목록 맨 위·왼쪽이 0인 좌표)
#[derive(Clone, Copy)]
enum Slot {
    Item(Side),
}

/// 드라이브 한 개
pub(super) struct Drive {
    pub(super) name: String,
    pub(super) path: String,
    pub(super) icon: i32,
    pub(super) free: u64,
    pub(super) total: u64,
}

/// 뒤에서 읽어 오는 "내 PC"·드라이브 목록
pub struct Places {
    /// 시스템 이미지 목록의 "내 PC" 아이콘. 읽기 전이나 없으면 -1 (글리프로 대신)
    pub(super) computer_icon: i32,
    pub(super) drives: Vec<Drive>,
}

impl Default for Places {
    fn default() -> Places {
        Places { computer_icon: -1, drives: Vec::new() }
    }
}

/// 드라이브 목록을 뒤에서 읽어 WM_APP_PLACES로 보낸다 (네트워크·광 드라이브가 느릴 수 있어 UI 스레드에서 읽지 않는다)
pub(super) fn spawn_places(hwnd: HWND) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let _com = shell::ComInit::sta();
        let drives = shell::list_drives()
            .into_iter()
            .map(|e| Drive { name: e.name_string(), path: e.target.as_deref().unwrap_or_default().to_string(), icon: e.icon, free: e.free, total: e.size })
            .collect();
        shell::post_box(owner.get(), crate::WM_APP_PLACES, Box::new(Places { computer_icon: shell::computer_icon(), drives }));
    });
}

/// Windows 설정의 "애니메이션 효과"가 켜져 있는지 (끄면 움직임 없이 바로 보인다)
pub(super) fn motion_on() -> bool {
    let mut on = BOOL(1);
    let read = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut BOOL as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    read.is_err() || on.as_bool()
}

impl App {
    // ---------- 상태 ----------

    /// 지금 보이는 패널: 떠 있는 것이 있으면 그것, 아니면 고정한 것
    pub(super) fn side_panel(&self) -> Option<Panel> {
        self.side_peek.or(self.side_pinned)
    }

    /// 패널이 칸 위에 떠 있는가 (고정한 패널 위에 다른 패널이 떠 있는 경우 포함)
    pub(super) fn side_floating(&self) -> bool {
        self.side_peek.is_some_and(|p| Some(p) != self.side_pinned)
    }

    /// 칸 영역의 왼쪽 끝: 막대 오른쪽, 고정한 패널이 있으면 그 오른쪽
    pub(super) fn panes_left(&self) -> f32 {
        BAR_W + MARGIN + if self.side_pinned.is_some() { self.side_w_shown() } else { 0.0 }
    }

    /// 창이 좁을 때 패널이 차지할 수 있는 가장 넓은 너비: 칸 나누기 판(166)과 작업 줄 오른쪽 단추·창 단추(약 260)가 겹치지 않게
    pub(super) fn side_w_max(&self) -> f32 {
        (self.w - BAR_W - MARGIN - 166.0 - 270.0).clamp(session::SIDE_W_RANGE.0, session::SIDE_W_RANGE.1)
    }

    /// 지금 창에서 실제로 보이는 패널 너비 (저장한 너비는 그대로 두고, 창이 좁으면 줄여 보인다)
    fn side_w_shown(&self) -> f32 {
        self.side_w.min(self.side_w_max())
    }

    // ---------- 배치 ----------

    /// 작업 공간 레일의 자리: 공간 아이콘, 새 공간(+), 맨 아래 설정
    pub(super) fn rail(&self) -> RailGeom {
        rail_geom(RAIL_TOP, self.h, self.workspaces.list.len(), self.workspaces.current.unwrap_or(0), true)
    }

    /// 도구 막대의 검색 단추 (누르면 검색 패널)
    pub(super) fn search_button(&self) -> R {
        self.toolbar().search
    }

    /// 레일 맨 아래 설정
    pub(super) fn settings_button(&self) -> R {
        self.rail().settings.unwrap_or(R::new(0.0, 0.0, 0.0, 0.0))
    }

    /// 지금 작업 공간에 고정한 것
    pub(super) fn pins(&self) -> &[crate::favorites::Favorite] {
        self.workspaces.current().map_or(&[], |w| w.pins.as_slice())
    }

    /// 패널 자리 (떠 있을 때도 같은 자리에 칸 위로 뜬다)
    pub(super) fn side_panel_rect(&self) -> Option<R> {
        self.side_panel().map(|_| R::new(BAR_W, WORKBAR_H, BAR_W + self.side_w_shown(), (self.h - MARGIN).max(WORKBAR_H + 100.0)))
    }

    /// 고정한 패널의 오른쪽 가장자리: 끌어 너비를 바꾼다
    pub(super) fn side_resize_rect(&self) -> Option<R> {
        if self.side_pinned.is_none() || self.side_floating() {
            return None;
        }
        let r = self.side_panel_rect()?;
        Some(R::new(r.r - 2.0, r.t, r.r + GAP - 2.0, r.b))
    }

    fn side_header(&self) -> Option<R> {
        let r = self.side_panel_rect()?;
        Some(R::new(r.l, r.t, r.r, r.t + SIDE_HEADER_H))
    }

    pub(super) fn side_pin_rect(&self) -> Option<R> {
        let h = self.side_header()?;
        let t = h.t + (TITLE_H - MINI) / 2.0;
        Some(R::new(h.r - 8.0 - MINI, t, h.r - 8.0, t + MINI))
    }

    /// 패널 머리의 작업 공간 이름 자리 (핀 왼쪽까지)
    pub(super) fn side_title_rect(&self) -> Option<R> {
        let (h, pin) = (self.side_header()?, self.side_pin_rect()?);
        Some(R::new(h.l + ROW_L + 10.0, h.t, pin.l - 6.0, h.t + TITLE_H))
    }

    /// '모든 공간' 줄의 작은 단추들 (오른쪽 끝부터)
    fn side_minis(&self) -> &'static [Side] {
        match self.side_panel() {
            Some(Panel::Main) => &[Side::FavAdd, Side::FavSort, Side::FavNewGroup],
            _ => &[],
        }
    }

    /// 작은 단추 i의 화면 자리: '모든 공간' 줄의 오른쪽 끝부터 (목록과 함께 스크롤된다)
    fn side_mini_rect(&self, i: usize) -> Option<R> {
        let (slots, _) = self.side_slots();
        let head = slots.into_iter().find(|(_, s)| matches!(s, Slot::Item(Side::CommonHead))).map(|(r, _)| self.side_at(r))?;
        let r = head.r - 2.0 - i as f32 * (MINI + 2.0);
        let t = head.t + (SIDE_LABEL_H - MINI) / 2.0;
        Some(R::new(r - MINI, t, r, t + MINI))
    }

    /// 가운데 목록이 보이는 곳 (머리 줄 아래부터 패널 아래쪽까지). 패널이 없으면 빈 사각형
    pub(super) fn side_list(&self) -> R {
        match self.side_panel_rect() {
            Some(r) => R::new(r.l, r.t + SIDE_HEADER_H, r.r, (r.b - 6.0).max(r.t + SIDE_HEADER_H)),
            None => R::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    /// 보이는 패널의 가운데 목록 자리들과 전체 높이. 좌표는 스크롤 전, 목록 맨 위·왼쪽이 0
    fn side_slots(&self) -> (Vec<(R, Slot)>, f32) {
        let mut out: Vec<(R, Slot)> = Vec::new();
        // 검색 패널의 결과 목록은 palette.rs가 따로 둔다
        if self.side_panel() != Some(Panel::Main) {
            return (out, 0.0);
        }
        let rr = self.side_w_shown() - 8.0;
        let row = |y: f32, h: f32| R::new(ROW_L, y, rr, y + h);
        // 위쪽: 이 공간에 고정한 것 (없으면 안내 자리)
        let mut y = 0.0;
        for i in 0..self.pins().len() {
            out.push((row(y, SIDE_ROW_H), Slot::Item(Side::Pin(i))));
            y += SIDE_ROW_H;
        }
        if out.is_empty() {
            out.push((row(y, HINT_H), Slot::Item(Side::PinEmpty)));
            y += HINT_H;
        }
        // 아래쪽 '모든 공간': 구역 이름 줄, 내 PC, 즐겨찾기. 위쪽이 짧으면 목록 아래 끝에 붙고, 길면 그 아래에 이어져 함께 스크롤된다
        let rows = self.favs.rows();
        let common_h = SIDE_LABEL_H + SIDE_ROW_H * (rows.len() + 1) as f32 + 4.0;
        let mut y = side_common_top(y, common_h, self.side_list().h(), COMMON_GAP);
        out.push((row(y, SIDE_LABEL_H), Slot::Item(Side::CommonHead)));
        y += SIDE_LABEL_H;
        out.push((row(y, SIDE_ROW_H), Slot::Item(Side::Computer)));
        y += SIDE_ROW_H;
        for r in rows {
            out.push((row(y, SIDE_ROW_H), Slot::Item(Side::Fav(r))));
            y += SIDE_ROW_H;
        }
        (out, y + 4.0)
    }

    /// 목록 좌표를 화면 좌표로 옮기는 양 (패널 왼쪽, 목록 위에서 스크롤만큼 뺀 것)
    fn side_offset(&self) -> (f32, f32) {
        let list = self.side_list();
        (list.l, list.t - self.side_scroll)
    }

    /// 화면 좌표로 옮긴 자리
    fn side_at(&self, r: R) -> R {
        let (dx, dy) = self.side_offset();
        R::new(r.l + dx, r.t + dy, r.r + dx, r.b + dy)
    }

    /// 그 즐겨찾기 줄의 화면 자리 (목록 밖으로 스크롤돼 있어도). 즐겨찾기 패널이 보이지 않으면 None
    pub(super) fn fav_row_rect(&self, row: Row) -> Option<R> {
        let (slots, _) = self.side_slots();
        slots.into_iter().find(|(_, s)| matches!(s, Slot::Item(Side::Fav(r)) if *r == row)).map(|(r, _)| self.side_at(r))
    }

    pub(super) fn side_clamp_scroll(&mut self) {
        let (_, content) = self.side_slots();
        let max = (content - self.side_list().h()).max(0.0);
        self.side_scroll = self.side_scroll.clamp(0.0, max);
    }

    /// 화면 위치 t..b가 목록 안에 보이도록 스크롤한다
    pub(super) fn side_reveal(&mut self, r: R) {
        let list = self.side_list();
        if r.t < list.t {
            self.side_scroll -= list.t - r.t;
        } else if r.b > list.b {
            self.side_scroll += r.b - list.b;
        }
        self.side_clamp_scroll();
    }

    pub(super) fn side_wheel(&mut self, dy: f32) {
        self.side_scroll += dy;
        self.side_clamp_scroll();
        if matches!(self.edit_mode, EditMode::FavRename(_)) {
            self.layout_edits();
        }
        // 줄이 밀렸으니 강조·말풍선은 마우스 아래 새 줄로
        self.refresh_hover();
        self.update_tip();
        self.invalidate();
    }

    /// 이름 바꾸기 입력 상자 자리 (그 줄의 이름 칸). 줄이 목록 밖이면 None
    pub(super) fn fav_rename_rect(&self, row: Row) -> Option<R> {
        let r = self.fav_row_rect(row)?;
        let list = self.side_list();
        if r.t < list.t - 1.0 || r.b > list.b + 1.0 {
            return None;
        }
        let l = r.l + label_x(row);
        Some(R::new(l - 4.0, r.t + 3.0, r.r - 6.0, r.b - 3.0))
    }

    /// 끄는 위치 y 아래 즐겨찾기 줄과 그 줄의 아래쪽 절반인지. 첫 줄 위면 첫 줄 위쪽, 마지막 줄 아래면 마지막 줄 아래쪽
    pub(super) fn fav_drop_row(&self, y: f32) -> Option<(Row, bool)> {
        let (slots, _) = self.side_slots();
        let rows: Vec<(R, Row)> = slots
            .into_iter()
            .filter_map(|(r, s)| match s {
                Slot::Item(Side::Fav(row)) => Some((self.side_at(r), row)),
                _ => None,
            })
            .collect();
        let (first, last) = (rows.first()?, rows.last()?);
        if y < first.0.t {
            return Some((first.1, false));
        }
        if y >= last.0.b {
            return Some((last.1, true));
        }
        rows.iter().find(|(r, _)| y >= r.t && y < r.b).map(|(r, row)| (*row, y >= (r.t + r.b) / 2.0))
    }

    // ---------- 열고 닫기 ----------

    /// 패널을 연다. pin이면 고정(칸을 밀어냄), 아니면 떠서(마우스가 떠나거나 위치를 고르면 닫힘)
    pub(super) fn show_side(&mut self, p: Panel, pin: bool) {
        let (before, pinned_before) = (self.side_panel(), self.side_pinned);
        if pin {
            self.side_pinned = Some(p);
            self.side_peek = None;
        } else if self.side_pinned == Some(p) {
            self.side_peek = None;
        } else {
            self.side_peek = Some(p);
        }
        self.side_changed(before, pinned_before);
    }

    /// 고정한 것까지 모두 닫는다
    pub(super) fn close_side(&mut self) {
        let (before, pinned_before) = (self.side_panel(), self.side_pinned);
        self.side_pinned = None;
        self.side_peek = None;
        self.side_changed(before, pinned_before);
    }

    /// 떠 있는 패널만 닫는다 (고정한 패널은 그대로)
    pub(super) fn close_peek(&mut self) {
        if self.side_peek.is_some() {
            let (before, pinned_before) = (self.side_panel(), self.side_pinned);
            self.side_peek = None;
            self.side_changed(before, pinned_before);
        }
    }

    /// 막대 아이콘을 눌렀다: 고정한 패널이면 닫고, 아니면(안 보이거나 떠 있으면) 고정해서 연다
    pub(super) fn bar_click(&mut self, p: Panel) {
        if self.side_floating() && self.side_pinned == Some(p) {
            self.close_peek();
        } else if self.side_pinned == Some(p) {
            self.close_side();
        } else {
            self.show_side(p, true);
        }
    }

    /// 패널 머리의 핀: 떠 있으면 고정, 고정이면 풀어서 떠 있게 (마우스가 떠나면 닫힌다)
    pub(super) fn side_toggle_pin(&mut self) {
        let Some(p) = self.side_panel() else { return };
        if self.side_floating() {
            self.show_side(p, true);
        } else {
            let (before, pinned_before) = (Some(p), self.side_pinned);
            self.side_pinned = None;
            self.side_peek = Some(p);
            self.side_changed(before, pinned_before);
        }
    }

    /// 패널이 바뀌거나 자리가 바뀐 뒤: 스크롤·입력 상자·칸 스크롤·마우스 아래 것을 다시 맞춘다
    fn side_changed(&mut self, before: Option<Panel>, pinned_before: Option<Panel>) {
        // 패널 자리가 비어 있다가 열릴 때만 펼친다: 고정 패널 위로 떠서 열리거나 고정 패널로 돌아올 때 그 패널이 깜빡이지 않게.
        // 검색 패널은 입력 상자(자식 창)가 함께 움직이지 못해 바로 연다 (펼치는 도중에 바뀌어도 그만둔다)
        match self.side_panel() {
            // 떠서 열 때만 펼친다. 고정해 열면 칸이 한 번에 자리를 옮기므로 패널만 펼쳐지면 어긋나 보인다
            Some(p) if before.is_none() && p != Panel::Search && self.side_floating() => self.start_side_anim(),
            Some(Panel::Search) | None => self.side_anim = None,
            Some(_) => {}
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_BAR_HOVER);
            let _ = KillTimer(Some(self.hwnd), TIMER_SIDE_CLOSE);
        }
        self.bar_arm = None;
        self.peek_leaving = false;
        if self.side_panel() != before {
            if let Some(b) = before {
                self.side_scroll_memo[b as usize] = self.side_scroll;
            }
            self.side_scroll = self.side_panel().map_or(0.0, |p| self.side_scroll_memo[p as usize]);
        }
        if self.side_panel() != Some(Panel::Main) {
            // 즐겨찾기 줄과 작업 공간 이름이 사라지면 그 이름 입력은 확정하고 키보드는 칸으로
            if matches!(self.edit_mode, EditMode::FavRename(_) | EditMode::WorkspaceRename) {
                self.commit_rename();
            }
            self.fav_blur();
        }
        // 검색 패널이 닫혔거나 다른 패널로 바뀌었다 (다른 패널이 잠깐 떠 있는 것은 아님: 고정 기준으로 본다)
        if pinned_before == Some(Panel::Search) && self.side_pinned != Some(Panel::Search) {
            self.pal_hidden();
        }
        self.side_clamp_scroll();
        self.after_resize_drag();
        // 가렸던 패널이 닫혀 검색 패널이 다시 보이면 키보드도 검색 상자로 돌아온다
        if self.pal_visible() && self.pal_focus {
            self.focus();
        }
        self.refresh_hover();
        self.update_tip();
    }

    /// 마우스가 움직였다 (끌기 중이 아닐 때): 지금 공간 아이콘에 머무르면 패널을 띄우고, 떠 있는 패널 밖으로 나가면 닫을 준비
    pub(super) fn side_track(&mut self, x: f32, y: f32) {
        // 머물러 떠서 열기는 패널을 고정하지 않았을 때만: 고정한 패널이 있으면 그 위에 다른 패널이 겹쳐 뜨지 않고, 누를 때만 바꾼다
        let over = match self.hover {
            Hit::Rail(i) if Some(i) == self.workspaces.current && self.side_pinned.is_none() && self.side_panel() != Some(Panel::Main) && self.settings.is_none() => Some(Panel::Main),
            _ => None,
        };
        if over != self.bar_arm {
            self.bar_arm = over;
            unsafe {
                if over.is_some() {
                    SetTimer(Some(self.hwnd), TIMER_BAR_HOVER, HOVER_OPEN_MS, None);
                } else {
                    let _ = KillTimer(Some(self.hwnd), TIMER_BAR_HOVER);
                }
            }
        }
        // 나가는 순간부터 재고, 돌아오면 멈춘다 (밖에서 계속 움직여도 늦춰지지 않게)
        if self.side_floating() {
            let inside = self.side_inside(x, y);
            if inside && self.peek_leaving {
                self.peek_leaving = false;
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_SIDE_CLOSE);
                }
            } else if !inside && !self.peek_leaving {
                self.peek_leaving = true;
                unsafe { SetTimer(Some(self.hwnd), TIMER_SIDE_CLOSE, PEEK_CLOSE_MS, None) };
            }
        }
    }

    /// 막대나 패널 위인가 (떠 있는 패널을 닫지 않을 곳)
    fn side_inside(&self, x: f32, y: f32) -> bool {
        x < BAR_W || self.side_panel_rect().is_some_and(|r| r.contains(x, y))
    }

    /// 마우스가 창을 떠났다
    pub(super) fn side_mouse_left(&mut self) {
        self.bar_arm = None;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_BAR_HOVER);
            if self.side_floating() && !self.peek_leaving {
                self.peek_leaving = true;
                SetTimer(Some(self.hwnd), TIMER_SIDE_CLOSE, PEEK_CLOSE_MS, None);
            }
        }
    }

    pub(super) fn on_bar_hover_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_BAR_HOVER);
        }
        let Some(p) = self.bar_arm.take() else { return };
        if matches!(self.hover, Hit::Rail(i) if Some(i) == self.workspaces.current) && self.side_pinned.is_none() && self.side_panel() != Some(p) && matches!(self.drag, Drag::None) && self.settings.is_none() && self.edit_mode == EditMode::None {
            self.show_side(p, false);
        }
    }

    pub(super) fn on_side_close_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SIDE_CLOSE);
        }
        self.peek_leaving = false;
        // 끌거나 이름을 바꾸는 중, 키보드가 즐겨찾기에 있을 때, 패널의 메뉴가 떠 있을 때는 닫지 않는다
        if !self.side_floating() || !matches!(self.drag, Drag::None) || matches!(self.edit_mode, EditMode::FavRename(_)) || self.fav_focus || self.menu_open {
            return;
        }
        // 그 사이 돌아왔으면 그대로 둔다
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_ok() && unsafe { WindowFromPoint(pt) } == self.hwnd {
            unsafe {
                let _ = ScreenToClient(self.hwnd, &mut pt);
            }
            let s = self.scale();
            if self.side_inside(pt.x as f32 / s, pt.y as f32 / s) {
                return;
            }
        }
        self.close_peek();
    }

    // ---------- 누르기 ----------

    /// 레일의 공간 아이콘을 눌렀다: 지금 공간이면 패널을 여닫고, 다른 공간이면 그 공간으로 바꾼다
    pub(super) fn rail_click(&mut self, i: usize) {
        if self.workspaces.current == Some(i) {
            self.bar_click(Panel::Main);
        } else {
            self.switch_workspace(i);
        }
    }

    /// 레일 (x < BAR_W)에서 누를 수 있는 것
    pub(super) fn bar_hit(&self, x: f32, y: f32) -> Hit {
        let m = self.rail();
        if let Some((i, _)) = m.spaces.iter().find(|(_, r)| r.contains(x, y)) {
            return Hit::Rail(*i);
        }
        if m.add.contains(x, y) {
            return Hit::RailAdd;
        }
        if m.settings.is_some_and(|r| r.contains(x, y)) {
            return Hit::SettingsButton;
        }
        Hit::None
    }

    /// 패널 안에서 누를 수 있는 것 (패널 자리 안이라고 확인한 뒤 부른다). 빈 곳은 SidePanel
    pub(super) fn sidebar_hit(&self, x: f32, y: f32) -> Hit {
        if self.side_panel() == Some(Panel::Search) {
            return self.pal_hit(x, y);
        }
        if self.side_pin_rect().is_some_and(|r| r.contains(x, y)) {
            return Hit::SidePin;
        }
        if self.side_list().contains(x, y) {
            // 작은 단추는 '모든 공간' 줄 위에 놓여 있으니 먼저 본다
            if self.side_minis_shown {
                for (i, s) in self.side_minis().iter().enumerate() {
                    if self.side_mini_rect(i).is_some_and(|r| r.contains(x, y)) {
                        return Hit::Side(*s);
                    }
                }
            }
            let (slots, _) = self.side_slots();
            for (r, Slot::Item(side)) in slots {
                if self.side_at(r).contains(x, y) {
                    return Hit::Side(side);
                }
            }
        }
        Hit::SidePanel
    }

    /// 끌어 놓기 대상: '모든 공간' 구역이면 즐겨찾기에 넣기(FavEmpty), 그 위쪽이면 이 공간에 고정(PinEmpty)
    pub(super) fn side_drop_at(&self, x: f32, y: f32) -> Option<Side> {
        if self.side_panel() != Some(Panel::Main) || !self.side_panel_rect().is_some_and(|r| r.contains(x, y)) {
            return None;
        }
        let head = self.side_item_rect(Side::CommonHead)?;
        Some(if y >= head.t { Side::FavEmpty } else { Side::PinEmpty })
    }

    /// 내 PC 줄의 위치
    fn side_path(&self, s: Side) -> Option<Location> {
        (s == Side::Computer).then_some(Location::Computer)
    }

    /// 패널 줄을 연다 (new_tab: 새 탭에서). 떠 있는 패널은 위치를 골랐으니 닫는다
    pub(super) fn side_open(&mut self, s: Side, new_tab: bool) {
        match s {
            Side::Fav(row) => return self.fav_open_row(row, new_tab),
            Side::Pin(i) => return self.pin_open(i, new_tab),
            _ => {}
        }
        let Some(loc) = self.side_path(s) else { return };
        let p = self.active;
        if new_tab {
            self.new_tab(p, Some(loc), true);
        } else {
            self.navigate(p, loc, None);
        }
        self.close_peek();
    }

    pub(super) fn side_mouse_down(&mut self, s: Side, y: f32, dbl: bool) {
        // 즐겨찾기 줄 밖의 두 번째 누름(두 번 클릭)은 무시한다: 첫 누름에서 이미 했다 (새 그룹이 두 개 생기지 않게)
        if dbl && !matches!(s, Side::Fav(_)) {
            return;
        }
        match s {
            Side::FavAdd => self.add_favorite(),
            Side::FavNewGroup => {
                self.add_fav_group();
            }
            Side::FavSort => {
                if let Some(r) = self.side_item_rect(Side::FavSort) {
                    let pt = self.to_screen(r.l, r.b + 2.0);
                    self.pending.push(Deferred::FavSortMenu { pt });
                }
            }
            Side::Fav(row) => self.fav_mouse_down(row, y, dbl),
            Side::FavEmpty | Side::PinEmpty | Side::CommonHead => {}
            Side::Pin(_) | Side::Computer => self.side_open(s, key_down(VK_CONTROL)),
        }
    }

    /// 그 줄·단추의 화면 자리
    fn side_item_rect(&self, s: Side) -> Option<R> {
        if let Some(i) = self.side_minis().iter().position(|m| *m == s) {
            return self.side_mini_rect(i);
        }
        let (slots, _) = self.side_slots();
        slots.into_iter().find(|(_, x)| matches!(x, Slot::Item(v) if *v == s)).map(|(r, _)| self.side_at(r))
    }

    /// 오른쪽 버튼: 즐겨찾기는 즐겨찾기 메뉴, 고정한 것은 고정 메뉴, 내 PC는 열기 메뉴
    pub(super) fn side_context_menu(&mut self, s: Side, x: f32, y: f32) {
        let pt = self.to_screen(x, y);
        match s {
            Side::Fav(row) => {
                if let Some(i) = self.favs.row_index(row) {
                    self.fav_sel = i;
                }
                self.pending.push(Deferred::FavMenu { row, pt });
            }
            Side::Pin(index) => self.pending.push(Deferred::PinMenu { index, pt }),
            Side::Computer => self.pending.push(Deferred::SideMenu { side: s, pt }),
            _ => {}
        }
        self.invalidate();
    }

    /// 마우스를 올려 두면 보여 줄 말풍선 (자리, 글자)
    pub(super) fn side_tip(&self, s: Side) -> Option<(R, String)> {
        let text = match s {
            Side::FavAdd => "고른 항목이나 지금 폴더를 모든 공간에 넣기".to_string(),
            Side::FavSort => "정렬".to_string(),
            Side::FavNewGroup => "새 그룹".to_string(),
            Side::Fav(Row::Item { group, index }) => self.favs.item(group, index)?.path.clone(),
            Side::Pin(i) => self.pins().get(i)?.path.clone(),
            _ => return None,
        };
        let r = self.side_item_rect(s)?;
        // 목록 밖으로 스크롤된 줄·단추에는 말풍선을 붙이지 않는다
        let list = self.side_list();
        (r.b > list.t && r.t < list.b).then_some((r, text))
    }

    // ---------- 마우스를 올릴 때만 보이는 단추·여는 움직임 ----------

    /// 작은 단추(머리 줄의 핀, '모든 공간' 줄의 추가·정렬·새 그룹)를 보일지: 마우스가 패널 위에 있거나 키보드가 즐겨찾기에 있을 때만
    fn side_hot(&self) -> bool {
        self.fav_focus || matches!(self.hover, Hit::Side(_) | Hit::SidePin | Hit::SidePanel)
    }

    /// 패널이 새로 열렸다: 막대 쪽에서 펼쳐지게 한다 (검색 패널은 입력 상자가 함께 움직이지 못해 바로 연다)
    fn start_side_anim(&mut self) {
        if !motion_on() {
            self.side_anim = None;
            return;
        }
        self.side_anim = Some(std::time::Instant::now());
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_SIDE_ANIM, 15, None);
        }
    }

    pub(super) fn on_side_anim_timer(&mut self) {
        if self.side_open_progress().is_none() {
            self.side_anim = None;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_SIDE_ANIM);
            }
        }
        self.invalidate();
    }

    /// 펼쳐지는 중이면 보일 너비의 비율 (빠르게 시작해 천천히 멈춘다). 다 펼쳤으면 None
    fn side_open_progress(&self) -> Option<f32> {
        let p = self.side_anim?.elapsed().as_secs_f32() / SIDE_ANIM_SECS;
        (p < 1.0).then(|| 1.0 - (1.0 - p).powi(4))
    }

    /// 즐겨찾기 항목에 마우스를 잠시 올려 두면 말풍선 대신 뜨는 정보 카드: 아이콘·이름, 종류, 전체 경로, 사진이면 작은 그림.
    /// 디스크를 읽지 않는 정보만 쓰고, 그림은 썸네일 작업(뒤)에서 만든다
    pub(super) fn paint_fav_card(&mut self, missing: &mut Vec<crate::thumbs::Request>) {
        let Hit::Side(s @ Side::Fav(Row::Item { group, index })) = self.hover else { return };
        if !self.tip_shown || self.menu_open || !matches!(self.drag, Drag::None) || self.edit_mode != EditMode::None {
            return;
        }
        let Some(f) = self.favs.item(group, index).cloned() else { return };
        let Some(row) = self.side_item_rect(s) else { return };
        let list = self.side_list();
        if row.b <= list.t || row.t >= list.b {
            return;
        }
        let ext = if f.is_dir { String::new() } else { util::file_name(&f.path).rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default() };
        let key = if f.is_dir { "\\dir".to_string() } else { ext.clone() };
        let (kind, icon) = self.types.entry(key).or_insert_with(|| shell::type_info(&ext, f.is_dir)).clone();
        // 공유 폴더·네트워크 드라이브의 사진은 그림을 건너뛴다 (꺼져 있거나 잠든 NAS면 썸네일 작업이 오래 붙잡힌다)
        let root = &f.path[..util::root_len(&f.path)];
        let remote = f.path.starts_with("\\\\") || unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(&HSTRING::from(root)) } == 4;
        let image = !f.is_dir && !remote && preview::is_image_ext(&ext);
        let t = self.theme;
        let w = 300.0;
        let h = if image { 80.0 + 152.0 } else { 80.0 };
        let l = (row.r + 8.0).min(self.w - MARGIN - w).max(BAR_W);
        let top = row.t.min(self.h - MARGIN - h).max(WORKBAR_H);
        let r = R::new(l, top, l + w, top + h);
        let scale = self.scale();
        let fade = super::anim_progress(self.tip_at, super::TIP_FADE_MS);
        let App { gfx: g, thumb_failed, .. } = self;
        let pushed = fade.is_some_and(|f| g.push_fade(f, -(1.0 - f) * 6.0, 0.0));
        if !t.high_contrast {
            super::soft_shadow(g, r, 10.0, 14.0, 0.14, t.dark);
        }
        g.fill_round(r, 10.0, t.pane);
        g.stroke_round(r.inset(0.5, 0.5), 10.0, t.border, 1.0);
        g.icon(icon, R::new(l + 16.0, top + 16.0, l + 32.0, top + 32.0));
        g.text(&f.name, R::new(l + 44.0, top + 12.0, r.r - 16.0, top + 34.0), Font::Strong, t.text);
        g.text(&kind, R::new(l + 44.0, top + 34.0, r.r - 16.0, top + 52.0), Font::Small, t.dim);
        g.text(&f.path, R::new(l + 16.0, top + 54.0, r.r - 16.0, top + 74.0), Font::Path, t.faint);
        if image {
            let ir = R::new(l + 16.0, top + 80.0, r.r - 16.0, r.b - 16.0);
            let px = ((ir.w() * scale) as u32).clamp(64, 512);
            let key = thumb_key(&f.path, 0, px);
            if !g.thumb(&key, ir, 1.0) {
                g.fill_round(ir, 4.0, t.field);
                if !thumb_failed.contains(&key) {
                    missing.push(crate::thumbs::Request { key, path: f.path.clone(), px });
                }
            }
        }
        if pushed {
            g.pop_fade();
        }
    }

    // ---------- 드라이브 ----------

    /// 드라이브 목록을 다시 읽는다 (읽는 중이면 끝난 뒤 한 번 더)
    pub(super) fn refresh_places(&mut self) {
        if self.places_busy {
            self.places_again = true;
            return;
        }
        self.places_busy = true;
        spawn_places(self.hwnd);
    }

    pub(super) fn on_places(&mut self, places: Places) {
        self.places = places;
        self.places_busy = false;
        if std::mem::take(&mut self.places_again) {
            self.refresh_places();
        }
        // 드라이브 수가 바뀌면 아래 줄들이 밀린다: 이름 바꾸기 상자와 마우스 아래 줄을 다시 맞춘다 (시작 화면의 드라이브 카드도)
        self.side_clamp_scroll();
        self.start_changed();
        if matches!(self.edit_mode, EditMode::FavRename(_)) {
            self.layout_edits();
        }
        self.invalidate();
    }

    // ---------- 그리기 ----------

    /// 확장자·폴더별 아이콘 (형식 캐시)
    pub(super) fn type_icon(&mut self, ext: &str, dir: bool) -> i32 {
        let key = if dir { "\\dir".to_string() } else { ext.to_string() };
        self.types.entry(key).or_insert_with(|| shell::type_info(ext, dir)).1
    }

    /// 작업 공간 레일: 공간 아이콘(지금 공간은 공간 색으로 채운다), 새 공간(+), 맨 아래 설정
    pub(super) fn paint_bar(&self) {
        let t = self.theme;
        let g = &self.gfx;
        if t.high_contrast {
            g.fill(R::new(BAR_W - 1.0, 0.0, BAR_W, self.h), t.border);
        }
        let current = self.workspaces.current.unwrap_or(0);
        let spaces: Vec<RailSpace> = self
            .workspaces
            .list
            .iter()
            .enumerate()
            .map(|(i, w)| {
                // 지금 공간은 화면을 따라간다: 저장해 둔 것이 아니라 지금 칸 나누기와 지금 강조 색
                if i == current {
                    RailSpace { color: t.accent, layout: self.layout }
                } else {
                    RailSpace { color: self.space_color(w), layout: Layout::ALL[(w.state.layout as usize).min(4)] }
                }
            })
            .collect();
        let hover = match self.hover {
            Hit::Rail(i) => RailHover::Space(i),
            Hit::RailAdd => RailHover::Add,
            Hit::SettingsButton => RailHover::Settings,
            _ => RailHover::None,
        };
        // 파일을 끌어 지금 공간 아이콘 위에 올리면(패널이 닫혀 있을 때) 이 공간에 고정할 수 있다고 켜 보인다
        let drop = (self.side_drop == Some(Side::PinEmpty) && self.side_panel() != Some(Panel::Main)).then_some(current);
        paint_rail(g, &t, &RailScene { geom: &self.rail(), spaces: &spaces, current, hover, drop });
    }

    pub(super) fn paint_sidebar(&mut self) {
        self.side_minis_shown = false;
        match (self.side_open_progress(), self.side_panel_rect()) {
            (Some(f), Some(_)) => {
                self.gfx.clip(R::new(BAR_W, 0.0, self.w, self.h));
                let pushed = self.gfx.push_fade(f, -(1.0 - f) * 12.0, 0.0);
                self.paint_side_panel();
                if pushed {
                    self.gfx.pop_fade();
                }
                self.gfx.unclip();
            }
            _ => self.paint_side_panel(),
        }
    }

    fn paint_side_panel(&mut self) {
        let (Some(panel), Some(pr)) = (self.side_panel(), self.side_panel_rect()) else { return };
        if panel == Panel::Search {
            return self.paint_search_panel();
        }
        let t = self.theme;
        let floating = self.side_floating();
        let list = self.side_list();
        let (slots, _) = self.side_slots();
        let (dx, dy) = self.side_offset();
        let rr = self.side_w_shown() - 8.0;
        let cur_dir = self.panes[self.active].tab().loc.dir().map(str::to_string);
        let at_computer = self.panes[self.active].tab().loc == Location::Computer;
        let is_current = |path: &str| cur_dir.as_deref().is_some_and(|d| util::eq_path(d, path));
        let hover = self.hover;
        let fav_rows = self.favs.rows();
        let sel_row = self.fav_focus.then(|| fav_rows.get(self.fav_sel).copied()).flatten();
        let flash_row = self.fav_flash;
        let editing = match self.edit_mode {
            EditMode::FavRename(r) => Some(r),
            _ => None,
        };
        let renaming_space = self.edit_mode == EditMode::WorkspaceRename;
        // 끌어서 옮기는 중: (끄는 줄, 놓을 줄, 아래쪽 절반)
        let moving = match self.drag {
            Drag::FavMove { from, y } => self.fav_drop_row(y).map(|(at, below)| (from, at, below)),
            _ => None,
        };
        let side_drop = self.side_drop;
        // 아이콘은 형식 캐시에서 (가변 빌림이 필요해 먼저 모아 둔다)
        let ext_of = |path: &str| util::file_name(path).rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        let kinds: Vec<(bool, String)> = fav_rows
            .iter()
            .filter_map(|r| match r {
                Row::Item { group, index } => self.favs.item(*group, *index).map(|f| (f.is_dir, ext_of(&f.path))),
                Row::Group(_) => None,
            })
            .collect();
        let mut fav_icons = kinds.iter().map(|(d, e)| self.type_icon(e, *d)).collect::<Vec<_>>().into_iter();
        let pins = self.pins().to_vec();
        let pin_icons: Vec<i32> = pins.iter().map(|f| self.type_icon(&ext_of(&f.path), f.is_dir)).collect();
        let add_key = self.keys.get(crate::keys::Action::AddFavorite).map(crate::keys::Combo::label);
        let pin = self.side_pin_rect();
        let title = self.side_title_rect();
        let name = self.workspaces.current_name().unwrap_or_default().to_string();
        // 작은 단추는 마우스가 패널 위에 있거나 키보드가 즐겨찾기에 있을 때만
        let hot = self.side_hot();
        self.side_minis_shown = hot;
        let minis: Vec<(Side, R)> = if hot { self.side_minis().iter().enumerate().filter_map(|(i, s)| self.side_mini_rect(i).map(|r| (*s, r))).collect() } else { Vec::new() };
        let App { gfx: g, favs, places, .. } = self;

        // 패널 바탕: 떠 있으면 그림자 달린 카드, 고정이면 창 바탕 그대로
        if floating {
            let card = R::new(pr.l - 12.0, pr.t, pr.r, pr.b);
            g.clip(R::new(BAR_W, pr.t - 40.0, pr.r + 40.0, pr.b + 40.0));
            if !t.high_contrast {
                super::soft_shadow(g, card, 12.0, 16.0, 0.16, t.dark);
            }
            g.fill_round(card, 12.0, t.pane);
            g.stroke_round(card.inset(0.5, 0.5), 12.0, t.border, 1.0);
            g.unclip();
        }
        // 머리 줄: 작업 공간 이름 (고치는 중이면 입력 상자 테두리만), 핀
        if let Some(tr) = title {
            if renaming_space {
                let er = R::new(tr.l - 4.0, tr.t + 3.0, tr.r, tr.b - 3.0);
                g.fill_round(er, R_ROW, t.pane);
                g.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
            } else {
                g.text(&name, tr, Font::Strong, t.text);
            }
        }
        let mini_btn = |g: &Gfx, r: R, glyph: u16, hovered: bool| {
            if hovered {
                g.fill_round(r, R_CTRL, hover_fill(&t));
            }
            g.glyph(glyph, r, false, if hovered { t.text } else { t.dim });
        };
        if let Some(r) = pin.filter(|_| hot || floating) {
            let over = hover == Hit::SidePin;
            if over {
                g.fill_round(r, R_CTRL, hover_fill(&t));
            }
            draw_pin(g, r, !floating, if over { t.text } else { t.dim });
        }

        // 가운데 목록
        g.clip(list);
        // 줄 안의 16px 아이콘 자리
        let icon_at = |r: R, l: f32| R::new(l, r.t + (SIDE_ROW_H - 16.0) / 2.0, l + 16.0, r.t + (SIDE_ROW_H + 16.0) / 2.0);
        for (r, slot) in &slots {
            let r = R::new(r.l + dx, r.t + dy, r.r + dx, r.b + dy);
            if r.b < list.t || r.t > list.b {
                if let Slot::Item(Side::Fav(Row::Item { .. })) = slot {
                    fav_icons.next();
                }
                continue;
            }
            let Slot::Item(s) = *slot;
            match s {
                Side::Pin(i) => {
                    let Some(f) = pins.get(i) else { continue };
                    side_row_bg(g, &t, r, f.is_dir && is_current(&f.path), hover == Hit::Side(s));
                    g.icon(pin_icons[i], icon_at(r, r.l + 10.0));
                    g.text(&f.name, R::new(r.l + 36.0, r.t, r.r - 8.0, r.b), Font::Normal, t.text);
                }
                Side::PinEmpty => {
                    let how = add_key.as_deref().map(|k| format!("책갈피나 {k}로 고정해 보세요")).unwrap_or_else(|| "책갈피로 고정해 보세요".to_string());
                    g.text("이 공간에 고정한 곳이 없어요", R::new(r.l + 10.0, r.t + 4.0, r.r - 6.0, r.t + 22.0), Font::Small, t.faint);
                    g.text(&how, R::new(r.l + 10.0, r.t + 22.0, r.r - 6.0, r.t + 40.0), Font::Small, t.faint);
                }
                // '모든 공간': 위쪽과 가르는 가는 선과 구역 이름. 작은 단추는 마우스가 패널 위에 있을 때만
                Side::CommonHead => {
                    let ly = r.t - COMMON_GAP / 2.0;
                    g.fill(R::new(r.l + 10.0, ly - 0.5, r.r - 2.0, ly + 0.5), if t.high_contrast { t.border } else { alpha(t.text, 0.08) });
                    g.text("모든 공간", R::new(r.l + 10.0, r.t, r.r - 8.0, r.b), Font::Small, t.faint);
                    for (m, mr) in &minis {
                        let glyph = match m {
                            Side::FavSort => G_SORT,
                            Side::FavNewGroup => G_NEW_GROUP,
                            _ => G_ADD,
                        };
                        mini_btn(g, *mr, glyph, hover == Hit::Side(*m));
                    }
                }
                // 작은 단추는 '모든 공간' 줄에서 그렸다. FavEmpty는 끌어 놓을 때만 쓴다
                Side::FavAdd | Side::FavSort | Side::FavNewGroup | Side::FavEmpty => {}
                Side::Fav(row) => {
                    let dragged = moving.is_some_and(|(from, ..)| from == row);
                    let fg = if dragged { t.faint } else { t.text };
                    let lx = r.l + label_x(row);
                    match row {
                        Row::Group(gi) => {
                            let Some(group) = favs.groups.get(gi) else { continue };
                            side_row_bg(g, &t, r, false, hover == Hit::Side(s));
                            let tw = R::new(r.l + 8.0, r.t, r.l + 24.0, r.b);
                            g.glyph(if group.collapsed { G_TWIST_CLOSED } else { G_TWIST_OPEN }, tw, true, t.faint);
                            if editing != Some(row) {
                                g.text(&group.name, R::new(lx, r.t, r.r - 8.0, r.b), Font::Normal, fg);
                            }
                            // 항목을 그룹 머리 줄에 끌어 놓으면 그 그룹으로
                            if let Some((Row::Item { .. }, Row::Group(at), _)) = moving {
                                if at == gi {
                                    g.stroke_round(r.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                                }
                            }
                        }
                        Row::Item { group, index } => {
                            let icon = fav_icons.next().unwrap_or(-1);
                            let Some(f) = favs.item(group, index) else { continue };
                            side_row_bg(g, &t, r, f.is_dir && is_current(&f.path), hover == Hit::Side(s));
                            // 이미 있는 것을 다시 넣으려 했을 때: 그 줄을 잠깐 강조
                            if flash_row == Some(row) {
                                g.fill_round(r, R_ROW, t.accent_soft);
                                g.stroke_round(r.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                            }
                            g.icon(icon, icon_at(r, lx - 26.0));
                            if editing != Some(row) {
                                g.text(&f.name, R::new(lx, r.t, r.r - 8.0, r.b), Font::Normal, fg);
                            }
                        }
                    }
                    // 키보드 포커스 테두리 (이름을 바꾸는 중이면 입력 상자 테두리만)
                    if sel_row == Some(row) && editing != Some(row) {
                        g.stroke_round(r.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
                    }
                    if editing == Some(row) {
                        let er = R::new(lx - 4.0, r.t + 3.0, r.r - 6.0, r.b - 3.0);
                        g.fill_round(er, R_ROW, t.pane);
                        g.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                    }
                }
                Side::Computer => {
                    side_row_bg(g, &t, r, at_computer, hover == Hit::Side(s));
                    let ir = icon_at(r, r.l + 10.0);
                    if places.computer_icon >= 0 {
                        g.icon(places.computer_icon, ir);
                    } else {
                        g.glyph(0xEC4E, R::new(ir.l, r.t, ir.r, r.b), false, t.dim);
                    }
                    g.text("내 PC", R::new(r.l + 36.0, r.t, r.r - 8.0, r.b), Font::Normal, t.text);
                }
            }
        }
        // 즐겨찾기를 끌어서 옮길 자리: 그룹 머리 줄에 항목을 놓는 게 아니면 줄 사이 선
        if let Some((from, at, below)) = moving {
            let line_y = match (from, at) {
                (Row::Item { .. }, Row::Group(_)) => None,
                (Row::Item { .. }, _) => fav_rect_in(&slots, at).map(|r| if below { r.b } else { r.t }),
                (Row::Group(_), _) => {
                    let slot = favs.group_drop_slot(at, below);
                    if slot < favs.groups.len() {
                        fav_rect_in(&slots, Row::Group(slot)).map(|r| r.t)
                    } else {
                        slots.iter().filter(|(_, s)| matches!(s, Slot::Item(Side::Fav(_)))).map(|(r, _)| r.b).last()
                    }
                }
            };
            if let Some(ly) = line_y {
                let ly = ly + dy;
                g.fill_round(R::new(dx + ROW_L + 8.0, ly - 1.5, dx + rr - 8.0, ly + 1.5), 1.5, t.accent);
            }
        }
        // 파일을 끌어 놓으려 할 때: 놓일 쪽(이 공간에 고정하는 위쪽, 또는 '모든 공간')의 줄 전체를 강조
        let area_of = |want: fn(Side) -> bool| {
            let mut rows = slots.iter().filter(|(_, Slot::Item(s))| want(*s)).map(|(r, _)| *r);
            let first = rows.next()?;
            let last = rows.next_back().unwrap_or(first);
            Some(R::new(dx + ROW_L - 2.0, first.t + dy - 2.0, dx + rr + 2.0, last.b + dy + 2.0))
        };
        let drop_area = match side_drop {
            Some(Side::PinEmpty) => area_of(|s| matches!(s, Side::Pin(_) | Side::PinEmpty)),
            Some(Side::FavEmpty) => area_of(|s| matches!(s, Side::Computer | Side::Fav(_))),
            _ => None,
        };
        if let Some(area) = drop_area {
            g.fill_round(area, 8.0, alpha(t.accent, 0.08));
            g.stroke_round(area.inset(0.75, 0.75), 8.0, t.accent, 1.5);
        }
        g.unclip();
    }
}

/// 내 PC 줄의 오른쪽 버튼 메뉴
pub(super) fn show_side_menu(hwnd: HWND, side: Side, pt: POINT) {
    let Some(loc) = with_app(|a| a.side_path(side)).flatten() else { return };
    let cmd = fav_ui::popup_menu(hwnd, pt, &[(1, "열기"), (2, "새 탭에서 열기")]);
    if cmd == 1 || cmd == 2 {
        with_app(|a| {
            let p = a.active;
            if cmd == 2 {
                a.new_tab(p, Some(loc), true);
            } else {
                a.navigate(p, loc, None);
            }
            a.close_peek();
        });
    }
}

/// 작업 줄 오른쪽 `…` 메뉴: 도움말·설정·정보 (예전 사이드바 아래쪽 줄을 대신한다)
pub(super) fn show_more_menu(hwnd: HWND, pt: POINT) {
    use crate::keys::Action;
    let Some((help, settings)) = with_app(|a| {
        let label = |act: Action, name: &str| match a.keys.get(act) {
            Some(c) => format!("{name}\t{}", c.label()),
            None => name.to_string(),
        };
        (label(Action::Help, "도움말"), label(Action::Settings, "설정"))
    }) else {
        return;
    };
    let cmd = fav_ui::popup_menu(hwnd, pt, &[(1, &help), (2, &settings), (0, ""), (3, "Sakadu 정보")]);
    with_app(|a| match cmd {
        1 => a.toggle_settings(settings_ui::Page::Help),
        2 => a.toggle_settings(settings_ui::Page::General),
        3 => a.toggle_settings(settings_ui::Page::About),
        _ => {}
    });
}

/// 줄 안에서 이름이 시작하는 x (아이콘·접기 표시 다음). 그룹 안 항목은 들여 쓴다
fn label_x(row: Row) -> f32 {
    match row {
        Row::Item { group: Some(_), .. } => 36.0 + INDENT,
        _ => 36.0,
    }
}

fn fav_rect_in(slots: &[(R, Slot)], row: Row) -> Option<R> {
    slots.iter().find(|(_, s)| matches!(s, Slot::Item(Side::Fav(r)) if *r == row)).map(|(r, _)| *r)
}
