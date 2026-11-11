//! 목록 스크롤

use super::*;

impl App {
    // ---------- 스크롤 ----------

    pub(super) fn grid(&self, p: usize) -> Option<(R, Grid)> {
        self.list_rect(p).map(|l| (l, Grid::of(l, self.panes[p].view)))
    }

    pub(super) fn clamp_scroll(&mut self, p: usize) {
        if let Some(max) = self.start_max_scroll(p) {
            let tab = self.panes[p].tab_mut();
            tab.scroll = tab.scroll.clamp(0.0, max);
            return;
        }
        let Some((list, grid)) = self.grid(p) else { return };
        let tab = self.panes[p].tab_mut();
        let max = (grid.content(tab.view.len()) - grid.page(list)).max(0.0);
        tab.scroll = tab.scroll.clamp(0.0, max);
    }

    pub(super) fn ensure_visible(&mut self, p: usize) {
        if self.panes[p].tab().loc == Location::Start {
            self.clamp_scroll(p);
            return;
        }
        let Some((list, grid)) = self.grid(p) else { return };
        let tab = self.panes[p].tab_mut();
        // 스크롤 0 기준 위치 (스크롤 방향으로)
        let it = grid.item(tab.cursor, 0.0);
        let (top, bottom) = if grid.horiz { (it.l - list.l, it.r - list.l + 4.0) } else { (it.t - list.t, it.b - list.t) };
        let page = grid.page(list);
        if top < tab.scroll {
            tab.scroll = top;
        } else if bottom > tab.scroll + page {
            tab.scroll = bottom - page;
        }
        self.clamp_scroll(p);
    }

    pub(super) fn scroll_by(&mut self, p: usize, dy: f32) {
        self.panes[p].tab_mut().scroll += dy;
        self.clamp_scroll(p);
        if self.edit_mode == EditMode::Rename(p) {
            self.layout_edits();
        }
        self.invalidate();
    }

    /// 한 화면에 들어가는 항목 수 (Page Up/Down 이동량)
    pub(super) fn page_rows(&self, p: usize) -> isize {
        self.grid(p)
            .map(|(l, g)| {
                if g.horiz {
                    ((l.w() / g.cell_w).floor() as isize).max(1) * g.rows as isize
                } else {
                    ((l.h() / g.cell_h).floor() as isize).max(1) * g.cols as isize
                }
            })
            .unwrap_or(10)
    }
}
