//! 즐겨찾기 팝업. 도구 막대 버튼이나 Ctrl+B로 열고, 표시 이름·순서·그룹은 사용자가 정한다

use super::*;
use crate::favorites::{self, Favorite, Row, SortBy};

const FAV_W: f32 = 480.0;
const FAV_HEAD_H: f32 = 46.0;
const FAV_FOOT_H: f32 = 30.0;
const FAV_ROW_H: f32 = 32.0;
/// 그룹 안 항목의 들여쓰기
const FAV_INDENT: f32 = 18.0;
pub(super) const G_STAR: u16 = 0xE734;
const G_EDIT: u16 = 0xE70F;
const G_REMOVE: u16 = 0xE711;
const G_SORT: u16 = 0xE8CB;
const G_NEW_GROUP: u16 = 0xE8F4;
const G_CHEVRON_DOWN: u16 = 0xE70D;
/// 그룹 옆 목록이 미끄러져 나오는 애니메이션
pub(super) const TIMER_FLY: usize = 40;
/// 마우스가 그룹을 벗어난 뒤 옆 목록을 닫기까지 (대각선으로 옮겨 가는 동안 닫히지 않게)
pub(super) const TIMER_FLY_CLOSE: usize = 41;
const FLY_W: f32 = 320.0;

const FAV_BUTTON_W: f32 = 100.0;

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

pub(super) fn show_fav_menu(hwnd: HWND, index: usize, pt: POINT) {
    let Some(info) = with_app(|a| {
        let row = a.favs.rows().get(index).copied()?;
        let groups: Vec<(String, usize)> = a.favs.groups.iter().map(|g| (g.name.clone(), g.items.len())).collect();
        let is_dir = match row {
            Row::Item { group, index } => a.favs.item(group, index).is_some_and(|f| f.is_dir),
            Row::Group(_) => true,
        };
        let collapsed = matches!(row, Row::Group(g) if a.favs.groups[g].collapsed);
        Some((row, groups, is_dir, collapsed))
    })
    .flatten() else {
        return;
    };
    match info {
        (Row::Group(g), groups, _, collapsed) => show_group_menu(hwnd, index, g, &groups[g], collapsed, pt),
        (Row::Item { group, .. }, groups, is_dir, _) => show_item_menu(hwnd, index, group, &groups, is_dir, pt),
    }
}

fn show_item_menu(hwnd: HWND, index: usize, current: Option<usize>, groups: &[(String, usize)], is_dir: bool, pt: POINT) {
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
        1 | 2 => a.fav_open_item(index, cmd == 2),
        3 => a.begin_fav_rename(index),
        4 => a.fav_nudge(index, true),
        5 => a.fav_nudge(index, false),
        6 => a.remove_favorite(index),
        NO_GROUP => a.fav_move_to_group(index, None),
        NEW_GROUP => a.fav_move_to_new_group(index),
        _ if cmd >= GROUP && ((cmd - GROUP) as usize) < groups.len() => a.fav_move_to_group(index, Some((cmd - GROUP) as usize)),
        _ => {}
    });
}

fn show_group_menu(hwnd: HWND, index: usize, g: usize, group: &(String, usize), collapsed: bool, pt: POINT) {
    let cmd = popup_menu(
        hwnd,
        pt,
        &[
            (1, if collapsed { "펼치기" } else { "접기" }),
            (0, ""),
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
        1 => a.fav_toggle_group(g),
        2 => a.begin_fav_rename(index),
        3 => a.fav_nudge(index, true),
        4 => a.fav_nudge(index, false),
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

impl App {
    pub(super) fn fav_rect(&self) -> R {
        let b = self.fav_button();
        let w = FAV_W.min(self.w - b.l - MARGIN).max(260.0);
        let rows = self.favs.rows().len().max(2) as f32;
        let max_h = (self.h - TOOLBAR_H - 12.0).max(160.0);
        let h = (FAV_HEAD_H + rows * FAV_ROW_H + FAV_FOOT_H + 8.0).min(max_h);
        R::new(b.l, TOOLBAR_H - 4.0, b.l + w, TOOLBAR_H - 4.0 + h)
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

    fn fav_add_button(&self) -> R {
        let r = self.fav_rect();
        R::new(r.r - 86.0, r.t + 9.0, r.r - 10.0, r.t + 37.0)
    }

    fn fav_sort_button(&self) -> R {
        let a = self.fav_add_button();
        R::new(a.l - 76.0, a.t, a.l - 6.0, a.b)
    }

    fn fav_group_button(&self) -> R {
        let s = self.fav_sort_button();
        R::new(s.l - 76.0, s.t, s.l - 6.0, s.b)
    }

    fn fav_row_buttons(row: R) -> (R, R) {
        let remove = R::new(row.r - 32.0, row.t + 4.0, row.r - 6.0, row.b - 4.0);
        let edit = R::new(remove.l - 28.0, remove.t, remove.l - 2.0, remove.b);
        (edit, remove)
    }

    fn fav_indent(row: Row) -> f32 {
        if matches!(row, Row::Item { group: Some(_), .. }) { FAV_INDENT } else { 0.0 }
    }

    /// 이름 바꾸기 입력 상자 자리 (표시 이름 칸)
    pub(super) fn fav_rename_rect(&self, i: usize) -> Option<R> {
        let row = self.fav_row(i);
        let list = self.fav_list();
        if row.t < list.t - 1.0 || row.b > list.b + 1.0 {
            return None;
        }
        let l = row.l + 36.0 + Self::fav_indent(*self.favs.rows().get(i)?);
        Some(R::new(l, row.t + 3.0, l + (row.r - l) * 0.5, row.b - 3.0))
    }

    /// 접힌 그룹에 마우스를 올리면 팝업 옆에 띄우는 목록의 자리
    fn fav_fly_rect(&self) -> Option<R> {
        let (g, _) = self.fav_fly?;
        let n = self.favs.groups.get(g)?.items.len();
        let row = self.fav_row(self.favs.row_index(Row::Group(g))?);
        let pop = self.fav_rect();
        let h = (n.max(1) as f32 * FAV_ROW_H + 12.0).min((self.h - TOOLBAR_H - 8.0).max(FAV_ROW_H + 12.0));
        // 오른쪽에 자리가 없으면 창 오른쪽 끝에 붙인다
        let l = if pop.r + 4.0 + FLY_W <= self.w - MARGIN { pop.r + 4.0 } else { (self.w - MARGIN - FLY_W).max(MARGIN) };
        let t = (row.t - 6.0).min(self.h - h - 8.0).max(TOOLBAR_H - 4.0);
        Some(R::new(l, t, l + FLY_W, t + h))
    }

    fn fav_fly_row(fly: R, j: usize) -> R {
        let y = fly.t + 6.0 + j as f32 * FAV_ROW_H;
        R::new(fly.l + 6.0, y, fly.r - 6.0, y + FAV_ROW_H)
    }

    /// 마우스가 옮겨 갈 때: 접힌 그룹 위면 옆 목록을 띄우고, 목록·그룹 밖으로 나가면 잠시 뒤 닫는다
    pub(super) fn fav_fly_hover(&mut self, hit: Hit) {
        let group = match hit {
            Hit::FavRow(i) => match self.favs.rows().get(i) {
                Some(Row::Group(g)) if self.favs.groups[*g].collapsed && !self.favs.groups[*g].items.is_empty() => Some(*g),
                _ => None,
            },
            Hit::FavFlyItem(_) | Hit::FavFlyPanel => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_FLY_CLOSE);
                }
                return;
            }
            _ => None,
        };
        unsafe {
            match group {
                Some(g) => {
                    let _ = KillTimer(Some(self.hwnd), TIMER_FLY_CLOSE);
                    if self.fav_fly.map(|(fg, _)| fg) != Some(g) {
                        self.fav_fly = Some((g, 0.0));
                        SetTimer(Some(self.hwnd), TIMER_FLY, 15, None);
                    }
                }
                None if self.fav_fly.is_some() => {
                    SetTimer(Some(self.hwnd), TIMER_FLY_CLOSE, 250, None);
                }
                None => {}
            }
        }
        self.invalidate();
    }

    pub(super) fn fav_fly_timer(&mut self, id: usize) {
        if id == TIMER_FLY_CLOSE {
            self.fav_fly_close();
            return;
        }
        match &mut self.fav_fly {
            Some((_, a)) if *a < 1.0 => *a = (*a + 0.2).min(1.0),
            _ => unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_FLY);
            },
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

    /// 옆 목록의 항목을 오른쪽 버튼으로 눌렀을 때: 그룹을 펼쳐 그 항목을 고르고 줄 번호를 돌려준다 (항목 메뉴용)
    pub(super) fn fav_fly_reveal(&mut self, j: usize) -> Option<usize> {
        let (g, _) = self.fav_fly?;
        self.fav_fly_close();
        self.favs.groups.get_mut(g)?.collapsed = false;
        let i = self.favs.row_index(Row::Item { group: Some(g), index: j })?;
        self.fav_changed(Some(Row::Item { group: Some(g), index: j }));
        Some(i)
    }

    pub(super) fn fav_hit(&self, x: f32, y: f32) -> Option<Hit> {
        if let Some(fly) = self.fav_fly_rect().filter(|f| f.contains(x, y)) {
            let n = self.fav_fly.and_then(|(g, _)| self.favs.groups.get(g)).map_or(0, |g| g.items.len());
            return Some((0..n).find(|&j| Self::fav_fly_row(fly, j).contains(x, y)).map_or(Hit::FavFlyPanel, Hit::FavFlyItem));
        }
        let r = self.fav_rect();
        if !r.contains(x, y) {
            return None;
        }
        if self.fav_add_button().contains(x, y) {
            return Some(Hit::FavAdd);
        }
        if self.fav_sort_button().contains(x, y) {
            return Some(Hit::FavSort);
        }
        if self.fav_group_button().contains(x, y) {
            return Some(Hit::FavNewGroup);
        }
        let list = self.fav_list();
        if list.contains(x, y) {
            let i = ((y - list.t + self.fav_scroll) / FAV_ROW_H).floor();
            if i >= 0.0 && (i as usize) < self.favs.rows().len() {
                let i = i as usize;
                let (edit, remove) = Self::fav_row_buttons(self.fav_row(i));
                return Some(if remove.contains(x, y) {
                    Hit::FavRemove(i)
                } else if edit.contains(x, y) {
                    Hit::FavEdit(i)
                } else {
                    Hit::FavRow(i)
                });
            }
        }
        Some(Hit::FavPanel)
    }

    pub(super) fn toggle_favorites(&mut self) {
        if self.fav_open {
            self.close_favorites();
        } else {
            self.open_favorites(None);
        }
    }

    pub(super) fn open_favorites(&mut self, select: Option<usize>) {
        self.close_palette();
        if !self.fav_open && self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        self.fav_open = true;
        self.fav_sel = select.unwrap_or(self.fav_sel).min(self.favs.rows().len().saturating_sub(1));
        self.hover = Hit::None;
        self.fav_ensure_visible();
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
        if matches!(self.drag, Drag::FavPress { .. } | Drag::FavMove { .. }) {
            self.drag = Drag::None;
        }
        self.layout_edits();
        self.invalidate();
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

    /// 줄을 고르고 보이게 한 뒤 저장한다 (항목을 옮기거나 그룹을 바꾼 다음)
    fn fav_changed(&mut self, select: Option<Row>) {
        if let Some(i) = select.and_then(|r| self.favs.row_index(r)) {
            self.fav_sel = i;
        }
        self.fav_sel = self.fav_sel.min(self.favs.rows().len().saturating_sub(1));
        self.save_favorites();
        self.fav_ensure_visible();
        self.invalidate();
    }

    /// 항목은 열고(폴더는 이동, 파일은 실행), 그룹 머리 줄은 접거나 펼친다.
    /// new_tab: 폴더는 새 탭, 파일은 그 파일이 있는 폴더를 새 탭에서 열고 선택
    pub(super) fn fav_open_item(&mut self, i: usize, new_tab: bool) {
        match self.favs.rows().get(i) {
            Some(Row::Group(g)) => self.fav_toggle_group(*g),
            Some(Row::Item { group, index }) => self.fav_open_entry(*group, *index, new_tab),
            None => {}
        }
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

    /// 팝업이 열려 있고 그룹 안(또는 그룹 머리 줄)을 고르고 있으면 그 그룹에, 아니면 그룹 없음 끝에 넣는다
    pub fn add_favorite_path(&mut self, path: &str, name: Option<String>) {
        let path = util::normalize(path);
        if let Some((group, index)) = self.favs.find(&path) {
            if let Some(g) = group {
                self.favs.groups[g].collapsed = false;
            }
            let row = self.favs.row_index(Row::Item { group, index });
            self.open_favorites(row);
            let p = self.active;
            self.flash(p, "이미 즐겨찾기에 있습니다");
            return;
        }
        let target = match self.fav_open.then(|| self.favs.rows().get(self.fav_sel).copied()).flatten() {
            Some(Row::Group(g)) | Some(Row::Item { group: Some(g), .. }) => Some(g),
            _ => None,
        };
        let name = name.unwrap_or_else(|| favorites::default_name(&path));
        let is_dir = shell::is_dir(&path);
        let (group, index) = self.favs.push(target, Favorite { name, path, is_dir });
        self.save_favorites();
        let i = self.favs.row_index(Row::Item { group, index });
        self.open_favorites(i);
        // 추가하자마자 원하는 표시 이름(예: "01.일간 업무")을 붙일 수 있게 한다. Enter면 그대로
        if let Some(i) = i {
            self.begin_fav_rename(i);
        }
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
        let i = self.favs.row_index(Row::Group(g)).unwrap_or(0);
        self.open_favorites(Some(i));
        self.begin_fav_rename(i);
        g
    }

    pub(super) fn begin_fav_rename(&mut self, i: usize) {
        let Some(name) = self.favs.rows().get(i).and_then(|r| self.favs.row_name(*r)).map(str::to_string) else { return };
        if !self.fav_open {
            self.open_favorites(Some(i));
        }
        self.fav_sel = i;
        self.fav_ensure_visible();
        self.edit_mode = EditMode::FavRename(i);
        unsafe {
            let _ = SetWindowTextW(self.rename_edit, &HSTRING::from(name));
            SendMessageW(self.rename_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    pub(super) fn commit_fav_rename(&mut self, i: usize) {
        let text = favorites::clean_name(&edit_text(self.rename_edit));
        self.close_edit();
        let Some(row) = self.favs.rows().get(i).copied() else { return };
        if !text.is_empty() && self.favs.set_row_name(row, text) {
            self.save_favorites();
        }
    }

    /// Del: 항목은 지우고, 그룹은 풀어서 안의 항목을 남긴다
    pub(super) fn remove_favorite(&mut self, i: usize) {
        let Some(row) = self.favs.rows().get(i).copied() else { return };
        if let EditMode::FavRename(_) = self.edit_mode {
            self.cancel_edit();
        }
        match row {
            Row::Item { group, index } => {
                self.favs.remove_item(group, index);
            }
            Row::Group(g) => self.favs.ungroup(g),
        }
        self.fav_changed(None);
    }

    pub(super) fn fav_toggle_group(&mut self, g: usize) {
        if let Some(group) = self.favs.groups.get_mut(g) {
            group.collapsed = !group.collapsed;
            self.fav_changed(Some(Row::Group(g)));
        }
    }

    pub(super) fn fav_ungroup(&mut self, g: usize) {
        self.favs.ungroup(g);
        self.fav_changed(None);
    }

    pub(super) fn fav_delete_group(&mut self, g: usize) {
        self.favs.delete_group(g);
        self.fav_changed(None);
    }

    fn fav_item_at(&self, i: usize) -> Option<(Option<usize>, usize)> {
        match self.favs.rows().get(i) {
            Some(Row::Item { group, index }) => Some((*group, *index)),
            _ => None,
        }
    }

    pub(super) fn fav_move_to_group(&mut self, i: usize, group: Option<usize>) {
        let Some(from) = self.fav_item_at(i) else { return };
        let to = self.favs.section(group).len();
        if let Some((group, index)) = self.favs.move_item(from, group, to) {
            self.fav_changed(Some(Row::Item { group, index }));
        }
    }

    pub(super) fn fav_move_to_new_group(&mut self, i: usize) {
        let Some(from) = self.fav_item_at(i) else { return };
        let g = self.favs.add_group("새 그룹".to_string());
        self.favs.move_item(from, Some(g), 0);
        self.fav_changed(None);
        if let Some(i) = self.favs.row_index(Row::Group(g)) {
            self.begin_fav_rename(i);
        }
    }

    /// Ctrl+↑↓ 또는 메뉴의 위로·아래로
    pub(super) fn fav_nudge(&mut self, i: usize, up: bool) {
        let Some(row) = self.favs.rows().get(i).copied() else { return };
        if let Some(moved) = self.favs.nudge(row, up) {
            self.fav_changed(Some(moved));
        }
    }

    pub(super) fn sort_favorites(&mut self, by: SortBy) {
        self.favs.sort(by);
        self.fav_changed(None);
    }

    /// 팝업이 열려 있을 때의 키. 처리했으면 true
    pub(super) fn fav_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let rows = self.favs.rows();
        let n = rows.len();
        let s = self.fav_sel;
        let page = ((self.fav_list().h() / FAV_ROW_H).floor() as usize).max(1);
        match vk {
            VK_ESCAPE => self.close_favorites(),
            VK_UP if ctrl => self.fav_nudge(s, true),
            VK_DOWN if ctrl => self.fav_nudge(s, false),
            VK_UP => self.fav_sel = s.saturating_sub(1),
            VK_DOWN => self.fav_sel = (s + 1).min(n.saturating_sub(1)),
            VK_PRIOR => self.fav_sel = s.saturating_sub(page),
            VK_NEXT => self.fav_sel = (s + page).min(n.saturating_sub(1)),
            VK_HOME => self.fav_sel = 0,
            VK_END => self.fav_sel = n.saturating_sub(1),
            // ←: 그룹을 접거나 그룹 안 항목에서 머리 줄로, →: 접힌 그룹을 펼친다
            VK_LEFT => match rows.get(s) {
                Some(Row::Group(g)) if !self.favs.groups[*g].collapsed => self.fav_toggle_group(*g),
                Some(Row::Item { group: Some(g), .. }) => self.fav_sel = self.favs.row_index(Row::Group(*g)).unwrap_or(s),
                _ => {}
            },
            VK_RIGHT => {
                if let Some(Row::Group(g)) = rows.get(s) {
                    if self.favs.groups[*g].collapsed {
                        self.fav_toggle_group(*g);
                    }
                }
            }
            VK_RETURN => {
                self.fav_open_item(s, ctrl);
                return true;
            }
            VK_F2 => {
                self.begin_fav_rename(s);
                return true;
            }
            VK_DELETE => self.remove_favorite(s),
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
        if self.fav_open {
            self.fav_ensure_visible();
        }
        self.invalidate();
        true
    }

    /// 팝업이 열려 있을 때 글자를 누르면 그 글자로 시작하는 다음 줄(항목·그룹)로 이동
    pub fn fav_type_char(&mut self, ch: u16) -> bool {
        if !self.fav_open || self.edit_mode != EditMode::None {
            return false;
        }
        let rows = self.favs.rows();
        let n = rows.len();
        let Some(c) = char::from_u32(ch as u32).filter(|c| !c.is_control()) else { return true };
        let needle = c.to_lowercase().to_string();
        for k in 1..=n {
            let i = (self.fav_sel + k) % n;
            if self.favs.row_name(rows[i]).is_some_and(|name| name.to_lowercase().starts_with(&needle)) {
                self.fav_sel = i;
                self.fav_ensure_visible();
                self.invalidate();
                break;
            }
        }
        true
    }

    /// 팝업이 열려 있을 때 왼쪽 버튼. 밖을 누르면 닫기만 하고 클릭은 소비한다
    pub(super) fn fav_mouse_down(&mut self, hit: Hit, y: f32) {
        match hit {
            Hit::FavAdd => self.add_favorite(),
            Hit::FavNewGroup => {
                self.add_fav_group();
            }
            Hit::FavSort => {
                let r = self.fav_sort_button();
                let pt = self.to_screen(r.l, r.b + 2.0);
                self.pending.push(Deferred::FavSortMenu { pt });
            }
            Hit::FavEdit(i) => self.begin_fav_rename(i),
            Hit::FavRemove(i) => self.remove_favorite(i),
            Hit::FavRow(i) => {
                self.fav_sel = i;
                self.drag = Drag::FavPress { index: i, y };
                unsafe { SetCapture(self.hwnd) };
            }
            Hit::FavFlyItem(j) => {
                if let Some((g, _)) = self.fav_fly {
                    self.fav_open_entry(Some(g), j, key_down(VK_CONTROL));
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

    /// 끌어 놓을 자리(줄 사이 경계, 0..=줄 수)
    fn fav_drop_slot(&self, y: f32) -> usize {
        let list = self.fav_list();
        let slot = ((y - list.t + self.fav_scroll) / FAV_ROW_H + 0.5).floor().max(0.0) as usize;
        slot.min(self.favs.rows().len())
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
                let slot = self.fav_drop_slot(y);
                match self.favs.rows().get(from).copied() {
                    // 항목은 놓은 자리의 그룹으로 들어가고, 그룹은 안의 항목과 함께 옮겨진다
                    Some(Row::Item { group, index }) => {
                        let (to_group, to_index) = self.favs.item_drop_target(slot);
                        if let Some((group, index)) = self.favs.move_item((group, index), to_group, to_index) {
                            self.fav_changed(Some(Row::Item { group, index }));
                        }
                    }
                    Some(Row::Group(g)) => {
                        let to = self.favs.group_drop_target(slot, g);
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

    pub(super) fn fav_wheel(&mut self, dy: f32) {
        self.fav_scroll += dy;
        self.fav_clamp_scroll();
        if self.edit_mode != EditMode::None {
            self.layout_edits();
        }
        self.invalidate();
    }

    pub(super) fn paint_favorites(&mut self) {
        let t = self.theme;
        let r = self.fav_rect();
        let list = self.fav_list();
        let (add, sort, group_b) = (self.fav_add_button(), self.fav_sort_button(), self.fav_group_button());
        let hover = self.hover;
        let (sel, scroll, edit_mode) = (self.fav_sel, self.fav_scroll, self.edit_mode);
        let moving = match self.drag {
            Drag::FavMove { from, y } => Some((from, self.fav_drop_slot(y))),
            _ => None,
        };
        let rows = self.favs.rows();
        let fly = self.fav_fly_rect().zip(self.fav_fly);
        let help = self.keys.get(crate::keys::Action::Help).map(crate::keys::Combo::label);
        let add_key = self.keys.get(crate::keys::Action::AddFavorite).map(crate::keys::Combo::label);
        let App { gfx, favs, types, .. } = self;
        let folder_icon = types.entry("\\dir".to_string()).or_insert_with(|| shell::type_info("", true)).1;

        // 그림자 + 본체
        let shadow = Color { r: 0.0, g: 0.0, b: 0.0, a: if t.dark { 0.45 } else { 0.12 } };
        gfx.fill_round(R::new(r.l - 2.0, r.t + 1.0, r.r + 2.0, r.b + 4.0), 12.0, shadow);
        gfx.fill_round(r, 10.0, t.pane);
        gfx.stroke_round(r.inset(0.5, 0.5), 10.0, t.border, 1.0);

        // 머리: 제목, 개수, 그룹·정렬·추가 버튼
        gfx.glyph(G_STAR, R::new(r.l + 12.0, r.t, r.l + 34.0, r.t + FAV_HEAD_H), false, t.accent);
        gfx.text("즐겨찾기", R::new(r.l + 38.0, r.t, r.l + 120.0, r.t + FAV_HEAD_H), Font::Strong, t.text);
        gfx.text(&format!("{}개", favs.count()), R::new(r.l + 104.0, r.t, group_b.l - 8.0, r.t + FAV_HEAD_H), Font::Normal, t.faint);
        for (b, glyph, label, h) in [(group_b, G_NEW_GROUP, "그룹", Hit::FavNewGroup), (sort, G_SORT, "정렬", Hit::FavSort), (add, G_ADD, "추가", Hit::FavAdd)] {
            gfx.fill_round(b, 6.0, if hover == h { t.sel_inactive } else { t.field });
            gfx.glyph(glyph, R::new(b.l + 6.0, b.t, b.l + 26.0, b.b), true, t.text);
            gfx.text(label, R::new(b.l + 28.0, b.t, b.r - 4.0, b.b), Font::Normal, t.text);
        }
        gfx.fill(R::new(r.l + 10.0, r.t + FAV_HEAD_H - 1.0, r.r - 10.0, r.t + FAV_HEAD_H), t.border);

        // 목록
        gfx.clip(list);
        if rows.is_empty() {
            let how = add_key.as_deref().map(|k| format!("경로 상자의 ☆나 {k}로")).unwrap_or_else(|| "경로 상자의 ☆로".to_string());
            gfx.text(
                &format!("즐겨찾기가 비어 있어요\n{how} 폴더·파일을 넣고, ‘그룹’으로 묶어 보세요"),
                list.inset(16.0, 8.0),
                Font::Hint,
                t.faint,
            );
        }
        let first = (scroll / FAV_ROW_H).floor().max(0.0) as usize;
        let count = (list.h() / FAV_ROW_H).ceil() as usize + 1;
        for i in first..(first + count).min(rows.len()) {
            let y = list.t + i as f32 * FAV_ROW_H - scroll;
            let row = R::new(list.l, y, list.r, y + FAV_ROW_H);
            let body = row.inset(2.0, 1.0);
            let row_hover = matches!(hover, Hit::FavRow(h) | Hit::FavEdit(h) | Hit::FavRemove(h) if h == i);
            let dragged = moving.is_some_and(|(from, _)| from == i);
            let flying = matches!((rows[i], fly), (Row::Group(g), Some((_, (fg, _)))) if g == fg);
            if i == sel {
                gfx.fill_round(body, 6.0, t.sel);
            } else if row_hover || flying {
                gfx.fill_round(body, 6.0, t.hover);
            }
            let fg = if dragged { t.faint } else { t.text };
            let indent = Self::fav_indent(rows[i]);
            let show_buttons = row_hover || i == sel;
            let text_r = if show_buttons { row.r - 66.0 } else { row.r - 12.0 };
            let name_l = row.l + 40.0 + indent;
            match rows[i] {
                Row::Group(g) => {
                    let group = &favs.groups[g];
                    let chevron = if group.collapsed { G_CHEVRON } else { G_CHEVRON_DOWN };
                    gfx.glyph(chevron, R::new(row.l + 10.0, y, row.l + 30.0, y + FAV_ROW_H), true, t.dim);
                    gfx.text(&group.name, R::new(name_l, y, text_r, y + FAV_ROW_H), Font::Strong, fg);
                    gfx.text(&format!("{}개", group.items.len()), R::new(name_l, y, text_r, y + FAV_ROW_H), Font::Right, t.faint);
                }
                Row::Item { group, index } => {
                    let Some(f) = favs.item(group, index) else { continue };
                    let icon = if f.is_dir {
                        folder_icon
                    } else {
                        let name = util::file_name(&f.path);
                        let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                        types.entry(ext.clone()).or_insert_with(|| shell::type_info(&ext, false)).1
                    };
                    gfx.icon(icon, R::new(row.l + 12.0 + indent, y + 8.0, row.l + 28.0 + indent, y + 24.0));
                    let split = name_l - 4.0 + (row.r - name_l) * 0.5;
                    gfx.text(&f.name, R::new(name_l, y, split - 8.0, y + FAV_ROW_H), Font::Normal, fg);
                    gfx.text(&f.path, R::new(split, y, text_r, y + FAV_ROW_H), Font::Path, t.faint);
                }
            }
            if show_buttons && !dragged {
                let (edit, remove) = Self::fav_row_buttons(row);
                for (b, glyph, h) in [(edit, G_EDIT, Hit::FavEdit(i)), (remove, G_REMOVE, Hit::FavRemove(i))] {
                    if hover == h {
                        gfx.fill_round(b, 5.0, t.sel_inactive);
                    }
                    gfx.glyph(glyph, b, true, t.dim);
                }
            }
            if edit_mode == EditMode::FavRename(i) {
                let er = R::new(row.l + 36.0 + indent, y + 3.0, row.l + 36.0 + indent + (row.r - row.l - 36.0 - indent) * 0.5, y + FAV_ROW_H - 3.0);
                gfx.fill_round(er, 4.0, t.pane);
                gfx.stroke_round(er.inset(0.75, 0.75), 4.0, t.accent, 1.5);
            }
        }
        // 끌어서 옮길 자리 표시
        if let Some((_, slot)) = moving {
            let ly = list.t + slot as f32 * FAV_ROW_H - scroll;
            gfx.fill_round(R::new(list.l + 8.0, ly - 1.5, list.r - 8.0, ly + 1.5), 1.5, t.accent);
        }
        gfx.unclip();

        gfx.text(
            &format!("클릭 열기 · F2 이름 · Del 삭제 · 끌어서 정리{}", help.as_deref().map(|h| format!(" · {h} 도움말")).unwrap_or_default()),
            R::new(r.l + 14.0, r.b - FAV_FOOT_H, r.r - 10.0, r.b - 2.0),
            Font::Normal,
            t.faint,
        );

        // 그룹 옆 목록: 왼쪽에서 살짝 미끄러지며 나타난다
        if let Some((fr, (g, a))) = fly {
            let Some(group) = favs.groups.get(g) else { return };
            let dx = (1.0 - a) * -14.0;
            let fr = R::new(fr.l + dx, fr.t, fr.r + dx, fr.b);
            let shadow = Color { r: 0.0, g: 0.0, b: 0.0, a: if t.dark { 0.45 } else { 0.12 } * a };
            gfx.fill_round(R::new(fr.l - 2.0, fr.t + 1.0, fr.r + 2.0, fr.b + 4.0), 12.0, shadow);
            gfx.fill_round(fr, 10.0, alpha(t.pane, a));
            gfx.stroke_round(fr.inset(0.5, 0.5), 10.0, alpha(t.border, a), 1.0);
            gfx.clip(fr);
            for (j, f) in group.items.iter().enumerate() {
                let row = Self::fav_fly_row(fr, j);
                if row.t > fr.b {
                    break;
                }
                if hover == Hit::FavFlyItem(j) {
                    gfx.fill_round(row.inset(0.0, 1.0), 6.0, alpha(t.hover, a));
                }
                let icon = if f.is_dir {
                    folder_icon
                } else {
                    let ext = util::file_name(&f.path).rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                    types.entry(ext.clone()).or_insert_with(|| shell::type_info(&ext, false)).1
                };
                gfx.icon(icon, R::new(row.l + 8.0, row.t + 8.0, row.l + 24.0, row.t + 24.0));
                let split = row.l + 34.0 + (row.w() - 34.0) * 0.55;
                gfx.text(&f.name, R::new(row.l + 34.0, row.t, split - 6.0, row.b), Font::Normal, alpha(t.text, a));
                gfx.text(util::file_name(&f.path), R::new(split, row.t, row.r - 8.0, row.b), Font::Path, alpha(t.faint, a));
            }
            gfx.unclip();
        }
    }
}
