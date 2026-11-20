//! 폴더 훑기: 폴더 하나를 읽어 색인에 넣을 항목으로 바꾼다. 폴더를 읽는 줄 여럿이 함께 부른다.
//! 링크는 따라가지 않고, 패키지는 파일 하나로, 홈의 라이브러리·휴지통과 다른 디스크(마운트 지점)는 안을 훑지 않는다.
//! 폴더를 읽어도 iCloud에만 있는 파일을 내려받게 하지 않는다 (prepare_thread)

use sakadu::nametree::{Item, Kind, classify};
use std::collections::HashSet;
use std::ffi::{CStr, c_int};

/// 폴더를 읽는 규칙
pub struct Rules {
    /// 홈 폴더: 그 바로 아래의 라이브러리·휴지통은 훑지 않는다
    pub home: String,
    /// 패키지(.app 등)를 Finder처럼 파일 하나로 본다. 측정할 때만 꺼서 안까지 훑는다
    pub packages_are_files: bool,
    /// 다른 디스크가 붙은 자리: 그 안으로 넘어가지 않는다 ('/' 아래의 /System/Volumes/Data처럼 같은 내용이 두 번 보이는 곳도)
    pub mounts: HashSet<String>,
}

/// 폴더 하나를 읽은 결과
pub enum Listed {
    Items(Vec<Item>),
    /// 권한이 없다 (맥이 접근을 막았거나 사용자가 허용하지 않았다)
    Denied,
    /// 이 맥에 내려받지 않은 폴더라 읽지 않았다 (읽으려면 내려받아야 한다)
    NotLocal,
    /// 그사이 없어졌다 (그 위 폴더의 알림이 색인에서 뺀다)
    Gone,
    Failed,
}

/// 폴더 dir을 읽는다
pub fn list(dir: &str, rules: &Rules) -> Listed {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => return failure(e),
    };
    let mut items = Vec::new();
    for entry in entries.flatten() {
        // 종류는 폴더를 읽을 때 함께 온다 (파일마다 따로 묻지 않는다)
        let Ok(t) = entry.file_type() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let (is_dir, is_link) = (t.is_dir(), t.is_symlink());
        let mut kind = classify(&rules.home, dir, &name, is_dir, is_link);
        if kind == Kind::File && is_dir && !is_link && !rules.packages_are_files {
            kind = Kind::Dir;
        }
        if kind == Kind::Dir && rules.mounts.contains(&sakadu::util::join(dir, &name)) {
            kind = Kind::Closed;
        }
        items.push(Item { name, kind });
    }
    Listed::Items(items)
}

fn failure(e: std::io::Error) -> Listed {
    match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => Listed::Gone,
        std::io::ErrorKind::PermissionDenied => Listed::Denied,
        _ if e.raw_os_error() == Some(libc::EDEADLK) => Listed::NotLocal,
        _ => {
            sakadu::diag::log(|| format!("search: a folder could not be read: {e}"));
            Listed::Failed
        }
    }
}

/// 지금 붙어 있는 디스크의 자리들 ('/'는 빼고). 꺼진 네트워크 디스크를 기다리지 않는다 (MNT_NOWAIT)
pub fn mount_points() -> HashSet<String> {
    // SAFETY: 받을 곳 없이 개수만 묻는다
    let n = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if n <= 0 {
        return HashSet::new();
    }
    // 그사이 붙은 디스크를 위해 넉넉히
    let mut list: Vec<libc::statfs> = Vec::with_capacity(n as usize + 16);
    let bytes = (list.capacity() * std::mem::size_of::<libc::statfs>()) as c_int;
    // SAFETY: list의 칸만큼만 채운다
    let got = unsafe { libc::getfsstat(list.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
    if got <= 0 {
        return HashSet::new();
    }
    // SAFETY: getfsstat이 앞의 got칸을 채웠다
    unsafe { list.set_len(got as usize) };
    list.iter()
        // SAFETY: f_mntonname은 NUL로 끝나는 C 문자열이다
        .map(|fs| unsafe { CStr::from_ptr(fs.f_mntonname.as_ptr()) }.to_string_lossy().into_owned())
        .filter(|p| p != "/")
        .collect()
}

/// 폴더를 읽는 줄의 준비: 낮은 우선순위(사용자가 하는 일을 늦추지 않게)로, iCloud에만 있는 항목을 내려받지 않게 한다
pub fn prepare_thread() {
    // SAFETY: 이 줄 자신의 설정만 바꾼다
    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0);
        setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, IOPOL_MATERIALIZE_DATALESS_FILES_OFF);
    }
}

/// 이 줄의 '내려받기' 설정 (시험에서 prepare_thread가 먹었는지 본다). 1이 꺼짐
#[cfg(test)]
pub fn dataless_policy() -> c_int {
    // SAFETY: 이 줄의 설정을 읽기만 한다
    unsafe { getiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD) }
}

/// sys/qos.h
const QOS_CLASS_UTILITY: u32 = 0x11;
/// sys/resource.h
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: c_int = 3;
const IOPOL_SCOPE_THREAD: c_int = 1;
pub const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: c_int = 1;

unsafe extern "C" {
    fn pthread_set_qos_class_self_np(class: u32, relative_priority: c_int) -> c_int;
    fn setiopolicy_np(iotype: c_int, scope: c_int, policy: c_int) -> c_int;
    #[cfg(test)]
    fn getiopolicy_np(iotype: c_int, scope: c_int) -> c_int;
}
