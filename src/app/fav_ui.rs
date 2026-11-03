//! 즐겨찾기 팝업. 도구 막대 버튼이나 Ctrl+B로 열고, 표시 이름·순서는 사용자가 정한다

use super::*;
use crate::favorites::{self, Favorite, SortBy};

const FAV_W: f32 = 480.0;
const FAV_HEAD_H: f32 = 46.0;
const FAV_FOOT_H: f32 = 30.0;
const FAV_ROW_H: f32 = 32.0;
pub(super) const G_STAR: u16 = 0xE734;
const G_EDIT: u16 = 0xE70F;
const G_REMOVE: u16 = 0xE711;
const G_SORT: u16 = 0xE8CB;

pub(super) fn fav_button() -> R {
    let l = hidden_button().r + 6.0;
    R::new(l, 8.0, l + 100.0, 38.0)
}

/// 네이티브 팝업 메뉴. id 0은 구분선. 고른 id(없으면 0)
fn popup_menu(hwnd: HWND, pt: POINT, items: &[(u32, &str)]) -> u32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        for (id, label) in items {
            let _ = if *id == 0 {
                AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())
            } else {
                AppendMenuW(menu, MF_STRING, *id as usize, &HSTRING::from(*label))
            };
        }
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;
        let _ = DestroyMenu(menu);
        cmd
    }
}

pub(super) fn show_fav_menu(hwnd: HWND, index: usize, pt: POINT) {
    let cmd = popup_menu(
        hwnd,
        pt,
        &[(1, "열기"), (2, "새 탭에서 열기"), (0, ""), (3, "이름 바꾸기\tF2"), (4, "위로 이동\tCtrl+↑"), (5, "아래로 이동\tCtrl+↓"), (0, ""), (6, "즐겨찾기에서 삭제\tDel")],
    );
    with_app(|a| match cmd {
        1 | 2 => a.fav_open_item(index, cmd == 2),
        3 => a.begin_fav_rename(index),
        4 if index > 0 => a.move_favorite(index, index - 1),
        5 => a.move_favorite(index, index + 1),
        6 => a.remove_favorite(index),
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
        let b = fav_button();
        let w = FAV_W.min(self.w - b.l - MARGIN).max(260.0);
        let rows = self.favs.len().max(2) as f32;
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

    fn fav_row_buttons(row: R) -> (R, R) {
        let remove = R::new(row.r - 32.0, row.t + 4.0, row.r - 6.0, row.b - 4.0);
        let edit = R::new(remove.l - 28.0, remove.t, remove.l - 2.0, remove.b);
        (edit, remove)
    }

    /// 이름 바꾸기 입력 상자 자리 (표시 이름 칸)
    pub(super) fn fav_rename_rect(&self, i: usize) -> Option<R> {
        let row = self.fav_row(i);
        let list = self.fav_list();
        if row.t < list.t - 1.0 || row.b > list.b + 1.0 {
            return None;
        }
        Some(R::new(row.l + 36.0, row.t + 3.0, row.l + 36.0 + (row.w() - 36.0) * 0.5, row.b - 3.0))
    }

    pub(super) fn fav_hit(&self, x: f32, y: f32) -> Option<Hit> {
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
        let list = self.fav_list();
        if list.contains(x, y) {
            let i = ((y - list.t + self.fav_scroll) / FAV_ROW_H).floor();
            if i >= 0.0 && (i as usize) < self.favs.len() {
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
        self.fav_sel = select.unwrap_or(self.fav_sel).min(self.favs.len().saturating_sub(1));
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
        if matches!(self.drag, Drag::FavPress { .. } | Drag::FavMove { .. }) {
            self.drag = Drag::None;
        }
        self.layout_edits();
        self.invalidate();
    }

    fn fav_clamp_scroll(&mut self) {
        let list = self.fav_list();
        let max = (self.favs.len() as f32 * FAV_ROW_H - list.h()).max(0.0);
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

    pub(super) fn fav_open_item(&mut self, i: usize, new_tab: bool) {
        let Some(f) = self.favs.get(i).cloned() else { return };
        self.close_favorites();
        let path = util::normalize(&f.path);
        let p = self.active;
        if !shell::is_dir(&path) {
            self.flash(p, format!("즐겨찾기 폴더를 찾을 수 없습니다: {}", f.path));
            return;
        }
        if new_tab {
            self.new_tab(p, Some(Location::Dir(path)), true);
        } else {
            self.navigate(p, Location::Dir(path), None);
        }
    }

    /// Ctrl+D·추가 버튼: 폴더 하나를 선택했으면 그 폴더, 아니면 현재 폴더
    pub(super) fn add_favorite(&mut self) {
        let tab = self.panes[self.active].tab();
        let picked = match tab.cursor_entry() {
            Some(e) if e.selected && e.is_dir() && tab.selection_stats().0 == 1 => {
                // 드라이브·사용자 폴더는 "로컬 디스크 (C:)" 같은 표시 이름을 그대로 쓴다
                tab.path_of(e).map(|p| (p, (e.kind != Kind::Dir).then(|| e.name_string())))
            }
            _ => None,
        };
        match picked.or_else(|| tab.loc.dir().map(|d| (d.to_string(), None))) {
            Some((path, name)) => self.add_favorite_path(&path, name),
            None => {
                let p = self.active;
                self.flash(p, "즐겨찾기에 추가할 폴더가 없습니다");
            }
        }
    }

    pub fn add_favorite_path(&mut self, path: &str, name: Option<String>) {
        let path = util::normalize(path);
        if let Some(i) = favorites::index_of(&self.favs, &path) {
            self.open_favorites(Some(i));
            let p = self.active;
            self.flash(p, "이미 즐겨찾기에 있습니다");
            return;
        }
        let name = name.unwrap_or_else(|| favorites::default_name(&path));
        self.favs.push(Favorite { name, path });
        self.save_favorites();
        let i = self.favs.len() - 1;
        self.open_favorites(Some(i));
        // 추가하자마자 원하는 표시 이름(예: "01.일간 업무")을 붙일 수 있게 한다. Enter면 그대로
        self.begin_fav_rename(i);
    }

    fn save_favorites(&mut self) {
        if let Err(e) = favorites::save(&self.favs) {
            let p = self.active;
            self.flash(p, format!("즐겨찾기를 저장하지 못했습니다: {e}"));
        }
    }

    pub(super) fn begin_fav_rename(&mut self, i: usize) {
        let Some(name) = self.favs.get(i).map(|f| f.name.clone()) else { return };
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
        let changed = match self.favs.get_mut(i) {
            Some(f) if !text.is_empty() && text != f.name => {
                f.name = text;
                true
            }
            _ => false,
        };
        if changed {
            self.save_favorites();
        }
    }

    pub(super) fn remove_favorite(&mut self, i: usize) {
        if i >= self.favs.len() {
            return;
        }
        if let EditMode::FavRename(_) = self.edit_mode {
            self.cancel_edit();
        }
        self.favs.remove(i);
        self.fav_sel = self.fav_sel.min(self.favs.len().saturating_sub(1));
        self.save_favorites();
        self.fav_clamp_scroll();
        self.invalidate();
    }

    pub(super) fn move_favorite(&mut self, from: usize, to: usize) {
        if from >= self.favs.len() || to >= self.favs.len() || from == to {
            return;
        }
        favorites::move_item(&mut self.favs, from, to);
        self.fav_sel = to;
        self.save_favorites();
        self.fav_ensure_visible();
        self.invalidate();
    }

    pub(super) fn sort_favorites(&mut self, by: SortBy) {
        favorites::sort(&mut self.favs, by);
        self.save_favorites();
        self.invalidate();
    }

    /// 팝업이 열려 있을 때의 키. 처리했으면 true
    pub(super) fn fav_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let n = self.favs.len();
        let s = self.fav_sel;
        let page = ((self.fav_list().h() / FAV_ROW_H).floor() as usize).max(1);
        match vk {
            VK_ESCAPE => self.close_favorites(),
            VK_UP if ctrl => {
                if s > 0 {
                    self.move_favorite(s, s - 1);
                }
            }
            VK_DOWN if ctrl => self.move_favorite(s, s + 1),
            VK_UP => self.fav_sel = s.saturating_sub(1),
            VK_DOWN => self.fav_sel = (s + 1).min(n.saturating_sub(1)),
            VK_PRIOR => self.fav_sel = s.saturating_sub(page),
            VK_NEXT => self.fav_sel = (s + page).min(n.saturating_sub(1)),
            VK_HOME => self.fav_sel = 0,
            VK_END => self.fav_sel = n.saturating_sub(1),
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
            _ if ctrl && vk.0 == b'B' as u16 => self.close_favorites(),
            _ if ctrl && vk.0 == b'D' as u16 => self.add_favorite(),
            _ => return false,
        }
        if self.fav_open {
            self.fav_ensure_visible();
        }
        self.invalidate();
        true
    }

    /// 팝업이 열려 있을 때 글자를 누르면 그 글자로 시작하는 다음 항목으로 이동
    pub fn fav_type_char(&mut self, ch: u16) -> bool {
        if !self.fav_open || self.edit_mode != EditMode::None {
            return false;
        }
        let n = self.favs.len();
        let Some(c) = char::from_u32(ch as u32).filter(|c| !c.is_control()) else { return true };
        let needle = c.to_lowercase().to_string();
        for k in 1..=n {
            let i = (self.fav_sel + k) % n;
            if self.favs[i].name.to_lowercase().starts_with(&needle) {
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
            Hit::FavPanel => {}
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

    /// 끌어 놓을 자리(항목 사이 경계, 0..=len)
    fn fav_drop_slot(&self, y: f32) -> usize {
        let list = self.fav_list();
        let slot = ((y - list.t + self.fav_scroll) / FAV_ROW_H + 0.5).floor().max(0.0) as usize;
        slot.min(self.favs.len())
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
                let to = if slot > from { slot - 1 } else { slot };
                self.move_favorite(from, to);
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
        let (add, sort) = (self.fav_add_button(), self.fav_sort_button());
        let hover = self.hover;
        let (sel, scroll, edit_mode) = (self.fav_sel, self.fav_scroll, self.edit_mode);
        let moving = match self.drag {
            Drag::FavMove { from, y } => Some((from, self.fav_drop_slot(y))),
            _ => None,
        };
        let App { gfx, favs, types, .. } = self;
        let folder_icon = types.entry("\\dir".to_string()).or_insert_with(|| shell::type_info("", true)).1;

        // 그림자 + 본체
        let shadow = Color { r: 0.0, g: 0.0, b: 0.0, a: if t.dark { 0.45 } else { 0.12 } };
        gfx.fill_round(R::new(r.l - 2.0, r.t + 1.0, r.r + 2.0, r.b + 4.0), 12.0, shadow);
        gfx.fill_round(r, 10.0, t.pane);
        gfx.stroke_round(r.inset(0.5, 0.5), 10.0, t.border, 1.0);

        // 머리: 제목, 개수, 정렬·추가 버튼
        gfx.glyph(G_STAR, R::new(r.l + 12.0, r.t, r.l + 34.0, r.t + FAV_HEAD_H), false, t.accent);
        gfx.text("즐겨찾기", R::new(r.l + 38.0, r.t, r.l + 120.0, r.t + FAV_HEAD_H), Font::Strong, t.text);
        gfx.text(&format!("{}개", favs.len()), R::new(r.l + 104.0, r.t, sort.l - 8.0, r.t + FAV_HEAD_H), Font::Normal, t.faint);
        for (b, glyph, label, h) in [(sort, G_SORT, "정렬", Hit::FavSort), (add, G_ADD, "추가", Hit::FavAdd)] {
            gfx.fill_round(b, 6.0, if hover == h { t.sel_inactive } else { t.field });
            gfx.glyph(glyph, R::new(b.l + 6.0, b.t, b.l + 26.0, b.b), true, t.text);
            gfx.text(label, R::new(b.l + 28.0, b.t, b.r - 4.0, b.b), Font::Normal, t.text);
        }
        gfx.fill(R::new(r.l + 10.0, r.t + FAV_HEAD_H - 1.0, r.r - 10.0, r.t + FAV_HEAD_H), t.border);

        // 목록
        gfx.clip(list);
        if favs.is_empty() {
            gfx.text(
                "즐겨찾기가 비어 있습니다\nCtrl+D 또는 ‘추가’로 현재 폴더를 넣고\n원하는 이름과 순서로 정리하세요",
                list.inset(16.0, 8.0),
                Font::Hint,
                t.faint,
            );
        }
        let first = (scroll / FAV_ROW_H).floor().max(0.0) as usize;
        let count = (list.h() / FAV_ROW_H).ceil() as usize + 1;
        for i in first..(first + count).min(favs.len()) {
            let y = list.t + i as f32 * FAV_ROW_H - scroll;
            let row = R::new(list.l, y, list.r, y + FAV_ROW_H);
            let body = row.inset(2.0, 1.0);
            let row_hover = matches!(hover, Hit::FavRow(h) | Hit::FavEdit(h) | Hit::FavRemove(h) if h == i);
            let dragged = moving.is_some_and(|(from, _)| from == i);
            if i == sel {
                gfx.fill_round(body, 6.0, t.sel);
            } else if row_hover {
                gfx.fill_round(body, 6.0, t.hover);
            }
            let fg = if dragged { t.faint } else { t.text };
            gfx.icon(folder_icon, R::new(row.l + 12.0, y + 8.0, row.l + 28.0, y + 24.0));
            let split = row.l + 36.0 + (row.w() - 36.0) * 0.5;
            let f = &favs[i];
            gfx.text(&f.name, R::new(row.l + 40.0, y, split - 8.0, y + FAV_ROW_H), Font::Normal, fg);
            let show_buttons = row_hover || i == sel;
            let path_r = if show_buttons { row.r - 66.0 } else { row.r - 12.0 };
            gfx.text(&f.path, R::new(split, y, path_r, y + FAV_ROW_H), Font::Path, t.faint);
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
                let er = R::new(row.l + 36.0, y + 3.0, split, y + FAV_ROW_H - 3.0);
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
            "클릭 열기 · Ctrl+Enter 새 탭 · F2 이름 · Del 삭제 · 끌거나 Ctrl+↑↓로 순서 변경",
            R::new(r.l + 14.0, r.b - FAV_FOOT_H, r.r - 10.0, r.b - 2.0),
            Font::Normal,
            t.faint,
        );
    }
}
