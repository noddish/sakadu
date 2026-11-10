//! 폴더 항목(Entry)과 폴더 열거, 네트워크 폴더 오류, 표시용 포맷.
//! 폴더 열거와 시간 변환은 Windows(Win32)와 맥·리눅스(표준 라이브러리, 맥 시간대는 Core Foundation)에 따로 있다.
//! 네트워크 연결은 Windows에서만 빌드된다 ("내 PC" 목록은 shell.rs)

#[cfg(windows)]
use crate::util;
#[cfg(windows)]
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, FILETIME, SYSTEMTIME};
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
#[cfg(windows)]
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
#[cfg(windows)]
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

#[derive(Debug, Clone)]
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

/// 네트워크 폴더를 열지 못했을 때 고칠 방법: 이 공유에 로그인(ask)하거나, 끊긴 연결 드라이브를 다시 연결한다
#[derive(Clone, Debug, PartialEq)]
pub struct NetFix {
    pub share: String,
    /// 로그인 창을 늘 띄운다 (계정·비밀번호 문제)
    pub ask: bool,
}

/// "\\서버\공유\a\b" → "\\서버\공유"
pub fn share_root(path: &str) -> Option<String> {
    let rest = path.strip_prefix("\\\\")?;
    let mut parts = rest.splitn(3, '\\');
    let server = parts.next().filter(|s| !s.is_empty())?;
    let share = parts.next().filter(|s| !s.is_empty())?;
    Some(format!("\\\\{server}\\{share}"))
}

/// 로그인이 필요해서 못 여는 오류 (접근 거부, 잘못된 비밀번호, 인증 안 됨, 다른 계정으로 연결됨, 계정 제한·잠김)
#[cfg(windows)]
fn needs_login(code: u32) -> bool {
    matches!(code, 5 | 86 | 1219 | 1244 | 1326 | 1327 | 1331 | 1385 | 1909)
}

/// 서버·연결에 닿지 못한 오류 (경로 없음, 네트워크 경로·이름 없음, 연결 끊김)
#[cfg(windows)]
fn unreachable(code: u32) -> bool {
    matches!(code, 3 | 53 | 64 | 67 | 1201 | 1222 | 2250)
}

#[cfg(windows)]
thread_local! {
    /// 이 스레드에서 list_dir이 마지막으로 받은 Win32 오류 번호
    static LAST_LIST_ERROR: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// 같은 스레드에서 방금 실패한 list_dir의 Win32 오류 번호
#[cfg(windows)]
pub fn last_list_error() -> u32 {
    LAST_LIST_ERROR.get()
}

/// 네트워크 폴더를 열지 못한 까닭(오류 번호 code)에 맞는 고칠 방법. 연결 드라이브 이름을 물으므로 뒤 스레드에서 부른다
#[cfg(windows)]
pub fn net_fix(dir: &str, code: u32) -> Option<NetFix> {
    if let Some(share) = share_root(dir) {
        return needs_login(code).then_some(NetFix { share, ask: true });
    }
    // 연결 드라이브(Z: 등)는 연결해 둔 원격 경로로
    let remote = mapped_remote(&dir.chars().take(2).collect::<String>())?;
    (needs_login(code) || unreachable(code)).then_some(NetFix { share: remote, ask: needs_login(code) })
}

/// 연결 드라이브의 원격 경로. 끊겨 있어도(ERROR_CONNECTION_UNAVAIL) 기억해 둔 이름을 준다
#[cfg(windows)]
fn mapped_remote(drive: &str) -> Option<String> {
    use windows::Win32::NetworkManagement::WNet::WNetGetConnectionW;
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let r = unsafe { WNetGetConnectionW(&HSTRING::from(drive), Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut len) };
    if r.0 != 0 && r.0 != 1201 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
}

/// Windows의 네트워크 로그인 창으로 공유에 연결한다 (계정·비밀번호는 Windows가 받고 저장한다).
/// 0이면 연결됨, 1223이면 사용자가 취소. 창이 떠 있는 동안 돌아오지 않으므로 App을 빌리지 않은 채 부른다
#[cfg(windows)]
pub fn net_connect(owner: windows::Win32::Foundation::HWND, share: &str, ask: bool) -> u32 {
    use windows::Win32::NetworkManagement::WNet::{CONNECT_INTERACTIVE, CONNECT_PROMPT, NETRESOURCEW, RESOURCETYPE_DISK, WNetUseConnectionW};
    let remote: Vec<u16> = share.encode_utf16().chain(Some(0)).collect();
    let nr = NETRESOURCEW { dwType: RESOURCETYPE_DISK, lpRemoteName: windows::core::PWSTR(remote.as_ptr() as *mut u16), ..Default::default() };
    let flags = if ask { CONNECT_INTERACTIVE | CONNECT_PROMPT } else { CONNECT_INTERACTIVE };
    unsafe { WNetUseConnectionW(Some(owner), &nr, windows::core::PCWSTR::null(), windows::core::PCWSTR::null(), flags, None, None, None) }.0
}

#[cfg(test)]
mod net_tests {
    use super::*;

    #[test]
    fn share_root_keeps_server_and_share() {
        assert_eq!(share_root("\\\\nas\\자료\\2026\\11월").as_deref(), Some("\\\\nas\\자료"));
        assert_eq!(share_root("\\\\192.168.0.10\\공유").as_deref(), Some("\\\\192.168.0.10\\공유"));
        assert_eq!(share_root("\\\\nas"), None);
        assert_eq!(share_root("C:\\자료"), None);
    }

    #[cfg(windows)]
    #[test]
    fn login_and_unreachable_codes_are_apart() {
        assert!(needs_login(1326) && needs_login(5) && !needs_login(53));
        assert!(unreachable(53) && unreachable(67) && !unreachable(1326));
    }
}

#[cfg(windows)]
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
        Err(e) => {
            LAST_LIST_ERROR.set(e.code().0 as u32 & 0xFFFF);
            return Err(e.message());
        }
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

#[cfg(windows)]
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

#[cfg(windows)]
pub fn filetime_u64(ft: &FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// 1601-01-01부터 1970-01-01까지 (FILETIME 단위 100ns)
#[cfg(not(windows))]
const UNIX_EPOCH_FT: u64 = 116_444_736_000_000_000;

/// 시각을 FILETIME 단위(1601년부터 100ns)로. Windows와 같은 값이라 날짜 표시 코드를 함께 쓴다
#[cfg(not(windows))]
pub fn filetime_of(t: std::time::SystemTime) -> u64 {
    let ticks = |d: std::time::Duration| d.as_secs() * 10_000_000 + u64::from(d.subsec_nanos()) / 100;
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => UNIX_EPOCH_FT + ticks(d),
        Err(e) => UNIX_EPOCH_FT.saturating_sub(ticks(e.duration())),
    }
}

/// 폴더 열거 (맥·리눅스). 심볼릭 링크는 가리키는 대상의 종류·크기를 쓰고, 대상이 없으면 링크 자체를 쓴다.
/// 숨김: 이름이 점으로 시작하거나, 맥 Finder의 '숨김' 표시(UF_HIDDEN)가 있는 항목
#[cfg(not(windows))]
pub fn list_dir(dir: &str) -> Result<Vec<Entry>, String> {
    let items = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(128);
    for item in items.flatten() {
        let path = item.path();
        let Ok(meta) = std::fs::metadata(&path).or_else(|_| std::fs::symlink_metadata(&path)) else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let kind = if meta.is_dir() { Kind::Dir } else { Kind::File };
        let mut e = Entry::from_str(&name, kind);
        e.attrs = if kind == Kind::Dir { ATTR_DIR } else { 0 };
        if name.starts_with('.') || finder_hidden(&meta) {
            e.attrs |= ATTR_HIDDEN;
        }
        if kind == Kind::File {
            e.size = meta.len();
        }
        e.modified = meta.modified().map(filetime_of).unwrap_or(0);
        out.push(e);
    }
    Ok(out)
}

#[cfg(target_os = "macos")]
fn finder_hidden(meta: &std::fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    // UF_HIDDEN (sys/stat.h)
    meta.st_flags() & 0x8000 != 0
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn finder_hidden(_meta: &std::fs::Metadata) -> bool {
    false
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
#[cfg(windows)]
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

/// 현지 시간 "2026-11-02 14:03" (맥·리눅스)
#[cfg(not(windows))]
pub fn format_time(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    match local_time(ft) {
        Some(t) => format!("{:04}-{:02}-{:02} {:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute),
        None => String::new(),
    }
}

/// 지금 시각 (FILETIME과 같은 100ns 단위)
#[cfg(windows)]
pub fn now_filetime() -> u64 {
    filetime_u64(&unsafe { GetSystemTimeAsFileTime() })
}

/// 지금 시각 (FILETIME과 같은 100ns 단위)
#[cfg(not(windows))]
pub fn now_filetime() -> u64 {
    filetime_of(std::time::SystemTime::now())
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

#[cfg(windows)]
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

/// 맥·리눅스: 그 시각의 현지 날짜·시·분
#[cfg(not(windows))]
fn local_time(ft: u64) -> Option<LocalTime> {
    let unix = (ft as i64 - UNIX_EPOCH_FT as i64).div_euclid(10_000_000);
    let local = unix + utc_offset_secs(unix);
    let (days, secs) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    Some(LocalTime { year, month, day, hour: (secs / 3600) as u16, minute: (secs % 3600 / 60) as u16 })
}

/// 그 시각(1970년부터 초)의 현지 시간대 UTC 차이(초). 맥은 시스템 시간대를 따른다 (일광 절약 시간 반영)
#[cfg(target_os = "macos")]
fn utc_offset_secs(unix_secs: i64) -> i64 {
    use objc2_core_foundation::CFTimeZone;
    // CFAbsoluteTime은 2001-01-01 UTC부터 초
    let at = (unix_secs - 978_307_200) as f64;
    CFTimeZone::system().map(|tz| tz.seconds_from_gmt(at) as i64).unwrap_or(0)
}

/// 맥이 아닌 유닉스는 아직 UTC로 보인다 (사카두가 지원하는 OS는 Windows와 맥)
#[cfg(all(not(windows), not(target_os = "macos")))]
fn utc_offset_secs(_unix_secs: i64) -> i64 {
    0
}

/// day_number의 거꾸로: 1970-01-01부터 며칠째인지 → (년, 월, 일)
#[cfg(not(windows))]
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
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
/// secs는 now - t (초). 시계가 조금 빠른 곳(NAS·다른 PC)에서 막 저장한 파일은 몇 분 앞설 수 있어 5분까지는 "방금",
/// 같은 날이면 "오늘", 그보다 먼 앞날은 날짜와 시각 그대로 (오래된 파일처럼 보이지 않게)
pub fn relative_label(t: LocalTime, now: LocalTime, secs: i64) -> String {
    if (-300..60).contains(&secs) {
        return "방금".to_string();
    }
    let days = day_number(now.year, now.month, now.day) - day_number(t.year, t.month, t.day);
    match days {
        0 => format!("오늘 {:02}:{:02}", t.hour, t.minute),
        1 => format!("어제 {:02}:{:02}", t.hour, t.minute),
        d if d > 1 && t.year == now.year => format!("{}월 {}일", t.month, t.day),
        d if d > 1 => format!("{:04}-{:02}-{:02}", t.year, t.month, t.day),
        _ => format!("{:04}-{:02}-{:02} {:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute),
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
        // 시계가 조금 빠른 곳에서 막 저장한 파일: 몇 분 앞서도 "방금", 같은 날이면 "오늘"
        assert_eq!(relative_label(at(2026, 11, 6, 14, 32), now, -120), "방금");
        assert_eq!(relative_label(at(2026, 11, 6, 16, 0), now, -5400), "오늘 16:00");
        // 그보다 먼 앞날은 날짜와 시각 그대로
        assert_eq!(relative_label(at(2026, 11, 7, 9, 0), now, -3600 * 18), "2026-11-07 09:00");
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

    #[cfg(windows)]
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

    #[cfg(windows)]
    #[test]
    fn missing_dir_is_error() {
        assert!(list_dir("C:\\__sakadu_does_not_exist__").is_err());
    }

    /// 맥·리눅스 폴더 열거: 한글 이름, 크기, 폴더, 점으로 시작하는 숨김, 없는 폴더는 오류
    #[cfg(not(windows))]
    #[test]
    fn lists_temp_dir_unix() {
        let dir = std::env::temp_dir().join(format!("sakadu-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("한글 파일.txt"), b"hello").unwrap();
        std::fs::write(dir.join(".숨김"), b"").unwrap();
        let entries = list_dir(dir.to_str().unwrap()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(entries.len(), 3);
        let file = entries.iter().find(|e| e.name_string() == "한글 파일.txt").unwrap();
        assert_eq!((file.kind, file.size, file.is_hidden()), (Kind::File, 5, false));
        assert!(file.modified > UNIX_EPOCH_FT);
        assert!(entries.iter().any(|e| e.kind == Kind::Dir && e.name_string() == "sub" && !e.is_hidden()));
        assert!(entries.iter().any(|e| e.name_string() == ".숨김" && e.is_hidden()));
        assert!(list_dir("/__sakadu_does_not_exist__").is_err());
    }

    /// 날짜 일련번호와 그 거꾸로가 맞고, FILETIME 변환과 현지 시각이 말이 된다
    #[cfg(not(windows))]
    #[test]
    fn dates_round_trip_unix() {
        for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 11, 10), (1601, 1, 1), (2100, 12, 31)] {
            assert_eq!(civil_from_days(day_number(y, m, d)), (y, m, d));
        }
        assert_eq!(filetime_of(std::time::UNIX_EPOCH), UNIX_EPOCH_FT);
        // 2027-01-15 08:00 UTC: 어느 시간대든 현지 날짜는 그 앞뒤 하루 안
        let t = local_time(filetime_of(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000))).unwrap();
        assert_eq!((t.year, t.month), (2027, 1));
        assert!((14..=16).contains(&t.day), "{t:?}");
        assert!(format_time(now_filetime()).starts_with("20"));
    }
}
