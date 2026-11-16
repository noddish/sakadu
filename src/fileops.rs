//! 맥 파일 작업의 바탕: 화면과 상관없는 판단(이름 규칙, 번호 붙인 이름, 이름에서 확장자 앞부분, 실패한 까닭을 알리는 말,
//! 되돌리기 기록)과 파일을 실제로 건드리는 가장 작은 호출들(폴더 만들기, 덮어쓰지 않는 이름 바꾸기, 파일 번호 읽기).
//! 맥 앱(src/mac)이 쓴다. Windows 앱의 파일 작업은 셸로 따로 한다 (app/file_ops.rs).
//! 원칙 두 가지: 덮어쓰지 않는다 (같은 이름이 있으면 실패한다). 파일을 없애는 호출은 여기에 두지 않는다

use crate::util;
use std::io;

/// 새로 만든 폴더의 이름. 같은 이름이 있으면 번호를 붙인다 ("새 폴더 2")
pub const NEW_FOLDER: &str = "새 폴더";

/// 맥에서 파일·폴더 이름으로 쓸 수 없으면 그 까닭.
/// '/'는 경로 구분자이고 ':'는 Finder가 '/'로 보여 주는 글자라 둘 다 받지 않는다.
/// 점으로 시작하는 이름은 숨김 항목이 되므로, 숨김 항목 보기를 켰을 때만 받는다 (끈 채로 바꾸면 그 항목이 목록에서 사라진다)
pub fn mac_name_problem(name: &str, show_hidden: bool) -> Option<String> {
    if name.is_empty() {
        return Some("이름이 비어 있어요".to_string());
    }
    if name == "." || name == ".." {
        return Some(format!("‘{name}’은 이름으로 쓸 수 없어요"));
    }
    if let Some(c) = name.chars().find(|c| matches!(c, '/' | ':') || (*c as u32) < 0x20) {
        return Some(if (c as u32) < 0x20 { "이름에 쓸 수 없는 제어 문자가 들어 있어요".to_string() } else { format!("이름에는 {c} 문자를 쓸 수 없어요") });
    }
    // 맥(APFS)의 이름 길이 한도는 UTF-8로 255바이트
    if name.len() > 255 {
        return Some("이름이 너무 길어요".to_string());
    }
    if name.starts_with('.') && !show_hidden {
        return Some("점으로 시작하면 숨김 항목이 돼요. 숨김 항목 보기를 켜고 바꿔 주세요".to_string());
    }
    None
}

/// n번째로 시도할 이름: 처음은 그대로, 그다음부터 번호를 붙인다 ("새 폴더", "새 폴더 2", "새 폴더 3")
pub fn numbered(base: &str, n: usize) -> String {
    if n <= 1 { base.to_string() } else { format!("{base} {n}") }
}

/// 같은 이름이 있을 때 n번째로 시도할 이름: 처음은 그대로, 그다음부터 번호를 붙인다.
/// 파일은 확장자 앞에 붙이고("보고서 2.docx"), 폴더와 확장자 없는 이름·점으로 시작하는 이름은 끝에 붙인다
pub fn numbered_name(name: &str, n: usize, is_dir: bool) -> String {
    if n <= 1 {
        return name.to_string();
    }
    match name.rfind('.') {
        Some(i) if i > 0 && !is_dir => format!("{} {n}{}", &name[..i], &name[i..]),
        _ => format!("{name} {n}"),
    }
}

/// 이름을 고칠 때 미리 골라 둘 앞부분의 길이 (UTF-16 단위: 맥과 Windows 입력 상자가 쓰는 단위).
/// 파일은 확장자 앞까지, 폴더와 확장자 없는 이름·점으로 시작하는 이름은 전체
pub fn stem_units(name: &str, is_dir: bool) -> usize {
    let stem = match name.rfind('.') {
        Some(i) if i > 0 && !is_dir => &name[..i],
        _ => name,
    };
    stem.encode_utf16().count()
}

/// 파일 작업이 실패한 까닭을 알림에 쓸 말로 (모르는 오류는 시스템이 준 글 그대로)
pub fn reason(e: &io::Error) -> String {
    use io::ErrorKind as K;
    match e.kind() {
        K::PermissionDenied => "권한이 없어요".to_string(),
        K::NotFound => "항목을 찾을 수 없어요".to_string(),
        K::AlreadyExists => "같은 이름이 이미 있어요".to_string(),
        K::ReadOnlyFilesystem => "읽기 전용 디스크예요".to_string(),
        K::StorageFull => "디스크에 남은 자리가 없어요".to_string(),
        K::InvalidFilename => "쓸 수 없는 이름이에요".to_string(),
        _ => e.to_string(),
    }
}

/// parent 안에 base 이름의 폴더를 만든다. 같은 이름이 있으면(파일·폴더·링크 무엇이든) 번호를 올려 가며 다시 만든다.
/// 있는지 먼저 묻지 않고 만들기가 실패하는 것으로 가리므로, 동시에 여럿이 만들어도 이름이 겹치지 않는다. 만든 이름을 돌려준다
pub fn make_dir_numbered(parent: &str, base: &str) -> io::Result<String> {
    for n in 1..10_000 {
        let name = numbered(base, n);
        match std::fs::create_dir(util::join(parent, &name)) {
            Ok(()) => return Ok(name),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists))
}

/// 휴지통으로 보낸 항목 하나
#[derive(Debug, Clone, PartialEq)]
pub struct Trashed {
    /// 원래 있던 폴더 안에서의 이름
    pub name: String,
    pub file_id: u64,
    /// 휴지통 안에서의 경로. 모르면 비어 있다 (그 항목은 되돌릴 수 없다)
    pub trash_path: String,
}

/// 되돌릴 수 있는 파일 작업 하나 (맥 앱의 것. Windows 앱은 undo.rs를 쓴다).
/// 되돌릴 때는 그 항목이 그때의 그 항목인지(파일 번호) 다시 보고, 덮어쓰지 않으며, 만든 것을 없앨 때도 휴지통으로 보낸다
#[derive(Debug, Clone, PartialEq)]
pub enum Undo {
    /// dir 안에서 from을 to로 이름을 바꿨다
    Rename { dir: String, from: String, to: String, file_id: u64 },
    /// dir 안에 name 폴더를 만들었다
    NewFolder { dir: String, name: String, file_id: u64 },
    /// dir 안의 항목들을 휴지통으로 보냈다
    Trash { dir: String, items: Vec<Trashed> },
    /// dir 안에 복사본들(이름, 파일 번호)을 만들었다. 되돌리면 그 복사본들을 휴지통으로 보낸다
    Copy { dir: String, items: Vec<(String, u64)> },
    /// 항목들을 폴더 to로 옮겼다: (원래 전체 경로, to 안에서의 지금 이름, 파일 번호). 되돌리면 원래 자리로 옮긴다
    Move { to: String, items: Vec<(String, String, u64)> },
}

impl Undo {
    /// 무엇을 되돌리는지 (알림에 쓴다)
    pub fn label(&self) -> String {
        match self {
            Undo::Rename { from, .. } => format!("이름을 ‘{from}’(으)로 되돌렸어요"),
            Undo::NewFolder { name, .. } => format!("만든 폴더 ‘{name}’을(를) 휴지통으로 보냈어요"),
            Undo::Trash { items, .. } => format!("{}개를 휴지통에서 되돌려 놓았어요", items.len()),
            Undo::Copy { items, .. } => format!("복사본 {}개를 휴지통으로 보냈어요", items.len()),
            Undo::Move { items, .. } => format!("{}개를 원래 자리로 되돌려 놓았어요", items.len()),
        }
    }
}

/// 되돌리기 기록에 남겨 두는 작업 수 (넘으면 가장 오래된 것부터 잊는다)
pub const UNDO_MAX: usize = 100;

/// 되돌리기 기록: 가장 나중에 한 작업부터 꺼낸다
#[derive(Debug, Default)]
pub struct UndoStack {
    list: std::collections::VecDeque<Undo>,
}

impl UndoStack {
    pub fn push(&mut self, undo: Undo) {
        self.list.push_back(undo);
        if self.list.len() > UNDO_MAX {
            self.list.pop_front();
        }
    }

    pub fn pop(&mut self) -> Option<Undo> {
        self.list.pop_back()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// 폴더가 비었는지. Finder가 폴더마다 남기는 .DS_Store만 있으면 빈 것으로 본다.
/// 만든 폴더를 되돌릴 때 쓴다: 그사이 무엇이 들어왔으면 함께 휴지통으로 보내지 않는다
pub fn dir_is_empty(path: &str) -> io::Result<bool> {
    for item in std::fs::read_dir(path)? {
        if item?.file_name() != ".DS_Store" {
            return Ok(false);
        }
    }
    Ok(true)
}

/// 그 항목이 다른 볼륨이 붙은 자리(마운트 지점)인지: 놓여 있는 폴더와 볼륨이 다르다.
/// 디스크 자체를 휴지통으로 보내려는 것을 막는다. 알 수 없으면 false.
/// 항목은 그 자체를 보고(링크면 링크), 놓여 있는 폴더는 링크를 따라가서 본다: 다른 볼륨을 가리키는 링크로 연 폴더에서
/// 폴더 쪽을 링크 자체로 보면, 그 안의 보통 파일이 모두 마운트 지점으로 보인다
#[cfg(unix)]
pub fn is_mount_point(path: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Some(parent) = util::parent(path) else { return true };
    match (std::fs::symlink_metadata(path), std::fs::metadata(&parent)) {
        (Ok(item), Ok(folder)) => item.dev() != folder.dev(),
        _ => false,
    }
}

/// 그 경로에 있는 항목 자체의 파일 번호 (링크면 가리키는 것이 아니라 링크의 번호).
/// 목록을 읽을 때 적어 둔 번호(Entry::file_id)와 비교해, 그 사이 다른 항목으로 바뀌지 않았는지 본다
#[cfg(unix)]
pub fn inode(path: &str) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::symlink_metadata(path)?.ino())
}

/// 두 경로가 같은 폴더 안의 같은 항목을 가리키는가: 대소문자를 가리지 않는 볼륨에서 대소문자만 다른 두 이름이 그렇다.
/// 같은 파일(같은 볼륨의 같은 파일 번호)이어도 두 이름이 폴더 목록에 따로 나오면 서로 다른 항목이다 (한 파일에 붙은 두 이름,
/// 하드 링크). 그런 둘 사이의 이름 바꾸기는 아무것도 바꾸지 않으면서 성공했다고 하므로, 같은 항목으로 보지 않는다
#[cfg(unix)]
fn same_entry(a: &str, b: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let same_file = matches!((std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)), (Ok(x), Ok(y)) if x.dev() == y.dev() && x.ino() == y.ino());
    same_file && !listed_apart(a, b)
}

/// 두 경로의 이름이 저마다 폴더 목록에 글자 그대로 나오는가. 서로 다른 폴더에 있거나 목록을 읽지 못하면 그렇다고 본다
/// (모를 때는 덮어쓰지 않는 쪽으로)
#[cfg(unix)]
fn listed_apart(a: &str, b: &str) -> bool {
    let (Some(dir), Some(other)) = (util::parent(a), util::parent(b)) else { return true };
    if dir != other {
        return true;
    }
    let Ok(items) = std::fs::read_dir(&dir) else { return true };
    let (name_a, name_b) = (util::file_name(a), util::file_name(b));
    let (mut has_a, mut has_b) = (false, false);
    for item in items.flatten() {
        let name = item.file_name();
        has_a |= name == name_a;
        has_b |= name == name_b;
    }
    has_a && has_b
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    /// rename(2)에 옵션을 더한 맥의 호출 (macOS 10.12부터)
    fn renamex_np(from: *const std::ffi::c_char, to: *const std::ffi::c_char, flags: std::ffi::c_uint) -> std::ffi::c_int;
}

/// renamex_np의 옵션: 받는 쪽 이름이 이미 있으면 EEXIST로 실패한다 (sys/stdio.h)
#[cfg(target_os = "macos")]
const RENAME_EXCL: std::ffi::c_uint = 0x0000_0004;
/// 그 볼륨이 이 옵션을 모른다 (sys/errno.h)
#[cfg(target_os = "macos")]
const ENOTSUP: i32 = 45;

/// from을 to로 이름을 바꾼다 (같은 볼륨 안에서 옮기는 것도 된다). to에 무엇이든 이미 있으면 덮어쓰지 않고 실패한다.
/// 링크는 링크 자체의 이름이 바뀐다. 있는지 확인과 바꾸기를 커널이 한 번에 하므로 그 사이에 끼어들 틈이 없다
#[cfg(target_os = "macos")]
pub fn rename_excl(from: &str, to: &str) -> io::Result<()> {
    let c = |s: &str| std::ffi::CString::new(s).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput));
    let (f, t) = (c(from)?, c(to)?);
    // SAFETY: 둘 다 이 함수가 끝날 때까지 살아 있는, NUL로 끝나는 C 문자열이다
    if unsafe { renamex_np(f.as_ptr(), t.as_ptr(), RENAME_EXCL) } == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    if err.kind() == io::ErrorKind::AlreadyExists && same_entry(from, to) {
        // '이미 있다'는 그 이름이 바로 자기 자신이다: 대소문자를 가리지 않는 볼륨에서 대소문자만 바꾸는 경우.
        // 같은 항목의 이름 바꾸기는 아무것도 덮어쓰지 않는다 (한 파일에 붙은 다른 이름은 same_entry가 걸러 낸다)
        return std::fs::rename(from, to);
    }
    if err.raw_os_error() == Some(ENOTSUP) {
        return rename_checked(from, to);
    }
    Err(err)
}

/// 덮어쓰지 않는 이름 바꾸기를 모르는 볼륨(일부 네트워크 디스크)에서: to가 없는 것을 확인한 뒤 바꾼다.
/// 확인과 바꾸기가 따로라 그 사이에 다른 프로그램이 같은 이름을 만들면 막지 못한다 (그런 볼륨에서는 더 나은 방법이 없다)
#[cfg(unix)]
pub fn rename_checked(from: &str, to: &str) -> io::Result<()> {
    match std::fs::symlink_metadata(to) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => std::fs::rename(from, to),
        Ok(_) if same_entry(from, to) => std::fs::rename(from, to),
        Ok(_) => Err(io::Error::from(io::ErrorKind::AlreadyExists)),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 시험마다 따로 쓰는 임시 폴더 (끝에서 지운다)
    fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("sakadu-fileops-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        util::normalize(&std::fs::canonicalize(&dir).unwrap().to_string_lossy().replace("\\\\?\\", ""))
    }

    #[cfg(unix)]
    fn names(dir: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    /// 이 파일의 시험이 아닌 부분에는 파일을 없애는 호출이 없다 (지우기는 휴지통뿐이고 맥 앱 쪽에 있다)
    #[test]
    fn engine_has_no_delete_calls() {
        let code = include_str!("fileops.rs").split("#[cfg(test)]").next().unwrap();
        for banned in ["remove_file", "remove_dir", "unlink", "rmdir", "removeItem"] {
            assert!(!code.contains(banned), "{banned}");
        }
    }

    #[test]
    fn mac_names_block_separators_and_hidden_surprises() {
        for bad in ["", ".", "..", "a/b", "보고서:최종.xlsx", "줄\n바꿈"] {
            assert!(mac_name_problem(bad, true).is_some(), "{bad:?}");
        }
        // Windows에서는 안 되지만 맥에서는 되는 이름들
        for ok in ["보고서 (최종).xlsx", "a?b*c.txt", "CON", "끝에 점.", " 앞 공백", "a\\b"] {
            assert!(mac_name_problem(ok, false).is_none(), "{ok:?}");
        }
        // 점으로 시작하는 이름은 숨김 항목 보기를 켰을 때만
        assert!(mac_name_problem(".env", false).is_some());
        assert!(mac_name_problem(".env", true).is_none());
        // 255바이트까지 (한글은 한 글자에 3바이트)
        assert!(mac_name_problem(&"가".repeat(85), false).is_none());
        assert!(mac_name_problem(&"가".repeat(86), false).is_some());
    }

    #[test]
    fn numbered_names_and_stems() {
        assert_eq!([1, 2, 3].map(|n| numbered(NEW_FOLDER, n)), ["새 폴더", "새 폴더 2", "새 폴더 3"]);
        // 복사본의 이름: 파일은 확장자 앞에, 폴더와 확장자 없는 이름은 끝에
        assert_eq!([1, 2, 10].map(|n| numbered_name("보고서.docx", n, false)), ["보고서.docx", "보고서 2.docx", "보고서 10.docx"]);
        assert_eq!(numbered_name("archive.tar.gz", 2, false), "archive.tar 2.gz");
        assert_eq!(numbered_name("README", 2, false), "README 2");
        assert_eq!(numbered_name(".gitignore", 2, false), ".gitignore 2");
        assert_eq!(numbered_name("자료.d", 2, true), "자료.d 2");
        assert!(Undo::Copy { dir: "/t".into(), items: vec![("a".into(), 1)] }.label().contains("복사본 1개"));
        assert!(Undo::Move { to: "/t".into(), items: vec![("/s/a".into(), "a".into(), 1)] }.label().contains("원래 자리"));
        assert_eq!(stem_units("보고서.xlsx", false), 3);
        assert_eq!(stem_units("archive.tar.gz", false), 11);
        assert_eq!(stem_units("README", false), 6);
        assert_eq!(stem_units(".gitignore", false), 10, "a leading dot is not an extension");
        assert_eq!(stem_units("자료.d", true), 4, "folders select the whole name");
        // 두 칸짜리 글자(이모지)는 UTF-16으로 2
        assert_eq!(stem_units("😀.png", false), 2);
    }

    #[test]
    fn reasons_read_as_korean() {
        assert_eq!(reason(&io::Error::from(io::ErrorKind::PermissionDenied)), "권한이 없어요");
        assert_eq!(reason(&io::Error::from(io::ErrorKind::AlreadyExists)), "같은 이름이 이미 있어요");
        assert!(!reason(&io::Error::other("이상한 오류")).is_empty());
    }

    /// 되돌리기 기록: 나중 것부터 나오고, 100개를 넘으면 가장 오래된 것을 잊는다
    #[test]
    fn undo_stack_is_last_in_first_out_and_bounded() {
        let made = |n: usize| Undo::NewFolder { dir: "/t".into(), name: format!("폴더 {n}"), file_id: n as u64 };
        let mut s = UndoStack::default();
        assert!(s.is_empty() && s.pop().is_none());
        for n in 0..UNDO_MAX + 5 {
            s.push(made(n));
        }
        assert_eq!(s.len(), UNDO_MAX);
        assert_eq!(s.pop(), Some(made(UNDO_MAX + 4)));
        let mut last = None;
        while let Some(u) = s.pop() {
            last = Some(u);
        }
        assert_eq!(last, Some(made(5)), "the five oldest were forgotten");
        // 알림 글
        let trashed = |name: &str| Trashed { name: name.into(), file_id: 1, trash_path: String::new() };
        assert!(Undo::Rename { dir: "/t".into(), from: "a.txt".into(), to: "b.txt".into(), file_id: 1 }.label().contains("a.txt"));
        assert!(Undo::Trash { dir: "/t".into(), items: vec![trashed("a"), trashed("b")] }.label().starts_with("2개"));
    }

    /// 빈 폴더: 아무것도 없거나 .DS_Store뿐일 때. 없는 폴더는 오류
    #[test]
    fn a_folder_with_only_ds_store_counts_as_empty() {
        let dir = temp("empty");
        assert!(dir_is_empty(&dir).unwrap());
        std::fs::write(util::join(&dir, ".DS_Store"), b"x").unwrap();
        assert!(dir_is_empty(&dir).unwrap());
        std::fs::write(util::join(&dir, "메모.txt"), b"x").unwrap();
        assert!(!dir_is_empty(&dir).unwrap());
        assert!(dir_is_empty(&util::join(&dir, "없는 곳")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 마운트 지점: 보통 폴더와 파일은 아니고, 볼륨의 맨 위는 맞다
    #[cfg(unix)]
    #[test]
    fn ordinary_items_are_not_mount_points() {
        let dir = temp("mount");
        std::fs::write(util::join(&dir, "a.txt"), b"a").unwrap();
        assert!(!is_mount_point(&dir) && !is_mount_point(&util::join(&dir, "a.txt")));
        assert!(is_mount_point("/"));
        // 다른 볼륨(/dev)을 가리키는 링크로 연 폴더: 그 안의 항목은 마운트 지점이 아니고, /dev 자체는 맞다.
        // 링크 자체는 놓여 있는 폴더와 같은 볼륨의 항목이다 (휴지통으로 보내면 링크만 간다)
        if std::fs::metadata("/dev/null").is_ok() && is_mount_point("/dev") {
            std::os::unix::fs::symlink("/dev", util::join(&dir, "다른 볼륨")).unwrap();
            assert!(!is_mount_point(&util::join(&util::join(&dir, "다른 볼륨"), "null")));
            assert!(!is_mount_point(&util::join(&dir, "다른 볼륨")));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 새 폴더: 같은 이름이 있으면 번호를 올리고, 파일이 그 이름을 쓰고 있어도 건드리지 않는다. 동시에 만들어도 겹치지 않는다
    #[test]
    fn new_folders_never_collide() {
        let dir = temp("mkdir");
        assert_eq!(make_dir_numbered(&dir, NEW_FOLDER).unwrap(), "새 폴더");
        std::fs::write(util::join(&dir, "새 폴더 2"), b"file").unwrap();
        assert_eq!(make_dir_numbered(&dir, NEW_FOLDER).unwrap(), "새 폴더 3");
        assert_eq!(std::fs::read(util::join(&dir, "새 폴더 2")).unwrap(), b"file", "the file with that name is untouched");
        let made: Vec<String> = std::thread::scope(|s| {
            let jobs: Vec<_> = (0..2).map(|_| s.spawn(|| (0..10).map(|_| make_dir_numbered(&dir, "동시").unwrap()).collect::<Vec<_>>())).collect();
            jobs.into_iter().flat_map(|j| j.join().unwrap()).collect()
        });
        let unique: std::collections::HashSet<&String> = made.iter().collect();
        assert_eq!((made.len(), unique.len()), (20, 20));
        assert!(made.iter().all(|n| std::fs::metadata(util::join(&dir, n)).unwrap().is_dir()));
        assert!(make_dir_numbered(&util::join(&dir, "없는 곳"), NEW_FOLDER).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 덮어쓰지 않는 이름 바꾸기: 파일·폴더·끊어진 링크 어느 것 위로도 가지 않고, 양쪽 모두 그대로 남는다
    #[cfg(target_os = "macos")]
    #[test]
    fn rename_never_replaces_what_is_there() {
        let dir = temp("excl");
        let p = |n: &str| util::join(&dir, n);
        std::fs::write(p("a.txt"), b"a").unwrap();
        std::fs::write(p("b.txt"), b"b").unwrap();
        std::fs::create_dir(p("폴더")).unwrap();
        std::fs::write(util::join(&p("폴더"), "안.txt"), b"in").unwrap();
        std::os::unix::fs::symlink(p("없는 곳"), p("끊어진 링크")).unwrap();
        for target in ["b.txt", "폴더", "끊어진 링크"] {
            let e = rename_excl(&p("a.txt"), &p(target)).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{target}");
            let e = rename_checked(&p("a.txt"), &p(target)).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{target}");
        }
        assert_eq!(std::fs::read(p("a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(p("b.txt")).unwrap(), b"b");
        assert_eq!(std::fs::read(util::join(&p("폴더"), "안.txt")).unwrap(), b"in");
        assert!(std::fs::symlink_metadata(p("끊어진 링크")).unwrap().file_type().is_symlink());
        // 폴더를 파일 이름 위로도 옮기지 않는다
        assert!(rename_excl(&p("폴더"), &p("b.txt")).is_err());
        // 빈 이름으로는 간다
        let id = inode(&p("a.txt")).unwrap();
        rename_excl(&p("a.txt"), &p("c.txt")).unwrap();
        assert_eq!((std::fs::read(p("c.txt")).unwrap(), inode(&p("c.txt")).unwrap()), (b"a".to_vec(), id));
        assert!(std::fs::symlink_metadata(p("a.txt")).is_err());
        rename_checked(&p("c.txt"), &p("d.txt")).unwrap();
        assert_eq!(names(&dir), ["b.txt", "d.txt", "끊어진 링크", "폴더"]);
        // 없는 것은 바꿀 수 없다
        assert_eq!(rename_excl(&p("a.txt"), &p("e.txt")).unwrap_err().kind(), io::ErrorKind::NotFound);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 링크의 이름을 바꾸면 링크 자체만 바뀐다: 가리키던 폴더와 그 안은 그대로다
    #[cfg(target_os = "macos")]
    #[test]
    fn rename_moves_the_link_itself() {
        let dir = temp("link");
        let p = |n: &str| util::join(&dir, n);
        std::fs::create_dir(p("진짜")).unwrap();
        std::fs::write(util::join(&p("진짜"), "안.txt"), b"in").unwrap();
        std::os::unix::fs::symlink(p("진짜"), p("링크")).unwrap();
        let (link, real) = (inode(&p("링크")).unwrap(), inode(&p("진짜")).unwrap());
        assert_ne!(link, real, "the number is the link's own");
        rename_excl(&p("링크"), &p("새 링크")).unwrap();
        assert!(std::fs::symlink_metadata(p("새 링크")).unwrap().file_type().is_symlink());
        assert_eq!((inode(&p("새 링크")).unwrap(), inode(&p("진짜")).unwrap()), (link, real));
        assert_eq!(std::fs::read(util::join(&p("진짜"), "안.txt")).unwrap(), b"in");
        assert_eq!(names(&dir), ["새 링크", "진짜"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 한 파일에 이름이 둘 붙어 있을 때(하드 링크): 한 이름을 다른 이름으로 바꾸지 않는다.
    /// 파일 번호가 같다고 '대소문자만 바꾸기'로 보면, 아무것도 바뀌지 않았는데 바꿨다고 하게 된다
    #[cfg(target_os = "macos")]
    #[test]
    fn another_name_of_the_same_file_is_not_a_case_only_rename() {
        let dir = temp("hardlink");
        let p = |n: &str| util::join(&dir, n);
        std::fs::write(p("원본.txt"), b"one").unwrap();
        std::fs::hard_link(p("원본.txt"), p("다른 이름.txt")).unwrap();
        assert_eq!(inode(&p("원본.txt")).unwrap(), inode(&p("다른 이름.txt")).unwrap());
        assert_eq!(rename_excl(&p("원본.txt"), &p("다른 이름.txt")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(rename_checked(&p("원본.txt"), &p("다른 이름.txt")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(names(&dir), ["다른 이름.txt", "원본.txt"]);
        // 다른 폴더에 있는 같은 파일의 이름 위로도 가지 않는다
        std::fs::create_dir(p("폴더")).unwrap();
        std::fs::hard_link(p("원본.txt"), util::join(&p("폴더"), "원본.txt")).unwrap();
        assert_eq!(rename_excl(&p("원본.txt"), &util::join(&p("폴더"), "원본.txt")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(names(&dir), ["다른 이름.txt", "원본.txt", "폴더"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 대소문자를 가리는 볼륨에서는 대소문자만 다른 두 이름이 서로 다른 파일이다: 한쪽을 다른 쪽 이름으로 바꾸지 않는다.
    /// (맥 기본 볼륨은 가리지 않아 두 이름이 같은 파일이 된다. 그때는 볼 것이 없다. CI는 가리는 디스크 이미지에서 한 번 더 돌린다)
    #[cfg(target_os = "macos")]
    #[test]
    fn different_cases_are_different_files_on_a_case_sensitive_volume() {
        let dir = temp("cs");
        let p = |n: &str| util::join(&dir, n);
        std::fs::write(p("Photo.JPG"), b"upper").unwrap();
        std::fs::write(p("photo.jpg"), b"lower").unwrap();
        if names(&dir).len() == 2 {
            for (from, to) in [("Photo.JPG", "photo.jpg"), ("photo.jpg", "Photo.JPG")] {
                assert_eq!(rename_excl(&p(from), &p(to)).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
                assert_eq!(rename_checked(&p(from), &p(to)).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
            }
            assert_eq!((std::fs::read(p("Photo.JPG")).unwrap(), std::fs::read(p("photo.jpg")).unwrap()), (b"upper".to_vec(), b"lower".to_vec()));
            // 목록에도 둘 다 나오고 파일 번호가 다르다
            let entries = crate::listing::list_dir(&dir).unwrap();
            assert_eq!(entries.len(), 2);
            assert_ne!(entries[0].file_id, entries[1].file_id);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 대소문자만 바꾸기: 대소문자를 가리지 않는 볼륨(맥 기본)에서도 되고, 같은 파일로 남는다
    #[cfg(target_os = "macos")]
    #[test]
    fn case_only_rename_keeps_the_same_file() {
        let dir = temp("case");
        let p = |n: &str| util::join(&dir, n);
        std::fs::write(p("report.txt"), b"r").unwrap();
        let id = inode(&p("report.txt")).unwrap();
        rename_excl(&p("report.txt"), &p("Report.TXT")).unwrap();
        assert_eq!(names(&dir), ["Report.TXT"]);
        assert_eq!((inode(&p("Report.TXT")).unwrap(), std::fs::read(p("Report.TXT")).unwrap()), (id, b"r".to_vec()));
        rename_checked(&p("Report.TXT"), &p("REPORT.txt")).unwrap();
        assert_eq!(names(&dir), ["REPORT.txt"]);
        assert_eq!(inode(&p("REPORT.txt")).unwrap(), id);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
