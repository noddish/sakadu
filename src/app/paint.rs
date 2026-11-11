//! 그리기

use super::*;
use sakadu::ui::paint::{EntryArt, FilterChip, PaneHover, PaneScene, paint_pane_head, paint_pane_border, paint_pane_list};

impl App {
    // ---------- 그리기 ----------

    pub(super) fn paint(&mut self) {
        // 목록이 마우스 아래에서 밀리면(휠·키보드 스크롤, 새로 고침) 강조된 줄이 엉뚱한 곳에 남지 않게 지금 자리로 맞춘다
        if matches!(self.drag, Drag::None) && matches!(self.hover, Hit::Row(..)) {
            let mut pt = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(self.hwnd, &mut pt);
            }
            let h = self.hit(pt.x as f32 / self.scale(), pt.y as f32 / self.scale());
            if h != self.hover {
                self.hover = h;
            }
        }
        self.sync_preview();
        self.place_filter_edit(false);
        if !self.gfx.begin(self.hwnd) {
            return;
        }
        self.gfx.clear(self.theme.window);
        // 설정 창은 창 전체를 덮으니 그 아래는 그리지 않는다
        if self.settings.is_some() {
            self.paint_settings();
            self.paint_caption();
            self.gfx.end();
            return;
        }
        self.paint_bar();
        self.paint_workbar();
        self.paint_caption();
        let mut missing = Vec::new();
        for (p, r) in self.pane_rects() {
            self.paint_pane(p, r, &mut missing);
        }
        self.paint_preview(&mut missing);
        // 패널은 칸 다음에: 떠 있을 때 칸 위에 그려진다
        self.paint_sidebar();
        let t = self.theme;
        let (sx, sy) = self.splitters();
        let hot_x = matches!(self.drag, Drag::SplitX) || self.hover == Hit::SplitX;
        let hot_y = matches!(self.drag, Drag::SplitY) || self.hover == Hit::SplitY;
        if let (Some(r), true) = (sx, hot_x) {
            let cx = (r.l + r.r) / 2.0;
            self.gfx.fill_round(R::new(cx - 1.5, r.t + 8.0, cx + 1.5, r.b - 8.0), 1.5, t.focus);
        }
        if let (Some(r), true) = (sy, hot_y) {
            let cy = (r.t + r.b) / 2.0;
            self.gfx.fill_round(R::new(r.l + 8.0, cy - 1.5, r.r - 8.0, cy + 1.5), 1.5, t.focus);
        }
        let hot_side = matches!(self.drag, Drag::SideW { .. }) || self.hover == Hit::SideResize;
        if let (Some(r), true) = (self.side_resize_rect(), hot_side) {
            let cx = (r.l + r.r) / 2.0;
            self.gfx.fill_round(R::new(cx - 1.5, r.t + 8.0, cx + 1.5, r.b - 8.0), 1.5, t.focus);
        }
        if self.view_menu.is_some() {
            self.paint_view_menu();
        }
        self.paint_toasts();
        self.paint_commands();
        self.paint_tip();
        self.paint_fav_card(&mut missing);
        self.gfx.end();
        self.request_thumbs(missing);
    }

    /// 알림: 칸 아래쪽 가운데 어두운 알약 (몇 초 뒤 사라진다).
    /// 검색 패널·보기 메뉴보다 위에 그려, 그 위에서 한 일(검색 결과 복사 등)의 결과도 보이게 한다
    pub(super) fn paint_toasts(&self) {
        let t = self.theme;
        let g = &self.gfx;
        for (p, _) in self.pane_rects() {
            let (Some(m), Some((tr, btn))) = (&self.panes[p].tab().message, self.toast_rects(p)) else { continue };
            let pushed = anim_progress(self.toast_at, TOAST_FADE_MS).is_some_and(|f| g.push_fade(f, 0.0, (1.0 - f) * 8.0));
            let (bg, fg) = if t.high_contrast {
                (t.sel, t.sel_text)
            } else if t.dark {
                (Color { r: 0.92, g: 0.93, b: 0.94, a: 1.0 }, Color { r: 0.12, g: 0.14, b: 0.16, a: 1.0 })
            } else {
                (Color { r: 0.12, g: 0.14, b: 0.16, a: 1.0 }, Color { r: 0.95, g: 0.95, b: 0.96, a: 1.0 })
            };
            let radius = tr.h() / 2.0;
            if !t.high_contrast {
                soft_shadow(g, tr, radius, 10.0, 0.18, t.dark);
            }
            g.fill_round(tr, radius, bg);
            if t.high_contrast {
                g.stroke_round(tr.inset(0.5, 0.5), radius, t.border, 1.0);
            }
            g.text(m, R::new(tr.l + 16.0, tr.t, btn.map_or(tr.r - 14.0, |b| b.l - 8.0), tr.b), Font::Normal, fg);
            if let Some(b) = btn {
                let hot = self.hover == Hit::ToastUndo(p);
                g.fill_round(b, b.h() / 2.0, alpha(fg, if hot { 0.26 } else { 0.14 }));
                let w = g.measure(TOAST_UNDO, Font::Normal);
                g.text(TOAST_UNDO, R::new(b.l + (b.w() - w) / 2.0, b.t, b.r, b.b), Font::Normal, fg);
            }
            if pushed {
                g.pop_fade();
            }
        }
    }

    /// 칸 영역 위 도구 막대: 뒤로·앞으로·위로, 폴더 이름, 작업 공간, 칸 나누기·보기 방식, 새 폴더, 미리 보기, `…`.
    /// 창 단추는 paint_caption
    pub(super) fn paint_workbar(&self) {
        let mut m = self.toolbar();
        // 폴더 이름은 작업 공간 단추 앞까지
        m.title.r = (self.workspace_button().l - 8.0).max(m.title.l);
        let p = self.active;
        let tab = self.panes[p].tab();
        let start = tab.loc == Location::Start;
        let title = self.toolbar_title();
        let hover = match self.hover {
            Hit::Back(_) => ToolbarHover::Back,
            Hit::Fwd(_) => ToolbarHover::Fwd,
            Hit::Up(_) => ToolbarHover::Up,
            Hit::LayoutPick => ToolbarHover::Layout,
            Hit::View(_) => ToolbarHover::View,
            Hit::NewFolder(_) => ToolbarHover::NewFolder,
            Hit::PreviewButton => ToolbarHover::Preview,
            Hit::More => ToolbarHover::More,
            _ => ToolbarHover::None,
        };
        let open = self.view_menu.map(|v| v.kind);
        paint_toolbar(
            &self.gfx,
            &self.theme,
            &ToolbarScene {
                geom: &m,
                title: &title,
                layout: self.layout,
                view: self.panes[p].view,
                can_back: !tab.back.is_empty(),
                can_fwd: !tab.fwd.is_empty(),
                can_up: can_go_up(&tab.loc),
                new_folder_ok: !start && tab.loc.dir().is_some(),
                view_ok: !start,
                preview_on: self.preview.is_some(),
                layout_open: open == Some(view_menu::MenuKind::Layout),
                view_open: open == Some(view_menu::MenuKind::View),
                hover,
            },
        );
        self.paint_workspace_button();
    }

    /// 단추에 잠시 올려 두면 보여 줄 (버튼 자리, 이름과 단축키)
    pub(super) fn tip(&self) -> Option<(R, String)> {
        if self.view_menu.is_some() || self.settings.is_some() || self.cmd.is_some() || !matches!(self.drag, Drag::None) {
            return None;
        }
        use crate::keys::Action;
        // "이름 (단축키)". 단축키를 비워 두었으면 이름만
        let named = |a: Action, name: &str| match self.keys.get(a) {
            Some(c) => format!("{name} ({})", c.label()),
            None => name.to_string(),
        };
        Some(match self.hover {
            Hit::LayoutPick => (self.toolbar().layout, "칸 나누기".to_string()),
            Hit::View(_) => (self.toolbar().view, "보기 방식".to_string()),
            Hit::NewFolder(_) => (self.toolbar().new_folder, named(Action::NewFolder, "새 폴더")),
            Hit::Back(_) => (self.toolbar().back, "뒤로 (Alt+←)".to_string()),
            Hit::Fwd(_) => (self.toolbar().fwd, "앞으로 (Alt+→)".to_string()),
            Hit::Up(_) => (self.toolbar().up, "위로 (Alt+↑)".to_string()),
            Hit::Side(s) => return self.side_tip(s),
            Hit::SearchButton => (
                self.search_button(),
                match &self.hotkey {
                    Some(hk) => format!("{} · 다른 앱에서는 {hk}", named(Action::Search, "검색")),
                    None => named(Action::Search, "검색"),
                },
            ),
            Hit::PreviewButton => (self.toolbar().preview, named(Action::Preview, "미리 보기 창")),
            Hit::SettingsButton => (self.settings_button(), named(Action::Settings, "설정")),
            Hit::Bar(p) if self.side_panel() == Some(p) => return None,
            Hit::Bar(Panel::Favorites) => (self.bar_rect(Panel::Favorites), named(Action::Favorites, "즐겨찾기")),
            Hit::Bar(p) => (self.bar_rect(p), p.title().to_string()),
            Hit::SidePin => (self.side_pin_rect()?, (if self.side_floating() { "고정해 두기" } else { "고정 풀기" }).to_string()),
            Hit::More => (self.toolbar().more, "도움말·설정".to_string()),
            Hit::PalClose => (self.pal_close_rect(), "검색 닫기".to_string()),
            Hit::Start(p, i) => return self.start_tip(p, i),
            Hit::PreviewPin | Hit::PreviewOpen => return self.preview_tip(),
            Hit::Workspace => (self.workspace_button(), "작업 공간 · 지금 화면을 저장하고 바꾸기".to_string()),
            _ => return None,
        })
    }

    /// 버튼 위에 마우스가 바뀌면 말풍선을 숨기고, 잠시 머무르면 보이게 한다
    pub(super) fn update_tip(&mut self) {
        // 말풍선이 떠 있던 채로 옆 단추로 옮기면 곧바로 보인다 (Windows 말풍선의 다시 보이기와 같다)
        let follow = std::mem::replace(&mut self.tip_shown, false);
        unsafe {
            if self.tip().is_some() {
                SetTimer(Some(self.hwnd), TIMER_TIP, if follow { 100 } else { 600 }, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_TIP);
            }
        }
    }

    pub(super) fn paint_tip(&self) {
        // 즐겨찾기 항목은 말풍선 대신 정보 카드 (sidebar::paint_fav_card)
        if matches!(self.hover, Hit::Side(Side::Fav(crate::favorites::Row::Item { .. }))) {
            return;
        }
        let Some((b, text)) = self.tip().filter(|_| self.tip_shown) else { return };
        let t = self.theme;
        let (label, key) = split_tip_key(&text);
        let key_w = key.map_or(0.0, |k| 8.0 + self.gfx.measure(k, Font::Small) + 10.0);
        let w = (self.gfx.measure(label, Font::Normal) + 20.0 + key_w).min((self.w - MARGIN * 2.0).max(40.0));
        // 막대 아이콘은 오른쪽에(아래쪽 설정이 창 밖으로 나가지 않게), 작업 줄 단추는 아래에.
        // 창 아래쪽 가까이(시작 화면 카드, 패널 마지막 줄)에서는 단추 위로 올린다
        let r = if b.r <= BAR_W {
            let cy = ((b.t + b.b) / 2.0).clamp(MARGIN + 14.0, (self.h - MARGIN - 14.0).max(MARGIN + 14.0));
            R::new(b.r + 8.0, cy - 14.0, b.r + 8.0 + w, cy + 14.0)
        } else {
            let l = ((b.l + b.r) / 2.0 - w / 2.0).clamp(MARGIN, (self.w - MARGIN - w).max(MARGIN));
            if b.b + 36.0 > self.h - MARGIN { R::new(l, b.t - 36.0, l + w, b.t - 8.0) } else { R::new(l, b.b + 8.0, l + w, b.b + 36.0) }
        };
        let (bg, fg) = match () {
            _ if t.high_contrast => (t.pane, t.text),
            _ if t.dark => (Color { r: 0.24, g: 0.25, b: 0.27, a: 1.0 }, t.text),
            _ => (t.text, Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }),
        };
        let fade = anim_progress(self.tip_at, TIP_FADE_MS);
        let pushed = fade.is_some_and(|f| {
            let off = (1.0 - f) * 4.0;
            if b.r <= BAR_W { self.gfx.push_fade(f, -off, 0.0) } else { self.gfx.push_fade(f, 0.0, -off) }
        });
        if !t.high_contrast {
            soft_shadow(&self.gfx, r, R_CTRL, 8.0, 0.14, t.dark);
        }
        self.gfx.fill_round(r, R_CTRL, bg);
        if t.high_contrast {
            self.gfx.stroke_round(r.inset(0.5, 0.5), R_CTRL, t.border, 1.0);
        }
        self.gfx.text(label, R::new(r.l + 10.0, r.t, r.r - 6.0 - key_w, r.b), Font::Normal, fg);
        if let Some(k) = key {
            let kr = R::new(r.r - 10.0 - (key_w - 8.0), r.t + 4.0, r.r - 10.0, r.b - 4.0);
            self.gfx.fill_round(kr, 4.0, alpha(fg, 0.16));
            self.gfx.text(k, R::new(kr.l + 5.0, kr.t, kr.r, kr.b), Font::Small, fg);
        }
        if pushed {
            self.gfx.pop_fade();
        }
    }

    pub(super) fn paint_pane(&mut self, p: usize, r: R, missing: &mut Vec<crate::thumbs::Request>) {
        let t = self.theme;
        let active = p == self.active;
        let multi = self.layout.count() > 1;
        let edit_mode = self.edit_mode;
        // 키보드가 사이드바 즐겨찾기에 있으면 칸의 포커스 테두리는 그리지 않는다 (포커스 표시는 한 곳에만)
        let keyboard_here = active && !self.fav_focus;
        let thumb_drag = matches!(self.drag, Drag::Thumb { pane, .. } if pane == p);
        // 끌고 있는 탭: (그 탭이 있던 칸, 탭), 놓을 곳 (칸, 위치)
        let (dragged_tab, tab_drop) = match self.drag {
            Drag::TabMove { pane, tab, x, y } => (Some((pane, tab)), self.tab_drop_target(x, y)),
            _ => (None, None),
        };
        // 끌어 놓을 대상: 다른 칸에서 끌어 온 탭이나 파일을 이 칸에, 또는 이 칸의 줄(폴더)에
        let tab_target = tab_drop.is_some_and(|(q, _)| q == p) && dragged_tab.is_some_and(|(from, _)| from != p);
        let drop_on_pane = tab_target || matches!(self.drop_hover, Some((dp, None)) if dp == p);
        let drop_row = match self.drop_hover {
            Some((dp, Some(vi))) if dp == p => Some(vi),
            _ => None,
        };
        // 경로 오른쪽 즐겨찾기 책갈피: 폴더일 때만 (켜짐 여부)
        let starred = self.panes[p].tab().loc.dir().map(|d| self.favs.find(&util::normalize(d)).is_some());
        // 네모 상자 고르기 (화면 좌표)
        let marquee = match self.drag {
            Drag::Marquee { pane, x0, y0, x, y } if pane == p => {
                let scroll = self.panes[p].tab().scroll;
                let (x0, y0) = if self.grid(p).is_some_and(|(_, g)| g.horiz) { (x0 - scroll, y0) } else { (x0, y0 - scroll) };
                Some(R::new(x0.min(x), y0.min(y), x0.max(x), y0.max(y)))
            }
            _ => None,
        };
        let g = geom(r);
        let (tabs, add) = self.chips(p, &g);
        let count_text = self.head_count_text(p);
        let count_rect = self.head_right_rect(p, &g);
        // 팝업이 열려 있으면 입력 상자(자식 창)는 화면 밖에 있으니 필터 글자를 직접 그린다
        let edit_in_chip = active && !(self.view_menu.is_some() || self.settings.is_some());
        let filter = (!self.panes[p].tab().filter.is_empty()).then(|| {
            let chip = filter_chip(&g);
            let (edit, count, clear) = self.filter_parts(p, chip);
            FilterChip { chip, edit, count, clear, count_text: self.filter_count_text(p), draw_text: !edit_in_chip }
        });
        // 시작 화면이면 머리글·목록 대신 그릴 카드와 줄
        let start = self.start_view(p);
        let folder_icon = if start.is_some() { self.type_icon("", true) } else { -1 };
        let start_focus = (keyboard_here && self.start_keys).then(|| self.panes[p].tab().cursor);
        let start_hover = match self.hover {
            Hit::Start(hp, i) if hp == p => Some(i),
            _ => None,
        };
        let hint = self.list_hint(p);
        // 늦게 읽히는 폴더의 막대 (0.3초 안에 끝나면 보이지 않는다, 애니메이션을 끄면 글자만)
        let loading = {
            let tab = self.panes[p].tab();
            let motion = sidebar::motion_on();
            tab.loading_since.filter(|_| !tab.offline).map(|s| s.elapsed().as_millis()).filter(|ms| *ms >= LOADING_SHOW_MS && motion).map(|ms| (ms % 1400) as f32 / 1400.0)
        };
        let net_button = self.net_button(p);
        let hover = pane_hover(self.hover, p);
        // 목록의 수정한 날짜를 짧게(오늘·어제) 쓸지와 그 기준 시각
        let (short_dates, now) = (self.look.dates != "full", crate::listing::now_filetime());
        let App { gfx, panes, types, thumb_failed, places, .. } = self;
        let pane = &panes[p];
        let scene = PaneScene {
            r,
            tabs: &pane.tabs,
            current: pane.active,
            sort: pane.sort,
            cols: pane.cols,
            view: pane.view,
            active,
            multi,
            keyboard_here,
            hover,
            chips: tabs,
            add,
            count_text,
            count_rect,
            count_hot: false,
            filter,
            starred,
            root: Some("내 PC"),
            address: edit_mode == EditMode::Address(p),
            address_text: None,
            renaming: edit_mode == EditMode::Rename(p),
            dragged_tab: dragged_tab.filter(|(q, _)| *q == p).map(|(_, i)| i),
            tab_drop: tab_drop.filter(|(q, _)| *q == p).map(|(_, at)| at),
            drop_on_pane,
            drop_row,
            marquee,
            scroll_drag: thumb_drag,
            hint,
            net_button,
            loading,
            short_dates,
            now,
        };
        paint_pane_head(gfx, &t, &scene);
        // 시작 화면: 머리글·목록 자리에 카드와 줄
        if let Some(sv) = &start {
            sv.paint(gfx, &t, places, folder_icon, start_hover, start_focus);
            paint_pane_border(gfx, &t, &scene);
            return;
        }
        paint_pane_list(gfx, &mut WinArt { types, thumb_failed, missing }, &t, &scene);
        paint_pane_border(gfx, &t, &scene);
    }

    /// 목록 자리의 안내 글: 불러오는 중(늦을 때만), 열 수 없음, 빈 폴더, 필터에 맞는 것 없음
    fn list_hint(&self, p: usize) -> Option<String> {
        let tab = self.panes[p].tab();
        match &tab.state {
            LoadState::Loading if tab.loading_since.is_some_and(|s| s.elapsed().as_millis() >= LOADING_SHOW_MS) => {
                let slow = tab.loading_since.is_some_and(|s| s.elapsed().as_secs() >= 3);
                let place = tab.loc.dir().unwrap_or_default();
                Some(if slow { format!("불러오는 중… 응답을 기다리고 있어요\n{place}\n다른 칸·탭은 그대로 쓸 수 있어요") } else { format!("불러오는 중…\n{place}") })
            }
            LoadState::Error(msg) => Some(match &tab.net_fix {
                Some(f) if f.ask => format!("로그인이 필요해요\n{}\n로그인하면 이 폴더를 열어요", f.share),
                Some(f) => format!("네트워크 드라이브의 연결이 끊겨 있어요\n{}\n다시 연결하거나 F5로 다시 시도해요", f.share),
                None => format!("폴더를 열 수 없어요\n{msg}\nF5를 누르면 다시 시도해요"),
            }),
            LoadState::Ready if tab.entries.is_empty() => Some("빈 폴더예요".to_string()),
            LoadState::Ready if tab.view.is_empty() && !tab.filter.is_empty() => Some(format!("필터 ‘{}’에 맞는 항목이 없어요", tab.filter)),
            LoadState::Ready if tab.view.is_empty() => Some("숨김 항목만 있어요".to_string()),
            _ => None,
        }
    }
}

/// 칸 p 안에서 마우스가 올라간 곳 (공통 칸 그리기에 넘긴다)
fn pane_hover(h: Hit, p: usize) -> PaneHover {
    match h {
        Hit::Tab(q, i) if q == p => PaneHover::Tab(i),
        Hit::TabClose(q, i) if q == p => PaneHover::TabClose(i),
        Hit::TabAdd(q) if q == p => PaneHover::TabAdd,
        Hit::Star(q) if q == p => PaneHover::Star,
        Hit::Crumb(q, i) if q == p => PaneHover::Crumb(i),
        Hit::CrumbEmpty(q) if q == p => PaneHover::CrumbEmpty,
        Hit::Header(q, key) if q == p => PaneHover::Header(key),
        Hit::Row(q, vi) if q == p => PaneHover::Row(vi),
        Hit::Thumb(q) | Hit::Track(q) if q == p => PaneHover::Scroll,
        Hit::FilterClear(q) if q == p => PaneHover::FilterClear,
        Hit::NetFix(q) if q == p => PaneHover::NetFix,
        _ => PaneHover::None,
    }
}

/// 칸 그리기의 Windows 갈고리: 시스템 아이콘·유형 이름, 썸네일 (아직 없는 썸네일은 missing에 모아 나중에 만든다)
struct WinArt<'a> {
    types: &'a mut HashMap<String, (String, i32)>,
    thumb_failed: &'a HashSet<String>,
    missing: &'a mut Vec<crate::thumbs::Request>,
}

impl EntryArt<Gfx> for WinArt<'_> {
    fn icon(&mut self, g: &mut Gfx, _: &Tab, e: &Entry, r: R) {
        let (_, type_icon) = type_info(self.types, e);
        g.icon(if e.icon >= 0 { e.icon } else { type_icon }, r);
    }

    fn thumbnail(&mut self, g: &mut Gfx, tab: &Tab, e: &Entry, bx: R, size: f32, opacity: f32) -> bool {
        let px = (size * g.dpi / 96.0).round() as u32;
        let Some(path) = tab.path_of(e) else { return false };
        let key = thumb_key(&path, e.modified, px);
        let ok = g.thumb(&key, bx, opacity);
        if !ok && !self.thumb_failed.contains(&key) {
            self.missing.push(crate::thumbs::Request { key, path, px });
        }
        ok
    }

    fn type_name(&mut self, _: &Tab, e: &Entry) -> String {
        type_info(self.types, e).0
    }
}
