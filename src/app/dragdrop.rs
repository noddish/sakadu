//! 드래그 앤 드롭

use super::*;

impl App {
    // ---------- 드래그 앤 드롭 ----------

    /// 드롭 위치의 대상. 폴더 행 위면 그 폴더, 패널 안 다른 곳이면 패널의 현재 폴더.
    /// 사이드바는 위쪽(고정 목록)이면 이 작업 공간에 고정, '모든 공간' 구역이면 즐겨찾기에 넣기
    pub fn drop_dir_at(&mut self, pt: POINTL) -> DropAt {
        let mut cp = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut cp);
        }
        let (x, y) = (cp.x as f32 / self.scale(), cp.y as f32 / self.scale());
        let over_side = self.settings.is_none() && (x < BAR_W || self.side_panel_rect().is_some_and(|r| r.contains(x, y)));
        if over_side && matches!(self.hover, Hit::Crumb(..)) {
            self.hover = Hit::None;
            self.invalidate();
        }
        // 레일의 지금 공간 아이콘 위에 놓으면 이 공간에 고정한다 (패널이 닫혀 있어도)
        if x < BAR_W && self.settings.is_none() {
            let side = matches!(self.bar_hit(x, y), Hit::Rail(i) if Some(i) == self.workspaces.current).then_some(Side::PinEmpty);
            if self.drop_hover.take().is_some() || side != self.side_drop {
                self.side_drop = side;
                self.invalidate();
            }
            return if side.is_some() { DropAt::Pins } else { DropAt::None };
        }
        if self.settings.is_none() && self.side_panel_rect().is_some_and(|r| r.contains(x, y)) {
            let side = self.side_drop_at(x, y);
            if self.drop_hover.take().is_some() || side != self.side_drop {
                self.side_drop = side;
                self.invalidate();
            }
            return match side {
                Some(Side::FavEmpty) => DropAt::Favorites,
                Some(_) => DropAt::Pins,
                None => DropAt::None,
            };
        }
        if self.side_drop.take().is_some() {
            self.invalidate();
        }
        let (hover, dir) = match self.hit(x, y) {
            Hit::Row(p, vi) => {
                let tab = self.panes[p].tab();
                match tab.entry_at(vi) {
                    Some(e) if e.is_dir() => (Some((p, Some(vi))), tab.path_of(e)),
                    _ => (Some((p, None)), tab.loc.dir().map(String::from)),
                }
            }
            Hit::Tab(p, t) => (Some((p, None)), self.panes[p].tabs[t].loc.dir().map(String::from)),
            // 경로 조각: 그 단계의 폴더로 놓는다. 칸 테두리 대신 조각을 강조한다 (아래 crumb_drop)
            Hit::Crumb(p, i) => {
                let target = self
                    .pane_rect(p)
                    .and_then(|r| crumbs(&self.gfx, geom(r).crumbs, &self.panes[p].tab().loc, Some("내 PC")).into_iter().nth(i))
                    .and_then(|c| c.target.dir().map(String::from));
                (None, target)
            }
            h => match h.pane() {
                Some(p) => (Some((p, None)), self.panes[p].tab().loc.dir().map(String::from)),
                None => (None, None),
            },
        };
        let hover = if dir.is_some() { hover } else { None };
        // 놓을 경로 조각은 마우스를 올린 것처럼 강조한다 (끄는 동안에는 마우스 이동 메시지가 오지 않는다)
        let crumb = match self.hit(x, y) {
            Hit::Crumb(p, i) if dir.is_some() => Some(Hit::Crumb(p, i)),
            _ => None,
        };
        if let Some(h) = crumb {
            if self.hover != h {
                self.hover = h;
                self.invalidate();
            }
        } else if matches!(self.hover, Hit::Crumb(..)) {
            self.hover = Hit::None;
            self.invalidate();
        }
        if hover != self.drop_hover {
            self.drop_hover = hover;
            self.invalidate();
        }
        dir.map_or(DropAt::None, DropAt::Dir)
    }

    pub fn clear_drop_hover(&mut self) {
        if self.drop_hover.take().is_some() | self.side_drop.take().is_some() {
            self.invalidate();
        }
        if matches!(self.hover, Hit::Crumb(..)) {
            self.hover = Hit::None;
            self.invalidate();
        }
    }
}
