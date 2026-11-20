//! 맥의 변경 기록(FSEvents): 지켜보는 폴더 아래 어디서든 항목이 바뀌면, 바뀐 폴더의 경로를 묶어서 알려 준다.
//! 폴더 하나하나를 여는 kqueue(watch.rs)와 달리 홈 폴더 전체 같은 큰 범위를 한 번에 본다. 검색 색인을 최신으로 두는 데 쓴다.
//! 알림은 폴더 단위다: 그 폴더 바로 안이 바뀌었다는 뜻이고, 무엇이 바뀌었는지는 그 폴더를 다시 읽어 안다.
//! 알림을 놓쳤다는 표시(MUST_SCAN_SUB_DIRS 등)가 오면 그 아래를 모두 다시 읽어야 한다

use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSString};
use std::ffi::{CStr, c_char, c_void};

/// 바뀐 폴더 하나
#[derive(Clone, Debug)]
pub struct Change {
    pub path: String,
    pub flags: u32,
}

/// 그 아래를 모두 다시 읽어야 한다 (알림이 너무 많아 묶였거나 놓쳤다)
pub const MUST_SCAN_SUB_DIRS: u32 = 0x1;
/// 알림을 놓쳤다 (받는 쪽 또는 커널 쪽에서). MUST_SCAN_SUB_DIRS와 함께 온다
pub const USER_DROPPED: u32 = 0x2;
pub const KERNEL_DROPPED: u32 = 0x4;
/// 지켜보는 폴더 자체(또는 그 위)가 옮겨졌거나 지워졌다 (CREATE_WATCH_ROOT로 만들었을 때)
pub const ROOT_CHANGED: u32 = 0x20;

/// 지금부터의 변화만 받는다
const SINCE_NOW: u64 = u64::MAX;
/// 지켜보는 폴더 자체가 바뀌어도 알린다
const CREATE_WATCH_ROOT: u32 = 0x4;

type StreamRef = *mut c_void;
type Callback = unsafe extern "C" fn(stream: StreamRef, info: *mut c_void, count: usize, paths: *mut c_void, flags: *const u32, ids: *const u64);

/// FSEventStreamContext
#[repr(C)]
struct Context {
    version: isize,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    copy_description: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn FSEventStreamCreate(allocator: *const c_void, callback: Callback, context: *const Context, paths: *const c_void, since: u64, latency: f64, flags: u32) -> StreamRef;
    fn FSEventStreamSetDispatchQueue(stream: StreamRef, queue: *const c_void);
    fn FSEventStreamStart(stream: StreamRef) -> u8;
    fn FSEventStreamFlushSync(stream: StreamRef);
    fn FSEventStreamStop(stream: StreamRef);
    fn FSEventStreamInvalidate(stream: StreamRef);
    fn FSEventStreamRelease(stream: StreamRef);
}

/// 알림을 받아 넘길 곳 (알림 줄에서 부른다)
struct Sink {
    deliver: Box<dyn Fn(Vec<Change>) + Send>,
}

/// 변경 기록 받기. 없애면 그만 받는다
pub struct Stream {
    raw: StreamRef,
    queue: DispatchRetained<DispatchQueue>,
    sink: *mut Sink,
    /// 시작했다 (멈추는 것은 시작한 흐름에만 한다)
    started: bool,
}

impl Stream {
    /// paths 아래의 변화를 받기 시작한다. latency(초)만큼 모아서 알림 줄에서 deliver(바뀐 폴더들)를 부른다. 시작하지 못하면 None
    pub fn start(paths: &[String], latency: f64, deliver: impl Fn(Vec<Change>) + Send + 'static) -> Option<Stream> {
        let list: Vec<Retained<NSString>> = paths.iter().map(|p| NSString::from_str(p)).collect();
        let array = NSArray::from_retained_slice(&list);
        let sink = Box::into_raw(Box::new(Sink { deliver: Box::new(deliver) }));
        let context = Context { version: 0, info: sink.cast(), retain: None, release: None, copy_description: None };
        // SAFETY: NSArray<NSString>은 CFArray<CFString>과 같은 것이다 (toll-free bridged). 만든 쪽이 배열을 복사해 둔다
        let raw = unsafe { FSEventStreamCreate(std::ptr::null(), on_events, &context, Retained::as_ptr(&array).cast(), SINCE_NOW, latency, CREATE_WATCH_ROOT) };
        if raw.is_null() {
            sakadu::diag::log(|| "search: FSEventStreamCreate failed".to_string());
            // SAFETY: 위에서 만든 상자를 아직 아무도 쓰지 않는다
            drop(unsafe { Box::from_raw(sink) });
            return None;
        }
        let queue = DispatchQueue::new("sakadu.search.changes", None);
        let mut stream = Stream { raw, queue, sink, started: false };
        // SAFETY: 알림은 이 줄 하나에서 차례로 온다
        unsafe { FSEventStreamSetDispatchQueue(raw, (&*stream.queue as *const DispatchQueue).cast()) };
        // SAFETY: 만들고 줄을 정한 흐름이다
        stream.started = unsafe { FSEventStreamStart(raw) } != 0;
        if !stream.started {
            sakadu::diag::log(|| "search: FSEventStreamStart failed".to_string());
            return None;
        }
        Some(stream)
    }

    /// 아직 넘기지 않은 알림을 지금 넘긴다 (넘길 때까지 기다린다). 시험과 측정에서 쓴다
    pub fn flush(&self) {
        // SAFETY: 시작한 흐름이다
        unsafe { FSEventStreamFlushSync(self.raw) };
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: 시작한 흐름이면 멈추고, 줄에서 떼고, 놓는다
        unsafe {
            if self.started {
                FSEventStreamStop(self.raw);
            }
            FSEventStreamInvalidate(self.raw);
            FSEventStreamRelease(self.raw);
        }
        // 이미 줄에 올라 있던 알림이 끝나기를 기다린 뒤 받는 곳을 놓는다
        self.queue.exec_sync(|| {});
        // SAFETY: 이제 알림 줄은 이것을 쓰지 않는다
        drop(unsafe { Box::from_raw(self.sink) });
    }
}

/// FSEvents가 알림 줄에서 부른다. 경로는 C 문자열 배열이다 (CFType으로 받는 옵션을 쓰지 않았다)
unsafe extern "C" fn on_events(_stream: StreamRef, info: *mut c_void, count: usize, paths: *mut c_void, flags: *const u32, _ids: *const u64) {
    // SAFETY: info는 Stream이 살아 있는 동안 있는 Sink이고, paths·flags는 count칸이다
    let (sink, paths) = unsafe { (&*(info as *const Sink), paths as *const *const c_char) };
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        // SAFETY: 위와 같다
        let (path, flag) = unsafe { (CStr::from_ptr(*paths.add(k)), *flags.add(k)) };
        out.push(Change { path: path.to_string_lossy().into_owned(), flags: flag });
    }
    (sink.deliver)(out);
}
