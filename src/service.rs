//! 검색 서비스. SYSTEM 권한으로 NTFS 인덱스를 만들어 유지하고, 이름 있는 파이프로 검색 요청에 답한다.
//! 같은 실행 파일을 `--service`(서비스), `--install-service`/`--uninstall-service`(관리자)로 실행한다.
//!
//! 프로토콜(UTF-8, 메시지 모드):
//!   요청 "Q\t<최대 개수>\t<검색어>" → "OK\t<전체 일치>\t<마이크로초>\n" + 줄마다 "D|F\t<경로>"
//!   요청 "S" → "OK\t<준비됨 0|1>\t<항목 수>\t<메모리 MB>\n" + 줄마다 "<드라이브>\t<항목 수>"

use crate::diag;
use crate::index::{self, Query, Volume, volume_io};
use crate::util::wide;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use windows::Win32::Foundation::*;
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile};
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Services::*;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

pub const SERVICE_NAME: &str = "SakaduIndex";
pub const PIPE_NAME: &str = r"\\.\pipe\SakaduIndex";

struct Shared {
    vols: RwLock<Vec<Volume>>,
    ready: AtomicBool,
}

static STOP: AtomicBool = AtomicBool::new(false);
static STATUS_HANDLE: AtomicUsize = AtomicUsize::new(0);

// ---------- 인덱스 ----------

/// 모든 NTFS 고정 디스크를 병렬로 인덱싱하고, 끝난 볼륨마다 저널 감시를 시작한다
fn start_indexing() -> Arc<Shared> {
    let shared = Arc::new(Shared { vols: RwLock::new(Vec::new()), ready: AtomicBool::new(false) });
    let drives = volume_io::ntfs_drives();
    diag::log(|| format!("index: drives {drives:?}"));
    let s = shared.clone();
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let built: Vec<Volume> = std::thread::scope(|sc| {
            let hs: Vec<_> = drives.iter().map(|&d| sc.spawn(move || volume_io::build(d))).collect();
            hs.into_iter().filter_map(|h| h.join().ok()).filter_map(|r| r.inspect_err(|e| diag::log(|| format!("index: build failed: {e}"))).ok()).collect()
        });
        let letters: Vec<char> = built.iter().map(|v| v.letter).collect();
        diag::log(|| format!("index: {} entries in {:?}", built.iter().map(|v| v.len()).sum::<usize>(), t.elapsed()));
        *s.vols.write().unwrap() = built;
        s.ready.store(true, Ordering::SeqCst);
        for letter in letters {
            let s = s.clone();
            std::thread::spawn(move || watch(s, letter));
        }
    });
    shared
}

/// 한 볼륨의 USN 저널을 계속 읽어 인덱스에 반영. 저널이 끊기면 다시 만든다
fn watch(shared: Arc<Shared>, letter: char) {
    let Ok(h) = volume_io::open(letter) else { return };
    let start = {
        let vols = shared.vols.read().unwrap();
        vols.iter().find(|v| v.letter == letter).map(|v| (v.journal_id, v.next_usn))
    };
    let Some((mut journal, mut usn)) = start else { return };
    while !STOP.load(Ordering::SeqCst) {
        match volume_io::read_changes(&h, journal, usn) {
            Ok((next, recs)) => {
                usn = next;
                if recs.is_empty() {
                    continue;
                }
                let mut vols = shared.vols.write().unwrap();
                if let Some(v) = vols.iter_mut().find(|v| v.letter == letter) {
                    for r in &recs {
                        index::apply(v, r);
                    }
                    v.next_usn = usn;
                    v.compact_if_needed();
                }
            }
            Err(e) => {
                diag::log(|| format!("index: journal {letter}: {e}, rebuilding"));
                std::thread::sleep(std::time::Duration::from_secs(5));
                match volume_io::build(letter) {
                    Ok(v) => {
                        journal = v.journal_id;
                        usn = v.next_usn;
                        let mut vols = shared.vols.write().unwrap();
                        if let Some(slot) = vols.iter_mut().find(|x| x.letter == letter) {
                            *slot = v;
                        }
                    }
                    Err(_) => return,
                }
            }
        }
    }
}

fn handle_request(shared: &Shared, req: &str) -> String {
    let mut parts = req.splitn(3, '\t');
    match parts.next() {
        Some("S") => {
            let vols = shared.vols.read().unwrap();
            let entries: usize = vols.iter().map(|v| v.len()).sum();
            let mem: usize = vols.iter().map(|v| v.memory()).sum();
            let mut out = format!("OK\t{}\t{}\t{}\n", shared.ready.load(Ordering::SeqCst) as u8, entries, mem >> 20);
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
            let (hits, total) = index::search(&refs, &q, limit);
            let mut out = format!("OK\t{}\t{}\n", total, t.elapsed().as_micros());
            for h in hits {
                out.push(if h.is_dir { 'D' } else { 'F' });
                out.push('\t');
                out.push_str(&h.path);
                out.push('\n');
            }
            out
        }
        _ => "ERR\tunknown request\n".to_string(),
    }
}

// ---------- 파이프 서버 ----------

/// 시스템·관리자는 전체, 로그인한(대화형) 사용자는 읽기/쓰기만 허용
fn pipe_security() -> Option<SECURITY_ATTRIBUTES> {
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)"), 1, &mut sd, None).ok()? };
    Some(SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() })
}

fn serve(shared: Arc<Shared>) {
    let sa = pipe_security();
    let name = HSTRING::from(PIPE_NAME);
    let mut req = vec![0u8; 64 * 1024];
    while !STOP.load(Ordering::SeqCst) {
        let pipe = unsafe {
            CreateNamedPipeW(
                &name,
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                4 << 20,
                64 * 1024,
                0,
                sa.as_ref().map(|s| s as *const _),
            )
        };
        if pipe.is_invalid() {
            diag::log(|| "pipe: CreateNamedPipe failed".to_string());
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        let connected = unsafe { ConnectNamedPipe(pipe, None) };
        if connected.is_ok() || connected.as_ref().err().is_some_and(|e| e.code() == ERROR_PIPE_CONNECTED.to_hresult()) {
            let mut got = 0u32;
            if unsafe { ReadFile(pipe, Some(&mut req), Some(&mut got), None) }.is_ok() && !STOP.load(Ordering::SeqCst) {
                let text = String::from_utf8_lossy(&req[..got as usize]).into_owned();
                let resp = handle_request(&shared, &text);
                unsafe {
                    let _ = WriteFile(pipe, Some(resp.as_bytes()), None, None);
                    let _ = FlushFileBuffers(pipe);
                }
            }
            unsafe {
                let _ = DisconnectNamedPipe(pipe);
            }
        }
        unsafe {
            let _ = CloseHandle(pipe);
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
            // ConnectNamedPipe에서 기다리는 서버를 깨운다
            let _ = query_raw("S");
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
        let scm = Sc(OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS).map_err(|e| format!("서비스 관리자를 열 수 없습니다: {e}"))?);
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
        .map_err(|e| format!("서비스를 만들 수 없습니다: {e}"))?);
        let mut desc = wide("Sakadu 파일 관리자의 빠른 파일 이름 검색(NTFS MFT·USN 저널 인덱스)");
        let info = SERVICE_DESCRIPTIONW { lpDescription: PWSTR(desc.as_mut_ptr()) };
        let _ = ChangeServiceConfig2W(svc.0, SERVICE_CONFIG_DESCRIPTION, Some(&info as *const _ as *const c_void));
        StartServiceW(svc.0, None).map_err(|e| format!("서비스를 시작할 수 없습니다: {e}"))?;
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
}

fn query_raw(req: &str) -> Result<String, SearchError> {
    static BUF_LEN: OnceLock<usize> = OnceLock::new();
    let len = *BUF_LEN.get_or_init(|| 4 << 20);
    let mut out = vec![0u8; len];
    let mut read = 0u32;
    let ok = unsafe {
        CallNamedPipeW(
            &HSTRING::from(PIPE_NAME),
            Some(req.as_ptr() as *const c_void),
            req.len() as u32,
            Some(out.as_mut_ptr() as *mut c_void),
            out.len() as u32,
            &mut read,
            3000,
        )
    };
    if !ok.as_bool() {
        let e = unsafe { GetLastError() };
        return Err(if e == ERROR_FILE_NOT_FOUND { SearchError::NotRunning } else { SearchError::Other(windows::core::Error::from(e.to_hresult()).message()) });
    }
    Ok(String::from_utf8_lossy(&out[..read as usize]).into_owned())
}

pub fn query(q: &str, limit: usize) -> Result<SearchResponse, SearchError> {
    let text = query_raw(&format!("Q\t{limit}\t{q}"))?;
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().unwrap_or("").split('\t').collect();
    if head.first() != Some(&"OK") {
        return Err(SearchError::Other(head.get(1).unwrap_or(&"응답 오류").to_string()));
    }
    let hits = lines.filter_map(|l| l.split_once('\t')).map(|(k, p)| (k == "D", p.to_string())).collect();
    Ok(SearchResponse { total: head.get(1).and_then(|n| n.parse().ok()).unwrap_or(0), micros: head.get(2).and_then(|n| n.parse().ok()).unwrap_or(0), hits })
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

    #[test]
    fn handles_requests() {
        let mut v = Volume::new('C');
        v.upsert(100, 5, "Users", true);
        v.upsert(101, 100, "보고서.docx", false);
        let shared = Shared { vols: RwLock::new(vec![v]), ready: AtomicBool::new(true) };
        let resp = handle_request(&shared, "Q\t10\t보고서");
        let mut lines = resp.lines();
        assert!(lines.next().unwrap().starts_with("OK\t1\t"));
        assert_eq!(lines.next(), Some("F\tC:\\Users\\보고서.docx"));
        assert!(handle_request(&shared, "S").starts_with("OK\t1\t2\t"));
        assert!(handle_request(&shared, "nope").starts_with("ERR"));
    }

    /// 이 PC의 설치 상태와 실제 응답이 맞는지 (설치 여부는 PC마다 달라 무시 테스트)
    #[test]
    #[ignore]
    fn installed_matches_pipe() {
        assert_eq!(is_installed(), status().is_ok());
    }
}
