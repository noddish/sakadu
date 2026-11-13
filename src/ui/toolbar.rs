//! 도구 막대: 창 위쪽 한 줄. 뒤로·앞으로(·위로), 지금 폴더 이름, 칸 나누기·보기 방식 단추, 새 폴더, 미리 보기, 검색, 더 보기.
//! 자리 계산과 그리기를 Windows와 맥이 함께 쓴다. 누르면 무엇을 할지는 각 앱이 정한다

use super::glyph::*;
use super::*;
use crate::palette::alpha;

/// 단추 높이와 폭
pub const TOOL_H: f32 = 32.0;
pub const TOOL_W: f32 = 36.0;
/// 단추 묶음 사이 간격
const TOOL_GAP: f32 = 8.0;
/// 고르는 단추(칸 나누기·보기 방식): 지금 모양 그림과 아래 화살표
const PICK_W: f32 = 52.0;
/// 단추 묶음의 모서리
const TOOL_R: f32 = 10.0;

/// 무엇을 둘지 (OS마다 다르다)
#[derive(Clone, Copy, Debug)]
pub struct ToolbarSpec {
    /// 쓸 수 있는 왼쪽·오른쪽 끝. 칸 카드의 가장자리와 맞춘다 (Windows는 오른쪽에서 창 단추 자리를 뺀다)
    pub left: f32,
    pub right: f32,
    /// 위로 단추 (Windows)
    pub up: bool,
    pub new_folder: bool,
    pub preview: bool,
    /// 검색 (누르면 검색 패널이 열린다)
    pub search: bool,
    /// 더 보기 `…` (Windows: 명령 찾기)
    pub more: bool,
}

/// 단추 자리. 폭이 0인 것은 없는 단추다 (그리지도 누르지도 않는다)
pub struct ToolbarGeom {
    /// 뒤로·앞으로(·위로)를 담는 판
    pub nav: R,
    pub back: R,
    pub fwd: R,
    pub up: R,
    /// 폴더 이름 자리 (오른쪽 단추 앞까지)
    pub title: R,
    pub layout: R,
    pub view: R,
    /// 새 폴더·미리 보기를 담는 판
    pub tools: R,
    pub new_folder: R,
    pub preview: R,
    pub search: R,
    pub more: R,
}

/// 좁아서 다 들어가지 않으면 더 보기, 미리 보기, 새 폴더, 검색, 위로 순으로 덜어 낸다 (모두 단축키나 메뉴로 된다).
/// 뒤로·앞으로와 칸 나누기·보기 방식은 늘 남는다
pub fn toolbar_geom(spec: &ToolbarSpec) -> ToolbarGeom {
    let mut s = *spec;
    let need = |s: &ToolbarSpec| {
        let nav = TOOL_W * if s.up { 3.0 } else { 2.0 };
        let tools = TOOL_W * (s.new_folder as u8 + s.preview as u8) as f32;
        let singles = (s.search as u8 + s.more as u8) as f32;
        let gaps = TOOL_GAP * (2.0 + (tools > 0.0) as u8 as f32 + singles);
        nav + PICK_W * 2.0 + tools + TOOL_W * singles + gaps
    };
    for drop in 0..5 {
        if need(&s) <= s.right - s.left {
            break;
        }
        match drop {
            0 => s.more = false,
            1 => s.preview = false,
            2 => s.new_folder = false,
            3 => s.search = false,
            _ => s.up = false,
        }
    }
    let (t, b) = ((WORKBAR_H - TOOL_H) / 2.0, (WORKBAR_H + TOOL_H) / 2.0);
    let back = R::new(s.left, t, s.left + TOOL_W, b);
    let fwd = R::new(back.r, t, back.r + TOOL_W, b);
    let up = R::new(fwd.r, t, fwd.r + if s.up { TOOL_W } else { 0.0 }, b);
    let nav = R::new(back.l, t, up.r, b);
    // 오른쪽 끝부터 왼쪽으로 놓는다
    let mut x = s.right;
    let more = R::new(x - if s.more { TOOL_W } else { 0.0 }, t, x, b);
    if s.more {
        x = more.l - TOOL_GAP;
    }
    let search = R::new(x - if s.search { TOOL_W } else { 0.0 }, t, x, b);
    if s.search {
        x = search.l - TOOL_GAP;
    }
    let preview = R::new(x - if s.preview { TOOL_W } else { 0.0 }, t, x, b);
    let new_folder = R::new(preview.l - if s.new_folder { TOOL_W } else { 0.0 }, t, preview.l, b);
    let tools = R::new(new_folder.l, t, preview.r, b);
    if tools.w() > 0.0 {
        x = tools.l - TOOL_GAP;
    }
    let view = R::new(x - PICK_W, t, x, b);
    let layout = R::new(view.l - TOOL_GAP - PICK_W, t, view.l - TOOL_GAP, b);
    let title_l = nav.r + 12.0;
    let title = R::new(title_l, t, (layout.l - 12.0).max(title_l), b);
    ToolbarGeom { nav, back, fwd, up, title, layout, view, tools, new_folder, preview, search, more }
}

/// 도구 막대에서 마우스가 올라간 곳 (앱의 Hit을 그리기 직전에 바꿔 넘긴다)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ToolbarHover {
    None,
    Back,
    Fwd,
    Up,
    Layout,
    View,
    NewFolder,
    Preview,
    Search,
    More,
}

/// 도구 막대를 그리는 데 필요한 것 (앱이 자기 상태로 채운다)
pub struct ToolbarScene<'a> {
    pub geom: &'a ToolbarGeom,
    /// 지금 칸의 폴더 이름
    pub title: &'a str,
    pub layout: Layout,
    pub view: ViewMode,
    pub can_back: bool,
    pub can_fwd: bool,
    pub can_up: bool,
    pub new_folder_ok: bool,
    /// 보기 방식을 고를 수 있는지 (시작 화면에는 없다)
    pub view_ok: bool,
    pub preview_on: bool,
    /// 검색 패널이 열려 있다
    pub search_on: bool,
    /// 칸 나누기·보기 방식 메뉴가 열려 있다
    pub layout_open: bool,
    pub view_open: bool,
    pub hover: ToolbarHover,
}

/// 위로 갈 곳이 있는지: 폴더면 위가 있다 (드라이브 맨 위는 '내 PC', 맥의 / 는 위가 없다)
pub fn can_go_up(loc: &Location) -> bool {
    match loc {
        Location::Dir(d) => !(d.starts_with('/') && util::parent(d).is_none()),
        _ => false,
    }
}

pub fn paint_toolbar(g: &impl Canvas, t: &Theme, s: &ToolbarScene) {
    let t = *t;
    let m = s.geom;
    let off = if t.high_contrast { t.faint } else { alpha(t.dim, 0.45) };
    let track = seg_track(&t);
    // 단추 하나: 올리면 판 안쪽에 옅은 바탕. 쓸 수 없으면 흐리게
    let button = |r: R, enabled: bool, hovered: bool| -> Color {
        if hovered && enabled {
            g.fill_round(r.inset(3.0, 3.0), TOOL_R - 3.0, hover_fill(&t));
        }
        if !enabled {
            off
        } else if hovered {
            t.text
        } else {
            t.dim
        }
    };

    // 뒤로·앞으로(·위로)
    g.fill_round(m.nav, TOOL_R, track);
    for (r, glyph, enabled, h) in [(m.back, G_BACK, s.can_back, ToolbarHover::Back), (m.fwd, G_FWD, s.can_fwd, ToolbarHover::Fwd), (m.up, G_UP, s.can_up, ToolbarHover::Up)] {
        if r.w() > 0.0 {
            let c = button(r, enabled, s.hover == h);
            g.glyph_font(glyph, r, Font::GlyphMid, c);
        }
    }
    if m.title.w() > 0.0 {
        g.text(s.title, m.title, Font::Strong, t.text);
    }

    // 고르는 단추: 지금 모양 그림과 아래 화살표. 메뉴가 열려 있으면 옅은 강조 색
    let pick = |r: R, open: bool, enabled: bool, hovered: bool| -> Color {
        g.fill_round(r, TOOL_R, if open { t.accent_soft } else { track });
        let c = if open { t.chip_text } else { button(r, enabled, hovered) };
        g.glyph(G_CHEVRON_DOWN, R::new(r.r - 20.0, r.t + 1.0, r.r - 6.0, r.b + 1.0), true, if open { c } else if enabled { t.faint } else { off });
        c
    };
    let c = pick(m.layout, s.layout_open, true, s.hover == ToolbarHover::Layout);
    draw_layout_icon(g, R::new(m.layout.l + 8.0, m.layout.t, m.layout.l + 32.0, m.layout.b), s.layout, c);
    let c = pick(m.view, s.view_open, s.view_ok, s.hover == ToolbarHover::View);
    draw_view_icon(g, R::new(m.view.l + 8.0, m.view.t, m.view.l + 32.0, m.view.b), s.view, c);

    // 새 폴더·미리 보기
    if m.tools.w() > 0.0 {
        g.fill_round(m.tools, TOOL_R, track);
    }
    if m.new_folder.w() > 0.0 {
        let c = button(m.new_folder, s.new_folder_ok, s.hover == ToolbarHover::NewFolder);
        g.glyph(G_NEW_FOLDER, m.new_folder, false, c);
    }
    if m.preview.w() > 0.0 {
        // 켜져 있으면 옅은 강조 색 바탕
        if s.preview_on {
            g.fill_round(m.preview.inset(3.0, 3.0), TOOL_R - 3.0, t.accent_soft);
            g.glyph(G_PREVIEW, m.preview, false, t.chip_text);
        } else {
            let c = button(m.preview, true, s.hover == ToolbarHover::Preview);
            g.glyph(G_PREVIEW, m.preview, false, c);
        }
    }
    if m.search.w() > 0.0 {
        // 검색 패널이 열려 있으면 옅은 강조 색 바탕
        g.fill_round(m.search, TOOL_R, if s.search_on { t.accent_soft } else { track });
        let c = if s.search_on { t.chip_text } else { button(m.search, true, s.hover == ToolbarHover::Search) };
        g.glyph(G_SEARCH, m.search, false, c);
    }
    if m.more.w() > 0.0 {
        g.fill_round(m.more, TOOL_R, track);
        let c = button(m.more, true, s.hover == ToolbarHover::More);
        g.glyph(G_MORE, m.more, false, c);
    }
}

/// 보기 방식 그림 (16×16, r 가운데). 아이콘 글꼴에 여섯 가지가 한 벌로 없어서 직접 그린다
pub fn draw_view_icon(g: &impl Canvas, r: R, view: ViewMode, c: Color) {
    let (x, y) = ((r.l + r.r) / 2.0 - 8.0, (r.t + r.b) / 2.0 - 8.0);
    let sq = |dx: f32, dy: f32, s: f32| R::new(x + dx, y + dy, x + dx + s, y + dy + s);
    // 가로 선: (dx0, dx1)을 dy 높이에, 두께 1.5
    let line = |dx0: f32, dx1: f32, dy: f32| g.fill_round(R::new(x + dx0, y + dy - 0.75, x + dx1, y + dy + 0.75), 0.75, c);
    match view {
        ViewMode::Huge => g.stroke_round(sq(2.75, 2.75, 10.5), 2.5, c, 1.5),
        ViewMode::Large => {
            for (dx, dy) in [(2.2, 2.2), (9.2, 2.2), (2.2, 9.2), (9.2, 9.2)] {
                g.stroke_round(sq(dx, dy, 4.6), 1.3, c, 1.4);
            }
        }
        ViewMode::Medium => {
            for dy in [2.0, 6.5, 11.0] {
                for dx in [2.0, 6.5, 11.0] {
                    g.fill_round(sq(dx, dy, 3.0), 0.8, c);
                }
            }
        }
        // 작은 아이콘: 네모와 짧은 이름이 두 줄로
        ViewMode::Small => {
            for dy in [4.0, 8.0, 12.0] {
                for dx in [1.8, 9.0] {
                    g.fill_round(sq(dx, dy - 1.2, 2.4), 0.6, c);
                    line(dx + 3.4, dx + 5.6, dy);
                }
            }
        }
        // 목록: 점과 긴 이름
        ViewMode::List => {
            for dy in [3.5, 8.0, 12.5] {
                g.fill_round(sq(1.7, dy - 1.1, 2.2), 1.1, c);
                line(5.4, 14.3, dy);
            }
        }
        // 자세히: 이름과 다른 열, 그 사이 열 구분선
        ViewMode::Details => {
            for dy in [3.5, 8.0, 12.5] {
                line(1.5, 7.9, dy);
                line(10.1, 14.5, dy);
            }
            g.fill(R::new(x + 8.5, y + 2.0, x + 9.5, y + 14.0), Color { a: c.a * 0.45, ..c });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(left: f32, right: f32) -> ToolbarSpec {
        ToolbarSpec { left, right, up: true, new_folder: true, preview: true, search: true, more: true }
    }

    /// 넓은 창: 모든 단추가 있고 왼쪽부터 순서대로 겹치지 않는다
    #[test]
    fn wide_toolbar_keeps_every_button_in_order() {
        let m = toolbar_geom(&spec(300.0, 1200.0));
        let order = [m.back, m.fwd, m.up, m.title, m.layout, m.view, m.new_folder, m.preview, m.search, m.more];
        assert!(order.iter().all(|r| r.w() > 0.0));
        assert!(order.windows(2).all(|w| w[0].r <= w[1].l), "buttons overlap");
        assert_eq!((m.back.l, m.more.r), (300.0, 1200.0));
        assert_eq!((m.tools.l, m.tools.r), (m.new_folder.l, m.preview.r));
    }

    /// 좁은 창: 덜 중요한 단추부터 빠지고, 남은 단추는 겹치지 않고 범위 안에 있다
    #[test]
    fn narrow_toolbar_drops_buttons_instead_of_overlapping() {
        for w in [520.0, 400.0, 340.0, 300.0, 260.0, 220.0, 200.0] {
            let m = toolbar_geom(&spec(100.0, 100.0 + w));
            let kept: Vec<R> = [m.back, m.fwd, m.up, m.layout, m.view, m.new_folder, m.preview, m.search, m.more].into_iter().filter(|r| r.w() > 0.0).collect();
            assert!(kept.windows(2).all(|p| p[0].r <= p[1].l), "w={w}");
            assert!(kept.iter().all(|r| r.l >= 100.0 && r.r <= 100.0 + w), "w={w}");
            assert!(m.back.w() > 0.0 && m.layout.w() > 0.0 && m.view.w() > 0.0, "w={w}");
        }
        // 더 보기가 가장 먼저 빠지고, 검색은 새 폴더·미리 보기보다 오래 남는다
        let m = toolbar_geom(&spec(0.0, 340.0));
        assert!(m.more.w() == 0.0 && m.back.w() > 0.0);
        let m = toolbar_geom(&spec(0.0, 280.0));
        assert!(m.tools.w() == 0.0 && m.search.w() > 0.0, "search outlives new folder and preview");
    }

    #[test]
    fn up_is_off_at_the_mac_root_and_on_drive_roots() {
        // 맥 경로(/)는 맥에서만 맨 위로 본다
        #[cfg(not(windows))]
        {
            assert!(!can_go_up(&Location::Dir("/".to_string())));
            assert!(can_go_up(&Location::Dir("/Users".to_string())));
        }
        assert!(can_go_up(&Location::Dir("C:\\".to_string())));
        assert!(!can_go_up(&Location::Computer));
    }
}
