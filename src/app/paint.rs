//! 그리기

use super::*;

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

    /// 칸 영역 위 작업 줄: 칸 나누기, 숨김 항목, 미리 보기. 창 단추는 paint_caption
    pub(super) fn paint_workbar(&self) {
        self.paint_workspace_button();
        let t = self.theme;
        let g = &self.gfx;
        // 판(well)과 같은 색이면 올려도 보이지 않으니 반투명으로 겹친다
        let hover_bg = sidebar::hover_fill(&t);
        // 칸 나누기: 한 덩어리 판 위에 지금 고른 것만 도드라지게
        let pill = self.layout_pill();
        g.fill_round(pill, pill.h() / 2.0, sidebar::seg_track(&t));
        for (i, r) in self.layout_buttons().iter().enumerate() {
            let layout = Layout::ALL[i];
            let current = layout == self.layout;
            if current {
                sidebar::seg_on(g, *r, r.h() / 2.0, &t);
            } else if self.hover == Hit::Layout(i) {
                g.fill_round(*r, r.h() / 2.0, hover_bg);
            }
            draw_layout_icon(g, *r, layout, if current || self.hover == Hit::Layout(i) { t.text } else { t.dim });
        }
        // 글자 없는 버튼: 올려 두면 이름과 단축키를 말풍선으로 보여 준다.
        // 쉴 때는 칸 나누기 그림과 같은 회색이라 아이콘마다 진하기가 달라 보이지 않는다
        let icon_button = |r: R, glyph: u16, on: bool, hovered: bool| {
            if on {
                sidebar::seg_on(g, r, R_CTRL, &t);
            } else if hovered {
                g.fill_round(r, R_CTRL, hover_bg);
            }
            g.glyph(glyph, r, false, if on || hovered { t.text } else { t.dim });
        };
        icon_button(self.hidden_button(), G_HIDDEN, self.show_hidden, self.hover == Hit::Hidden);
        icon_button(self.preview_button(), G_PREVIEW, self.preview.is_some(), self.hover == Hit::PreviewButton);
        icon_button(self.more_button(), G_MORE, false, self.hover == Hit::More);
    }

    /// 단추에 잠시 올려 두면 보여 줄 (버튼 자리, 이름과 단축키)
    pub(super) fn tip(&self) -> Option<(R, String)> {
        if self.view_menu.is_some() || self.settings.is_some() || self.cmd.is_some() || !matches!(self.drag, Drag::None) {
            return None;
        }
        use crate::keys::Action;
        const LAYOUTS: [Action; 5] = [Action::Layout1, Action::Layout2, Action::Layout3, Action::Layout4, Action::Layout5];
        // "이름 (단축키)". 단축키를 비워 두었으면 이름만
        let named = |a: Action, name: &str| match self.keys.get(a) {
            Some(c) => format!("{name} ({})", c.label()),
            None => name.to_string(),
        };
        Some(match self.hover {
            Hit::Layout(i) => (self.layout_buttons()[i], named(LAYOUTS[i], crate::keys::name(LAYOUTS[i]))),
            Hit::NewFolder(p) => (geom(self.pane_rect(p)?).new_folder, named(Action::NewFolder, "새 폴더")),
            Hit::Back(p) => (geom(self.pane_rect(p)?).back, "뒤로 (Alt+←)".to_string()),
            Hit::Fwd(p) => (geom(self.pane_rect(p)?).fwd, "앞으로 (Alt+→)".to_string()),
            Hit::Up(p) => (geom(self.pane_rect(p)?).up, "위로 (Alt+↑)".to_string()),
            Hit::Hidden => (self.hidden_button(), named(Action::Hidden, if self.show_hidden { "숨김 항목 감추기" } else { "숨김 항목 보기" })),
            Hit::Side(s) => return self.side_tip(s),
            Hit::SearchButton => (
                self.search_button(),
                match &self.hotkey {
                    Some(hk) => format!("{} · 다른 앱에서는 {hk}", named(Action::Search, "검색")),
                    None => named(Action::Search, "검색"),
                },
            ),
            Hit::PreviewButton => (self.preview_button(), named(Action::Preview, "미리 보기 창")),
            Hit::SettingsButton => (self.settings_button(), named(Action::Settings, "설정")),
            Hit::Bar(p) if self.side_panel() == Some(p) => return None,
            Hit::Bar(Panel::Favorites) => (self.bar_rect(Panel::Favorites), named(Action::Favorites, "즐겨찾기")),
            Hit::Bar(p) => (self.bar_rect(p), p.title().to_string()),
            Hit::SidePin => (self.side_pin_rect()?, (if self.side_floating() { "고정해 두기" } else { "고정 풀기" }).to_string()),
            Hit::More => (self.more_button(), "도움말·설정".to_string()),
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
        let hover = self.hover;
        let drop_hover = self.drop_hover;
        let edit_mode = self.edit_mode;
        // 키보드가 사이드바 즐겨찾기에 있으면 칸의 포커스 테두리는 그리지 않는다 (포커스 표시는 한 곳에만)
        let keyboard_here = active && !self.fav_focus;
        let thumb_drag = matches!(self.drag, Drag::Thumb { pane, .. } if pane == p);
        // 끌고 있는 탭: (그 탭이 있던 칸, 탭), 놓을 곳 (칸, 위치)
        let (dragged_tab, tab_drop) = match self.drag {
            Drag::TabMove { pane, tab, x, y } => (Some((pane, tab)), self.tab_drop_target(x, y)),
            _ => (None, None),
        };
        // 경로 오른쪽 즐겨찾기 별: 폴더일 때만 (켜짐 여부)
        let view_menu_open = self.view_menu.is_some_and(|m| m.pane == p);
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
        let (trs, add) = self.chips(p, &g);
        let count_text = self.head_count_text(p);
        let right = self.head_right_rect(p, &g);
        let fchip = filter_chip(&g);
        let (fedit, fcount, fclear) = self.filter_parts(p, fchip);
        let fcount_text = self.filter_count_text(p);
        // 팝업이 열려 있으면 입력 상자(자식 창)는 화면 밖에 있으니 필터 글자를 직접 그린다
        let edit_in_chip = active && !(self.view_menu.is_some() || self.settings.is_some());
        // 시작 화면이면 머리글·목록 대신 그릴 카드와 줄
        let start = self.start_view(p);
        let folder_icon = if start.is_some() { self.type_icon("", true) } else { -1 };
        let start_focus = (keyboard_here && self.start_keys).then(|| self.panes[p].tab().cursor);
        // 목록의 수정한 날짜를 짧게(오늘·어제) 쓸지와 그 기준 시각
        let (short_dates, now) = (self.look.dates != "full", crate::listing::now_filetime());
        let motion = sidebar::motion_on();
        let net_btn = self.net_button(p);
        let App { gfx, panes, types, thumb_failed, places, .. } = self;
        let pane = &panes[p];
        let tab = pane.tab();

        // 카드: 밝은 테마는 테두리 없이 옅은 그림자로 떠 있고, 지금 칸은 조금 더 높이 뜬다 (고대비는 테두리로)
        if active && multi && !t.high_contrast {
            soft_shadow(gfx, r, RADIUS, 12.0, 0.10, t.dark);
        } else if !t.high_contrast && !t.dark {
            soft_shadow(gfx, r, RADIUS, 6.0, 0.06, false);
        }
        gfx.fill_round(r, RADIUS, t.pane);
        // 지금 칸: 위쪽 가장자리 안쪽에 얇은 강조 선 (칸이 여럿일 때 어느 칸에서 일하는지 한눈에)
        if active && multi {
            gfx.fill_round(R::new(r.l + RADIUS, r.t + 1.0, r.r - RADIUS, r.t + 3.0), 1.0, if t.high_contrast { t.focus } else { t.accent });
        }

        // 칩 줄: 탭 칩. 지금 탭은 바탕이 있는 칩(지금 칸이면 옅은 강조 색), 나머지는 글자만이고 올리면 옅은 바탕.
        // 오른쪽 자리(필터 칩·개수) 앞까지만 그린다 (좁은 칸에서 겹치면 누르기도 오른쪽 자리가 먼저)
        gfx.clip(R::new(r.l + 1.0, g.head.t, (right.l - 4.0).max(r.l + 1.0), g.head.b));
        let hovered_tab = match hover {
            Hit::Tab(hp, ht) | Hit::TabClose(hp, ht) if hp == p => Some(ht),
            _ => None,
        };
        for (i, tr) in trs.iter().enumerate() {
            // 칸이 좁아 가려진 탭 (휠·Ctrl+Tab으로 닿는다)
            if tr.w() <= 0.0 {
                continue;
            }
            let current = i == pane.active;
            let hovered = hovered_tab == Some(i);
            if current {
                gfx.fill_round(*tr, tr.h() / 2.0, sidebar::well(&t));
                // 고대비에서는 바탕색만으로 구분이 어려우니 테두리를 더한다
                if t.high_contrast {
                    gfx.stroke_round(tr.inset(0.5, 0.5), tr.h() / 2.0, if active { t.focus } else { t.pane_border }, 1.0);
                }
            } else if hovered {
                gfx.fill_round(*tr, tr.h() / 2.0, t.hover);
            }
            let closable = pane.tabs.len() > 1 && (current || hovered);
            let label = R::new(tr.l + 12.0, tr.t, tr.r - if closable { 26.0 } else { 10.0 }, tr.b);
            let fg = t.text;
            let color = if dragged_tab == Some((p, i)) { alpha(fg, 0.5) } else { fg };
            gfx.text(&pane.tabs[i].loc.title(), label, Font::Normal, color);
            if closable {
                let cr = close_rect(*tr);
                if hover == Hit::TabClose(p, i) {
                    gfx.fill_round(cr, cr.h() / 2.0, alpha(t.text, 0.12));
                }
                gfx.glyph(G_CLOSE, cr, true, if hover == Hit::TabClose(p, i) { t.text } else { t.dim });
            }
        }
        // 새 탭(+): 평소엔 흐리게, 올리면 옅은 바탕
        if hover == Hit::TabAdd(p) {
            gfx.fill_round(add, add.h() / 2.0, t.hover);
        }
        gfx.glyph_font(G_ADD, add, Font::GlyphMid, if hover == Hit::TabAdd(p) { t.text } else { t.dim });
        // 끌어 놓을 자리
        if let Some((_, at)) = tab_drop.filter(|(q, _)| *q == p) {
            let x = trs.get(at).map(|tr| tr.l - 2.0).or(trs.last().map(|tr| tr.r + 2.0)).unwrap_or(g.head.l + 8.0);
            gfx.fill_round(R::new(x - 1.5, add.t, x + 1.5, add.b), 1.5, t.accent);
        }
        gfx.unclip();

        // 칩 줄 오른쪽: 필터 칩(글자를 치면), 아니면 항목 수·고른 개수
        if tab.filter.is_empty() {
            gfx.text(&count_text, right, Font::Small, t.faint);
        } else {
            gfx.fill_round(fchip, fchip.h() / 2.0, t.accent_soft);
            if t.high_contrast {
                gfx.stroke_round(fchip.inset(0.5, 0.5), fchip.h() / 2.0, t.border, 1.0);
            }
            gfx.glyph(G_FILTER, R::new(fchip.l + 8.0, fchip.t, fchip.l + 26.0, fchip.b), true, t.chip_text);
            // 지금 칸에서는 입력 상자(자식 창)가 이 자리에 글자를 그린다
            if !edit_in_chip {
                gfx.text(&tab.filter, fedit, Font::Normal, t.text);
            }
            let cw = gfx.measure(&fcount_text, Font::Small);
            gfx.text(&fcount_text, R::new(fcount.r - cw - 1.0, fcount.t, fcount.r, fcount.b), Font::Small, t.dim);
            if hover == Hit::FilterClear(p) {
                gfx.fill_round(fclear, fclear.h() / 2.0, alpha(t.text, 0.12));
            }
            gfx.glyph(G_CLOSE, fclear, true, t.dim);
        }

        // 경로 줄: 탐색 단추
        let off = if t.high_contrast { t.faint } else { alpha(t.dim, 0.45) };
        let nav_btn = |gfx: &Gfx, rr: R, glyph: u16, font: Font, enabled: bool, hovered: bool| {
            if rr.w() <= 0.0 {
                return;
            }
            if hovered && enabled {
                gfx.fill_round(rr, R_CTRL, t.hover);
            }
            gfx.glyph_font(glyph, rr, font, if !enabled { off } else if hovered { t.text } else { t.dim });
        };
        // 이동 화살표는 가운데 크기(12px), 새 폴더·별·보기는 도구 아이콘 크기(16px)
        nav_btn(gfx, g.back, G_BACK, Font::GlyphMid, !tab.back.is_empty(), hover == Hit::Back(p));
        nav_btn(gfx, g.fwd, G_FWD, Font::GlyphMid, !tab.fwd.is_empty(), hover == Hit::Fwd(p));
        nav_btn(gfx, g.up, G_UP, Font::GlyphMid, matches!(tab.loc, Location::Dir(_)), hover == Hit::Up(p));
        // 시작 화면에는 새 폴더·보기 단추가 없다. 좁은 칸의 경로 입력 상자가 그 자리를 덮으면 그리지 않는다
        if start.is_none() && !(edit_mode == EditMode::Address(p) && g.crumb.w() < ADDR_MIN_W) {
            nav_btn(gfx, g.new_folder, G_NEW_FOLDER, Font::Glyph, tab.loc.dir().is_some(), hover == Hit::NewFolder(p));
            // 보기 방식: 지금 보기 그림과 아래 화살표 (누르면 보기 메뉴)
            if hover == Hit::View(p) || view_menu_open {
                gfx.fill_round(g.view, R_CTRL, t.hover);
            }
            let view_glyph = if matches!(pane.view, ViewMode::Huge | ViewMode::Large | ViewMode::Medium) { G_GRID } else { G_LIST };
            let view_hot = hover == Hit::View(p) || view_menu_open;
            gfx.glyph(view_glyph, R::new(g.view.l + 2.0, g.view.t, g.view.l + 26.0, g.view.b), false, if view_hot { t.text } else { t.dim });
            gfx.glyph(G_CHEVRON_DOWN, R::new(g.view.r - 16.0, g.view.t + 1.0, g.view.r - 4.0, g.view.b + 1.0), true, t.faint);
        }

        // 경로: 상자 없이 글자만. 경로 위에 올리면 옅은 바탕(빈 곳을 누르면 경로 입력), 조각에 올리면 그 조각이 한 단계 진하게
        if edit_mode == EditMode::Address(p) {
            let ar = addr_rect(&g);
            gfx.fill_round(ar, R_CTRL, t.field);
            gfx.stroke_round(ar.inset(0.75, 0.75), R_CTRL, t.accent, 1.5);
        } else {
            let over_path = matches!(hover, Hit::CrumbEmpty(hp) | Hit::Crumb(hp, _) if hp == p);
            if over_path {
                gfx.fill_round(g.crumb, R_CTRL, t.hover);
            }
            let cs = crumbs(gfx, g.crumbs, &tab.loc, Some("내 PC"));
            let n = cs.len();
            // 별 단추: 이 폴더가 즐겨찾기에 있으면 채운 별(강조 색), 없으면 빈 별
            if let Some(on) = starred.filter(|_| g.star.w() > 0.0) {
                let hot = hover == Hit::Star(p);
                if hot {
                    gfx.fill_round(g.star, R_CTRL, t.hover);
                }
                gfx.glyph(if on { G_STAR_FILL } else { G_STAR }, g.star, false, if on || hot { t.text } else { t.dim });
            }
            gfx.clip(g.crumbs);
            for (i, c) in cs.iter().enumerate() {
                let last = i + 1 == n;
                if hover == Hit::Crumb(p, i) {
                    gfx.fill_round(R::new(c.x0, g.crumb.t + 2.0, c.x1, g.crumb.b - 2.0), R_ROW, t.sel_inactive);
                }
                if c.label == "…" && !last {
                    gfx.glyph(G_MORE, R::new(c.x0, g.crumb.t, c.x1, g.crumb.b), true, t.dim);
                } else {
                    gfx.text(
                        &c.label,
                        R::new(c.x0 + CRUMB_PAD, g.crumb.t, c.x1 - CRUMB_PAD / 2.0, g.crumb.b),
                        Font::Normal,
                        if last { t.text } else { t.dim },
                    );
                }
                if !last {
                    gfx.glyph(G_CHEVRON, R::new(c.x1, g.crumb.t, c.x1 + CHEV_W, g.crumb.b), true, t.faint);
                }
            }
            gfx.unclip();
        }

        // 테두리: 모든 칸이 같은 옅은 선. 끌어 놓을 대상(파일, 다른 칸에서 끌어 온 탭)일 때만 강조 색으로.
        // 고대비에서는 모든 칸을 2px로 두고 활성 칸만 포커스 색으로 바꿔 두께는 그대로 둔다
        let tab_target = tab_drop.is_some_and(|(q, _)| q == p) && dragged_tab.is_some_and(|(from, _)| from != p);
        let dropping = tab_target || matches!(drop_hover, Some((dp, None)) if dp == p);
        let border_w = if t.high_contrast || dropping { 2.0 } else if t.dark { 1.0 } else { 0.0 };
        let border_c = match () {
            _ if dropping => t.accent,
            _ if t.high_contrast && active && multi => t.focus,
            _ => t.pane_border,
        };

        // 시작 화면: 머리글·목록 자리에 카드와 줄
        if let Some(sv) = &start {
            let hovered = match hover {
                Hit::Start(hp, i) if hp == p => Some(i),
                _ => None,
            };
            sv.paint(gfx, &t, places, folder_icon, hovered, start_focus);
            if border_w > 0.0 {
                gfx.stroke_round(r.inset(border_w / 2.0, border_w / 2.0), RADIUS, border_c, border_w);
            }
            return;
        }

        // 열 머리글
        let cols = columns(g.list, pane.cols);
        let header = |gfx: &Gfx, (l, rr): (f32, f32), label: &str, key: SortKey, right: bool| {
            let sorted = pane.sort.key == key;
            let color = if sorted || hover == Hit::Header(p, key) { t.text } else { t.dim };
            let tr = R::new(l + 8.0, g.header.t, rr - 8.0, g.header.b);
            gfx.text(label, tr, if right { Font::Right } else { Font::Normal }, color);
            if sorted {
                let glyph = if pane.sort.desc { G_SORT_DOWN } else { G_SORT_UP };
                let gx = if right { l } else { (tr.l + gfx.measure(label, Font::Normal) + 4.0).min(rr - 18.0) };
                gfx.glyph(glyph, R::new(gx, g.header.t, gx + 16.0, g.header.b), true, t.dim);
            }
        };
        if pane.view == ViewMode::Details {
            header(gfx, (cols.name.0 - 4.0, cols.name.1), "이름", SortKey::Name, false);
            if let Some(c) = cols.modified {
                header(gfx, c, "수정한 날짜", SortKey::Modified, false);
            }
            if let Some(c) = cols.kind {
                header(gfx, c, "유형", SortKey::Type, false);
            }
            if let Some(c) = cols.size {
                header(gfx, c, "크기", SortKey::Size, true);
            }
            gfx.fill(R::new(g.list.l + 8.0, g.header.b - 1.0, g.list.r - SCROLL_W - 6.0, g.header.b), t.border);
        } else {
            // 아이콘·목록 보기: 열 대신 정렬 버튼. 지금 기준은 바탕이 있는 버튼으로 보인다
            for (key, label, cr) in sort_chips(gfx, g.header, pane.sort.key) {
                let sorted = pane.sort.key == key;
                let hot = hover == Hit::Header(p, key);
                if sorted {
                    gfx.fill_round(cr, cr.h() / 2.0, sidebar::well(&t));
                    if t.high_contrast {
                        gfx.stroke_round(cr.inset(0.5, 0.5), cr.h() / 2.0, t.border, 1.0);
                    }
                } else if hot {
                    gfx.fill_round(cr, cr.h() / 2.0, t.hover);
                }
                let tr = R::new(cr.l + 12.0, cr.t, cr.r - if sorted { 28.0 } else { 12.0 }, cr.b);
                gfx.text(label, tr, Font::Normal, if sorted || hot { t.text } else { t.dim });
                if sorted {
                    let glyph = if pane.sort.desc { G_SORT_DOWN } else { G_SORT_UP };
                    gfx.glyph(glyph, R::new(cr.r - 26.0, cr.t, cr.r - 10.0, cr.b), true, t.dim);
                }
            }
        }

        // 행·칸: 보이는 부분만 그린다
        gfx.clip(g.list);
        let grid = Grid::of(g.list, pane.view);
        let thumb_size = if let CellKind::Tile(s) = grid.cell { s } else { THUMB };
        let thumb_px = (thumb_size * gfx.dpi / 96.0).round() as u32;
        // 여러 개를 골랐으면 그중 포커스가 어느 것인지도 보인다
        let many_selected = tab.selection_stats().0 > 1;
        for vi in grid.visible(g.list, tab.scroll, tab.view.len()) {
            let e = &tab.entries[tab.view[vi] as usize];
            let it = grid.item(vi, tab.scroll);
            // 선택(바탕)과 키보드 포커스(테두리)는 따로 보인다. 포커스는 활성 칸에만, 선택한 하나와 겹치면 바탕만으로
            let focused = keyboard_here && vi == tab.cursor && (!e.selected || many_selected);
            let fg = if e.selected && active { t.sel_text } else { t.text };
            if grid.cell == CellKind::Icon {
                // 작은 아이콘·목록: 칸 하나에 작은 아이콘과 이름
                let cell = R::new(it.l + 1.0, it.t + 1.0, it.r - 3.0, it.b - 1.0);
                if e.selected {
                    gfx.fill_round(cell, R_ROW, if active { t.sel } else { t.sel_inactive });
                } else if hover == Hit::Row(p, vi) {
                    gfx.fill_round(cell, R_ROW, t.hover);
                }
                if drop_hover == Some((p, Some(vi))) {
                    gfx.stroke_round(cell.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                } else if focused {
                    gfx.stroke_round(cell.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
                }
                let (_, type_icon) = type_info(types, e);
                let iy = it.t + (ROW_H - 16.0) / 2.0;
                gfx.icon(if e.icon >= 0 { e.icon } else { type_icon }, R::new(it.l + 4.0, iy, it.l + 20.0, iy + 16.0));
                if edit_mode == EditMode::Rename(p) && vi == tab.cursor {
                    let er = R::new(it.l + 24.0, it.t + 2.0, it.r - 6.0, it.b - 2.0);
                    gfx.fill_round(er, R_ROW, t.pane);
                    gfx.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                } else {
                    let fg = if e.is_hidden() && !(e.selected && active) { t.faint } else { fg };
                    gfx.text_w(e.name(), R::new(it.l + 28.0, it.t, it.r - 8.0, it.b), Font::Normal, fg);
                }
                continue;
            }
            if let CellKind::Tile(thumb) = grid.cell {
                let tile = R::new(it.l + 4.0, it.t + 4.0, it.r - 4.0, it.b - 4.0);
                if e.selected {
                    gfx.fill_round(tile, R_ROW, if active { t.sel } else { t.sel_inactive });
                } else if hover == Hit::Row(p, vi) {
                    gfx.fill_round(tile, R_ROW, t.hover);
                }
                if drop_hover == Some((p, Some(vi))) {
                    gfx.stroke_round(tile.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                } else if focused {
                    gfx.stroke_round(tile.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
                }
                let cx = (it.l + it.r) / 2.0;
                let bx = R::new(cx - thumb / 2.0, it.t + 8.0, cx + thumb / 2.0, it.t + 8.0 + thumb);
                let opacity = if e.is_hidden() { 0.55 } else { 1.0 };
                let drawn = match tab.path_of(e) {
                    Some(path) => {
                        let key = thumb_key(&path, e.modified, thumb_px);
                        let ok = gfx.thumb(&key, bx, opacity);
                        if !ok && !thumb_failed.contains(&key) {
                            missing.push(crate::thumbs::Request { key, path, px: thumb_px });
                        }
                        ok
                    }
                    None => false,
                };
                if !drawn {
                    // 썸네일이 오기 전까지는 작은 아이콘
                    let (_, type_icon) = type_info(types, e);
                    let my = (bx.t + bx.b) / 2.0;
                    gfx.icon(if e.icon >= 0 { e.icon } else { type_icon }, R::new(cx - 16.0, my - 16.0, cx + 16.0, my + 16.0));
                }
                let cap = R::new(tile.l + 4.0, bx.b + 6.0, tile.r - 4.0, tile.b - 2.0);
                if edit_mode == EditMode::Rename(p) && vi == tab.cursor {
                    let er = R::new(it.l + 4.0, bx.b + 6.0, it.r - 8.0, bx.b + 30.0);
                    gfx.fill_round(er, R_ROW, t.pane);
                    gfx.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                } else {
                    gfx.text_w(e.name(), cap, Font::Caption, if e.is_hidden() { t.faint } else { fg });
                }
                continue;
            }
            let y = it.t;
            let row = R::new(g.list.l + 8.0, y + 1.0, g.list.r - SCROLL_W - 6.0, y + ROW_H - 1.0);
            if e.selected {
                gfx.fill_round(row, R_ROW, if active { t.sel } else { t.sel_inactive });
            } else if hover == Hit::Row(p, vi) {
                gfx.fill_round(row, R_ROW, t.hover);
            }
            if drop_hover == Some((p, Some(vi))) {
                gfx.stroke_round(row.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
            } else if focused {
                gfx.stroke_round(row.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), R_ROW, t.focus, FOCUS_W);
            }
            // 고대비에서는 선택 바탕(Highlight) 위 글자를 모두 HighlightText로
            let dim = if e.selected && active && t.high_contrast { t.sel_text } else { t.dim };
            let (type_name, type_icon) = type_info(types, e);
            gfx.icon(if e.icon >= 0 { e.icon } else { type_icon }, R::new(cols.name.0 + 4.0, y + (ROW_H - 16.0) / 2.0, cols.name.0 + 20.0, y + (ROW_H + 16.0) / 2.0));
            let fg = if e.is_hidden() && !(e.selected && active) { t.faint } else { fg };
            gfx.text_w(e.name(), R::new(cols.name.0 + 28.0, y, cols.name.1 - 8.0, y + ROW_H), Font::Normal, fg);
            if let Some((l, rr)) = cols.modified {
                let s = match e.kind {
                    Kind::Drive if e.size > 0 => format!("{} 사용 가능", format_size(e.free)),
                    Kind::File | Kind::Dir if short_dates => crate::listing::format_time_short(e.modified, now),
                    Kind::File | Kind::Dir => format_time(e.modified),
                    _ => String::new(),
                };
                gfx.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Normal, dim);
            }
            if let Some((l, rr)) = cols.kind {
                let s = if e.kind == Kind::Drive { drive_type_label(e.drive_type) } else { type_name.as_str() };
                gfx.text(s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Normal, dim);
            }
            if let Some((l, rr)) = cols.size {
                let s = match e.kind {
                    Kind::File => format_size(e.size),
                    Kind::Drive if e.size > 0 => format_size(e.size),
                    _ => String::new(),
                };
                gfx.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Right, dim);
            }
            if edit_mode == EditMode::Rename(p) && vi == tab.cursor {
                let er = R::new(cols.name.0 + 24.0, y + 2.0, cols.name.1 - 4.0, y + ROW_H - 2.0);
                gfx.fill_round(er, R_ROW, t.pane);
                gfx.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
            }
        }
        gfx.unclip();

        let hint = match &tab.state {
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
        };
        if let Some(h) = hint {
            gfx.text(&h, R::new(g.list.l + 24.0, g.list.t + 24.0, g.list.r - 24.0, g.list.t + 110.0), Font::Hint, t.faint);
        }
        // 로그인·다시 연결: 이 칸의 할 일 하나뿐이라 강조 색 단추
        if let Some((b, label)) = net_btn {
            let k = if hover == Hit::NetFix(p) { 0.88 } else { 1.0 };
            gfx.fill_round(b, R_CTRL, Color { r: t.accent.r * k, g: t.accent.g * k, b: t.accent.b * k, a: 1.0 });
            let lw = gfx.measure(label, Font::Normal);
            gfx.text(label, R::new(b.l + (b.w() - lw) / 2.0, b.t, b.r, b.b), Font::Normal, if t.dark { t.window } else { Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 } });
        }
        // 연결이 끊겨 기억한 목록을 보이는 중이면 흐리게
        if tab.offline {
            gfx.fill(g.list, alpha(t.pane, 0.45));
        }

        if let Some((_, thumb)) = grid.bar(g.list, tab.view.len(), tab.scroll) {
            let hot = thumb_drag || matches!(hover, Hit::Thumb(hp) | Hit::Track(hp) if hp == p);
            let thumb = if hot { thumb } else { thumb.inset(1.0, 0.0) };
            gfx.fill_round(thumb, thumb.w() / 2.0, if hot { t.dim } else { t.thumb });
        }

        if let Some(m) = marquee {
            gfx.clip(g.list);
            gfx.fill(m, alpha(t.accent, 0.15));
            gfx.stroke_round(m.inset(0.5, 0.5), 1.0, t.accent, 1.0);
            gfx.unclip();
        }

        // 늦게 읽히는 폴더: 머리글 아래를 오가는 가는 막대 (0.3초 안에 끝나면 보이지 않는다, 애니메이션을 끄면 글자만)
        if let Some(ms) = tab.loading_since.filter(|_| !tab.offline).map(|s| s.elapsed().as_millis()).filter(|ms| *ms >= LOADING_SHOW_MS && motion) {
            let track = R::new(g.list.l + 8.0, g.header.b - 2.0, g.list.r - SCROLL_W - 6.0, g.header.b);
            let seg = track.w() * 0.3;
            let x = track.l - seg + (track.w() + seg) * ((ms % 1400) as f32 / 1400.0);
            gfx.clip(track);
            gfx.fill_round(R::new(x, track.t, x + seg, track.b), 1.0, t.accent);
            gfx.unclip();
        }

        if border_w > 0.0 {
            gfx.stroke_round(r.inset(border_w / 2.0, border_w / 2.0), RADIUS, border_c, border_w);
        }
    }
}
