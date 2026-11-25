//! 명령 메뉴 (⌘K, Windows Ctrl+K와 같다): 열린 탭·고정한 것·즐겨찾기·최근 위치·작업 공간으로 가기와 명령 실행을 한 상자에서.
//! 줄 고르기·판의 자리·그리기는 sakadu::ui::commands. 입력 칸은 창 쪽의 세 번째 글 상자로, 판의 입력 자리(cmd_field_rect)에 놓인다:
//! 글이 바뀌면 cmd_typed, 그 칸의 키(↑↓, Page Up/Down, Return, ⌘Return 새 탭, ⌥Return 옆 칸, Esc)는 cmd_key로 받는다.
//! 경로처럼 보이는 글(/…, ~/…)은 경로 입력과 같은 길(뒷줄에서 확인)로 간다. 디스크 전체에서 찾기는 검색(⌘F)으로 이어 준다

use super::*;
use sakadu::ui::commands::{CmdArt, CmdHit, CmdItem, CmdMenu, CmdOpen, CmdScene, CmdSource, CmdStep, cmd_input_rect, cmd_items, cmd_list, cmd_row_text, cmd_step, paint_cmd_scrim, paint_commands};

/// 줄 아이콘: 그 경로의 Finder 아이콘. 시작 화면 탭과 친 경로는 폴더 그림
struct MacCmdArt {
    dim: Color,
}

impl CmdArt<Gfx> for MacCmdArt {
    fn icon(&mut self, g: &mut Gfx, _i: usize, item: &CmdItem, r: R) {
        match item {
            CmdItem::Tab { path: Some(path), .. } | CmdItem::Fav { path, .. } | CmdItem::Pin { path, .. } | CmdItem::Recent(path) => g.icon_for_path(path, r),
            _ => g.glyph(G_PLACES, r, false, self.dim),
        }
    }
}

impl App {
    /// ⌘K: 열려 있으면 닫는다. 경로·이름을 고치는 중에는 열지 않는다. 떠 있는 메뉴는 닫는다
    pub fn toggle_commands(&mut self) {
        if self.cmd.is_some() {
            return self.close_commands();
        }
        if self.edit != Editing::None {
            return;
        }
        self.menu = None;
        self.cmd_text.clear();
        self.cmd = Some(CmdMenu::new(self.cmd_items("")));
    }

    pub fn close_commands(&mut self) {
        self.cmd = None;
    }

    /// 보여 줄 줄 (공통 규칙: sakadu::ui::commands::cmd_items). 명령은 맥에서 도는 것만
    fn cmd_items(&self, text: &str) -> Vec<CmdItem> {
        let src = CmdSource {
            panes: (0..self.layout.count()).map(|p| self.panes[p].tabs.as_slice()).collect(),
            pins: self.pins(),
            favs: &self.favs,
            recent: &self.recent,
            workspaces: &self.workspaces,
            runs: mac_runs,
        };
        cmd_items(&src, text)
    }

    /// 판 자리: 창 가운데, 위에서 20%
    pub(super) fn cmd_rect(&self) -> Option<R> {
        self.cmd.as_ref().map(|c| c.rect(self.w, self.h))
    }

    /// 입력 칸 자리 (돋보기 오른쪽. 창 쪽 글 상자가 여기에 놓인다). 닫혀 있으면 None
    pub fn cmd_field_rect(&self) -> Option<R> {
        self.cmd_rect().map(cmd_input_rect)
    }

    /// 입력 칸의 글 (열 때마다 비운다)
    pub fn cmd_text(&self) -> &str {
        &self.cmd_text
    }

    /// 입력 칸의 글이 바뀌었다 (창 쪽이 글자마다 알려 준다. 한글은 조합 중인 글자까지): 줄을 다시 고르고 맨 위 줄을 고른다
    pub fn cmd_typed(&mut self, text: &str) {
        if self.cmd.is_none() || self.cmd_text == text {
            return;
        }
        self.cmd_text = text.to_string();
        let items = self.cmd_items(text);
        if let Some(c) = self.cmd.as_mut() {
            c.set_items(items);
        }
    }

    /// 명령 메뉴가 열려 있으면 (x, y)에 무엇이 있는지 (판 밖은 닫기)
    pub(super) fn cmd_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let r = self.cmd_rect()?;
        Some(match self.cmd.as_ref()?.hit(r, x, y) {
            CmdHit::Scrim => Hit::CmdScrim,
            CmdHit::Row(i) => Hit::CmdRow(i),
            CmdHit::Panel => Hit::CmdPanel,
        })
    }

    /// 명령 메뉴가 열려 있는 동안의 누름: 줄이면 연다(⌘는 새 탭, ⌥는 옆 칸), 판 밖이면 닫는다
    pub(super) fn cmd_press(&mut self, hit: Hit, m: Mods) {
        match hit {
            Hit::CmdRow(i) => self.cmd_run(i, open_how(m)),
            Hit::CmdPanel => {}
            _ => self.close_commands(),
        }
    }

    /// 판 위의 스크롤
    pub(super) fn cmd_wheel(&mut self, dy: f32) {
        let Some(list) = self.cmd_rect().map(cmd_list) else { return };
        if let Some(c) = self.cmd.as_mut() {
            c.wheel(dy, list.h());
        }
    }

    /// 명령 메뉴의 키 (입력 칸이 넘겨준다): ↑↓ 줄 옮기기(끝에서 넘어간다), Page Up/Down, Return 열기(⌘ 새 탭, ⌥ 옆 칸), Esc 닫기.
    /// Tab은 삼킨다. 받았으면 true
    pub fn cmd_key(&mut self, key: Key, m: Mods) -> bool {
        let Some((n, sel)) = self.cmd.as_ref().map(|c| (c.items.len(), c.sel)) else { return false };
        let step = match key {
            Key::Up => CmdStep::Up,
            Key::Down => CmdStep::Down,
            Key::PageUp => CmdStep::PageUp,
            Key::PageDown => CmdStep::PageDown,
            Key::Return => {
                self.cmd_run(sel, open_how(m));
                return true;
            }
            Key::Esc => {
                self.close_commands();
                return true;
            }
            Key::Tab => return true,
            _ => return false,
        };
        let list = self.cmd_rect().map(cmd_list);
        if let Some(c) = self.cmd.as_mut() {
            c.sel = cmd_step(sel, n, step);
            if let Some(list) = list {
                c.reveal(list.h());
            }
        }
        true
    }

    /// 고른 줄을 연다·실행한다
    fn cmd_run(&mut self, i: usize, how: CmdOpen) {
        let Some(item) = self.cmd.as_ref().and_then(|c| c.items.get(i)).cloned() else { return };
        self.close_commands();
        // 옆 칸: 칸이 여럿이면 지금 칸 다음 칸에서 연다
        if how == CmdOpen::Other && self.layout.count() > 1 && !matches!(item, CmdItem::Tab { .. } | CmdItem::Command(_) | CmdItem::Workspace(..) | CmdItem::SearchDisk(_)) {
            self.cycle_pane();
        }
        let new_tab = how == CmdOpen::NewTab;
        let p = self.active;
        match item {
            CmdItem::Tab { pane, tab, .. } => {
                if pane < self.layout.count() && tab < self.panes[pane].tabs.len() {
                    self.activate_pane(pane);
                    self.activate_tab(pane, tab);
                }
            }
            CmdItem::Fav { path, is_dir, .. } | CmdItem::Pin { path, is_dir, .. } => self.open_place(p, path, is_dir, new_tab),
            CmdItem::Recent(path) => self.open_place(p, path, true, new_tab),
            CmdItem::GoPath(text) => self.probe_path(p, text, new_tab),
            CmdItem::Workspace(i, _) => self.switch_workspace(i),
            CmdItem::Command(a) => {
                self.run_action(a);
            }
            // 검색을 열고 그 글로 찾는다 (검색이 꺼져 있으면 '검색 켜기' 카드가 보인다)
            CmdItem::SearchDisk(q) => {
                self.open_find();
                self.find_typed(&q);
            }
        }
    }

    /// 고정한 것·즐겨찾기·최근 위치를 연다: 폴더는 이 탭(new_tab이면 새 탭)에서, 파일은 macOS '열기'로.
    /// 폴더 읽기는 뒷줄에서 하므로 꺼진 네트워크 폴더여도 멈추지 않는다 (열지 못하면 그 칸에 알리고 최근 위치에서 뺀다)
    fn open_place(&mut self, p: usize, path: String, is_dir: bool, new_tab: bool) {
        if !is_dir {
            self.open_files.push(path);
        } else if new_tab {
            self.open_tab(p, Location::Dir(path), None);
        } else {
            self.navigate(p, Location::Dir(path), None);
        }
    }

    /// 명령 메뉴를 그린다: 창 전체를 옅게 가리고 그 위에 판. 창에서는 입력 글자를 글 상자가 그리고, 그림 시험에서만 직접 그린다
    pub(super) fn paint_cmd(&self, g: &mut Gfx, t: &Theme) {
        let (Some(c), Some(r)) = (self.cmd.as_ref(), self.cmd_rect()) else { return };
        paint_cmd_scrim(g, t, self.w, self.h);
        paint_commands(g, &mut MacCmdArt { dim: t.dim }, t, &CmdScene { menu: c, rect: r, multi: self.layout.count() > 1 });
        if self.standalone {
            let field = cmd_input_rect(r);
            let (text, color) = if self.cmd_text.is_empty() { (CMD_PLACEHOLDER, t.faint) } else { (self.cmd_text.as_str(), t.text) };
            g.text(text, field, Font::Normal, color);
        }
    }

    /// 창 시험·시험이 보는 명령 메뉴: (입력 글, 줄의 이름, 고른 줄). 닫혀 있으면 None
    pub fn cmd_seen(&self) -> Option<(String, Vec<String>, usize)> {
        let c = self.cmd.as_ref()?;
        let multi = self.layout.count() > 1;
        Some((self.cmd_text.clone(), c.items.iter().map(|it| cmd_row_text(it, multi).0).collect(), c.sel))
    }
}

/// 입력 칸이 비었을 때 보이는 안내 (창 쪽 글 상자에도 같은 글을 둔다)
pub const CMD_PLACEHOLDER: &str = "폴더·탭·명령 이름";

/// 누른 수정 키에 따라 열 곳: ⌘는 새 탭, ⌥는 옆 칸, 아니면 지금 칸
fn open_how(m: Mods) -> CmdOpen {
    if m.cmd {
        CmdOpen::NewTab
    } else if m.alt {
        CmdOpen::Other
    } else {
        CmdOpen::Here
    }
}
