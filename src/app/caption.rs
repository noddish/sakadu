//! 제목 표시줄을 작업 줄과 합친다: Windows 기본 제목 줄 대신 칸 영역 위 작업 줄과 사이드바 맨 위가 창 맨 위에 오고,
//! 오른쪽 끝에 최소화·최대화·닫기를 직접 그린다.
//! 그 줄의 빈 곳은 Windows에 제목 줄(HTCAPTION)이라고 알려 끌기·두 번 눌러 최대화·창 메뉴가 그대로 되고,
//! 최대화 버튼은 HTMAXBUTTON으로 알려 Windows 11 스냅 레이아웃이 뜬다

use super::*;

/// 창 조절 버튼 하나의 크기 (Windows 11과 같은 너비). 높이는 작업 줄과 같아서 글리프가 작업 줄의 다른 단추와 같은 줄에 온다
pub(super) const CAPTION_BTN_W: f32 = 46.0;
const CAPTION_BTN_H: f32 = WORKBAR_H;
const G_MINIMIZE: u16 = 0xE921;
const G_MAXIMIZE: u16 = 0xE922;
const G_RESTORE: u16 = 0xE923;
const G_CLOSE_WINDOW: u16 = 0xE8BB;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptionButton {
    Min,
    Max,
    Close,
}

impl CaptionButton {
    const ALL: [CaptionButton; 3] = [CaptionButton::Min, CaptionButton::Max, CaptionButton::Close];

    fn code(self) -> u32 {
        match self {
            CaptionButton::Min => HTMINBUTTON,
            CaptionButton::Max => HTMAXBUTTON,
            CaptionButton::Close => HTCLOSE,
        }
    }

    fn from_code(code: u32) -> Option<CaptionButton> {
        CaptionButton::ALL.into_iter().find(|b| b.code() == code)
    }
}

impl App {
    /// 작업 줄에서 창 조절 버튼이 차지하는 왼쪽 끝 (DIP)
    pub(super) fn caption_left(&self) -> f32 {
        self.w - 3.0 * CAPTION_BTN_W
    }

    fn caption_rect(&self, b: CaptionButton) -> R {
        let i = CaptionButton::ALL.iter().position(|x| *x == b).unwrap_or(0) as f32;
        let l = self.caption_left() + i * CAPTION_BTN_W;
        R::new(l, 0.0, l + CAPTION_BTN_W, CAPTION_BTN_H)
    }

    fn caption_at(&self, x: f32, y: f32) -> Option<CaptionButton> {
        CaptionButton::ALL.into_iter().find(|b| self.caption_rect(*b).contains(x, y))
    }

    /// WM_NCHITTEST에서 DefWindowProc가 HTCLIENT라고 한 곳을 다시 나눈다. None이면 그대로 HTCLIENT
    pub fn nc_hit(&self, lp: LPARAM) -> Option<u32> {
        let mut pt = POINT { x: loword(lp.0 as usize) as i16 as i32, y: hiword(lp.0 as usize) as i16 as i32 };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }
        let s = self.scale();
        let (x, y) = (pt.x as f32 / s, pt.y as f32 / s);
        if let Some(b) = self.caption_at(x, y) {
            return Some(b.code());
        }
        // 위 가장자리: 창 크기 조절 (최대화했으면 없음). 양쪽 끝은 모서리 조절
        if !unsafe { IsZoomed(self.hwnd) }.as_bool() {
            let edge = crate::resize_border(self.hwnd);
            if pt.y < edge {
                let w = (self.w * s) as i32;
                return Some(if pt.x < edge * 2 {
                    HTTOPLEFT
                } else if pt.x >= w - edge * 2 {
                    HTTOPRIGHT
                } else {
                    HTTOP
                });
            }
        }
        if y >= WORKBAR_H {
            return None;
        }
        // 팝업이 열려 있으면 빈 곳을 눌러 닫을 수 있게 그대로 둔다
        if self.pal_open || self.view_menu.is_some() {
            return None;
        }
        match self.hit(x, y) {
            Hit::None | Hit::Set(settings_ui::SetHit::Panel) => Some(HTCAPTION),
            _ => None,
        }
    }

    /// 창 조절 버튼 위의 비클라이언트 마우스 메시지. Some이면 처리를 마쳤다(DefWindowProc를 부르지 않는다:
    /// 부르면 Windows가 예전 모양 버튼을 그 위에 그린다)
    pub fn on_nc_mouse(&mut self, msg: u32, wp: WPARAM) -> Option<LRESULT> {
        let over = CaptionButton::from_code(wp.0 as u32);
        match msg {
            WM_NCMOUSEMOVE => {
                let hover = over.map_or(Hit::None, Hit::Caption);
                if hover != self.hover {
                    self.hover = hover;
                    self.invalidate();
                }
                if !self.nc_tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    self.nc_tracking = unsafe { TrackMouseEvent(&mut tme) }.is_ok();
                }
                None
            }
            WM_NCMOUSELEAVE => {
                self.nc_tracking = false;
                if matches!(self.hover, Hit::Caption(_)) {
                    self.hover = Hit::None;
                }
                self.caption_pressed = None;
                self.invalidate();
                None
            }
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
                let b = over?;
                self.caption_pressed = Some(b);
                self.invalidate();
                Some(LRESULT(0))
            }
            WM_NCLBUTTONUP => {
                let b = over?;
                let pressed = self.caption_pressed.take();
                self.invalidate();
                if pressed == Some(b) {
                    let cmd = match b {
                        CaptionButton::Min => SC_MINIMIZE,
                        CaptionButton::Max if unsafe { IsZoomed(self.hwnd) }.as_bool() => SC_RESTORE,
                        CaptionButton::Max => SC_MAXIMIZE,
                        CaptionButton::Close => SC_CLOSE,
                    };
                    unsafe {
                        let _ = PostMessageW(Some(self.hwnd), WM_SYSCOMMAND, WPARAM(cmd as usize), LPARAM(0));
                    }
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    /// 창 조절 버튼: 마우스를 올리면 옅은 바탕, 닫기는 빨간 바탕. 창이 비활성이면 흐리게
    pub(super) fn paint_caption(&self) {
        let t = self.theme;
        let g = &self.gfx;
        let zoomed = unsafe { IsZoomed(self.hwnd) }.as_bool();
        for b in CaptionButton::ALL {
            let r = self.caption_rect(b);
            let hovered = self.hover == Hit::Caption(b);
            let pressed = hovered && self.caption_pressed == Some(b);
            let mut fg = if self.window_active { t.text } else { t.dim };
            if t.high_contrast {
                if hovered {
                    g.fill(r, t.sel);
                    fg = t.sel_text;
                }
            } else if b == CaptionButton::Close && hovered {
                let red = Color { r: 0.769, g: 0.169, b: 0.110, a: if pressed { 0.85 } else { 1.0 } };
                g.fill(r, red);
                fg = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
            } else if hovered {
                let a = match (t.dark, pressed) {
                    (false, false) => 0.06,
                    (false, true) => 0.10,
                    (true, false) => 0.09,
                    (true, true) => 0.14,
                };
                g.fill(r, Color { r: if t.dark { 1.0 } else { 0.0 }, g: if t.dark { 1.0 } else { 0.0 }, b: if t.dark { 1.0 } else { 0.0 }, a });
            }
            let glyph = match b {
                CaptionButton::Min => G_MINIMIZE,
                CaptionButton::Max if zoomed => G_RESTORE,
                CaptionButton::Max => G_MAXIMIZE,
                CaptionButton::Close => G_CLOSE_WINDOW,
            };
            g.glyph(glyph, r, true, fg);
        }
    }
}
