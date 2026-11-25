//! Ctrl+K 명령 메뉴: 열린 탭·고정한 것·즐겨찾기·최근 위치·작업 공간으로 가기와 명령 실행을 한 상자에서.
//! 디스크 전체에서 찾기는 검색 패널(Ctrl+F)이 맡고, 여기서는 마지막 줄로만 이어 준다.
//! 줄 고르기·판의 자리·그리기는 맥과 함께 쓰는 sakadu::ui::commands. 여기는 입력 상자(자식 창), 키, 고른 줄 열기, 떠오르는 효과

use super::*;
pub(super) use sakadu::ui::commands::CmdMenu;
use sakadu::ui::commands::{CmdArt, CmdHit, CmdItem, CmdOpen, CmdScene, CmdSource, CmdStep, cmd_input_rect, cmd_items, cmd_list, cmd_step, paint_cmd_scrim, paint_commands as paint_cmd_panel};

pub(super) const CMD_FADE_MS: f32 = 160.0;

/// 줄 아이콘: 미리 구해 둔 시스템 아이콘 목록 번호 (없으면 -1)
struct WinCmdArt<'a> {
    icons: &'a [i32],
}

impl CmdArt<Gfx> for WinCmdArt<'_> {
    fn icon(&mut self, g: &mut Gfx, i: usize, _item: &CmdItem, r: R) {
        if let Some(&icon) = self.icons.get(i).filter(|icon| **icon >= 0) {
            g.icon(icon, r);
        }
    }
}

impl App {
    /// Ctrl+K: 열려 있으면 닫는다. 이름 바꾸기·경로 입력 중에는 열지 않는다
    pub(super) fn toggle_commands(&mut self) {
        if self.cmd.is_some() {
            return self.close_commands();
        }
        if self.edit_mode != EditMode::None || self.settings.is_some() {
            return;
        }
        if self.view_menu.is_some() {
            self.close_view_menu();
        }
        unsafe {
            let _ = SetWindowTextW(self.cmd_edit, &HSTRING::new());
        }
        self.cmd = Some(CmdMenu::new(self.cmd_items("")));
        self.start_anim(|a| &mut a.cmd_at);
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    pub(super) fn close_commands(&mut self) {
        if self.cmd.take().is_some() {
            self.layout_edits();
            self.focus();
            self.invalidate();
        }
    }

    /// 입력이 바뀌었다
    pub(super) fn cmd_rebuild(&mut self) {
        if self.cmd.is_none() {
            return;
        }
        let items = self.cmd_items(&edit_text(self.cmd_edit));
        if let Some(c) = self.cmd.as_mut() {
            c.set_items(items);
        }
        self.layout_edits();
        self.invalidate();
    }

    /// 보여 줄 줄 (공통 규칙: sakadu::ui::commands::cmd_items)
    fn cmd_items(&self, text: &str) -> Vec<CmdItem> {
        let src = CmdSource {
            panes: (0..self.layout.count()).map(|p| self.panes[p].tabs.as_slice()).collect(),
            pins: self.pins(),
            favs: &self.favs,
            recent: &self.recent,
            workspaces: &self.workspaces,
            runs: |_| true,
        };
        cmd_items(&src, text)
    }

    /// 카드 자리: 창 가운데, 위에서 20%
    pub(super) fn cmd_rect(&self) -> Option<R> {
        self.cmd.as_ref().map(|c| c.rect(self.w, self.h))
    }

    /// 입력 상자 자리 (돋보기 오른쪽)
    pub(super) fn cmd_input_rect(&self) -> Option<R> {
        self.cmd_rect().map(cmd_input_rect)
    }

    fn cmd_list(&self) -> Option<R> {
        self.cmd_rect().map(cmd_list)
    }

    /// 명령 메뉴가 열려 있으면 무엇을 눌렀는지 (카드 밖은 닫기)
    pub(super) fn cmd_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let r = self.cmd_rect()?;
        Some(match self.cmd.as_ref()?.hit(r, x, y) {
            CmdHit::Scrim => Hit::CmdScrim,
            CmdHit::Row(i) => Hit::CmdRow(i),
            CmdHit::Panel => Hit::CmdPanel,
        })
    }

    pub(super) fn cmd_click(&mut self, hit: Hit) {
        match hit {
            Hit::CmdRow(i) => {
                let how = if key_down(VK_CONTROL) { CmdOpen::NewTab } else if key_down(VK_MENU) { CmdOpen::Other } else { CmdOpen::Here };
                self.cmd_run(i, how);
            }
            Hit::CmdPanel => {}
            _ => self.close_commands(),
        }
    }


    pub(super) fn cmd_hover(&mut self, hit: Hit) {
        if let (Hit::CmdRow(i), Some(c)) = (hit, self.cmd.as_mut()) {
            c.sel = i;
        }
    }

    pub(super) fn cmd_wheel(&mut self, dy: f32) {
        let Some(list) = self.cmd_list() else { return };
        if let Some(c) = self.cmd.as_mut() {
            c.wheel(dy, list.h());
        }
        self.invalidate();
    }

    fn cmd_ensure_visible(&mut self) {
        let Some(list) = self.cmd_list() else { return };
        if let Some(c) = self.cmd.as_mut() {
            c.reveal(list.h());
        }
    }

    /// 명령 메뉴 입력 상자에서 누른 키
    pub(super) fn cmd_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let Some(n) = self.cmd.as_ref().map(|c| c.items.len()) else { return false };
        let sel = self.cmd.as_ref().map_or(0, |c| c.sel);
        let to = match vk {
            VK_UP => Some(cmd_step(sel, n, CmdStep::Up)),
            VK_DOWN => Some(cmd_step(sel, n, CmdStep::Down)),
            VK_PRIOR => Some(cmd_step(sel, n, CmdStep::PageUp)),
            VK_NEXT => Some(cmd_step(sel, n, CmdStep::PageDown)),
            VK_RETURN => {
                let how = if key_down(VK_CONTROL) { CmdOpen::NewTab } else if key_down(VK_MENU) { CmdOpen::Other } else { CmdOpen::Here };
                self.cmd_run(sel, how);
                return true;
            }
            VK_ESCAPE => {
                self.close_commands();
                return true;
            }
            VK_TAB => return true,
            // Ctrl+K를 다시 누르면 닫는다
            _ if key_down(VK_CONTROL) && vk.0 == b'K' as u16 => {
                self.close_commands();
                return true;
            }
            _ => return false,
        };
        if let (Some(i), Some(c)) = (to, self.cmd.as_mut()) {
            c.sel = i;
        }
        self.cmd_ensure_visible();
        self.invalidate();
        true
    }

    /// 고른 줄을 연다·실행한다
    fn cmd_run(&mut self, i: usize, how: CmdOpen) {
        let Some(item) = self.cmd.as_ref().and_then(|c| c.items.get(i)).cloned() else { return };
        self.close_commands();
        // 옆 칸: 칸이 여럿이면 지금 칸 다음 칸에서 연다
        if how == CmdOpen::Other && self.layout.count() > 1 && !matches!(item, CmdItem::Tab { .. } | CmdItem::Command(_) | CmdItem::Workspace(..) | CmdItem::SearchDisk(_)) {
            let next = (self.active + 1) % self.layout.count();
            self.activate_pane(next);
        }
        let new_tab = how == CmdOpen::NewTab;
        match item {
            CmdItem::Tab { pane, tab, .. } => {
                if pane < self.layout.count() && tab < self.panes[pane].tabs.len() {
                    self.activate_pane(pane);
                    self.activate_tab(pane, tab);
                }
            }
            CmdItem::Fav { group, index, .. } => self.fav_open_entry(group, index, new_tab),
            CmdItem::Pin { index, .. } => self.pin_open(index, new_tab),
            CmdItem::Recent(path) => {
                let p = self.active;
                let tab = self.panes[p].tab();
                let then = ProbeThen::Favorite { tab: tab.id, from: tab.loc.clone(), new_tab, is_dir: Some(true) };
                self.probe(p, path, then);
            }
            CmdItem::GoPath(text) => {
                let p = self.active;
                let tab = self.panes[p].tab();
                let then = if new_tab { ProbeThen::Favorite { tab: tab.id, from: tab.loc.clone(), new_tab, is_dir: None } } else { ProbeThen::Address { tab: tab.id, from: tab.loc.clone() } };
                self.probe(p, text, then);
            }
            CmdItem::Workspace(i, _) => self.switch_workspace(i),
            CmdItem::Command(a) => self.run_action(a),
            CmdItem::SearchDisk(q) => {
                if !self.pal_shown() {
                    self.open_palette();
                }
                unsafe {
                    let _ = SetWindowTextW(self.search_edit, &HSTRING::from(q.as_str()));
                }
                self.pal_query_changed();
            }
        }
        self.invalidate();
    }

    pub(super) fn paint_commands(&mut self) {
        let Some(r) = self.cmd_rect() else { return };
        let t = self.theme;
        let (w, h) = (self.w, self.h);
        let multi = self.layout.count() > 1;
        let folder_icon = self.type_icon("", true);
        // 줄마다 아이콘 (즐겨찾기 파일은 형식 아이콘)
        let items = self.cmd.as_ref().map(|c| c.items.clone()).unwrap_or_default();
        let icons: Vec<i32> = items
            .iter()
            .map(|it| match it {
                CmdItem::Fav { path, is_dir: false, .. } | CmdItem::Pin { path, is_dir: false, .. } => {
                    let ext = util::file_name(path).rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                    self.type_icon(&ext, false)
                }
                CmdItem::Tab { .. } | CmdItem::Fav { .. } | CmdItem::Pin { .. } | CmdItem::Recent(_) | CmdItem::GoPath(_) => folder_icon,
                _ => -1,
            })
            .collect();
        let fade = anim_progress(self.cmd_at, CMD_FADE_MS);
        let App { gfx: g, cmd, .. } = self;
        let Some(c) = cmd.as_ref() else { return };

        // 뒤를 옅게 가린다
        paint_cmd_scrim(g, &t, w, h);
        let pushed = fade.is_some_and(|f| g.push_fade(f, 0.0, -(1.0 - f) * 6.0));
        paint_cmd_panel(g, &mut WinCmdArt { icons: &icons }, &t, &CmdScene { menu: c, rect: r, multi });
        if pushed {
            g.pop_fade();
        }
    }
}
