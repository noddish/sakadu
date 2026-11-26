//! 오른쪽 미리 보기 창 (Alt+P). 활성 패널에서 선택한 항목을 보여 준다.
//! 그림·글 파일을 고르면 저절로 뜨는 작은 카드는 preview_card.rs (이 창이 열려 있으면 카드는 뜨지 않는다).
//! 사진은 셸 썸네일, 텍스트는 앞부분, 그 밖은 Windows 미리 보기 처리기(PDF·Office 등. 엑셀 처리기는 매크로 통합 문서(xlsm)를 받지 않는다),
//! 처리기가 없으면 셸 썸네일(동영상 장면, 음악 앨범 그림 등), 그것도 없으면 큰 아이콘과 정보.
//! 한글 문서는 안에 든 미리 보기 그림(없으면 미리 보기 글), SVG는 직접 그린다 (thumbs.rs). 바로 가기는 가리키는 파일을 본다.
//! 머리의 핀으로 파일을 고정하면 선택을 바꿔도 그 파일을 계속 보여 준다. 고정한 파일은 파일 번호로 기억해,
//! 이름이 바뀌거나 옮겨지면 새 경로를 따라가고 지워지면 고정을 푼다 (확인은 뒤에서).
//! 미리 보기 처리기는 다른 프로세스(prevhost)에서 돈다. 열기·크기 맞추기·닫기는 따로 도는 일꾼 스레드가 해서,
//! 큰 엑셀·PDF를 처리기가 오래 읽어도 창은 멈추지 않는다. 처리기가 다 그리기 전까지는 '불러오는 중…'을 보인다.

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
/// 머리: 아이콘, 이름과 그 아래 종류·크기, 오른쪽에 고정·열기 단추 (자동 미리 보기 카드의 아래 줄과 같은 구성).
/// 그림 크기·수정한 날짜·위치는 이름에 마우스를 올리면 말풍선으로 보인다
const HEAD_H: f32 = 56.0;
const BTN: f32 = 28.0;
const G_OPEN: u16 = 0xE8A7;
/// 정보 카드의 그림 크기(DIP)
const INFO_IMG: f32 = 128.0;
/// 이 크기 이하면 확장자를 몰라도 텍스트인지 열어 본다
const SNIFF_MAX: u64 = 2 * 1024 * 1024;
const TEXT_READ: u64 = 64 * 1024;
const TEXT_LINES: usize = 600;

pub(super) use sakadu::ui::preview_card::{IMAGE_EXTS, TEXT_EXTS};

/// 셸이 그리지 못해 사카두가 직접 그리는 그림 형식 (thumbs.rs): SVG, 한글 문서의 미리 보기 그림, 포토샵의 합친 그림, 일러스트레이터 안의 PDF
pub(super) const DRAWN_EXTS: &[&str] = &["svg", "hwp", "psd", "psb", "ai"];

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
    /// 셸 썸네일만 (동영상 장면, 음악 앨범 그림 등): 없으면 큰 아이콘 대신 '미리 볼 수 없어요'
    Thumb,
    Text(String),
    /// html을 Edge로 찍은 화면 (page.rs). 찍지 못하면 글로 바꾼다
    Page,
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
    /// 처리기 창(호스트)을 둔 자리(클라이언트 픽셀). 처리기를 쓰지 않으면 None
    host_rect: Option<RECT>,
    /// 처리기가 다 열려 그렸다. 그 전에는 호스트 창을 숨기고 '불러오는 중…'을 보인다
    handler_ready: bool,
    /// 호스트 창을 보이고 있다
    host_shown: bool,
    /// 지금 대상을 읽기 시작한 때 ('불러오는 중…'은 0.3초가 지나야 보인다)
    since: std::time::Instant,
    /// 내용을 읽을 경로: 바로 가기면 가리키는 파일, 아니면 None(대상 그대로)
    source: Option<String>,
}

impl Preview {
    pub(super) fn new() -> Preview {
        Preview {
            target: None,
            pin: None,
            dims: None,
            content: Content::Waiting,
            seq: next_seq(),
            scroll: 0.0,
            layout: None,
            host_rect: None,
            handler_ready: false,
            host_shown: false,
            since: std::time::Instant::now(),
            source: None,
        }
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
    /// 한글 문서에 미리 보기 그림이 있다 (없으면 Text로 미리 보기 글을 보낸다)
    HwpImage { seq: u64 },
    /// 미리 보기 처리기가 열려 그렸다 / 열지 못했다 (일꾼 스레드에서)
    HandlerReady { seq: u64 },
    HandlerFailed { seq: u64 },
    /// 움직이는 gif의 프레임 (gif.rs). 움직이지 않을 gif면 None
    Gif { key: String, frames: Option<crate::thumbs::GifFrames> },
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
pub(super) fn next_seq() -> u64 {
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

/// 머리의 단추: (고정, 기본 앱으로 열기)
fn head_buttons(pr: R) -> (R, R) {
    let t = pr.t + (HEAD_H - BTN) / 2.0;
    let open = R::new(pr.r - 12.0 - BTN, t, pr.r - 12.0, t + BTN);
    let pin = R::new(open.l - 4.0 - BTN, open.t, open.l - 4.0, open.b);
    (pin, open)
}

/// 머리의 글자 자리: (이름, 그 아래 종류·크기)
fn head_text(pr: R) -> (R, R) {
    let (pin, _) = head_buttons(pr);
    let (l, r) = (pr.l + 40.0, pin.l - 8.0);
    (R::new(l, pr.t + 10.0, r, pr.t + 30.0), R::new(l, pr.t + 30.0, r, pr.t + 46.0))
}

/// 그림 파일 확장자인지 (즐겨찾기 카드의 작은 그림)
pub(super) fn is_image_ext(ext: &str) -> bool {
    IMAGE_EXTS.contains(&ext)
}

pub(super) fn ext_of(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() || name.starts_with('.') => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// 확장자에 등록된 셸 확장(iid 종류)의 CLSID
fn shell_extension(ext: &str, iid: PCWSTR) -> Option<GUID> {
    if ext.is_empty() {
        return None;
    }
    let mut buf = [0u16; 64];
    let mut len = buf.len() as u32;
    unsafe {
        AssocQueryStringW(ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, &HSTRING::from(format!(".{ext}")), iid, Some(PWSTR(buf.as_mut_ptr())), &mut len)
            .ok()
            .ok()?;
        CLSIDFromString(&HSTRING::from(util::from_wide(&buf))).ok()
    }
}

/// 확장자에 등록된 미리 보기 처리기.
/// (엑셀 처리기는 매크로 통합 문서를 경로·내용·확장자를 바꾼 복사본 어느 쪽으로 넘겨도 받지 않아, 같은 형식의 처리기로 대신 보지 않는다)
fn handler_for(ext: &str) -> Option<GUID> {
    shell_extension(ext, w!("{8895b1c6-b41f-4c1c-a562-0d564250836f}"))
}

/// 셸 썸네일을 만들 수 있는 형식인지 (문서 첫 장, 동영상 장면, 음악 앨범 그림 등). 바로 가기·실행 파일은 뺀다 (아이콘이 나온다)
pub(super) fn thumb_provider(ext: &str) -> bool {
    !matches!(ext, "" | "lnk" | "url" | "exe" | "dll" | "msi" | "appref-ms" | "ico" | "cur")
        && shell_extension(ext, w!("{e357fccd-a995-4576-b01f-234630154e96}")).is_some()
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
    Some(tidy_text(&s))
}

/// 미리 보기에 보일 글: 앞쪽 줄만, 탭은 칸 넷으로
fn tidy_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(48 * 1024));
    for line in s.lines().take(TEXT_LINES) {
        out.push_str(&line.replace('\t', "    "));
        out.push('\n');
    }
    out
}

/// 한글 문서: 미리 보기 그림이 있으면 그림으로, 없으면 미리 보기 글로
fn spawn_hwp(hwnd: HWND, seq: u64, path: String) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let _com = shell::ComInit::sta();
        let msg = if crate::thumbs::hwp_has_image(&path) {
            PreviewMsg::HwpImage { seq }
        } else {
            PreviewMsg::Text { seq, text: crate::thumbs::hwp_text(&path).map(|s| tidy_text(&s)) }
        };
        post(owner.get(), WM_APP_PREVIEW, Box::new(msg));
    });
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

pub(super) fn spawn_text(hwnd: HWND, seq: u64, path: String) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let text = std::fs::File::open(&path).and_then(|f| f.take(TEXT_READ).read_to_end(&mut buf)).ok().and_then(|_| decode_text(&buf));
        post(owner.get(), WM_APP_PREVIEW, Box::new(PreviewMsg::Text { seq, text }));
    });
}

/// 압축 파일 안의 맨 위 항목을 글로 보인다 (폴더 먼저). 셸이 읽지 못하는 압축 파일은 정보만
fn spawn_archive_list(hwnd: HWND, seq: u64, path: String) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let _com = shell::ComInit::sta();
        let text = crate::archive::opens_as_folder(&path).then(|| crate::archive::list(&path, &path).ok()).flatten().map(|v| sakadu::archive_path::archive_text(&v));
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
pub(super) fn image_px(longest_dip: f32, scale: f32) -> u32 {
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

    /// 머리(아이콘·이름·종류) 아래 전부
    fn preview_body(&self) -> Option<R> {
        self.preview_rect().map(|pr| {
            let t = pr.t + HEAD_H;
            R::new(pr.l + 16.0, t, pr.r - 16.0, (pr.b - 16.0).max(t + 40.0))
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
        let show = want.is_some() && pv.handler_ready;
        if want != pv.host_rect || show != pv.host_shown {
            let moved = want != pv.host_rect;
            pv.host_rect = want;
            pv.host_shown = show;
            unsafe {
                match want {
                    Some(r) => {
                        let vis = if show { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
                        let _ = SetWindowPos(self.preview_host, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE | vis);
                    }
                    None => {
                        let _ = ShowWindow(self.preview_host, SW_HIDE);
                    }
                }
            }
            if moved || !same {
                self.pending.push(Deferred::PreviewSync);
            }
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
        // html 화면을 이 크기로 이미 찍지 못했는지 보려고 그림 크기를 미리 잰다 (그리기와 같은 키)
        let page_px = self.preview_body().map(|b| image_px(b.w().max(b.h()), self.scale()));
        let Some(pv) = self.preview.as_mut() else { return };
        let Some(t) = pv.target.clone() else { return };
        // 바로 가기는 가리키는 파일을 본다 (링크에 적힌 대상만 읽고, 대상에 다시 묻지 않는다)
        pv.source = None;
        let mut ext = ext_of(&t.name);
        if t.kind == Kind::File && ext == "lnk"
            && let Some((target, false)) = shell::shortcut_target(&t.path)
        {
            ext = ext_of(util::file_name(&target));
            pv.source = Some(target);
        }
        let path = pv.source.clone().unwrap_or_else(|| t.path.clone());
        let e = ext.as_str();
        // 압축 파일 안의 파일은 정보만 (풀지 않고는 읽을 수 없다)
        pv.content = if t.kind != Kind::File || e == "lnk" || crate::archive::inside(&path) {
            Content::Info
        } else if crate::archive::is_archive_name(&t.name) {
            spawn_archive_list(hwnd, pv.seq, path);
            Content::Waiting
        } else if IMAGE_EXTS.contains(&e) {
            spawn_dims(hwnd, pv.seq, path);
            Content::Image
        } else if e == "hwp" {
            spawn_hwp(hwnd, pv.seq, path);
            Content::Waiting
        } else if DRAWN_EXTS.contains(&e) {
            Content::Image
        } else if crate::page::is_page_ext(e)
            && crate::page::available()
            // 앞서 찍지 못한 html(인터넷에서 받은 파일 등)은 실패가 다시 오지 않으니 처음부터 코드로
            && !page_px.is_some_and(|px| self.thumb_failed.contains(&thumb_key(&path, t.modified, px)))
        {
            Content::Page
        } else if TEXT_EXTS.contains(&e) {
            spawn_text(hwnd, pv.seq, path);
            Content::Waiting
        } else if let Some(clsid) = handler_for(e) {
            spawn_zone(hwnd, pv.seq, path, clsid);
            Content::Waiting
        } else if thumb_provider(e) {
            Content::Thumb
        } else if t.size <= SNIFF_MAX {
            spawn_text(hwnd, pv.seq, path);
            Content::Waiting
        } else {
            Content::Info
        };
        crate::diag::log(|| format!("preview: {} -> {}", t.name, match pv.content {
            Content::Image => "image",
            Content::Thumb => "thumbnail",
            Content::Page => "page",
            Content::Handler(_) => "handler",
            Content::Info => "info",
            Content::Blocked => "blocked",
            _ => "text",
        }));
        self.invalidate();
    }

    pub(super) fn on_preview_msg(&mut self, m: PreviewMsg) {
        let m = match m {
            PreviewMsg::Gif { key, frames } => return self.on_gif_frames(key, frames),
            m => m,
        };
        // 자동 미리 보기 카드가 읽어 달라고 한 글이면 카드가 받는다
        if let PreviewMsg::Text { seq, text } = &m
            && self.card_text(*seq, text)
        {
            return;
        }
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
            PreviewMsg::HwpImage { seq } if seq == pv.seq => pv.content = Content::Image,
            PreviewMsg::Zone { seq, clsid, internet } if seq == pv.seq => {
                crate::diag::log(|| format!("preview: mark of the web = {internet}"));
                pv.content = if internet { Content::Blocked } else { Content::Handler(clsid) };
                pv.handler_ready = false;
            }
            PreviewMsg::HandlerReady { seq } if seq == pv.seq => pv.handler_ready = true,
            PreviewMsg::HandlerFailed { seq } => return self.handler_failed(seq),
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

    /// 미리 보기 머리의 단추와 이름 자리 (그 밖의 미리 보기 자리는 None: 지금처럼 아무것도 누르지 않는다)
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
        // 머리의 나머지(아이콘·이름): 누르면 아무 일도 없고, 올려 두면 자세한 정보가 뜬다
        if pv.target.is_some() && y < pr.t + HEAD_H {
            return Some(Hit::PreviewHead);
        }
        None
    }

    pub(super) fn preview_tip(&self) -> Option<(R, String)> {
        let pr = self.preview_rect()?;
        let pv = self.preview.as_ref()?;
        let target = pv.target.as_ref()?;
        let (pin, open) = head_buttons(pr);
        match self.hover {
            Hit::PreviewPin if pv.pin.is_some() => Some((pin, "고정 풀기 · 다시 선택을 따라가요".to_string())),
            Hit::PreviewPin => Some((pin, "이 파일을 고정 · 다른 파일을 골라도 계속 보여요".to_string())),
            Hit::PreviewOpen => Some((open, if target.kind == Kind::File { "기본 앱으로 열기" } else { "열기" }.to_string())),
            // 이름: 머리에 늘어놓지 않는 것들 (그림 크기, 수정한 날짜, 위치)
            Hit::PreviewHead => {
                let mut parts = Vec::new();
                if let Some((w, h)) = pv.dims {
                    parts.push(format!("{w} × {h}"));
                }
                let modified = format_time(target.modified);
                if target.kind != Kind::Drive && !modified.is_empty() {
                    parts.push(modified);
                }
                parts.extend(util::parent(&target.path));
                (!parts.is_empty()).then(|| (head_text(pr).0, parts.join(" · ")))
            }
            _ => None,
        }
    }

    /// 지금 띄워야 할 처리기 (execute 쪽에서 물어본다)
    fn handler_wanted(&self) -> Option<Wanted> {
        let pv = self.preview.as_ref()?;
        let (Content::Handler(clsid), Some(t), Some(r)) = (&pv.content, &pv.target, pv.host_rect) else { return None };
        Some(Wanted {
            seq: pv.seq,
            path: pv.source.clone().unwrap_or_else(|| t.path.clone()),
            clsid: *clsid,
            host: shell::SendHwnd(self.preview_host),
            owner: shell::SendHwnd(self.hwnd),
            rect: RECT { left: 0, top: 0, right: r.right - r.left, bottom: r.bottom - r.top },
            bg: colorref(self.theme.pane),
            fg: colorref(self.theme.text),
        })
    }

    /// 미리 보기 창에 보이는 gif: (썸네일 키, 경로, 긴 변 px). 그림으로 보이는 gif만 (gif.rs가 움직인다)
    pub(super) fn pane_gif(&self) -> Option<(String, String, u32)> {
        let pv = self.preview.as_ref()?;
        let t = pv.target.as_ref()?;
        let body = self.preview_body()?;
        let source = pv.source.clone().unwrap_or_else(|| t.path.clone());
        if !matches!(pv.content, Content::Image) || ext_of(util::file_name(&source)) != "gif" {
            return None;
        }
        let px = image_px(body.w().max(body.h()), self.scale());
        Some((thumb_key(&source, t.modified, px), source, px))
    }

    /// 그림을 만들지 못했다(key): 미리 보기 창이 html 화면을 기다리고 있었으면 지금처럼 글(코드)로 보인다
    pub(super) fn page_shot_failed(&mut self, key: &str) {
        let hwnd = self.hwnd;
        let Some(pv) = self.preview.as_mut() else { return };
        let Some(t) = &pv.target else { return };
        let source = pv.source.clone().unwrap_or_else(|| t.path.clone());
        if matches!(pv.content, Content::Page) && key.ends_with(&format!("|{}|{source}", t.modified)) {
            spawn_text(hwnd, pv.seq, source);
            pv.content = Content::Waiting;
        }
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
        let hwnd = self.hwnd;
        let anim = self.gif_now();
        let App { gfx, preview, thumb_failed, .. } = self;
        let Some(pv) = preview.as_mut() else { return };
        if !t.high_contrast && !t.dark {
            soft_shadow(gfx, pr, RADIUS, 6.0, 0.06, false);
        }
        gfx.fill_round(pr, RADIUS, t.pane);
        if t.high_contrast || t.dark {
            gfx.stroke_round(pr.inset(0.5, 0.5), RADIUS, t.pane_border, 1.0);
        }
        // 머리: 고정(핀)·열기 단추. 고정했으면 핀 단추가 옅은 강조 색으로 켜진다
        let pinned = pv.pin.is_some();
        let (pin_r, open_r) = head_buttons(pr);
        let has = pv.target.is_some();
        let button_bg = |gfx: &Gfx, r: R, hovered: bool| -> Color {
            if hovered && has {
                gfx.fill_round(r, R_CTRL, sidebar::hover_fill(&t));
            }
            if !has { t.faint } else if hovered { t.text } else { t.dim }
        };
        if pinned && has && !t.high_contrast {
            gfx.fill_round(pin_r, R_CTRL, t.accent_soft);
            sidebar::draw_pin(gfx, pin_r, true, t.chip_text);
        } else {
            let c = button_bg(gfx, pin_r, hover == Hit::PreviewPin);
            sidebar::draw_pin(gfx, pin_r, pinned, c);
        }
        let c = button_bg(gfx, open_r, hover == Hit::PreviewOpen);
        gfx.glyph(G_OPEN, open_r, false, c);
        let Some(target) = &pv.target else {
            gfx.text("파일을 선택하면\n여기에서 미리 봐요", R::new(pr.l + 16.0, pr.t + HEAD_H + 8.0, pr.r - 16.0, pr.t + HEAD_H + 88.0), Font::Hint, t.faint);
            return;
        };
        // 아이콘, 이름, 그 아래 종류 · 크기 (좁으면 크기만)
        let (name_r, sub_r) = head_text(pr);
        gfx.icon(target.icon, R::new(pr.l + 16.0, pr.t + 20.0, pr.l + 32.0, pr.t + 36.0));
        gfx.text(&target.name, name_r, Font::Strong, t.text);
        let size = matches!(target.kind, Kind::File).then(|| format_size(target.size));
        let both = match &size {
            Some(s) => format!("{} · {s}", target.type_name),
            None => target.type_name.clone(),
        };
        let sub = match size {
            Some(s) if gfx.measure(&both, Font::Small) > sub_r.w() => s,
            _ => both,
        };
        gfx.text(&sub, sub_r, Font::Small, t.dim);

        // 그림이 있으면 그리고, 없으면 요청 목록에 넣는다 (only_thumb이면 썸네일만: 큰 아이콘을 받지 않는다)
        let source = pv.source.clone().unwrap_or_else(|| target.path.clone());
        let mut image = |gfx: &mut Gfx, r: R, px: u32, only_thumb: bool| -> bool {
            let key = thumb_key(&source, target.modified, px);
            // 움직이는 gif는 지금 프레임을, 아니면 썸네일(첫 장)을
            if anim.as_ref().is_some_and(|(k, i)| *k == key && gfx.anim_frame(&key, *i, r, 1.0)) || gfx.thumb(&key, r, 1.0) {
                return true;
            }
            if !thumb_failed.contains(&key) {
                // 미리 보기 그림은 목록보다 먼저 만든다
                missing.insert(0, Request { key, path: source.clone(), px, only_thumb });
            }
            false
        };
        match &pv.content {
            Content::Waiting => {}
            Content::Handler(_) if pv.handler_ready => {}
            Content::Handler(_) => {
                // 큰 엑셀·PDF는 처리기가 오래 읽는다: 창은 그대로 쓸 수 있고, 다 그리면 처리기 창이 덮는다
                let waited = pv.since.elapsed().as_millis();
                if waited >= 300 {
                    gfx.text("불러오는 중…", body, Font::Hint, t.faint);
                } else {
                    unsafe { SetTimer(Some(hwnd), TIMER_REDRAW, (300 - waited) as u32, None) };
                }
            }
            Content::Image | Content::Thumb | Content::Page => {
                let px = image_px(body.w().max(body.h()), scale);
                // 페이지는 위에서부터 읽으니 창의 위쪽에 붙인다 (찍은 화면의 비율로 자리를 잡는다)
                let r = match pv.content {
                    Content::Page => R::new(body.l, body.t, body.r, (body.t + body.w() * crate::page::VIEW.1 as f32 / crate::page::VIEW.0 as f32).min(body.b)),
                    _ => body,
                };
                if !image(gfx, r, px, matches!(pv.content, Content::Thumb | Content::Page)) {
                    let failed = thumb_failed.contains(&thumb_key(&source, target.modified, px));
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
                gfx.fill_round(body, R_CTRL, t.field);
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
                if !image(gfx, ir, (INFO_IMG * scale).round() as u32, false) {
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

#[derive(Clone)]
struct Wanted {
    seq: u64,
    path: String,
    clsid: GUID,
    /// 처리기가 그려 넣을 창과 결과를 알릴 창
    host: shell::SendHwnd,
    owner: shell::SendHwnd,
    rect: RECT,
    bg: COLORREF,
    fg: COLORREF,
}

/// 처리기 일꾼에게 보내는 일
enum HandlerCmd {
    /// 이 파일의 처리기를 연다 (열려 있던 것은 닫는다)
    Show(Wanted),
    /// 열려 있는 처리기(순번)의 크기를 맞춘다
    Resize(u64, RECT),
    Unload,
    /// 앱을 닫는다: 처리기를 닫고 알린 뒤 끝낸다
    Quit(std::sync::mpsc::Sender<()>),
}

thread_local! {
    /// 일꾼에게 마지막으로 보낸 상태 (순번, 자리). 같은 일을 다시 보내지 않는다
    static SENT: Cell<Option<(u64, RECT)>> = const { Cell::new(None) };
    static WORKER: RefCell<Option<std::sync::mpsc::Sender<HandlerCmd>>> = const { RefCell::new(None) };
}

fn send_handler(cmd: HandlerCmd) {
    WORKER.with(|w| {
        let mut w = w.borrow_mut();
        let tx = w.get_or_insert_with(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || handler_worker(rx));
            tx
        });
        let _ = tx.send(cmd);
    });
}

/// 띄워 둔 처리기를 App이 원하는 상태(없음 / 다른 파일 / 크기 변경)에 맞추도록 일꾼에게 맡긴다 (창은 기다리지 않는다)
pub(super) fn sync_handler() {
    let want = with_app(|a| a.handler_wanted()).flatten();
    let sent = SENT.get();
    let cmd = match (&want, sent) {
        (Some(w), Some((seq, rect))) if w.seq == seq => (w.rect != rect).then_some(HandlerCmd::Resize(seq, w.rect)),
        (Some(w), _) => Some(HandlerCmd::Show(w.clone())),
        (None, Some(_)) => Some(HandlerCmd::Unload),
        (None, None) => None,
    };
    SENT.set(want.map(|w| (w.seq, w.rect)));
    if let Some(cmd) = cmd {
        send_handler(cmd);
    }
}

/// 창을 닫을 때: 처리기를 닫고 잠깐(최대 2초) 기다린다. 큰 파일을 읽는 중이라 늦으면 기다리지 않고 닫는다
pub fn unload_handler() {
    SENT.set(None);
    if WORKER.with(|w| w.borrow().is_none()) {
        return;
    }
    let (ack, done) = std::sync::mpsc::channel();
    send_handler(HandlerCmd::Quit(ack));
    WORKER.with(|w| *w.borrow_mut() = None);
    let _ = done.recv_timeout(std::time::Duration::from_secs(2));
}

/// 처리기 일꾼: 밀린 일은 마지막 열기(또는 닫기)부터만 한다 (파일을 빠르게 옮겨 다니면 지나간 파일은 열지 않는다)
fn handler_worker(rx: std::sync::mpsc::Receiver<HandlerCmd>) {
    let _com = shell::ComInit::sta();
    let mut loaded: Option<(u64, IPreviewHandler)> = None;
    let unload = |loaded: &mut Option<(u64, IPreviewHandler)>| {
        if let Some((_, h)) = loaded.take() {
            unsafe {
                let _ = h.Unload();
            }
        }
    };
    while let Ok(first) = rx.recv() {
        let mut cmds: Vec<HandlerCmd> = std::iter::once(first).chain(rx.try_iter()).collect();
        let start = cmds.iter().rposition(|c| !matches!(c, HandlerCmd::Resize(..))).unwrap_or(0);
        for cmd in cmds.drain(start..) {
            match cmd {
                HandlerCmd::Show(w) => {
                    unload(&mut loaded);
                    let t = std::time::Instant::now();
                    crate::diag::log(|| format!("preview handler: opening seq {}", w.seq));
                    let msg = match unsafe { open_handler(&w) } {
                        Ok(h) => {
                            crate::diag::log(|| format!("preview handler: opened in {:?}", t.elapsed()));
                            loaded = Some((w.seq, h));
                            PreviewMsg::HandlerReady { seq: w.seq }
                        }
                        Err(e) => {
                            crate::diag::log(|| format!("preview handler: failed {e:?}"));
                            PreviewMsg::HandlerFailed { seq: w.seq }
                        }
                    };
                    shell::post_box(w.owner.get(), WM_APP_PREVIEW, Box::new(msg));
                }
                HandlerCmd::Resize(seq, rect) => {
                    if let Some((_, h)) = loaded.as_ref().filter(|(s, _)| *s == seq) {
                        unsafe {
                            let _ = h.SetRect(&rect);
                        }
                    }
                }
                HandlerCmd::Unload => unload(&mut loaded),
                HandlerCmd::Quit(ack) => {
                    unload(&mut loaded);
                    let _ = ack.send(());
                    return;
                }
            }
        }
    }
    unload(&mut loaded);
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
        h.SetWindow(w.host.get(), &w.rect)?;
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
