//! 폴더 감시: 보이는 폴더에 항목이 생기거나 없어지거나 이름이 바뀌면 알려 준다 (그러면 앱이 그 폴더를 다시 읽는다).
//! 맥의 kqueue로 폴더 하나하나를 본다. 그 폴더 바로 안의 변화만 온다: 하위 폴더 안에서 일어난 일은 오지 않는다
//! (홈 폴더를 볼 때 그 아래 어디선가 파일이 바뀔 때마다 다시 읽지 않는다). 있는 파일의 내용만 바뀐 것(크기, 고친 날짜)도
//! 오지 않는다: 그것은 창이 다시 앞으로 올 때의 다시 읽기가 맡는다.
//! 기다리는 일은 뒷줄 하나가 하고, 폴더를 여는 일은 폴더마다 따로 띄운 줄이 한다: 꺼진 네트워크 폴더를 여느라
//! 오래 걸려도 창도, 다른 폴더의 감시도 멈추지 않는다.
//! 폴더를 읽는 쪽은 읽기 전에 그 폴더의 감시가 걸리기를 기다린다 (Handle::wait_watched): 감시보다 먼저 읽으면
//! 읽은 뒤부터 감시가 걸리기까지의 변화를 놓친다

use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

/// 변화를 모아서 알리는 시간: 파일 여럿이 한꺼번에 생길 때 하나하나마다 알리지 않는다
const SETTLE: Duration = Duration::from_millis(200);
/// 폴더에서 보는 것: 안의 항목이 생기거나 없어지거나 이름이 바뀜(WRITE), 그리고 폴더 자체가 없어지거나 옮겨짐
const WATCHED: u32 = libc::NOTE_WRITE | GONE;
/// 폴더 자체가 지워졌거나, 이름이 바뀌었거나(옮겨졌거나), 디스크가 빠졌다: 열어 둔 것으로는 그 경로를 더 볼 수 없다
const GONE: u32 = libc::NOTE_DELETE | libc::NOTE_RENAME | libc::NOTE_REVOKE;

enum Cmd {
    /// 감시할 폴더 전체: 빠진 것은 그만 보고, 새로 든 것은 보기 시작한다
    Set(Vec<String>),
    /// 따로 띄운 줄이 폴더를 열었다 (또는 열지 못했다)
    Opened(String, std::io::Result<File>),
    /// 그 폴더의 감시가 걸리면 (또는 걸 수 없다고 정해지면) 답한다
    Ready(String, Sender<()>),
    Stop,
    /// 앞서 보낸 것을 다 처리하고 여는 중인 폴더가 없을 때 답한다 (시험에서 기다린다)
    #[cfg(test)]
    Sync(Sender<()>),
}

/// 시험에서 폴더 열기를 이만큼(밀리초) 늦춘다: 감시가 걸리기를 기다리는 쪽이 정말 기다리는지 본다
#[cfg(test)]
static OPEN_DELAY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 감시하는 줄에 말을 거는 손잡이. 다른 줄(폴더를 읽는 줄)로 넘겨 쓸 수 있다
#[derive(Clone)]
pub struct Handle {
    kq: Arc<OwnedFd>,
    tx: Sender<Cmd>,
}

impl Handle {
    /// 보냈으면 true (감시하는 줄이 끝났으면 false)
    fn send(&self, cmd: Cmd) -> bool {
        let sent = self.tx.send(cmd).is_ok();
        if sent {
            wake(&self.kq);
        }
        sent
    }

    /// 폴더 dir의 감시가 걸릴 때까지 기다린다. 걸 수 없다고 정해져도(열지 못했다, 감시해 달라고 한 폴더가 아니다) 돌아온다.
    /// 폴더는 이 뒤에 읽는다: 그래야 읽는 사이와 그 뒤의 변화가 모두 알림으로 온다.
    /// 여는 데 오래 걸리면(꺼진 네트워크 폴더) limit까지만 기다린다
    pub fn wait_watched(&self, dir: &str, limit: Duration) {
        let (tx, rx) = channel();
        if self.send(Cmd::Ready(dir.to_string(), tx)) {
            let _ = rx.recv_timeout(limit);
        }
    }
}

/// 폴더 감시. 없애면 감시하는 줄도 끝난다
pub struct Watcher {
    handle: Handle,
}

impl Watcher {
    /// 감시하는 줄을 띄운다. 감시하는 폴더가 바뀌면 그 줄에서 on_change(폴더)를 부른다. 띄우지 못하면 None
    pub fn start(on_change: impl Fn(String) + Send + 'static) -> Option<Watcher> {
        // SAFETY: 인자가 없는 시스템 호출이다
        let raw = unsafe { libc::kqueue() };
        if raw < 0 {
            sakadu::diag::log(|| format!("watch: kqueue failed: {}", std::io::Error::last_os_error()));
            return None;
        }
        // SAFETY: 방금 받은, 다른 곳에서 쓰지 않는 열린 번호다
        let kq = Arc::new(unsafe { OwnedFd::from_raw_fd(raw) });
        // 기다리는 줄을 깨우는 데 쓸 사건
        if !change(&kq, kev(0, libc::EVFILT_USER, libc::EV_ADD | libc::EV_CLEAR, 0)) {
            sakadu::diag::log(|| format!("watch: no wake event: {}", std::io::Error::last_os_error()));
            return None;
        }
        let (tx, rx) = channel();
        let (thread_kq, thread_tx) = (kq.clone(), tx.clone());
        let spawned = std::thread::Builder::new().name("watch".to_string()).spawn(move || run(thread_kq, thread_tx, rx, on_change));
        if let Err(e) = spawned {
            sakadu::diag::log(|| format!("watch: no thread: {e}"));
            return None;
        }
        Some(Watcher { handle: Handle { kq, tx } })
    }

    /// 감시할 폴더를 이 목록으로 맞춘다
    pub fn set(&self, dirs: Vec<String>) {
        self.handle.send(Cmd::Set(dirs));
    }

    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    /// 앞서 보낸 것을 감시하는 줄이 다 처리할 때까지 기다린다
    #[cfg(test)]
    fn sync(&self) {
        let (tx, rx) = channel();
        self.handle.send(Cmd::Sync(tx));
        rx.recv_timeout(Duration::from_secs(10)).expect("the watcher thread answers");
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.handle.send(Cmd::Stop);
    }
}

fn kev(ident: usize, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
    libc::kevent { ident, filter, flags, fflags, data: 0, udata: std::ptr::null_mut() }
}

/// kqueue에 사건 하나를 등록하거나 일으킨다. 됐으면 true
fn change(kq: &OwnedFd, ev: libc::kevent) -> bool {
    // SAFETY: ev는 이 호출 동안 살아 있고, 받을 사건은 없다고(0칸) 넘긴다
    unsafe { libc::kevent(kq.as_raw_fd(), &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) >= 0 }
}

/// 기다리는 줄을 깨운다
fn wake(kq: &OwnedFd) {
    change(kq, kev(0, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER));
}

/// 폴더를 알림용으로만 연다 (O_EVTONLY: 열어 둬도 그 디스크를 꺼내는 것을 막지 않는다). 링크면 가리키는 폴더가 열린다
fn open_dir(dir: &str) -> std::io::Result<File> {
    File::options().read(true).custom_flags(libc::O_EVTONLY).open(dir)
}

/// 감시하는 줄의 상태
struct State {
    kq: Arc<OwnedFd>,
    tx: Sender<Cmd>,
    /// 감시해 달라는 폴더
    want: Vec<String>,
    /// 열어서 감시하고 있는 폴더 (닫으면 커널이 그 감시도 지운다)
    open: Vec<(String, File)>,
    /// 여는 중인 폴더
    opening: Vec<String>,
    /// 폴더의 감시가 걸리기를 기다리는 쪽 (폴더, 답할 곳)
    ready: Vec<(String, Sender<()>)>,
}

impl State {
    /// 폴더를 따로 띄운 줄에서 연다. 다 열면 Opened로 돌아온다
    fn start_open(&mut self, dir: String) {
        if self.opening.contains(&dir) || self.open.iter().any(|(d, _)| *d == dir) {
            return;
        }
        self.opening.push(dir.clone());
        let (kq, tx) = (self.kq.clone(), self.tx.clone());
        let opener = std::thread::Builder::new().name("watch-open".to_string()).spawn(move || {
            #[cfg(test)]
            {
                std::thread::sleep(Duration::from_millis(OPEN_DELAY.load(std::sync::atomic::Ordering::Relaxed)));
            }
            let result = open_dir(&dir);
            if tx.send(Cmd::Opened(dir, result)).is_ok() {
                wake(&kq);
            }
        });
        if let Err(e) = opener {
            sakadu::diag::log(|| format!("watch: no opener thread: {e}"));
            if let Some(dir) = self.opening.pop() {
                self.answer(&dir);
            }
        }
    }

    /// 폴더 dir의 감시가 걸리기를 기다리던 쪽에 답한다 (걸지 못했어도: 더 기다릴 것이 없다)
    fn answer(&mut self, dir: &str) {
        self.ready.retain(|(d, done)| {
            if d == dir {
                let _ = done.send(());
            }
            d != dir
        });
    }

    /// 폴더 dir의 감시가 걸리면 done에 답한다. 여는 중이 아니면(이미 걸렸거나, 걸 수 없거나, 감시할 폴더가 아니면) 바로 답한다
    fn wait(&mut self, dir: String, done: Sender<()>) {
        if self.opening.contains(&dir) {
            self.ready.push((dir, done));
        } else {
            let _ = done.send(());
        }
    }

    fn set(&mut self, dirs: Vec<String>) {
        self.want = dirs;
        let want = &self.want;
        self.open.retain(|(d, _)| want.contains(d));
        for dir in self.want.clone() {
            self.start_open(dir);
        }
    }

    fn opened(&mut self, dir: String, result: std::io::Result<File>) {
        self.opening.retain(|d| *d != dir);
        // 여는 사이 그 폴더를 더는 보지 않게 됐으면 그대로 닫는다
        if self.want.contains(&dir) && !self.open.iter().any(|(d, _)| *d == dir) {
            match result {
                Ok(file) => {
                    if change(&self.kq, kev(file.as_raw_fd() as usize, libc::EVFILT_VNODE, libc::EV_ADD | libc::EV_CLEAR, WATCHED)) {
                        self.open.push((dir.clone(), file));
                    } else {
                        sakadu::diag::log(|| format!("watch: cannot watch {dir}: {}", std::io::Error::last_os_error()));
                    }
                }
                Err(e) => sakadu::diag::log(|| format!("watch: cannot open {dir}: {e}")),
            }
        }
        // 감시를 건 다음에 답한다: 기다리던 쪽은 이제 그 폴더를 읽는다
        self.answer(&dir);
    }
}

fn run(kq: Arc<OwnedFd>, tx: Sender<Cmd>, rx: Receiver<Cmd>, on_change: impl Fn(String)) {
    let mut st = State { kq, tx, want: Vec::new(), open: Vec::new(), opening: Vec::new(), ready: Vec::new() };
    // 바뀌었다고 알릴 폴더 (모으는 중)와 알릴 때
    let mut dirty: Vec<String> = Vec::new();
    let mut due: Option<Instant> = None;
    // 폴더 자체가 없어져 감시를 잃은 것: 알릴 때 그 경로를 다시 열어 본다
    let mut lost: Vec<String> = Vec::new();
    #[cfg(test)]
    let mut waiting: Vec<Sender<()>> = Vec::new();
    let mut events = [kev(0, 0, 0, 0); 16];
    loop {
        let wait = due.map(|d| d.saturating_duration_since(Instant::now()));
        let ts = wait.map(|w| libc::timespec { tv_sec: w.as_secs() as libc::time_t, tv_nsec: w.subsec_nanos() as libc::c_long });
        let timeout = ts.as_ref().map_or(std::ptr::null(), |t| t as *const libc::timespec);
        // SAFETY: events는 16칸이고, timeout은 없거나 이 호출 동안 살아 있는 ts를 가리킨다
        let n = unsafe { libc::kevent(st.kq.as_raw_fd(), std::ptr::null(), 0, events.as_mut_ptr(), events.len() as libc::c_int, timeout) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            sakadu::diag::log(|| format!("watch: waiting failed: {e}"));
            return;
        }
        // 폴더의 변화를 먼저 본다 (그다음에 받은 명령이 그 폴더를 닫을 수 있다)
        for ev in &events[..n as usize] {
            let (filter, ident, fflags) = (ev.filter, ev.ident, ev.fflags);
            if filter != libc::EVFILT_VNODE {
                continue;
            }
            let Some(i) = st.open.iter().position(|(_, f)| f.as_raw_fd() as usize == ident) else { continue };
            let dir = if fflags & GONE != 0 {
                let (dir, _) = st.open.swap_remove(i);
                if !lost.contains(&dir) {
                    lost.push(dir.clone());
                }
                dir
            } else {
                st.open[i].0.clone()
            };
            if !dirty.contains(&dir) {
                dirty.push(dir);
            }
            due.get_or_insert_with(|| Instant::now() + SETTLE);
        }
        loop {
            match rx.try_recv() {
                Ok(Cmd::Set(dirs)) => st.set(dirs),
                Ok(Cmd::Opened(dir, result)) => st.opened(dir, result),
                Ok(Cmd::Ready(dir, done)) => st.wait(dir, done),
                #[cfg(test)]
                Ok(Cmd::Sync(done)) => waiting.push(done),
                Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => break,
            }
        }
        if due.is_some_and(|d| Instant::now() >= d) {
            due = None;
            // 없어진 폴더의 자리에 새 폴더가 있으면(폴더를 통째로 바꿔 놓았다) 그것을 본다
            for dir in lost.drain(..) {
                if st.want.contains(&dir) {
                    st.start_open(dir);
                }
            }
            for dir in dirty.drain(..) {
                on_change(dir);
            }
        }
        #[cfg(test)]
        {
            if st.opening.is_empty() && due.is_none() {
                for done in waiting.drain(..) {
                    let _ = done.send(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// 알림이 와야 할 때 기다리는 시간 (넉넉히)
    const SOON: Duration = Duration::from_secs(10);
    /// 알림이 오지 않아야 할 때 지켜보는 시간 (모아서 알리는 시간보다 길게)
    const QUIET: Duration = Duration::from_millis(700);

    fn temp(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sakadu-watch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::canonicalize(&root).unwrap()
    }

    fn text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    /// 그 폴더들을 감시하기 시작하고, 다 열어 둘 때까지 기다린다
    fn watching(dirs: &[&Path]) -> (Watcher, Receiver<String>) {
        let (tx, rx) = channel();
        let w = Watcher::start(move |dir| {
            let _ = tx.send(dir);
        })
        .expect("kqueue");
        w.set(dirs.iter().map(|d| text(d)).collect());
        w.sync();
        (w, rx)
    }

    #[test]
    fn tells_when_an_item_is_added_renamed_or_removed() {
        let dir = temp("items");
        let (_w, rx) = watching(&[&dir]);
        let name = text(&dir);
        // 한 일마다: 먼저 조용한지 보고(앞의 일로 알림이 또 오지 않는다), 일을 하고, 알림을 받는다
        let steps: [(&str, &dyn Fn()); 5] = [
            ("a file is made", &|| std::fs::write(dir.join("a.txt"), b"x").unwrap()),
            ("it is renamed", &|| std::fs::rename(dir.join("a.txt"), dir.join("b.txt")).unwrap()),
            ("a folder is made", &|| std::fs::create_dir(dir.join("sub")).unwrap()),
            ("the file is removed", &|| std::fs::remove_file(dir.join("b.txt")).unwrap()),
            ("the folder is moved away", &|| std::fs::rename(dir.join("sub"), dir.parent().unwrap().join(format!("sakadu-watch-moved-{}", std::process::id()))).unwrap()),
        ];
        for (what, step) in steps {
            assert!(rx.recv_timeout(QUIET).is_err(), "quiet before {what}");
            step();
            assert_eq!(rx.recv_timeout(SOON).as_deref(), Ok(name.as_str()), "{what}");
        }
        std::fs::remove_dir_all(dir.parent().unwrap().join(format!("sakadu-watch-moved-{}", std::process::id()))).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 여럿이 한꺼번에 생겨도 하나하나마다 알리지 않는다
    #[test]
    fn many_changes_at_once_are_told_together() {
        let dir = temp("burst");
        let (_w, rx) = watching(&[&dir]);
        for i in 0..200 {
            std::fs::write(dir.join(format!("{i}.txt")), b"x").unwrap();
        }
        assert_eq!(rx.recv_timeout(SOON).as_deref(), Ok(text(&dir).as_str()));
        let mut told = 1;
        while rx.recv_timeout(QUIET).is_ok() {
            told += 1;
        }
        assert!(told < 50, "200 files were told {told} times");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 그 폴더 바로 안의 변화만 온다. 하위 폴더 안의 일과, 있는 파일의 내용만 바뀐 것은 오지 않는다
    /// (뒤의 것은 이 감시 방식의 한계다: 달라지면 문서의 안내도 고친다)
    #[test]
    fn only_the_folder_itself_is_watched() {
        use std::io::Write;
        let dir = temp("depth");
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        let (_w, rx) = watching(&[&dir]);
        std::fs::write(dir.join("sub").join("deep.txt"), b"x").unwrap();
        assert!(rx.recv_timeout(QUIET).is_err(), "a change inside a subfolder is not this folder's change");
        let mut f = std::fs::OpenOptions::new().append(true).open(dir.join("a.txt")).unwrap();
        f.write_all(b"more").unwrap();
        drop(f);
        assert!(rx.recv_timeout(QUIET).is_err(), "new content in a file that is already there is not told");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stops_telling_about_a_folder_it_no_longer_watches() {
        let (a, b) = (temp("left"), temp("right"));
        let (w, rx) = watching(&[&a, &b]);
        w.set(vec![text(&b)]);
        w.sync();
        std::fs::write(a.join("x"), b"x").unwrap();
        assert!(rx.recv_timeout(QUIET).is_err());
        std::fs::write(b.join("x"), b"x").unwrap();
        assert_eq!(rx.recv_timeout(SOON).as_deref(), Ok(text(&b).as_str()));
        // 다시 보기로 하면 다시 온다
        w.set(vec![text(&a), text(&b)]);
        w.sync();
        std::fs::write(a.join("y"), b"y").unwrap();
        assert_eq!(rx.recv_timeout(SOON).as_deref(), Ok(text(&a).as_str()));
        std::fs::remove_dir_all(a).unwrap();
        std::fs::remove_dir_all(b).unwrap();
    }

    /// 감시하던 폴더가 옮겨지고 그 자리에 새 폴더가 생기면 새 폴더를 본다. 옮겨진 옛 폴더는 더 보지 않는다
    #[test]
    fn a_folder_swapped_for_another_is_watched_again() {
        let root = temp("swap");
        let dir = root.join("d");
        std::fs::create_dir(&dir).unwrap();
        let (w, rx) = watching(&[&dir]);
        let name = text(&dir);
        std::fs::rename(&dir, root.join("old")).unwrap();
        std::fs::create_dir(&dir).unwrap();
        assert_eq!(rx.recv_timeout(SOON).as_deref(), Ok(name.as_str()), "the folder itself went away");
        w.sync();
        assert!(rx.recv_timeout(QUIET).is_err());
        std::fs::write(dir.join("x"), b"x").unwrap();
        assert_eq!(rx.recv_timeout(SOON).as_deref(), Ok(name.as_str()), "the new folder at that path is watched");
        std::fs::write(root.join("old").join("y"), b"y").unwrap();
        assert!(rx.recv_timeout(QUIET).is_err(), "the folder that moved away is not");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 읽기 전에 감시가 걸리기를 기다린다: 폴더를 여는 동안에는 돌아오지 않고, 걸리면 돌아온다. 바로 그 뒤의 변화도 알림으로 온다.
    /// 오래 걸리면 정한 시간까지만 기다리고, 열 수 없는 폴더와 감시해 달라고 하지 않은 폴더는 기다릴 것이 없어 곧 돌아온다
    #[test]
    fn waits_until_a_folder_is_watched() {
        use std::sync::atomic::Ordering;
        let dir = temp("ready");
        let (name, missing) = (text(&dir), text(&dir.join("not-here")));
        let (tx, rx) = channel();
        let w = Watcher::start(move |dir| {
            let _ = tx.send(dir);
        })
        .expect("kqueue");
        let handle = w.handle();
        OPEN_DELAY.store(1000, Ordering::Relaxed);
        let began = Instant::now();
        w.set(vec![name.clone(), missing.clone()]);
        handle.wait_watched(&name, Duration::from_millis(50));
        let gave_up = began.elapsed();
        handle.wait_watched(&name, SOON);
        let watched = began.elapsed();
        OPEN_DELAY.store(0, Ordering::Relaxed);
        assert!(gave_up < Duration::from_millis(800), "waits no longer than the limit: {gave_up:?}");
        assert!(watched >= Duration::from_millis(1000), "came back before the folder was opened: {watched:?}");
        std::fs::write(dir.join("x"), b"x").unwrap();
        assert_eq!(rx.recv_timeout(SOON).as_deref(), Ok(name.as_str()), "a change right after the wait is told");
        let began = Instant::now();
        handle.wait_watched(&missing, SOON);
        handle.wait_watched("/sakadu/never/asked", SOON);
        assert!(began.elapsed() < SOON / 2, "nothing to wait for: {:?}", began.elapsed());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 없는 폴더를 보라고 해도 멈추지 않고, 다른 폴더는 그대로 본다
    #[test]
    fn a_folder_that_cannot_be_opened_is_skipped() {
        let dir = temp("missing");
        let (_w, rx) = watching(&[&dir.join("not-here"), &dir]);
        std::fs::write(dir.join("x"), b"x").unwrap();
        assert_eq!(rx.recv_timeout(SOON).as_deref(), Ok(text(&dir).as_str()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
