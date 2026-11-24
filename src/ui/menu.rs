//! 사카두 메뉴: 단추 메뉴(칸 나누기, 보기 방식)와 오른쪽 단추 메뉴를 운영 체제 기본 메뉴 대신 같은 모양으로 직접 그린다.
//! 흰 카드(떠 있는 다른 것과 같은 그림자와 둥근 모서리)에 줄을 놓고, 고른 것은 강조 색 체크, 가리킨 줄은 창 바탕과 같은 빛의 회색이다.
//! 자리 정하기(붙인 자리 아래, 모자라면 위, 창 밖으로 나가지 않게), 누른 줄 찾기, 키로 줄 옮기기도 여기서 한다.
//! Windows와 맥이 같이 쓴다

use super::glyph::*;
use super::{Canvas, Layout, draw_layout_icon, soft_shadow, toolbar::draw_view_icon, well};
use crate::draw::{Color, Font, R};
use crate::palette::{R_PANE, R_ROW, Theme, alpha};
use crate::pane::ViewMode;

/// 줄 높이, 카드 안쪽 여백, 구분선이 차지하는 높이, 가장 좁은 폭
pub const MENU_ROW_H: f32 = 28.0;
pub const MENU_PAD: f32 = 6.0;
pub const MENU_SEP_H: f32 = 9.0;
pub const MENU_MIN_W: f32 = 180.0;
/// 체크 자리, 그림 자리, 줄 오른쪽 여백
const CHECK_W: f32 = 26.0;
const ICON_W: f32 = 26.0;
const RIGHT_PAD: f32 = 14.0;

/// 줄 이름 앞의 그림
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum MenuIcon {
    #[default]
    None,
    /// 아이콘 글꼴의 그림
    Glyph(u16),
    /// 칸 나누기 모양
    Layout(Layout),
    /// 보기 방식 모양
    View(ViewMode),
    /// 색 동그라미 (작업 공간 색)
    Swatch(Color),
}

/// 메뉴 줄 하나
#[derive(Clone, PartialEq, Debug, Default)]
pub struct MenuLine {
    pub label: String,
    pub icon: MenuIcon,
    /// 지금 것 (강조 색 체크)
    pub checked: bool,
    /// 쓸 수 없는 줄은 흐리게 그리고 고를 수 없다
    pub disabled: bool,
    /// 오른쪽 끝의 흐린 글 (단축키)
    pub hint: String,
    /// 이 줄 앞에 구분선
    pub sep_before: bool,
    /// 하위 메뉴가 있다 (오른쪽 끝에 화살표)
    pub sub: bool,
}

impl MenuLine {
    pub fn new(label: impl Into<String>) -> MenuLine {
        MenuLine { label: label.into(), ..MenuLine::default() }
    }

    pub fn checked(mut self, on: bool) -> MenuLine {
        self.checked = on;
        self
    }

    pub fn icon(mut self, icon: MenuIcon) -> MenuLine {
        self.icon = icon;
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> MenuLine {
        self.hint = hint.into();
        self
    }

    pub fn sep(mut self, before: bool) -> MenuLine {
        self.sep_before = before;
        self
    }
}

/// 줄들을 다 담는 높이 (안쪽 여백 포함)
pub fn menu_height(lines: &[MenuLine]) -> f32 {
    MENU_PAD * 2.0 + lines.iter().map(|l| MENU_ROW_H + if l.sep_before { MENU_SEP_H } else { 0.0 }).sum::<f32>()
}

/// 이름이 시작하는 자리 (줄 왼쪽에서): 체크 자리, 그림이 있는 메뉴면 그림 자리까지
fn label_x(lines: &[MenuLine]) -> f32 {
    CHECK_W + if lines.iter().any(|l| l.icon != MenuIcon::None) { ICON_W } else { 0.0 } + 4.0
}

/// 가장 긴 줄이 들어가는 폭 (MENU_MIN_W보다 좁지 않게, max_w보다 넓지 않게)
pub fn menu_width(g: &impl Canvas, lines: &[MenuLine], max_w: f32) -> f32 {
    let widest = lines
        .iter()
        .map(|l| {
            let hint = if l.hint.is_empty() { 0.0 } else { g.measure(&l.hint, Font::Small) + 16.0 };
            let sub = if l.sub { 16.0 } else { 0.0 };
            g.measure(&l.label, Font::Normal) + hint + sub
        })
        .fold(0.0, f32::max);
    (MENU_PAD * 2.0 + label_x(lines) + widest + RIGHT_PAD).clamp(MENU_MIN_W.min(max_w), max_w.max(MENU_MIN_W.min(max_w)))
}

/// 메뉴 자리: 붙인 자리(anchor) 아래에 왼쪽 끝을 맞춘다. 아래가 모자라면 위로, 위도 모자라면 넓은 쪽에 높이를 줄여 놓는다.
/// 오른쪽으로 넘치면 왼쪽으로 민다. bounds(창 안쪽) 밖으로 나가지 않는다
pub fn menu_place(anchor: R, w: f32, h: f32, bounds: R) -> R {
    let w = w.min(bounds.w());
    let l = anchor.l.min(bounds.r - w).max(bounds.l);
    let below = bounds.b - (anchor.b + 4.0);
    let above = (anchor.t - 4.0) - bounds.t;
    let (t, h) = if h <= below {
        (anchor.b + 4.0, h)
    } else if h <= above {
        (anchor.t - 4.0 - h, h)
    } else if below >= above {
        (anchor.b + 4.0, below.max(MENU_ROW_H + MENU_PAD * 2.0).min(bounds.h()))
    } else {
        let h = above.max(MENU_ROW_H + MENU_PAD * 2.0).min(bounds.h());
        (anchor.t - 4.0 - h, h)
    };
    let t = t.max(bounds.t).min(bounds.b - h);
    R::new(l, t, l + w, t + h)
}

/// 줄마다의 자리. 줄이 메뉴보다 길면 scroll만큼 위로 밀린다 (보이지 않는 줄도 자리는 있다)
pub fn menu_rows(menu: R, lines: &[MenuLine], scroll: f32) -> Vec<R> {
    let mut y = menu.t + MENU_PAD - scroll;
    lines
        .iter()
        .map(|l| {
            if l.sep_before {
                y += MENU_SEP_H;
            }
            let r = R::new(menu.l + MENU_PAD, y, menu.r - MENU_PAD, y + MENU_ROW_H);
            y += MENU_ROW_H;
            r
        })
        .collect()
}

/// 스크롤할 수 있는 끝 (줄이 메뉴에 다 들어가면 0)
pub fn menu_max_scroll(menu: R, lines: &[MenuLine]) -> f32 {
    (menu_height(lines) - menu.h()).max(0.0)
}

/// 줄 i가 보이도록 맞춘 스크롤
pub fn menu_reveal(menu: R, lines: &[MenuLine], scroll: f32, i: usize) -> f32 {
    let Some(r) = menu_rows(menu, lines, scroll).get(i).copied() else { return scroll };
    let (top, bottom) = (menu.t + MENU_PAD, menu.b - MENU_PAD);
    let s = if r.t < top {
        scroll - (top - r.t)
    } else if r.b > bottom {
        scroll + (r.b - bottom)
    } else {
        scroll
    };
    s.clamp(0.0, menu_max_scroll(menu, lines))
}

/// 그 자리의 고를 수 있는 줄 (구분선·쓸 수 없는 줄·안쪽 여백이면 None)
pub fn menu_hit(menu: R, lines: &[MenuLine], scroll: f32, x: f32, y: f32) -> Option<usize> {
    if !menu.inset(0.0, MENU_PAD).contains(x, y) {
        return None;
    }
    menu_rows(menu, lines, scroll).iter().position(|r| r.contains(x, y)).filter(|i| !lines[*i].disabled)
}

/// 키로 옮긴 다음 줄: 아래(down)나 위로, 쓸 수 없는 줄은 건너뛰고 끝에서는 반대쪽 끝으로 돈다. 가리킨 줄이 없으면 맨 위나 맨 아래부터
pub fn menu_step(lines: &[MenuLine], from: Option<usize>, down: bool) -> Option<usize> {
    let n = lines.len();
    if n == 0 {
        return None;
    }
    let mut i = match from {
        Some(i) if i < n => i,
        _ => {
            if down {
                n - 1
            } else {
                0
            }
        }
    };
    for _ in 0..n {
        i = if down { (i + 1) % n } else { (i + n - 1) % n };
        if !lines[i].disabled {
            return Some(i);
        }
    }
    None
}

/// 그릴 메뉴: 자리, 줄, 가리킨 줄(마우스·키보드), 스크롤
pub struct MenuScene<'a> {
    pub rect: R,
    pub lines: &'a [MenuLine],
    pub sel: Option<usize>,
    pub scroll: f32,
}

pub fn paint_menu(g: &impl Canvas, t: &Theme, s: &MenuScene) {
    let r = s.rect;
    // 그림자 + 흰 카드 (떠 있는 다른 것과 같은 그림자). 테두리는 아주 옅게
    if !t.high_contrast {
        soft_shadow(g, r, R_PANE, 12.0, 0.16, t.dark);
    }
    g.fill_round(r, R_PANE, t.pane);
    g.stroke_round(r.inset(0.5, 0.5), R_PANE, if t.high_contrast { t.border } else { t.pane_border }, 1.0);
    let x0 = label_x(s.lines);
    g.clip(r.inset(1.0, MENU_PAD));
    for (i, (line, ir)) in s.lines.iter().zip(menu_rows(r, s.lines, s.scroll)).enumerate() {
        if ir.b < r.t || ir.t > r.b {
            continue;
        }
        if line.sep_before {
            let y = ir.t - MENU_SEP_H / 2.0;
            g.fill(R::new(ir.l + 8.0, y - 0.5, ir.r - 8.0, y + 0.5), if t.high_contrast { t.border } else { alpha(t.text, 0.08) });
        }
        let picked = s.sel == Some(i) && !line.disabled;
        // 가리킨 줄: 창 바탕과 같은 빛의 회색 (고대비는 선택 색)
        if picked {
            g.fill_round(ir, R_ROW, if t.high_contrast { t.sel } else { well(t) });
        }
        let ink = if line.disabled {
            t.faint
        } else if picked && t.high_contrast {
            t.sel_text
        } else {
            t.text
        };
        let dim = if picked && t.high_contrast { t.sel_text } else { t.dim };
        if line.checked {
            let c = if t.high_contrast { ink } else { t.accent };
            g.glyph(G_CHECK, R::new(ir.l + 2.0, ir.t, ir.l + CHECK_W, ir.b), true, c);
        }
        let icon = R::new(ir.l + CHECK_W, ir.t, ir.l + CHECK_W + ICON_W - 4.0, ir.b);
        match line.icon {
            MenuIcon::None => {}
            MenuIcon::Glyph(code) => g.glyph(code, icon, false, dim),
            MenuIcon::Layout(l) => draw_layout_icon(g, icon, l, dim),
            MenuIcon::View(v) => draw_view_icon(g, icon, v, dim),
            MenuIcon::Swatch(c) => {
                let (cx, cy) = ((icon.l + icon.r) / 2.0, (icon.t + icon.b) / 2.0);
                g.fill_round(R::new(cx - 6.0, cy - 6.0, cx + 6.0, cy + 6.0), 6.0, c);
            }
        }
        let mut right = ir.r - RIGHT_PAD + 4.0;
        if line.sub {
            g.glyph(G_CHEVRON, R::new(right - 14.0, ir.t, right, ir.b), true, dim);
            right -= 16.0;
        }
        if !line.hint.is_empty() {
            let w = g.measure(&line.hint, Font::Small);
            g.text(&line.hint, R::new(right - w, ir.t, right, ir.b), Font::Small, if line.disabled { t.faint } else { dim });
            right -= w + 16.0;
        }
        g.text(&line.label, R::new(ir.l + x0, ir.t, right, ir.b), Font::Normal, ink);
    }
    g.unclip();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<MenuLine> {
        (0..n).map(|i| MenuLine::new(format!("줄 {i}"))).collect()
    }

    /// 붙인 자리 아래에 놓고, 아래가 모자라면 위로, 오른쪽으로 넘치면 왼쪽으로 민다. 둘 다 모자라면 넓은 쪽에 높이를 줄인다
    #[test]
    fn menus_open_below_and_flip_or_shrink_to_stay_inside() {
        let bounds = R::new(8.0, 8.0, 1000.0, 600.0);
        let r = menu_place(R::new(100.0, 40.0, 140.0, 70.0), 200.0, 150.0, bounds);
        assert_eq!((r.l, r.t, r.w(), r.h()), (100.0, 74.0, 200.0, 150.0));
        // 아래가 모자라면 위로
        let r = menu_place(R::new(100.0, 500.0, 100.0, 500.0), 200.0, 150.0, bounds);
        assert_eq!((r.t, r.b), (346.0, 496.0));
        // 오른쪽 끝에서는 왼쪽으로
        let r = menu_place(R::new(950.0, 40.0, 990.0, 70.0), 200.0, 150.0, bounds);
        assert_eq!(r.r, 1000.0);
        // 아주 긴 메뉴는 넓은 쪽(아래)에 높이를 줄여 넣는다
        let r = menu_place(R::new(100.0, 40.0, 140.0, 70.0), 200.0, 2000.0, bounds);
        assert!(r.t >= 74.0 && r.b <= 600.0, "{r:?}");
    }

    /// 구분선은 줄 사이에 자리를 차지하고, 누른 자리의 줄을 찾을 때 구분선과 쓸 수 없는 줄은 고르지 않는다
    #[test]
    fn menu_rows_leave_room_for_separators_and_skip_disabled_lines() {
        let mut l = lines(4);
        l[2].sep_before = true;
        l[3].disabled = true;
        let menu = R::new(0.0, 0.0, 200.0, menu_height(&l));
        assert_eq!(menu.h(), MENU_PAD * 2.0 + MENU_ROW_H * 4.0 + MENU_SEP_H);
        let rows = menu_rows(menu, &l, 0.0);
        assert_eq!(rows[2].t - rows[1].b, MENU_SEP_H);
        assert_eq!(menu_hit(menu, &l, 0.0, 50.0, rows[1].t + 4.0), Some(1));
        assert_eq!(menu_hit(menu, &l, 0.0, 50.0, rows[2].t - 3.0), None, "the separator");
        assert_eq!(menu_hit(menu, &l, 0.0, 50.0, rows[3].t + 4.0), None, "a disabled line");
        assert_eq!(menu_hit(menu, &l, 0.0, 50.0, -5.0), None);
        // 키: 쓸 수 없는 줄은 건너뛰고 끝에서 돈다
        assert_eq!(menu_step(&l, None, true), Some(0));
        assert_eq!(menu_step(&l, Some(2), true), Some(0));
        assert_eq!(menu_step(&l, Some(0), false), Some(2));
        assert_eq!(menu_step(&l, None, false), Some(2));
        assert_eq!(menu_step(&[], None, true), None);
    }

    /// 메뉴보다 긴 목록은 스크롤하고, 키로 옮긴 줄이 보이게 맞춘다
    #[test]
    fn long_menus_scroll_to_show_the_picked_line() {
        let l = lines(40);
        let menu = R::new(0.0, 0.0, 200.0, 300.0);
        let max = menu_max_scroll(menu, &l);
        assert!(max > 0.0);
        let s = menu_reveal(menu, &l, 0.0, 39);
        assert_eq!(s, max);
        let rows = menu_rows(menu, &l, s);
        assert!(rows[39].b <= menu.b - MENU_PAD + 0.01);
        assert_eq!(menu_reveal(menu, &l, s, 0), 0.0);
        assert_eq!(menu_hit(menu, &l, s, 50.0, rows[39].t + 4.0), Some(39));
    }
}
