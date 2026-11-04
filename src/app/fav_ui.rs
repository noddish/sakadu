//! 즐겨찾기 팝업. 도구 막대 버튼이나 Ctrl+B로 연다.
//! 왼쪽에는 그룹 없는 즐겨찾기와 그룹이 이름만 한 줄씩, 그룹에 마우스를 올리거나 →를 누르면 오른쪽에 그 안의 항목이 열린다.
//! 경로는 줄에 잠시 머물면 말풍선으로 보인다. 표시 이름·순서·그룹은 사용자가 정한다

use super::*;
use crate::favorites::{self, Favorite, Row, SortBy};

const FAV_W: f32 = 300.0;
const FAV_HEAD_H: f32 = 44.0;
const FAV_FOOT_H: f32 = 28.0;
const FAV_ROW_H: f32 = 32.0;
pub(super) const G_STAR: u16 = 0xE734;
const G_SORT: u16 = 0xE8CB;
const G_NEW_GROUP: u16 = 0xE8F4;
const G_GROUP: u16 = 0xE8B7;
/// 오른쪽 메뉴가 미끄러져 나오는 애니메이션
pub(super) const TIMER_FLY: usize = 40;
/// 오른쪽 메뉴 쪽으로 가는 길에 다른 줄을 지날 때, 바로 바꾸지 않고 잠시 뒤 다시 본다
pub(super) const TIMER_FLY_CLOSE: usize = 41;
/// 줄에 잠시 머물면 경로 말풍선
pub(super) const TIMER_FAV_TIP: usize = 43;
const FLY_W: f32 = 300.0;

const FAV_BUTTON_W: f32 = 100.0;

/// 그룹의 오른쪽 메뉴
#[derive(Clone, Copy, Debug)]
pub(super) struct Fly {
    group: usize,
    /// 나타난 정도 0~1
    anim: f32,
    /// 키보드가 오른쪽 메뉴에 있으면 그 줄 (마우스를 올린 줄도)
    sel: Option<usize>,
    scroll: f32,
}

impl App {
    /// 도구 막대 가운데에 즐겨찾기 버튼과 검색 상자를 한 묶음으로 둔다. 창이 좁으면 왼쪽 버튼들 바로 뒤에 붙는다
    pub(super) fn fav_button(&self) -> R {
        let w = if self.fav_button_compact() { 32.0 } else { FAV_BUTTON_W };
        let group = w + 6.0 + palette::SEARCH_W;
        let l = (self.w / 2.0 - group / 2.0).max(hidden_button().r + 16.0);
        R::new(l, 8.0, l + w, 38.0)
    }

    /// 창이 좁으면(크기 150% 등) 즐겨찾기 버튼은 별 아이콘만 두어 검색 상자가 오른쪽 버튼을 덮지 않게 한다
    pub(super) fn fav_button_compact(&self) -> bool {
        let room = self.preview_button().l - 10.0 - (hidden_button().r + 16.0 + FAV_BUTTON_W + 6.0);
        room < palette::SEARCH_MIN_W
    }
}

/// 네이티브 팝업 메뉴. id 0은 구분선. 고른 id(없으면 0)
fn popup_menu(hwnd: HWND, pt: POINT, items: &[(u32, &str)]) -> u32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        append_items(menu, items);
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;
        let _ = DestroyMenu(menu);
        cmd
    }
}

fn append_items(menu: HMENU, items: &[(u32, &str)]) {
    for (id, label) in items {
        unsafe {
            let _ = if *id == 0 {
                AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())
            } else {
                AppendMenuW(menu, MF_STRING, *id as usize, &HSTRING::from(*label))
            };
        }
    }
}

/// 즐겨찾기 줄(왼쪽 목록의 항목·그룹, 오른쪽 메뉴의 항목)의 오른쪽 버튼 메뉴
pub(super) fn show_fav_menu(hwnd: HWND, row: Row, pt: POINT) {
    let Some((groups, is_dir)) = with_app(|a| {
        let groups: Vec<(String, usize)> = a.favs.groups.iter().map(|g| (g.name.clone(), g.items.len())).collect();
        let is_dir = match row {
            Row::Item { group, index } => a.favs.item(group, index)?.is_dir,
            Row::Group(g) => {
                a.favs.groups.get(g)?;
                true
            }
        };
        Some((groups, is_dir))
    })
    .flatten() else {
        return;
    };
    match row {
        Row::Group(g) => show_group_menu(hwnd, g, &groups[g], pt),
        Row::Item { group, .. } => show_item_menu(hwnd, row, group, &groups, is_dir, pt),
    }
}

fn show_item_menu(hwnd: HWND, row: Row, current: Option<usize>, groups: &[(String, usize)], is_dir: bool, pt: POINT) {
    const NO_GROUP: u32 = 7;
    const NEW_GROUP: u32 = 8;
    const GROUP: u32 = 100;
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let Ok(sub) = CreatePopupMenu() else {
            let _ = DestroyMenu(menu);
            return;
        };
        let check = |on: bool| MF_STRING | if on { MF_CHECKED } else { MF_UNCHECKED };
        for (g, (name, _)) in groups.iter().enumerate() {
            let _ = AppendMenuW(sub, check(current == Some(g)), GROUP as usize + g, &HSTRING::from(name.as_str()));
        }
        if !groups.is_empty() {
            let _ = AppendMenuW(sub, check(current.is_none()), NO_GROUP as usize, w!("그룹 없음"));
            let _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
        }
        let _ = AppendMenuW(sub, MF_STRING, NEW_GROUP as usize, w!("새 그룹을 만들어 넣기"));
        append_items(menu, &[(1, "열기"), (2, if is_dir { "새 탭에서 열기" } else { "위치 열기 (새 탭)" }), (0, ""), (3, "이름 바꾸기\tF2"), (4, "위로 이동\tCtrl+↑"), (5, "아래로 이동\tCtrl+↓")]);
        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("그룹으로 이동"));
        append_items(menu, &[(0, ""), (6, "즐겨찾기에서 삭제\tDel")]);
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;
        let _ = DestroyMenu(menu);
        cmd
    };
    with_app(|a| match cmd {
        1 | 2 => a.fav_open_row(row, cmd == 2),
        3 => a.begin_fav_rename(row),
        4 => a.fav_nudge(row, true),
        5 => a.fav_nudge(row, false),
        6 => a.remove_favorite(row),
        NO_GROUP => a.fav_move_to_group(row, None),
        NEW_GROUP => a.fav_move_to_new_group(row),
        _ if cmd >= GROUP && ((cmd - GROUP) as usize) < groups.len() => a.fav_move_to_group(row, Some((cmd - GROUP) as usize)),
        _ => {}
    });
}

fn show_group_menu(hwnd: HWND, g: usize, group: &(String, usize), pt: POINT) {
    let cmd = popup_menu(
        hwnd,
        pt,
        &[
            (2, "이름 바꾸기\tF2"),
            (3, "위로 이동\tCtrl+↑"),
            (4, "아래로 이동\tCtrl+↓"),
            (0, ""),
            (5, "그룹 풀기 (안의 즐겨찾기는 남김)\tDel"),
            (6, "그룹과 안의 즐겨찾기 삭제…"),
        ],
    );
    if cmd == 6 {
        let (name, count) = group;
        let text = format!("‘{name}’ 그룹과 안에 든 즐겨찾기 {count}개를 지울까요?");
        if unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Sakadu"), MB_YESNO | MB_ICONQUESTION) } != IDYES {
            return;
        }
    }
    with_app(|a| match cmd {
        2 => a.begin_fav_rename(Row::Group(g)),
        3 => a.fav_nudge(Row::Group(g), true),
        4 => a.fav_nudge(Row::Group(g), false),
        5 => a.fav_ungroup(g),
        6 => a.fav_delete_group(g),
        _ => {}
    });
}

pub(super) fn show_fav_sort_menu(hwnd: HWND, pt: POINT) {
    let cmd = popup_menu(hwnd, pt, &[(1, "이름순 (가나다, 숫자는 크기순)"), (2, "이름 역순"), (3, "경로순")]);
    let by = match cmd {
        1 => SortBy::Name,
        2 => SortBy::NameDesc,
        3 => SortBy::Path,
        _ => return,
    };
    with_app(|a| a.sort_favorites(by));
}

/// 점 p가 세 점 a·b·c로 된 삼각형 안(경계 포함)에 있는가
fn in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |p1: (f32, f32), p2: (f32, f32)| (p.0 - p2.0) * (p1.1 - p2.1) - (p1.0 - p2.0) * (p.1 - p2.1);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

impl App {
    // ---------- 배치 ----------

    pub(super) fn fav_rect(&self) -> R {
        let b = self.fav_button();
        let w = FAV_W.min(self.w - 2.0 * MARGIN).max(200.0);
        let l = b.l.min(self.w - MARGIN - w).max(MARGIN);
        let rows = self.favs.rows().len().max(2) as f32;
        let max_h = (self.h - TOOLBAR_H - 12.0).max(160.0);
        let h = (FAV_HEAD_H + rows * FAV_ROW_H + FAV_FOOT_H + 8.0).min(max_h);
        R::new(l, TOOLBAR_H - 4.0, l + w, TOOLBAR_H - 4.0 + h)
    }

    fn fav_list(&self) -> R {
        let r = self.fav_rect();
        R::new(r.l + 6.0, r.t + FAV_HEAD_H, r.r - 6.0, r.b - FAV_FOOT_H)
    }

    fn fav_row(&self, i: usize) -> R {
        let list = self.fav_list();
        let y = list.t + i as f32 * FAV_ROW_H - self.fav_scroll;
        R::new(list.l, y, list.r, y + FAV_ROW_H)
    }

    /// 머리의 아이콘 버튼: (새 그룹, 정렬, 추가)
    fn fav_head_buttons(&self) -> [R; 3] {
        let r = self.fav_rect();
        let add = R::new(r.r - 38.0, r.t + 8.0, r.r - 10.0, r.t + 36.0);
        let sort = R::new(add.l - 32.0, add.t, add.l - 4.0, add.b);
        let group = R::new(sort.l - 32.0, add.t, sort.l - 4.0, add.b);
        [group, sort, add]
    }

    /// 오른쪽 메뉴 자리: 그룹 줄 높이에서 시작하고, 오른쪽에 자리가 없으면 왼쪽에, 그것도 없으면 창 오른쪽 끝에
    fn fav_fly_rect(&self) -> Option<R> {
        let fly = self.fav_fly?;
        let n = self.favs.groups.get(fly.group)?.items.len();
        let pop = self.fav_rect();
        let row = self.favs.row_index(Row::Group(fly.group)).map(|i| self.fav_row(i)).unwrap_or(pop);
        let top_min = TOOLBAR_H - 4.0;
        let h = (n.max(1) as f32 * FAV_ROW_H + 12.0).min((self.h - 8.0 - top_min).max(FAV_ROW_H + 12.0));
        let l = if pop.r + 4.0 + FLY_W <= self.w - MARGIN {
            pop.r + 4.0
        } else if pop.l - 4.0 - FLY_W >= MARGIN {
            pop.l - 4.0 - FLY_W
        } else {
            (self.w - MARGIN - FLY_W).max(MARGIN)
        };
        let t = (row.t - 6.0).min(self.h - h - 8.0).max(top_min);
        Some(R::new(l, t, l + FLY_W, t + h))
    }

    fn fav_fly_list(fly: R) -> R {
        fly.inset(6.0, 6.0)
    }

    fn fav_fly_row(&self, fly: R, j: usize) -> R {
        let list = Self::fav_fly_list(fly);
        let y = list.t + j as f32 * FAV_ROW_H - self.fav_fly.map_or(0.0, |f| f.scroll);
        R::new(list.l, y, list.r, y + FAV_ROW_H)
    }

    /// 이름 바꾸기 입력 상자 자리 (그 줄의 이름 칸). 줄이 보이지 않으면 None
    pub(super) fn fav_rename_rect(&self, row: Row) -> Option<R> {
        let (r, clip) = match row {
            Row::Item { group: Some(g), index } => {
                let fly = self.fav_fly_rect().filter(|_| self.fav_fly.is_some_and(|f| f.group == g))?;
                (self.fav_fly_row(fly, index), Self::fav_fly_list(fly))
            }
            _ => (self.fav_row(self.favs.row_index(row)?), self.fav_list()),
        };
        if r.t < clip.t - 1.0 || r.b > clip.b + 1.0 {
            return None;
        }
        Some(R::new(r.l + 34.0, r.t + 3.0, r.r - 8.0, r.b - 3.0))
    }

    pub(super) fn fav_hit(&self, x: f32, y: f32) -> Option<Hit> {
        if let Some(fly) = self.fav_fly_rect().filter(|f| f.contains(x, y)) {
            let n = self.fav_fly.and_then(|f| self.favs.groups.get(f.group)).map_or(0, |g| g.items.len());
            let list = Self::fav_fly_list(fly);
            let hit = (0..n).find(|&j| list.contains(x, y) && self.fav_fly_row(fly, j).contains(x, y));
            return Some(hit.map_or(Hit::FavFlyPanel, Hit::FavFlyItem));
        }
        let r = self.fav_rect();
        if !r.contains(x, y) {
            return None;
        }
        let [group_b, sort, add] = self.fav_head_buttons();
        if add.contains(x, y) {
            return Some(Hit::FavAdd);
        }
        if sort.contains(x, y) {
            return Some(Hit::FavSort);
        }
        if group_b.contains(x, y) {
            return Some(Hit::FavNewGroup);
        }
        let list = self.fav_list();
        if list.contains(x, y) {
            let i = ((y - list.t + self.fav_scroll) / FAV_ROW_H).floor();
            if i >= 0.0 && (i as usize) < self.favs.rows().len() {
                return Some(Hit::FavRow(i as usize));
            }
        }
        Some(Hit::FavPanel)
    }

    /// 누른 곳의 즐겨찾기 줄 (왼쪽 목록의 항목·그룹, 오른쪽 메뉴의 항목)
    pub(super) fn fav_row_at(&self, hit: Hit) -> Option<Row> {
        match hit {
            Hit::FavRow(i) => self.favs.rows().get(i).copied(),
            Hit::FavFlyItem(j) => self.fav_fly.map(|f| Row::Item { group: Some(f.group), index: j }),
            _ => None,
        }
    }

    // ---------- 열고 닫기, 고르기 ----------

    pub(super) fn toggle_favorites(&mut self) {
        if self.fav_open {
            self.close_favorites();
        } else {
            self.open_favorites(None);
        }
    }

    pub(super) fn open_favorites(&mut self, select: Option<Row>) {
        self.close_palette();
        if !self.fav_open && self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        self.fav_open = true;
        self.hover = Hit::None;
        match select {
            Some(row) => self.fav_select(row),
            None => {
                self.fav_sel = self.fav_sel.min(self.favs.rows().len().saturating_sub(1));
                self.fav_ensure_visible();
            }
        }
        // 필터 입력 상자(자식 창)가 팝업 위에 그려지지 않도록 화면 밖으로 옮긴다. 포커스는 유지
        self.layout_edits();
        self.invalidate();
    }

    pub(super) fn close_favorites(&mut self) {
        if !self.fav_open {
            return;
        }
        if let EditMode::FavRename(_) = self.edit_mode {
            self.commit_rename();
        }
        self.fav_open = false;
        self.fav_fly_close();
        self.fav_hide_tip();
        if matches!(self.drag, Drag::FavPress { .. } | Drag::FavMove { .. }) {
            self.drag = Drag::None;
        }
        self.layout_edits();
        self.invalidate();
    }

    /// 그 줄을 고른다. 그룹이면 오른쪽에 그 그룹을 보이고, 그룹 안 항목이면 오른쪽 메뉴에서 고른다
    pub(super) fn fav_select(&mut self, row: Row) {
        match row {
            Row::Item { group: Some(g), index } => {
                if let Some(i) = self.favs.row_index(Row::Group(g)) {
                    self.fav_sel = i;
                }
                self.fav_ensure_visible();
                self.fav_fly_show(g, false);
                if let Some(f) = &mut self.fav_fly {
                    f.sel = Some(index);
                }
                self.fav_fly_ensure_visible();
            }
            other => {
                if let Some(i) = self.favs.row_index(other) {
                    self.fav_sel = i;
                }
                self.fav_ensure_visible();
                match other {
                    Row::Group(g) => {
                        self.fav_fly_show(g, false);
                        if let Some(f) = &mut self.fav_fly {
                            f.sel = None;
                        }
                    }
                    _ => self.fav_fly_close(),
                }
            }
        }
        self.invalidate();
    }

    /// 오른쪽 메뉴를 그 그룹으로. 이미 열려 있으면 내용만 바꾸고, 처음 열 때만 미끄러져 나온다
    fn fav_fly_show(&mut self, g: usize, animate: bool) {
        if g >= self.favs.groups.len() {
            return self.fav_fly_close();
        }
        match &mut self.fav_fly {
            Some(f) if f.group == g => {}
            Some(f) => *f = Fly { group: g, anim: f.anim, sel: None, scroll: 0.0 },
            None => {
                self.fav_fly = Some(Fly { group: g, anim: if animate { 0.0 } else { 1.0 }, sel: None, scroll: 0.0 });
                if animate {
                    unsafe { SetTimer(Some(self.hwnd), TIMER_FLY, 15, None) };
                }
            }
        }
        self.invalidate();
    }

    fn fav_fly_close(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_FLY);
            let _ = KillTimer(Some(self.hwnd), TIMER_FLY_CLOSE);
        }
        if self.fav_fly.take().is_some() {
            self.invalidate();
        }
    }

    fn fav_clamp_scroll(&mut self) {
        let list = self.fav_list();
        let max = (self.favs.rows().len() as f32 * FAV_ROW_H - list.h()).max(0.0);
        self.fav_scroll = self.fav_scroll.clamp(0.0, max);
    }

    fn fav_ensure_visible(&mut self) {
        let list = self.fav_list();
        let y = self.fav_sel as f32 * FAV_ROW_H;
        if y < self.fav_scroll {
            self.fav_scroll = y;
        } else if y + FAV_ROW_H > self.fav_scroll + list.h() {
            self.fav_scroll = y + FAV_ROW_H - list.h();
        }
        self.fav_clamp_scroll();
    }

    fn fav_fly_clamp(&mut self) {
        let Some(list) = self.fav_fly_rect().map(Self::fav_fly_list) else { return };
        let n = self.fav_fly.and_then(|f| self.favs.groups.get(f.group)).map_or(0, |g| g.items.len());
        if let Some(f) = &mut self.fav_fly {
            let max = (n as f32 * FAV_ROW_H - list.h()).max(0.0);
            f.scroll = f.scroll.clamp(0.0, max);
        }
    }

    fn fav_fly_ensure_visible(&mut self) {
        let Some(list) = self.fav_fly_rect().map(Self::fav_fly_list) else { return };
        if let Some(f) = &mut self.fav_fly {
            if let Some(j) = f.sel {
                let y = j as f32 * FAV_ROW_H;
                if y < f.scroll {
                    f.scroll = y;
                } else if y + FAV_ROW_H > f.scroll + list.h() {
                    f.scroll = y + FAV_ROW_H - list.h();
                }
            }
        }
        self.fav_fly_clamp();
    }

    // ---------- 마우스가 옮겨 갈 때 ----------

    /// 마우스가 다른 곳으로 옮겨 갔다. 왼쪽 줄 위면 그 줄을 고르고(그룹이면 오른쪽에 보이고),
    /// 오른쪽 메뉴 쪽으로 대각선으로 가는 중이면 지나가는 줄 때문에 바꾸지 않고 잠시 뒤 다시 본다
    pub(super) fn fav_fly_hover(&mut self, hit: Hit, x: f32, y: f32) {
        // 이름을 바꾸거나 끌어서 옮기는 동안은 그대로 둔다
        if matches!(self.edit_mode, EditMode::FavRename(_)) || !matches!(self.drag, Drag::None) {
            return;
        }
        self.fav_tip_hover(hit);
        match hit {
            Hit::FavFlyItem(j) => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_FLY_CLOSE);
                }
                if let Some(f) = &mut self.fav_fly {
                    f.sel = Some(j);
                }
            }
            Hit::FavFlyPanel => unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_FLY_CLOSE);
            },
            Hit::FavRow(i) => {
                if self.fav_aiming(x, y) {
                    unsafe { SetTimer(Some(self.hwnd), TIMER_FLY_CLOSE, 300, None) };
                } else {
                    self.fav_follow_row(i);
                }
            }
            // 머리·빈 곳·팝업 밖으로 나가도 열린 메뉴는 그대로 (메뉴처럼)
            _ => {}
        }
        self.invalidate();
    }

    /// 왼쪽 줄을 따라간다: 그 줄을 고르고, 그룹이면 오른쪽에 보이고 아니면 오른쪽 메뉴를 닫는다
    fn fav_follow_row(&mut self, i: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_FLY_CLOSE);
        }
        self.fav_sel = i;
        match self.favs.rows().get(i) {
            Some(Row::Group(g)) => {
                self.fav_fly_show(*g, true);
                if let Some(f) = &mut self.fav_fly {
                    f.sel = None;
                }
            }
            _ => self.fav_fly_close(),
        }
    }

    /// 오른쪽 메뉴를 향해 가는 중인가: 바로 전 위치와 메뉴의 가까운 쪽 두 모서리로 된 삼각형 안으로 움직였으면
    fn fav_aiming(&self, x: f32, y: f32) -> bool {
        let Some(fly) = self.fav_fly_rect() else { return false };
        let from = self.fav_pointer;
        let edge = if fly.l >= from.0 { fly.l } else { fly.r };
        // 메뉴 쪽으로 조금이라도 다가가야 한다 (제자리·반대 방향은 아님)
        let toward = if fly.l >= from.0 { x > from.0 } else { x < from.0 };
        toward && in_triangle((x, y), from, (edge, fly.t - 4.0), (edge, fly.b + 4.0))
    }

    pub(super) fn fav_fly_timer(&mut self, id: usize) {
        unsafe {
            if id != TIMER_FLY {
                let _ = KillTimer(Some(self.hwnd), id);
            }
        }
        match id {
            // 대각선으로 가다 멈췄으면 지금 마우스 아래 줄을 따른다
            TIMER_FLY_CLOSE => {
                if let Hit::FavRow(i) = self.hover {
                    self.fav_follow_row(i);
                }
            }
            TIMER_FAV_TIP => {
                if let Some((_, shown)) = &mut self.fav_tip {
                    *shown = true;
                }
            }
            _ => match &mut self.fav_fly {
                Some(f) if f.anim < 1.0 => f.anim = (f.anim + 0.2).min(1.0),
                _ => unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_FLY);
                },
            },
        }
        self.invalidate();
    }

    /// 항목 줄에 잠시 머물면 경로를 말풍선으로 (평소에는 이름을 읽을 자리를 넓게 둔다)
    fn fav_tip_hover(&mut self, hit: Hit) {
        let row = self.fav_row_at(hit).filter(|r| matches!(r, Row::Item { .. }));
        if self.fav_tip.map(|(r, _)| r) == row {
            return;
        }
        self.fav_hide_tip();
        if let Some(r) = row {
            self.fav_tip = Some((r, false));
            unsafe { SetTimer(Some(self.hwnd), TIMER_FAV_TIP, 600, None) };
        }
    }

    fn fav_hide_tip(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_FAV_TIP);
        }
        if self.fav_tip.take().is_some_and(|(_, shown)| shown) {
            self.invalidate();
        }
    }

    // ---------- 열기·추가·편집 ----------

    /// 줄을 고르고 보이게 한 뒤 저장한다 (항목을 옮기거나 그룹을 바꾼 다음)
    fn fav_changed(&mut self, select: Option<Row>) {
        // 그룹 순서·개수가 바뀌었을 수 있으니 오른쪽 메뉴의 그룹이 남아 있는지 본다
        if self.fav_fly.is_some_and(|f| f.group >= self.favs.groups.len()) {
            self.fav_fly_close();
        }
        self.fav_sel = self.fav_sel.min(self.favs.rows().len().saturating_sub(1));
        if let Some(f) = &mut self.fav_fly {
            let n = self.favs.groups[f.group].items.len();
            f.sel = f.sel.filter(|_| n > 0).map(|j| j.min(n - 1));
        }
        match select {
            Some(row) => self.fav_select(row),
            None => self.fav_ensure_visible(),
        }
        self.fav_fly_clamp();
        self.save_favorites();
        self.invalidate();
    }

    /// 왼쪽 줄을 연다: 항목은 열고, 그룹은 오른쪽 메뉴로 들어간다
    pub(super) fn fav_open_item(&mut self, i: usize, new_tab: bool) {
        if let Some(row) = self.favs.rows().get(i).copied() {
            self.fav_open_row(row, new_tab);
        }
    }

    /// 항목은 열고(폴더는 이동, 파일은 실행), 그룹은 오른쪽 메뉴의 첫 줄로 들어간다.
    /// new_tab: 폴더는 새 탭, 파일은 그 파일이 있는 폴더를 새 탭에서 열고 선택
    pub(super) fn fav_open_row(&mut self, row: Row, new_tab: bool) {
        match row {
            Row::Group(g) => self.fav_enter_group(g),
            Row::Item { group, index } => self.fav_open_entry(group, index, new_tab),
        }
    }

    fn fav_enter_group(&mut self, g: usize) {
        if let Some(i) = self.favs.row_index(Row::Group(g)) {
            self.fav_sel = i;
        }
        self.fav_fly_show(g, true);
        let n = self.favs.groups.get(g).map_or(0, |g| g.items.len());
        if let Some(f) = &mut self.fav_fly {
            f.sel = (n > 0).then_some(f.sel.unwrap_or(0));
        }
        self.fav_fly_ensure_visible();
    }

    fn fav_open_entry(&mut self, group: Option<usize>, index: usize, new_tab: bool) {
        let Some(f) = self.favs.item(group, index).cloned() else { return };
        self.close_favorites();
        let path = util::normalize(&f.path);
        let p = self.active;
        if shell::is_dir(&path) {
            if new_tab {
                self.new_tab(p, Some(Location::Dir(path)), true);
            } else {
                self.navigate(p, Location::Dir(path), None);
            }
        } else if shell::exists(&path) {
            let dir = util::parent(&path);
            if !new_tab {
                self.pending.push(Deferred::Open { pane: p, paths: vec![path], dir });
            } else if let Some(dir) = dir {
                let name = util::file_name(&path).to_string();
                self.new_tab(p, Some(Location::Dir(dir)), true);
                self.panes[p].tab_mut().restore = Some(Restore::select(&name));
            }
        } else {
            self.flash(p, format!("즐겨찾기 항목을 찾을 수 없습니다: {}", f.path));
        }
    }

    /// Ctrl+D·추가 버튼: 폴더·파일 하나를 선택했으면 그것, 아니면 현재 폴더
    pub(super) fn add_favorite(&mut self) {
        let tab = self.panes[self.active].tab();
        let picked = match tab.cursor_entry() {
            Some(e) if e.selected && tab.selection_stats().0 == 1 => {
                // 드라이브·사용자 폴더는 "로컬 디스크 (C:)" 같은 표시 이름을 그대로 쓴다
                tab.path_of(e).map(|p| (p, matches!(e.kind, Kind::Drive | Kind::Known).then(|| e.name_string())))
            }
            _ => None,
        };
        match picked.or_else(|| tab.loc.dir().map(|d| (d.to_string(), None))) {
            Some((path, name)) => self.add_favorite_path(&path, name),
            None => {
                let p = self.active;
                self.flash(p, "즐겨찾기에 추가할 폴더나 파일이 없습니다");
            }
        }
    }

    /// 팝업이 열려 있고 그룹(또는 그 오른쪽 메뉴)을 고르고 있으면 그 그룹에, 아니면 그룹 없음 끝에 넣는다
    pub fn add_favorite_path(&mut self, path: &str, name: Option<String>) {
        let path = util::normalize(path);
        if let Some((group, index)) = self.favs.find(&path) {
            self.open_favorites(Some(Row::Item { group, index }));
            let p = self.active;
            self.flash(p, "이미 즐겨찾기에 있습니다");
            return;
        }
        let target = if !self.fav_open {
            None
        } else if let Some(f) = self.fav_fly.filter(|f| f.sel.is_some()) {
            Some(f.group)
        } else {
            match self.favs.rows().get(self.fav_sel) {
                Some(Row::Group(g)) => Some(*g),
                _ => None,
            }
        };
        let name = name.unwrap_or_else(|| favorites::default_name(&path));
        let is_dir = shell::is_dir(&path);
        let (group, index) = self.favs.push(target, Favorite { name, path, is_dir });
        self.save_favorites();
        let row = Row::Item { group, index };
        self.open_favorites(Some(row));
        // 추가하자마자 원하는 표시 이름(예: "01.일간 업무")을 붙일 수 있게 한다. Enter면 그대로
        self.begin_fav_rename(row);
    }

    /// 경로 상자의 별: 이 폴더를 즐겨찾기에 바로 넣거나 뺀다 (팝업을 열지 않는다)
    pub(super) fn toggle_star(&mut self, p: usize) {
        let Some(dir) = self.panes[p].tab().loc.dir().map(util::normalize) else { return };
        let before = self.favs.clone();
        let msg = match self.favs.find(&dir) {
            Some((group, index)) => {
                self.favs.remove_item(group, index);
                "즐겨찾기에서 뺐습니다"
            }
            None => {
                self.favs.push(None, Favorite { name: favorites::default_name(&dir), path: dir, is_dir: true });
                "즐겨찾기에 넣었습니다 · 이름과 그룹은 즐겨찾기 창에서 정리할 수 있어요"
            }
        };
        // 저장하지 못했으면 별 표시도 원래대로 두고 오류만 보인다 (성공 문구로 덮지 않게)
        match favorites::save(&self.favs) {
            Ok(()) => self.flash(p, msg),
            Err(e) => {
                self.favs = before;
                self.flash(p, format!("즐겨찾기를 저장하지 못해 바꾸지 않았습니다: {e}"));
            }
        }
    }

    fn save_favorites(&mut self) {
        if let Err(e) = favorites::save(&self.favs) {
            let p = self.active;
            self.flash(p, format!("즐겨찾기를 저장하지 못했습니다: {e}"));
        }
    }

    pub(super) fn add_fav_group(&mut self) -> usize {
        let g = self.favs.add_group("새 그룹".to_string());
        self.save_favorites();
        self.open_favorites(Some(Row::Group(g)));
        self.begin_fav_rename(Row::Group(g));
        g
    }

    pub(super) fn begin_fav_rename(&mut self, row: Row) {
        let Some(name) = self.favs.row_name(row).map(str::to_string) else { return };
        if self.fav_open {
            self.fav_select(row);
        } else {
            self.open_favorites(Some(row));
        }
        self.fav_hide_tip();
        self.edit_mode = EditMode::FavRename(row);
        unsafe {
            let _ = SetWindowTextW(self.rename_edit, &HSTRING::from(name));
            SendMessageW(self.rename_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    pub(super) fn commit_fav_rename(&mut self, row: Row) {
        let text = favorites::clean_name(&edit_text(self.rename_edit));
        self.close_edit();
        if !text.is_empty() && self.favs.set_row_name(row, text) {
            self.save_favorites();
        }
    }

    /// Del: 항목은 지우고, 그룹은 풀어서 안의 항목을 남긴다
    pub(super) fn remove_favorite(&mut self, row: Row) {
        if let EditMode::FavRename(_) = self.edit_mode {
            self.cancel_edit();
        }
        match row {
            Row::Item { group, index } => {
                self.favs.remove_item(group, index);
            }
            Row::Group(g) => {
                self.fav_fly_close();
                self.favs.ungroup(g);
            }
        }
        self.fav_changed(None);
    }

    pub(super) fn fav_ungroup(&mut self, g: usize) {
        self.fav_fly_close();
        self.favs.ungroup(g);
        self.fav_changed(None);
    }

    pub(super) fn fav_delete_group(&mut self, g: usize) {
        self.fav_fly_close();
        self.favs.delete_group(g);
        self.fav_changed(None);
    }

    pub(super) fn fav_move_to_group(&mut self, row: Row, group: Option<usize>) {
        let Row::Item { group: from_group, index } = row else { return };
        let to = self.favs.section(group).len();
        if let Some((group, index)) = self.favs.move_item((from_group, index), group, to) {
            self.fav_changed(Some(Row::Item { group, index }));
        }
    }

    pub(super) fn fav_move_to_new_group(&mut self, row: Row) {
        let Row::Item { group, index } = row else { return };
        let g = self.favs.add_group("새 그룹".to_string());
        self.favs.move_item((group, index), Some(g), 0);
        self.fav_changed(Some(Row::Group(g)));
        self.begin_fav_rename(Row::Group(g));
    }

    /// Ctrl+↑↓ 또는 메뉴의 위로·아래로
    pub(super) fn fav_nudge(&mut self, row: Row, up: bool) {
        if matches!(row, Row::Group(_)) {
            self.fav_fly_close();
        }
        if let Some(moved) = self.favs.nudge(row, up) {
            self.fav_changed(Some(moved));
        }
    }

    pub(super) fn sort_favorites(&mut self, by: SortBy) {
        self.fav_fly_close();
        self.favs.sort(by);
        self.fav_changed(None);
    }

    // ---------- 키보드 ----------

    /// 팝업이 열려 있을 때의 키. 처리했으면 true.
    /// 왼쪽: ↑↓로 고르고 →·Enter로 그룹에 들어가거나 항목을 연다. 오른쪽: ↑↓, Enter, ←·Esc로 왼쪽으로
    pub(super) fn fav_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        self.fav_hide_tip();
        if let Some(f) = self.fav_fly.filter(|f| f.sel.is_some()) {
            return self.fav_fly_key(vk, f);
        }
        let ctrl = key_down(VK_CONTROL);
        let rows = self.favs.rows();
        let n = rows.len();
        let s = self.fav_sel.min(n.saturating_sub(1));
        let page = ((self.fav_list().h() / FAV_ROW_H).floor() as usize).max(1);
        let go = |a: &mut App, i: usize| {
            if let Some(row) = rows.get(i) {
                a.fav_select(*row);
            }
        };
        match vk {
            VK_ESCAPE => self.close_favorites(),
            VK_UP if ctrl => self.fav_nudge_at(s, true),
            VK_DOWN if ctrl => self.fav_nudge_at(s, false),
            VK_UP => go(self, s.saturating_sub(1)),
            VK_DOWN => go(self, (s + 1).min(n.saturating_sub(1))),
            VK_PRIOR => go(self, s.saturating_sub(page)),
            VK_NEXT => go(self, (s + page).min(n.saturating_sub(1))),
            VK_HOME => go(self, 0),
            VK_END => go(self, n.saturating_sub(1)),
            VK_RIGHT => {
                if let Some(Row::Group(g)) = rows.get(s) {
                    self.fav_enter_group(*g);
                }
            }
            VK_RETURN => {
                if let Some(row) = rows.get(s) {
                    self.fav_open_row(*row, ctrl);
                }
                return true;
            }
            VK_F2 => {
                if let Some(row) = rows.get(s) {
                    self.begin_fav_rename(*row);
                }
                return true;
            }
            VK_DELETE => {
                if let Some(row) = rows.get(s) {
                    self.remove_favorite(*row);
                }
            }
            _ => return self.fav_shared_key(vk),
        }
        self.invalidate();
        true
    }

    fn fav_nudge_at(&mut self, i: usize, up: bool) {
        if let Some(row) = self.favs.rows().get(i).copied() {
            self.fav_nudge(row, up);
        }
    }

    /// 오른쪽 메뉴에 키보드가 있을 때
    fn fav_fly_key(&mut self, vk: VIRTUAL_KEY, f: Fly) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let g = f.group;
        let n = self.favs.groups.get(g).map_or(0, |g| g.items.len());
        let j = f.sel.unwrap_or(0).min(n.saturating_sub(1));
        let page = self.fav_fly_rect().map_or(1, |r| ((Self::fav_fly_list(r).h() / FAV_ROW_H).floor() as usize).max(1));
        let row = Row::Item { group: Some(g), index: j };
        let go = |a: &mut App, to: usize| {
            if let Some(f) = &mut a.fav_fly {
                f.sel = Some(to.min(n.saturating_sub(1)));
            }
            a.fav_fly_ensure_visible();
        };
        match vk {
            // ←·Esc: 왼쪽 목록으로 (오른쪽 메뉴는 보인 채로)
            VK_LEFT | VK_ESCAPE => {
                if let Some(f) = &mut self.fav_fly {
                    f.sel = None;
                }
            }
            VK_UP if ctrl => self.fav_nudge(row, true),
            VK_DOWN if ctrl => self.fav_nudge(row, false),
            VK_UP => go(self, j.saturating_sub(1)),
            VK_DOWN => go(self, j + 1),
            VK_PRIOR => go(self, j.saturating_sub(page)),
            VK_NEXT => go(self, j + page),
            VK_HOME => go(self, 0),
            VK_END => go(self, n.saturating_sub(1)),
            VK_RIGHT => {}
            VK_RETURN => {
                if n > 0 {
                    self.fav_open_entry(Some(g), j, ctrl);
                }
                return true;
            }
            VK_F2 => {
                if n > 0 {
                    self.begin_fav_rename(row);
                }
                return true;
            }
            VK_DELETE => {
                if n > 0 {
                    self.remove_favorite(row);
                }
            }
            _ => return self.fav_shared_key(vk),
        }
        self.invalidate();
        true
    }

    /// 양쪽 메뉴에서 같은 키: Tab은 닫고 넘기고, 바꿀 수 있는 단축키(즐겨찾기·추가·설정·도움말·검색)
    fn fav_shared_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        match vk {
            VK_TAB => {
                self.close_favorites();
                return self.on_key(vk);
            }
            _ => match self.keys.find(crate::keys::Combo::now(vk.0)) {
                Some(crate::keys::Action::Favorites) => self.close_favorites(),
                Some(crate::keys::Action::AddFavorite) => self.add_favorite(),
                Some(a @ (crate::keys::Action::Settings | crate::keys::Action::Help | crate::keys::Action::Search)) => {
                    self.close_favorites();
                    self.run_action(a);
                }
                _ => return false,
            },
        }
        self.invalidate();
        true
    }

    /// 팝업이 열려 있을 때 글자를 누르면, 키보드가 있는 쪽 메뉴에서 그 글자로 시작하는 다음 줄로 이동
    pub fn fav_type_char(&mut self, ch: u16) -> bool {
        if !self.fav_open || self.edit_mode != EditMode::None {
            return false;
        }
        let Some(c) = char::from_u32(ch as u32).filter(|c| !c.is_control()) else { return true };
        let needle = c.to_lowercase().to_string();
        let starts = |name: &str| name.to_lowercase().starts_with(&needle);
        if let Some(f) = self.fav_fly.filter(|f| f.sel.is_some()) {
            let items = self.favs.section(Some(f.group));
            let n = items.len();
            let from = f.sel.unwrap_or(0);
            if let Some(j) = (1..=n).map(|k| (from + k) % n).find(|&j| starts(&items[j].name)) {
                if let Some(f) = &mut self.fav_fly {
                    f.sel = Some(j);
                }
                self.fav_fly_ensure_visible();
            }
        } else {
            let rows = self.favs.rows();
            let n = rows.len();
            if let Some(i) = (1..=n).map(|k| (self.fav_sel + k) % n).find(|&i| self.favs.row_name(rows[i]).is_some_and(starts)) {
                self.fav_select(rows[i]);
            }
        }
        self.invalidate();
        true
    }

    // ---------- 누르기·끌기·휠 ----------

    /// 팝업이 열려 있을 때 왼쪽 버튼. 밖을 누르면 닫기만 하고 클릭은 소비한다
    pub(super) fn fav_mouse_down(&mut self, hit: Hit, y: f32) {
        self.fav_hide_tip();
        match hit {
            Hit::FavAdd => self.add_favorite(),
            Hit::FavNewGroup => {
                self.add_fav_group();
            }
            Hit::FavSort => {
                let r = self.fav_head_buttons()[1];
                let pt = self.to_screen(r.l, r.b + 2.0);
                self.pending.push(Deferred::FavSortMenu { pt });
            }
            Hit::FavRow(i) => {
                self.fav_sel = i;
                self.drag = Drag::FavPress { index: i, y };
                unsafe { SetCapture(self.hwnd) };
            }
            Hit::FavFlyItem(j) => {
                if let Some(f) = self.fav_fly {
                    self.fav_open_entry(Some(f.group), j, key_down(VK_CONTROL));
                }
            }
            Hit::FavPanel | Hit::FavFlyPanel => {}
            _ => self.close_favorites(),
        }
    }

    pub(super) fn fav_drag_move(&mut self, y: f32) {
        self.drag = match self.drag {
            Drag::FavPress { index, y: y0 } if (y - y0).abs() > 4.0 => Drag::FavMove { from: index, y },
            Drag::FavMove { from, .. } => Drag::FavMove { from, y },
            _ => return,
        };
        // 끌기 시작하면 오른쪽 메뉴는 닫는다 (그룹 줄 위에 놓으면 그 그룹으로 들어간다)
        self.fav_fly_close();
        // 목록 끝에 가까이 끌면 자동 스크롤
        let list = self.fav_list();
        if y < list.t + 10.0 {
            self.fav_scroll -= 8.0;
        } else if y > list.b - 10.0 {
            self.fav_scroll += 8.0;
        }
        self.fav_clamp_scroll();
        self.invalidate();
    }

    /// 끄는 위치(줄 단위)
    fn fav_drag_pos(&self, y: f32) -> f32 {
        (y - self.fav_list().t + self.fav_scroll) / FAV_ROW_H
    }

    /// 처리했으면 true
    pub(super) fn fav_mouse_up(&mut self, ctrl: bool) -> bool {
        let handled = match self.drag {
            Drag::FavPress { index, .. } => {
                self.drag = Drag::None;
                self.fav_open_item(index, ctrl);
                true
            }
            Drag::FavMove { from, y } => {
                self.drag = Drag::None;
                let pos = self.fav_drag_pos(y);
                match self.favs.rows().get(from).copied() {
                    // 항목은 그룹 줄 위에 놓으면 그 그룹으로, 그 밖에는 놓은 자리로. 그룹은 그룹끼리 순서를 바꾼다
                    Some(Row::Item { group, index }) => {
                        let (to_group, to_index) = self.favs.item_drop_target(pos);
                        if let Some((group, index)) = self.favs.move_item((group, index), to_group, to_index) {
                            self.fav_changed(Some(Row::Item { group, index }));
                        }
                    }
                    Some(Row::Group(g)) => {
                        let to = self.favs.group_drop_target(pos, g);
                        if let Some(to) = self.favs.move_group(g, to) {
                            self.fav_changed(Some(Row::Group(to)));
                        }
                    }
                    None => {}
                }
                true
            }
            _ => false,
        };
        if handled {
            unsafe {
                let _ = ReleaseCapture();
            }
            self.invalidate();
        }
        handled
    }

    /// 휠: 마우스 아래 메뉴만 따로 스크롤한다. 메뉴 밖이면 아무것도 움직이지 않는다
    pub(super) fn fav_wheel_at(&mut self, x: f32, y: f32, dy: f32) {
        self.fav_hide_tip();
        if self.fav_fly_rect().is_some_and(|r| r.contains(x, y)) {
            if let Some(f) = &mut self.fav_fly {
                f.scroll += dy;
            }
            self.fav_fly_clamp();
        } else if self.fav_rect().contains(x, y) {
            self.fav_scroll += dy;
            self.fav_clamp_scroll();
        } else {
            return;
        }
        if self.edit_mode != EditMode::None {
            self.layout_edits();
        }
        self.invalidate();
    }

    // ---------- 그리기 ----------

    pub(super) fn paint_favorites(&mut self) {
        let t = self.theme;
        let r = self.fav_rect();
        let list = self.fav_list();
        let [group_b, sort, add] = self.fav_head_buttons();
        let hover = self.hover;
        let (sel, scroll, edit_mode) = (self.fav_sel, self.fav_scroll, self.edit_mode);
        // 끄는 중: (끄는 줄, 끄는 위치를 줄 단위로)
        let moving = match self.drag {
            Drag::FavMove { from, y } => Some((from, self.fav_drag_pos(y))),
            _ => None,
        };
        let rows = self.favs.rows();
        let fly = self.fav_fly_rect().zip(self.fav_fly);
        let fly_rows: Vec<R> = match fly {
            Some((fr, f)) => (0..self.favs.groups.get(f.group).map_or(0, |g| g.items.len())).map(|j| self.fav_fly_row(fr, j)).collect(),
            None => Vec::new(),
        };
        let keyboard_in_fly = self.fav_fly.is_some_and(|f| f.sel.is_some());
        // 경로 말풍선: (말풍선을 붙일 줄, 경로)
        let tip = self.fav_tip.filter(|(_, shown)| *shown).and_then(|(row, _)| {
            let Row::Item { group, index } = row else { return None };
            let path = self.favs.item(group, index)?.path.clone();
            let at = match group {
                Some(_) => fly_rows.get(index).copied()?,
                None => self.fav_row(self.favs.row_index(row)?),
            };
            Some((at, path))
        });
        let tip_w = tip.as_ref().map(|(_, p)| self.gfx.measure(p, Font::Small) + 20.0);
        let help = self.keys.get(crate::keys::Action::Help).map(crate::keys::Combo::label);
        let add_key = self.keys.get(crate::keys::Action::AddFavorite).map(crate::keys::Combo::label);
        let (w, h) = (self.w, self.h);
        let App { gfx, favs, types, .. } = self;
        let folder_icon = types.entry("\\dir".to_string()).or_insert_with(|| shell::type_info("", true)).1;
        let mut icon_of = |f: &Favorite| {
            if f.is_dir {
                folder_icon
            } else {
                let ext = util::file_name(&f.path).rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                types.entry(ext.clone()).or_insert_with(|| shell::type_info(&ext, false)).1
            }
        };
        let shadow = |a: f32| Color { r: 0.0, g: 0.0, b: 0.0, a: if t.dark { 0.45 } else { 0.12 } * a };

        // 그림자 + 본체
        gfx.fill_round(R::new(r.l - 2.0, r.t + 1.0, r.r + 2.0, r.b + 4.0), 12.0, shadow(1.0));
        gfx.fill_round(r, 10.0, t.pane);
        gfx.stroke_round(r.inset(0.5, 0.5), 10.0, t.border, 1.0);

        // 머리: 제목·개수, 오른쪽에 아이콘 버튼 (이름은 마우스를 올리면)
        gfx.glyph(G_STAR, R::new(r.l + 12.0, r.t, r.l + 34.0, r.t + FAV_HEAD_H), false, t.accent);
        gfx.text("즐겨찾기", R::new(r.l + 38.0, r.t, group_b.l - 4.0, r.t + FAV_HEAD_H), Font::Strong, t.text);
        let title_w = gfx.measure("즐겨찾기", Font::Strong);
        gfx.text(&format!("{}개", favs.count()), R::new(r.l + 44.0 + title_w, r.t, group_b.l - 4.0, r.t + FAV_HEAD_H), Font::Small, t.faint);
        for (b, glyph, label, hh) in [(group_b, G_NEW_GROUP, "새 그룹", Hit::FavNewGroup), (sort, G_SORT, "정렬", Hit::FavSort), (add, G_ADD, "지금 폴더 추가", Hit::FavAdd)] {
            if hover == hh {
                gfx.fill_round(b, 6.0, t.hover);
            }
            gfx.glyph(glyph, b, true, t.text);
            if hover == hh {
                let lw = gfx.measure(label, Font::Small) + 16.0;
                let lr = R::new((b.r - lw).max(r.l + 4.0), b.b + 4.0, (b.r - lw).max(r.l + 4.0) + lw, b.b + 26.0);
                gfx.fill_round(lr, 5.0, t.text);
                gfx.text(label, R::new(lr.l + 8.0, lr.t, lr.r - 4.0, lr.b), Font::Small, t.pane);
            }
        }
        gfx.fill(R::new(r.l + 10.0, r.t + FAV_HEAD_H - 1.0, r.r - 10.0, r.t + FAV_HEAD_H), t.border);

        // 왼쪽 목록: 이름만 한 줄씩
        gfx.clip(list);
        if rows.is_empty() {
            let how = add_key.as_deref().map(|k| format!("경로 상자의 ☆나 {k}로")).unwrap_or_else(|| "경로 상자의 ☆로".to_string());
            gfx.text(&format!("즐겨찾기가 비어 있어요\n{how} 폴더·파일을 넣어 보세요"), list.inset(12.0, 8.0), Font::Hint, t.faint);
        }
        let first = (scroll / FAV_ROW_H).floor().max(0.0) as usize;
        let count = (list.h() / FAV_ROW_H).ceil() as usize + 1;
        let drop_into = moving.and_then(|(from, pos)| match rows.get(from) {
            Some(Row::Item { .. }) => match rows.get(pos.floor().max(0.0) as usize) {
                Some(Row::Group(g)) if pos >= 0.0 => Some(*g),
                _ => None,
            },
            _ => None,
        });
        for i in first..(first + count).min(rows.len()) {
            let y = list.t + i as f32 * FAV_ROW_H - scroll;
            let row = R::new(list.l, y, list.r, y + FAV_ROW_H);
            let body = row.inset(2.0, 1.0);
            let dragged = moving.is_some_and(|(from, _)| from == i);
            let open_group = matches!((rows[i], fly), (Row::Group(g), Some((_, f))) if g == f.group);
            // 고른 줄(마우스를 올린 줄도)은 진하게, 키보드가 오른쪽에 있으면 그 그룹은 옅게
            if i == sel && !(keyboard_in_fly && open_group) {
                gfx.fill_round(body, 6.0, t.sel);
            } else if open_group || hover == Hit::FavRow(i) {
                gfx.fill_round(body, 6.0, t.hover);
            }
            if matches!(rows[i], Row::Group(g) if drop_into == Some(g)) {
                gfx.stroke_round(body.inset(0.75, 0.75), 6.0, t.accent, 1.5);
            }
            let fg = if dragged { t.faint } else { t.text };
            match rows[i] {
                Row::Group(g) => {
                    let group = &favs.groups[g];
                    gfx.glyph(G_GROUP, R::new(row.l + 10.0, y, row.l + 30.0, y + FAV_ROW_H), true, t.dim);
                    let n = format!("{}", group.items.len());
                    let nw = gfx.measure(&n, Font::Small);
                    gfx.text(&group.name, R::new(row.l + 36.0, y, row.r - 34.0 - nw, y + FAV_ROW_H), Font::Normal, fg);
                    gfx.text(&n, R::new(row.l + 36.0, y, row.r - 28.0, y + FAV_ROW_H), Font::Right, t.faint);
                    gfx.glyph(G_CHEVRON, R::new(row.r - 26.0, y, row.r - 8.0, y + FAV_ROW_H), true, t.dim);
                }
                Row::Item { group, index } => {
                    let Some(f) = favs.item(group, index) else { continue };
                    gfx.icon(icon_of(f), R::new(row.l + 12.0, y + 8.0, row.l + 28.0, y + 24.0));
                    gfx.text(&f.name, R::new(row.l + 36.0, y, row.r - 10.0, y + FAV_ROW_H), Font::Normal, fg);
                }
            }
            if let (EditMode::FavRename(er), true) = (edit_mode, i == sel) {
                if er == rows[i] {
                    let er = R::new(row.l + 34.0, y + 3.0, row.r - 8.0, y + FAV_ROW_H - 3.0);
                    gfx.fill_round(er, 4.0, t.pane);
                    gfx.stroke_round(er.inset(0.75, 0.75), 4.0, t.accent, 1.5);
                }
            }
        }
        // 끌어서 옮길 자리 (그룹 줄 위가 아니면 줄 사이 선)
        if let Some((from, pos)) = moving.filter(|_| drop_into.is_none()) {
            let n_items = favs.items.len();
            let slot = match rows.get(from) {
                Some(Row::Group(_)) => (pos.round().max(0.0) as usize).clamp(n_items, rows.len()),
                _ => (pos.round().max(0.0) as usize).min(n_items),
            };
            let ly = list.t + slot as f32 * FAV_ROW_H - scroll;
            gfx.fill_round(R::new(list.l + 8.0, ly - 1.5, list.r - 8.0, ly + 1.5), 1.5, t.accent);
        }
        gfx.unclip();

        gfx.text(
            &format!("→ 그룹 · Enter 열기 · F2 이름 · Del 삭제{}", help.as_deref().map(|h| format!(" · {h} 도움말")).unwrap_or_default()),
            R::new(r.l + 14.0, r.b - FAV_FOOT_H, r.r - 10.0, r.b - 2.0),
            Font::Small,
            t.faint,
        );

        // 오른쪽 메뉴: 처음 열 때만 왼쪽에서 살짝 미끄러져 나오고, 그룹을 바꾸면 내용만 바뀐다
        if let Some((fr, f)) = fly {
            let Some(group) = favs.groups.get(f.group) else { return };
            let a = f.anim;
            let dx = (1.0 - a) * if fr.l > r.l { -14.0 } else { 14.0 };
            let fr = R::new(fr.l + dx, fr.t, fr.r + dx, fr.b);
            gfx.fill_round(R::new(fr.l - 2.0, fr.t + 1.0, fr.r + 2.0, fr.b + 4.0), 12.0, shadow(a));
            gfx.fill_round(fr, 10.0, alpha(t.pane, a));
            gfx.stroke_round(fr.inset(0.5, 0.5), 10.0, alpha(t.border, a), 1.0);
            let flist = Self::fav_fly_list(fr);
            gfx.clip(flist);
            if group.items.is_empty() {
                gfx.text("비어 있어요 · 즐겨찾기를 이 그룹 줄에 끌어 놓으세요", flist.inset(8.0, 0.0), Font::Small, alpha(t.faint, a));
            }
            for (j, item) in group.items.iter().enumerate() {
                let Some(row) = fly_rows.get(j).map(|rr| R::new(rr.l + dx, rr.t, rr.r + dx, rr.b)) else { break };
                if row.b < flist.t || row.t > flist.b {
                    continue;
                }
                if f.sel == Some(j) {
                    gfx.fill_round(row.inset(0.0, 1.0), 6.0, alpha(if keyboard_in_fly { t.sel } else { t.hover }, a));
                } else if hover == Hit::FavFlyItem(j) {
                    gfx.fill_round(row.inset(0.0, 1.0), 6.0, alpha(t.hover, a));
                }
                gfx.icon(icon_of(item), R::new(row.l + 8.0, row.t + 8.0, row.l + 24.0, row.t + 24.0));
                gfx.text(&item.name, R::new(row.l + 34.0, row.t, row.r - 8.0, row.b), Font::Normal, alpha(t.text, a));
                if edit_mode == EditMode::FavRename(Row::Item { group: Some(f.group), index: j }) {
                    let er = R::new(row.l + 34.0, row.t + 3.0, row.r - 8.0, row.b - 3.0);
                    gfx.fill_round(er, 4.0, t.pane);
                    gfx.stroke_round(er.inset(0.75, 0.75), 4.0, t.accent, 1.5);
                }
            }
            gfx.unclip();
        }

        // 경로 말풍선: 줄 아래(자리가 없으면 위)에
        if let (Some((at, path)), Some(tw)) = (tip, tip_w) {
            let tw = tw.min(w - 2.0 * MARGIN);
            let l = (at.l + 30.0).min(w - MARGIN - tw).max(MARGIN);
            let top = if at.b + 30.0 <= h - 4.0 { at.b + 2.0 } else { at.t - 28.0 };
            let tr = R::new(l, top, l + tw, top + 26.0);
            gfx.fill_round(R::new(tr.l - 1.0, tr.t + 1.0, tr.r + 1.0, tr.b + 3.0), 7.0, shadow(0.8));
            gfx.fill_round(tr, 6.0, t.pane);
            gfx.stroke_round(tr.inset(0.5, 0.5), 6.0, t.border, 1.0);
            gfx.text(&path, R::new(tr.l + 10.0, tr.t, tr.r - 8.0, tr.b), Font::Small, t.text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 오른쪽 메뉴 쪽으로 대각선으로 가는 움직임은 삼각형 안, 다른 줄로 곧장 내려가는 움직임은 밖
    #[test]
    fn aiming_triangle() {
        let (from, tl, bl) = ((100.0, 100.0), (300.0, 80.0), (300.0, 400.0));
        assert!(in_triangle((140.0, 130.0), from, tl, bl), "diagonal toward the menu");
        assert!(!in_triangle((100.0, 140.0), from, tl, bl), "straight down to another row");
        assert!(!in_triangle((60.0, 120.0), from, tl, bl), "away from the menu");
    }
}
