//! 라이트/다크/고대비 색상 팔레트와 시스템 앱 테마 감지.
//! 칸·탭·파일 줄·메뉴의 상태 표시(활성 칸의 지금 탭, 선택, 키보드 포커스 등)는 모두 여기 값을 쓴다

use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor, SYS_COLOR_INDEX,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW};
use windows::core::{PCWSTR, PWSTR, w};

pub type Color = D2D1_COLOR_F;

/// 둥글기: 팝업, 파일 줄·메뉴 줄. 크게 둥글리지 않는다 (칸 카드는 app.rs RADIUS)
pub const R_PANE: f32 = 6.0;
pub const R_ROW: f32 = 4.0;
/// 키보드 포커스 테두리 두께
pub const FOCUS_W: f32 = 1.25;

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
    /// Windows 고대비 모드 (시스템 색을 그대로 쓴다)
    pub high_contrast: bool,
    pub window: Color,
    pub pane: Color,
    pub strip: Color,
    pub field: Color,
    pub text: Color,
    pub dim: Color,
    pub faint: Color,
    pub border: Color,
    pub hover: Color,
    /// 활성 칸에서 선택한 파일 (바탕·글자)
    pub sel: Color,
    pub sel_text: Color,
    /// 비활성 칸에 남아 있는 선택
    pub sel_inactive: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub thumb: Color,
    /// 모든 칸의 경계. 활성 여부와 상관없이 같은 색·두께라 칸을 옮겨도 화면이 흔들리지 않는다
    pub pane_border: Color,
    /// 활성 칸의 지금 탭 칩 글자: 옅은 강조 색(accent_soft) 바탕 위에서 또렷한 강조 색.
    /// 지금 작업 중인 칸을 이 칩과 카드 그림자로 알아본다
    pub chip_text: Color,
    /// 키보드 포커스 테두리
    pub focus: Color,
}

impl Theme {
    pub const LIGHT: Theme = Theme {
        dark: false,
        high_contrast: false,
        window: hex(0xEDEFF2),
        pane: hex(0xFFFFFF),
        strip: hex(0xF4F5F7),
        field: hex(0xF0F2F5),
        text: hex(0x1B1C1F),
        dim: hex(0x5F636B),
        faint: hex(0x9EA2AA),
        border: hex(0xDCDFE4),
        hover: hex(0xF0F1F3),
        sel: hex(0xD5E4FD),
        sel_text: hex(0x1B1C1F),
        sel_inactive: hex(0xE6E8EC),
        accent: hex(0x2F6FEB),
        accent_soft: hex(0xE1EAFC),
        thumb: hex(0xC4C8CF),
        pane_border: hex(0xD5D8DD),
        chip_text: hex(0x2F6FEB),
        focus: hex(0x6A6F78),
    };

    /// 라이트를 뒤집지 않고 어두운 바탕에서의 명암에 맞췄다: 활성 칸의 지금 탭은 밝은 회색, 선택은 어두운 파랑
    pub const DARK: Theme = Theme {
        dark: true,
        high_contrast: false,
        window: hex(0x141518),
        pane: hex(0x1E1F23),
        strip: hex(0x18191C),
        field: hex(0x2A2C31),
        text: hex(0xE7E8EB),
        dim: hex(0xA0A3AB),
        faint: hex(0x6E717A),
        border: hex(0x2E3036),
        hover: hex(0x27292E),
        sel: hex(0x23406F),
        sel_text: hex(0xF2F4F7),
        sel_inactive: hex(0x30333A),
        accent: hex(0x5B9BFF),
        accent_soft: hex(0x1F3354),
        thumb: hex(0x4B4E56),
        pane_border: hex(0x34363C),
        chip_text: hex(0x5B9BFF),
        focus: hex(0xA3A8B2),
    };

    /// 고대비 모드: 시스템 색으로만 만든다. 활성 칸의 지금 탭과 선택은 강조(Highlight), 포커스는 HotLight 색
    fn high_contrast(window: Color, text: Color, highlight: Color, highlight_text: Color, gray: Color, hot: Color, face: Color) -> Theme {
        let dark = window.r * 0.299 + window.g * 0.587 + window.b * 0.114 < 0.5;
        let soft = mix(window, text, 0.18);
        Theme {
            dark,
            high_contrast: true,
            window,
            pane: window,
            strip: face,
            field: window,
            text,
            dim: text,
            faint: gray,
            border: text,
            hover: soft,
            sel: highlight,
            sel_text: highlight_text,
            sel_inactive: mix(window, text, 0.3),
            accent: highlight,
            accent_soft: mix(window, highlight, 0.3),
            thumb: text,
            pane_border: text,
            chip_text: text,
            focus: hot,
        }
    }

    /// 설정의 테마("system"·"light"·"dark")와 강조 색(ACCENTS의 키)에 맞춘 색. Windows 고대비 모드면 그 색을 따른다.
    /// SAKADU_THEME=dark|light|contrast가 있으면 테마는 그것을 따른다 (스크린샷·시험용, contrast는 '고대비 검정' 색)
    pub fn resolve(mode: &str, accent: &str) -> Theme {
        let forced = std::env::var("SAKADU_THEME").ok();
        if forced.as_deref() == Some("contrast") {
            return Self::high_contrast(hex(0x000000), hex(0xFFFFFF), hex(0x1AEBFF), hex(0x000000), hex(0x3FF23F), hex(0xFFFF00), hex(0x000000));
        }
        if forced.is_none() && high_contrast_on() {
            let sys = |i: SYS_COLOR_INDEX| from_colorref(unsafe { GetSysColor(i) });
            return Self::high_contrast(
                sys(COLOR_WINDOW),
                sys(COLOR_WINDOWTEXT),
                sys(COLOR_HIGHLIGHT),
                sys(COLOR_HIGHLIGHTTEXT),
                sys(COLOR_GRAYTEXT),
                sys(COLOR_HOTLIGHT),
                sys(COLOR_BTNFACE),
            );
        }
        let dark = match forced.as_deref() {
            Some("dark") => true,
            Some("light") => false,
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
        // 활성 칸의 지금 탭 칩 글자도 강조 색을 따르되, 옅은 강조 색 바탕 위에서 또렷하게
        t.chip_text = chip_text(t.accent, t.accent_soft, t.text);
        t
    }
}

/// 글자와 바탕의 체감 대비 (APCA Lc, 절댓값 0~106쯤). WCAG 대비는 청록·주황 같은 중간 밝기 색에
/// 검은 글자를 고르는 경향이 있어, 사람이 실제로 읽기 쉬운 쪽에 가까운 APCA를 쓴다
pub fn apca(text: Color, bg: Color) -> f32 {
    let y = |c: Color| {
        let y = 0.2126729 * c.r.powf(2.4) + 0.7151522 * c.g.powf(2.4) + 0.0721750 * c.b.powf(2.4);
        if y < 0.022 { y + (0.022 - y).powf(1.414) } else { y }
    };
    let (yt, yb) = (y(text), y(bg));
    let s = if yb > yt { (yb.powf(0.56) - yt.powf(0.57)) * 1.14 } else { (yb.powf(0.65) - yt.powf(0.62)) * 1.14 };
    if s.abs() < 0.1 { 0.0 } else { (s.abs() - 0.027) * 100.0 }
}

/// 탭 칩 글자(13px 굵게)에 필요한 체감 대비
const CHIP_LC: f32 = 60.0;

/// 활성 칸의 지금 탭 칩 글자: 옅은 강조 색(soft) 바탕 위의 강조 색. 충분히 또렷하지 않으면
/// 본문 글자 색(ink) 쪽으로 조금씩 옮긴다 (밝은 테마는 어둡게, 어두운 테마는 밝게)
pub fn chip_text(accent: Color, soft: Color, ink: Color) -> Color {
    let mut fg = accent;
    for _ in 0..12 {
        if apca(fg, soft) >= CHIP_LC {
            break;
        }
        fg = mix(fg, ink, 0.08);
    }
    fg
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

/// HIGHCONTRASTW (SPI_GETHIGHCONTRAST)
#[repr(C)]
struct HighContrast {
    size: u32,
    flags: u32,
    scheme: PWSTR,
}

/// Windows 고대비 모드가 켜져 있는지
pub fn high_contrast_on() -> bool {
    let mut hc = HighContrast { size: size_of::<HighContrast>() as u32, flags: 0, scheme: PWSTR::null() };
    let ok = unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.size, Some(&mut hc as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    // HCF_HIGHCONTRASTON
    ok.is_ok() && hc.flags & 1 != 0
}

fn from_colorref(v: u32) -> Color {
    Color { r: (v & 0xFF) as f32 / 255.0, g: ((v >> 8) & 0xFF) as f32 / 255.0, b: ((v >> 16) & 0xFF) as f32 / 255.0, a: 1.0 }
}

pub fn colorref(c: Color) -> COLORREF {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(ch(c.r) | (ch(c.g) << 8) | (ch(c.b) << 16))
}

pub fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 지금 탭 칩 글자: 어떤 강조 색이든 옅은 강조 색 바탕 위에서 체감 대비(APCA) 60 이상이고, 강조 색에서 크게 벗어나지 않는다
    #[test]
    fn chip_text_stays_readable_on_every_accent() {
        for dark in [false, true] {
            let base = if dark { Theme::DARK } else { Theme::LIGHT };
            for a in &ACCENTS[1..] {
                let c = hex(if dark { a.3 } else { a.2 });
                let soft = mix(base.pane, c, if dark { 0.22 } else { 0.12 });
                let fg = chip_text(c, soft, base.text);
                assert!(apca(fg, soft) >= CHIP_LC, "{} dark={dark} {:?} Lc {}", a.0, fg, apca(fg, soft));
                assert!((fg.r - c.r).abs() < 0.5 && (fg.g - c.g).abs() < 0.5 && (fg.b - c.b).abs() < 0.5, "{} too far", a.0);
            }
        }
        // APCA 기준값: 흰 바탕 검은 글자는 100 안팎, 같은 색끼리는 0
        let white = hex(0xFFFFFF);
        assert!(apca(hex(0x000000), white) > 100.0);
        assert_eq!(apca(white, white), 0.0);
    }
}
