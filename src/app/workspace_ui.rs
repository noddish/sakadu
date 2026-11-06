//! 작업 줄 왼쪽의 작업 공간 단추와 메뉴: 지금 화면을 이름 붙여 저장하고, 저장한 화면으로 바꾼다.
//! 바꿀 때 지금 작업 공간은 그 순간의 화면으로 저장해 두고(save_session도 늘 지금 작업 공간을 함께 저장한다),
//! 고른 작업 공간의 칸 나누기·칸·탭, 사이드 패널, 미리 보기와 고정한 파일, 검색어를 되살린다.
//! 저장 형식은 workspaces.rs

use super::*;
use crate::workspaces::{self, Workspace};

impl App {
    /// 작업 줄 맨 왼쪽(칸 영역이 시작하는 곳)의 작업 공간 단추. 칸 나누기 판은 그 오른쪽에 놓인다
    pub(super) fn workspace_button(&self) -> R {
        // 이름을 고치는 동안은 넉넉하게 (긴 이름의 앞부분이 입력 상자 밖으로 밀리지 않게)
        let w = if self.edit_mode == EditMode::WorkspaceRename {
            220.0
        } else {
            (self.gfx.measure(&self.workspace_label(), Font::Strong) + 40.0).clamp(96.0, 220.0)
        };
        let l = self.panes_left();
        R::new(l, 7.0, l + w, 37.0)
    }

    fn workspace_label(&self) -> String {
        self.workspaces.current_name().unwrap_or("작업 공간").to_string()
    }

    pub(super) fn paint_workspace_button(&self) {
        let t = self.theme;
        let g = &self.gfx;
        let r = self.workspace_button();
        if self.edit_mode == EditMode::WorkspaceRename {
            let er = r.inset(2.0, 3.0);
            g.fill_round(er, R_ROW, t.pane);
            g.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
            return;
        }
        if self.hover == Hit::Workspace {
            g.fill_round(r, 6.0, t.sel_inactive);
        }
        // 작업 공간이 없으면 이름 대신 흐린 "작업 공간"
        let named = self.workspaces.current.is_some();
        g.text(&self.workspace_label(), R::new(r.l + 10.0, r.t, r.r - 24.0, r.b), Font::Strong, if named { t.text } else { t.dim });
        g.glyph(G_CHEVRON_DOWN, R::new(r.r - 22.0, r.t + 1.0, r.r - 8.0, r.b + 1.0), true, t.faint);
    }

    /// 이름 입력 상자 자리 (단추 안)
    pub(super) fn workspace_rename_rect(&self) -> R {
        self.workspace_button().inset(8.0, 0.0)
    }

    /// 지금 화면 (창 위치·화면 설정까지. 작업 공간으로 쓸 때는 바꿀 때 그 부분을 쓰지 않는다)
    pub(super) fn screen_state(&self) -> session::Session {
        let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        unsafe {
            let _ = GetWindowPlacement(self.hwnd, &mut wp);
        }
        let r = wp.rcNormalPosition;
        session::Session {
            window: Some([r.left, r.top, r.right - r.left, r.bottom - r.top]),
            maximized: wp.showCmd == SW_SHOWMAXIMIZED.0 as u32,
            layout: self.layout.index() as u8,
            split: self.split,
            active: self.active,
            show_hidden: self.show_hidden,
            preview: self.preview.is_some(),
            new_tab: self.new_tab_home.clone(),
            recent: self.recent.clone(),
            side: self.side_pinned.map_or("none", Panel::key).to_string(),
            side_w: self.side_w,
            pin: self.preview_pin_path(),
            query: edit_text(self.search_edit),
            scope_here: self.pal_here,
            folded: Fold::ALL.into_iter().filter(|f| self.folded[*f as usize]).map(|f| f.key().to_string()).collect(),
            look: self.look.clone(),
            panes: self
                .panes
                .iter()
                .map(|p| session::PaneState {
                    tabs: p.tabs.iter().map(|t| t.loc.to_session()).collect(),
                    active: p.active,
                    cols: p.cols,
                    sort_key: p.sort.key.to_u8(),
                    sort_desc: p.sort.desc,
                    view: p.view.code(),
                })
                .collect(),
        }
    }

    /// 지금 작업 공간에 지금 화면을 적어 두고 파일로 저장한다
    pub(super) fn save_workspaces(&mut self) {
        if let Some(c) = self.workspaces.current {
            let state = self.screen_state();
            if let Some(w) = self.workspaces.list.get_mut(c) {
                w.state = state;
            }
        }
        if let Err(e) = workspaces::save(&self.workspaces) {
            crate::diag::log(|| format!("workspaces: save failed: {e}"));
        }
    }

    /// 지금 화면을 새 작업 공간으로 저장하고, 단추에서 이름을 바로 고칠 수 있게 한다
    pub(super) fn new_workspace(&mut self) {
        // 지금 작업 공간은 지금 화면으로 저장해 둔다 (새 작업 공간도 같은 화면에서 시작한다)
        self.save_workspaces();
        let title = self.panes[self.active].tab().loc.title();
        let base = match self.panes[self.active].tab().loc {
            Location::Dir(_) => title,
            _ => format!("작업 공간 {}", self.workspaces.list.len() + 1),
        };
        let name = self.workspaces.unique_name(&workspaces::clean_name(&base), None);
        self.workspaces.list.push(Workspace { name, state: self.screen_state() });
        self.workspaces.current = Some(self.workspaces.list.len() - 1);
        self.save_workspaces();
        self.begin_workspace_rename();
    }

    pub(super) fn begin_workspace_rename(&mut self) {
        let Some(name) = self.workspaces.current_name().map(str::to_string) else { return };
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        self.edit_mode = EditMode::WorkspaceRename;
        // 넓힌 자리에 먼저 놓고 글자를 넣는다 (좁은 상자에서 통째로 고르면 앞부분이 밀린 채 남는다)
        self.layout_edits();
        unsafe {
            let _ = SetWindowTextW(self.rename_edit, &HSTRING::from(name));
            SendMessageW(self.rename_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
        self.focus();
        self.invalidate();
    }

    pub(super) fn commit_workspace_rename(&mut self) {
        let text = workspaces::clean_name(&edit_text(self.rename_edit));
        self.close_edit();
        let Some(c) = self.workspaces.current else { return };
        if text.is_empty() || self.workspaces.list[c].name == text {
            return;
        }
        let name = self.workspaces.unique_name(&text, Some(c));
        self.workspaces.list[c].name = name;
        self.save_workspaces();
    }

    /// 저장한 작업 공간 i로 바꾼다. 지금 작업 공간은 지금 화면으로 저장해 둔다
    pub(super) fn switch_workspace(&mut self, i: usize) {
        if self.workspaces.current == Some(i) || i >= self.workspaces.list.len() {
            return;
        }
        self.save_workspaces();
        self.workspaces.current = Some(i);
        let state = self.workspaces.list[i].state.clone();
        self.apply_screen(&state);
        self.save_workspaces();
        self.save_session();
    }

    /// 지금 작업 공간을 지운다. 다른 작업 공간이 있으면 그리로 바꾸고, 없으면 지금 화면은 그대로 둔다
    pub(super) fn delete_workspace(&mut self) {
        let Some(c) = self.workspaces.current else { return };
        if self.edit_mode == EditMode::WorkspaceRename {
            self.cancel_edit();
        }
        self.workspaces.list.remove(c);
        self.workspaces.current = None;
        if self.workspaces.list.is_empty() {
            self.save_workspaces();
            self.invalidate();
            return;
        }
        let next = c.min(self.workspaces.list.len() - 1);
        self.workspaces.current = Some(next);
        let state = self.workspaces.list[next].state.clone();
        self.apply_screen(&state);
        self.save_workspaces();
        self.save_session();
    }

    /// 저장해 둔 화면으로 바꾼다: 칸 나누기·칸·탭, 사이드 패널(너비 포함), 미리 보기와 고정한 파일, 검색어·범위.
    /// 창 위치·숨김 항목·화면 설정·최근 위치는 작업 공간과 상관없어 그대로 둔다
    fn apply_screen(&mut self, s: &session::Session) {
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        self.close_view_menu();
        self.close_settings();
        self.fav_blur();
        self.close_peek();
        self.drag = Drag::None;
        self.start_keys = false;
        // 칸·탭 (App::new와 같은 규칙: 저장된 칸이 없으면 기본 위치)
        for (i, default) in default_locations().into_iter().enumerate() {
            let ps = s.panes.get(i);
            let locs: Vec<Location> = ps.map(|p| p.tabs.iter().map(|t| Location::from_session(t)).collect()).unwrap_or_else(|| vec![default]);
            let mut tabs: Vec<Tab> = locs.into_iter().map(|loc| Tab::new(self.next_tab_id(), loc)).collect();
            if tabs.is_empty() {
                tabs.push(Tab::new(self.next_tab_id(), Location::Start));
            }
            let pane = &mut self.panes[i];
            pane.active = ps.map(|p| p.active.min(tabs.len() - 1)).unwrap_or(0);
            pane.tabs = tabs;
            if let Some(p) = ps {
                pane.sort = Sort { key: SortKey::from_u8(p.sort_key), desc: p.sort_desc };
                pane.cols = p.cols.map(|c| c.clamp(60.0, 400.0));
                pane.view = ViewMode::from_code(p.view);
            }
        }
        self.layout = Layout::ALL[(s.layout as usize).min(4)];
        self.split = (s.split.0.clamp(0.15, 0.85), s.split.1.clamp(0.15, 0.85));
        self.active = s.active.min(self.layout.count() - 1);
        self.stale = [false; 4];
        // 사이드 패널: 검색 패널이면 검색어와 범위까지
        self.side_w = s.side_w.clamp(session::SIDE_W_RANGE.0, session::SIDE_W_RANGE.1);
        match Panel::from_key(&s.side) {
            Some(Panel::Search) => self.restore_search(&s.query, s.scope_here),
            other => {
                match other {
                    Some(p) => self.show_side(p, true),
                    None => self.close_side(),
                }
                self.set_search_text(&s.query);
                self.pal_here = s.scope_here;
            }
        }
        // 미리 보기: 켜고 끄기, 고정한 파일
        if s.preview != self.preview.is_some() {
            self.toggle_preview();
        }
        self.preview_restore_pin(s.pin.as_deref());
        for p in 0..self.layout.count() {
            self.load(p, false);
        }
        self.sync_watches();
        let filter = self.panes[self.active].tab().filter.clone();
        self.set_filter_edit_text(&filter);
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.layout_edits();
        self.update_title();
        self.refresh_hover();
        self.focus();
        self.invalidate();
    }
}

/// 작업 공간 메뉴: 저장한 작업 공간(지금 것에 체크, 이름 옆에 한 줄 요약), 새로 저장, 이름 바꾸기, 지우기
pub(super) fn show_workspace_menu(hwnd: HWND, pt: POINT) {
    let Some((items, count, current)) = with_app(|a| {
        let list = &a.workspaces.list;
        let mut items: Vec<(String, bool)> =
            list.iter().enumerate().map(|(i, w)| (format!("{}\t{}", w.name, workspaces::summary(&w.state)), a.workspaces.current == Some(i))).collect();
        if !items.is_empty() {
            items.push((String::new(), false));
        }
        items.push(("지금 화면을 새 작업 공간으로 저장".to_string(), false));
        if a.workspaces.current.is_some() {
            items.push(("이름 바꾸기".to_string(), false));
            items.push(("이 작업 공간 지우기…".to_string(), false));
        }
        (items, list.len(), a.workspaces.current)
    }) else {
        return;
    };
    let cmd = settings_ui::pick_menu(hwnd, pt, &items);
    // 메뉴 순서: 작업 공간들, (구분선), 새로 저장, 이름 바꾸기, 지우기
    let first_action = if count > 0 { count + 2 } else { 1 };
    match cmd {
        0 => {}
        n if n <= count => {
            with_app(|a| a.switch_workspace(n - 1));
        }
        n if n == first_action => {
            with_app(|a| a.new_workspace());
        }
        n if n == first_action + 1 => {
            with_app(|a| a.begin_workspace_rename());
        }
        n if n == first_action + 2 => {
            let Some(c) = current else { return };
            let Some((name, next)) = with_app(|a| {
                let list = &a.workspaces.list;
                let next = (list.len() > 1).then(|| list[if c + 1 < list.len() { c + 1 } else { c - 1 }].name.clone());
                (list[c].name.clone(), next)
            }) else {
                return;
            };
            let after = match next {
                Some(n) => format!("지우면 ‘{n}’ 작업 공간으로 바뀌어요."),
                None => "지금 화면은 그대로 남아요.".to_string(),
            };
            let text = format!("‘{name}’ 작업 공간을 지울까요?\n{after}");
            if unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Sakadu"), MB_YESNO | MB_ICONQUESTION) } == IDYES {
                with_app(|a| a.delete_workspace());
            }
        }
        _ => {}
    }
}
