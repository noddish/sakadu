//! 칸·작업 줄·미리 보기 자리 계산 (App 상태에 따라)

use super::*;

impl App {
    // ---------- 레이아웃 ----------

    /// 칸이 놓이는 곳: 아이콘 막대(고정한 패널이 있으면 그 패널) 오른쪽, 작업 줄 아래 (미리 보기가 열려 있으면 그만큼 비운다)
    pub(super) fn area(&self) -> R {
        let l = self.panes_left();
        let right = self.w - MARGIN - if self.preview.is_some() { self.preview_w() + GAP } else { 0.0 };
        R::new(l, WORKBAR_H, right.max(l + 100.0), (self.h - MARGIN).max(WORKBAR_H + 100.0))
    }

    /// 칸 나누기 버튼 5개를 한 덩어리로 담는 판. 작업 줄에서 칸과 같은 왼쪽 선
    pub(super) fn layout_pill(&self) -> R {
        let l = self.workspace_button().r + 8.0;
        R::new(l, 6.0, l + 6.0 + 5.0 * 32.0, 38.0)
    }

    pub(super) fn layout_buttons(&self) -> [R; 5] {
        let l0 = self.layout_pill().l + 3.0;
        std::array::from_fn(|i| {
            let l = l0 + i as f32 * 32.0;
            R::new(l, 9.0, l + 32.0, 35.0)
        })
    }

    /// 작업 줄 오른쪽: 창 단추 바로 앞에 미리 보기, 그 앞에 숨김 항목, 그 앞에 `…`
    pub(super) fn preview_button(&self) -> R {
        let r = self.caption_left() - 8.0;
        R::new(r - 32.0, 6.0, r, 38.0)
    }

    pub(super) fn hidden_button(&self) -> R {
        let p = self.preview_button();
        R::new(p.l - 36.0, 6.0, p.l - 4.0, 38.0)
    }

    pub(super) fn more_button(&self) -> R {
        let h = self.hidden_button();
        R::new(h.l - 36.0, 6.0, h.l - 4.0, 38.0)
    }

    pub(super) fn pane_rects(&self) -> Vec<(usize, R)> {
        pane_rects(self.area(), self.split, self.layout)
    }

    pub(super) fn pane_rect(&self, p: usize) -> Option<R> {
        self.pane_rects().into_iter().find(|(i, _)| *i == p).map(|(_, r)| r)
    }

    pub(super) fn splitters(&self) -> (Option<R>, Option<R>) {
        splitters(self.area(), self.split, self.layout)
    }

    pub(super) fn list_rect(&self, p: usize) -> Option<R> {
        self.pane_rect(p).map(|r| geom(r).list)
    }

    /// 칸 p의 칩 줄 오른쪽 글자: 불러오는 중이거나, 항목 수와 고른 개수·크기
    pub(super) fn head_count_text(&self, p: usize) -> String {
        let tab = self.panes[p].tab();
        if tab.loc == Location::Start {
            return String::new();
        }
        match &tab.state {
            LoadState::Loading => "불러오는 중…".to_string(),
            LoadState::Error(_) => "열 수 없음".to_string(),
            LoadState::Ready if tab.offline => "연결 끊김 · 마지막으로 본 목록".to_string(),
            LoadState::Ready if tab.loading_since.is_some_and(|s| s.elapsed().as_millis() >= LOADING_SHOW_MS * 2) => "새로 고치는 중…".to_string(),
            LoadState::Ready => {
                let (sel, bytes) = tab.selection_stats();
                let n = tab.view.len();
                match (sel, bytes) {
                    (0, _) => format!("{}개", util::grouped(n as u64)),
                    (s, 0) => format!("{}개 중 {}개 선택", util::grouped(n as u64), util::grouped(s as u64)),
                    (s, b) => format!("{}개 중 {}개 선택 · {}", util::grouped(n as u64), util::grouped(s as u64), format_size(b)),
                }
            }
        }
    }

    /// 칩 줄 오른쪽 자리의 폭: 필터 칩, 아니면 항목 수 글자 (너무 길면 칩에 자리를 남기고 줄인다)
    pub(super) fn head_right_w(&self, p: usize, g: &PaneGeom) -> f32 {
        if !self.panes[p].tab().filter.is_empty() {
            return filter_chip(g).w();
        }
        self.gfx.measure(&self.head_count_text(p), Font::Small).min(g.head.w() * 0.35)
    }

    /// 칩 줄 오른쪽 자리: 필터 칩, 아니면 항목 수 글자. 좁은 칸에서 탭 칩과 겹치면 이쪽이 위에 그려지고 먼저 눌린다
    pub(super) fn head_right_rect(&self, p: usize, g: &PaneGeom) -> R {
        if !self.panes[p].tab().filter.is_empty() {
            return filter_chip(g);
        }
        let w = self.head_right_w(p, g);
        R::new(g.head.r - 8.0 - w, g.head.t + 8.0, g.head.r - 8.0, g.head.t + 36.0)
    }

    /// 칸 p의 탭 칩 자리와 새 탭(+) 자리
    pub(super) fn chips(&self, p: usize, g: &PaneGeom) -> (Vec<R>, R) {
        let pane = &self.panes[p];
        let n = pane.tabs.len();
        let natural: Vec<f32> = pane
            .tabs
            .iter()
            .enumerate()
            .map(|(_, tab)| {
                // 지금 탭만 굵게 그리지만 폭은 모두 굵은 글자로 잰다: 탭을 누를 때마다 옆 탭이 밀리지 않게
                (self.gfx.measure(&tab.loc.title(), Font::Normal) + 22.0 + if n > 1 { 20.0 } else { 0.0 }).clamp(52.0, 180.0)
            })
            .collect();
        let strip = chip_strip(g, self.head_right_w(p, g));
        let trs = tab_rects(strip, &natural, pane.active);
        let add = add_rect(strip, &trs);
        (trs, add)
    }

    /// 필터 칩 안의 개수: 고른 게 있으면 "고른/전체개"
    pub(super) fn filter_count_text(&self, p: usize) -> String {
        if let Some(n) = self.start_count(p) {
            return format!("{n}개");
        }
        let tab = self.panes[p].tab();
        match tab.selection_stats().0 {
            0 => format!("{}개", util::grouped(tab.view.len() as u64)),
            s => format!("{}/{}개", util::grouped(s as u64), util::grouped(tab.view.len() as u64)),
        }
    }

    /// 필터 칩 안: (입력 상자 자리, 개수 글자 자리, × 단추).
    /// 개수 자리는 "0,000개"만큼은 늘 잡아 둬, 개수가 바뀌어도 입력 상자(자식 창)가 개수를 덮지 않게 한다
    pub(super) fn filter_parts(&self, p: usize, chip: R) -> (R, R, R) {
        let n = self.filter_count_text(p);
        let nw = self.gfx.measure(&n, Font::Small).max(self.gfx.measure("0,000개", Font::Small));
        let clear = R::new(chip.r - 24.0, chip.t + 4.0, chip.r - 4.0, chip.b - 4.0);
        let count = R::new(clear.l - 4.0 - nw, chip.t, clear.l - 4.0, chip.b);
        let edit = R::new(chip.l + 28.0, chip.t, (count.l - 6.0).max(chip.l + 60.0), chip.b);
        (edit, count, clear)
    }

    pub(super) fn hit(&self, x: f32, y: f32) -> Hit {
        if self.settings.is_some() {
            return Hit::Set(self.settings_hit(x, y));
        }
        // 명령 메뉴는 모든 것 위에 떠 있다 (밖을 누르면 닫힌다)
        if let Some(h) = self.cmd_hit(x, y) {
            return h;
        }
        if x < BAR_W {
            return self.bar_hit(x, y);
        }
        if let Some(h) = self.view_menu_hit(x, y) {
            return h;
        }
        for (p, _) in self.pane_rects() {
            if let Some((pill, btn)) = self.toast_rects(p) {
                if btn.is_some_and(|b| b.contains(x, y)) {
                    return Hit::ToastUndo(p);
                }
                if pill.contains(x, y) {
                    return Hit::Toast(p);
                }
            }
        }
        // 패널은 칸보다 위에 있다 (떠 있을 때 그 아래 칸을 누르지 않게)
        if self.side_panel_rect().is_some_and(|r| r.contains(x, y)) {
            return self.sidebar_hit(x, y);
        }
        if self.side_resize_rect().is_some_and(|r| r.contains(x, y)) {
            return Hit::SideResize;
        }
        for (p, _) in self.pane_rects() {
            if self.net_button(p).is_some_and(|(b, _)| b.contains(x, y)) {
                return Hit::NetFix(p);
            }
        }
        if y < WORKBAR_H {
            if self.workspace_button().contains(x, y) {
                return Hit::Workspace;
            }
            if let Some(i) = self.layout_buttons().iter().position(|r| r.contains(x, y)) {
                return Hit::Layout(i);
            }
            if self.hidden_button().contains(x, y) {
                return Hit::Hidden;
            }
            if self.preview_button().contains(x, y) {
                return Hit::PreviewButton;
            }
            if self.more_button().contains(x, y) {
                return Hit::More;
            }
            return Hit::None;
        }
        if let Some(h) = self.preview_hit(x, y) {
            return h;
        }
        let (sx, sy) = self.splitters();
        if sx.is_some_and(|r| r.contains(x, y)) {
            return Hit::SplitX;
        }
        if sy.is_some_and(|r| r.contains(x, y)) {
            return Hit::SplitY;
        }
        for (p, r) in self.pane_rects() {
            if !r.contains(x, y) {
                continue;
            }
            let g = geom(r);
            let pane = &self.panes[p];
            let tab = pane.tab();
            if g.head.contains(x, y) {
                // 오른쪽 자리(필터 칩·개수)는 탭 칩 위에 그려지므로 먼저 본다
                let right = self.head_right_rect(p, &g);
                if !tab.filter.is_empty() && right.contains(x, y) {
                    if self.filter_parts(p, right).2.contains(x, y) {
                        return Hit::FilterClear(p);
                    }
                    return Hit::Pane(p);
                }
                if x >= right.l - 4.0 {
                    return Hit::TabStrip(p);
                }
                let (trs, add) = self.chips(p, &g);
                for (i, tr) in trs.iter().enumerate() {
                    if tr.contains(x, y) {
                        if pane.tabs.len() > 1 && close_rect(*tr).contains(x, y) {
                            return Hit::TabClose(p, i);
                        }
                        return Hit::Tab(p, i);
                    }
                }
                if add.contains(x, y) {
                    return Hit::TabAdd(p);
                }
                return Hit::TabStrip(p);
            }
            if g.back.contains(x, y) {
                return Hit::Back(p);
            }
            if g.fwd.contains(x, y) {
                return Hit::Fwd(p);
            }
            if g.up.contains(x, y) {
                return Hit::Up(p);
            }
            // 시작 화면에는 보기·새 폴더 단추가 없다
            let start = tab.loc == Location::Start;
            if !start && g.view.contains(x, y) {
                return Hit::View(p);
            }
            if !start && g.new_folder.contains(x, y) {
                return Hit::NewFolder(p);
            }
            if tab.loc.dir().is_some() && self.edit_mode != EditMode::Address(p) && g.star.contains(x, y) {
                return Hit::Star(p);
            }
            if g.crumb.contains(x, y) {
                let cs = crumbs(&self.gfx, g.crumbs, &tab.loc, Some("내 PC"));
                return match cs.iter().position(|c| x >= c.x0 && x < c.x1) {
                    Some(i) => Hit::Crumb(p, i),
                    None => Hit::CrumbEmpty(p),
                };
            }
            if start {
                return self.start_hit(p, x, y);
            }
            if g.header.contains(x, y) {
                if pane.view != ViewMode::Details {
                    return match sort_chips(&self.gfx, g.header, pane.sort.key).into_iter().find(|(_, _, r)| r.contains(x, y)) {
                        Some((key, _, _)) => Hit::Header(p, key),
                        None => Hit::Pane(p),
                    };
                }
                let cols = columns(g.list, pane.cols);
                for (ci, c) in cols.resizable().iter().enumerate() {
                    if let Some((l, _)) = c {
                        if (x - l).abs() <= 4.0 {
                            return Hit::HeaderResize(p, ci);
                        }
                    }
                }
                let key = if cols.size.is_some_and(|(l, _)| x >= l) {
                    SortKey::Size
                } else if cols.kind.is_some_and(|(l, _)| x >= l) {
                    SortKey::Type
                } else if cols.modified.is_some_and(|(l, _)| x >= l) {
                    SortKey::Modified
                } else {
                    SortKey::Name
                };
                return Hit::Header(p, key);
            }
            if g.list.contains(x, y) {
                let grid = Grid::of(g.list, pane.view);
                if let Some((track, thumb)) = grid.bar(g.list, tab.view.len(), tab.scroll) {
                    if if grid.horiz { y >= track.t } else { x >= track.l } {
                        return if thumb.contains(x, y) { Hit::Thumb(p) } else { Hit::Track(p) };
                    }
                }
                return match grid.at(x, y, tab.scroll, tab.view.len()) {
                    Some(vi) => Hit::Row(p, vi),
                    None => Hit::ListEmpty(p),
                };
            }
            return Hit::Pane(p);
        }
        Hit::None
    }
}
