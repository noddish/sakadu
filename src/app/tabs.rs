//! 탭과 칸(패널) 다루기

use super::*;

impl App {
    // ---------- 탭·패널 ----------

    /// 위치 없이 여는 새 탭(Ctrl+T·+·탭 줄 더블클릭)의 위치: 설정에 따라 내 PC, 정해 둔 폴더, 지금 탭과 같은 폴더
    pub(super) fn new_tab_location(&self, p: usize) -> Location {
        if self.new_tab_home == session::NEW_TAB_SAME { self.panes[p].tab().loc.clone() } else { Location::from_session(&self.new_tab_home) }
    }

    pub fn new_tab(&mut self, p: usize, loc: Option<Location>, activate: bool) {
        let loc = loc.unwrap_or_else(|| self.new_tab_location(p));
        // 뒤에 여는 탭(가운데 클릭)은 넣지 않는다: 그대로 보이는 시작 화면의 줄이 마우스 아래에서 밀리지 않게
        if activate {
            self.note_recent(&loc);
        }
        let id = self.next_tab_id();
        let pane = &mut self.panes[p];
        let idx = pane.active + 1;
        pane.tabs.insert(idx, Tab::new(id, loc));
        if activate {
            pane.active = idx;
            self.after_tab_switch(p);
        } else {
            self.load_at(p, idx, false);
        }
        self.invalidate();
    }

    pub(super) fn close_tab(&mut self, p: usize, t: usize) {
        let pane = &mut self.panes[p];
        if pane.tabs.len() <= 1 || t >= pane.tabs.len() {
            return;
        }
        let was_active = pane.active == t;
        let closed = pane.tabs.remove(t);
        // Ctrl+Shift+T로 다시 열 수 있게 위치와 뒤로·앞으로 기록을 남긴다
        self.closed_tabs.push((p, closed.loc, closed.back, closed.fwd));
        if self.closed_tabs.len() > CLOSED_TABS_KEEP {
            self.closed_tabs.remove(0);
        }
        let pane = &mut self.panes[p];
        if t < pane.active || pane.active >= pane.tabs.len() {
            pane.active = pane.active.saturating_sub(1);
        }
        if was_active {
            self.after_tab_switch(p);
        }
        self.invalidate();
    }

    /// 마지막에 닫은 탭을 닫았던 칸(이제 없으면 지금 칸)에 다시 연다
    pub(super) fn reopen_closed_tab(&mut self) {
        let Some((p, loc, back, fwd)) = self.closed_tabs.pop() else {
            self.flash(self.active, "다시 열 탭이 없어요");
            return;
        };
        let p = if p < self.layout.count() { p } else { self.active };
        self.activate_pane(p);
        let id = self.next_tab_id();
        let pane = &mut self.panes[p];
        let idx = pane.active + 1;
        let mut tab = Tab::new(id, loc);
        tab.back = back;
        tab.fwd = fwd;
        pane.tabs.insert(idx, tab);
        pane.active = idx;
        self.after_tab_switch(p);
        self.invalidate();
    }

    /// 끌던 탭을 놓을 곳: (칸, 들어갈 위치). 탭 줄 위면 놓은 자리, 칸 본문이면 그 칸의 맨 끝. 칸 밖이면 None
    pub(super) fn tab_drop_target(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let (q, r) = self.pane_rects().into_iter().find(|(_, r)| r.contains(x, y))?;
        let g = geom(r);
        let n = self.panes[q].tabs.len();
        let at = if g.head.contains(x, y) { drop_index(&self.chips(q, &g).0, x) } else { n };
        Some((q, at))
    }

    /// 탭을 같은 칸 안에서 옮기거나 다른 칸으로 보낸다.
    /// 칸에 하나뿐인 탭은 그 칸을 비울 수 없으니 남겨 두고, 대상 칸에 같은 위치를 새 탭으로 연다
    pub(super) fn move_tab(&mut self, p: usize, t: usize, q: usize, at: usize) {
        if t >= self.panes[p].tabs.len() {
            return;
        }
        if p == q {
            let pane = &mut self.panes[p];
            let to = (if at > t { at - 1 } else { at }).min(pane.tabs.len() - 1);
            if to != t {
                let tab = pane.tabs.remove(t);
                pane.tabs.insert(to, tab);
                pane.active = to;
            }
            self.invalidate();
            return;
        }
        let tab = if self.panes[p].tabs.len() > 1 {
            let src = &mut self.panes[p];
            let tab = src.tabs.remove(t);
            if t < src.active || src.active >= src.tabs.len() {
                src.active = src.active.saturating_sub(1);
            }
            self.after_tab_switch(p);
            tab
        } else {
            let loc = self.panes[p].tab().loc.clone();
            Tab::new(self.next_tab_id(), loc)
        };
        let dst = &mut self.panes[q];
        let at = at.min(dst.tabs.len());
        dst.tabs.insert(at, tab);
        dst.active = at;
        self.activate_pane(q);
        self.after_tab_switch(q);
    }

    pub(super) fn activate_tab(&mut self, p: usize, t: usize) {
        if t < self.panes[p].tabs.len() && t != self.panes[p].active {
            self.panes[p].active = t;
            self.after_tab_switch(p);
        }
    }

    pub(super) fn cycle_tab(&mut self, p: usize, back: bool) {
        let n = self.panes[p].tabs.len();
        let a = self.panes[p].active;
        self.activate_tab(p, if back { (a + n - 1) % n } else { (a + 1) % n });
    }

    /// 탭을 바꾸면 그 탭을 다시 읽는다(보이지 않는 동안 바뀐 내용 반영)
    pub(super) fn after_tab_switch(&mut self, p: usize) {
        if matches!(self.edit_mode, EditMode::Rename(e) | EditMode::Address(e) if e == p) {
            self.cancel_edit();
        }
        self.start_keys = false;
        let t = self.panes[p].active;
        let fresh = self.panes[p].tab().generation == 0;
        self.load_at(p, t, !fresh);
        if p == self.active {
            let f = self.panes[p].tab().filter.clone();
            self.set_filter_edit_text(&f);
            // "현재 폴더" 범위로 찾는 중이면 이 탭의 폴더로 다시 찾는다
            self.pal_follow_scope();
        }
        self.sync_watches();
        self.update_title();
        self.layout_edits();
        self.invalidate();
    }

    pub(super) fn activate_pane(&mut self, p: usize) {
        if p == self.active || p >= self.layout.count() {
            return;
        }
        match self.edit_mode {
            EditMode::None => {}
            // 이름(파일·즐겨찾기·작업 공간)은 확정하고, 경로 입력은 취소한다 (탐색기와 같음)
            EditMode::Rename(_) | EditMode::FavRename(_) | EditMode::WorkspaceRename => self.commit_rename(),
            EditMode::Address(_) => self.cancel_edit(),
        }
        self.active = p;
        self.start_keys = false;
        let f = self.panes[p].tab().filter.clone();
        self.set_filter_edit_text(&f);
        // "현재 폴더" 범위로 찾는 중이면 이 칸의 폴더로 다시 찾는다
        self.pal_follow_scope();
        self.layout_edits();
        self.update_title();
        self.invalidate();
    }

    pub(super) fn cycle_pane(&mut self, back: bool) {
        let n = self.layout.count();
        let next = if back { (self.active + n - 1) % n } else { (self.active + 1) % n };
        self.activate_pane(next);
    }

    pub(super) fn set_layout(&mut self, l: Layout) {
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        self.close_view_menu();
        let before = self.layout.count();
        self.layout = l;
        if self.active >= l.count() {
            self.active = 0;
            let f = self.panes[0].tab().filter.clone();
            self.set_filter_edit_text(&f);
        }
        for p in 0..l.count() {
            if self.panes[p].tab().generation == 0 {
                self.load(p, false);
            } else if p >= before {
                // 숨어 있는 동안에는 감시하지 않았으니, 다시 보일 때 선택·스크롤을 지키며 새로 읽는다
                self.load(p, true);
            }
            self.clamp_scroll(p);
        }
        // 지금 칸이 바뀌었으면(칸을 줄여 지금 칸이 사라짐) "현재 폴더" 범위 검색도 따라간다
        self.pal_follow_scope();
        self.sync_watches();
        self.layout_edits();
        self.update_title();
        self.invalidate();
    }

    pub(super) fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        let hidden = self.show_hidden;
        for pane in &mut self.panes {
            let sort = pane.sort;
            for tab in &mut pane.tabs {
                tab.rebuild_view(sort, hidden);
            }
        }
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.layout_edits();
        self.invalidate();
    }

    pub(super) fn set_view(&mut self, p: usize, v: ViewMode) {
        if self.panes[p].view == v {
            return;
        }
        if self.edit_mode == EditMode::Rename(p) {
            self.cancel_edit();
        }
        self.panes[p].view = v;
        self.ensure_visible(p);
        self.invalidate();
    }

    pub(super) fn on_thumb(&mut self, img: crate::thumbs::Image) {
        self.thumb_wanted.remove(&img.key);
        if img.pixels.is_empty() {
            if self.thumb_failed.len() > 4096 {
                self.thumb_failed.clear();
            }
            self.thumb_failed.insert(img.key);
            // 미리 보기가 이 그림을 기다리고 있으면 "미리 볼 수 없습니다"로 바꿔 그린다
            self.invalidate();
            return;
        }
        if self.gfx.add_thumb(img.key, img.w, img.h, &img.pixels) {
            self.invalidate();
        }
    }

    /// 화면에 필요한데 없는 그림만 요청한다. 같은 목록을 다시 보내지 않는다
    pub(super) fn request_thumbs(&mut self, missing: Vec<crate::thumbs::Request>) {
        let keys: HashSet<String> = missing.iter().map(|r| r.key.clone()).collect();
        if keys != self.thumb_wanted {
            self.thumb_wanted = keys;
            crate::thumbs::request(self.hwnd, missing);
        }
    }

    pub(super) fn toggle_sort(&mut self, p: usize, key: SortKey) {
        let pane = &mut self.panes[p];
        if pane.sort.key == key {
            pane.sort.desc = !pane.sort.desc;
        } else {
            // 날짜·크기는 큰 값(최신·큰 파일)부터 보는 경우가 많다
            pane.sort = Sort { key, desc: matches!(key, SortKey::Modified | SortKey::Size) };
        }
        let sort = pane.sort;
        for tab in &mut pane.tabs {
            tab.rebuild_view(sort, self.show_hidden);
        }
        self.ensure_visible(p);
    }
}
