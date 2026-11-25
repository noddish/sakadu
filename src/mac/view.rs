//! 맥 창: NSApplication 위임, NSWindow, 직접 그리는 NSView, 앱 메뉴, 입력 상자(NSTextField: 경로, 작업 공간 이름, 항목 이름)와
//! 검색 입력 칸(NSTextField 하나 더: 검색하는 동안 늘 보이고, 칸에서 일하는 동안에도 검색어를 지킨다),
//! 명령 메뉴(⌘K)의 입력 칸(또 하나: 판이 떠 있는 동안 키보드를 갖는다).
//! 검색은 자체 색인(search.rs)을 이 쪽이 띄우고, 입력할 때마다 뒷줄에서 찾는다.
//! 마우스·스크롤·키를 App으로 넘기고, 폴더 읽기·경로 확인·미리 보기 카드의 그림과 글 읽기·아이콘 보기의 썸네일 읽기는
//! 뒷줄에서 하고 메인 줄(dispatch)로 돌려받는다. 파일 작업(새 폴더, 이름 바꾸기, 휴지통, 복사, 되돌리기)은 뒷줄 하나가 맡긴 순서대로 하나씩 한다.
//! 클립보드에는 파일 주소로 담고 읽는다 (Finder와 주고받을 수 있다).
//! 끌어서 놓기도 파일 주소로 한다: 고른 줄을 끌어 내면 맥의 끌기를 시작하고(Finder나 다른 칸에 놓을 수 있다),
//! 끌려 온 파일은 App에 어디에 놓일지 묻고 놓는다. 압축 파일 안의 것은 App이 임시 폴더에 꺼낸 뒤에 끌고, 받는 쪽에 복사만 허락한다.
//! 훑어보기(스페이스, ⌘Y)는 맥의 훑어보기 창(QLPreviewPanel)에 고른 파일을 띄운다. 그 창의 내용은 이 뷰가 댄다.
//! 오른쪽 미리 보기 창(⌥⌘P)에서 문서(PDF·Office 따위)는 그 본문 자리에 Quick Look 보기(QLPreviewView)를 하위 뷰로 둔다.
//! 고정한 파일이 그대로 있는지도 뒷줄에서 확인한다.
//! 디스크가 붙고 떨어지는 것은 NSWorkspace의 알림으로 듣고 사이드바를 고친다. 디스크 꺼내기와 네트워크 공유 연결(volumes.rs)은 뒷줄에서 한다.
//! 끝낼 때 화면 상태(세션)와 작업 공간을 저장하고, 다음에 그대로 연다.
//! 설정 창(앱 메뉴의 '설정…', ⌘,)은 본 창과 따로 뜬다 (settings.rs).
//! Finder가 열어 달라고 보낸 폴더·파일('다음으로 열기', 서비스의 'Sakadu에서 열기', Dock 아이콘에 놓기)은 앱 위임이 받아 새 탭으로 연다.
//! 메뉴 막대 대기(창을 닫으면 숨기만 한다), 어디서든 찾기(전역 단축키), 로그인할 때 열기는 standby.rs.
//! 창 시험(smoke)은 이 창을 그대로 열고 실제 입력의 길로 파일 작업을 돌려 본다 (CI의 맥 러너에서)

use crate::app::{App, Key, LIGHTS_BOTTOM, LOADING_NOTE_MS, Mods, Paste, VolumeEvent};
use sakadu::ui::ROW_H;
use crate::gfx::Gfx;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::ProtocolObject;
use objc2::ffi::NSInteger;
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, Sel};
use objc2::{AnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType, NSBitmapImageFileType, NSColor,
    NSColorSpace, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSCursor, NSEvent, NSEventModifierFlags, NSEventType, NSFocusRingType, NSFont, NSGraphicsContext, NSMenu, NSMenuItem,
    NSDragOperation, NSDraggingContext, NSDraggingDestination, NSDraggingInfo, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSPasteboard, NSPasteboardTypeFileURL,
    NSPasteboardWriting,
    NSTextField, NSTextFieldDelegate, NSTextView, NSTitlebarSeparatorStyle, NSToolbar, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow, NSWindowButton, NSWindowDelegate,
    NSWindowStyleMask, NSWindowTitleVisibility, NSWindowToolbarStyle, NSWorkspace, NSWorkspaceDidMountNotification, NSWorkspaceDidRenameVolumeNotification,
    NSWorkspaceDidUnmountNotification, NSWorkspaceVolumeOldURLKey, NSWorkspaceVolumeURLKey,
};
use objc2_foundation::{NSArray, NSDictionary, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString, NSURL};
use sakadu::draw::Color;
use sakadu::palette::{System, Theme};
use std::cell::{OnceCell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

mod settings;
mod smoke;
mod standby;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// 창 시험을 하는 중이면 그 시험 폴더 (창을 이 폴더로 열고, 창이 뜨면 시험을 시작한다)
    static SMOKE: RefCell<Option<String>> = const { RefCell::new(None) };
    /// 입력 상자를 App 상태에 맞추는 중 (sync_field). 그동안 상자의 편집이 끝났다는 알림은 이쪽이 한 일이라 받지 않는다
    static SYNCING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static VIEW: OnceCell<Retained<SakaduView>> = const { OnceCell::new() };
    /// 잘라 둔 것을 클립보드에 담은 직후의 클립보드 번호. 붙여넣을 때 번호가 그대로면(그 사이 다른 것이 담기지 않았으면) 옮긴다
    static CUT: std::cell::Cell<Option<isize>> = const { std::cell::Cell::new(None) };
    /// 끌려 온 것과 받는 폴더가 같은 디스크인지 물어 둔 것: (끌려 온 첫 항목, 받는 폴더, 같은 디스크인지).
    /// 끄는 동안 마우스가 움직일 때마다 디스크에 묻지 않게 한다
    static SAME_VOLUME: RefCell<Option<(String, String, bool)>> = const { RefCell::new(None) };
    /// 훑어보기 창에 대고 있는 것: (경로들, 지금 보이는 것의 번호). 그 창이 물으면 이것을 준다
    static LOOK: RefCell<(Vec<String>, usize)> = const { RefCell::new((Vec::new(), 0)) };
    /// 이 뷰가 훑어보기 창의 내용을 대고 있다 (beginPreviewPanelControl:부터 endPreviewPanelControl:까지)
    static LOOK_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 훑어보기 창의 클래스 (처음 쓸 때 찾는다. 찾지 못하면 None)
    static LOOK_CLASS: OnceCell<Option<&'static AnyClass>> = const { OnceCell::new() };
    /// 파일 작업을 하는 뒷줄로 가는 길 (처음 맡길 때 그 줄을 만든다)
    static OPS: OnceCell<std::sync::mpsc::Sender<crate::fileops::Op>> = const { OnceCell::new() };
    /// 폴더 감시 (처음 쓸 때 띄운다). 띄우지 못했으면 None: 그때는 창이 다시 앞으로 올 때와 파일 작업 뒤에만 다시 읽는다
    static WATCH: OnceCell<Option<crate::watch::Watcher>> = const { OnceCell::new() };
    /// 사이드바 줄의 움직임 때문에 다음 그림을 부탁해 두었다 (schedule_flow)
    static FLOW_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 지금 끌어 내는 것이 압축 파일에서 꺼낸 임시 복사본이다: 받는 쪽에 복사만 허락한다
    static DRAG_COPY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 앱이 뜨는 중에(창을 만들기 전에) Finder가 열어 달라고 보낸 경로. 창을 만든 뒤에 연다
    static OPEN_LATER: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// 가장 나중에 낸 미리 보기(카드나 오른쪽 창) 요청의 번호. 뒷줄이 그림을 읽기 직전에 보고, 그 사이 다른 요청이 나왔으면 읽지 않는다
static LATEST_PREVIEW: AtomicU64 = AtomicU64::new(0);
/// 그림을 읽기 전에 잠깐 기다리는 시간 (화살표 키를 누르고 있을 때 지나가는 사진을 하나하나 읽지 않게)
const PREVIEW_SETTLE: Duration = Duration::from_millis(60);
/// 폴더를 읽기 전에 그 폴더의 감시가 걸리기를 기다리는 한도. 보통은 곧 걸린다. 꺼진 네트워크 폴더처럼 여는 데
/// 오래 걸리면 이만큼만 기다리고 읽는다
const WATCH_WAIT: Duration = Duration::from_secs(2);

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
    /// 입력 상자 (경로, 작업 공간 이름, 항목 이름). 입력할 때만 보인다 (한글 조합은 맥이 처리)
    field: OnceCell<Retained<NSTextField>>,
    /// 입력 상자가 지금 무엇을 담고 있는지: 어느 칸의 경로인지, 작업 공간 이름인지, 어느 항목의 이름인지 (App::edit_key. 바뀌면 다시 채운다)
    field_pane: std::cell::Cell<Option<u64>>,
    /// 검색 입력 칸 (도구 막대에서 늘어난 칸의 글자 자리)
    search: OnceCell<Retained<NSTextField>>,
    /// 지난번에 맞출 때 검색 입력 칸이 키보드를 갖고 있었는지. 그 사이 키보드를 가져갔으면 사용자가 그 칸을 누른 것이다
    search_keys: std::cell::Cell<bool>,
    /// 명령 메뉴(⌘K)의 입력 칸 (판의 입력 자리. 명령 메뉴가 떠 있는 동안 늘 키보드를 갖는다)
    commands: OnceCell<Retained<NSTextField>>,
    /// 지금 보이는 마우스 모양 (바뀔 때만 다시 정한다: 입력 상자 위의 글자 커서를 덮지 않게)
    pointer: std::cell::Cell<crate::app::Pointer>,
    /// 오른쪽 미리 보기 창의 본문을 덮는 Quick Look 보기 (QLPreviewView). 처음 쓸 때 만든다. 만들지 못했으면 None
    look_view: OnceCell<Option<Retained<NSView>>>,
    /// 그 보기에 넘긴 파일 (경로, 수정한 때). 바뀔 때만 다시 넘긴다 (그릴 때마다 넘기면 처음부터 다시 그린다)
    look_item: RefCell<Option<(String, u64)>>,
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
            // 맥 설정의 '동작 줄이기'를 켰으면 사이드바 줄이 미끄러지지 않고 바로 옮겨진다
            let motion = !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
            let moving = with_app(|a| {
                a.scale = scale;
                a.motion = motion;
                a.resize(size.width as f32, size.height as f32);
                // 사이드바 줄을 끌며 목록 끝에 대고 있으면 지난 그림 뒤로 지난 때만큼 스크롤한다 (마우스가 멈춰 있어도)
                let edge = a.side_edge_step(std::time::Instant::now());
                a.paint(&mut g);
                a.side_moving() || edge
            });
            // 사이드바 줄이 아직 미끄러지는 중이거나 끌기 자동 스크롤 중이면 다음 그림을 곧 다시 그린다
            if moving == Some(true) {
                schedule_flow();
            }
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
        /// 사이드바의 고정한 줄이나 즐겨찾기 항목이면 그 줄의 메뉴 (옮기기, '모든 공간'에 두고 빼기, 고정 풀기, 그룹으로 옮기기),
        /// 즐겨찾기 그룹 머리면 그룹 메뉴 (이름 바꾸기, 그룹 풀기, 그룹 지우기),
        /// 목록의 줄이면 고른 항목들의 메뉴 (열기, Finder에서 보기, 이름 바꾸기, 휴지통으로 이동), 목록의 빈 곳이면 새 폴더와 새로 고침
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
            // 고른 줄을 누른 채 움직였으면 App이 끌어 낼 항목들을 내준다: 맥의 끌기를 시작한다.
            // 압축 파일 안의 것은 App이 먼저 꺼내 달라고 맡긴다 (꺼낸 뒤 계속 누르고 움직이면 그때 꺼낸 것을 끈다)
            let (drag, queued) = with_app(|a| {
                a.mouse_dragged(x, y);
                (a.take_drag().map(|paths| (paths, a.drag_copy)), a.ops_waiting())
            })
            .unwrap_or((None, false));
            // 꺼내기를 맡겼으면 그 작업을 보낸다. 끌기를 시작하면서 App이 누르는 동안 미뤄 둔 읽기 결과를 적용했을 수도 있다:
            // 이어서 읽을 것이 있으면 보낸다
            if drag.is_some() || queued {
                pump();
            }
            if let Some((paths, copy_only)) = drag {
                self.begin_drag(&paths, e, copy_only);
            }
            self.sync_pointer();
            self.setNeedsDisplay(true);
        }

        /// 떼기: 여럿을 고른 것 가운데 하나를 눌렀다 떼면 이때 그 줄만 남는다. 고른 것이 바뀌므로 입력 뒤처럼 처리한다
        /// (새로 고른 파일의 미리 보기를 읽기 시작한다)
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _e: &NSEvent) {
            with_app(App::mouse_up);
            self.sync_pointer();
            self.after_input();
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            if with_app(|a| a.mouse_moved(&self.ivars().measure, x, y)).unwrap_or(false) {
                self.setNeedsDisplay(true);
            }
            self.sync_pointer();
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _e: &NSEvent) {
            with_app(App::mouse_left);
            self.sync_pointer();
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
            if !self.handle_key(e) {
                let _: () = unsafe { msg_send![super(self), keyDown: e] };
            }
        }

        /// 훑어보기 창이 내용을 댈 곳을 찾는다 (창이 뜰 때 응답 사슬을 거슬러 묻는다): 이 뷰가 댄다
        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts_preview_panel_control(&self, _panel: &AnyObject) -> bool {
            true
        }

        /// 훑어보기 창의 내용(무엇을 보일지)과 키를 이 뷰가 맡는다. 창은 둘 다 약하게 잡는다 (이 뷰가 더 오래 산다).
        /// 맡는 것과 창이 떠 있는 것은 다르다: 창을 닫아도 이 뷰가 계속 맡고 있을 수 있다 (떠 있는지는 look_shown이 창에 묻는다)
        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin_preview_panel_control(&self, panel: &AnyObject) {
            LOOK_ON.set(true);
            let at = LOOK.with(|l| l.borrow().1) as NSInteger;
            // SAFETY: QLPreviewPanel의 속성과 메서드다. 내용을 대는 쪽과 키를 받는 쪽으로 이 뷰를 건다
            unsafe {
                let _: () = msg_send![panel, setDataSource: self];
                let _: () = msg_send![panel, setDelegate: self];
                let _: () = msg_send![panel, reloadData];
                let _: () = msg_send![panel, setCurrentPreviewItemIndex: at];
            }
        }

        /// 훑어보기 창의 내용을 더는 이 뷰가 대지 않는다
        #[unsafe(method(endPreviewPanelControl:))]
        fn end_preview_panel_control(&self, panel: &AnyObject) {
            LOOK_ON.set(false);
            // SAFETY: 위에서 건 것을 푼다
            unsafe {
                let _: () = msg_send![panel, setDataSource: None::<&AnyObject>];
                let _: () = msg_send![panel, setDelegate: None::<&AnyObject>];
            }
        }

        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_preview_items(&self, _panel: &AnyObject) -> NSInteger {
            LOOK.with(|l| l.borrow().0.len()) as NSInteger
        }

        /// 훑어보기 창에 보일 index번째 항목: 파일 주소 (NSURL은 훑어보기 항목이 될 수 있다)
        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn preview_item(&self, _panel: &AnyObject, index: NSInteger) -> Option<Retained<NSURL>> {
            let path = LOOK.with(|l| usize::try_from(index).ok().and_then(|i| l.borrow().0.get(i).cloned()));
            path.map(|p| NSURL::fileURLWithPath(&NSString::from_str(&p)))
        }

        /// 훑어보기 창이 앞에 있을 때 누른 키: 목록이 받게 넘긴다 (화살표로 고른 것이 바뀌면 창도 따라간다).
        /// Esc는 넘기지 않는다: 창이 받아 스스로 닫는다. 목록이 받지 않은 키는 창이 받는다
        #[unsafe(method(previewPanel:handleEvent:))]
        fn preview_handle_event(&self, _panel: &AnyObject, e: &NSEvent) -> bool {
            e.r#type() == NSEventType::KeyDown && !matches!(key_of(e.keyCode()), Some(Key::Esc)) && self.handle_key(e)
        }

        /// 편집 메뉴의 모두 선택(⌘A): 목록에 키 입력이 있을 때 모든 줄을 고른다
        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            with_app(App::select_all);
            self.after_input();
        }

        /// 편집 메뉴의 복사하기(⌘C): 키 입력이 목록에 있을 때 고른 항목들을 클립보드에 담는다 (입력 상자에서는 입력 상자가 받는다)
        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            with_app(App::copy_selected);
            self.after_input();
        }

        /// 편집 메뉴의 오려두기(⌘X): 고른 항목들을 잘라 둔다. 붙여넣을 때 옮겨지고, 그 전까지 파일은 그대로다
        #[unsafe(method(cut:))]
        fn cut(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            with_app(App::cut_selected);
            self.after_input();
        }

        /// 편집 메뉴의 붙여넣기(⌘V): 클립보드의 파일들을 지금 칸의 폴더로 복사한다 (잘라 둔 것이면 옮긴다). 누르고 있어도 한 번만 한다
        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            if held_key(self.mtm()) {
                return;
            }
            with_app(|a| a.ask_paste(false));
            self.after_input();
        }

        /// 편집 메뉴의 실행 취소(⌘Z): 키 입력이 목록에 있을 때 가장 나중에 한 파일 작업(이름 바꾸기, 새 폴더, 휴지통, 복사)을 되돌린다.
        /// 이름이나 경로, 검색어를 치는 중에 여기까지 오면(입력 상자가 직접 받지 않으면) 파일 작업이 아니라 친 글자를 되돌린다:
        /// 이 뷰가 받지 않았을 때 창이 하던 대로, 창의 되돌리기 관리자에 넘긴다
        #[unsafe(method(undo:))]
        fn undo(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            if with_app(|a| a.edit != crate::app::Editing::None || a.find_wants_keys()).unwrap_or(false) {
                if let Some(typed) = self.undoManager()
                    && typed.canUndo()
                {
                    typed.undo();
                }
                return;
            }
            // 누르고 있어도 한 번에 하나만 되돌린다
            if held_key(self.mtm()) {
                return;
            }
            with_app(App::undo);
            self.after_input();
        }

        /// 편집 메뉴의 찾기(⌘F): 검색을 열고 키보드를 입력 칸에 둔다
        #[unsafe(method(sakaduFind:))]
        fn sakadu_find(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            with_app(App::open_find);
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
        fn do_command(&self, control: &NSControl, _text_view: &NSTextView, command: Sel) -> Bool {
            if self.is_search(control as *const NSControl as *const AnyObject) {
                return Bool::new(self.search_command(command));
            }
            if self.is_commands(control as *const NSControl as *const AnyObject) {
                return Bool::new(self.commands_command(command));
            }
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

        /// 다른 곳을 눌러 입력을 마치면 취소. 검색 입력 칸은 키보드만 떠나고 검색은 그대로다
        #[unsafe(method(controlTextDidEndEditing:))]
        fn did_end_editing(&self, n: &NSNotification) {
            if self.is_search_note(n) {
                self.ivars().search_keys.set(false);
                if !SYNCING.get() {
                    with_app(App::find_blur);
                    self.setNeedsDisplay(true);
                }
                return;
            }
            // 명령 메뉴의 입력 칸은 판을 눌러도 메뉴가 그대로다 (맞출 때 키보드를 다시 준다). 닫는 것은 Esc·판 밖 누르기·⌘K
            if self.is_commands_note(n) {
                return;
            }
            // 상자를 맞추는 중(띄우기, 옮기기, 감추기)에 맥이 편집을 끝냈다 다시 시작하는 것은 사용자가 입력을 그만둔 것이 아니다
            if !SYNCING.get() && with_app(App::end_editing).unwrap_or(false) {
                self.after_input();
            }
        }

        /// 검색 입력 칸의 글이 바뀌었다: 글자마다 찾는다 (한글은 조합 중인 글자까지). 명령 메뉴의 입력 칸이면 줄을 다시 고른다
        #[unsafe(method(controlTextDidChange:))]
        fn did_change(&self, n: &NSNotification) {
            if self.is_commands_note(n) {
                let Some(field) = self.ivars().commands.get() else { return };
                let text = field.currentEditor().map_or_else(|| field.stringValue().to_string(), |e| e.string().to_string());
                with_app(|a| a.cmd_typed(&text));
                // 줄 수에 따라 판 높이가 바뀌면 창이 낮을 때 판이 옮겨 간다: 입력 칸도 따라 옮긴다
                self.sync_field();
                self.setNeedsDisplay(true);
                return;
            }
            if !self.is_search_note(n) {
                return;
            }
            let Some(field) = self.ivars().search.get() else { return };
            // 조합 중인 글자까지 든 편집기의 글을 읽는다 (상자의 값을 읽으면 편집이 확정되어 조합이 끊길 수 있다)
            let text = field.currentEditor().map_or_else(|| field.stringValue().to_string(), |e| e.string().to_string());
            with_app(|a| {
                a.open_find();
                a.find_typed(&text);
            });
            self.ivars().search_keys.set(true);
            pump();
            self.setNeedsDisplay(true);
        }
    }

    unsafe impl NSTextFieldDelegate for SakaduView {}

    unsafe impl NSDraggingSource for SakaduView {
        /// 끌어 낸 항목을 받는 쪽이 할 수 있는 일: 복사하거나 옮기기. 지우기는 내주지 않는다 (Dock의 휴지통에 놓아도 지워지지 않는다).
        /// 압축 파일에서 꺼낸 것은 복사만 (Windows와 같다)
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operations(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            if DRAG_COPY.get() {
                return NSDragOperation::Copy;
            }
            NSDragOperation(NSDragOperation::Copy.0 | NSDragOperation::Move.0 | NSDragOperation::Generic.0)
        }

        /// 끌어 낸 것의 끌기가 끝났다. 어딘가에 놓였으면(다른 앱에 놓았으면 그 앱이 옮기거나 복사했다) 보이는 칸을 다시 읽는다.
        /// 놓이지 않고 취소됐어도, 끄는 동안 미뤄 둔 읽기 결과와 다시 읽기는 여기서 한다 (창 밖에서 취소되면 다른 알림이 오지 않는다)
        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn drag_ended(&self, _session: &NSDraggingSession, _point: NSPoint, operation: NSDragOperation) {
            DRAG_COPY.set(false);
            if operation != NSDragOperation::None {
                with_app(App::refresh_shown);
            }
            with_app(App::drag_exit);
            self.after_input();
        }
    }

    unsafe impl NSDraggingDestination for SakaduView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.drag_over(sender)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.drag_over(sender)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            with_app(App::drag_exit);
            // 끌려 온 것이 위에 있는 동안 미뤄 둔 다시 읽기가 있으면 읽는다
            pump();
            self.setNeedsDisplay(true);
        }

        /// 놓았다: 그 자리의 폴더로 옮기거나 복사하기 시작한다
        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            match drop_kind(sender.draggingSourceOperationMask()) {
                Some(how) => {
                    let paths = board_paths(&sender.draggingPasteboard());
                    let p = self.convertPoint_fromView(sender.draggingLocation(), None);
                    let started = with_app(|a| a.drop_paths(&self.ivars().measure, p.x as f32, p.y as f32, paths, how)).unwrap_or(false);
                    self.after_input();
                    started
                }
                // 복사도 옮기기도 허락되지 않은 끌기(⌥⌘: 가상본 만들기)는 받지 않는다
                None => {
                    self.refuse_drag();
                    false
                }
            }
        }

        #[unsafe(method(draggingEnded:))]
        fn dragging_ended(&self, _sender: &ProtocolObject<dyn NSDraggingInfo>) {
            with_app(App::drag_exit);
            pump();
            SAME_VOLUME.with(|v| v.borrow_mut().take());
            self.setNeedsDisplay(true);
        }
    }
);

impl SakaduView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let measure = Gfx::bitmap(8.0, 8.0, 1.0).expect("글자 폭을 잴 그림판");
        let this = Self::alloc(mtm).set_ivars(ViewIvars {
            measure,
            field: OnceCell::new(),
            field_pane: std::cell::Cell::new(None),
            search: OnceCell::new(),
            search_keys: std::cell::Cell::new(false),
            commands: OnceCell::new(),
            pointer: std::cell::Cell::new(crate::app::Pointer::Arrow),
            look_view: OnceCell::new(),
            look_item: RefCell::new(None),
        });
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        // 입력 상자와 검색 입력 칸: 테두리·바탕 없이 (바탕과 강조 색 테두리는 App이 그린다)
        let plain_field = || {
            let field = NSTextField::initWithFrame(NSTextField::alloc(mtm), NSRect::ZERO);
            field.setBezeled(false);
            field.setBordered(false);
            field.setDrawsBackground(false);
            field.setFocusRingType(NSFocusRingType::None);
            field.setFont(Some(&NSFont::systemFontOfSize(13.0)));
            field.setHidden(true);
            unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*view))) };
            view.addSubview(&field);
            field
        };
        let field = plain_field();
        let search = plain_field();
        let _ = view.ivars().search.set(search);
        let commands = plain_field();
        commands.setPlaceholderString(Some(&NSString::from_str(crate::app::CMD_PLACEHOLDER)));
        let _ = view.ivars().commands.set(commands);
        // 끌려 오는 파일을 받는다 (Finder, 다른 앱, 사카두의 다른 칸)
        view.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
        let _ = view.ivars().field.set(field);
        // 마우스 올림: 보이는 곳 전체, 창이 앞에 있을 때
        let opts = NSTrackingAreaOptions::MouseMoved | NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveInKeyWindow | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(), NSRect::ZERO, opts, Some(&view), None) };
        view.addTrackingArea(&area);
        view
    }

    /// 마우스 모양을 App이 원하는 것으로 (열·칸 경계 위에서는 끄는 방향의 화살표). 바뀔 때만 정한다
    fn sync_pointer(&self) {
        let want = with_app(|a| a.pointer()).unwrap_or(crate::app::Pointer::Arrow);
        if self.ivars().pointer.replace(want) == want {
            return;
        }
        // 맥 13에도 있는 모양을 쓴다 (새 이름 columnResizeCursor는 15부터)
        #[allow(deprecated)]
        let cursor = match want {
            crate::app::Pointer::Arrow => NSCursor::arrowCursor(),
            crate::app::Pointer::LeftRight => NSCursor::resizeLeftRightCursor(),
            crate::app::Pointer::UpDown => NSCursor::resizeUpDownCursor(),
        };
        cursor.set();
    }

    /// 이벤트 위치 (뷰 좌표, 왼쪽 위가 0)
    fn point(&self, e: &NSEvent) -> (f32, f32) {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        (p.x as f32, p.y as f32)
    }

    /// 고른 항목들을 끌어 낸다: 파일 주소로 내주므로 Finder나 다른 앱, 사카두의 다른 칸에 놓을 수 있다.
    /// 끌리는 그림은 그 파일의 아이콘이다 (여럿이면 조금씩 어긋나게 겹친다). copy_only면 받는 쪽에 복사만 허락한다
    /// (압축 파일에서 꺼낸 임시 복사본)
    fn begin_drag(&self, paths: &[String], event: &NSEvent, copy_only: bool) {
        DRAG_COPY.set(copy_only);
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        let items: Vec<Retained<NSDraggingItem>> = paths
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let path = NSString::from_str(path);
                let url: Retained<ProtocolObject<dyn NSPasteboardWriting>> = ProtocolObject::from_retained(NSURL::fileURLWithPath(&path));
                let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), &url);
                let icon = NSWorkspace::sharedWorkspace().iconForFile(&path);
                let shift = i.min(6) as f64 * 3.0;
                let frame = NSRect::new(NSPoint::new(at.x - 16.0 + shift, at.y - 16.0 + shift), NSSize::new(32.0, 32.0));
                let picture: &objc2::runtime::AnyObject = &icon;
                unsafe { item.setDraggingFrame_contents(frame, Some(picture)) };
                item
            })
            .collect();
        if !items.is_empty() {
            self.beginDraggingSessionWithItems_event_source(&NSArray::from_retained_slice(&items), event, ProtocolObject::from_ref(self));
        }
    }

    /// 끌려 온 것이 지금 어디에 놓일지 App에 묻고(그 자리를 강조한다), 놓으면 무엇을 할지 돌려준다:
    /// 복사면 마우스에 더하기 표시가 붙고, 놓을 수 없는 곳이면 아무것도 하지 않는다
    fn drag_over(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        // 복사도 옮기기도 허락되지 않은 끌기(⌥⌘: 가상본 만들기)는 받지 않는다: 놓일 곳도 강조하지 않는다
        let Some(how) = drop_kind(sender.draggingSourceOperationMask()) else {
            self.refuse_drag();
            return NSDragOperation::None;
        };
        let paths = board_paths(&sender.draggingPasteboard());
        let p = self.convertPoint_fromView(sender.draggingLocation(), None);
        let dir = with_app(|a| a.drag_over(&self.ivars().measure, p.x as f32, p.y as f32, &paths)).flatten();
        // 놓일 곳이 다른 칸으로 옮겨 갔으면, 앞의 칸에서 미뤄 둔 다시 읽기가 생길 수 있다
        pump();
        self.setNeedsDisplay(true);
        let (Some(dir), Some(first)) = (dir, paths.first()) else { return NSDragOperation::None };
        match how {
            Paste::Copy => NSDragOperation::Copy,
            Paste::Drop if !same_volume(first, &dir) => NSDragOperation::Copy,
            _ => NSDragOperation::Move,
        }
    }

    /// 받지 않는 끌기: 앞서 강조해 둔 놓일 곳이 있으면 지운다 (끄는 중에 누른 키가 바뀌면 받던 끌기가 받지 않는 끌기가 된다)
    fn refuse_drag(&self) {
        with_app(App::drag_exit);
        // 끌려 온 것이 위에 있는 동안 미뤄 둔 다시 읽기가 있으면 읽는다
        pump();
        self.setNeedsDisplay(true);
    }

    /// 키 하나를 App에 넘긴다 (이 뷰가 받은 키, 그리고 훑어보기 창이 넘겨준 키). 받았으면 true
    fn handle_key(&self, e: &NSEvent) -> bool {
        let key = key_of(e.keyCode());
        // 파일을 만들거나 고치는 키, 열고 닫는 키, 단축키 표의 동작은 누르고 있어도 한 번만 받는다
        if e.isARepeat() && key.is_some_and(|k| !with_app(|a| a.key_repeats(k, mods(e))).unwrap_or(false)) {
            return true;
        }
        // 스페이스·⌘Y가 열지 닫을지는 훑어보기 창이 지금 떠 있는지로 정한다: 창에 물어 App에 맞춰 둔다
        let shown = key.is_some() && look_shown();
        let handled = key.is_some_and(|k| {
            with_app(|a| {
                a.look_open = shown;
                a.key(k, mods(e))
            })
            .unwrap_or(false)
        });
        if handled {
            self.after_input();
        }
        handled
    }

    /// obj가 검색 입력 칸인가
    fn is_search(&self, obj: *const AnyObject) -> bool {
        self.ivars().search.get().is_some_and(|f| std::ptr::eq(Retained::as_ptr(f) as *const AnyObject, obj))
    }

    /// 알림이 검색 입력 칸에서 왔는가
    fn is_search_note(&self, n: &NSNotification) -> bool {
        n.object().is_some_and(|o| self.is_search(Retained::as_ptr(&o)))
    }

    /// obj가 명령 메뉴의 입력 칸인가
    fn is_commands(&self, obj: *const AnyObject) -> bool {
        self.ivars().commands.get().is_some_and(|f| std::ptr::eq(Retained::as_ptr(f) as *const AnyObject, obj))
    }

    /// 알림이 명령 메뉴의 입력 칸에서 왔는가
    fn is_commands_note(&self, n: &NSNotification) -> bool {
        n.object().is_some_and(|o| self.is_commands(Retained::as_ptr(&o)))
    }

    /// 명령 메뉴 입력 칸에서 누른 키: ↑↓ 줄 고르기, Page Up/Down, Return 열기(⌘Return 새 탭, ⌥Return 옆 칸), Esc 닫기, Tab은 삼킨다.
    /// 받았으면 true (나머지는 글 상자가 한다)
    fn commands_command(&self, command: Sel) -> bool {
        let event = NSApplication::sharedApplication(self.mtm()).currentEvent();
        // ⌘Return은 글 상자에 정해진 일이 없어 noop:으로 온다: 지금 누른 키가 Return인지 본다
        let returned = event.as_ref().is_some_and(|e| e.r#type() == NSEventType::KeyDown && matches!(e.keyCode(), 0x24 | 0x4C));
        let m = event.as_ref().filter(|e| e.r#type() == NSEventType::KeyDown).map(|e| mods(e)).unwrap_or_default();
        let key = if command == sel!(moveUp:) {
            Key::Up
        } else if command == sel!(moveDown:) {
            Key::Down
        } else if command == sel!(scrollPageUp:) || command == sel!(pageUp:) {
            Key::PageUp
        } else if command == sel!(scrollPageDown:) || command == sel!(pageDown:) {
            Key::PageDown
        } else if command == sel!(insertNewline:) || command == sel!(insertNewlineIgnoringFieldEditor:) || (command == sel!(noop:) && returned) {
            Key::Return
        } else if command == sel!(cancelOperation:) {
            Key::Esc
        } else if command == sel!(insertTab:) || command == sel!(insertBacktab:) {
            Key::Tab
        } else {
            return false;
        };
        let handled = with_app(|a| a.cmd_key(key, m)).unwrap_or(false);
        if handled {
            self.after_input();
        }
        handled
    }

    /// 검색 입력 칸에서 누른 키: ↑↓ 결과 고르기, Return 열기, ⌘Return 칸에서 보기, Esc 닫기. 받았으면 true (나머지는 글 상자가 한다)
    fn search_command(&self, command: Sel) -> bool {
        let event = NSApplication::sharedApplication(self.mtm()).currentEvent();
        let cmd = event.as_ref().is_some_and(|e| e.modifierFlags().contains(NSEventModifierFlags::Command));
        // ⌘Return은 글 상자에 정해진 일이 없어 noop:으로 온다: 지금 누른 키가 Return인지 본다
        let returned = event.as_ref().is_some_and(|e| e.r#type() == NSEventType::KeyDown && matches!(e.keyCode(), 0x24 | 0x4C));
        let key = if command == sel!(moveUp:) {
            Key::Up
        } else if command == sel!(moveDown:) {
            Key::Down
        } else if command == sel!(scrollPageUp:) || command == sel!(pageUp:) {
            Key::PageUp
        } else if command == sel!(scrollPageDown:) || command == sel!(pageDown:) {
            Key::PageDown
        } else if command == sel!(insertNewline:) || (command == sel!(noop:) && returned) {
            Key::Return
        } else if command == sel!(cancelOperation:) {
            Key::Esc
        } else {
            return false;
        };
        let handled = with_app(|a| {
            // 메뉴가 열려 있으면 메뉴가 받는다 (검색 입력 칸에 키보드를 둔 채 도구 막대 메뉴를 열 수 있다)
            if a.menu_shown().is_some() {
                return a.key(key, Mods::default());
            }
            a.open_find();
            a.find_key(key, cmd)
        })
        .unwrap_or(false);
        if handled {
            self.after_input();
        }
        handled
    }

    /// 입력 뒤: 쌓인 폴더 읽기를 뒷줄로 보내고, 열 파일을 열고, 제목을 맞추고, 다시 그린다
    fn after_input(&self) {
        pump();
        // 훑어보기 창을 열거나 닫아 달라고 했으면 (스페이스, ⌘Y). App을 빌리지 않은 채로 한다: 창이 열리며 이 뷰에 묻는다
        if let Some(open) = with_app(|a| a.look_request.take()).flatten() {
            show_look(open);
        }
        sync_look();
        if let (Some(w), Some(title)) = (self.window(), with_app(|a| a.title())) {
            w.setTitle(&NSString::from_str(&title));
        }
        self.sync_field();
        self.setNeedsDisplay(true);
        settings::refresh();
    }

    /// 입력 상자를 App 상태에 맞춘다: 입력 중이면 그 자리(칸의 경로 자리, 사이드바의 공간 이름 자리, 목록 줄의 이름 자리)에 보이고 키 입력을 받는다
    fn sync_field(&self) {
        SYNCING.set(true);
        self.place_field();
        self.place_search();
        // 명령 메뉴의 입력 칸을 마지막에: 떠 있는 동안은 이 칸이 키보드를 갖는다
        self.place_commands();
        SYNCING.set(false);
        self.place_look_view();
    }

    /// 미리 보기 창 본문의 Quick Look 보기를 App에 맞춘다: 보여야 하면 그 자리에 그 파일로 보이고, 아니면 감추고 파일을 놓는다
    /// (감춘 동영상이 계속 소리 내지 않게). 처음 보여야 할 때 만들고, 만들 수 없으면 App에 알린다 (그때부터 큰 썸네일로 보인다)
    fn place_look_view(&self) {
        let want = with_app(|a| a.look_view()).flatten();
        let ivars = self.ivars();
        let Some((r, path, modified)) = want else {
            if let Some(Some(v)) = ivars.look_view.get()
                && !v.isHidden()
            {
                v.setHidden(true);
                // SAFETY: QLPreviewView의 previewItem 속성 (nil이면 보던 것을 놓는다)
                let _: () = unsafe { msg_send![&**v, setPreviewItem: None::<&AnyObject>] };
                ivars.look_item.replace(None);
            }
            return;
        };
        let Some(v) = ivars.look_view.get_or_init(|| self.make_look_view()) else {
            with_app(App::look_view_missing);
            pump();
            self.setNeedsDisplay(true);
            return;
        };
        v.setFrame(NSRect::new(NSPoint::new(r.l as f64, r.t as f64), NSSize::new(r.w().max(0.0) as f64, r.h().max(0.0) as f64)));
        let item = (path, modified);
        if ivars.look_item.borrow().as_ref() != Some(&item) {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&item.0));
            // SAFETY: QLPreviewView의 previewItem 속성. NSURL은 QLPreviewItem을 따른다
            let _: () = unsafe { msg_send![&**v, setPreviewItem: &*url] };
            ivars.look_item.replace(Some(item));
        }
        v.setHidden(false);
    }

    /// 미리 보기 창 본문의 Quick Look 보기를 만든다 (Quartz를 열어 QLPreviewView를 찾는다). 만들 수 없으면 None
    fn make_look_view(&self) -> Option<Retained<NSView>> {
        look_class()?;
        let Some(cls) = AnyClass::get(c"QLPreviewView") else {
            sakadu::diag::log(|| "quick look: no QLPreviewView".to_string());
            return None;
        };
        // SAFETY: alloc 뒤의 지정 초기화 메서드 -[QLPreviewView initWithFrame:style:] (QLPreviewViewStyleNormal = 0)
        let view: Option<Retained<NSView>> = unsafe {
            let fresh: Allocated<NSView> = msg_send![cls, alloc];
            msg_send![fresh, initWithFrame: NSRect::ZERO, style: 0usize]
        };
        let view = view?;
        view.setHidden(true);
        self.addSubview(&view);
        Some(view)
    }

    /// 명령 메뉴의 입력 칸을 App에 맞춘다: 명령 메뉴가 떠 있으면 판의 입력 자리에 보이고 키보드를 받는다. 닫히면 감추고 키보드를 뷰에 돌려준다
    /// (검색 입력 칸이 키보드를 원하면 다음에 맞출 때 그 칸이 가져간다)
    fn place_commands(&self) {
        let Some(field) = self.ivars().commands.get() else { return };
        let typing = field.currentEditor().is_some();
        let Some((rect, text, color)) = with_app(|a| (a.cmd_field_rect(), a.cmd_text().to_string(), a.theme.text)) else { return };
        let window = self.window();
        match rect {
            Some(r) => {
                let h = 18.0;
                field.setFrame(NSRect::new(NSPoint::new(r.l as f64, (r.t + (r.h() - h) / 2.0) as f64), NSSize::new(r.w().max(0.0) as f64, h as f64)));
                field.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(color.r as f64, color.g as f64, color.b as f64, 1.0)));
                // 열 때마다 빈 칸으로 (App이 글을 비워 둔다)
                if field.isHidden() {
                    field.setStringValue(&NSString::from_str(&text));
                    field.setHidden(false);
                }
                if !typing && let Some(w) = &window {
                    w.makeFirstResponder(Some(field));
                }
            }
            None if !field.isHidden() => {
                field.setHidden(true);
                if typing && let Some(w) = &window {
                    w.makeFirstResponder(Some(self));
                }
            }
            None => {}
        }
    }

    /// 검색 입력 칸을 App에 맞춘다: 검색이 열려 있으면 도구 막대의 늘어난 칸에 보이고, 검색이 키보드를 원하면 키보드를 준다.
    /// 칸에서 일하라고 키보드를 내주었으면(결과를 칸에서 열었을 때) 뷰가 키보드를 받는다
    fn place_search(&self) {
        let Some(field) = self.ivars().search.get() else { return };
        // 지난번에 맞춘 뒤 사용자가 입력 칸을 눌러 키보드를 가져갔으면 App에 알린다
        let typing = field.currentEditor().is_some();
        if typing && !self.ivars().search_keys.get() {
            with_app(App::open_find);
        }
        let Some((rect, text, wants, color)) = with_app(|a| (a.find_field_rect(), a.find_text().to_string(), a.find_wants_keys(), a.theme.text)) else { return };
        let window = self.window();
        match rect {
            Some(r) => {
                let h = 18.0;
                field.setFrame(NSRect::new(NSPoint::new(r.l as f64, (r.t + (r.h() - h) / 2.0) as f64), NSSize::new(r.w().max(0.0) as f64, h as f64)));
                field.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(color.r as f64, color.g as f64, color.b as f64, 1.0)));
                if field.isHidden() {
                    field.setStringValue(&NSString::from_str(&text));
                    field.setHidden(false);
                } else if !typing && field.stringValue().to_string() != text {
                    // 앱이 검색어를 바꿨다 (명령 메뉴의 '디스크에서 찾기')
                    field.setStringValue(&NSString::from_str(&text));
                }
                // 키보드를 주면 글 전체가 골라진다: 새로 치면 바뀌고, 이어 쓰려면 →
                if wants && !typing {
                    if let Some(w) = &window {
                        w.makeFirstResponder(Some(field));
                    }
                } else if !wants && typing && let Some(w) = &window {
                    w.makeFirstResponder(Some(self));
                }
            }
            None if !field.isHidden() => {
                field.setHidden(true);
                if typing && let Some(w) = &window {
                    w.makeFirstResponder(Some(self));
                }
            }
            None => {}
        }
        self.ivars().search_keys.set(field.currentEditor().is_some());
    }

    fn place_field(&self) {
        let Some(field) = self.ivars().field.get() else { return };
        let Some((rect, text, color, pane, pad, select)) = with_app(|a| {
            // 이름을 고치던 항목이 그 자리에 없으면 여기서 고치기가 끝난다
            a.sync_edit();
            (a.address_rect(), a.address_text(), a.theme.text, a.edit_key(), a.address_pad(), a.address_select())
        }) else {
            return;
        };
        match rect {
            Some(r) => {
                // 한 줄 글자가 세로 가운데에 오게 (시스템 13pt 글꼴의 줄 높이 약 17)
                let h = 18.0;
                let frame = NSRect::new(NSPoint::new((r.l + pad) as f64, (r.t + (r.h() - h) / 2.0) as f64), NSSize::new((r.w() - pad * 2.0).max(0.0) as f64, h as f64));
                field.setFrame(frame);
                field.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(color.r as f64, color.g as f64, color.b as f64, 1.0)));
                // 처음 열 때와, 입력 중에 다른 입력(다른 칸의 경로, 공간 이름, 다른 항목의 이름)으로 옮겼을 때 그 글로 다시 채운다
                if field.isHidden() || self.ivars().field_pane.get() != pane {
                    self.ivars().field_pane.set(pane);
                    field.setStringValue(&NSString::from_str(&text));
                    field.setHidden(false);
                    // 키를 받게 하면 글 전체가 골라진다. selectText:를 따로 부르지 않는다: 이미 편집 중인 상자에 부르면 편집을 한 번
                    // 끝냈다가 다시 시작해서, 그 '끝남' 알림이 막 연 입력을 닫았다 (창 시험에서 이름 상자가 뜨자마자 닫혔다)
                    if let Some(w) = self.window() {
                        w.makeFirstResponder(Some(field));
                    }
                    // 항목 이름은 확장자 앞까지만 골라 둔다 (바로 치면 이름만 바뀐다)
                    if let (Some(n), Some(editor)) = (select, field.currentEditor()) {
                        editor.setSelectedRange(NSRange::new(0, n));
                    }
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

/// 지금 처리하는 것이 키를 누르고 있어서 되풀이된 키 입력인지 (메뉴 단축키로 온 붙여넣기·실행 취소를 한 번만 받으려고 본다)
fn held_key(mtm: MainThreadMarker) -> bool {
    NSApplication::sharedApplication(mtm).currentEvent().is_some_and(|e| e.r#type() == NSEventType::KeyDown && e.isARepeat())
}

/// 파일들을 클립보드에 파일 주소로 담는다 (Finder나 다른 앱에 붙여넣을 수 있다).
/// cut이면 잘라 둔 것으로 기억한다: 다른 앱에는 보통의 복사로 보이고, 사카두에서 붙여넣을 때만 옮긴다
fn write_clipboard(paths: &[String], cut: bool) {
    let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
        paths.iter().map(|p| ProtocolObject::from_retained(NSURL::fileURLWithPath(&NSString::from_str(p)))).collect();
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    let written = board.writeObjects(&NSArray::from_retained_slice(&items));
    if !written {
        sakadu::diag::log(|| "clipboard: write failed".to_string());
    }
    CUT.set((cut && written).then(|| board.changeCount()));
}

/// 클립보드에 담긴 파일들의 경로 (Finder에서 복사한 것도 읽는다)와, 그것이 사카두에서 잘라 둔 것 그대로이면 그때의
/// 클립보드 번호. 파일이 아닌 것(글자, 그림)만 있으면 비어 있다
fn read_clipboard() -> (Vec<String>, Option<isize>) {
    let board = NSPasteboard::generalPasteboard();
    let cut = CUT.get().filter(|count| *count == board.changeCount());
    (board_paths(&board), cut)
}

/// 끌어다 놓을 때 누른 키에 따라 무엇을 할지. 맥은 누른 키에 맞춰 끌어 낸 쪽이 허락한 일을 줄여서 알려 준다:
/// ⌥를 누르면 복사만 남고, ⌘를 누르면 옮기기만 남는다. 아무 키도 누르지 않았으면 디스크에 따라 정한다
/// (같은 디스크면 옮기고 다른 디스크면 복사).
/// 복사도 옮기기도 남지 않았으면 None: 받지 않는다. ⌥⌘를 누르면 가상본 만들기만 남는데, 그것은 사카두에 없는 일이다
/// (그때 옮겨 버리면 가상본을 바랐는데 원본이 자리를 뜬다)
fn drop_kind(allowed: NSDragOperation) -> Option<Paste> {
    let copy = allowed.0 & NSDragOperation::Copy.0 != 0;
    let moving = allowed.0 & (NSDragOperation::Move.0 | NSDragOperation::Generic.0) != 0;
    match (copy, moving) {
        (true, true) => Some(Paste::Drop),
        (true, false) => Some(Paste::Copy),
        (false, true) => Some(Paste::Move),
        (false, false) => None,
    }
}

/// 끌려 온 항목과 받는 폴더가 같은 디스크에 있는지 (마우스에 붙일 표시를 정할 때만 쓴다. 실제로 옮길지 복사할지는
/// 놓은 뒤 실행할 때 정해진다). 끄는 동안 같은 것을 되풀이해 묻지 않게 적어 둔다. 알 수 없으면 같다고 본다
fn same_volume(item: &str, dir: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    SAME_VOLUME.with(|cell| {
        let mut known = cell.borrow_mut();
        if let Some((i, d, same)) = known.as_ref()
            && i == item
            && d == dir
        {
            return *same;
        }
        let same = match (std::fs::symlink_metadata(item), std::fs::metadata(dir)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev(),
            _ => true,
        };
        *known = Some((item.to_string(), dir.to_string(), same));
        same
    })
}

/// 그 클립보드(보통의 클립보드나 끌기의 클립보드, 서비스가 넘긴 판)에 담긴 파일들의 경로
fn board_paths(board: &NSPasteboard) -> Vec<String> {
    let Some(items) = board.pasteboardItems() else { return Vec::new() };
    let mut out = Vec::new();
    for item in items.iter() {
        let Some(text) = item.stringForType(unsafe { NSPasteboardTypeFileURL }) else { continue };
        let Some(url) = NSURL::URLWithString(&text) else { continue };
        out.extend(url_path(url));
    }
    out
}

/// 파일 주소의 경로. Finder는 파일 번호로 된 주소를 줄 때가 있다: 경로로 된 주소로 바꿔 읽는다.
/// 경로는 받은 글자 그대로 쓴다 (사용자가 친 경로처럼 다듬으면 이름 끝의 공백이나 따옴표가 잘려 다른 파일이 된다).
/// 다만 한글은 모아 쓴 꼴(NFC)로 바꾼다: 파일 주소의 경로는 자모를 풀어 쓴 꼴(NFD)로 와서, 그대로 복사하면 복사본의
/// 이름이 풀어 쓴 꼴로 생기고 목록에서 엉뚱한 자리에 선다 (창 시험에서 찾았다). 맥의 디스크는 두 꼴을 같은 이름으로 찾는다
fn url_path(url: Retained<NSURL>) -> Option<String> {
    let url = url.filePathURL().unwrap_or(url);
    url.path().map(|path| path.precomposedStringWithCanonicalMapping().to_string())
}

/// Finder가 열어 달라고 보낸 것 ('다음으로 열기', 서비스의 'Sakadu에서 열기', Dock 아이콘에 놓기): 지금 칸에 새 탭으로 연다
/// (있는지는 뒷줄에서 확인한다). 창을 앞으로 가져온다 (대기 중에 숨겨 두었으면 다시 띄운다).
/// 앱이 아직 뜨는 중이면 창을 만든 뒤에 연다
fn open_from_finder(mtm: MainThreadMarker, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    if APP.with(|a| a.borrow().is_none()) {
        OPEN_LATER.with(|l| l.borrow_mut().extend(paths));
        return;
    }
    with_app(|a| a.open_external(paths));
    standby::show_window(mtm);
    after_menu();
}

/// 작업 공간과 화면 상태(세션)를 저장한다 (끝낼 때, 대기 중에 창을 숨길 때)
fn save_state() {
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

/// 사이드바 줄이 미끄러지는 동안(그리고 끄는 줄을 목록 끝에 대고 있어 자동 스크롤하는 동안): 한 틀(약 1/60초) 뒤에 다시 그려 달라고 한다.
/// 이미 부탁해 두었으면 그대로 둔다 (그릴 때마다 부탁이 쌓이지 않게). 다 미끄러지면 그리기가 더는 부탁하지 않는다
fn schedule_flow() {
    if FLOW_PENDING.get() {
        return;
    }
    let Ok(when) = DispatchTime::try_from(Duration::from_millis(15)) else { return };
    FLOW_PENDING.set(true);
    let _ = DispatchQueue::main().after(when, || {
        FLOW_PENDING.set(false);
        VIEW.with(|v| {
            if let Some(v) = v.get() {
                v.setNeedsDisplay(true);
            }
        });
    });
}

/// 뒷줄 결과를 받은 뒤 (메인 줄에서): 입력 상자를 App 상태에 맞추고 다시 그려 달라고 한다.
/// 새 폴더의 이름 상자는 입력이 아니라 뒷줄 결과(폴더를 만들고 다시 읽은 것)로 뜨므로, 그리기를 기다리지 않고 여기서 맞춘다
fn redraw() {
    VIEW.with(|v| {
        if let Some(v) = v.get() {
            v.sync_field();
            v.setNeedsDisplay(true);
        }
    });
    sync_look();
    settings::refresh();
}

/// 훑어보기 창의 클래스. Quartz 프레임워크(훑어보기 창이 든 QuickLookUI를 품는다)를 처음 한 번 열어 찾는다.
/// 실행 파일에 링크하지 않고 이렇게 여는 것은, 열지 못하더라도 앱은 뜨게 하려는 것이다
fn look_class() -> Option<&'static AnyClass> {
    LOOK_CLASS.with(|cell| {
        *cell.get_or_init(|| {
            // SAFETY: NUL로 끝나는 고정 경로다. 받은 핸들은 닫지 않는다 (앱이 끝날 때까지 쓴다)
            let lib = unsafe { libc::dlopen(c"/System/Library/Frameworks/Quartz.framework/Quartz".as_ptr(), libc::RTLD_LAZY) };
            if lib.is_null() {
                sakadu::diag::log(|| "quick look: Quartz did not load".to_string());
                return None;
            }
            let cls = AnyClass::get(c"QLPreviewPanel");
            if cls.is_none() {
                sakadu::diag::log(|| "quick look: no QLPreviewPanel".to_string());
            }
            // 이 뷰가 훑어보기 창의 내용과 키를 맡는다고 런타임에도 밝혀 둔다 (그 프로토콜들은 위의 프레임워크를 열어야 생긴다).
            // 창이 그것을 따지지 않으면 하는 일은 없다
            for name in [c"QLPreviewPanelDataSource", c"QLPreviewPanelDelegate"] {
                if let Some(proto) = AnyProtocol::get(name) {
                    // SAFETY: 이미 등록된 클래스에 프로토콜을 더하는 것은 런타임이 허락한다. 그 메서드들은 define_class에 있다
                    unsafe { objc2::ffi::class_addProtocol(SakaduView::class() as *const AnyClass as *mut AnyClass, proto) };
                }
            }
            cls
        })
    })
}

/// 훑어보기 창. create가 false면 이미 만들어진 것만 돌려준다 (없으면 None)
fn look_panel(create: bool) -> Option<Retained<AnyObject>> {
    let cls = look_class()?;
    // SAFETY: QLPreviewPanel의 클래스 메서드들이다 (인자 없이 BOOL과 그 창을 돌려준다)
    unsafe {
        if !create {
            let exists: bool = msg_send![cls, sharedPreviewPanelExists];
            if !exists {
                return None;
            }
        }
        msg_send![cls, sharedPreviewPanel]
    }
}

/// 훑어보기 창이 지금 화면에 떠 있는지. 한 번도 열지 않았으면 프레임워크를 열지 않고 false
fn look_shown() -> bool {
    let Some(cls) = LOOK_CLASS.with(|cell| cell.get().copied().flatten()) else { return false };
    // SAFETY: QLPreviewPanel의 클래스 메서드와 NSWindow의 isVisible이다
    unsafe {
        let exists: bool = msg_send![cls, sharedPreviewPanelExists];
        if !exists {
            return false;
        }
        let panel: Option<Retained<AnyObject>> = msg_send![cls, sharedPreviewPanel];
        panel.is_some_and(|panel| msg_send![&*panel, isVisible])
    }
}

/// 훑어보기 창을 연다(open) 또는 닫는다. 열 때는 보여 줄 것을 먼저 맞춘다: 창이 열리며 곧바로 묻는다
fn show_look(open: bool) {
    if !open {
        if let Some(panel) = look_panel(false) {
            // SAFETY: NSWindow의 orderOut:
            let _: () = unsafe { msg_send![&*panel, orderOut: None::<&AnyObject>] };
        }
        return;
    }
    let Some(items) = with_app(|a| a.look_items()) else { return };
    if items.0.is_empty() {
        return;
    }
    LOOK.with(|l| *l.borrow_mut() = items);
    match look_panel(true) {
        // SAFETY: NSWindow의 makeKeyAndOrderFront:와 QLPreviewPanel의 메서드들
        Some(panel) => unsafe {
            let _: () = msg_send![&*panel, makeKeyAndOrderFront: None::<&AnyObject>];
            // 닫았던 창을 다시 열 때는 이 뷰가 계속 맡고 있어 begin…이 다시 오지 않을 수 있다: 새 내용을 직접 알린다
            if LOOK_ON.get() {
                let at = LOOK.with(|l| l.borrow().1) as NSInteger;
                let _: () = msg_send![&*panel, reloadData];
                let _: () = msg_send![&*panel, setCurrentPreviewItemIndex: at];
            }
        },
        None => {
            with_app(App::look_failed);
            redraw();
        }
    }
}

/// 훑어보기 창이 떠 있고 이 뷰의 것을 보이고 있으면, 보여 줄 것을 App의 지금 선택에 맞춘다. 달라졌을 때만 창에 알린다
/// (그릴 때마다 다시 읽게 하면 깜빡인다). 보여 줄 것이 없어졌으면 창을 닫는다
fn sync_look() {
    // 이 뷰가 맡고 있어도 창이 닫혀 있으면 알리지 않는다 (다시 열 때 show_look이 새로 알린다)
    if !LOOK_ON.get() || !look_shown() {
        return;
    }
    let Some(items) = with_app(|a| a.look_items()) else { return };
    let before = LOOK.with(|l| std::mem::replace(&mut *l.borrow_mut(), items.clone()));
    if before == items {
        return;
    }
    let Some(panel) = look_panel(false) else { return };
    // SAFETY: QLPreviewPanel의 메서드들이다. 창은 이 안에서 곧바로 LOOK을 물을 수 있다 (App은 빌리지 않은 채다)
    unsafe {
        if items.0.is_empty() {
            let _: () = msg_send![&*panel, orderOut: None::<&AnyObject>];
            return;
        }
        if before.0 != items.0 {
            let _: () = msg_send![&*panel, reloadData];
        }
        let _: () = msg_send![&*panel, setCurrentPreviewItemIndex: items.1 as NSInteger];
    }
}

/// 쌓인 폴더 읽기 요청은 뒷줄에서 읽어 메인 줄로 돌려받고, 열어 달라는 파일은 macOS '열기'로 연다
/// ('열기'로 여는 앱은 사카두의 권한을 물려받지 않는다). 미리 보기 카드의 그림과 글도 뒷줄에서 읽는다
fn pump() {
    pump_find();
    // 시작 화면 위치 칸의 디스크 용량: 뒷줄에서 읽는다 (/Volumes의 바로가기를 따라갈 때 디스크를 볼 수 있다)
    if let Some(Some(paths)) = with_app(App::take_volume_ask)
        && !paths.is_empty()
    {
        std::thread::spawn(move || {
            let space = crate::app::volume_space(&paths);
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.set_volumes(space));
                redraw();
            });
        });
    }
    // 디스크 꺼내기와 네트워크 공유 연결: 오래 걸릴 수 있어 뒷줄에서 (연결은 맥의 로그인 창을 기다릴 수도 있다)
    for path in with_app(App::take_ejects).unwrap_or_default() {
        std::thread::spawn(move || {
            let result = crate::volumes::eject(&path);
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.on_ejected(&path, result));
                redraw();
            });
        });
    }
    for probe in with_app(App::take_mount_asks).unwrap_or_default() {
        std::thread::spawn(move || {
            let result = crate::volumes::mount(&probe.text);
            DispatchQueue::main().exec_async(move || {
                // 붙은 디스크를 사이드바에 먼저 넣는다 (맥의 알림보다 먼저 올 수 있다). 그리고 그 자리로 간다
                with_app(|a| {
                    a.on_volumes(places(), None);
                    a.on_mounted(&probe, result);
                });
                pump();
                redraw();
            });
        });
    }
    // Finder에서 열어 달라고 보낸 것: 있는지 뒷줄에서 확인하고(꺼진 네트워크 폴더라도 멈추지 않게) 새 탭으로 연다
    for paths in with_app(App::take_external).unwrap_or_default() {
        std::thread::spawn(move || {
            let results = crate::app::resolve_external(&paths);
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.on_external(results));
                after_menu();
            });
        });
    }
    // 설정·도움말 단축키: 설정 창을 연다 (단축키와 도움말이 거기 있다)
    if with_app(|a| std::mem::take(&mut a.open_settings)) == Some(true)
        && let Some(mtm) = MainThreadMarker::new()
    {
        settings::open(mtm);
    }
    // 클립보드: 담아 달라는 것을 담고, 붙여넣어 달라고 했으면 읽어서 넘긴다 (그러면 아래에서 맡길 작업이 생긴다)
    if let Some((out, cut, wanted, left)) = with_app(|a| (a.clip_out.take(), std::mem::take(&mut a.clip_cut), a.clip_in.take(), a.clip_left.take())) {
        // 잘라 둔 것을 옮기는 묶음이 끝났다: 옮겨 간 것은 클립보드에서 빼고, 옮기지 못한 것만 잘라 둔 채로 남긴다
        // (다른 곳에 다시 붙여넣을 수 있게). 클립보드가 그 묶음을 시작할 때의 그것 그대로일 때만 한다:
        // 그 사이 다른 것이 담겼거나 다른 것을 새로 잘라 뒀으면(번호가 다르다) 건드리지 않는다
        if let Some((mark, left)) = left
            && CUT.get() == Some(mark as isize)
            && NSPasteboard::generalPasteboard().changeCount() == mark as isize
        {
            if left.is_empty() {
                NSPasteboard::generalPasteboard().clearContents();
                CUT.set(None);
            } else {
                write_clipboard(&left, true);
            }
        }
        if let Some(paths) = out {
            write_clipboard(&paths, cut);
        }
        if let Some(force_move) = wanted {
            let (paths, was_cut) = read_clipboard();
            let how = if let Some(mark) = was_cut {
                Paste::Cut(mark as i64)
            } else if force_move {
                Paste::Move
            } else {
                Paste::Copy
            };
            with_app(|a| a.paste_paths(paths, how));
        }
    }
    let Some((reqs, probes, previews, files, ops, reveal)) =
        with_app(|a| (a.take_requests(), a.take_probes(), a.take_previews(), std::mem::take(&mut a.open_files), a.take_ops(), std::mem::take(&mut a.reveal_files)))
    else {
        return;
    };
    run_ops(ops);
    watch_dirs();
    // Finder에서 보기: Finder를 앞으로 불러 그 항목들을 고른다
    if !reveal.is_empty() {
        let urls: Vec<Retained<NSURL>> = reveal.iter().map(|f| NSURL::fileURLWithPath(&NSString::from_str(f))).collect();
        NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&urls));
    }
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
    // 미리 보기 창에 고정한 파일이 그대로 있는지 (꺼진 네트워크 폴더라도 창이 멈추지 않게 뒷줄에서)
    for c in with_app(App::take_pin_checks).unwrap_or_default() {
        std::thread::spawn(move || {
            let found = crate::app::check_pin(&c);
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.on_pin_checked(&c, found));
                // 옮겨졌거나 바뀌었으면 새로 읽는다
                pump();
                redraw();
            });
        });
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
        let watch = WATCH.with(|cell| cell.get().and_then(|w| w.as_ref()).map(crate::watch::Watcher::handle));
        std::thread::spawn(move || {
            // 감시가 걸린 뒤에 읽는다: 감시보다 먼저 읽으면 읽은 뒤부터 감시가 걸리기까지의 변화를 놓친다.
            // 압축 파일 안은 감시하지 않으므로 기다리지 않는다
            if let Some(watch) = watch.as_ref().filter(|_| r.archive.is_none()) {
                watch.wait_watched(&r.dir, WATCH_WAIT);
            }
            let result = crate::app::list_request(&r);
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.on_listed(r.tab, r.generation, result));
                // 읽기 결과가 새 요청을 낼 수 있다 (세션에서 연 폴더가 없으면 홈을 읽는다)
                pump();
                redraw();
            });
        });
    }
    for f in files {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&f));
        if !NSWorkspace::sharedWorkspace().openURL(&url) {
            sakadu::diag::log(|| format!("open failed: {f}"));
        }
    }
    // 시스템 설정 같은 주소 (검색의 '설정에서 허용')
    if let Some(Some(u)) = with_app(|a| a.open_url.take())
        && let Some(url) = NSURL::URLWithString(&NSString::from_str(&u))
        && !NSWorkspace::sharedWorkspace().openURL(&url)
    {
        sakadu::diag::log(|| format!("open failed: {u}"));
    }
}

/// 검색: 색인을 시작해 달라고 했으면 시작하고(색인이 바뀔 때마다 같은 검색어로 다시 찾게 알림을 건다),
/// 쌓인 검색은 뒷줄에서 찾아 메인 줄로 돌려받는다
fn pump_find() {
    if let Some(Some(opts)) = with_app(App::take_index_start) {
        let index = crate::search::Search::start(opts, || {
            DispatchQueue::main().exec_async(|| {
                with_app(App::index_changed);
                pump();
                redraw();
            });
        });
        with_app(|a| a.set_index(index));
    }
    let asks = with_app(|a| {
        a.find_sync();
        a.take_find_asks()
    })
    .unwrap_or_default();
    for ask in asks {
        std::thread::spawn(move || {
            let found = ask.run();
            DispatchQueue::main().exec_async(move || {
                with_app(|a| a.on_found(&ask, found));
                // 결과보다 먼저 누른 Return이 폴더를 열었을 수 있다
                pump();
                if let Some(v) = VIEW.with(|v| v.get().cloned()) {
                    v.sync_field();
                }
                redraw();
            });
        });
    }
}

/// 파일 작업을 뒷줄 하나에 맡긴다. 그 줄이 맡긴 순서대로 하나씩 실행하고(둘이 한꺼번에 파일을 건드리지 않는다),
/// 하나가 끝날 때마다 결과를 메인 줄로 돌려준다. 결과에 따라 폴더를 다시 읽으므로 이어서 pump를 돈다
fn run_ops(ops: Vec<crate::fileops::Op>) {
    if ops.is_empty() {
        return;
    }
    OPS.with(|cell| {
        let tx = cell.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<crate::fileops::Op>();
            std::thread::spawn(move || {
                for op in rx {
                    let result = crate::fileops::run(&op.job);
                    DispatchQueue::main().exec_async(move || {
                        with_app(|a| a.on_op_done(&op, result));
                        pump();
                        redraw();
                    });
                }
            });
            tx
        });
        for op in ops {
            if tx.send(op).is_err() {
                sakadu::diag::log(|| "file ops: worker is gone".to_string());
            }
        }
    });
}

/// 보이는 폴더가 달라졌으면 감시할 폴더를 거기에 맞춘다. 감시하던 폴더가 바뀌면 감시하는 줄이 알려 오고,
/// 메인 줄에서 App에 넘겨 그 폴더를 다시 읽는다
fn watch_dirs() {
    let Some(dirs) = with_app(|a| a.take_watch_dirs()).flatten() else { return };
    WATCH.with(|cell| {
        let watcher = cell.get_or_init(|| {
            crate::watch::Watcher::start(|dir| {
                DispatchQueue::main().exec_async(move || {
                    with_app(|a| a.on_dir_changed(&dir));
                    pump();
                });
            })
        });
        if let Some(w) = watcher {
            w.set(dirs);
        }
    });
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
        // Return과 숫자판의 Enter
        0x24 | 0x4C => Key::Return,
        // 지우기(⌫)
        0x33 => Key::Delete,
        0x31 => Key::Char(' '),
        _ => {
            // 글자·숫자·기호 (자판의 자리. 한글 2벌식도 같은 자리를 쓴다)와 F키
            if let Some(i) = KEY_CHARS.iter().position(|k| k.0 == code) {
                return Some(Key::Char(KEY_CHARS[i].1));
            }
            return F_KEYS.iter().position(|k| *k == code).map(|n| Key::F(n as u8 + 1));
        }
    })
}

/// 맥 자판의 키 번호 ↔ 글자 (US 자판의 자리, Carbon Events.h의 kVK_ANSI_*)
const KEY_CHARS: [(u16, char); 47] = [
    (0x00, 'a'),
    (0x0B, 'b'),
    (0x08, 'c'),
    (0x02, 'd'),
    (0x0E, 'e'),
    (0x03, 'f'),
    (0x05, 'g'),
    (0x04, 'h'),
    (0x22, 'i'),
    (0x26, 'j'),
    (0x28, 'k'),
    (0x25, 'l'),
    (0x2E, 'm'),
    (0x2D, 'n'),
    (0x1F, 'o'),
    (0x23, 'p'),
    (0x0C, 'q'),
    (0x0F, 'r'),
    (0x01, 's'),
    (0x11, 't'),
    (0x20, 'u'),
    (0x09, 'v'),
    (0x0D, 'w'),
    (0x07, 'x'),
    (0x10, 'y'),
    (0x06, 'z'),
    (0x1D, '0'),
    (0x12, '1'),
    (0x13, '2'),
    (0x14, '3'),
    (0x15, '4'),
    (0x17, '5'),
    (0x16, '6'),
    (0x1A, '7'),
    (0x1C, '8'),
    (0x19, '9'),
    (0x1B, '-'),
    (0x18, '='),
    (0x21, '['),
    (0x1E, ']'),
    (0x2A, '\\'),
    (0x29, ';'),
    (0x27, '\''),
    (0x2B, ','),
    (0x2F, '.'),
    (0x2C, '/'),
    (0x32, '`'),
];

/// F1~F20의 키 번호 (kVK_F1 …)
const F_KEYS: [u16; 20] = [0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A];

/// 사이드바 '모든 공간'에 늘 있는 줄: 홈 아래 폴더들과 연결된 디스크 (/Volumes 안. 숨긴 것(타임머신의 .timemachine 따위)은 빼고).
/// 디스크가 붙고 떨어질 때마다 다시 읽는다 (/Volumes만 읽고 디스크 안은 보지 않아 꺼진 서버를 기다리지 않는다)
fn places() -> Vec<(String, String)> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let mut out = vec![("홈".to_string(), home.clone())];
    for (name, sub) in [("데스크탑", "Desktop"), ("문서", "Documents"), ("다운로드", "Downloads")] {
        out.push((name.to_string(), format!("{home}/{sub}")));
    }
    out.push(("응용 프로그램".to_string(), "/Applications".to_string()));
    if let Ok(vols) = std::fs::read_dir("/Volumes") {
        let mut v: Vec<(String, String)> = vols
            .flatten()
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().to_string_lossy().into_owned()))
            .filter(|(name, _)| !name.starts_with('.'))
            .collect();
        v.sort();
        out.extend(v);
    }
    out
}

/// 디스크 알림을 듣는다: 붙었다, 떨어졌다, 이름이 바뀌었다 (맥은 NSWorkspace의 알림 센터로 알린다. 받는 쪽은 앱 위임의 volumesChanged:)
fn watch_volumes(delegate: &Delegate) {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: 맥이 정한 알림 이름들이다
    let names = unsafe { [NSWorkspaceDidMountNotification, NSWorkspaceDidUnmountNotification, NSWorkspaceDidRenameVolumeNotification] };
    for name in names {
        // SAFETY: 앱 위임은 앱이 끝날 때까지 산다 (run이 쥐고 있다). volumesChanged:는 define_class에 있고 알림 하나를 받는다
        unsafe { center.addObserver_selector_name_object(delegate, sel!(volumesChanged:), Some(name), None) };
    }
}

/// 디스크 알림에서 무엇이 일어났는지 읽는다 (자리는 알림에 든 디스크 주소에서). 읽지 못하면 None (사이드바만 다시 읽는다)
fn volume_event(n: &NSNotification) -> Option<VolumeEvent> {
    let info = n.userInfo()?;
    let path_of = |key: &NSString| {
        let url = info.objectForKey(key)?.downcast::<NSURL>().ok()?;
        url.path().map(|p| sakadu::util::normalize(&p.to_string()))
    };
    let name = n.name();
    // SAFETY: 맥이 정한 알림 이름과 키들이다
    unsafe {
        if &*name == NSWorkspaceDidMountNotification {
            path_of(NSWorkspaceVolumeURLKey).map(VolumeEvent::Mounted)
        } else if &*name == NSWorkspaceDidUnmountNotification {
            path_of(NSWorkspaceVolumeURLKey).map(VolumeEvent::Unmounted)
        } else if &*name == NSWorkspaceDidRenameVolumeNotification {
            Some(VolumeEvent::Renamed { from: path_of(NSWorkspaceVolumeOldURLKey)?, to: path_of(NSWorkspaceVolumeURLKey)? })
        } else {
            None
        }
    }
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
            let smoke = SMOKE.with(|s| s.borrow().clone());
            // 로그인 항목으로 열렸는지는 앱이 뜨는 동안에만 읽을 수 있다
            let at_login = smoke.is_none() && standby::launched_at_login();
            let window = open_window(mtm, smoke.clone());
            // 디스크가 붙고 떨어지는 것을 듣는다 (사이드바의 디스크 줄)
            watch_volumes(self);
            // 메뉴 막대의 키는 단축키 표에서 온다: 창(App)을 만든 뒤에 짓는다
            main_menu(mtm, &app);
            apply_theme(mtm);
            window.setDelegate(Some(ProtocolObject::from_ref(self)));
            let _ = self.ivars().window.set(window.clone());
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            // 서비스 메뉴의 'Sakadu에서 열기'를 이 위임이 받는다 (Info.plist의 NSServices)
            let provider: &AnyObject = self;
            // SAFETY: 이 위임에 openInSakadu:userData:error:가 있다 (define_class)
            unsafe { app.setServicesProvider(Some(provider)) };
            // 메뉴 막대 대기(아이콘)와 어디서든 찾기 키
            standby::sync(mtm);
            standby::register_hotkey();
            // 뜨는 동안 Finder가 열어 달라고 보낸 것
            let later = OPEN_LATER.with(|l| std::mem::take(&mut *l.borrow_mut()));
            // 대기를 켜 두었으면 로그인할 때는 창 없이 메뉴 막대 아이콘만으로 시작한다 (Windows의 창 없이 트레이에서 시작과 같다)
            let hidden = with_app(|a| a.starts_hidden(at_login, !later.is_empty())).unwrap_or(false);
            if !later.is_empty() {
                with_app(|a| a.open_external(later));
                pump();
            }
            if !hidden {
                window.makeKeyAndOrderFront(None);
                #[allow(deprecated)]
                app.activateIgnoringOtherApps(true);
            }
            if let Some(dir) = smoke {
                smoke::start(dir);
            }
        }

        /// 마지막 창을 닫으면 끝낸다. 메뉴 막대에서 대기 중이면 끝내지 않는다 (본 창은 닫는 대신 숨긴다: windowShouldClose:)
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn quit_after_last_window(&self, _app: &NSApplication) -> bool {
            !with_app(|a| a.stays_after_close()).unwrap_or(false)
        }

        /// Dock 아이콘을 눌렀다: 본 창이 숨어 있거나(대기 중에 닫았다) 최소화돼 있으면 다시 띄운다
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_reopen(&self, _app: &NSApplication, _visible: bool) -> bool {
            if standby::main_window().is_some_and(|w| !w.isVisible()) {
                standby::show_window(self.mtm());
            }
            false
        }

        /// Finder의 '다음으로 열기'나 Dock 아이콘에 놓은 폴더 (Info.plist의 CFBundleDocumentTypes): 새 탭으로 연다
        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _app: &NSApplication, urls: &NSArray<NSURL>) {
            let paths = urls.iter().filter_map(url_path).collect();
            open_from_finder(self.mtm(), paths);
        }

        /// 끝낼 때(⌘Q, 창 닫기, 메뉴 막대 아이콘의 끝내기) 작업 공간과 화면 상태를 저장하고 어디서든 찾기 키를 놓는다
        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _n: &NSNotification) {
            save_state();
            standby::unregister_hotkey();
        }
    }

    unsafe impl NSWindowDelegate for Delegate {
        /// 창이 다시 앞으로 왔다: 다른 앱(Finder 등)에서 바뀐 것이 보이게 보이는 칸을 다시 읽는다 (맥 버전에는 아직 폴더 감시가 없다)
        #[unsafe(method(windowDidBecomeKey:))]
        fn did_become_key(&self, _n: &NSNotification) {
            with_app(App::refresh_shown);
            pump();
            redraw();
        }

        /// 본 창의 닫기 단추: 메뉴 막대에서 대기 중이면 닫지 않고 숨기기만 한다 (앱이 끝나지 않는다. Windows의 트레이로 숨기기와 같다).
        /// 숨기기 전에 지금 화면을 저장하고(숨어 있는 동안 맥을 꺼도 남게), 설정 창과 훑어보기 창도 닫는다
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, sender: &NSWindow) -> bool {
            // (define_class 안의 bool 메서드에서는 return을 쓰지 않는다: 돌려주는 꼴이 Bool로 바뀐다)
            let hide = with_app(|a| a.stays_after_close()).unwrap_or(false);
            if hide {
                save_state();
                settings::close();
                if look_shown() {
                    show_look(false);
                }
                sender.orderOut(None);
            }
            !hide
        }

        /// 본 창이 닫힌다: 설정 창도 닫는다 (마지막 창이 닫히면 앱이 끝나는 동작을 그대로 둔다)
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, _n: &NSNotification) {
            settings::close();
        }
    }

    impl Delegate {
        /// 앱 메뉴의 '설정…'(⌘,): 어느 창이 앞에 있어도 앱 위임까지 온다
        #[unsafe(method(sakaduSettings:))]
        fn sakadu_settings(&self, _sender: Option<&AnyObject>) {
            settings::open(self.mtm());
        }

        /// 서비스 메뉴의 'Sakadu에서 열기' (Info.plist의 NSServices가 부르는 이름): 판에 담긴 폴더·파일을 새 탭으로 연다
        #[unsafe(method(openInSakadu:userData:error:))]
        fn open_in_sakadu(&self, board: &NSPasteboard, _data: Option<&NSString>, _error: *mut *mut NSString) {
            open_from_finder(self.mtm(), board_paths(board));
        }

        /// 메뉴 막대 아이콘의 '창 열기'
        #[unsafe(method(sakaduShowWindow:))]
        fn sakadu_show_window(&self, _sender: Option<&AnyObject>) {
            standby::show_window(self.mtm());
        }

        /// 메뉴 막대 아이콘의 '찾기' (어디서든 찾기 키와 같다)
        #[unsafe(method(sakaduFindAnywhere:))]
        fn sakadu_find_anywhere(&self, _sender: Option<&AnyObject>) {
            standby::find_anywhere(self.mtm());
        }

        /// 디스크가 붙거나 떨어지거나 이름이 바뀌었다 (NSWorkspace의 알림. watch_volumes가 건다): 사이드바의 디스크 줄을 다시 읽고,
        /// 떨어진 디스크를 보던 탭은 홈으로, 이름이 바뀐 디스크를 보던 탭은 새 자리로
        #[unsafe(method(volumesChanged:))]
        fn volumes_changed(&self, n: &NSNotification) {
            let event = volume_event(n);
            with_app(|a| a.on_volumes(places(), event));
            pump();
            redraw();
        }

        /// 메뉴 막대의 동작 줄 (파일·보기·이동·도움말): 줄의 tag가 단축키 표(ACTIONS)의 번호다.
        /// 메뉴 단축키(⌘T, ⌘W, ⇧⌘N…)는 뷰의 handle_key를 거치지 않으므로, 단축키 표의 동작은 누르고 있어도 한 번만 한다는 규칙을 여기서도 지킨다
        #[unsafe(method(sakaduAction:))]
        fn sakadu_action(&self, sender: &NSMenuItem) {
            if held_key(self.mtm()) {
                return;
            }
            let Some(&(a, ..)) = usize::try_from(sender.tag()).ok().and_then(|i| sakadu::keys::ACTIONS.get(i)) else { return };
            with_app(|app| app.run_action(a));
            after_menu();
        }

        /// 이동 메뉴의 뒤로·앞으로·위 폴더 (tag 0, 1, 2. 바꾸지 않는 키)
        #[unsafe(method(sakaduGo:))]
        fn sakadu_go(&self, sender: &NSMenuItem) {
            let tag = sender.tag();
            with_app(|app| {
                let p = app.active_pane();
                match tag {
                    0 => app.go_back(p),
                    1 => app.go_forward(p),
                    _ => app.go_up(p),
                }
            });
            after_menu();
        }
    }
);

/// 메뉴 막대에서 동작을 한 뒤: 본 창을 입력 뒤처럼 맞춘다 (읽기 시작, 입력 상자, 다시 그리기)
fn after_menu() {
    VIEW.with(|v| {
        if let Some(v) = v.get() {
            v.after_input();
        }
    });
}

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars::default());
        unsafe { msg_send![super(this), init] }
    }
}

/// 창: 제목 표시줄을 투명하게 하고 내용이 창 전체를 쓴다 (신호등은 사이드바 위에 겹친다).
/// start가 있으면 그 폴더에서 시작한다 (없으면 홈). 띄우는 것(makeKeyAndOrderFront)은 부르는 쪽이 한다:
/// 대기를 켜고 로그인할 때 열리면 창 없이 시작한다
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
        app.load_find_setting();
        app.set_keymap(sakadu::keys::Keymap::load());
        app.load_standby();
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
    window
}

/// 앱 메뉴: Sakadu(정보·설정·가리기·끝내기), 파일, 편집(실행 취소·오려두기·복사·붙여넣기·모두 선택·찾기), 보기, 이동, 윈도(최소화·확대/축소), 도움말.
/// 편집 메뉴의 줄은 키 입력을 가진 쪽이 받는다: 입력 상자에서는 글자에, 목록에서는 실행 취소가 파일 작업 되돌리기, 전체 선택이 모든 줄 고르기.
/// 동작 줄의 키는 단축키 표(App::keymap)에서 온다: 설정에서 키를 바꾸면 다시 짓는다
pub(super) fn main_menu(mtm: MainThreadMarker, app: &NSApplication) {
    use sakadu::keys::{ACTIONS, Action, name};
    let keys = with_app(|a| a.keymap().clone()).unwrap_or_default();
    let item = |menu: &NSMenu, title: &str, action: Sel, key: &str, mods: Option<NSEventModifierFlags>| {
        let it = unsafe { menu.addItemWithTitle_action_keyEquivalent(&NSString::from_str(title), Some(action), &NSString::from_str(key)) };
        if let Some(m) = mods {
            it.setKeyEquivalentModifierMask(m);
        }
        it
    };
    // 단축키 표의 키를 줄에 단다 (⌘나 ⌃가 없는 키는 달지 않는다: 메뉴 키는 입력 상자의 글자 입력보다 먼저 받아 간다)
    let keyed = |menu: &NSMenu, title: &str, action: Sel, a: Action| {
        let it = item(menu, title, action, "", None);
        if let Some((key, mods)) = keys.get(a).and_then(menu_key) {
            it.setKeyEquivalent(&NSString::from_str(&key));
            it.setKeyEquivalentModifierMask(mods);
        }
        it
    };
    let act = |menu: &NSMenu, a: Action| {
        let it = keyed(menu, name(a), sel!(sakaduAction:), a);
        it.setTag(ACTIONS.iter().position(|x| x.0 == a).unwrap_or(0) as NSInteger);
    };
    let go = |menu: &NSMenu, title: &str, tag: NSInteger, key: &str, mods: NSEventModifierFlags| {
        item(menu, title, sel!(sakaduGo:), key, Some(mods)).setTag(tag);
    };
    let submenu = |bar: &NSMenu, title: &str| {
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
        let top = NSMenuItem::new(mtm);
        top.setSubmenu(Some(&menu));
        bar.addItem(&top);
        menu
    };
    let sep = |menu: &NSMenu| menu.addItem(&NSMenuItem::separatorItem(mtm));
    let cmd = NSEventModifierFlags::Command;
    let bar = NSMenu::new(mtm);
    let m = submenu(&bar, "Sakadu");
    item(&m, "Sakadu 정보", sel!(orderFrontStandardAboutPanel:), "", None);
    sep(&m);
    keyed(&m, "설정…", sel!(sakaduSettings:), Action::Settings);
    sep(&m);
    item(&m, "Sakadu 가리기", sel!(hide:), "h", None);
    item(&m, "기타 가리기", sel!(hideOtherApplications:), "h", Some(cmd | NSEventModifierFlags::Option));
    item(&m, "모두 보기", sel!(unhideAllApplications:), "", None);
    sep(&m);
    item(&m, "Sakadu 종료", sel!(terminate:), "q", None);
    let f = submenu(&bar, "파일");
    for a in [Action::NewTab, Action::NewFolder, Action::CloseTab, Action::ReopenTab] {
        act(&f, a);
    }
    sep(&f);
    for a in [Action::Rename, Action::AddFavorite] {
        act(&f, a);
    }
    let e = submenu(&bar, "편집");
    keyed(&e, "실행 취소", sel!(undo:), Action::Undo);
    item(&e, "실행 복귀", sel!(redo:), "z", Some(cmd | NSEventModifierFlags::Shift));
    sep(&e);
    item(&e, "오려두기", sel!(cut:), "x", None);
    item(&e, "복사하기", sel!(copy:), "c", None);
    item(&e, "붙여넣기", sel!(paste:), "v", None);
    item(&e, "전체 선택", sel!(selectAll:), "a", None);
    sep(&e);
    keyed(&e, "찾기", sel!(sakaduFind:), Action::Search);
    let v = submenu(&bar, "보기");
    act(&v, Action::Hidden);
    act(&v, Action::Preview);
    sep(&v);
    for a in [Action::Huge, Action::Thumbs, Action::Medium, Action::SmallIcons, Action::List, Action::Details] {
        act(&v, a);
    }
    sep(&v);
    for a in [Action::Layout1, Action::Layout2, Action::Layout3, Action::Layout4, Action::Layout5] {
        act(&v, a);
    }
    sep(&v);
    act(&v, Action::Refresh);
    let g = submenu(&bar, "이동");
    act(&g, Action::Address);
    act(&g, Action::Commands);
    sep(&g);
    go(&g, "뒤로", 0, "[", cmd);
    go(&g, "앞으로", 1, "]", cmd);
    go(&g, "위 폴더", 2, "\u{F700}", cmd);
    let w = submenu(&bar, "윈도우");
    item(&w, "최소화", sel!(performMiniaturize:), "m", None);
    item(&w, "확대/축소", sel!(performZoom:), "", None);
    let h = submenu(&bar, "도움말");
    act(&h, Action::Help);
    app.setMainMenu(Some(&bar));
    app.setWindowsMenu(Some(&w));
    app.setHelpMenu(Some(&h));
}

/// 단축키를 메뉴의 키로: (키 글자, 수정 키). ⌘나 ⌃가 없으면 None (메뉴에 달지 않고 목록이 키로 받는다)
fn menu_key(c: sakadu::keys::Combo) -> Option<(String, NSEventModifierFlags)> {
    if !(c.cmd || c.ctrl) {
        return None;
    }
    // AppKit의 기능 키 글자 (NSUpArrowFunctionKey 등)
    let special = |u: u32| char::from_u32(u).map(String::from);
    let key = match c.vk {
        0x41..=0x5A => ((c.vk as u8) as char).to_ascii_lowercase().to_string(),
        0x30..=0x39 => ((c.vk as u8) as char).to_string(),
        0x70..=0x83 => special(0xF704 + (c.vk - 0x70) as u32)?,
        0x26 => special(0xF700)?,
        0x28 => special(0xF701)?,
        0x25 => special(0xF702)?,
        0x27 => special(0xF703)?,
        0x24 => special(0xF729)?,
        0x23 => special(0xF72B)?,
        0x21 => special(0xF72C)?,
        0x22 => special(0xF72D)?,
        0x0D => "\r".to_string(),
        0x08 => "\u{8}".to_string(),
        0x09 => "\t".to_string(),
        0x1B => "\u{1b}".to_string(),
        0x20 => " ".to_string(),
        vk => sakadu::keys::key_name(vk).filter(|n| n.chars().count() == 1)?,
    };
    let mut mods = NSEventModifierFlags::empty();
    for (on, flag) in [(c.cmd, NSEventModifierFlags::Command), (c.ctrl, NSEventModifierFlags::Control), (c.alt, NSEventModifierFlags::Option), (c.shift, NSEventModifierFlags::Shift)] {
        if on {
            mods |= flag;
        }
    }
    Some((key, mods))
}

/// 앱의 모양을 App의 설정(시스템·밝게·어둡게)으로 정한다. 바뀌면 본 창이 맥 모양 바뀜으로 받아 테마를 다시 정한다
pub(super) fn apply_theme(mtm: MainThreadMarker) {
    let pref = with_app(|a| a.theme_pref().to_string()).unwrap_or_default();
    let name = match pref.as_str() {
        "light" => Some(unsafe { NSAppearanceNameAqua }),
        "dark" => Some(unsafe { NSAppearanceNameDarkAqua }),
        _ => None,
    };
    let appearance = name.and_then(NSAppearance::appearanceNamed);
    NSApplication::sharedApplication(mtm).setAppearance(appearance.as_deref());
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
    window.makeKeyAndOrderFront(None);
    with_app(|a| {
        for p in 0..2 {
            a.refresh(p);
        }
        a.settle();
    });
    save_window(&window, out)
}

/// 창을 창 틀(신호등, 제목 표시줄)까지 그려 PNG로 저장한다
fn save_window(window: &NSWindow, out: &std::path::Path) -> Result<(), String> {
    window.displayIfNeeded();
    let frame_view = window.contentView().and_then(|v| unsafe { v.superview() }).ok_or("창 틀을 찾지 못함")?;
    let b = frame_view.bounds();
    let rep = frame_view.bitmapImageRepForCachingDisplayInRect(b).ok_or("창 그림판을 만들지 못함")?;
    frame_view.cacheDisplayInRect_toBitmapImageRep(b, &rep);
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }.ok_or("PNG로 바꾸지 못함")?;
    std::fs::write(out, data.to_vec()).map_err(|e| e.to_string())
}

/// 설정 창 확인용 (CI): 본 창을 start 폴더로 열고 설정 창을 띄워 그 창을 PNG로 저장한다.
/// search면 검색을 켠 모습으로 (start 폴더만 훑는다: 러너의 홈 폴더를 훑으면 아무도 누르지 않는 권한 창이 뜰 수 있다)
pub fn snapshot_settings(out: &std::path::Path, start: String, dark: bool, search: bool) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("메인 줄이 아님")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let main = open_window(mtm, Some(start.clone()));
    main.makeKeyAndOrderFront(None);
    with_app(|a| {
        if search {
            a.set_find_roots(vec![start]);
            a.set_find_on(true);
        }
        a.settle();
    });
    settings::open(mtm);
    settings::set_dark(dark);
    let window = settings::shown().ok_or("설정 창이 뜨지 않음")?;
    save_window(&window, out)
}

/// 창 시험: 임시 폴더에 시험 폴더를 만들고, 보통 실행과 같은 길로 창을 그 폴더에 연 뒤 시험 대본을 돌린다 (smoke.rs).
/// 화면 상태와 작업 공간은 읽지도 저장하지도 않는다. 다 하면 결과에 따라 0이나 1로 끝난다
pub fn smoke() -> Result<(), String> {
    let dir = smoke::make_folder().map_err(|e| format!("시험 폴더: {e}"))?;
    println!("smoke folder: {dir}");
    SMOKE.with(|s| *s.borrow_mut() = Some(dir));
    run();
    Ok(())
}

/// 창을 띄우고 앱을 돌린다 (창을 닫으면 끝난다. 메뉴 막대에서 대기 중이면 창만 숨는다)
pub fn run() {
    let mtm = MainThreadMarker::new().expect("메인 줄에서 시작");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = Delegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 훑어보기 창: 프레임워크를 열어 창의 클래스를 찾고, 이 뷰가 부르는 그 창의 메서드 이름과 이 뷰가 대는 메서드 이름이
    /// 맥이 아는 이름과 맞는지 본다 (하나라도 틀리면 창이 비어 보이거나 앱이 멈춘다). 창을 띄우지는 않는다
    #[test]
    fn quick_look_names_match_the_system() {
        let cls = look_class().expect("QLPreviewPanel is found after opening Quartz");
        for sel in [sel!(sharedPreviewPanel), sel!(sharedPreviewPanelExists)] {
            assert!(cls.metaclass().instance_method(sel).is_some(), "class method {sel:?}");
        }
        for sel in [sel!(setDataSource:), sel!(setDelegate:), sel!(reloadData), sel!(setCurrentPreviewItemIndex:), sel!(makeKeyAndOrderFront:), sel!(orderOut:), sel!(isVisible)] {
            assert!(cls.instance_method(sel).is_some(), "panel method {sel:?}");
        }
        let view = SakaduView::class();
        for (proto, sel) in [
            (c"QLPreviewPanelDataSource", sel!(numberOfPreviewItemsInPreviewPanel:)),
            (c"QLPreviewPanelDataSource", sel!(previewPanel:previewItemAtIndex:)),
            (c"QLPreviewPanelDelegate", sel!(previewPanel:handleEvent:)),
        ] {
            let p = AnyProtocol::get(proto).unwrap_or_else(|| panic!("protocol {proto:?}"));
            // 꼭 있어야 하는 메서드든 골라 넣는 메서드든, 그 프로토콜에 그 이름이 있다
            let declared = [true, false].into_iter().any(|required| {
                // SAFETY: 등록된 프로토콜과 선택자로 묻기만 한다
                let d = unsafe { objc2::ffi::protocol_getMethodDescription(p, sel, Bool::new(required), Bool::YES) };
                d.name == Some(sel)
            });
            assert!(declared, "{proto:?} has no {sel:?}");
            assert!(view.conforms_to(p) && view.responds_to(sel), "the view takes {sel:?}");
        }
        // 응답 사슬에서 물을 때 쓰는 이름은 NSObject의 비공식 프로토콜이라 위처럼 따질 수 없다: 이 뷰에 있기만 본다
        for sel in [sel!(acceptsPreviewPanelControl:), sel!(beginPreviewPanelControl:), sel!(endPreviewPanelControl:)] {
            assert!(view.instance_method(sel).is_some(), "{sel:?}");
        }
    }

    /// 끌어 낸 쪽이 허락한 일에서 할 일을 고른다. 복사도 옮기기도 없으면 받지 않는다
    #[test]
    fn drop_kind_follows_what_the_source_allows() {
        let mask = |ops: &[NSDragOperation]| NSDragOperation(ops.iter().fold(0, |all, op| all | op.0));
        // 아무 키도 누르지 않았다: 사카두가 끌어 낸 것(복사, 옮기기), Finder가 끌어 낸 것(모두)
        assert_eq!(drop_kind(mask(&[NSDragOperation::Copy, NSDragOperation::Move, NSDragOperation::Generic])), Some(Paste::Drop));
        assert_eq!(drop_kind(NSDragOperation::Every), Some(Paste::Drop));
        // ⌥: 복사만 남는다. ⌘: 옮기기만 남는다
        assert_eq!(drop_kind(NSDragOperation::Copy), Some(Paste::Copy));
        assert_eq!(drop_kind(mask(&[NSDragOperation::Copy, NSDragOperation::Link])), Some(Paste::Copy));
        assert_eq!(drop_kind(NSDragOperation::Generic), Some(Paste::Move));
        assert_eq!(drop_kind(NSDragOperation::Move), Some(Paste::Move));
        // ⌥⌘: 가상본 만들기만 남는다. 옮기지도 복사하지도 않는다
        assert_eq!(drop_kind(NSDragOperation::Link), None);
        assert_eq!(drop_kind(NSDragOperation::None), None);
        assert_eq!(drop_kind(mask(&[NSDragOperation::Link, NSDragOperation::Delete, NSDragOperation::Private])), None);
    }
}
