//! 창 시험 (`sakadu-mac --smoke`): 맥 러너에서 실제 창을 열고, 실제 창이 입력을 받는 길(뷰의 keyDown:, 편집 메뉴가 보내는
//! copy:·paste:·undo:, 이름 상자의 Return)로 폴더 감시, 이름 바꾸기, 되돌리기, 복사·붙여넣기, 휴지통, 훑어보기를 끝까지 돌려 본다.
//! 경로 입력(⌘L), 새 폴더(⇧⌘N), 잘라내기·붙여넣기(옮기기)도 같은 길로 해 본다.
//! 검색(⌘F)도: 시험 폴더만 훑게 하고 켠 뒤, 검색 입력 칸에 쳐서 찾고 그 칸의 키(↓, Return, Esc)로 고르고 연다.
//! 설정 창도: 앱 메뉴의 '설정…'이 보내는 동작으로 열고, 스위치를 눌러 본 창의 값이 바뀌는지, 본 창에서 바꾼 값을 따라오는지 본다.
//! 메뉴(앱이 그린다)도: 목록의 빈 곳을 오른쪽 단추로 눌러 열고, 실제 키(Esc, ↓, Return)로 닫고 고른다.
//! 새 탭 시작 화면(⌘T)과 명령 메뉴(⌘K)도: 시작 화면을 키로 열어 보고, 명령 메뉴의 입력 칸에 쳐서 걸러 보고 Esc로 닫는다.
//! 사이드바 끌기도: 둘을 고정하고 실제 마우스 입력(mouseDown:·mouseDragged:·mouseUp:)으로 첫 줄을 아래로 끌어 순서를 바꾸고, 눌렀다 떼서 연다.
//! 오른쪽 미리 보기 창(⌥⌘P)도: 열어 글 파일의 글을 보고, PDF를 고르면 본문 자리에 Quick Look 보기가 뜨는지 보고, 닫는다.
//! 단위 시험은 App만 돌리고 창 쪽 코드는 타입 검사만 됐다: 그 틈을 메운다. 끌어서 놓기는 받는 쪽만 해 본다: 맥이 끌기 중에
//! 주는 정보(NSDraggingInfo)를 흉내 낸 것을 뷰의 draggingEntered:·performDragOperation:에 넘긴다 (끌어 내는 쪽은 흉내 낼 수 없다).
//! 시험 폴더는 임시 폴더에 새로 만들고 그 안에서만 한다 (휴지통으로 보낸 것은 되돌리기로 다시 꺼낸다).
//! 걸음마다 `ok`나 `FAIL`을 찍고, 하나라도 실패하면 1로, 메인 줄이 답하지 않으면 2로 끝난다

use super::*;
use crate::app::{MenuKind, Seen};
use sakadu::keys::Action;
use sakadu::pane::{Sort, SortKey};
use std::path::Path;
use std::sync::mpsc;
use std::time::Instant;

/// 키 번호 (맥 가상 키 코드)
const HOME: u16 = 0x73;
const UP: u16 = 0x7E;
const DOWN: u16 = 0x7D;
const RETURN: u16 = 0x24;
const DELETE: u16 = 0x33;
const Y: u16 = 0x10;
const L: u16 = 0x25;
const N: u16 = 0x2D;
const F: u16 = 0x03;
const PERIOD: u16 = 0x2F;
const ESC: u16 = 0x35;
const KEY_J: u16 = 0x26;
const KEY_K: u16 = 0x28;
const KEY_P: u16 = 0x23;
const KEY_T: u16 = 0x11;
const KEY_W: u16 = 0x0D;

/// 흉내 낸 끌기 정보: 끌려 온 것을 담은 붙여넣기 판, 창 좌표의 마우스 자리, 끌어 낸 쪽이 허락한 일
struct DragIvars {
    board: Retained<NSPasteboard>,
    at: NSPoint,
    allowed: NSDragOperation,
}

define_class!(
    /// 맥이 끌기 중에 받는 쪽에 주는 정보(NSDraggingInfo)를 흉내 낸 것. 뷰가 묻는 세 가지만 답한다.
    /// 그 프로토콜을 따른다고 밝히지는 않는다: 밝히면 objc2가 필수 메서드(animatesToDestination 등)를 모두 갖추라고 멈춘다.
    /// 뷰는 이것을 그 프로토콜로 받아 메시지로만 물으므로, 묻는 메서드만 있으면 된다
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SakaduSmokeDrag"]
    #[ivars = DragIvars]
    struct FakeDrag;

    unsafe impl NSObjectProtocol for FakeDrag {}

    impl FakeDrag {
        #[unsafe(method_id(draggingPasteboard))]
        fn dragging_pasteboard(&self) -> Retained<NSPasteboard> {
            self.ivars().board.clone()
        }

        #[unsafe(method(draggingLocation))]
        fn dragging_location(&self) -> NSPoint {
            self.ivars().at
        }

        #[unsafe(method(draggingSourceOperationMask))]
        fn dragging_source_operation_mask(&self) -> NSDragOperation {
            self.ivars().allowed
        }
    }
);

impl FakeDrag {
    fn new(mtm: MainThreadMarker, board: Retained<NSPasteboard>, at: NSPoint, allowed: NSDragOperation) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DragIvars { board, at, allowed });
        // SAFETY: NSObject의 init
        unsafe { msg_send![super(this), init] }
    }
}

/// 시험 폴더를 새로 만든다: 폴더 하나, 글 파일 둘
pub(super) fn make_folder() -> std::io::Result<String> {
    let root = std::env::temp_dir().join(format!("sakadu-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("폴더"))?;
    std::fs::write(root.join("가나다.txt"), "훑어보기로 보는 글\n")?;
    std::fs::write(root.join("메모.txt"), "메모\n")?;
    Ok(sakadu::util::normalize(&std::fs::canonicalize(&root)?.to_string_lossy()))
}

/// 시험을 뒷줄에서 시작한다 (창이 뜬 뒤 메인 줄에서 부른다). 다 하면 시험 폴더를 지우고 앱을 끝낸다
pub(super) fn start(dir: String) {
    std::thread::spawn(move || {
        let failed = script(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        if failed == 0 {
            println!("smoke: all passed");
        } else {
            println!("smoke: {failed} failed");
        }
        DispatchQueue::main().exec_async(move || std::process::exit(i32::from(failed > 0)));
    });
}

/// 메인 줄에서 f를 돌려 그 값을 받는다. 메인 줄이 20초 넘게 답하지 않으면 끝낸다 (창이 멈췄다)
fn on_main<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    DispatchQueue::main().exec_async(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| {
        println!("FAIL the main thread did not answer for 20 s");
        std::process::exit(2)
    })
}

/// 지금 칸에서 앱이 보이는 것
fn seen() -> Option<Seen> {
    on_main(|| with_app(|a| a.seen()))
}

/// 키 하나를 실제 창이 키를 받는 길(뷰의 keyDown:)로 보낸다
fn key(code: u16, cmd: bool) {
    key_with(code, if cmd { NSEventModifierFlags::Command } else { NSEventModifierFlags::empty() });
}

fn key_with(code: u16, flags: NSEventModifierFlags) {
    on_main(move || {
        let Some(view) = VIEW.with(|v| v.get().cloned()) else { return };
        let number = view.window().map_or(0, |w| w.windowNumber());
        let none = NSString::from_str("");
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            number,
            None,
            &none,
            &none,
            false,
            code,
        );
        if let Some(e) = event {
            view.keyDown(&e);
        }
    });
}

/// 편집 메뉴가 보내는 동작(copy:, paste:, undo:)을 메뉴가 보내는 대로 뷰에 보낸다
fn act(f: fn(&SakaduView)) {
    on_main(move || {
        if let Some(view) = VIEW.with(|v| v.get().cloned()) {
            f(&view);
        }
    });
}

fn exists(dir: &str, name: &str) -> bool {
    std::fs::symlink_metadata(Path::new(dir).join(name)).is_ok()
}

/// 시험 결과를 모은다
struct Run {
    failed: usize,
}

impl Run {
    fn check(&mut self, what: &str, ok: bool, why: impl FnOnce() -> String) -> bool {
        if ok {
            println!("ok   {what}");
        } else {
            self.failed += 1;
            println!("FAIL {what}: {}", why());
        }
        ok
    }

    /// cond가 참이 될 때까지 기다린다 (0.05초마다, 길어야 limit초)
    fn wait(&mut self, what: &str, limit: u64, cond: impl Fn() -> bool) -> bool {
        let began = Instant::now();
        while began.elapsed() < Duration::from_secs(limit) {
            if cond() {
                return self.check(what, true, String::new);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let last = seen();
        self.check(what, false, || format!("not within {limit} s (the list shows {last:?})"))
    }

    /// 지금 칸에서 그 이름의 줄로 실제 키(Home, 그리고 ↓를 그만큼)를 눌러 옮긴다. 그 줄만 골라진다.
    /// 앞 걸음의 결과로 목록을 다시 읽는 중일 수 있어서, 그 이름이 목록에 오고 앱이 일을 마칠 때까지 먼저 기다린다
    fn select(&mut self, name: &str) -> bool {
        let listed = |s: &Seen| s.ready && !s.busy && s.names.iter().any(|n| n == name);
        if !self.wait(&format!("{name} is in the list"), 5, || seen().is_some_and(|s| listed(&s))) {
            return false;
        }
        let Some(i) = seen().and_then(|s| s.names.iter().position(|n| n == name)) else {
            return self.check(&format!("{name} stays in the list"), false, || format!("{:?}", seen()));
        };
        key(HOME, false);
        for _ in 0..i {
            key(DOWN, false);
        }
        self.wait(&format!("keys move the cursor to {name}"), 5, || seen().is_some_and(|s| s.cursor == name && s.selected == [name]))
    }
}

/// 시험 대본. 실패한 걸음의 수를 돌려준다
fn script(dir: &str) -> usize {
    let mut run = Run { failed: 0 };
    // 창이 열리고 시험 폴더를 읽었다
    let first = ["폴더", "가나다.txt", "메모.txt"];
    if !run.wait("the window lists the test folder", 10, || seen().is_some_and(|s| s.ready && s.dir == dir && s.names == first)) {
        return run.failed;
    }
    let active = on_main(|| {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().expect("main"));
        (app.isActive(), VIEW.with(|v| v.get().and_then(|v| v.window()).is_some_and(|w| w.isKeyWindow())))
    });
    println!("note app active: {}, window is key: {}", active.0, active.1);

    // 폴더 감시: 밖에서 만든 파일이 목록에 저절로 나타난다
    let made = std::fs::write(Path::new(dir).join("새 파일.txt"), b"x").is_ok();
    run.check("a file is made from outside", made, String::new);
    run.wait("folder watching shows it without a refresh", 5, || seen().is_some_and(|s| s.names.iter().any(|n| n == "새 파일.txt")));

    // 이름 바꾸기: Return으로 이름 상자를 열고, 상자에 치고 Return을 누른다
    if run.select("메모.txt") {
        key(RETURN, false);
        // 어디까지 됐는지: 앱이 고치기를 시작했는지, 상자가 보이는지, 상자가 키를 받는지, 앱의 알림
        let state = on_main(|| {
            let s = with_app(|a| a.seen());
            let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().field.get().cloned()));
            let shown = field.as_ref().is_some_and(|f| !f.isHidden());
            let typing = field.as_ref().is_some_and(|f| f.currentEditor().is_some());
            format!("editing {:?}, field shown {shown}, field has the keyboard {typing}, note {:?}", s.as_ref().map(|s| s.editing.clone()), s.and_then(|s| s.flash))
        });
        println!("note after Return: {state}");
        run.wait("Return opens the name field on the row", 5, || field_text().as_deref() == Some("메모.txt"));
        let typed = type_in_field("메모 바뀜.txt");
        run.check("typing a name and Return in the field", typed.is_ok(), || format!("{typed:?}"));
        if run.wait("the file is renamed on disk", 5, || exists(dir, "메모 바뀜.txt") && !exists(dir, "메모.txt")) {
            // 이름 상자는 앱이 그 결과를 받을 때 닫힌다. 되돌리기는 그 뒤에 보낸다
            run.wait("the field closes and the renamed row is selected", 5, || seen().is_some_and(|s| s.editing.is_none() && s.cursor == "메모 바뀜.txt" && !s.busy));
            // 되돌리기: 편집 메뉴의 실행 취소(⌘Z)가 보내는 undo:
            // SAFETY: 이 뷰의 undo: (편집 메뉴가 보내는 그대로)
            act(|v| unsafe { msg_send![v, undo: None::<&AnyObject>] });
            run.wait("undo: brings the old name back", 5, || exists(dir, "메모.txt") && !exists(dir, "메모 바뀜.txt"));
        } else {
            // 다음 걸음이 이 파일을 쓰므로, 이름을 고치다 만 상태를 치운다
            on_main(|| with_app(App::cancel_address));
        }
    }

    // 복사와 붙여넣기: 편집 메뉴의 copy:와 paste:, 맥의 클립보드
    if run.select("메모.txt") {
        // SAFETY: 이 뷰의 copy:
        act(|v| unsafe { msg_send![v, copy: None::<&AnyObject>] });
        // 클립보드를 거친 경로는 글자 모양(한글 자모의 정규화)이 달라질 수 있어 같은 파일인지로 본다
        use std::os::unix::fs::MetadataExt;
        let inode = |p: &str| std::fs::metadata(p).map(|m| m.ino()).ok();
        let want = inode(&sakadu::util::join(dir, "메모.txt"));
        let board = on_main(|| read_clipboard().0);
        run.check("copy: puts the file on the clipboard", board.len() == 1 && want.is_some() && inode(&board[0]) == want, || format!("{board:?}"));
        // SAFETY: 이 뷰의 paste:
        act(|v| unsafe { msg_send![v, paste: None::<&AnyObject>] });
        run.wait("paste: makes a numbered copy", 5, || exists(dir, "메모 2.txt"));
        run.wait("and the list shows it", 5, || seen().is_some_and(|s| s.names.iter().any(|n| n == "메모 2.txt")));
    }

    // 휴지통: ⌘⌫, 그리고 되돌리기로 다시 꺼낸다
    if run.select("메모 2.txt") {
        key(DELETE, true);
        // 디스크에서 사라진 것만으로는 앱이 그 결과를 받았는지(되돌리기 기록이 생겼는지) 모른다: 앱이 결과를 다 받을 때까지 기다린다
        if run.wait("cmd-delete sends the file to the Trash", 5, || !exists(dir, "메모 2.txt") && seen().is_some_and(|s| !s.busy)) {
            // SAFETY: 이 뷰의 undo:
            act(|v| unsafe { msg_send![v, undo: None::<&AnyObject>] });
            run.wait("undo: puts it back from the Trash", 5, || exists(dir, "메모 2.txt"));
        }
    }

    // 훑어보기: ⌘Y로 열고 닫는다 (글 파일이라 스페이스는 카드를 크게 한다)
    if run.select("가나다.txt") {
        key(Y, true);
        let opened = run.wait("cmd-Y opens the Quick Look panel", 10, || on_main(|| look_shown() && LOOK_ON.get()));
        // 어디까지 됐는지: 창의 클래스를 찾았는지, 창이 있는지, 떠 있는지, 이 뷰가 내용을 맡았는지, 앱의 알림
        let state = on_main(|| {
            let cls = LOOK_CLASS.with(|cell| cell.get().copied().flatten()).is_some();
            let panel = look_panel(false);
            // SAFETY: NSWindow의 isVisible과 isKeyWindow
            let (visible, key) = panel.as_ref().map_or((false, false), |p| unsafe { (msg_send![&**p, isVisible], msg_send![&**p, isKeyWindow]) });
            let flash = with_app(|a| a.seen().flash).flatten();
            format!("class {cls}, panel {}, visible {visible}, panel is key {key}, view controls it {}, note {flash:?}", panel.is_some(), LOOK_ON.get())
        });
        println!("note quick look: {state}");
        let items = on_main(|| LOOK.with(|l| l.borrow().clone()));
        let want = (vec![sakadu::util::join(dir, "가나다.txt")], 0);
        run.check("the panel is given the selected file", items == want, || format!("{items:?}"));
        if opened {
            key(Y, true);
            run.wait("cmd-Y closes it", 5, || on_main(|| !look_shown()));
        }
    }

    // 오른쪽 미리 보기 창: ⌥⌘P로 열면 고른 글 파일의 글이 보인다. PDF를 고르면 본문 자리에 Quick Look 보기(QLPreviewView)가 뜬다.
    // ⌥⌘P로 닫으면 칸들이 다시 넓어지고 그 보기도 감춰진다 (PDF는 시험 폴더에 잠깐 만들었다 지운다)
    let alt_cmd = NSEventModifierFlags::Command | NSEventModifierFlags::Option;
    if run.select("가나다.txt") {
        key_with(KEY_P, alt_cmd);
        let shown = || on_main(|| with_app(|a| a.preview_seen()).flatten());
        let text_shown = || shown().is_some_and(|(n, t)| n.as_deref() == Some("가나다.txt") && t.is_some_and(|t| t.starts_with("훑어보기로 보는 글")));
        if run.wait("alt-cmd-P opens the preview pane with the selected file's text", 5, text_shown) {
            let pdf = Path::new(dir).join("문서.pdf");
            run.check("a PDF is made in the test folder", std::fs::write(&pdf, crate::render::tiny_pdf(200, 300)).is_ok(), String::new);
            if run.select("문서.pdf") {
                run.wait("a PDF shows in a Quick Look view over the pane body", 10, || on_main(|| look_view_showing().is_some_and(|p| p.ends_with("문서.pdf"))));
            }
            let _ = std::fs::remove_file(&pdf);
            run.wait("the PDF leaves the list", 5, || seen().is_some_and(|s| s.ready && !s.names.iter().any(|n| n == "문서.pdf")));
        }
        key_with(KEY_P, alt_cmd);
        run.wait("alt-cmd-P closes the pane and hides the Quick Look view", 5, || shown().is_none() && on_main(|| look_view_showing().is_none()));
    }

    // 경로 입력: ⌘L로 상자를 열고 하위 폴더의 경로를 쳐서 Return. ⌘↑로 돌아오면 그 폴더가 골라져 있다
    let sub = sakadu::util::join(dir, "폴더");
    key(L, true);
    if run.wait("cmd-L opens the path field with the folder", 5, || field_text().as_deref() == Some(dir)) {
        let typed = type_in_field(&sub);
        run.check("typing a path and Return in the field", typed.is_ok(), || format!("{typed:?}"));
        let at_sub = sub.clone();
        if run.wait("the pane goes to the typed folder", 5, move || seen().is_some_and(|s| s.ready && s.dir == at_sub)) {
            key(UP, true);
            run.wait("cmd-up goes back with the folder selected", 5, || seen().is_some_and(|s| s.ready && s.dir == dir && s.cursor == "폴더"));
        }
    } else {
        on_main(|| with_app(App::cancel_address));
    }

    // 새 폴더: ⇧⌘N으로 만들면 그 줄에 이름 상자가 뜬다. 이름을 쳐서 Return
    key_with(N, NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
    if run.wait("shift-cmd-N makes a folder and opens its name field", 5, || field_text().as_deref() == Some("새 폴더")) {
        let typed = type_in_field("자료");
        run.check("naming the new folder", typed.is_ok(), || format!("{typed:?}"));
        run.wait("the new folder has that name", 5, || {
            std::fs::metadata(Path::new(dir).join("자료")).is_ok_and(|m| m.is_dir()) && !exists(dir, "새 폴더") && seen().is_some_and(|s| s.editing.is_none() && !s.busy)
        });
    } else {
        on_main(|| with_app(App::cancel_address));
    }

    // 잘라내기와 붙여넣기: cut:으로 잘라 두고, 하위 폴더에 들어가 paste:로 옮긴다. undo:로 돌아온다
    if run.select("새 파일.txt") {
        // SAFETY: 이 뷰의 cut:
        act(|v| unsafe { msg_send![v, cut: None::<&AnyObject>] });
        if run.select("폴더") {
            // ⌘↓: 고른 폴더로 들어간다
            key(DOWN, true);
            let at_sub = sub.clone();
            if run.wait("cmd-down opens the folder", 5, move || seen().is_some_and(|s| s.ready && s.dir == at_sub)) {
                // SAFETY: 이 뷰의 paste:
                act(|v| unsafe { msg_send![v, paste: None::<&AnyObject>] });
                let moved = run.wait("paste: after cut: moves the file", 5, || {
                    exists(&sub, "새 파일.txt") && !exists(dir, "새 파일.txt") && seen().is_some_and(|s| !s.busy)
                });
                if moved {
                    // SAFETY: 이 뷰의 undo:
                    act(|v| unsafe { msg_send![v, undo: None::<&AnyObject>] });
                    run.wait("undo: moves it back", 5, || exists(dir, "새 파일.txt") && !exists(&sub, "새 파일.txt") && seen().is_some_and(|s| !s.busy));
                }
                key(UP, true);
                run.wait("cmd-up goes back", 5, || seen().is_some_and(|s| s.ready && s.dir == dir));
            }
        }
    }

    // 끌어다 놓기 (받는 쪽): 다른 폴더의 파일을 '폴더' 줄 위로 끌어 와 놓는다. 같은 디스크라 옮겨진다. undo:로 돌아간다
    let outside = std::env::temp_dir().join(format!("sakadu-smoke-src-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    let made = std::fs::create_dir_all(&outside).and_then(|_| std::fs::write(outside.join("밖.txt"), b"x")).is_ok();
    let src = sakadu::util::normalize(&std::fs::canonicalize(&outside).unwrap_or(outside.clone()).to_string_lossy());
    let src_file = sakadu::util::join(&src, "밖.txt");
    run.check("a file is made in another folder", made, String::new);
    if made && run.wait("the 폴더 row is in the list", 5, || seen().is_some_and(|s| s.ready && !s.busy && s.names.iter().any(|n| n == "폴더"))) {
        let file = src_file.clone();
        let (hover, dropped) = on_main(move || drop_on_row(&file, "폴더"));
        run.check("a drag over the 폴더 row offers a move", hover == Some(NSDragOperation::Move), || format!("{hover:?}"));
        run.check("dropping it starts the move", dropped, String::new);
        let moved = run.wait("the file is moved into 폴더", 5, || exists(&sub, "밖.txt") && !exists(&src, "밖.txt") && seen().is_some_and(|s| !s.busy));
        if moved {
            // SAFETY: 이 뷰의 undo:
            act(|v| unsafe { msg_send![v, undo: None::<&AnyObject>] });
            run.wait("undo: moves it back out", 5, || exists(&src, "밖.txt") && !exists(&sub, "밖.txt"));
        }
    }
    let _ = std::fs::remove_dir_all(&outside);

    // 검색: 시험 폴더만 훑게 하고 ⌘F로 연다. 켜면 색인을 만들고, 입력 칸에 친 글로 찾는다.
    // 그 칸의 ↓로 결과를 고르고, Return으로 폴더를 칸에서 연다(키보드는 칸으로). 다시 ⌘F로 칸에 돌아와 Esc로 닫는다
    let root = dir.to_string();
    on_main(move || with_app(|a| a.set_find_roots(vec![root])));
    key(F, true);
    if run.wait("cmd-F opens search with the keyboard in its field", 5, || find_state().is_some_and(|s| s.0 && s.1)) {
        on_main(|| {
            with_app(App::enable_find);
            pump();
        });
        run.wait("turning search on indexes the test folder", 10, || find_state().is_some_and(|s| s.4));
        let typed = type_in_search("메모");
        run.check("typing in the search field", typed.is_ok(), || format!("{typed:?}"));
        run.wait("typing finds the files", 5, || find_state().is_some_and(|s| s.2.len() == 2 && s.2.iter().all(|n| n.starts_with("메모"))));
        let moved = search_command(sel!(moveDown:));
        run.check("down in the field", moved.is_ok(), || format!("{moved:?}"));
        run.wait("down picks the next result", 5, || find_state().is_some_and(|s| s.3 == 1));
        let _ = type_in_search("자료");
        run.wait("the folder is found", 5, || find_state().is_some_and(|s| s.2 == ["자료"]));
        let _ = search_command(sel!(insertNewline:));
        let folder = sakadu::util::join(dir, "자료");
        run.wait("Return opens the folder in the pane and gives it the keyboard", 5, move || {
            seen().is_some_and(|s| s.ready && s.dir == folder) && find_state().is_some_and(|s| s.0 && !s.1)
        });
        key(UP, true);
        run.wait("cmd-up goes back", 5, || seen().is_some_and(|s| s.ready && s.dir == dir));
        key(F, true);
        run.wait("cmd-F gives the field the keyboard again", 5, || find_state().is_some_and(|s| s.1));
        let _ = search_command(sel!(cancelOperation:));
        run.wait("Esc closes search", 5, || find_state().is_some_and(|s| !s.0) && on_main(|| VIEW.with(|v| v.get().and_then(|v| v.ivars().search.get().cloned())).is_some_and(|f| f.isHidden())));
    }

    // 설정 창: 앱 메뉴의 '설정…'이 보내는 동작(응답 사슬을 따라 앱 위임까지)으로 연다. 다시 열어도 같은 창이다.
    // 숨김 스위치를 누르면 본 창의 값이 바뀌고, 본 창에서 ⌘⇧.를 누르면 스위치가 따라온다.
    // 검색 스위치로 끄고 다시 켠다 (위에서 시험 폴더만 훑게 해 두었다). Esc로 닫는다
    send_settings_action();
    if run.wait("the settings menu action opens the settings window", 5, || on_main(|| settings::shown().is_some())) {
        let first = on_main(|| settings::shown().map(|w| Retained::as_ptr(&w) as usize));
        send_settings_action();
        let again = on_main(|| settings::shown().map(|w| Retained::as_ptr(&w) as usize));
        run.check("opening it again keeps one window", first.is_some() && first == again, || format!("{first:?} {again:?}"));
        click_switch("hidden");
        run.wait("the hidden switch shows hidden items in the main window", 5, || on_main(|| with_app(|a| a.prefs().hidden)) == Some(true));
        key_with(PERIOD, NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
        run.wait("cmd-shift-period in the main window turns the switch off", 5, || switch_on("hidden") == Some(false));
        run.check("the search switch shows search is on", switch_on("search") == Some(true), || format!("{:?}", switch_on("search")));
        click_switch("search");
        run.wait("the search switch turns search off and drops the index", 5, || on_main(|| with_app(|a| (a.prefs().search_on, a.find_seen().4))) == Some((false, false)));
        click_switch("search");
        run.wait("turning it on again indexes the test folder", 10, || on_main(|| with_app(|a| a.prefs().search_on && a.find_seen().4)) == Some(true));
        // 단축키: '새 탭' 단추를 누르고 ⌘J를 누르면 새 탭이 ⌘J가 된다. 본 창에서 ⌘J로 탭이 늘고, 기본으로 되돌리면 ⌘T로 돌아온다
        let title = || on_main(|| settings::key_button(Action::NewTab).map(|b| b.title().to_string()));
        run.check("the new-tab button shows its key", title().as_deref() == Some("⌘T"), || format!("{:?}", title()));
        on_main(|| {
            if let Some(b) = settings::key_button(Action::NewTab) {
                // SAFETY: NSControl의 performClick:
                unsafe { b.performClick(None) };
            }
        });
        run.wait("pressing it waits for a key", 5, || title().as_deref() == Some("키를 누르세요"));
        on_main(|| {
            let Some(w) = settings::shown() else { return };
            let j = NSString::from_str("j");
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyDown,
                NSPoint::new(0.0, 0.0),
                NSEventModifierFlags::Command,
                0.0,
                w.windowNumber(),
                None,
                &j,
                &j,
                false,
                KEY_J,
            );
            if let Some(e) = event {
                w.sendEvent(&e);
            }
        });
        run.wait("cmd-J becomes the new-tab key", 5, || title().as_deref() == Some("⌘J"));
        let tabs = || on_main(|| with_app(|a| a.seen().tabs));
        let before = tabs();
        key(KEY_J, true);
        run.wait("cmd-J opens a tab in the main window", 5, || tabs().zip(before).is_some_and(|(now, was)| now == was + 1));
        // 새 탭은 시작 화면으로 열린다 (설정의 기본): 닫아서 시험 폴더의 탭으로 돌아온다
        key(KEY_W, true);
        run.wait("cmd-W closes that tab again", 5, || tabs() == before && seen().is_some_and(|s| s.dir == dir && !s.start));
        on_main(|| with_app(App::reset_keys));
        on_main(settings::refresh);
        run.wait("resetting brings back cmd-T", 5, || title().as_deref() == Some("⌘T"));
        on_main(|| {
            let Some(w) = settings::shown() else { return };
            let none = NSString::from_str("");
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyDown,
                NSPoint::new(0.0, 0.0),
                NSEventModifierFlags::empty(),
                0.0,
                w.windowNumber(),
                None,
                &none,
                &none,
                false,
                ESC,
            );
            if let Some(e) = event {
                w.sendEvent(&e);
            }
        });
        run.wait("Esc closes the settings window", 5, || on_main(|| settings::shown().is_none()));
    }

    // 메뉴: 앱이 그린다. 목록의 빈 곳을 오른쪽 단추로 누르면(실제 창이 받는 rightMouseDown:) 빈 곳 메뉴가 뜬다.
    // Esc로 닫고, 다시 열어 ↓ 세 번으로 '새로 고침'을 가리키고 Return으로 고른다
    let shown = || on_main(|| with_app(|a| a.menu_shown().map(|(k, _)| k))).flatten();
    right_click_blank();
    if run.wait("a right click on the empty list opens the Sakadu menu", 5, || shown() == Some(MenuKind::Blank)) {
        key(ESC, false);
        run.wait("Esc closes the menu", 5, || shown().is_none());
        right_click_blank();
        run.wait("it opens again", 5, || shown() == Some(MenuKind::Blank));
        for _ in 0..3 {
            key(DOWN, false);
        }
        key(RETURN, false);
        run.wait("Return picks the line and closes the menu", 5, || shown().is_none() && seen().is_some_and(|s| s.ready));
    }

    // 정렬: 지금 칸의 '이름' 머리글을 실제로 누르면(mouseDown:·mouseUp:) 거꾸로 정렬되고, 한 번 더 누르면 돌아온다
    let sort = || on_main(|| with_app(|a| a.active_sort()));
    let before = seen().map(|s| s.names).unwrap_or_default();
    click_header(SortKey::Name);
    let backwards = Some(Sort { key: SortKey::Name, desc: true });
    run.wait("clicking the name header sorts the list backwards", 5, || sort() == backwards && seen().is_some_and(|s| s.names != before));
    click_header(SortKey::Name);
    run.wait("a second click sorts it forwards again", 5, || seen().is_some_and(|s| s.names == before));

    // 새 탭 시작 화면: ⌘T로 시작 화면 탭이 열린다 (설정의 기본). 처음 Return은 첫 칸에 테두리만, 다음 Return은 그곳을 연다
    // (시험에서 다녀간 폴더가 최근 위치에 있다). ⌘W로 그 탭을 닫으면 시험 폴더의 탭으로 돌아온다
    let tabs = || on_main(|| with_app(|a| a.seen().tabs));
    let before = tabs();
    key(KEY_T, true);
    if run.wait("cmd-T opens a start tab", 5, || seen().is_some_and(|s| s.start && s.ready) && tabs().zip(before).is_some_and(|(now, was)| now == was + 1)) {
        key(RETURN, false);
        key(RETURN, false);
        run.wait("Return on the start screen opens the first place", 5, || seen().is_some_and(|s| !s.start && s.ready && !s.dir.is_empty()));
        key(KEY_W, true);
        run.wait("cmd-W closes that tab", 5, || tabs() == before && seen().is_some_and(|s| s.dir == dir && !s.start));
    }

    // 명령 메뉴: ⌘K로 판이 뜨고 입력 칸이 키보드를 갖는다. 치면 그 글이 든 줄만 남고(최근 위치 '폴더', 명령 '새 폴더', 디스크에서 찾기),
    // 입력 칸의 Esc로 닫힌다
    key(KEY_K, true);
    if run.wait("cmd-K opens the command menu with the keyboard in its field", 5, || cmd_state().is_some_and(|s| s.0)) {
        let all = cmd_state().map_or(0, |s| s.1.len());
        let typed = type_in_commands("폴더");
        run.check("typing in the command field", typed.is_ok(), || format!("{typed:?}"));
        run.wait("typing narrows the list to lines with the word", 5, || cmd_state().is_some_and(|s| s.1.len() >= 2 && s.1.len() < all && s.1.iter().all(|n| n.contains("폴더"))));
        let closed = commands_command(sel!(cancelOperation:));
        run.check("Esc in the command field", closed.is_ok(), || format!("{closed:?}"));
        run.wait("Esc closes the menu and hides its field", 5, || {
            cmd_state().is_none() && on_main(|| VIEW.with(|v| v.get().and_then(|v| v.ivars().commands.get().cloned())).is_some_and(|f| f.isHidden()))
        });
    }

    // 사이드바 끌기: ⌃⌘T로 '폴더'와 '가나다.txt'를 이 공간에 고정하고, 첫 줄을 실제로 눌러(mouseDown:) 한 줄 아래로 끌어(mouseDragged:)
    // 떼면(mouseUp:) 순서가 바뀐다. 움직이지 않고 눌렀다 떼면 그 줄을 연다 (누를 때가 아니라 뗄 때 연다)
    let pins = || on_main(|| with_app(|a| a.workspaces().current().map(|w| w.pins.iter().map(|f| f.name.clone()).collect::<Vec<_>>()).unwrap_or_default())).unwrap_or_default();
    for name in ["폴더", "가나다.txt"] {
        if run.select(name) {
            key_with(KEY_T, NSEventModifierFlags::Control | NSEventModifierFlags::Command);
        }
    }
    if run.wait("ctrl-cmd-T pins both to this workspace", 5, || pins() == ["폴더", "가나다.txt"]) {
        let pressed = press_side("폴더", ROW_H);
        run.check("the pinned row is in the sidebar", pressed, || format!("{:?}", pins()));
        run.wait("dragging the first pin below the second swaps them", 5, || pins() == ["가나다.txt", "폴더"]);
        press_side("폴더", 0.0);
        run.wait("a click without moving opens the pinned folder", 5, || seen().is_some_and(|s| s.ready && s.dir == sub));
        key(UP, true);
        run.wait("cmd-up goes back", 5, || seen().is_some_and(|s| s.ready && s.dir == dir));
    }
    run.failed
}

/// 사이드바에서 그 이름의 줄을 실제 창이 받는 길로 누르고(mouseDown:), dy만큼 아래로 몇 번에 나눠 끌고(mouseDragged:), 뗀다(mouseUp:).
/// dy가 0이면 끌지 않고 눌렀다 뗀다. 그 줄이 사이드바에 보이지 않으면 false (창 좌표는 왼쪽 아래가 0)
fn press_side(label: &'static str, dy: f32) -> bool {
    on_main(move || {
        let Some(view) = VIEW.with(|v| v.get().cloned()) else { return false };
        let Some(window) = view.window() else { return false };
        let Some((x, y)) = with_app(|a| a.side_row_center(label)).flatten() else { return false };
        let event = |kind: NSEventType, y: f32| {
            let at = view.convertPoint_toView(NSPoint::new(x as f64, y as f64), None);
            NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(kind, at, NSEventModifierFlags::empty(), 0.0, window.windowNumber(), None, 0, 1, 1.0)
        };
        if let Some(e) = event(NSEventType::LeftMouseDown, y) {
            view.mouseDown(&e);
        }
        if dy != 0.0 {
            for step in [0.25, 0.6, 1.0] {
                if let Some(e) = event(NSEventType::LeftMouseDragged, y + dy * step) {
                    view.mouseDragged(&e);
                }
            }
        }
        if let Some(e) = event(NSEventType::LeftMouseUp, y + dy) {
            view.mouseUp(&e);
        }
        true
    })
}

/// 명령 메뉴가 보이는 것: (입력 칸이 키보드를 가졌는지, 줄의 이름). 닫혀 있으면 None
fn cmd_state() -> Option<(bool, Vec<String>)> {
    on_main(|| {
        let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().commands.get().cloned()));
        let typing = field.is_some_and(|f| !f.isHidden() && f.currentEditor().is_some());
        with_app(|a| a.cmd_seen()).flatten().map(|(_, names, _)| (typing, names))
    })
}

/// 명령 메뉴 입력 칸의 글을 바꾼다: 글을 모두 고르고 그 자리에 친다 (글 상자가 바뀌었다고 알리는 것까지 치는 것과 같다)
fn type_in_commands(text: &str) -> Result<(), &'static str> {
    let text = text.to_string();
    on_main(move || {
        let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().commands.get().cloned())).ok_or("no command field")?;
        let editor = field.currentEditor().ok_or("the command field does not have the keyboard")?;
        // SAFETY: NSText의 selectAll:과 NSResponder의 insertText: (키로 친 글이 가는 길)
        unsafe {
            editor.selectAll(None);
            let _: () = msg_send![&*editor, insertText: &*NSString::from_str(&text)];
        }
        Ok(())
    })
}

/// 명령 메뉴 입력 칸이 키를 받았을 때 하는 그대로 명령(cancelOperation: 등)을 그 위임에 묻는다
fn commands_command(command: Sel) -> Result<(), &'static str> {
    on_main(move || {
        let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().commands.get().cloned())).ok_or("no command field")?;
        let editor = field.currentEditor().ok_or("the command field does not have the keyboard")?;
        // SAFETY: NSResponder의 doCommandBySelector:
        let _: () = unsafe { msg_send![&*editor, doCommandBySelector: command] };
        Ok(())
    })
}

/// 지금 칸의 key 열 머리글 가운데를 실제 창이 받는 길(mouseDown:·mouseUp:)로 누른다 (창 좌표는 왼쪽 아래가 0)
fn click_header(key: SortKey) {
    on_main(move || {
        let Some(view) = VIEW.with(|v| v.get().cloned()) else { return };
        let Some(window) = view.window() else { return };
        let Some((x, y)) = with_app(|a| a.header_point(&view.ivars().measure, key)).flatten() else { return };
        let at = view.convertPoint_toView(NSPoint::new(x as f64, y as f64), None);
        for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                at,
                NSEventModifierFlags::empty(),
                0.0,
                window.windowNumber(),
                None,
                0,
                1,
                1.0,
            );
            let Some(e) = event else { continue };
            if kind == NSEventType::LeftMouseDown {
                view.mouseDown(&e);
            } else {
                view.mouseUp(&e);
            }
        }
    });
}

/// 지금 칸 목록의 아래쪽 빈 곳을 오른쪽 단추로 누른다 (실제 창이 받는 rightMouseDown:. 창 좌표는 왼쪽 아래가 0)
fn right_click_blank() {
    on_main(|| {
        let Some(view) = VIEW.with(|v| v.get().cloned()) else { return };
        let Some(window) = view.window() else { return };
        let at = NSPoint::new(view.bounds().size.width * 0.3, 60.0);
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::RightMouseDown,
            at,
            NSEventModifierFlags::empty(),
            0.0,
            window.windowNumber(),
            None,
            0,
            1,
            1.0,
        );
        if let Some(e) = event {
            view.rightMouseDown(&e);
        }
    });
}

/// 앱 메뉴의 '설정…'이 보내는 동작을 메뉴처럼 보낸다 (받는 쪽을 정하지 않으면 응답 사슬을 따라 앱 위임까지 간다)
fn send_settings_action() {
    on_main(|| {
        let Some(mtm) = MainThreadMarker::new() else { return };
        // SAFETY: NSApplication의 sendAction:to:from:. 받는 쪽은 앱 위임의 sakaduSettings:다
        let _ = unsafe { NSApplication::sharedApplication(mtm).sendAction_to_from(sel!(sakaduSettings:), None, None) };
    });
}

/// 설정 창의 스위치를 누른다 (사람이 누른 것처럼 동작을 보낸다)
fn click_switch(name: &'static str) {
    on_main(move || {
        if let Some(s) = settings::switch(name) {
            // SAFETY: NSControl의 performClick:
            unsafe { s.performClick(None) };
        }
    });
}

/// 설정 창의 스위치가 켜져 있는지
fn switch_on(name: &'static str) -> Option<bool> {
    on_main(move || settings::switch(name).map(|s| s.state() == NSControlStateValueOn))
}

/// 검색이 보이는 것: (열렸는지, 입력 칸이 키보드를 가졌는지(앱이 그렇게 알고 창에서도 그렇다), 결과의 이름, 고른 줄, 다 훑었는지)
fn find_state() -> Option<(bool, bool, Vec<String>, usize, bool)> {
    on_main(|| {
        let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().search.get().cloned()));
        let typing = field.is_some_and(|f| !f.isHidden() && f.currentEditor().is_some());
        with_app(|a| a.find_seen()).map(|(open, keys, names, sel, ready)| (open, keys && typing, names, sel, ready))
    })
}

/// 검색 입력 칸의 글을 바꾼다: 글을 모두 고르고 그 자리에 친다 (글 상자가 바뀌었다고 알리는 것까지 치는 것과 같다)
fn type_in_search(text: &str) -> Result<(), &'static str> {
    let text = text.to_string();
    on_main(move || {
        let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().search.get().cloned())).ok_or("no search field")?;
        let editor = field.currentEditor().ok_or("the search field does not have the keyboard")?;
        // SAFETY: NSText의 selectAll:과 NSResponder의 insertText: (키로 친 글이 가는 길)
        unsafe {
            editor.selectAll(None);
            let _: () = msg_send![&*editor, insertText: &*NSString::from_str(&text)];
        }
        Ok(())
    })
}

/// 검색 입력 칸이 키를 받았을 때 하는 그대로 명령(moveDown:, insertNewline:, cancelOperation:)을 그 위임에 묻는다
fn search_command(command: Sel) -> Result<(), &'static str> {
    on_main(move || {
        let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().search.get().cloned())).ok_or("no search field")?;
        let editor = field.currentEditor().ok_or("the search field does not have the keyboard")?;
        // SAFETY: NSResponder의 doCommandBySelector:
        let _: () = unsafe { msg_send![&*editor, doCommandBySelector: command] };
        Ok(())
    })
}

/// 이름 상자(경로 입력도 같은 상자)에 글을 넣고 Return을 누른다: 필드 편집기가 Return을 받았을 때 하는 그대로
/// 명령(insertNewline:)을 먼저 그 위임(상자, 그리고 이 뷰)에 묻는다
fn type_in_field(text: &str) -> Result<(), &'static str> {
    let text = text.to_string();
    on_main(move || {
        let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().field.get().cloned())).ok_or("no name field")?;
        let editor = field.currentEditor().ok_or("the field does not have the keyboard")?;
        editor.setString(&NSString::from_str(&text));
        // SAFETY: NSResponder의 doCommandBySelector:
        let _: () = unsafe { msg_send![&*editor, doCommandBySelector: sel!(insertNewline:)] };
        Ok(())
    })
}

/// 파일 path를 지금 칸의 row 줄 위로 끌어 와 놓는다 (메인 줄에서): 맥이 하는 순서대로 draggingEntered:, performDragOperation:,
/// draggingEnded:를 뷰에 보낸다. 끌어 낸 쪽은 Finder처럼 모든 일을 허락한다. (그 줄 위에서 받은 답, 놓기를 시작했는지)
fn drop_on_row(path: &str, row: &str) -> (Option<NSDragOperation>, bool) {
    let Some(view) = VIEW.with(|v| v.get().cloned()) else { return (None, false) };
    let Some((x, y)) = with_app(|a| a.row_center(row)).flatten() else { return (None, false) };
    let at = view.convertPoint_toView(NSPoint::new(x as f64, y as f64), None);
    let board = NSPasteboard::pasteboardWithUniqueName();
    board.clearContents();
    let item: Retained<ProtocolObject<dyn NSPasteboardWriting>> = ProtocolObject::from_retained(NSURL::fileURLWithPath(&NSString::from_str(path)));
    if !board.writeObjects(&NSArray::from_retained_slice(&[item])) {
        return (None, false);
    }
    let allowed = NSDragOperation(NSDragOperation::Copy.0 | NSDragOperation::Move.0 | NSDragOperation::Generic.0 | NSDragOperation::Link.0);
    let drag = FakeDrag::new(view.mtm(), board, at, allowed);
    // SAFETY: 이 뷰의 NSDraggingDestination 메서드들 (맥이 끌기 중에 부르는 그대로)
    unsafe {
        let hover: NSDragOperation = msg_send![&*view, draggingEntered: &*drag];
        let dropped: bool = msg_send![&*view, performDragOperation: &*drag];
        let _: () = msg_send![&*view, draggingEnded: &*drag];
        (Some(hover), dropped)
    }
}

/// 미리 보기 창의 Quick Look 보기가 보이면 그 보기에 넘긴 파일 (메인 줄에서)
fn look_view_showing() -> Option<String> {
    let view = VIEW.with(|v| v.get().cloned())?;
    let shown = view.ivars().look_view.get().cloned().flatten().is_some_and(|q| !q.isHidden());
    shown.then(|| view.ivars().look_item.borrow().as_ref().map(|(p, _)| p.clone())).flatten()
}

/// 이름 상자가 보이면 그 글
fn field_text() -> Option<String> {
    on_main(|| {
        let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().field.get().cloned()))?;
        (!field.isHidden()).then(|| field.stringValue().to_string())
    })
}
