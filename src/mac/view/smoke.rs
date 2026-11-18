//! 창 시험 (`sakadu-mac --smoke`): 맥 러너에서 실제 창을 열고, 실제 창이 입력을 받는 길(뷰의 keyDown:, 편집 메뉴가 보내는
//! copy:·paste:·undo:, 이름 상자의 Return)로 폴더 감시, 이름 바꾸기, 되돌리기, 복사·붙여넣기, 휴지통, 훑어보기를 끝까지 돌려 본다.
//! 단위 시험은 App만 돌리고 창 쪽 코드는 타입 검사만 됐다: 그 틈을 메운다. 끌어서 놓기는 흉내 낼 수 없어 하지 않는다.
//! 시험 폴더는 임시 폴더에 새로 만들고 그 안에서만 한다 (휴지통으로 보낸 것은 되돌리기로 다시 꺼낸다).
//! 걸음마다 `ok`나 `FAIL`을 찍고, 하나라도 실패하면 1로, 메인 줄이 답하지 않으면 2로 끝난다

use super::*;
use crate::app::Seen;
use std::path::Path;
use std::sync::mpsc;
use std::time::Instant;

/// 키 번호 (맥 가상 키 코드)
const HOME: u16 = 0x73;
const DOWN: u16 = 0x7D;
const RETURN: u16 = 0x24;
const DELETE: u16 = 0x33;
const Y: u16 = 0x10;

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
    on_main(move || {
        let Some(view) = VIEW.with(|v| v.get().cloned()) else { return };
        let number = view.window().map_or(0, |w| w.windowNumber());
        let flags = if cmd { NSEventModifierFlags::Command } else { NSEventModifierFlags::empty() };
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
        let typed = on_main(|| {
            let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().field.get().cloned())).ok_or("no name field")?;
            let editor = field.currentEditor().ok_or("the name field does not have the keyboard")?;
            editor.setString(&NSString::from_str("메모 바뀜.txt"));
            // Return을 누르면 필드 편집기가 하는 그대로: 명령(insertNewline:)을 먼저 그 위임(이름 상자, 그리고 이 뷰)에 묻는다
            // SAFETY: NSResponder의 doCommandBySelector:
            let _: () = unsafe { msg_send![&*editor, doCommandBySelector: sel!(insertNewline:)] };
            Ok::<(), &'static str>(())
        });
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
    run.failed
}

/// 이름 상자가 보이면 그 글
fn field_text() -> Option<String> {
    on_main(|| {
        let field = VIEW.with(|v| v.get().and_then(|v| v.ivars().field.get().cloned()))?;
        (!field.isHidden()).then(|| field.stringValue().to_string())
    })
}
