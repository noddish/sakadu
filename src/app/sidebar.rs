//! 창 왼쪽의 아이콘 막대와 그 옆에 여는 패널.
//! 막대에는 바로가기(내 PC·드라이브·검색에 넣은 폴더)·즐겨찾기·검색·설정 아이콘이 늘 보이고, 패널은 한 번에 하나만 연다.
//! 검색 패널(palette.rs)은 늘 고정으로만 열린다 (떠 있다가 닫히면 입력 중인 검색이 사라지니까).
//! 아이콘에 잠깐 머무르면 패널이 칸 위에 떠서 열리고(마우스가 떠나거나 위치를 고르면 닫힘), 아이콘을 누르거나 패널의 핀을 누르면
//! 고정돼 칸을 밀어내며 오른쪽 가장자리를 끌어 너비를 바꿀 수 있다.
//! 패널 가운데 목록의 줄 배치는 `side_slots` 한 곳에서 정해 그리기·누르기·끌어 놓기가 같이 쓴다.
//! 즐겨찾기의 동작(열기·추가·이름·그룹·키보드·끌어서 옮기기)은 fav_ui.rs

use super::*;
use crate::favorites::Row;

/// 아이콘 막대 너비 (DIP). 패널이나 칸 영역은 그 오른쪽부터
pub(super) const BAR_W: f32 = 44.0;
const BAR_ITEM: f32 = 32.0;
const BAR_PITCH: f32 = 44.0;
/// 패널 머리 줄 (제목, 작은 단추, 핀)
const HEADER_H: f32 = 44.0;
pub(super) const SIDE_ROW_H: f32 = 32.0;
const ROW_L: f32 = 8.0;
const HEAD_GAP: f32 = 16.0;
const HEAD_H: f32 = 24.0;
const MINI: f32 = 28.0;
const DRIVE_H: f32 = 44.0;
/// 그룹 안 항목의 들여쓰기
const INDENT: f32 = 20.0;
/// 아이콘에 머무른 뒤 패널이 떠서 열리기까지
const HOVER_OPEN_MS: u32 = 300;
/// 떠 있는 패널 밖으로 나간 뒤 닫히기까지
const PEEK_CLOSE_MS: u32 = 400;
pub(super) const TIMER_BAR_HOVER: usize = 46;
pub(super) const TIMER_SIDE_CLOSE: usize = 47;
/// 패널이 열릴 때 펼쳐지는 움직임 (Windows에서 애니메이션을 끄면 하지 않는다)
pub(super) const TIMER_SIDE_ANIM: usize = 48;
const SIDE_ANIM_SECS: f32 = 0.18;
const G_PLACES: u16 = 0xE8B7;
pub(super) const G_PIN: u16 = 0xE718;
/// 채운 핀의 안쪽 (바늘이 없어 G_PIN을 겹쳐 그린다: draw_pin)
const G_PIN_FILL: u16 = 0xE841;
const G_SORT: u16 = 0xE8CB;
const G_NEW_GROUP: u16 = 0xE8F4;
const G_TWIST_OPEN: u16 = 0xE70D;
const G_TWIST_CLOSED: u16 = 0xE76C;

/// 막대 아이콘으로 여는 패널
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Places,
    Favorites,
    Search,
}

impl Panel {
    pub const ALL: [Panel; 3] = [Panel::Places, Panel::Favorites, Panel::Search];

    /// 세션에 저장하는 이름
    pub fn key(self) -> &'static str {
        match self {
            Panel::Places => "places",
            Panel::Favorites => "favorites",
            Panel::Search => "search",
        }
    }

    pub fn from_key(key: &str) -> Option<Panel> {
        Panel::ALL.into_iter().find(|p| p.key() == key)
    }

    pub fn title(self) -> &'static str {
        match self {
            Panel::Places => "바로가기",
            Panel::Favorites => "즐겨찾기",
            Panel::Search => "검색",
        }
    }

    fn glyph(self) -> u16 {
        match self {
            Panel::Places => G_PLACES,
            Panel::Favorites => fav_ui::G_STAR,
            Panel::Search => G_SEARCH,
        }
    }
}

/// 바로가기 패널에서 접을 수 있는 구역
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fold {
    Drives,
    Folders,
}

impl Fold {
    pub const ALL: [Fold; 2] = [Fold::Drives, Fold::Folders];

    /// 세션에 저장하는 이름
    pub fn key(self) -> &'static str {
        match self {
            Fold::Drives => "drives",
            Fold::Folders => "folders",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Fold::Drives => "드라이브",
            Fold::Folders => "검색에 넣은 폴더",
        }
    }
}

/// 패널 안에서 누를 수 있는 것
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Side {
    /// 바로가기 패널 구역 머리 (누르면 접고 펼친다)
    Fold(Fold),
    FavNewGroup,
    FavSort,
    FavAdd,
    Fav(Row),
    /// 즐겨찾기가 비었을 때의 안내 자리 (여기에 끌어 놓아도 추가된다)
    FavEmpty,
    Computer,
    Drive(usize),
    /// 지금 칸의 폴더(공유폴더·USB)를 검색에 넣기
    FolderAdd,
    Folder(usize),
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
        let drives = crate::listing::list_drives()
            .into_iter()
            .map(|e| Drive { name: e.name_string(), path: e.target.as_deref().unwrap_or_default().to_string(), icon: e.icon, free: e.free, total: e.size })
            .collect();
        shell::post_box(owner.get(), crate::WM_APP_PLACES, Box::new(Places { computer_icon: shell::computer_icon(), drives }));
    });
}

/// 창 바탕 위에 한 단계 들어간 면 (칸 나누기 판, 지금 위치 줄)
pub(super) fn well(t: &Theme) -> Color {
    if t.high_contrast { t.window } else { t.sel_inactive }
}

/// 고르기 판(칸 나누기, 검색 범위, 설정의 고르기)의 들어간 바탕. 어두운 테마는 바탕보다 어둡게 겹친다
pub(super) fn seg_track(t: &Theme) -> Color {
    if t.high_contrast {
        t.window
    } else if t.dark {
        Color { r: 0.0, g: 0.0, b: 0.0, a: 0.28 }
    } else {
        t.sel_inactive
    }
}

/// 고르기 판에서 고른 칸·켜진 단추: 밝은 테마는 흰 면 + 아주 작은 그림자, 어두운 테마는 한 단계 밝은 면 + 위쪽 빛 선 (떠 보이게)
pub(super) fn seg_on(g: &Gfx, r: R, radius: f32, t: &Theme) {
    if t.high_contrast {
        g.fill_round(r, radius, t.pane);
        g.stroke_round(r.inset(0.5, 0.5), radius, t.border, 1.0);
    } else if t.dark {
        g.fill_round(r, radius, t.sel_inactive);
        g.fill(R::new(r.l + radius, r.t + 0.5, r.r - radius, r.t + 1.5), Color { r: 1.0, g: 1.0, b: 1.0, a: 0.10 });
    } else {
        super::soft_shadow(g, r, radius, 3.0, 0.12, false);
        g.fill_round(r, radius, t.pane);
    }
}

/// 줄의 마우스 올림 바탕: 창 바탕에서도 보이도록 반투명으로 겹친다
pub(super) fn hover_fill(t: &Theme) -> Color {
    if t.high_contrast {
        t.hover
    } else if t.dark {
        Color { r: 1.0, g: 1.0, b: 1.0, a: 0.06 }
    } else {
        Color { r: 0.06, g: 0.09, b: 0.16, a: 0.05 }
    }
}

/// 핀 그림. 고정했으면 채운 핀(E841)에 빈 핀(E718)을 겹쳐 바늘까지 그린다 (E841만으로는 바늘 없는 덩어리)
pub(super) fn draw_pin(g: &Gfx, r: R, pinned: bool, c: Color) {
    if pinned {
        g.glyph(G_PIN_FILL, r, false, c);
    }
    g.glyph(G_PIN, r, false, c);
}

fn bar_item(y: f32) -> R {
    let l = (BAR_W - BAR_ITEM) / 2.0;
    R::new(l, y, l + BAR_ITEM, y + BAR_ITEM)
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

    pub(super) fn bar_rect(&self, p: Panel) -> R {
        let i = Panel::ALL.iter().position(|x| *x == p).unwrap_or(0) as f32;
        bar_item(6.0 + i * BAR_PITCH)
    }

    /// 막대의 검색 아이콘 (누르면 검색 패널)
    pub(super) fn search_button(&self) -> R {
        self.bar_rect(Panel::Search)
    }

    /// 막대 맨 아래 설정
    pub(super) fn settings_button(&self) -> R {
        bar_item(self.h - 8.0 - BAR_ITEM)
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
        Some(R::new(r.l, r.t, r.r, r.t + HEADER_H))
    }

    pub(super) fn side_pin_rect(&self) -> Option<R> {
        let h = self.side_header()?;
        let t = h.t + (HEADER_H - MINI) / 2.0;
        Some(R::new(h.r - 8.0 - MINI, t, h.r - 8.0, t + MINI))
    }

    /// 머리 줄의 작은 단추들 (핀 왼쪽, 핀에 가까운 것부터)
    fn side_minis(&self) -> &'static [Side] {
        match self.side_panel() {
            Some(Panel::Favorites) => &[Side::FavAdd, Side::FavSort, Side::FavNewGroup],
            _ => &[],
        }
    }

    fn side_mini_rect(&self, i: usize) -> Option<R> {
        let pin = self.side_pin_rect()?;
        let r = pin.l - 4.0 - i as f32 * (MINI + 2.0);
        Some(R::new(r - MINI, pin.t, r, pin.b))
    }

    /// 가운데 목록이 보이는 곳 (머리 줄 아래부터 패널 아래쪽까지). 패널이 없으면 빈 사각형
    pub(super) fn side_list(&self) -> R {
        match self.side_panel_rect() {
            Some(r) => R::new(r.l, r.t + HEADER_H + 2.0, r.r, (r.b - 6.0).max(r.t + HEADER_H + 2.0)),
            None => R::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    /// 지금 칸의 폴더가 디스크 검색 밖(공유폴더·USB)이고 아직 검색에 넣지 않았으면 그 폴더.
    /// 드라이브 종류를 묻는 시스템 호출은 폴더가 바뀔 때만 한다
    pub(super) fn side_outside(&self) -> Option<String> {
        let dir = self.panes[self.active].tab().loc.dir()?.to_string();
        let outside = {
            let mut cache = self.outside_cache.borrow_mut();
            if cache.0 != dir {
                *cache = (dir.clone(), crate::folders::outside_disk_index(&dir));
            }
            cache.1
        };
        (outside && self.folders.covering(&dir).is_none()).then_some(dir)
    }

    /// 보이는 패널의 가운데 목록 자리들과 전체 높이. 좌표는 스크롤 전, 목록 맨 위·왼쪽이 0
    fn side_slots(&self) -> (Vec<(R, Slot)>, f32) {
        let mut out: Vec<(R, Slot)> = Vec::new();
        let rr = self.side_w_shown() - 8.0;
        let mini = |h: R, i: usize| {
            let r = h.r - i as f32 * (MINI + 2.0);
            R::new(r - MINI, h.t, r, h.b)
        };
        // 접을 수 있는 구역 머리: 줄 전체를 누를 수 있다 (오른쪽 끝의 작은 단추가 위에 놓인다)
        let fold_head = |out: &mut Vec<(R, Slot)>, y: &mut f32, f: Fold| -> R {
            *y += HEAD_GAP;
            let r = R::new(ROW_L, *y, rr, *y + HEAD_H);
            out.push((r, Slot::Item(Side::Fold(f))));
            *y += HEAD_H + 4.0;
            R::new(ROW_L + 10.0, r.t, rr - 6.0, r.b)
        };
        let mut y = 4.0;
        match self.side_panel() {
            // 검색 패널의 결과 목록은 palette.rs가 따로 둔다
            None | Some(Panel::Search) => {}
            Some(Panel::Favorites) => {
                let rows = self.favs.rows();
                for row in &rows {
                    out.push((R::new(ROW_L, y, rr, y + SIDE_ROW_H), Slot::Item(Side::Fav(*row))));
                    y += SIDE_ROW_H;
                }
                if rows.is_empty() {
                    out.push((R::new(ROW_L, y, rr, y + 44.0), Slot::Item(Side::FavEmpty)));
                    y += 44.0;
                }
            }
            Some(Panel::Places) => {
                // 드라이브: 내 PC, 드라이브마다 남은 용량 막대 (첫 머리는 위 여백 없이)
                y -= HEAD_GAP;
                fold_head(&mut out, &mut y, Fold::Drives);
                if !self.is_folded(Fold::Drives) {
                    out.push((R::new(ROW_L, y, rr, y + SIDE_ROW_H), Slot::Item(Side::Computer)));
                    y += SIDE_ROW_H;
                    for i in 0..self.places.drives.len() {
                        out.push((R::new(ROW_L, y, rr, y + DRIVE_H), Slot::Item(Side::Drive(i))));
                        y += DRIVE_H;
                    }
                }

                // 검색에 넣은 폴더: 지금 칸이 검색 밖이면 머리 줄에 +
                let h = fold_head(&mut out, &mut y, Fold::Folders);
                if self.side_outside().is_some() {
                    out.push((mini(h, 0), Slot::Item(Side::FolderAdd)));
                }
                if !self.is_folded(Fold::Folders) {
                    let n = self.folders.list().len();
                    for i in 0..n {
                        out.push((R::new(ROW_L, y, rr, y + SIDE_ROW_H), Slot::Item(Side::Folder(i))));
                        y += SIDE_ROW_H;
                    }
                    if n == 0 {
                        y += 40.0;
                    }
                }
            }
        }
        (out, y + 8.0)
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
        if self.side_panel() != Some(Panel::Favorites) {
            // 즐겨찾기 줄이 사라지면 그 줄의 이름 입력은 확정하고 키보드는 칸으로
            if matches!(self.edit_mode, EditMode::FavRename(_)) {
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

    /// 마우스가 움직였다 (끌기 중이 아닐 때): 아이콘에 머무르면 패널을 띄우고, 떠 있는 패널 밖으로 나가면 닫을 준비
    pub(super) fn side_track(&mut self, x: f32, y: f32) {
        // 머물러 떠서 열기는 패널을 고정하지 않았을 때만: 고정한 패널이 있으면 그 위에 다른 패널이 겹쳐 뜨지 않고, 누를 때만 바꾼다
        let over = match self.hover {
            Hit::Bar(p) if self.side_pinned.is_none() && self.side_panel() != Some(p) && self.settings.is_none() => Some(p),
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
        if self.hover == Hit::Bar(p) && self.side_pinned.is_none() && self.side_panel() != Some(p) && matches!(self.drag, Drag::None) && self.settings.is_none() && self.edit_mode == EditMode::None {
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

    /// 막대 (x < BAR_W)에서 누를 수 있는 것
    pub(super) fn bar_hit(&self, x: f32, y: f32) -> Hit {
        // 검색 아이콘은 머물러도 띄우지 않으니 Bar가 아닌 따로 된 Hit
        if self.search_button().contains(x, y) {
            return Hit::SearchButton;
        }
        for p in Panel::ALL {
            if self.bar_rect(p).contains(x, y) {
                return Hit::Bar(p);
            }
        }
        if self.settings_button().contains(x, y) {
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
        if self.side_minis_shown {
            for (i, s) in self.side_minis().iter().enumerate() {
                if self.side_mini_rect(i).is_some_and(|r| r.contains(x, y)) {
                    return Hit::Side(*s);
                }
            }
        }
        if self.side_list().contains(x, y) {
            let (slots, _) = self.side_slots();
            // 겹친 자리(구역 머리 줄 위의 + 단추)는 나중에 놓인 것이 위에 있다
            for (r, Slot::Item(side)) in slots.into_iter().rev() {
                if side == Side::FolderAdd && !self.side_minis_shown {
                    continue;
                }
                if self.side_at(r).contains(x, y) {
                    return Hit::Side(side);
                }
            }
        }
        Hit::SidePanel
    }

    /// 끌어 놓기 대상: 즐겨찾기 패널이면 어디든 즐겨찾기에 넣기, 바로가기 패널은 드라이브·검색 폴더 줄이면 그 폴더
    pub(super) fn side_drop_at(&self, x: f32, y: f32) -> Option<(Side, Option<String>)> {
        if !self.side_panel_rect().is_some_and(|r| r.contains(x, y)) {
            return None;
        }
        match self.side_panel()? {
            Panel::Search => None,
            Panel::Favorites => Some((Side::FavEmpty, None)),
            Panel::Places => match self.sidebar_hit(x, y) {
                Hit::Side(s @ Side::Drive(i)) => Some((s, self.places.drives.get(i).map(|d| d.path.clone()))),
                Hit::Side(s @ Side::Folder(i)) => Some((s, self.folders.list().get(i).map(|(p, _)| p.clone()))),
                _ => None,
            },
        }
    }

    /// 드라이브·검색 폴더·내 PC 줄의 경로
    fn side_path(&self, s: Side) -> Option<Location> {
        match s {
            Side::Computer => Some(Location::Computer),
            Side::Drive(i) => self.places.drives.get(i).map(|d| Location::Dir(d.path.clone())),
            Side::Folder(i) => self.folders.list().get(i).map(|(p, _)| Location::Dir(p.clone())),
            _ => None,
        }
    }

    /// 패널 줄을 연다 (new_tab: 새 탭에서). 떠 있는 패널은 위치를 골랐으니 닫는다
    pub(super) fn side_open(&mut self, s: Side, new_tab: bool) {
        if let Side::Fav(row) = s {
            return self.fav_open_row(row, new_tab);
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
            Side::Fold(f) => self.toggle_fold(f),
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
            Side::FolderAdd => {
                if let Some(dir) = self.side_outside() {
                    let p = self.active;
                    match self.folders.add(&dir) {
                        Ok(()) => self.flash(p, format!("검색에 넣었어요 · 훑는 동안에도 찾을 수 있어요: {dir}")),
                        Err(e) => self.flash(p, format!("검색 폴더 목록을 저장하지 못했어요: {e}")),
                    }
                }
            }
            Side::Fav(row) => self.fav_mouse_down(row, y, dbl),
            Side::FavEmpty => {}
            // 두 번째 누름은 무시한다 (첫 누름에서 이미 열었다)
            _ if dbl => {}
            _ => self.side_open(s, key_down(VK_CONTROL)),
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

    /// 오른쪽 버튼: 즐겨찾기는 즐겨찾기 메뉴, 드라이브·검색 폴더는 그 메뉴
    pub(super) fn side_context_menu(&mut self, s: Side, x: f32, y: f32) {
        let pt = self.to_screen(x, y);
        match s {
            Side::Fav(row) => {
                if let Some(i) = self.favs.row_index(row) {
                    self.fav_sel = i;
                }
                self.pending.push(Deferred::FavMenu { row, pt });
            }
            Side::Computer | Side::Drive(_) | Side::Folder(_) => self.pending.push(Deferred::SideMenu { side: s, pt }),
            _ => {}
        }
        self.invalidate();
    }

    /// 마우스를 올려 두면 보여 줄 말풍선 (자리, 글자)
    pub(super) fn side_tip(&self, s: Side) -> Option<(R, String)> {
        use crate::keys::Action;
        let named = |a: Action, name: &str| match self.keys.get(a) {
            Some(c) => format!("{name} ({})", c.label()),
            None => name.to_string(),
        };
        let text = match s {
            Side::FavAdd => named(Action::AddFavorite, "고른 항목이나 지금 폴더를 즐겨찾기에"),
            Side::FavSort => "정렬".to_string(),
            Side::FavNewGroup => "새 그룹".to_string(),
            Side::FolderAdd => "지금 폴더도 검색하기".to_string(),
            Side::Fav(Row::Item { group, index }) => self.favs.item(group, index)?.path.clone(),
            Side::Drive(i) => self.places.drives.get(i)?.path.clone(),
            Side::Folder(i) => {
                let (path, state) = self.folders.list().get(i)?.clone();
                match state {
                    crate::folders::State::Scanning => format!("{path} · 훑는 중"),
                    crate::folders::State::Ready(n) => format!("{path} · {}개", util::grouped(n as u64)),
                    crate::folders::State::Offline => format!("{path} · 연결 안 됨"),
                }
            }
            _ => return None,
        };
        let r = self.side_item_rect(s)?;
        if self.side_minis().contains(&s) {
            return Some((r, text));
        }
        // 목록 밖으로 스크롤된 줄에는 말풍선을 붙이지 않는다
        let list = self.side_list();
        (r.b > list.t && r.t < list.b).then_some((r, text))
    }

    // ---------- 구역 접기·마우스를 올릴 때만 보이는 단추·여는 움직임 ----------

    pub(super) fn is_folded(&self, f: Fold) -> bool {
        self.folded[f as usize]
    }

    fn toggle_fold(&mut self, f: Fold) {
        self.folded[f as usize] = !self.folded[f as usize];
        self.side_clamp_scroll();
        self.refresh_hover();
        self.invalidate();
    }

    /// 패널 머리의 작은 단추(추가·정렬·새 그룹, 검색 폴더 +)를 보일지: 마우스가 패널 위에 있거나 키보드가 즐겨찾기에 있을 때만
    fn side_hot(&self) -> bool {
        self.fav_focus || matches!(self.hover, Hit::Side(_) | Hit::SidePin | Hit::SidePanel)
    }

    /// 패널이 새로 열렸다: 막대 쪽에서 펼쳐지게 한다 (검색 패널은 입력 상자가 함께 움직이지 못해 바로 연다)
    fn start_side_anim(&mut self) {
        let mut on = BOOL(1);
        let read = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut BOOL as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
        if read.is_ok() && !on.as_bool() {
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
        let App { gfx: g, thumb_failed, .. } = self;
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

    /// 아이콘 막대: 바로가기·즐겨찾기·검색, 맨 아래 설정. 보이는 패널(열린 검색 창)의 아이콘은 켜진 모양
    pub(super) fn paint_bar(&self) {
        let t = self.theme;
        let g = &self.gfx;
        let hover = self.hover;
        if t.high_contrast {
            g.fill(R::new(BAR_W - 1.0, 0.0, BAR_W, self.h), t.border);
        }
        let item = |r: R, glyph: u16, on: bool, hovered: bool| {
            if on {
                seg_on(g, r, R_CTRL, &t);
            } else if hovered {
                g.fill_round(r, R_CTRL, hover_fill(&t));
            }
            g.glyph(glyph, r, false, if on || hovered { t.text } else { t.dim });
        };
        for p in [Panel::Places, Panel::Favorites] {
            // 파일을 끌어 별 위에 올리면 즐겨찾기에 넣을 수 있다고 켜 보인다
            let drop = p == Panel::Favorites && self.side_drop == Some(Side::FavEmpty) && self.side_panel() != Some(Panel::Favorites);
            item(self.bar_rect(p), p.glyph(), self.side_panel() == Some(p) || drop, hover == Hit::Bar(p));
        }
        item(self.search_button(), G_SEARCH, self.pal_shown(), hover == Hit::SearchButton);
        item(self.settings_button(), G_SETTINGS, false, hover == Hit::SettingsButton);
    }

    pub(super) fn paint_sidebar(&mut self) {
        self.side_minis_shown = false;
        match (self.side_open_progress(), self.side_panel_rect()) {
            (Some(f), Some(pr)) => {
                // 그림자까지 들어가게 조금 넉넉히
                let w = (pr.w() + 24.0) * f;
                self.gfx.clip(R::new(BAR_W, 0.0, BAR_W + w, self.h));
                self.paint_side_panel();
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
        // 끌어서 옮기는 중: (끄는 줄, 놓을 줄, 아래쪽 절반)
        let moving = match self.drag {
            Drag::FavMove { from, y } => self.fav_drop_row(y).map(|(at, below)| (from, at, below)),
            _ => None,
        };
        let side_drop = self.side_drop;
        // 즐겨찾기 아이콘은 형식 캐시에서 (가변 빌림이 필요해 먼저 모아 둔다)
        let kinds: Vec<(bool, String)> = fav_rows
            .iter()
            .filter_map(|r| match r {
                Row::Item { group, index } => self.favs.item(*group, *index).map(|f| (f.is_dir, util::file_name(&f.path).rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default())),
                Row::Group(_) => None,
            })
            .collect();
        let mut fav_icons = kinds.iter().map(|(d, e)| self.type_icon(e, *d)).collect::<Vec<_>>().into_iter();
        let folder_icon = self.type_icon("", true);
        let folders = self.folders.list();
        let add_key = self.keys.get(crate::keys::Action::AddFavorite).map(crate::keys::Combo::label);
        let pin = self.side_pin_rect();
        // 머리의 작은 단추는 마우스가 패널 위에 있거나 키보드가 즐겨찾기에 있을 때만
        let hot = self.side_hot();
        self.side_minis_shown = hot;
        let minis: Vec<(Side, R)> = if hot { self.side_minis().iter().enumerate().filter_map(|(i, s)| self.side_mini_rect(i).map(|r| (*s, r))).collect() } else { Vec::new() };
        let title_r = (0..self.side_minis().len()).filter_map(|i| self.side_mini_rect(i)).map(|r| r.l).chain(pin.map(|r| r.l)).fold(pr.r - 8.0, f32::min) - 6.0;
        let folded = self.folded;
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
        // 머리 줄: 제목, (즐겨찾기면 추가·정렬·새 그룹), 핀
        g.text(panel.title(), R::new(pr.l + ROW_L + 10.0, pr.t, title_r, pr.t + HEADER_H), Font::Strong, t.text);
        let mini_btn = |g: &Gfx, r: R, glyph: u16, hovered: bool| {
            if hovered {
                g.fill_round(r, R_CTRL, hover_fill(&t));
            }
            g.glyph(glyph, r, false, if hovered { t.text } else { t.dim });
        };
        for (s, r) in &minis {
            let glyph = match s {
                Side::FavSort => G_SORT,
                Side::FavNewGroup => G_NEW_GROUP,
                _ => G_ADD,
            };
            mini_btn(g, *r, glyph, hover == Hit::Side(*s));
        }
        if let Some(r) = pin {
            let hot = hover == Hit::SidePin;
            if hot {
                g.fill_round(r, R_CTRL, hover_fill(&t));
            }
            draw_pin(g, r, !floating, if hot { t.text } else { t.dim });
        }

        // 가운데 목록
        g.clip(list);
        let row_bg = |g: &Gfx, r: R, current: bool, hovered: bool| {
            if current {
                g.fill_round(r, R_ROW, well(&t));
                // 고대비에서는 들어간 면이 창 바탕과 같은 색이라 테두리로 지금 위치를 보인다
                if t.high_contrast {
                    g.stroke_round(r.inset(0.5, 0.5), R_ROW, t.border, 1.0);
                }
            }
            if hovered {
                g.fill_round(r, R_ROW, hover_fill(&t));
            }
        };
        for (r, slot) in &slots {
            let r = R::new(r.l + dx, r.t + dy, r.r + dx, r.b + dy);
            if r.b < list.t || r.t > list.b {
                if let Slot::Item(Side::Fav(Row::Item { .. })) = slot {
                    fav_icons.next();
                }
                continue;
            }
            match *slot {
                Slot::Item(s @ Side::FolderAdd) => {
                    if hot {
                        mini_btn(g, r, G_ADD, hover == Hit::Side(s));
                    }
                }
                // 구역 머리: 접힘 표시와 이름. 올리면 옅은 바탕
                Slot::Item(s @ Side::Fold(f)) => {
                    if hover == Hit::Side(s) {
                        g.fill_round(r, R_ROW, hover_fill(&t));
                    }
                    let closed = folded[f as usize];
                    let lw = g.measure(f.label(), Font::Small);
                    g.text(f.label(), R::new(r.l + 10.0, r.t, r.r - 8.0, r.b), Font::Small, t.faint);
                    let cx = r.l + 10.0 + lw + 4.0;
                    g.glyph(if closed { G_TWIST_CLOSED } else { G_TWIST_OPEN }, R::new(cx, r.t + 1.0, cx + 12.0, r.b + 1.0), true, t.faint);
                }
                // 머리 줄 단추는 위에서 그렸다
                Slot::Item(Side::FavAdd | Side::FavSort | Side::FavNewGroup) => {}
                Slot::Item(Side::FavEmpty) => {
                    if side_drop == Some(Side::FavEmpty) {
                        g.fill_round(r, R_ROW, alpha(t.accent, 0.10));
                    }
                    let how = add_key.as_deref().map(|k| format!("별(☆)이나 {k}로 넣어 보세요")).unwrap_or_else(|| "경로 오른쪽 별(☆)로 넣어 보세요".to_string());
                    g.text("즐겨찾기가 비어 있어요", R::new(r.l + 10.0, r.t + 4.0, r.r - 6.0, r.t + 22.0), Font::Small, t.faint);
                    g.text(&how, R::new(r.l + 10.0, r.t + 22.0, r.r - 6.0, r.t + 40.0), Font::Small, t.faint);
                }
                Slot::Item(s @ Side::Fav(row)) => {
                    let dragged = moving.is_some_and(|(from, ..)| from == row);
                    let fg = if dragged { t.faint } else { t.text };
                    let lx = r.l + label_x(row);
                    match row {
                        Row::Group(gi) => {
                            let Some(group) = favs.groups.get(gi) else { continue };
                            row_bg(g, r, false, hover == Hit::Side(s));
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
                            row_bg(g, r, f.is_dir && is_current(&f.path), hover == Hit::Side(s));
                            // 이미 있는 것을 다시 넣으려 했을 때: 그 줄을 잠깐 강조
                            if flash_row == Some(row) {
                                g.fill_round(r, R_ROW, t.accent_soft);
                                g.stroke_round(r.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                            }
                            let il = lx - 26.0;
                            g.icon(icon, R::new(il, r.t + 8.0, il + 16.0, r.t + 24.0));
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
                Slot::Item(s @ Side::Computer) => {
                    row_bg(g, r, at_computer, hover == Hit::Side(s));
                    let il = r.l + 10.0;
                    if places.computer_icon >= 0 {
                        g.icon(places.computer_icon, R::new(il, r.t + 8.0, il + 16.0, r.t + 24.0));
                    } else {
                        g.glyph(0xEC4E, R::new(il, r.t, il + 16.0, r.b), false, t.dim);
                    }
                    g.text("내 PC", R::new(r.l + 36.0, r.t, r.r - 8.0, r.b), Font::Normal, t.text);
                }
                Slot::Item(s @ Side::Drive(i)) => {
                    let Some(d) = places.drives.get(i) else { continue };
                    row_bg(g, r, is_current(&d.path), hover == Hit::Side(s) || side_drop == Some(s));
                    if side_drop == Some(s) {
                        g.stroke_round(r.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                    }
                    let head = R::new(r.l, r.t + 6.0, r.r, r.t + 24.0);
                    g.icon(d.icon, R::new(r.l + 10.0, head.t + 1.0, r.l + 26.0, head.t + 17.0));
                    let free = (d.total > 0).then(|| format!("{} 남음", format_size(d.free)));
                    let fw = free.as_deref().map_or(0.0, |f| g.measure(f, Font::Small));
                    g.text(&d.name, R::new(r.l + 36.0, head.t, r.r - 8.0, head.b), Font::Normal, t.text);
                    if let Some(f) = &free {
                        g.text(f, R::new(r.r - 10.0 - fw, head.b, r.r - 8.0, head.b + 18.0), Font::Small, t.faint);
                        // 쓴 만큼 채운 막대. 거의 찼으면(90% 넘게) 빨간색
                        let used = (1.0 - d.free as f32 / d.total as f32).clamp(0.0, 1.0);
                        let bar = R::new(r.l + 36.0, head.b + 7.0, (r.r - 10.0 - fw - 8.0).max(r.l + 60.0), head.b + 11.0);
                        g.fill_round(bar, 2.0, alpha(t.text, 0.10));
                        let fill = if used > 0.9 { Color { r: 0.80, g: 0.20, b: 0.16, a: 1.0 } } else { t.faint };
                        g.fill_round(R::new(bar.l, bar.t, bar.l + bar.w() * used, bar.b), 2.0, fill);
                    }
                }
                Slot::Item(s @ Side::Folder(i)) => {
                    let Some((path, state)) = folders.get(i) else { continue };
                    row_bg(g, r, is_current(path), hover == Hit::Side(s) || side_drop == Some(s));
                    if side_drop == Some(s) {
                        g.stroke_round(r.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                    }
                    g.icon(folder_icon, R::new(r.l + 10.0, r.t + 8.0, r.l + 26.0, r.t + 24.0));
                    let note = match state {
                        crate::folders::State::Scanning => Some("훑는 중"),
                        crate::folders::State::Offline => Some("연결 안 됨"),
                        crate::folders::State::Ready(_) => None,
                    };
                    let nw = note.map_or(0.0, |n| g.measure(n, Font::Small));
                    let name = if util::file_name(path).is_empty() { path.as_str() } else { util::file_name(path) };
                    g.text(name, R::new(r.l + 36.0, r.t, r.r - 16.0 - nw, r.b), Font::Normal, if note == Some("연결 안 됨") { t.faint } else { t.text });
                    if let Some(n) = note {
                        g.text(n, R::new(r.r - 10.0 - nw, r.t, r.r - 8.0, r.b), Font::Small, t.faint);
                    }
                }
            }
        }
        // 검색 폴더가 없으면 안내
        if panel == Panel::Places && folders.is_empty() && !folded[Fold::Folders as usize] {
            if let Some((r, _)) = slots.iter().rev().find(|(_, s)| matches!(s, Slot::Item(Side::Fold(Fold::Folders)))) {
                let (l, top) = (dx + ROW_L + 10.0, r.b + dy + 4.0);
                g.text("공유폴더·USB를 열고 +를 누르면", R::new(l, top, dx + rr - 6.0, top + 18.0), Font::Small, t.faint);
                g.text("함께 찾아요", R::new(l, top + 18.0, dx + rr - 6.0, top + 36.0), Font::Small, t.faint);
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
        // 파일을 끌어 즐겨찾기 쪽에 놓으려 할 때: 즐겨찾기 목록 전체를 강조
        if side_drop == Some(Side::FavEmpty) && panel == Panel::Favorites && !favs.rows().is_empty() {
            let top = slots.first().map_or(0.0, |(r, _)| r.t);
            let bottom = slots.iter().filter(|(_, s)| matches!(s, Slot::Item(Side::Fav(_)))).map(|(r, _)| r.b).last().unwrap_or(top);
            let area = R::new(dx + ROW_L - 2.0, top + dy - 4.0, dx + rr + 2.0, bottom + dy + 2.0);
            g.fill_round(area, 8.0, alpha(t.accent, 0.08));
            g.stroke_round(area.inset(0.75, 0.75), 8.0, t.accent, 1.5);
        }
        g.unclip();
    }
}

/// 내 PC·드라이브·검색 폴더 줄의 오른쪽 버튼 메뉴
pub(super) fn show_side_menu(hwnd: HWND, side: Side, pt: POINT) {
    let Some(loc) = with_app(|a| a.side_path(side)).flatten() else { return };
    let mut items: Vec<(u32, &str)> = vec![(1, "열기"), (2, "새 탭에서 열기")];
    match side {
        Side::Drive(_) => items.extend([(0, ""), (3, "속성")]),
        Side::Folder(_) => items.extend([(0, ""), (4, "검색에서 빼기…")]),
        _ => {}
    }
    let cmd = fav_ui::popup_menu(hwnd, pt, &items);
    let dir = loc.dir().map(str::to_string);
    match (cmd, dir) {
        // 메뉴가 떠 있는 동안 드라이브 목록이 바뀌었을 수 있으니 번호로 다시 찾지 않고 메뉴를 연 줄의 위치를 연다
        (1 | 2, _) => {
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
        (3, Some(d)) => {
            let _ = shell::invoke_verb(hwnd, &[d], "properties");
        }
        (4, Some(d)) => {
            if let Some(folders) = with_app(|a| a.folders.clone()) {
                remove_search_folder(hwnd, &folders, &d);
            }
        }
        _ => {}
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
