//! 창 메시지 처리: handle, 마우스, 키

use super::*;

impl App {
    // ---------- 메시지 처리 ----------

    pub fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let hwnd = self.hwnd;
        unsafe {
            match msg {
                WM_PAINT => {
                    let mut ps = PAINTSTRUCT::default();
                    BeginPaint(hwnd, &mut ps);
                    self.paint();
                    let _ = EndPaint(hwnd, &ps);
                }
                WM_ERASEBKGND => return Some(LRESULT(1)),
                WM_SIZE => self.on_size(loword(lp.0 as usize) as u32, hiword(lp.0 as usize) as u32),
                WM_MOUSEMOVE => {
                    let (x, y) = self.dip(lp);
                    self.on_mouse_move(x, y);
                }
                WM_MOUSELEAVE => {
                    self.tracking = false;
                    self.side_mouse_left();
                    if self.hover != Hit::None {
                        self.hover = Hit::None;
                        self.update_tip();
                        self.invalidate();
                    }
                }
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                    let (x, y) = self.dip(lp);
                    self.on_lbutton_down(x, y, msg == WM_LBUTTONDBLCLK);
                }
                WM_LBUTTONUP => self.on_lbutton_up(),
                WM_RBUTTONDOWN => {
                    let (x, y) = self.dip(lp);
                    self.on_rbutton_down(x, y);
                }
                WM_RBUTTONUP => {
                    let (x, y) = self.dip(lp);
                    if !self.on_rbutton_up(x, y) {
                        return None;
                    }
                }
                WM_MBUTTONUP => {
                    let (x, y) = self.dip(lp);
                    self.on_mbutton_up(x, y);
                }
                WM_XBUTTONUP => {
                    let (x, y) = self.dip(lp);
                    let p = self.hit(x, y).pane().unwrap_or(self.active);
                    match hiword(wp.0) {
                        XBUTTON1 => self.go_back(p),
                        XBUTTON2 => self.go_forward(p),
                        _ => {}
                    }
                    return Some(LRESULT(1));
                }
                WM_MOUSEWHEEL => self.on_wheel(wp, lp),
                WM_CAPTURECHANGED => {
                    if !matches!(self.drag, Drag::None) {
                        self.drag = Drag::None;
                        self.invalidate();
                    }
                }
                WM_SETCURSOR => {
                    if loword(lp.0 as usize) as u32 == HTCLIENT && self.on_set_cursor() {
                        return Some(LRESULT(1));
                    }
                    return None;
                }
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    self.key_repeat = lp.0 & (1 << 30) != 0;
                    if !self.on_key(VIRTUAL_KEY(wp.0 as u16)) {
                        return None;
                    }
                }
                WM_SETFOCUS => self.focus(),
                WM_ACTIVATE => {
                    // 다른 창으로 전환하면 보기 메뉴를 닫는다 (WA_INACTIVE).
                    // 검색 패널은 다른 앱에서 무언가 복사해 오는 동안에도 적어 둔 검색어가 그대로 있도록 열어 둔다
                    self.window_active = loword(wp.0) != 0;
                    self.invalidate();
                    if loword(wp.0) == 0 {
                        self.close_view_menu();
                        self.close_peek();
                    } else {
                        // 관리자 승인(검색 서비스 설치) 등을 마치고 돌아오면 설정 창의 상태를 다시 읽는다.
                        // 다른 곳에서 파일을 옮겼을 수 있으니 드라이브 남은 용량도 다시 읽는다
                        self.refresh_settings();
                        self.refresh_places();
                        // 다른 앱에서 고정한 파일의 이름을 바꾸거나 옮겼을 수 있다
                        self.preview_pin_check();
                        // 비활성인 동안 남겨 둔 알림은 이제부터 잠시 뒤 지운다
                        if self.panes.iter().any(|pn| pn.tabs.iter().any(|t| t.message.is_some())) {
                            SetTimer(Some(hwnd), TIMER_FLASH, FLASH_MS, None);
                        }
                    }
                    return None;
                }
                WM_COMMAND => {
                    if hiword(wp.0) as u32 == EN_CHANGE {
                        match loword(wp.0) as i32 {
                            ID_FILTER => self.sync_filter_from_edit(),
                            ID_SEARCH => self.pal_query_changed(),
                            ID_CMD => self.cmd_rebuild(),
                            _ => {}
                        }
                    }
                }
                WM_TIMER => {
                    let id = wp.0;
                    if id == TIMER_SEARCH_READY {
                        self.pal_ready_timer();
                    } else if id == TIMER_SEARCH || id == palette::TIMER_SEARCH_AGAIN {
                        let _ = KillTimer(Some(hwnd), id);
                        self.pal_run_query();
                    } else if id == TIMER_PREVIEW {
                        self.start_preview();
                    } else if id == TIMER_PROBE {
                        self.on_probe_timer();
                    } else if id == TIMER_REDRAW {
                        let _ = KillTimer(Some(hwnd), id);
                        self.invalidate();
                    } else if id == TIMER_LOADING {
                        // 읽는 중인 탭이 남아 있는 동안만 다시 그려 "불러오는 중…"을 띄운다
                        let busy = (0..self.layout.count()).any(|p| self.panes[p].tab().loading_since.is_some());
                        if !busy {
                            let _ = KillTimer(Some(hwnd), id);
                        } else if sidebar::motion_on() {
                            // 로딩 막대가 움직이도록 자주 다시 그린다
                            SetTimer(Some(hwnd), id, 32, None);
                        }
                        self.refresh_cursor();
                        self.invalidate();
                    } else if id == TIMER_FAV_FLASH {
                        let _ = KillTimer(Some(hwnd), id);
                        if self.fav_flash.take().is_some() {
                            self.invalidate();
                        }
                    } else if id == TIMER_FLASH {
                        let _ = KillTimer(Some(hwnd), id);
                        // 다른 앱을 쓰거나 트레이에 숨어 있는 동안 끝난 작업의 알림은 돌아왔을 때 보이게 남겨 둔다
                        if self.window_active && IsWindowVisible(hwnd).as_bool() {
                            self.clear_flash();
                        }
                    } else if id == TIMER_TIP {
                        let _ = KillTimer(Some(hwnd), id);
                        self.start_anim(|a| &mut a.tip_at);
                        self.tip_shown = true;
                        self.invalidate();
                    } else if id == TIMER_OFFLINE {
                        let mut any = false;
                        for p in 0..self.layout.count() {
                            let tab = self.panes[p].tab();
                            if tab.offline {
                                any = true;
                                if tab.loading_since.is_none() {
                                    self.load(p, true);
                                }
                            }
                        }
                        if !any {
                            let _ = KillTimer(Some(hwnd), id);
                        }
                    } else if id == TIMER_WHEEL {
                        self.on_wheel_timer();
                    } else if id == TIMER_ANIM {
                        if !self.anims_running() {
                            let _ = KillTimer(Some(hwnd), id);
                        }
                        self.invalidate();
                    } else if id == TIMER_BAR_HOVER {
                        self.on_bar_hover_timer();
                    } else if id == TIMER_SIDE_CLOSE {
                        self.on_side_close_timer();
                    } else if id == TIMER_SIDE_ANIM {
                        self.on_side_anim_timer();
                    } else if id == TIMER_DATES {
                        if self.look.dates != "full" && IsWindowVisible(hwnd).as_bool() {
                            self.invalidate();
                        }
                    } else if (TIMER_REFRESH..TIMER_REFRESH + 4).contains(&id) {
                        let _ = KillTimer(Some(hwnd), id);
                        let p = id - TIMER_REFRESH;
                        if p < self.layout.count() {
                            // 트레이에 숨어 있는 동안에는 읽지 않고, 다시 보일 때 읽는다
                            if IsWindowVisible(hwnd).as_bool() {
                                self.load(p, true);
                                // 파일 작업·이름 바꾸기로 바뀐 것을 검색 결과와 고정한 미리 보기에도 반영한다
                                self.pal_requery();
                                self.preview_pin_check();
                            } else {
                                self.stale[p] = true;
                            }
                        }
                    }
                }
                // 고대비 모드를 켜고 끄면 시스템 색이 바뀐다
                WM_SYSCOLORCHANGE => {
                    self.theme = crate::theme::resolve(&self.look.theme, &self.look.accent);
                    self.apply_window_theme();
                    return None;
                }
                WM_SETTINGCHANGE => {
                    if wp.0 == SPI_SETWHEELSCROLLLINES.0 as usize {
                        self.wheel_lines = wheel_lines();
                    }
                    let colors = wp.0 == SPI_SETHIGHCONTRAST.0 as usize;
                    if colors || lp.0 != 0 && PCWSTR(lp.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                        self.theme = crate::theme::resolve(&self.look.theme, &self.look.accent);
                        self.apply_window_theme();
                        let tip = self.tray_tip();
                        if let Some(t) = &mut self.tray {
                            t.update(false, &tip);
                        }
                    }
                    return None;
                }
                // 시계를 바꾸면 짧은 날짜(오늘·어제)를 다시 그린다
                WM_TIMECHANGE => self.invalidate(),
                // Windows를 끄거나 로그아웃할 때: 트레이에 숨어 있어도 지금 화면을 저장한다
                WM_QUERYENDSESSION => {
                    self.save_session();
                    return Some(LRESULT(1));
                }
                WM_CLOSE => {
                    self.save_session();
                    let to_tray = self.tray.is_some();
                    crate::diag::log(|| format!("close: {}", if to_tray { "hide to tray" } else { "destroy window" }));
                    self.pending.push(if to_tray { Deferred::HideToTray } else { Deferred::Close });
                }
                // 즐겨찾기에 키보드가 있을 때의 글자: 그 글자로 시작하는 줄로
                WM_CHAR if self.fav_focus => {
                    self.fav_type_char(wp.0 as u16);
                }
                // USB·네트워크 드라이브를 꽂거나 빼면 사이드바 드라이브 목록을 다시 읽는다
                // (DBT_DEVICEARRIVAL 0x8000, DBT_DEVICEREMOVECOMPLETE 0x8004)
                WM_DEVICECHANGE => {
                    if matches!(wp.0, 0x8000 | 0x8004) {
                        self.refresh_places();
                    }
                    return None;
                }
                crate::WM_APP_PLACES => self.on_places(*Box::from_raw(lp.0 as *mut sidebar::Places)),
                WM_APP_LISTED => self.on_listed(*Box::from_raw(lp.0 as *mut Listed)),
                crate::WM_APP_PROBED => self.on_probed(*Box::from_raw(lp.0 as *mut Probed)),
                crate::WM_APP_PALMETA => self.on_pal_meta(*Box::from_raw(lp.0 as *mut palette::PalMeta)),
                WM_APP_ICONS => self.on_icons(*Box::from_raw(lp.0 as *mut IconsReady)),
                WM_APP_SEARCH => self.on_search_done(*Box::from_raw(lp.0 as *mut SearchDone)),
                crate::WM_APP_FILEOP => self.on_file_op(*Box::from_raw(lp.0 as *mut shell::OpReport)),
                crate::WM_APP_THUMB => self.on_thumb(*Box::from_raw(lp.0 as *mut crate::thumbs::Image)),
                crate::WM_APP_PREVIEW => self.on_preview_msg(*Box::from_raw(lp.0 as *mut PreviewMsg)),
                m if (WM_APP_NOTIFY..WM_APP_NOTIFY + 4).contains(&m) => {
                    shell::ack_notify(wp, lp);
                    // 연속된 변경(복사 중 등)을 한 번의 새로 고침으로 묶는다
                    SetTimer(Some(hwnd), TIMER_REFRESH + (m - WM_APP_NOTIFY) as usize, 250, None);
                }
                _ => return None,
            }
        }
        Some(LRESULT(0))
    }

    pub(super) fn on_mouse_move(&mut self, x: f32, y: f32) {
        match self.drag {
            Drag::SplitX => {
                let a = self.area();
                self.split.0 = ((x - a.l) / (a.w() - GAP)).clamp(0.15, 0.85);
                self.after_resize_drag();
            }
            Drag::SplitY => {
                let a = self.area();
                self.split.1 = ((y - a.t) / (a.h() - GAP)).clamp(0.15, 0.85);
                self.after_resize_drag();
            }
            Drag::Col { pane, col, start_x, start_w } => {
                self.panes[pane].cols[col] = (start_w + (start_x - x)).clamp(60.0, 400.0);
                self.invalidate();
            }
            Drag::Thumb { pane, grab } => {
                let Some((list, grid)) = self.grid(pane) else { return };
                let tab = self.panes[pane].tab();
                let content = grid.content(tab.view.len());
                if let Some((track, thumb)) = grid.bar(list, tab.view.len(), tab.scroll) {
                    let max = content - grid.page(list);
                    let (_, thumb_len, track_start, track_len) = grid.bar_span(track, thumb);
                    let ratio = (grid.along(x, y) - grab - track_start) / (track_len - thumb_len).max(1.0);
                    self.panes[pane].tab_mut().scroll = ratio * max;
                    self.scroll_by(pane, 0.0);
                }
            }
            Drag::Press { pane, x: x0, y: y0, .. } => {
                if (x - x0).abs() > 4.0 || (y - y0).abs() > 4.0 {
                    let paths = self.panes[pane].tab().selected_paths();
                    self.drag = Drag::None;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    crate::diag::log(|| format!("drag: start with {} item(s)", paths.len()));
                    if !paths.is_empty() {
                        self.pending.push(Deferred::DragOut(paths));
                    }
                }
            }
            Drag::FavPress { .. } | Drag::FavMove { .. } => self.fav_drag_move(y),
            Drag::SideW { start_x, start_w } => {
                let lo = session::SIDE_W_RANGE.0;
                self.side_w = (start_w + x - start_x).clamp(lo, self.side_w_max());
                self.side_clamp_scroll();
                self.after_resize_drag();
            }
            Drag::TabPress { pane, tab, x: x0, y: y0 } => {
                if (x - x0).abs() > 6.0 || (y - y0).abs() > 6.0 {
                    self.drag = Drag::TabMove { pane, tab, x, y };
                    self.invalidate();
                }
            }
            Drag::TabMove { pane, tab, .. } => {
                self.drag = Drag::TabMove { pane, tab, x, y };
                self.invalidate();
            }
            Drag::MarqueeArm { pane, x: x0, y: y0, additive } => {
                if (x - x0).abs() > 4.0 || (y - y0).abs() > 4.0 {
                    let tab = self.panes[pane].tab_mut();
                    if !additive {
                        tab.clear_selection();
                    }
                    self.marquee_base = tab.selection_mask();
                    // 시작점은 스크롤 0 기준으로 둔다 (목록 보기는 가로 스크롤)
                    let scroll = tab.scroll;
                    let horiz = self.grid(pane).is_some_and(|(_, g)| g.horiz);
                    let (x0, y0) = if horiz { (x0 + scroll, y0) } else { (x0, y0 + scroll) };
                    self.update_marquee(pane, x0, y0, x, y);
                }
            }
            Drag::Marquee { pane, x0, y0, .. } => self.update_marquee(pane, x0, y0, x, y),
            Drag::None => {
                let h = self.hit(x, y);
                if h != self.hover {
                    let on_toast = |h: Hit| matches!(h, Hit::Toast(_) | Hit::ToastUndo(_));
                    match (on_toast(self.hover), on_toast(h)) {
                        (false, true) => unsafe {
                            let _ = KillTimer(Some(self.hwnd), TIMER_FLASH);
                        },
                        (true, false) => unsafe {
                            SetTimer(Some(self.hwnd), TIMER_FLASH, 3000, None);
                        },
                        _ => {}
                    }
                    self.hover = h;
                    self.cmd_hover(h);
                    self.view_menu_hover(h);
                    self.update_tip();
                    self.invalidate();
                }
                self.side_track(x, y);
                if !self.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    self.tracking = unsafe { TrackMouseEvent(&mut tme) }.is_ok();
                }
            }
        }
    }

    /// 자세히 보기에서 이름 칸 밖(날짜·유형·크기)은 끌면 네모 상자로 고른다. 이름 칸은 파일 끌기
    pub(super) fn marquee_column(&self, p: usize, x: f32) -> bool {
        self.panes[p].view == ViewMode::Details && self.list_rect(p).is_some_and(|l| x >= columns(l, self.panes[p].cols).name.1)
    }

    pub(super) fn update_marquee(&mut self, p: usize, x0: f32, y0: f32, x: f32, y: f32) {
        self.drag = Drag::Marquee { pane: p, x0, y0, x, y };
        let Some((list, grid)) = self.grid(p) else { return };
        // 목록 밖으로 벗어나면 그쪽으로 스크롤 (목록 보기는 왼쪽·오른쪽, 나머지는 위·아래)
        let (pos, lo, hi) = if grid.horiz { (x, list.l, list.r) } else { (y, list.t, list.b) };
        if pos < lo {
            self.scroll_by(p, -(lo - pos).min(40.0));
        } else if pos > hi {
            self.scroll_by(p, (pos - hi).min(40.0));
        }
        let tab = self.panes[p].tab_mut();
        let (sx, sy) = if grid.horiz { (tab.scroll, 0.0) } else { (0.0, tab.scroll) };
        let (t, b) = (y0.min(y + sy), y0.max(y + sy));
        let (l, r) = (x0.min(x + sx), x0.max(x + sx));
        tab.select_marquee(&self.marquee_base, |vi| {
            let it = grid.item(vi, 0.0);
            it.l < r && it.r > l && it.t < b && it.b > t
        });
        self.invalidate();
    }

    pub(super) fn after_resize_drag(&mut self) {
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.pal_clamp_scroll();
        self.layout_edits();
        self.invalidate();
    }

    pub(super) fn on_lbutton_down(&mut self, x: f32, y: f32, dbl: bool) {
        // 앞 누름이 검색 패널·보기 메뉴·설정을 열거나 닫았으면, 두 번 클릭의 두 번째 누름은 그 아래 다른 것을 누르지 않게 버린다
        if dbl && std::mem::take(&mut self.swallow_dbl) {
            return;
        }
        let popups = |a: &App| (a.pal_shown(), a.view_menu.is_some(), a.settings.is_some());
        let before = popups(self);
        // 시작 화면 카드를 눌러 옮겨 갔으면 두 번째 누름이 새 목록의 항목을 열지 않게 버린다
        let start = matches!(self.hit(x, y), Hit::Start(..));
        self.press(x, y, dbl);
        self.swallow_dbl = popups(self) != before || (start && !dbl);
    }

    pub(super) fn press(&mut self, x: f32, y: f32, dbl: bool) {
        let hit = self.hit(x, y);
        if self.cmd.is_some() {
            return self.cmd_click(hit);
        }
        // 시작 화면 항목은 누른 순간의 위치를 연다 (아래의 이름 확정 등으로 목록이 바뀌어도)
        let start_target = match hit {
            Hit::Start(p, i) => self.start_target(p, i),
            _ => None,
        };
        // 누르면 말풍선을 거두고 기다리던 것도 지운다 (누른 뒤에 말풍선·정보 카드가 뒤늦게 뜨지 않게)
        self.tip_shown = false;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_TIP);
        }
        // 마우스를 쓰면 시작 화면의 키보드 초점 테두리는 감춘다
        self.start_keys = false;
        if let Hit::Set(h) = hit {
            self.settings_click(h, x, y);
            return;
        }
        // 사이드바 단추·드라이브·폴더 줄의 두 번째 누름(두 번 클릭)은 아무것도 하지 않는다.
        // 첫 누름이 시작한 일(새 그룹·추가 뒤 이름 입력 등)을 끝내 버리지 않게
        if dbl && matches!(hit, Hit::Side(s) if !matches!(s, Side::Fav(_))) {
            return;
        }
        // 막대 아이콘·핀·`…`·검색 패널 단추도 두 번째 누름은 무시한다 (첫 누름이 연 것을 바로 닫지 않게)
        if dbl
            && matches!(
                hit,
                Hit::Bar(_) | Hit::SidePin | Hit::More | Hit::SearchButton | Hit::PalClose | Hit::PalScope(_) | Hit::PalInstall | Hit::PalAddFolder
                    | Hit::PreviewPin | Hit::PreviewChip | Hit::PreviewOpen | Hit::Workspace
            )
        {
            return;
        }
        // 떠 있는 패널 밖을 누르면 패널은 닫고, 누른 것은 그대로 처리한다
        if self.side_floating() && !matches!(hit, Hit::Side(_) | Hit::SidePin | Hit::SidePanel | Hit::Bar(_) | Hit::SearchButton | Hit::SettingsButton) {
            self.close_peek();
        }
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        // 검색 패널 밖을 누르면 키보드는 검색 상자에서 칸으로 (패널과 결과는 그대로)
        if self.pal_focus && !matches!(hit, Hit::PalRow(_) | Hit::PalScope(_) | Hit::PalInstall | Hit::PalAddFolder | Hit::PalClose | Hit::SidePanel | Hit::SearchButton) {
            self.pal_blur();
        }
        // 보기 메뉴가 열려 있으면 메뉴 밖 클릭은 메뉴를 닫기만 한다 (보기 버튼은 아래에서 닫기·다른 칸 메뉴 열기)
        if self.view_menu.is_some() && !matches!(hit, Hit::View(_)) {
            self.view_menu_mouse_down(hit);
            self.invalidate();
            return;
        }
        // 즐겨찾기 줄 밖을 누르면 키보드는 칸으로 돌아간다
        if self.fav_focus && !matches!(hit, Hit::Side(Side::Fav(_))) {
            self.fav_blur();
        }
        if let Some(p) = hit.pane() {
            self.activate_pane(p);
        }
        if !matches!(hit, Hit::CrumbEmpty(_)) {
            self.end_edits_for_click();
        }
        let capture = |app: &mut App, d: Drag| {
            app.drag = d;
            unsafe { SetCapture(app.hwnd) };
        };
        match hit {
            Hit::Layout(i) => self.set_layout(Layout::ALL[i]),
            Hit::Side(s) => self.side_mouse_down(s, y, dbl),
            // 막대의 검색 아이콘: 패널이 보이면 닫고, 아니면 연다
            Hit::SearchButton => {
                if self.pal_shown() {
                    self.close_palette();
                } else {
                    self.open_palette();
                }
            }
            Hit::PalRow(_) | Hit::PalInstall | Hit::PalAddFolder | Hit::PalScope(_) => self.pal_mouse_down(hit, dbl),
            Hit::PalClose => self.close_palette(),
            Hit::NewFolder(p) => self.new_folder(p),
            Hit::Hidden => self.toggle_hidden(),
            Hit::PreviewButton => self.toggle_preview(),
            Hit::PreviewPin | Hit::PreviewChip => self.toggle_preview_pin(),
            Hit::PreviewOpen => self.preview_open(),
            Hit::Workspace => {
                let r = self.workspace_button();
                let pt = self.to_screen(r.l, r.b + 2.0);
                self.pending.push(Deferred::WorkspaceMenu { pt });
            }
            Hit::SettingsButton => self.toggle_settings(settings_ui::Page::General),
            Hit::Bar(panel) => self.bar_click(panel),
            Hit::SidePin => self.side_toggle_pin(),
            Hit::SidePanel => {}
            Hit::SideResize => capture(self, Drag::SideW { start_x: x, start_w: self.side_w }),
            Hit::More => {
                let r = self.more_button();
                let pt = self.to_screen(r.l, r.b + 2.0);
                self.pending.push(Deferred::MoreMenu { pt });
            }
            Hit::View(p) => self.toggle_view_menu(p),
            Hit::SplitX => capture(self, Drag::SplitX),
            Hit::SplitY => capture(self, Drag::SplitY),
            Hit::Tab(p, t) => {
                self.activate_tab(p, t);
                capture(self, Drag::TabPress { pane: p, tab: t, x, y });
            }
            Hit::TabClose(p, t) => self.close_tab(p, t),
            Hit::TabAdd(p) => self.new_tab(p, None, true),
            Hit::TabStrip(p) if dbl => self.new_tab(p, None, true),
            Hit::Back(p) => self.go_back(p),
            Hit::Fwd(p) => self.go_forward(p),
            Hit::Up(p) => self.go_up(p),
            Hit::Crumb(p, i) => {
                let Some(r) = self.pane_rect(p) else { return };
                let cur = self.panes[p].tab().loc.clone();
                if let Some(c) = crumbs(&self.gfx, geom(r).crumbs, &cur, Some("내 PC")).into_iter().nth(i) {
                    if c.target == cur {
                        self.refresh(p);
                    } else {
                        let select = child_toward(&c.target, &cur);
                        self.navigate(p, c.target, select);
                    }
                }
            }
            Hit::CrumbEmpty(p) => self.begin_address(p),
            Hit::Star(p) => self.toggle_star(p),
            Hit::Header(p, key) => self.toggle_sort(p, key),
            Hit::HeaderResize(p, col) => {
                let start_w = self.panes[p].cols[col];
                capture(self, Drag::Col { pane: p, col, start_x: x, start_w });
            }
            Hit::Thumb(p) => {
                let Some((list, grid)) = self.grid(p) else { return };
                let tab = self.panes[p].tab();
                if let Some((track, thumb)) = grid.bar(list, tab.view.len(), tab.scroll) {
                    let (start, ..) = grid.bar_span(track, thumb);
                    capture(self, Drag::Thumb { pane: p, grab: grid.along(x, y) - start });
                }
            }
            Hit::Track(p) => {
                let Some((list, grid)) = self.grid(p) else { return };
                let tab = self.panes[p].tab();
                if let Some((track, thumb)) = grid.bar(list, tab.view.len(), tab.scroll) {
                    let (start, ..) = grid.bar_span(track, thumb);
                    let page = grid.page(list);
                    self.scroll_by(p, if grid.along(x, y) < start { -page } else { page });
                }
            }
            Hit::Row(p, vi) if !dbl && self.marquee_column(p, x) => {
                // 누르기만 하면 그 줄을 고르고, 끌면 네모 상자 고르기
                let tab = self.panes[p].tab_mut();
                if ctrl {
                    tab.toggle(vi);
                } else if shift {
                    tab.select_range(vi, false);
                } else {
                    tab.select_only(vi);
                }
                capture(self, Drag::MarqueeArm { pane: p, x, y, additive: ctrl });
            }
            Hit::Row(p, vi) => {
                if dbl {
                    self.drag = Drag::None;
                    self.open_items(p, false);
                    return;
                }
                let tab = self.panes[p].tab_mut();
                let was_selected = tab.is_selected(vi);
                if ctrl && shift {
                    tab.select_range(vi, true);
                } else if ctrl {
                    tab.toggle(vi);
                } else if shift {
                    tab.select_range(vi, false);
                } else if !was_selected {
                    tab.select_only(vi);
                } else {
                    tab.cursor = vi;
                }
                // 이미 선택된 여러 항목을 끌 수 있도록, 단일 선택 전환은 버튼을 뗄 때 한다
                capture(self, Drag::Press { pane: p, vi, x, y, select_on_up: !ctrl && !shift && was_selected });
            }
            Hit::ListEmpty(p) => {
                if !ctrl {
                    self.panes[p].tab_mut().clear_selection();
                }
                capture(self, Drag::MarqueeArm { pane: p, x, y, additive: ctrl });
            }
            Hit::FilterClear(_) => self.clear_filter(),
            Hit::ToastUndo(p) => self.toast_undo_click(p),
            Hit::NetFix(p) => self.net_fix_click(p),
            Hit::Toast(_) => {}
            // 한 번 누르면 연다 (Ctrl이면 새 탭에서)
            Hit::Start(p, _) if !dbl => {
                if let Some(loc) = start_target {
                    self.start_go(p, loc, ctrl, true);
                }
            }
            _ => {}
        }
        self.invalidate();
    }

    pub(super) fn on_lbutton_up(&mut self) {
        if self.fav_mouse_up(key_down(VK_CONTROL)) {
            return;
        }
        if let Drag::Press { pane, vi, select_on_up: true, .. } = self.drag {
            self.panes[pane].tab_mut().select_only(vi);
        }
        if let Drag::TabMove { pane, tab, x, y } = self.drag {
            if let Some((q, at)) = self.tab_drop_target(x, y) {
                self.move_tab(pane, tab, q, at);
            }
        }
        if !matches!(self.drag, Drag::None) {
            self.drag = Drag::None;
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        self.invalidate();
    }

    pub(super) fn on_rbutton_down(&mut self, x: f32, y: f32) {
        let hit = self.hit(x, y);
        if self.settings.is_some() || self.cmd.is_some() {
            return;
        }
        // 즐겨찾기 줄 밖을 누르면 키보드는 칸으로 (그 뒤 Delete가 즐겨찾기를 지우지 않게). 검색 상자도 마찬가지
        if self.fav_focus && !matches!(hit, Hit::Side(Side::Fav(_))) {
            self.fav_blur();
        }
        if self.pal_focus && !matches!(hit, Hit::PalRow(_) | Hit::PalScope(_) | Hit::PalInstall | Hit::PalAddFolder | Hit::PalClose | Hit::SidePanel) {
            self.pal_blur();
        }
        if let Hit::PalRow(i) = hit {
            self.pal_right_press(i);
            return;
        }
        if matches!(hit, Hit::PalInstall | Hit::PalAddFolder | Hit::PalScope(_) | Hit::PalClose) {
            return;
        }
        if let Some(p) = hit.pane() {
            self.activate_pane(p);
        }
        self.end_edits_for_click();
        if let Hit::Side(_) = hit {
            self.invalidate();
            return;
        }
        match hit {
            Hit::Row(p, vi) => {
                let tab = self.panes[p].tab_mut();
                if tab.is_selected(vi) {
                    tab.cursor = vi;
                } else {
                    tab.select_only(vi);
                }
            }
            Hit::ListEmpty(p) => self.panes[p].tab_mut().clear_selection(),
            _ => {}
        }
        // 셸 메뉴는 확장을 불러오느라 수십~수백 ms가 걸리므로 버튼을 누르는 순간 만들기 시작한다
        if let Some((_, paths, dir)) = self.menu_target(x, y) {
            self.pending.push(Deferred::PrepareMenu { paths, dir });
        }
        self.invalidate();
    }

    /// 우클릭 위치의 메뉴 대상: (패널, 선택 항목 경로 — 빈 곳이면 비어 있음, 현재 폴더)
    pub(super) fn menu_target(&self, x: f32, y: f32) -> Option<(usize, Vec<String>, Option<String>)> {
        let (p, items) = match self.hit(x, y) {
            Hit::Row(p, _) => (p, true),
            Hit::ListEmpty(p) | Hit::Pane(p) | Hit::Header(p, _) => (p, false),
            _ => return None,
        };
        let tab = self.panes[p].tab();
        // 시작 화면에는 셸 메뉴를 띄울 폴더가 없다 (항목 메뉴는 따로)
        if tab.loc == Location::Start {
            return None;
        }
        let paths = if items { tab.selected_paths() } else { Vec::new() };
        Some((p, paths, tab.loc.dir().map(String::from)))
    }

    pub(super) fn on_rbutton_up(&mut self, x: f32, y: f32) -> bool {
        if self.settings.is_some() || self.cmd.is_some() {
            return true;
        }
        if let Hit::PalRow(_) = self.hit(x, y) {
            let pt = self.to_screen(x, y);
            self.pending.push(Deferred::PalMenu { pt });
            return true;
        }
        if self.view_menu.is_some() {
            self.close_view_menu();
            return true;
        }
        if let Hit::Side(s) = self.hit(x, y) {
            self.side_context_menu(s, x, y);
            return true;
        }
        if let Hit::Start(p, i) = self.hit(x, y) {
            self.start_context_menu(p, i, x, y);
            return true;
        }
        let Some((pane, paths, dir)) = self.menu_target(x, y) else { return false };
        let pt = self.to_screen(x, y);
        self.pending.push(Deferred::Menu { pane, paths, dir, pt });
        true
    }

    pub(super) fn on_mbutton_up(&mut self, x: f32, y: f32) {
        if self.settings.is_some() || self.cmd.is_some() {
            return;
        }
        let start_target = match self.hit(x, y) {
            Hit::Start(p, i) => self.start_target(p, i).map(|loc| (p, loc)),
            _ => None,
        };
        if self.fav_focus && !matches!(self.hit(x, y), Hit::Side(Side::Fav(_))) {
            self.fav_blur();
        }
        if let Hit::PalRow(i) = self.hit(x, y) {
            self.pal_activate(i, palette::OpenMode::NewTab);
            return;
        }
        if self.view_menu.is_some() {
            self.close_view_menu();
            return;
        }
        match self.hit(x, y) {
            Hit::Side(s @ (Side::Fav(crate::favorites::Row::Item { .. }) | Side::Computer | Side::Drive(_) | Side::Folder(_))) => self.side_open(s, true),
            Hit::Tab(p, t) | Hit::TabClose(p, t) => self.close_tab(p, t),
            Hit::Start(..) => {
                if let Some((p, loc)) = start_target {
                    self.start_go(p, loc, true, false);
                }
            }
            Hit::Row(p, vi) => {
                let tab = self.panes[p].tab();
                if let Some(path) = tab.entry_at(vi).filter(|e| e.is_dir()).and_then(|e| tab.path_of(e)) {
                    self.new_tab(p, Some(Location::Dir(path)), false);
                }
            }
            _ => {}
        }
    }

    pub(super) fn on_wheel(&mut self, wp: WPARAM, lp: LPARAM) {
        let delta = hiword(wp.0) as i16 as f32;
        let mut pt = POINT { x: loword(lp.0 as usize) as i16 as i32, y: hiword(lp.0 as usize) as i16 as i32 };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }
        let (x, y) = (pt.x as f32 / self.scale(), pt.y as f32 / self.scale());
        if self.settings.is_some() {
            self.settings_wheel(-delta / 120.0 * self.wheel_lines * ROW_H);
            return;
        }
        // 명령 메뉴가 열려 있으면 그 목록만 움직인다
        if self.cmd.is_some() {
            if self.cmd_rect().is_some_and(|r| r.contains(x, y)) {
                self.cmd_wheel(-delta / 120.0 * self.wheel_lines * ROW_H);
            }
            return;
        }
        if self.pal_visible() && self.pal_list().contains(x, y) {
            self.pal_wheel(-delta / 120.0 * self.wheel_lines * ROW_H);
            return;
        }
        // 보기 메뉴가 열려 있으면 휠로 뒤쪽 목록이 움직이지 않게
        if self.view_menu.is_some() {
            return;
        }
        if self.side_panel_rect().is_some_and(|r| r.contains(x, y)) {
            self.side_wheel(-delta / 120.0 * self.wheel_lines * ROW_H);
            return;
        }
        if self.preview_rect().is_some_and(|r| r.contains(x, y)) {
            self.preview_wheel(-delta / 120.0 * self.wheel_lines * ROW_H);
            return;
        }
        let hit = self.hit(x, y);
        // 탭 줄에서 휠: 옆 탭으로 (탭이 많아 가려진 탭도 닿게)
        if let Hit::Tab(p, _) | Hit::TabClose(p, _) | Hit::TabStrip(p) | Hit::TabAdd(p) = hit {
            self.cycle_tab(p, delta > 0.0);
            self.invalidate();
            return;
        }
        let p = hit.pane().unwrap_or(self.active);
        // 아이콘 보기는 한 번에 한 줄, 목록 보기는 한 열(가로), 나머지는 세 줄
        let step = match self.grid(p) {
            _ if self.panes[p].tab().loc == Location::Start => self.wheel_lines * ROW_H,
            Some((_, g)) if g.horiz => g.cell_w,
            Some((_, g)) if matches!(g.cell, CellKind::Tile(_)) => g.cell_h,
            _ => self.wheel_lines * ROW_H,
        };
        let dy = -delta / 120.0 * step;
        if !sidebar::motion_on() || self.edit_mode == EditMode::Rename(p) {
            self.wheel = None;
            return self.scroll_by(p, dy);
        }
        let (id, cur) = (self.panes[p].tab().id, self.panes[p].tab().scroll);
        let base = match self.wheel {
            Some(w) if w.pane == p && w.tab == id && (w.last - cur).abs() < 0.5 => w.target,
            _ => cur,
        };
        // 목표도 목록 끝을 넘지 않게 (clamp_scroll로 잰다)
        self.panes[p].tab_mut().scroll = base + dy;
        self.clamp_scroll(p);
        let target = self.panes[p].tab().scroll;
        self.panes[p].tab_mut().scroll = cur;
        self.wheel = Some(Wheel { pane: p, tab: id, target, last: cur });
        unsafe { SetTimer(Some(self.hwnd), TIMER_WHEEL, 15, None) };
        self.on_wheel_timer();
    }

    /// 마우스가 칸 위에 그대로 있을 때도 커서 모양(기다림 표시)을 지금 상태로 맞춘다
    pub(super) fn refresh_cursor(&self) {
        if !matches!(self.drag, Drag::None) || !unsafe { GetCapture() }.is_invalid() {
            return;
        }
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() || unsafe { WindowFromPoint(pt) } != self.hwnd {
            return;
        }
        let _ = unsafe { ScreenToClient(self.hwnd, &mut pt) };
        let (x, y) = (pt.x as f32 / self.scale(), pt.y as f32 / self.scale());
        if self.pane_rects().iter().any(|(_, r)| r.contains(x, y)) {
            self.on_set_cursor();
        }
    }

    pub(super) fn on_set_cursor(&self) -> bool {
        let id = match self.drag {
            Drag::SplitX | Drag::Col { .. } | Drag::SideW { .. } => IDC_SIZEWE,
            Drag::SplitY => IDC_SIZENS,
            _ => {
                let mut pt = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut pt);
                    let _ = ScreenToClient(self.hwnd, &mut pt);
                }
                let (x, y) = (pt.x as f32 / self.scale(), pt.y as f32 / self.scale());
                let hit = self.hit(x, y);
                let under = self.pane_rects().into_iter().find(|(_, r)| r.contains(x, y)).map(|(q, _)| q);
                let waiting = !self.probes.is_empty()
                    || under.is_some_and(|q| !self.panes[q].tab().offline && self.panes[q].tab().loading_since.is_some_and(|s| s.elapsed().as_millis() >= LOADING_SHOW_MS));
                match hit {
                    Hit::SplitX | Hit::HeaderResize(..) | Hit::SideResize => IDC_SIZEWE,
                    Hit::SplitY => IDC_SIZENS,
                    Hit::CrumbEmpty(_) => IDC_IBEAM,
                    _ if waiting => IDC_APPSTARTING,
                    _ => IDC_ARROW,
                }
            }
        };
        unsafe {
            SetCursor(LoadCursorW(None, id).ok());
        }
        true
    }

    pub(super) fn move_cursor_key(&mut self, p: usize, vk: VIRTUAL_KEY, shift: bool, ctrl: bool) {
        let page = self.page_rows(p);
        let grid = self.grid(p).map(|(_, g)| g);
        let cols = grid.map(|g| g.cols as isize).unwrap_or(1);
        let tab = self.panes[p].tab_mut();
        let n = tab.view.len();
        if n == 0 {
            return;
        }
        let cur = tab.cursor as isize;
        let last = n as isize - 1;
        // 목록 보기: 위·아래는 한 칸, 왼쪽·오른쪽은 옆 열로 (오른쪽 끝 열이 짧으면 마지막 항목으로)
        if let Some(g) = grid.filter(|g| g.horiz) {
            let rows = g.rows as isize;
            let to = match vk {
                VK_UP => cur - 1,
                VK_DOWN => cur + 1,
                VK_LEFT if cur < rows => cur,
                VK_LEFT => cur - rows,
                VK_RIGHT if cur + rows > last && cur / rows < last / rows => last,
                VK_RIGHT if cur + rows > last => cur,
                VK_RIGHT => cur + rows,
                VK_PRIOR => cur - page,
                VK_NEXT => cur + page,
                VK_HOME => 0,
                _ => last,
            };
            tab.move_cursor(to.clamp(0, last) as usize, shift, ctrl);
            self.ensure_visible(p);
            return;
        }
        let to = match vk {
            VK_UP if cur < cols => cur,
            VK_UP => cur - cols,
            // 마지막 줄이 덜 찼으면 아래 칸 대신 마지막 항목으로
            VK_DOWN if cur + cols > last && cur / cols < last / cols => last,
            VK_DOWN if cur + cols > last => cur,
            VK_DOWN => cur + cols,
            VK_LEFT => cur - 1,
            VK_RIGHT => cur + 1,
            VK_PRIOR => cur - page,
            VK_NEXT => cur + page,
            VK_HOME => 0,
            _ => last,
        };
        tab.move_cursor(to.clamp(0, last) as usize, shift, ctrl);
        self.ensure_visible(p);
    }

    pub(super) fn escape(&mut self, p: usize) {
        if !self.panes[p].tab().filter.is_empty() {
            self.clear_filter();
        } else {
            self.panes[p].tab_mut().clear_selection();
        }
    }

    /// 단축키. 처리했으면 true
    /// 바꿀 수 있는 단축키에 매인 동작
    pub(super) fn run_action(&mut self, a: crate::keys::Action) {
        use crate::keys::Action;
        let p = self.active;
        match a {
            Action::Search => self.toggle_palette(),
            Action::Commands => self.toggle_commands(),
            Action::Favorites => self.toggle_fav_focus(),
            Action::AddFavorite => self.add_favorite(),
            Action::NewTab => self.new_tab(p, None, true),
            Action::CloseTab => {
                let t = self.panes[p].active;
                self.close_tab(p, t);
            }
            Action::ReopenTab => self.reopen_closed_tab(),
            Action::NewFolder => self.new_folder(p),
            Action::Hidden => self.toggle_hidden(),
            Action::Preview => self.toggle_preview(),
            Action::Undo => self.undo_last(),
            Action::Layout1 => self.set_layout(Layout::ALL[0]),
            Action::Layout2 => self.set_layout(Layout::ALL[1]),
            Action::Layout3 => self.set_layout(Layout::ALL[2]),
            Action::Layout4 => self.set_layout(Layout::ALL[3]),
            Action::Layout5 => self.set_layout(Layout::ALL[4]),
            Action::Thumbs => self.set_view(p, ViewMode::Large),
            Action::Huge => self.set_view(p, ViewMode::Huge),
            Action::Medium => self.set_view(p, ViewMode::Medium),
            Action::SmallIcons => self.set_view(p, ViewMode::Small),
            Action::List => self.set_view(p, ViewMode::List),
            Action::Details => self.set_view(p, ViewMode::Details),
            Action::Address => self.begin_address(p),
            Action::Rename => self.begin_rename(p),
            Action::Refresh => self.refresh(p),
            Action::Settings => self.toggle_settings(settings_ui::Page::General),
            Action::Help => self.toggle_settings(settings_ui::Page::Help),
        }
    }

    pub(super) fn on_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        // 누르고 있어 반복되는 Enter·Delete·Backspace·F2는 한 번만 (폴더를 열자마자 그 안의 첫 항목을 열거나, 여러 단계 위로 가지 않게)
        if self.key_repeat && matches!(vk, VK_RETURN | VK_DELETE | VK_BACK | VK_F2) {
            return true;
        }
        if self.settings.is_some() {
            return self.settings_key(vk);
        }
        if self.view_menu.is_some() {
            return self.view_menu_key(vk);
        }
        if self.fav_focus {
            return self.fav_key(vk);
        }
        if let Some(a) = self.keys.find(crate::keys::Combo::now(vk.0)) {
            self.run_action(a);
            self.invalidate();
            return true;
        }
        // 시작 화면: 화살표는 카드·줄 사이를 옮기고, Enter는 고른 곳을 연다
        if self.panes[self.active].tab().loc == Location::Start && self.start_key(self.active, vk) {
            return true;
        }
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let alt = key_down(VK_MENU);
        let p = self.active;
        match vk {
            VK_TAB if ctrl => self.cycle_tab(p, shift),
            VK_TAB if !alt => self.cycle_pane(shift),
            VK_LEFT if alt => self.go_back(p),
            VK_RIGHT if alt => self.go_forward(p),
            VK_UP if alt => self.go_up(p),
            VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_HOME | VK_END if !alt => self.move_cursor_key(p, vk, shift, ctrl),
            VK_LEFT | VK_RIGHT if !alt && self.panes[p].view != ViewMode::Details => self.move_cursor_key(p, vk, shift, ctrl),
            VK_RETURN if alt => self.properties(p),
            VK_RETURN => self.open_items(p, ctrl),
            VK_BACK if !ctrl && !alt => self.go_up(p),
            // 경로 입력은 바꿀 수 있는 단축키 말고도 탐색기와 같은 F4·Alt+D로 늘 된다
            VK_F4 if !alt && !ctrl => self.begin_address(p),
            // Ctrl+F4는 탭 닫기 (탐색기·브라우저와 같음). Alt+F4는 여기서 처리하지 않아야 창이 닫힌다
            VK_F4 if ctrl && !alt => {
                let t = self.panes[p].active;
                self.close_tab(p, t);
            }
            VK_DELETE if !ctrl && !alt => self.delete(p, shift),
            VK_ESCAPE => self.escape(p),
            VK_APPS => self.context_menu_at_cursor(p),
            VK_F10 if shift => self.context_menu_at_cursor(p),
            _ if ctrl && !alt => {
                let c = vk.0 as u8;
                if vk.0 > 0xFF {
                    return false;
                }
                // 바꿀 수 없는 기본 조작 (나머지 Ctrl 단축키는 keys.rs의 키 맵에서 처리)
                match c {
                    b'A' => self.panes[p].tab_mut().select_all(),
                    b'C' if shift => self.copy_paths(p),
                    b'C' => self.copy(p, false),
                    b'X' => self.copy(p, true),
                    b'V' => self.paste(p),
                    _ => return false,
                }
            }
            _ if alt && vk.0 == b'D' as u16 => self.begin_address(p),
            _ => return false,
        }
        self.invalidate();
        true
    }

    /// 입력 상자에서 누른 키. 필터 상자는 글자 입력만 받고 탐색 키는 목록으로 넘긴다
    pub fn on_edit_key(&mut self, id: i32, vk: VIRTUAL_KEY) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let alt = key_down(VK_MENU);
        match id {
            ID_SEARCH => self.pal_key(vk),
            ID_CMD => self.cmd_key(vk),
            // 보기 메뉴가 열려 있거나 키보드가 즐겨찾기에 있으면 필터 상자의 키는 그쪽 조작에 쓴다
            ID_FILTER if self.view_menu.is_some() => self.view_menu_key(vk),
            ID_FILTER if self.fav_focus => self.fav_key(vk),
            ID_FILTER => {
                let len = unsafe { GetWindowTextLengthW(self.filter_edit) };
                let empty = len == 0;
                let (sel_start, sel_end) = edit_sel(self.filter_edit);
                let text_sel = sel_start != sel_end;
                // 필터 글자가 있어도 목록을 다루는 키: 처음·끝, 아이콘 보기의 좌우, 글자 끝에서 누른 Delete(파일 지우기)
                let list_key = match vk {
                    VK_HOME | VK_END => true,
                    VK_LEFT | VK_RIGHT => self.panes[self.active].view != ViewMode::Details,
                    VK_DELETE => !text_sel && sel_end as i32 >= len,
                    _ => false,
                };
                let text_key = !list_key && matches!(vk, VK_BACK | VK_DELETE | VK_HOME | VK_END | VK_LEFT | VK_RIGHT);
                let forward = match vk {
                    VK_ESCAPE if !empty => {
                        self.clear_filter();
                        return true;
                    }
                    // 빈 필터를 공백으로 시작하지 않는다
                    VK_SPACE if empty && !ctrl && !alt => return true,
                    _ if list_key && !alt => true,
                    _ if text_key && !empty && !alt => false,
                    // 필터를 건 채로도 Ctrl+A·Z는 목록(전체 고르기·되돌리기)에 쓴다. Ctrl+C·X는 필터 글자를 골랐을 때만 글자에,
                    // Ctrl+V는 클립보드에 파일이 있으면 목록(붙여넣기)에 쓴다
                    _ if ctrl && !empty && matches!(vk.0 as u8, b'C' | b'X') => !text_sel,
                    _ if ctrl && !empty && vk.0 as u8 == b'V' => shell::clipboard_has_files(),
                    VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_RETURN | VK_TAB | VK_ESCAPE | VK_APPS => true,
                    _ if (VK_F1.0..=VK_F24.0).contains(&vk.0) => true,
                    _ if text_key || ctrl || alt => true,
                    _ => false,
                };
                forward && self.on_key(vk)
            }
            ID_RENAME => match vk {
                VK_RETURN => {
                    self.commit_rename_with(true);
                    true
                }
                VK_TAB => {
                    self.commit_rename();
                    true
                }
                VK_ESCAPE => {
                    self.cancel_edit();
                    true
                }
                _ => false,
            },
            ID_ADDR => match vk {
                VK_RETURN => {
                    self.commit_address();
                    true
                }
                VK_ESCAPE | VK_TAB => {
                    self.cancel_edit();
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// 입력 상자가 포커스를 잃으면: 이름 바꾸기는 확정, 주소 입력은 취소
    pub fn on_edit_blur(&mut self, id: i32) {
        match (id, self.edit_mode) {
            (ID_RENAME, EditMode::Rename(_) | EditMode::FavRename(_) | EditMode::WorkspaceRename) => self.commit_rename(),
            (ID_ADDR, EditMode::Address(_)) => self.cancel_edit(),
            _ => {}
        }
    }
}
