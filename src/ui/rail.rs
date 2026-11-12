//! 작업 공간 레일: 창 왼쪽에 세로로 놓인 공간 아이콘. 공간마다 색과 칸 나누기 모양이 있고, 그 아래에 새 공간(+)이 온다.
//! Windows는 맨 아래에 설정 단추를 둔다. 자리 계산과 그리기를 Windows와 맥이 함께 쓴다. 누르면 무엇을 할지는 각 앱이 정한다

use super::glyph::*;
use super::*;
use crate::palette::{alpha, apca, mix};

/// 공간 아이콘 한 변과 아이콘 사이 간격을 더한 거리
pub const RAIL_ITEM: f32 = 32.0;
pub const RAIL_PITCH: f32 = 40.0;
const RAIL_R: f32 = 10.0;
/// 지금 공간의 테두리 고리: 아이콘에서 떨어진 거리
const RING_GAP: f32 = 3.0;

/// 레일의 자리
#[derive(Clone, Debug, PartialEq)]
pub struct RailGeom {
    /// 보이는 공간 아이콘 (공간 번호, 자리). 창이 낮아 다 들어가지 않으면 지금 공간이 보이는 범위만 남는다
    pub spaces: Vec<(usize, R)>,
    /// 새 공간(+)
    pub add: R,
    /// 설정 단추 (Windows)
    pub settings: Option<R>,
}

fn item(y: f32) -> R {
    let l = (BAR_W - RAIL_ITEM) / 2.0;
    R::new(l, y, l + RAIL_ITEM, y + RAIL_ITEM)
}

/// top: 첫 아이콘의 위쪽, bottom: 레일의 아래 끝, n: 공간 수, current: 지금 공간
pub fn rail_geom(top: f32, bottom: f32, n: usize, current: usize, settings: bool) -> RailGeom {
    let settings = settings.then(|| item(bottom - 8.0 - RAIL_ITEM));
    let end = settings.map_or(bottom - 8.0, |s| s.t - 8.0);
    // + 단추 자리를 뺀 나머지에 들어가는 공간 수 (적어도 하나: 지금 공간)
    let room = (((end - top) / RAIL_PITCH).floor() as usize).saturating_sub(1).max(1);
    let first = if n <= room { 0 } else { (current + 1).saturating_sub(room).min(n - room) };
    let shown = n.min(room);
    let spaces = (0..shown).map(|k| (first + k, item(top + k as f32 * RAIL_PITCH))).collect();
    RailGeom { spaces, add: item(top + shown as f32 * RAIL_PITCH), settings }
}

/// 공간 하나의 모습
#[derive(Clone, Copy, Debug)]
pub struct RailSpace {
    pub color: Color,
    pub layout: Layout,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RailHover {
    None,
    Space(usize),
    Add,
    Settings,
}

pub struct RailScene<'a> {
    pub geom: &'a RailGeom,
    /// 모든 공간 (번호는 geom.spaces의 공간 번호)
    pub spaces: &'a [RailSpace],
    pub current: usize,
    pub hover: RailHover,
    /// 파일을 끌어 올려 둔 공간 (놓으면 그 공간에 고정된다)
    pub drop: Option<usize>,
}

/// 색 면 위에 올릴 그림 색: 흰색과 검정 가운데 더 또렷한 쪽
fn ink_on(bg: Color) -> Color {
    let white = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    let black = Color { r: 0.07, g: 0.08, b: 0.10, a: 1.0 };
    if apca(white, bg) >= apca(black, bg) { white } else { black }
}

pub fn paint_rail(g: &impl Canvas, t: &Theme, s: &RailScene) {
    for (i, r) in &s.geom.spaces {
        let Some(space) = s.spaces.get(*i) else { continue };
        let hovered = s.hover == RailHover::Space(*i);
        let ring = R::new(r.l - RING_GAP, r.t - RING_GAP, r.r + RING_GAP, r.b + RING_GAP);
        if t.high_contrast {
            // 고대비: 지금 공간은 강조 색 면, 나머지는 테두리만
            if *i == s.current {
                g.fill_round(*r, RAIL_R, t.accent);
            } else {
                g.stroke_round(r.inset(0.5, 0.5), RAIL_R, if hovered { t.focus } else { t.border }, 1.0);
            }
            draw_layout_icon(g, *r, space.layout, if *i == s.current { t.sel_text } else { t.text });
        } else if *i == s.current {
            // 지금 공간: 공간 색으로 채우고 같은 색 고리를 두른다
            g.stroke_round(ring.inset(0.75, 0.75), RAIL_R + RING_GAP, space.color, 1.5);
            g.fill_round(*r, RAIL_R, space.color);
            draw_layout_icon(g, *r, space.layout, ink_on(space.color));
        } else {
            // 다른 공간: 창 바탕에 공간 색을 옅게 섞은 면에 공간 색 그림
            g.fill_round(*r, RAIL_R, mix(t.window, space.color, if hovered { 0.30 } else { 0.18 }));
            draw_layout_icon(g, *r, space.layout, mix(space.color, t.text, if t.dark { 0.0 } else { 0.15 }));
        }
        if s.drop == Some(*i) {
            g.stroke_round(ring.inset(0.75, 0.75), RAIL_R + RING_GAP, t.accent, 1.5);
        }
    }
    let plain = |r: R, glyph: u16, hovered: bool, filled: bool| {
        if t.high_contrast {
            if hovered {
                g.fill_round(r, RAIL_R, t.hover);
            }
        } else if filled || hovered {
            g.fill_round(r, RAIL_R, alpha(t.text, if hovered { 0.10 } else { 0.05 }));
        }
        g.glyph(glyph, r, false, if hovered { t.text } else { t.dim });
    };
    plain(s.geom.add, G_ADD, s.hover == RailHover::Add, true);
    if let Some(r) = s.geom.settings {
        plain(r, G_SETTINGS, s.hover == RailHover::Settings, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// 넉넉한 창: 공간이 차례로 놓이고 그 아래 +, 맨 아래 설정. 서로 겹치지 않는다
    #[test]
    fn spaces_stack_above_add_and_settings_stays_at_the_bottom() {
        let m = rail_geom(10.0, 600.0, 3, 0, true);
        assert_eq!(m.spaces.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1, 2]);
        let mut order: Vec<R> = m.spaces.iter().map(|(_, r)| *r).collect();
        order.push(m.add);
        order.push(m.settings.unwrap());
        assert!(order.windows(2).all(|w| w[0].b <= w[1].t), "{order:?}");
        assert_eq!((m.spaces[0].1.t, m.settings.unwrap().b), (10.0, 592.0));
        assert!(order.iter().all(|r| r.l >= 0.0 && r.r <= BAR_W && r.w() == RAIL_ITEM));
        assert_eq!(rail_geom(52.0, 600.0, 3, 0, false).settings, None);
    }

    /// 낮은 창: 다 들어가지 않으면 지금 공간이 보이는 범위만 남고, +와 설정을 가리지 않는다
    #[test]
    fn a_short_window_keeps_the_current_space_in_view() {
        for current in 0..9 {
            let m = rail_geom(10.0, 260.0, 9, current, true);
            let shown: Vec<usize> = m.spaces.iter().map(|(i, _)| *i).collect();
            assert!(shown.contains(&current), "current {current} of {shown:?}");
            assert!(shown.windows(2).all(|w| w[1] == w[0] + 1));
            assert!(m.add.b <= m.settings.unwrap().t, "current {current}");
        }
        // 아주 낮아도 지금 공간 하나는 남는다
        assert_eq!(rail_geom(10.0, 60.0, 4, 2, true).spaces.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [2]);
    }

    #[derive(Default)]
    struct Rec {
        fills: RefCell<Vec<(R, Color)>>,
        strokes: RefCell<Vec<R>>,
    }

    impl Canvas for Rec {
        fn fill(&self, _: R, _: Color) {}
        fn fill_round(&self, r: R, _: f32, c: Color) {
            self.fills.borrow_mut().push((r, c));
        }
        fn stroke_round(&self, r: R, _: f32, _: Color, _: f32) {
            self.strokes.borrow_mut().push(r);
        }
        fn text(&self, _: &str, _: R, _: Font, _: Color) {}
        fn measure(&self, _: &str, _: Font) -> f32 {
            0.0
        }
        fn glyph(&self, _: u16, _: R, _: bool, _: Color) {}
        fn glyph_font(&self, _: u16, _: R, _: Font, _: Color) {}
        fn clip(&self, _: R) {}
        fn unclip(&self) {}
    }

    /// 지금 공간만 공간 색으로 채우고 고리를 두른다. 다른 공간은 옅은 면이다
    #[test]
    fn only_the_current_space_is_filled_with_its_color() {
        let t = Theme::LIGHT;
        let purple = Color { r: 0.48, g: 0.30, b: 0.88, a: 1.0 };
        let orange = Color { r: 0.85, g: 0.45, b: 0.10, a: 1.0 };
        let spaces = [RailSpace { color: purple, layout: Layout::TwoV }, RailSpace { color: orange, layout: Layout::Three }];
        let m = rail_geom(10.0, 600.0, 2, 1, false);
        let g = Rec::default();
        paint_rail(&g, &t, &RailScene { geom: &m, spaces: &spaces, current: 1, hover: RailHover::None, drop: None });
        let fills = g.fills.borrow();
        let fill_of = |r: R| fills.iter().find(|(x, _)| *x == r).map(|(_, c)| *c).unwrap();
        assert_eq!(fill_of(m.spaces[1].1), orange);
        let other = fill_of(m.spaces[0].1);
        assert!(other != purple && other.r > purple.r, "the other space is a soft tint: {other:?}");
        // 고리는 지금 공간 둘레에만
        let rings = g.strokes.borrow();
        assert!(rings.iter().any(|r| r.t < m.spaces[1].1.t && r.b > m.spaces[1].1.b));
        assert!(!rings.iter().any(|r| r.t < m.spaces[0].1.t && r.b > m.spaces[0].1.b && r.b < m.spaces[1].1.t));
    }
}
