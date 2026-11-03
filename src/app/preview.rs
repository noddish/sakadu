//! 오른쪽 미리 보기 창 (Alt+P). 활성 패널에서 선택한 항목을 보여 준다.
//! 사진은 셸 썸네일, 텍스트는 앞부분, 그 밖은 Windows 미리 보기 처리기(PDF·Office 등),
//! 처리기가 없으면 큰 아이콘과 정보.
//! 미리 보기 처리기는 다른 프로세스에서 돌고 호출 중에 메시지를 돌리므로 App 빌림 밖(execute)에서만 다룬다.

use super::*;
use crate::thumbs::Request;
use crate::WM_APP_PREVIEW;
use windows::Win32::Graphics::DirectWrite::IDWriteTextLayout;
use windows::Win32::System::Com::{CLSCTX_LOCAL_SERVER, CLSIDFromString, CoCreateInstance, STGM_READ, STGM_SHARE_DENY_NONE};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};
use windows::Win32::UI::Shell::{
    ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, AssocQueryStringW, IInitializeWithItem, IPreviewHandler, IPreviewHandlerVisuals,
    IShellItem, SHCreateItemFromParsingName, SHCreateStreamOnFileEx,
};
use windows::core::{GUID, Interface, PWSTR};

pub(super) const PREVIEW_W: f32 = 340.0;
pub(super) const TIMER_PREVIEW: usize = 30;
pub(super) const G_PREVIEW: u16 = 0xE90D;
const HEAD_H: f32 = 56.0;
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
}

pub(super) struct Preview {
    target: Option<Target>,
    content: Content,
    /// 늦게 도착한 이전 결과를 버리기 위한 순번
    seq: u64,
    scroll: f32,
    layout: Option<(f32, IDWriteTextLayout)>,
    /// 처리기 창(호스트)을 둔 자리(클라이언트 픽셀). 숨겼으면 None
    host_rect: Option<RECT>,
}

impl Preview {
    pub(super) fn new() -> Preview {
        Preview { target: None, content: Content::Waiting, seq: 0, scroll: 0.0, layout: None, host_rect: None }
    }
}

pub struct TextReady {
    seq: u64,
    text: Option<String>,
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
        post(owner.get(), WM_APP_PREVIEW, Box::new(TextReady { seq, text }));
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
        Some(R::new(self.w - MARGIN - self.preview_w(), TOOLBAR_H, self.w - MARGIN, (self.h - MARGIN).max(TOOLBAR_H + 100.0)))
    }

    fn preview_body(&self) -> Option<R> {
        self.preview_rect().map(|pr| R::new(pr.l + 8.0, pr.t + HEAD_H + 6.0, pr.r - 8.0, (pr.b - 8.0).max(pr.t + HEAD_H + 40.0)))
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
        let App { panes, types, preview, .. } = self;
        let Some(pv) = preview.as_mut() else { return };
        let tab = panes[p].tab();
        let target = tab.entry_at(tab.cursor).filter(|e| e.selected).and_then(|e| {
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
        });
        let same = match (&pv.target, &target) {
            (Some(a), Some(b)) => a.path == b.path && a.modified == b.modified && a.size == b.size,
            (None, None) => true,
            _ => false,
        };
        if !same {
            pv.seq += 1;
            pv.target = target;
            pv.content = Content::Waiting;
            pv.scroll = 0.0;
            pv.layout = None;
            unsafe {
                if pv.target.is_some() {
                    SetTimer(Some(hwnd), TIMER_PREVIEW, 120, None);
                } else {
                    let _ = KillTimer(Some(hwnd), TIMER_PREVIEW);
                }
            }
        }
        // 처리기 창: 처리기를 쓸 때만, 팝업이 열려 있으면 가리지 않게 숨긴다
        let want = match (&self.preview, self.fav_open || self.pal_open || self.settings.is_some()) {
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
            Content::Image
        } else if TEXT_EXTS.contains(&ext.as_str()) {
            spawn_text(hwnd, pv.seq, t.path.clone());
            Content::Waiting
        } else if let Some(clsid) = handler_for(&ext) {
            Content::Handler(clsid)
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
            _ => "text",
        }));
        self.invalidate();
    }

    pub(super) fn on_preview_text(&mut self, r: TextReady) {
        let Some(pv) = self.preview.as_mut() else { return };
        if pv.seq != r.seq {
            return;
        }
        pv.content = match r.text {
            Some(t) => Content::Text(t),
            None => Content::Info,
        };
        pv.layout = None;
        self.invalidate();
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
        let App { gfx, preview, thumb_failed, .. } = self;
        let Some(pv) = preview.as_mut() else { return };
        gfx.fill_round(pr, RADIUS, t.pane);
        gfx.stroke_round(pr.inset(0.5, 0.5), RADIUS, t.border, 1.0);
        let Some(target) = &pv.target else {
            gfx.text("파일을 선택하면\n여기에서 미리 봅니다", R::new(body.l + 16.0, body.t + 40.0, body.r - 16.0, body.t + 120.0), Font::Hint, t.faint);
            return;
        };
        gfx.icon(target.icon, R::new(pr.l + 14.0, pr.t + 12.0, pr.l + 30.0, pr.t + 28.0));
        gfx.text(&target.name, R::new(pr.l + 38.0, pr.t + 8.0, pr.r - 14.0, pr.t + 32.0), Font::Strong, t.text);
        let sub = match target.kind {
            Kind::File => format!("{} · {}", target.type_name, format_size(target.size)),
            _ => target.type_name.clone(),
        };
        gfx.text(&sub, R::new(pr.l + 38.0, pr.t + 28.0, pr.r - 14.0, pr.t + 50.0), Font::Normal, t.dim);
        gfx.fill(R::new(pr.l + 10.0, pr.t + HEAD_H - 1.0, pr.r - 10.0, pr.t + HEAD_H), t.border);

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
                if !image(gfx, body.inset(4.0, 4.0), px) {
                    gfx.text("불러오는 중…", body, Font::Hint, t.faint);
                }
            }
            Content::Text(s) => {
                gfx.fill_round(body, 6.0, t.field);
                let w = body.w() - 20.0;
                if pv.layout.as_ref().is_none_or(|(lw, _)| (*lw - w).abs() > 0.5) {
                    pv.layout = gfx.layout(s, Font::Mono, w).map(|l| (w, l));
                }
                if let Some((_, l)) = &pv.layout {
                    gfx.clip(body.inset(1.0, 1.0));
                    gfx.draw_layout(l, body.l + 10.0, body.t + 8.0 - pv.scroll, t.text);
                    gfx.unclip();
                }
            }
            Content::Info => {
                let cx = (body.l + body.r) / 2.0;
                let ir = R::new(cx - INFO_IMG / 2.0, body.t + 16.0, cx + INFO_IMG / 2.0, body.t + 16.0 + INFO_IMG);
                if !image(gfx, ir, (INFO_IMG * scale).round() as u32) {
                    gfx.icon(target.icon, R::new(cx - 16.0, ir.t + INFO_IMG / 2.0 - 16.0, cx + 16.0, ir.t + INFO_IMG / 2.0 + 16.0));
                }
                let mut y = ir.b + 20.0;
                let mut row = |label: &str, value: &str| {
                    if value.is_empty() {
                        return;
                    }
                    gfx.text(label, R::new(body.l + 8.0, y, body.r - 8.0, y + 20.0), Font::Normal, t.faint);
                    gfx.text(value, R::new(body.l + 8.0, y + 20.0, body.r - 8.0, y + 40.0), Font::Path, t.text);
                    y += 48.0;
                };
                if target.kind != Kind::Drive {
                    row("수정한 날짜", &format_time(target.modified));
                }
                row("위치", &util::parent(&target.path).unwrap_or_default());
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
