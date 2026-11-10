//! Windows 쪽 테마: 팔레트(공통 코드 `palette`)에 Windows 설정(앱 모드, 고대비, 강조 색)을 넣어 테마를 정한다

pub use sakadu::palette::*;

use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor, SYS_COLOR_INDEX,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW};
use windows::core::{PCWSTR, PWSTR, w};

/// Windows 설정에서 읽는 값
struct Win;

impl System for Win {
    fn prefers_dark(&self) -> bool {
        system_prefers_dark()
    }
    fn high_contrast(&self) -> Option<[Color; 7]> {
        if !high_contrast_on() {
            return None;
        }
        let sys = |i: SYS_COLOR_INDEX| from_colorref(unsafe { GetSysColor(i) });
        Some([
            sys(COLOR_WINDOW),
            sys(COLOR_WINDOWTEXT),
            sys(COLOR_HIGHLIGHT),
            sys(COLOR_HIGHLIGHTTEXT),
            sys(COLOR_GRAYTEXT),
            sys(COLOR_HOTLIGHT),
            sys(COLOR_BTNFACE),
        ])
    }
    fn accent(&self, dark: bool) -> Option<Color> {
        windows_accent(dark)
    }
}

/// 설정의 테마와 강조 색에 Windows 설정을 더한 색 (`Theme::resolve` 참고)
pub fn resolve(mode: &str, accent: &str) -> Theme {
    Theme::resolve(mode, accent, &Win)
}

/// 강조 색 키의 색. "system"이면 Windows 강조 색
pub fn accent_color(key: &str, dark: bool) -> Option<Color> {
    sakadu::palette::accent_color(key, dark, &Win)
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
