//! 사이드바 목록의 동작. 즐겨찾기('모든 공간'): 열기·추가·이름 바꾸기·그룹·정렬·메뉴·키보드·끌어서 옮기기.
//! 이 공간에 고정한 것(작업 공간마다 따로): 책갈피·Ctrl+D·끌어 놓기로 고정, 열기, 순서, '모든 공간'으로 옮기기.
//! 줄 배치와 그리기는 sidebar.rs. 그룹은 그 자리에서 접고 펼치고, 표시 이름·순서·그룹은 사용자가 정한다.
//! Ctrl+B는 키보드를 사이드바 목록으로 옮긴다(다시 누르거나 Esc면 칸으로). ↑↓는 화면 순서대로 고정한 곳 → 내 PC → 즐겨찾기를 오간다

use super::sidebar::SIDE_ROW_H;
use super::*;
use crate::favorites::{self, Favorite, Row, SortBy};

/// 키보드가 즐겨찾기 줄이 아니라 그 위의 줄에 있을 때 그 줄 (지금 작업 공간에 고정한 곳의 번호, 또는 내 PC)
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum TopSel {
    Pin(usize),
    Computer,
}

/// 네이티브 팝업 메뉴. id 0은 구분선. 고른 id(없으면 0)
pub(super) fn popup_menu(hwnd: HWND, pt: POINT, items: &[(u32, &str)]) -> u32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        append_items(menu, items);
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;
        let _ = DestroyMenu(menu);
        cmd
    }
}

pub(super) fn append_items(menu: HMENU, items: &[(u32, &str)]) {
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

/// 즐겨찾기 줄(항목·그룹)의 오른쪽 버튼 메뉴
pub(super) fn show_fav_menu(hwnd: HWND, row: Row, pt: POINT) {
    let Some((groups, is_dir, collapsed)) = with_app(|a| {
        let groups: Vec<(String, usize)> = a.favs.groups.iter().map(|g| (g.name.clone(), g.items.len())).collect();
        let (is_dir, collapsed) = match row {
            Row::Item { group, index } => (a.favs.item(group, index)?.is_dir, false),
            Row::Group(g) => (true, a.favs.groups.get(g)?.collapsed),
        };
        Some((groups, is_dir, collapsed))
    })
    .flatten() else {
        return;
    };
    match row {
        Row::Group(g) => show_group_menu(hwnd, g, &groups[g], collapsed, pt),
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
        append_items(menu, &[(9, "이 작업 공간에만 두기"), (0, ""), (6, "모든 공간에서 빼기\tDel")]);
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
        9 => a.fav_move_to_pins(row),
        NO_GROUP => a.fav_move_to_group(row, None),
        NEW_GROUP => a.fav_move_to_new_group(row),
        _ if cmd >= GROUP && ((cmd - GROUP) as usize) < groups.len() => a.fav_move_to_group(row, Some((cmd - GROUP) as usize)),
        _ => {}
    });
}

fn show_group_menu(hwnd: HWND, g: usize, group: &(String, usize), collapsed: bool, pt: POINT) {
    let cmd = popup_menu(
        hwnd,
        pt,
        &[
            (1, if collapsed { "펼치기\t→" } else { "접기\t←" }),
            (0, ""),
            (2, "이름 바꾸기\tF2"),
            (3, "위로 이동\tCtrl+↑"),
            (4, "아래로 이동\tCtrl+↓"),
            (0, ""),
            (5, "그룹 풀기 (안의 즐겨찾기는 남김)\tDel"),
            (6, "그룹과 안의 즐겨찾기 빼기…"),
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
        1 => a.fav_set_collapsed(g, !collapsed),
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

/// 이 공간에 고정한 줄의 오른쪽 버튼 메뉴
pub(super) fn show_pin_menu(hwnd: HWND, index: usize, pt: POINT) {
    let Some((is_dir, count)) = with_app(|a| a.pins().get(index).map(|f| (f.is_dir, a.pins().len()))).flatten() else { return };
    let mut items: Vec<(u32, &str)> = vec![(1, "열기"), (2, if is_dir { "새 탭에서 열기" } else { "위치 열기 (새 탭)" }), (0, ""), (7, "이름 바꾸기\tF2")];
    if index > 0 {
        items.push((3, "위로 이동"));
    }
    if index + 1 < count {
        items.push((4, "아래로 이동"));
    }
    items.extend([(5, "모든 공간에 두기"), (0, ""), (6, "고정 풀기")]);
    let cmd = popup_menu(hwnd, pt, &items);
    with_app(|a| match cmd {
        1 | 2 => a.pin_open(index, cmd == 2),
        3 => a.pin_nudge(index, true),
        4 => a.pin_nudge(index, false),
        5 => a.pin_to_common(index),
        6 => a.unpin(index),
        7 => a.begin_pin_rename(index),
        _ => {}
    });
}

impl App {
    // ---------- 키보드 자리, 고르기 ----------

    /// Ctrl+B: 키보드를 사이드바 즐겨찾기로 옮기고, 이미 거기 있으면 칸으로 돌아간다
    pub(super) fn toggle_fav_focus(&mut self) {
        if self.fav_focus {
            self.fav_blur();
        } else {
            self.focus_favorites();
        }
    }

    /// 키보드를 즐겨찾기 목록으로 옮긴다 (패널이 안 보이거나 떠 있으면 고정해서 열고, 고른 줄이 보이게)
    fn focus_favorites(&mut self) {
        self.close_palette();
        self.close_view_menu();
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        if self.side_panel() != Some(Panel::Main) || self.side_floating() {
            self.show_side(Panel::Main, true);
        }
        self.fav_focus = true;
        self.fav_sel = self.fav_sel.min(self.favs.rows().len().saturating_sub(1));
        // 이 작업 공간에서 처음 들어오면 맨 위 줄(고정한 곳이 있으면 그 첫 줄)에서 시작한다. 그 뒤로는 있던 자리로 돌아온다
        if !std::mem::replace(&mut self.side_kb_seen, true) && !self.pins().is_empty() {
            self.top_sel = Some(TopSel::Pin(0));
        }
        match self.top_sel_now() {
            Some(top) => self.set_top(top),
            None => self.fav_ensure_visible(),
        }
        self.focus();
        self.invalidate();
    }

    /// 키보드가 위쪽 줄(고정한 곳·내 PC)에 있으면 그 줄. 고정한 것이 줄었으면 남은 줄로 맞추고,
    /// 즐겨찾기가 하나도 없으면 고를 줄은 위쪽뿐이라 내 PC로 본다
    pub(super) fn top_sel_now(&self) -> Option<TopSel> {
        let pins = self.pins().len();
        match self.top_sel {
            Some(TopSel::Pin(_)) if pins == 0 => Some(TopSel::Computer),
            Some(TopSel::Pin(i)) => Some(TopSel::Pin(i.min(pins - 1))),
            Some(TopSel::Computer) => Some(TopSel::Computer),
            None => self.favs.rows().is_empty().then_some(TopSel::Computer),
        }
    }

    /// 키보드를 위쪽 줄로 옮기고 그 줄이 보이게 한다
    fn set_top(&mut self, top: TopSel) {
        self.top_sel = Some(top);
        let side = match top {
            TopSel::Pin(i) => Side::Pin(i),
            TopSel::Computer => Side::Computer,
        };
        if let Some(r) = self.side_item_rect(side) {
            self.side_reveal(r);
        }
        self.invalidate();
    }

    /// 키보드가 위쪽 줄(고정한 곳·내 PC)에 있을 때의 키. 아래로 내려가면 즐겨찾기 첫 줄로 넘어간다
    fn top_key(&mut self, top: TopSel, vk: VIRTUAL_KEY) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let pins = self.pins().len();
        let first = if pins > 0 { TopSel::Pin(0) } else { TopSel::Computer };
        // 즐겨찾기 쪽으로 넘어간다 (즐겨찾기가 없으면 그대로)
        let to_favs = |a: &mut App, last: bool| {
            let rows = a.favs.rows();
            if let Some(row) = if last { rows.last() } else { rows.first() } {
                a.fav_select(*row);
            }
        };
        let menu = vk == VK_APPS || (vk == VK_F10 && shift);
        match (vk, top) {
            (VK_ESCAPE | VK_TAB, _) => self.fav_blur(),
            // Ctrl+↑↓: 고정한 곳의 순서를 바꾼다
            (VK_UP, TopSel::Pin(i)) if ctrl => {
                if i > 0 {
                    self.pin_nudge(i, true);
                    self.set_top(TopSel::Pin(i - 1));
                }
            }
            (VK_DOWN, TopSel::Pin(i)) if ctrl => {
                if i + 1 < pins {
                    self.pin_nudge(i, false);
                    self.set_top(TopSel::Pin(i + 1));
                }
            }
            (VK_UP | VK_DOWN, _) if ctrl => {}
            (VK_UP, TopSel::Pin(i)) => self.set_top(TopSel::Pin(i.saturating_sub(1))),
            (VK_UP, TopSel::Computer) => {
                if pins > 0 {
                    self.set_top(TopSel::Pin(pins - 1));
                }
            }
            (VK_DOWN, TopSel::Pin(i)) => self.set_top(if i + 1 < pins { TopSel::Pin(i + 1) } else { TopSel::Computer }),
            (VK_DOWN | VK_NEXT, TopSel::Computer) => to_favs(self, false),
            (VK_NEXT, TopSel::Pin(_)) => self.set_top(TopSel::Computer),
            (VK_HOME | VK_PRIOR, _) => self.set_top(first),
            (VK_END, _) => to_favs(self, true),
            (VK_RETURN, _) => {
                let side = match top {
                    TopSel::Pin(i) => Side::Pin(i),
                    TopSel::Computer => Side::Computer,
                };
                self.side_open(side, ctrl);
                return true;
            }
            (VK_F2, TopSel::Pin(i)) => {
                self.begin_pin_rename(i);
                return true;
            }
            // Del: 고정을 푼다 (내 PC는 늘 있다)
            (VK_DELETE, TopSel::Pin(i)) => self.unpin(i),
            (_, TopSel::Pin(i)) if menu => {
                if let Some(r) = self.side_item_rect(Side::Pin(i)) {
                    let pt = self.to_screen(r.l + 24.0, r.b);
                    self.pending.push(Deferred::PinMenu { index: i, pt });
                }
            }
            (_, TopSel::Computer) if menu => {
                if let Some(r) = self.side_item_rect(Side::Computer) {
                    let pt = self.to_screen(r.l + 24.0, r.b);
                    self.pending.push(Deferred::SideMenu { side: Side::Computer, pt });
                }
            }
            _ => return self.fav_shared_key(vk),
        }
        self.invalidate();
        true
    }

    /// 키보드를 칸으로 돌려준다
    pub(super) fn fav_blur(&mut self) {
        if !self.fav_focus {
            return;
        }
        self.fav_focus = false;
        self.focus();
        self.invalidate();
    }

    /// 그 줄을 고르고 보이게 한다. 접힌 그룹 안 항목이면 그 그룹을 펼친다
    pub(super) fn fav_select(&mut self, row: Row) {
        self.top_sel = None;
        if let Row::Item { group: Some(g), .. } = row {
            if self.favs.set_collapsed(g, false) {
                self.save_favorites();
            }
        }
        if let Some(i) = self.favs.row_index(row) {
            self.fav_sel = i;
        }
        self.fav_ensure_visible();
        self.invalidate();
    }

    fn fav_ensure_visible(&mut self) {
        let Some(row) = self.favs.rows().get(self.fav_sel).copied() else { return };
        if let Some(r) = self.fav_row_rect(row) {
            self.side_reveal(r);
        }
    }

    /// 그룹을 접거나 펼친다. 접은 그룹 안을 고르고 있었으면 그 그룹 줄로 옮긴다
    pub(super) fn fav_set_collapsed(&mut self, g: usize, collapsed: bool) {
        let cur = self.favs.rows().get(self.fav_sel).copied();
        if !self.favs.set_collapsed(g, collapsed) {
            return;
        }
        if matches!(self.edit_mode, EditMode::FavRename(Row::Item { group: Some(h), .. }) if h == g) {
            self.cancel_edit();
        }
        let keep = match cur {
            Some(Row::Item { group: Some(h), .. }) if h == g && collapsed => Some(Row::Group(g)),
            other => other,
        };
        let n = self.favs.rows().len();
        self.fav_sel = keep.and_then(|r| self.favs.row_index(r)).unwrap_or(self.fav_sel).min(n.saturating_sub(1));
        self.save_favorites();
        self.side_clamp_scroll();
        self.refresh_hover();
        self.layout_edits();
        self.invalidate();
    }

    /// 줄이 밀렸으니 마우스 아래 것을 다시 본다 (움직이기 전까지 엉뚱한 줄이 강조되지 않게).
    /// 마우스가 다른 창 위에 있으면 아무것도 가리키지 않는다 (겹친 창 아래 좌표를 우리 줄로 착각하지 않게)
    pub(super) fn refresh_hover(&mut self) {
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() || unsafe { WindowFromPoint(pt) } != self.hwnd {
            self.hover = Hit::None;
            return;
        }
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }
        let s = self.scale();
        self.hover = self.hit(pt.x as f32 / s, pt.y as f32 / s);
    }

    // ---------- 열기·추가·편집 ----------

    /// 줄을 고르고 보이게 한 뒤 저장한다 (항목을 옮기거나 그룹을 바꾼 다음)
    fn fav_changed(&mut self, select: Option<Row>) {
        self.fav_flash = None;
        self.fav_sel = self.fav_sel.min(self.favs.rows().len().saturating_sub(1));
        if let Some(row) = select {
            self.fav_select(row);
        }
        self.side_clamp_scroll();
        self.save_favorites();
        self.refresh_hover();
        self.layout_edits();
        self.invalidate();
    }

    /// 항목은 열고(폴더는 이동, 파일은 실행), 그룹은 접거나 펼친다.
    /// new_tab: 폴더는 새 탭, 파일은 그 파일이 있는 폴더를 새 탭에서 열고 선택
    pub(super) fn fav_open_row(&mut self, row: Row, new_tab: bool) {
        match row {
            Row::Group(g) => {
                let collapsed = self.favs.groups.get(g).is_some_and(|g| g.collapsed);
                self.fav_set_collapsed(g, !collapsed);
            }
            Row::Item { group, index } => self.fav_open_entry(group, index, new_tab),
        }
    }

    pub(super) fn fav_open_entry(&mut self, group: Option<usize>, index: usize, new_tab: bool) {
        let Some(f) = self.favs.item(group, index).cloned() else { return };
        self.open_favorite(&f, new_tab);
    }

    /// 즐겨찾기나 고정한 것을 연다
    fn open_favorite(&mut self, f: &Favorite, new_tab: bool) {
        self.fav_blur();
        // 떠 있는 패널은 위치를 골랐으니 닫는다
        self.close_peek();
        let p = self.active;
        // 있는지·폴더인지는 뒤에서 확인한다 (꺼진 NAS 즐겨찾기를 눌러도 창이 멈추지 않게)
        let tab = self.panes[p].tab();
        let then = ProbeThen::Favorite { tab: tab.id, from: tab.loc.clone(), new_tab, is_dir: Some(f.is_dir) };
        self.probe(p, util::normalize(&f.path), then);
    }

    /// 고정하거나 즐겨찾기에 넣을 것: 폴더·파일 하나를 선택했으면 그것, 아니면 현재 폴더 (경로, 정해 둔 표시 이름)
    fn favorite_target(&self) -> Option<(String, Option<String>)> {
        let tab = self.panes[self.active].tab();
        let picked = match tab.cursor_entry() {
            Some(e) if e.selected && tab.selection_stats().0 == 1 => {
                // 드라이브·사용자 폴더는 "로컬 디스크 (C:)" 같은 표시 이름을 그대로 쓴다
                tab.path_of(e).map(|p| (p, matches!(e.kind, Kind::Drive | Kind::Known).then(|| e.name_string())))
            }
            _ => None,
        };
        picked.or_else(|| tab.loc.dir().map(|d| (d.to_string(), None)))
    }

    /// '모든 공간' 줄의 추가 단추 (키보드가 즐겨찾기 목록에 있을 때의 Ctrl+D도): 즐겨찾기에 넣는다
    pub(super) fn add_favorite(&mut self) {
        match self.favorite_target() {
            Some((path, name)) => self.add_favorite_path(&path, name),
            None => {
                let p = self.active;
                self.flash(p, "모든 공간에 넣을 폴더나 파일이 없어요");
            }
        }
    }

    // ---------- 이 공간에 고정한 것 ----------

    /// 고정한 것이 바뀐 뒤: 작업 공간 파일에 저장하고 줄 자리를 다시 맞춘다
    fn pins_changed(&mut self) {
        self.save_workspaces();
        self.side_clamp_scroll();
        self.refresh_hover();
        self.layout_edits();
        self.invalidate();
    }

    /// 고정한 것을 연다 (즐겨찾기와 같다: 폴더는 이동, 파일은 실행. new_tab이면 새 탭)
    pub(super) fn pin_open(&mut self, i: usize, new_tab: bool) {
        let Some(f) = self.pins().get(i).cloned() else { return };
        self.open_favorite(&f, new_tab);
    }

    /// Ctrl+D: 폴더·파일 하나를 선택했으면 그것, 아니면 현재 폴더를 지금 작업 공간에 고정한다
    pub(super) fn add_pin(&mut self) {
        let p = self.active;
        let Some((path, name)) = self.favorite_target() else {
            return self.flash(p, "고정할 폴더나 파일이 없어요");
        };
        let path = util::normalize(&path);
        let is_dir = shell::is_dir(&path);
        let Some(ws) = self.workspaces.current_mut() else { return };
        if ws.pin_index(&path).is_some() {
            return self.flash(p, "이미 이 공간에 고정돼 있어요");
        }
        let i = ws.pin(&path, is_dir);
        if let Some(name) = name {
            ws.pins[i].name = name;
        }
        self.pins_changed();
        self.flash(p, "이 공간에 고정했어요");
    }

    /// 파일 목록에서 끌어 사이드바의 고정 목록(또는 레일의 지금 공간 아이콘)에 놓은 항목들 (이미 있는 것은 건너뛴다)
    pub fn pin_paths(&mut self, paths: &[String]) {
        let p = self.active;
        let mut added = 0;
        for path in paths {
            let path = util::normalize(path);
            let is_dir = shell::is_dir(&path);
            if let Some(ws) = self.workspaces.current_mut().filter(|ws| ws.pin_index(&path).is_none()) {
                ws.pin(&path, is_dir);
                added += 1;
            }
        }
        if added == 0 {
            return self.flash(p, "이미 이 공간에 고정돼 있어요");
        }
        self.pins_changed();
        self.flash(p, format!("이 공간에 {added}개를 고정했어요"));
    }

    /// 고정한 줄의 표시 이름을 그 자리에서 고친다 (즐겨찾기 줄과 같다. 폴더·파일의 이름은 바뀌지 않는다)
    pub(super) fn begin_pin_rename(&mut self, i: usize) {
        let Some(name) = self.pins().get(i).map(|f| f.name.clone()) else { return };
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        self.close_palette();
        // 입력 상자는 작업 공간 패널이 고정돼 있어야 자리가 있다 (떠 있는 패널은 마우스가 떠나면 닫힌다)
        if self.side_panel() != Some(Panel::Main) || self.side_floating() {
            self.show_side(Panel::Main, true);
            self.side_anim = None;
        }
        self.set_top(TopSel::Pin(i));
        self.edit_mode = EditMode::PinRename(i);
        unsafe {
            let _ = SetWindowTextW(self.rename_edit, &HSTRING::from(name));
            SendMessageW(self.rename_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    pub(super) fn commit_pin_rename(&mut self, i: usize) {
        let text = favorites::clean_name(&edit_text(self.rename_edit));
        self.close_edit();
        let Some(f) = self.workspaces.current_mut().and_then(|ws| ws.pins.get_mut(i)).filter(|f| !text.is_empty() && f.name != text) else { return };
        f.name = text;
        self.pins_changed();
    }

    /// 고정한 줄이 바뀌기 전에: 그 줄의 이름을 고치는 중이면 그만둔다 (줄 번호가 달라진다)
    fn end_pin_rename(&mut self) {
        if let EditMode::PinRename(_) = self.edit_mode {
            self.cancel_edit();
        }
    }

    pub(super) fn unpin(&mut self, i: usize) {
        self.end_pin_rename();
        let Some(ws) = self.workspaces.current_mut().filter(|ws| i < ws.pins.len()) else { return };
        ws.pins.remove(i);
        self.pins_changed();
    }

    /// 고정한 것을 한 줄 위·아래로
    pub(super) fn pin_nudge(&mut self, i: usize, up: bool) {
        self.end_pin_rename();
        let j = if up { i.wrapping_sub(1) } else { i + 1 };
        let Some(ws) = self.workspaces.current_mut().filter(|ws| i < ws.pins.len() && j < ws.pins.len()) else { return };
        ws.pins.swap(i, j);
        self.pins_changed();
    }

    /// 고정한 것을 '모든 공간'(즐겨찾기)으로 옮긴다: 어느 작업 공간에서나 보이게.
    /// 즐겨찾기 파일에 먼저 저장하고, 저장됐을 때만 이 공간에서 뺀다 (저장하지 못했는데 빼면 다음에 열 때 양쪽 모두에서 사라진다)
    pub(super) fn pin_to_common(&mut self, i: usize) {
        self.end_pin_rename();
        let Some(f) = self.pins().get(i).cloned() else { return };
        if self.favs.find(&f.path).is_none() {
            let before = self.favs.clone();
            self.favs.push(None, f);
            if let Err(e) = favorites::save(&self.favs) {
                self.favs = before;
                let p = self.active;
                return self.flash(p, format!("즐겨찾기를 저장하지 못해 옮기지 않았어요: {e}"));
            }
        }
        self.unpin(i);
    }

    /// 즐겨찾기 항목을 '모든 공간'에서 빼서 지금 작업 공간에만 고정한다.
    /// 작업 공간 파일에 먼저 저장하고, 저장됐을 때만 즐겨찾기에서 뺀다 (pin_to_common과 같은 이유)
    pub(super) fn fav_move_to_pins(&mut self, row: Row) {
        let Row::Item { group, index } = row else { return };
        let Some(f) = self.favs.item(group, index).cloned() else { return };
        if let EditMode::FavRename(_) = self.edit_mode {
            self.cancel_edit();
        }
        let Some(ws) = self.workspaces.current_mut() else { return };
        if ws.pin_index(&f.path).is_none() {
            ws.pins.push(f);
            if let Err(e) = self.try_save_workspaces() {
                if let Some(ws) = self.workspaces.current_mut() {
                    ws.pins.pop();
                }
                let p = self.active;
                return self.flash(p, format!("작업 공간을 저장하지 못해 옮기지 않았어요: {e}"));
            }
        }
        self.favs.remove_item(group, index);
        self.fav_changed(None);
        self.pins_changed();
    }

    /// 키보드가 즐겨찾기에 있고 그룹(또는 그 안 항목)을 고르고 있으면 그 그룹에, 아니면 그룹 없음 끝에 넣고 이름을 바로 고칠 수 있게 한다
    pub fn add_favorite_path(&mut self, path: &str, name: Option<String>) {
        let path = util::normalize(path);
        if let Some((group, index)) = self.favs.find(&path) {
            // 어느 줄인지 보이게 그 줄을 펼쳐 보이고 잠깐 강조한다. 키보드는 옮기지 않는다
            // (이어서 누른 Delete가 칸의 파일 대신 즐겨찾기를 지우지 않게)
            let row = Row::Item { group, index };
            self.fav_select(row);
            self.fav_flash = Some(row);
            unsafe { SetTimer(Some(self.hwnd), super::TIMER_FAV_FLASH, 1500, None) };
            let p = self.active;
            self.flash(p, "이미 즐겨찾기에 있어요");
            return;
        }
        let target = if self.fav_focus {
            match self.favs.rows().get(self.fav_sel) {
                Some(Row::Group(g) | Row::Item { group: Some(g), .. }) => Some(*g),
                _ => None,
            }
        } else {
            None
        };
        let name = name.unwrap_or_else(|| favorites::default_name(&path));
        let is_dir = shell::is_dir(&path);
        let (group, index) = self.favs.push(target, Favorite { name, path, is_dir });
        self.save_favorites();
        // 추가하자마자 원하는 표시 이름(예: "01.일간 업무")을 붙일 수 있게 한다. Enter면 그대로
        self.begin_fav_rename(Row::Item { group, index });
    }

    /// 파일 목록에서 끌어 사이드바 즐겨찾기에 놓은 항목들 (이미 있는 것은 건너뛴다)
    pub fn add_favorite_paths(&mut self, paths: &[String]) {
        let before = self.favs.clone();
        let mut added = 0;
        let mut last = None;
        for path in paths {
            let path = util::normalize(path);
            if self.favs.find(&path).is_some() {
                continue;
            }
            let is_dir = shell::is_dir(&path);
            let (group, index) = self.favs.push(None, Favorite { name: favorites::default_name(&path), path, is_dir });
            last = Some(Row::Item { group, index });
            added += 1;
        }
        let p = self.active;
        let Some(row) = last else {
            self.flash(p, "이미 즐겨찾기에 있어요");
            return;
        };
        match favorites::save(&self.favs) {
            Ok(()) => {
                self.fav_select(row);
                self.side_clamp_scroll();
                self.refresh_hover();
                self.layout_edits();
                self.flash(p, format!("즐겨찾기에 {added}개를 넣었어요"));
            }
            Err(e) => {
                self.favs = before;
                self.fav_sel = self.fav_sel.min(self.favs.rows().len().saturating_sub(1));
                self.side_clamp_scroll();
                self.flash(p, format!("즐겨찾기를 저장하지 못해 넣지 않았어요: {e}"));
            }
        }
        self.invalidate();
    }

    /// 경로 오른쪽 책갈피: 이 폴더를 지금 작업 공간에 고정하거나 푼다.
    /// 고정하지는 않았지만 '모든 공간'(즐겨찾기)에 있는 폴더면 거기서 뺀다 (켜져 보이는 책갈피를 끄는 동작)
    pub(super) fn toggle_star(&mut self, p: usize) {
        let Some(dir) = self.panes[p].tab().loc.dir().map(util::normalize) else { return };
        if let Some(i) = self.workspaces.current().and_then(|w| w.pin_index(&dir)) {
            self.unpin(i);
            return self.flash(p, "이 공간에서 고정을 풀었어요");
        }
        let Some((group, index)) = self.favs.find(&dir) else {
            if let Some(ws) = self.workspaces.current_mut() {
                ws.pin(&dir, true);
            }
            self.pins_changed();
            return self.flash(p, "이 공간에 고정했어요 · 사이드바에서 우클릭하면 모든 공간에 둘 수 있어요");
        };
        let before = self.favs.clone();
        self.favs.remove_item(group, index);
        // 저장하지 못했으면 책갈피 표시도 원래대로 두고 오류만 보인다 (성공 문구로 덮지 않게)
        match favorites::save(&self.favs) {
            Ok(()) => {
                self.fav_changed(None);
                self.flash(p, "모든 공간에서 뺐어요");
            }
            Err(e) => {
                self.favs = before;
                self.flash(p, format!("즐겨찾기를 저장하지 못해 바꾸지 않았어요: {e}"));
            }
        }
    }

    fn save_favorites(&mut self) {
        if let Err(e) = favorites::save(&self.favs) {
            let p = self.active;
            self.flash(p, format!("즐겨찾기를 저장하지 못했어요: {e}"));
        }
    }

    pub(super) fn add_fav_group(&mut self) -> usize {
        let g = self.favs.add_group("새 그룹".to_string());
        self.save_favorites();
        self.begin_fav_rename(Row::Group(g));
        g
    }

    pub(super) fn begin_fav_rename(&mut self, row: Row) {
        let Some(name) = self.favs.row_name(row).map(str::to_string) else { return };
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        self.close_palette();
        // 입력 상자는 작업 공간 패널이 고정돼 있어야 자리가 있다 (떠 있는 패널은 마우스가 떠나면 닫힌다)
        if self.side_panel() != Some(Panel::Main) || self.side_floating() {
            self.show_side(Panel::Main, true);
            // 입력 상자는 곧바로 제자리에 놓이니 패널은 펼치지 않고 바로 연다
            self.side_anim = None;
        }
        self.fav_select(row);
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
            Row::Group(g) => self.favs.ungroup(g),
        }
        self.fav_changed(None);
    }

    pub(super) fn fav_ungroup(&mut self, g: usize) {
        self.favs.ungroup(g);
        self.fav_changed(None);
    }

    pub(super) fn fav_delete_group(&mut self, g: usize) {
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
        if let Some(moved) = self.favs.nudge(row, up) {
            self.fav_changed(Some(moved));
        }
    }

    pub(super) fn sort_favorites(&mut self, by: SortBy) {
        self.favs.sort(by);
        self.fav_changed(None);
    }

    // ---------- 키보드 ----------

    /// 키보드가 즐겨찾기에 있을 때의 키. 처리했으면 true.
    /// ↑↓로 고르고, Enter는 항목을 열거나 그룹을 접고 펼친다. →·←는 그룹 펼치기·접기, Esc·Tab은 칸으로
    pub(super) fn fav_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        // 키보드가 위쪽 줄(고정한 곳·내 PC)에 있으면 그쪽 키로
        if let Some(top) = self.top_sel_now() {
            return self.top_key(top, vk);
        }
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let rows = self.favs.rows();
        let n = rows.len();
        let s = self.fav_sel.min(n.saturating_sub(1));
        let page = ((self.side_list().h() / SIDE_ROW_H).floor() as usize).max(1);
        let go = |a: &mut App, i: usize| {
            if let Some(row) = rows.get(i) {
                a.fav_select(*row);
            }
        };
        match vk {
            VK_ESCAPE | VK_TAB => self.fav_blur(),
            VK_UP if ctrl => self.fav_nudge_at(s, true),
            VK_DOWN if ctrl => self.fav_nudge_at(s, false),
            // 첫 즐겨찾기 줄에서 위로 가면 그 위의 내 PC, 고정한 곳으로 이어진다
            VK_UP | VK_PRIOR if s == 0 => self.set_top(TopSel::Computer),
            VK_UP => go(self, s.saturating_sub(1)),
            VK_DOWN => go(self, (s + 1).min(n.saturating_sub(1))),
            VK_PRIOR => go(self, s.saturating_sub(page)),
            VK_NEXT => go(self, (s + page).min(n.saturating_sub(1))),
            VK_HOME => self.set_top(if self.pins().is_empty() { TopSel::Computer } else { TopSel::Pin(0) }),
            VK_END => go(self, n.saturating_sub(1)),
            // →: 접힌 그룹은 펼치고, 펼친 그룹은 첫 항목으로
            VK_RIGHT => {
                if let Some(Row::Group(g)) = rows.get(s) {
                    if self.favs.groups.get(*g).is_some_and(|g| g.collapsed) {
                        self.fav_set_collapsed(*g, false);
                    } else if matches!(rows.get(s + 1), Some(Row::Item { group: Some(h), .. }) if h == g) {
                        go(self, s + 1);
                    }
                }
            }
            // ←: 그룹 안 항목은 그룹 줄로, 펼친 그룹은 접는다
            VK_LEFT => match rows.get(s) {
                Some(Row::Item { group: Some(g), .. }) => self.fav_select(Row::Group(*g)),
                Some(Row::Group(g)) => self.fav_set_collapsed(*g, true),
                _ => {}
            },
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
            VK_APPS => self.fav_menu_at_sel(),
            VK_F10 if shift => self.fav_menu_at_sel(),
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

    /// 메뉴 키: 고른 줄 아래에 그 줄의 메뉴
    fn fav_menu_at_sel(&mut self) {
        let Some(row) = self.favs.rows().get(self.fav_sel).copied() else { return };
        let Some(r) = self.fav_row_rect(row) else { return };
        let pt = self.to_screen(r.l + 24.0, r.b);
        self.pending.push(Deferred::FavMenu { row, pt });
    }

    /// 단축키: 즐겨찾기 단축키는 칸으로 돌아가고, 추가는 키보드가 있는 쪽에 넣고(고정한 곳이면 이 공간에 고정, 즐겨찾기면 고른 그룹에),
    /// 그 밖의 동작은 칸으로 돌아가 실행한다
    fn fav_shared_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        use crate::keys::Action;
        match self.keys.find(crate::keys::Combo::now(vk.0)) {
            Some(Action::Favorites) => self.fav_blur(),
            Some(Action::AddFavorite) if matches!(self.top_sel_now(), Some(TopSel::Pin(_))) => self.add_pin(),
            Some(Action::AddFavorite) => self.add_favorite(),
            Some(a) => {
                self.fav_blur();
                self.run_action(a);
            }
            None => return false,
        }
        self.invalidate();
        true
    }

    /// 키보드가 즐겨찾기에 있을 때 글자를 누르면, 그 글자로 시작하는 다음 줄로 이동
    pub fn fav_type_char(&mut self, ch: u16) -> bool {
        if !self.fav_focus || self.edit_mode != EditMode::None {
            return false;
        }
        let Some(c) = char::from_u32(ch as u32).filter(|c| !c.is_control()) else { return true };
        let needle = c.to_lowercase().to_string();
        let starts = |name: &str| name.to_lowercase().starts_with(&needle);
        let rows = self.favs.rows();
        let n = rows.len();
        if let Some(i) = (1..=n).map(|k| (self.fav_sel + k) % n).find(|&i| self.favs.row_name(rows[i]).is_some_and(starts)) {
            self.fav_select(rows[i]);
        }
        self.invalidate();
        true
    }

    // ---------- 누르기·끌기 ----------

    /// 즐겨찾기 줄을 눌렀다. 떼면 열고(그룹은 접고 펼치고), 끌면 순서를 바꾼다
    pub(super) fn fav_mouse_down(&mut self, row: Row, y: f32, dbl: bool) {
        if let Some(i) = self.favs.row_index(row) {
            self.fav_sel = i;
            self.top_sel = None;
        }
        // 두 번째 누름은 무시한다 (첫 누름에서 이미 열었다: 파일을 두 번 실행하지 않게)
        if dbl {
            return;
        }
        self.drag = Drag::FavPress { row, y };
        unsafe { SetCapture(self.hwnd) };
    }

    pub(super) fn fav_drag_move(&mut self, y: f32) {
        self.drag = match self.drag {
            Drag::FavPress { row, y: y0 } if (y - y0).abs() > 4.0 => Drag::FavMove { from: row, y },
            Drag::FavMove { from, .. } => Drag::FavMove { from, y },
            _ => return,
        };
        // 목록 끝에 가까이 끌면 자동 스크롤
        let list = self.side_list();
        if y < list.t + 12.0 {
            self.side_scroll -= 8.0;
        } else if y > list.b - 12.0 {
            self.side_scroll += 8.0;
        }
        self.side_clamp_scroll();
        self.invalidate();
    }

    /// 처리했으면 true
    pub(super) fn fav_mouse_up(&mut self, ctrl: bool) -> bool {
        match self.drag {
            Drag::FavPress { row, .. } => {
                self.drag = Drag::None;
                self.release_capture();
                self.fav_open_row(row, ctrl);
            }
            Drag::FavMove { from, y } => {
                self.drag = Drag::None;
                self.release_capture();
                let Some((at, below)) = self.fav_drop_row(y) else { return true };
                match from {
                    // 항목은 그룹 머리 줄 위에 놓으면 그 그룹으로, 항목 줄이면 그 위·아래 자리로
                    Row::Item { group, index } => {
                        let (to_group, to_index) = self.favs.item_drop_target(at, below);
                        if let Some((group, index)) = self.favs.move_item((group, index), to_group, to_index) {
                            self.fav_changed(Some(Row::Item { group, index }));
                        }
                    }
                    // 그룹은 그룹끼리 순서를 바꾼다
                    Row::Group(g) => {
                        let to = self.favs.group_drop_target(at, below, g);
                        if let Some(to) = self.favs.move_group(g, to) {
                            self.fav_changed(Some(Row::Group(to)));
                        }
                    }
                }
            }
            _ => return false,
        }
        self.invalidate();
        true
    }

    fn release_capture(&self) {
        unsafe {
            let _ = ReleaseCapture();
        }
    }
}
