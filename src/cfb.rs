//! 복합 파일(CFB, OLE2 구조적 저장소) 읽기. 한글 문서(HWP 5)의 미리 보기 그림(PrvImage)·글(PrvText)처럼
//! 맨 위 저장소에 든 스트림 하나를 통째로 꺼낸다. Windows는 COM(IStorage)으로 읽고(thumbs.rs), 맥은 이것으로 읽는다.
//! 읽기만 하고, 하위 저장소 안의 스트림은 찾지 않는다.
//! 망가진 파일에서도 멈추거나 끝없이 돌지 않는다: 섹터 번호가 파일 밖이거나, 사슬이 고리를 이루거나, 스트림 크기만큼
//! 이어지지 않으면 그 스트림은 None이다. build는 시험·그림용으로 맨 위 스트림만 담은 작은 파일을 만든다

use std::io::{Read, Seek, SeekFrom};

const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
/// 섹터 번호의 특별한 값. MAX_REG보다 큰 번호는 섹터가 아니다
const MAX_REG: u32 = 0xFFFF_FFFA;
const DIFAT_SECT: u32 = 0xFFFF_FFFC;
const FAT_SECT: u32 = 0xFFFF_FFFD;
const END: u32 = 0xFFFF_FFFE;
const FREE: u32 = 0xFFFF_FFFF;
/// 디렉터리 항목의 '없음' (형제·자식이 없다)
const NONE: u32 = 0xFFFF_FFFF;
/// 머리에 적히는 FAT 섹터 번호의 수
const HEADER_FAT: usize = 109;
/// 작은 섹터의 크기와, 이보다 작은 스트림을 작은 섹터에 두는 기준 (build가 쓴다. 읽을 때는 머리의 값을 따른다)
const MINI: usize = 64;
const CUTOFF: usize = 4096;
/// 디렉터리 항목 하나의 크기
const ENTRY: usize = 128;
const STREAM: u8 = 2;
const ROOT: u8 = 5;
/// 스트림 하나를 이만큼까지만 읽는다 (Windows thumbs.rs와 같은 한도. 미리 보기 그림·글은 훨씬 작다)
pub const STREAM_MAX: u64 = 64 << 20;

/// 디렉터리 항목 (쓰는 것만)
struct Entry {
    name: String,
    kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
}

/// 열어 둔 복합 파일: 머리, FAT, 작은 섹터 FAT, 디렉터리를 읽어 두고 스트림은 부를 때 읽는다
pub struct Cfb<R> {
    src: R,
    len: u64,
    sector: usize,
    cutoff: u64,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    entries: Vec<Entry>,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u32s(b: &[u8]) -> impl Iterator<Item = u32> + '_ {
    b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c))
}

/// table(FAT나 작은 섹터 FAT)을 따라 start부터 끝 표시까지의 섹터 번호. 표 밖으로 나가거나, 끝 표시 없이 다른 특별한 값을 만나거나,
/// 한 번 지난 섹터로 돌아오면(고리) None
fn chain(table: &[u32], start: u32) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    let mut seen = vec![false; table.len()];
    let mut n = start;
    while n != END {
        let i = n as usize;
        if n > MAX_REG || i >= table.len() || std::mem::replace(&mut seen[i], true) {
            return None;
        }
        out.push(n);
        n = table[i];
    }
    Some(out)
}

impl<R: Read + Seek> Cfb<R> {
    /// 머리를 확인하고 FAT·디렉터리를 읽는다. 복합 파일이 아니거나 그 뼈대가 망가졌으면 None
    pub fn open(mut src: R) -> Option<Cfb<R>> {
        let len = src.seek(SeekFrom::End(0)).ok()?;
        let mut h = [0u8; 512];
        src.seek(SeekFrom::Start(0)).ok()?;
        src.read_exact(&mut h).ok()?;
        if h[..8] != SIGNATURE || u16_at(&h, 0x1C) != 0xFFFE {
            return None;
        }
        // 버전 3은 512바이트, 버전 4는 4096바이트 섹터. 작은 섹터는 늘 64바이트
        let major = u16_at(&h, 0x1A);
        if !matches!((major, u16_at(&h, 0x1E)), (3, 9) | (4, 12)) || u16_at(&h, 0x20) != 6 {
            return None;
        }
        let sector = if major == 3 { 512 } else { 4096 };
        // 파일에 든 섹터 수 (머리 섹터 빼고. 마지막 섹터를 덜 채운 파일도 있어 올린다). FAT 섹터가 이보다 많을 수는 없다
        let sectors = len.saturating_sub(sector as u64).div_ceil(sector as u64);
        let n_fat = u32_at(&h, 0x2C) as u64;
        if n_fat > sectors {
            return None;
        }
        let mut cfb = Cfb { src, len, sector, cutoff: u64::from(u32_at(&h, 0x38)), fat: Vec::new(), mini_fat: Vec::new(), entries: Vec::new() };
        // FAT 섹터의 번호: 머리에 109개까지, 넘는 것은 DIFAT 섹터의 사슬에 (섹터마다 마지막 칸이 다음 DIFAT 섹터).
        // DIFAT 섹터 하나를 읽을 때마다 번호가 그만큼 늘어 고리를 이뤄도 n_fat개를 채우면 끝난다
        let n_fat = n_fat as usize;
        let mut fat_sectors: Vec<u32> = (0..HEADER_FAT.min(n_fat)).map(|i| u32_at(&h, 0x4C + i * 4)).collect();
        let mut next = u32_at(&h, 0x44);
        while fat_sectors.len() < n_fat {
            let s = cfb.read_sector(next)?;
            let per = sector / 4 - 1;
            fat_sectors.extend(u32s(&s[..per * 4]));
            next = u32_at(&s, per * 4);
        }
        fat_sectors.truncate(n_fat);
        for n in fat_sectors {
            let s = cfb.read_sector(n)?;
            cfb.fat.extend(u32s(&s));
        }
        // 디렉터리: 맨 앞이 뿌리 항목이다
        for n in chain(&cfb.fat, u32_at(&h, 0x30))? {
            let s = cfb.read_sector(n)?;
            for e in s.as_chunks::<ENTRY>().0 {
                let name_len = usize::from(u16_at(e, 0x40));
                let units: Vec<u16> = e[..64].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take((name_len / 2).saturating_sub(1)).collect();
                let size = u64::from(u32_at(e, 0x78)) | (u64::from(u32_at(e, 0x7C)) << 32);
                cfb.entries.push(Entry {
                    name: String::from_utf16_lossy(&units),
                    kind: e[0x42],
                    left: u32_at(e, 0x44),
                    right: u32_at(e, 0x48),
                    child: u32_at(e, 0x4C),
                    start: u32_at(e, 0x74),
                    // 버전 3은 크기의 아래 32비트만 쓴다 (위 32비트에 쓰레기를 남긴 파일이 있다)
                    size: if major == 3 { size & 0xFFFF_FFFF } else { size },
                });
            }
        }
        if cfb.entries.first().is_none_or(|root| root.kind != ROOT) {
            return None;
        }
        // 작은 섹터 FAT (작은 스트림이 없는 파일은 없다)
        let mini_start = u32_at(&h, 0x3C);
        if mini_start <= MAX_REG {
            for n in chain(&cfb.fat, mini_start)? {
                let s = cfb.read_sector(n)?;
                cfb.mini_fat.extend(u32s(&s));
            }
        }
        Some(cfb)
    }

    /// 섹터 n의 내용. 파일 밖이면 None, 파일 끝에서 덜 채운 마지막 섹터는 뒤를 0으로 채운다
    fn read_sector(&mut self, n: u32) -> Option<Vec<u8>> {
        let at = (u64::from(n) + 1) * self.sector as u64;
        if n > MAX_REG || at >= self.len {
            return None;
        }
        let mut buf = vec![0u8; self.sector];
        let take = (self.len - at).min(self.sector as u64) as usize;
        self.src.seek(SeekFrom::Start(at)).ok()?;
        self.src.read_exact(&mut buf[..take]).ok()?;
        Some(buf)
    }

    /// 뿌리 저장소 바로 아래의 스트림 name (대소문자는 가리지 않는다). 뿌리의 자식에서 시작해 형제 나무를 훑는다
    fn find(&self, name: &str) -> Option<&Entry> {
        let mut seen = vec![false; self.entries.len()];
        let mut stack = vec![self.entries.first()?.child];
        while let Some(id) = stack.pop() {
            let i = id as usize;
            let Some(e) = self.entries.get(i) else { continue };
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            if e.kind == STREAM && e.name.eq_ignore_ascii_case(name) {
                return Some(e);
            }
            stack.extend([e.left, e.right].into_iter().filter(|s| *s != NONE));
        }
        None
    }

    /// 맨 위 스트림 name을 통째로 읽는다. 없거나, 너무 크거나(STREAM_MAX), 사슬이 망가졌으면 None
    pub fn stream(&mut self, name: &str) -> Option<Vec<u8>> {
        let (start, size) = self.find(name).map(|e| (e.start, e.size))?;
        if size > STREAM_MAX {
            return None;
        }
        let size = size as usize;
        let mut out = Vec::new();
        if (size as u64) < self.cutoff {
            // 작은 스트림: 뿌리 항목이 가리키는 작은 섹터 저장소 안에 64바이트씩 놓인다
            let (root_start, root_size) = (self.entries[0].start, self.entries[0].size);
            let minis = chain(&self.mini_fat, start)?;
            if minis.len() * MINI < size {
                return None;
            }
            let store = chain(&self.fat, root_start)?;
            for k in minis {
                let off = k as usize * MINI;
                if off as u64 + MINI as u64 > root_size {
                    return None;
                }
                let s = self.read_sector(*store.get(off / self.sector)?)?;
                let at = off % self.sector;
                out.extend_from_slice(&s[at..at + MINI]);
                if out.len() >= size {
                    break;
                }
            }
        } else {
            let sectors = chain(&self.fat, start)?;
            if sectors.len() * self.sector < size {
                return None;
            }
            for n in sectors {
                out.extend(self.read_sector(n)?);
                if out.len() >= size {
                    break;
                }
            }
        }
        out.truncate(size);
        (out.len() == size).then_some(out)
    }
}

/// 시험·그림용: 맨 위에 스트림들만 담은 버전 3 복합 파일 (512바이트 섹터. 4096바이트보다 작은 스트림은 64바이트 작은 섹터에).
/// 섹터는 디렉터리, 작은 섹터 FAT, 작은 섹터 저장소, 큰 스트림들, FAT, DIFAT 차례로 놓는다.
/// FAT 섹터가 머리에 다 적히지 않으면(약 7MB를 넘으면) DIFAT 섹터를 더한다. 이름은 31자까지
pub fn build(streams: &[(&str, &[u8])]) -> Vec<u8> {
    const SECTOR: usize = 512;
    const PER: usize = SECTOR / 4;
    fn link(fat: &mut [u32], first: usize, n: usize) {
        for k in 0..n {
            fat[first + k] = if k + 1 == n { END } else { (first + k + 1) as u32 };
        }
    }
    fn put16(b: &mut [u8], at: usize, v: u16) {
        b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn put32(b: &mut [u8], at: usize, v: u32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn small(d: &[u8]) -> bool {
        d.len() < CUTOFF
    }
    // 작은 스트림을 작은 섹터 저장소에 잇대어 놓는다
    let mut starts = vec![END; streams.len()];
    let mut mini_fat: Vec<u32> = Vec::new();
    let mut store: Vec<u8> = Vec::new();
    for (i, (_, d)) in streams.iter().enumerate().filter(|(_, (_, d))| small(d) && !d.is_empty()) {
        let first = mini_fat.len();
        let n = d.len().div_ceil(MINI);
        mini_fat.resize(first + n, 0);
        link(&mut mini_fat, first, n);
        starts[i] = first as u32;
        store.extend_from_slice(d);
        store.resize(mini_fat.len() * MINI, 0);
    }
    let dir_n = (streams.len() + 1).div_ceil(SECTOR / ENTRY);
    let mini_fat_n = (mini_fat.len() * 4).div_ceil(SECTOR);
    let store_n = store.len().div_ceil(SECTOR);
    let mut next = dir_n + mini_fat_n + store_n;
    for (i, (_, d)) in streams.iter().enumerate().filter(|(_, (_, d))| !small(d)) {
        starts[i] = next as u32;
        next += d.len().div_ceil(SECTOR);
    }
    let data_n = next;
    // FAT 섹터 수: FAT는 자기 섹터와 DIFAT 섹터의 자리도 적는다. DIFAT 섹터 하나는 FAT 섹터 번호 127개를 담는다
    let mut fat_n: usize = 1;
    let difat_n = loop {
        let difat_n = fat_n.saturating_sub(HEADER_FAT).div_ceil(PER - 1);
        let need = (data_n + fat_n + difat_n).div_ceil(PER);
        if need <= fat_n {
            break difat_n;
        }
        fat_n = need;
    };
    let mut fat = vec![FREE; fat_n * PER];
    link(&mut fat, 0, dir_n);
    link(&mut fat, dir_n, mini_fat_n);
    link(&mut fat, dir_n + mini_fat_n, store_n);
    for (i, (_, d)) in streams.iter().enumerate().filter(|(_, (_, d))| !small(d)) {
        link(&mut fat, starts[i] as usize, d.len().div_ceil(SECTOR));
    }
    fat[data_n..data_n + fat_n].fill(FAT_SECT);
    fat[data_n + fat_n..data_n + fat_n + difat_n].fill(DIFAT_SECT);

    let mut out = vec![0u8; SECTOR];
    out[..8].copy_from_slice(&SIGNATURE);
    put16(&mut out, 0x18, 0x3E);
    put16(&mut out, 0x1A, 3);
    put16(&mut out, 0x1C, 0xFFFE);
    put16(&mut out, 0x1E, 9);
    put16(&mut out, 0x20, 6);
    put32(&mut out, 0x2C, fat_n as u32);
    put32(&mut out, 0x30, 0);
    put32(&mut out, 0x38, CUTOFF as u32);
    put32(&mut out, 0x3C, if mini_fat_n > 0 { dir_n as u32 } else { END });
    put32(&mut out, 0x40, mini_fat_n as u32);
    put32(&mut out, 0x44, if difat_n > 0 { (data_n + fat_n) as u32 } else { END });
    put32(&mut out, 0x48, difat_n as u32);
    for i in 0..HEADER_FAT {
        put32(&mut out, 0x4C + i * 4, if i < fat_n { (data_n + i) as u32 } else { FREE });
    }
    // 디렉터리: 뿌리, 그리고 스트림들 (항목 번호는 넘겨받은 차례). 빈 칸은 형제·자식이 없다
    let mut dir = vec![0u8; dir_n * SECTOR];
    for id in 0..dir_n * (SECTOR / ENTRY) {
        for at in [0x44, 0x48, 0x4C] {
            put32(&mut dir, id * ENTRY + at, NONE);
        }
    }
    // 스트림들은 이름 차례의 이진 탐색 나무로 잇는다 (COM은 이름을 이 나무에서 찾아 내려간다): 짧은 이름이 앞이고,
    // 길이가 같으면 대문자로 바꿔 견준다. 가운데를 뿌리로 고르게 나누고, 덜 찬 가장 깊은 줄만 빨강으로 칠한다 (레드·블랙 나무의 규칙)
    let mut order: Vec<usize> = (0..streams.len()).collect();
    order.sort_by_cached_key(|&i| {
        let name = streams[i].0;
        (name.encode_utf16().count(), name.to_uppercase().encode_utf16().collect::<Vec<u16>>())
    });
    fn tree(order: &[usize], depth: u32, links: &mut [(u32, u32, u32)]) -> u32 {
        if order.is_empty() {
            return NONE;
        }
        let mid = order.len() / 2;
        let left = tree(&order[..mid], depth + 1, links);
        let right = tree(&order[mid + 1..], depth + 1, links);
        links[order[mid]] = (left, right, depth);
        (order[mid] + 1) as u32
    }
    let mut links = vec![(NONE, NONE, 0u32); streams.len()];
    let top = tree(&order, 0, &mut links);
    let deepest = links.iter().map(|l| l.2).max().unwrap_or(0);
    let red_row = links.iter().filter(|l| l.2 == deepest).count() < 1 << deepest;
    let mut entry = |id: usize, name: &str, kind: u8, (left, right, depth): (u32, u32, u32), child: u32, start: u32, size: usize| {
        let e = &mut dir[id * ENTRY..(id + 1) * ENTRY];
        let units: Vec<u16> = name.encode_utf16().take(31).collect();
        for (k, u) in units.iter().enumerate() {
            put16(e, k * 2, *u);
        }
        put16(e, 0x40, ((units.len() + 1) * 2) as u16);
        e[0x42] = kind;
        // 0 빨강, 1 검정
        e[0x43] = u8::from(!(kind == STREAM && red_row && depth == deepest));
        put32(e, 0x44, left);
        put32(e, 0x48, right);
        put32(e, 0x4C, child);
        put32(e, 0x74, start);
        put32(e, 0x78, size as u32);
    };
    let store_start = if store_n > 0 { (dir_n + mini_fat_n) as u32 } else { END };
    entry(0, "Root Entry", ROOT, (NONE, NONE, 0), top, store_start, store.len());
    for (i, (name, d)) in streams.iter().enumerate() {
        entry(i + 1, name, STREAM, links[i], NONE, starts[i], d.len());
    }
    out.extend(dir);
    // 작은 섹터 FAT의 빈 칸은 FREE (0xFF 네 바이트)
    let mut words: Vec<u8> = mini_fat.iter().flat_map(|w| w.to_le_bytes()).collect();
    words.resize(mini_fat_n * SECTOR, 0xFF);
    out.extend(words);
    store.resize(store_n * SECTOR, 0);
    out.extend(&store);
    for (_, d) in streams.iter().filter(|(_, d)| !small(d)) {
        out.extend_from_slice(d);
        out.resize(out.len().div_ceil(SECTOR) * SECTOR, 0);
    }
    out.extend(fat.iter().flat_map(|w| w.to_le_bytes()));
    // DIFAT 섹터: 머리에 적지 못한 FAT 섹터 번호 127개씩, 마지막 칸은 다음 DIFAT 섹터
    let rest: Vec<u32> = (HEADER_FAT..fat_n).map(|i| (data_n + i) as u32).collect();
    for k in 0..difat_n {
        let mut words: Vec<u32> = rest.iter().skip(k * (PER - 1)).take(PER - 1).copied().collect();
        words.resize(PER - 1, FREE);
        words.push(if k + 1 < difat_n { (data_n + fat_n + k + 1) as u32 } else { END });
        out.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn open(b: &[u8]) -> Option<Cfb<Cursor<&[u8]>>> {
        Cfb::open(Cursor::new(b))
    }

    fn pattern(n: usize, seed: u8) -> Vec<u8> {
        (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed).wrapping_add((i >> 8) as u8)).collect()
    }

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    /// 한글 문서 같은 파일: 큰 그림(일반 섹터), 작은 글(작은 섹터), 그 밖의 스트림 몇 개 (디렉터리가 두 섹터에 걸친다)
    fn hwp_like() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let image = pattern(5000, 7);
        let text = utf16("회의 자료\r\n둘째 줄");
        let extra: Vec<Vec<u8>> = (0..5).map(|i| pattern(100 + i * 900, i as u8)).collect();
        let mut streams: Vec<(&str, &[u8])> = vec![("FileHeader", &extra[0]), ("PrvImage", &image), ("DocInfo", &extra[1])];
        streams.extend([("PrvText", text.as_slice()), ("Scripts", &extra[2]), ("DocOptions", &extra[3]), ("BodyText", &extra[4])]);
        (build(&streams), image, text)
    }

    /// FAT에서 sector 다음 섹터를 value로 바꾼다 (FAT 섹터의 자리는 머리에 적혀 있다)
    fn set_fat(file: &mut [u8], sector: u32, value: u32) {
        let fat_sector = u32_at(file, 0x4C + sector as usize / 128 * 4) as usize;
        let at = (fat_sector + 1) * 512 + sector as usize % 128 * 4;
        file[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// 디렉터리 항목 id의 at 자리(u32)를 바꾼다
    fn set_entry(file: &mut [u8], id: usize, at: usize, value: u32) {
        let dir = u32_at(file, 0x30) as usize;
        let first = chain_in_file(file, dir as u32);
        let base = (first[id / 4] as usize + 1) * 512 + id % 4 * ENTRY + at;
        file[base..base + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// 시험 파일(FAT 섹터가 머리에 다 적힌 것)에서 섹터 사슬
    fn chain_in_file(file: &[u8], start: u32) -> Vec<u32> {
        let n_fat = u32_at(file, 0x2C) as usize;
        let fat: Vec<u32> = (0..n_fat).flat_map(|i| {
            let at = (u32_at(file, 0x4C + i * 4) as usize + 1) * 512;
            u32s(&file[at..at + 512]).collect::<Vec<_>>()
        }).collect();
        chain(&fat, start).unwrap()
    }

    #[test]
    fn reads_big_and_small_streams_at_the_top() {
        let (file, image, text) = hwp_like();
        let mut f = open(&file).expect("a valid file");
        assert_eq!(f.stream("PrvImage"), Some(image));
        assert_eq!(f.stream("PrvText"), Some(text.clone()));
        // 이름의 대소문자는 가리지 않는다 (복합 파일의 규칙). 없는 이름과 뿌리 항목은 스트림이 아니다
        assert_eq!(f.stream("prvtext"), Some(text));
        assert_eq!(f.stream("BodyText").map(|b| b.len()), Some(100 + 4 * 900));
        assert_eq!(f.stream("없음"), None);
        assert_eq!(f.stream("Root Entry"), None);
        // 작은 섹터와 일반 섹터의 경계 (4096바이트부터 일반 섹터), 빈 스트림
        let (below, at) = (pattern(CUTOFF - 1, 1), pattern(CUTOFF, 2));
        let file = build(&[("Below", &below), ("At", &at), ("Empty", &[])]);
        let mut f = open(&file).unwrap();
        assert_eq!((f.stream("Below"), f.stream("At"), f.stream("Empty")), (Some(below), Some(at), Some(Vec::new())));
        // 스트림이 하나도 없는 파일
        assert_eq!(open(&build(&[])).unwrap().stream("PrvText"), None);
    }

    /// 7MB가 넘는 파일: FAT 섹터 번호가 머리에 다 적히지 않아 DIFAT 섹터 사슬을 따라간다
    #[test]
    fn follows_the_difat_chain_of_big_files() {
        let big = pattern(110 * 128 * 512, 3);
        let tail = utf16("끝");
        let file = build(&[("Big", &big), ("PrvText", &tail)]);
        assert!(u32_at(&file, 0x48) >= 1 && u32_at(&file, 0x2C) as usize > HEADER_FAT, "the file has DIFAT sectors");
        let mut f = open(&file).unwrap();
        assert!(f.stream("Big") == Some(big), "the big stream reads back");
        assert_eq!(f.stream("PrvText"), Some(tail));
    }

    /// 복합 파일이 아니거나 머리가 망가진 것: 열지 않는다
    #[test]
    fn refuses_what_is_not_a_compound_file() {
        assert!(open(b"").is_none() && open(b"hello").is_none() && open(&[0u8; 2048]).is_none());
        let (file, ..) = hwp_like();
        let with = |at: usize, v: &[u8]| {
            let mut f = file.clone();
            f[at..at + v.len()].copy_from_slice(v);
            f
        };
        // 섹터 크기, 작은 섹터 크기, 버전, 바이트 순서
        assert!(open(&with(0x1E, &30u16.to_le_bytes())).is_none());
        assert!(open(&with(0x20, &7u16.to_le_bytes())).is_none());
        assert!(open(&with(0x1A, &4u16.to_le_bytes())).is_none());
        assert!(open(&with(0x1C, &[0xFF, 0xFF])).is_none());
        // FAT 섹터 수가 파일보다 많다 (큰 표를 만들지 않는다)
        assert!(open(&with(0x2C, &u32::MAX.to_le_bytes())).is_none());
        // 디렉터리가 파일 밖에서 시작하거나, 맨 앞 항목이 뿌리가 아니다
        assert!(open(&with(0x30, &9999u32.to_le_bytes())).is_none());
        assert!(open(&with(512 + 0x42, &[STREAM])).is_none());
    }

    /// 사슬이 망가진 파일: 그 스트림만 None이고, 고리를 이뤄도 끝난다
    #[test]
    fn broken_chains_give_none_without_looping() {
        let big = pattern(5000, 9);
        let small = utf16("작은 글");
        let file = build(&[("Big", &big), ("Small", &small)]);
        // 디렉터리 1, 작은 섹터 FAT 1, 작은 섹터 저장소 1, 그 뒤가 큰 스트림 (10섹터)
        let first = 3u32;
        assert_eq!(chain_in_file(&file, first).len(), 10);
        let broken = |patch: &dyn Fn(&mut Vec<u8>)| {
            let mut f = file.clone();
            patch(&mut f);
            let mut c = open(&f).expect("the frame is intact");
            (c.stream("Big"), c.stream("Small"))
        };
        // 마지막 섹터가 첫 섹터로 돌아간다, 섹터가 자기 자신을 가리킨다, 파일 밖 섹터, 빈 섹터 표시
        assert_eq!(broken(&|f| set_fat(f, first + 9, first)).0, None);
        assert_eq!(broken(&|f| set_fat(f, first + 4, first + 4)).0, None);
        assert_eq!(broken(&|f| set_fat(f, first + 4, 1_000_000)).0, None);
        assert_eq!(broken(&|f| set_fat(f, first + 4, FREE)).0, None);
        // 사슬보다 큰 크기, 64MB보다 큰 크기
        assert_eq!(broken(&|f| set_entry(f, 1, 0x78, 20_000)).0, None);
        assert_eq!(broken(&|f| set_entry(f, 1, 0x78, (STREAM_MAX + 1) as u32)).0, None);
        // 작은 스트림: 작은 섹터 FAT의 고리, 작은 섹터 저장소 밖의 작은 섹터
        let mini_fat_at = 2 * 512;
        let r = broken(&|f| f[mini_fat_at..mini_fat_at + 4].copy_from_slice(&0u32.to_le_bytes()));
        assert_eq!(r, (Some(big.clone()), None));
        assert_eq!(broken(&|f| set_entry(f, 2, 0x74, 50)).1, None);
        // 형제 나무의 고리: 자기를 오른쪽 형제로, 뿌리의 자식이 뿌리
        assert_eq!(broken(&|f| set_entry(f, 2, 0x48, 2)), (Some(big.clone()), Some(small.clone())));
        let mut f = file.clone();
        set_entry(&mut f, 1, 0x48, 1);
        set_entry(&mut f, 0, 0x4C, 0);
        assert_eq!(open(&f).unwrap().stream("Small"), None);
        // 잘린 파일: 뼈대가 남으면 열리지만 잘려 나간 섹터의 스트림은 없다
        let cut = &file[..file.len() - 512 * 3];
        if let Some(mut c) = open(cut) {
            assert_eq!(c.stream("Big"), None);
        }
        assert!(open(&file[..600]).is_none());
    }

    /// 아무렇게나 망가뜨린 파일 수천 개: 어느 것도 멈추거나 패닉하지 않는다
    #[test]
    fn random_damage_never_panics() {
        let (file, ..) = hwp_like();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..3000 {
            let mut f = file.clone();
            for _ in 0..1 + rand() % 8 {
                let at = rand() as usize % f.len();
                f[at] = rand() as u8;
            }
            if round % 5 == 0 {
                f.truncate(rand() as usize % f.len());
            }
            if let Some(mut c) = open(&f) {
                let _ = c.stream("PrvImage");
                let _ = c.stream("PrvText");
            }
        }
    }
}
