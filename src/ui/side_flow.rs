//! 사이드바 줄을 끌어 옮기기: Windows와 맥이 함께 쓴다. 끄는 줄은 흰 카드로 떠서 마우스를 따라오고, 다른 줄은 놓일 빈자리를
//! 만들며 미끄러진다. 그룹 안에 빈자리가 생기면 그 그룹을 둥근 판으로 묶어 보인다. 놓을 곳은 보이는 빈자리 그대로 정한다 (drop_plan).
//! 끌기가 아니어도 줄의 자리가 바뀌면 지난 자리에서 새 자리로 미끄러져 간다 (SideFlow. 시스템에서 움직임을 끄면 바로 옮겨진다).
//! 목록의 줄 배치, 누르기, 놓은 뒤 실제로 옮기기와 줄 그리기는 각 앱이 한다: 여기에는 자리 계산과 판·카드 바탕 그리기만 있다

use super::{A_HOVER, A_PRESS, Canvas, SHADOW_M, SP_HAIR, SP_XS, soft_shadow};
use crate::draw::R;
use crate::favorites::{Favorites, Row};
use crate::palette::{R_ROW, Theme, alpha};
use std::collections::HashMap;
use std::time::Instant;

/// 끌어서 놓을 곳 (끄는 줄을 빼낸 뒤 기준)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropTo {
    /// 이 공간에 고정한 줄 사이
    Pin(usize),
    /// '모든 공간'의 항목 자리 (그룹, 자리)
    FavItem(Option<usize>, usize),
    /// 그룹 순서
    FavGroup(usize),
}

/// 목록의 한 자리가 끌기에서 맡는 몫
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlowKind {
    /// 이 공간에 고정한 줄 (고정한 것 가운데 번호)
    Pin(usize),
    /// 고정한 것이 없을 때의 안내 자리: 고정 목록 구역이 여기서 시작한다
    PinHint,
    /// '모든 공간'의 즐겨찾기 줄 (항목, 그룹 머리)
    Fav(Row),
    /// 즐겨찾기 바로 위의 줄 (Windows는 내 PC, 맥은 마지막 바로가기): 즐겨찾기가 하나도 없으면 그 아래가 즐겨찾기 구역의 시작
    FavAnchor,
    /// 그 밖의 줄 (구역 이름, 바로가기): 끌기에서 자리가 바뀌지 않는다
    Other,
}

/// 목록의 한 자리: 위·아래 (스크롤 전, 목록 맨 위가 0인 좌표)와 그 몫
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowSlot {
    pub t: f32,
    pub b: f32,
    pub kind: FlowKind,
}

/// 끄는 동안의 그림과 놓을 곳
#[derive(Clone, Debug, PartialEq)]
pub struct DropPlan {
    pub to: DropTo,
    /// 카드 위쪽(목록 좌표)과 높이
    pub card_t: f32,
    pub card_h: f32,
    /// 자리(slots 차례)마다 보일 위쪽 (목록 좌표). None은 끄는 줄과, 그룹을 끌 때 그 안 줄
    pub targets: Vec<Option<f32>>,
    /// 빈자리 위쪽과, 그 빈자리를 품는 그룹
    pub gap_t: f32,
    pub gap_group: Option<usize>,
}

/// 남은 줄 n개를 start부터 차례로 놓았을 때, 가운데가 center보다 위에 있는 줄 수 = 빈자리 차례
pub fn gap_index(n: usize, start: f32, row_h: f32, center: f32) -> usize {
    (0..n).filter(|k| start + *k as f32 * row_h + row_h / 2.0 < center).count()
}

/// 구역의 줄(자리 번호 idx, 감춘 줄은 targets가 None)을 start부터 차례로 다시 놓는다. gap번째 앞에는 한 줄만큼 비운다.
/// 빈자리의 위쪽을 돌려준다
pub fn lay_out(targets: &mut [Option<f32>], idx: &[usize], start: f32, row_h: f32, gap: Option<usize>) -> f32 {
    let mut y = start;
    let mut gap_t = start;
    let mut k = 0;
    for &i in idx {
        if targets[i].is_none() {
            continue;
        }
        if gap == Some(k) {
            gap_t = y;
            y += row_h;
        }
        targets[i] = Some(y);
        y += row_h;
        k += 1;
    }
    if gap == Some(k) {
        gap_t = y;
    }
    gap_t
}

/// 끄는 동안의 그림과 놓을 곳.
/// slots: 목록의 자리 (보이는 차례), dragged: 끄는 줄 (고정한 줄이나 즐겨찾기 줄), card_t: 카드 위쪽 (목록 좌표),
/// over_pins: 마우스가 '모든 공간' 이름 줄보다 위(고정한 쪽)에 있는지. 목록 밖이면 None이고, 그때는 끄는 줄이 있던 쪽으로 본다
pub fn drop_plan(slots: &[FlowSlot], dragged: FlowKind, card_t: f32, row_h: f32, over_pins: Option<bool>, favs: &Favorites) -> DropPlan {
    let center = card_t + row_h / 2.0;
    // 끄는 줄은 감춘다 (카드로 그린다). 그룹을 끌면 그 안 줄도 함께 끌린다
    let hidden = |s: FlowKind| s == dragged || matches!((dragged, s), (FlowKind::Fav(Row::Group(g)), FlowKind::Fav(Row::Item { group: Some(h), .. })) if g == h);
    let mut targets: Vec<Option<f32>> = slots.iter().map(|s| (!hidden(s.kind)).then_some(s.t)).collect();
    let pins: Vec<usize> = (0..slots.len()).filter(|&i| matches!(slots[i].kind, FlowKind::Pin(_))).collect();
    let fav_idx: Vec<usize> = (0..slots.len()).filter(|&i| matches!(slots[i].kind, FlowKind::Fav(_))).collect();
    // 구역의 첫 자리: 고정한 것은 맨 위(안내 자리 포함), 즐겨찾기는 그 바로 위 줄의 아래
    let pin_start = slots.iter().find(|s| matches!(s.kind, FlowKind::Pin(_) | FlowKind::PinHint)).map_or(0.0, |s| s.t);
    let fav_start = fav_idx.first().map(|&i| slots[i].t).or_else(|| slots.iter().rfind(|s| s.kind == FlowKind::FavAnchor).map(|s| s.b)).unwrap_or(0.0);
    let left = |idx: &[usize]| idx.iter().filter(|&&i| targets[i].is_some()).count();
    let fav_rows = |targets: &[Option<f32>]| -> Vec<Row> {
        fav_idx
            .iter()
            .filter(|&&i| targets[i].is_some())
            .filter_map(|&i| match slots[i].kind {
                FlowKind::Fav(r) => Some(r),
                _ => None,
            })
            .collect()
    };
    let (to, gap_t, gap_group) = match dragged {
        // 그룹은 그룹 사이 틈(머리 줄 앞이나 맨 끝)에만 놓는다: 카드 위쪽에 가장 가까운 틈
        FlowKind::Fav(Row::Group(_)) => {
            let rem = fav_rows(&targets);
            let bounds: Vec<usize> = (0..=rem.len()).filter(|&b| b == rem.len() || matches!(rem[b], Row::Group(_))).collect();
            let b = bounds
                .iter()
                .copied()
                .min_by(|a, c| {
                    let d = |k: usize| (fav_start + k as f32 * row_h - card_t).abs();
                    d(*a).total_cmp(&d(*c))
                })
                .unwrap_or(rem.len());
            let gap_t = lay_out(&mut targets, &fav_idx, fav_start, row_h, Some(b));
            lay_out(&mut targets, &pins, pin_start, row_h, None);
            (DropTo::FavGroup(Favorites::group_gap_target(&rem, b)), gap_t, None)
        }
        // 항목·고정한 줄: '모든 공간' 이름 줄보다 위면 고정한 쪽, 아니면 '모든 공간' 쪽에 빈자리
        _ => {
            let to_pins = over_pins.unwrap_or(matches!(dragged, FlowKind::Pin(_)));
            if to_pins {
                let gap = gap_index(left(&pins), pin_start, row_h, center);
                let gap_t = lay_out(&mut targets, &pins, pin_start, row_h, Some(gap));
                lay_out(&mut targets, &fav_idx, fav_start, row_h, None);
                (DropTo::Pin(gap), gap_t, None)
            } else {
                let rem = fav_rows(&targets);
                let gap = gap_index(rem.len(), fav_start, row_h, center);
                let (group, index) = favs.item_gap_target(&rem, gap);
                let gap_t = lay_out(&mut targets, &fav_idx, fav_start, row_h, Some(gap));
                lay_out(&mut targets, &pins, pin_start, row_h, None);
                (DropTo::FavItem(group, index), gap_t, group)
            }
        }
    };
    DropPlan { to, card_t, card_h: row_h, targets, gap_t, gap_group }
}

/// 그룹 g의 판이 덮을 위·아래 (목록 좌표): 그 그룹의 줄(머리와 안의 항목)이 지금 보이는 자리(shown, 감춘 줄은 None)와
/// 끌어 온 줄이 들어갈 빈자리(gap: 위쪽과 높이)까지. 덮을 것이 없으면 None
pub fn group_span(slots: &[FlowSlot], shown: &[Option<f32>], g: usize, gap: Option<(f32, f32)>) -> Option<(f32, f32)> {
    let (mut top, mut bot) = (f32::MAX, f32::MIN);
    for (s, y) in slots.iter().zip(shown) {
        let in_group = matches!(s.kind, FlowKind::Fav(Row::Group(h)) | FlowKind::Fav(Row::Item { group: Some(h), .. }) if h == g);
        if let (true, Some(y)) = (in_group, *y) {
            top = top.min(y);
            bot = bot.max(y + (s.b - s.t));
        }
    }
    if let Some((gap_t, h)) = gap {
        top = top.min(gap_t);
        bot = bot.max(gap_t + h);
    }
    (top < bot).then_some((top, bot))
}

/// dt초 동안 목표까지 남은 거리 가운데 가는 몫 (0.06초 시상수). 움직임을 끄면 한 번에 다 간다
pub fn ease_k(dt: f32, motion: bool) -> f32 {
    if motion { 1.0 - (-dt / 0.06).exp() } else { 1.0 }
}

/// 한 줄이 목표(target)로 k만큼 다가간 자리와, 아직 움직이는지. 0.5보다 가까우면 목표에 붙인다
pub fn ease(cur: f32, target: f32, k: f32) -> (f32, bool) {
    let y = cur + (target - cur) * k;
    if (target - y).abs() < 0.5 { (target, false) } else { (y, true) }
}

/// 줄마다 지금 보이는 위쪽 (목록 좌표). 그릴 때마다 목표 자리로 조금씩 다가간다
#[derive(Default)]
pub struct SideFlow {
    /// 줄 이름(같은 줄이면 자리가 바뀌어도 같은 이름)마다 보이는 위쪽
    y: HashMap<String, f32>,
    /// 아직 움직이는 줄이 있다 (앱이 곧 다시 그린다)
    pub moving: bool,
    /// 지난번 걸음의 때
    at: Option<Instant>,
}

impl SideFlow {
    /// 줄 key를 마지막 걸음에서 그린 위쪽 (그리지 않은 줄이면 None). 미끄러지는 동안 누르기를 눈에 보이는 줄에 맞추려고 앱이 본다
    pub fn drawn(&self, key: &str) -> Option<f32> {
        self.y.get(key).copied()
    }

    /// 놓은 줄이 카드 자리(y)에서 새 자리로 미끄러져 들어가게 한다
    pub fn start_at(&mut self, key: String, y: f32) {
        self.y.insert(key, y);
        self.moving = true;
    }

    /// 줄마다 이번에 그릴 위쪽: 지난번 자리에서 목표 자리로 다가간다 (0.06초 시상수). 처음 보는 줄은 바로 목표 자리.
    /// keys와 targets는 같은 차례 (목표가 None인 줄은 그리지 않는다). motion이 false면(시스템에서 움직임을 껐다) 바로 목표 자리
    pub fn step(&mut self, keys: &[String], targets: &[Option<f32>], now: Instant, motion: bool) -> Vec<Option<f32>> {
        // 멈춰 있다 움직이기 시작하는 첫 그림은 한 틀(1/60초)만큼만 (오래 쉬다 그리면 한 번에 다 가 버리지 않게)
        let dt = match (self.moving, self.at) {
            (true, Some(at)) => now.duration_since(at).as_secs_f32().min(0.05),
            _ => 1.0 / 60.0,
        };
        self.at = Some(now);
        let k = ease_k(dt, motion);
        let mut next = HashMap::with_capacity(keys.len());
        let mut moving = false;
        let out = keys
            .iter()
            .zip(targets)
            .map(|(key, t)| {
                let t = (*t)?;
                let cur = self.y.get(key).copied().unwrap_or(t);
                let (y, still) = ease(cur, t, k);
                moving |= still;
                next.insert(key.clone(), y);
                Some(y)
            })
            .collect();
        self.y = next;
        self.moving = moving;
        out
    }
}

/// 그룹 머리 줄의 움직임 열쇠 (SideFlow의 줄 이름): 그룹 이름에, 그보다 앞에 있는 같은 이름 그룹의 수를 붙인다.
/// 이름은 고칠 수 있고 겹칠 수도 있다 (새 그룹 둘의 이름 고치기를 둘 다 그만두면 둘 다 '새 그룹'). 이름만 쓰면 두 그룹이 한 자리를
/// 나눠 써서 앞 그룹이 뒤 그룹의 자리에서 끝없이 다시 미끄러진다. 이름이 겹치지 않으면 그룹 차례가 바뀌어도 열쇠가 그대로다
pub fn group_flow_key(favs: &Favorites, g: usize) -> String {
    let Some(name) = favs.groups.get(g).map(|grp| grp.name.as_str()) else { return "g|".to_string() };
    let twins = favs.groups[..g].iter().filter(|grp| grp.name == name).count();
    format!("g|{name}|{twins}")
}

/// 그룹 판: 끌어 온 줄이 들어갈 그룹(strong, 진하게)이나 마우스가 올라간 그룹 머리의 그룹(옅게)을 묶는 둥근 판
pub fn paint_group_plate(g: &impl Canvas, t: &Theme, r: R, strong: bool) {
    g.fill_round(r, R_ROW + SP_XS, if t.high_contrast { t.hover } else { alpha(t.text, if strong { A_PRESS } else { A_HOVER }) });
}

/// 끌어 놓으려 할 때 놓일 구역(줄들을 감싼 자리)의 강조
pub fn paint_drop_area(g: &impl Canvas, t: &Theme, area: R) {
    g.fill_round(area, R_ROW + SP_HAIR, alpha(t.accent, A_PRESS));
    g.stroke_round(area.inset(0.75, 0.75), R_ROW + SP_HAIR, t.accent, 1.5);
}

/// 끄는 줄의 흰 카드 바탕: 그림자, 면, 테두리 (아이콘과 이름은 부르는 쪽이 그 위에 그린다)
pub fn paint_drag_card(g: &impl Canvas, t: &Theme, r: R) {
    if !t.high_contrast {
        soft_shadow(g, r, R_ROW, SHADOW_M.0, SHADOW_M.1, t.dark);
    }
    g.fill_round(r, R_ROW, t.pane);
    g.stroke_round(r.inset(0.5, 0.5), R_ROW, if t.high_contrast { t.focus } else { t.border }, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::favorites::{Favorite, Group};

    const H: f32 = 28.0;

    fn fav(name: &str) -> Favorite {
        Favorite { name: name.into(), path: format!("/{name}"), is_dir: true }
    }

    /// 빈자리 차례: 카드 가운데보다 위에 가운데가 있는 줄 수. 다시 놓으면 그 앞에 한 줄만큼 빈다
    #[test]
    fn rows_make_room_where_the_card_is() {
        assert_eq!(gap_index(3, 0.0, 28.0, 0.0), 0);
        assert_eq!(gap_index(3, 0.0, 28.0, 15.0), 1);
        assert_eq!(gap_index(3, 0.0, 28.0, 999.0), 3);
        let mut t = vec![Some(0.0), None, Some(56.0), Some(84.0)];
        let gap_t = lay_out(&mut t, &[0, 1, 2, 3], 0.0, 28.0, Some(1));
        assert_eq!(t, [Some(0.0), None, Some(56.0), Some(84.0)]);
        assert_eq!(gap_t, 28.0);
        let mut t = vec![Some(0.0), None, Some(56.0), Some(84.0)];
        let gap_t = lay_out(&mut t, &[0, 1, 2, 3], 0.0, 28.0, Some(3));
        assert_eq!((t, gap_t), (vec![Some(0.0), None, Some(28.0), Some(56.0)], 84.0), "the gap at the end");
    }

    /// 고정한 줄 셋, 구역 이름, 바로가기(즐겨찾기 바로 위 줄), 그 아래 즐겨찾기 줄들. 줄은 모두 H 높이로 이어진다
    fn slots(pins: usize, favs: &[Row]) -> Vec<FlowSlot> {
        let mut kinds: Vec<FlowKind> = (0..pins).map(FlowKind::Pin).collect();
        if pins == 0 {
            kinds.push(FlowKind::PinHint);
        }
        kinds.extend([FlowKind::Other, FlowKind::FavAnchor]);
        kinds.extend(favs.iter().map(|r| FlowKind::Fav(*r)));
        kinds.iter().enumerate().map(|(i, k)| FlowSlot { t: i as f32 * H, b: (i + 1) as f32 * H, kind: *k }).collect()
    }

    /// 그룹 없는 항목 하나, 펼친 그룹(항목 둘), 접은 그룹
    fn sample() -> Favorites {
        Favorites {
            items: vec![fav("a")],
            groups: vec![Group { name: "g".into(), collapsed: false, items: vec![fav("b"), fav("c")] }, Group { name: "h".into(), collapsed: true, items: vec![fav("d")] }],
        }
    }

    /// 고정한 줄을 아래로 끌면: 그 줄은 감춰지고, 카드 가운데를 지난 줄은 위로 올라가 빈자리를 내준다. '모든 공간'은 그대로
    #[test]
    fn a_pin_dragged_down_opens_a_gap_below_the_rows_it_passed() {
        let favs = sample();
        let s = slots(3, &favs.rows());
        // 첫 줄을 한 줄 조금 못 되게 끌었다: 카드 가운데가 위로 올라온 둘째 줄의 가운데를 지났고 셋째 줄의 가운데는 지나지 않았다
        let p = drop_plan(&s, FlowKind::Pin(0), H - 2.0, H, Some(true), &favs);
        assert_eq!(p.to, DropTo::Pin(1));
        assert_eq!(p.targets[..3], [None, Some(0.0), Some(2.0 * H)]);
        assert_eq!((p.gap_t, p.gap_group, p.card_t, p.card_h), (H, None, H - 2.0, H));
        assert!(p.targets[3..].iter().zip(&s[3..]).all(|(t, s)| *t == Some(s.t)), "the common block stays put");
        // 목록 밖으로 끌면 있던 쪽(고정한 것)에 남는다
        assert!(matches!(drop_plan(&s, FlowKind::Pin(0), 900.0, H, None, &favs).to, DropTo::Pin(2)));
    }

    /// 즐겨찾기 쪽에 놓으면 빈자리 바로 위 줄로 들어갈 곳을 정한다: 펼친 그룹 안이면 그 그룹(판을 그린다), 그룹 머리 아래면 그 그룹 맨 앞
    #[test]
    fn an_item_dropped_among_the_favorites_goes_where_the_gap_is() {
        let favs = sample();
        let rows = favs.rows();
        let s = slots(2, &rows);
        let fav_t = s.iter().find(|x| matches!(x.kind, FlowKind::Fav(_))).unwrap().t;
        // 고정한 첫 줄을 '모든 공간'의 둘째 항목 자리(그룹 머리 다음)로: 그룹 g의 맨 앞
        let p = drop_plan(&s, FlowKind::Pin(0), fav_t + 2.0 * H - 4.0, H, Some(false), &favs);
        assert_eq!((p.to, p.gap_group), (DropTo::FavItem(Some(0), 0), Some(0)));
        assert_eq!(p.gap_t, fav_t + 2.0 * H);
        // 고정한 남은 줄은 위로 당겨지고, 빈자리 아래 줄은 한 줄 내려간다
        assert_eq!(p.targets[1], Some(0.0));
        assert_eq!(p.targets[s.len() - 1], Some(fav_t + 5.0 * H));
        // 그룹 안 항목 c를 맨 위로: 그룹 없는 맨 앞, 판은 없다
        let c = FlowKind::Fav(Row::Item { group: Some(0), index: 1 });
        let p = drop_plan(&s, c, fav_t - 10.0, H, Some(false), &favs);
        assert_eq!((p.to, p.gap_group, p.gap_t), (DropTo::FavItem(None, 0), None, fav_t));
        // 즐겨찾기 항목을 고정한 쪽으로: 고정한 줄 사이에 빈자리
        let p = drop_plan(&s, c, H - 4.0, H, Some(true), &favs);
        assert_eq!((p.to, p.gap_t), (DropTo::Pin(1), H));
    }

    /// 그룹을 끌면 안의 줄도 함께 감춰지고, 그룹 사이 틈(머리 줄 앞이나 맨 끝)에만 놓인다
    #[test]
    fn a_group_moves_between_groups_only() {
        let favs = sample();
        let rows = favs.rows();
        let s = slots(1, &rows);
        let g = FlowKind::Fav(Row::Group(0));
        let p = drop_plan(&s, g, 900.0, H, Some(false), &favs);
        let hidden: Vec<usize> = (0..s.len()).filter(|&i| p.targets[i].is_none()).collect();
        assert_eq!(hidden.iter().map(|&i| s[i].kind).collect::<Vec<_>>(), [g, FlowKind::Fav(Row::Item { group: Some(0), index: 0 }), FlowKind::Fav(Row::Item { group: Some(0), index: 1 })]);
        // 맨 끝으로: 남은 그룹(h) 뒤
        assert_eq!((p.to, p.gap_group), (DropTo::FavGroup(1), None));
        // 맨 위 가까이: 그룹 없는 항목은 그룹들보다 늘 위라, 첫 틈은 h 머리 앞
        let fav_t = s.iter().find(|x| matches!(x.kind, FlowKind::Fav(_))).unwrap().t;
        assert_eq!(drop_plan(&s, g, fav_t, H, Some(false), &favs).to, DropTo::FavGroup(0));
    }

    /// 고정한 것이 없으면 안내 자리에서, 즐겨찾기가 없으면 그 바로 위 줄 아래에서 빈자리가 시작한다
    #[test]
    fn empty_blocks_start_at_the_hint_and_below_the_anchor() {
        let favs = Favorites { items: vec![fav("a")], groups: Vec::new() };
        let s = slots(0, &favs.rows());
        let item = FlowKind::Fav(Row::Item { group: None, index: 0 });
        let p = drop_plan(&s, item, 0.0, H, Some(true), &favs);
        assert_eq!((p.to, p.gap_t), (DropTo::Pin(0), s[0].t));
        let none = Favorites::default();
        let s = slots(1, &[]);
        let anchor = s.iter().find(|x| x.kind == FlowKind::FavAnchor).unwrap().b;
        let p = drop_plan(&s, FlowKind::Pin(0), anchor, H, Some(false), &none);
        assert_eq!((p.to, p.gap_t), (DropTo::FavItem(None, 0), anchor));
    }

    /// 그룹 판은 그 그룹의 보이는 줄과 빈자리를 덮는다
    #[test]
    fn the_group_plate_covers_its_rows_and_the_gap() {
        let favs = sample();
        let s = slots(1, &favs.rows());
        let shown: Vec<Option<f32>> = s.iter().map(|x| Some(x.t)).collect();
        let head = s.iter().position(|x| x.kind == FlowKind::Fav(Row::Group(0))).unwrap();
        assert_eq!(group_span(&s, &shown, 0, None), Some((s[head].t, s[head + 2].b)));
        assert_eq!(group_span(&s, &shown, 0, Some((s[head + 2].b, H))), Some((s[head].t, s[head + 2].b + H)));
        assert_eq!(group_span(&s, &vec![None; s.len()], 0, None), None);
        assert_eq!(group_span(&s, &shown, 1, None), Some((s[head + 3].t, s[head + 3].b)), "a collapsed group is only its header");
    }

    /// 움직임: 처음 보는 줄은 바로 목표, 다음부터는 목표로 다가간다. 움직임을 끄면 한 번에 간다
    #[test]
    fn rows_ease_toward_their_targets() {
        assert_eq!(ease_k(0.5, false), 1.0);
        let k = ease_k(1.0 / 60.0, true);
        assert!(k > 0.2 && k < 0.3, "{k}");
        assert_eq!(ease(0.0, 100.0, 1.0), (100.0, false));
        assert_eq!(ease(99.8, 100.0, 0.1), (100.0, false), "close enough snaps");
        let (y, moving) = ease(0.0, 100.0, k);
        assert!(moving && y > 0.0 && y < 100.0);
        let mut f = SideFlow::default();
        let keys = vec!["a".to_string(), "b".to_string()];
        let now = Instant::now();
        assert_eq!(f.step(&keys, &[Some(0.0), Some(28.0)], now, true), [Some(0.0), Some(28.0)]);
        assert!(!f.moving);
        // 자리가 바뀌면 지난 자리에서 다가간다. 목표가 없는 줄은 그리지 않는다
        let out = f.step(&keys, &[Some(28.0), None], now, true);
        assert!(f.moving && out[1].is_none() && out[0].is_some_and(|y| y > 0.0 && y < 28.0), "{out:?}");
        // 앱이 누르기를 맞추는 그린 자리: 이번에 그린 그대로, 그리지 않은 줄은 없다
        assert_eq!((f.drawn("a"), f.drawn("b")), (out[0], None));
        // 놓은 줄은 카드 자리에서 들어온다. 움직임을 끄면 바로 목표
        f.start_at("b".to_string(), 200.0);
        assert_eq!(f.step(&keys, &[Some(28.0), Some(0.0)], now, false), [Some(28.0), Some(0.0)]);
        assert!(!f.moving);
    }

    /// 그룹의 움직임 열쇠: 이름이 같은 그룹도 따로 움직이고(한 자리를 나눠 쓰면 끝없이 미끄러진다), 이름이 다르면 차례가 바뀌어도 그대로
    #[test]
    fn groups_with_the_same_name_move_apart() {
        let group = |name: &str| Group { name: name.into(), collapsed: false, items: Vec::new() };
        let mut favs = Favorites { items: Vec::new(), groups: vec![group("새 그룹"), group("업무"), group("새 그룹")] };
        let keys: Vec<String> = (0..3).map(|g| group_flow_key(&favs, g)).collect();
        assert_eq!(keys.iter().collect::<std::collections::HashSet<_>>().len(), 3, "{keys:?}");
        assert_eq!(group_flow_key(&favs, 9), "g|", "no such group");
        // 이름이 하나뿐인 그룹은 옮겨도 같은 열쇠
        let work = group_flow_key(&favs, 1);
        favs.move_group(1, 0);
        assert_eq!(group_flow_key(&favs, 0), work);
        // 같은 이름의 두 머리 줄이 각자 제자리로 가서 멈춘다
        let keys: Vec<String> = (0..3).map(|g| group_flow_key(&favs, g)).collect();
        let mut f = SideFlow::default();
        let now = Instant::now();
        f.step(&keys, &[Some(0.0), Some(28.0), Some(56.0)], now, true);
        let mut out = Vec::new();
        for i in 1..200 {
            out = f.step(&keys, &[Some(0.0), Some(56.0), Some(28.0)], now + std::time::Duration::from_millis(16 * i), true);
        }
        assert!(!f.moving, "{out:?}");
        assert_eq!(out, [Some(0.0), Some(56.0), Some(28.0)]);
    }
}
