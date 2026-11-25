//! 디스크와 네트워크 공유 (맥): 붙어 있는 디스크 목록(용량, 네트워크 디스크인지), 디스크 꺼내기,
//! 주소 칸에 친 smb:// 따위의 공유에 연결하기.
//! 연결은 맥의 NetFS(/System/Library/Frameworks/NetFS.framework)를 처음 쓸 때 열어서 한다 (실행 파일에 링크하지 않는다).
//! 이름과 암호는 넘기지 않는다: 필요하면 맥이 제 로그인 창을 띄우고, 키체인에 저장해 둔 것도 맥이 쓴다.
//! 연결과 꺼내기는 오래 걸릴 수 있어 창 쪽이 뒷줄에서 부른다

use objc2::rc::Retained;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSArray, NSString, NSURL};
use std::ffi::{CStr, c_int, c_void};
use std::sync::OnceLock;

const NETFS: &CStr = c"/System/Library/Frameworks/NetFS.framework/NetFS";

/// NetFSMountURLSync (NetFS.h): 주소, 붙일 자리(비우면 /Volumes 아래), 이름, 암호, 여는 선택지, 붙이는 선택지, 붙은 자리들(CFArray, 받은 쪽이 놓는다)
type MountSync = unsafe extern "C" fn(*const c_void, *const c_void, *const c_void, *const c_void, *mut c_void, *mut c_void, *mut *const c_void) -> c_int;

/// 사용자가 로그인 창에서 그만뒀다 (userCanceledErr)
const USER_CANCELED: c_int = -128;
/// 서버에 열 수 있는 공유 폴더가 없다 (NetFS.h의 ENETFSNOSHARESAVAIL, NetAuth의 kNetAuthErrorNoSharesAvailable)
const NO_SHARES: [c_int; 2] = [-5998, -6003];

/// 붙어 있는 디스크 하나 (getfsstat)
pub struct Mount {
    /// 붙은 자리 ("/Volumes/USB")
    pub on: String,
    /// 어디서 왔나 (디스크 장치 "/dev/disk4s1", 공유 "//hong@nas.local/사진", NFS "nas:/export")
    pub from: String,
    /// 남은 바이트, 전체 바이트
    pub free: u64,
    pub total: u64,
    /// 이 맥에 붙은 디스크다 (네트워크 디스크가 아니다)
    pub local: bool,
}

/// 지금 붙어 있는 디스크들. 꺼진 네트워크 디스크를 기다리지 않는다 (MNT_NOWAIT: 맥이 알고 있는 값으로)
pub fn mounts() -> Vec<Mount> {
    // SAFETY: 받을 곳 없이 개수만 묻는다
    let n = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if n <= 0 {
        return Vec::new();
    }
    // 그사이 붙은 디스크를 위해 넉넉히
    let mut list: Vec<libc::statfs> = Vec::with_capacity(n as usize + 16);
    let bytes = (list.capacity() * std::mem::size_of::<libc::statfs>()) as c_int;
    // SAFETY: list의 칸만큼만 채운다
    let got = unsafe { libc::getfsstat(list.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
    if got <= 0 {
        return Vec::new();
    }
    // SAFETY: getfsstat이 앞의 got칸을 채웠다
    unsafe { list.set_len(got as usize) };
    // SAFETY: 이름 칸들은 NUL로 끝나는 C 문자열이다
    let text = |s: &[std::ffi::c_char]| unsafe { CStr::from_ptr(s.as_ptr()) }.to_string_lossy().into_owned();
    list.iter()
        .map(|fs| {
            let block = u64::from(fs.f_bsize);
            Mount {
                on: text(&fs.f_mntonname),
                from: text(&fs.f_mntfromname),
                free: fs.f_bavail.saturating_mul(block),
                total: fs.f_blocks.saturating_mul(block),
                local: fs.f_flags & libc::MNT_LOCAL as u32 != 0,
            }
        })
        .collect()
}

/// path가 네트워크 디스크 위에 있는가 (가장 가까운 붙은 자리로 본다). 모르면 false
pub fn on_network(path: &str) -> bool {
    let p = std::path::Path::new(path);
    mounts().into_iter().filter(|m| p.starts_with(&m.on)).max_by_key(|m| m.on.len()).is_some_and(|m| !m.local)
}

/// 디스크를 꺼낸다 (Finder의 꺼내기와 같다: 같은 장치의 다른 부분도 함께 떼고 장치를 꺼낸다). 쓰는 파일이 있으면 맥이 거절한다.
/// 실패하면 맥이 준 까닭
pub fn eject(path: &str) -> Result<(), String> {
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), true);
        NSWorkspace::sharedWorkspace().unmountAndEjectDeviceAtURL_error(&url).map_err(|e| e.localizedDescription().to_string())
    })
}

/// NetFS의 연결 함수 (처음 쓸 때 한 번 연다. 열지 못했으면 None)
fn mount_fn() -> Option<MountSync> {
    static LOADED: OnceLock<Option<MountSync>> = OnceLock::new();
    *LOADED.get_or_init(|| {
        // SAFETY: NUL로 끝나는 고정 경로다. 받은 핸들은 닫지 않는다 (앱이 끝날 때까지 쓴다)
        let h = unsafe { libc::dlopen(NETFS.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
        if h.is_null() {
            sakadu::diag::log(|| "netfs: the framework did not load".to_string());
            return None;
        }
        // SAFETY: 이름은 NUL로 끝난다. 찾은 함수의 C 선언은 MountSync와 같다 (NetFS.h)
        let p = unsafe { libc::dlsym(h, c"NetFSMountURLSync".as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, MountSync>(p) })
    })
}

/// 네트워크 공유(url: sakadu::util::share_url로 다듬은 주소)에 연결하고 붙은 자리를 돌려준다 (뒷줄에서 부른다: 로그인 창의 답이나
/// 꺼진 서버를 기다리는 동안 이 줄이 멈춘다). 이미 연결돼 있으면 그 자리를 찾아 돌려준다. 하지 못하면 알림에 쓸 까닭
pub fn mount(url: &str) -> Result<String, String> {
    let Some(f) = mount_fn() else { return Err("이 Mac에서 네트워크 연결 기능을 찾지 못했어요".to_string()) };
    let (status, first) = objc2::rc::autoreleasepool(|_| {
        let Some(u) = NSURL::URLWithString(&NSString::from_str(url)) else { return (libc::EINVAL, None) };
        let mut points: *const c_void = std::ptr::null();
        let none = std::ptr::null();
        // SAFETY: NSURL은 CFURL과 같은 것이다 (toll-free). 이름·암호·선택지는 비워 맥의 기본(필요하면 로그인 창)을 쓴다.
        // 돌려받은 배열은 만든 것을 받은 것이라(+1) 아래에서 Retained로 받아 놓는다
        let status = unsafe { f(Retained::as_ptr(&u).cast(), none, none, none, std::ptr::null_mut(), std::ptr::null_mut(), &mut points) };
        // SAFETY: NetFS가 넘긴 CFArray<CFString> (NSArray<NSString>과 같은 것). null이면 None
        let list = unsafe { Retained::<NSArray<NSString>>::from_raw(points as *mut NSArray<NSString>) };
        (status, list.and_then(|l| l.firstObject()).map(|s| s.to_string()))
    });
    match (status, first) {
        (0, Some(at)) => Ok(at),
        (0 | libc::EEXIST, _) => find_share(url).ok_or_else(|| "연결했지만 붙은 자리를 찾지 못했어요".to_string()),
        (code, _) => {
            sakadu::diag::log(|| format!("netfs: mount failed with {code}"));
            Err(mount_reason(code))
        }
    }
}

/// 연결하지 못한 까닭 (알림에 쓴다). 번호의 뜻은 NetFS·errno의 것
fn mount_reason(code: c_int) -> String {
    match code {
        USER_CANCELED | libc::ECANCELED => "연결을 그만뒀어요".to_string(),
        libc::EAUTH | libc::EACCES | libc::EPERM => "이름이나 암호가 맞지 않거나 열 권한이 없어요".to_string(),
        libc::ETIMEDOUT | libc::EHOSTUNREACH | libc::EHOSTDOWN | libc::ENETUNREACH | libc::ECONNREFUSED | libc::ENOENT => "서버에 닿지 않아요 · 주소와 네트워크를 확인해 주세요".to_string(),
        c if NO_SHARES.contains(&c) => "서버에 열 수 있는 공유 폴더가 없어요".to_string(),
        libc::EINVAL => "주소를 읽지 못했어요".to_string(),
        c => format!("맥이 연결을 거절했어요 (오류 {c})"),
    }
}

/// 이미 붙어 있는 그 공유의 자리: 붙은 디스크 가운데 출처의 서버 이름과 공유 경로가 주소의 것과 같은 것
fn find_share(url: &str) -> Option<String> {
    let (server, share) = share_parts(url)?;
    mounts().into_iter().find(|m| share_matches(&m.from, &server, &share)).map(|m| m.on)
}

/// 주소("smb://이름@서버:포트/공유/")의 서버 이름과 공유 경로 (host_of, share_path). 공유 폴더가 없으면 None
fn share_parts(url: &str) -> Option<(String, String)> {
    let rest = url.split_once("://")?.1;
    let (server, path) = rest.split_once('/')?;
    Some((host_of(server)?, share_path(path)?))
}

/// 붙은 디스크의 출처(from)가 그 서버의 그 공유인가. 서버 이름과 공유 경로를 통째로 견준다
/// (nas.local이 othernas.local이나 nas.local.example에 맞지 않게, 사진이 사진 폴더에 맞지 않게)
fn share_matches(from: &str, server: &str, share: &str) -> bool {
    source_parts(from).is_some_and(|(h, p)| h == server && p == share)
}

/// 붙은 디스크 출처의 서버 이름과 공유 경로. smbfs·afpfs는 "//[이름[;도메인][:암호]@]서버[:포트]/공유",
/// NFS는 "서버:/내보낸/경로", ftp는 주소 꼴("ftp://서버/경로")이다. 디스크 장치("/dev/disk4s1") 따위는 None
fn source_parts(from: &str) -> Option<(String, String)> {
    let (server, path) = match from.strip_prefix("//").or_else(|| from.split_once("://").map(|(_, rest)| rest)) {
        Some(rest) => rest.split_once('/')?,
        // NFS: 서버 뒤 ':' 다음이 '/'로 시작하는 경로 (IPv6 서버는 [ ] 안이라 그 뒤의 ':'에서 자른다)
        None => {
            let at = if from.starts_with('[') { from.find("]:")? + 1 } else { from.find(':')? };
            let (server, path) = (&from[..at], &from[at + 1..]);
            if server.contains('/') || !path.starts_with('/') {
                return None;
            }
            (server, path)
        }
    };
    Some((host_of(server)?, share_path(path)?))
}

/// 서버 칸("이름;도메인:암호@서버:포트", "[IPv6]:포트")의 서버 이름: 소문자로, %를 푼 것. 비었으면 None
fn host_of(server: &str) -> Option<String> {
    let s = server.rsplit('@').next()?;
    let host = if s.starts_with('[') { s.find(']').map_or(s, |i| &s[..=i]) } else { s.split(':').next()? };
    (!host.is_empty()).then(|| decode(host).to_lowercase())
}

/// 공유 경로: 앞뒤 '/'를 떼고 %를 풀어 소문자로 (SMB 공유 이름은 대소문자를 가리지 않는다). 비었으면 None
fn share_path(path: &str) -> Option<String> {
    let p = path.trim_matches('/');
    (!p.is_empty()).then(|| decode(p).to_lowercase())
}

/// 주소의 %XX를 푼다 (UTF-8이 아니면 그대로 둔다)
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (b[i], b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 붙은 디스크 목록: 시동 디스크가 있고, 이 맥에 붙은 디스크로 보인다
    #[test]
    fn mounts_list_the_boot_disk_as_local() {
        let all = mounts();
        assert!(all.iter().any(|m| m.on == "/" && m.local && m.total > 0), "{:?}", all.iter().map(|m| &m.on).collect::<Vec<_>>());
        assert!(!on_network("/"));
        assert!(!on_network(&std::env::temp_dir().to_string_lossy()));
    }

    /// 이미 붙어 있는 공유를 찾을 때: 서버 이름과 공유 경로(%를 풀고 대소문자 없이)를 통째로 견준다.
    /// 출처의 이름·도메인·암호·포트는 보지 않고, 다른 서버 이름 안에 든 이름이나 다른 공유 안의 폴더는 맞지 않는다
    #[test]
    fn a_mounted_share_is_found_by_server_and_share_name() {
        let url = sakadu::util::share_url("smb://hong@NAS.local/사진 폴더/").unwrap();
        let (server, share) = share_parts(&url).unwrap();
        assert_eq!((server.as_str(), share.as_str()), ("nas.local", "사진 폴더"));
        let matches = |from: &str| share_matches(from, &server, &share);
        assert!(matches("//hong@nas.local/%EC%82%AC%EC%A7%84%20%ED%8F%B4%EB%8D%94"));
        assert!(matches("//GUEST:@NAS.local/사진 폴더"));
        assert!(matches("//WORK;hong@nas.local:445/사진 폴더/"), "domain and port");
        assert!(matches("//nas.local/사진 폴더"), "no user");
        assert!(!matches("//hong@nas.local/사진"), "another share");
        assert!(!matches("//hong@nas.local/사진 폴더/하위"), "a folder inside");
        assert!(!matches("//hong@other.local/사진 폴더"), "another server");
        assert!(!matches("//user@othernas.local/사진 폴더"), "a server whose name ends with it");
        assert!(!matches("//user@nas.local.example/사진 폴더"), "a server whose name starts with it");
        assert!(!matches("//nas.local@other.local/사진 폴더"), "the server is after the last @");
        assert!(!matches("/dev/disk4s1") && !matches("map auto_home"), "not a share");
        assert_eq!(share_parts("smb://nas.local"), None, "no share in the address");
        // NFS: "서버:/경로"의 경로 전체, IPv6 서버, ftp의 주소 꼴
        let nfs = share_parts("nfs://nas:2049/volume1/data").unwrap();
        assert_eq!(nfs, ("nas".to_string(), "volume1/data".to_string()));
        assert!(share_matches("NAS:/volume1/data", &nfs.0, &nfs.1));
        assert!(!share_matches("othernas:/volume1/data", &nfs.0, &nfs.1));
        assert!(!share_matches("nas:/volume2/data", &nfs.0, &nfs.1));
        let v6 = share_parts("smb://[FE80::1]:445/공유").unwrap();
        assert_eq!(v6.0, "[fe80::1]");
        assert!(share_matches("//hong@[fe80::1]/공유", &v6.0, &v6.1));
        assert!(share_matches("[fe80::1]:/공유", &v6.0, &v6.1));
        assert!(!share_matches("//hong@[fe80::2]/공유", &v6.0, &v6.1));
        let ftp = share_parts("ftp://ftp.example.com/pub/").unwrap();
        assert!(share_matches("ftp://ftp.example.com/pub", &ftp.0, &ftp.1));
        assert!(!share_matches("ftp://myftp.example.com/pub", &ftp.0, &ftp.1));
        assert_eq!(decode("a%20b%ZZ%"), "a b%ZZ%");
    }

    /// 연결 함수: 맥의 NetFS에서 찾는다 (연결은 하지 않는다)
    #[test]
    fn netfs_is_found() {
        assert!(mount_fn().is_some());
    }
}
