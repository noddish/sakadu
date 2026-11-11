//! 칸 하나 그리기: Windows와 맥이 함께 쓴다.
//! 앱은 자기 상태로 `PaneScene`(무엇을 그릴지)을 채우고, 여기서 그린다(어떻게 그릴지).
//! 아이콘·썸네일·유형 이름처럼 OS마다 다른 것만 `EntryArt`로 받는다

use super::glyph::*;
use super::*;
use crate::listing::{Entry, Kind, drive_type_label, format_size, format_time, format_time_short};
use crate::palette::{FOCUS_W, R_CTRL, R_ROW, alpha};
use crate::pane::{Pane, Tab};

/// 칸 안에서 마우스가 올라간 곳 (앱의 Hit을 그리기 직전에 바꿔 넘긴다)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PaneHover {
    None,
    Tab(usize),
    TabClose(usize),
    TabAdd,
    Back,
    Fwd,
    Up,
    NewFolder,
    View,
    Star,
    Crumb(usize),
    CrumbEmpty,
    Header(SortKey),
    Row(usize),
    /// 스크롤 막대(손잡이나 길)
    Scroll,
    FilterClear,
    NetFix,
}

/// 칩 줄 오른쪽의 필터 칩 (필터 글자가 있을 때)
pub struct FilterChip {
    pub chip: R,
    /// 입력 상자 자리, 개수 글자 자리, × 단추
    pub edit: R,
    pub count: R,
    pub clear: R,
    pub count_text: String,
    /// 필터 글자를 직접 그린다 (입력 상자가 그 자리에 없을 때)
    pub draw_text: bool,
}

/// 칸 하나를 그리는 데 필요한 것 (앱이 자기 상태로 채운다)
pub struct PaneScene<'a> {
    pub r: R,
    pub pane: &'a Pane,
    /// 지금 칸, 칸이 여럿인지
    pub active: bool,
    pub multi: bool,
    /// 키보드 포커스가 이 칸 목록에 있다 (포커스 테두리)
    pub keyboard_here: bool,
    pub hover: PaneHover,
    /// 탭 칩 자리와 새 탭(+) 자리
    pub tabs: Vec<R>,
    pub add: R,
    /// 칩 줄 오른쪽 글(항목 수·고른 개수)과 그 자리. hot이면 진하게 (잠깐 뜨는 알림 글)
    pub count_text: String,
    pub count_rect: R,
    pub count_hot: bool,
    pub filter: Option<FilterChip>,
    /// 경로 오른쪽 즐겨찾기 별 (폴더일 때만): 켜짐 여부
    pub starred: Option<bool>,
    /// 경로 맨 앞 이름 (Windows "내 PC")
    pub root: Option<&'a str>,
    /// 경로 입력 중. 입력 상자가 없으면(그림 시험) 그 글자를 직접 그린다
    pub address: bool,
    pub address_text: Option<&'a str>,
    /// 새 폴더·보기 단추를 둘지 (시작 화면에는 없다), 쓸 수 있는지
    pub tools: bool,
    pub new_folder_ok: bool,
    pub view_ok: bool,
    pub view_menu_open: bool,
    /// 이름 바꾸는 중 (커서 줄에 입력 자리)
    pub renaming: bool,
    /// 이 칸에서 끌고 있는 탭, 이 칸에 탭을 놓을 자리
    pub dragged_tab: Option<usize>,
    pub tab_drop: Option<usize>,
    /// 끌어 온 것(파일, 다른 칸의 탭)을 이 칸에 놓는다 / 이 줄(폴더)에 놓는다
    pub drop_on_pane: bool,
    pub drop_row: Option<usize>,
    /// 네모 상자 고르기 (화면 좌표)
    pub marquee: Option<R>,
    /// 스크롤 막대를 잡고 끄는 중
    pub scroll_drag: bool,
    /// 목록 자리 안내 글 (빈 폴더, 불러오는 중, 열 수 없음)
    pub hint: Option<String>,
    /// 로그인·다시 연결 단추
    pub net_button: Option<(R, &'a str)>,
    /// 늦게 읽히는 폴더의 오가는 막대: 위치(0~1)
    pub loading: Option<f32>,
    /// 수정한 날짜를 짧게(오늘·어제) 쓸지와 그 기준 시각
    pub short_dates: bool,
    pub now: u64,
}

/// OS마다 다른 그림: 항목 아이콘, 썸네일, 유형 이름
pub trait EntryArt<G> {
    fn icon(&mut self, g: &mut G, e: &Entry, r: R);
    /// 썸네일을 bx 안에 그린다 (size는 DIP 크기). 아직 없으면 false
    fn thumbnail(&mut self, g: &mut G, tab: &Tab, e: &Entry, bx: R, size: f32, opacity: f32) -> bool;
    fn type_name(&mut self, e: &Entry) -> String;
}

/// 칸 테두리 두께와 색: 모든 칸이 같은 옅은 선. 끌어 놓을 대상일 때만 강조 색으로.
/// 고대비에서는 모든 칸을 2px로 두고 활성 칸만 포커스 색으로 바꿔 두께는 그대로 둔다
fn border(t: &Theme, s: &PaneScene) -> (f32, Color) {
    let dropping = s.drop_on_pane;
    let border_w = if t.high_contrast || dropping { 2.0 } else if t.dark { 1.0 } else { 0.0 };
    let border_c = match () {
        _ if dropping => t.accent,
        _ if t.high_contrast && s.active && s.multi => t.focus,
        _ => t.pane_border,
    };
    (border_w, border_c)
}

/// 카드, 칩 줄(탭·새 탭·개수 또는 필터 칩), 경로 줄(탐색 단추·경로·별·경로 입력 자리)
pub fn paint_pane_head(gfx: &impl Canvas, t: &Theme, s: &PaneScene) {
    let t = *t;
    let (r, p_hover, pane) = (s.r, s.hover, s.pane);
    let (active, multi) = (s.active, s.multi);
    let tab = pane.tab();
    let g = geom(r);
    let (trs, add) = (&s.tabs, s.add);

    // 카드: 밝은 테마는 테두리 없이 옅은 그림자로 떠 있고, 지금 칸은 조금 더 높이 뜬다 (고대비는 테두리로)
    if active && multi && !t.high_contrast {
        soft_shadow(gfx, r, RADIUS, 12.0, 0.10, t.dark);
    } else if !t.high_contrast && !t.dark {
        soft_shadow(gfx, r, RADIUS, 6.0, 0.06, false);
    }
    gfx.fill_round(r, RADIUS, t.pane);
    // 지금 칸: 위쪽 가장자리 안쪽에 얇은 강조 선 (칸이 여럿일 때 어느 칸에서 일하는지 한눈에)
    if active && multi {
        gfx.fill_round(R::new(r.l + RADIUS, r.t + 1.0, r.r - RADIUS, r.t + 3.0), 1.0, if t.high_contrast { t.focus } else { t.accent });
    }

    // 칩 줄: 탭 칩. 지금 탭은 바탕이 있는 칩(지금 칸이면 옅은 강조 색), 나머지는 글자만이고 올리면 옅은 바탕.
    // 오른쪽 자리(필터 칩·개수) 앞까지만 그린다 (좁은 칸에서 겹치면 누르기도 오른쪽 자리가 먼저)
    let right = s.filter.as_ref().map_or(s.count_rect, |f| f.chip);
    gfx.clip(R::new(r.l + 1.0, g.head.t, (right.l - 4.0).max(r.l + 1.0), g.head.b));
    let hovered_tab = match p_hover {
        PaneHover::Tab(ht) | PaneHover::TabClose(ht) => Some(ht),
        _ => None,
    };
    for (i, tr) in trs.iter().enumerate() {
        // 칸이 좁아 가려진 탭 (휠·Ctrl+Tab으로 닿는다)
        if tr.w() <= 0.0 {
            continue;
        }
        let current = i == pane.active;
        let hovered = hovered_tab == Some(i);
        if current {
            gfx.fill_round(*tr, tr.h() / 2.0, well(&t));
            // 고대비에서는 바탕색만으로 구분이 어려우니 테두리를 더한다
            if t.high_contrast {
                gfx.stroke_round(tr.inset(0.5, 0.5), tr.h() / 2.0, if active { t.focus } else { t.pane_border }, 1.0);
            }
        } else if hovered {
            gfx.fill_round(*tr, tr.h() / 2.0, t.hover);
        }
        let closable = pane.tabs.len() > 1 && (current || hovered);
        let label = R::new(tr.l + 12.0, tr.t, tr.r - if closable { 26.0 } else { 10.0 }, tr.b);
        let fg = t.text;
        let color = if s.dragged_tab == Some(i) { alpha(fg, 0.5) } else { fg };
        gfx.text(&pane.tabs[i].loc.title(), label, Font::Normal, color);
        if closable {
            let cr = close_rect(*tr);
            if p_hover == PaneHover::TabClose(i) {
                gfx.fill_round(cr, cr.h() / 2.0, alpha(t.text, 0.12));
            }
            gfx.glyph(G_CLOSE, cr, true, if p_hover == PaneHover::TabClose(i) { t.text } else { t.dim });
        }
    }
    // 새 탭(+): 평소엔 흐리게, 올리면 옅은 바탕
    if p_hover == PaneHover::TabAdd {
        gfx.fill_round(add, add.h() / 2.0, t.hover);
    }
    gfx.glyph_font(G_ADD, add, Font::GlyphMid, if p_hover == PaneHover::TabAdd { t.text } else { t.dim });
    // 끌어 놓을 자리
    if let Some(at) = s.tab_drop {
        let x = trs.get(at).map(|tr| tr.l - 2.0).or(trs.last().map(|tr| tr.r + 2.0)).unwrap_or(g.head.l + 8.0);
        gfx.fill_round(R::new(x - 1.5, add.t, x + 1.5, add.b), 1.5, t.accent);
    }
    gfx.unclip();

    // 칩 줄 오른쪽: 필터 칩(글자를 치면), 아니면 항목 수·고른 개수
    match &s.filter {
        None => gfx.text(&s.count_text, s.count_rect, Font::Small, if s.count_hot { t.text } else { t.faint }),
        Some(f) => {
            let fchip = f.chip;
            gfx.fill_round(fchip, fchip.h() / 2.0, t.accent_soft);
            if t.high_contrast {
                gfx.stroke_round(fchip.inset(0.5, 0.5), fchip.h() / 2.0, t.border, 1.0);
            }
            gfx.glyph(G_FILTER, R::new(fchip.l + 8.0, fchip.t, fchip.l + 26.0, fchip.b), true, t.chip_text);
            // 지금 칸에서는 입력 상자(자식 창)가 이 자리에 글자를 그린다
            if f.draw_text {
                gfx.text(&tab.filter, f.edit, Font::Normal, t.text);
            }
            let cw = gfx.measure(&f.count_text, Font::Small);
            gfx.text(&f.count_text, R::new(f.count.r - cw - 1.0, f.count.t, f.count.r, f.count.b), Font::Small, t.dim);
            if p_hover == PaneHover::FilterClear {
                gfx.fill_round(f.clear, f.clear.h() / 2.0, alpha(t.text, 0.12));
            }
            gfx.glyph(G_CLOSE, f.clear, true, t.dim);
        }
    }

    // 경로 줄: 탐색 단추
    let off = if t.high_contrast { t.faint } else { alpha(t.dim, 0.45) };
    let nav_btn = |rr: R, glyph: u16, font: Font, enabled: bool, hovered: bool| {
        if rr.w() <= 0.0 {
            return;
        }
        if hovered && enabled {
            gfx.fill_round(rr, R_CTRL, t.hover);
        }
        gfx.glyph_font(glyph, rr, font, if !enabled { off } else if hovered { t.text } else { t.dim });
    };
    // 이동 화살표는 가운데 크기(12px), 새 폴더·별·보기는 도구 아이콘 크기(16px)
    nav_btn(g.back, G_BACK, Font::GlyphMid, !tab.back.is_empty(), p_hover == PaneHover::Back);
    nav_btn(g.fwd, G_FWD, Font::GlyphMid, !tab.fwd.is_empty(), p_hover == PaneHover::Fwd);
    nav_btn(g.up, G_UP, Font::GlyphMid, up_ok(&tab.loc), p_hover == PaneHover::Up);
    // 시작 화면에는 새 폴더·보기 단추가 없다. 좁은 칸의 경로 입력 상자가 그 자리를 덮으면 그리지 않는다
    if s.tools && !(s.address && g.crumb.w() < ADDR_MIN_W) {
        nav_btn(g.new_folder, G_NEW_FOLDER, Font::Glyph, s.new_folder_ok, p_hover == PaneHover::NewFolder);
        // 보기 방식: 지금 보기 그림과 아래 화살표 (누르면 보기 메뉴)
        let view_hot = s.view_ok && (p_hover == PaneHover::View || s.view_menu_open);
        if view_hot {
            gfx.fill_round(g.view, R_CTRL, t.hover);
        }
        let view_glyph = if matches!(pane.view, ViewMode::Huge | ViewMode::Large | ViewMode::Medium) { G_GRID } else { G_LIST };
        let view_color = if !s.view_ok { off } else if view_hot { t.text } else { t.dim };
        gfx.glyph(view_glyph, R::new(g.view.l + 2.0, g.view.t, g.view.l + 26.0, g.view.b), false, view_color);
        gfx.glyph(G_CHEVRON_DOWN, R::new(g.view.r - 16.0, g.view.t + 1.0, g.view.r - 4.0, g.view.b + 1.0), true, t.faint);
    }

    // 경로: 상자 없이 글자만. 경로 위에 올리면 옅은 바탕(빈 곳을 누르면 경로 입력), 조각에 올리면 그 조각이 한 단계 진하게
    if s.address {
        let ar = addr_rect(&g);
        gfx.fill_round(ar, R_CTRL, t.field);
        gfx.stroke_round(ar.inset(0.75, 0.75), R_CTRL, t.accent, 1.5);
        if let Some(text) = s.address_text {
            gfx.text(text, R::new(ar.l + 8.0, ar.t, ar.r - 8.0, ar.b), Font::Path, t.text);
        }
    } else {
        let over_path = matches!(p_hover, PaneHover::CrumbEmpty | PaneHover::Crumb(_));
        if over_path {
            gfx.fill_round(g.crumb, R_CTRL, t.hover);
        }
        let cs = crumbs(gfx, g.crumbs, &tab.loc, s.root);
        let n = cs.len();
        // 별 단추: 이 폴더가 즐겨찾기에 있으면 채운 별(강조 색), 없으면 빈 별
        if let Some(on) = s.starred.filter(|_| g.star.w() > 0.0) {
            let hot = p_hover == PaneHover::Star;
            if hot {
                gfx.fill_round(g.star, R_CTRL, t.hover);
            }
            gfx.glyph(if on { G_STAR_FILL } else { G_STAR }, g.star, false, if on || hot { t.text } else { t.dim });
        }
        gfx.clip(g.crumbs);
        for (i, c) in cs.iter().enumerate() {
            let last = i + 1 == n;
            if p_hover == PaneHover::Crumb(i) {
                gfx.fill_round(R::new(c.x0, g.crumb.t + 2.0, c.x1, g.crumb.b - 2.0), R_ROW, t.sel_inactive);
            }
            if c.label == "…" && !last {
                gfx.glyph(G_MORE, R::new(c.x0, g.crumb.t, c.x1, g.crumb.b), true, t.dim);
            } else {
                gfx.text(&c.label, R::new(c.x0 + CRUMB_PAD, g.crumb.t, c.x1 - CRUMB_PAD / 2.0, g.crumb.b), Font::Normal, if last { t.text } else { t.dim });
            }
            if !last {
                gfx.glyph(G_CHEVRON, R::new(c.x1, g.crumb.t, c.x1 + CHEV_W, g.crumb.b), true, t.faint);
            }
        }
        gfx.unclip();
    }
}

/// 위로 갈 곳이 있는지: 폴더면 위가 있다 (드라이브 맨 위는 '내 PC', 맥의 / 는 위가 없다)
fn up_ok(loc: &Location) -> bool {
    match loc {
        Location::Dir(d) => !(d.starts_with('/') && util::parent(d).is_none()),
        _ => false,
    }
}

/// 열 머리글, 줄·칸, 안내 글, 네트워크 단추, 스크롤 막대, 네모 고르기, 불러오기 막대
pub fn paint_pane_list<G: Canvas>(gfx: &mut G, art: &mut impl EntryArt<G>, t: &Theme, s: &PaneScene) {
    let t = *t;
    let (p_hover, pane) = (s.hover, s.pane);
    let active = s.active;
    let keyboard_here = s.keyboard_here;
    let tab = pane.tab();
    let g = geom(s.r);
    let (short_dates, now) = (s.short_dates, s.now);

    // 열 머리글
    let cols = columns(g.list, pane.cols);
    let header = |gfx: &G, (l, rr): (f32, f32), label: &str, key: SortKey, right: bool| {
        let sorted = pane.sort.key == key;
        let color = if sorted || p_hover == PaneHover::Header(key) { t.text } else { t.dim };
        let tr = R::new(l + 8.0, g.header.t, rr - 8.0, g.header.b);
        gfx.text(label, tr, if right { Font::Right } else { Font::Normal }, color);
        if sorted {
            let glyph = if pane.sort.desc { G_SORT_DOWN } else { G_SORT_UP };
            let gx = if right { l } else { (tr.l + gfx.measure(label, Font::Normal) + 4.0).min(rr - 18.0) };
            gfx.glyph(glyph, R::new(gx, g.header.t, gx + 16.0, g.header.b), true, t.dim);
        }
    };
    if pane.view == ViewMode::Details {
        header(gfx, (cols.name.0 - 4.0, cols.name.1), "이름", SortKey::Name, false);
        if let Some(c) = cols.modified {
            header(gfx, c, "수정한 날짜", SortKey::Modified, false);
        }
        if let Some(c) = cols.kind {
            header(gfx, c, "유형", SortKey::Type, false);
        }
        if let Some(c) = cols.size {
            header(gfx, c, "크기", SortKey::Size, true);
        }
        gfx.fill(R::new(g.list.l + 8.0, g.header.b - 1.0, g.list.r - SCROLL_W - 6.0, g.header.b), t.border);
    } else {
        // 아이콘·목록 보기: 열 대신 정렬 버튼. 지금 기준은 바탕이 있는 버튼으로 보인다
        for (key, label, cr) in sort_chips(gfx, g.header, pane.sort.key) {
            let sorted = pane.sort.key == key;
            let hot = p_hover == PaneHover::Header(key);
            if sorted {
                gfx.fill_round(cr, cr.h() / 2.0, well(&t));
                if t.high_contrast {
                    gfx.stroke_round(cr.inset(0.5, 0.5), cr.h() / 2.0, t.border, 1.0);
                }
            } else if hot {
                gfx.fill_round(cr, cr.h() / 2.0, t.hover);
            }
            let tr = R::new(cr.l + 12.0, cr.t, cr.r - if sorted { 28.0 } else { 12.0 }, cr.b);
            gfx.text(label, tr, Font::Normal, if sorted || hot { t.text } else { t.dim });
            if sorted {
                let glyph = if pane.sort.desc { G_SORT_DOWN } else { G_SORT_UP };
                gfx.glyph(glyph, R::new(cr.r - 26.0, cr.t, cr.r - 10.0, cr.b), true, t.dim);
            }
        }
    }

    // 행·칸: 보이는 부분만 그린다
    gfx.clip(g.list);
    let grid = Grid::of(g.list, pane.view);
    // 여러 개를 골랐으면 그중 포커스가 어느 것인지도 보인다
    let many_selected = tab.selection_stats().0 > 1;
    for vi in grid.visible(g.list, tab.scroll, tab.view.len()) {
        let e = &tab.entries[tab.view[vi] as usize];
        let it = grid.item(vi, tab.scroll);
        // 선택(바탕)과 키보드 포커스(테두리)는 따로 보인다. 포커스는 활성 칸에만, 선택한 하나와 겹치면 바탕만으로
        let focused = keyboard_here && vi == tab.cursor && (!e.selected || many_selected);
        let fg = if e.selected && active { t.sel_text } else { t.text };
        let renaming_here = s.renaming && vi == tab.cursor;
        if grid.cell == CellKind::Icon {
            // 작은 아이콘·목록: 칸 하나에 작은 아이콘과 이름
            let cell = R::new(it.l + 1.0, it.t + 1.0, it.r - 3.0, it.b - 1.0);
            if e.selected {
                gfx.fill_round(cell, R_ROW, if active { t.sel } else { t.sel_inactive });
            } else if p_hover == PaneHover::Row(vi) {
                gfx.fill_round(cell, R_ROW, t.hover);
            }
            if s.drop_row == Some(vi) {
                gfx.stroke_round(cell.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
            } else if focused {
                gfx.stroke_round(cell.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
            }
            let iy = it.t + (ROW_H - 16.0) / 2.0;
            art.icon(gfx, e, R::new(it.l + 4.0, iy, it.l + 20.0, iy + 16.0));
            if renaming_here {
                let er = R::new(it.l + 24.0, it.t + 2.0, it.r - 6.0, it.b - 2.0);
                gfx.fill_round(er, R_ROW, t.pane);
                gfx.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
            } else {
                let fg = if e.is_hidden() && !(e.selected && active) { t.faint } else { fg };
                gfx.text_w(e.name(), R::new(it.l + 28.0, it.t, it.r - 8.0, it.b), Font::Normal, fg);
            }
            continue;
        }
        if let CellKind::Tile(thumb) = grid.cell {
            let tile = R::new(it.l + 4.0, it.t + 4.0, it.r - 4.0, it.b - 4.0);
            if e.selected {
                gfx.fill_round(tile, R_ROW, if active { t.sel } else { t.sel_inactive });
            } else if p_hover == PaneHover::Row(vi) {
                gfx.fill_round(tile, R_ROW, t.hover);
            }
            if s.drop_row == Some(vi) {
                gfx.stroke_round(tile.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
            } else if focused {
                gfx.stroke_round(tile.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
            }
            let cx = (it.l + it.r) / 2.0;
            let bx = R::new(cx - thumb / 2.0, it.t + 8.0, cx + thumb / 2.0, it.t + 8.0 + thumb);
            let opacity = if e.is_hidden() { 0.55 } else { 1.0 };
            if !art.thumbnail(gfx, tab, e, bx, thumb, opacity) {
                // 썸네일이 오기 전까지는 작은 아이콘
                let my = (bx.t + bx.b) / 2.0;
                art.icon(gfx, e, R::new(cx - 16.0, my - 16.0, cx + 16.0, my + 16.0));
            }
            let cap = R::new(tile.l + 4.0, bx.b + 6.0, tile.r - 4.0, tile.b - 2.0);
            if renaming_here {
                let er = R::new(it.l + 4.0, bx.b + 6.0, it.r - 8.0, bx.b + 30.0);
                gfx.fill_round(er, R_ROW, t.pane);
                gfx.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
            } else {
                gfx.text_w(e.name(), cap, Font::Caption, if e.is_hidden() { t.faint } else { fg });
            }
            continue;
        }
        let y = it.t;
        let row = R::new(g.list.l + 8.0, y + 1.0, g.list.r - SCROLL_W - 6.0, y + ROW_H - 1.0);
        if e.selected {
            gfx.fill_round(row, R_ROW, if active { t.sel } else { t.sel_inactive });
        } else if p_hover == PaneHover::Row(vi) {
            gfx.fill_round(row, R_ROW, t.hover);
        }
        if s.drop_row == Some(vi) {
            gfx.stroke_round(row.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
        } else if focused {
            gfx.stroke_round(row.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
        }
        // 고대비에서는 선택 바탕(Highlight) 위 글자를 모두 HighlightText로
        let dim = if e.selected && active && t.high_contrast { t.sel_text } else { t.dim };
        art.icon(gfx, e, R::new(cols.name.0 + 4.0, y + (ROW_H - 16.0) / 2.0, cols.name.0 + 20.0, y + (ROW_H + 16.0) / 2.0));
        let fg = if e.is_hidden() && !(e.selected && active) { t.faint } else { fg };
        gfx.text_w(e.name(), R::new(cols.name.0 + 28.0, y, cols.name.1 - 8.0, y + ROW_H), Font::Normal, fg);
        if let Some((l, rr)) = cols.modified {
            let s = match e.kind {
                Kind::Drive if e.size > 0 => format!("{} 사용 가능", format_size(e.free)),
                Kind::File | Kind::Dir if short_dates => format_time_short(e.modified, now),
                Kind::File | Kind::Dir => format_time(e.modified),
                _ => String::new(),
            };
            gfx.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Normal, dim);
        }
        if let Some((l, rr)) = cols.kind {
            let s = if e.kind == Kind::Drive { drive_type_label(e.drive_type).to_string() } else { art.type_name(e) };
            gfx.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Normal, dim);
        }
        if let Some((l, rr)) = cols.size {
            let s = match e.kind {
                Kind::File => format_size(e.size),
                Kind::Drive if e.size > 0 => format_size(e.size),
                _ => String::new(),
            };
            gfx.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Right, dim);
        }
        if renaming_here {
            let er = R::new(cols.name.0 + 24.0, y + 2.0, cols.name.1 - 4.0, y + ROW_H - 2.0);
            gfx.fill_round(er, R_ROW, t.pane);
            gfx.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
        }
    }
    gfx.unclip();

    if let Some(h) = &s.hint {
        gfx.text(h, R::new(g.list.l + 24.0, g.list.t + 24.0, g.list.r - 24.0, g.list.t + 110.0), Font::Hint, t.faint);
    }
    // 로그인·다시 연결: 이 칸의 할 일 하나뿐이라 강조 색 단추
    if let Some((b, label)) = s.net_button {
        let k = if p_hover == PaneHover::NetFix { 0.88 } else { 1.0 };
        gfx.fill_round(b, R_CTRL, Color { r: t.accent.r * k, g: t.accent.g * k, b: t.accent.b * k, a: 1.0 });
        let lw = gfx.measure(label, Font::Normal);
        gfx.text(label, R::new(b.l + (b.w() - lw) / 2.0, b.t, b.r, b.b), Font::Normal, if t.dark { t.window } else { Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 } });
    }
    // 연결이 끊겨 기억한 목록을 보이는 중이면 흐리게
    if tab.offline {
        gfx.fill(g.list, alpha(t.pane, 0.45));
    }

    if let Some((_, thumb)) = grid.bar(g.list, tab.view.len(), tab.scroll) {
        let hot = s.scroll_drag || p_hover == PaneHover::Scroll;
        let thumb = if hot { thumb } else { thumb.inset(1.0, 0.0) };
        gfx.fill_round(thumb, thumb.w() / 2.0, if hot { t.dim } else { t.thumb });
    }

    if let Some(m) = s.marquee {
        gfx.clip(g.list);
        gfx.fill(m, alpha(t.accent, 0.15));
        gfx.stroke_round(m.inset(0.5, 0.5), 1.0, t.accent, 1.0);
        gfx.unclip();
    }

    // 늦게 읽히는 폴더: 머리글 아래를 오가는 가는 막대 (0.3초 안에 끝나면 보이지 않는다, 애니메이션을 끄면 글자만)
    if let Some(phase) = s.loading {
        let track = R::new(g.list.l + 8.0, g.header.b - 2.0, g.list.r - SCROLL_W - 6.0, g.header.b);
        let seg = track.w() * 0.3;
        let x = track.l - seg + (track.w() + seg) * phase;
        gfx.clip(track);
        gfx.fill_round(R::new(x, track.t, x + seg, track.b), 1.0, t.accent);
        gfx.unclip();
    }
}

/// 칸 테두리 (맨 마지막에 그린다)
pub fn paint_pane_border(gfx: &impl Canvas, t: &Theme, s: &PaneScene) {
    let (border_w, border_c) = border(t, s);
    if border_w > 0.0 {
        gfx.stroke_round(s.r.inset(border_w / 2.0, border_w / 2.0), RADIUS, border_c, border_w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::Kind;
    use crate::pane::{DEFAULT_COLS, LoadState, Sort};
    use std::cell::RefCell;

    /// 그린 글자만 적어 두는 가짜 그림판 (글자 폭은 한 글자에 7)
    #[derive(Default)]
    struct Rec {
        texts: RefCell<Vec<String>>,
    }

    impl Canvas for Rec {
        fn fill(&self, _: R, _: Color) {}
        fn fill_round(&self, _: R, _: f32, _: Color) {}
        fn stroke_round(&self, _: R, _: f32, _: Color, _: f32) {}
        fn text(&self, s: &str, _: R, _: Font, _: Color) {
            self.texts.borrow_mut().push(s.to_string());
        }
        fn measure(&self, s: &str, _: Font) -> f32 {
            s.chars().count() as f32 * 7.0
        }
        fn glyph(&self, _: u16, _: R, _: bool, _: Color) {}
        fn glyph_font(&self, _: u16, _: R, _: Font, _: Color) {}
        fn clip(&self, _: R) {}
        fn unclip(&self) {}
    }

    /// 유형 이름을 몇 번 물었는지 센다
    #[derive(Default)]
    struct Art {
        type_names: usize,
    }

    impl EntryArt<Rec> for Art {
        fn icon(&mut self, _: &mut Rec, _: &Entry, _: R) {}
        fn thumbnail(&mut self, _: &mut Rec, _: &Tab, _: &Entry, _: R, _: f32, _: f32) -> bool {
            false
        }
        fn type_name(&mut self, _: &Entry) -> String {
            self.type_names += 1;
            "유형".to_string()
        }
    }

    /// 파일 n개가 든 폴더 탭 하나와, 이름이 다른 빈 탭 하나
    fn pane(n: usize) -> Pane {
        let mut tab = Tab::new(1, Location::Dir("C:\\자료".to_string()));
        tab.entries = (0..n).map(|i| Entry::from_str(&format!("파일{i:03}.txt"), Kind::File)).collect();
        tab.view = (0..n as u32).collect();
        tab.state = LoadState::Ready;
        let other = Tab::new(2, Location::Dir("C:\\숨은탭".to_string()));
        Pane { tabs: vec![tab, other], active: 0, sort: Sort::default(), cols: DEFAULT_COLS, view: ViewMode::Details }
    }

    fn scene(pane: &Pane, r: R, tabs: Vec<R>) -> PaneScene<'_> {
        PaneScene {
            r,
            pane,
            active: true,
            multi: false,
            keyboard_here: true,
            hover: PaneHover::None,
            tabs,
            add: R::new(r.l + 200.0, r.t + 8.0, r.l + 228.0, r.t + 36.0),
            count_text: "10개".to_string(),
            count_rect: R::new(r.r - 60.0, r.t + 8.0, r.r - 8.0, r.t + 36.0),
            count_hot: false,
            filter: None,
            starred: Some(false),
            root: None,
            address: false,
            address_text: None,
            tools: true,
            new_folder_ok: true,
            view_ok: true,
            view_menu_open: false,
            renaming: false,
            dragged_tab: None,
            tab_drop: None,
            drop_on_pane: false,
            drop_row: None,
            marquee: None,
            scroll_drag: false,
            hint: None,
            net_button: None,
            loading: None,
            short_dates: true,
            now: 0,
        }
    }

    #[test]
    fn hidden_tab_chips_are_not_drawn() {
        let p = pane(0);
        let r = R::new(0.0, 0.0, 800.0, 600.0);
        // 둘째 탭은 칸이 좁아 가려졌다 (폭 0)
        let tabs = vec![R::new(8.0, 8.0, 120.0, 36.0), R::new(120.0, 8.0, 120.0, 36.0)];
        let g = Rec::default();
        paint_pane_head(&g, &Theme::LIGHT, &scene(&p, r, tabs));
        let texts = g.texts.borrow();
        assert!(texts.contains(&p.tabs[0].loc.title()));
        assert!(!texts.contains(&p.tabs[1].loc.title()));
    }

    #[test]
    fn only_visible_rows_are_drawn_once_with_their_type() {
        let p = pane(200);
        // 유형 열이 생기는 폭 (590 이상), 목록 높이에 줄 몇 개만 보인다
        let r = R::new(0.0, 0.0, 800.0, 400.0);
        let mut g = Rec::default();
        let mut art = Art::default();
        paint_pane_list(&mut g, &mut art, &Theme::LIGHT, &scene(&p, r, Vec::new()));
        let texts = g.texts.borrow();
        let names: Vec<&String> = texts.iter().filter(|s| s.starts_with("파일")).collect();
        let visible = Grid::of(geom(r).list, ViewMode::Details).visible(geom(r).list, 0.0, 200).count();
        assert!(visible > 0 && visible < 200);
        assert_eq!(names.len(), visible);
        assert!(names.windows(2).all(|w| w[0] != w[1]));
        assert_eq!(art.type_names, visible);
    }
}
