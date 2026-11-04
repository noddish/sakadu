//! 검색에 넣은 폴더: 공유폴더·USB처럼 검색 서비스의 디스크 인덱스 밖에 있는 폴더.
//! 검색 서비스(SYSTEM)는 사용자의 네트워크 연결을 볼 수 없어서 앱이 사용자 권한으로 직접 훑는다.
//! 이름만 메모리에 두고(디스크에 저장하지 않음) 폴더 변경 알림으로 갱신하며, 놓친 변경은 주기적으로 다시 훑어 잡는다.
//! 폴더 목록은 %APPDATA%\Sakadu\search-folders.txt 에 한 줄에 하나씩 저장한다.

use crate::index::{Hit, Query};
use crate::listing::{self, Entry};
use crate::util;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_INCOMPLETE, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::{HRESULT, HSTRING, PCWSTR};

const HEADER: &str = "sakadu-search-folders 1";
/// 변경 알림을 받는 곳도, 알림이 오지 않는 서버를 대비해 이 간격으로 다시 훑는다
const RESCAN_WATCHED: Duration = Duration::from_secs(6 * 3600);
/// 변경 알림을 지원하지 않는 곳은 이 간격으로 다시 훑는다
const RESCAN_UNWATCHED: Duration = Duration::from_secs(30 * 60);
/// 연결되지 않을 때 다시 시도하는 간격
const RETRY: Duration = Duration::from_secs(5 * 60);
const DRIVE_FIXED: u32 = 3;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Scanning,
    Ready(usize),
    Offline,
}

/// 상대 폴더 경로("" = 루트, "a\b") → 그 안의 (이름, 폴더 여부)
type Tree = HashMap<String, Vec<(String, bool)>>;

struct Root {
    path: String,
    tree: RwLock<Tree>,
    state: Mutex<State>,
    stop: AtomicBool,
}

/// 검색에 넣은 폴더 목록. 복제해도 같은 목록을 가리킨다 (검색 스레드에 넘길 때)
#[derive(Clone, Default)]
pub struct Folders {
    roots: Arc<Mutex<Vec<Arc<Root>>>>,
}

impl Folders {
    /// 저장된 폴더를 불러와 백그라운드에서 훑기 시작한다
    pub fn start() -> Folders {
        let f = Folders::default();
        for path in load() {
            f.spawn(path);
        }
        f
    }

    fn spawn(&self, path: String) {
        let root = Arc::new(Root { path, tree: RwLock::new(Tree::new()), state: Mutex::new(State::Scanning), stop: AtomicBool::new(false) });
        self.roots.lock().unwrap().push(root.clone());
        std::thread::spawn(move || run(&root));
    }

    pub fn list(&self) -> Vec<(String, State)> {
        self.roots.lock().unwrap().iter().map(|r| (r.path.clone(), r.state.lock().unwrap().clone())).collect()
    }

    /// path를 포함하는(같거나 상위인) 검색 폴더
    pub fn covering(&self, path: &str) -> Option<String> {
        self.root_for(path).map(|r| r.path.clone())
    }

    fn root_for(&self, path: &str) -> Option<Arc<Root>> {
        self.roots.lock().unwrap().iter().find(|r| is_within(path, &r.path)).cloned()
    }

    /// 넣고 저장한다. 그 아래에 이미 넣어 둔 폴더는 이 폴더에 합친다
    pub fn add(&self, path: &str) -> std::io::Result<()> {
        let path = util::normalize(path);
        self.remove_where(|r| is_within(&r.path, &path));
        self.spawn(path);
        self.save()
    }

    pub fn remove(&self, path: &str) -> std::io::Result<()> {
        self.remove_where(|r| util::eq_path(&r.path, path));
        self.save()
    }

    fn remove_where(&self, f: impl Fn(&Root) -> bool) {
        self.roots.lock().unwrap().retain(|r| {
            let gone = f(r);
            if gone {
                r.stop.store(true, Ordering::Relaxed);
            }
            !gone
        });
    }

    fn save(&self) -> std::io::Result<()> {
        let paths: Vec<String> = self.roots.lock().unwrap().iter().map(|r| r.path.clone()).collect();
        save(&paths)
    }

    /// 다 훑은 폴더의 항목 수 합
    pub fn entry_count(&self) -> usize {
        self.list().iter().map(|(_, s)| if let State::Ready(n) = s { *n } else { 0 }).sum()
    }

    /// (상위 결과, 전체 일치 개수). 점수는 디스크 인덱스 검색과 같은 기준
    pub fn search(&self, q: &Query, limit: usize) -> (Vec<Hit>, usize) {
        let roots = self.roots.lock().unwrap().clone();
        let mut hits = Vec::new();
        for root in &roots {
            let tree = root.tree.read().unwrap();
            for (dir, items) in tree.iter() {
                let base = full_path(&root.path, dir);
                for (name, is_dir) in items {
                    let Some(s) = q.name_score(name, *is_dir) else { continue };
                    let path = util::join(&base, name);
                    if let Some(score) = q.path_score(s, &path) {
                        hits.push(Hit { path, is_dir: *is_dir, score });
                    }
                }
            }
        }
        let total = hits.len();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.len().cmp(&b.path.len())));
        hits.truncate(limit);
        (hits, total)
    }

    /// 폴더를 열어 읽은 목록이 인덱스와 다르면(변경 알림을 놓친 경우) 그 폴더를 다시 읽는다
    pub fn observe(&self, dir: &str, entries: &[Entry]) {
        let Some(root) = self.root_for(dir) else { return };
        let rel = rel_path(&root.path, dir);
        // 큰 공유폴더도 UI가 멈추지 않게 인덱스 쪽을 한 번 모아 두고 맞춰 본다 (항목마다 전체를 훑지 않음)
        let same = root.tree.read().unwrap().get(&rel).is_some_and(|old| {
            let known: HashSet<(&str, bool)> = old.iter().map(|(n, d)| (n.as_str(), *d)).collect();
            old.len() == entries.len() && entries.iter().all(|e| known.contains(&(e.name_string().as_str(), e.is_dir())))
        });
        if !same {
            std::thread::spawn(move || refresh_dir(&root, &rel));
        }
    }
}

/// 검색 서비스의 디스크 인덱스가 다루지 않는 곳인가 (공유폴더, 네트워크·이동식 드라이브, NTFS가 아닌 디스크)
pub fn outside_disk_index(path: &str) -> bool {
    if path.starts_with("\\\\") {
        return true;
    }
    let b = path.as_bytes();
    if b.len() < 2 || b[1] != b':' {
        return false;
    }
    let root = HSTRING::from(format!("{}:\\", &path[..1]));
    unsafe {
        if GetDriveTypeW(&root) != DRIVE_FIXED {
            return true;
        }
        let mut fs = [0u16; 32];
        GetVolumeInformationW(&root, None, None, None, None, Some(&mut fs)).is_ok() && util::from_wide(&fs) != "NTFS"
    }
}

/// path가 root와 같거나 그 아래인가 (대소문자 무시)
fn is_within(path: &str, root: &str) -> bool {
    let (p, r) = (path.to_lowercase(), root.to_lowercase());
    let r = r.trim_end_matches('\\');
    p == r || p.starts_with(&format!("{r}\\"))
}

fn rel_path(root: &str, path: &str) -> String {
    path.get(root.trim_end_matches('\\').len()..).unwrap_or("").trim_matches('\\').to_string()
}

fn full_path(root: &str, rel: &str) -> String {
    if rel.is_empty() { root.to_string() } else { util::join(root, rel) }
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}\\{name}") }
}

fn count(tree: &Tree) -> usize {
    tree.values().map(Vec::len).sum()
}

fn sleep(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while !stop.load(Ordering::Relaxed) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// 폴더 하나를 맡는 스레드: 훑고, 변경을 따라가고, 때가 되면 다시 훑는다. 목록에서 빼면 끝난다
fn run(root: &Root) {
    let stop = &root.stop;
    while !stop.load(Ordering::Relaxed) {
        let mut tree = Tree::new();
        let started = Instant::now();
        match scan(&root.path, "", stop, &mut tree) {
            Ok(()) => {
                let n = count(&tree);
                crate::diag::log(|| format!("search folder {}: {n} entries in {:?}", root.path, started.elapsed()));
                *root.tree.write().unwrap() = tree;
                *root.state.lock().unwrap() = State::Ready(n);
            }
            Err(_) if stop.load(Ordering::Relaxed) => return,
            Err(e) => {
                // 연결이 끊긴 동안에도 마지막으로 훑은 내용은 검색에 남겨 둔다
                crate::diag::log(|| format!("search folder {}: {e}", root.path));
                *root.state.lock().unwrap() = State::Offline;
                sleep(stop, RETRY);
                continue;
            }
        }
        if !watch(root, RESCAN_WATCHED) {
            sleep(stop, RESCAN_UNWATCHED);
        }
    }
}

/// rel 폴더와 그 아래를 모두 훑어 tree에 넣는다. rel 자체를 못 열면 Err, 그 아래에서 못 여는 폴더는 비워 둔다
fn scan(root: &str, rel: &str, stop: &AtomicBool, tree: &mut Tree) -> Result<(), String> {
    let mut stack = vec![rel.to_string()];
    let mut top = true;
    while let Some(dir) = stack.pop() {
        if stop.load(Ordering::Relaxed) {
            return Err("멈춤".into());
        }
        let items = match list(root, &dir) {
            Ok(items) => items,
            Err(e) if top => return Err(e),
            Err(_) => Vec::new(),
        };
        top = false;
        for (name, _, descend) in &items {
            if *descend {
                stack.push(join_rel(&dir, name));
            }
        }
        tree.insert(dir, items.into_iter().map(|(n, d, _)| (n, d)).collect());
    }
    Ok(())
}

/// 폴더 하나의 (이름, 폴더 여부, 들어가서 훑을지). 연결 지점(정션·심볼릭 링크)은 고리가 생길 수 있어 들어가지 않는다
fn list(root: &str, rel: &str) -> Result<Vec<(String, bool, bool)>, String> {
    Ok(listing::list_dir(&full_path(root, rel))?
        .iter()
        .map(|e| (e.name_string(), e.is_dir(), e.is_dir() && e.attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0))
        .collect())
}

/// 폴더 하나를 다시 읽어 바뀐 만큼 고친다. 새 하위 폴더는 통째로 훑고, 없어진 하위 폴더는 그 아래까지 지운다
fn refresh_dir(root: &Root, rel: &str) {
    // 아직 모르는 폴더는 상위 폴더를 다시 읽을 때 함께 훑는다
    let Some(old) = root.tree.read().unwrap().get(rel).cloned() else { return };
    // 못 읽으면(방금 지워진 폴더 등) 상위 폴더 쪽 변경에서 정리된다
    let Ok(items) = list(&root.path, rel) else { return };
    let old_dirs: HashSet<&str> = old.iter().filter(|(_, d)| *d).map(|(n, _)| n.as_str()).collect();
    let new_dirs: HashSet<&str> = items.iter().filter(|(_, d, _)| *d).map(|(n, ..)| n.as_str()).collect();
    let mut added = Tree::new();
    for (name, _, descend) in &items {
        if *descend && !old_dirs.contains(name.as_str()) {
            let _ = scan(&root.path, &join_rel(rel, name), &root.stop, &mut added);
        }
    }
    let mut tree = root.tree.write().unwrap();
    for (name, is_dir) in &old {
        if *is_dir && !new_dirs.contains(name.as_str()) {
            let gone = join_rel(rel, name);
            let prefix = format!("{gone}\\");
            tree.retain(|k, _| *k != gone && !k.starts_with(&prefix));
        }
    }
    tree.insert(rel.to_string(), items.into_iter().map(|(n, d, _)| (n, d)).collect());
    tree.extend(added);
    let n = count(&tree);
    drop(tree);
    let mut state = root.state.lock().unwrap();
    if matches!(*state, State::Ready(_)) {
        *state = State::Ready(n);
    }
}

/// 변경 알림을 받아 바뀐 폴더만 다시 읽는다. 다시 훑을 때가 됐거나 알림을 놓쳤거나(버퍼 넘침) 연결이 끊기면 true,
/// 이 폴더가 변경 알림을 지원하지 않으면 false. 멈추라고 하면 true를 돌려주고 run이 끝낸다
fn watch(root: &Root, rescan_after: Duration) -> bool {
    let stop = &root.stop;
    unsafe {
        let Ok(dir) = CreateFileW(
            &HSTRING::from(root.path.as_str()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        ) else {
            return false;
        };
        let Ok(event) = CreateEventW(None, true, false, PCWSTR::null()) else {
            let _ = CloseHandle(dir);
            return false;
        };
        // 네트워크 폴더가 한 번에 받을 수 있는 최대 크기(64KB). DWORD 단위로 잡아 정렬을 맞춘다
        let mut buf = vec![0u32; 16 * 1024];
        let deadline = Instant::now() + rescan_after;
        let mut worked = false;
        let result = loop {
            let mut ov = OVERLAPPED { hEvent: event, ..Default::default() };
            let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME;
            if ReadDirectoryChangesW(dir, buf.as_mut_ptr() as *mut _, (buf.len() * 4) as u32, true, filter, None, Some(&mut ov), None).is_err() {
                break worked;
            }
            let mut bytes = 0u32;
            let done = loop {
                match GetOverlappedResultEx(dir, &ov, &mut bytes, 1000, false) {
                    Ok(()) => break Some(true),
                    Err(e) if e.code() == HRESULT::from_win32(WAIT_TIMEOUT.0) || e.code() == ERROR_IO_INCOMPLETE.to_hresult() => {
                        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
                            let _ = CancelIoEx(dir, Some(&ov));
                            let _ = GetOverlappedResult(dir, &ov, &mut bytes, true);
                            break None;
                        }
                    }
                    Err(_) => break Some(false),
                }
            };
            match done {
                None => break true,
                Some(false) => break worked,
                Some(true) if bytes == 0 => break true,
                Some(true) => {
                    worked = true;
                    let bytes = std::slice::from_raw_parts(buf.as_ptr() as *const u8, bytes as usize);
                    for d in changed_dirs(bytes) {
                        refresh_dir(root, &d);
                    }
                }
            }
        };
        let _ = CloseHandle(event);
        let _ = CloseHandle(dir);
        result
    }
}

/// 변경 알림 버퍼(FILE_NOTIFY_INFORMATION 연속)에서 바뀐 항목들의 상위 폴더(상대 경로)를 상위 쪽부터 모은다
fn changed_dirs(buf: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut off = 0;
    while off + 12 <= buf.len() {
        let u32_at = |i: usize| u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) as usize;
        let (next, name_len) = (u32_at(off), u32_at(off + 8));
        let Some(name) = buf.get(off + 12..off + 12 + name_len) else { break };
        let units: Vec<u16> = name.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let rel = String::from_utf16_lossy(&units);
        let parent = rel.rfind('\\').map(|i| rel[..i].to_string()).unwrap_or_default();
        if !out.iter().any(|d| util::eq_path(d, &parent)) {
            out.push(parent);
        }
        if next == 0 {
            break;
        }
        off += next;
    }
    // 상위 폴더를 먼저 읽어야 새로 생긴 하위 폴더를 통째로 훑을 수 있다
    out.sort_by_key(|d| if d.is_empty() { 0 } else { d.matches('\\').count() + 1 });
    out
}

// ---------- 목록 저장 ----------

fn file_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Sakadu").join("search-folders.txt"))
}

pub fn load() -> Vec<String> {
    file_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default()
}

fn save(paths: &[String]) -> std::io::Result<()> {
    let path = file_path().ok_or_else(|| std::io::Error::other("APPDATA 없음"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serialize(paths))?;
    std::fs::rename(tmp, path)
}

fn serialize(paths: &[String]) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    for p in paths {
        out.push_str(p);
        out.push('\n');
    }
    out
}

fn parse(text: &str) -> Vec<String> {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next().map(str::trim) != Some(HEADER) {
        return Vec::new();
    }
    lines.map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sakadu-folders-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn wait_for(what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn names(f: &Folders, q: &str) -> Vec<String> {
        f.search(&Query::parse(q), 100).0.into_iter().map(|h| util::file_name(&h.path).to_string()).collect()
    }

    #[test]
    fn roundtrip_and_header() {
        let paths = vec!["\\\\nas\\scan".to_string(), "E:\\".to_string()];
        assert_eq!(parse(&serialize(&paths)), paths);
        assert!(parse("something else\nC:\\").is_empty());
    }

    #[test]
    fn within_and_relative() {
        assert!(is_within("\\\\NAS\\scan\\2026", "\\\\nas\\scan"));
        assert!(is_within("\\\\nas\\scan", "\\\\nas\\scan"));
        assert!(!is_within("\\\\nas\\scanner", "\\\\nas\\scan"));
        assert!(is_within("E:\\photos", "E:\\"));
        assert_eq!(rel_path("\\\\nas\\scan", "\\\\nas\\scan\\a\\b"), "a\\b");
        assert_eq!(rel_path("E:\\", "E:\\a"), "a");
        assert_eq!(rel_path("\\\\nas\\scan", "\\\\nas\\scan"), "");
    }

    #[test]
    fn parses_change_buffer() {
        // FILE_NOTIFY_INFORMATION 두 개: "a\b.txt" 추가, "c" 추가
        let mut buf = Vec::new();
        for (i, name) in ["a\\b.txt", "c"].iter().enumerate() {
            let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let size = (12 + units.len()).div_ceil(4) * 4;
            let next = if i == 0 { size as u32 } else { 0 };
            buf.extend(next.to_le_bytes());
            buf.extend(1u32.to_le_bytes());
            buf.extend((units.len() as u32).to_le_bytes());
            buf.extend(&units);
            buf.resize(buf.len().div_ceil(4) * 4, 0);
        }
        assert_eq!(changed_dirs(&buf), vec!["".to_string(), "a".to_string()]);
    }

    /// 실제 폴더를 훑고, 변경 알림으로 추가·삭제·새 하위 폴더를 따라가는지
    #[test]
    fn scans_and_follows_changes() {
        let dir = temp_dir("watch");
        std::fs::create_dir_all(dir.join("업무\\2026")).unwrap();
        std::fs::write(dir.join("업무\\2026\\견적서_0420.pdf"), b"x").unwrap();
        std::fs::write(dir.join("readme.txt"), b"x").unwrap();
        let f = Folders::default();
        f.spawn(dir.to_string_lossy().into_owned());
        wait_for("first scan", || matches!(f.list()[0].1, State::Ready(_)));
        assert_eq!(f.list()[0].1, State::Ready(4));
        assert_eq!(names(&f, "0420"), vec!["견적서_0420.pdf"]);
        assert_eq!(names(&f, "견적 업무\\"), vec!["견적서_0420.pdf"]);

        std::fs::create_dir_all(dir.join("새 폴더\\안쪽")).unwrap();
        std::fs::write(dir.join("새 폴더\\안쪽\\scan_0001.pdf"), b"x").unwrap();
        wait_for("new nested file", || names(&f, "scan_0001") == vec!["scan_0001.pdf"]);
        std::fs::remove_dir_all(dir.join("업무")).unwrap();
        wait_for("removed subtree", || names(&f, "0420").is_empty() && names(&f, "2026").is_empty());

        f.remove_where(|_| true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 성능: 1만 개가 든 넣은 폴더를 다시 열 때 읽은 목록을 인덱스와 맞춰 보는 시간 (UI 스레드에서 돈다)
    /// (cargo test --release bench_observe_large_folder -- --ignored --nocapture)
    #[test]
    #[ignore]
    fn bench_observe_large_folder() {
        let names: Vec<String> = (0..10_000).map(|i| format!("scan_{i:05}.pdf")).collect();
        let mut tree = Tree::new();
        tree.insert(String::new(), names.iter().map(|n| (n.clone(), false)).collect());
        let f = Folders::default();
        let root = Root { path: "\\\\nas\\bench".into(), tree: RwLock::new(tree), state: Mutex::new(State::Ready(10_000)), stop: AtomicBool::new(false) };
        f.roots.lock().unwrap().push(Arc::new(root));
        // 탐색기 목록은 이름순이라 인덱스와 순서가 다를 수 있다: 거꾸로 넘긴다
        let entries: Vec<Entry> = names.iter().rev().map(|n| Entry::from_str(n, crate::listing::Kind::File)).collect();
        for round in 1..=3 {
            let t0 = Instant::now();
            f.observe("\\\\nas\\bench", &entries);
            println!("round {round}: observe 10000 entries: {:?}", t0.elapsed());
        }
    }

    /// 실제 공유폴더에서 훑기·검색 확인 (SAKADU_TEST_FOLDER, SAKADU_TEST_QUERY). 이름은 출력하지 않고 개수만
    #[test]
    #[ignore]
    fn scans_real_folder_from_env() {
        let (Ok(path), Ok(q)) = (std::env::var("SAKADU_TEST_FOLDER"), std::env::var("SAKADU_TEST_QUERY")) else { return };
        let f = Folders::default();
        let started = Instant::now();
        f.spawn(path);
        wait_for("scan", || !matches!(f.list()[0].1, State::Scanning));
        let scanned = started.elapsed();
        let t = Instant::now();
        let (hits, total) = f.search(&Query::parse(&q), 200);
        println!("state={:?} scan={scanned:?} search={:?} total={total} hits={}", f.list()[0].1, t.elapsed(), hits.len());
        f.remove_where(|_| true);
    }

    #[test]
    fn missing_folder_goes_offline() {
        let f = Folders::default();
        f.spawn(std::env::temp_dir().join(format!("sakadu-no-such-folder-{}", std::process::id())).to_string_lossy().into_owned());
        wait_for("offline", || f.list()[0].1 == State::Offline);
        f.remove_where(|_| true);
    }
}
