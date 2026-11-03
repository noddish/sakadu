//! 썸네일·큰 아이콘을 백그라운드 스레드에서 만든다.
//! 화면에 보이는 것만 요청하고, 새 요청이 오면 대기열을 통째로 바꿔 스크롤로 지나간 항목은 버린다.

use crate::WM_APP_THUMB;
use crate::shell::{self, SendHwnd};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Condvar, Mutex};
use windows::Win32::Foundation::{HWND, SIZE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT};
use windows::core::HSTRING;

pub struct Request {
    pub key: String,
    pub path: String,
    /// 긴 변의 픽셀 수
    pub px: u32,
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
        let (w, h, pixels) = render(&req.path, req.px).unwrap_or_default();
        shell::post_box(owner.get(), WM_APP_THUMB, Box::new(Image { key: req.key, w, h, pixels }));
    }
}

/// 셸이 주는 그림: 사진·동영상·문서는 썸네일, 나머지는 큰 아이콘
pub fn render(path: &str, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let hbmp = factory.GetImage(SIZE { cx: px as i32, cy: px as i32 }, SIIGBF_RESIZETOFIT).ok()?;
        let r = read_bitmap(hbmp);
        let _ = DeleteObject(hbmp.into());
        r
    }
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
        let (w, h, px) = render(&exe.to_string_lossy(), 96).expect("이미지");
        assert!(w > 0 && h > 0 && w <= 96 && h <= 96, "{w}x{h}");
        assert_eq!(px.len(), (w * h * 4) as usize);
        let windir = std::env::var("WINDIR").unwrap();
        let (w, h, _) = render(&windir, 256).expect("폴더 이미지");
        assert!(w > 0 && h > 0);
    }
}
