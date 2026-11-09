//! 검색 서비스. SYSTEM 권한으로 NTFS 인덱스를 만들어 유지하고, 이름 있는 파이프로 검색 요청에 답한다.
//! 같은 실행 파일을 `--service`(서비스), `--install-service`/`--uninstall-service`(관리자)로 실행한다.
//!
//! 프로토콜(UTF-8, 메시지 모드):
//!   요청 "Q\t<최대 개수>\t<검색어>" → "OK\t<전체 일치>\t<마이크로초>\t<준비됨 0|1>\t<읽지 못한 드라이브>\n" + 줄마다 "D|F\t<경로>"
//!   요청 "S" → "OK\t<준비됨 0|1>\t<항목 수>\t<메모리 MB>\t<읽지 못한 드라이브>\n" + 줄마다 "<드라이브>\t<항목 수>"
//!   요청 "M\t<최대 개수>\t<검색어>\x1f<검색어>…" → "OK\t<검색어 수>\n" + 검색어마다
//!        "R\t<전체 일치>\t<마이크로초>\t<준비됨>\t<읽지 못한 드라이브>\t<줄 수>\n" + 줄마다 "D|F\t<경로>" (여러 줄 검색을 한 번에)
//!   머리줄 뒤쪽 칸과 "M"은 나중에 더한 것이라, 예전 서비스의 응답에 없으면 준비됨·실패 없음으로 보고 "M"은 하나씩 묻는다

use crate::{acl, diag};
use crate::index::{self, Query, Volume, volume_io};
use crate::util::wide;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use windows::Win32::Foundation::*;
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_GENERIC_READ, FILE_SHARE_NONE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FlushFileBuffers, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Services::*;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

pub const SERVICE_NAME: &str = "SakaduIndex";
pub const PIPE_NAME: &str = r"\\.\pipe\SakaduIndex";

struct Shared {
    vols: RwLock<Vec<Volume>>,
    /// 모든 드라이브의 첫 목록 만들기가 끝났는지 (성공·실패 상관없이)
    ready: AtomicBool,
    /// 목록을 만들지 못했거나 갱신이 끊긴 드라이브 글자
    failed: RwLock<String>,
    /// 요청한 계정(로그온 세션)마다 볼 수 있는 폴더 판정
    sessions: acl::Sessions,
}

impl Shared {
    fn new(vols: Vec<Volume>, ready: bool) -> Shared {
        Shared { vols: RwLock::new(vols), ready: AtomicBool::new(ready), failed: RwLock::new(String::new()), sessions: acl::Sessions::default() }
    }

    fn mark_failed(&self, letter: char) {
        let mut f = self.failed.write().unwrap();
        if !f.contains(letter) {
            f.push(letter);
        }
    }
}

static STOP: AtomicBool = AtomicBool::new(false);
static STATUS_HANDLE: AtomicUsize = AtomicUsize::new(0);

// ---------- 인덱스 ----------

/// 모든 NTFS 고정 디스크를 병렬로 인덱싱한다. 끝난 볼륨부터 바로 검색에 넣고 저널 감시를 시작해,
/// 큰 드라이브를 기다리는 동안에도 먼저 끝난 드라이브는 찾을 수 있다
fn start_indexing() -> Arc<Shared> {
    let shared = Arc::new(Shared::new(Vec::new(), false));
    let drives = volume_io::ntfs_drives();
    diag::log(|| format!("index: drives {drives:?}"));
    let s = shared.clone();
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        std::thread::scope(|sc| {
            for &d in &drives {
                let s = s.clone();
                sc.spawn(move || match volume_io::build(d) {
                    Ok(v) => {
                        {
                            let mut vols = s.vols.write().unwrap();
                            vols.push(v);
                            // 검색 결과 순서가 끝난 순서에 따라 달라지지 않게 드라이브 순으로 둔다
                            vols.sort_by_key(|v| v.letter);
                        }
                        std::thread::spawn(move || watch(s, d));
                    }
                    Err(e) => {
                        diag::log(|| format!("index: build {d}: failed: {e}"));
                        s.mark_failed(d);
                    }
                });
            }
        });
        diag::log(|| format!("index: {} entries in {:?}", s.vols.read().unwrap().iter().map(|v| v.len()).sum::<usize>(), t.elapsed()));
        s.ready.store(true, Ordering::SeqCst);
        // 목록을 만드는 동안 먼저 물어 온 계정(로그인하며 켜진 앱 등)은 다 만든 지금 판정을 채워 둔다
        for session in s.sessions.all() {
            if session.begin_warm() {
                warm(&s, &session);
                session.end_warm();
            }
        }
    });
    shared
}

/// 한 볼륨의 USN 저널을 계속 읽어 인덱스에 반영한다.
/// 감시가 끊기면(저널 오류·볼륨을 열 수 없음) 다시 만들 때까지 그 드라이브를 검색에서 뺀다.
/// 변경을 모르는 채로 예전 목록과 권한 판정으로 답하면, 그사이 거둔 권한이 반영되지 않기 때문이다
fn watch(shared: Arc<Shared>, letter: char) {
    let start = shared.vols.read().unwrap().iter().find(|v| v.letter == letter).map(|v| (v.journal_id, v.next_usn));
    let Some(mut cursor) = start else { return };
    while !STOP.load(Ordering::SeqCst) {
        match volume_io::open(letter) {
            Ok(h) => follow(&shared, letter, &h, &mut cursor),
            Err(e) => diag::log(|| format!("index: open {letter}: {e}")),
        }
        if STOP.load(Ordering::SeqCst) {
            return;
        }
        suspend(&shared, letter);
        // 다시 만들 때까지 잠시마다 시도한다 (처음엔 곧, 실패가 이어지면 1분마다)
        let mut wait = std::time::Duration::from_secs(5);
        loop {
            std::thread::sleep(wait);
            if STOP.load(Ordering::SeqCst) {
                return;
            }
            match volume_io::build(letter) {
                Ok(v) => {
                    cursor = (v.journal_id, v.next_usn);
                    resume(&shared, v);
                    break;
                }
                Err(e) => {
                    diag::log(|| format!("index: rebuild {letter}: {e}"));
                    wait = std::time::Duration::from_secs(60);
                }
            }
        }
    }
}

/// 저널을 읽어 반영한다. 오류가 나면 돌아온다
fn follow(shared: &Shared, letter: char, h: &volume_io::VolumeHandle, cursor: &mut (u64, i64)) {
    while !STOP.load(Ordering::SeqCst) {
        let (next, recs) = match volume_io::read_changes(h, cursor.0, cursor.1) {
            Ok(r) => r,
            Err(e) => {
                diag::log(|| format!("index: journal {letter}: {e}"));
                return;
            }
        };
        cursor.1 = next;
        if recs.is_empty() {
            continue;
        }
        let mut vols = shared.vols.write().unwrap();
        let Some(v) = vols.iter_mut().find(|v| v.letter == letter) else { return };
        for r in &recs {
            index::apply(v, r);
        }
        v.next_usn = next;
        v.compact_if_needed();
        // 인덱스를 바꾼 것과 같은 쓰기 잠금 안에서 판정도 지운다. 잠금을 풀고 지우면 그사이 검색이
        // 끼어들어, 아직 지우지 않은 부모의 옛 판정을 바탕으로 자식을 다시 '보임'으로 남길 수 있다
        forget_changed_folders(&shared.sessions, v, &recs);
    }
}

/// 감시가 끊긴 드라이브를 검색에서 빼고 실패로 알린다
fn suspend(shared: &Shared, letter: char) {
    let mut vols = shared.vols.write().unwrap();
    vols.retain(|v| v.letter != letter);
    shared.sessions.reset_volume(letter);
    shared.mark_failed(letter);
    diag::log(|| format!("index: {letter} left out of search until rebuilt"));
}

/// 다시 만든 드라이브를 검색에 넣는다. 칸 번호가 새로 매겨졌으니 판정도 모두 지운다
fn resume(shared: &Shared, v: Volume) {
    let letter = v.letter;
    let mut vols = shared.vols.write().unwrap();
    vols.retain(|x| x.letter != letter);
    vols.push(v);
    vols.sort_by_key(|v| v.letter);
    shared.sessions.reset_volume(letter);
    shared.failed.write().unwrap().retain(|c| c != letter);
    diag::log(|| format!("index: {letter} back in search"));
}

/// 폴더가 생기거나 지워지면 그 칸의 계정별 판정을, 폴더가 옮겨지거나 권한이 바뀌면 그 폴더와 그 아래 폴더들의 판정을 지운다.
/// (드라이브 전체를 지우면 그때마다 첫 검색이 몇 초씩 느려진다.)
/// 파일의 권한은 목록에 보이는지와 상관없다 (담은 폴더를 열 수 있으면 이름이 보인다).
/// 인덱스 쓰기 잠금을 쥔 채로 부른다
fn forget_changed_folders(sessions: &acl::Sessions, v: &Volume, recs: &[index::UsnRecord]) {
    if !sessions.any() {
        return;
    }
    let (mut slots, mut moved) = (Vec::new(), Vec::new());
    for r in recs.iter().filter(|r| r.attrs & index::ATTR_DIR != 0) {
        if r.reason & (index::REASON_RENAME_NEW | index::REASON_SECURITY_CHANGE) != 0 {
            moved.push(index::record_of(r.frn));
        } else if r.reason & (index::REASON_CREATE | index::REASON_DELETE) != 0 {
            slots.push(index::record_of(r.frn));
        }
    }
    if !slots.is_empty() {
        sessions.reset_slots(v.letter, &slots);
    }
    if !moved.is_empty() {
        sessions.reset_subtrees(v, &moved);
    }
}

/// 누가 묻는지. 서비스는 늘 파이프 상대 계정으로 걸러 답한다 (시험에서만 거르지 않음)
enum Asker {
    Session(Arc<acl::Session>),
    #[cfg(test)]
    Everyone,
}

fn handle_request(shared: &Shared, req: &str, asker: Asker) -> String {
    let mut parts = req.splitn(3, '\t');
    match parts.next() {
        Some("S") => {
            let vols = shared.vols.read().unwrap();
            let entries: usize = vols.iter().map(|v| v.len()).sum();
            let mem: usize = vols.iter().map(|v| v.memory()).sum();
            let failed = shared.failed.read().unwrap().clone();
            let mut out = format!("OK\t{}\t{}\t{}\t{failed}\n", shared.ready.load(Ordering::SeqCst) as u8, entries, mem >> 20);
            for v in vols.iter() {
                out.push_str(&format!("{}\t{}\n", v.letter, v.len()));
            }
            out
        }
        Some("Q") => {
            let limit = parts.next().and_then(|n| n.parse().ok()).unwrap_or(100usize).min(500);
            let q = Query::parse(parts.next().unwrap_or(""));
            let t = std::time::Instant::now();
            let vols = shared.vols.read().unwrap();
            let refs: Vec<&Volume> = vols.iter().collect();
            let viewer = viewer(shared, asker, &refs);
            let (hits, total) = index::search_visible(&refs, &q, limit, &|vi, i| viewer.as_ref().is_none_or(|v| v.visible(&refs, vi, i)));
            let ready = shared.ready.load(Ordering::SeqCst) as u8;
            let failed = shared.failed.read().unwrap().clone();
            let mut out = format!("OK\t{}\t{}\t{ready}\t{failed}\n", total, t.elapsed().as_micros());
            push_hits(&mut out, &hits);
            out
        }
        // 여러 검색어를 한 번에 (엑셀 열을 붙여넣은 여러 줄 검색). 파이프를 한 번만 오가고 인덱스도 한 번만 잠근다
        Some("M") => {
            let limit = parts.next().and_then(|n| n.parse().ok()).unwrap_or(100usize).min(500);
            let qs: Vec<&str> = parts.next().unwrap_or("").split('\u{1F}').collect();
            let vols = shared.vols.read().unwrap();
            let refs: Vec<&Volume> = vols.iter().collect();
            let viewer = viewer(shared, asker, &refs);
            let visible = |vi: usize, i: u32| viewer.as_ref().is_none_or(|v| v.visible(&refs, vi, i));
            let ready = shared.ready.load(Ordering::SeqCst) as u8;
            let failed = shared.failed.read().unwrap().clone();
            let mut out = format!("OK\t{}\n", qs.len());
            for q in qs {
                let t = std::time::Instant::now();
                let (hits, total) = index::search_visible(&refs, &Query::parse(q), limit, &visible);
                out.push_str(&format!("R\t{total}\t{}\t{ready}\t{failed}\t{}\n", t.elapsed().as_micros(), hits.len()));
                push_hits(&mut out, &hits);
            }
            out
        }
        _ => "ERR\tunknown request\n".to_string(),
    }
}

/// 이 계정이 모든 폴더를 볼 수 있는지 미리 판정해 둔다. 인덱스 갱신을 오래 막지 않게 조금씩 끊어서 한다
fn warm(shared: &Shared, session: &Arc<acl::Session>) {
    const CHUNK: usize = 20_000;
    let t = std::time::Instant::now();
    let letters: Vec<char> = shared.vols.read().unwrap().iter().map(|v| v.letter).collect();
    for letter in letters {
        let mut start = 0;
        loop {
            if STOP.load(Ordering::SeqCst) {
                return;
            }
            let (end, slots) = {
                let vols = shared.vols.read().unwrap();
                let Some(v) = vols.iter().find(|v| v.letter == letter) else { break };
                let viewer = shared.sessions.viewer(session, &[v]);
                let end = (start + CHUNK).min(v.slots());
                for i in start..end {
                    if v.is_dir_slot(i) {
                        viewer.listable(&[v], 0, i as u32);
                    }
                }
                (end, v.slots())
            };
            if end >= slots {
                break;
            }
            start = end;
            // 인덱스 갱신(쓰기 잠금)이 끼어들 틈을 준다. 읽기 잠금을 곧바로 다시 잡으면
            // Windows의 잠금은 쓰는 쪽을 먼저 들여보내지 않아 새 파일 반영이 몇 초씩 밀린다
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    diag::log(|| format!("acl: warmed in {:?}", t.elapsed()));
}

/// 이 요청의 판정기 (시험용 전체 보기면 None)
fn viewer(shared: &Shared, asker: Asker, vols: &[&Volume]) -> Option<acl::Viewer> {
    match asker {
        Asker::Session(s) => Some(shared.sessions.viewer(&s, vols)),
        #[cfg(test)]
        Asker::Everyone => None,
    }
}

fn push_hits(out: &mut String, hits: &[index::Hit]) {
    for h in hits {
        out.push(if h.is_dir { 'D' } else { 'F' });
        out.push('\t');
        out.push_str(&h.path);
        out.push('\n');
    }
}

// ---------- 파이프 서버 ----------

/// 시스템·관리자는 전체, 로그인한(대화형) 사용자는 읽기/쓰기만 허용
/// 로그인한 사용자(IU)에게 주는 통로 권한: 읽기 전부 + 데이터·속성·확장 속성 쓰기 (FILE_GENERIC_READ | FILE_WRITE_DATA |
/// FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES). GENERIC_WRITE를 주면 FILE_APPEND_DATA(= FILE_CREATE_PIPE_INSTANCE)도 따라가,
/// 같은 PC에 로그인한 다른 사용자가 같은 이름의 가짜 통로를 열어 검색어를 엿보거나 가짜 결과를 줄 수 있다
const IU_PIPE_ACCESS: u32 = 0x0012_019B;
/// 앱이 통로를 열 때 청하는 권한 (IU_PIPE_ACCESS 안에서): 읽기, 요청 쓰기, 메시지 모드로 바꾸기(속성 쓰기)
const CLIENT_PIPE_ACCESS: u32 = FILE_GENERIC_READ.0 | FILE_WRITE_DATA.0 | FILE_WRITE_ATTRIBUTES.0;

fn pipe_sddl() -> String {
    format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;{IU_PIPE_ACCESS:#x};;;IU)")
}

fn pipe_security() -> Option<SECURITY_ATTRIBUTES> {
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(pipe_sddl()), 1, &mut sd, None).ok()? };
    Some(SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() })
}

fn serve(shared: Arc<Shared>) {
    let sa = pipe_security();
    let name = HSTRING::from(PIPE_NAME);
    let mut req = vec![0u8; 64 * 1024];
    // 통로는 처음 한 번만 첫 인스턴스(최대 1개)로 만들고, 요청마다 끊고 다시 받으며 계속 쓴다.
    // 다른 프로세스가 이 이름을 먼저 잡고 있으면 만들기가 실패해 남이 만든 통로에 끼지 않고(앱도 서버를 확인한다),
    // 최대 1개라 누구도 두 번째 인스턴스를 만들지 못한다. 요청마다 닫고 다시 만들면 앱이 아직 핸들을 쥐고 있는 동안
    // 첫 인스턴스를 만들지 못해 검색이 멈췄다
    let mut owned: Option<PipeHandle> = None;
    while !STOP.load(Ordering::SeqCst) {
        let pipe = match &owned {
            Some(p) => p.0,
            None => {
                let h = unsafe {
                    CreateNamedPipeW(
                        &name,
                        PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                        PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                        1,
                        4 << 20,
                        64 * 1024,
                        0,
                        sa.as_ref().map(|s| s as *const _),
                    )
                };
                if h.is_invalid() {
                    diag::log(|| "pipe: CreateNamedPipe failed".to_string());
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    continue;
                }
                owned = Some(PipeHandle(h));
                h
            }
        };
        let connected = unsafe { ConnectNamedPipe(pipe, None) };
        let gone = connected.as_ref().err().is_some_and(|e| e.code() == ERROR_NO_DATA.to_hresult());
        if !gone && connected.as_ref().err().is_some_and(|e| e.code() != ERROR_PIPE_CONNECTED.to_hresult()) {
            // 통로가 망가졌다: 닫고 다시 만든다
            diag::log(|| format!("pipe: ConnectNamedPipe failed ({:?}), recreating", connected.as_ref().err()));
            owned = None;
            continue;
        }
        if !gone {
            let mut got = 0u32;
            if unsafe { ReadFile(pipe, Some(&mut req), Some(&mut got), None) }.is_ok() && !STOP.load(Ordering::SeqCst) {
                let text = String::from_utf8_lossy(&req[..got as usize]).into_owned();
                // 요청한 계정이 볼 수 있는 것만 답한다. 계정을 알 수 없으면 답하지 않는다
                let session = acl::Client::of_pipe(pipe).map(|c| shared.sessions.session(c));
                let resp = match &session {
                    Some(s) => handle_request(&shared, &text, Asker::Session(s.clone())),
                    None => "ERR\t요청한 계정을 확인하지 못했어요\n".to_string(),
                };
                unsafe {
                    let _ = WriteFile(pipe, Some(resp.as_bytes()), None, None);
                    let _ = FlushFileBuffers(pipe);
                }
                // 검색 창을 열면 먼저 상태를 묻는다: 그때 이 계정의 폴더 판정을 미리 채워, 첫 검색이 느리지 않게 한다
                if let Some(s) = session.filter(|s| text.starts_with('S') && s.begin_warm()) {
                    let shared = shared.clone();
                    std::thread::spawn(move || {
                        warm(&shared, &s);
                        s.end_warm();
                    });
                }
            }
        }
        // 앱이 아직 핸들을 쥐고 있어도 끊으면 다음 앱을 받을 수 있다
        unsafe {
            let _ = DisconnectNamedPipe(pipe);
        }
    }
}

/// 관리자 창에서 콘솔 없이 바로 돌리는 개발용 모드 (`--serve`)
pub fn run_foreground() {
    let shared = start_indexing();
    serve(shared);
}

// ---------- Windows 서비스 ----------

fn set_status(state: SERVICE_STATUS_CURRENT_STATE, accept: u32) {
    let h = STATUS_HANDLE.load(Ordering::SeqCst);
    if h == 0 {
        return;
    }
    let st = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: accept,
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 5000,
    };
    unsafe {
        let _ = SetServiceStatus(SERVICE_STATUS_HANDLE(h as *mut c_void), &st);
    }
}

unsafe extern "system" fn control_handler(control: u32, _event: u32, _data: *mut c_void, _ctx: *mut c_void) -> u32 {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            STOP.store(true, Ordering::SeqCst);
            set_status(SERVICE_STOP_PENDING, 0);
            // ConnectNamedPipe에서 기다리는 서버를 깨운다 (자기 통로라 서버 확인은 하지 않는다)
            let _ = transact(PIPE_NAME, b"S", false);
            NO_ERROR.0
        }
        SERVICE_CONTROL_INTERROGATE => NO_ERROR.0,
        _ => ERROR_CALL_NOT_IMPLEMENTED.0,
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    let name = HSTRING::from(SERVICE_NAME);
    let Ok(h) = (unsafe { RegisterServiceCtrlHandlerExW(&name, Some(control_handler), None) }) else { return };
    STATUS_HANDLE.store(h.0 as usize, Ordering::SeqCst);
    set_status(SERVICE_RUNNING, SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN);
    run_foreground();
    set_status(SERVICE_STOPPED, 0);
}

/// 서비스 제어 관리자가 `--service`로 실행했을 때
pub fn run_as_service() -> windows::core::Result<()> {
    let mut name = wide(SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW { lpServiceName: PWSTR(name.as_mut_ptr()), lpServiceProc: Some(service_main) },
        SERVICE_TABLE_ENTRYW::default(),
    ];
    unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) }
}

// ---------- 설치·제거 (관리자 권한으로 실행) ----------

struct Sc(SC_HANDLE);
impl Drop for Sc {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn install_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into())).join("Sakadu")
}

/// 실행 중이면 멈추고 지운다 (재설치·제거 공용)
fn remove_existing(scm: &Sc) {
    unsafe {
        let Ok(svc) = OpenServiceW(scm.0, &HSTRING::from(SERVICE_NAME), SERVICE_ALL_ACCESS) else { return };
        let svc = Sc(svc);
        let mut st = SERVICE_STATUS::default();
        let _ = ControlService(svc.0, SERVICE_CONTROL_STOP, &mut st);
        for _ in 0..50 {
            if QueryServiceStatus(svc.0, &mut st).is_err() || st.dwCurrentState == SERVICE_STOPPED {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let _ = DeleteService(svc.0);
    }
}

pub fn install() -> Result<(), String> {
    unsafe {
        let scm = Sc(OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS).map_err(|e| format!("서비스 관리자를 열 수 없어요: {e}"))?);
        remove_existing(&scm);
        // SYSTEM으로 실행할 파일은 관리자만 쓸 수 있는 위치에 둔다
        let dir = install_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let target = dir.join("sakadu.exe");
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        if me != target {
            // 이전 파일이 아직 잠겨 있을 수 있어 잠시 재시도
            let mut last = None;
            for _ in 0..20 {
                match std::fs::copy(&me, &target) {
                    Ok(_) => {
                        last = None;
                        break;
                    }
                    Err(e) => last = Some(e),
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            if let Some(e) = last {
                return Err(format!("{}: {e}", target.display()));
            }
        }
        let bin = HSTRING::from(format!("\"{}\" --service", target.display()));
        let svc = Sc(CreateServiceW(
            scm.0,
            &HSTRING::from(SERVICE_NAME),
            w!("Sakadu 검색 인덱스"),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            &bin,
            PCWSTR::null(),
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
        )
        .map_err(|e| format!("서비스를 만들 수 없어요: {e}"))?);
        let mut desc = wide("Sakadu 파일 관리자의 빠른 파일 이름 검색(NTFS MFT·USN 저널 인덱스)");
        let info = SERVICE_DESCRIPTIONW { lpDescription: PWSTR(desc.as_mut_ptr()) };
        let _ = ChangeServiceConfig2W(svc.0, SERVICE_CONFIG_DESCRIPTION, Some(&info as *const _ as *const c_void));
        StartServiceW(svc.0, None).map_err(|e| format!("서비스를 시작할 수 없어요: {e}"))?;
    }
    Ok(())
}

/// 서비스가 등록돼 있는지 (관리자 권한 없이 조회). 설정 메뉴 표시용
pub fn is_installed() -> bool {
    unsafe {
        let Ok(scm) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) else { return false };
        let scm = Sc(scm);
        OpenServiceW(scm.0, &HSTRING::from(SERVICE_NAME), SERVICE_QUERY_STATUS).map(Sc).is_ok()
    }
}

pub fn uninstall() -> Result<(), String> {
    unsafe {
        let scm = Sc(OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS).map_err(|e| e.to_string())?);
        remove_existing(&scm);
    }
    let _ = std::fs::remove_dir_all(install_dir());
    Ok(())
}

// ---------- 클라이언트 (앱에서 사용) ----------

#[derive(Debug)]
pub enum SearchError {
    /// 서비스가 설치되지 않았거나 멈춤
    NotRunning,
    Other(String),
}

pub struct SearchResponse {
    pub total: usize,
    pub micros: u64,
    pub hits: Vec<(bool, String)>,
    /// 서비스가 모든 드라이브 목록을 다 만들었는지 (아니면 결과가 일부만일 수 있음)
    pub ready: bool,
    /// 읽지 못한 드라이브 글자 (그 드라이브는 결과에 없음)
    pub failed: String,
}

fn query_raw(req: &str) -> Result<String, SearchError> {
    transact(PIPE_NAME, req.as_bytes(), true)
}

/// 닫을 때 CloseHandle
struct PipeHandle(HANDLE);
impl Drop for PipeHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// 서비스 관리자가 아는 검색 서비스의 프로세스 번호 (돌고 있지 않으면 None)
fn service_pid() -> Option<u32> {
    unsafe {
        let scm = Sc(OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT).ok()?);
        let svc = Sc(OpenServiceW(scm.0, &HSTRING::from(SERVICE_NAME), SERVICE_QUERY_STATUS).ok()?);
        let mut st = SERVICE_STATUS_PROCESS::default();
        let buf = std::slice::from_raw_parts_mut(&mut st as *mut SERVICE_STATUS_PROCESS as *mut u8, size_of::<SERVICE_STATUS_PROCESS>());
        let mut need = 0u32;
        QueryServiceStatusEx(svc.0, SC_STATUS_PROCESS_INFO, Some(buf), &mut need).ok()?;
        (st.dwProcessId != 0).then_some(st.dwProcessId)
    }
}

/// 통로로 요청 하나를 보내고 답을 받는다.
/// 신원 확인만 허락해(SECURITY_IDENTIFICATION) 상대가 이 계정을 흉내 내지 못하게 하고, check_server면 상대가 정말
/// 검색 서비스(서비스 관리자가 아는 프로세스)인지 확인한 뒤에야 검색어를 보낸다
fn transact(name: &str, req: &[u8], check_server: bool) -> Result<String, SearchError> {
    let name = HSTRING::from(name);
    let open = || unsafe { CreateFileW(&name, CLIENT_PIPE_ACCESS, FILE_SHARE_NONE, None, OPEN_EXISTING, SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, None) };
    let mut opened = open();
    if opened.as_ref().err().is_some_and(|e| e.code() == ERROR_PIPE_BUSY.to_hresult()) {
        unsafe {
            let _ = WaitNamedPipeW(&name, 3000);
        }
        opened = open();
    }
    let pipe = match opened {
        Ok(h) => PipeHandle(h),
        Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => return Err(SearchError::NotRunning),
        Err(e) => return Err(SearchError::Other(e.message())),
    };
    if check_server {
        let mut pid = 0u32;
        let known = unsafe { GetNamedPipeServerProcessId(pipe.0, &mut pid) }.is_ok() && service_pid() == Some(pid);
        if !known {
            diag::log(|| format!("pipe: server process {pid} is not the search service"));
            return Err(SearchError::Other("검색 서비스를 확인하지 못했어요".to_string()));
        }
    }
    let mode = PIPE_READMODE_MESSAGE;
    unsafe { SetNamedPipeHandleState(pipe.0, Some(&mode), None, None) }.map_err(|e| SearchError::Other(e.message()))?;
    let mut out = vec![0u8; 4 << 20];
    let mut read = 0u32;
    let sent = unsafe { TransactNamedPipe(pipe.0, Some(req.as_ptr() as *const c_void), req.len() as u32, Some(out.as_mut_ptr() as *mut c_void), out.len() as u32, &mut read, None) };
    let mut total = read as usize;
    match sent {
        Ok(()) => {}
        // 답이 버퍼보다 크면 나머지를 이어 읽는다
        Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => loop {
            out.resize(total + (1 << 20), 0);
            let mut got = 0u32;
            let more = unsafe { ReadFile(pipe.0, Some(&mut out[total..]), Some(&mut got), None) };
            total += got as usize;
            match more {
                Ok(()) => break,
                Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => continue,
                Err(e) => return Err(SearchError::Other(e.message())),
            }
        },
        Err(e) => return Err(SearchError::Other(e.message())),
    }
    Ok(String::from_utf8_lossy(&out[..total]).into_owned())
}

pub fn query(q: &str, limit: usize) -> Result<SearchResponse, SearchError> {
    let text = query_raw(&format!("Q\t{limit}\t{q}"))?;
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().unwrap_or("").split('\t').collect();
    if head.first() != Some(&"OK") {
        return Err(SearchError::Other(head.get(1).unwrap_or(&"응답 오류").to_string()));
    }
    let hits = lines.filter_map(|l| l.split_once('\t')).map(|(k, p)| (k == "D", p.to_string())).collect();
    Ok(SearchResponse {
        total: head.get(1).and_then(|n| n.parse().ok()).unwrap_or(0),
        micros: head.get(2).and_then(|n| n.parse().ok()).unwrap_or(0),
        hits,
        ready: head.get(3) != Some(&"0"),
        failed: head.get(4).map(|s| s.trim().to_string()).unwrap_or_default(),
    })
}

/// 한 번에 묻는 검색어 수 (응답이 파이프 버퍼를 넘지 않게)
const BATCH: usize = 16;

/// 여러 검색어를 묶어서 묻는다. 결과는 검색어 순서대로.
/// 묶음 요청을 모르는 예전 서비스면 하나씩 묻는다
pub fn query_many(qs: &[&str], limit: usize) -> Vec<Result<SearchResponse, SearchError>> {
    if qs.len() <= 1 {
        return qs.iter().map(|q| query(q, limit)).collect();
    }
    let mut out = Vec::with_capacity(qs.len());
    for chunk in qs.chunks(BATCH) {
        let batch = query_raw(&format!("M\t{limit}\t{}", chunk.join("\u{1F}"))).and_then(|text| parse_batch(&text));
        match batch {
            Ok(rs) if rs.len() == chunk.len() => out.extend(rs.into_iter().map(Ok)),
            Err(SearchError::NotRunning) => out.extend(chunk.iter().map(|_| Err(SearchError::NotRunning))),
            _ => out.extend(chunk.iter().map(|q| query(q, limit))),
        }
    }
    out
}

fn parse_batch(text: &str) -> Result<Vec<SearchResponse>, SearchError> {
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().unwrap_or("").split('\t').collect();
    if head.first() != Some(&"OK") {
        return Err(SearchError::Other(head.get(1).unwrap_or(&"응답 오류").to_string()));
    }
    let mut out = Vec::new();
    while let Some(line) = lines.next() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.first() != Some(&"R") || f.len() < 6 {
            return Err(SearchError::Other("응답 오류".to_string()));
        }
        let n: usize = f[5].parse().unwrap_or(0);
        let hits = lines.by_ref().take(n).filter_map(|l| l.split_once('\t')).map(|(k, p)| (k == "D", p.to_string())).collect();
        out.push(SearchResponse {
            total: f[1].parse().unwrap_or(0),
            micros: f[2].parse().unwrap_or(0),
            hits,
            ready: f[3] != "0",
            failed: f[4].trim().to_string(),
        });
    }
    Ok(out)
}

/// (인덱스 준비 완료, 항목 수)
pub fn status() -> Result<(bool, usize), SearchError> {
    let text = query_raw("S")?;
    let head: Vec<&str> = text.lines().next().unwrap_or("").split('\t').collect();
    Ok((head.get(1) == Some(&"1"), head.get(2).and_then(|n| n.parse().ok()).unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 로그인한 사용자에게 통로 인스턴스를 만드는 권한(FILE_CREATE_PIPE_INSTANCE = 4)을 주지 않고, 앱이 청하는 권한은 그 안에 있다
    #[test]
    fn pipe_dacl_withholds_instance_creation() {
        assert_eq!(IU_PIPE_ACCESS & 0x4, 0);
        assert_eq!(CLIENT_PIPE_ACCESS & !IU_PIPE_ACCESS, 0);
        assert!(pipe_sddl().contains("(A;;0x12019b;;;IU)"));
    }

    /// 새 권한의 통로로 요청 하나를 주고받고, 같은 이름의 첫 인스턴스는 다시 만들 수 없다
    #[test]
    fn pipe_round_trip_with_restricted_dacl() {
        let name = format!("\\\\.\\pipe\\SakaduTest-{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], "");
        let sa = pipe_security().expect("security descriptor");
        let h = HSTRING::from(name.as_str());
        let mode = PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        let server = unsafe { CreateNamedPipeW(&h, PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE, mode, PIPE_UNLIMITED_INSTANCES, 4096, 4096, 0, Some(&sa)) };
        assert!(!server.is_invalid());
        // 남이 이미 잡은 이름에는 첫 인스턴스를 만들 수 없다 (서비스가 가짜 통로에 끼지 않음)
        let squat = unsafe { CreateNamedPipeW(&h, PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE, mode, PIPE_UNLIMITED_INSTANCES, 4096, 4096, 0, Some(&sa)) };
        assert!(squat.is_invalid());
        let server = PipeHandle(server);
        let raw = server.0.0 as usize;
        let t = std::thread::spawn(move || unsafe {
            let p = HANDLE(raw as *mut c_void);
            let _ = ConnectNamedPipe(p, None);
            let mut buf = [0u8; 64];
            let mut got = 0u32;
            let _ = ReadFile(p, Some(&mut buf), Some(&mut got), None);
            let reply = format!("OK\t{}", String::from_utf8_lossy(&buf[..got as usize]));
            let _ = WriteFile(p, Some(reply.as_bytes()), None, None);
            let _ = FlushFileBuffers(p);
        });
        let resp = transact(&name, b"S", false).expect("transact");
        t.join().unwrap();
        assert_eq!(resp, "OK\tS");
        drop(server);
    }

    /// 서비스가 쓰는 방식: 최대 1개인 첫 인스턴스를 끊고 다시 받으며 쓴다. 앞 앱이 핸들을 쥐고 있어도 다음 앱이 바로 붙고,
    /// 그동안 누구도 같은 이름의 인스턴스를 더 만들지 못한다
    #[test]
    fn single_instance_serves_next_client_while_previous_holds_handle() {
        let name = format!("\\\\.\\pipe\\SakaduTest1-{}", std::process::id());
        let sa = pipe_security().expect("security descriptor");
        let h = HSTRING::from(name.as_str());
        let mode = PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        let server = unsafe { CreateNamedPipeW(&h, PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE, mode, 1, 4096, 4096, 0, Some(&sa)) };
        assert!(!server.is_invalid());
        let extra = unsafe { CreateNamedPipeW(&h, PIPE_ACCESS_DUPLEX, mode, 1, 4096, 4096, 0, Some(&sa)) };
        assert!(extra.is_invalid(), "a second instance must not be possible");
        let server = PipeHandle(server);
        let raw = server.0.0 as usize;
        let t = std::thread::spawn(move || unsafe {
            let p = HANDLE(raw as *mut c_void);
            for _ in 0..2 {
                let _ = ConnectNamedPipe(p, None);
                let mut buf = [0u8; 64];
                let mut got = 0u32;
                let _ = ReadFile(p, Some(&mut buf), Some(&mut got), None);
                let _ = WriteFile(p, Some(&buf[..got as usize]), None, None);
                let _ = FlushFileBuffers(p);
                let _ = DisconnectNamedPipe(p);
            }
        });
        // 첫 앱은 답을 받은 뒤에도 핸들을 쥐고 있다
        let first = unsafe { CreateFileW(&h, CLIENT_PIPE_ACCESS, FILE_SHARE_NONE, None, OPEN_EXISTING, SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, None) }.expect("first client");
        let first = PipeHandle(first);
        let pmode = PIPE_READMODE_MESSAGE;
        unsafe { SetNamedPipeHandleState(first.0, Some(&pmode), None, None) }.unwrap();
        let mut out = [0u8; 64];
        let mut read = 0u32;
        unsafe { TransactNamedPipe(first.0, Some(b"one".as_ptr() as *const c_void), 3, Some(out.as_mut_ptr() as *mut c_void), 64, &mut read, None) }.unwrap();
        assert_eq!(&out[..read as usize], b"one");
        let started = std::time::Instant::now();
        assert_eq!(transact(&name, b"two", false).expect("second client"), "two");
        assert!(started.elapsed() < std::time::Duration::from_millis(900), "second client waited {:?}", started.elapsed());
        t.join().unwrap();
        drop(first);
        drop(server);
    }

    #[test]
    fn handles_requests() {
        let mut v = Volume::new('C');
        v.upsert(100, 5, "Users", true);
        v.upsert(101, 100, "보고서.docx", false);
        let shared = Shared::new(vec![v], true);
        let resp = handle_request(&shared, "Q\t10\t보고서", Asker::Everyone);
        let mut lines = resp.lines();
        let head: Vec<&str> = lines.next().unwrap().split('\t').collect();
        assert_eq!((head[0], head[1], head[3], head[4]), ("OK", "1", "1", ""));
        assert_eq!(lines.next(), Some("F\tC:\\Users\\보고서.docx"));
        assert!(handle_request(&shared, "S", Asker::Everyone).starts_with("OK\t1\t2\t"));
        assert!(handle_request(&shared, "nope", Asker::Everyone).starts_with("ERR"));
        // 준비 중이거나 읽지 못한 드라이브가 있으면 머리줄로 알린다 ("결과 없음"과 구분)
        shared.ready.store(false, Ordering::SeqCst);
        shared.mark_failed('D');
        let head = handle_request(&shared, "Q\t10\t없는이름", Asker::Everyone);
        assert!(head.starts_with("OK\t0\t") && head.lines().next().unwrap().ends_with("\t0\tD"), "{head}");
    }

    /// 감시가 끊긴 드라이브는 다시 만들 때까지 검색에서 빠지고 실패로 알린다 (오래된 목록·권한으로 답하지 않게)
    #[test]
    fn suspended_drive_is_left_out_until_rebuilt() {
        let make = |letter: char| {
            let mut v = Volume::new(letter);
            v.upsert(100, 5, "보고서.docx", false);
            v
        };
        let shared = Shared::new(vec![make('C'), make('D')], true);
        let total = |s: &Shared| handle_request(s, "Q\t10\t보고서", Asker::Everyone).split('\t').nth(1).unwrap().to_string();
        assert_eq!(total(&shared), "2");
        suspend(&shared, 'D');
        let head = handle_request(&shared, "Q\t10\t보고서", Asker::Everyone);
        assert!(head.starts_with("OK\t1\t") && head.lines().next().unwrap().ends_with("\tD"), "{head}");
        resume(&shared, make('D'));
        assert_eq!(total(&shared), "2");
        assert!(shared.failed.read().unwrap().is_empty());
    }

    /// 여러 검색어를 한 번에: 검색어마다 하나씩 물은 것과 같은 결과가 같은 순서로
    #[test]
    fn batch_matches_single_queries() {
        let mut v = Volume::new('C');
        v.upsert(100, 5, "업무", true);
        v.upsert(101, 100, "견적서_0420.xlsx", false);
        v.upsert(102, 100, "계약서_0420.pdf", false);
        v.upsert(103, 100, "견적서_0507.xlsx", false);
        let shared = Shared::new(vec![v], true);
        let qs = ["견적서", "없는이름", "0420"];
        let batch = parse_batch(&handle_request(&shared, &format!("M\t10\t{}", qs.join("\u{1F}")), Asker::Everyone)).unwrap();
        assert_eq!(batch.len(), 3);
        for (q, b) in qs.iter().zip(&batch) {
            let one = handle_request(&shared, &format!("Q\t10\t{q}"), Asker::Everyone);
            let hits: Vec<(bool, String)> = one.lines().skip(1).filter_map(|l| l.split_once('\t')).map(|(k, p)| (k == "D", p.to_string())).collect();
            assert_eq!(b.hits, hits, "{q}");
            assert_eq!(b.total, one.split('\t').nth(1).unwrap().parse::<usize>().unwrap());
            assert!(b.ready && b.failed.is_empty());
        }
        // 예전 서비스는 "M"을 모른다 → 오류로 받아 하나씩 묻게 된다
        assert!(parse_batch("ERR\tunknown request\n").is_err());
    }

    /// 이 PC의 설치 상태와 실제 응답이 맞는지 (설치 여부는 PC마다 달라 무시 테스트)
    #[test]
    #[ignore]
    fn installed_matches_pipe() {
        assert_eq!(is_installed(), status().is_ok());
    }
}
