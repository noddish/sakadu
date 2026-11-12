//! 작업 공간: 왼쪽 레일의 공간 아이콘으로 바꾸고, 새로 만들고, 이름과 색을 바꾸고, 지운다.
//! 바꿀 때 지금 작업 공간은 그 순간의 화면으로 저장해 두고(save_session도 늘 지금 작업 공간을 함께 저장한다),
//! 고른 작업 공간의 칸 나누기·칸·탭, 사이드 패널, 미리 보기와 고정한 파일, 검색어를 되살린다. 강조 색도 그 공간의 색으로 바뀐다.
//! 레일의 자리와 그리기는 sidebar.rs, 고정한 것의 동작은 fav_ui.rs, 저장 형식은 workspaces.rs

use super::*;
use crate::workspaces::{self, Workspace};

impl App {
    /// 시작할 때: 세션이 다른 작업 공간(또는 작업 공간 없는 예전 빌드)의 화면이면 지금 작업 공간의 화면으로 연다.
    /// 그러지 않으면 그 화면이 지금 작업 공간에 덮어써진다
    pub(super) fn restore_workspace_if_needed(&mut self) {
        if !std::mem::take(&mut self.ws_restore) {
            return;
        }
        if let Some(state) = self.workspaces.current.and_then(|c| self.workspaces.list.get(c)).map(|w| w.state.clone()) {
            self.apply_screen(&state);
        }
    }

    /// 작업 공간이 하나도 없으면 지금 화면으로 '기본' 공간을 만든다 (레일에는 늘 지금 공간이 있다)
    pub(super) fn ensure_workspace(&mut self) {
        if self.workspaces.list.is_empty() {
            let state = self.screen_state();
            self.workspaces.ensure_one(|| state);
            self.save_workspaces();
        }
    }

    /// 지금 쓸 강조 색의 키: 지금 작업 공간의 색, 정하지 않았으면 설정의 강조 색
    fn accent_key(&self) -> &str {
        self.workspaces.current().map(|w| w.color.as_str()).filter(|c| !c.is_empty()).unwrap_or(&self.look.accent)
    }

    /// 지금 설정과 작업 공간 색에 맞춘 테마 (설정·시스템 색·작업 공간이 바뀐 뒤 다시 정한다)
    pub(super) fn resolve_theme(&self) -> crate::theme::Theme {
        crate::theme::resolve(&self.look.theme, self.accent_key())
    }

    /// 작업 공간이나 그 색이 바뀐 뒤: 강조 색을 그 공간의 색으로
    fn apply_space_color(&mut self) {
        self.theme = self.resolve_theme();
        self.apply_window_theme();
    }

    /// 그 작업 공간의 색 (레일 아이콘에 쓴다). 색을 정하지 않았으면 설정의 강조 색
    pub(super) fn space_color(&self, w: &Workspace) -> Color {
        let key = if w.color.is_empty() { self.look.accent.as_str() } else { w.color.as_str() };
        crate::theme::accent_color(key, self.theme.dark).unwrap_or(self.theme.accent)
    }

    /// 이름 입력 상자의 테두리 자리 (사이드바 패널 머리의 이름 줄)
    pub(super) fn workspace_rename_rect(&self) -> R {
        self.side_title_rect().map_or(R::new(0.0, 0.0, 0.0, 0.0), |r| R::new(r.l - 4.0, r.t + 3.0, r.r, r.b - 3.0))
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
            pin: self.preview_pin().map(|(p, _)| p),
            pin_id: self.preview_pin().and_then(|(_, id)| id),
            ws: self.workspaces.current_name().map(str::to_string),
            query: edit_text(self.search_edit),
            scope_here: self.pal_here,
            // 접는 구역이 있던 바로가기 패널은 없어졌다 (예전 빌드가 읽는 값이라 자리만 남긴다)
            folded: Vec::new(),
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

    /// 지금 작업 공간에 지금 화면을 적어 두고 파일로 저장한다. 저장하지 못했으면 그 오류
    pub(super) fn try_save_workspaces(&mut self) -> std::io::Result<()> {
        if let Some(c) = self.workspaces.current {
            let state = self.screen_state();
            if let Some(w) = self.workspaces.list.get_mut(c) {
                w.state = state;
            }
        }
        workspaces::save(&self.workspaces)
    }

    /// 지금 작업 공간에 지금 화면을 적어 두고 파일로 저장한다 (저장하지 못하면 기록만 남긴다)
    pub(super) fn save_workspaces(&mut self) {
        if let Err(e) = self.try_save_workspaces() {
            crate::diag::log(|| format!("workspaces: save failed: {e}"));
        }
    }

    /// 지금 화면을 새 작업 공간으로 저장하고, 패널 머리에서 이름을 바로 고칠 수 있게 한다.
    /// 색은 다른 공간이 쓰지 않는 것을 주고, 고정한 것은 비운 채로 시작한다
    pub(super) fn new_workspace(&mut self) {
        // 지금 작업 공간은 지금 화면으로 저장해 둔다 (새 작업 공간도 같은 화면에서 시작한다)
        self.save_workspaces();
        let title = self.panes[self.active].tab().loc.title();
        let base = match self.panes[self.active].tab().loc {
            Location::Dir(_) => title,
            _ => format!("작업 공간 {}", self.workspaces.list.len() + 1),
        };
        let name = self.workspaces.unique_name(&workspaces::clean_name(&base), None);
        let mut ws = Workspace::new(name, self.screen_state());
        ws.color = self.workspaces.next_color(&self.look.accent).to_string();
        self.workspaces.list.push(ws);
        self.workspaces.current = Some(self.workspaces.list.len() - 1);
        self.save_workspaces();
        self.apply_space_color();
        self.begin_workspace_rename();
    }

    pub(super) fn begin_workspace_rename(&mut self) {
        let Some(name) = self.workspaces.current_name().map(str::to_string) else { return };
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        // 이름은 패널 머리에 있다: 패널이 닫혀 있거나 떠 있거나 검색 패널이면 작업 공간 패널을 고정해 연다
        if self.side_panel() != Some(Panel::Main) || self.side_floating() {
            self.show_side(Panel::Main, true);
            // 입력 상자는 곧바로 제자리에 놓이니 패널은 펼치지 않고 바로 연다
            self.side_anim = None;
        }
        self.edit_mode = EditMode::WorkspaceRename;
        // 자리에 먼저 놓고 글자를 넣는다 (좁은 상자에서 통째로 고르면 앞부분이 밀린 채 남는다)
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
        self.apply_space_color();
        let state = self.workspaces.list[i].state.clone();
        self.apply_screen(&state);
        self.save_workspaces();
        self.save_session();
    }

    /// 작업 공간 i를 지운다 (마지막 하나는 지우지 않는다). 지금 공간을 지웠으면 그 자리의 다음 공간으로 바꾼다
    pub(super) fn delete_workspace_at(&mut self, i: usize) {
        if self.edit_mode == EditMode::WorkspaceRename {
            self.cancel_edit();
        }
        // 다른 공간을 지울 때는 지금 화면을 지금 공간에 먼저 적어 둔다
        if self.workspaces.current != Some(i) {
            self.save_workspaces();
        }
        if self.workspaces.remove(i) {
            self.apply_space_color();
            if let Some(state) = self.workspaces.current().map(|w| w.state.clone()) {
                self.apply_screen(&state);
            }
        }
        self.side_clamp_scroll();
        self.save_workspaces();
        self.save_session();
        self.invalidate();
    }

    /// 작업 공간 i의 색을 바꾼다 (빈 글자면 설정의 강조 색을 따른다)
    pub(super) fn set_workspace_color(&mut self, i: usize, color: &str) {
        let Some(w) = self.workspaces.list.get_mut(i) else { return };
        w.color = color.to_string();
        if self.workspaces.current == Some(i) {
            self.apply_space_color();
        }
        self.save_workspaces();
        self.invalidate();
    }

    /// 저장해 둔 화면으로 바꾼다: 칸 나누기·칸·탭, 사이드 패널(너비 포함), 미리 보기와 고정한 파일, 검색어·범위.
    /// 창 위치·숨김 항목·화면 설정·최근 위치는 작업 공간과 상관없어 그대로 둔다
    fn apply_screen(&mut self, s: &session::Session) {
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        // 닫은 탭은 그 작업 공간의 것이라 다른 화면에 다시 열지 않는다
        self.closed_tabs.clear();
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
        self.preview_restore_pin(s.pin.as_deref(), s.pin_id);
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

/// '이 작업 공간을 지울까요?' 확인. 지우면 어디로 바뀌는지도 알린다 (지금 공간을 지울 때)
fn confirm_delete_workspace(hwnd: HWND, index: usize) -> bool {
    let Some((name, next)) = with_app(|a| {
        let list = &a.workspaces.list;
        let name = list.get(index)?.name.clone();
        let next = (a.workspaces.current == Some(index) && list.len() > 1).then(|| list[if index + 1 < list.len() { index + 1 } else { index - 1 }].name.clone());
        Some((name, next))
    })
    .flatten() else {
        return false;
    };
    let after = next.map_or(String::new(), |n| format!("\n지우면 ‘{n}’ 작업 공간으로 바뀌어요."));
    let text = format!("‘{name}’ 작업 공간을 지울까요?\n이 공간에 고정한 것도 함께 지워져요.{after}");
    unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Sakadu"), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

/// 레일의 작업 공간 아이콘 메뉴: (다른 공간이면) 이 공간으로 바꾸기, (지금 공간이면) 이름 바꾸기, 색, 지우기
pub(super) fn show_rail_menu(hwnd: HWND, index: usize, pt: POINT) {
    const SWITCH: u32 = 1;
    const RENAME: u32 = 2;
    const DELETE: u32 = 3;
    const COLOR_NONE: u32 = 10;
    const COLOR: u32 = 11;
    let Some((color, is_current, count)) = with_app(|a| a.workspaces.list.get(index).map(|w| (w.color.clone(), a.workspaces.current == Some(index), a.workspaces.list.len()))).flatten() else {
        return;
    };
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let Ok(sub) = CreatePopupMenu() else {
            let _ = DestroyMenu(menu);
            return;
        };
        let check = |on: bool| MF_STRING | if on { MF_CHECKED } else { MF_UNCHECKED };
        for (k, key) in workspaces::COLORS.iter().enumerate() {
            let label = crate::theme::ACCENTS.iter().find(|a| a.0 == *key).map_or(*key, |a| a.1);
            let _ = AppendMenuW(sub, check(color == *key), COLOR as usize + k, &HSTRING::from(label));
        }
        let _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(sub, check(color.is_empty()), COLOR_NONE as usize, w!("설정의 강조 색 따르기"));
        fav_ui::append_items(menu, &[if is_current { (RENAME, "이름 바꾸기") } else { (SWITCH, "이 공간으로 바꾸기") }]);
        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("색"));
        if count > 1 {
            fav_ui::append_items(menu, &[(0, ""), (DELETE, "이 작업 공간 지우기…")]);
        }
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;
        let _ = DestroyMenu(menu);
        cmd
    };
    match cmd {
        SWITCH => {
            with_app(|a| a.switch_workspace(index));
        }
        RENAME => {
            with_app(|a| a.begin_workspace_rename());
        }
        DELETE => {
            if confirm_delete_workspace(hwnd, index) {
                with_app(|a| a.delete_workspace_at(index));
            }
        }
        COLOR_NONE => {
            with_app(|a| a.set_workspace_color(index, ""));
        }
        c if c >= COLOR && ((c - COLOR) as usize) < workspaces::COLORS.len() => {
            with_app(|a| a.set_workspace_color(index, workspaces::COLORS[(c - COLOR) as usize]));
        }
        _ => {}
    }
}

/// 작업 공간 목록 메뉴 (레일의 + 를 우클릭): 모든 작업 공간(지금 것에 체크, 이름 옆에 한 줄 요약), 새로 만들기, 이름 바꾸기, 지우기.
/// 창이 낮아 레일에 다 보이지 않는 공간도 여기서 고른다
pub(super) fn show_workspace_menu(hwnd: HWND, pt: POINT) {
    let Some((items, count, current)) = with_app(|a| {
        let list = &a.workspaces.list;
        // 메뉴 글자의 &는 단축 글자 표시가 되니 &&로. 지금 작업 공간의 요약은 저장된 것이 아니라 지금 화면으로
        let menu_text = |s: &str| s.replace('&', "&&");
        let mut items: Vec<(String, bool)> = list
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let current = a.workspaces.current == Some(i);
                let summary = if current { workspaces::summary(&a.screen_state()) } else { workspaces::summary(&w.state) };
                (format!("{}\t{}", menu_text(&w.name), menu_text(&summary)), current)
            })
            .collect();
        if !items.is_empty() {
            items.push((String::new(), false));
        }
        // 지금 작업 공간은 화면을 따라가니 "다른 이름으로 저장"이 아니라 지금 화면에서 새로 시작하는 것
        items.push(("새 작업 공간 만들기 (지금 화면으로)".to_string(), false));
        items.push(("이름 바꾸기".to_string(), false));
        // 마지막 하나는 지우지 않는다
        if list.len() > 1 {
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
            if confirm_delete_workspace(hwnd, c) {
                with_app(|a| a.delete_workspace_at(c));
            }
        }
        _ => {}
    }
}
