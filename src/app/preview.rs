//! 오른쪽 미리 보기 창 (Alt+P). 활성 패널에서 선택한 항목을 보여 준다.
//! 사진은 셸 썸네일, 텍스트는 앞부분, 그 밖은 Windows 미리 보기 처리기(PDF·Office 등),
//! 처리기가 없으면 큰 아이콘과 정보.
//! 머리의 핀으로 파일을 고정하면 선택을 바꿔도 그 파일을 계속 보여 준다. 고정한 파일은 파일 번호로 기억해,
//! 이름이 바뀌거나 옮겨지면 새 경로를 따라가고 지워지면 고정을 푼다 (확인은 뒤에서).
//! 미리 보기 처리기는 다른 프로세스에서 돌고 호출 중에 메시지를 돌리므로 App 빌림 밖(execute)에서만 다룬다.

use super::*;
use crate::thumbs::Request;
use crate::WM_APP_PREVIEW;
use windows::Win32::Graphics::DirectWrite::IDWriteTextLayout;
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory, WICDecodeMetadataCacheOnDemand};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, CLSIDFromString, CoCreateInstance, STGM_READ, STGM_SHARE_DENY_NONE};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};
use windows::Win32::UI::Shell::{
    ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, AssocQueryStringW, IInitializeWithItem, IPreviewHandler, IPreviewHandlerVisuals,
    IShellItem, SHCreateItemFromParsingName, SHCreateStreamOnFileEx,
};
use windows::core::{GUID, Interface, PWSTR};

pub(super) const PREVIEW_W: f32 = 340.0;
pub(super) const TIMER_PREVIEW: usize = 30;
/// 머리: 이름표·단추 줄, 이름 줄, 고정 칩·종류 줄
const HEAD_H: f32 = 108.0;
const BAR_H: f32 = 44.0;
const BTN: f32 = 28.0;
const G_OPEN: u16 = 0xE8A7;
const CHIP_PINNED: &str = "파일 고정";
const CHIP_FOLLOW: &str = "선택 따라가기";
/// 아래 정보 줄 (수정한 날짜·위치) 높이
const INFO_H: f32 = 76.0;
/// 정보 카드의 그림 크기(DIP)
const INFO_IMG: f32 = 128.0;
/// 이 크기 이하면 확장자를 몰라도 텍스트인지 열어 본다
const SNIFF_MAX: u64 = 2 * 1024 * 1024;
const TEXT_READ: u64 = 64 * 1024;
const TEXT_LINES: usize = 600;

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "jfif", "png", "gif", "bmp", "dib", "webp", "heic", "heif", "avif", "tif", "tiff", "ico"];
const TEXT_EXTS: &[&str] = &[
    "txt", "log", "md", "markdown", "csv", "tsv", "json", "jsonc", "xml", "yaml", "yml", "toml", "ini", "cfg", "conf", "config", "env",
    "gitignore", "editorconfig", "properties", "rs", "c", "h", "cpp", "hpp", "cc", "cs", "java", "kt", "kts", "go", "py", "rb", "php",
    "js", "mjs", "cjs", "ts", "tsx", "jsx", "css", "scss", "less", "html", "htm", "vue", "svelte", "sql", "sh", "bash", "zsh", "ps1",
    "psm1", "bat", "cmd", "lua", "r", "swift", "dart", "gradle", "cmake", "srt", "vtt", "nfo", "reg", "lock",
];

#[derive(Clone)]
struct Target {
    path: String,
    name: String,
    kind: Kind,
    size: u64,
    modified: u64,
    type_name: String,
    icon: i32,
}

enum Content {
    /// 고른 직후 잠깐 기다리는 중 (키를 누르고 있을 때 하나하나 열지 않도록)
    Waiting,
    Image,
    Text(String),
    Handler(GUID),
    Info,
    /// 인터넷에서 받은 파일이라 미리 보기 처리기로 열지 않았다
    Blocked,
}

/// 고정해 둔 파일. 선택을 바꿔도 target을 바꾸지 않는다
struct Pin {
    /// (볼륨, 파일 번호). 고정한 직후 뒤에서 읽어 채운다. 번호로 다시 열 수 있는 것은 같은 NTFS 드라이브 안뿐이라,
    /// 다른 드라이브로 옮기거나 공유 폴더·FAT USB에서 이름을 바꾸면 찾지 못하고 고정이 풀린다
    id: Option<(u32, u64)>,
    /// 늦게 도착한 이전 확인 결과를 버리기 위한 순번 (고정할 때마다 바뀐다)
    round: u64,
    /// 뒤에서 확인하는 중이면 true. 그사이 다시 확인할 일이 생기면 again
    busy: bool,
    again: bool,
}

pub(super) struct Preview {
    target: Option<Target>,
    pin: Option<Pin>,
    /// 그림 파일의 가로×세로 (픽셀). 뒤에서 읽어 채운다
    dims: Option<(u32, u32)>,
    content: Content,
    /// 늦게 도착한 이전 결과를 버리기 위한 순번
    seq: u64,
    scroll: f32,
    layout: Option<(f32, IDWriteTextLayout)>,
    /// 처리기 창(호스트)을 둔 자리(클라이언트 픽셀). 숨겼으면 None
    host_rect: Option<RECT>,
    /// 지금 대상을 읽기 시작한 때 ('불러오는 중…'은 0.3초가 지나야 보인다)
    since: std::time::Instant,
}

impl Preview {
    pub(super) fn new() -> Preview {
        Preview { target: None, pin: None, dims: None, content: Content::Waiting, seq: next_seq(), scroll: 0.0, layout: None, host_rect: None, since: std::time::Instant::now() }
    }
}

/// 뒤에서 읽은 미리 보기 결과 (WM_APP_PREVIEW)
pub enum PreviewMsg {
    /// 텍스트 파일 앞부분. 이진 파일이면 None
    Text { seq: u64, text: Option<String> },
    /// 그림 파일의 가로×세로
    Dims { seq: u64, dims: (u32, u32) },
    /// 미리 보기 처리기로 열기 전에 확인한 인터넷 표시 (Mark of the Web)
    Zone { seq: u64, clsid: GUID, internet: bool },
    /// 고정한 파일의 지금 자리. 찾지 못했으면(지워짐·휴지통) None
    Pin { round: u64, found: Option<Found> },
}

pub struct Found {
    path: String,
    id: Option<(u32, u64)>,
    size: u64,
    modified: u64,
    dir: bool,
}

/// 뒤에서 읽은 결과가 지금 것인지 가리는 순번. 미리 보기를 닫았다 다시 열어도 되풀이되지 않게 앱 전체에서 하나씩 늘린다
/// (닫기 전에 시작한 읽기·확인이 늦게 도착해 새 대상에 붙지 않게)
fn next_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 휴지통이나, 열린 채 지워져 사라지기를 기다리는 곳($Extend\$Deleted). 파일 번호로 찾았더니 여기 있으면 지운 것으로 본다
fn in_trash(path: &str) -> bool {
    let p = path.to_lowercase();
    p.contains("\\$recycle.bin\\") || p.contains("\\$extend\\")
}

/// 고정한 파일이 지금 어디 있는지 뒤에서 확인한다.
/// 같은 경로에 파일이 있으면 그 경로를 지킨다: 저장할 때 원본을 백업 이름(notes.txt~ 등)으로 바꾸고 새로 쓰는 프로그램은
/// 번호가 바뀌지만 사용자에게는 같은 파일이다. 경로가 비었을 때만 번호로 옮겨 간 곳을 찾는다
fn spawn_pin_check(hwnd: HWND, round: u64, path: String, id: Option<(u32, u64)>) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        use std::os::windows::fs::MetadataExt;
        let at = match (crate::undo::file_id(&path), id) {
            (Some(_), _) => Some(path),
            (None, Some(want)) => crate::undo::path_of_file_id(&path, want).filter(|p| !in_trash(p)),
            (None, None) => None,
        };
        let found = at.and_then(|p| {
            let md = std::fs::metadata(&p).ok()?;
            Some(Found { id: crate::undo::file_id(&p), size: md.file_size(), modified: md.last_write_time(), dir: md.is_dir(), path: p })
        });
        post(owner.get(), WM_APP_PREVIEW, Box::new(PreviewMsg::Pin { round, found }));
    });
}

/// 그림 파일의 가로×세로를 뒤에서 읽는다 (WIC가 머리만 읽는다). 읽지 못하면 보내지 않는다
fn spawn_dims(hwnd: HWND, seq: u64, path: String) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let _com = shell::ComInit::sta();
        let dims = unsafe {
            CoCreateInstance::<_, IWICImagingFactory>(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok().and_then(|f| {
                let frame = f.CreateDecoderFromFilename(&HSTRING::from(path.as_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?.GetFrame(0).ok()?;
                let (mut w, mut h) = (0u32, 0u32);
                frame.GetSize(&mut w, &mut h).ok()?;
                (w > 0 && h > 0).then_some((w, h))
            })
        };
        if let Some(dims) = dims {
            post(owner.get(), WM_APP_PREVIEW, Box::new(PreviewMsg::Dims { seq, dims }));
        }
    });
}

/// 머리 첫 줄의 단추: (고정, 기본 앱으로 열기)
fn head_buttons(pr: R) -> (R, R) {
    let open = R::new(pr.r - 12.0 - BTN, pr.t + 8.0, pr.r - 12.0, pr.t + 8.0 + BTN);
    let pin = R::new(open.l - 4.0 - BTN, open.t, open.l - 4.0, open.b);
    (pin, open)
}

/// 고정 칩 (이름 아래 줄 왼쪽)
fn chip_rect(gfx: &Gfx, pr: R) -> R {
    let w = gfx.measure(CHIP_PINNED, Font::Small).max(gfx.measure(CHIP_FOLLOW, Font::Small)) + 22.0;
    let t = pr.t + BAR_H + 28.0;
    R::new(pr.l + 16.0, t, (pr.l + 16.0 + w).min(pr.r - 16.0), t + 24.0)
}

/// 그림 파일 확장자인지 (즐겨찾기 카드의 작은 그림)
pub(super) fn is_image_ext(ext: &str) -> bool {
    IMAGE_EXTS.contains(&ext)
}

fn ext_of(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() || name.starts_with('.') => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// 확장자에 등록된 미리 보기 처리기
fn handler_for(ext: &str) -> Option<GUID> {
    if ext.is_empty() {
        return None;
    }
    let mut buf = [0u16; 64];
    let mut len = buf.len() as u32;
    unsafe {
        AssocQueryStringW(
            ASSOCF_INIT_DEFAULTTOSTAR,
            ASSOCSTR_SHELLEXTENSION,
            &HSTRING::from(format!(".{ext}")),
            w!("{8895b1c6-b41f-4c1c-a562-0d564250836f}"),
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
        .ok()
        .ok()?;
        CLSIDFromString(&HSTRING::from(util::from_wide(&buf))).ok()
    }
}

/// 텍스트 파일 앞부분을 글자로. 이진 파일이면 None
pub fn decode_text(b: &[u8]) -> Option<String> {
    let utf16 = |b: &[u8], be: bool| {
        let units: Vec<u16> = b.chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&units)
    };
    let s = if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, false)
    } else if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, true)
    } else if b[..b.len().min(8192)].contains(&0) {
        return None;
    } else {
        match std::str::from_utf8(b) {
            Ok(s) => s.to_string(),
            // 읽기를 자른 자리가 글자 중간이면 그 앞까지
            Err(e) if e.error_len().is_none() => String::from_utf8_lossy(&b[..e.valid_up_to()]).into_owned(),
            Err(_) => ansi(b),
        }
    };
    let mut out = String::with_capacity(s.len().min(48 * 1024));
    for line in s.lines().take(TEXT_LINES) {
        out.push_str(&line.replace('\t', "    "));
        out.push('\n');
    }
    Some(out)
}

/// UTF-8이 아닌 옛 한글 파일(CP949 등)은 시스템 코드 페이지로
fn ansi(b: &[u8]) -> String {
    use windows::Win32::Globalization::{CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};
    unsafe {
        let n = MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), b, None);
        if n <= 0 {
            return String::from_utf8_lossy(b).into_owned();
        }
        let mut w = vec![0u16; n as usize];
        MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), b, Some(&mut w));
        String::from_utf16_lossy(&w)
    }
}

fn spawn_text(hwnd: HWND, seq: u64, path: String) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let text = std::fs::File::open(&path).and_then(|f| f.take(TEXT_READ).read_to_end(&mut buf)).ok().and_then(|_| decode_text(&buf));
        post(owner.get(), WM_APP_PREVIEW, Box::new(PreviewMsg::Text { seq, text }));
    });
}

/// Zone.Identifier 내용이 인터넷(3)·제한된 사이트(4) 영역인지
fn zone_is_internet(zone_identifier: &str) -> bool {
    zone_identifier.lines().filter_map(|l| l.trim().trim_matches('\0').strip_prefix("ZoneId=")).any(|z| z.trim().parse::<u32>().is_ok_and(|n| n >= 3))
}

/// 인터넷에서 받은 파일인지 (Mark of the Web). 탐색기는 2025년 10월 보안 업데이트부터 이런 파일의 미리 보기를 막는다:
/// 바깥 서버 경로를 숨긴 HTML·문서를 미리 보기만 해도 Windows 로그인 정보(NTLM 해시)가 그 서버로 갈 수 있어서다
fn from_internet(path: &str) -> bool {
    std::fs::read(format!("{path}:Zone.Identifier")).is_ok_and(|b| zone_is_internet(&String::from_utf8_lossy(&b)))
}

/// 미리 보기 처리기로 열기 전에 인터넷 표시를 뒤에서 확인한다 (네트워크 경로라도 창이 멈추지 않게)
fn spawn_zone(hwnd: HWND, seq: u64, path: String, clsid: GUID) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let internet = from_internet(&path);
        post(owner.get(), WM_APP_PREVIEW, Box::new(PreviewMsg::Zone { seq, clsid, internet }));
    });
}

/// 그림 요청 크기를 단계로 맞춰, 창 크기를 바꾸는 동안 매번 새로 만들지 않게 한다
fn image_px(longest_dip: f32, scale: f32) -> u32 {
    (((longest_dip * scale) / 256.0).ceil() as u32 * 256).clamp(256, 1536)
}

impl App {
    pub(super) fn preview_w(&self) -> f32 {
        PREVIEW_W.min(self.w * 0.4)
    }

    pub(super) fn preview_rect(&self) -> Option<R> {
        self.preview.as_ref()?;
        Some(R::new(self.w - MARGIN - self.preview_w(), WORKBAR_H, self.w - MARGIN, (self.h - MARGIN).max(WORKBAR_H + 100.0)))
    }

    /// 머리(아이콘·이름·종류)와 아래 정보 줄 사이
    fn preview_body(&self) -> Option<R> {
        self.preview_rect().map(|pr| {
            let t = pr.t + HEAD_H + 16.0;
            R::new(pr.l + 16.0, t, pr.r - 16.0, (pr.b - INFO_H - 16.0).max(t + 40.0))
        })
    }

    pub(super) fn toggle_preview(&mut self) {
        if self.preview.take().is_some() {
            unsafe {
                let _ = ShowWindow(self.preview_host, SW_HIDE);
            }
            self.pending.push(Deferred::PreviewSync);
        } else {
            self.preview = Some(Preview::new());
        }
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.layout_edits();
        self.invalidate();
    }

    /// 그리기 전에: 대상이 바뀌었으면 새로 읽을 준비, 처리기 창 자리 맞추기
    pub(super) fn sync_preview(&mut self) {
        let hwnd = self.hwnd;
        let p = self.active;
        // 검색 패널에서 결과를 고르고 있으면 칸의 선택 대신 그 결과
        let search = if self.preview.is_some() { self.search_preview_pick() } else { None };
        let App { panes, types, preview, .. } = self;
        let Some(pv) = preview.as_mut() else { return };
        let tab = panes[p].tab();
        let pinned = pv.pin.is_some();
        let from_search = search.filter(|_| !pinned).map(|(path, name, is_dir, size, modified, type_name, icon)| Target {
            path,
            name,
            kind: if is_dir { Kind::Dir } else { Kind::File },
            size,
            modified,
            type_name,
            icon,
        });
        let target = from_search.or_else(|| tab.entry_at(tab.cursor).filter(|e| e.selected && !pinned).and_then(|e| {
            let (type_name, icon) = type_info(types, e);
            Some(Target {
                path: tab.path_of(e)?,
                name: e.name_string(),
                kind: e.kind,
                size: e.size,
                modified: e.modified,
                type_name: if e.kind == Kind::Drive { drive_type_label(e.drive_type).to_string() } else { type_name },
                icon: if e.icon >= 0 { e.icon } else { icon },
            })
        }));
        let same = pinned
            || match (&pv.target, &target) {
                (Some(a), Some(b)) => a.path == b.path && a.modified == b.modified && a.size == b.size,
                (None, None) => true,
                _ => false,
            };
        if !same {
            pv.seq = next_seq();
            pv.since = std::time::Instant::now();
            pv.target = target;
            pv.content = Content::Waiting;
            pv.scroll = 0.0;
            pv.layout = None;
            pv.dims = None;
            unsafe {
                if pv.target.is_some() {
                    SetTimer(Some(hwnd), TIMER_PREVIEW, 120, None);
                } else {
                    let _ = KillTimer(Some(hwnd), TIMER_PREVIEW);
                }
            }
        }
        // 처리기 창: 처리기를 쓸 때만, 팝업이 열려 있으면 가리지 않게 숨긴다
        let want = match (&self.preview, self.settings.is_some()) {
            (Some(Preview { content: Content::Handler(_), .. }), false) => self.preview_body().map(|b| {
                let s = self.scale();
                RECT { left: (b.l * s).round() as i32, top: (b.t * s).round() as i32, right: (b.r * s).round() as i32, bottom: (b.b * s).round() as i32 }
            }),
            _ => None,
        };
        let Some(pv) = self.preview.as_mut() else { return };
        if want != pv.host_rect {
            pv.host_rect = want;
            unsafe {
                match want {
                    Some(r) => {
                        let _ = SetWindowPos(self.preview_host, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
                    }
                    None => {
                        let _ = ShowWindow(self.preview_host, SW_HIDE);
                    }
                }
            }
            self.pending.push(Deferred::PreviewSync);
        } else if !same {
            self.pending.push(Deferred::PreviewSync);
        }
    }

    /// 잠깐 기다린 뒤: 무엇으로 보여 줄지 정하고 읽기 시작
    pub(super) fn start_preview(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_PREVIEW);
        }
        let hwnd = self.hwnd;
        let Some(pv) = self.preview.as_mut() else { return };
        let Some(t) = pv.target.clone() else { return };
        let ext = ext_of(&t.name);
        pv.content = if t.kind != Kind::File {
            Content::Info
        } else if IMAGE_EXTS.contains(&ext.as_str()) {
            spawn_dims(hwnd, pv.seq, t.path.clone());
            Content::Image
        } else if TEXT_EXTS.contains(&ext.as_str()) {
            spawn_text(hwnd, pv.seq, t.path.clone());
            Content::Waiting
        } else if let Some(clsid) = handler_for(&ext) {
            spawn_zone(hwnd, pv.seq, t.path.clone(), clsid);
            Content::Waiting
        } else if t.size <= SNIFF_MAX {
            spawn_text(hwnd, pv.seq, t.path.clone());
            Content::Waiting
        } else {
            Content::Info
        };
        crate::diag::log(|| format!("preview: {} -> {}", t.name, match pv.content {
            Content::Image => "image",
            Content::Handler(_) => "handler",
            Content::Info => "info",
            Content::Blocked => "blocked",
            _ => "text",
        }));
        self.invalidate();
    }

    pub(super) fn on_preview_msg(&mut self, m: PreviewMsg) {
        let Some(pv) = self.preview.as_mut() else { return };
        match m {
            PreviewMsg::Text { seq, text } if seq == pv.seq => {
                pv.content = match text {
                    Some(t) => Content::Text(t),
                    None => Content::Info,
                };
                pv.layout = None;
            }
            PreviewMsg::Dims { seq, dims } if seq == pv.seq => pv.dims = Some(dims),
            PreviewMsg::Zone { seq, clsid, internet } if seq == pv.seq => {
                crate::diag::log(|| format!("preview: mark of the web = {internet}"));
                pv.content = if internet { Content::Blocked } else { Content::Handler(clsid) };
            }
            PreviewMsg::Pin { round, found } => return self.on_pin_checked(round, found),
            _ => return,
        }
        self.invalidate();
    }

    // ---------- 고정 ----------

    /// 핀·칩: 지금 보이는 파일을 고정하거나, 고정을 풀고 다시 선택을 따라간다
    pub(super) fn toggle_preview_pin(&mut self) {
        let Some(pv) = self.preview.as_mut() else { return };
        if pv.pin.take().is_none() {
            if pv.target.is_none() {
                return;
            }
            pv.pin = Some(Pin { id: None, round: next_seq(), busy: false, again: false });
            // 파일 번호를 읽어 둔다 (이름이 바뀌거나 옮겨져도 찾을 수 있게)
            self.preview_pin_check();
        }
        self.invalidate();
    }

    /// 고정해 둔 파일의 경로와 파일 번호 (세션·작업 공간에 저장한다)
    pub(super) fn preview_pin(&self) -> Option<(String, Option<(u32, u64)>)> {
        let pv = self.preview.as_ref()?;
        let pin = pv.pin.as_ref()?;
        pv.target.as_ref().map(|t| (t.path.clone(), pin.id))
    }

    /// 세션·작업 공간에 저장해 둔 고정 파일을 되살린다 (None이면 고정을 풀고 다시 선택을 따라간다).
    /// 크기·종류는 모르니 뒤에서 확인한 결과가 오면 채우고 그때 읽기 시작한다 (없어졌으면 고정을 푼다)
    pub(super) fn preview_restore_pin(&mut self, path: Option<&str>, id: Option<(u32, u64)>) {
        let Some(pv) = self.preview.as_mut() else { return };
        let Some(path) = path else {
            pv.pin = None;
            self.invalidate();
            return;
        };
        let name = util::file_name(path).to_string();
        // 드라이브 뿌리는 이름이 바뀌거나 옮겨지지 않으니 확인 없이 바로 보인다
        let drive = util::parent(path).is_none();
        let ext = if drive { String::new() } else { ext_of(&name) };
        let key = if drive { "\\dir".to_string() } else { ext.clone() };
        let (type_name, icon) = self.types.entry(key).or_insert_with(|| shell::type_info(&ext, drive)).clone();
        let (kind, type_name) = if drive {
            (Kind::Drive, drive_type_label(unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(&HSTRING::from(path)) }).to_string())
        } else {
            (Kind::File, type_name)
        };
        pv.target = Some(Target { path: path.to_string(), name, kind, size: 0, modified: 0, type_name, icon });
        pv.pin = Some(Pin { id, round: next_seq(), busy: false, again: false });
        pv.seq = next_seq();
        pv.since = std::time::Instant::now();
        pv.content = Content::Waiting;
        pv.scroll = 0.0;
        pv.layout = None;
        pv.dims = None;
        unsafe {
            if drive {
                SetTimer(Some(self.hwnd), TIMER_PREVIEW, 120, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_PREVIEW);
            }
        }
        // 이전 대상의 처리기를 내린다
        self.pending.push(Deferred::PreviewSync);
        self.preview_pin_check();
        self.invalidate();
    }

    /// 고정한 파일이 그대로 있는지 뒤에서 확인한다 (파일 작업 뒤, 폴더가 바뀐 알림 뒤, 창으로 돌아올 때)
    pub(super) fn preview_pin_check(&mut self) {
        let hwnd = self.hwnd;
        let Some(pv) = self.preview.as_mut() else { return };
        let (Some(pin), Some(t)) = (pv.pin.as_mut(), &pv.target) else { return };
        if t.kind == Kind::Drive {
            return;
        }
        if pin.busy {
            pin.again = true;
            return;
        }
        pin.busy = true;
        spawn_pin_check(hwnd, pin.round, t.path.clone(), pin.id);
    }

    fn on_pin_checked(&mut self, round: u64, found: Option<Found>) {
        let Some(pv) = self.preview.as_mut() else { return };
        let Some(pin) = pv.pin.as_mut().filter(|p| p.round == round) else { return };
        pin.busy = false;
        let again = std::mem::take(&mut pin.again);
        let Some(f) = found else {
            // 지워졌거나 휴지통으로 갔다: 고정을 풀고 다시 선택을 따라간다
            pv.pin = None;
            self.flash(self.active, "고정한 파일을 찾을 수 없어 고정을 풀었어요");
            self.invalidate();
            return;
        };
        pin.id = f.id.or(pin.id);
        let Some(t) = pv.target.as_mut() else { return };
        let moved = t.path != f.path;
        // 되살린 고정(종류를 몰라 파일로 둔 것)이 폴더였다
        let became_dir = f.dir && t.kind == Kind::File;
        if became_dir {
            t.kind = Kind::Dir;
            let (type_name, icon) = self.types.entry("\\dir".to_string()).or_insert_with(|| shell::type_info("", true)).clone();
            t.type_name = type_name;
            t.icon = icon;
        }
        let changed = became_dir || (t.kind == Kind::File && (t.size != f.size || t.modified != f.modified));
        if moved || changed {
            if moved {
                let name = util::file_name(&f.path).to_string();
                // 확장자가 바뀌었을 때만 종류·아이콘을 다시 정한다
                if !f.dir && ext_of(&name) != ext_of(&t.name) {
                    let ext = ext_of(&name);
                    let (type_name, icon) = self.types.entry(ext.clone()).or_insert_with(|| shell::type_info(&ext, false)).clone();
                    t.type_name = type_name;
                    t.icon = icon;
                }
                t.name = name;
            }
            if t.kind == Kind::File {
                t.size = f.size;
                t.modified = f.modified;
            }
            t.path = f.path;
            self.preview_reload();
        }
        if again {
            self.preview_pin_check();
        }
        self.invalidate();
    }

    /// 같은 대상을 처음부터 다시 읽는다 (고정한 파일이 옮겨지거나 바뀌었을 때)
    fn preview_reload(&mut self) {
        let Some(pv) = self.preview.as_mut() else { return };
        pv.seq = next_seq();
        pv.since = std::time::Instant::now();
        pv.content = Content::Waiting;
        pv.scroll = 0.0;
        pv.layout = None;
        pv.dims = None;
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_PREVIEW, 120, None);
        }
        self.pending.push(Deferred::PreviewSync);
    }

    /// 머리의 열기 단추: 파일은 기본 앱으로, 폴더·드라이브는 지금 칸에서 연다
    pub(super) fn preview_open(&mut self) {
        let Some(t) = self.preview.as_ref().and_then(|pv| pv.target.clone()) else { return };
        let p = self.active;
        if t.kind == Kind::File {
            self.pending.push(Deferred::Open { pane: p, paths: vec![t.path.clone()], dir: util::parent(&t.path) });
        } else {
            self.navigate(p, Location::Dir(t.path), None);
        }
    }

    /// 미리 보기 머리의 단추·칩 (그 밖의 미리 보기 자리는 None: 지금처럼 아무것도 누르지 않는다)
    pub(super) fn preview_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let pr = self.preview_rect().filter(|r| r.contains(x, y))?;
        let pv = self.preview.as_ref()?;
        let (pin, open) = head_buttons(pr);
        if pin.contains(x, y) {
            return Some(Hit::PreviewPin);
        }
        if open.contains(x, y) {
            return Some(Hit::PreviewOpen);
        }
        if pv.target.is_some() && chip_rect(&self.gfx, pr).contains(x, y) {
            return Some(Hit::PreviewChip);
        }
        None
    }

    pub(super) fn preview_tip(&self) -> Option<(R, String)> {
        let pr = self.preview_rect()?;
        let pv = self.preview.as_ref()?;
        pv.target.as_ref()?;
        let (pin, open) = head_buttons(pr);
        match self.hover {
            Hit::PreviewPin if pv.pin.is_some() => Some((pin, "고정 풀기 · 다시 선택을 따라가요".to_string())),
            Hit::PreviewPin => Some((pin, "이 파일을 고정 · 다른 파일을 골라도 계속 보여요".to_string())),
            Hit::PreviewOpen => Some((open, if pv.target.as_ref().is_some_and(|t| t.kind == Kind::File) { "기본 앱으로 열기" } else { "열기" }.to_string())),
            _ => None,
        }
    }

    /// 지금 띄워야 할 처리기 (execute 쪽에서 물어본다)
    fn handler_wanted(&self) -> Option<Wanted> {
        let pv = self.preview.as_ref()?;
        let (Content::Handler(clsid), Some(t), Some(r)) = (&pv.content, &pv.target, pv.host_rect) else { return None };
        Some(Wanted {
            seq: pv.seq,
            path: t.path.clone(),
            clsid: *clsid,
            host: self.preview_host,
            rect: RECT { left: 0, top: 0, right: r.right - r.left, bottom: r.bottom - r.top },
            bg: colorref(self.theme.pane),
            fg: colorref(self.theme.text),
        })
    }

    fn handler_failed(&mut self, seq: u64) {
        if let Some(pv) = self.preview.as_mut().filter(|pv| pv.seq == seq) {
            pv.content = Content::Info;
            self.invalidate();
        }
    }

    pub(super) fn preview_wheel(&mut self, dy: f32) -> bool {
        let Some(body) = self.preview_body() else { return false };
        let Some(pv) = self.preview.as_mut() else { return false };
        let Some((_, l)) = &pv.layout else { return true };
        let max = (Gfx::layout_height(l) + 16.0 - body.h()).max(0.0);
        pv.scroll = (pv.scroll + dy).clamp(0.0, max);
        self.invalidate();
        true
    }

    pub(super) fn paint_preview(&mut self, missing: &mut Vec<Request>) {
        let (Some(pr), Some(body)) = (self.preview_rect(), self.preview_body()) else { return };
        let t = self.theme;
        let scale = self.scale();
        let hover = self.hover;
        let (hwnd, short_dates) = (self.hwnd, self.look.dates != "full");
        let App { gfx, preview, thumb_failed, .. } = self;
        let Some(pv) = preview.as_mut() else { return };
        if !t.high_contrast && !t.dark {
            soft_shadow(gfx, pr, RADIUS, 6.0, 0.06, false);
        }
        gfx.fill_round(pr, RADIUS, t.pane);
        if t.high_contrast || t.dark {
            gfx.stroke_round(pr.inset(0.5, 0.5), RADIUS, t.pane_border, 1.0);
        }
        // 머리 첫 줄: 이름표, 고정(핀)·열기 단추. 고정했으면 핀이 켜진 모양
        let pinned = pv.pin.is_some();
        let (pin_r, open_r) = head_buttons(pr);
        gfx.text("미리 보기", R::new(pr.l + 16.0, pr.t, pin_r.l - 8.0, pr.t + 44.0), Font::Small, t.faint);
        let has = pv.target.is_some();
        let button_bg = |gfx: &Gfx, r: R, hovered: bool| -> Color {
            if hovered && has {
                gfx.fill_round(r, R_CTRL, t.hover);
            }
            if !has { t.faint } else if hovered { t.text } else { t.dim }
        };
        let c = button_bg(gfx, pin_r, hover == Hit::PreviewPin);
        sidebar::draw_pin(gfx, pin_r, pinned, c);
        let c = button_bg(gfx, open_r, hover == Hit::PreviewOpen);
        gfx.glyph(G_OPEN, open_r, false, c);
        let Some(target) = &pv.target else {
            gfx.text("파일을 선택하면\n여기에서 미리 봐요", R::new(pr.l + 16.0, pr.t + BAR_H + 8.0, pr.r - 16.0, pr.t + BAR_H + 88.0), Font::Hint, t.faint);
            return;
        };
        // 이름 줄: 아이콘, 이름
        let nt = pr.t + BAR_H;
        gfx.icon(target.icon, R::new(pr.l + 16.0, nt + 3.0, pr.l + 32.0, nt + 19.0));
        gfx.text(&target.name, R::new(pr.l + 42.0, nt, pr.r - 16.0, nt + 22.0), Font::Strong, t.text);
        // 고정 칩(누르면 고정·풀기)과 종류 · 크기 (그림이면 가로×세로까지)
        let chip = chip_rect(gfx, pr);
        let chip_hot = hover == Hit::PreviewChip;
        // 고정됨: 흰 면 + 가는 선 (켜짐 상태는 작업 줄 단추와 같은 모양), 마우스를 대면 옅게 겹친다
        gfx.fill_round(chip, chip.h() / 2.0, if pinned { t.pane } else { t.field });
        if chip_hot {
            gfx.fill_round(chip, chip.h() / 2.0, sidebar::hover_fill(&t));
        }
        if t.high_contrast || pinned {
            gfx.stroke_round(chip.inset(0.5, 0.5), chip.h() / 2.0, t.border, 1.0);
        }
        gfx.text(if pinned { CHIP_PINNED } else { CHIP_FOLLOW }, R::new(chip.l + 11.0, chip.t, chip.r - 6.0, chip.b), Font::Small, if pinned { t.text } else { t.dim });
        let size = matches!(target.kind, Kind::File).then(|| format_size(target.size));
        let dims = pv.dims.map(|(w, h)| format!("{w} × {h}"));
        let sub_r = R::new(chip.r + 10.0, chip.t, pr.r - 16.0, chip.b);
        let join = |parts: &[Option<&str>]| parts.iter().flatten().copied().collect::<Vec<_>>().join(" · ");
        let tries = [
            join(&[Some(&target.type_name), size.as_deref(), dims.as_deref()]),
            join(&[Some(&target.type_name), size.as_deref()]),
            join(&[size.as_deref().or(Some(&target.type_name))]),
        ];
        let sub = tries.iter().find(|s| gfx.measure(s, Font::Small) <= sub_r.w()).unwrap_or(&tries[2]);
        gfx.text(sub, sub_r, Font::Small, t.dim);
        gfx.fill(R::new(pr.l, pr.t + HEAD_H - 1.0, pr.r, pr.t + HEAD_H), t.border);

        // 아래 정보 줄: 수정한 날짜, 위치
        let info = R::new(pr.l, pr.b - INFO_H, pr.r, pr.b);
        gfx.fill(R::new(pr.l, info.t, pr.r, info.t + 1.0), t.border);
        let label_w = gfx.measure("수정한 날짜", Font::Small) + 14.0;
        let mut y = info.t + 16.0;
        let mut row = |label: &str, value: &str| {
            if value.is_empty() {
                return;
            }
            gfx.text(label, R::new(info.l + 16.0, y, info.l + 16.0 + label_w, y + 20.0), Font::Small, t.faint);
            gfx.text(value, R::new(info.l + 16.0 + label_w, y, info.r - 16.0, y + 20.0), Font::Path, t.text);
            y += 24.0;
        };
        if target.kind != Kind::Drive {
            let full = format_time(target.modified);
            let short = if short_dates && target.modified != 0 { crate::listing::format_time_short(target.modified, crate::listing::now_filetime()) } else { String::new() };
            row("수정한 날짜", &if short.is_empty() || short == full { full } else { format!("{short} · {full}") });
        }
        row("위치", &util::parent(&target.path).unwrap_or_default());

        // 그림이 있으면 그리고, 없으면 요청 목록에 넣는다
        let mut image = |gfx: &mut Gfx, r: R, px: u32| -> bool {
            let key = thumb_key(&target.path, target.modified, px);
            if gfx.thumb(&key, r, 1.0) {
                return true;
            }
            if !thumb_failed.contains(&key) {
                // 미리 보기 그림은 목록보다 먼저 만든다
                missing.insert(0, Request { key, path: target.path.clone(), px });
            }
            false
        };
        match &pv.content {
            Content::Waiting | Content::Handler(_) => {}
            Content::Image => {
                let px = image_px(body.w().max(body.h()), scale);
                if !image(gfx, body, px) {
                    let failed = thumb_failed.contains(&thumb_key(&target.path, target.modified, px));
                    let waited = pv.since.elapsed().as_millis();
                    if failed {
                        gfx.text("미리 볼 수 없어요", body, Font::Hint, t.faint);
                    } else if waited >= 300 {
                        gfx.text("불러오는 중…", body, Font::Hint, t.faint);
                    } else {
                        // 그림이 곧 오면 글자를 보이지 않는다. 늦으면 0.3초에 다시 그려 띄운다
                        unsafe { SetTimer(Some(hwnd), TIMER_REDRAW, (300 - waited) as u32, None) };
                    }
                }
            }
            Content::Text(s) => {
                gfx.fill_round(body, 4.0, t.field);
                let w = body.w() - 24.0;
                if pv.layout.as_ref().is_none_or(|(lw, _)| (*lw - w).abs() > 0.5) {
                    pv.layout = gfx.layout(s, Font::Mono, w).map(|l| (w, l));
                }
                if let Some((_, l)) = &pv.layout {
                    gfx.clip(body.inset(1.0, 1.0));
                    gfx.draw_layout(l, body.l + 12.0, body.t + 10.0 - pv.scroll, t.text);
                    gfx.unclip();
                }
            }
            Content::Blocked => {
                gfx.text(
                    "인터넷에서 받은 파일이라 미리 보기를 막았어요\n바깥 주소를 숨긴 파일은 미리 보기만으로도 Windows 로그인 정보를 보낼 수 있어요\n믿을 수 있는 파일이면 파일 속성에서 ‘차단 해제’를 누르세요",
                    body,
                    Font::Hint,
                    t.faint,
                );
            }
            // 폴더·드라이브·알 수 없는 파일: 큰 그림(없으면 아이콘)만 가운데에
            Content::Info => {
                let (cx, cy) = ((body.l + body.r) / 2.0, (body.t + body.b) / 2.0);
                let ir = R::new(cx - INFO_IMG / 2.0, cy - INFO_IMG / 2.0, cx + INFO_IMG / 2.0, cy + INFO_IMG / 2.0);
                if !image(gfx, ir, (INFO_IMG * scale).round() as u32) {
                    gfx.icon(target.icon, R::new(cx - 16.0, cy - 16.0, cx + 16.0, cy + 16.0));
                }
            }
        }
    }
}

pub(super) fn thumb_key(path: &str, modified: u64, px: u32) -> String {
    format!("{px}|{modified}|{path}")
}

// ---------- 미리 보기 처리기 (App 빌림 밖에서) ----------

struct Wanted {
    seq: u64,
    path: String,
    clsid: GUID,
    host: HWND,
    rect: RECT,
    bg: COLORREF,
    fg: COLORREF,
}

struct Loaded {
    seq: u64,
    handler: IPreviewHandler,
    rect: RECT,
}

thread_local! {
    static LOADED: RefCell<Option<Loaded>> = const { RefCell::new(None) };
    /// 처리기를 여는 동안(다른 프로세스 호출 중 메시지가 돈다) 다시 들어온 요청은 끝난 뒤 한 번 더 맞춘다
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static AGAIN: Cell<bool> = const { Cell::new(false) };
}

/// 띄워 둔 처리기를 App이 원하는 상태(없음 / 다른 파일 / 크기 변경)에 맞춘다
pub(super) fn sync_handler() {
    if BUSY.get() {
        AGAIN.set(true);
        return;
    }
    BUSY.set(true);
    loop {
        AGAIN.set(false);
        let want = with_app(|a| a.handler_wanted()).flatten();
        let cur = LOADED.with(|l| l.borrow().as_ref().map(|l| (l.seq, l.rect)));
        match (&want, cur) {
            (Some(w), Some((seq, rect))) if w.seq == seq => {
                if w.rect != rect {
                    LOADED.with(|l| {
                        if let Some(l) = l.borrow_mut().as_mut() {
                            l.rect = w.rect;
                            unsafe {
                                let _ = l.handler.SetRect(&w.rect);
                            }
                        }
                    });
                }
            }
            _ => {
                unload_handler();
                if let Some(w) = want {
                    let t = std::time::Instant::now();
                    match unsafe { open_handler(&w) } {
                        Ok(handler) => {
                            crate::diag::log(|| format!("preview handler: opened in {:?}", t.elapsed()));
                            LOADED.with(|l| *l.borrow_mut() = Some(Loaded { seq: w.seq, handler, rect: w.rect }));
                        }
                        Err(e) => {
                            crate::diag::log(|| format!("preview handler: failed {e:?}"));
                            with_app(|a| a.handler_failed(w.seq));
                        }
                    }
                }
            }
        }
        if !AGAIN.get() {
            break;
        }
    }
    BUSY.set(false);
}

/// 창을 닫을 때와 다른 파일로 바뀔 때
pub fn unload_handler() {
    if let Some(old) = LOADED.with(|l| l.borrow_mut().take()) {
        unsafe {
            let _ = old.handler.Unload();
        }
    }
}

unsafe fn open_handler(w: &Wanted) -> windows::core::Result<IPreviewHandler> {
    unsafe {
        // 처리기가 죽어도 이 앱은 살아 있도록 별도 프로세스(prevhost)에서만 연다
        let h: IPreviewHandler = CoCreateInstance(&w.clsid, None, CLSCTX_LOCAL_SERVER)?;
        let path = HSTRING::from(w.path.as_str());
        if let Ok(i) = h.cast::<IInitializeWithFile>() {
            i.Initialize(&path, STGM_READ.0)?;
        } else if let Ok(i) = h.cast::<IInitializeWithItem>() {
            let item: IShellItem = SHCreateItemFromParsingName(&path, None)?;
            i.Initialize(&item, STGM_READ.0)?;
        } else {
            let i = h.cast::<IInitializeWithStream>()?;
            let stream = SHCreateStreamOnFileEx(&path, (STGM_READ | STGM_SHARE_DENY_NONE).0, 0, false, None)?;
            i.Initialize(&stream, STGM_READ.0)?;
        }
        if let Ok(v) = h.cast::<IPreviewHandlerVisuals>() {
            let _ = v.SetBackgroundColor(w.bg);
            let _ = v.SetTextColor(w.fg);
        }
        h.SetWindow(w.host, &w.rect)?;
        h.DoPreview()?;
        Ok(h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 인터넷(3)·제한된 사이트(4)만 막고, 내 컴퓨터(0)·인트라넷(1)·신뢰할 수 있는 사이트(2)는 연다
    #[test]
    fn mark_of_the_web_blocks_internet_zones_only() {
        assert!(zone_is_internet("[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://example.com/\r\n"));
        assert!(zone_is_internet("[ZoneTransfer]\nZoneId=4\n"));
        assert!(!zone_is_internet("[ZoneTransfer]\r\nZoneId=2\r\n"));
        assert!(!zone_is_internet("[ZoneTransfer]\r\nZoneId=1\r\n"));
        assert!(!zone_is_internet(""));
        assert!(!from_internet("C:\\__sakadu_no_such_file__.pdf"));
    }

    #[test]
    fn decodes_common_encodings() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFhi\r\nthere").as_deref(), Some("hi\nthere\n"));
        assert_eq!(decode_text(b"\xFF\xFEh\0i\0").as_deref(), Some("hi\n"));
        assert_eq!(decode_text("한글".as_bytes()).as_deref(), Some("한글\n"));
        // 읽기를 글자 중간에서 자른 UTF-8
        let cut = &"가나".as_bytes()[..4];
        assert_eq!(decode_text(cut).as_deref(), Some("가\n"));
        assert_eq!(decode_text(b"a\tb").as_deref(), Some("a    b\n"));
    }

    #[test]
    fn rejects_binary() {
        assert_eq!(decode_text(b"MZ\x90\0\x03\0\0\0"), None);
    }

    #[test]
    fn extensions() {
        assert_eq!(ext_of("photo.JPG"), "jpg");
        assert_eq!(ext_of(".gitignore"), "gitignore");
        assert_eq!(ext_of("Makefile"), "");
        assert_eq!(ext_of("a.tar.gz"), "gz");
    }

    #[test]
    fn trash_locations_count_as_deleted() {
        assert!(in_trash("C:\\$Recycle.Bin\\S-1-5-21-1\\$RAB12.png"));
        assert!(in_trash("D:\\$Extend\\$Deleted\\0001"));
        assert!(!in_trash("C:\\Users\\a\\recycle.bin\\x.png"));
    }

    #[test]
    fn image_px_steps() {
        assert_eq!(image_px(300.0, 1.0), 512);
        assert_eq!(image_px(300.0, 1.5), 512);
        assert_eq!(image_px(5000.0, 2.0), 1536);
    }

    /// 이 PC에 등록된 처리기 조회 (없을 수도 있어 결과만 출력)
    #[test]
    #[ignore]
    fn lists_handlers() {
        for ext in ["pdf", "docx", "xlsx", "txt", "html", "mp4", "svg"] {
            println!("{ext}: {:?}", handler_for(ext));
        }
    }
}
