//! 새 탭 시작 화면 ("어디로 이동할까요?"): 고정한 폴더(이 공간에 고정한 것과 즐겨찾기) 칸, 최근 위치, 드라이브(내 PC 포함).
//! 칸과 줄에는 이름만 보인다. 경로는 마우스를 올리거나 키보드로 고른 줄에, 남은 용량은 말풍선에 보인다.
//! 칩 줄·경로 줄은 다른 위치와 같고, 그 아래(머리글·목록 자리)를 이 화면이 쓴다.
//! 자리·이웃·그리기는 맥과 함께 쓰는 sakadu::ui::start. 여기는 드라이브·아이콘, 키, 오른쪽 버튼 메뉴, 최근 위치 목록.
//! 키보드 초점 테두리는 화살표·Enter·필터 글자로 키보드를 쓴 뒤에만 보인다 (탭을 열자마자 첫 카드에 테두리가 생기지 않게)

use super::*;
pub use sakadu::ui::start::StartItem;
use sakadu::ui::start::{self as shared, Nav, StartArt, StartData, StartDrive, StartView, neighbor, paint_start, start_area};

const G_COMPUTER: u16 = 0xEC4E;

/// 시작 화면 아이콘: 시스템 아이콘 목록의 폴더·드라이브·내 PC 아이콘
struct WinStartArt<'a> {
    places: &'a sidebar::Places,
    folder_icon: i32,
}

impl StartArt<Gfx> for WinStartArt<'_> {
    fn icon(&mut self, g: &mut Gfx, t: &Theme, item: &StartItem, r: R) {
        match item {
            StartItem::Fav { .. } => g.icon(self.folder_icon, r),
            // 드라이브 자체(C: 등)는 드라이브 아이콘, 나머지는 폴더 아이콘
            StartItem::Recent(path) => {
                let drive = self.places.drives.iter().find(|d| d.path.trim_end_matches('\\').eq_ignore_ascii_case(path.trim_end_matches('\\')));
                g.icon(drive.map_or(self.folder_icon, |d| d.icon), r);
            }
            StartItem::Computer => {
                if self.places.computer_icon >= 0 {
                    g.icon(self.places.computer_icon, r);
                } else {
                    g.glyph(G_COMPUTER, r, false, t.dim);
                }
            }
            StartItem::Drive(di) => {
                if let Some(d) = self.places.drives.get(*di) {
                    g.icon(d.icon, r);
                }
            }
        }
    }
}

/// 드라이브 칸에 보일 것 (places.drives와 같은 순서)
fn start_drives(places: &sidebar::Places) -> Vec<StartDrive<'_>> {
    places.drives.iter().map(|d| StartDrive { name: &d.name, free: d.free, total: d.total }).collect()
}

/// 칸 p의 시작 화면을 그린다. hovered: 마우스가 올라간 항목, focus: 키보드 초점 테두리를 그릴 항목
pub(super) fn paint_start_view(g: &mut Gfx, t: &Theme, places: &sidebar::Places, folder_icon: i32, v: &StartView, hovered: Option<usize>, focus: Option<usize>) {
    paint_start(g, &mut WinStartArt { places, folder_icon }, t, v, &start_drives(places), hovered, focus);
}

impl App {
    /// 칸 p의 시작 화면 배치. 지금 탭이 시작 화면이 아니거나 칸이 안 보이면 None
    pub(super) fn start_view(&self, p: usize) -> Option<StartView> {
        let tab = self.panes[p].tab();
        if tab.loc != Location::Start {
            return None;
        }
        let r = self.pane_rect(p)?;
        let area = start_area(&geom(r));
        let v = self.start_layout(area, &tab.filter, tab.scroll);
        // 즐겨찾기·드라이브·최근 위치가 줄어 스크롤이 끝을 넘었으면 끝에 맞춘 배치로 (그리기·누르기가 같은 자리를 쓰게)
        let max = (v.height - area.h()).max(0.0);
        Some(if tab.scroll > max { self.start_layout(area, &tab.filter, max) } else { v })
    }

    /// 제목 → 즐겨찾기 폴더 카드 → 최근 위치 줄 → 드라이브 카드 (공통 배치: sakadu::ui::start::start_layout)
    fn start_layout(&self, area: R, filter: &str, scroll: f32) -> StartView {
        let drives = start_drives(&self.places);
        shared::start_layout(area, filter, scroll, &StartData { pins: self.pins(), favs: &self.favs, recent: &self.recent, computer: true, drives: &drives, drives_label: "드라이브" })
    }

    /// 시작 화면의 스크롤 끝. 시작 화면이 아니면 None
    pub(super) fn start_max_scroll(&self, p: usize) -> Option<f32> {
        self.start_view(p).map(|v| (v.height - v.area.h()).max(0.0))
    }

    /// 보이는 항목 수 (필터 칩의 개수)
    pub(super) fn start_count(&self, p: usize) -> Option<usize> {
        self.start_view(p).map(|v| v.items.len())
    }

    /// 칸 p의 시작 화면 안 (x, y)
    pub(super) fn start_hit(&self, p: usize, x: f32, y: f32) -> Hit {
        let Some(v) = self.start_view(p) else { return Hit::Pane(p) };
        match v.items.iter().position(|(_, r)| r.contains(x, y)) {
            Some(i) if v.area.contains(x, y) => Hit::Start(p, i),
            _ => Hit::Pane(p),
        }
    }

    fn start_location(&self, it: &StartItem) -> Option<Location> {
        match it {
            StartItem::Fav { path, .. } | StartItem::Recent(path) => Some(Location::Dir(util::normalize(path))),
            StartItem::Computer => Some(Location::Computer),
            StartItem::Drive(i) => self.places.drives.get(*i).map(|d| Location::Dir(d.path.clone())),
        }
    }

    /// 시작 화면 항목 i의 위치. 누른 순간에 정해 두면 그 뒤 목록이 바뀌어도 누른 것을 연다
    pub(super) fn start_target(&self, p: usize, i: usize) -> Option<Location> {
        let v = self.start_view(p)?;
        v.items.get(i).and_then(|(it, _)| self.start_location(it))
    }

    /// 시작 화면에서 고른 위치로 간다. new_tab이면 새 탭에서(activate: 그 탭으로 옮겨 감), 아니면 이 탭에서
    pub(super) fn start_go(&mut self, p: usize, loc: Location, new_tab: bool, activate: bool) {
        self.start_keys = false;
        if new_tab {
            self.new_tab(p, Some(loc), activate);
        } else {
            self.navigate(p, loc, None);
        }
    }

    /// 마우스를 올려 두면 보여 줄 것: 경로, 드라이브는 남은 용량
    pub(super) fn start_tip(&self, p: usize, i: usize) -> Option<(R, String)> {
        let v = self.start_view(p)?;
        let (it, r) = v.items.get(i)?;
        let text = match it {
            StartItem::Fav { path, .. } | StartItem::Recent(path) => path.clone(),
            StartItem::Drive(d) => {
                let d = self.places.drives.get(*d)?;
                if d.total > 0 { format!("{} 남음", format_size(d.free)) } else { d.path.clone() }
            }
            StartItem::Computer => return None,
        };
        (r.b > v.area.t && r.t < v.area.b).then_some((*r, text))
    }

    /// 시작 화면에서 누른 키. 처리했으면 true (나머지는 칸의 보통 키 처리로)
    pub(super) fn start_key(&mut self, p: usize, vk: VIRTUAL_KEY) -> bool {
        let (ctrl, shift) = (key_down(VK_CONTROL), key_down(VK_SHIFT));
        if key_down(VK_MENU) {
            return false;
        }
        let Some(v) = self.start_view(p) else { return false };
        let n = v.items.len();
        let cur = self.panes[p].tab().cursor.min(n.saturating_sub(1));
        match vk {
            VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END if !ctrl && !shift => {
                if n == 0 {
                    return true;
                }
                // 처음 누른 화살표는 테두리만 보인다 (어디에 있는지 먼저 보이게)
                let rects: Vec<R> = v.items.iter().map(|(_, r)| *r).collect();
                let nav = match vk {
                    VK_LEFT => Nav::Left,
                    VK_RIGHT => Nav::Right,
                    VK_UP => Nav::Up,
                    VK_DOWN => Nav::Down,
                    VK_HOME => Nav::Home,
                    _ => Nav::End,
                };
                let next = if self.start_keys { neighbor(&rects, cur, nav) } else { cur };
                self.start_keys = true;
                self.panes[p].tab_mut().cursor = next;
                self.start_reveal(p);
            }
            VK_PRIOR | VK_NEXT => {
                let page = (v.area.h() - 40.0).max(40.0);
                self.scroll_by(p, if vk == VK_PRIOR { -page } else { page });
            }
            // 테두리가 보이거나 글자로 걸러 두었을 때만 연다 (아니면 테두리부터 보인다): 탭을 열자마자 Enter로 엉뚱한 곳에 가지 않게
            VK_RETURN if !shift => {
                let filtered = !self.panes[p].tab().filter.is_empty();
                match v.items.get(cur).and_then(|(it, _)| self.start_location(it)) {
                    Some(loc) if self.start_keys || filtered => self.start_go(p, loc, ctrl, true),
                    _ => self.start_keys = n > 0,
                }
            }
            // 최근 위치 줄이면 목록에서 뺀다. 다른 것은 지우지 않는다 (즐겨찾기·드라이브는 여기서 지울 것이 아니다)
            VK_DELETE if !ctrl => {
                if let (true, Some((StartItem::Recent(path), _))) = (self.start_keys, v.items.get(cur)) {
                    let path = path.clone();
                    self.forget_recent(&path);
                }
            }
            VK_APPS => self.start_menu_at_cursor(p),
            VK_F10 if shift => self.start_menu_at_cursor(p),
            _ => return false,
        }
        self.invalidate();
        true
    }

    /// 키보드로 옮긴 항목이 보이게 스크롤한다. 첫 항목이면 제목까지 보이게 맨 위로
    fn start_reveal(&mut self, p: usize) {
        let Some(v) = self.start_view(p) else { return };
        let cur = self.panes[p].tab().cursor;
        let Some((_, r)) = v.items.get(cur) else { return };
        let tab = self.panes[p].tab_mut();
        if cur == 0 {
            tab.scroll = 0.0;
        } else if r.t - 8.0 < v.area.t {
            tab.scroll -= v.area.t - (r.t - 8.0);
        } else if r.b + 8.0 > v.area.b {
            tab.scroll += r.b + 8.0 - v.area.b;
        }
        self.clamp_scroll(p);
    }

    /// 오른쪽 버튼 메뉴 (열기·새 탭에서 열기, 최근 위치면 빼기, 폴더·드라이브면 속성)
    pub(super) fn start_context_menu(&mut self, p: usize, i: usize, x: f32, y: f32) {
        let Some(v) = self.start_view(p) else { return };
        let Some((item, _)) = v.items.get(i) else { return };
        let Some(loc) = self.start_location(item) else { return };
        self.panes[p].tab_mut().cursor = i;
        let pt = self.to_screen(x, y);
        self.pending.push(Deferred::StartMenu { pane: p, item: item.clone(), loc, pt });
        self.invalidate();
    }

    fn start_menu_at_cursor(&mut self, p: usize) {
        let Some(n) = self.start_count(p).filter(|n| *n > 0) else { return };
        let cur = self.panes[p].tab().cursor.min(n - 1);
        self.panes[p].tab_mut().cursor = cur;
        self.start_keys = true;
        // 스크롤로 밖에 나가 있을 수 있으니 먼저 보이게 하고 그 자리를 다시 구한다
        self.start_reveal(p);
        let Some((_, r)) = self.start_view(p).and_then(|v| v.items.into_iter().nth(cur)) else { return };
        self.start_context_menu(p, cur, r.l + 12.0, r.b - 4.0);
    }

    /// 다녀간 폴더를 최근 위치 맨 앞에 둔다 (같은 폴더는 한 번만)
    pub(super) fn note_recent(&mut self, loc: &Location) {
        if shared::note_recent(&mut self.recent, loc) {
            self.start_changed();
        }
    }

    /// 최근 위치에서 뺀다 (메뉴·Delete, 또는 지워진 폴더)
    pub(super) fn forget_recent(&mut self, path: &str) {
        shared::forget_recent(&mut self.recent, path);
        self.start_changed();
    }

    /// 시작 화면에 보이는 것(최근 위치·드라이브)이 바뀐 뒤: 스크롤을 끝에 맞추고 마우스 아래 항목을 다시 정한다
    pub(super) fn start_changed(&mut self) {
        for p in 0..self.layout.count() {
            if self.panes[p].tab().loc == Location::Start {
                self.clamp_scroll(p);
            }
        }
        self.refresh_hover();
        self.invalidate();
    }
}

/// 시작 화면 항목의 오른쪽 버튼 메뉴. 메뉴가 떠 있는 동안 목록이 바뀔 수 있어, 위치는 메뉴를 열 때 정해 둔 것을 쓴다
pub(super) fn show_start_menu(hwnd: HWND, pane: usize, item: StartItem, loc: Location, pt: POINT) {
    let mut items: Vec<(u32, &str)> = vec![(1, "열기"), (2, "새 탭에서 열기")];
    match item {
        StartItem::Recent(_) => items.extend([(0, ""), (3, "최근 위치에서 빼기")]),
        StartItem::Fav { .. } | StartItem::Drive(_) => items.extend([(0, ""), (4, "속성")]),
        StartItem::Computer => {}
    }
    let cmd = fav_ui::popup_menu(hwnd, pt, &items);
    match cmd {
        1 | 2 => {
            with_app(|a| {
                let new_tab = cmd == 2 || a.panes[pane].tab().loc != Location::Start;
                a.start_go(pane, loc, new_tab, true);
            });
        }
        3 => {
            if let StartItem::Recent(path) = &item {
                with_app(|a| a.forget_recent(path));
            }
        }
        4 => {
            if let Some(d) = loc.dir() {
                let _ = shell::invoke_verb(hwnd, &[d.to_string()], "properties");
            }
        }
        _ => {}
    }
}
