//! 검색 서비스(SYSTEM)가 요청한 Windows 계정이 볼 수 있는 항목만 돌려주게 한다.
//! 항목이 보이려면 드라이브 루트부터 그 항목을 담은 폴더까지 모두 그 계정이 "폴더 내용 보기"(FILE_LIST_DIRECTORY)를
//! 할 수 있어야 한다 (탐색기로 열어 볼 수 있는 범위와 같다). 볼 수 없는 항목은 경로·순위·전체 개수에도 넣지 않는다.
//!
//! 폴더마다 판정은 로그온 세션별로 기억해 둔다. 폴더가 새로 생기거나 지워지면 그 칸만,
//! 폴더 이름이 바뀌거나(옮김 포함) 권한이 바뀌면 그 드라이브 판정을 모두 지우고 다시 한다.

use crate::index::{ROOT_RECORD, Volume};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::*;
use windows::Win32::Storage::FileSystem::{FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY};
use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
use windows::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};
use windows::core::{BOOL, HSTRING};

const UNKNOWN: u8 = 0;
const ALLOWED: u8 = 1;
const DENIED: u8 = 2;
/// 기억해 둘 로그온 세션 수 (오래 안 쓴 것부터 버린다)
const MAX_SESSIONS: usize = 8;

/// 요청한 계정의 토큰 (AccessCheck용 가장 토큰)
struct Token(HANDLE);
unsafe impl Send for Token {}
unsafe impl Sync for Token {}
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// 파이프로 요청한 계정
pub struct Client {
    token: Token,
    /// 로그온 세션 (같은 사람이라도 관리자 권한으로 올린 창은 다른 세션)
    session: u64,
}

impl Client {
    /// 파이프 상대의 토큰을 얻는다. 얻지 못하면 None (요청을 받지 않는다)
    pub fn of_pipe(pipe: HANDLE) -> Option<Client> {
        unsafe {
            ImpersonateNamedPipeClient(pipe).ok()?;
            let mut tok = HANDLE::default();
            let opened = OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut tok);
            // 서비스 스레드는 곧바로 SYSTEM으로 돌아온다 (나머지 일은 SYSTEM 권한으로 하고 판정만 이 토큰으로)
            let _ = RevertToSelf();
            opened.ok()?;
            let token = Token(tok);
            let mut stats = TOKEN_STATISTICS::default();
            let mut len = 0u32;
            GetTokenInformation(token.0, TokenStatistics, Some(&mut stats as *mut _ as *mut _), size_of::<TOKEN_STATISTICS>() as u32, &mut len).ok()?;
            let session = (stats.AuthenticationId.HighPart as u32 as u64) << 32 | stats.AuthenticationId.LowPart as u64;
            Some(Client { token, session })
        }
    }
}

/// 칸별 판정 (모름·보임·안 보임). 검색 스레드들이 잠금 없이 함께 읽고 쓴다
type States = Arc<Vec<AtomicU8>>;

/// 한 로그온 세션의 판정: 드라이브마다 칸별
pub struct Session {
    id: u64,
    token: Token,
    vols: RwLock<Vec<(char, States)>>,
    used: Mutex<Instant>,
    /// 미리 판정하는 스레드가 돌고 있는지 (하나만)
    warming: AtomicBool,
}

impl Session {
    /// 미리 판정을 시작해도 되면 true (이미 돌고 있으면 false)
    pub fn begin_warm(&self) -> bool {
        self.warming.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).is_ok()
    }

    pub fn end_warm(&self) {
        self.warming.store(false, Ordering::Release);
    }

    fn reset(&self, letter: char, slot: Option<u32>) {
        let vols = self.vols.read().unwrap();
        for (_, states) in vols.iter().filter(|(l, _)| *l == letter) {
            match slot {
                Some(s) => {
                    if let Some(a) = states.get(s as usize) {
                        a.store(UNKNOWN, Ordering::Relaxed);
                    }
                }
                None => states.iter().for_each(|a| a.store(UNKNOWN, Ordering::Relaxed)),
            }
        }
    }
}

/// 세션별 판정 모음 (서비스에 하나)
#[derive(Default)]
pub struct Sessions {
    list: Mutex<Vec<Arc<Session>>>,
}

impl Sessions {
    /// 이 계정(로그온 세션)의 판정 모음. 처음이면 만든다
    pub fn session(&self, client: Client) -> Arc<Session> {
        let mut list = self.list.lock().unwrap();
        let session = match list.iter().find(|s| s.id == client.session).cloned() {
            Some(s) => s,
            None => {
                if list.len() >= MAX_SESSIONS {
                    let oldest = list.iter().enumerate().min_by_key(|(_, s)| *s.used.lock().unwrap()).map(|(i, _)| i).unwrap_or(0);
                    list.remove(oldest);
                }
                let s = Arc::new(Session {
                    id: client.session,
                    token: client.token,
                    vols: RwLock::new(Vec::new()),
                    used: Mutex::new(Instant::now()),
                    warming: AtomicBool::new(false),
                });
                list.push(s.clone());
                s
            }
        };
        *session.used.lock().unwrap() = Instant::now();
        session
    }

    /// 이 세션으로 vols를 볼 준비: 기억해 둔 판정을 꺼내고, 칸 수가 늘어난 드라이브는 늘린다
    pub fn viewer(&self, session: &Arc<Session>, vols: &[&Volume]) -> Viewer {
        let session = session.clone();
        // 드라이브의 칸이 늘었으면 판정을 옮겨 담은 더 큰 배열로 바꾼다 (진행 중인 다른 요청은 예전 배열을 그대로 쓴다)
        let states = {
            let mut sv = session.vols.write().unwrap();
            vols.iter()
                .map(|v| {
                    let grown = |old: &[AtomicU8]| -> States {
                        Arc::new((0..v.slots()).map(|i| AtomicU8::new(old.get(i).map_or(UNKNOWN, |a| a.load(Ordering::Relaxed)))).collect())
                    };
                    match sv.iter_mut().find(|(l, _)| *l == v.letter) {
                        Some((_, states)) => {
                            if states.len() < v.slots() {
                                *states = grown(states);
                            }
                            states.clone()
                        }
                        None => {
                            let states = grown(&[]);
                            sv.push((v.letter, states.clone()));
                            states
                        }
                    }
                })
                .collect()
        };
        Viewer { session, states }
    }

    /// 폴더 칸 하나가 새로 생기거나 지워졌다
    pub fn reset_slot(&self, letter: char, slot: u32) {
        for s in self.list.lock().unwrap().iter() {
            s.reset(letter, Some(slot));
        }
    }

    /// 폴더가 옮겨졌거나 권한이 바뀌었다: 그 아래 판정이 모두 달라질 수 있다
    pub fn reset_volume(&self, letter: char) {
        for s in self.list.lock().unwrap().iter() {
            s.reset(letter, None);
        }
    }
}

/// 한 요청 동안 쓰는 판정기. 검색 스레드들이 함께 쓴다
pub struct Viewer {
    session: Arc<Session>,
    /// 검색에 넘긴 볼륨 순서대로의 칸별 판정
    states: Vec<States>,
}

impl Viewer {
    /// vols[vi]의 항목 i가 이 계정에게 보이는가
    pub fn visible(&self, vols: &[&Volume], vi: usize, i: u32) -> bool {
        let Some(parent) = vols[vi].parent_of(i) else { return false };
        self.listable(vols, vi, parent)
    }

    /// vols[vi]의 폴더 dir까지 (루트부터) 모두 내용 보기가 되는가. 미리 판정할 때도 쓴다
    pub fn listable(&self, vols: &[&Volume], vi: usize, dir: u32) -> bool {
        dir_visible(vols[vi], dir, &self.states[vi], |d| vols[vi].path(d).is_some_and(|p| can_list(&self.session.token, &p)))
    }
}

/// 루트부터 dir까지 모두 내용 보기가 되는가. 판정은 states에 기억한다 (모르는 칸만 check로 묻는다)
fn dir_visible(vol: &Volume, dir: u32, states: &[AtomicU8], check: impl Fn(u32) -> bool) -> bool {
    let state = |d: u32| states.get(d as usize).map_or(UNKNOWN, |a| a.load(Ordering::Relaxed));
    // 위로 올라가며 아는 칸을 찾고, 모르는 칸은 위에서부터 판정한다
    let mut chain = Vec::new();
    let mut cur = dir;
    let mut ok = loop {
        match state(cur) {
            ALLOWED => break true,
            DENIED => break false,
            _ => {}
        }
        chain.push(cur);
        if cur == ROOT_RECORD {
            break true;
        }
        match vol.parent_of(cur) {
            Some(p) if chain.len() < 512 => cur = p,
            // 부모를 모르는 칸(인덱스가 아직 따라오지 못함 등)은 보이지 않는 것으로
            _ => break false,
        }
    };
    for d in chain.into_iter().rev() {
        ok = ok && check(d);
        if let Some(a) = states.get(d as usize) {
            a.store(if ok { ALLOWED } else { DENIED }, Ordering::Relaxed);
        }
    }
    ok
}

/// token의 계정이 path 폴더의 내용을 볼 수 있는가. 보안 설명자는 SYSTEM으로 읽고 판정만 그 계정으로 한다.
/// 읽지 못하면 볼 수 없는 것으로 (닫힌 쪽으로)
fn can_list(token: &Token, path: &str) -> bool {
    let long = HSTRING::from(format!("\\\\?\\{path}"));
    let info = (OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION).0;
    unsafe {
        // 대부분 1KB 안에 들어온다: 크기를 따로 묻지 않고 한 번에 읽고, 모자랄 때만 다시
        let mut buf = vec![0u64; 128];
        let mut need = 0u32;
        let mut sd = PSECURITY_DESCRIPTOR(buf.as_mut_ptr() as *mut _);
        if !GetFileSecurityW(&long, info, Some(sd), (buf.len() * 8) as u32, &mut need).as_bool() {
            if need as usize <= buf.len() * 8 {
                return false;
            }
            buf = vec![0u64; (need as usize).div_ceil(8)];
            sd = PSECURITY_DESCRIPTOR(buf.as_mut_ptr() as *mut _);
            if !GetFileSecurityW(&long, info, Some(sd), (buf.len() * 8) as u32, &mut need).as_bool() {
                return false;
            }
        }
        let mapping = GENERIC_MAPPING { GenericRead: FILE_GENERIC_READ.0, GenericWrite: FILE_GENERIC_WRITE.0, GenericExecute: FILE_GENERIC_EXECUTE.0, GenericAll: FILE_ALL_ACCESS.0 };
        let mut privileges = [0u8; 256];
        let mut plen = privileges.len() as u32;
        let (mut granted, mut status) = (0u32, BOOL(0));
        AccessCheck(sd, token.0, FILE_LIST_DIRECTORY.0, &mapping, Some(privileges.as_mut_ptr() as *mut PRIVILEGE_SET), &mut plen, &mut granted, &mut status).is_ok()
            && status.as_bool()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C:\ ─ Users ─ alice ─ secret ─ plan.txt
    ///             └ bob ─ notes.txt
    fn sample() -> Volume {
        let mut v = Volume::new('C');
        v.upsert(100, 5, "Users", true);
        v.upsert(101, 100, "alice", true);
        v.upsert(102, 101, "secret", true);
        v.upsert(103, 102, "plan.txt", false);
        v.upsert(104, 100, "bob", true);
        v.upsert(105, 104, "notes.txt", false);
        v
    }

    /// 담은 폴더 하나라도 내용 보기가 안 되면 그 아래는 모두 보이지 않고, 판정은 칸마다 한 번만 묻는다
    #[test]
    fn hidden_below_any_unlistable_folder() {
        let v = sample();
        let states: Vec<AtomicU8> = (0..v.slots()).map(|_| AtomicU8::new(UNKNOWN)).collect();
        let asked = Mutex::new(Vec::new());
        // bob은 alice 폴더를 열 수 없다 (secret 폴더 자체는 열 수 있게 되어 있어도)
        let check = |d: u32| {
            asked.lock().unwrap().push(d);
            d != 101
        };
        assert!(!dir_visible(&v, v.parent_of(103).unwrap(), &states, check), "plan.txt under alice\\secret");
        assert!(!dir_visible(&v, v.parent_of(102).unwrap(), &states, check), "secret folder name itself");
        assert!(dir_visible(&v, v.parent_of(105).unwrap(), &states, check), "bob\\notes.txt");
        assert!(dir_visible(&v, v.parent_of(101).unwrap(), &states, check), "alice folder name is listed in Users");
        let asked = asked.into_inner().unwrap();
        let mut unique = asked.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(asked.len(), unique.len(), "each folder checked once: {asked:?}");
        // 권한이 바뀌면 다시 묻는다
        states[101].store(UNKNOWN, Ordering::Relaxed);
        states[102].store(UNKNOWN, Ordering::Relaxed);
        assert!(dir_visible(&v, 102, &states, |_| true));
    }

    /// 이 프로세스 계정의 가장 토큰 (파이프 상대 대신)
    fn current_client() -> Client {
        unsafe {
            let mut tok = HANDLE::default();
            windows::Win32::System::Threading::OpenProcessToken(windows::Win32::System::Threading::GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut tok)
                .unwrap();
            let mut imp = HANDLE::default();
            DuplicateToken(tok, SecurityIdentification, &mut imp).unwrap();
            let _ = CloseHandle(tok);
            Client { token: Token(imp), session: 1 }
        }
    }

    /// 성능: 실제 폴더 트리(SAKADU_ACL_BENCH_DIR, 기본 C:\Users)로 만든 인덱스에서 넓은 검색어를 이 계정으로 거를 때.
    /// 처음(폴더마다 권한을 읽음)과 그다음(기억해 둔 판정)을 잰다 (cargo test --release bench_filter_real_tree -- --ignored --nocapture)
    #[test]
    #[ignore]
    fn bench_filter_real_tree() {
        use crate::index::{Query, search, search_visible};
        let root = std::env::var("SAKADU_ACL_BENCH_DIR").unwrap_or_else(|_| "C:\\Users".into());
        let mut v = Volume::new('C');
        let mut next = 100u64;
        let mut parent = ROOT_RECORD as u64;
        for comp in root[3..].split('\\').filter(|c| !c.is_empty()) {
            v.upsert(next, parent, comp, true);
            parent = next;
            next += 1;
        }
        let mut stack = vec![(std::path::PathBuf::from(&root), parent)];
        while let Some((dir, frn)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let is_dir = e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false);
                v.upsert(next, frn, &e.file_name().to_string_lossy(), is_dir);
                if is_dir && next < 400_000 {
                    stack.push((e.path(), next));
                }
                next += 1;
            }
        }
        let sessions = Sessions::default();
        let vols = [&v];
        for (round, q) in [(1, "e"), (2, "e"), (3, "a")] {
            let query = Query::parse(q);
            let viewer = sessions.viewer(&sessions.session(current_client()), &vols);
            let t0 = Instant::now();
            let (_, total) = search_visible(&vols, &query, 200, &|vi, i| viewer.visible(&vols, vi, i));
            let filtered = t0.elapsed();
            let t1 = Instant::now();
            let (_, all) = search(&vols, &query, 200);
            println!("round {round} '{q}': {} entries · visible {total} of {all} · filtered {filtered:?} vs unfiltered {:?}", v.len(), t1.elapsed());
        }
    }

    /// 실제 판정: 이 계정(테스트를 돌리는 사람)은 자기 프로필은 보고, 다른 계정 전용 폴더는 보지 못한다
    #[test]
    #[ignore]
    fn current_user_sees_own_profile() {
        unsafe {
            let mut tok = HANDLE::default();
            windows::Win32::System::Threading::OpenProcessToken(windows::Win32::System::Threading::GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut tok)
                .unwrap();
            let mut imp = HANDLE::default();
            DuplicateToken(tok, SecurityIdentification, &mut imp).unwrap();
            let _ = CloseHandle(tok);
            let token = Token(imp);
            let me = std::env::var("USERPROFILE").unwrap();
            assert!(can_list(&token, &me), "own profile");
            assert!(can_list(&token, "C:\\"), "drive root");
            // SYSTEM 전용 폴더는 관리자로 실행해도 내용 보기가 안 된다
            assert!(!can_list(&token, "C:\\System Volume Information"), "system-only folder");
        }
    }

    /// 실제 권한으로 검색 걸러 내기: 볼 수 없는 폴더(System Volume Information) 안의 항목은 결과·전체 개수에 없고,
    /// 자기 프로필 안의 같은 이름은 보인다 (cargo test hides_entries_in_unlistable -- --ignored)
    #[test]
    #[ignore]
    fn hides_entries_in_unlistable_folder() {
        use crate::index::{Query, search_visible};
        let me = std::env::var("USERPROFILE").unwrap();
        let mut v = Volume::new('C');
        v.upsert(100, 5, "System Volume Information", true);
        v.upsert(101, 100, "sakadu-acl-비밀.txt", false);
        let mut parent = 5u64;
        let mut frn = 200u64;
        for comp in me[3..].split('\\') {
            v.upsert(frn, parent, comp, true);
            parent = frn;
            frn += 1;
        }
        v.upsert(frn, parent, "sakadu-acl-비밀.txt", false);
        let sessions = Sessions::default();
        let vols = [&v];
        let viewer = sessions.viewer(&sessions.session(current_client()), &vols);
        let (hits, total) = search_visible(&vols, &Query::parse("sakadu-acl-비밀"), 10, &|vi, i| viewer.visible(&vols, vi, i));
        assert_eq!(total, 1, "hidden entry not counted");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.starts_with(&me), "{}", hits[0].path);
        // 폴더 이름 자체는 C:\를 볼 수 있으니 보인다
        let (_, folder) = search_visible(&vols, &Query::parse("System Volume"), 10, &|vi, i| viewer.visible(&vols, vi, i));
        assert_eq!(folder, 1);
    }
}
