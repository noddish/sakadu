//! 오른쪽 미리 보기 창 (⌥⌘P. Windows의 Alt+P 창과 같은 일): 칸 영역 오른쪽에 붙어, 지금 칸에서 고른 항목을 보인다.
//! 그림은 ImageIO, 글은 앞부분(스크롤된다), 한글 문서는 안에 든 미리 보기 그림·글(sakadu::cfb).
//! PDF·Office·iWork·동영상 같은 그 밖의 파일은 창 쪽이 본문 자리에 띄우는 Quick Look 보기(QLPreviewView)가 그린다
//! (그 보기를 만들 수 없으면 큰 Quick Look 썸네일). 폴더와 미리 보지 않는 파일은 큰 아이콘과 종류·크기·수정한 때.
//! 압축 파일은 맨 위 항목의 목록을 글로 보이고(Windows와 같다), 압축 파일 안의 항목은 정보만 (디스크에 없는 파일이다).
//! 머리의 핀으로 고정하면 다른 항목을 골라도 그 파일을 계속 보인다. 고정한 파일은 (디스크, 파일 번호)로 기억해, 같은 경로에 없으면
//! 원래 폴더에서 같은 번호를 찾고(이름 바꾸기), 없으면 파일 번호로 지금 자리를 찾는다(다른 폴더로 옮기기).
//! 지워졌거나 휴지통으로 갔으면 고정을 푼다 (확인은 뒷줄에서. Windows와 같은 규칙).
//! 열려 있는 동안 자동 미리 보기 카드는 뜨지 않는다. 열림과 고정한 파일은 세션에 남는다 (작업 공간과 상관없다)

use super::*;

/// 창의 폭 (창이 좁으면 창 폭의 40%까지. Windows와 같다)
const PANE_W: f32 = 340.0;
/// 머리: 아이콘, 이름과 그 아래 종류·크기, 오른쪽에 고정·열기 단추 (Windows preview.rs와 같은 치수)
const HEAD_H: f32 = 56.0;
const BTN: f32 = 28.0;
/// 기본 앱으로 열기 단추 (Windows와 같은 글자 번호. 맥은 SF Symbols로 바꿔 그린다)
const G_OPEN: u16 = 0xE8A7;
/// 정보(폴더, 미리 보지 않는 파일)의 아이콘 크기
const INFO_IMG: f32 = 128.0;
/// 창에 보이는 글의 줄 수 (카드보다 많이. 읽는 양은 카드와 같은 64KB)
pub(super) const PANE_LINES: usize = 600;
/// 확장자 없는 파일은 이 크기까지만 글인지 읽어 본다 (Windows와 같다)
const SNIFF_MAX: u64 = 2 * 1024 * 1024;

/// 창이 보이는 항목
#[derive(Clone, PartialEq, Debug)]
struct Target {
    path: String,
    name: String,
    /// 폴더 (앱 꾸러미 같은 패키지도)
    dir: bool,
    size: u64,
    modified: u64,
    /// 압축 파일 안의 항목 (디스크에 없다: 정보만 보이고 고정하지 않는다)
    inside: bool,
}

/// 본문에 보이는 것
enum Body {
    /// 읽는 중 (0.3초가 지나면 '불러오는 중…')
    Waiting,
    /// 그림. true면 문서의 썸네일(한글 문서의 미리 보기 그림 포함)이라 가는 테두리를 두른다
    Picture(Picture, bool),
    Text(String),
    /// 창 쪽의 Quick Look 보기가 본문을 덮는다
    Look,
    /// 그림을 읽지 못했다
    Broken,
    /// 큰 아이콘과 종류·크기·수정한 때
    Info,
}

/// 고정해 둔 파일 (Windows preview.rs의 Pin과 같다)
struct Pin {
    /// (디스크, 파일 번호). 고정한 뒤 뒷줄에서 읽어 채운다
    id: Option<(u32, u64)>,
    /// 늦게 온 이전 확인 결과를 버리는 순번 (고정할 때마다 바뀐다)
    round: u64,
    /// 뒷줄에서 확인하는 중이면 true. 그사이 다시 확인할 일이 생기면 again
    busy: bool,
    again: bool,
}

/// 열린 미리 보기 창의 상태
pub(super) struct PreviewPane {
    target: Option<Target>,
    pin: Option<Pin>,
    body: Body,
    /// 지금 본문을 읽으라고 낸 요청의 번호 (다른 요청의 결과는 버린다. 읽을 것이 없으면 0)
    asked: u64,
    scroll: f32,
    /// 지금 대상을 보이기 시작한 때 ('불러오는 중…'은 0.3초가 지나야 보인다)
    since: Instant,
    /// 세션에서 되살린 고정 파일을 아직 확인하지 않았다: 종류·크기를 몰라 읽지 않고 기다린다
    unchecked: bool,
}

impl PreviewPane {
    fn new() -> PreviewPane {
        PreviewPane { target: None, pin: None, body: Body::Waiting, asked: 0, scroll: 0.0, since: Instant::now(), unchecked: false }
    }
}

/// 고정한 파일 확인 요청: 창 쪽이 뒷줄에서 check_pin을 돌려 on_pin_checked로 돌려준다
pub struct PinCheck {
    round: u64,
    path: String,
    id: Option<(u32, u64)>,
}

/// 확인한 고정 파일의 지금 모습
pub struct PinFound {
    path: String,
    id: Option<(u32, u64)>,
    size: u64,
    modified: u64,
    dir: bool,
}

/// 고정한 파일이 지금 어디 있는지 확인한다 (뒷줄에서 부른다). 찾지 못했으면(지워짐, 휴지통) None.
/// 같은 경로에 있으면 그 경로를 지킨다: 저장할 때 새 파일로 바꿔 쓰는 앱은 파일 번호가 바뀌지만 사용자에게는 같은 파일이다.
/// 경로가 비었을 때만 번호로 옮겨 간 곳을 찾는다 (Windows와 같다)
pub fn check_pin(c: &PinCheck) -> Option<PinFound> {
    use std::os::unix::fs::MetadataExt;
    let path = if std::fs::metadata(&c.path).is_ok() { c.path.clone() } else { moved_to(&c.path, c.id?)? };
    let m = std::fs::metadata(&path).ok()?;
    Some(PinFound { id: Some((m.dev() as u32, m.ino())), size: m.len(), modified: m.modified().map(listing::filetime_of).unwrap_or(0), dir: m.is_dir(), path })
}

/// 경로에 없는 고정 파일을 파일 번호로 찾는다: 먼저 원래 폴더에서(같은 폴더 안의 이름 바꾸기), 없으면 맥이 파일 번호로 여는 길로.
/// 휴지통에 있으면 지운 것으로 본다
fn moved_to(old: &str, (dev, ino): (u32, u64)) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let same = |p: &std::path::Path| std::fs::metadata(p).is_ok_and(|m| m.dev() as u32 == dev && m.ino() == ino);
    if let Some(parent) = util::parent(old)
        && let Ok(items) = std::fs::read_dir(&parent)
        && let Some(hit) = items.flatten().find(|i| same(&i.path()))
    {
        return Some(util::join(&parent, &hit.file_name().to_string_lossy()));
    }
    path_of_id(dev, ino).filter(|p| !in_trash(p))
}

/// 파일 번호로 지금 경로를 찾는다: /.vol/디스크/번호로 열어 그 경로를 묻는다 (맥의 APFS·HFS+가 이 길을 준다)
fn path_of_id(dev: u32, ino: u64) -> Option<String> {
    let vol = std::ffi::CString::new(format!("/.vol/{}/{ino}", dev as i32)).ok()?;
    // SAFETY: NUL로 끝나는 경로를 읽기 전용으로 연다 (장치 파일이어도 기다리지 않게)
    let fd = unsafe { libc::open(vol.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    let mut buf = [0 as libc::c_char; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH는 PATH_MAX 바이트 버퍼에 NUL로 끝나는 경로를 쓴다. 연 것은 바로 닫는다
    let got = unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) };
    unsafe { libc::close(fd) };
    if got < 0 {
        return None;
    }
    // SAFETY: 성공했으면 buf는 NUL로 끝난다
    let path = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
    Some(util::normalize(&path))
}

/// 휴지통 안의 경로인지 (홈의 .Trash, 다른 디스크의 .Trashes)
fn in_trash(path: &str) -> bool {
    path.contains("/.Trash/") || path.contains("/.Trashes/")
}

/// 대상을 무엇으로 보일지
#[derive(Clone, Copy, PartialEq, Debug)]
enum Plan {
    Info,
    /// 뒷줄에서 읽는다 (그림, 글, 문서: 한글 문서의 미리 보기나 Quick Look 썸네일)
    Read(CardKind),
    Look,
}

/// 확장자와 크기로 정한다. look이 false면(Quick Look 보기를 만들 수 없다) 문서는 큰 썸네일로.
/// 압축 파일은 안의 목록(글), 압축 파일 안의 항목은 정보만
fn plan(t: &Target, look: bool) -> Plan {
    if t.dir || t.inside {
        return Plan::Info;
    }
    if archive_path::is_archive_name(&t.name) {
        return Plan::Read(CardKind::Archive);
    }
    let Some(ext) = ext_of(&t.name) else {
        // 확장자 없는 작은 파일은 글인지 읽어 본다 (README, Makefile 따위)
        return if t.size <= SNIFF_MAX { Plan::Read(CardKind::Text) } else { Plan::Info };
    };
    match card_kind(&t.name) {
        Some(CardKind::Doc) if look && ext != "hwp" => Plan::Look,
        Some(kind) => Plan::Read(kind),
        None => Plan::Info,
    }
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

/// 그림을 읽을 픽셀 수: 본문의 긴 쪽 × 화면 배율을 256 단위로 올린 값 (Windows image_px와 같은 단계, 맥 카드의 큰 그림까지)
fn image_px(longest: f32, scale: f32) -> u32 {
    (((longest * scale) / 256.0).ceil() as u32 * 256).clamp(256, PX_LARGE)
}

/// 글을 끝까지 내렸을 때의 스크롤 (위아래 여백 10씩)
fn text_max_scroll(s: &str, body: R) -> f32 {
    (s.lines().count() as f32 * MONO_LINE_H + 20.0 - body.h()).max(0.0)
}

impl App {
    fn preview_w(&self) -> f32 {
        PANE_W.min(self.w * 0.4)
    }

    /// 창의 자리 (닫혀 있으면 None): 칸 영역의 오른쪽 끝에 붙는다
    pub(super) fn preview_rect(&self) -> Option<R> {
        self.preview.as_ref()?;
        let c = self.content_area();
        Some(R::new((c.r - self.preview_w()).max(c.l), c.t, c.r, c.b))
    }

    /// 머리 아래 본문
    pub(super) fn preview_body(&self) -> Option<R> {
        self.preview_rect().map(|pr| {
            let t = pr.t + HEAD_H;
            R::new(pr.l + 16.0, t, pr.r - 16.0, (pr.b - 16.0).max(t + 40.0))
        })
    }

    /// ⌥⌘P: 창을 열고 닫는다. 열면 지금 고른 항목을 보인다 (다음 맞추기에서). 닫으면 고정도 풀린다
    pub fn toggle_preview(&mut self) {
        if self.preview.take().is_none() {
            self.preview = Some(PreviewPane::new());
        }
        // 칸들의 폭이 바뀌었다
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
    }

    /// 지금 칸의 커서 항목 (골라져 있을 때)
    fn preview_target_now(&self) -> Option<Target> {
        let tab = self.panes[self.active].tab();
        let e = tab.cursor_entry().filter(|e| e.selected)?;
        Some(Target { path: tab.path_of(e)?, name: e.name_string(), dir: e.kind != Kind::File, size: e.size, modified: e.modified, inside: tab.in_archive })
    }

    /// 창을 지금 고른 항목에 맞춘다 (고정했으면 그대로): 바뀌었으면 처음부터 보인다. take_previews가 이것을 거친다
    pub(super) fn sync_preview(&mut self) {
        let Some(pv) = self.preview.as_ref() else { return };
        if pv.pin.is_some() {
            return;
        }
        let now = self.preview_target_now();
        if now == pv.target {
            return;
        }
        if let Some(pv) = self.preview.as_mut() {
            pv.target = now;
        }
        self.start_preview_body();
    }

    /// 지금 대상을 처음부터 보인다: 무엇으로 보일지 정하고, 읽을 것이 있으면 요청을 쌓는다
    fn start_preview_body(&mut self) {
        let look = self.look_view_ok;
        let px = self.preview_body().map_or(PX_SMALL, |b| image_px(b.w().max(b.h()), self.scale));
        let seq = self.next_seq();
        let Some(pv) = self.preview.as_mut() else { return };
        pv.scroll = 0.0;
        pv.since = Instant::now();
        pv.body = Body::Waiting;
        pv.asked = 0;
        let Some(t) = pv.target.as_ref().filter(|_| !pv.unchecked) else { return };
        match plan(t, look) {
            Plan::Info => pv.body = Body::Info,
            Plan::Look => pv.body = Body::Look,
            Plan::Read(kind) => {
                pv.asked = seq;
                let key = CardKey { path: t.path.clone(), modified: t.modified, size: t.size };
                let px = if matches!(kind, CardKind::Text | CardKind::Archive) { 0 } else { px };
                self.previews.push(PreviewRequest { seq, key, px, kind, pane: true });
            }
        }
    }

    /// 뒷줄에서 읽은 창의 본문. 지금 대상의 요청이 아니면 버린다
    pub(super) fn on_preview_loaded(&mut self, req: &PreviewRequest, result: Option<Preview>) {
        let Some(pv) = self.preview.as_mut().filter(|pv| pv.asked != 0 && pv.asked == req.seq) else { return };
        pv.body = match result {
            Some(Preview::Image(pic)) => Body::Picture(pic, req.kind == CardKind::Doc),
            Some(Preview::Text(s)) => Body::Text(s),
            None if req.kind == CardKind::Image => Body::Broken,
            // 글로 읽히지 않는 파일, 그림·글이 없는 한글 문서, 썸네일이 없는 문서
            None => Body::Info,
        };
    }

    // ---------- 고정 ----------

    /// 핀: 지금 보이는 항목을 고정하거나, 고정을 풀고 다시 고른 것을 따라간다
    pub fn toggle_preview_pin(&mut self) {
        let round = self.next_seq();
        let Some(pv) = self.preview.as_mut() else { return };
        if pv.pin.take().is_some() || pv.target.is_none() {
            return;
        }
        // 압축 파일 안의 항목은 디스크에서 다시 찾을 수 없어 고정하지 않는다
        if pv.target.as_ref().is_some_and(|t| t.inside) {
            self.flash = Some((self.active, "압축 파일 안의 항목은 고정할 수 없어요".to_string()));
            return;
        }
        pv.pin = Some(Pin { id: None, round, busy: false, again: false });
        // 파일 번호를 읽어 둔다 (이름이 바뀌거나 옮겨져도 찾을 수 있게)
        self.pin_check();
    }

    /// 고정해 둔 파일의 경로와 파일 번호 (세션에 남긴다)
    pub(super) fn preview_pin(&self) -> Option<(String, Option<(u32, u64)>)> {
        let pv = self.preview.as_ref()?;
        let pin = pv.pin.as_ref()?;
        pv.target.as_ref().map(|t| (t.path.clone(), pin.id))
    }

    /// 세션에 남긴 창 열림과 고정한 파일을 되살린다. 고정한 파일은 종류·크기를 몰라 뒷줄에서 확인한 뒤에 읽는다
    /// (없어졌으면 고정을 푼다)
    pub(super) fn restore_preview(&mut self, open: bool, pin: Option<&str>, id: Option<(u32, u64)>) {
        self.preview = open.then(PreviewPane::new);
        let Some(path) = pin.filter(|_| open) else { return };
        let round = self.next_seq();
        if let Some(pv) = self.preview.as_mut() {
            pv.target = Some(Target { path: path.to_string(), name: util::file_name(path).to_string(), dir: false, size: 0, modified: 0, inside: false });
            pv.pin = Some(Pin { id, round, busy: false, again: false });
            pv.unchecked = true;
        }
        self.pin_check();
    }

    /// 고정한 파일이 그대로 있는지 뒷줄에서 확인해 달라고 한다 (파일 작업 뒤, 폴더가 바뀐 알림 뒤, 창이 다시 앞으로 올 때).
    /// 확인하는 중이면 끝난 뒤 한 번 더 한다
    pub(super) fn pin_check(&mut self) {
        let Some(pv) = self.preview.as_mut() else { return };
        let (Some(pin), Some(t)) = (pv.pin.as_mut(), pv.target.as_ref()) else { return };
        if pin.busy {
            pin.again = true;
            return;
        }
        pin.busy = true;
        self.pin_checks.push(PinCheck { round: pin.round, path: t.path.clone(), id: pin.id });
    }

    /// 쌓인 고정 확인 요청 (창 쪽이 뒷줄에서 check_pin을 돌린다)
    pub fn take_pin_checks(&mut self) -> Vec<PinCheck> {
        std::mem::take(&mut self.pin_checks)
    }

    /// 뒷줄에서 확인한 고정 파일: 없어졌으면 고정을 풀고(다시 고른 것을 따라간다), 옮겨졌거나 바뀌었으면 새로 읽는다
    pub fn on_pin_checked(&mut self, c: &PinCheck, found: Option<PinFound>) {
        let Some(pv) = self.preview.as_mut() else { return };
        let Some(pin) = pv.pin.as_mut().filter(|p| p.round == c.round) else { return };
        pin.busy = false;
        let again = std::mem::take(&mut pin.again);
        let Some(f) = found else {
            pv.pin = None;
            pv.unchecked = false;
            self.flash = Some((self.active, "고정한 파일을 찾을 수 없어 고정을 풀었어요".to_string()));
            return;
        };
        pin.id = f.id.or(pin.id);
        let Some(t) = pv.target.as_mut() else { return };
        let changed = pv.unchecked || t.path != f.path || t.dir != f.dir || (!f.dir && (t.size != f.size || t.modified != f.modified));
        if changed {
            t.name = util::file_name(&f.path).to_string();
            t.path = f.path;
            t.dir = f.dir;
            t.size = if f.dir { 0 } else { f.size };
            t.modified = f.modified;
            pv.unchecked = false;
            self.start_preview_body();
        }
        if again {
            self.pin_check();
        }
    }

    // ---------- 누르기·스크롤 ----------

    /// 창 위를 가리키면: 머리의 단추, 그 밖은 창이 받는다 (아래 칸을 누르지 않게)
    pub(super) fn preview_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let pr = self.preview_rect().filter(|r| r.contains(x, y))?;
        let (pin, open) = head_buttons(pr);
        Some(if pin.contains(x, y) {
            Hit::PreviewPin
        } else if open.contains(x, y) {
            Hit::PreviewOpen
        } else {
            Hit::PreviewArea
        })
    }

    /// 머리의 열기 단추: 파일은 기본 앱으로 (압축 파일은 지금 칸에서 그 안으로, 압축 파일 안의 파일은 꺼내서), 폴더는 지금 칸에서 연다
    pub(super) fn open_preview_target(&mut self) {
        let Some(t) = self.preview.as_ref().and_then(|pv| pv.target.clone()) else { return };
        if t.dir {
            self.navigate(self.active, Location::Dir(t.path), None);
        } else {
            self.open_paths(self.active, vec![t.path], false);
        }
    }

    /// 창 위의 스크롤: 글이면 글을 움직인다. 창 위였으면 true (뒤의 칸은 움직이지 않는다)
    pub(super) fn preview_wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        if !self.preview_rect().is_some_and(|r| r.contains(x, y)) {
            return false;
        }
        let Some(body) = self.preview_body() else { return true };
        if let Some(pv) = self.preview.as_mut()
            && let Body::Text(s) = &pv.body
        {
            pv.scroll = (pv.scroll + dy).clamp(0.0, text_max_scroll(s, body));
        }
        true
    }

    // ---------- Quick Look 보기 (창 쪽이 띄운다) ----------

    /// 창 쪽이 본문 자리에 띄울 Quick Look 보기: (자리, 파일, 수정한 때). 앱이 그리는 메뉴나 명령 메뉴가 떠 있으면
    /// 그것을 가리지 않게 None (보기는 앱이 그린 것 위에 놓인다)
    pub fn look_view(&self) -> Option<(R, String, u64)> {
        let pv = self.preview.as_ref()?;
        if self.menu.is_some() || self.cmd.is_some() {
            return None;
        }
        let (Body::Look, Some(t)) = (&pv.body, &pv.target) else { return None };
        Some((self.preview_body()?, t.path.clone(), t.modified))
    }

    /// 창 쪽이 Quick Look 보기를 만들지 못했다: 문서는 큰 Quick Look 썸네일로 보인다
    pub fn look_view_missing(&mut self) {
        self.look_view_ok = false;
        if self.preview.as_ref().is_some_and(|pv| matches!(pv.body, Body::Look)) {
            self.start_preview_body();
        }
    }

    /// 창 시험: 창이 열려 있으면 (보이는 항목의 이름, 보이는 글)
    pub fn preview_seen(&self) -> Option<(Option<String>, Option<String>)> {
        let pv = self.preview.as_ref()?;
        let text = match &pv.body {
            Body::Text(s) => Some(s.clone()),
            _ => None,
        };
        Some((pv.target.as_ref().map(|t| t.name.clone()), text))
    }

    /// 시험: 본문에 보이는 것의 종류와 (그림이면) 크기
    #[cfg(test)]
    pub(super) fn preview_body_seen(&self) -> Option<(&'static str, Option<(usize, usize)>)> {
        Some(match &self.preview.as_ref()?.body {
            Body::Waiting => ("waiting", None),
            Body::Picture(pic, _) => ("picture", Some(pic.size())),
            Body::Text(_) => ("text", None),
            Body::Look => ("look", None),
            Body::Broken => ("broken", None),
            Body::Info => ("info", None),
        })
    }

    /// 시험: 글의 스크롤
    #[cfg(test)]
    pub(super) fn preview_scroll(&self) -> f32 {
        self.preview.as_ref().map_or(0.0, |pv| pv.scroll)
    }

    /// 시험: 머리의 (고정, 열기) 단추 자리
    #[cfg(test)]
    pub(super) fn preview_buttons(&self) -> Option<(R, R)> {
        self.preview_rect().map(head_buttons)
    }

    // ---------- 그리기 ----------

    /// 창: 머리(아이콘, 이름, 종류·크기, 핀·열기 단추)와 본문
    pub(super) fn paint_preview(&self, g: &mut Gfx, t: &Theme) {
        let (Some(pv), Some(pr), Some(body)) = (self.preview.as_ref(), self.preview_rect(), self.preview_body()) else { return };
        if !t.high_contrast && !t.dark {
            soft_shadow(g, pr, RADIUS, 6.0, 0.06, false);
        }
        g.fill_round(pr, RADIUS, t.pane);
        if t.high_contrast || t.dark {
            g.stroke_round(pr.inset(0.5, 0.5), RADIUS, t.pane_border, 1.0);
        }
        // 단추: 고정했으면 핀이 옅은 강조 색으로 켜진다. 보일 것이 없으면 흐리다
        let has = pv.target.is_some();
        let pinned = pv.pin.is_some() && has;
        let (pin_r, open_r) = head_buttons(pr);
        for (r, hit, glyph) in [(pin_r, Hit::PreviewPin, if pinned { G_PIN_FILL } else { G_PIN }), (open_r, Hit::PreviewOpen, G_OPEN)] {
            let lit = pinned && hit == Hit::PreviewPin;
            let hot = has && self.hover == hit;
            if lit && !t.high_contrast {
                g.fill_round(r, R_CTRL, t.accent_soft);
            } else if hot {
                g.fill_round(r, R_CTRL, hover_fill(t));
            }
            let c = if lit { t.chip_text } else if !has { t.faint } else if hot { t.text } else { t.dim };
            g.glyph(glyph, r, false, c);
        }
        let Some(target) = &pv.target else {
            Canvas::text(&*g, "파일을 고르면\n여기에서 미리 봐요", R::new(pr.l + 16.0, pr.t + HEAD_H + 8.0, pr.r - 16.0, pr.t + HEAD_H + 88.0), Font::Hint, t.faint);
            return;
        };
        // 아이콘, 이름, 그 아래 종류 · 크기 (좁으면 크기만). 되살린 고정 파일은 확인하기 전까지 이름만
        let (name_r, sub_r) = head_text(pr);
        let icon = |g: &Gfx, r: R| if target.inside { g.icon_for_kind(&target.name, target.dir, r) } else { g.icon_for_path(&target.path, r) };
        icon(g, R::new(pr.l + 16.0, pr.t + 20.0, pr.l + 32.0, pr.t + 36.0));
        g.text(&target.name, name_r, Font::Strong, t.text);
        let detail = self.preview_detail(target, pv.unchecked);
        let size = (!target.dir && !pv.unchecked).then(|| listing::format_size(target.size));
        let sub = match size {
            Some(s) if g.measure(&detail, Font::Small) > sub_r.w() => s,
            _ => detail,
        };
        g.text(&sub, sub_r, Font::Small, t.dim);

        match &pv.body {
            Body::Waiting => {
                if pv.since.elapsed().as_millis() >= LOADING_NOTE_MS as u128 {
                    g.text("불러오는 중…", body, Font::Hint, t.faint);
                }
            }
            Body::Picture(pic, framed) => {
                g.picture(pic, body);
                if *framed {
                    g.stroke_round(Gfx::fit(pic, body).inset(-0.5, -0.5), 2.0, alpha(t.text, 0.14), 1.0);
                }
            }
            Body::Text(s) => {
                g.fill_round(body, R_CTRL, t.field);
                g.clip(body.inset(1.0, 1.0));
                // 보이는 줄만 그린다 (위아래 여백 10)
                let scroll = pv.scroll.min(text_max_scroll(s, body));
                let top = body.t + 10.0 - scroll;
                let first = ((scroll - 10.0) / MONO_LINE_H).floor().max(0.0) as usize;
                for (i, line) in s.lines().enumerate().skip(first) {
                    let y = top + i as f32 * MONO_LINE_H;
                    if y > body.b {
                        break;
                    }
                    g.text(line, R::new(body.l + 12.0, y, body.r - 8.0, y + MONO_LINE_H), Font::Mono, t.text);
                }
                g.unclip();
            }
            // 창 쪽의 Quick Look 보기가 덮는다
            Body::Look => {}
            Body::Broken => g.text("미리 볼 수 없어요", body, Font::Hint, t.faint),
            // 큰 아이콘, 그 아래 종류·크기와 수정한 때
            Body::Info => {
                let (cx, cy) = ((body.l + body.r) / 2.0, (body.t + body.b) / 2.0 - 32.0);
                icon(g, R::new(cx - INFO_IMG / 2.0, cy - INFO_IMG / 2.0, cx + INFO_IMG / 2.0, cy + INFO_IMG / 2.0));
                let modified = listing::format_time(target.modified);
                let lines = [self.preview_detail(target, false), if modified.is_empty() { String::new() } else { format!("수정한 날짜 {modified}") }];
                for (i, line) in lines.iter().enumerate() {
                    let y = cy + INFO_IMG / 2.0 + 16.0 + i as f32 * 20.0;
                    g.text(line, R::new(body.l, y, body.r, y + 20.0), Font::Hint, t.dim);
                }
            }
        }
    }

    /// 머리의 이름 아래 글: 종류 · 크기 (폴더는 종류만). 확인하기 전의 고정 파일은 비운다
    fn preview_detail(&self, target: &Target, unchecked: bool) -> String {
        if unchecked {
            return String::new();
        }
        let kind = finder_kind(&self.kinds, &target.name, target.dir, || (!target.inside).then(|| target.path.clone()));
        if target.dir { kind } else { card_detail(&kind, target.size) }
    }
}
