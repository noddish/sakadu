//! 사카두 메뉴 창 (Windows). 운영 체제 메뉴(TrackPopupMenuEx) 대신 테두리 없는 작은 창을 띄워 공통 메뉴 모양(sakadu::ui::menu)으로
//! 그린다. 부르는 쪽은 TrackPopupMenuEx(TPM_RETURNCMD)처럼 고른 번호를 돌려받는다(고르지 않았으면 0).
//! 메뉴가 떠 있는 동안 이 함수가 메시지 루프를 돌며 마우스(잡아 두기)와 키를 받는다. 그 밖의 메시지는 그대로 보내서
//! 본 창은 계속 그려진다. 하위 메뉴는 옆에 창을 하나 더 띄운다. 모서리와 그림자는 Windows가 그린다
//! (Windows 11의 둥근 모서리, 창 그림자). 메뉴 밖을 누르거나, Esc, 이 앱이 앞에 있다가 다른 앱이 앞으로 오면 닫힌다.
//! 떠 있는 메뉴 창들은 스레드 하나에 두어, 창 프로시저도 WM_PAINT를 받으면 그린다 (창 그림을 찍을 때처럼 Windows가 바로 그려 달라고 할 때)

use crate::gfx::{Gfx, TextLook};
use crate::theme::{Theme, colorref};
use sakadu::draw::R;
use sakadu::ui::menu::{MENU_PAD, MenuIcon, MenuLine, MenuScene, menu_height, menu_hit, menu_icon_rect, menu_max_scroll, menu_reveal, menu_rows, menu_step, menu_width, paint_menu};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{BeginPaint, ClientToScreen, EndPaint, GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromPoint, PAINTSTRUCT, ValidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Controls::HIMAGELIST;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, VIRTUAL_KEY, VK_DOWN, VK_END, VK_ESCAPE, VK_F10, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// 메뉴 줄 (부르는 쪽이 만든다)
pub enum Entry {
    /// 고르면 그 번호를 돌려준다
    Item(u32, MenuLine),
    /// 하위 메뉴
    Sub(MenuLine, Vec<Entry>),
    /// 구분선
    Sep,
}

impl Entry {
    pub fn item(id: u32, label: &str) -> Entry {
        Entry::Item(id, line(label))
    }

    pub fn check(id: u32, label: &str, on: bool) -> Entry {
        Entry::Item(id, line(label).checked(on))
    }

    pub fn sub(label: &str, items: Vec<Entry>) -> Entry {
        let mut l = line(label);
        l.sub = true;
        Entry::Sub(l, items)
    }
}

/// (번호, 글) 목록을 메뉴 줄로. 번호 0은 구분선 (지금까지의 Windows 메뉴 목록과 같은 꼴)
pub fn from_pairs(items: &[(u32, &str)]) -> Vec<Entry> {
    items.iter().map(|(id, l)| if *id == 0 { Entry::Sep } else { Entry::item(*id, l) }).collect()
}

/// Windows 메뉴 글을 메뉴 줄로: 탭 뒤는 오른쪽 끝의 흐린 글, 밑줄 글자 표시는 뺀다 (clean)
fn line(label: &str) -> MenuLine {
    match label.split_once('\t') {
        Some((name, hint)) => MenuLine::new(clean(name)).hint(clean(hint)),
        None => MenuLine::new(clean(label)),
    }
}

/// 메뉴 글의 밑줄 글자 표시를 뺀다: "열기(&O)"처럼 괄호에 든 것은 괄호째, 그 밖의 홑 &는 &만. &&는 &
fn clean(s: &str) -> String {
    let mut s = s.replace("&&", "\u{1}");
    while let Some(i) = s.find("(&") {
        let mut rest = s[i + 2..].chars();
        match (rest.next(), rest.next()) {
            (Some(c), Some(')')) => s.replace_range(i..i + 3 + c.len_utf8(), ""),
            _ => break,
        }
    }
    s.replace('&', "").replace('\u{1}', "&").trim_end().to_string()
}

/// 탐색기 메뉴 글: 탭 뒤(단축키)는 버리고 밑줄 글자 표시를 뺀다
pub fn plain(label: &str) -> String {
    clean(label.split_once('\t').map_or(label, |(name, _)| name))
}

/// 줄 앞 그림 (미리 곱한 BGRA 픽셀). 줄의 MenuIcon::Image(번호)가 이 목록의 번호다
#[derive(Clone)]
pub struct Icon {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}

/// 메뉴를 그릴 모양: 테마, 글꼴, 크기, 글자 표시 (본 창의 설정)
#[derive(Clone)]
pub struct Style {
    pub theme: Theme,
    pub font: String,
    pub zoom: f32,
    pub text: String,
}

impl Default for Style {
    fn default() -> Style {
        Style { theme: Theme::LIGHT, font: String::new(), zoom: 1.0, text: "gray".to_string() }
    }
}

/// 줄을 고르면: 돌려줄 번호, 하위 메뉴(맨 위 줄 목록에서의 자리)
enum Pick {
    Id(u32),
    Sub(Vec<usize>),
}

/// 떠 있는 메뉴 창 하나 (맨 위 메뉴와 열린 하위 메뉴들)
struct Level {
    hwnd: HWND,
    gfx: Gfx,
    lines: Vec<MenuLine>,
    picks: Vec<Pick>,
    /// 화면 좌표(픽셀)의 창 자리
    rect: RECT,
    /// 메뉴 크기 (DIP)
    w: f32,
    h: f32,
    sel: Option<usize>,
    scroll: f32,
    /// 이 하위 메뉴를 연 위 메뉴의 줄
    from: Option<usize>,
    /// 줄마다의 그림 (MenuIcon::Image)
    icons: Vec<Option<Icon>>,
}

impl Level {
    fn menu(&self) -> R {
        R::new(0.0, 0.0, self.w, self.h)
    }

    fn scale(&self) -> f32 {
        self.gfx.dpi / 96.0
    }

    fn contains(&self, p: POINT) -> bool {
        p.x >= self.rect.left && p.x < self.rect.right && p.y >= self.rect.top && p.y < self.rect.bottom
    }

    /// 화면 좌표의 그 자리에 있는 고를 수 있는 줄
    fn hit(&self, p: POINT) -> Option<usize> {
        let s = self.scale();
        let (x, y) = ((p.x - self.rect.left) as f32 / s, (p.y - self.rect.top) as f32 / s);
        menu_hit(self.menu(), &self.lines, self.scroll, x, y)
    }

    /// 줄 i의 화면 좌표 자리 (하위 메뉴를 그 옆에 연다)
    fn row_rect(&self, i: usize) -> RECT {
        let s = self.scale();
        let r = menu_rows(self.menu(), &self.lines, self.scroll)[i];
        RECT { left: self.rect.left, top: self.rect.top + (r.t * s) as i32, right: self.rect.right, bottom: self.rect.top + (r.b * s) as i32 }
    }

    fn draw(&mut self, theme: &Theme) {
        if !self.gfx.begin(self.hwnd) {
            return;
        }
        self.gfx.clear(theme.pane);
        paint_menu(&self.gfx, theme, &MenuScene { rect: self.menu(), lines: &self.lines, sel: self.sel, scroll: self.scroll, card: false });
        // 줄 앞 그림: 메뉴 창마다 그 창의 그리기 판에 한 번 올리고 그린다 (판을 새로 만들었으면 다시 올린다)
        let rows = menu_rows(self.menu(), &self.lines, self.scroll);
        for (j, icon) in self.icons.iter().enumerate() {
            let Some(icon) = icon else { continue };
            let r = menu_icon_rect(rows[j]);
            if r.t < 0.0 || r.b > self.h {
                continue;
            }
            let key = j.to_string();
            let opacity = if self.lines[j].disabled { 0.4 } else { 1.0 };
            if !self.gfx.thumb(&key, r, opacity) && self.gfx.add_thumb(key.clone(), icon.w, icon.h, &icon.pixels) {
                self.gfx.thumb(&key, r, opacity);
            }
        }
        self.gfx.end();
    }

    /// 바뀐 것을 바로 그린다 (WM_PAINT를 기다리지 않는다)
    fn paint(&mut self, theme: &Theme) {
        self.draw(theme);
        unsafe {
            let _ = ValidateRect(Some(self.hwnd), None);
        }
    }

    fn close(&self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// 떠 있는 메뉴 (한 번에 하나): 창들, 그리는 모양, 본 창, 줄 앞 그림들
struct Open {
    levels: Vec<Level>,
    style: Style,
    dpi: f32,
    owner: HWND,
    icons: Vec<Icon>,
}

thread_local! {
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
    /// 잡아 둔 마우스를 잃었다 (다른 창이 가져갔다): 메뉴를 닫는다
    static LOST_CAPTURE: Cell<bool> = const { Cell::new(false) };
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
}

const CLASS: windows::core::PCWSTR = w!("SakaduMenu");
/// 다른 앱이 앞으로 왔는지 보려고 깨는 간격
const TICK_MS: u32 = 100;

/// 메뉴 창을 놓을 자리
enum Place {
    /// pt를 왼쪽 위로 (above면 왼쪽 아래로: 트레이)
    At(POINT, bool),
    /// 위 메뉴의 줄(row) 옆, 위 메뉴(parent) 바깥에
    Beside(RECT, RECT),
}

/// 루프가 할 일
enum Step {
    Go,
    /// 끝 (고른 번호, 고르지 않았으면 0)
    Done(u32),
}

/// 메뉴를 띄우고 고른 번호를 돌려준다 (고르지 않았으면 0). pt는 화면 좌표의 메뉴 왼쪽 위.
/// above면 pt 위로 띄운다 (작업 표시줄의 알림 영역 메뉴). App을 빌린 채로 부르지 않는다
pub fn show(owner: HWND, pt: POINT, entries: &[Entry], above: bool) -> u32 {
    show_with_icons(owner, pt, entries, above, &[])
}

/// show와 같고, 줄 앞 그림(MenuIcon::Image의 번호)을 함께 준다
pub fn show_with_icons(owner: HWND, pt: POINT, entries: &[Entry], above: bool, icons: &[Icon]) -> u32 {
    if OPEN.with(|o| o.borrow().is_some()) || !register() {
        return 0;
    }
    let style = crate::with_app(|a| a.menu_style()).unwrap_or_default();
    let dpi = unsafe { GetDpiForWindow(owner) }.max(96) as f32 * style.zoom;
    let Some(root) = open_level(owner, entries, &[], &style, dpi, Place::At(pt, above), icons) else { return 0 };
    let root_hwnd = root.hwnd;
    OPEN.with(|o| *o.borrow_mut() = Some(Open { levels: vec![root], style, dpi, owner, icons: icons.to_vec() }));
    LOST_CAPTURE.set(false);
    unsafe {
        SetCapture(root_hwnd);
        let _ = SetTimer(Some(root_hwnd), 1, TICK_MS, None);
    }
    with_open(|o| {
        let theme = o.style.theme;
        for l in &mut o.levels {
            l.paint(&theme);
        }
    });
    let foreground = unsafe { GetForegroundWindow() };
    // 메뉴 위로 마우스가 움직였거나 메뉴 안에서 눌렀다: 그 뒤의 떼기만 고른다 (메뉴를 연 누름의 떼기로 고르지 않게)
    let mut armed = false;
    let mut picked = 0;
    loop {
        let mut msg = MSG::default();
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 == 0 {
            // 끝내라는 메시지는 바깥 루프가 받게 다시 넣는다
            unsafe { PostQuitMessage(msg.wParam.0 as i32) };
            break;
        }
        if got.0 == -1 {
            break;
        }
        let step = match msg.message {
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDBLCLK => {
                let mut p = POINT { x: (msg.lParam.0 & 0xFFFF) as i16 as i32, y: ((msg.lParam.0 >> 16) & 0xFFFF) as i16 as i32 };
                unsafe {
                    let _ = ClientToScreen(msg.hwnd, &mut p);
                }
                with_open(|o| mouse(o, entries, msg.message, p, &mut armed)).unwrap_or(Step::Done(0))
            }
            WM_MOUSEWHEEL => {
                let p = POINT { x: (msg.lParam.0 & 0xFFFF) as i16 as i32, y: ((msg.lParam.0 >> 16) & 0xFFFF) as i16 as i32 };
                let delta = ((msg.wParam.0 >> 16) & 0xFFFF) as i16 as f32;
                with_open(|o| {
                    let theme = o.style.theme;
                    if let Some(l) = o.levels.iter_mut().rev().find(|l| l.contains(p)) {
                        let max = menu_max_scroll(l.menu(), &l.lines);
                        l.scroll = (l.scroll - delta / 120.0 * 3.0 * 28.0).clamp(0.0, max);
                        l.paint(&theme);
                    }
                });
                Step::Go
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => with_open(|o| key(o, entries, VIRTUAL_KEY(msg.wParam.0 as u16))).unwrap_or(Step::Done(0)),
            // 글자와 키 떼기는 메뉴가 삼킨다 (본 창에 가지 않게)
            WM_CHAR | WM_SYSCHAR | WM_KEYUP | WM_SYSKEYUP => Step::Go,
            _ => {
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                Step::Go
            }
        };
        if let Step::Done(id) = step {
            picked = id;
            break;
        }
        // 잡아 둔 마우스를 잃었거나, 이 앱이 앞에 있다가 다른 앱이 앞으로 왔으면 닫는다
        // (이 앱이 앞에 없을 때 연 메뉴, 곧 화면 밖 시험 창의 메뉴는 다른 앱끼리 앞뒤가 바뀌어도 그대로 둔다)
        let now = unsafe { GetForegroundWindow() };
        if LOST_CAPTURE.get() || (own(foreground) && !own(now)) {
            break;
        }
    }
    unsafe {
        let _ = KillTimer(Some(root_hwnd), 1);
        if GetCapture() == root_hwnd {
            let _ = ReleaseCapture();
        }
    }
    if let Some(o) = OPEN.with(|o| o.borrow_mut().take()) {
        for l in o.levels.iter().rev() {
            l.close();
        }
    }
    picked
}

/// 떠 있는 메뉴를 빌려 f를 한다 (창 프로시저가 그리는 중이면 None)
fn with_open<T>(f: impl FnOnce(&mut Open) -> T) -> Option<T> {
    OPEN.with(|o| o.try_borrow_mut().ok().and_then(|mut o| o.as_mut().map(f)))
}

/// 이 앱의 창인가
fn own(h: HWND) -> bool {
    if h.is_invalid() {
        return true;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid == unsafe { GetCurrentProcessId() }
}

/// 맨 위 줄 목록에서 path 자리의 하위 메뉴 줄 목록
fn entries_at<'a>(root: &'a [Entry], path: &[usize]) -> &'a [Entry] {
    let mut cur = root;
    for i in path {
        match cur.get(*i) {
            Some(Entry::Sub(_, items)) => cur = items,
            _ => return &[],
        }
    }
    cur
}

/// 마우스: 움직이면 아래 줄을 가리키고 하위 메뉴를 연다. 메뉴 밖을 누르면 닫고(끝), 줄 위에서 떼면 고른다(끝)
fn mouse(o: &mut Open, root: &[Entry], msg: u32, p: POINT, armed: &mut bool) -> Step {
    let Some(i) = o.levels.iter().rposition(|l| l.contains(p)) else {
        // 메뉴 밖을 누르면 닫기만 한다 (그 누름은 뒤로 넘기지 않는다)
        return if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) { Step::Done(0) } else { Step::Go };
    };
    let item = o.levels[i].hit(p);
    match msg {
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDBLCLK => {
            *armed = true;
            if let Some(j) = item {
                point(o, root, i, j, false);
            }
            Step::Go
        }
        // 떼기: 하위 메뉴가 아닌 줄이면 고른다
        _ => match (*armed, item.map(|j| &o.levels[i].picks[j])) {
            (true, Some(Pick::Id(id))) => Step::Done(*id),
            _ => Step::Go,
        },
    }
}

/// 메뉴 i의 줄 j를 가리킨다: 더 깊은 메뉴는 닫고, 하위 메뉴 줄이면 그 하위 메뉴를 연다 (first면 그 첫 줄을 가리킨다)
fn point(o: &mut Open, root: &[Entry], i: usize, j: usize, first: bool) {
    let theme = o.style.theme;
    let already = o.levels.get(i + 1).is_some_and(|l| l.from == Some(j));
    if !already {
        while o.levels.len() > i + 1 {
            if let Some(l) = o.levels.pop() {
                l.close();
            }
        }
    }
    if o.levels[i].sel != Some(j) {
        o.levels[i].sel = Some(j);
        o.levels[i].paint(&theme);
    }
    if let Pick::Sub(path) = &o.levels[i].picks[j]
        && !already
    {
        let path = path.clone();
        let row = o.levels[i].row_rect(j);
        let parent = o.levels[i].rect;
        if let Some(mut sub) = open_level(o.owner, entries_at(root, &path), &path, &o.style, o.dpi, Place::Beside(row, parent), &o.icons) {
            sub.from = Some(j);
            if first {
                sub.sel = menu_step(&sub.lines, None, true);
            }
            sub.paint(&theme);
            o.levels.push(sub);
        }
    } else if first && let Some(l) = o.levels.get_mut(i + 1) {
        l.sel = menu_step(&l.lines, None, true);
        l.paint(&theme);
    }
}

/// 키: ↑↓·Home·End(PgUp·PgDn) 줄 옮기기, →·Return 하위 메뉴 열기, ←·Esc 하위 메뉴 닫기(맨 위면 Esc로 끝),
/// Return·스페이스 고르기, Alt·F10 끝
fn key(o: &mut Open, root: &[Entry], vk: VIRTUAL_KEY) -> Step {
    let theme = o.style.theme;
    let i = o.levels.len() - 1;
    let sel = o.levels[i].sel;
    let sub_sel = sel.filter(|j| matches!(o.levels[i].picks[*j], Pick::Sub(_)));
    match vk {
        VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT => {
            let l = &mut o.levels[i];
            l.sel = match vk {
                VK_UP => menu_step(&l.lines, l.sel, false),
                VK_DOWN => menu_step(&l.lines, l.sel, true),
                VK_HOME | VK_PRIOR => menu_step(&l.lines, None, true),
                _ => menu_step(&l.lines, None, false),
            };
            if let Some(j) = l.sel {
                l.scroll = menu_reveal(l.menu(), &l.lines, l.scroll, j);
            }
            l.paint(&theme);
            Step::Go
        }
        VK_RIGHT => {
            if let Some(j) = sub_sel {
                point(o, root, i, j, true);
            }
            Step::Go
        }
        VK_LEFT | VK_ESCAPE if i > 0 => {
            if let Some(l) = o.levels.pop() {
                l.close();
            }
            Step::Go
        }
        VK_ESCAPE | VK_MENU | VK_F10 => Step::Done(0),
        VK_RETURN | VK_SPACE => match sel.map(|j| &o.levels[i].picks[j]) {
            Some(Pick::Id(id)) => Step::Done(*id),
            Some(Pick::Sub(_)) => {
                if let Some(j) = sub_sel {
                    point(o, root, i, j, true);
                }
                Step::Go
            }
            None => Step::Go,
        },
        _ => Step::Go,
    }
}

fn register() -> bool {
    if REGISTERED.get() {
        return true;
    }
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return false };
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            // 창 그림자 (Windows 11에서는 둥근 모서리를 따라간다)
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(menu_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return false;
        }
    }
    REGISTERED.set(true);
    true
}

unsafe extern "system" fn menu_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // 눌러도 본 창의 활성을 빼앗지 않는다
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe { BeginPaint(h, &mut ps) };
            with_open(|o| {
                let theme = o.style.theme;
                if let Some(l) = o.levels.iter_mut().find(|l| l.hwnd == h) {
                    l.draw(&theme);
                }
            });
            unsafe {
                let _ = EndPaint(h, &ps);
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            LOST_CAPTURE.set(true);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
    }
}

/// 그 자리가 있는 모니터의 작업 영역 (픽셀). 어느 모니터에도 없는 자리(화면 밖에 띄운 시험 창)면 그 자리 둘레를 넓게:
/// 메뉴를 화면 안으로 끌어오지 않는다
fn work_area(p: POINT) -> RECT {
    unsafe {
        let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONULL);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !mon.is_invalid() && GetMonitorInfoW(mon, &mut info).as_bool() {
            info.rcWork
        } else {
            RECT { left: p.x - 4000, top: p.y - 4000, right: p.x + 4000, bottom: p.y + 4000 }
        }
    }
}

/// 메뉴 창 하나를 연다 (path는 맨 위 줄 목록에서 이 줄 목록의 자리). 줄이 없으면 None
fn open_level(owner: HWND, entries: &[Entry], path: &[usize], style: &Style, dpi: f32, place: Place, icons: &[Icon]) -> Option<Level> {
    let mut lines = Vec::new();
    let mut picks = Vec::new();
    let mut sep = false;
    for (k, e) in entries.iter().enumerate() {
        match e {
            Entry::Sep => sep = !lines.is_empty(),
            Entry::Item(id, l) => {
                lines.push(l.clone().sep(sep));
                picks.push(Pick::Id(*id));
                sep = false;
            }
            Entry::Sub(l, _) => {
                lines.push(l.clone().sep(sep));
                picks.push(Pick::Sub([path, &[k]].concat()));
                sep = false;
            }
        }
    }
    if lines.is_empty() {
        return None;
    }
    let line_icons = lines.iter().map(|l| if let MenuIcon::Image(k) = l.icon { icons.get(k as usize).cloned() } else { None }).collect();
    let mut gfx = Gfx::new(dpi, HIMAGELIST::default(), &style.font).ok()?;
    gfx.set_text_look(TextLook::from_key(&style.text));
    let scale = dpi / 96.0;
    let anchor = match place {
        Place::At(pt, _) => pt,
        Place::Beside(row, _) => POINT { x: row.right, y: row.top },
    };
    let work = work_area(anchor);
    let (work_w, work_h) = (work.right - work.left, work.bottom - work.top);
    let w = menu_width(&gfx, &lines, (work_w as f32 / scale).min(480.0));
    let (wpx, full_h) = ((w * scale).ceil() as i32, (menu_height(&lines) * scale).ceil() as i32);
    let hpx = full_h.min(work_h);
    let (x, y) = match place {
        Place::At(pt, above) => {
            let x = pt.x.min(work.right - wpx).max(work.left);
            let below = if pt.y + hpx <= work.bottom { pt.y } else { pt.y - hpx };
            let y = if above && pt.y - hpx >= work.top { pt.y - hpx } else { below };
            (x, y.min(work.bottom - hpx).max(work.top))
        }
        Place::Beside(row, parent) => {
            let pad = (MENU_PAD * scale) as i32;
            let x = if parent.right - 2 + wpx <= work.right { parent.right - 2 } else { parent.left - wpx + 2 };
            (x.max(work.left), (row.top - pad).min(work.bottom - hpx).max(work.top))
        }
    };
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST, CLASS, w!(""), WS_POPUP, x, y, wpx, hpx, Some(owner), None, Some(instance.into()), None).ok()?
    };
    unsafe {
        // Windows 11: 둥근 모서리와 테두리 색 (Windows 10은 그대로 네모)
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as *const c_void, size_of_val(&corner) as u32);
        let border: COLORREF = colorref(style.theme.pane_border);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &border as *const _ as *const c_void, size_of::<COLORREF>() as u32);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Some(Level { hwnd, gfx, lines, picks, rect: RECT { left: x, top: y, right: x + wpx, bottom: y + hpx }, w, h: hpx as f32 / scale, sel: None, scroll: 0.0, from: None, icons: line_icons })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows 메뉴 글: 탭 뒤는 단축키, &&는 &, 홑 &는 뺀다. 번호 0은 구분선
    #[test]
    fn windows_menu_text_becomes_menu_lines() {
        let l = line("이름 바꾸기\tF2");
        assert_eq!((l.label.as_str(), l.hint.as_str()), ("이름 바꾸기", "F2"));
        assert_eq!(line("R&&D 폴더").label, "R&D 폴더");
        assert_eq!(line("&열기").label, "열기");
        // 탐색기 메뉴 글: 괄호에 든 밑줄 글자는 괄호째, 탭 뒤 단축키는 버린다
        assert_eq!(plain("열기(&O)"), "열기");
        assert_eq!(plain("다른 이름으로 저장(&A)..."), "다른 이름으로 저장...");
        assert_eq!(plain("복사(&C)\tCtrl+C"), "복사");
        assert_eq!(plain("7-Zip"), "7-Zip");
        let e = from_pairs(&[(1, "열기"), (0, ""), (2, "닫기")]);
        assert!(matches!(e.as_slice(), [Entry::Item(1, _), Entry::Sep, Entry::Item(2, _)]));
    }

    /// 하위 메뉴는 맨 위 줄 목록에서의 자리로 찾는다
    #[test]
    fn submenus_are_found_by_their_path() {
        let root = vec![Entry::item(1, "열기"), Entry::Sep, Entry::sub("색", vec![Entry::item(10, "파랑"), Entry::sub("더", vec![Entry::item(20, "보라")])])];
        assert_eq!(entries_at(&root, &[]).len(), 3);
        assert!(matches!(entries_at(&root, &[2]), [Entry::Item(10, _), Entry::Sub(..)]));
        assert!(matches!(entries_at(&root, &[2, 1]), [Entry::Item(20, _)]));
        assert!(entries_at(&root, &[0]).is_empty(), "not a submenu");
    }
}
