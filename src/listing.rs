//! 폴더 열거, "내 PC" 목록, 표시용 포맷

use crate::{shell, util};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, FILETIME, SYSTEMTIME};
use windows::Win32::Storage::FileSystem::{
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, WIN32_FIND_DATAW,
};
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos,
};
use windows::core::HSTRING;

pub const ATTR_HIDDEN: u32 = 0x2;
pub const ATTR_DIR: u32 = 0x10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    File,
    Dir,
    Drive,
    Known,
}

#[derive(Debug)]
pub struct Entry {
    /// 끝에 NUL을 포함해 Win32 API에 바로 넘길 수 있게 저장
    name: Box<[u16]>,
    pub attrs: u32,
    pub size: u64,
    pub modified: u64,
    pub kind: Kind,
    pub free: u64,
    pub drive_type: u32,
    /// "내 PC" 항목처럼 이름과 경로가 다른 경우의 전체 경로
    pub target: Option<Box<str>>,
    /// 시스템 이미지 목록 인덱스. -1이면 확장자 기준 아이콘을 쓴다
    pub icon: i32,
    pub selected: bool,
}

impl Entry {
    pub fn new(name: &[u16], kind: Kind) -> Entry {
        let mut v = Vec::with_capacity(name.len() + 1);
        v.extend_from_slice(name);
        v.push(0);
        Entry {
            name: v.into_boxed_slice(),
            attrs: 0,
            size: 0,
            modified: 0,
            kind,
            free: 0,
            drive_type: 0,
            target: None,
            icon: -1,
            selected: false,
        }
    }

    pub fn from_str(name: &str, kind: Kind) -> Entry {
        Entry::new(&name.encode_utf16().collect::<Vec<_>>(), kind)
    }

    pub fn name(&self) -> &[u16] {
        &self.name[..self.name.len() - 1]
    }

    pub fn name_z(&self) -> &[u16] {
        &self.name
    }

    pub fn name_string(&self) -> String {
        String::from_utf16_lossy(self.name())
    }

    pub fn is_dir(&self) -> bool {
        self.kind != Kind::File
    }

    pub fn is_hidden(&self) -> bool {
        self.attrs & ATTR_HIDDEN != 0
    }

    /// 확장자(점 제외). 파일만 해당하고, 맨 앞의 점(".gitignore")은 확장자로 보지 않는다
    pub fn ext(&self) -> &[u16] {
        if self.kind != Kind::File {
            return &[];
        }
        let n = self.name();
        match n.iter().rposition(|&c| c == b'.' as u16) {
            Some(i) if i > 0 => &n[i + 1..],
            _ => &[],
        }
    }

    pub fn ext_lower(&self) -> String {
        String::from_utf16_lossy(self.ext()).to_lowercase()
    }
}

pub fn list_dir(dir: &str) -> Result<Vec<Entry>, String> {
    let pattern = HSTRING::from(util::join(dir, "*"));
    let mut data = WIN32_FIND_DATAW::default();
    let handle = match unsafe {
        FindFirstFileExW(
            &pattern,
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    } {
        Ok(h) => h,
        // 비어 있는 드라이브 루트는 "."조차 없어서 이 오류가 난다
        Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => return Ok(Vec::new()),
        Err(e) => return Err(e.message()),
    };
    let mut out = Vec::with_capacity(128);
    loop {
        if let Some(e) = from_find_data(&data) {
            out.push(e);
        }
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(handle);
    }
    Ok(out)
}

fn from_find_data(d: &WIN32_FIND_DATAW) -> Option<Entry> {
    let len = d.cFileName.iter().position(|&c| c == 0).unwrap_or(d.cFileName.len());
    let name = &d.cFileName[..len];
    let dot = b'.' as u16;
    if name == [dot] || name == [dot, dot] {
        return None;
    }
    let kind = if d.dwFileAttributes & ATTR_DIR != 0 { Kind::Dir } else { Kind::File };
    let mut e = Entry::new(name, kind);
    e.attrs = d.dwFileAttributes;
    e.size = ((d.nFileSizeHigh as u64) << 32) | d.nFileSizeLow as u64;
    e.modified = filetime_u64(&d.ftLastWriteTime);
    Some(e)
}

/// "내 PC": 주요 사용자 폴더 + 드라이브. 네트워크·광학 드라이브 때문에 느릴 수 있어 백그라운드에서 호출한다
pub fn list_computer() -> Vec<Entry> {
    let mut out = Vec::new();
    for id in [
        FOLDERID_Desktop,
        FOLDERID_Downloads,
        FOLDERID_Documents,
        FOLDERID_Pictures,
        FOLDERID_Music,
        FOLDERID_Videos,
    ] {
        if let Some(path) = shell::known_folder(&id) {
            let (name, icon) = shell::display_info(&path);
            let mut e = Entry::from_str(&name, Kind::Known);
            e.attrs = ATTR_DIR;
            e.icon = icon;
            e.target = Some(path.into());
            out.push(e);
        }
    }
    out.extend(list_drives());
    out
}

/// 드라이브마다 표시 이름·아이콘·종류·남은/전체 용량. 네트워크·광 드라이브는 느릴 수 있으니 백그라운드에서 부른다
pub fn list_drives() -> Vec<Entry> {
    let mut out = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u8 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i) as char);
        let h = HSTRING::from(root.as_str());
        let (name, icon) = shell::display_info(&root);
        let mut e = Entry::from_str(&name, Kind::Drive);
        e.attrs = ATTR_DIR;
        e.icon = icon;
        e.drive_type = unsafe { GetDriveTypeW(&h) };
        let (mut free, mut total) = (0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(&h, Some(&mut free), Some(&mut total), None) }.is_ok() {
            e.free = free;
            e.size = total;
        }
        e.target = Some(root.into());
        out.push(e);
    }
    out
}

pub fn filetime_u64(ft: &FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// 탐색기처럼 읽기 쉬운 크기: 512 B, 1.5 KB, 23 KB, 4.7 GB
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v < 10.0 { format!("{:.1} {}", v, UNITS[u]) } else { format!("{:.0} {}", v, UNITS[u]) }
}

/// 현지 시간 "2026-11-02 14:03" (일광 절약 시간 반영)
pub fn format_time(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            return String::new();
        }
    }
    format!("{:04}-{:02}-{:02} {:02}:{:02}", local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute)
}

/// 지금 시각 (FILETIME과 같은 100ns 단위)
pub fn now_filetime() -> u64 {
    filetime_u64(&unsafe { GetSystemTimeAsFileTime() })
}

/// 현지 시각의 날짜·시·분
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocalTime {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: u16,
    pub minute: u16,
}

fn local_time(ft: u64) -> Option<LocalTime> {
    let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&ft, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(LocalTime { year: local.wYear as i64, month: local.wMonth as i64, day: local.wDay as i64, hour: local.wHour, minute: local.wMinute })
}

/// 날짜의 일련번호 (1970-01-01이 0). 두 날짜가 며칠 떨어졌는지 셀 때 쓴다
fn day_number(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 목록의 짧은 날짜: 1분 안이면 "방금", 오늘·어제는 "오늘 10:24"·"어제 18:40", 올해는 "11월 3일", 그 전은 "2025-03-04".
/// secs는 now - t (초). 시계가 어긋나 앞날이면 날짜 그대로
pub fn relative_label(t: LocalTime, now: LocalTime, secs: i64) -> String {
    if (0..60).contains(&secs) {
        return "방금".to_string();
    }
    let days = day_number(now.year, now.month, now.day) - day_number(t.year, t.month, t.day);
    match days {
        0 if secs >= 0 => format!("오늘 {:02}:{:02}", t.hour, t.minute),
        1 => format!("어제 {:02}:{:02}", t.hour, t.minute),
        d if d > 1 && t.year == now.year => format!("{}월 {}일", t.month, t.day),
        _ => format!("{:04}-{:02}-{:02}", t.year, t.month, t.day),
    }
}

/// 목록의 수정한 날짜를 짧게 (now는 now_filetime). 읽을 수 없으면 빈 글자
pub fn format_time_short(ft: u64, now: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    match (local_time(ft), local_time(now)) {
        (Some(t), Some(n)) => relative_label(t, n, (now as i64 - ft as i64) / 10_000_000),
        _ => String::new(),
    }
}

pub fn drive_type_label(t: u32) -> &'static str {
    match t {
        2 => "이동식 디스크",
        3 => "로컬 디스크",
        4 => "네트워크 드라이브",
        5 => "CD/DVD 드라이브",
        6 => "RAM 디스크",
        _ => "드라이브",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i64, month: i64, day: i64, hour: u16, minute: u16) -> LocalTime {
        LocalTime { year, month, day, hour, minute }
    }

    #[test]
    fn short_dates_read_like_the_concept() {
        let now = at(2026, 11, 6, 14, 30);
        assert_eq!(relative_label(at(2026, 11, 6, 14, 29), now, 30), "방금");
        assert_eq!(relative_label(at(2026, 11, 6, 10, 24), now, 4 * 3600), "오늘 10:24");
        assert_eq!(relative_label(at(2026, 11, 5, 18, 40), now, 20 * 3600), "어제 18:40");
        assert_eq!(relative_label(at(2026, 11, 3, 9, 0), now, 3 * 86400), "11월 3일");
        assert_eq!(relative_label(at(2025, 3, 4, 9, 0), now, 400 * 86400), "2025-03-04");
        // 해가 바뀐 첫날의 어제는 지난해 12월 31일
        assert_eq!(relative_label(at(2025, 12, 31, 23, 50), at(2026, 1, 1, 0, 10), 1200), "어제 23:50");
        // 3월 1일 전날은 윤년이면 2월 29일
        assert_eq!(relative_label(at(2028, 2, 29, 8, 0), at(2028, 3, 1, 8, 0), 86400), "어제 08:00");
        // 시계가 어긋나 조금 앞선 시각은 날짜 그대로
        assert_eq!(relative_label(at(2026, 11, 7, 9, 0), now, -3600 * 18), "2026-11-07");
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(23 * 1024), "23 KB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn extensions() {
        assert_eq!(Entry::from_str("a.TXT", Kind::File).ext_lower(), "txt");
        assert_eq!(Entry::from_str(".gitignore", Kind::File).ext_lower(), "");
        assert_eq!(Entry::from_str("noext", Kind::File).ext_lower(), "");
        assert_eq!(Entry::from_str("dir.d", Kind::Dir).ext_lower(), "");
    }

    #[test]
    fn lists_temp_dir() {
        let dir = std::env::temp_dir().join(format!("sakadu-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("한글 파일.txt"), b"hello").unwrap();
        let entries = list_dir(dir.to_str().unwrap()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(entries.len(), 2);
        let file = entries.iter().find(|e| e.kind == Kind::File).unwrap();
        assert_eq!(file.name_string(), "한글 파일.txt");
        assert_eq!(file.size, 5);
        assert!(entries.iter().any(|e| e.kind == Kind::Dir && e.name_string() == "sub"));
    }

    #[test]
    fn missing_dir_is_error() {
        assert!(list_dir("C:\\__sakadu_does_not_exist__").is_err());
    }
}
