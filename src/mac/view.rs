//! 맥 창: NSApplication 위임, NSWindow, 직접 그리는 NSView, 앱 메뉴, 입력 상자(NSTextField: 경로, 작업 공간 이름).
//! 마우스·스크롤·키를 App으로 넘기고, 폴더 읽기·경로 확인·미리 보기 카드의 그림과 글 읽기·아이콘 보기의 썸네일 읽기는
//! 뒷줄에서 하고 메인 줄(dispatch)로 돌려받는다.
//! 끝낼 때 화면 상태(세션)와 작업 공간을 저장하고, 다음에 그대로 연다

use crate::app::{App, Key, LIGHTS_BOTTOM, LOADING_NOTE_MS, MenuKind, Mods};
use sakadu::ui::ROW_H;
use crate::gfx::Gfx;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::runtime::{Bool, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType, NSBitmapImageFileType, NSColor,
    NSColorSpace, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSEvent, NSEventModifierFlags, NSFocusRingType, NSFont, NSGraphicsContext, NSMenu, NSMenuItem,
    NSTextField, NSTextFieldDelegate, NSTextView, NSTitlebarSeparatorStyle, NSToolbar, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow, NSWindowButton, NSWindowDelegate,
    NSWindowStyleMask, NSWindowTitleVisibility, NSWindowToolbarStyle, NSWorkspace,
};
use objc2_foundation::{NSArray, NSDictionary, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};
use sakadu::draw::Color;
use sakadu::palette::{System, Theme};
use std::cell::{OnceCell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static VIEW: OnceCell<Retained<SakaduView>> = const { OnceCell::new() };
}

/// 가장 나중에 낸 미리 보기 카드 요청의 번호. 뒷줄이 그림을 읽기 직전에 보고, 그 사이 다른 요청이 나왔으면 읽지 않는다
static LATEST_PREVIEW: AtomicU64 = AtomicU64::new(0);
/// 그림을 읽기 전에 잠깐 기다리는 시간 (화살표 키를 누르고 있을 때 지나가는 사진을 하나하나 읽지 않게)
const PREVIEW_SETTLE: Duration = Duration::from_millis(60);

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

/// 맥 설정에서 읽은 모양(어두운지)과 강조 색. 테마는 App이 이 값과 지금 작업 공간의 색으로 정한다
fn system(mtm: MainThreadMarker) -> (bool, Option<Color>) {
    let sys = Mac(mtm);
    let dark = sys.prefers_dark();
    (dark, sys.accent(dark))
}

pub struct ViewIvars {
    /// 그리기 밖(누르기 판정)에서 글자 폭을 재는 그림판
    measure: Gfx,
    /// 입력 상자 (경로, 작업 공간 이름). 입력할 때만 보인다 (한글 조합은 맥이 처리)
    field: OnceCell<Retained<NSTextField>>,
    /// 입력 상자가 지금 무엇을 담고 있는지: 어느 칸의 경로인지, 작업 공간 이름인지 (App::edit_key. 바뀌면 다시 채운다)
    field_pane: std::cell::Cell<Option<usize>>,
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
            let mut g = Gfx::for_context(&ns.CGContext());
            let size = self.bounds().size;
            // 화면 배율 (레티나는 2): 썸네일을 몇 픽셀로 읽을지 정한다
            let scale = self.window().map_or(2.0, |w| w.backingScaleFactor() as f32);
            with_app(|a| {
                a.scale = scale;
                a.resize(size.width as f32, size.height as f32);
                a.paint(&mut g);
            });
            self.sync_field();
            // 방금 그린 화면에 보이는데 아직 없는 썸네일을 읽기 시작한다
            pump_thumbs();
        }

        /// 제목 표시줄 자리까지 이 뷰가 덮고 있다. 기본값(true)이면 그 자리의 클릭이 창 끌기로만 쓰여 도구 막대 단추가 눌리지 않는다
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            let m = mods(e);
            let clicks = e.clickCount().max(1) as usize;
            // 도구 막대의 빈 곳: 끌면 창이 움직이고, 두 번 누르면 확대·축소 (제목 표시줄과 같게)
            if with_app(|a| a.drag_zone(&self.ivars().measure, x, y)).unwrap_or(false) {
                if let Some(w) = self.window() {
                    if clicks == 2 {
                        w.performZoom(None);
                    } else {
                        w.performWindowDragWithEvent(e);
                    }
                }
                return;
            }
            with_app(|a| a.mouse_down(&self.ivars().measure, x, y, m, clicks));
            self.after_input();
        }

        /// 오른쪽 단추: 작업 공간 아이콘이면 그 공간의 메뉴 (색, 이름 바꾸기, 지우기),
        /// 사이드바의 고정한 줄이나 즐겨찾기 항목이면 그 줄의 메뉴 (옮기기, '모든 공간'에 두고 빼기, 고정 풀기)
        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            if with_app(|a| a.right_down(&self.ivars().measure, x, y)).unwrap_or(false) {
                self.after_input();
            }
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

        /// 메뉴(도구 막대 단추, 작업 공간 아이콘, 사이드바 줄)에서 고른 줄. tag에 메뉴 종류와 줄 번호가 들어 있다 (MenuKind::tag)
        #[unsafe(method(pickMenu:))]
        fn pick_menu(&self, sender: &NSMenuItem) {
            let Some((kind, item)) = MenuKind::from_tag(sender.tag()) else { return };
            with_app(|a| a.pick_menu(kind, item));
            self.after_input();
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            let (dark, accent) = system(self.mtm());
            with_app(|a| a.set_system(dark, accent));
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
            if with_app(App::end_editing).unwrap_or(false) {
                self.after_input();
            }
        }
    }

    unsafe impl NSTextFieldDelegate for SakaduView {}
);

impl SakaduView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let measure = Gfx::bitmap(8.0, 8.0, 1.0).expect("글자 폭을 잴 그림판");
        let this = Self::alloc(mtm).set_ivars(ViewIvars { measure, field: OnceCell::new(), field_pane: std::cell::Cell::new(None) });
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        // 입력 상자: 테두리·바탕 없이 (바탕과 강조 색 테두리는 App이 그린다)
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

    /// 입력 뒤: 쌓인 폴더 읽기를 뒷줄로 보내고, 열 파일을 열고, 제목을 맞추고, 다시 그린다. 메뉴를 열어 달라고 했으면 연다
    fn after_input(&self) {
        pump();
        if let (Some(w), Some(title)) = (self.window(), with_app(|a| a.title())) {
            w.setTitle(&NSString::from_str(&title));
        }
        self.sync_field();
        self.setNeedsDisplay(true);
        if let Some((kind, anchor)) = with_app(|a| a.take_menu()).flatten() {
            self.pop_menu(kind, anchor);
        }
    }

    /// 단추(도구 막대 단추, 작업 공간 아이콘)나 사이드바 줄 아래에 맥 기본 메뉴를 연다: 이름, 지금 것에 체크. 고르면 pickMenu:로 온다.
    /// 메뉴가 떠 있는 동안 이 함수는 돌아오지 않으므로 App을 빌린 채로 부르지 않는다
    fn pop_menu(&self, kind: MenuKind, anchor: sakadu::draw::R) {
        let mtm = self.mtm();
        let Some((items, brk)) = with_app(|a| (a.menu_items(kind), a.menu_break(kind))) else { return };
        let menu = NSMenu::new(mtm);
        for (i, (title, on)) in items.iter().enumerate() {
            // 구분선: 보기 방식과 숨김 항목 보기 사이, 공간 색과 그 밖의 줄 사이, 고정한 줄을 옮기는 줄과 그 밖의 줄 사이
            if brk == Some(i) {
                menu.addItem(&NSMenuItem::separatorItem(mtm));
            }
            let it = unsafe { menu.addItemWithTitle_action_keyEquivalent(&NSString::from_str(title), Some(sel!(pickMenu:)), &NSString::from_str("")) };
            unsafe { it.setTarget(Some(self)) };
            it.setTag(kind.tag(i));
            it.setState(if *on { NSControlStateValueOn } else { NSControlStateValueOff });
        }
        // 뷰는 위아래를 뒤집어 쓰므로(왼쪽 위가 0) 단추(줄) 아래 끝이 곧 메뉴의 위
        menu.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(anchor.l as f64, anchor.b as f64 + 4.0), Some(self));
    }

    /// 입력 상자를 App 상태에 맞춘다: 입력 중이면 그 자리(칸의 경로 자리, 사이드바의 공간 이름 자리)에 보이고 키 입력을 받는다
    fn sync_field(&self) {
        let Some(field) = self.ivars().field.get() else { return };
        let Some((rect, text, color, pane)) = with_app(|a| (a.address_rect(), a.address_text(), a.theme.text, a.edit_key())) else { return };
        match rect {
            Some(r) => {
                // 한 줄 글자가 세로 가운데에 오게 (시스템 13pt 글꼴의 줄 높이 약 17)
                let h = 18.0;
                let frame = NSRect::new(NSPoint::new((r.l + 6.0) as f64, (r.t + (r.h() - h) / 2.0) as f64), NSSize::new((r.w() - 12.0).max(0.0) as f64, h as f64));
                field.setFrame(frame);
                field.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(color.r as f64, color.g as f64, color.b as f64, 1.0)));
                // 처음 열 때와, 입력 중에 다른 입력(다른 칸의 경로, 공간 이름)으로 옮겼을 때 그 글로 다시 채운다
                if field.isHidden() || self.ivars().field_pane.get() != pane {
                    self.ivars().field_pane.set(pane);
                    field.setStringValue(&NSString::from_str(&text));
                    field.setHidden(false);
                    if let Some(w) = self.window() {
                        w.makeFirstResponder(Some(field));
                    }
                    unsafe { field.selectText(None) };
                }
            }
            None if !field.isHidden() => {
                self.ivars().field_pane.set(None);
                field.setHidden(true);
                if let Some(w) = self.window() {
                    w.makeFirstResponder(Some(self));
                }
            }
            None => {}
        }
    }
}

/// 다시 그려 달라고 한다 (메인 줄에서)
fn redraw() {
    VIEW.with(|v| {
        if let Some(v) = v.get() {
            v.setNeedsDisplay(true);
        }
    });
}

/// 쌓인 폴더 읽기 요청은 뒷줄에서 읽어 메인 줄로 돌려받고, 열어 달라는 파일은 macOS '열기'로 연다
/// ('열기'로 여는 앱은 사카두의 권한을 물려받지 않는다). 미리 보기 카드의 그림과 글도 뒷줄에서 읽는다
fn pump() {
    let Some((reqs, probes, previews, files)) = with_app(|a| (a.take_requests(), a.take_probes(), a.take_previews(), std::mem::take(&mut a.open_files))) else { return };
    let mut picture = false;
    for r in previews {
        LATEST_PREVIEW.store(r.seq, Ordering::Relaxed);
        picture |= r.px > 0;
        std::thread::spawn(move || {
            // 그림은 잠깐 기다렸다가, 그 사이 다른 요청이 나왔으면(다른 파일을 골랐으면) 읽지 않는다
            if r.px > 0 {
                std::thread::sleep(PREVIEW_SETTLE);
                if LATEST_PREVIEW.load(Ordering::Relaxed) != r.seq {
                    return;
                }
            }
            let result = crate::app::load_preview(&r);
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.on_previewed(&r, result));
                redraw();
            });
        });
    }
    // 그림이 늦으면 카드에 '불러오는 중…'이 보이게, 그때 한 번 더 그린다
    if picture && let Ok(when) = DispatchTime::try_from(Duration::from_millis(LOADING_NOTE_MS + 20)) {
        let _ = DispatchQueue::main().after(when, redraw);
    }
    for p in probes {
        std::thread::spawn(move || {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            let result = crate::app::resolve_input(&p.text, &home);
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.on_probed(&p, result));
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
                // 읽기 결과가 새 요청을 낼 수 있다 (세션에서 연 폴더가 없으면 홈을 읽는다)
                pump();
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

/// 아이콘 보기의 썸네일을 뒷줄에서 읽어 메인 줄로 돌려받는다. App::take_thumbs가 지금 보이는데 아직 없는 것만,
/// 읽는 중인 것이 둘을 넘지 않게 내준다. 그린 뒤(drawRect)마다 부르므로 스크롤하거나 폴더를 옮기면 그때 보이는 것을 읽는다.
/// 하나를 다 읽으면 다음 것을 이어서 읽고 다시 그린다
fn pump_thumbs() {
    let Some(reqs) = with_app(|a| a.take_thumbs()) else { return };
    for r in reqs {
        std::thread::spawn(move || {
            let pic = crate::app::load_thumb(&r);
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.on_thumb(&r, pic));
                pump_thumbs();
                redraw();
            });
        });
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
        0x7B => Key::Left,
        0x7C => Key::Right,
        0x73 => Key::Home,
        0x77 => Key::End,
        0x74 => Key::PageUp,
        0x79 => Key::PageDown,
        0x30 => Key::Tab,
        0x35 => Key::Esc,
        0x31 => Key::Char(' '),
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

/// 사이드바 '모든 공간'에 늘 있는 줄: 홈 아래 폴더들과 연결된 디스크
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

        /// 끝낼 때(⌘Q, 창 닫기) 작업 공간과 화면 상태를 저장한다
        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _n: &NSNotification) {
            let s = with_app(|a| {
                a.persist.then(|| {
                    a.save_workspaces();
                    a.to_session()
                })
            })
            .flatten();
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

/// 창: 제목 표시줄을 투명하게 하고 내용이 창 전체를 쓴다 (신호등은 사이드바 위에 겹친다).
/// start가 있으면 그 폴더에서 시작한다 (없으면 홈)
fn open_window(mtm: MainThreadMarker, start: Option<String>) -> Retained<NSWindow> {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1280.0, 800.0));
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable | NSWindowStyleMask::FullSizeContentView;
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false) };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitlebarAppearsTransparent(true);
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    // 빈 도구 막대를 달면 제목 표시줄이 도구 막대 높이(52)가 되어, 신호등이 우리가 그리는 도구 막대 단추와 같은 높이에 온다.
    // 최신 macOS에서는 도구 막대가 있는 창의 모서리가 Finder처럼 더 둥글다
    window.setToolbar(Some(&NSToolbar::new(mtm)));
    window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    window.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
    window.setContentMinSize(NSSize::new(760.0, 420.0));
    window.center();
    let _ = window.setFrameAutosaveName(&NSString::from_str("SakaduMain"));

    let home = sakadu::util::normalize(&std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
    let first = start.as_deref().map(sakadu::util::normalize).unwrap_or_else(|| home.clone());
    let (dark, accent) = system(mtm);
    let mut app = App::new(1280.0, 800.0, if dark { Theme::DARK } else { Theme::LIGHT }, &first, sakadu::favorites::load(), places());
    app.set_system(dark, accent);
    // 보통 실행: 지난번 작업 공간과 화면 상태로 열고 끝낼 때 저장한다 (CI 창 확인처럼 시작 폴더를 준 경우는 그대로)
    if start.is_none() {
        app.persist = true;
        app.open_saved(sakadu::workspaces::load(), sakadu::session::load().as_ref(), &home);
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
