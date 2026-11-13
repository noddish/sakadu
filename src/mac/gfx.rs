//! Core Graphics·Core Text 그리기. Windows `gfx.rs`(Direct2D)와 같은 이름·좌표(왼쪽 위가 0, pt 단위)를 쓴다.
//! 지금은 비트맵(PNG로 저장)에 그리고, 다음 단계에서 창(NSView)에 그릴 때도 같은 함수를 쓴다

use objc2::rc::Retained;
use objc2_app_kit::{NSCompositingOperation, NSFont, NSFontWeightRegular, NSFontWeightSemibold, NSGraphicsContext, NSImage, NSImageSymbolConfiguration, NSWorkspace};
use objc2_core_foundation::{CFAttributedString, CFDictionary, CFNumber, CFRetained, CFString, CFType, CFURL, CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize, kCFBooleanTrue};
use objc2_core_graphics::{CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, CGInterpolationQuality, CGPath, kCGColorSpaceSRGB};
use objc2_core_text::{CTFont, CTLine, CTLineTruncationType, kCTFontAttributeName, kCTForegroundColorFromContextAttributeName};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURL, NSURLLocalizedTypeDescriptionKey};
use objc2_image_io::{
    CGImageDestination, CGImageSource, kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceShouldCacheImmediately,
    kCGImageSourceThumbnailMaxPixelSize,
};
use sakadu::draw::{Color, Font, R};

/// 고정폭 글(글 파일 미리 보기)의 줄 높이
pub const MONO_LINE_H: f32 = 17.0;

/// 읽어 둔 그림. 뒷줄에서 읽어(Gfx::load_picture) 메인 줄에서 그린다 (CGImage는 만든 뒤 바뀌지 않아 줄 사이로 넘겨도 된다)
pub struct Picture {
    img: CFRetained<CGImage>,
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
        0xE8B7 => "folder",
        0xE718 => "pin",
        0xE841 => "pin.fill",
        0xE8CB => "arrow.up.arrow.down",
        // 책갈피 (ui::glyph::G_BOOKMARK, G_BOOKMARK_FILL)
        0xFFF0 => "bookmark",
        0xFFF1 => "bookmark.fill",
        // 자동 미리 보기의 눈, 미리 보기 카드의 크게 보기·작게 보기 (ui::glyph::G_EYE, G_EXPAND, G_SHRINK)
        0xE890 => "eye",
        0xE740 => "arrow.up.left.and.arrow.down.right",
        0xE73F => "arrow.down.right.and.arrow.up.left",
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
            NSFont::monospacedSystemFontOfSize_weight(12.0, unsafe { NSFontWeightRegular }),
            font(24.0, false),
            font(12.0, false),
            font(12.0, false),
        ];
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
            Font::Right => r.r - w,
            Font::Hint | Font::Caption | Font::Glyph | Font::GlyphSmall | Font::GlyphMid => r.l + (r.w() - w) / 2.0,
            _ => r.l,
        };
        // 위아래 뒤집힌 좌표: 기준선은 글자 덩어리를 세로 가운데에 둔 곳
        let base = r.t + (r.h() + (ascent - descent) as f32) / 2.0;
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
        let max = CFNumber::new_i32(max_px.min(i32::MAX as u32) as i32);
        let yes: &CFType = unsafe { kCFBooleanTrue }?;
        // 원본이 작아도 늘 새로 만들고(파일에 든 작은 썸네일을 쓰지 않는다), 여기서 바로 풀어 둔다 (그릴 때 메인 줄에서 풀지 않게)
        let keys: [&CFString; 4] =
            unsafe { [kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceShouldCacheImmediately, kCGImageSourceThumbnailMaxPixelSize] };
        let values: [&CFType; 4] = [yes, yes, yes, max.as_ref()];
        let options = CFDictionary::from_slices(&keys, &values);
        let img = unsafe { source.thumbnail_at_index(0, Some(options.as_opaque())) }?;
        let pic = Picture { img };
        (pic.longest() > 0).then_some(pic)
    }

    /// 그림을 r 안에 비율을 지켜 가운데에 그린다. 자리가 그림보다 크면 늘리지 않고 제 크기(1픽셀 = 1pt)로 둔다
    pub fn picture(&self, pic: &Picture, r: R) {
        self.picture_alpha(pic, r, 1.0);
    }

    /// picture와 같되 opacity(0~1)만큼 비치게 그린다 (숨김 파일의 썸네일은 흐리게)
    pub fn picture_alpha(&self, pic: &Picture, r: R, opacity: f32) {
        let (w, h) = pic.size();
        let (w, h) = (w as f32, h as f32);
        if w <= 0.0 || h <= 0.0 || r.w() <= 0.0 || r.h() <= 0.0 {
            return;
        }
        let k = (r.w() / w).min(r.h() / h).min(1.0);
        let (dw, dh) = (w * k, h * k);
        let (x, y) = (r.l + (r.w() - dw) / 2.0, r.t + (r.h() - dh) / 2.0);
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
}
