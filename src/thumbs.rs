//! 썸네일·큰 아이콘을 백그라운드 스레드에서 만든다.
//! 화면에 보이는 것만 요청하고, 새 요청이 오면 대기열을 통째로 바꿔 스크롤로 지나간 항목은 버린다.
//! 셸이 그리지 못하는 것은 직접 만든다: 한글 문서(hwp)는 파일 안의 미리 보기 그림(PrvImage), SVG는 Direct2D로 그린다.
//! 포토샵은 합친 그림(sakadu::psd), 일러스트레이터는 안의 PDF 첫 장(짧은 프로세스에서), html은 Edge로 찍은 화면(page.rs).
//! 움직이는 gif의 프레임도 여기서 읽는다 (gif_frames)

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
        "psd" | "psb" => Some(psd_image(path, px)),
        "ai" => Some(ai_image(path, px)),
        // html은 카드·미리 보기 창(썸네일만 달라는 요청)에서만 Edge로 찍는다. 목록은 지금처럼 아이콘
        "html" | "htm" if only_thumb => Some(crate::page::shot(path).and_then(|png| image_file(&png.to_string_lossy(), px))),
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

/// 포토샵 파일: 합친 그림을 읽어(sakadu::psd) px 안에 맞춘다. 합친 그림이 없으면 파일 안의 작은 JPEG 썸네일로.
/// 읽을 때는 px의 두 배까지만 솎아 읽고(큰 파일도 메모리를 적게), 남은 줄이기는 WIC가 부드럽게 한다
fn psd_image(path: &str, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    use sakadu::psd::Picture;
    use windows::Win32::Graphics::Imaging::GUID_WICPixelFormat32bppRGBA;
    let picture = sakadu::psd::read(std::path::Path::new(path), px.saturating_mul(2).clamp(1, 4096))?;
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        match picture {
            Picture::Pixels { w, h, rgba } => {
                let bmp = wic.CreateBitmapFromMemory(w, h, &GUID_WICPixelFormat32bppRGBA, w * 4, &rgba).ok()?;
                wic_fit(&wic, &bmp.cast().ok()?, px)
            }
            Picture::Jpeg { data, bgr } => {
                let stream = wic.CreateStream().ok()?;
                stream.InitializeFromMemory(&data).ok()?;
                let frame = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?.GetFrame(0).ok()?;
                let (w, h, mut out) = wic_fit(&wic, &frame.cast().ok()?, px)?;
                // 옛 썸네일(리소스 1033)은 빨강과 파랑이 바뀌어 들어 있다
                if bgr {
                    for p in out.as_chunks_mut::<4>().0 {
                        p.swap(0, 2);
                    }
                }
                Some((w, h, out))
            }
        }
    }
}

/// 일러스트레이터 파일. 요즘 ai는 'PDF 호환 파일'로 저장돼 PDF와 같으니 첫 장을 Windows의 PDF 그리기로 그린다.
/// PDF가 아닌 옛 ai는 파일 안의 XMP 썸네일(작은 JPEG)로
fn ai_image(path: &str, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    let head = read_head(path, 1024)?;
    let pdf = head.windows(5).any(|w| w == b"%PDF-");
    pdf.then(|| pdf_page_image(path, px)).flatten().or_else(|| xmp_thumbnail(path, px))
}

/// 파일 앞부분 (n바이트까지)
fn read_head(path: &str, n: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(n);
    std::fs::File::open(path).ok()?.take(n as u64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// PDF 첫 장을 다른 프로세스에서 그릴 때 이 실행 파일에 넘기는 인자: `sakadu.exe --pdf-page <px> <원본> <결과.png>`
pub const PDF_ARG: &str = "--pdf-page";

/// PDF의 첫 장을 긴 변 px로 그린다. Windows의 PDF 그리기(Windows.Data.Pdf)를 Direct2D와 한 프로세스에서 함께 쓰면
/// 프로세스가 끝날 때 죽는다(시험에서 늘 그랬다). 그래서 이 실행 파일을 PDF만 그리는 짧은 프로세스로 한 번 더 띄워
/// 그린 PNG를 받는다. 15초가 넘으면 끝낸다
fn pdf_page_image(path: &str, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let out = std::env::temp_dir().join(format!("sakadu-pdf-{}-{}.png", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let mut cmd = pdf_command(path, px, &out)?;
    let done = run_hidden(&mut cmd, std::time::Duration::from_secs(15));
    let r = done.then(|| image_file(&out.to_string_lossy(), px)).flatten();
    let _ = std::fs::remove_file(&out);
    r
}

#[cfg(not(test))]
fn pdf_command(path: &str, px: u32, out: &std::path::Path) -> Option<std::process::Command> {
    let mut c = std::process::Command::new(std::env::current_exe().ok()?);
    c.arg(PDF_ARG).arg(px.to_string()).arg(path).arg(out);
    Some(c)
}

/// 시험에서는 시험 실행 파일의 pdf_child 시험을 PDF 그리는 프로세스로 띄운다 (인자 대신 환경 변수로 넘긴다)
#[cfg(test)]
fn pdf_command(path: &str, px: u32, out: &std::path::Path) -> Option<std::process::Command> {
    let mut c = std::process::Command::new(std::env::current_exe().ok()?);
    c.args(["--exact", "thumbs::tests::pdf_child", "--nocapture", "--test-threads=1"]);
    c.env("SAKADU_PDF_CHILD", format!("{px}\n{path}\n{}", out.display()));
    Some(c)
}

/// 이 실행 파일이 PDF를 그리는 프로세스로 떴으면 그려서 쓰고 바로 끝낸다 (main이 맨 처음 부른다). 아니면 돌아온다
pub fn run_pdf_child() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.len() != 4 || args[0].to_str() != Some(PDF_ARG) {
        return;
    }
    let px = args[1].to_str().and_then(|s| s.parse().ok());
    end_now(px.is_some_and(|px| pdf_child(&args[2].to_string_lossy(), px, std::path::Path::new(&args[3]))));
}

/// PDF 그리는 프로세스의 일: 그린 PNG를 out에 쓴다
fn pdf_child(path: &str, px: u32, out: &std::path::Path) -> bool {
    let _com = shell::ComInit::mta();
    pdf_page_png(path, px).is_some_and(|png| std::fs::write(out, png).is_ok())
}

/// 정리 없이 바로 끝낸다 (PDF 그리기가 남긴 것을 정리하다 죽지 않게)
fn end_now(ok: bool) -> ! {
    use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
    unsafe {
        let _ = TerminateProcess(GetCurrentProcess(), if ok { 0 } else { 1 });
    }
    std::process::exit(if ok { 0 } else { 1 })
}

/// PDF 첫 장을 긴 변 px로 그린 PNG (Windows.Data.Pdf, 흰 바탕). WinRT 비동기 호출을 기다리므로 다중 스레드 아파트에서 부른다
fn pdf_page_png(path: &str, px: u32) -> Option<Vec<u8>> {
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::InMemoryRandomAccessStream;
    use windows::Win32::System::WinRT::CreateStreamOverRandomAccessStream;
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path)).ok()?.join().ok()?;
    let doc = PdfDocument::LoadFromFileAsync(&file).ok()?.join().ok()?;
    let page = doc.GetPage(0).ok()?;
    let size = page.Size().ok()?;
    let opts = PdfPageRenderOptions::new().ok()?;
    if size.Width >= size.Height {
        opts.SetDestinationWidth(px).ok()?;
    } else {
        opts.SetDestinationHeight(px).ok()?;
    }
    let out = InMemoryRandomAccessStream::new().ok()?;
    page.RenderWithOptionsToStreamAsync(&out, &opts).ok()?.join().ok()?;
    let len = out.Size().ok()?.min(256 << 20) as usize;
    out.Seek(0).ok()?;
    let mut buf = vec![0u8; len];
    let mut got = 0u32;
    unsafe {
        let stream: IStream = CreateStreamOverRandomAccessStream(&out).ok()?;
        stream.Read(buf.as_mut_ptr() as *mut c_void, len as u32, Some(&mut got)).ok().ok()?;
    }
    buf.truncate(got as usize);
    (!buf.is_empty()).then_some(buf)
}

/// 그림 파일(PNG 등)을 px 안에 맞춘 그림으로
pub(crate) fn image_file(path: &str, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let frame = wic.CreateDecoderFromFilename(&HSTRING::from(path), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?.GetFrame(0).ok()?;
        wic_fit(&wic, &frame.cast().ok()?, px)
    }
}

/// 창 없이 낮은 우선순위로 실행하고 끝나기를 기다린다. 제때 끝나지 않으면 끝낸다. 잘 끝났으면 true
pub(crate) fn run_hidden(cmd: &mut std::process::Command, timeout: std::time::Duration) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
    cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let Ok(mut child) = cmd.spawn() else { return false };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// 어도비 파일의 XMP 안에 든 썸네일(xmpGImg:image, base64 JPEG). 앞쪽 16MB에서 찾는다
fn xmp_thumbnail(path: &str, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    let data = read_head(path, 16 << 20)?;
    let jpeg = xmp_thumbnail_bytes(&data)?;
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = wic.CreateStream().ok()?;
        stream.InitializeFromMemory(&jpeg).ok()?;
        let frame = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?.GetFrame(0).ok()?;
        wic_fit(&wic, &frame.cast().ok()?, px)
    }
}

/// XMP의 `<xmpGImg:image>` 안 base64를 풀어 JPEG 바이트로 (줄바꿈은 `&#xA;`로 적혀 있기도 하다)
fn xmp_thumbnail_bytes(data: &[u8]) -> Option<Vec<u8>> {
    let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
    let open = b"<xmpGImg:image>";
    let start = find(data, open)? + open.len();
    let len = find(&data[start..], b"</xmpGImg:image>")?;
    let body = String::from_utf8_lossy(&data[start..start + len]).replace("&#xA;", "").replace("&#xD;", "");
    let jpeg = base64_decode(&body)?;
    jpeg.starts_with(&[0xFF, 0xD8]).then_some(jpeg)
}

/// base64를 푼다 (공백·줄바꿈은 건너뛴다). 다른 글자가 있으면 None
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\r' | b'\n' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// 움직이는 gif: 프레임마다 (가로, 세로, 미리 곱한 BGRA)와 머무는 시간(ms)
pub type GifFrames = (Vec<(u32, u32, Vec<u8>)>, Vec<u32>);

/// gif를 겹칠 때 원래 크기로 쓰는 판(그리고 프레임 조각) 하나의 한도. 처리 방식 3은 판을 하나 더 복사해 두니 합쳐도 64MB 안.
/// 판 크기는 파일 머리에 적힌 값이라, 작은 파일이 큰 판을 적어 메모리를 크게 잡지 못하게 만들기 전에 본다
const GIF_CANVAS_MAX: usize = 32 << 20;

/// 움직이는 gif의 프레임: 한 장씩 처리 방식대로 겹친 그림을 긴 변 px 안으로 줄인 것과, 프레임마다 머무는 시간(ms).
/// 한 장뿐이거나, max_frames장을 넘거나, 줄인 그림이 모두 max_bytes를 넘으면 None (첫 장만 보인다)
pub(crate) fn gif_frames(path: &str, px: u32, max_frames: usize, max_bytes: usize) -> Option<GifFrames> {
    use windows::Win32::Graphics::Imaging::IWICMetadataQueryReader;
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantToUInt16};
    use windows::core::{PCWSTR, w};
    let read = |r: &IWICMetadataQueryReader, name: PCWSTR| -> Option<u16> {
        let mut pv = PROPVARIANT::default();
        unsafe {
            r.GetMetadataByName(name, &mut pv).ok()?;
            PropVariantToUInt16(&pv).ok()
        }
    };
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let dec = wic.CreateDecoderFromFilename(&HSTRING::from(path), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
        let n = dec.GetFrameCount().ok()? as usize;
        if !(2..=max_frames).contains(&n) {
            return None;
        }
        let meta = dec.GetMetadataQueryReader().ok()?;
        let (cw, ch) = (read(&meta, w!("/logscrdesc/Width"))? as usize, read(&meta, w!("/logscrdesc/Height"))? as usize);
        if cw == 0 || ch == 0 || cw * ch * 4 > GIF_CANVAS_MAX {
            return None;
        }
        let mut canvas = vec![0u8; cw * ch * 4];
        let (mut frames, mut delays, mut bytes) = (Vec::with_capacity(n), Vec::with_capacity(n), 0usize);
        for i in 0..n {
            let frame = dec.GetFrame(i as u32).ok()?;
            let fm = frame.GetMetadataQueryReader().ok()?;
            let at = (read(&fm, w!("/imgdesc/Left")).unwrap_or(0) as usize, read(&fm, w!("/imgdesc/Top")).unwrap_or(0) as usize);
            let delay = read(&fm, w!("/grctlext/Delay")).unwrap_or(0) as u32 * 10;
            let disposal = read(&fm, w!("/grctlext/Disposal")).unwrap_or(0);
            let (mut fw, mut fh) = (0u32, 0u32);
            frame.GetSize(&mut fw, &mut fh).ok()?;
            if fw as usize * fh as usize * 4 > GIF_CANVAS_MAX {
                return None;
            }
            let conv = wic.CreateFormatConverter().ok()?;
            conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
            let mut piece = vec![0u8; (fw * fh * 4) as usize];
            conv.CopyPixels(std::ptr::null(), fw * 4, &mut piece).ok()?;
            let before = (disposal == 3).then(|| canvas.clone());
            let size = (fw as usize, fh as usize);
            gif_put(&mut canvas, (cw, ch), &piece, size, at);
            let bmp = wic.CreateBitmapFromMemory(cw as u32, ch as u32, &GUID_WICPixelFormat32bppPBGRA, cw as u32 * 4, &canvas).ok()?;
            let fit = wic_fit(&wic, &bmp.cast().ok()?, px)?;
            bytes += fit.2.len();
            if bytes > max_bytes {
                return None;
            }
            frames.push(fit);
            // 지연이 아주 짧은(0·10ms) gif는 브라우저처럼 0.1초로 본다
            delays.push(if delay < 20 { 100 } else { delay });
            match disposal {
                2 => gif_clear(&mut canvas, (cw, ch), size, at),
                3 => canvas = before.unwrap_or(canvas),
                _ => {}
            }
        }
        Some((frames, delays))
    }
}

/// gif 프레임 조각을 판의 at 자리에 겹친다. 투명한 칸은 아래를 그대로 둔다 (gif의 투명은 있거나 없거나 둘뿐)
fn gif_put(canvas: &mut [u8], (cw, ch): (usize, usize), piece: &[u8], (fw, fh): (usize, usize), (x0, y0): (usize, usize)) {
    for y in 0..fh.min(ch.saturating_sub(y0)) {
        for x in 0..fw.min(cw.saturating_sub(x0)) {
            let s = (y * fw + x) * 4;
            if piece[s + 3] != 0 {
                let d = ((y0 + y) * cw + x0 + x) * 4;
                canvas[d..d + 4].copy_from_slice(&piece[s..s + 4]);
            }
        }
    }
}

/// 처리 방식 2(바탕으로 되돌리기): 그 프레임 자리를 투명하게
fn gif_clear(canvas: &mut [u8], (cw, ch): (usize, usize), (fw, fh): (usize, usize), (x0, y0): (usize, usize)) {
    for y in y0..(y0 + fh).min(ch) {
        let row = y * cw;
        let (a, b) = ((row + x0.min(cw)) * 4, (row + (x0 + fw).min(cw)) * 4);
        canvas[a..b].fill(0);
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

    /// 200×100pt 한 장에 파란 사각형 하나인 PDF (xref 자리를 맞춰 만든다)
    fn tiny_pdf() -> Vec<u8> {
        let content = "0 0.4 0.9 rg 0 0 200 100 re f";
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    /// 일러스트레이터 파일: PDF로 저장된 ai는 첫 장을 그린다 (2:1 판은 2:1로, 긴 변 px). PDF도 썸네일도 없는 ai는 썸네일만 달라면 실패
    #[test]
    fn renders_the_pdf_inside_an_illustrator_file() {
        let _com = shell::ComInit::sta();
        let d = temp_dir("ai");
        let f = d.join("로고.ai");
        std::fs::write(&f, tiny_pdf()).unwrap();
        let (w, h, px) = render(&f.to_string_lossy(), 96, true).expect("pdf page");
        assert_eq!((w, h), (96, 48));
        let mid = ((h / 2 * w + w / 2) * 4) as usize;
        assert!(px[mid] > 150 && px[mid + 2] < 60 && px[mid + 3] == 255, "blue fill (BGRA): {:?}", &px[mid..mid + 4]);
        let old = d.join("옛 파일.ai");
        std::fs::write(&old, b"%!PS-Adobe-3.0\n%%Creator: Adobe Illustrator\n").unwrap();
        assert!(render(&old.to_string_lossy(), 96, true).is_none());
        std::fs::remove_dir_all(d).unwrap();
    }

    /// 포토샵 파일: 합친 그림을 그대로 (4×2, 왼쪽 반 빨강·오른쪽 반 파랑, 압축 없는 RGB). 깨진 파일은 썸네일만 달라면 실패
    #[test]
    fn renders_the_merged_image_of_a_photoshop_file() {
        let _com = shell::ComInit::sta();
        let d = temp_dir("psd");
        let mut psd = b"8BPS".to_vec();
        psd.extend([0, 1, 0, 0, 0, 0, 0, 0, 0, 3]);
        psd.extend(2u32.to_be_bytes());
        psd.extend(4u32.to_be_bytes());
        psd.extend([0, 8, 0, 3]);
        psd.extend([0u8; 12]);
        psd.extend([0, 0]);
        let half = |a: u8, b: u8| [a, a, b, b, a, a, b, b];
        psd.extend(half(255, 0));
        psd.extend(half(0, 0));
        psd.extend(half(0, 255));
        let f = d.join("포스터.psd");
        std::fs::write(&f, &psd).unwrap();
        let (w, h, px) = render(&f.to_string_lossy(), 64, true).expect("merged image");
        assert_eq!((w, h), (4, 2));
        assert_eq!((&px[0..4], &px[12..16]), (&[0, 0, 255, 255][..], &[255, 0, 0, 255][..]), "BGRA: red left, blue right");
        let broken = d.join("깨진.psd");
        std::fs::write(&broken, &psd[..30]).unwrap();
        assert!(render(&broken.to_string_lossy(), 64, true).is_none());
        std::fs::remove_dir_all(d).unwrap();
    }

    /// PDF를 그리는 프로세스 노릇. 위 시험이 이 시험 실행 파일을 다시 띄울 때만(환경 변수로 일을 받았을 때만) 일한다
    #[test]
    fn pdf_child() {
        let Ok(job) = std::env::var("SAKADU_PDF_CHILD") else { return };
        let mut it = job.lines();
        let (px, path, out) = (it.next().and_then(|s| s.parse().ok()), it.next(), it.next());
        end_now(matches!((px, path, out), (Some(px), Some(path), Some(out)) if super::pdf_child(path, px, std::path::Path::new(out))));
    }

    /// XMP 썸네일: base64를 풀고(엔티티로 적힌 줄바꿈은 건너뛴다) JPEG일 때만 준다
    #[test]
    fn reads_the_base64_thumbnail_in_xmp() {
        assert_eq!(base64_decode("aGVsbG8=").as_deref(), Some(&b"hello"[..]));
        assert_eq!(base64_decode("aGVs\r\nbG8").as_deref(), Some(&b"hello"[..]));
        assert_eq!(base64_decode("a*b"), None);
        let xmp = b"<x:xmpmeta><xmpGImg:format>JPEG</xmpGImg:format><xmpGImg:image>/9j/4AAQ&#xA;SkZJRg==</xmpGImg:image></x:xmpmeta>";
        assert_eq!(xmp_thumbnail_bytes(xmp), Some(vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46]));
        assert_eq!(xmp_thumbnail_bytes(b"<xmpGImg:image>aGVsbG8=</xmpGImg:image>"), None, "not a JPEG");
        assert_eq!(xmp_thumbnail_bytes(b"no thumbnail here"), None);
    }

    type GifPiece = (u16, u16, u16, u16, u8, u16, Vec<u8>);

    /// 작은 gif를 손으로 만든다. 색표는 검정·빨강·파랑·흰색. frames: (왼쪽, 위, 가로, 세로, 처리 방식, 지연(1/100초), 색 번호들).
    /// LZW는 코드마다 앞에 지우기 코드를 넣어 코드 길이가 3비트에 머물게 한다 (압축은 안 되지만 올바른 gif다)
    fn tiny_gif(w: u16, h: u16, frames: &[GifPiece]) -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend(w.to_le_bytes());
        g.extend(h.to_le_bytes());
        g.extend([0x91, 0, 0]);
        g.extend([0, 0, 0, 255, 0, 0, 0, 0, 255, 255, 255, 255]);
        for (l, t, fw, fh, disposal, delay, idx) in frames {
            g.extend([0x21, 0xF9, 4, disposal << 2]);
            g.extend(delay.to_le_bytes());
            g.extend([0, 0, 0x2C]);
            for v in [*l, *t, *fw, *fh] {
                g.extend(v.to_le_bytes());
            }
            g.extend([0, 2]);
            let (mut bits, mut n, mut data) = (0u32, 0u32, Vec::new());
            let mut put = |code: u32| {
                bits |= code << n;
                n += 3;
                while n >= 8 {
                    data.push(bits as u8);
                    bits >>= 8;
                    n -= 8;
                }
            };
            for &i in idx {
                put(4);
                put(i as u32);
            }
            put(5);
            if n > 0 {
                data.push(bits as u8);
            }
            for chunk in data.chunks(255) {
                g.push(chunk.len() as u8);
                g.extend(chunk);
            }
            g.push(0);
        }
        g.push(0x3B);
        g
    }

    /// 움직이는 gif: 프레임마다 앞 그림 위에 겹친 판을 주고(처리 방식 2는 그 자리를 비운다), 짧은 지연은 0.1초로.
    /// 한 장짜리와 너무 많은 프레임은 움직이지 않는다
    #[test]
    fn reads_animated_gif_frames_composited() {
        let _com = shell::ComInit::sta();
        let d = temp_dir("gif");
        let f = d.join("움직임.gif");
        // 1: 2×2 빨강(0.05초) → 2: 오른쪽 아래 한 칸만 파랑, 보인 뒤 그 자리를 비움 → 3: 왼쪽 위 한 칸만 흰색
        let frames = [(0, 0, 2, 2, 1, 5, vec![1, 1, 1, 1]), (1, 1, 1, 1, 2, 0, vec![2]), (0, 0, 1, 1, 1, 30, vec![3])];
        std::fs::write(&f, tiny_gif(2, 2, &frames)).unwrap();
        let (got, delays) = gif_frames(&f.to_string_lossy(), 64, 400, 1 << 20).expect("frames");
        assert_eq!(delays, [50, 100, 300]);
        assert_eq!(got.iter().map(|(w, h, _)| (*w, *h)).collect::<Vec<_>>(), [(2, 2); 3]);
        // BGRA로 칸마다 (파랑 성분, 빨강 성분)
        let px = |i: usize, k: usize| (got[i].2[k * 4], got[i].2[k * 4 + 2]);
        assert_eq!([px(0, 0), px(0, 3)], [(0, 255), (0, 255)]);
        assert_eq!([px(1, 0), px(1, 3)], [(0, 255), (255, 0)], "blue over red");
        // 처리 방식 2는 그 자리를 바탕(투명)으로 되돌린다: 오른쪽 아래는 비고, 왼쪽 위에 흰색
        assert_eq!([px(2, 0), px(2, 3)], [(255, 255), (0, 0)]);
        assert_eq!((got[2].2[3], got[2].2[15]), (255, 0), "white opaque, disposed cell transparent");
        assert!(gif_frames(&f.to_string_lossy(), 64, 2, 1 << 20).is_none(), "too many frames");
        assert!(gif_frames(&f.to_string_lossy(), 64, 400, 20).is_none(), "too many bytes");
        // 머리에 아주 큰 판(60000×60000)을 적은 작은 gif: 판을 잡기 전에 멈춘다 (첫 장만)
        let huge = d.join("큰 판.gif");
        std::fs::write(&huge, tiny_gif(60000, 60000, &frames)).unwrap();
        assert!(gif_frames(&huge.to_string_lossy(), 64, 400, 1 << 20).is_none());
        let one = d.join("한 장.gif");
        std::fs::write(&one, tiny_gif(2, 2, &frames[..1])).unwrap();
        assert!(gif_frames(&one.to_string_lossy(), 64, 400, 1 << 20).is_none());
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
        // 맥이 쓰는 공통 복합 파일 읽기(sakadu::cfb)도 COM이 쓴 파일에서 같은 스트림을 읽고,
        // 그 시험용 파일 만들기(cfb::build)가 만든 파일은 COM도 읽는다
        let open = |p: &std::path::Path| sakadu::cfb::Cfb::open(std::fs::File::open(p).unwrap()).expect("compound file");
        assert_eq!(open(&with).stream("PrvImage"), Some(tiny_bmp()));
        let text: Vec<u8> = "회의 자료\r\n둘째 줄".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!((open(&without).stream("PrvText"), open(&without).stream("PrvImage")), (Some(text.clone()), None));
        let built = d.join("만든 문서.hwp");
        let big: Vec<u8> = (0..9000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&built, sakadu::cfb::build(&[("PrvImage", &tiny_bmp()), ("BodyText", &big), ("PrvText", &text)])).unwrap();
        assert!(hwp_has_image(&built.to_string_lossy()));
        assert_eq!(hwp_text(&built.to_string_lossy()).as_deref(), Some("회의 자료\r\n둘째 줄"));
        assert_eq!(read_stream(&hwp_storage(&built.to_string_lossy()).unwrap(), "BodyText"), Some(big));
        std::fs::remove_dir_all(d).unwrap();
    }
}
