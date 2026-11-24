//! 썸네일·큰 아이콘을 백그라운드 스레드에서 만든다.
//! 화면에 보이는 것만 요청하고, 새 요청이 오면 대기열을 통째로 바꿔 스크롤로 지나간 항목은 버린다.
//! 셸이 그리지 못하는 것은 직접 만든다: 한글 문서(hwp)는 파일 안의 미리 보기 그림(PrvImage), SVG는 Direct2D로 그린다

use crate::WM_APP_THUMB;
use crate::shell::{self, SendHwnd};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Condvar, Mutex};
use windows::Win32::Foundation::{HWND, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{D2D_SIZE_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1CreateFactory, ID2D1DeviceContext5, ID2D1Factory,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory, WICBitmapCacheOnLoad, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::{IStorage, STGFMT_STORAGE, StgOpenStorageEx};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IStream, STATFLAG_NONAME, STGM_READ, STGM_SHARE_DENY_NONE, STGM_SHARE_EXCLUSIVE, STGM_TRANSACTED};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SHCreateStreamOnFileEx, SIIGBF_RESIZETOFIT, SIIGBF_THUMBNAILONLY};
use windows::core::{HSTRING, Interface};

pub struct Request {
    pub key: String,
    pub path: String,
    /// 긴 변의 픽셀 수
    pub px: u32,
    /// 썸네일만 (문서·동영상 미리 보기): 썸네일이 없으면 큰 아이콘을 주지 않고 실패로 알린다
    pub only_thumb: bool,
}

pub struct Image {
    pub key: String,
    pub w: u32,
    pub h: u32,
    /// 미리 곱한 알파의 BGRA, 위에서 아래로. 비어 있으면 만들지 못한 것
    pub pixels: Vec<u8>,
}

struct Queue {
    hwnd: Option<SendHwnd>,
    items: VecDeque<Request>,
    started: bool,
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue { hwnd: None, items: VecDeque::new(), started: false });
static WAKE: Condvar = Condvar::new();
/// 동영상·문서 썸네일은 하나에 수백 ms 걸리기도 하므로 둘을 함께 돌린다
const WORKERS: usize = 2;

/// 대기 중인 요청을 reqs로 바꾼다. 앞쪽부터 처리
pub fn request(hwnd: HWND, reqs: Vec<Request>) {
    let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    q.hwnd = Some(SendHwnd(hwnd));
    q.items = reqs.into();
    if !q.started {
        q.started = true;
        for _ in 0..WORKERS {
            std::thread::spawn(worker);
        }
    }
    drop(q);
    WAKE.notify_all();
}

fn worker() {
    let _com = shell::ComInit::sta();
    loop {
        let (owner, req) = {
            let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(h) = q.hwnd {
                    if let Some(r) = q.items.pop_front() {
                        break (h, r);
                    }
                }
                q = WAKE.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        let (w, h, pixels) = render(&req.path, req.px, req.only_thumb).unwrap_or_default();
        shell::post_box(owner.get(), WM_APP_THUMB, Box::new(Image { key: req.key, w, h, pixels }));
    }
}

/// 셸이 주는 그림: 사진·동영상·문서는 썸네일, 나머지는 큰 아이콘 (only_thumb이면 썸네일만).
/// 한글 문서와 SVG는 직접 그린다 (못 그리면 셸에 맡겨 아이콘이라도)
pub fn render(path: &str, px: u32, only_thumb: bool) -> Option<(u32, u32, Vec<u8>)> {
    let own = match ext_lower(path).as_str() {
        "hwp" => Some(hwp_image(path, px)),
        "svg" => Some(svg_image(path, px)),
        _ => None,
    };
    match own {
        Some(Some(r)) => return Some(r),
        Some(None) if only_thumb => return None,
        _ => {}
    }
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let flags = if only_thumb { SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY } else { SIIGBF_RESIZETOFIT };
        let hbmp = factory.GetImage(SIZE { cx: px as i32, cy: px as i32 }, flags).ok()?;
        let r = read_bitmap(hbmp);
        let _ = DeleteObject(hbmp.into());
        r
    }
}

fn ext_lower(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// 한글 문서(HWP 5, OLE 복합 파일)를 읽기만 하는 저장소로 연다. 한글이 열어 두고 있어도 열리게 공유 모드는 막지 않는다
fn hwp_storage(path: &str) -> Option<IStorage> {
    unsafe {
        let mut p: *mut c_void = std::ptr::null_mut();
        StgOpenStorageEx(&HSTRING::from(path), STGM_READ | STGM_TRANSACTED | STGM_SHARE_DENY_NONE, STGFMT_STORAGE, 0, None, None, &IStorage::IID, &mut p).ok()?;
        (!p.is_null()).then(|| IStorage::from_raw(p))
    }
}

/// 저장소 안 스트림(name)을 통째로 읽는다 (64MB까지)
fn read_stream(stg: &IStorage, name: &str) -> Option<Vec<u8>> {
    unsafe {
        let s: IStream = stg.OpenStream(&HSTRING::from(name), None, STGM_READ | STGM_SHARE_EXCLUSIVE, 0).ok()?;
        let mut st = Default::default();
        s.Stat(&mut st, STATFLAG_NONAME).ok()?;
        let len = st.cbSize.min(64 << 20) as usize;
        let mut buf = vec![0u8; len];
        let mut got = 0u32;
        s.Read(buf.as_mut_ptr() as *mut c_void, len as u32, Some(&mut got)).ok().ok()?;
        buf.truncate(got as usize);
        (!buf.is_empty()).then_some(buf)
    }
}

/// 한글 문서에 들어 있는 미리 보기 그림이 있는지 (한글이 저장할 때 넣는다. 없는 문서도 있다)
pub fn hwp_has_image(path: &str) -> bool {
    hwp_storage(path).is_some_and(|stg| unsafe { stg.OpenStream(&HSTRING::from("PrvImage"), None, STGM_READ | STGM_SHARE_EXCLUSIVE, 0) }.is_ok())
}

/// 한글 문서의 미리 보기 글 (PrvText, UTF-16). 앞부분만 들어 있다
pub fn hwp_text(path: &str) -> Option<String> {
    let b = read_stream(&hwp_storage(path)?, "PrvText")?;
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let s = String::from_utf16_lossy(&units);
    let s = s.trim_end_matches('\0').trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// 한글 문서의 미리 보기 그림(PrvImage: PNG·GIF·BMP)을 px 안에 맞춘 그림으로
fn hwp_image(path: &str, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    let bytes = read_stream(&hwp_storage(path)?, "PrvImage")?;
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = wic.CreateStream().ok()?;
        stream.InitializeFromMemory(&bytes).ok()?;
        let frame = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?.GetFrame(0).ok()?;
        wic_fit(&wic, &frame.cast().ok()?, px)
    }
}

/// WIC 그림을 긴 변이 px를 넘지 않게 줄여(키우지는 않는다) 미리 곱한 BGRA로
fn wic_fit(wic: &IWICImagingFactory, src: &IWICBitmapSource, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let (mut w, mut h) = (0u32, 0u32);
        src.GetSize(&mut w, &mut h).ok()?;
        if w == 0 || h == 0 {
            return None;
        }
        let k = (px as f32 / w.max(h) as f32).min(1.0);
        let (nw, nh) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
        let scaled: IWICBitmapSource = if (nw, nh) == (w, h) {
            src.clone()
        } else {
            let s = wic.CreateBitmapScaler().ok()?;
            s.Initialize(src, nw, nh, WICBitmapInterpolationModeFant).ok()?;
            s.cast().ok()?
        };
        let conv = wic.CreateFormatConverter().ok()?;
        conv.Initialize(&scaled, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
        let mut out = vec![0u8; (nw * nh * 4) as usize];
        conv.CopyPixels(std::ptr::null(), nw * 4, &mut out).ok()?;
        Some((nw, nh, out))
    }
}

/// SVG를 px×px 안에 비율을 지켜 그린다 (Windows 10 1703부터 있는 Direct2D SVG). 빈 둘레는 잘라 낸다
fn svg_image(path: &str, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
        let bmp = wic.CreateBitmap(px, px, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad).ok()?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let rt = factory.CreateWicBitmapRenderTarget(&bmp, &props).ok()?;
        let dc: ID2D1DeviceContext5 = rt.cast().ok()?;
        let file = SHCreateStreamOnFileEx(&HSTRING::from(path), (STGM_READ | STGM_SHARE_DENY_NONE).0, 0, false, None).ok()?;
        let doc = dc.CreateSvgDocument(&file, D2D_SIZE_F { width: px as f32, height: px as f32 }).ok()?;
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        dc.DrawSvgDocument(&doc);
        rt.EndDraw(None, None).ok()?;
        drop(rt);
        let mut out = vec![0u8; (px * px * 4) as usize];
        bmp.CopyPixels(std::ptr::null(), px * 4, &mut out).ok()?;
        Some(trim_clear(px, px, out))
    }
}

/// 둘레의 완전히 투명한 줄·칸을 잘라 낸다 (SVG는 정사각형 판에 비율을 지켜 그려 위아래나 양옆이 빈다)
fn trim_clear(w: u32, h: u32, px: Vec<u8>) -> (u32, u32, Vec<u8>) {
    let (w, h) = (w as usize, h as usize);
    let opaque = |x: usize, y: usize| px[(y * w + x) * 4 + 3] != 0;
    let rows: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| opaque(x, y))).collect();
    let cols: Vec<usize> = (0..w).filter(|&x| (0..h).any(|y| opaque(x, y))).collect();
    let (Some(&t), Some(&b), Some(&l), Some(&r)) = (rows.first(), rows.last(), cols.first(), cols.last()) else { return (w as u32, h as u32, px) };
    let (nw, nh) = (r - l + 1, b - t + 1);
    let mut out = Vec::with_capacity(nw * nh * 4);
    for y in t..=b {
        out.extend_from_slice(&px[(y * w + l) * 4..(y * w + r + 1) * 4]);
    }
    (nw as u32, nh as u32, out)
}

fn read_bitmap(hbmp: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(hbmp.into(), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void)) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 {
            return None;
        }
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // 음수 높이 = 위에서 아래로
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; w as usize * h as usize * 4];
        let hdc = GetDC(None);
        let lines = GetDIBits(hdc, hbmp, 0, h as u32, Some(px.as_mut_ptr() as *mut c_void), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(None, hdc);
        if lines == 0 {
            return None;
        }
        fix_alpha(&mut px, bm.bmBitsPixel == 32);
        Some((w as u32, h as u32, px))
    }
}

/// 알파가 없는 그림(JPEG 썸네일 등)은 불투명으로, 미리 곱하지 않은 알파면 곱해서 Direct2D 형식에 맞춘다
fn fix_alpha(px: &mut [u8], has_alpha: bool) {
    if !has_alpha || px.chunks_exact(4).all(|p| p[3] == 0) {
        for p in px.chunks_exact_mut(4) {
            p[3] = 255;
        }
        return;
    }
    if px.chunks_exact(4).any(|p| p[0] > p[3] || p[1] > p[3] || p[2] > p[3]) {
        for p in px.chunks_exact_mut(4) {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_when_alpha_missing() {
        let mut px = vec![10, 20, 30, 0, 40, 50, 60, 0];
        fix_alpha(&mut px, true);
        assert_eq!(px, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn premultiplies_straight_alpha() {
        let mut px = vec![200, 100, 0, 128];
        fix_alpha(&mut px, true);
        assert_eq!(px, vec![100, 50, 0, 128]);
    }

    #[test]
    fn keeps_premultiplied() {
        let mut px = vec![100, 50, 0, 128, 0, 0, 0, 0];
        fix_alpha(&mut px, true);
        assert_eq!(px, vec![100, 50, 0, 128, 0, 0, 0, 0]);
    }

    /// 실제 셸에서 이 실행 파일의 그림을 받아 본다
    #[test]
    #[ignore]
    fn renders_shell_image() {
        let _com = shell::ComInit::sta();
        let exe = std::env::current_exe().unwrap();
        let (w, h, px) = render(&exe.to_string_lossy(), 96, false).expect("이미지");
        assert!(w > 0 && h > 0 && w <= 96 && h <= 96, "{w}x{h}");
        assert_eq!(px.len(), (w * h * 4) as usize);
        let windir = std::env::var("WINDIR").unwrap();
        let (w, h, _) = render(&windir, 256, false).expect("폴더 이미지");
        assert!(w > 0 && h > 0);
        // 썸네일만 달라고 하면 썸네일이 없는 것(실행 파일)은 아이콘 대신 실패
        assert!(render(&exe.to_string_lossy(), 96, true).is_none());
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sakadu-thumbs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// SVG는 Direct2D로 그린다: 2:1 그림은 정사각형 판에 비율을 지켜 그린 뒤 빈 둘레를 잘라 2:1로
    #[test]
    fn renders_svg_keeping_its_shape() {
        let _com = shell::ComInit::sta();
        let d = temp_dir("svg");
        let f = d.join("도형.svg");
        std::fs::write(&f, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><rect width="200" height="100" fill="#3070e0"/></svg>"##).unwrap();
        let (w, h, px) = render(&f.to_string_lossy(), 96, true).expect("svg");
        assert_eq!((w, h), (96, 48));
        let mid = ((h / 2 * w + w / 2) * 4) as usize;
        assert_eq!(px[mid + 3], 255, "opaque fill");
        assert!(px[mid] > px[mid + 2], "blue: {:?}", &px[mid..mid + 4]);
        std::fs::remove_dir_all(d).unwrap();
    }

    /// 2×1 크기의 24비트 BMP (왼쪽 빨강, 오른쪽 파랑)
    fn tiny_bmp() -> Vec<u8> {
        let mut b = Vec::new();
        let pixels: [u8; 8] = [0, 0, 255, 255, 0, 0, 0, 0];
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(14u32 + 40 + 8).to_le_bytes());
        b.extend_from_slice(&[0, 0, 0, 0]);
        b.extend_from_slice(&(14u32 + 40).to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0u8; 24]);
        b.extend_from_slice(&pixels);
        b
    }

    /// 한글 문서처럼 PrvImage·PrvText 스트림을 담은 복합 파일을 만든다
    fn fake_hwp(path: &std::path::Path, image: bool) {
        use windows::Win32::System::Com::StructuredStorage::StgCreateStorageEx;
        use windows::Win32::System::Com::{STGM_CREATE, STGM_READWRITE, STGM_WRITE};
        unsafe {
            let mut p: *mut c_void = std::ptr::null_mut();
            StgCreateStorageEx(&HSTRING::from(path.to_string_lossy().as_ref()), STGM_CREATE | STGM_READWRITE | STGM_SHARE_EXCLUSIVE, STGFMT_STORAGE, 0, None, None, &IStorage::IID, &mut p).unwrap();
            let stg = IStorage::from_raw(p);
            let put = |name: &str, bytes: &[u8]| {
                let s = stg.CreateStream(&HSTRING::from(name), STGM_CREATE | STGM_WRITE | STGM_SHARE_EXCLUSIVE, 0, 0).unwrap();
                s.Write(bytes.as_ptr() as *const c_void, bytes.len() as u32, None).ok().unwrap();
            };
            if image {
                put("PrvImage", &tiny_bmp());
            }
            let text: Vec<u8> = "회의 자료\r\n둘째 줄".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
            put("PrvText", &text);
            stg.Commit(0).unwrap();
        }
    }

    /// 한글 문서: 안에 든 미리 보기 그림(키우지 않는다)과 미리 보기 글을 읽는다. 그림이 없는 문서는 썸네일만 달라면 실패
    #[test]
    fn reads_the_preview_image_and_text_inside_hwp() {
        let _com = shell::ComInit::sta();
        let d = temp_dir("hwp");
        let with = d.join("보고서.hwp");
        fake_hwp(&with, true);
        assert!(hwp_has_image(&with.to_string_lossy()));
        let (w, h, px) = render(&with.to_string_lossy(), 64, true).expect("PrvImage");
        assert_eq!((w, h), (2, 1));
        assert_eq!((px[2], px[3], px[4], px[7]), (255, 255, 255, 255), "red then blue, opaque: {px:?}");
        assert_eq!(hwp_text(&with.to_string_lossy()).as_deref(), Some("회의 자료\r\n둘째 줄"));
        let without = d.join("그림 없음.hwp");
        fake_hwp(&without, false);
        assert!(!hwp_has_image(&without.to_string_lossy()));
        assert!(render(&without.to_string_lossy(), 64, true).is_none());
        assert!(hwp_text(&without.to_string_lossy()).is_some());
        std::fs::remove_dir_all(d).unwrap();
    }
}
