//! Core Graphics·Core Text 그리기. Windows `gfx.rs`(Direct2D)와 같은 이름·좌표(왼쪽 위가 0, pt 단위)를 쓴다.
//! 지금은 비트맵(PNG로 저장)에 그리고, 다음 단계에서 창(NSView)에 그릴 때도 같은 함수를 쓴다

use block2::RcBlock;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{ClassType, msg_send};
use objc2_app_kit::{NSCompositingOperation, NSFont, NSFontWeightRegular, NSFontWeightSemibold, NSGraphicsContext, NSImage, NSImageSymbolConfiguration, NSWorkspace};
use objc2_core_foundation::{CFAttributedString, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CFURL, CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize, kCFBooleanTrue};
use objc2_core_graphics::{CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, CGInterpolationQuality, CGPath, kCGColorSpaceSRGB};
use objc2_core_text::{CTFont, CTLine, CTLineTruncationType, kCTFontAttributeName, kCTForegroundColorFromContextAttributeName};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURL, NSURLLocalizedTypeDescriptionKey};
use objc2_image_io::{
    CGImageDestination, CGImageSource, kCGImagePropertyGIFDelayTime, kCGImagePropertyGIFDictionary, kCGImagePropertyGIFLoopCount, kCGImagePropertyGIFUnclampedDelayTime,
    kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceShouldCacheImmediately, kCGImageSourceThumbnailMaxPixelSize,
};
use sakadu::draw::{Color, Font, R};
use std::ffi::CStr;
use std::ptr::NonNull;
use std::sync::{OnceLock, mpsc};
use std::time::Duration;

/// 고정폭 글(글 파일 미리 보기)의 줄 높이
pub const MONO_LINE_H: f32 = 17.0;

/// Quick Look 썸네일 생성기가 든 프레임워크. 실행 파일에 링크하지 않고 처음 쓸 때 연다 (열지 못해도 앱은 뜬다)
const QL_THUMBNAILING: &CStr = c"/System/Library/Frameworks/QuickLookThumbnailing.framework/QuickLookThumbnailing";
/// QLThumbnailGenerationRequestRepresentationTypeThumbnail: 파일 내용으로 만든 썸네일만 받는다.
/// 파일 아이콘은 받지 않는다 (썸네일이 없는 파일은 지금처럼 사카두가 아이콘을 그린다)
const QL_THUMBNAIL: usize = 1 << 2;
/// 썸네일 하나를 기다리는 가장 긴 시간. 넘으면 요청을 거두고 만들지 못한 것으로 본다 (답이 없는 NAS의 파일 같은 것)
const QL_WAIT: Duration = Duration::from_secs(15);

/// 움직이는 gif를 다 풀어 두는 한도: 장이 이보다 많으면 첫 장만 보인다
pub const FRAMES_MAX: usize = 400;
/// 보이는 크기(긴 쪽을 요청한 픽셀 수까지 줄인 것)로 푼 장들이 차지할 메모리의 한도. 넘으면 첫 장만 보인다
pub const FRAMES_BYTES_MAX: usize = 48 << 20;

/// 읽어 둔 그림. 뒷줄에서 읽어(Gfx::load_picture) 메인 줄에서 그린다 (CGImage는 만든 뒤 바뀌지 않아 줄 사이로 넘겨도 된다)
#[derive(Clone)]
pub struct Picture {
    img: CFRetained<CGImage>,
}

/// 움직이는 gif의 장들과 장마다 머무는 시간 (같은 수. 첫 장만 담았으면 시간은 비어 있다)
pub struct Frames {
    pub pics: Vec<Picture>,
    pub delays: Vec<Duration>,
}

/// 장들을 다 풀어 둘 만한가: 둘 이상이고, FRAMES_MAX장 이하이고, 한 장이 w×h 픽셀(보이는 크기)일 때 다 합쳐 FRAMES_BYTES_MAX 이하
pub fn frames_fit(count: usize, w: usize, h: usize) -> bool {
    (2..=FRAMES_MAX).contains(&count) && count.saturating_mul(w).saturating_mul(h).saturating_mul(4) <= FRAMES_BYTES_MAX
}

/// 장에 머무는 시간 (gif에 적힌 초). 20ms보다 짧거나 적혀 있지 않으면 100ms로 본다: 0으로 적은 gif가 흔하고, 브라우저도 그렇게 늦춘다.
/// gif에 적을 수 있는 가장 긴 시간(655.35초)을 넘지 않는다
pub fn frame_delay_of(secs: Option<f64>) -> Duration {
    match secs {
        Some(s) if s >= 0.02 => Duration::from_secs_f64(s.min(655.35)),
        _ => Duration::from_millis(100),
    }
}

impl Picture {
    /// 가로·세로 픽셀 수
    pub fn size(&self) -> (usize, usize) {
        (CGImage::width(Some(&self.img)), CGImage::height(Some(&self.img)))
    }

    /// 긴 쪽의 픽셀 수
    pub fn longest(&self) -> u32 {
        let (w, h) = self.size();
        w.max(h) as u32
    }

    /// 시험용: 빈 그림 (파일을 읽지 않고 만든다)
    #[cfg(test)]
    pub fn blank(w: usize, h: usize) -> Option<Picture> {
        let g = Gfx::bitmap(w as f32, h as f32, 1.0)?;
        CGBitmapContextCreateImage(g.c()).map(|img| Picture { img })
    }

    /// 시험용: 그림 한가운데 픽셀의 색 [빨강, 초록, 파랑, 불투명도]. 투명한 그림판에 제 크기로 그려 읽는다
    #[cfg(test)]
    pub fn center_color(&self) -> Option<[u8; 4]> {
        let (w, h) = self.size();
        let g = Gfx::bitmap(w as f32, h as f32, 1.0)?;
        g.picture(self, R::new(0.0, 0.0, w as f32, h as f32));
        let data = objc2_core_graphics::CGBitmapContextGetData(g.c()) as *const u8;
        if data.is_null() {
            return None;
        }
        let at = (h / 2) * objc2_core_graphics::CGBitmapContextGetBytesPerRow(g.c()) + (w / 2) * 4;
        // SAFETY: 그림판의 픽셀은 한 줄의 바이트 수 × h이고, at은 가운데 줄의 가운데 픽셀(4바이트)이라 그 안이다
        let px = unsafe { std::slice::from_raw_parts(data.add(at), 4) };
        Some([px[0], px[1], px[2], px[3]])
    }
}

pub struct Gfx {
    ctx: CFRetained<CGContext>,
    /// AppKit 그림(심벌·파일 아이콘)을 같은 비트맵에 그리기 위한 틀
    ns: Retained<NSGraphicsContext>,
    /// Font 순서와 같다
    fonts: Vec<Retained<NSFont>>,
}

fn rect(r: R) -> CGRect {
    CGRect::new(CGPoint::new(r.l as CGFloat, r.t as CGFloat), CGSize::new(r.w().max(0.0) as CGFloat, r.h().max(0.0) as CGFloat))
}

/// Windows 아이콘 글꼴(Segoe Fluent Icons) 글자를 SF Symbols 이름으로. 화면 코드는 Windows와 같은 번호를 쓴다
fn symbol_name(code: u16) -> &'static str {
    match code {
        0xE72B => "arrow.left",
        0xE72A => "arrow.right",
        0xE74A => "arrow.up",
        0xE710 => "plus",
        0xE8BB => "xmark",
        0xE76B => "chevron.left",
        0xE76C => "chevron.right",
        0xE70D => "chevron.down",
        0xE70E => "chevron.up",
        0xE734 => "star",
        0xE735 => "star.fill",
        0xE8F4 => "folder.badge.plus",
        0xE7B3 => "eye",
        0xE71C => "line.3.horizontal.decrease",
        0xE8FD => "list.bullet",
        0xF0E2 => "square.grid.2x2",
        0xE713 => "gearshape",
        0xE712 => "ellipsis",
        0xE90D => "sidebar.right",
        0xE721 => "magnifyingglass",
        // 체크 표시 (검색 켜기 카드의 고르기 칸)
        0xE73E => "checkmark",
        0xE8B7 => "folder",
        0xE718 => "pin",
        0xE841 => "pin.fill",
        0xE8CB => "arrow.up.arrow.down",
        // 책갈피 (ui::glyph::G_BOOKMARK, G_BOOKMARK_FILL)
        0xFFF0 => "bookmark",
        0xFFF1 => "bookmark.fill",
        // 자동 미리 보기의 눈, 미리 보기 카드의 크게 보기·작게 보기 (ui::glyph::G_EYE, G_EXPAND, G_SHRINK)
        0xE890 => "eye",
        // 숨김 항목을 감춘 눈, 자동 미리 보기 단추 (ui::glyph::G_EYE_OFF, G_AUTO_PREVIEW)
        0xED1A => "eye.slash",
        0xE8A1 => "doc.text.magnifyingglass",
        0xE740 => "arrow.up.left.and.arrow.down.right",
        0xE73F => "arrow.down.right.and.arrow.up.left",
        // 미리 보기 창 머리의 '기본 앱으로 열기' (Windows preview.rs의 G_OPEN)
        0xE8A7 => "arrow.up.right.square",
        _ => "questionmark.square.dashed",
    }
}

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
        // Core Text 한 줄 그리기라, 여러 줄 글(목록 안내)은 줄마다 20pt씩 내려 그린다.
        // 고정폭 글(글 파일 미리 보기)은 줄 사이를 좁힌다. 긴 줄은 넘기지 않고 끝을 줄인다
        if s.contains('\n') {
            let line_h = if matches!(f, Font::Mono) { MONO_LINE_H } else { 20.0 };
            for (i, line) in s.lines().enumerate() {
                let y = r.t + i as f32 * line_h;
                Gfx::text(self, line, R::new(r.l, y, r.r, y + line_h), f, c);
            }
            return;
        }
        Gfx::text(self, s, r, f, c)
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

impl Gfx {
    /// 너비 w × 높이 h(pt) 그림판. scale배 픽셀로 그린다 (레티나는 2)
    pub fn bitmap(w: f32, h: f32, scale: f32) -> Option<Gfx> {
        let (pw, ph) = ((w * scale).round() as usize, (h * scale).round() as usize);
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
        let ctx = unsafe { CGBitmapContextCreate(std::ptr::null_mut(), pw, ph, 8, pw * 4, Some(&space), CGImageAlphaInfo::PremultipliedLast.0) }?;
        // 왼쪽 위를 0으로, pt 단위로
        CGContext::translate_ctm(Some(&ctx), 0.0, ph as CGFloat);
        CGContext::scale_ctm(Some(&ctx), scale as CGFloat, -scale as CGFloat);
        Some(Self::with(ctx))
    }

    /// 창(위아래를 뒤집은 NSView)의 drawRect: 안에서 지금 그림판에 그린다
    pub fn for_context(ctx: &CGContext) -> Gfx {
        Self::with(unsafe { CFRetained::retain(std::ptr::NonNull::from(ctx)) })
    }

    fn with(ctx: CFRetained<CGContext>) -> Gfx {
        // 위아래를 뒤집은 좌표에서 글자가 거꾸로 서지 않게
        CGContext::set_text_matrix(Some(&ctx), CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: -1.0, tx: 0.0, ty: 0.0 });
        let ns = NSGraphicsContext::graphicsContextWithCGContext_flipped(&ctx, true);
        // 글꼴은 한 번에 한 줄에서만 만든다. 시험은 여러 줄에서 한꺼번에 그림판을 만드는데, 그때 맥이 가끔 글꼴을 주지 못했다
        // (CI에서 monospacedSystemFontOfSize가 빈 값을 돌려줘 시험이 멈춘 적이 있다). 창에서는 메인 줄 하나만 만든다
        static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _making = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let font = |size: CGFloat, strong: bool| NSFont::systemFontOfSize_weight(size, unsafe { if strong { NSFontWeightSemibold } else { NSFontWeightRegular } });
        // Windows gfx의 글꼴 표와 같은 순서·크기 (아이콘 글꼴 자리는 글자 크기만 쓴다)
        let fonts = vec![
            font(13.0, false),
            font(13.0, false),
            font(13.0, true),
            font(16.0, false),
            font(10.0, false),
            font(13.0, false),
            font(13.0, false),
            font(12.0, false),
            mono_font(12.0, || font(12.0, false)),
            font(15.0, true),
            // Small: 고정폭 숫자 (개수·진행 수치)
            digit_font(12.0, || font(12.0, false)),
            font(12.0, false),
            font(12.0, true),
            // Name, Meta·MetaRight (날짜·크기 열: 고정폭 숫자)
            font(13.0, false),
            digit_font(13.0, || font(13.0, false)),
            digit_font(13.0, || font(13.0, false)),
        ];
        debug_assert_eq!(fonts.len(), Font::COUNT);
        Gfx { ctx, ns, fonts }
    }

    fn c(&self) -> Option<&CGContext> {
        Some(&self.ctx)
    }

    fn set_fill(&self, c: Color) {
        CGContext::set_rgb_fill_color(self.c(), c.r as CGFloat, c.g as CGFloat, c.b as CGFloat, c.a as CGFloat);
    }

    pub fn clear(&self, c: Color) {
        self.set_fill(c);
        CGContext::fill_rect(self.c(), CGRect::new(CGPoint::new(-1.0e4, -1.0e4), CGSize::new(2.0e4, 2.0e4)));
    }

    pub fn fill(&self, r: R, c: Color) {
        self.set_fill(c);
        CGContext::fill_rect(self.c(), rect(r));
    }

    fn round_path(r: R, radius: f32) -> CFRetained<CGPath> {
        let k = radius.min(r.w() / 2.0).min(r.h() / 2.0).max(0.0) as CGFloat;
        unsafe { CGPath::with_rounded_rect(rect(r), k, k, std::ptr::null()) }
    }

    pub fn fill_round(&self, r: R, radius: f32, c: Color) {
        if r.w() <= 0.0 || r.h() <= 0.0 {
            return;
        }
        self.set_fill(c);
        CGContext::add_path(self.c(), Some(&Self::round_path(r, radius)));
        CGContext::fill_path(self.c());
    }

    pub fn stroke_round(&self, r: R, radius: f32, c: Color, width: f32) {
        if r.w() <= 0.0 || r.h() <= 0.0 {
            return;
        }
        CGContext::set_rgb_stroke_color(self.c(), c.r as CGFloat, c.g as CGFloat, c.b as CGFloat, c.a as CGFloat);
        CGContext::set_line_width(self.c(), width as CGFloat);
        CGContext::add_path(self.c(), Some(&Self::round_path(r, radius)));
        CGContext::stroke_path(self.c());
    }

    pub fn clip(&self, r: R) {
        CGContext::save_g_state(self.c());
        CGContext::clip_to_rect(self.c(), rect(r));
    }

    pub fn unclip(&self) {
        CGContext::restore_g_state(self.c());
    }

    fn line(&self, s: &str, f: Font) -> CFRetained<CTLine> {
        let font: &CTFont = unsafe { &*(Retained::as_ptr(&self.fonts[f as usize]) as *const CTFont) };
        let keys: [&CFString; 2] = unsafe { [kCTFontAttributeName, kCTForegroundColorFromContextAttributeName] };
        let yes: &CFType = unsafe { kCFBooleanTrue }.expect("kCFBooleanTrue");
        let values: [&CFType; 2] = [font.as_ref(), yes];
        let attrs = CFDictionary::from_slices(&keys, &values);
        let text = CFString::from_str(s);
        let attributed = unsafe { CFAttributedString::new(None, Some(&text), Some(attrs.as_opaque())) }.expect("CFAttributedString");
        unsafe { CTLine::with_attributed_string(&attributed) }
    }

    /// 글자 폭 (pt)
    pub fn measure(&self, s: &str, f: Font) -> f32 {
        let l = self.line(s, f);
        unsafe { l.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) as f32 }
    }

    /// 한 줄 글자: 세로 가운데, 넘치면 말줄임 (경로는 가운데를 줄인다). Right는 오른쪽, Hint·Caption은 가운데 정렬
    pub fn text(&self, s: &str, r: R, f: Font, c: Color) {
        if s.is_empty() || r.w() <= 0.0 {
            return;
        }
        let mut line = self.line(s, f);
        let (mut ascent, mut descent, mut leading) = (0.0, 0.0, 0.0);
        let mut w = unsafe { line.typographic_bounds(&mut ascent, &mut descent, &mut leading) } as f32;
        if w > r.w() {
            let kind = if matches!(f, Font::Path) { CTLineTruncationType::Middle } else { CTLineTruncationType::End };
            let token = self.line("…", f);
            if let Some(t) = unsafe { CTLine::truncated_line(&line, r.w() as f64, kind, Some(&token)) } {
                line = t;
                w = unsafe { line.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) } as f32;
            }
        }
        let x = match f {
            Font::Right | Font::MetaRight => r.r - w,
            Font::Hint | Font::Caption | Font::Glyph | Font::GlyphSmall | Font::GlyphMid => r.l + (r.w() - w) / 2.0,
            _ => r.l,
        };
        // 위아래 뒤집힌 좌표. 기준선은 주 글꼴(시스템 글꼴) 지표로 고정한다: 한글(Apple SD Gothic Neo)처럼 대체 글꼴이 끼어도
        // 그 지표가 섞이지 않아 한글 줄과 영문 줄의 기준선이 같다 (sakadu::typo::uniform_line, Windows의 UNIFORM 줄 간격과 같은 생각).
        // 글자 상자는 r 안에서 세로 가운데
        let font = &self.fonts[f as usize];
        let size = font.pointSize() as f32;
        let base = if size > 0.0 {
            let (line_h, baseline) = sakadu::typo::uniform_line(size, font.ascender() as f32 / size, -font.descender() as f32 / size);
            r.t + sakadu::typo::text_top_in_row(r.h(), line_h) + baseline
        } else {
            r.t + (r.h() + (ascent - descent) as f32) / 2.0
        };
        self.set_fill(c);
        CGContext::save_g_state(self.c());
        CGContext::clip_to_rect(self.c(), rect(R::new(r.l - 1.0, r.t - 4.0, r.r + 1.0, r.b + 4.0)));
        CGContext::set_text_position(self.c(), x as CGFloat, base as CGFloat);
        unsafe { line.draw(&self.ctx) };
        CGContext::restore_g_state(self.c());
    }

    /// 아이콘 글자(Windows Segoe 번호)를 SF Symbols로. small은 10pt, 아니면 16pt
    pub fn glyph(&self, code: u16, r: R, small: bool, c: Color) {
        self.glyph_font(code, r, if small { Font::GlyphSmall } else { Font::Glyph }, c);
    }

    /// 크기를 고른 아이콘 (Glyph 16, GlyphMid 12, GlyphSmall 10pt). 템플릿 그림을 c 색으로 칠한다
    pub fn glyph_font(&self, code: u16, r: R, f: Font, c: Color) {
        let size = match f {
            Font::GlyphSmall => 10.0,
            Font::GlyphMid => 12.0,
            _ => 15.0,
        };
        let name = NSString::from_str(symbol_name(code));
        let Some(img) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, None) else { return };
        let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(size, unsafe { NSFontWeightRegular });
        let Some(img) = img.imageWithSymbolConfiguration(&config) else { return };
        let s = img.size();
        let (w, h) = (s.width as f32, s.height as f32);
        let dst = R::new(r.l + (r.w() - w) / 2.0, r.t + (r.h() - h) / 2.0, r.l + (r.w() + w) / 2.0, r.t + (r.h() + h) / 2.0);
        // 검은 템플릿 그림을 따로 그린 뒤 그 모양에만 색을 칠한다
        CGContext::save_g_state(self.c());
        unsafe { CGContext::begin_transparency_layer(self.c(), None) };
        self.draw_image(&img, dst);
        CGContext::set_blend_mode(self.c(), objc2_core_graphics::CGBlendMode::SourceIn);
        self.fill(dst.inset(-1.0, -1.0), c);
        CGContext::end_transparency_layer(self.c());
        CGContext::restore_g_state(self.c());
    }

    /// AppKit 그림을 이 그림판에 그린다 (그동안만 이 그림판을 AppKit의 지금 그림판으로 둔다)
    fn draw_image(&self, img: &NSImage, r: R) {
        let dst = NSRect::new(NSPoint::new(r.l as CGFloat, r.t as CGFloat), NSSize::new(r.w() as CGFloat, r.h() as CGFloat));
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&self.ns));
        unsafe { img.drawInRect_fromRect_operation_fraction_respectFlipped_hints(dst, NSRect::ZERO, NSCompositingOperation::SourceOver, 1.0, true, None) };
        NSGraphicsContext::restoreGraphicsState_class();
    }

    /// 그림 파일을 읽는다. 긴 쪽이 max_px를 넘으면 그만큼으로 줄여 읽고(ImageIO가 읽으면서 줄인다), 사진에 적힌 회전도 반영한다.
    /// 읽지 못하면(그림이 아니거나 맥이 모르는 형식) None. 그리지 않고 읽기만 하므로 뒷줄에서 불러도 된다
    pub fn load_picture(path: &str, max_px: u32) -> Option<Picture> {
        let url = CFURL::from_file_path(path)?;
        let source = unsafe { CGImageSource::with_url(&url, None) }?;
        Self::thumbnail(&source, 0, max_px)
    }

    /// 메모리에 든 그림(PNG·GIF·BMP 따위. 한글 문서 안의 미리 보기 그림)을 load_picture와 같게 읽는다
    pub fn load_picture_data(bytes: &[u8], max_px: u32) -> Option<Picture> {
        let data = CFData::from_bytes(bytes);
        let source = unsafe { CGImageSource::with_data(&data, None) }?;
        Self::thumbnail(&source, 0, max_px)
    }

    /// 움직이는 gif의 장들을 load_picture처럼 줄여 읽고, 장마다 머무는 시간을 읽는다. 읽지 못하면 None.
    /// 장이 하나뿐이거나 다 풀기에 많으면(frames_fit) 첫 장만 담는다. 중간에 망가진 장이 있으면 그 앞까지만 (브라우저처럼).
    /// 다 풀 때까지 이 줄을 붙잡으므로 뒷줄에서만 부른다
    pub fn load_frames(path: &str, max_px: u32) -> Option<Frames> {
        let url = CFURL::from_file_path(path)?;
        let source = unsafe { CGImageSource::with_url(&url, None) }?;
        let first = Self::thumbnail(&source, 0, max_px)?;
        let count = unsafe { source.count() };
        let (w, h) = first.size();
        let mut pics = vec![first];
        if !frames_fit(count, w, h) {
            return Some(Frames { pics, delays: Vec::new() });
        }
        for i in 1..count {
            let Some(pic) = Self::thumbnail(&source, i, max_px) else { break };
            pics.push(pic);
        }
        let delays = if pics.len() > 1 { (0..pics.len()).map(|i| frame_delay(&source, i)).collect() } else { Vec::new() };
        Some(Frames { pics, delays })
    }

    /// 그림 원본의 index번째 장에서 긴 쪽이 max_px 안에 드는 그림을 만든다 (사진에 적힌 회전도 반영한다)
    fn thumbnail(source: &CGImageSource, index: usize, max_px: u32) -> Option<Picture> {
        let max = CFNumber::new_i32(max_px.min(i32::MAX as u32) as i32);
        let yes: &CFType = unsafe { kCFBooleanTrue }?;
        // 원본이 작아도 늘 새로 만들고(파일에 든 작은 썸네일을 쓰지 않는다), 여기서 바로 풀어 둔다 (그릴 때 메인 줄에서 풀지 않게)
        let keys: [&CFString; 4] =
            unsafe { [kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceShouldCacheImmediately, kCGImageSourceThumbnailMaxPixelSize] };
        let values: [&CFType; 4] = [yes, yes, yes, max.as_ref()];
        let options = CFDictionary::from_slices(&keys, &values);
        let img = unsafe { source.thumbnail_at_index(index, Some(options.as_opaque())) }?;
        let pic = Picture { img };
        (pic.longest() > 0).then_some(pic)
    }

    /// Quick Look이 만든 썸네일을 받는다: PDF의 첫 쪽, 동영상의 한 장면, 문서·글의 첫머리처럼 그림 파일이 아닌 것.
    /// 긴 쪽이 max_px 안에 들어오게 만든다. 만들지 못하면(썸네일을 만드는 확장이 없는 형식, 너무 오래 걸림) None.
    /// icon이면 Finder 아이콘처럼 꾸민 것(아이콘 보기), 아니면 내용 그대로(미리 보기 카드·창의 큰 그림).
    /// 다 될 때까지 이 줄을 붙잡으므로 뒷줄에서만 부른다
    pub fn load_quick_look(path: &str, max_px: u32, icon: bool) -> Option<Picture> {
        let (generator_class, request_class) = quick_look_classes()?;
        // SAFETY: +[QLThumbnailGenerator sharedGenerator] (클래스 속성)
        let generator: Option<Retained<AnyObject>> = unsafe { msg_send![generator_class, sharedGenerator] };
        let generator = generator?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let side = max_px as CGFloat;
        // SAFETY: alloc 뒤의 지정 초기화 메서드. 크기는 pt, 배율 1이라 픽셀 수와 같다
        let request: Option<Retained<AnyObject>> = unsafe {
            let fresh: Allocated<AnyObject> = msg_send![request_class, alloc];
            msg_send![fresh, initWithFileAtURL: &*url, size: CGSize::new(side, side), scale: 1.0 as CGFloat, representationTypes: QL_THUMBNAIL]
        };
        let request = request?;
        // 아이콘 보기: Finder처럼 파일 아이콘 모양으로 꾸며 받는다 (테두리, 접힌 모서리, 그림자). 꾸미지 않은 썸네일은 글·문서가
        // 테두리 없는 흰 종이라 밝은 칸에서는 묻히고 어두운 칸에서는 흰 네모로 튄다 (큰 그림에는 앱이 가는 테두리를 두른다)
        if icon {
            // SAFETY: QLThumbnailGenerationRequest의 BOOL 속성 iconMode의 설정 메서드
            let _: () = unsafe { msg_send![&*request, setIconMode: true] };
        }
        let (tx, rx) = mpsc::channel();
        // 생성기가 자기 줄에서 한 번 부른다 (실패하면 썸네일 없이). 받은 그림은 넘기기 전에 붙잡는다: 표현과 함께 사라지지 않게
        let done = RcBlock::new(move |thumb: *mut AnyObject, _error: *mut AnyObject| {
            // SAFETY: 받은 것은 QLThumbnailRepresentation이거나 null이다. CGImage는 그것의 읽기 전용 속성이다
            let img: Option<NonNull<CGImage>> = unsafe { thumb.as_ref() }.and_then(|t| NonNull::new(unsafe { msg_send![t, CGImage] }));
            // SAFETY: 살아 있는 CGImage를 하나 더 붙잡는다 (받는 쪽이 이미 기다리지 않으면 그냥 놓는다)
            let _ = tx.send(img.map(|p| unsafe { CFRetained::retain(p) }));
        });
        // SAFETY: 요청과 끝났을 때 부를 블록을 넘긴다 (생성기가 블록을 복사해 붙잡는다)
        let _: () = unsafe { msg_send![&*generator, generateBestRepresentationForRequest: &*request, completionHandler: &*done] };
        match rx.recv_timeout(QL_WAIT) {
            Ok(img) => img.map(|img| Picture { img }).filter(|pic| pic.longest() > 0),
            Err(_) => {
                sakadu::diag::log(|| format!("quick look thumbnail gave no answer in {} s", QL_WAIT.as_secs()));
                // SAFETY: 끝나지 않은 요청을 거둔다
                let _: () = unsafe { msg_send![&*generator, cancelRequest: &*request] };
                None
            }
        }
    }

    /// 그림을 r 안에 비율을 지켜 가운데에 그린다. 자리가 그림보다 크면 늘리지 않고 제 크기(1픽셀 = 1pt)로 둔다
    pub fn picture(&self, pic: &Picture, r: R) {
        self.picture_alpha(pic, r, 1.0);
    }

    /// 그림이 r 안에 놓이는 자리 (picture가 그리는 곳: 비율을 지켜 가운데, 늘리지 않는다)
    pub fn fit(pic: &Picture, r: R) -> R {
        let (w, h) = pic.size();
        let (w, h) = (w as f32, h as f32);
        if w <= 0.0 || h <= 0.0 || r.w() <= 0.0 || r.h() <= 0.0 {
            return R::new(r.l, r.t, r.l, r.t);
        }
        let k = (r.w() / w).min(r.h() / h).min(1.0);
        let (dw, dh) = (w * k, h * k);
        let (x, y) = (r.l + (r.w() - dw) / 2.0, r.t + (r.h() - dh) / 2.0);
        R::new(x, y, x + dw, y + dh)
    }

    /// picture와 같되 opacity(0~1)만큼 비치게 그린다 (숨김 파일의 썸네일은 흐리게)
    pub fn picture_alpha(&self, pic: &Picture, r: R, opacity: f32) {
        let at = Self::fit(pic, r);
        let (x, y, dw, dh) = (at.l, at.t, at.w(), at.h());
        if dw <= 0.0 || dh <= 0.0 {
            return;
        }
        // 위아래를 뒤집은 좌표라 그대로 그리면 그림이 거꾸로 선다: 그림을 그리는 동안만 그림의 아래 끝을 0으로 다시 뒤집는다
        CGContext::save_g_state(self.c());
        CGContext::translate_ctm(self.c(), x as CGFloat, (y + dh) as CGFloat);
        CGContext::scale_ctm(self.c(), 1.0, -1.0);
        CGContext::set_alpha(self.c(), opacity as CGFloat);
        CGContext::set_interpolation_quality(self.c(), CGInterpolationQuality::High);
        CGContext::draw_image(self.c(), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(dw as CGFloat, dh as CGFloat)), Some(&pic.img));
        CGContext::restore_g_state(self.c());
    }

    /// 그 경로의 Finder '종류' 이름 ("폴더", "PDF 문서"). 읽지 못하면 None
    pub fn type_description(path: &str) -> Option<String> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let mut value = None;
        unsafe { url.getResourceValue_forKey_error(&mut value, NSURLLocalizedTypeDescriptionKey) }.ok()?;
        value?.downcast::<NSString>().ok().map(|s| s.to_string())
    }

    /// 그 경로의 Finder 아이콘 (파일 종류·폴더)
    pub fn icon_for_path(&self, path: &str, r: R) {
        let img = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
        self.draw_image(&img, r);
    }

    /// 디스크에 없는 항목(압축 파일 안)의 아이콘: 폴더면 폴더, 파일이면 그 확장자의 아이콘 (확장자가 없으면 문서).
    /// 경로로 물으면 맥은 없는 경로에 빈 문서 아이콘을 준다 (폴더도)
    pub fn icon_for_kind(&self, name: &str, dir: bool, r: R) {
        let ext = name.rsplit_once('.').map(|(_, x)| x).filter(|x| !x.is_empty());
        let ty = if dir { "public.folder" } else { ext.unwrap_or("public.data") };
        // 맥 12부터 바뀐 이름(iconForContentType:)은 UTType이 필요하다. 이 호출은 확장자와 UTI 글을 그대로 받는다
        #[allow(deprecated)]
        let img = NSWorkspace::sharedWorkspace().iconForFileType(&NSString::from_str(ty));
        self.draw_image(&img, r);
    }

    /// 지금까지 그린 그림을 PNG로
    pub fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        self.ns.flushGraphics();
        let img = CGBitmapContextCreateImage(self.c()).ok_or("그림을 만들지 못함")?;
        let url = CFURL::from_file_path(path).ok_or("경로를 URL로 바꾸지 못함")?;
        let png = CFString::from_static_str("public.png");
        let dest = unsafe { CGImageDestination::with_url(&url, &png, 1, None) }.ok_or("PNG 파일을 만들지 못함")?;
        unsafe { dest.add_image(&img, None) };
        if unsafe { dest.finalize() } { Ok(()) } else { Err("PNG를 쓰지 못함".into()) }
    }

    /// 끝없이 되풀이하는 gif를 쓴다: w×h, 장마다 한 가지 색([빨강, 초록, 파랑])과 머무는 초. 시험과 창 시험의 파일이다
    pub fn write_gif(path: &std::path::Path, w: usize, h: usize, frames: &[([u8; 3], f64)]) -> Result<(), String> {
        let url = CFURL::from_file_path(path).ok_or("경로를 URL로 바꾸지 못함")?;
        let gif = CFString::from_static_str("com.compuserve.gif");
        let dest = unsafe { CGImageDestination::with_url(&url, &gif, frames.len(), None) }.ok_or("gif 파일을 만들지 못함")?;
        let forever = CFNumber::new_i32(0);
        unsafe { dest.set_properties(Some(gif_properties(kCGImagePropertyGIFLoopCount, &forever).as_opaque())) };
        for &([r, g, b], secs) in frames {
            let canvas = Gfx::bitmap(w as f32, h as f32, 1.0).ok_or("그림판을 만들지 못함")?;
            canvas.fill(R::new(0.0, 0.0, w as f32, h as f32), Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 });
            let img = CGBitmapContextCreateImage(canvas.c()).ok_or("그림을 만들지 못함")?;
            let delay = CFNumber::new_f64(secs);
            unsafe { dest.add_image(&img, Some(gif_properties(kCGImagePropertyGIFDelayTime, &delay).as_opaque())) };
        }
        if unsafe { dest.finalize() } { Ok(()) } else { Err("gif를 쓰지 못함".into()) }
    }
}

/// gif의 index번째 장에 머무는 시간. 브라우저처럼 적힌 그대로의 값(UnclampedDelayTime)을 먼저 보고, 없으면 DelayTime
fn frame_delay(source: &CGImageSource, index: usize) -> Duration {
    let secs = unsafe { source.properties_at_index(index, None) }.and_then(|props| {
        // SAFETY: ImageIO의 속성 사전은 글자 열쇠를 쓴다. 값의 종류는 아래에서 확인한다
        let props = unsafe { props.cast_unchecked::<CFString, CFType>() };
        let gif = props.get(unsafe { kCGImagePropertyGIFDictionary })?.downcast::<CFDictionary>().ok()?;
        let gif = unsafe { gif.cast_unchecked::<CFString, CFType>() };
        let seconds = |key: &CFString| gif.get(key).and_then(|v| v.downcast::<CFNumber>().ok()).and_then(|n| n.as_f64());
        seconds(unsafe { kCGImagePropertyGIFUnclampedDelayTime }).or_else(|| seconds(unsafe { kCGImagePropertyGIFDelayTime }))
    });
    frame_delay_of(secs)
}

/// gif 속성 사전 {"{GIF}": {key: value}} (gif를 쓸 때)
fn gif_properties(key: &CFString, value: &CFNumber) -> CFRetained<CFDictionary<CFString, CFType>> {
    let inner = CFDictionary::<CFString, CFType>::from_slices(&[key], &[value.as_ref()]);
    CFDictionary::from_slices(&[unsafe { kCGImagePropertyGIFDictionary }], &[inner.as_ref()])
}

/// Quick Look 썸네일 생성기와 그 요청의 클래스. 그 프레임워크를 처음 한 번 열어 찾는다 (뒷줄 여럿이 함께 불러도 한 번만 연다)
fn quick_look_classes() -> Option<(&'static AnyClass, &'static AnyClass)> {
    static OPENED: OnceLock<bool> = OnceLock::new();
    let opened = *OPENED.get_or_init(|| {
        // SAFETY: NUL로 끝나는 고정 경로다. 받은 핸들은 닫지 않는다 (앱이 끝날 때까지 쓴다)
        let lib = unsafe { libc::dlopen(QL_THUMBNAILING.as_ptr(), libc::RTLD_LAZY) };
        if lib.is_null() {
            sakadu::diag::log(|| "quick look: QuickLookThumbnailing did not load".to_string());
        }
        !lib.is_null()
    });
    if !opened {
        return None;
    }
    Some((AnyClass::get(c"QLThumbnailGenerator")?, AnyClass::get(c"QLThumbnailGenerationRequest")?))
}

/// 숫자만 고정폭인 시스템 글꼴 (날짜·크기·개수). 맥 시스템 글꼴도 기본 숫자는 비례폭이다.
/// 고정폭 글꼴처럼 빈 값을 받을 수 있어 직접 묻고, 없으면 fallback(보통 글꼴)
fn digit_font(size: CGFloat, fallback: impl Fn() -> Retained<NSFont>) -> Retained<NSFont> {
    // SAFETY: +[NSFont monospacedDigitSystemFontOfSize:weight:] (macOS 10.11부터). 빈 값일 수 있어 Option으로 받는다
    let font: Option<Retained<NSFont>> = unsafe { msg_send![NSFont::class(), monospacedDigitSystemFontOfSize: size, weight: NSFontWeightRegular] };
    font.unwrap_or_else(fallback)
}

/// 고정폭 시스템 글꼴. 맥이 가끔 빈 값을 돌려줘서(CI의 시험에서 두 번, 글꼴을 한 줄에서만 만들게 한 뒤에도 한 번)
/// 몇 번 다시 묻고, 그래도 없으면 사용자 고정폭 글꼴, 그것도 없으면 fallback을 쓴다.
/// 바인딩(monospacedSystemFontOfSize_weight)은 빈 값이면 멈추므로 직접 묻는다
fn mono_font(size: CGFloat, fallback: impl Fn() -> Retained<NSFont>) -> Retained<NSFont> {
    for attempt in 1..=3u64 {
        // SAFETY: +[NSFont monospacedSystemFontOfSize:weight:] (macOS 10.15부터). 빈 값일 수 있어 Option으로 받는다
        let font: Option<Retained<NSFont>> = unsafe { msg_send![NSFont::class(), monospacedSystemFontOfSize: size, weight: NSFontWeightRegular] };
        if let Some(font) = font {
            return font;
        }
        sakadu::diag::log(|| format!("monospaced system font was not given (try {attempt})"));
        std::thread::sleep(std::time::Duration::from_millis(20 * attempt));
    }
    NSFont::userFixedPitchFontOfSize(size).unwrap_or_else(fallback)
}
