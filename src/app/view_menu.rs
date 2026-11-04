//! 보기 방식 메뉴: 경로 상자 오른쪽 보기 버튼을 누르면 버튼 아래에 뜨는 메뉴.
//! Windows 기본 메뉴 대신 즐겨찾기 팝업과 같은 모양(그림자·둥근 모서리·테마 색)으로 직접 그린다

use super::*;

const ITEM_H: f32 = 32.0;
const PAD: f32 = 6.0;
const MENU_W: f32 = 240.0;
const G_CHECK: u16 = 0xE73E;

/// 열린 보기 메뉴
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct ViewMenu {
    pub pane: usize,
    /// 마우스나 ↑↓로 가리킨 줄
    pub sel: usize,
}

impl App {
    /// 메뉴 자리: 보기 버튼 바로 아래, 오른쪽 끝을 버튼에 맞춘다. 창 밖으로 나가지 않게
    pub(super) fn view_menu_rect(&self) -> Option<R> {
        let m = self.view_menu?;
        if m.pane >= self.layout.count() {
            return None;
        }
        let b = geom(self.pane_rect(m.pane)?).view;
        let h = PAD * 2.0 + ITEM_H * ViewMode::ALL.len() as f32;
        let w = MENU_W.min(self.w - 2.0 * MARGIN);
        let l = (b.r - w).max(MARGIN);
        let t = (b.b + 4.0).min(self.h - h - MARGIN).max(MARGIN);
        Some(R::new(l, t, l + w, t + h))
    }

    fn view_menu_item(menu: R, i: usize) -> R {
        let t = menu.t + PAD + i as f32 * ITEM_H;
        R::new(menu.l + PAD, t, menu.r - PAD, t + ITEM_H)
    }

    pub(super) fn view_menu_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let r = self.view_menu_rect()?;
        if !r.contains(x, y) {
            return None;
        }
        Some((0..ViewMode::ALL.len()).find(|&i| Self::view_menu_item(r, i).contains(x, y)).map_or(Hit::ViewMenuPanel, Hit::ViewMenuItem))
    }

    /// 보기 버튼: 닫혀 있으면 열고, 같은 칸의 메뉴가 열려 있으면 닫는다
    pub(super) fn toggle_view_menu(&mut self, p: usize) {
        if self.view_menu.is_some_and(|m| m.pane == p) {
            self.close_view_menu();
            return;
        }
        self.close_favorites();
        self.close_palette();
        self.end_edits_for_click();
        self.active = p;
        let sel = ViewMode::ALL.iter().position(|v| *v == self.panes[p].view).unwrap_or(0);
        self.view_menu = Some(ViewMenu { pane: p, sel });
        // 필터 상자(자식 창)가 메뉴를 가리지 않게 비켜 둔다
        self.layout_edits();
        self.invalidate();
    }

    pub(super) fn close_view_menu(&mut self) {
        if self.view_menu.take().is_some() {
            self.layout_edits();
            self.invalidate();
        }
    }

    fn pick_view(&mut self, i: usize) {
        let Some(m) = self.view_menu else { return };
        self.close_view_menu();
        if let Some(v) = ViewMode::ALL.get(i) {
            self.set_view(m.pane, *v);
            self.save_session();
        }
    }

    /// 메뉴가 열려 있을 때 누른 곳. 메뉴 밖을 누르면 닫기만 하고 그 클릭은 뒤로 넘기지 않는다 (Windows 메뉴와 같게)
    pub(super) fn view_menu_mouse_down(&mut self, hit: Hit) {
        match hit {
            Hit::ViewMenuItem(i) => self.pick_view(i),
            Hit::ViewMenuPanel => {}
            _ => self.close_view_menu(),
        }
    }

    /// 마우스를 올린 줄을 가리킨다 (키보드로 가리킨 줄과 같은 표시)
    pub(super) fn view_menu_hover(&mut self, hit: Hit) {
        if let (Hit::ViewMenuItem(i), Some(m)) = (hit, self.view_menu.as_mut()) {
            m.sel = i;
        }
    }

    /// 메뉴가 열려 있는 동안의 키. 메뉴 조작 밖의 키는 삼킨다
    pub(super) fn view_menu_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        if self.view_menu_rect().is_none() {
            self.view_menu = None;
            return false;
        }
        let Some(m) = self.view_menu.as_mut() else { return false };
        let n = ViewMode::ALL.len();
        match vk {
            VK_UP => m.sel = (m.sel + n - 1) % n,
            VK_DOWN => m.sel = (m.sel + 1) % n,
            VK_HOME | VK_PRIOR => m.sel = 0,
            VK_END | VK_NEXT => m.sel = n - 1,
            VK_RETURN | VK_SPACE => {
                let i = m.sel;
                self.pick_view(i);
                return true;
            }
            VK_ESCAPE | VK_TAB | VK_LEFT | VK_RIGHT => {
                self.close_view_menu();
                return true;
            }
            _ => return true,
        }
        self.invalidate();
        true
    }

    pub(super) fn paint_view_menu(&mut self) {
        let (Some(r), Some(m)) = (self.view_menu_rect(), self.view_menu) else { return };
        let t = self.theme;
        let current = self.panes[m.pane].view;
        use crate::keys::Action;
        let action = |v: ViewMode| match v {
            ViewMode::Huge => Action::Huge,
            ViewMode::Large => Action::Thumbs,
            ViewMode::Medium => Action::Medium,
            ViewMode::Small => Action::SmallIcons,
            ViewMode::List => Action::List,
            ViewMode::Details => Action::Details,
        };
        let keys: Vec<String> = ViewMode::ALL.iter().map(|v| self.keys.get(action(*v)).map(|c| c.label()).unwrap_or_default()).collect();
        let gfx = &self.gfx;
        let shadow = Color { r: 0.0, g: 0.0, b: 0.0, a: if t.dark { 0.45 } else { 0.12 } };

        // 그림자 + 본체 (즐겨찾기 팝업과 같은 모양)
        gfx.fill_round(R::new(r.l - 2.0, r.t + 1.0, r.r + 2.0, r.b + 4.0), R_PANE + 2.0, shadow);
        gfx.fill_round(r, R_PANE, t.pane);
        gfx.stroke_round(r.inset(0.5, 0.5), R_PANE, t.pane_border, 1.0);

        for (i, v) in ViewMode::ALL.into_iter().enumerate() {
            let ir = Self::view_menu_item(r, i);
            if i == m.sel {
                gfx.fill_round(ir, R_ROW, t.hover);
            }
            let on = v == current;
            if on {
                gfx.glyph(G_CHECK, R::new(ir.l + 6.0, ir.t, ir.l + 28.0, ir.b), true, t.accent);
            }
            let key_w = if keys[i].is_empty() { 0.0 } else { gfx.measure(&keys[i], Font::Small) + 12.0 };
            gfx.text(v.label(), R::new(ir.l + 34.0, ir.t, ir.r - key_w - 8.0, ir.b), if on { Font::Strong } else { Font::Normal }, t.text);
            if key_w > 0.0 {
                gfx.text(&keys[i], R::new(ir.r - key_w, ir.t, ir.r - 10.0, ir.b), Font::Small, t.dim);
            }
        }
    }
}
