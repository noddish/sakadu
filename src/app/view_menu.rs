//! 도구 막대의 펼치는 메뉴: 보기 방식 단추나 칸 나누기 단추를 누르면 그 아래에 뜬다.
//! Windows 기본 메뉴 대신 떠 있는 패널과 같은 결(그림자·둥근 모서리·테마 색)로 직접 그린다

use super::*;
use crate::keys::Action;

const ITEM_H: f32 = 32.0;
const PAD: f32 = 4.0;
const MENU_W: f32 = 240.0;
const G_CHECK: u16 = 0xE73E;
const LAYOUT_KEYS: [Action; 5] = [Action::Layout1, Action::Layout2, Action::Layout3, Action::Layout4, Action::Layout5];

/// 어느 메뉴인지
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum MenuKind {
    /// 보기 방식 여섯 가지(ViewMode::ALL 순서)와 맨 아래 '숨김 항목 보기'
    View,
    /// 칸 나누기 다섯 가지 (Layout::ALL 순서)
    Layout,
}

impl MenuKind {
    fn len(self) -> usize {
        match self {
            MenuKind::View => ViewMode::ALL.len() + 1,
            MenuKind::Layout => Layout::ALL.len(),
        }
    }
}

/// 열린 메뉴
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct ViewMenu {
    pub kind: MenuKind,
    /// 보기 방식을 바꿀 칸 (칸 나누기 메뉴에서는 쓰지 않는다)
    pub pane: usize,
    /// 마우스나 ↑↓로 가리킨 줄
    pub sel: usize,
}

fn view_action(v: ViewMode) -> Action {
    match v {
        ViewMode::Huge => Action::Huge,
        ViewMode::Large => Action::Thumbs,
        ViewMode::Medium => Action::Medium,
        ViewMode::Small => Action::SmallIcons,
        ViewMode::List => Action::List,
        ViewMode::Details => Action::Details,
    }
}

impl App {
    /// 메뉴 자리: 누른 단추 바로 아래, 오른쪽 끝을 단추에 맞춘다. 창 밖으로 나가지 않게
    pub(super) fn view_menu_rect(&self) -> Option<R> {
        let m = self.view_menu?;
        if m.kind == MenuKind::View && m.pane >= self.layout.count() {
            return None;
        }
        let bar = self.toolbar();
        let b = if m.kind == MenuKind::View { bar.view } else { bar.layout };
        let h = PAD * 2.0 + ITEM_H * m.kind.len() as f32;
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
        let n = self.view_menu?.kind.len();
        Some((0..n).find(|&i| Self::view_menu_item(r, i).contains(x, y)).map_or(Hit::ViewMenuPanel, Hit::ViewMenuItem))
    }

    fn open_menu(&mut self, kind: MenuKind, pane: usize, sel: usize) {
        self.fav_blur();
        // 검색 패널은 두고 키보드만 칸으로 (보기를 바꾸고 결과로 돌아갈 수 있게)
        self.pal_blur();
        self.end_edits_for_click();
        self.view_menu = Some(ViewMenu { kind, pane, sel });
        self.start_anim(|a| &mut a.view_menu_at);
        // 필터 상자(자식 창)가 메뉴를 가리지 않게 비켜 둔다
        self.layout_edits();
        self.invalidate();
    }

    /// 보기 방식 단추: 닫혀 있으면 열고, 보기 메뉴가 열려 있으면 닫는다. 칸 p의 보기를 바꾼다
    pub(super) fn toggle_view_menu(&mut self, p: usize) {
        if self.view_menu.is_some_and(|m| m.kind == MenuKind::View) {
            self.close_view_menu();
            return;
        }
        self.active = p;
        let sel = ViewMode::ALL.iter().position(|v| *v == self.panes[p].view).unwrap_or(0);
        self.open_menu(MenuKind::View, p, sel);
    }

    /// 칸 나누기 단추: 닫혀 있으면 열고, 칸 나누기 메뉴가 열려 있으면 닫는다
    pub(super) fn toggle_layout_menu(&mut self) {
        if self.view_menu.is_some_and(|m| m.kind == MenuKind::Layout) {
            self.close_view_menu();
            return;
        }
        self.open_menu(MenuKind::Layout, self.active, self.layout.index());
    }

    pub(super) fn close_view_menu(&mut self) {
        if self.view_menu.take().is_some() {
            self.layout_edits();
            self.invalidate();
        }
    }

    fn pick_menu_item(&mut self, i: usize) {
        let Some(m) = self.view_menu else { return };
        self.close_view_menu();
        match m.kind {
            MenuKind::View => match ViewMode::ALL.get(i) {
                Some(v) => {
                    self.set_view(m.pane, *v);
                    self.save_session();
                }
                // 맨 아래 줄: 숨김 항목 보기
                None => self.toggle_hidden(),
            },
            MenuKind::Layout => {
                if let Some(l) = Layout::ALL.get(i) {
                    self.set_layout(*l);
                }
            }
        }
    }

    /// 메뉴가 열려 있을 때 누른 곳. 메뉴 밖을 누르면 닫기만 하고 그 클릭은 뒤로 넘기지 않는다 (Windows 메뉴와 같게)
    pub(super) fn view_menu_mouse_down(&mut self, hit: Hit) {
        match hit {
            Hit::ViewMenuItem(i) => self.pick_menu_item(i),
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
        // Alt+F4는 메뉴를 닫고 창 닫기로 넘긴다
        if vk == VK_F4 && key_down(VK_MENU) {
            self.close_view_menu();
            return false;
        }
        if self.view_menu_rect().is_none() {
            self.view_menu = None;
            return false;
        }
        let Some(m) = self.view_menu.as_mut() else { return false };
        let n = m.kind.len();
        match vk {
            VK_UP => m.sel = (m.sel + n - 1) % n,
            VK_DOWN => m.sel = (m.sel + 1) % n,
            VK_HOME | VK_PRIOR => m.sel = 0,
            VK_END | VK_NEXT => m.sel = n - 1,
            VK_RETURN | VK_SPACE => {
                let i = m.sel;
                self.pick_menu_item(i);
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
        // 줄마다: (이름, 지금 것인지, 단축키 글자)
        let key = |a: Action| self.keys.get(a).map(|c| c.label()).unwrap_or_default();
        let items: Vec<(&str, bool, String)> = match m.kind {
            MenuKind::View => {
                let current = self.panes[m.pane].view;
                let mut v: Vec<_> = ViewMode::ALL.iter().map(|v| (v.label(), *v == current, key(view_action(*v)))).collect();
                v.push(("숨김 항목 보기", self.show_hidden, key(Action::Hidden)));
                v
            }
            MenuKind::Layout => Layout::ALL.iter().zip(LAYOUT_KEYS).map(|(l, a)| (crate::keys::name(a), *l == self.layout, key(a))).collect(),
        };
        let gfx = &self.gfx;
        // 열릴 때 위에서 4px 내려오며 나타난다
        let pushed = anim_progress(self.view_menu_at, MENU_FADE_MS).is_some_and(|f| gfx.push_fade(f, 0.0, -(1.0 - f) * 4.0));

        // 그림자 + 본체 (떠 있는 다른 것과 같은 그림자)
        if !t.high_contrast {
            soft_shadow(gfx, r, R_PANE, 12.0, 0.16, t.dark);
        }
        gfx.fill_round(r, R_PANE, t.pane);
        gfx.stroke_round(r.inset(0.5, 0.5), R_PANE, t.pane_border, 1.0);

        for (i, (label, on, keys)) in items.iter().enumerate() {
            let ir = Self::view_menu_item(r, i);
            if i == m.sel {
                gfx.fill_round(ir, R_ROW, t.hover);
            }
            // 보기 방식과 숨김 항목 사이의 가는 선
            if m.kind == MenuKind::View && i == ViewMode::ALL.len() {
                gfx.fill(R::new(ir.l + 6.0, ir.t, ir.r - 6.0, ir.t + 1.0), t.border);
            }
            if *on {
                gfx.glyph(G_CHECK, R::new(ir.l + 4.0, ir.t, ir.l + 26.0, ir.b), true, t.text);
            }
            // 그림: 고를 모양을 이름 앞에
            let icon = R::new(ir.l + 28.0, ir.t, ir.l + 52.0, ir.b);
            match m.kind {
                MenuKind::View => match ViewMode::ALL.get(i) {
                    Some(v) => draw_view_icon(gfx, icon, *v, t.dim),
                    None => gfx.glyph(G_HIDDEN, icon, false, t.dim),
                },
                MenuKind::Layout => draw_layout_icon(gfx, icon, Layout::ALL[i], t.dim),
            }
            let key_w = if keys.is_empty() { 0.0 } else { gfx.measure(keys, Font::Small) + 12.0 };
            gfx.text(label, R::new(ir.l + 60.0, ir.t, ir.r - key_w - 8.0, ir.b), Font::Normal, t.text);
            if key_w > 0.0 {
                gfx.text(keys, R::new(ir.r - key_w, ir.t, ir.r - 10.0, ir.b), Font::Small, t.dim);
            }
        }
        if pushed {
            gfx.pop_fade();
        }
    }
}
