//! NTFS 파일 이름 인덱스.
//! MFT를 FSCTL_ENUM_USN_DATA로 한 번 열거해 만들고, USN 저널(FSCTL_READ_USN_JOURNAL)로 실시간 갱신한다.
//! 항목 번호는 MFT 레코드 번호(파일 참조 번호 하위 48비트)를 그대로 써서 해시 표 없이 바로 찾는다.
//! 볼륨을 여는 데 관리자 권한이 필요하므로 검색 서비스(service.rs) 안에서 돌린다.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU32, Ordering};

const NO_PARENT: u32 = u32::MAX;
const FLAG_USED: u8 = 1;
const FLAG_DIR: u8 = 2;
/// NTFS 루트 디렉터리의 MFT 레코드 번호
pub const ROOT_RECORD: u32 = 5;

pub fn record_of(frn: u64) -> u32 {
    (frn & 0x0000_FFFF_FFFF_FFFF) as u32
}

/// 한 볼륨의 이름 인덱스
pub struct Volume {
    pub letter: char,
    parent: Vec<u32>,
    name_off: Vec<u32>,
    name_len: Vec<u16>,
    flags: Vec<u8>,
    /// 모든 이름을 이어 붙인 UTF-8 저장소
    names: Vec<u8>,
    live: usize,
    /// 이름이 바뀌거나 지워져 더 이상 쓰지 않는 바이트 수 (일정 이상이면 압축)
    garbage: usize,
    pub journal_id: u64,
    pub next_usn: i64,
}

impl Volume {
    pub fn new(letter: char) -> Volume {
        Volume {
            letter,
            parent: Vec::new(),
            name_off: Vec::new(),
            name_len: Vec::new(),
            flags: Vec::new(),
            names: Vec::new(),
            live: 0,
            garbage: 0,
            journal_id: 0,
            next_usn: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.live
    }

    fn ensure(&mut self, rec: u32) {
        let need = rec as usize + 1;
        if self.flags.len() < need {
            self.parent.resize(need, NO_PARENT);
            self.name_off.resize(need, 0);
            self.name_len.resize(need, 0);
            self.flags.resize(need, 0);
        }
    }

    pub fn upsert(&mut self, frn: u64, parent_frn: u64, name: &str, is_dir: bool) {
        let rec = record_of(frn);
        self.ensure(rec);
        let i = rec as usize;
        if self.flags[i] & FLAG_USED != 0 {
            self.garbage += self.name_len[i] as usize;
        } else {
            self.live += 1;
        }
        let bytes = &name.as_bytes()[..name.len().min(u16::MAX as usize)];
        self.name_off[i] = self.names.len() as u32;
        self.name_len[i] = bytes.len() as u16;
        self.names.extend_from_slice(bytes);
        self.parent[i] = record_of(parent_frn);
        self.flags[i] = FLAG_USED | if is_dir { FLAG_DIR } else { 0 };
    }

    pub fn remove(&mut self, frn: u64) {
        let i = record_of(frn) as usize;
        if i < self.flags.len() && self.flags[i] & FLAG_USED != 0 {
            self.flags[i] = 0;
            self.garbage += self.name_len[i] as usize;
            self.live -= 1;
        }
    }

    fn used(&self, i: usize) -> bool {
        self.flags[i] & FLAG_USED != 0
    }

    fn name_bytes(&self, i: usize) -> &[u8] {
        let off = self.name_off[i] as usize;
        &self.names[off..off + self.name_len[i] as usize]
    }

    pub fn is_dir(&self, i: u32) -> bool {
        self.flags[i as usize] & FLAG_DIR != 0
    }

    /// "C:\폴더\파일" 형태의 전체 경로. 부모 사슬이 끊겼으면 None (삭제 중인 항목 등)
    /// 항목 칸 수 (지운 칸 포함). 칸 번호별로 따로 기억해 둘 때 크기로 쓴다
    pub fn slots(&self) -> usize {
        self.flags.len()
    }

    /// 칸 i가 지금 쓰이는 폴더인가
    pub fn is_dir_slot(&self, i: usize) -> bool {
        i < self.flags.len() && self.used(i) && self.flags[i] & FLAG_DIR != 0
    }

    /// 담은 폴더의 칸 (없거나 지운 항목이면 None)
    pub fn parent_of(&self, i: u32) -> Option<u32> {
        let c = i as usize;
        (c < self.flags.len() && self.used(c) && self.parent[c] != NO_PARENT).then(|| self.parent[c])
    }

    pub fn path(&self, i: u32) -> Option<String> {
        let mut parts: Vec<&[u8]> = Vec::new();
        let mut cur = i;
        for _ in 0..512 {
            if cur == ROOT_RECORD {
                let mut s = format!("{}:", self.letter);
                for p in parts.iter().rev() {
                    s.push('\\');
                    s.push_str(std::str::from_utf8(p).ok()?);
                }
                if parts.is_empty() {
                    s.push('\\');
                }
                return Some(s);
            }
            let c = cur as usize;
            if c >= self.flags.len() || !self.used(c) {
                return None;
            }
            parts.push(self.name_bytes(c));
            cur = self.parent[c];
        }
        None
    }

    /// 버려진 이름이 절반을 넘으면 저장소를 다시 채운다
    pub fn compact_if_needed(&mut self) {
        if self.garbage < 1 << 20 || self.garbage * 2 < self.names.len() {
            return;
        }
        let mut names = Vec::with_capacity(self.names.len() - self.garbage);
        for i in 0..self.flags.len() {
            if self.used(i) {
                let off = names.len() as u32;
                names.extend_from_slice(self.name_bytes(i));
                self.name_off[i] = off;
            }
        }
        self.names = names;
        self.garbage = 0;
    }

    /// 처음 만든 뒤 배열이 자라며 남은 여유 공간을 돌려준다
    pub fn shrink(&mut self) {
        self.parent.shrink_to_fit();
        self.name_off.shrink_to_fit();
        self.name_len.shrink_to_fit();
        self.flags.shrink_to_fit();
        self.names.shrink_to_fit();
    }

    /// 대략적인 메모리 사용량(바이트)
    pub fn memory(&self) -> usize {
        self.names.capacity() + self.flags.capacity() * (4 + 4 + 2 + 1)
    }
}

// ---------- USN 레코드 ----------

#[derive(Debug, Clone, PartialEq)]
pub struct UsnRecord {
    pub frn: u64,
    pub parent: u64,
    pub usn: i64,
    pub reason: u32,
    pub attrs: u32,
    pub name: String,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// USN_RECORD_V2 묶음을 해석한다. 다른 버전이나 잘린 레코드는 건너뛴다
pub fn parse_records(buf: &[u8]) -> Vec<UsnRecord> {
    let mut out = Vec::new();
    let mut o = 0;
    while o + 60 <= buf.len() {
        let len = u32_at(buf, o) as usize;
        if len < 60 || o + len > buf.len() {
            break;
        }
        let rec = &buf[o..o + len];
        if u16_at(rec, 4) == 2 {
            let name_len = u16_at(rec, 56) as usize;
            let name_off = u16_at(rec, 58) as usize;
            if name_off + name_len <= len {
                let units: Vec<u16> = rec[name_off..name_off + name_len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                out.push(UsnRecord {
                    frn: u64_at(rec, 8),
                    parent: u64_at(rec, 16),
                    usn: u64_at(rec, 24) as i64,
                    reason: u32_at(rec, 40),
                    attrs: u32_at(rec, 52),
                    name: String::from_utf16_lossy(&units),
                });
            }
        }
        o += len;
    }
    out
}

pub const REASON_CREATE: u32 = 0x100;
pub const REASON_DELETE: u32 = 0x200;
pub const REASON_RENAME_OLD: u32 = 0x1000;
pub const REASON_RENAME_NEW: u32 = 0x2000;
/// 권한(보안 설명자)이 바뀜. 검색 서비스가 계정별로 기억해 둔 폴더 판정을 다시 하게 한다
pub const REASON_SECURITY_CHANGE: u32 = 0x800;
pub const ATTR_DIR: u32 = 0x10;

/// 저널 변경 하나를 인덱스에 반영
pub fn apply(vol: &mut Volume, r: &UsnRecord) {
    if r.reason & REASON_DELETE != 0 {
        vol.remove(r.frn);
    } else if r.reason & (REASON_CREATE | REASON_RENAME_NEW) != 0 {
        vol.upsert(r.frn, r.parent, &r.name, r.attrs & ATTR_DIR != 0);
    }
}

// ---------- 검색 ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub path: String,
    pub is_dir: bool,
    pub score: i32,
}

pub struct Query {
    /// 이름에 모두 들어 있어야 하는 낱말(소문자)
    terms: Vec<Vec<u8>>,
    /// '\'가 들어간 낱말은 전체 경로에서 찾는다
    path_terms: Vec<String>,
}

impl Query {
    pub fn parse(q: &str) -> Query {
        let mut terms = Vec::new();
        let mut path_terms = Vec::new();
        for w in q.split_whitespace() {
            let w = w.to_lowercase().replace('/', "\\");
            if w.contains('\\') {
                path_terms.push(w);
            } else {
                terms.push(w.into_bytes());
            }
        }
        Query { terms, path_terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.path_terms.is_empty()
    }

    // 이 인덱스 밖의 항목(검색에 넣은 폴더, 서비스가 돌려준 결과)에도 search와 같은 기준으로 점수를 매긴다

    /// 이름만 보고 매기는 1차 점수. 이름이 맞지 않으면 None
    pub fn name_score(&self, name: &str, is_dir: bool) -> Option<i32> {
        if self.terms.is_empty() { Some(0) } else { name_score(name.as_bytes(), &self.terms, is_dir) }
    }

    /// 1차 점수에 전체 경로를 반영한 최종 점수. 경로가 경로 낱말을 모두 포함하지 않으면 None
    pub fn path_score(&self, name_score: i32, path: &str) -> Option<i32> {
        let lower = path.to_lowercase();
        self.path_terms.iter().all(|t| lower.contains(t.as_str())).then(|| name_score - path_penalty(&lower))
    }
}

/// 대소문자 무시(ASCII) 부분 문자열 위치. needle은 소문자여야 한다
fn find_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    let first = needle[0];
    for i in 0..=hay.len() - needle.len() {
        if hay[i].to_ascii_lowercase() == first && hay[i..i + needle.len()].iter().zip(needle).all(|(a, b)| a.to_ascii_lowercase() == *b) {
            return Some(i);
        }
    }
    None
}

fn is_word_start(name: &[u8], pos: usize) -> bool {
    pos == 0 || matches!(name[pos - 1], b' ' | b'_' | b'-' | b'.' | b'(' | b'[' | b'@')
}

/// 이름만 보고 매기는 1차 점수. 일치하지 않으면 None
fn name_score(name: &[u8], terms: &[Vec<u8>], is_dir: bool) -> Option<i32> {
    let mut s = 0;
    for (k, t) in terms.iter().enumerate() {
        let pos = find_ci(name, t)?;
        if k == 0 {
            let stem = name.iter().rposition(|&c| c == b'.').filter(|&p| p > 0).map(|p| &name[..p]).unwrap_or(name);
            if name.len() == t.len() {
                s += 1000;
            } else if stem.len() == t.len() && pos == 0 {
                s += 800;
            } else if pos == 0 {
                s += 600;
            } else if is_word_start(name, pos) {
                s += 300;
            }
        } else {
            s += 50;
        }
    }
    s -= name.len().min(200) as i32;
    if is_dir {
        s += 30;
    }
    Some(s)
}

/// 시스템·개발 도구 폴더 안의 결과는 아래로 (사용자 파일이 먼저 보이게)
fn path_penalty(lower_path: &str) -> i32 {
    const NOISY: [&str; 10] = [
        "\\windows\\",
        "\\appdata\\",
        "\\$recycle.bin\\",
        "\\programdata\\",
        "\\program files",
        "\\node_modules\\",
        "\\.git\\",
        "\\system volume information\\",
        "\\$extend\\",
        "\\.cargo\\",
    ];
    let depth = lower_path.matches('\\').count() as i32;
    let noisy = if NOISY.iter().any(|n| lower_path.contains(n)) { 400 } else { 0 };
    depth * 4 + noisy
}

// 폴더별 경로 조건 판정 (search 한 번 동안 memo에 기억). 0은 아직 모름, DIR_FAIL은 그 폴더 바로 아래 항목은 맞을 수 없음,
// 그 밖에는 DIR_KNOWN에 "이름 앞부분까지 봐야 하는 낱말" 비트(낱말 j → 1 << (j + 2))를 더한 값
const DIR_FAIL: u32 = 1;
const DIR_KNOWN: u32 = 2;
/// 폴더별 판정에 비트로 담을 수 있는 경로 낱말 수
const MEMO_TERMS: usize = 30;

/// 경로 낱말에서 마지막 '\' 뒤 부분 (이름 앞부분과 맞춰 볼 부분)
fn after_last_sep(t: &str) -> &str {
    &t[t.rfind('\\').map_or(0, |k| k + 1)..]
}

fn starts_with_lower(name: &str, prefix: &str) -> bool {
    let mut lower = name.chars().flat_map(char::to_lowercase);
    prefix.chars().all(|c| lower.next() == Some(c))
}

/// 폴더 경로 앞부분을 만드는 스레드별 버퍼. 바로 전에 만든 경로와 겹치는 앞부분은 다시 만들지 않는다
#[derive(Default)]
struct PrefixBuf {
    /// "c:\users\hong\"처럼 소문자로 만든 경로
    s: String,
    /// s에 든 폴더들: 루트 쪽부터 (레코드 번호, 그 폴더 이름 뒤 '\'까지의 길이)
    parts: Vec<(u32, usize)>,
    chain: Vec<u32>,
}

impl PrefixBuf {
    /// 폴더 d 바로 아래 항목 경로의 앞부분을 s에 만든다. 그 항목의 path()가 None이면 false (깊이 제한도 같게 맞춤)
    fn build(&mut self, vol: &Volume, d: u32) -> bool {
        self.chain.clear();
        let mut keep = None;
        let mut cur = d;
        for _ in 0..511 {
            if cur == ROOT_RECORD {
                keep = Some(0);
                break;
            }
            if let Some(k) = self.parts.iter().position(|&(r, _)| r == cur) {
                keep = Some(k + 1);
                break;
            }
            let c = cur as usize;
            if c >= vol.flags.len() || !vol.used(c) {
                return false;
            }
            self.chain.push(cur);
            cur = vol.parent[c];
        }
        let Some(keep) = keep.filter(|k| k + self.chain.len() <= 510) else { return false };
        self.parts.truncate(keep);
        match self.parts.last() {
            Some(&(_, end)) => self.s.truncate(end),
            None => {
                self.s.clear();
                self.s.push(vol.letter.to_ascii_lowercase());
                self.s.push_str(":\\");
            }
        }
        while let Some(c) = self.chain.pop() {
            let Ok(name) = std::str::from_utf8(vol.name_bytes(c as usize)) else {
                self.parts.clear();
                return false;
            };
            self.s.extend(name.chars().flat_map(char::to_lowercase));
            self.s.push('\\');
            self.parts.push((c, self.s.len()));
        }
        true
    }
}

/// 항목 i의 전체 경로가 경로 낱말을 모두 포함하는가.
/// 경로는 "폴더 경로\" + 이름이고 이름에는 '\'가 없으므로, 낱말은 폴더 쪽에 통째로 들어 있거나
/// 마지막 '\'까지가 폴더 쪽 끝과 같고 나머지가 이름 앞부분이어야 한다. 폴더 쪽 판정은 폴더마다 한 번만 한다
fn path_matches(vol: &Volume, i: usize, terms: &[String], memo: &[AtomicU32], buf: &mut PrefixBuf) -> bool {
    let p = vol.parent[i] as usize;
    if i == ROOT_RECORD as usize || p >= memo.len() {
        return vol.path(i as u32).is_some_and(|path| {
            let lower = path.to_lowercase();
            terms.iter().all(|t| lower.contains(t.as_str()))
        });
    }
    let mut state = memo[p].load(Ordering::Relaxed);
    if state == 0 {
        state = DIR_FAIL;
        if buf.build(vol, p as u32) {
            state = DIR_KNOWN;
            for (j, t) in terms.iter().enumerate() {
                if buf.s.contains(t.as_str()) {
                    continue;
                }
                let rest = after_last_sep(t);
                if rest.is_empty() || !buf.s.ends_with(&t[..t.len() - rest.len()]) {
                    state = DIR_FAIL;
                    break;
                }
                state |= 1 << (j + 2);
            }
        }
        memo[p].store(state, Ordering::Relaxed);
    }
    if state == DIR_FAIL {
        return false;
    }
    let Ok(name) = std::str::from_utf8(vol.name_bytes(i)) else { return false };
    terms.iter().enumerate().all(|(j, t)| state & (1 << (j + 2)) == 0 || starts_with_lower(name, after_last_sep(t)))
}

/// 여러 볼륨에서 찾는다. (상위 결과, 전체 일치 개수)
#[cfg(test)]
pub fn search(vols: &[&Volume], q: &Query, limit: usize) -> (Vec<Hit>, usize) {
    search_visible(vols, q, limit, &|_, _| true)
}

/// visible(볼륨 번호, 칸)이 false인 항목은 순위·전체 개수에서도 빠진다 (요청한 계정이 볼 수 없는 항목)
pub fn search_visible(vols: &[&Volume], q: &Query, limit: usize, visible: &(dyn Fn(usize, u32) -> bool + Sync)) -> (Vec<Hit>, usize) {
    if q.is_empty() {
        return (Vec::new(), 0);
    }
    // 1차: 이름 점수로 볼륨·스레드별 상위 후보만 남긴다
    const CANDIDATES: usize = 600;
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
    let mut cands: Vec<(i32, usize, u32)> = Vec::new();
    let mut total = 0usize;
    for (vi, vol) in vols.iter().enumerate() {
        let n = vol.flags.len();
        // 경로 조건이 있으면 폴더별 판정을 스레드끼리 함께 기억한다 (낱말이 너무 많으면 비워 두고 항목마다 전체 경로로 판정)
        let memo: Vec<AtomicU32> =
            if q.path_terms.is_empty() || q.path_terms.len() > MEMO_TERMS { Vec::new() } else { (0..n).map(|_| AtomicU32::new(0)).collect() };
        let memo = &memo;
        let chunk = n.div_ceil(threads).max(1);
        let parts: Vec<(usize, Vec<(i32, usize, u32)>)> = std::thread::scope(|sc| {
            let handles: Vec<_> = (0..n)
                .step_by(chunk)
                .map(|start| {
                    let end = (start + chunk).min(n);
                    sc.spawn(move || {
                        let mut heap: BinaryHeap<Reverse<(i32, usize, u32)>> = BinaryHeap::new();
                        let mut count = 0;
                        let mut prefix = PrefixBuf::default();
                        for i in start..end {
                            if !vol.used(i) {
                                continue;
                            }
                            let score = if q.terms.is_empty() { Some(0) } else { name_score(vol.name_bytes(i), &q.terms, vol.flags[i] & FLAG_DIR != 0) };
                            if let Some(s) = score {
                                if !q.path_terms.is_empty() && !path_matches(vol, i, &q.path_terms, memo, &mut prefix) {
                                    continue;
                                }
                                if !visible(vi, i as u32) {
                                    continue;
                                }
                                count += 1;
                                heap.push(Reverse((s, vi, i as u32)));
                                if heap.len() > CANDIDATES {
                                    heap.pop();
                                }
                            }
                        }
                        (count, heap.into_iter().map(|Reverse(x)| x).collect())
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        for (c, v) in parts {
            total += c;
            cands.extend(v);
        }
    }
    cands.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    cands.truncate(CANDIDATES);

    // 2차: 후보의 경로를 만들어 감점을 반영
    let mut hits: Vec<Hit> = cands
        .into_iter()
        .filter_map(|(s, vi, i)| {
            let path = vols[vi].path(i)?;
            Some(Hit { score: s - path_penalty(&path.to_lowercase()), is_dir: vols[vi].is_dir(i), path })
        })
        .collect();
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.len().cmp(&b.path.len())));
    hits.truncate(limit);
    (hits, total)
}

// ---------- 볼륨 접근 (관리자 권한 필요) ----------

pub mod volume_io {
    use super::*;
    use windows::Win32::Foundation::{CloseHandle, ERROR_HANDLE_EOF, GENERIC_READ, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
        OPEN_EXISTING,
    };
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::core::HSTRING;

    const FSCTL_ENUM_USN_DATA: u32 = 0x0009_00B3;
    const FSCTL_QUERY_USN_JOURNAL: u32 = 0x0009_00F4;
    const FSCTL_READ_USN_JOURNAL: u32 = 0x0009_00BB;
    const DRIVE_FIXED: u32 = 3;

    #[repr(C)]
    struct MftEnumDataV0 {
        start_frn: u64,
        low_usn: i64,
        high_usn: i64,
    }

    #[repr(C)]
    #[derive(Default)]
    pub struct JournalData {
        pub id: u64,
        pub first_usn: i64,
        pub next_usn: i64,
        pub lowest_valid_usn: i64,
        pub max_usn: i64,
        pub max_size: u64,
        pub allocation_delta: u64,
    }

    #[repr(C)]
    struct ReadJournalDataV0 {
        start_usn: i64,
        reason_mask: u32,
        return_only_on_close: u32,
        timeout: u64,
        bytes_to_wait_for: u64,
        journal_id: u64,
    }

    pub struct VolumeHandle(pub HANDLE);
    unsafe impl Send for VolumeHandle {}
    impl Drop for VolumeHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// 고정 디스크 중 NTFS 볼륨의 드라이브 문자
    pub fn ntfs_drives() -> Vec<char> {
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| (b'A' + i) as char)
            .filter(|c| {
                let root = HSTRING::from(format!("{c}:\\"));
                if unsafe { GetDriveTypeW(&root) } != DRIVE_FIXED {
                    return false;
                }
                let mut fs = [0u16; 32];
                unsafe { GetVolumeInformationW(&root, None, None, None, None, Some(&mut fs)) }.is_ok() && crate::util::from_wide(&fs) == "NTFS"
            })
            .collect()
    }

    pub fn open(letter: char) -> windows::core::Result<VolumeHandle> {
        let h = unsafe {
            CreateFileW(
                &HSTRING::from(format!("\\\\.\\{letter}:")),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )?
        };
        Ok(VolumeHandle(h))
    }

    pub fn query_journal(h: &VolumeHandle) -> windows::core::Result<JournalData> {
        let mut jd = JournalData::default();
        unsafe {
            DeviceIoControl(
                h.0,
                FSCTL_QUERY_USN_JOURNAL,
                None,
                0,
                Some(&mut jd as *mut _ as *mut _),
                size_of::<JournalData>() as u32,
                None,
                None,
            )?
        };
        Ok(jd)
    }

    /// MFT 전체를 열거해 인덱스를 만든다
    pub fn build(letter: char) -> windows::core::Result<Volume> {
        let h = open(letter)?;
        let jd = query_journal(&h)?;
        let mut vol = Volume::new(letter);
        let mut med = MftEnumDataV0 { start_frn: 0, low_usn: 0, high_usn: jd.next_usn };
        // u64 버퍼로 잡아 8바이트 정렬을 보장
        let mut buf = vec![0u64; 128 * 1024];
        loop {
            let mut got = 0u32;
            let r = unsafe {
                DeviceIoControl(
                    h.0,
                    FSCTL_ENUM_USN_DATA,
                    Some(&med as *const _ as *const _),
                    size_of::<MftEnumDataV0>() as u32,
                    Some(buf.as_mut_ptr() as *mut _),
                    (buf.len() * 8) as u32,
                    Some(&mut got),
                    None,
                )
            };
            if let Err(e) = r {
                if e.code() == ERROR_HANDLE_EOF.to_hresult() {
                    break;
                }
                return Err(e);
            }
            let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, got as usize) };
            if bytes.len() < 8 {
                break;
            }
            med.start_frn = u64_at(bytes, 0);
            for r in parse_records(&bytes[8..]) {
                vol.upsert(r.frn, r.parent, &r.name, r.attrs & ATTR_DIR != 0);
            }
        }
        vol.journal_id = jd.id;
        vol.next_usn = jd.next_usn;
        vol.shrink();
        Ok(vol)
    }

    /// 저널에서 start_usn 이후 변경을 읽는다. 변경이 생길 때까지 기다린다(블로킹).
    /// (다음 시작 USN, 변경 레코드)
    pub fn read_changes(h: &VolumeHandle, journal_id: u64, start_usn: i64) -> windows::core::Result<(i64, Vec<UsnRecord>)> {
        let rd = ReadJournalDataV0 {
            start_usn,
            reason_mask: REASON_CREATE | REASON_DELETE | REASON_RENAME_OLD | REASON_RENAME_NEW | REASON_SECURITY_CHANGE,
            return_only_on_close: 0,
            timeout: 0,
            bytes_to_wait_for: 1,
            journal_id,
        };
        let mut buf = vec![0u64; 16 * 1024];
        let mut got = 0u32;
        unsafe {
            DeviceIoControl(
                h.0,
                FSCTL_READ_USN_JOURNAL,
                Some(&rd as *const _ as *const _),
                size_of::<ReadJournalDataV0>() as u32,
                Some(buf.as_mut_ptr() as *mut _),
                (buf.len() * 8) as u32,
                Some(&mut got),
                None,
            )?
        };
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, got as usize) };
        if bytes.len() < 8 {
            return Ok((start_usn, Vec::new()));
        }
        Ok((u64_at(bytes, 0) as i64, parse_records(&bytes[8..])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: u64 = ROOT_RECORD as u64;

    fn sample() -> Volume {
        let mut v = Volume::new('C');
        v.upsert(100, ROOT, "Users", true);
        v.upsert(101, 100, "hong", true);
        v.upsert(102, 101, "업무", true);
        v.upsert(103, 102, "일간 업무 보고.xlsx", false);
        v.upsert(104, 102, "보고서.docx", false);
        v.upsert(200, ROOT, "Windows", true);
        v.upsert(201, 200, "report.log", false);
        v.upsert(105, 101, "report.pdf", false);
        v
    }

    #[test]
    fn builds_paths() {
        let v = sample();
        assert_eq!(v.path(103).as_deref(), Some("C:\\Users\\hong\\업무\\일간 업무 보고.xlsx"));
        assert_eq!(v.path(ROOT_RECORD).as_deref(), Some("C:\\"));
        assert_eq!(v.len(), 8);
    }

    #[test]
    fn search_ranks_user_files_above_system() {
        let v = sample();
        let (hits, total) = search(&[&v], &Query::parse("report"), 10);
        assert_eq!(total, 2);
        assert_eq!(hits[0].path, "C:\\Users\\hong\\report.pdf");
        assert_eq!(hits[1].path, "C:\\Windows\\report.log");
    }

    #[test]
    fn korean_and_multiple_terms() {
        let v = sample();
        let (hits, _) = search(&[&v], &Query::parse("보고 업무"), 10);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("일간 업무 보고.xlsx"));
        let (hits, _) = search(&[&v], &Query::parse("REPORT.PDF"), 10);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn path_terms_filter_by_folder() {
        let v = sample();
        let (hits, _) = search(&[&v], &Query::parse("보고 업무\\"), 10);
        assert_eq!(hits.len(), 2, "both files under 업무\\: {hits:?}");
    }

    #[test]
    fn path_terms_count_only_full_matches() {
        let v = sample();
        // 이름은 report.pdf, report.log 둘이 맞지만 경로까지 맞는 것은 하나
        let (hits, total) = search(&[&v], &Query::parse("report hong\\"), 10);
        assert_eq!((hits.len(), total), (1, 1), "{hits:?}");
        // 경로 낱말만 있거나, 낱말이 폴더 경로와 이름에 걸치는 경우
        for (q, n) in [("업무\\", 2), ("hong\\업무\\", 2), ("c:\\users\\", 5), ("\\windows\\", 1), ("업무\\일간", 1), ("\\보고", 1)] {
            let (hits, total) = search(&[&v], &Query::parse(q), 10);
            assert_eq!((hits.len(), total), (n, n), "{q}: {hits:?}");
        }
    }

    #[test]
    fn path_terms_reach_beyond_top_candidates() {
        // 이름 점수가 더 높은 일치 항목이 후보 수(600)보다 많아도 경로까지 맞는 항목을 놓치지 않는다
        let mut v = Volume::new('C');
        v.upsert(100, ROOT, "기타", true);
        v.upsert(101, ROOT, "업무", true);
        for k in 0..1000u64 {
            v.upsert(1000 + k, 100, &format!("report{k}.txt"), false);
        }
        for k in 0..3u64 {
            v.upsert(5000 + k, 101, &format!("report with a much longer name {k}.txt"), false);
        }
        let (hits, total) = search(&[&v], &Query::parse("report 업무\\"), 10);
        assert_eq!((hits.len(), total), (3, 3), "{hits:?}");
        assert!(hits.iter().all(|h| h.path.starts_with("C:\\업무\\")));
    }

    #[test]
    fn prefix_buf_reuses_shared_part() {
        let v = sample();
        let mut buf = PrefixBuf::default();
        // 아래로, 위로, 다른 가지로, 루트로 오가며 앞 결과를 이어 써도 path()와 같아야 한다
        for d in [102, 100, 101, 200, 102, ROOT_RECORD, 102] {
            assert!(buf.build(&v, d));
            let path = v.path(d).unwrap().to_lowercase();
            assert_eq!(buf.s, if d == ROOT_RECORD { path } else { path + "\\" });
        }
        assert!(!buf.build(&v, 999), "없는 폴더");
        assert!(buf.build(&v, 101));
        assert_eq!(buf.s, "c:\\users\\hong\\");
    }

    #[test]
    fn rename_move_and_delete() {
        let mut v = sample();
        // 이름 바꾸며 다른 폴더로 이동
        apply(&mut v, &UsnRecord { frn: 104, parent: 101, usn: 1, reason: REASON_RENAME_NEW, attrs: 0, name: "최종 보고서.docx".into() });
        assert_eq!(v.path(104).as_deref(), Some("C:\\Users\\hong\\최종 보고서.docx"));
        apply(&mut v, &UsnRecord { frn: 104, parent: 101, usn: 2, reason: REASON_DELETE, attrs: 0, name: "최종 보고서.docx".into() });
        assert_eq!(v.path(104), None);
        assert_eq!(v.len(), 7);
        // 새 파일 (상위 48비트 시퀀스 번호는 무시)
        apply(&mut v, &UsnRecord { frn: (3u64 << 48) | 300, parent: 102, usn: 3, reason: REASON_CREATE, attrs: 0, name: "새 문서.txt".into() });
        assert_eq!(v.path(300).as_deref(), Some("C:\\Users\\hong\\업무\\새 문서.txt"));
    }

    #[test]
    fn parses_usn_record_bytes() {
        fn record(frn: u64, parent: u64, reason: u32, attrs: u32, name: &str) -> Vec<u8> {
            let units: Vec<u16> = name.encode_utf16().collect();
            let len = (60 + units.len() * 2).div_ceil(8) * 8;
            let mut b = vec![0u8; len];
            b[0..4].copy_from_slice(&(len as u32).to_le_bytes());
            b[4..6].copy_from_slice(&2u16.to_le_bytes());
            b[8..16].copy_from_slice(&frn.to_le_bytes());
            b[16..24].copy_from_slice(&parent.to_le_bytes());
            b[40..44].copy_from_slice(&reason.to_le_bytes());
            b[52..56].copy_from_slice(&attrs.to_le_bytes());
            b[56..58].copy_from_slice(&((units.len() * 2) as u16).to_le_bytes());
            b[58..60].copy_from_slice(&60u16.to_le_bytes());
            for (k, u) in units.iter().enumerate() {
                b[60 + k * 2..62 + k * 2].copy_from_slice(&u.to_le_bytes());
            }
            b
        }
        let mut buf = record(7, 5, REASON_CREATE, 0x10, "폴더");
        buf.extend(record(8, 7, REASON_RENAME_NEW, 0x20, "a.txt"));
        let recs = parse_records(&buf);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].name, "폴더");
        assert_eq!(recs[1].parent, 7);
    }

    /// 성능 측정: 항목 200만 개 인덱스 검색 (cargo test --release bench_search -- --ignored --nocapture)
    #[test]
    #[ignore]
    fn bench_search_2m() {
        let words = ["report", "보고서", "invoice", "사진", "budget", "회의록", "draft", "final", "data", "로그"];
        let exts = ["txt", "docx", "xlsx", "jpg", "pdf", "rs", "log"];
        let mut v = Volume::new('C');
        let t = std::time::Instant::now();
        let mut frn = 100u64;
        for d in 0..2000u64 {
            let dir = frn;
            v.upsert(dir, ROOT, &format!("folder_{d}_{}", words[(d % 10) as usize]), true);
            frn += 1;
            for k in 0..1000u64 {
                v.upsert(frn, dir, &format!("{}_{k}_{}.{}", words[((k * 7 + d) % 10) as usize], d, exts[(k % 7) as usize]), false);
                frn += 1;
            }
        }
        println!("built {} entries in {:?}, ~{} MB", v.len(), t.elapsed(), v.memory() >> 20);
        for q in ["report", "보고서 final", "budget_123_45", "folder_1999", "zzz_nothing"] {
            let t = std::time::Instant::now();
            let (hits, total) = search(&[&v], &Query::parse(q), 100);
            println!("{q:>16}: {total:>8} matches, top {:>3} in {:?}", hits.len(), t.elapsed());
        }
    }

    /// 성능 측정: 경로 낱말 검색. 실제 디스크처럼 폴더가 많고 깊은 트리 (cargo test --release bench_path -- --ignored --nocapture)
    #[test]
    #[ignore]
    fn bench_path_terms_deep() {
        const WORDS: [&str; 10] = ["report", "보고서", "invoice", "사진", "budget", "회의록", "draft", "final", "data", "로그"];
        // 실제 MFT처럼 레코드 번호가 트리 순서와 어긋나게 섞는다 (홀수 곱은 2^21 안에서 일대일)
        fn rec(n: u64) -> u64 {
            100 + (n.wrapping_mul(0x9E37_79B1) & ((1 << 21) - 1))
        }
        // 10 × 10 × 10 × 10 × 20 = 폴더 21만 개, 맨 아래 폴더마다 파일 8개
        fn fill(out: &mut Vec<(u64, u64, String, bool)>, parent: u64, depth: usize) {
            let fan = if depth == 4 { 20 } else { 10 };
            for k in 0..fan {
                let dir = rec(out.len() as u64);
                out.push((dir, parent, format!("{k}_{}", WORDS[(k + depth) % 10]), true));
                if depth == 4 {
                    for f in 0..8 {
                        out.push((rec(out.len() as u64), dir, format!("{}_{f}.txt", WORDS[(f + k) % 10]), false));
                    }
                } else {
                    fill(out, dir, depth + 1);
                }
            }
        }
        let t = std::time::Instant::now();
        let mut entries = Vec::new();
        fill(&mut entries, ROOT, 0);
        // 인덱스를 만들 때처럼 레코드 번호 순서로 넣는다
        entries.sort_unstable_by_key(|e| e.0);
        let mut v = Volume::new('C');
        for (frn, parent, name, is_dir) in &entries {
            v.upsert(*frn, *parent, name, *is_dir);
        }
        println!("built {} entries in {:?}, ~{} MB", v.len(), t.elapsed(), v.memory() >> 20);
        for q in ["report", "zzz_nothing", "report\\", "final report\\", "\\budget", "3_사진\\2_사진\\", "3_final\\final", "zzz\\"] {
            let query = Query::parse(q);
            let mut best = std::time::Duration::MAX;
            let (mut total, mut top) = (0, 0);
            for _ in 0..7 {
                let t = std::time::Instant::now();
                let (hits, n) = search(&[&v], &query, 100);
                best = best.min(t.elapsed());
                (total, top) = (n, hits.len());
            }
            println!("{q:>16}: {total:>8} matches, top {top:>3}, best of 7 {best:?}");
        }
    }

    #[test]
    fn compaction_keeps_names() {
        // 압축 기준(버려진 이름 1MB 이상)을 넘기도록 충분히 많이
        let mut v = Volume::new('D');
        for k in 0..40_000u64 {
            v.upsert(10 + k, ROOT, &format!("file-{k}-with-a-long-name.txt"), false);
        }
        for k in 0..40_000u64 {
            v.upsert(10 + k, ROOT, &format!("renamed-{k}.txt"), false);
        }
        v.compact_if_needed();
        assert_eq!(v.path(10 + 7).as_deref(), Some("D:\\renamed-7.txt"));
        assert_eq!(v.garbage, 0);
    }
}
