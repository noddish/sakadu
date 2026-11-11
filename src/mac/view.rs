//! 맥 창: NSApplication 위임, NSWindow, 직접 그리는 NSView, 앱 메뉴, 경로 입력 상자(NSTextField).
//! 마우스·스크롤·키를 App으로 넘기고, 폴더 읽기·경로 확인은 뒷줄에서 하고 메인 줄(dispatch)로 돌려받는다.
//! 끝낼 때 화면 상태(세션)를 저장하고, 다음에 그대로 연다

use crate::app::{App, Key, LIGHTS_BOTTOM, Mods, ROW_H};
use crate::gfx::Gfx;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::runtime::{Bool, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType, NSBitmapImageFileType, NSColor,
    NSColorSpace, NSControl, NSControlTextEditingDelegate, NSEvent, NSEventModifierFlags, NSFocusRingType, NSFont, NSGraphicsContext, NSMenu, NSMenuItem,
    NSTextField, NSTextFieldDelegate, NSTextView, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow, NSWindowButton, NSWindowDelegate,
    NSWindowStyleMask, NSWindowTitleVisibility, NSWorkspace,
};
use objc2_foundation::{NSArray, NSDictionary, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};
use sakadu::draw::Color;
use sakadu::palette::{System, Theme};
use std::cell::{OnceCell, RefCell};

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static VIEW: OnceCell<Retained<SakaduView>> = const { OnceCell::new() };
}

fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|a| a.borrow_mut().as_mut().map(f))
}

/// 맥 설정: 밝은·어두운 모양과 강조 색 ('대비 증가'는 3단계에서)
struct Mac(MainThreadMarker);

impl System for Mac {
    fn prefers_dark(&self) -> bool {
        let appearance = NSApplication::sharedApplication(self.0).effectiveAppearance();
        let names = NSArray::from_slice(&[unsafe { NSAppearanceNameAqua }, unsafe { NSAppearanceNameDarkAqua }]);
        appearance.bestMatchFromAppearancesWithNames(&names).is_some_and(|n| &*n == unsafe { NSAppearanceNameDarkAqua })
    }
    fn high_contrast(&self) -> Option<[Color; 7]> {
        None
    }
    fn accent(&self, _dark: bool) -> Option<Color> {
        let c = NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        Some(Color { r: c.redComponent() as f32, g: c.greenComponent() as f32, b: c.blueComponent() as f32, a: 1.0 })
    }
}

fn theme(mtm: MainThreadMarker) -> Theme {
    Theme::resolve("system", "system", &Mac(mtm))
}

pub struct ViewIvars {
    /// 그리기 밖(누르기 판정)에서 글자 폭을 재는 그림판
    measure: Gfx,
    /// 경로 입력 상자. 입력할 때만 보인다 (한글 조합은 맥이 처리)
    field: OnceCell<Retained<NSTextField>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SakaduView"]
    #[ivars = ViewIvars]
    pub struct SakaduView;

    impl SakaduView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let Some(ns) = NSGraphicsContext::currentContext() else { return };
            let g = Gfx::for_context(&ns.CGContext());
            let size = self.bounds().size;
            with_app(|a| {
                a.resize(size.width as f32, size.height as f32);
                a.paint(&g);
            });
            self.sync_field();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            let m = mods(e);
            let clicks = e.clickCount().max(1) as usize;
            with_app(|a| a.mouse_down(&self.ivars().measure, x, y, m, clicks));
            self.after_input();
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            with_app(|a| a.mouse_dragged(x, y));
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _e: &NSEvent) {
            with_app(App::mouse_up);
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            if with_app(|a| a.mouse_moved(&self.ivars().measure, x, y)).unwrap_or(false) {
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _e: &NSEvent) {
            with_app(|a| a.hover = crate::app::Hit::None);
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            // 트랙패드는 정밀한 점 단위(관성 포함), 마우스 휠은 한 칸에 세 줄 (Windows와 같은 줄 수)
            let dy = e.scrollingDeltaY() as f32;
            let dy = if e.hasPreciseScrollingDeltas() { -dy } else { -dy * ROW_H * 3.0 };
            with_app(|a| a.scroll(&self.ivars().measure, x, y, dy));
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, e: &NSEvent) {
            let handled = key_of(e.keyCode()).is_some_and(|k| with_app(|a| a.key(k, mods(e))).unwrap_or(false));
            if handled {
                self.after_input();
            } else {
                let _: () = unsafe { msg_send![super(self), keyDown: e] };
            }
        }

        /// 편집 메뉴의 모두 선택(⌘A): 목록에 키 입력이 있을 때 모든 줄을 고른다
        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            with_app(App::select_all);
            self.after_input();
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            let t = theme(self.mtm());
            with_app(|a| a.theme = t);
            self.setNeedsDisplay(true);
        }
    }

    unsafe impl NSObjectProtocol for SakaduView {}

    unsafe impl NSControlTextEditingDelegate for SakaduView {
        /// Return은 확정, Esc는 취소. 한글을 조합하는 중의 Return은 맥이 조합을 마치는 데 써서 여기로 오지 않는다
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &NSControl, _text_view: &NSTextView, command: Sel) -> Bool {
            if command == sel!(insertNewline:) {
                let text = self.ivars().field.get().map(|f| f.stringValue().to_string()).unwrap_or_default();
                with_app(|a| a.commit_address(&text));
            } else if command == sel!(cancelOperation:) {
                with_app(App::cancel_address);
            } else {
                return Bool::NO;
            }
            self.after_input();
            Bool::YES
        }

        /// 다른 곳을 눌러 입력을 마치면 취소
        #[unsafe(method(controlTextDidEndEditing:))]
        fn did_end_editing(&self, _n: &NSNotification) {
            if with_app(|a| a.editing.take().is_some()).unwrap_or(false) {
                self.after_input();
            }
        }
    }

    unsafe impl NSTextFieldDelegate for SakaduView {}
);

impl SakaduView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let measure = Gfx::bitmap(8.0, 8.0, 1.0).expect("글자 폭을 잴 그림판");
        let this = Self::alloc(mtm).set_ivars(ViewIvars { measure, field: OnceCell::new() });
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        // 경로 입력 상자: 테두리·바탕 없이 (바탕과 강조 색 테두리는 App이 그린다)
        let field = NSTextField::initWithFrame(NSTextField::alloc(mtm), NSRect::ZERO);
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        field.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        field.setHidden(true);
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*view))) };
        view.addSubview(&field);
        let _ = view.ivars().field.set(field);
        // 마우스 올림: 보이는 곳 전체, 창이 앞에 있을 때
        let opts = NSTrackingAreaOptions::MouseMoved | NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveInKeyWindow | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(), NSRect::ZERO, opts, Some(&view), None) };
        view.addTrackingArea(&area);
        view
    }

    /// 이벤트 위치 (뷰 좌표, 왼쪽 위가 0)
    fn point(&self, e: &NSEvent) -> (f32, f32) {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        (p.x as f32, p.y as f32)
    }

    /// 입력 뒤: 쌓인 폴더 읽기를 뒷줄로 보내고, 열 파일을 열고, 제목을 맞추고, 다시 그린다
    fn after_input(&self) {
        pump();
        if let (Some(w), Some(title)) = (self.window(), with_app(|a| a.title())) {
            w.setTitle(&NSString::from_str(&title));
        }
        self.sync_field();
        self.setNeedsDisplay(true);
    }

    /// 경로 입력 상자를 App 상태에 맞춘다: 입력 중이면 그 칸의 경로 자리에 보이고 키 입력을 받는다
    fn sync_field(&self) {
        let Some(field) = self.ivars().field.get() else { return };
        let Some((rect, text, color)) = with_app(|a| (a.address_rect(), a.address_text(), a.theme.text)) else { return };
        match rect {
            Some(r) => {
                // 한 줄 글자가 세로 가운데에 오게 (시스템 13pt 글꼴의 줄 높이 약 17)
                let h = 18.0;
                let frame = NSRect::new(NSPoint::new((r.l + 6.0) as f64, (r.t + (r.h() - h) / 2.0) as f64), NSSize::new((r.w() - 12.0).max(0.0) as f64, h as f64));
                field.setFrame(frame);
                field.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(color.r as f64, color.g as f64, color.b as f64, 1.0)));
                if field.isHidden() {
                    field.setStringValue(&NSString::from_str(&text));
                    field.setHidden(false);
                    if let Some(w) = self.window() {
                        w.makeFirstResponder(Some(field));
                    }
                    unsafe { field.selectText(None) };
                }
            }
            None if !field.isHidden() => {
                field.setHidden(true);
                if let Some(w) = self.window() {
                    w.makeFirstResponder(Some(self));
                }
            }
            None => {}
        }
    }
}

/// 쌓인 폴더 읽기 요청은 뒷줄에서 읽어 메인 줄로 돌려받고, 열어 달라는 파일은 macOS '열기'로 연다
/// ('열기'로 여는 앱은 사카두의 권한을 물려받지 않는다)
fn pump() {
    let Some((reqs, probes, files)) = with_app(|a| (a.take_requests(), a.take_probes(), std::mem::take(&mut a.open_files))) else { return };
    for p in probes {
        std::thread::spawn(move || {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            let result = crate::app::resolve_input(&p.text, &home);
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.on_probed(p.pane, p.tab, result));
                // 확인한 폴더로 옮겼으면 그 폴더를 읽는다
                pump();
                VIEW.with(|v| {
                    if let Some(v) = v.get() {
                        v.setNeedsDisplay(true);
                    }
                });
            });
        });
    }
    for r in reqs {
        std::thread::spawn(move || {
            let result = sakadu::listing::list_dir(&r.dir);
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.on_listed(r.tab, r.generation, result));
                VIEW.with(|v| {
                    if let Some(v) = v.get() {
                        v.setNeedsDisplay(true);
                    }
                });
            });
        });
    }
    for f in files {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&f));
        if !NSWorkspace::sharedWorkspace().openURL(&url) {
            sakadu::diag::log(|| format!("open failed: {f}"));
        }
    }
}

fn mods(e: &NSEvent) -> Mods {
    let f = e.modifierFlags();
    Mods {
        cmd: f.contains(NSEventModifierFlags::Command),
        shift: f.contains(NSEventModifierFlags::Shift),
        alt: f.contains(NSEventModifierFlags::Option),
        ctrl: f.contains(NSEventModifierFlags::Control),
    }
}

/// 맥 키 번호(kVK_*, 자판 배열과 상관없는 물리 자리) → 키. 한/영 상태와 상관없이 ⌘T 같은 단축키가 된다
fn key_of(code: u16) -> Option<Key> {
    Some(match code {
        0x7E => Key::Up,
        0x7D => Key::Down,
        0x73 => Key::Home,
        0x77 => Key::End,
        0x74 => Key::PageUp,
        0x79 => Key::PageDown,
        0x30 => Key::Tab,
        0x35 => Key::Esc,
        0x00 => Key::Char('a'),
        0x25 => Key::Char('l'),
        0x05 => Key::Char('g'),
        0x1F => Key::Char('o'),
        0x0F => Key::Char('r'),
        0x11 => Key::Char('t'),
        0x0D => Key::Char('w'),
        0x21 => Key::Char('['),
        0x1E => Key::Char(']'),
        0x2F => Key::Char('.'),
        0x12 => Key::Char('1'),
        0x13 => Key::Char('2'),
        0x14 => Key::Char('3'),
        0x15 => Key::Char('4'),
        0x17 => Key::Char('5'),
        _ => return None,
    })
}

/// 홈 아래 바로가기 줄과 연결된 디스크
fn places() -> Vec<(String, String)> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let mut out = vec![("홈".to_string(), home.clone())];
    for (name, sub) in [("데스크탑", "Desktop"), ("문서", "Documents"), ("다운로드", "Downloads")] {
        out.push((name.to_string(), format!("{home}/{sub}")));
    }
    out.push(("응용 프로그램".to_string(), "/Applications".to_string()));
    if let Ok(vols) = std::fs::read_dir("/Volumes") {
        let mut v: Vec<(String, String)> = vols.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().to_string_lossy().into_owned())).collect();
        v.sort();
        out.extend(v);
    }
    out
}

#[derive(Default)]
pub struct DelegateIvars {
    window: OnceCell<Retained<NSWindow>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SakaduDelegate"]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _n: &NSNotification) {
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            main_menu(mtm, &app);
            let window = open_window(mtm, None);
            window.setDelegate(Some(ProtocolObject::from_ref(self)));
            let _ = self.ivars().window.set(window);
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn quit_after_last_window(&self, _app: &NSApplication) -> bool {
            true
        }

        /// 끝낼 때(⌘Q, 창 닫기) 화면 상태를 저장한다
        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _n: &NSNotification) {
            let s = with_app(|a| a.persist.then(|| a.to_session())).flatten();
            if let Some(s) = s
                && let Err(e) = sakadu::session::save(&s)
            {
                sakadu::diag::log(|| format!("session: {e}"));
            }
        }
    }

    unsafe impl NSWindowDelegate for Delegate {}
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars::default());
        unsafe { msg_send![super(this), init] }
    }
}

/// 모양 2의 창: 제목 표시줄을 투명하게 하고 내용이 창 전체를 쓴다 (신호등은 사이드바 위에 겹친다).
/// start가 있으면 그 폴더에서 시작한다 (없으면 홈)
fn open_window(mtm: MainThreadMarker, start: Option<String>) -> Retained<NSWindow> {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1280.0, 800.0));
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable | NSWindowStyleMask::FullSizeContentView;
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false) };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitlebarAppearsTransparent(true);
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setContentMinSize(NSSize::new(760.0, 420.0));
    window.center();
    let _ = window.setFrameAutosaveName(&NSString::from_str("SakaduMain"));

    let home = sakadu::util::normalize(&std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
    let first = start.as_deref().map(sakadu::util::normalize).unwrap_or_else(|| home.clone());
    let mut app = App::new(1280.0, 800.0, theme(mtm), &first, sakadu::favorites::load(), places());
    // 보통 실행: 지난번 화면 상태로 열고 끝낼 때 저장한다 (CI 창 확인처럼 시작 폴더를 준 경우는 그대로)
    if start.is_none() {
        app.persist = true;
        if let Some(s) = sakadu::session::load() {
            app.restore(&s, &home);
        }
    }
    app.lights_bottom = lights_bottom(&window);
    window.setTitle(&NSString::from_str(&app.title()));
    APP.with(|a| *a.borrow_mut() = Some(app));

    let view = SakaduView::new(mtm, frame);
    window.setContentView(Some(&view));
    window.makeFirstResponder(Some(&view));
    VIEW.with(|v| {
        let _ = v.set(view);
    });
    pump();
    window.makeKeyAndOrderFront(None);
    window
}

/// 앱 메뉴: Sakadu(정보·가리기·끝내기), 편집(입력 상자의 실행 취소·복사·붙여넣기·모두 선택), 윈도(최소화·확대/축소)
fn main_menu(mtm: MainThreadMarker, app: &NSApplication) {
    let item = |menu: &NSMenu, title: &str, action: Sel, key: &str, mods: Option<NSEventModifierFlags>| {
        let it = unsafe { menu.addItemWithTitle_action_keyEquivalent(&NSString::from_str(title), Some(action), &NSString::from_str(key)) };
        if let Some(m) = mods {
            it.setKeyEquivalentModifierMask(m);
        }
    };
    let submenu = |bar: &NSMenu, title: &str| {
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
        let top = NSMenuItem::new(mtm);
        top.setSubmenu(Some(&menu));
        bar.addItem(&top);
        menu
    };
    let bar = NSMenu::new(mtm);
    let m = submenu(&bar, "Sakadu");
    item(&m, "Sakadu 정보", sel!(orderFrontStandardAboutPanel:), "", None);
    m.addItem(&NSMenuItem::separatorItem(mtm));
    item(&m, "Sakadu 가리기", sel!(hide:), "h", None);
    item(&m, "기타 가리기", sel!(hideOtherApplications:), "h", Some(NSEventModifierFlags::Command | NSEventModifierFlags::Option));
    item(&m, "모두 보기", sel!(unhideAllApplications:), "", None);
    m.addItem(&NSMenuItem::separatorItem(mtm));
    item(&m, "Sakadu 종료", sel!(terminate:), "q", None);
    let e = submenu(&bar, "편집");
    item(&e, "실행 취소", sel!(undo:), "z", None);
    item(&e, "실행 복귀", sel!(redo:), "z", Some(NSEventModifierFlags::Command | NSEventModifierFlags::Shift));
    e.addItem(&NSMenuItem::separatorItem(mtm));
    item(&e, "오려두기", sel!(cut:), "x", None);
    item(&e, "복사하기", sel!(copy:), "c", None);
    item(&e, "붙여넣기", sel!(paste:), "v", None);
    item(&e, "전체 선택", sel!(selectAll:), "a", None);
    let w = submenu(&bar, "윈도우");
    item(&w, "최소화", sel!(performMiniaturize:), "m", None);
    item(&w, "확대/축소", sel!(performZoom:), "", None);
    app.setMainMenu(Some(&bar));
    app.setWindowsMenu(Some(&w));
}

/// 신호등(닫기 단추)의 아래 끝을 창 위에서부터 잰 값
fn lights_bottom(window: &NSWindow) -> f32 {
    let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else { return LIGHTS_BOTTOM };
    let Some(sup) = (unsafe { close.superview() }) else { return LIGHTS_BOTTOM };
    // 창 좌표는 왼쪽 아래가 0이라, 위에서부터의 거리는 창 높이 - 단추 아래 끝
    let f = sup.convertRect_toView(close.frame(), None);
    ((window.frame().size.height - f.origin.y) as f32).clamp(12.0, 60.0)
}

/// 창 확인용 (CI): 창을 열어 start 폴더를 읽고, 창 틀(신호등 포함)을 그린 그림을 out에 PNG로 저장한 뒤 끝낸다.
/// 앱 반복을 돌리지 않으므로 폴더는 바로 읽는다
pub fn snapshot(out: &std::path::Path, start: String) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("메인 줄이 아님")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let window = open_window(mtm, Some(start));
    with_app(|a| {
        for p in 0..2 {
            a.refresh(p);
        }
        a.settle();
    });
    window.displayIfNeeded();
    let frame_view = window.contentView().and_then(|v| unsafe { v.superview() }).ok_or("창 틀을 찾지 못함")?;
    let b = frame_view.bounds();
    let rep = frame_view.bitmapImageRepForCachingDisplayInRect(b).ok_or("창 그림판을 만들지 못함")?;
    frame_view.cacheDisplayInRect_toBitmapImageRep(b, &rep);
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }.ok_or("PNG로 바꾸지 못함")?;
    std::fs::write(out, data.to_vec()).map_err(|e| e.to_string())
}

/// 창을 띄우고 앱을 돌린다 (창을 닫으면 끝난다)
pub fn run() {
    let mtm = MainThreadMarker::new().expect("메인 줄에서 시작");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = Delegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}
