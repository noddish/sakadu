//! 파일 작업 (복사·이동·삭제·새 폴더·되돌리기)

use super::*;

impl App {
    // ---------- 파일 작업 ----------

    pub(super) fn delete(&mut self, p: usize, permanent: bool) {
        let tab = self.panes[p].tab();
        if tab.loc == Location::Computer {
            return;
        }
        // 선택을 풀어 둔 상태에서는 커서 항목으로 대신하지 않는다
        let paths = tab.marked_paths();
        if paths.is_empty() {
            self.flash(p, "지울 항목을 먼저 고르세요");
            return;
        }
        shell::spawn_file_op(self.hwnd, FileOp::Delete { paths, permanent });
    }

    /// Ctrl+Shift+C: 고른 항목의 경로를 글자로 복사한다 (한 줄에 하나, 따옴표 없이). 고른 게 없으면 지금 폴더
    pub(super) fn copy_paths(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        let mut paths = tab.marked_paths();
        if paths.is_empty() {
            paths.extend(tab.loc.dir().map(String::from));
        }
        if paths.is_empty() {
            return;
        }
        let n = paths.len();
        if shell::set_clipboard_text(self.hwnd, &paths.join("\r\n")).is_ok() {
            self.flash(p, if n == 1 { format!("경로를 복사했어요: {}", paths[0]) } else { format!("경로 {n}개를 복사했어요") });
        }
    }

    pub(super) fn copy(&mut self, p: usize, cut: bool) {
        let tab = self.panes[p].tab();
        // 고른 것이 없으면(커서만 남은 상태) 아무것도 올리지 않는다 (탐색기와 같음)
        let paths = tab.marked_paths();
        if paths.is_empty() {
            self.flash(p, if cut { "잘라낼 항목을 먼저 고르세요" } else { "복사할 항목을 먼저 고르세요" });
            return;
        }
        let n = paths.len();
        self.pending.push(Deferred::Verb { paths, verb: if cut { "cut" } else { "copy" } });
        self.flash(p, format!("항목 {n}개를 {}", if cut { "잘라냈어요" } else { "복사했어요" }));
    }

    pub(super) fn paste(&mut self, p: usize) {
        if let Some(dest) = self.panes[p].tab().loc.dir().map(String::from) {
            self.pending.push(Deferred::Paste { pane: p, dest });
        }
    }

    pub(super) fn new_folder(&mut self, p: usize) {
        let Some(dir) = self.panes[p].tab().loc.dir().map(String::from) else { return };
        let mut name = "새 폴더".to_string();
        let mut n = 2;
        while shell::exists(&util::join(&dir, &name)) {
            name = format!("새 폴더 ({n})");
            n += 1;
        }
        let path = util::join(&dir, &name);
        match std::fs::create_dir(&path) {
            Ok(()) => {
                // 만든 직후 표식을 남겨, 되돌릴 때 그사이 자료가 들어왔으면 지우지 않게 한다
                if let Some(s) = crate::undo::stamp(&path, true) {
                    self.push_undo(crate::undo::Undo::Create(path, s));
                }
                // 필터에 걸려 새 폴더가 보이지 않으면 다른 항목에 이름 상자가 열리므로 필터를 먼저 푼다
                if !self.panes[p].tab().filter.is_empty() {
                    if p == self.active {
                        self.clear_filter();
                    } else {
                        self.panes[p].tab_mut().filter.clear();
                    }
                }
                let tab = self.panes[p].tab_mut();
                tab.restore = Some(Restore { rename: true, scroll: tab.scroll, cursor_index: tab.cursor, ..Restore::select(&name) });
                self.load(p, true);
            }
            Err(e) => self.flash(p, format!("폴더를 만들 수 없어요: {e}")),
        }
    }

    pub(super) fn push_undo(&mut self, u: crate::undo::Undo) {
        crate::diag::log(|| format!("undo: recorded ({})", u.describe()));
        self.undo.push(u);
        self.undo_rev += 1;
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
    }

    /// Ctrl+Z: 이 앱에서 한 마지막 파일 작업을 되돌린다
    pub(super) fn undo_last(&mut self) {
        let p = self.active;
        // 진행 중인 작업은 끝나야 기록이 생긴다: 지금 되돌리면 그 앞 작업이 되돌려진다
        if shell::file_ops_running() {
            self.flash(p, "진행 중인 작업이 끝난 뒤 되돌릴 수 있어요");
            return;
        }
        match self.undo.pop() {
            Some(u) => {
                self.undo_rev += 1;
                self.flash(p, u.describe());
                shell::spawn_file_op(self.hwnd, FileOp::Undo(u));
            }
            None => self.flash(p, "되돌릴 작업이 없어요"),
        }
    }

    pub(super) fn on_file_op(&mut self, r: shell::OpReport) {
        if let Some(u) = r.undo {
            // 옮기기·복사·휴지통: 무엇을 했는지와 되돌리기 단추를 알림으로 (Ctrl+Z와 같은 기록을 쓴다)
            let text = u.done_text();
            self.push_undo(u);
            if let Some(text) = text {
                let p = self.active;
                self.flash(p, text);
                self.toast_undo = Some((p, self.undo_rev));
                unsafe { SetTimer(Some(self.hwnd), TIMER_FLASH, UNDO_TOAST_MS, None) };
            }
        }
        if let Some(o) = r.undone {
            self.on_undone(o);
        }
        if let Some(cut) = r.cut {
            self.after_cut_paste(cut);
        }
        // 복사·이동·삭제로 남은 용량이 바뀌었다
        self.refresh_places();
        // 고정한 미리 보기 파일의 이름이 바뀌었거나 옮겨졌거나 지워졌을 수 있다
        self.preview_pin_check();
    }

    /// 되돌리기가 일부만 됐으면 알리고, 실패한 항목은 기록으로 되돌려 놓아 다시 시도할 수 있게 한다
    pub(super) fn on_undone(&mut self, o: crate::undo::Undone) {
        let failed = o.retry.as_ref().map_or(0, |r| r.len());
        if let Some(r) = o.retry {
            self.push_undo(r);
        }
        let key = self.keys.get(crate::keys::Action::Undo).map_or_else(|| "되돌리기".to_string(), |c| c.label());
        let changed = match o.changed.as_slice() {
            [] => String::new(),
            [one] => format!("‘{}’ 항목은 그사이 바뀌었거나 없어져 그대로 두었어요", util::file_name(one)),
            many => format!("{}개는 그사이 바뀌었거나 없어져 그대로 두었어요", many.len()),
        };
        let msg = match (failed, changed.is_empty()) {
            (0, true) => return,
            (0, false) => changed,
            (n, true) => format!("{n}개를 되돌리지 못했어요 · {key}로 다시 시도"),
            (n, false) => format!("{n}개를 되돌리지 못했어요({key}로 다시 시도) · {changed}"),
        };
        self.flash(self.active, msg);
    }

    /// 알림의 되돌리기: 알린 뒤로 다른 작업·되돌리기가 없었을 때만 (그 작업이 기록 맨 위에 있다)
    pub(super) fn toast_undo_click(&mut self, p: usize) {
        let ours = self.toast_undo.is_some_and(|(tp, rev)| tp == p && rev == self.undo_rev);
        self.panes[p].tab_mut().message = None;
        self.toast_undo = None;
        if ours {
            self.undo_last();
        }
        self.invalidate();
    }

    /// 칸 p의 알림 자리: (알약, 되돌리기 단추)
    pub(super) fn toast_rects(&self, p: usize) -> Option<(R, Option<R>)> {
        let r = self.pane_rect(p)?;
        let m = self.panes[p].tab().message.as_ref()?;
        let g = &self.gfx;
        let list = geom(r).list;
        let undo = self.toast_undo.is_some_and(|(tp, rev)| tp == p && rev == self.undo_rev);
        let bw = if undo { g.measure(TOAST_UNDO, Font::Normal) + 24.0 } else { 0.0 };
        let extra = if undo { bw + 8.0 } else { 0.0 };
        let tw = g.measure(m, Font::Normal).min((r.w() - 80.0 - extra).max(40.0));
        let cx = (r.l + r.r) / 2.0;
        let w = tw + 30.0 + extra;
        let pill = R::new(cx - w / 2.0, list.b - 52.0, cx + w / 2.0, list.b - 16.0);
        let btn = undo.then(|| R::new(pill.r - 6.0 - bw, pill.t + 5.0, pill.r - 6.0, pill.b - 5.0));
        Some((pill, btn))
    }

    /// 잘라내기 붙여넣기가 끝나면 클립보드를 정리한다. 다 옮겼으면 비우고, 남은 파일이 있으면 그것만 다시 잘라 둔다.
    /// 그사이 사용자가 다른 것을 복사했으면 건드리지 않는다
    pub(super) fn after_cut_paste(&mut self, cut: shell::CutLeft) {
        self.moving.end(cut.id);
        let untouched = shell::clipboard_seq() == cut.seq || shell::clipboard_files().is_some_and(|(paths, is_move)| is_move && paths == cut.clip);
        if !untouched || cut.left == cut.clip {
            return;
        }
        if cut.left.is_empty() {
            shell::clear_clipboard();
        } else if let Err(e) = shell::set_clipboard_files(self.hwnd, &cut.left, true) {
            crate::diag::log(|| format!("cut: could not keep remaining files on clipboard: {e}"));
        }
    }

    pub(super) fn properties(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        let any_selected = tab.selection_stats().0 > 0;
        let paths = if any_selected {
            tab.selected_paths()
        } else {
            match tab.loc.dir() {
                Some(d) => vec![d.to_string()],
                None => return,
            }
        };
        self.pending.push(Deferred::Verb { paths, verb: "properties" });
    }

    pub(super) fn context_menu_at_cursor(&mut self, p: usize) {
        let Some((list, grid)) = self.grid(p) else { return };
        let tab = self.panes[p].tab();
        let it = grid.item(tab.cursor, tab.scroll);
        let pt = self.to_screen(it.l + 36.0, it.b.clamp(list.t, list.b));
        // 고른 것이 없으면 빈 곳 메뉴 (커서만 남은 항목의 메뉴를 띄우지 않는다). 빈 곳 메뉴가 없는 내 PC는 커서 항목
        let dir = tab.loc.dir().map(String::from);
        let paths = if dir.is_none() { tab.selected_paths() } else { tab.marked_paths() };
        self.pending.push(Deferred::Menu { pane: p, paths, dir, pt });
    }
}
