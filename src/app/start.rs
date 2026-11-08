//! 새 탭 시작 화면 ("어디로 이동할까요?"): 즐겨찾기 폴더 카드, 최근 위치, 드라이브(내 PC 포함).
//! 칩 줄·경로 줄은 다른 위치와 같고, 그 아래(머리글·목록 자리)를 이 화면이 쓴다.
//! 자리는 `start_layout` 한 곳에서 정해 그리기·누르기·키보드·스크롤이 같이 쓴다.
//! 키보드 초점 테두리는 화살표·Enter·필터 글자로 키보드를 쓴 뒤에만 보인다 (탭을 열자마자 첫 카드에 테두리가 생기지 않게)

use super::*;

/// 세션에 남기는 최근 위치 수와 화면에 보이는 수
const RECENT_KEEP: usize = 20;
const RECENT_SHOWN: usize = 8;
/// 내용 좌우 여백과 가장 넓을 때의 폭 (넓은 칸에서 카드가 한없이 늘어나지 않게)
const PAD: f32 = 24.0;
const MAX_W: f32 = 760.0;
const CARD_MIN_W: f32 = 150.0;
const CARD_H: f32 = 64.0;
const CARD_GAP: f32 = 10.0;
const CARD_R: f32 = 8.0;
const LINE_H: f32 = 32.0;
const TITLE_H: f32 = 36.0;
/// 시작 화면 영역이 이보다 낮으면 제목을 그리지 않는다
const TITLE_MIN_AREA_H: f32 = 360.0;
const SECTION_H: f32 = 26.0;
const SECTION_GAP: f32 = 20.0;
const NOTE_H: f32 = 22.0;
const G_RECENT: u16 = 0xE823;
const G_COMPUTER: u16 = 0xEC4E;

/// 시작 화면에서 고를 수 있는 것
#[derive(Clone, PartialEq, Debug)]
pub enum StartItem {
    /// 즐겨찾기 폴더 (보이는 이름, 경로)
    Fav { name: String, path: String },
    Recent(String),
    Computer,
    /// places.drives의 번호
    Drive(usize),
}

/// 시작 화면 배치. 좌표는 지금 스크롤이 반영된 화면 좌표
pub(super) struct StartView {
    title: R,
    heads: Vec<(&'static str, R)>,
    items: Vec<(StartItem, R)>,
    /// 즐겨찾기가 없을 때·찾는 글자와 맞는 게 없을 때의 안내
    notes: Vec<(String, R)>,
    /// 스크롤과 상관없는 내용 전체 높이
    height: f32,
    scroll: f32,
    area: R,
}

/// 칸 안에서 시작 화면이 놓이는 곳 (경로 줄 아래 전부)
fn start_area(g: &PaneGeom) -> R {
    R::new(g.list.l, g.header.t, g.list.r, g.list.b)
}

/// 최근 위치의 보이는 이름 (드라이브 뿌리는 "C:")
fn recent_label(path: &str) -> &str {
    let n = util::file_name(path);
    if n.is_empty() { path } else { n }
}

/// 카드를 cols개씩 줄지어 놓고 y를 마지막 줄 아래로 옮긴다
fn place_cards(list: Vec<StartItem>, x0: f32, y: &mut f32, cols: usize, card_w: f32, out: &mut Vec<(StartItem, R)>) {
    let rows = list.len().div_ceil(cols);
    for (i, it) in list.into_iter().enumerate() {
        let l = x0 + (i % cols) as f32 * (card_w + CARD_GAP);
        let t = *y + (i / cols) as f32 * (CARD_H + CARD_GAP);
        out.push((it, R::new(l, t, l + card_w, t + CARD_H)));
    }
    if rows > 0 {
        *y += rows as f32 * (CARD_H + CARD_GAP) - CARD_GAP;
    }
}

/// 화살표 방향의 다음 항목: ←→는 앞뒤, ↑↓는 바로 위·아래 줄에서 가로 가운데가 가장 가까운 것, Home·End는 처음·끝
fn neighbor(rects: &[R], from: usize, vk: VIRTUAL_KEY) -> usize {
    let Some(cur) = rects.get(from) else { return 0 };
    match vk {
        VK_LEFT => from.saturating_sub(1),
        VK_RIGHT => (from + 1).min(rects.len() - 1),
        VK_HOME => 0,
        VK_END => rects.len() - 1,
        VK_UP | VK_DOWN => {
            let down = vk == VK_DOWN;
            // 지금 줄과 겹치지 않는 것 중 가장 가까운 줄
            let row_t = rects
                .iter()
                .filter(|r| if down { r.t >= cur.b - 1.0 } else { r.b <= cur.t + 1.0 })
                .map(|r| r.t)
                .reduce(|a, b| if down { a.min(b) } else { a.max(b) });
            let Some(row_t) = row_t else { return from };
            let cx = (cur.l + cur.r) / 2.0;
            let off = |r: &R| ((r.l + r.r) / 2.0 - cx).abs();
            rects
                .iter()
                .enumerate()
                .filter(|(_, r)| (r.t - row_t).abs() < 1.0)
                .min_by(|(_, a), (_, b)| off(a).total_cmp(&off(b)))
                .map_or(from, |(i, _)| i)
        }
        _ => from,
    }
}

impl StartView {
    /// 그리기. hovered: 마우스가 올라간 항목, focus: 키보드 초점 테두리를 그릴 항목
    pub(super) fn paint(&self, g: &mut Gfx, t: &Theme, places: &sidebar::Places, folder_icon: i32, hovered: Option<usize>, focus: Option<usize>) {
        g.clip(self.area);
        if self.title.h() > 0.0 {
            g.text("어디로 이동할까요?", self.title, Font::Title, t.text);
        }
        // 최근 위치: 이름 열 폭을 맞춰 경로가 같은 자리에서 시작하게
        let name_col = self
            .items
            .iter()
            .filter_map(|(it, r)| match it {
                StartItem::Recent(path) => Some((g.measure(recent_label(path), Font::Normal) + 4.0).min(r.w() * 0.45)),
                _ => None,
            })
            .fold(0.0f32, f32::max);
        for (label, r) in &self.heads {
            g.text(label, *r, Font::Small, t.faint);
        }
        for (s, r) in &self.notes {
            g.text(s, *r, Font::Small, t.faint);
        }
        let focus = focus.map(|f| f.min(self.items.len().saturating_sub(1)));
        for (i, (it, r)) in self.items.iter().enumerate() {
            let r = *r;
            if r.b < self.area.t || r.t > self.area.b {
                continue;
            }
            let hot = hovered == Some(i);
            let radius = if let StartItem::Recent(path) = it {
                if hot {
                    g.fill_round(r, R_ROW, t.hover);
                }
                g.glyph_font(G_RECENT, R::new(r.l + 12.0, r.t, r.l + 28.0, r.b), Font::GlyphMid, t.dim);
                let name = recent_label(path);
                g.text(name, R::new(r.l + 36.0, r.t, r.l + 36.0 + name_col, r.b), Font::Normal, t.text);
                // 이름 뒤에 어디 있는 폴더인지 흐리게 (길면 가운데를 줄인다)
                if let Some(parent) = util::parent(path) {
                    g.text(&parent, R::new(r.l + 36.0 + name_col + 16.0, r.t, r.r - 8.0, r.b), Font::Path, t.faint);
                }
                R_ROW
            } else {
                // 카드: 옅은 면과 테두리, 올리면 한 단계 진한 면. 첫 줄 아이콘·이름, 둘째 줄 위치나 남은 용량
                g.fill_round(r, CARD_R, if hot { t.sel_inactive } else { t.field });
                g.stroke_round(r.inset(0.5, 0.5), CARD_R, t.border, 1.0);
                let ic = R::new(r.l + 12.0, r.t + 13.0, r.l + 28.0, r.t + 29.0);
                let name_r = R::new(r.l + 36.0, r.t + 10.0, r.r - 10.0, r.t + 32.0);
                let sub_r = R::new(r.l + 36.0, r.t + 32.0, r.r - 10.0, r.t + 50.0);
                match it {
                    StartItem::Fav { name, path } => {
                        g.icon(folder_icon, ic);
                        g.text(name, name_r, Font::Strong, t.text);
                        if let Some(parent) = util::parent(path) {
                            g.text(&parent, sub_r, Font::Path, t.faint);
                        }
                    }
                    StartItem::Computer => {
                        if places.computer_icon >= 0 {
                            g.icon(places.computer_icon, ic);
                        } else {
                            g.glyph(G_COMPUTER, ic, false, t.dim);
                        }
                        g.text("내 PC", name_r, Font::Strong, t.text);
                    }
                    StartItem::Drive(di) => {
                        if let Some(d) = places.drives.get(*di) {
                            g.icon(d.icon, ic);
                            g.text(&d.name, name_r, Font::Strong, t.text);
                            if d.total > 0 {
                                let free = format!("{} 남음", format_size(d.free));
                                g.text(&free, sub_r, Font::Small, t.faint);
                                // 쓴 만큼 채운 막대. 거의 찼으면(90% 넘게) 빨간색
                                let used = (1.0 - d.free as f32 / d.total as f32).clamp(0.0, 1.0);
                                let bar = R::new(r.l + 12.0, r.b - 9.0, r.r - 12.0, r.b - 6.0);
                                g.fill_round(bar, 1.5, t.border);
                                let fill = if used > 0.9 { Color { r: 0.80, g: 0.20, b: 0.16, a: 1.0 } } else { t.faint };
                                g.fill_round(R::new(bar.l, bar.t, bar.l + bar.w() * used, bar.b), 1.5, fill);
                            }
                        }
                    }
                    StartItem::Recent(_) => {}
                }
                CARD_R
            };
            if focus == Some(i) {
                g.stroke_round(r.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), radius, t.focus, FOCUS_W);
            }
        }
        g.unclip();
        // 넘치면 목록처럼 오른쪽에 스크롤 손잡이 (보이기만 한다. 휠·Page Up/Down으로 움직인다)
        if let Some((_, thumb)) = scroll_metrics(self.area, self.height, self.scroll) {
            g.fill_round(thumb, 3.0, t.thumb);
        }
    }
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

    /// 제목 → 즐겨찾기 폴더 카드 → 최근 위치 줄 → 드라이브 카드. 필터 글자가 있으면 이름에 그 글자가 든 것만
    fn start_layout(&self, area: R, filter: &str, scroll: f32) -> StartView {
        let needle = filter.to_lowercase();
        let fits = |label: &str| needle.is_empty() || label.to_lowercase().contains(&needle);
        let w = (area.w() - PAD * 2.0).clamp(0.0, MAX_W);
        let x0 = area.l + PAD;
        let top = area.t - scroll;
        let mut y = top + 16.0;
        let title = if area.h() >= TITLE_MIN_AREA_H { R::new(x0, y, x0 + w, y + TITLE_H) } else { R::new(x0, y, x0 + w, y) };
        if title.h() > 0.0 {
            y += TITLE_H + 6.0;
        }
        let cols = (((w + CARD_GAP) / (CARD_MIN_W + CARD_GAP)).floor() as usize).clamp(1, 4);
        let card_w = ((w - CARD_GAP * (cols - 1) as f32) / cols as f32).max(0.0);
        let mut heads = Vec::new();
        let mut items = Vec::new();
        let mut notes = Vec::new();
        let fav_dirs = || self.favs.items.iter().chain(self.favs.groups.iter().flat_map(|g| g.items.iter())).filter(|f| f.is_dir);

        let favs: Vec<StartItem> = fav_dirs().filter(|f| fits(&f.name)).map(|f| StartItem::Fav { name: f.name.clone(), path: f.path.clone() }).collect();
        if !favs.is_empty() || needle.is_empty() {
            heads.push(("즐겨찾기", R::new(x0, y, x0 + w, y + SECTION_H)));
            y += SECTION_H + 4.0;
            if favs.is_empty() {
                notes.push(("경로 오른쪽 별(☆)로 즐겨찾기에 넣은 폴더가 여기에 카드로 보여요".to_string(), R::new(x0, y, x0 + w, y + NOTE_H)));
                y += NOTE_H;
            }
            place_cards(favs, x0, &mut y, cols, card_w, &mut items);
            y += SECTION_GAP;
        }

        // 즐겨찾기에 있는 폴더는 위 카드로 이미 보이니 빼고
        let recent: Vec<StartItem> = self
            .recent
            .iter()
            .filter(|r| !fav_dirs().any(|f| util::eq_path(&f.path, r)))
            .filter(|r| fits(recent_label(r)))
            .take(RECENT_SHOWN)
            .map(|r| StartItem::Recent(r.clone()))
            .collect();
        if !recent.is_empty() {
            heads.push(("최근 위치", R::new(x0, y, x0 + w, y + SECTION_H)));
            y += SECTION_H + 2.0;
            for it in recent {
                items.push((it, R::new(x0, y, x0 + w, y + LINE_H)));
                y += LINE_H;
            }
            y += SECTION_GAP;
        }

        let mut drives = Vec::new();
        if fits("내 PC") {
            drives.push(StartItem::Computer);
        }
        drives.extend(self.places.drives.iter().enumerate().filter(|(_, d)| fits(&d.name)).map(|(i, _)| StartItem::Drive(i)));
        if !drives.is_empty() {
            heads.push(("드라이브", R::new(x0, y, x0 + w, y + SECTION_H)));
            y += SECTION_H + 4.0;
            place_cards(drives, x0, &mut y, cols, card_w, &mut items);
            y += SECTION_GAP;
        }

        if items.is_empty() && !needle.is_empty() {
            notes.push((format!("‘{filter}’와(과) 맞는 위치가 없습니다"), R::new(x0, y, x0 + w, y + NOTE_H)));
            y += NOTE_H;
        }
        StartView { title, heads, items, notes, height: y - top, scroll, area }
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

    /// 마우스를 올려 두면 보여 줄 경로
    pub(super) fn start_tip(&self, p: usize, i: usize) -> Option<(R, String)> {
        let v = self.start_view(p)?;
        let (it, r) = v.items.get(i)?;
        let text = match it {
            StartItem::Fav { path, .. } | StartItem::Recent(path) => path.clone(),
            StartItem::Drive(d) => self.places.drives.get(*d)?.path.clone(),
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
                let next = if self.start_keys { neighbor(&rects, cur, vk) } else { cur };
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
        let Location::Dir(d) = loc else { return };
        self.recent.retain(|r| !util::eq_path(r, d));
        self.recent.insert(0, d.clone());
        self.recent.truncate(RECENT_KEEP);
        self.start_changed();
    }

    /// 최근 위치에서 뺀다 (메뉴·Delete, 또는 지워진 폴더)
    pub(super) fn forget_recent(&mut self, path: &str) {
        self.recent.retain(|r| !util::eq_path(r, path));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 카드 세 개짜리 두 줄(마지막 줄은 하나) 아래에 넓은 줄 두 개
    fn sample() -> Vec<R> {
        let card = |c: f32, row: f32| R::new(c * 110.0, row * 70.0, c * 110.0 + 100.0, row * 70.0 + 60.0);
        vec![card(0.0, 0.0), card(1.0, 0.0), card(2.0, 0.0), card(0.0, 1.0), R::new(0.0, 150.0, 320.0, 182.0), R::new(0.0, 182.0, 320.0, 214.0)]
    }

    #[test]
    fn arrows_move_between_cards_and_rows() {
        let r = sample();
        assert_eq!(neighbor(&r, 1, VK_RIGHT), 2);
        assert_eq!(neighbor(&r, 5, VK_RIGHT), 5, "stops at the end");
        assert_eq!(neighbor(&r, 0, VK_LEFT), 0, "stops at the start");
        assert_eq!(neighbor(&r, 1, VK_DOWN), 3, "the only card below");
        assert_eq!(neighbor(&r, 2, VK_DOWN), 3);
        assert_eq!(neighbor(&r, 3, VK_DOWN), 4, "down into the rows");
        assert_eq!(neighbor(&r, 4, VK_DOWN), 5);
        assert_eq!(neighbor(&r, 5, VK_DOWN), 5, "nothing below the last row");
        assert_eq!(neighbor(&r, 4, VK_UP), 3);
        assert_eq!(neighbor(&r, 3, VK_UP), 0, "the card straight above");
        assert_eq!(neighbor(&r, 0, VK_UP), 0);
        assert_eq!(neighbor(&r, 3, VK_END), 5);
        assert_eq!(neighbor(&r, 4, VK_HOME), 0);
    }

    #[test]
    fn recent_label_keeps_drive_roots_readable() {
        assert_eq!(recent_label("C:\\Users\\a\\문서"), "문서");
        assert_eq!(recent_label("D:\\"), "D:");
    }
}
