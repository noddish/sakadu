//! 트레이 대기, 창·세션 저장

use super::*;

impl App {
    // ---------- 트레이 대기 ----------

    /// 켜 두면 창을 닫아도 끝내지 않고 알림 영역에 남아, 전역 단축키로 다시 띄운다
    pub fn set_tray(&mut self, on: bool) {
        if !on {
            self.tray = None;
        } else if self.tray.is_none() {
            self.tray = Some(crate::tray::TrayIcon::add(self.hwnd, &self.tray_tip()));
        }
    }

    pub(super) fn tray_tip(&self) -> String {
        match &self.hotkey {
            Some(key) => format!("Sakadu · {key}로 검색"),
            None => "Sakadu".to_string(),
        }
    }

    /// 탐색기가 다시 시작돼 알림 영역 아이콘이 사라졌을 때
    pub fn on_taskbar_created(&mut self) {
        let tip = self.tray_tip();
        if let Some(t) = &mut self.tray {
            t.update(true, &tip);
        }
    }

    /// 숨겨 둔 창이 다시 보일 때: 그사이 바뀐 폴더를 읽는다
    pub fn on_shown(&mut self) {
        for p in 0..self.layout.count() {
            if std::mem::take(&mut self.stale[p]) {
                self.load(p, true);
            }
        }
        self.invalidate();
    }

    /// 트레이 메뉴의 끝내기
    pub fn quit(&mut self) {
        if unsafe { IsWindowVisible(self.hwnd) }.as_bool() {
            self.save_session();
        }
        self.pending.push(Deferred::Close);
    }

    // ---------- 창·세션 ----------

    pub(super) fn update_title(&self) {
        // 관리자 권한으로 실행되면 일반 권한 탐색기와의 끌어서 놓기가 Windows 보안(UIPI)으로 막히므로 표시해 둔다
        let admin = if unsafe { IsUserAnAdmin() }.as_bool() { " (관리자)" } else { "" };
        let title = format!("{} - Sakadu{admin}", self.panes[self.active].tab().loc.title());
        unsafe {
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(title));
        }
    }

    pub(super) fn apply_window_theme(&mut self) {
        unsafe {
            let dark = BOOL::from(self.theme.dark);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const c_void, 4);
            // 제목 표시줄을 창 바탕색과 맞춰 한 덩어리처럼 보이게 (Windows 11)
            let caption = colorref(self.theme.window);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_CAPTION_COLOR, &caption as *const _ as *const c_void, 4);
            let text = colorref(self.theme.text);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_TEXT_COLOR, &text as *const _ as *const c_void, 4);
        }
        let t = self.theme;
        let colors = unsafe {
            EditColors {
                text: colorref(t.text),
                field: colorref(t.field),
                field_brush: CreateSolidBrush(colorref(t.field)),
                pane: colorref(t.pane),
                pane_brush: CreateSolidBrush(colorref(t.pane)),
                chip: colorref(t.accent_soft),
                chip_brush: CreateSolidBrush(colorref(t.accent_soft)),
                rename: self.rename_edit,
                preview_host: self.preview_host,
                filter: self.filter_edit,
                cmd: self.cmd_edit,
            }
        };
        if let Some(old) = EDIT_COLORS.replace(Some(colors)) {
            unsafe {
                let _ = DeleteObject(old.field_brush.into());
                let _ = DeleteObject(old.pane_brush.into());
                let _ = DeleteObject(old.chip_brush.into());
            }
        }
        for h in [self.filter_edit, self.rename_edit, self.addr_edit, self.search_edit, self.cmd_edit] {
            unsafe {
                let _ = InvalidateRect(Some(h), None, true);
            }
        }
        self.invalidate();
    }

    /// 세션을 저장한다. 작업 공간이 있으면 지금 작업 공간에도 지금 화면을 적어 둔다
    pub(super) fn save_session(&mut self) {
        let s = self.screen_state();
        let _ = session::save(&s);
        if let Some(w) = self.workspaces.current.and_then(|c| self.workspaces.list.get_mut(c)) {
            w.state = s;
            if let Err(e) = crate::workspaces::save(&self.workspaces) {
                crate::diag::log(|| format!("workspaces: save failed: {e}"));
            }
        }
    }

    pub fn shutdown(&mut self) {
        for w in self.watches.iter_mut() {
            if let Some((id, _)) = w.take() {
                shell::unwatch(id);
            }
        }
        unsafe {
            let _ = RevokeDragDrop(self.hwnd);
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_SEARCH);
        }
        self.tray = None;
    }

    /// 창 DPI가 바뀌었을 때(모니터 이동 등). 설정의 크기(%)를 곱해 그린다
    pub fn on_dpi(&mut self, dpi: f32) {
        self.gfx.set_dpi(dpi * self.zoom());
        self.apply_edit_font();
    }

    /// 설정의 크기 (1.0 = 100%)
    pub fn zoom(&self) -> f32 {
        self.look.zoom as f32 / 100.0
    }

    /// 화면 설정을 바꾼 뒤: 바뀐 것만 다시 적용하고 저장한다.
    /// 색(테마·강조 색)만 바뀌면 글꼴을 다시 만들거나 아이콘·썸네일을 비우지 않는다
    pub(super) fn apply_look(&mut self, changed: LookPart) {
        self.theme = self.resolve_theme();
        if changed == LookPart::Font {
            if let Err(e) = self.gfx.set_font(&self.look.font) {
                crate::diag::log(|| format!("font: {e}"));
            }
            self.apply_edit_font();
        }
        if changed == LookPart::Text {
            self.gfx.set_text_look(crate::gfx::TextLook::from_key(&self.look.text));
        }
        if changed == LookPart::Zoom {
            // 크기가 바뀌면 아이콘·썸네일 비트맵 크기도 달라지므로 다시 만든다 (on_dpi)
            let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96) as f32;
            self.on_dpi(dpi);
            let mut rc = RECT::default();
            unsafe {
                let _ = GetClientRect(self.hwnd, &mut rc);
            }
            self.on_size(rc.right as u32, rc.bottom as u32);
        } else if matches!(changed, LookPart::Font | LookPart::Text) {
            // 글자 폭이 달라지므로 입력 상자 자리를 다시 잡는다
            self.layout_edits();
        }
        self.apply_window_theme();
        self.save_session();
    }

    pub(super) fn on_size(&mut self, wpx: u32, hpx: u32) {
        self.w = wpx as f32 / self.scale();
        self.h = hpx as f32 / self.scale();
        self.gfx.resize(wpx, hpx);
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.side_clamp_scroll();
        self.pal_clamp_scroll();
        self.layout_edits();
        self.invalidate();
    }
}
