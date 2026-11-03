//! 라이트/다크 색상 팔레트와 시스템 앱 테마 감지

use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{PCWSTR, w};

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

    /// 설정의 테마("system"·"light"·"dark")와 강조 색(ACCENTS의 키)에 맞춘 색.
    /// SAKADU_THEME=dark|light가 있으면 테마는 그것을 따른다 (스크린샷·시험용)
    pub fn resolve(mode: &str, accent: &str) -> Theme {
        let dark = match std::env::var("SAKADU_THEME").as_deref() {
            Ok("dark") => true,
            Ok("light") => false,
            _ => match mode {
                "dark" => true,
                "light" => false,
                _ => system_prefers_dark(),
            },
        };
        let mut t = if dark { Self::DARK } else { Self::LIGHT };
        // 기본 파랑은 손으로 맞춘 색을 그대로 쓰고, 나머지는 강조 색을 바탕색에 섞어 만든다
        if accent != "blue" {
            if let Some(c) = accent_color(accent, dark) {
                t.accent = c;
                t.accent_soft = mix(t.pane, c, if dark { 0.22 } else { 0.12 });
                t.sel = mix(t.pane, c, if dark { 0.35 } else { 0.2 });
            }
        }
        t
    }
}

/// 강조 색: (설정 키, 이름, 밝은 테마 색, 어두운 테마 색). "system"은 Windows 강조 색
pub const ACCENTS: [(&str, &str, u32, u32); 7] = [
    ("system", "Windows 강조 색", 0, 0),
    ("blue", "파랑", 0x2F6FEB, 0x5B9BFF),
    ("green", "초록", 0x1E9E5A, 0x4CC38A),
    ("purple", "보라", 0x7B4DE0, 0xA98BFF),
    ("orange", "주황", 0xD9731A, 0xFFA552),
    ("pink", "분홍", 0xD6457C, 0xFF7EB0),
    ("teal", "청록", 0x0F9AA8, 0x3CC8D6),
];

pub fn accent_color(key: &str, dark: bool) -> Option<Color> {
    if key == "system" {
        return windows_accent(dark);
    }
    ACCENTS.iter().find(|a| a.0 == key).map(|a| hex(if dark { a.3 } else { a.2 }))
}

/// Windows 설정의 강조 색. 밝은 테마에는 진한 변형, 어두운 테마에는 밝은 변형을 쓴다 (Windows 11 앱과 같게)
fn windows_accent(dark: bool) -> Option<Color> {
    // AccentPalette: 8가지 색 × RGBA. 1 = 밝게 2단계, 4 = 어둡게 1단계
    let mut buf = [0u8; 32];
    let mut size = 32u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent"),
            w!("AccentPalette"),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if r != ERROR_SUCCESS || size < 32 {
        return None;
    }
    let i = if dark { 4 } else { 16 };
    Some(Color { r: buf[i] as f32 / 255.0, g: buf[i + 1] as f32 / 255.0, b: buf[i + 2] as f32 / 255.0, a: 1.0 })
}

fn mix(a: Color, b: Color, k: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * k, g: a.g + (b.g - a.g) * k, b: a.b + (b.b - a.b) * k, a: 1.0 }
}

/// Windows 설정 > 개인 설정 > 색의 "앱 모드"가 어둡게인지
pub fn system_prefers_dark() -> bool {
    personalize_is_dark(w!("AppsUseLightTheme"))
}

/// "Windows 모드"(작업 표시줄·알림 영역)가 어둡게인지. 트레이 아이콘 색을 고를 때 쓴다
pub fn taskbar_is_dark() -> bool {
    personalize_is_dark(w!("SystemUsesLightTheme"))
}

fn personalize_is_dark(value_name: PCWSTR) -> bool {
    let mut value: u32 = 1;
    let mut size = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            value_name,
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
