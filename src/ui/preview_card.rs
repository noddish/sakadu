//! 자동 미리 보기 카드: 미리 볼 수 있는 파일(그림·글)을 고르면 칸 위에 뜨는 작은 카드.
//! 자리 계산과 그리기를 Windows와 맥이 함께 쓴다. 그림을 읽어 그리는 것과 글을 읽는 것은 각 앱이 한다.
//! 카드는 지금 칸이 아닌 칸의 오른쪽 아래에 놓여, 고른 파일의 이름을 가리지 않는다.
//! 칸이 하나뿐이면 그 칸의 오른쪽 아래에 놓인다 (이름 열이 보일 만큼 넓을 때만)

use super::glyph::*;
use super::*;
use crate::palette::alpha;

pub const CARD_W: f32 = 288.0;
/// 카드와 칸 모서리 사이
pub const CARD_MARGIN: f32 = 24.0;
/// 작은 카드의 그림·글 자리 높이
const BODY_H: f32 = 184.0;
/// 아래쪽 이름 줄 (이름, 종류·크기, 크게 보기·닫기 단추)
const FOOT_H: f32 = 56.0;
const BTN: f32 = 28.0;
/// 카드가 이보다 좁아지면 놓지 않는다
const MIN_W: f32 = 200.0;
/// 지금 칸에 카드를 놓을 때 왼쪽에 남겨야 하는 폭 (고른 파일의 이름이 보이게)
const NAME_ROOM: f32 = 200.0;

/// 카드의 자리
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardGeom {
    /// 카드가 놓인 칸
    pub host: usize,
    pub card: R,
    /// 그림·글 자리
    pub body: R,
    /// 이름 줄
    pub foot: R,
    pub expand: R,
    pub close: R,
}

/// panes: 보이는 칸(번호, 자리), active: 지금 칸(고른 파일이 있는 칸), large: 크게 보기(놓인 칸을 채운다. 지금 칸이면 이름 자리는 남긴다).
/// 놓을 만한 칸이 없으면(좁거나 낮으면) None
pub fn card_geom(panes: &[(usize, R)], active: usize, large: bool) -> Option<CardGeom> {
    let small_h = BODY_H + FOOT_H;
    let fits = |r: &R, own: bool| r.w() >= MIN_W + 2.0 * CARD_MARGIN + if own { NAME_ROOM } else { 0.0 } && r.h() >= small_h + 2.0 * CARD_MARGIN;
    // 다른 칸 가운데 창의 오른쪽 아래에 가장 가까운 칸: 아래쪽이 먼저, 그다음 오른쪽
    let mut others: Vec<&(usize, R)> = panes.iter().filter(|(i, r)| *i != active && fits(r, false)).collect();
    others.sort_by(|a, b| (b.1.b, b.1.r).partial_cmp(&(a.1.b, a.1.r)).unwrap_or(std::cmp::Ordering::Equal));
    let (host, r) = *others.first().copied().or_else(|| panes.iter().find(|(i, r)| *i == active && fits(r, true)))?;
    // 지금 칸에 놓을 때는 왼쪽의 이름 자리를 늘 남긴다: 작은 카드는 그만큼 좁아지고, 큰 카드도 이름 자리 오른쪽만 채운다
    // (고른 파일의 이름이 보이고, 이름을 눌러 다른 파일로 옮겨 갈 수 있게)
    let left = r.l + CARD_MARGIN + if host == active { NAME_ROOM } else { 0.0 };
    let card = if large {
        R::new(left, r.t + CARD_MARGIN, r.r - CARD_MARGIN, r.b - CARD_MARGIN)
    } else {
        let w = CARD_W.min(r.r - CARD_MARGIN - left);
        R::new(r.r - CARD_MARGIN - w, r.b - CARD_MARGIN - small_h, r.r - CARD_MARGIN, r.b - CARD_MARGIN)
    };
    let foot = R::new(card.l, card.b - FOOT_H, card.r, card.b);
    let body = R::new(card.l, card.t, card.r, foot.t);
    let bt = foot.t + (FOOT_H - BTN) / 2.0;
    let close = R::new(card.r - 12.0 - BTN, bt, card.r - 12.0, bt + BTN);
    let expand = R::new(close.l - 8.0 - BTN, bt, close.l - 8.0, bt + BTN);
    Some(CardGeom { host, card, body, foot, expand, close })
}

/// 카드에서 마우스가 올라간 곳
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CardHover {
    None,
    Expand,
    Close,
}

/// 카드에 보일 내용
pub enum CardBody<'a> {
    /// 그림 (CardArt::image가 그린다). 아직 그리지 못하면 note를 보인다
    Image,
    /// 글 파일의 앞부분
    Text(&'a str),
}

/// 카드를 그리는 데 필요한 것 (앱이 자기 상태로 채운다)
pub struct CardScene<'a> {
    pub geom: &'a CardGeom,
    pub name: &'a str,
    /// 종류와 크기 ("PNG 이미지 · 248 KB")
    pub detail: &'a str,
    pub body: CardBody<'a>,
    /// 그림을 그리지 못했을 때 보일 글 ("불러오는 중…", "미리 볼 수 없어요"). 없으면 비워 둔다
    pub note: Option<&'a str>,
    pub large: bool,
    pub hover: CardHover,
}

/// 그림을 그리는 쪽 (Windows: 셸 썸네일, 맥: 읽어 둔 그림)
pub trait CardArt<G: Canvas> {
    /// 그림을 r 안에 비율을 지켜 가운데에 그린다. 아직 없으면(읽는 중이거나 읽지 못했으면) false
    fn image(&mut self, g: &mut G, r: R) -> bool;
}

pub fn paint_preview_card<G: Canvas>(g: &mut G, art: &mut impl CardArt<G>, t: &Theme, s: &CardScene) {
    let t = *t;
    let m = s.geom;
    if !t.high_contrast {
        soft_shadow(g, m.card, RADIUS, 20.0, 0.22, t.dark);
    }
    g.fill_round(m.card, RADIUS, t.pane);
    if t.high_contrast || t.dark {
        g.stroke_round(m.card.inset(0.5, 0.5), RADIUS, t.pane_border, 1.0);
    }
    // 그림·글 자리: 카드 가장자리에서 12만큼 안쪽
    let inner = R::new(m.body.l + 12.0, m.body.t + 12.0, m.body.r - 12.0, m.body.b - 8.0);
    match s.body {
        CardBody::Image => {
            if !art.image(g, inner)
                && let Some(note) = s.note
            {
                g.text(note, inner, Font::Hint, t.faint);
            }
        }
        CardBody::Text(text) => {
            g.fill_round(inner, 8.0, t.field);
            g.clip(inner.inset(1.0, 1.0));
            g.text(text, R::new(inner.l + 12.0, inner.t + 8.0, inner.r - 8.0, inner.b + 400.0), Font::Mono, t.text);
            g.unclip();
        }
    }
    // 이름 줄: 이름, 종류·크기, 단추
    g.fill(R::new(m.card.l, m.foot.t, m.card.r, m.foot.t + 1.0), if t.high_contrast { t.border } else { alpha(t.text, 0.08) });
    let text_r = m.expand.l - 8.0;
    g.text(s.name, R::new(m.foot.l + 16.0, m.foot.t + 9.0, text_r, m.foot.t + 29.0), Font::Strong, t.text);
    g.text(s.detail, R::new(m.foot.l + 16.0, m.foot.t + 29.0, text_r, m.foot.t + 47.0), Font::Small, t.dim);
    for (r, glyph, h) in [(m.expand, if s.large { G_SHRINK } else { G_EXPAND }, CardHover::Expand), (m.close, G_CLOSE, CardHover::Close)] {
        let hot = s.hover == h;
        if t.high_contrast {
            g.stroke_round(r.inset(0.5, 0.5), BTN / 2.0, if hot { t.focus } else { t.border }, 1.0);
        } else {
            g.fill_round(r, BTN / 2.0, alpha(t.text, if hot { 0.12 } else { 0.05 }));
        }
        g.glyph_font(glyph, r, Font::GlyphMid, if hot { t.text } else { t.dim });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes(layout: Layout) -> Vec<(usize, R)> {
        pane_rects(R::new(272.0, 52.0, 1256.0, 804.0), (0.5, 0.5), layout)
    }

    fn rect_of(p: &[(usize, R)], i: usize) -> R {
        p.iter().find(|(k, _)| *k == i).unwrap().1
    }

    /// 두 칸: 카드는 지금 칸이 아닌 칸의 오른쪽 아래에, 모서리에서 24 떨어져 놓인다
    #[test]
    fn card_sits_in_the_other_pane_bottom_right() {
        let p = panes(Layout::TwoV);
        let m = card_geom(&p, 0, false).unwrap();
        let host = rect_of(&p, 1);
        assert_eq!(m.host, 1);
        assert_eq!((m.card.r, m.card.b, m.card.w()), (host.r - CARD_MARGIN, host.b - CARD_MARGIN, CARD_W));
        // 그림 자리와 이름 줄이 카드를 위아래로 나누고, 단추는 이름 줄 안에서 겹치지 않는다
        assert_eq!((m.body.t, m.body.b, m.foot.b), (m.card.t, m.foot.t, m.card.b));
        assert!(m.expand.r <= m.close.l && m.close.r <= m.card.r && m.expand.t >= m.foot.t && m.expand.b <= m.foot.b);
        // 지금 칸이 오른쪽이면 왼쪽 칸으로 비켜난다
        assert_eq!(card_geom(&p, 1, false).unwrap().host, 0);
    }

    /// 칸이 여럿이면 창의 오른쪽 아래에 가장 가까운 다른 칸에 놓인다
    #[test]
    fn card_moves_aside_when_the_active_pane_is_in_the_corner() {
        // 네 칸: 0 왼쪽 위, 1 오른쪽 위, 2 왼쪽 아래, 3 오른쪽 아래
        let p = panes(Layout::Four);
        let corner = p.iter().max_by(|a, b| (a.1.b, a.1.r).partial_cmp(&(b.1.b, b.1.r)).unwrap()).unwrap().0;
        for active in 0..4 {
            let m = card_geom(&p, active, false).unwrap();
            assert_ne!(m.host, active);
            if active != corner {
                assert_eq!(m.host, corner, "active {active}");
            } else {
                // 지금 칸이 오른쪽 아래면 같은 줄의 왼쪽 칸으로
                let host = rect_of(&p, m.host);
                assert_eq!(host.b, rect_of(&p, corner).b);
                assert!(host.r < rect_of(&p, corner).r);
            }
        }
        // 세 칸(왼쪽 큰 칸, 오른쪽 위아래): 오른쪽 아래가 지금 칸이면 왼쪽 큰 칸으로
        let p = panes(Layout::Three);
        let corner = p.iter().max_by(|a, b| (a.1.b, a.1.r).partial_cmp(&(b.1.b, b.1.r)).unwrap()).unwrap().0;
        let big = p.iter().min_by(|a, b| a.1.l.partial_cmp(&b.1.l).unwrap()).unwrap().0;
        assert_eq!(card_geom(&p, corner, false).unwrap().host, big);
        // 위아래 두 칸: 아래가 지금 칸이면 위 칸으로
        let p = panes(Layout::TwoH);
        assert_eq!(card_geom(&p, 1, false).unwrap().host, 0);
    }

    /// 한 칸: 넓으면 그 칸의 오른쪽 아래에 놓여 왼쪽에 이름 자리가 남고, 좁거나 낮으면 놓지 않는다
    #[test]
    fn single_pane_keeps_room_for_names_or_shows_no_card() {
        let wide = [(0usize, R::new(52.0, 52.0, 1256.0, 804.0))];
        let m = card_geom(&wide, 0, false).unwrap();
        assert_eq!(m.host, 0);
        assert!(m.card.l - wide[0].1.l >= NAME_ROOM);
        assert_eq!(card_geom(&[(0, R::new(52.0, 52.0, 452.0, 804.0))], 0, false), None, "too narrow to keep the names visible");
        // 겨우 들어가는 폭(448~511): 카드가 좁아져 이름 자리를 남긴다. 크게 봐도 이름 자리는 남는다
        for w in [448.0, 460.0, 511.0, 512.0, 700.0] {
            let pane = R::new(52.0, 52.0, 52.0 + w, 804.0);
            for large in [false, true] {
                let m = card_geom(&[(0, pane)], 0, large).unwrap();
                assert!(m.card.l - pane.l >= NAME_ROOM + CARD_MARGIN, "w={w} large={large}: {}", m.card.l - pane.l);
                assert!(m.card.w() >= MIN_W && m.card.r == pane.r - CARD_MARGIN, "w={w} large={large}");
            }
        }
        assert_eq!(card_geom(&[(0, R::new(52.0, 52.0, 1256.0, 300.0))], 0, false), None, "too short");
        // 좁은 다른 칸에는 카드가 그 칸 폭에 맞춰 줄어든다
        let narrow = [(0usize, R::new(52.0, 52.0, 700.0, 804.0)), (1usize, R::new(708.0, 52.0, 968.0, 804.0))];
        let m = card_geom(&narrow, 0, false).unwrap();
        assert_eq!((m.host, m.card.w()), (1, 260.0 - 2.0 * CARD_MARGIN));
    }

    /// 크게 보기: 놓인 칸을 여백만 남기고 채운다
    #[test]
    fn large_card_fills_its_pane() {
        let p = panes(Layout::TwoV);
        let small = card_geom(&p, 0, false).unwrap();
        let large = card_geom(&p, 0, true).unwrap();
        assert_eq!(large.host, small.host);
        assert_eq!(large.card, rect_of(&p, 1).inset(CARD_MARGIN, CARD_MARGIN));
        assert!(large.body.h() > small.body.h() && large.foot.h() == small.foot.h());
    }
}
