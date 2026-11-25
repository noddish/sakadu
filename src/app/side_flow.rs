//! 사이드바 줄의 움직임. 끄는 줄은 흰 카드로 떠서 마우스를 따라오고, 다른 줄은 놓일 빈자리를 만들며 미끄러진다.
//! 그룹 안에 빈자리가 생기면 그 그룹을 둥근 판으로 묶어 보인다. 놓을 곳은 보이는 빈자리 그대로 정한다 (DropPlan).
//! 끌기가 아니어도 줄의 자리가 바뀌면(그룹 만들기·옮기기·고정·풀기) 새 자리로 미끄러져 간다.
//! 시스템의 '애니메이션 효과'를 끄면 미끄러지지 않고 바로 옮겨진다.
//! 빈자리·놓을 곳 계산과 움직임 걸음은 맥과 함께 쓰는 공통 코드(ui::side_flow)다: 여기서는 사이드바 자리를 그것에 넘기고 놓은 뒤 옮긴다

use super::sidebar::{Side, Slot, motion_on};
use super::*;
use crate::favorites::Row;
use sakadu::ui::side_flow::{FlowKind, FlowSlot, drop_plan};

pub(super) use sakadu::ui::side_flow::{DropTo, SideFlow};

/// 끄는 동안의 그림과 놓을 곳 (공통 drop_plan의 결과에 끄는 줄을 더한 것)
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

/// 사이드바 줄이 공통 끌기 계산에서 맡는 몫: 내 PC는 즐겨찾기 바로 위 줄이다
pub(super) fn flow_kind(s: Side) -> FlowKind {
    match s {
        Side::Pin(i) => FlowKind::Pin(i),
        Side::PinEmpty => FlowKind::PinHint,
        Side::Fav(row) => FlowKind::Fav(row),
        Side::Computer => FlowKind::FavAnchor,
        Side::CommonHead | Side::FavNewGroup | Side::FavSort | Side::FavAdd | Side::FavEmpty => FlowKind::Other,
    }
}

/// 공통 끌기 계산에 넘길 목록 자리들 (side_slots 차례)
pub(super) fn flow_slots(slots: &[(R, Slot)]) -> Vec<FlowSlot> {
    slots.iter().map(|(r, Slot::Item(s))| FlowSlot { t: r.t, b: r.b, kind: flow_kind(*s) }).collect()
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

    /// 끄는 동안의 그림과 놓을 곳 (공통 drop_plan: 빈자리는 카드 가운데가 지난 줄 다음, 그룹은 그룹 사이 틈에만). 끄는 중이 아니면 None
    pub(super) fn side_drop_plan(&self, slots: &[(R, Slot)]) -> Option<DropPlan> {
        let (dragged, y, grab) = self.side_dragged()?;
        let card_t = self.side_list_y(y) - grab;
        // '모든 공간' 이름 줄보다 위면 고정한 쪽 (목록 밖이면 끄는 줄이 있던 쪽)
        let p = drop_plan(&flow_slots(slots), flow_kind(dragged), card_t, SIDE_ROW_H, self.side_drop_to_pins(y), &self.favs);
        Some(DropPlan { to: p.to, dragged, card_t: p.card_t, card_h: p.card_h, targets: p.targets, gap_t: p.gap_t, gap_group: p.gap_group })
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
        self.side_flow.start_at(key, y);
    }

    /// 줄마다 이번에 그릴 위쪽: 지난번 자리에서 목표 자리로 다가간다 (공통 SideFlow, 0.06초 시상수). 처음 보는 줄은 바로 목표 자리.
    /// 아직 움직이면 다음 그림을 위해 애니메이션 타이머를 건다
    pub(super) fn side_flow_step(&mut self, keys: &[String], targets: &[Option<f32>]) -> Vec<Option<f32>> {
        let out = self.side_flow.step(keys, targets, std::time::Instant::now(), motion_on());
        if self.side_flow.moving {
            unsafe { SetTimer(Some(self.hwnd), TIMER_ANIM, 15, None) };
        }
        out
    }
}
