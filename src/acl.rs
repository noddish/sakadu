//! 검색 서비스(SYSTEM)가 요청한 Windows 계정이 볼 수 있는 항목만 돌려주게 한다.
//! 항목이 보이려면 드라이브 루트부터 그 항목을 담은 폴더까지 모두 그 계정이 "폴더 내용 보기"(FILE_LIST_DIRECTORY)를
//! 할 수 있어야 한다 (탐색기로 열어 볼 수 있는 범위와 같다). 볼 수 없는 항목은 경로·순위·전체 개수에도 넣지 않는다.
//!
//! 폴더마다 판정은 로그온 세션별로 기억해 둔다. 폴더가 새로 생기거나 지워지면 그 칸만,
//! 폴더가 옮겨지거나(이름 바꾸기 포함) 권한이 바뀌면 그 폴더와 그 아래 폴더들의 판정을 지우고 다시 한다.
//! 칸마다 세대 번호를 두어, 판정하는 동안 그 칸이 지워졌으면 그 결과는 남기지 않는다.

use crate::index::{ROOT_RECORD, Volume};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::*;
use windows::Win32::Storage::FileSystem::{FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY};
use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
use windows::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};
use windows::core::{BOOL, HSTRING};

/// 칸 값의 아래 2비트: 판정. 나머지: 세대 (지울 때마다 1씩)
const UNKNOWN: u32 = 0;
const ALLOWED: u32 = 1;
const DENIED: u32 = 2;
const STATE: u32 = 3;
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
    /// 권한 판정에 쓰이는 토큰 내용(사용자·그룹과 그 속성·제한 SID·무결성 수준·앱 컨테이너).
    /// 같은 로그온 세션에도 권한을 줄인 토큰 등 여러 토큰이 있을 수 있어, 판정은 이것까지 같을 때만 함께 쓴다
    context: Vec<u8>,
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
            let context = security_context(token.0)?;
            Some(Client { token, session, context })
        }
    }
}

/// GetTokenInformation 결과 (정렬을 맞춘 버퍼)
fn token_info(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Option<Vec<u64>> {
    unsafe {
        let mut len = 0u32;
        let _ = GetTokenInformation(token, class, None, 0, &mut len);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        GetTokenInformation(token, class, Some(buf.as_mut_ptr() as *mut _), len, &mut len).ok()?;
        Some(buf)
    }
}

fn push_sid(out: &mut Vec<u8>, sid: PSID, attrs: u32) {
    if sid.0.is_null() {
        out.push(0);
        return;
    }
    let bytes = unsafe { std::slice::from_raw_parts(sid.0 as *const u8, GetLengthSid(sid) as usize) };
    out.push(bytes.len() as u8);
    out.extend_from_slice(bytes);
    out.extend_from_slice(&attrs.to_le_bytes());
}

fn push_groups(out: &mut Vec<u8>, token: HANDLE, class: TOKEN_INFORMATION_CLASS) {
    let Some(buf) = token_info(token, class) else {
        out.extend_from_slice(&u32::MAX.to_le_bytes());
        return;
    };
    unsafe {
        let g = &*(buf.as_ptr() as *const TOKEN_GROUPS);
        out.extend_from_slice(&g.GroupCount.to_le_bytes());
        for sa in std::slice::from_raw_parts(g.Groups.as_ptr(), g.GroupCount as usize) {
            push_sid(out, sa.Sid, sa.Attributes);
        }
    }
}

/// AccessCheck 결과를 좌우하는 토큰 내용을 이어 붙인 값 (같으면 판정도 같다)
fn security_context(token: HANDLE) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    unsafe {
        let user = token_info(token, TokenUser)?;
        let u = &*(user.as_ptr() as *const TOKEN_USER);
        push_sid(&mut out, u.User.Sid, u.User.Attributes);
        push_groups(&mut out, token, TokenGroups);
        push_groups(&mut out, token, TokenRestrictedSids);
        match token_info(token, TokenIntegrityLevel) {
            Some(b) => {
                let l = &*(b.as_ptr() as *const TOKEN_MANDATORY_LABEL);
                push_sid(&mut out, l.Label.Sid, l.Label.Attributes);
            }
            None => out.push(0),
        }
        match token_info(token, TokenAppContainerSid) {
            Some(b) => push_sid(&mut out, (*(b.as_ptr() as *const TOKEN_APPCONTAINER_INFORMATION)).TokenAppContainer, 0),
            None => out.push(0),
        }
        push_groups(&mut out, token, TokenCapabilities);
    }
    Some(out)
}

/// 칸별 판정 (세대 + 모름·보임·안 보임). 검색 스레드들이 잠금 없이 함께 읽고 쓴다
type States = Arc<Vec<AtomicU32>>;

/// 판정을 지운다: 세대를 올리고 모름으로. 진행 중이던 판정은 세대가 달라 남지 않는다
fn forget(a: &AtomicU32) {
    let mut v = a.load(Ordering::Acquire);
    while let Err(now) = a.compare_exchange_weak(v, ((v >> 2).wrapping_add(1)) << 2 | UNKNOWN, Ordering::AcqRel, Ordering::Acquire) {
        v = now;
    }
}

/// 한 보안 컨텍스트(로그온 세션 + 토큰 내용)의 판정: 드라이브마다 칸별
pub struct Session {
    id: u64,
    context: Vec<u8>,
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

    /// slots의 판정을 지운다 (None이면 그 드라이브 전체)
    fn reset(&self, letter: char, slots: Option<&[u32]>) {
        let vols = self.vols.read().unwrap();
        for (_, states) in vols.iter().filter(|(l, _)| *l == letter) {
            match slots {
                Some(list) => list.iter().filter_map(|&s| states.get(s as usize)).for_each(forget),
                None => states.iter().for_each(forget),
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
    /// 이 계정(로그온 세션과 토큰 내용)의 판정 모음. 처음이면 만든다
    pub fn session(&self, client: Client) -> Arc<Session> {
        let mut list = self.list.lock().unwrap();
        let session = match list.iter().find(|s| s.id == client.session && s.context == client.context).cloned() {
            Some(s) => s,
            None => {
                if list.len() >= MAX_SESSIONS {
                    let oldest = list.iter().enumerate().min_by_key(|(_, s)| *s.used.lock().unwrap()).map(|(i, _)| i).unwrap_or(0);
                    list.remove(oldest);
                }
                let s = Arc::new(Session {
                    id: client.session,
                    context: client.context,
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
                    let grown = |old: &[AtomicU32]| -> States {
                        Arc::new((0..v.slots()).map(|i| AtomicU32::new(old.get(i).map_or(UNKNOWN, |a| a.load(Ordering::Acquire)))).collect())
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

    /// 지금 기억하는 세션들
    pub fn all(&self) -> Vec<Arc<Session>> {
        self.list.lock().unwrap().clone()
    }

    /// 판정을 기억하는 세션이 있는지 (없으면 지울 것도 없다)
    pub fn any(&self) -> bool {
        !self.list.lock().unwrap().is_empty()
    }

    /// 폴더 칸들이 새로 생기거나 지워졌다
    pub fn reset_slots(&self, letter: char, slots: &[u32]) {
        for s in self.list.lock().unwrap().iter() {
            s.reset(letter, Some(slots));
        }
    }

    /// 폴더들이 옮겨졌거나 권한이 바뀌었다: 그 폴더와 그 아래 폴더들의 판정이 달라질 수 있다
    pub fn reset_subtrees(&self, vol: &Volume, dirs: &[u32]) {
        let under = folders_under(vol, dirs);
        self.reset_slots(vol.letter, &under);
    }

    /// 칸 번호가 모두 새로 매겨졌다 (인덱스를 다시 만듦)
    pub fn reset_volume(&self, letter: char) {
        for s in self.list.lock().unwrap().iter() {
            s.reset(letter, None);
        }
    }
}

/// dirs 가운데 하나이거나 그 아래에 있는 폴더 칸들. 폴더마다 위로 한 번씩만 올라가도록 기억하며 훑는다
fn folders_under(vol: &Volume, dirs: &[u32]) -> Vec<u32> {
    const UNSEEN: u8 = 0;
    const YES: u8 = 1;
    const NO: u8 = 2;
    let n = vol.slots();
    let mut memo = vec![UNSEEN; n];
    for &d in dirs {
        if let Some(m) = memo.get_mut(d as usize) {
            *m = YES;
        }
    }
    let mut out = Vec::new();
    let mut path = Vec::new();
    for i in 0..n {
        if !vol.is_dir_slot(i) {
            continue;
        }
        path.clear();
        let mut cur = i as u32;
        let answer = loop {
            match memo.get(cur as usize).copied() {
                Some(YES) => break YES,
                Some(NO) => break NO,
                _ => {}
            }
            path.push(cur);
            if cur == ROOT_RECORD || path.len() > 512 {
                break NO;
            }
            match vol.parent_of(cur) {
                Some(p) => cur = p,
                None => break NO,
            }
        };
        for &c in &path {
            if let Some(m) = memo.get_mut(c as usize) {
                *m = answer;
            }
        }
        if answer == YES {
            out.push(i as u32);
        }
    }
    out
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
fn dir_visible(vol: &Volume, dir: u32, states: &[AtomicU32], check: impl Fn(u32) -> bool) -> bool {
    // 위로 올라가며 아는 칸을 찾고, 모르는 칸은 (그때 본 값과 함께) 모아 위에서부터 판정한다
    let mut chain: Vec<(u32, Option<u32>)> = Vec::new();
    let mut cur = dir;
    let mut ok = loop {
        let seen = states.get(cur as usize).map(|a| a.load(Ordering::Acquire));
        match seen.map(|v| v & STATE) {
            Some(ALLOWED) => break true,
            Some(DENIED) => break false,
            _ => {}
        }
        chain.push((cur, seen));
        if cur == ROOT_RECORD {
            break true;
        }
        match vol.parent_of(cur) {
            Some(p) if chain.len() < 512 => cur = p,
            // 부모를 모르는 칸(인덱스가 아직 따라오지 못함 등)은 보이지 않는 것으로
            _ => break false,
        }
    };
    for (d, seen) in chain.into_iter().rev() {
        ok = ok && check(d);
        // 판정하는 동안 이 칸이 지워졌으면(세대가 바뀜) 남기지 않는다
        if let (Some(a), Some(seen)) = (states.get(d as usize), seen) {
            let _ = a.compare_exchange(seen, (seen & !STATE) | if ok { ALLOWED } else { DENIED }, Ordering::AcqRel, Ordering::Relaxed);
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
        let states: Vec<AtomicU32> = (0..v.slots()).map(|_| AtomicU32::new(UNKNOWN)).collect();
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
        // 권한이 바뀌면(alice 폴더와 그 아래를 지움) 다시 묻는다
        let under = folders_under(&v, &[101]);
        assert_eq!(under, [101, 102], "alice and its subfolders only");
        under.iter().for_each(|&s| forget(&states[s as usize]));
        assert!(dir_visible(&v, 102, &states, |_| true));
    }

    /// 판정하는 동안 그 칸이 지워지면(권한·폴더가 바뀜) 늦게 끝난 판정은 남지 않는다
    #[test]
    fn late_result_is_dropped_after_reset() {
        let v = sample();
        let states: Vec<AtomicU32> = (0..v.slots()).map(|_| AtomicU32::new(UNKNOWN)).collect();
        // alice 폴더를 판정하는 도중에 그 폴더의 권한이 바뀌었다
        let seen = dir_visible(&v, 101, &states, |d| {
            if d == 101 {
                forget(&states[101]);
            }
            true
        });
        assert!(seen, "this query still answers with what it checked");
        assert_eq!(states[101].load(Ordering::Acquire) & STATE, UNKNOWN, "but the stale answer is not kept");
        assert_eq!(states[100].load(Ordering::Acquire) & STATE, ALLOWED, "untouched parent is kept");
        // 다음 판정은 새로 묻는다
        assert!(!dir_visible(&v, 101, &states, |d| d != 101));
        assert_eq!(states[101].load(Ordering::Acquire) & STATE, DENIED);
    }

    /// 이 프로세스 계정의 가장 토큰 (파이프 상대 대신)
    fn current_client() -> Client {
        unsafe {
            let mut tok = HANDLE::default();
            windows::Win32::System::Threading::OpenProcessToken(windows::Win32::System::Threading::GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut tok)
                .unwrap();
            client_from(tok)
        }
    }

    /// 주 토큰을 판정용 가장 토큰으로 바꿔 Client로 (주 토큰은 닫는다)
    fn client_from(primary: HANDLE) -> Client {
        unsafe {
            let mut imp = HANDLE::default();
            DuplicateToken(primary, SecurityIdentification, &mut imp).unwrap();
            let _ = CloseHandle(primary);
            let mut stats = TOKEN_STATISTICS::default();
            let mut len = 0u32;
            GetTokenInformation(imp, TokenStatistics, Some(&mut stats as *mut _ as *mut _), size_of::<TOKEN_STATISTICS>() as u32, &mut len).unwrap();
            let session = (stats.AuthenticationId.HighPart as u32 as u64) << 32 | stats.AuthenticationId.LowPart as u64;
            Client { context: security_context(imp).unwrap(), token: Token(imp), session }
        }
    }

    /// 같은 로그온 세션이라도 토큰 내용이 다르면 판정을 함께 쓰지 않는다
    #[test]
    fn same_logon_different_tokens_do_not_share() {
        let fake = |context: &[u8]| Client { token: Token(HANDLE::default()), session: 7, context: context.to_vec() };
        let sessions = Sessions::default();
        let a = sessions.session(fake(b"user+groups"));
        let again = sessions.session(fake(b"user+groups"));
        let restricted = sessions.session(fake(b"user+groups(deny-only)"));
        assert!(Arc::ptr_eq(&a, &again), "same context reuses the cache");
        assert!(!Arc::ptr_eq(&a, &restricted), "restricted token gets its own cache");
    }

    /// 실제 토큰: 같은 로그온 세션에서 자기 SID를 거부 전용으로 줄인 토큰은 자기 프로필 안을 볼 수 없고,
    /// 먼저 보통 토큰으로 판정해 둔 결과를 물려받지도 않는다 (cargo test restricted_token -- --ignored)
    #[test]
    #[ignore]
    fn restricted_token_in_same_logon_gets_own_answer() {
        use crate::index::{Query, search_visible};
        let me = std::env::var("USERPROFILE").unwrap();
        let mut v = Volume::new('C');
        let (mut parent, mut frn) = (5u64, 200u64);
        for comp in me[3..].split('\\') {
            v.upsert(frn, parent, comp, true);
            parent = frn;
            frn += 1;
        }
        v.upsert(frn, parent, "sakadu-acl-토큰시험.txt", false);
        let normal = current_client();
        let restricted = unsafe {
            let mut tok = HANDLE::default();
            windows::Win32::System::Threading::OpenProcessToken(
                windows::Win32::System::Threading::GetCurrentProcess(),
                TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY,
                &mut tok,
            )
            .unwrap();
            let user = token_info(tok, TokenUser).unwrap();
            let me_sid = (*(user.as_ptr() as *const TOKEN_USER)).User;
            let disable = [SID_AND_ATTRIBUTES { Sid: me_sid.Sid, Attributes: 0 }];
            let mut r = HANDLE::default();
            CreateRestrictedToken(tok, CREATE_RESTRICTED_TOKEN_FLAGS(0), Some(&disable), None, None, &mut r).unwrap();
            let _ = CloseHandle(tok);
            client_from(r)
        };
        assert_eq!(normal.session, restricted.session, "same logon session");
        assert_ne!(normal.context, restricted.context);
        let sessions = Sessions::default();
        let vols = [&v];
        let q = Query::parse("sakadu-acl-토큰시험");
        let a = sessions.session(normal);
        let va = sessions.viewer(&a, &vols);
        assert_eq!(search_visible(&vols, &q, 10, &|vi, i| va.visible(&vols, vi, i)).1, 1, "normal token sees it");
        let b = sessions.session(restricted);
        let vb = sessions.viewer(&b, &vols);
        assert_eq!(search_visible(&vols, &q, 10, &|vi, i| vb.visible(&vols, vi, i)).1, 0, "restricted token does not inherit the answer");
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
