//! 왼쪽 사이드바: 창 전체 높이. 맨 위 검색 상자, 가운데 목록(즐겨찾기 · 드라이브 · 검색에 넣은 폴더), 아래쪽 설정·도움말.
//! 도구 막대 줄을 없애고 그 단추들을 쓰임에 맞는 자리로 옮긴 것이다(칸 나누기·숨김·미리 보기는 작업 줄에).
//! 가운데 목록의 줄 배치는 `side_slots` 한 곳에서 정해 그리기·누르기·끌어 놓기가 같이 쓴다.
//! 즐겨찾기의 동작(열기·추가·이름·그룹·키보드·끌어서 옮기기)은 fav_ui.rs

use super::*;
use crate::favorites::Row;

/// 사이드바 너비 (DIP). 칸 영역은 그 오른쪽 MARGIN부터
pub(super) const SIDEBAR_W: f32 = 232.0;
pub(super) const SIDE_ROW_H: f32 = 30.0;
const ROW_L: f32 = 8.0;
const ROW_R: f32 = SIDEBAR_W - 8.0;
const HEAD_GAP: f32 = 16.0;
const HEAD_H: f32 = 22.0;
const MINI: f32 = 22.0;
const DRIVE_H: f32 = 44.0;
/// 그룹 안 항목의 들여쓰기
const INDENT: f32 = 20.0;
const G_HELP: u16 = 0xE897;
const G_SORT: u16 = 0xE8CB;
const G_NEW_GROUP: u16 = 0xE8F4;
const G_TWIST_OPEN: u16 = 0xE70D;
const G_TWIST_CLOSED: u16 = 0xE76C;

/// 사이드바 가운데 목록에서 누를 수 있는 것
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Side {
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

impl Side {
    fn is_fav(self) -> bool {
        matches!(self, Side::Fav(_) | Side::FavEmpty)
    }
}

/// 가운데 목록의 한 자리 (스크롤 전, 목록 맨 위가 0인 좌표)
#[derive(Clone, Copy)]
enum Slot {
    Head(&'static str),
    Item(Side),
}

/// 드라이브 한 개
pub(super) struct Drive {
    name: String,
    path: String,
    icon: i32,
    free: u64,
    total: u64,
}

/// 뒤에서 읽어 오는 "내 PC"·드라이브 목록
#[derive(Default)]
pub struct Places {
    computer_icon: i32,
    drives: Vec<Drive>,
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

/// 창 바탕 위에 한 단계 들어간 면 (검색 상자, 칸 나누기 판, 지금 위치 줄)
pub(super) fn well(t: &Theme) -> Color {
    if t.high_contrast { t.window } else { t.sel_inactive }
}

/// 사이드바 줄의 마우스 올림 바탕: 창 바탕에서도 보이도록 반투명으로 겹친다
fn hover_fill(t: &Theme) -> Color {
    if t.high_contrast {
        t.hover
    } else if t.dark {
        Color { r: 1.0, g: 1.0, b: 1.0, a: 0.06 }
    } else {
        Color { r: 0.06, g: 0.09, b: 0.16, a: 0.05 }
    }
}

impl App {
    // ---------- 배치 ----------

    /// 맨 위 검색 상자 (누르면 검색 팔레트)
    pub(super) fn search_button(&self) -> R {
        R::new(12.0, 10.0, SIDEBAR_W - 12.0, 44.0)
    }

    /// 아래쪽: 설정, 그 아래 도움말
    pub(super) fn settings_button(&self) -> R {
        let b = self.h - 8.0;
        R::new(ROW_L, b - 2.0 * SIDE_ROW_H, ROW_R, b - SIDE_ROW_H)
    }

    pub(super) fn help_button(&self) -> R {
        let b = self.h - 8.0;
        R::new(ROW_L, b - SIDE_ROW_H, ROW_R, b)
    }

    /// 가운데 목록이 보이는 곳 (검색 상자 아래부터 아래쪽 구분선 위까지)
    pub(super) fn side_list(&self) -> R {
        let t = self.search_button().b + 6.0;
        R::new(0.0, t, SIDEBAR_W, (self.settings_button().t - 7.0).max(t))
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

    /// 가운데 목록의 자리들과 전체 높이. 좌표는 스크롤 전, 목록 맨 위가 0
    fn side_slots(&self) -> (Vec<(R, Slot)>, f32) {
        let mut out: Vec<(R, Slot)> = Vec::new();
        let mut y = 0.0;
        let head = |out: &mut Vec<(R, Slot)>, y: &mut f32, label: &'static str| -> R {
            *y += HEAD_GAP;
            let r = R::new(ROW_L + 10.0, *y, ROW_R - 6.0, *y + HEAD_H);
            out.push((r, Slot::Head(label)));
            *y += HEAD_H + 4.0;
            r
        };
        let mini = |h: R, i: usize| {
            let r = h.r - i as f32 * (MINI + 2.0);
            R::new(r - MINI, h.t, r, h.b)
        };

        // 즐겨찾기: 머리 줄 오른쪽에 추가·정렬·새 그룹
        let h = head(&mut out, &mut y, "즐겨찾기");
        for (i, s) in [Side::FavAdd, Side::FavSort, Side::FavNewGroup].into_iter().enumerate() {
            out.push((mini(h, i), Slot::Item(s)));
        }
        let rows = self.favs.rows();
        for row in &rows {
            out.push((R::new(ROW_L, y, ROW_R, y + SIDE_ROW_H), Slot::Item(Side::Fav(*row))));
            y += SIDE_ROW_H;
        }
        if rows.is_empty() {
            out.push((R::new(ROW_L, y, ROW_R, y + 44.0), Slot::Item(Side::FavEmpty)));
            y += 44.0;
        }

        // 드라이브: 내 PC, 드라이브마다 남은 용량 막대
        head(&mut out, &mut y, "드라이브");
        out.push((R::new(ROW_L, y, ROW_R, y + SIDE_ROW_H), Slot::Item(Side::Computer)));
        y += SIDE_ROW_H;
        for i in 0..self.places.drives.len() {
            out.push((R::new(ROW_L, y, ROW_R, y + DRIVE_H), Slot::Item(Side::Drive(i))));
            y += DRIVE_H;
        }

        // 검색에 넣은 폴더: 지금 칸이 검색 밖이면 머리 줄에 +
        let h = head(&mut out, &mut y, "검색에 넣은 폴더");
        if self.side_outside().is_some() {
            out.push((mini(h, 0), Slot::Item(Side::FolderAdd)));
        }
        let n = self.folders.list().len();
        for i in 0..n {
            out.push((R::new(ROW_L, y, ROW_R, y + SIDE_ROW_H), Slot::Item(Side::Folder(i))));
            y += SIDE_ROW_H;
        }
        if n == 0 {
            y += 40.0;
        }
        (out, y + 8.0)
    }

    /// 화면 좌표로 옮긴 자리
    fn side_at(&self, r: R) -> R {
        let dy = self.side_list().t - self.side_scroll;
        R::new(r.l, r.t + dy, r.r, r.b + dy)
    }

    /// 그 즐겨찾기 줄의 화면 자리 (목록 밖으로 스크롤돼 있어도)
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

    // ---------- 누르기 ----------

    pub(super) fn sidebar_hit(&self, x: f32, y: f32) -> Hit {
        if self.search_button().contains(x, y) {
            return Hit::SearchButton;
        }
        if self.settings_button().contains(x, y) {
            return Hit::SettingsButton;
        }
        if self.help_button().contains(x, y) {
            return Hit::HelpButton;
        }
        let list = self.side_list();
        if !list.contains(x, y) {
            return Hit::None;
        }
        let (slots, _) = self.side_slots();
        for (r, s) in slots {
            if let Slot::Item(side) = s {
                if self.side_at(r).contains(x, y) {
                    return Hit::Side(side);
                }
            }
        }
        Hit::None
    }

    /// 끌어 놓기 대상: 즐겨찾기 쪽(줄·안내·머리)이면 즐겨찾기에 넣기, 드라이브·검색 폴더 줄이면 그 폴더
    pub(super) fn side_drop_at(&self, x: f32, y: f32) -> Option<(Side, Option<String>)> {
        if x >= SIDEBAR_W || !self.side_list().contains(x, y) {
            return None;
        }
        let (slots, _) = self.side_slots();
        // 즐겨찾기 머리부터 마지막 즐겨찾기 줄까지는 어디든 즐겨찾기에 넣기
        let fav_end = slots.iter().filter(|(_, s)| matches!(s, Slot::Item(side) if side.is_fav())).map(|(r, _)| r.b).fold(0.0, f32::max);
        let top = self.side_at(R::new(0.0, 0.0, 0.0, fav_end));
        if y < top.b {
            return Some((Side::FavEmpty, None));
        }
        match self.sidebar_hit(x, y) {
            Hit::Side(s @ Side::Drive(i)) => Some((s, self.places.drives.get(i).map(|d| d.path.clone()))),
            Hit::Side(s @ Side::Folder(i)) => Some((s, self.folders.list().get(i).map(|(p, _)| p.clone()))),
            _ => None,
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

    /// 사이드바 줄을 연다 (new_tab: 새 탭에서)
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
    }

    pub(super) fn side_mouse_down(&mut self, s: Side, y: f32, dbl: bool) {
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
            Side::FolderAdd => {
                if let Some(dir) = self.side_outside() {
                    let p = self.active;
                    match self.folders.add(&dir) {
                        Ok(()) => self.flash(p, format!("검색에 넣었습니다 · 훑는 동안에도 찾을 수 있어요: {dir}")),
                        Err(e) => self.flash(p, format!("검색 폴더 목록을 저장하지 못했습니다: {e}")),
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
                    crate::folders::State::Ready(n) => format!("{path} · {n}개"),
                    crate::folders::State::Offline => format!("{path} · 연결되지 않음"),
                }
            }
            _ => return None,
        };
        Some((self.side_item_rect(s)?, text))
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
        self.side_clamp_scroll();
        self.invalidate();
    }

    // ---------- 그리기 ----------

    /// 확장자·폴더별 아이콘 (형식 캐시)
    fn type_icon(&mut self, ext: &str, dir: bool) -> i32 {
        let key = if dir { "\\dir".to_string() } else { ext.to_string() };
        self.types.entry(key).or_insert_with(|| shell::type_info(ext, dir)).1
    }

    pub(super) fn paint_sidebar(&mut self) {
        let t = self.theme;
        let list = self.side_list();
        let (slots, _) = self.side_slots();
        let cur_dir = self.panes[self.active].tab().loc.dir().map(str::to_string);
        let at_computer = self.panes[self.active].tab().loc == Location::Computer;
        let is_current = |path: &str| cur_dir.as_deref().is_some_and(|d| util::eq_path(d, path));
        let hover = self.hover;
        let fav_rows = self.favs.rows();
        let sel_row = self.fav_focus.then(|| fav_rows.get(self.fav_sel).copied()).flatten();
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
        let search_key = self.keys.get(crate::keys::Action::Search).map(|c| c.label());
        let (sb, settings, help) = (self.search_button(), self.settings_button(), self.help_button());
        let dy = list.t - self.side_scroll;
        let App { gfx: g, favs, places, pal_open, .. } = self;

        // 검색 상자. 팔레트가 열려 있으면 눌린 모양
        let on = *pal_open;
        g.fill_round(sb, 8.0, if on { t.accent_soft } else { well(&t) });
        if !on && hover == Hit::SearchButton {
            g.fill_round(sb, 8.0, hover_fill(&t));
        }
        if t.high_contrast {
            g.stroke_round(sb.inset(0.5, 0.5), 8.0, t.border, 1.0);
        }
        g.glyph(G_SEARCH, R::new(sb.l + 10.0, sb.t, sb.l + 28.0, sb.b), true, if on { t.accent } else { t.dim });
        g.text("검색", R::new(sb.l + 36.0, sb.t, sb.r - 8.0, sb.b), Font::Normal, if on { t.accent } else { t.faint });
        // 바꾼 검색 단축키를 그대로 보여 준다 (비워 두었으면 없음)
        if let Some(k) = &search_key {
            g.text(k, R::new(sb.l + 36.0, sb.t, sb.r - 10.0, sb.b), Font::Right, t.faint);
        }

        // 가운데 목록
        g.clip(list);
        let row_bg = |g: &Gfx, r: R, current: bool, hovered: bool| {
            if current {
                g.fill_round(r, 6.0, well(&t));
            }
            if hovered {
                g.fill_round(r, 6.0, hover_fill(&t));
            }
        };
        for (r, slot) in &slots {
            let r = R::new(r.l, r.t + dy, r.r, r.b + dy);
            if r.b < list.t || r.t > list.b {
                if let Slot::Item(Side::Fav(Row::Item { .. })) = slot {
                    fav_icons.next();
                }
                continue;
            }
            match *slot {
                Slot::Head(label) => g.text(label, r, Font::Small, t.faint),
                Slot::Item(s @ (Side::FavAdd | Side::FavSort | Side::FavNewGroup | Side::FolderAdd)) => {
                    if hover == Hit::Side(s) {
                        g.fill_round(r, 5.0, hover_fill(&t));
                    }
                    let glyph = match s {
                        Side::FavSort => G_SORT,
                        Side::FavNewGroup => G_NEW_GROUP,
                        _ => G_ADD,
                    };
                    g.glyph(glyph, r, true, if hover == Hit::Side(s) { t.text } else { t.dim });
                }
                Slot::Item(Side::FavEmpty) => {
                    if side_drop == Some(Side::FavEmpty) {
                        g.fill_round(r, 6.0, alpha(t.accent, 0.10));
                    }
                    let how = add_key.as_deref().map(|k| format!("별(☆)이나 {k}로 넣어 보세요")).unwrap_or_else(|| "경로 상자의 별(☆)로 넣어 보세요".to_string());
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
                            let n = group.items.len().to_string();
                            let nw = g.measure(&n, Font::Small);
                            if editing != Some(row) {
                                g.text(&group.name, R::new(lx, r.t, r.r - 16.0 - nw, r.b), Font::Normal, fg);
                            }
                            g.text(&n, R::new(r.r - 10.0 - nw, r.t, r.r - 8.0, r.b), Font::Small, t.faint);
                            // 항목을 그룹 머리 줄에 끌어 놓으면 그 그룹으로
                            if let Some((Row::Item { .. }, Row::Group(at), _)) = moving {
                                if at == gi {
                                    g.stroke_round(r.inset(0.75, 0.75), 6.0, t.accent, 1.5);
                                }
                            }
                        }
                        Row::Item { group, index } => {
                            let icon = fav_icons.next().unwrap_or(-1);
                            let Some(f) = favs.item(group, index) else { continue };
                            row_bg(g, r, f.is_dir && is_current(&f.path), hover == Hit::Side(s));
                            let il = lx - 26.0;
                            g.icon(icon, R::new(il, r.t + 7.0, il + 16.0, r.t + 23.0));
                            if editing != Some(row) {
                                g.text(&f.name, R::new(lx, r.t, r.r - 8.0, r.b), Font::Normal, fg);
                            }
                        }
                    }
                    if sel_row == Some(row) {
                        g.stroke_round(r.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), 6.0, t.focus, FOCUS_W);
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
                        g.icon(places.computer_icon, R::new(il, r.t + 7.0, il + 16.0, r.t + 23.0));
                    } else {
                        g.glyph(0xEC4E, R::new(il, r.t, il + 16.0, r.b), true, t.dim);
                    }
                    g.text("내 PC", R::new(r.l + 36.0, r.t, r.r - 8.0, r.b), Font::Normal, t.text);
                }
                Slot::Item(s @ Side::Drive(i)) => {
                    let Some(d) = places.drives.get(i) else { continue };
                    row_bg(g, r, is_current(&d.path), hover == Hit::Side(s) || side_drop == Some(s));
                    if side_drop == Some(s) {
                        g.stroke_round(r.inset(0.75, 0.75), 6.0, t.accent, 1.5);
                    }
                    let head = R::new(r.l, r.t + 6.0, r.r, r.t + 24.0);
                    g.icon(d.icon, R::new(r.l + 10.0, head.t + 1.0, r.l + 26.0, head.t + 17.0));
                    let free = (d.total > 0).then(|| format!("{} 남음", format_size(d.free)));
                    let fw = free.as_deref().map_or(0.0, |f| g.measure(f, Font::Small));
                    g.text(&d.name, R::new(r.l + 36.0, head.t, r.r - 16.0 - fw, head.b), Font::Normal, t.text);
                    if let Some(f) = &free {
                        g.text(f, R::new(r.r - 10.0 - fw, head.t, r.r - 8.0, head.b), Font::Small, t.faint);
                        // 쓴 만큼 채운 막대. 거의 찼으면(90% 넘게) 빨간색
                        let used = (1.0 - d.free as f32 / d.total as f32).clamp(0.0, 1.0);
                        let bar = R::new(r.l + 36.0, head.b + 7.0, r.r - 10.0, head.b + 11.0);
                        g.fill_round(bar, 2.0, well(&t));
                        let fill = if used > 0.9 { Color { r: 0.80, g: 0.20, b: 0.16, a: 1.0 } } else { t.faint };
                        g.fill_round(R::new(bar.l, bar.t, bar.l + bar.w() * used, bar.b), 2.0, fill);
                    }
                }
                Slot::Item(s @ Side::Folder(i)) => {
                    let Some((path, state)) = folders.get(i) else { continue };
                    row_bg(g, r, is_current(path), hover == Hit::Side(s) || side_drop == Some(s));
                    if side_drop == Some(s) {
                        g.stroke_round(r.inset(0.75, 0.75), 6.0, t.accent, 1.5);
                    }
                    g.icon(folder_icon, R::new(r.l + 10.0, r.t + 7.0, r.l + 26.0, r.t + 23.0));
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
        if folders.is_empty() {
            if let Some((r, _)) = slots.iter().rev().find(|(_, s)| matches!(s, Slot::Head("검색에 넣은 폴더"))) {
                let top = r.b + dy + 4.0;
                g.text("공유폴더·USB를 열고 +를 누르면", R::new(ROW_L + 10.0, top, ROW_R - 6.0, top + 18.0), Font::Small, t.faint);
                g.text("함께 찾아요", R::new(ROW_L + 10.0, top + 18.0, ROW_R - 6.0, top + 36.0), Font::Small, t.faint);
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
                g.fill_round(R::new(ROW_L + 8.0, ly - 1.5, ROW_R - 8.0, ly + 1.5), 1.5, t.accent);
            }
        }
        // 파일을 끌어 즐겨찾기 쪽에 놓으려 할 때: 즐겨찾기 영역 전체를 강조
        if side_drop == Some(Side::FavEmpty) && !favs.rows().is_empty() {
            let top = slots.iter().find(|(_, s)| matches!(s, Slot::Head(_))).map_or(0.0, |(r, _)| r.t);
            let bottom = slots.iter().filter(|(_, s)| matches!(s, Slot::Item(Side::Fav(_)))).map(|(r, _)| r.b).last().unwrap_or(top);
            let area = R::new(ROW_L - 2.0, top + dy - 4.0, ROW_R + 2.0, bottom + dy + 2.0);
            g.fill_round(area, 8.0, alpha(t.accent, 0.08));
            g.stroke_round(area.inset(0.75, 0.75), 8.0, t.accent, 1.5);
        }
        g.unclip();

        // 아래쪽: 구분선, 설정, 도움말
        g.fill(R::new(settings.l + 8.0, settings.t - 6.0, settings.r - 8.0, settings.t - 5.0), t.border);
        for (r, glyph, label, h) in [(settings, G_SETTINGS, "설정", Hit::SettingsButton), (help, G_HELP, "도움말", Hit::HelpButton)] {
            if hover == h {
                g.fill_round(r, 6.0, hover_fill(&t));
            }
            g.glyph(glyph, R::new(r.l + 8.0, r.t, r.l + 28.0, r.b), true, t.dim);
            g.text(label, R::new(r.l + 36.0, r.t, r.r - 8.0, r.b), Font::Normal, t.text);
        }
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
        (1 | 2, _) => {
            with_app(|a| a.side_open(side, cmd == 2));
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
