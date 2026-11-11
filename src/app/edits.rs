//! 입력 상자 (필터·이름 바꾸기·경로 입력 등)

use super::*;

impl App {
    // ---------- 입력 상자 ----------

    pub(super) fn apply_edit_font(&mut self) {
        let font = make_edit_font(self.gfx.dpi, &self.look.font);
        for h in [self.filter_edit, self.rename_edit, self.addr_edit, self.search_edit, self.cmd_edit] {
            unsafe {
                SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
                // EC_LEFTMARGIN | EC_RIGHTMARGIN, 좌우 2px
                SendMessageW(h, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(2 | (2 << 16))));
            }
        }
        if !self.edit_font.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.edit_font.into());
            }
        }
        self.edit_font = font;
    }

    pub(super) fn rename_rect(&self, p: usize) -> Option<R> {
        let (list, grid) = self.grid(p)?;
        let pane = &self.panes[p];
        let tab = pane.tab();
        let it = grid.item(tab.cursor, tab.scroll);
        if it.t < list.t - 1.0 || it.b > list.b + 1.0 || it.l < list.l - 1.0 || it.r > list.r + 1.0 {
            return None;
        }
        match grid.cell {
            CellKind::Tile(thumb) => {
                // 그림 아래 이름 자리
                let y = it.t + 8.0 + thumb + 6.0;
                return Some(R::new(it.l + 4.0, y, it.r - 8.0, y + 24.0));
            }
            CellKind::Icon => return Some(R::new(it.l + 24.0, it.t + 2.0, it.r - 6.0, it.b - 2.0)),
            CellKind::Row => {}
        }
        let cols = columns(list, pane.cols);
        Some(R::new(cols.name.0 + 24.0, it.t + 2.0, cols.name.1 - 4.0, it.b - 2.0))
    }

    /// 실제 Win32 입력 상자(한글 IME 지원)를 그려 둔 자리 위에 맞춘다.
    /// 이름을 바꾸는 줄(파일·즐겨찾기)이 보이는 곳 밖으로 밀려났으면(휠·스크롤 막대·창 크기·칸 나누기 등) 입력을 확정한다:
    /// 입력 상자를 숨기기만 하면 키보드가 메인 창으로 가, 이어서 누른 Enter·Delete가 고른 파일에 먹는다
    pub(super) fn layout_edits(&mut self) {
        let lost = match self.edit_mode {
            EditMode::Rename(p) => self.rename_rect(p).is_none(),
            EditMode::FavRename(row) => self.fav_rename_rect(row).is_none(),
            _ => false,
        };
        if lost {
            self.commit_rename();
        }
        let s = self.scale();
        let place = |h: HWND, r: Option<R>| unsafe {
            match r {
                Some(r) => {
                    let _ = SetWindowPos(
                        h,
                        None,
                        (r.l * s).round() as i32,
                        (r.t * s).round() as i32,
                        (r.w() * s).round() as i32,
                        (r.h() * s).round() as i32,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
                None => {
                    let _ = ShowWindow(h, SW_HIDE);
                }
            }
        };
        let edit_h = 20.0;
        let center = |b: R, lp: f32, rp: f32| R::new(b.l + lp, b.t + (b.h() - edit_h) / 2.0, b.r - rp, b.t + (b.h() + edit_h) / 2.0);
        self.place_filter_edit(true);
        // 다른 패널이 잠깐 가리는 동안 검색 상자는 숨기지 않고 화면 밖에 둔다 (숨기면 키보드 포커스가 메인 창으로 가 버린다)
        let search_at = if self.pal_visible() {
            Some(center(self.pal_input_rect(), 4.0, 4.0))
        } else if self.pal_shown() {
            Some(R::new(-4000.0, -4000.0, -3800.0, -3980.0))
        } else {
            None
        };
        place(self.search_edit, search_at);
        place(self.cmd_edit, self.cmd_input_rect().map(|r| center(r, 0.0, 0.0)));
        let addr = match self.edit_mode {
            EditMode::Address(p) => self.pane_rect(p).map(|r| center(addr_rect(&geom(r)), 8.0, 8.0)),
            _ => None,
        };
        place(self.addr_edit, addr);
        let ren = match self.edit_mode {
            EditMode::Rename(p) => self.rename_rect(p).map(|r| center(r, 4.0, 4.0)),
            EditMode::FavRename(row) => self.fav_rename_rect(row).map(|r| center(r, 4.0, 4.0)),
            EditMode::WorkspaceRename => Some(center(self.workspace_rename_rect(), 0.0, 0.0)),
            _ => None,
        };
        place(self.rename_edit, ren);
    }

    /// 필터 상자 자리 (DIP). 활성 칸의 필터 칩 안에 둔다. 필터가 비어 있거나 팝업이 열려 있으면 화면 밖에 두되
    /// 포커스는 그대로 둬, 목록에서 바로 글자를 치면 걸러진다 (첫 글자가 들어오면 칩이 생겨 그 안으로 옮긴다)
    pub(super) fn filter_edit_rect(&self) -> R {
        let popup = self.view_menu.is_some() || self.settings.is_some();
        match self.pane_rect(self.active) {
            Some(r) if !popup && !self.panes[self.active].tab().filter.is_empty() => {
                let b = self.filter_parts(self.active, filter_chip(&geom(r))).0;
                // 떠 있는 패널이 그 자리를 덮으면 입력 상자(자식 창은 늘 그림 위에 보인다)도 숨긴다
                if self.side_floating() && self.side_panel_rect().is_some_and(|p| p.l < b.r && p.r > b.l && p.t < b.b && p.b > b.t) {
                    return R::new(-4000.0, -4000.0, -3800.0, -3980.0);
                }
                // 입력 상자 글자는 위쪽에 붙어 그려지므로 칩 안 다른 글자(개수)와 줄이 맞게 2px 내린다
                let t = b.t + (b.h() - 20.0) / 2.0 + 2.0;
                R::new(b.l, t, b.r, t + 20.0)
            }
            _ => R::new(-4000.0, -4000.0, -3800.0, -3980.0),
        }
    }

    /// 필터 상자를 제자리로. force가 아니면 자리가 바뀌었을 때만 옮긴다.
    /// 칩 안 개수 글자(고른 개수 포함)는 고르거나 목록이 바뀔 때마다 폭이 달라지므로 그릴 때마다 맞춘다
    pub(super) fn place_filter_edit(&self, force: bool) {
        let s = self.scale();
        let r = self.filter_edit_rect();
        let px = ((r.l * s).round() as i32, (r.t * s).round() as i32, (r.w() * s).round() as i32, (r.h() * s).round() as i32);
        if !force && self.filter_placed.get() == px {
            return;
        }
        self.filter_placed.set(px);
        unsafe {
            let _ = SetWindowPos(self.filter_edit, None, px.0, px.1, px.2, px.3, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    }

    pub(super) fn set_filter_edit_text(&self, s: &str) {
        if edit_text(self.filter_edit) != s {
            let n = s.encode_utf16().count();
            unsafe {
                let _ = SetWindowTextW(self.filter_edit, &HSTRING::from(s));
                // 캐럿은 친 직후처럼 끝에 둔다 (칸·탭을 오간 뒤에도 Delete가 같은 뜻이 되게)
                SendMessageW(self.filter_edit, EM_SETSEL, Some(WPARAM(n)), Some(LPARAM(n as isize)));
            }
        }
    }

    pub(super) fn sync_filter_from_edit(&mut self) {
        let text = edit_text(self.filter_edit);
        let p = self.active;
        let (sort, hidden) = (self.panes[p].sort, self.show_hidden);
        let tab = self.panes[p].tab_mut();
        if tab.filter == text {
            // 목록 쪽에서 필터를 이미 비웠어도(다른 위치로 이동 등) 입력 상자 자리는 맞춘다
            self.layout_edits();
            return;
        }
        tab.filter = text;
        tab.rebuild_view(sort, hidden);
        // 입력하는 동안 첫 일치 항목을 선택해 Enter로 바로 열 수 있게 한다
        if !tab.filter.is_empty() && !tab.view.is_empty() {
            tab.select_only(0);
        }
        tab.scroll = 0.0;
        // 시작 화면도 글자를 치면 첫 일치 항목에 초점을 두어 Enter로 바로 연다
        if tab.loc == Location::Start {
            tab.cursor = 0;
            self.start_keys = !tab.filter.is_empty();
        }
        self.ensure_visible(p);
        // 필터 칩이 생기거나 없어지고, 개수 글자 폭이 바뀌니 입력 상자 자리를 다시 잡는다
        self.layout_edits();
        self.invalidate();
    }

    pub(super) fn clear_filter(&mut self) {
        self.set_filter_edit_text("");
        self.sync_filter_from_edit();
    }

    pub fn begin_rename(&mut self, p: usize) {
        // 열려 있는 다른 입력을 먼저 끝낸다 (이름은 확정, 경로는 취소). 칸을 바꾸기 전에 해야 앞 이름이 새 항목에 붙지 않는다
        self.end_edits_for_click();
        self.activate_pane(p);
        // 이 칸에서 기다리던 경로 확인은 버린다 (도착해서 이름 상자를 닫지 않게)
        self.bump_probe_seq(p);
        let tab = self.panes[p].tab();
        if tab.loc == Location::Computer {
            return;
        }
        let Some(e) = tab.cursor_entry() else { return };
        // 고른 것 없이 커서만 남아 있으면 탐색기처럼 아무것도 하지 않는다
        if !e.selected {
            return;
        }
        let Some(path) = tab.path_of(e) else { return };
        let name = e.name_string();
        let dir = e.is_dir();
        // 파일은 확장자를 뺀 부분만 선택 (탐색기와 같음)
        let stem = match name.rfind('.') {
            Some(i) if i > 0 && e.kind == Kind::File => name[..i].encode_utf16().count(),
            _ => name.encode_utf16().count(),
        };
        self.rename = Some(RenameCtx { pane: p, tab: tab.id, path, old: name.clone(), dir });
        self.edit_mode = EditMode::Rename(p);
        self.ensure_visible(p);
        unsafe {
            let _ = SetWindowTextW(self.rename_edit, &HSTRING::from(name));
            SendMessageW(self.rename_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(stem as isize)));
        }
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    pub(super) fn commit_rename(&mut self) {
        self.commit_rename_with(false);
    }

    /// 이름 바꾸기 확정. enter: Enter로 확정했으면 쓸 수 없는 이름일 때 상자를 친 그대로 열어 둔 채 알린다
    pub(super) fn commit_rename_with(&mut self, enter: bool) {
        if let EditMode::FavRename(row) = self.edit_mode {
            self.commit_fav_rename(row);
            return;
        }
        if self.edit_mode == EditMode::WorkspaceRename {
            self.commit_workspace_rename();
            return;
        }
        let Some(ctx) = self.rename.take() else { return };
        // 끝의 공백·점은 Windows가 떼어 내므로 미리 뗀다 (탐색기와 같음)
        let new_name = edit_text(self.rename_edit).trim().trim_end_matches(['.', ' ']).to_string();
        if new_name.is_empty() || new_name == ctx.old {
            self.close_edit();
            return;
        }
        if let Some(msg) = util::name_problem(&new_name) {
            self.flash(ctx.pane, msg);
            if enter {
                self.rename = Some(ctx);
                return;
            }
            self.close_edit();
            return;
        }
        self.close_edit();
        // 확장자를 바꾸거나 지우면 탐색기처럼 한 번 묻는다 (아니요면 친 이름 그대로 상자를 다시 연다)
        if !ctx.dir && ext_of_name(&ctx.old) != ext_of_name(&new_name) {
            self.pending.push(Deferred::ConfirmRename { tab: ctx.tab, path: ctx.path, old: ctx.old, new: new_name });
            return;
        }
        self.start_rename(ctx.tab, ctx.path, new_name);
    }

    /// 이름 바꾸기 작업을 시작한다. 바뀐 이름을 고르고(스크롤은 그대로), 필터에 걸려 사라질 이름이면 필터를 푼다
    pub(super) fn start_rename(&mut self, tab_id: u64, path: String, new_name: String) {
        // 확인 창을 기다리는 동안 다른 폴더로 옮긴 탭이면 고를 것도 알릴 것도 없다
        if let Some((p, t)) = self.tab_showing(tab_id, &path) {
            let tab = &mut self.panes[p].tabs[t];
            tab.restore = Some(Restore { scroll: tab.scroll, cursor_index: tab.cursor, ..Restore::select(&new_name) });
            // 필터에 걸리지 않는 이름이면 목록에서 숨는다. 필터는 그대로 두고(클릭 도중에 목록을 바꾸지 않게) 알린다
            let filter = tab.filter.clone();
            if !filter.is_empty() && !new_name.to_lowercase().contains(&filter.to_lowercase()) {
                self.flash(p, format!("‘{new_name}’ · 필터 ‘{filter}’에 맞지 않아 목록에서 숨겨져요 · Esc로 필터 풀기"));
            }
        }
        shell::spawn_file_op(self.hwnd, FileOp::Rename { path, name: new_name });
    }

    /// 탭 tab_id가 아직 path가 있는 폴더를 보고 있으면 (칸, 탭)
    pub(super) fn tab_showing(&self, tab_id: u64, path: &str) -> Option<(usize, usize)> {
        let parent = util::parent(path)?;
        self.find_tab(tab_id).filter(|&(p, t)| self.panes[p].tabs[t].loc.dir().is_some_and(|d| util::eq_path(d, &parent)))
    }

    /// 확장자 경고에서 '아니요': 친 이름을 그대로 둔 채 그 항목의 이름 상자를 다시 연다
    pub(super) fn reopen_rename(&mut self, tab_id: u64, path: &str, old: &str, typed: &str) {
        let Some((p, t)) = self.tab_showing(tab_id, path) else { return };
        if self.panes[p].active != t {
            return;
        }
        let key = old.to_lowercase();
        let tab = self.panes[p].tab_mut();
        let Some(vi) = tab.view.iter().position(|&i| tab.entries[i as usize].name_string().to_lowercase() == key) else { return };
        tab.select_only(vi);
        self.begin_rename(p);
        if self.edit_mode == EditMode::Rename(p) {
            let stem = match typed.rfind('.') {
                Some(i) if i > 0 => typed[..i].encode_utf16().count(),
                _ => typed.encode_utf16().count(),
            };
            unsafe {
                let _ = SetWindowTextW(self.rename_edit, &HSTRING::from(typed));
                SendMessageW(self.rename_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(stem as isize)));
            }
        }
    }

    /// 이름 상자에 쓸 수 없는 글자를 치면: 넣지 않고 알린다. 파일·폴더 이름 바꾸기일 때만 (즐겨찾기·작업 공간 이름은 표시용)
    pub fn rename_char_blocked(&mut self) -> bool {
        let EditMode::Rename(p) = self.edit_mode else { return false };
        self.flash(p, "이름에는 \\ / : * ? \" < > | 를 쓸 수 없어요");
        true
    }

    pub(super) fn begin_address(&mut self, p: usize) {
        // 이름 상자가 열려 있으면 친 이름을 확정한 뒤 경로 입력으로 넘어간다
        self.end_edits_for_click();
        self.bump_probe_seq(p);
        self.activate_pane(p);
        let text = self.panes[p].tab().loc.dir().unwrap_or("").to_string();
        self.edit_mode = EditMode::Address(p);
        unsafe {
            let _ = SetWindowTextW(self.addr_edit, &HSTRING::from(text));
            SendMessageW(self.addr_edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    pub(super) fn commit_address(&mut self) {
        let EditMode::Address(p) = self.edit_mode else { return };
        let text = edit_text(self.addr_edit);
        self.close_edit();
        let tab = self.panes[p].tab();
        let then = ProbeThen::Address { tab: tab.id, from: tab.loc.clone() };
        self.probe(p, text, then);
    }

    /// 경로를 뒤 스레드에서 확인하고 나서 연다 (on_probed). 꺼진 NAS를 확인하는 동안에도 창은 그대로 쓸 수 있다
    pub(super) fn probe(&mut self, p: usize, text: String, then: ProbeThen) {
        let seq = self.bump_probe_seq(p);
        self.probe_next += 1;
        let id = self.probe_next;
        self.probes.push((id, p, text.trim().to_string()));
        spawn_probe(self.hwnd, id, seq, p, text, then);
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_PROBE, 400, None);
        }
    }

    /// 칸 p의 요청 번호를 올린다: 그 칸에서 아직 기다리는 경로 확인의 결과는 버려진다
    pub(super) fn bump_probe_seq(&mut self, p: usize) -> u64 {
        let p = p.min(3);
        self.probe_seq[p] += 1;
        self.probe_seq[p]
    }

    /// 경로 확인이 오래 걸리면 기다리는 중임을 알린다. 기다리는 동안 몇 초마다 다시 띄워 알림이 사라지지 않게 한다
    pub(super) fn on_probe_timer(&mut self) {
        let Some((_, p, text)) = self.probes.first().cloned() else {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_PROBE);
            }
            return;
        };
        let p = p.min(self.layout.count().saturating_sub(1));
        let msg = format!("여는 중… {text}");
        self.flash(p, msg.clone());
        self.probe_shown = Some((self.panes[p].tab().id, msg));
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_PROBE, 2500, None);
        }
    }

    /// 경로 확인 중이라고 띄운 알림만 지운다 (다른 알림은 그대로)
    pub(super) fn clear_probe_note(&mut self) {
        let Some((tab, msg)) = self.probe_shown.take() else { return };
        if let Some((p, t)) = self.find_tab(tab) {
            let tab = &mut self.panes[p].tabs[t];
            if tab.message.as_deref() == Some(msg.as_str()) {
                tab.message = None;
                self.invalidate();
            }
        }
    }

    pub(super) fn on_probed(&mut self, pr: Probed) {
        self.probes.retain(|(id, _, _)| *id != pr.id);
        self.clear_probe_note();
        if self.probes.is_empty() {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_PROBE);
            }
        }
        self.refresh_cursor();
        let p = pr.pane.min(self.layout.count().saturating_sub(1));
        // 기다리는 동안 그 칸에서 다른 일을 시작했거나(경로 입력·이름 바꾸기·다른 확인), 탭을 닫거나 옮겼으면 버린다
        let newer = pr.seq != self.probe_seq[pr.pane.min(3)];
        let moved = |a: &App, tab: u64, from: &Location| a.panes[p].tab().id != tab || &a.panes[p].tab().loc != from;
        match pr.then {
            ProbeThen::Address { tab, from } => {
                if newer || moved(self, tab, &from) {
                    return;
                }
                match pr.result {
                    Ok((loc, select)) => self.navigate(p, loc, select),
                    Err(_) if net_text(&pr.shown) => self.navigate(p, Location::Dir(pr.shown.trim().to_string()), None),
                    Err(msg) => self.flash(p, msg),
                }
            }
            ProbeThen::External => match pr.result {
                Ok((loc, select)) => {
                    // 지금 탭이 이미 그 폴더면 그대로 (대소문자는 가리지 않는다)
                    let same = match (&self.panes[p].tab().loc, &loc) {
                        (Location::Dir(a), Location::Dir(b)) => util::eq_path(a, b),
                        (a, b) => a == b,
                    };
                    if select.is_none() && same {
                        return;
                    }
                    self.new_tab(p, Some(loc), true);
                    if let Some(name) = select {
                        self.panes[p].tab_mut().restore = Some(Restore::select(&name));
                    }
                }
                Err(_) => self.flash(p, format!("경로를 찾을 수 없어요: {}", pr.shown)),
            },
            ProbeThen::Favorite { tab, from, new_tab, is_dir } => {
                if !new_tab && (newer || moved(self, tab, &from)) {
                    return;
                }
                match pr.result {
                    Ok((loc, None)) => {
                        if new_tab {
                            self.new_tab(p, Some(loc), true);
                        } else {
                            self.navigate(p, loc, None);
                        }
                    }
                    Ok((Location::Dir(dir), Some(name))) => {
                        if new_tab {
                            self.new_tab(p, Some(Location::Dir(dir)), true);
                            self.panes[p].tab_mut().restore = Some(Restore::select(&name));
                        } else {
                            let path = util::join(&dir, &name);
                            self.pending.push(Deferred::Open { pane: p, paths: vec![path], dir: Some(dir) });
                        }
                    }
                    Ok(_) => {}
                    // 꺼졌거나 로그인이 필요한 공유의 즐겨찾기: 그 폴더(파일이면 담은 폴더)를 열어 단추·기억한 목록을 보인다
                    Err(_) if net_text(&pr.shown) => {
                        let path = pr.shown.trim().to_string();
                        let dir = if is_dir == Some(false) { util::parent(&path).unwrap_or(path.clone()) } else { path.clone() };
                        if new_tab {
                            self.new_tab(p, Some(Location::Dir(dir)), true);
                        } else {
                            self.navigate(p, Location::Dir(dir), None);
                        }
                    }
                    Err(_) => self.flash(p, format!("즐겨찾기 항목을 찾을 수 없어요: {}", pr.shown)),
                }
            }
        }
    }

    pub(super) fn close_edit(&mut self) {
        self.edit_mode = EditMode::None;
        unsafe {
            let _ = ShowWindow(self.rename_edit, SW_HIDE);
            let _ = ShowWindow(self.addr_edit, SW_HIDE);
        }
        self.focus();
        self.invalidate();
    }

    pub(super) fn cancel_edit(&mut self) {
        self.rename = None;
        self.close_edit();
    }

    /// 목록을 클릭하면 이름 바꾸기는 확정, 주소 입력은 취소 (탐색기와 같음)
    pub(super) fn end_edits_for_click(&mut self) {
        match self.edit_mode {
            EditMode::Rename(_) | EditMode::FavRename(_) | EditMode::WorkspaceRename => self.commit_rename(),
            EditMode::Address(_) => self.cancel_edit(),
            EditMode::None => {}
        }
    }

    /// 칸 아래쪽에 잠깐 알림을 띄운다 (몇 초 뒤 사라진다)
    pub(super) fn flash(&mut self, p: usize, msg: impl Into<String>) {
        self.toast_undo = None;
        self.panes[p].tab_mut().message = Some(msg.into());
        self.start_anim(|a| &mut a.toast_at);
        unsafe { SetTimer(Some(self.hwnd), TIMER_FLASH, FLASH_MS, None) };
        self.invalidate();
    }

    /// 페이드를 시작한다 (Windows에서 애니메이션을 끄면 하지 않는다)
    pub(super) fn start_anim(&mut self, at: fn(&mut App) -> &mut Option<std::time::Instant>) {
        *at(self) = sidebar::motion_on().then(std::time::Instant::now);
        if at(self).is_some() {
            unsafe { SetTimer(Some(self.hwnd), TIMER_ANIM, 15, None) };
        }
    }

    pub(super) fn anims_running(&self) -> bool {
        anim_progress(self.tip_at, TIP_FADE_MS).is_some()
            || anim_progress(self.toast_at, TOAST_FADE_MS).is_some()
            || anim_progress(self.view_menu_at, MENU_FADE_MS).is_some()
            || anim_progress(self.cmd_at, commands::CMD_FADE_MS).is_some()
    }

    /// 휠 한 걸음: 목표 자리 쪽으로 남은 거리의 35%씩 (0.2초쯤에 닿는다)
    pub(super) fn on_wheel_timer(&mut self) {
        let Some(w) = self.wheel else {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_WHEEL);
            }
            return;
        };
        let alive = w.pane < self.layout.count() && self.panes[w.pane].tab().id == w.tab && (self.panes[w.pane].tab().scroll - w.last).abs() < 0.5;
        let mut done = !alive;
        if alive {
            let cur = self.panes[w.pane].tab().scroll;
            let next = if (w.target - cur).abs() < 1.0 { w.target } else { cur + (w.target - cur) * 0.35 };
            self.panes[w.pane].tab_mut().scroll = next;
            self.clamp_scroll(w.pane);
            let now = self.panes[w.pane].tab().scroll;
            done = (now - w.target).abs() < 0.5 || (now - cur).abs() < 0.01;
            self.wheel = Some(Wheel { last: now, ..w });
            self.invalidate();
        }
        if done {
            self.wheel = None;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_WHEEL);
            }
        }
    }

    pub(super) fn clear_flash(&mut self) {
        self.toast_undo = None;
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                tab.message = None;
            }
        }
        self.invalidate();
    }

    /// 보이는 칸들의 지금 폴더 (활성 칸부터, 같은 폴더는 한 번)
    pub(super) fn visible_dirs(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in std::iter::once(self.active).chain(0..self.layout.count()) {
            if let Some(d) = self.panes[p].tab().loc.dir() {
                if !out.iter().any(|x| util::eq_path(x, d)) {
                    out.push(d.to_string());
                }
            }
        }
        out
    }
}
