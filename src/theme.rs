//! 라이트/다크 색상 팔레트와 시스템 앱 테마 감지

use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::w;

pub type Color = D2D1_COLOR_F;

const fn hex(v: u32) -> Color {
    Color {
        r: ((v >> 16) & 0xFF) as f32 / 255.0,
        g: ((v >> 8) & 0xFF) as f32 / 255.0,
        b: (v & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub window: Color,
    pub pane: Color,
    pub strip: Color,
    pub field: Color,
    pub text: Color,
    pub dim: Color,
    pub faint: Color,
    pub border: Color,
    pub hover: Color,
    pub sel: Color,
    pub sel_inactive: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub thumb: Color,
}

impl Theme {
    pub const LIGHT: Theme = Theme {
        dark: false,
        window: hex(0xEDEFF2),
        pane: hex(0xFFFFFF),
        strip: hex(0xF4F5F7),
        field: hex(0xF0F2F5),
        text: hex(0x1B1C1F),
        dim: hex(0x5F636B),
        faint: hex(0x9EA2AA),
        border: hex(0xDCDFE4),
        hover: hex(0xEEF2F7),
        sel: hex(0xD5E4FD),
        sel_inactive: hex(0xE6E8EC),
        accent: hex(0x2F6FEB),
        accent_soft: hex(0xE1EAFC),
        thumb: hex(0xC4C8CF),
    };

    pub const DARK: Theme = Theme {
        dark: true,
        window: hex(0x141518),
        pane: hex(0x1E1F23),
        strip: hex(0x18191C),
        field: hex(0x2A2C31),
        text: hex(0xE7E8EB),
        dim: hex(0xA0A3AB),
        faint: hex(0x6E717A),
        border: hex(0x2E3036),
        hover: hex(0x282A2F),
        sel: hex(0x23406F),
        sel_inactive: hex(0x30333A),
        accent: hex(0x5B9BFF),
        accent_soft: hex(0x1F3354),
        thumb: hex(0x4B4E56),
    };

    /// 시스템 앱 모드를 따른다. SAKADU_THEME=dark|light로 강제할 수 있다
    pub fn system() -> Theme {
        let dark = match std::env::var("SAKADU_THEME").as_deref() {
            Ok("dark") => true,
            Ok("light") => false,
            _ => system_prefers_dark(),
        };
        if dark { Self::DARK } else { Self::LIGHT }
    }
}

/// Windows 설정 > 개인 설정 > 색의 "앱 모드"가 어둡게인지
pub fn system_prefers_dark() -> bool {
    let mut value: u32 = 1;
    let mut size = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    r == ERROR_SUCCESS && value == 0
}

pub fn colorref(c: Color) -> COLORREF {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(ch(c.r) | (ch(c.g) << 8) | (ch(c.b) << 16))
}

pub fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}
