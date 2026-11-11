//! 이동: 경로·뒤로·앞으로·위로·열기

use super::*;

impl App {
    // ---------- 이동 ----------

    pub fn navigate(&mut self, p: usize, loc: Location, select: Option<String>) {
        if matches!(self.edit_mode, EditMode::Rename(e) | EditMode::Address(e) if e == p) {
            self.cancel_edit();
        }
        let tab = self.panes[p].tab_mut();
        tab.nav_push(loc);
        tab.reset_view_state();
        tab.restore = select.map(|s| Restore::select(&s));
        self.load(p, false);
        self.after_location_change(p);
    }

    /// 바깥(폴더 열기·Win+E·명령줄)에서 넘어온 경로. 폴더는 활성 패널의 새 탭, 파일은 위치를 열고 선택.
    /// 경로가 없으면(Win+E) 창만 앞으로
    pub fn open_external(&mut self, path: Option<String>) {
        let Some(raw) = path else { return };
        // 설정 화면 뒤에 새 탭이 가려지지 않게 닫는다
        self.close_settings();
        self.fav_blur();
        let p = self.active;
        // 꺼진 NAS 경로여도 보낸 쪽(다른 Sakadu·탐색기 대신 연 창)을 기다리게 하지 않게, 확인은 뒤에서 한다
        self.probe(p, raw, ProbeThen::External);
    }

    pub(super) fn after_location_change(&mut self, p: usize) {
        let loc = self.panes[p].tab().loc.clone();
        self.note_recent(&loc);
        self.start_keys = false;
        if p == self.active {
            self.set_filter_edit_text("");
            self.layout_edits();
            // "현재 폴더" 범위로 찾는 중이면 바뀐 폴더로 다시 찾는다
            self.pal_follow_scope();
        }
        self.sync_watches();
        self.update_title();
        self.invalidate();
    }

    pub(super) fn go_back(&mut self, p: usize) {
        let tab = self.panes[p].tab_mut();
        let from = tab.loc.clone();
        if tab.nav_back() {
            let select = child_toward(&tab.loc, &from);
            tab.reset_view_state();
            tab.restore = select.map(|s| Restore::select(&s));
            self.load(p, false);
            self.after_location_change(p);
        }
    }

    pub(super) fn go_forward(&mut self, p: usize) {
        let tab = self.panes[p].tab_mut();
        if tab.nav_forward() {
            tab.reset_view_state();
            self.load(p, false);
            self.after_location_change(p);
        }
    }

    pub(super) fn go_up(&mut self, p: usize) {
        if let Location::Dir(d) = self.panes[p].tab().loc.clone() {
            match util::parent(&d) {
                Some(parent) => self.navigate(p, Location::Dir(parent), Some(util::file_name(&d).to_string())),
                None => self.navigate(p, Location::Computer, Some(d)),
            }
        }
    }

    pub(super) fn refresh(&mut self, p: usize) {
        // 감시 등록에 실패했던 폴더는 F5로 다시 시도한다
        if self.watches[p].as_ref().is_some_and(|(id, _)| *id == 0) {
            self.watches[p] = None;
        }
        self.load(p, true);
    }

    pub(super) fn open_items(&mut self, p: usize, new_tab: bool) {
        let tab = self.panes[p].tab();
        let Some(cur) = tab.cursor_entry() else { return };
        if cur.is_dir() {
            let Some(path) = tab.path_of(cur) else { return };
            if new_tab {
                self.new_tab(p, Some(Location::Dir(path)), false);
            } else {
                self.navigate(p, Location::Dir(path), None);
            }
            return;
        }
        let mut paths: Vec<String> = tab
            .view
            .iter()
            .map(|&i| &tab.entries[i as usize])
            .filter(|e| e.selected && !e.is_dir())
            .filter_map(|e| tab.path_of(e))
            .collect();
        if paths.is_empty() {
            paths.extend(tab.path_of(cur));
        }
        let dir = tab.loc.dir().map(String::from);
        if paths.len() > OPEN_ASK_OVER {
            self.pending.push(Deferred::ConfirmOpen { pane: p, paths });
        } else {
            self.pending.push(Deferred::Open { pane: p, paths, dir });
        }
    }
}
