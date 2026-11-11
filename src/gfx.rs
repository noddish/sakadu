//! Direct2D/DirectWrite 렌더링 래퍼. 좌표는 모두 DIP(96 DPI 기준) 단위

pub use sakadu::draw::{Font, R};

use sakadu::draw::Color;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, RECT};
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Controls::{HIMAGELIST, ILD_NORMAL, ImageList_GetIcon};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetClientRect};
use windows::core::{BOOL, HSTRING, PCWSTR, Result, w};

/// 공통 화면 코드(sakadu::ui)가 쓰는 그리기: 같은 이름의 메서드를 그대로 부른다
impl sakadu::ui::Canvas for Gfx {
    fn fill(&self, r: R, c: Color) {
        Gfx::fill(self, r, c)
    }
    fn fill_round(&self, r: R, radius: f32, c: Color) {
        Gfx::fill_round(self, r, radius, c)
    }
    fn stroke_round(&self, r: R, radius: f32, c: Color, width: f32) {
        Gfx::stroke_round(self, r, radius, c, width)
    }
    fn text(&self, s: &str, r: R, f: Font, c: Color) {
        Gfx::text(self, s, r, f, c)
    }
    fn text_w(&self, s: &[u16], r: R, f: Font, c: Color) {
        Gfx::text_w(self, s, r, f, c)
    }
    fn measure(&self, s: &str, f: Font) -> f32 {
        Gfx::measure(self, s, f)
    }
    fn glyph(&self, code: u16, r: R, small: bool, c: Color) {
        Gfx::glyph(self, code, r, small, c)
    }
    fn glyph_font(&self, code: u16, r: R, f: Font, c: Color) {
        Gfx::glyph_font(self, code, r, f, c)
    }
    fn clip(&self, r: R) {
        Gfx::clip(self, r)
    }
    fn unclip(&self) {
        Gfx::unclip(self)
    }
}

/// Direct2D 사각형
fn d2d(r: R) -> D2D_RECT_F {
    D2D_RECT_F { left: r.l, top: r.t, right: r.r, bottom: r.b }
}

/// Direct2D 색 (공통 `Color`와 배치가 같다)
fn d2c(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// 썸네일 비트맵 캐시 상한. 96 DIP 썸네일(150%에서 약 80KB)이 이 개수까지
const MAX_THUMBS: usize = 360;
/// 썸네일 캐시 픽셀 상한 (4바이트씩, 약 64MB). 아주 큰 아이콘(256 DIP)은 100%에서 250장쯤
const MAX_THUMB_PIXELS: u64 = 16 * 1024 * 1024;

pub struct Gfx {
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    fonts: Vec<IDWriteTextFormat>,
    icons: HashMap<i32, ID2D1Bitmap>,
    /// 키 → (비트맵, 마지막으로 그린 순번)
    thumbs: HashMap<String, (ID2D1Bitmap, u64)>,
    clock: u64,
    imagelist: HIMAGELIST,
    pub dpi: f32,
    text_look: TextLook,
    /// GDI ClearType처럼 획을 픽셀에 맞추는 설정
    sharp_params: Option<IDWriteRenderingParams>,
    /// 시스템 기본(부드럽게). 예전 방식 글자와 아이콘 글리프에 쓴다
    soft_params: Option<IDWriteRenderingParams>,
    /// 페이드에 쓰는 레이어 (렌더 타깃마다 한 번 만든다)
    layer: RefCell<Option<ID2D1Layer>>,
}

/// 글자 그리기 방식 (설정 → 화면 → 글자 표시)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TextLook {
    /// 획을 픽셀에 맞추지 않고 부드럽게 (DirectWrite 기본)
    Soft,
    /// GDI ClearType처럼 획을 픽셀에 맞춘다 (Q-Dir 같은 기존 Windows 앱과 같은 방식)
    Sharp,
    /// Sharp처럼 맞추되 ClearType 색 번짐 없이 회색조로
    Gray,
}

impl TextLook {
    /// 설정 파일 값. 모르는 값이면 회색조
    pub fn from_key(key: &str) -> TextLook {
        match key {
            "soft" => TextLook::Soft,
            "sharp" => TextLook::Sharp,
            _ => TextLook::Gray,
        }
    }
}

fn family_exists(dw: &IDWriteFactory, name: PCWSTR) -> bool {
    unsafe {
        let mut coll = None;
        if dw.GetSystemFontCollection(&mut coll, false).is_err() {
            return false;
        }
        let Some(coll) = coll else { return false };
        let (mut idx, mut exists) = (0u32, BOOL(0));
        coll.FindFamilyName(name, &mut idx, &mut exists).is_ok() && exists.as_bool()
    }
}

/// 설정에서 고른 글꼴이 있으면 그것, 없으면 Windows 11 글꼴(없으면 Windows 10 글꼴)
fn ui_family(dw: &IDWriteFactory, chosen: &str) -> HSTRING {
    let name = HSTRING::from(chosen);
    if !chosen.is_empty() && family_exists(dw, PCWSTR(name.as_ptr())) {
        return name;
    }
    if family_exists(dw, w!("Segoe UI Variable Text")) { HSTRING::from("Segoe UI Variable Text") } else { HSTRING::from("Segoe UI") }
}

impl Gfx {
    /// font: 설정에서 고른 글꼴 이름 (빈 문자열이면 기본)
    pub fn new(dpi: f32, imagelist: HIMAGELIST, font: &str) -> Result<Gfx> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let fonts = build_fonts(&dwrite, &ui_family(&dwrite, font))?;
            // 감마·대비·ClearType 정도는 시스템(ClearType 텍스트 조정) 값을 그대로 쓰고, 그리기 방식만 GDI식으로 바꾼다
            let soft_params = dwrite.CreateRenderingParams().ok();
            let sharp_params = soft_params.as_ref().and_then(|p| {
                dwrite
                    .CreateCustomRenderingParams(p.GetGamma(), p.GetEnhancedContrast(), p.GetClearTypeLevel(), p.GetPixelGeometry(), DWRITE_RENDERING_MODE_GDI_CLASSIC)
                    .ok()
            });
            Ok(Gfx {
                factory,
                dwrite,
                wic,
                rt: None,
                brush: None,
                fonts,
                icons: HashMap::new(),
                thumbs: HashMap::new(),
                clock: 0,
                imagelist,
                dpi,
                text_look: TextLook::Gray,
                sharp_params,
                soft_params,
                layer: RefCell::new(None),
            })
        }
    }

    /// 화면 글꼴을 바꾼다 (빈 문자열이면 기본). 없는 글꼴이면 기본으로
    pub fn set_font(&mut self, font: &str) -> Result<()> {
        self.fonts = build_fonts(&self.dwrite, &ui_family(&self.dwrite, font))?;
        Ok(())
    }

    /// 이 PC에 설치된 글꼴 이름 (사용자 언어 이름이 있으면 그것, 가나다순)
    pub fn installed_fonts(&self) -> Vec<String> {
        let mut out = Vec::new();
        unsafe {
            let mut coll = None;
            if self.dwrite.GetSystemFontCollection(&mut coll, false).is_err() {
                return out;
            }
            let Some(coll) = coll else { return out };
            let mut locale = [0u16; 85];
            if GetUserDefaultLocaleName(&mut locale) == 0 {
                locale[0] = 0;
            }
            for i in 0..coll.GetFontFamilyCount() {
                let Ok(names) = coll.GetFontFamily(i).and_then(|f| f.GetFamilyNames()) else { continue };
                let (mut idx, mut exists) = (0u32, BOOL(0));
                if names.FindLocaleName(PCWSTR(locale.as_ptr()), &mut idx, &mut exists).is_err() || !exists.as_bool() {
                    idx = 0;
                }
                let len = names.GetStringLength(idx).unwrap_or(0) as usize;
                let mut buf = vec![0u16; len + 1];
                if names.GetString(idx, &mut buf).is_ok() {
                    let name = String::from_utf16_lossy(&buf[..len]);
                    if !name.starts_with('@') {
                        out.push(name);
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

fn build_fonts(dwrite: &IDWriteFactory, ui: &HSTRING) -> Result<Vec<IDWriteTextFormat>> {
    unsafe {
        {
            let ui: PCWSTR = PCWSTR(ui.as_ptr());
            let icons = if family_exists(dwrite, w!("Segoe Fluent Icons")) { w!("Segoe Fluent Icons") } else { w!("Segoe MDL2 Assets") };
            let mut locale = [0u16; 85];
            if GetUserDefaultLocaleName(&mut locale) == 0 {
                locale[0] = 0;
            }

            let make = |family: PCWSTR, size: f32, weight: DWRITE_FONT_WEIGHT, align: DWRITE_TEXT_ALIGNMENT, wrap: bool| -> Result<IDWriteTextFormat> {
                let f = dwrite.CreateTextFormat(
                    family,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    PCWSTR(locale.as_ptr()),
                )?;
                f.SetTextAlignment(align)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                if wrap {
                    f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                } else {
                    f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                    let sign = dwrite.CreateEllipsisTrimmingSign(&f)?;
                    let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                    f.SetTrimming(&trim, &sign)?;
                }
                Ok(f)
            };
            let path = make(ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING, false)?;
            let sign = dwrite.CreateEllipsisTrimmingSign(&path)?;
            let keep_last = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: '\\' as u32, delimiterCount: 1 };
            path.SetTrimming(&keep_last, &sign)?;
            let caption = make(ui, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER, true)?;
            caption.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            // 긴 파일 이름은 공백이 없어도 글자 단위로 끊는다
            caption.SetWordWrapping(DWRITE_WORD_WRAPPING_EMERGENCY_BREAK)?;
            let sign = dwrite.CreateEllipsisTrimmingSign(&caption)?;
            let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
            caption.SetTrimming(&trim, &sign)?;
            let mono_family = if family_exists(dwrite, w!("Cascadia Mono")) { w!("Cascadia Mono") } else { w!("Consolas") };
            let mono = make(mono_family, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING, true)?;
            mono.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            mono.SetWordWrapping(DWRITE_WORD_WRAPPING_EMERGENCY_BREAK)?;
            // Font 순서와 같게
            Ok(vec![
                make(ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING, false)?,
                make(ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_TRAILING, false)?,
                make(ui, 13.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING, false)?,
                make(icons, 16.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER, false)?,
                make(icons, 10.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER, false)?,
                make(ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER, true)?,
                path,
                caption,
                mono,
                make(ui, 24.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING, false)?,
                make(ui, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING, false)?,
                make(icons, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER, false)?,
            ])
        }
    }
}

impl Gfx {
    /// 그리기 시작. 렌더 타깃이 없으면(최초·장치 손실 후) 새로 만든다
    pub fn begin(&mut self, hwnd: HWND) -> bool {
        unsafe {
            if self.rt.is_none() {
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_UNKNOWN, alphaMode: D2D1_ALPHA_MODE_UNKNOWN },
                    dpiX: self.dpi,
                    dpiY: self.dpi,
                    usage: D2D1_RENDER_TARGET_USAGE_NONE,
                    minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                };
                let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd,
                    pixelSize: D2D_SIZE_U { width: rc.right.max(1) as u32, height: rc.bottom.max(1) as u32 },
                    presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                };
                let Ok(rt) = self.factory.CreateHwndRenderTarget(&props, &hprops) else { return false };
                let black = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
                let Ok(brush) = rt.CreateSolidColorBrush(&black, None) else { return false };
                self.rt = Some(rt);
                self.brush = Some(brush);
                self.apply_text_look();
            }
            self.rt.as_ref().unwrap().BeginDraw();
        }
        true
    }

    pub fn end(&mut self) {
        if let Some(rt) = &self.rt {
            if let Err(e) = unsafe { rt.EndDraw(None, None) } {
                if e.code() == D2DERR_RECREATE_TARGET {
                    self.discard();
                }
            }
        }
    }

    pub fn discard(&mut self) {
        *self.layer.get_mut() = None;
        self.icons.clear();
        self.thumbs.clear();
        self.brush = None;
        self.rt = None;
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(rt) = &self.rt {
            unsafe {
                let _ = rt.Resize(&D2D_SIZE_U { width: width.max(1), height: height.max(1) });
            }
        }
    }

    pub fn set_dpi(&mut self, dpi: f32) {
        self.dpi = dpi;
        // 아이콘·썸네일 비트맵은 DPI에 따라 크기가 달라지므로 새로 만든다
        self.icons.clear();
        self.thumbs.clear();
        if let Some(rt) = &self.rt {
            unsafe { rt.SetDpi(dpi, dpi) };
        }
    }

    fn rt(&self) -> &ID2D1HwndRenderTarget {
        self.rt.as_ref().expect("begin() 이후에만 호출")
    }

    fn brush(&self, c: Color) -> &ID2D1SolidColorBrush {
        let b = self.brush.as_ref().expect("begin() 이후에만 호출");
        unsafe { b.SetColor(&d2c(c)) };
        b
    }

    pub fn clear(&self, c: Color) {
        unsafe { self.rt().Clear(Some(&d2c(c))) };
    }

    pub fn fill(&self, r: R, c: Color) {
        unsafe { self.rt().FillRectangle(&d2d(r), self.brush(c)) };
    }

    pub fn fill_round(&self, r: R, radius: f32, c: Color) {
        let rr = D2D1_ROUNDED_RECT { rect: d2d(r), radiusX: radius, radiusY: radius };
        unsafe { self.rt().FillRoundedRectangle(&rr, self.brush(c)) };
    }

    pub fn stroke_round(&self, r: R, radius: f32, c: Color, width: f32) {
        let rr = D2D1_ROUNDED_RECT { rect: d2d(r), radiusX: radius, radiusY: radius };
        unsafe { self.rt().DrawRoundedRectangle(&rr, self.brush(c), width, None) };
    }

    pub fn text(&self, s: &str, r: R, f: Font, c: Color) {
        let w: Vec<u16> = s.encode_utf16().collect();
        self.text_w(&w, r, f, c);
    }

    pub fn text_w(&self, s: &[u16], r: R, f: Font, c: Color) {
        if self.text_look == TextLook::Soft {
            self.draw(s, r, f, c, DWRITE_MEASURING_MODE_NATURAL);
            return;
        }
        // 글자 시작·끝을 픽셀 경계에 맞춰야 GDI식 그리기에서 획이 두 픽셀로 번지지 않는다
        let k = self.dpi / 96.0;
        let snap = |v: f32| (v * k).round() / k;
        self.draw(s, R::new(snap(r.l), r.t, snap(r.r), r.b), f, c, DWRITE_MEASURING_MODE_GDI_CLASSIC);
    }

    /// 글자 그리기 방식을 바꾼다. 렌더 타깃이 있으면 바로 적용
    pub fn set_text_look(&mut self, look: TextLook) {
        self.text_look = look;
        self.apply_text_look();
    }

    fn apply_text_look(&self) {
        let Some(rt) = &self.rt else { return };
        let params = if self.text_look == TextLook::Soft { &self.soft_params } else { &self.sharp_params };
        let aa = if self.text_look == TextLook::Gray { D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE } else { D2D1_TEXT_ANTIALIAS_MODE_DEFAULT };
        unsafe {
            if let Some(p) = params {
                rt.SetTextRenderingParams(p);
            }
            rt.SetTextAntialiasMode(aa);
        }
    }

    fn draw(&self, s: &[u16], r: R, f: Font, c: Color, mode: DWRITE_MEASURING_MODE) {
        if s.is_empty() || r.w() <= 0.0 {
            return;
        }
        unsafe {
            self.rt().DrawText(s, &self.fonts[f as usize], &d2d(r), self.brush(c), D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, mode)
        };
    }

    /// 아이콘 글리프는 ClearType 색 번짐이 눈에 띄므로 회색조로, 힌팅이 거의 없는 아이콘 글꼴이라 원래의 부드러운 방식으로 그린다
    /// 아이콘 글리프. small이면 장식 크기(10px), 아니면 도구 아이콘 크기(16px)
    pub fn glyph(&self, code: u16, r: R, small: bool, c: Color) {
        self.glyph_font(code, r, if small { Font::GlyphSmall } else { Font::Glyph }, c);
    }

    /// 크기를 직접 고른 아이콘 글리프 (Glyph·GlyphMid·GlyphSmall)
    pub fn glyph_font(&self, code: u16, r: R, f: Font, c: Color) {
        use sakadu::ui::glyph::{G_BOOKMARK, G_BOOKMARK_FILL};
        if code == G_BOOKMARK || code == G_BOOKMARK_FILL {
            self.bookmark(r, f, code == G_BOOKMARK_FILL, c);
            return;
        }
        let rt = self.rt();
        unsafe {
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            if let Some(p) = &self.soft_params {
                rt.SetTextRenderingParams(p);
            }
        }
        self.draw(&[code], r, f, c, DWRITE_MEASURING_MODE_NATURAL);
        self.apply_text_look();
    }

    /// 책갈피: 세로로 긴 띠, 아래가 V로 파인 모양. Segoe 아이콘 글꼴에 없어서 도형으로 그린다 (r 가운데, 글리프와 같은 크기)
    fn bookmark(&self, r: R, f: Font, filled: bool, c: Color) {
        let size = match f {
            Font::GlyphSmall => 10.0,
            Font::GlyphMid => 12.0,
            _ => 16.0,
        };
        let (w, h) = (size * 0.56, size * 0.78);
        let (cx, cy) = ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0);
        let (l, t, rr, b) = (cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0);
        let pt = |x: f32, y: f32| windows_numerics::Vector2 { X: x, Y: y };
        unsafe {
            let Ok(geo) = self.factory.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(pt(l, t), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&[pt(rr, t), pt(rr, b), pt(cx, b - w * 0.42), pt(l, b)]);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_err() {
                return;
            }
            // 모서리를 둥글게 이어 다른 아이콘의 선과 결을 맞춘다
            let props = D2D1_STROKE_STYLE_PROPERTIES { lineJoin: D2D1_LINE_JOIN_ROUND, ..Default::default() };
            let style = self.factory.CreateStrokeStyle(&props, None).ok();
            let brush = self.brush(c);
            if filled {
                self.rt().FillGeometry(&geo, brush, None);
            }
            self.rt().DrawGeometry(&geo, brush, size * 0.085, style.as_ref());
        }
    }

    /// 그릴 때와 같은 방식으로 배치한다. 너비 재기와 그리기가 어긋나지 않게
    fn text_layout(&self, s: &[u16], f: Font, w: f32, h: f32) -> Option<IDWriteTextLayout> {
        let format = &self.fonts[f as usize];
        unsafe {
            if self.text_look == TextLook::Soft {
                self.dwrite.CreateTextLayout(s, format, w, h).ok()
            } else {
                // GDI식: 글자 간격을 픽셀 단위로
                self.dwrite.CreateGdiCompatibleTextLayout(s, format, w, h, self.dpi / 96.0, None, false).ok()
            }
        }
    }

    pub fn measure(&self, s: &str, f: Font) -> f32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Some(layout) = self.text_layout(&w, f, 10000.0, 100.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_err() {
                return 0.0;
            }
            m.widthIncludingTrailingWhitespace
        }
    }

    /// 이어서 그리는 것을 투명도 opacity로 겹치고 (dx, dy)만큼 옮긴다. true를 받았으면 pop_fade로 닫는다
    pub fn push_fade(&self, opacity: f32, dx: f32, dy: f32) -> bool {
        let mut layer = self.layer.borrow_mut();
        if layer.is_none() {
            *layer = unsafe { self.rt().CreateLayer(None) }.ok();
        }
        let Some(l) = layer.as_ref() else { return false };
        let params = D2D1_LAYER_PARAMETERS {
            contentBounds: D2D_RECT_F { left: -1.0e6, top: -1.0e6, right: 1.0e6, bottom: 1.0e6 },
            geometricMask: ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: windows_numerics::Matrix3x2::identity(),
            opacity: opacity.clamp(0.0, 1.0),
            opacityBrush: ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS_NONE,
        };
        unsafe {
            self.rt().PushLayer(&params, l);
            self.rt().SetTransform(&windows_numerics::Matrix3x2::translation(dx, dy));
        }
        true
    }

    pub fn pop_fade(&self) {
        unsafe {
            self.rt().SetTransform(&windows_numerics::Matrix3x2::identity());
            self.rt().PopLayer();
        }
    }

    pub fn clip(&self, r: R) {
        unsafe { self.rt().PushAxisAlignedClip(&d2d(r), D2D1_ANTIALIAS_MODE_ALIASED) };
    }

    pub fn unclip(&self) {
        unsafe { self.rt().PopAxisAlignedClip() };
    }

    /// 시스템 이미지 목록의 아이콘을 그린다. 변환한 비트맵은 인덱스별로 캐시
    pub fn icon(&mut self, index: i32, r: R) {
        if index < 0 || self.rt.is_none() {
            return;
        }
        if !self.icons.contains_key(&index) {
            match self.make_icon(index) {
                Some(b) => {
                    self.icons.insert(index, b);
                }
                None => return,
            }
        }
        let bmp = &self.icons[&index];
        unsafe {
            self.rt().DrawBitmap(bmp, Some(&d2d(r)), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
        }
    }

    /// 백그라운드에서 만든 픽셀(미리 곱한 BGRA)을 비트맵으로 올린다. 렌더 타깃이 아직 없으면 false
    pub fn add_thumb(&mut self, key: String, w: u32, h: u32, pixels: &[u8]) -> bool {
        let Some(rt) = &self.rt else { return false };
        if pixels.len() < (w * h * 4) as usize {
            return false;
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let Ok(bmp) = (unsafe { rt.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(pixels.as_ptr() as *const _), w * 4, &props) }) else {
            return false;
        };
        // 개수나 픽셀 양(아주 큰 아이콘은 한 장이 크다)이 넘치면 가장 오래 안 그린 4분의 1을 버린다
        let pixels_of = |b: &ID2D1Bitmap| {
            let s = unsafe { b.GetPixelSize() };
            s.width as u64 * s.height as u64
        };
        let total: u64 = self.thumbs.values().map(|v| pixels_of(&v.0)).sum();
        if !self.thumbs.is_empty() && (self.thumbs.len() >= MAX_THUMBS || total + w as u64 * h as u64 > MAX_THUMB_PIXELS) {
            let mut ages: Vec<u64> = self.thumbs.values().map(|v| v.1).collect();
            ages.sort_unstable();
            let cut = ages[ages.len() / 4];
            self.thumbs.retain(|_, v| v.1 > cut);
        }
        self.clock += 1;
        self.thumbs.insert(key, (bmp, self.clock));
        true
    }

    /// 썸네일을 r 안에 비율을 지켜 가운데 그린다(키우지는 않는다). 캐시에 없으면 false
    pub fn thumb(&mut self, key: &str, r: R, opacity: f32) -> bool {
        self.clock += 1;
        let clock = self.clock;
        let Some((bmp, used)) = self.thumbs.get_mut(key) else { return false };
        *used = clock;
        let px = unsafe { bmp.GetPixelSize() };
        let scale = self.dpi / 96.0;
        let (nw, nh) = (px.width as f32 / scale, px.height as f32 / scale);
        let k = (r.w() / nw).min(r.h() / nh).min(1.0);
        let (w, h) = (nw * k, nh * k);
        let (x, y) = ((r.l + r.r - w) / 2.0, (r.t + r.b - h) / 2.0);
        let dst = R::new(x.round(), y.round(), (x + w).round(), (y + h).round());
        let bmp = bmp.clone();
        unsafe {
            self.rt().DrawBitmap(&bmp, Some(&d2d(dst)), opacity, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
        }
        true
    }

    /// 긴 글(미리 보기)을 매번 다시 배치하지 않도록 레이아웃을 만들어 둔다
    pub fn layout(&self, s: &str, f: Font, w: f32) -> Option<IDWriteTextLayout> {
        let w16: Vec<u16> = s.encode_utf16().collect();
        self.text_layout(&w16, f, w.max(1.0), 1.0e6)
    }

    pub fn layout_height(l: &IDWriteTextLayout) -> f32 {
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { l.GetMetrics(&mut m) }.map(|_| m.height).unwrap_or(0.0)
    }

    pub fn draw_layout(&self, l: &IDWriteTextLayout, x: f32, y: f32, c: Color) {
        unsafe {
            self.rt().DrawTextLayout(windows_numerics::Vector2 { X: x, Y: y }, l, self.brush(c), D2D1_DRAW_TEXT_OPTIONS_CLIP);
        }
    }

    fn make_icon(&self, index: i32) -> Option<ID2D1Bitmap> {
        unsafe {
            let hicon = ImageList_GetIcon(self.imagelist, index, ILD_NORMAL);
            if hicon.is_invalid() {
                return None;
            }
            let bmp = (|| -> Result<ID2D1Bitmap> {
                let src = self.wic.CreateBitmapFromHICON(hicon)?;
                let conv = self.wic.CreateFormatConverter()?;
                conv.Initialize(&src, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
                self.rt().CreateBitmapFromWicBitmap(&conv, None)
            })();
            let _ = DestroyIcon(hicon);
            bmp.ok()
        }
    }
}
