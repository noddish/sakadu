//! 사이드바 줄의 움직임. 끄는 줄은 흰 카드로 떠서 마우스를 따라오고, 다른 줄은 놓일 빈자리를 만들며 미끄러진다.
//! 그룹 안에 빈자리가 생기면 그 그룹을 둥근 판으로 묶어 보인다. 놓을 곳은 보이는 빈자리 그대로 정한다 (DropPlan).
//! 끌기가 아니어도 줄의 자리가 바뀌면(그룹 만들기·옮기기·고정·풀기) 새 자리로 미끄러져 간다.
//! 시스템의 '애니메이션 효과'를 끄면 미끄러지지 않고 바로 옮겨진다

use super::sidebar::{Side, Slot, motion_on};
use super::*;
use crate::favorites::{Favorites, Row};

/// 끌어서 놓을 곳 (끄는 줄을 빼낸 뒤 기준)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DropTo {
    /// 이 공간에 고정한 줄 사이
    Pin(usize),
    /// '모든 공간'의 항목 자리 (그룹, 자리)
    FavItem(Option<usize>, usize),
    /// 그룹 순서
    FavGroup(usize),
}

/// 끄는 동안의 그림과 놓을 곳
pub(super) struct DropPlan {
    pub to: DropTo,
    /// 끄는 줄
    pub dragged: Side,
    /// 카드 위쪽(목록 좌표)과 높이
    pub card_t: f32,
    pub card_h: f32,
    /// 자리(side_slots 차례)마다 보일 위쪽 (목록 좌표). None은 끄는 줄과, 그룹을 끌 때 그 안 줄
    pub targets: Vec<Option<f32>>,
    /// 빈자리 위쪽과, 그 빈자리를 품는 그룹
    pub gap_t: f32,
    pub gap_group: Option<usize>,
}

/// 줄마다 지금 보이는 위쪽 (목록 좌표). 목표 자리로 조금씩 다가간다
#[derive(Default)]
pub(super) struct SideFlow {
    y: HashMap<String, f32>,
    /// 아직 움직이는 줄이 있다 (애니메이션 타이머를 돌린다)
    pub moving: bool,
    at: Option<std::time::Instant>,
}

/// 남은 줄 n개를 start부터 차례로 놓았을 때, 가운데가 center보다 위에 있는 줄 수 = 빈자리 차례
fn gap_index(n: usize, start: f32, row_h: f32, center: f32) -> usize {
    (0..n).filter(|k| start + *k as f32 * row_h + row_h / 2.0 < center).count()
}

/// 구역의 줄(자리 번호 idx, 감춘 줄은 targets가 None)을 start부터 차례로 다시 놓는다. gap번째 앞에는 한 줄만큼 비운다.
/// 빈자리의 위쪽을 돌려준다
fn lay_out(targets: &mut [Option<f32>], idx: &[usize], start: f32, row_h: f32, gap: Option<usize>) -> f32 {
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

impl App {
    /// 화면 y를 사이드바 목록 좌표(스크롤 전)로
    pub(super) fn side_list_y(&self, y: f32) -> f32 {
        y - self.side_list().t + self.side_scroll
    }

    /// 끄는 줄, 마우스 y(화면), 카드에서 잡은 자리
    fn side_dragged(&self) -> Option<(Side, f32, f32)> {
        match self.drag {
            Drag::FavMove { from, y, grab } => Some((Side::Fav(from), y, grab)),
            Drag::PinMove { from, y, grab } => Some((Side::Pin(from), y, grab)),
            _ => None,
        }
    }

    /// 끄는 동안의 그림과 놓을 곳. 끄는 중이 아니면 None
    pub(super) fn side_drop_plan(&self, slots: &[(R, Slot)]) -> Option<DropPlan> {
        let (dragged, y, grab) = self.side_dragged()?;
        let row_h = SIDE_ROW_H;
        let card_t = self.side_list_y(y) - grab;
        let center = card_t + row_h / 2.0;
        let side_of = |i: usize| match slots[i].1 {
            Slot::Item(s) => s,
        };
        // 끄는 줄은 감춘다 (카드로 그린다). 그룹을 끌면 그 안 줄도 함께 끌린다
        let hidden = |s: Side| {
            s == dragged || matches!((dragged, s), (Side::Fav(Row::Group(g)), Side::Fav(Row::Item { group: Some(h), .. })) if g == h)
        };
        let mut targets: Vec<Option<f32>> = (0..slots.len()).map(|i| (!hidden(side_of(i))).then_some(slots[i].0.t)).collect();
        let pins: Vec<usize> = (0..slots.len()).filter(|&i| matches!(side_of(i), Side::Pin(_))).collect();
        let favs: Vec<usize> = (0..slots.len()).filter(|&i| matches!(side_of(i), Side::Fav(_))).collect();
        // 구역의 첫 자리: 고정한 것은 맨 위(안내 자리 포함), 즐겨찾기는 '내 PC' 바로 아래
        let pin_start = (0..slots.len()).find(|&i| matches!(side_of(i), Side::Pin(_) | Side::PinEmpty)).map_or(0.0, |i| slots[i].0.t);
        let fav_start = favs
            .first()
            .map(|&i| slots[i].0.t)
            .or_else(|| (0..slots.len()).find(|&i| side_of(i) == Side::Computer).map(|i| slots[i].0.b))
            .unwrap_or(0.0);
        let left = |idx: &[usize]| idx.iter().filter(|&&i| targets[i].is_some()).count();
        let fav_rows = |targets: &[Option<f32>]| -> Vec<Row> {
            favs.iter()
                .filter(|&&i| targets[i].is_some())
                .filter_map(|&i| match side_of(i) {
                    Side::Fav(r) => Some(r),
                    _ => None,
                })
                .collect()
        };
        let (to, gap_t, gap_group) = match dragged {
            // 그룹은 그룹 사이 틈(머리 줄 앞이나 맨 끝)에만 놓는다: 카드 위쪽에 가장 가까운 틈
            Side::Fav(Row::Group(_)) => {
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
                let gap_t = lay_out(&mut targets, &favs, fav_start, row_h, Some(b));
                lay_out(&mut targets, &pins, pin_start, row_h, None);
                (DropTo::FavGroup(Favorites::group_gap_target(&rem, b)), gap_t, None)
            }
            // 항목·고정한 줄: '모든 공간' 이름 줄보다 위면 고정한 쪽, 아니면 '모든 공간' 쪽에 빈자리
            _ => {
                let to_pins = self.side_drop_to_pins(y).unwrap_or(matches!(dragged, Side::Pin(_)));
                if to_pins {
                    let gap = gap_index(left(&pins), pin_start, row_h, center);
                    let gap_t = lay_out(&mut targets, &pins, pin_start, row_h, Some(gap));
                    lay_out(&mut targets, &favs, fav_start, row_h, None);
                    (DropTo::Pin(gap), gap_t, None)
                } else {
                    let rem = fav_rows(&targets);
                    let gap = gap_index(rem.len(), fav_start, row_h, center);
                    let (group, index) = self.favs.item_gap_target(&rem, gap);
                    let gap_t = lay_out(&mut targets, &favs, fav_start, row_h, Some(gap));
                    lay_out(&mut targets, &pins, pin_start, row_h, None);
                    (DropTo::FavItem(group, index), gap_t, group)
                }
            }
        };
        Some(DropPlan { to, dragged, card_t, card_h: row_h, targets, gap_t, gap_group })
    }

    /// 누른 줄에서 잡은 자리 (줄 위쪽에서 마우스까지)
    pub(super) fn side_grab(&self, s: Side, y: f32) -> f32 {
        let (slots, _) = self.side_slots();
        let top = slots.iter().find(|(_, Slot::Item(t))| *t == s).map_or(0.0, |(r, _)| r.t);
        (self.side_list_y(y) - top).clamp(0.0, SIDE_ROW_H)
    }

    /// 끌어서 놓았다: 보이던 빈자리 그대로 옮기고, 옮긴 줄은 카드 자리에서 새 자리로 미끄러져 들어가게 한다
    pub(super) fn side_drop(&mut self, p: DropPlan) {
        let key = match (p.dragged, p.to) {
            // 고정한 것 안에서: 빼낸 뒤 기준 자리를 pin_move의 옮기기 전 기준으로
            (Side::Pin(i), DropTo::Pin(gap)) => {
                let key = self.side_flow_key(Side::Pin(i));
                self.pin_move(i, if gap >= i { gap + 1 } else { gap });
                key
            }
            // '모든 공간' → 이 공간: '이 작업 공간에만 두기'와 같고, 놓은 자리에 끼운다
            (Side::Fav(row @ Row::Item { group, index }), DropTo::Pin(gap)) => {
                let path = self.favs.item(group, index).map(|f| f.path.clone()).unwrap_or_default();
                let before = self.pins().len();
                self.fav_move_to_pins(row);
                if self.pins().len() > before {
                    self.pin_move(before, gap);
                }
                format!("p|{}", path.to_lowercase())
            }
            // 이 공간 → '모든 공간': '모든 공간에 두기'와 같고, 놓은 자리(그룹)에 끼운다
            (Side::Pin(i), DropTo::FavItem(group, index)) => {
                let Some(path) = self.pins().get(i).map(|f| f.path.clone()) else { return };
                let already = self.favs.find(&path).is_some();
                self.pin_to_common(i);
                if !already
                    && let Some(from) = self.favs.find(&path)
                    && let Some((g, at)) = self.favs.move_item_to(from, group, index)
                {
                    self.fav_changed(Some(Row::Item { group: g, index: at }));
                }
                format!("f|{}", path.to_lowercase())
            }
            (Side::Fav(Row::Item { group, index }), DropTo::FavItem(to, at)) => {
                let key = self.side_flow_key(p.dragged);
                if let Some((g, i)) = self.favs.move_item_to((group, index), to, at) {
                    self.fav_changed(Some(Row::Item { group: g, index: i }));
                }
                key
            }
            (Side::Fav(Row::Group(g)), DropTo::FavGroup(to)) => {
                let key = self.side_flow_key(p.dragged);
                if let Some(to) = self.favs.move_group(g, to) {
                    self.fav_changed(Some(Row::Group(to)));
                }
                key
            }
            _ => return,
        };
        self.side_flow_start_at(key, p.card_t);
    }

    /// 자리를 줄 이름(움직임을 이어 가는 열쇠)으로: 같은 줄이면 자리가 바뀌어도 같은 이름
    pub(super) fn side_flow_key(&self, s: Side) -> String {
        match s {
            Side::Pin(i) => format!("p|{}", self.pins().get(i).map(|f| f.path.to_lowercase()).unwrap_or_default()),
            Side::Fav(Row::Item { group, index }) => format!("f|{}", self.favs.item(group, index).map(|f| f.path.to_lowercase()).unwrap_or_default()),
            Side::Fav(Row::Group(g)) => format!("g|{}", self.favs.groups.get(g).map(|g| g.name.as_str()).unwrap_or_default()),
            other => format!("{other:?}"),
        }
    }

    /// 놓은 줄이 카드 자리에서 새 자리로 미끄러져 들어가게 한다
    pub(super) fn side_flow_start_at(&mut self, key: String, y: f32) {
        self.side_flow.y.insert(key, y);
        self.side_flow.moving = true;
    }

    /// 줄마다 이번에 그릴 위쪽: 지난번 자리에서 목표 자리로 다가간다 (0.06초 시상수). 처음 보는 줄은 바로 목표 자리
    pub(super) fn side_flow_step(&mut self, keys: &[String], targets: &[Option<f32>]) -> Vec<Option<f32>> {
        let now = std::time::Instant::now();
        // 멈춰 있다 움직이기 시작하는 첫 그림은 한 틀(1/60초)만큼만 (오래 쉬다 그리면 한 번에 다 가 버리지 않게)
        let dt = match (self.side_flow.moving, self.side_flow.at) {
            (true, Some(at)) => now.duration_since(at).as_secs_f32().min(0.05),
            _ => 1.0 / 60.0,
        };
        self.side_flow.at = Some(now);
        let k = if motion_on() { 1.0 - (-dt / 0.06).exp() } else { 1.0 };
        let mut next = HashMap::with_capacity(keys.len());
        let mut moving = false;
        let out = keys
            .iter()
            .zip(targets)
            .map(|(key, t)| {
                let t = (*t)?;
                let cur = self.side_flow.y.get(key).copied().unwrap_or(t);
                let mut y = cur + (t - cur) * k;
                if (t - y).abs() < 0.5 {
                    y = t;
                } else {
                    moving = true;
                }
                next.insert(key.clone(), y);
                Some(y)
            })
            .collect();
        self.side_flow.y = next;
        self.side_flow.moving = moving;
        if moving {
            unsafe { SetTimer(Some(self.hwnd), TIMER_ANIM, 15, None) };
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
