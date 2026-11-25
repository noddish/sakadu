//! 압축 파일(zip·7z 등)을 폴더처럼 다루는 공통 규칙 (Win32·AppKit 없음): 어떤 이름이 압축 파일인지, 경로의 어디까지가
//! 압축 파일인지, 압축 파일 안 항목 이름을 다듬어 폴더 목록으로 엮는 법, 손상된 압축 파일을 가리는 표식, 미리 보기 글.
//! 경로는 "…\자료.zip\폴더"(맥은 "…/자료.zip/폴더")처럼 압축 파일 이름 뒤로 이어 쓴다.
//! 읽기는 운영 체제마다 따로 있다 (Windows는 셸의 압축 폴더: src/archive.rs, 맥은 libarchive: src/mac/archive.rs)

use crate::hangul;
use crate::listing::{ATTR_DIR, ATTR_HIDDEN, Entry, Kind, format_size};
use crate::util;
use std::collections::HashMap;

/// 폴더처럼 들어가 보는 압축 형식 (확장자, 소문자). 읽지 못하는 PC·맥에서는 기본 프로그램으로 연다
pub const EXTS: &[&str] = &["zip", "7z", "rar", "tar", "tgz", "gz", "tbz2", "bz2", "txz", "xz", "tzst", "zst", "cab"];

/// 이름이 압축 파일 확장자로 끝나는지
pub fn is_archive_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => EXTS.iter().any(|x| x.eq_ignore_ascii_case(ext)),
        _ => false,
    }
}

/// 경로가 압축 파일 자체이거나 그 안이면, 그 압축 파일의 경로 (맨 앞의 압축 파일).
/// 이름이 압축 확장자인 폴더("자료.zip"이라는 폴더)는 그냥 폴더다 (파일인지는 디스크에 묻는다)
pub fn archive_of(path: &str) -> Option<String> {
    archive_of_with(path, |p| std::fs::metadata(p).is_ok_and(|m| m.is_file()))
}

/// archive_of와 같되, 그 자리가 파일인지는 is_file에 묻는다 (압축 확장자로 끝나는 조각에서만 묻는다)
pub fn archive_of_with(path: &str, is_file: impl Fn(&str) -> bool) -> Option<String> {
    let sep = util::sep_of(path);
    let mut end = 0;
    for part in path.split(sep) {
        let start = end;
        end = start + part.len();
        if (start > 0 || !part.is_empty()) && is_archive_name(part) && is_file(&path[..end]) {
            return Some(path[..end].to_string());
        }
        // 구분자('\', '/')는 한 바이트다
        end += 1;
    }
    None
}

/// 경로가 압축 파일 '안'의 항목인지 (압축 파일 자체는 아니다). 압축 파일 자체는 보통 파일처럼 옮기고 끈다
pub fn inside(path: &str) -> bool {
    archive_of(path).is_some_and(|a| !util::eq_path(&a, path))
}

/// 압축 파일 archive 안의 경로 path에서 압축 파일 뒤의 안쪽 경로 (맨 위면 ""). 조각은 압축 파일 안 이름처럼 '/'로 잇는다.
/// path가 archive 자신이나 그 안이 아니면 None
pub fn inner_path(path: &str, archive: &str) -> Option<String> {
    // 한글처럼 여러 바이트인 글자 가운데를 자르지 않게 글자 경계에서만 비교한다
    let head = path.get(..archive.len()).filter(|h| util::eq_path(h, archive))?;
    let rest = &path[head.len()..];
    let sep = util::sep_of(archive);
    if !rest.is_empty() && !rest.starts_with(sep) {
        return None;
    }
    Some(rest.split(sep).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/"))
}

/// 압축 파일에 적힌 항목 이름을 안쪽 경로로 다듬는다: 앞의 "./", 빈 조각과 ".", 끝의 '/'를 뺀다.
/// 압축 파일 밖을 가리키는 이름(절대 경로, ".." 조각)과 빈 이름은 None: 그런 항목은 보이지도 꺼내지도 않는다
pub fn entry_path(raw: &str) -> Option<String> {
    if raw.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// 압축 파일 안 항목 하나 (압축 파일에 적힌 차례대로): entry_path로 다듬은 안쪽 경로, 폴더인지, 크기, 수정한 때 (FILETIME 단위, 모르면 0)
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub modified: u64,
}

/// 안쪽 경로 path가 폴더 dir("" 이면 맨 위) 안이면 그 뒤의 나머지 (dir 자신이면 ""). 아니면 None.
/// 맥이 만든 압축 파일은 한글 이름이 자모로 나뉘어 적혀 있기도 해서, 둘 다 모아 쓴 꼴로 맞춘다
pub fn rest_under(path: &str, dir: &str) -> Option<String> {
    let (path, dir) = (hangul::compose(path), hangul::compose(dir));
    if dir.is_empty() {
        return Some(path.into_owned());
    }
    if path == dir {
        return Some(String::new());
    }
    path.strip_prefix(&*dir).and_then(|r| r.strip_prefix('/')).map(str::to_string)
}

/// 폴더 inner("" 이면 맨 위)의 바로 아래 항목들 (압축 파일 안의 목록). 폴더 항목이 따로 적혀 있지 않아도 그 안의 항목이 있으면
/// 폴더로 보인다. 이름은 한글을 모아 쓴 꼴로 보이고 (꺼낼 때도 그 꼴로 맞춘다), 같은 이름은 한 번만 (폴더가 이기고, 크기와 때는
/// 나중에 적힌 것). 점으로 시작하는 이름과 맥이 붙이는 __MACOSX 폴더는 숨김
pub fn children(items: &[Item], inner: &str) -> Vec<Entry> {
    struct Child {
        name: String,
        dir: bool,
        size: u64,
        modified: u64,
    }
    let mut out: Vec<Child> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for it in items {
        let Some(rest) = rest_under(&it.path, inner).filter(|r| !r.is_empty()) else { continue };
        // 더 깊은 항목이면 그 첫 조각이 폴더다 (그 폴더의 크기와 때는 모른다)
        let (name, deeper) = match rest.split_once('/') {
            Some((first, _)) => (first.to_string(), true),
            None => (rest, false),
        };
        let dir = deeper || it.dir;
        let i = *at.entry(name.clone()).or_insert_with(|| {
            out.push(Child { name, dir, size: 0, modified: 0 });
            out.len() - 1
        });
        let c = &mut out[i];
        c.dir |= dir;
        if !deeper {
            c.modified = it.modified;
            c.size = it.size;
        }
    }
    out.into_iter()
        .map(|c| {
            let mut e = Entry::from_str(&c.name, if c.dir { Kind::Dir } else { Kind::File });
            e.attrs = if c.dir { ATTR_DIR } else { 0 };
            if c.name.starts_with('.') || c.name == "__MACOSX" {
                e.attrs |= ATTR_HIDDEN;
            }
            e.size = if c.dir { 0 } else { c.size };
            e.modified = c.modified;
            e
        })
        .collect()
}

/// 안쪽 경로 inner가 폴더면 Some(true), 파일이면 Some(false), 압축 파일에 없으면 None. 맨 위("")는 폴더다
pub fn kind_of(items: &[Item], inner: &str) -> Option<bool> {
    if inner.is_empty() {
        return Some(true);
    }
    let mut file = false;
    for it in items {
        match rest_under(&it.path, inner).as_deref() {
            Some("") if it.dir => return Some(true),
            Some("") => file = true,
            Some(_) => return Some(true),
            None => {}
        }
    }
    file.then_some(false)
}

/// zip의 끝 레코드는 마지막 22바이트에 주석(최대 65,535바이트)이 붙은 자리에 있다
const ZIP_TAIL: u64 = 22 + 65_535;

/// 압축 파일에 그 형식의 표식이 없는지 (파일을 읽지 못해도 없다고 본다).
/// 맨 위가 빈 목록일 때 정말 빈 압축 파일인지, 손상되거나 다른 형식이라 읽지 못한 것인지 가린다
pub fn marks_missing(archive: &str) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let ext = archive.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let Ok(mut f) = std::fs::File::open(archive) else { return true };
    let mut head = [0u8; 8];
    let n = f.read(&mut head).unwrap_or(0);
    let mut tail = Vec::new();
    if ext == "zip" {
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if f.seek(SeekFrom::Start(len.saturating_sub(ZIP_TAIL))).is_err() || f.read_to_end(&mut tail).is_err() {
            return true;
        }
    }
    unreadable(&ext, &head[..n], &tail)
}

/// 빈 목록을 준 압축 파일이 정말 빈 것인지, 손상되거나 다른 형식이라 읽지 못한 것인지 표식으로 가린다.
/// zip은 끝 레코드(PK 05 06), 나머지는 맨 앞의 표식. tar는 빈 파일이 0으로 채워져 있어 가리지 않는다
fn unreadable(ext: &str, head: &[u8], tail: &[u8]) -> bool {
    let starts = |m: &[u8]| head.starts_with(m);
    match ext {
        "zip" => !tail.windows(4).any(|w| w == [0x50, 0x4b, 0x05, 0x06]),
        "7z" => !starts(&[0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c]),
        "rar" => !starts(&[0x52, 0x61, 0x72, 0x21, 0x1a, 0x07]),
        "gz" | "tgz" => !starts(&[0x1f, 0x8b]),
        "bz2" | "tbz2" => !starts(b"BZh"),
        "xz" | "txz" => !starts(&[0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]),
        "zst" | "tzst" => !starts(&[0x28, 0xb5, 0x2f, 0xfd]),
        "cab" => !starts(b"MSCF"),
        _ => false,
    }
}

/// 압축 파일 미리 보기 글: 항목 수, 폴더(이름 뒤에 경로 구분자), 파일(이름 · 크기). 폴더 먼저, 이름은 자연 순서. 너무 많으면 앞부분만
pub fn archive_text(entries: &[Entry]) -> String {
    const SHOW: usize = 300;
    let mut v: Vec<&Entry> = entries.iter().collect();
    v.sort_by(|a, b| b.is_dir().cmp(&a.is_dir()).then_with(|| crate::natural::cmp_z(a.name_z(), b.name_z())));
    let dirs = v.iter().filter(|e| e.is_dir()).count();
    let mut out = format!("압축 파일 안 · 폴더 {dirs}개, 파일 {}개\n\n", v.len() - dirs);
    for e in v.iter().take(SHOW) {
        if e.is_dir() {
            out.push_str(&format!("{}{}\n", e.name_string(), std::path::MAIN_SEPARATOR));
        } else {
            out.push_str(&format!("{} · {}\n", e.name_string(), format_size(e.size)));
        }
    }
    if v.len() > SHOW {
        out.push_str(&format!("… 외 {}개\n", v.len() - SHOW));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_names() {
        assert!(is_archive_name("자료.zip"));
        assert!(is_archive_name("a.tar.GZ"));
        assert!(is_archive_name("b.7z"));
        assert!(!is_archive_name(".zip"));
        assert!(!is_archive_name("zip"));
        assert!(!is_archive_name("notes.txt"));
    }

    /// 빈 압축 파일(주석이 긴 빈 zip, 0으로 채운 빈 tar)은 읽은 것이고, 표식이 없는 것만 읽지 못한 것이다
    #[test]
    fn empty_archives_are_not_broken() {
        let mut empty_zip = vec![0x50, 0x4b, 0x05, 0x06];
        empty_zip.resize(22, 0);
        assert!(!unreadable("zip", &empty_zip[..8], &empty_zip));
        // 주석 2,000바이트가 붙은 빈 zip
        let mut commented = empty_zip.clone();
        commented.extend(std::iter::repeat_n(b'x', 2000));
        assert!(!unreadable("zip", &commented[..8], &commented));
        // 0으로 채운 빈 tar (10,240바이트)
        assert!(!unreadable("tar", &[0; 8], &[]));
        assert!(!unreadable("7z", &[0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c, 0, 4], &[]));
        assert!(!unreadable("gz", &[0x1f, 0x8b, 8, 0], &[]));
        // 아무 바이트나 든 가짜 압축 파일
        let junk = [0xc8, 0x9f, 0xe3, 0x11, 0x42, 0x07, 0x99, 0x31];
        assert!(unreadable("zip", &junk, &junk));
        assert!(unreadable("7z", &junk, &[]));
        assert!(unreadable("gz", &junk, &[]));
    }

    /// 표식은 파일에서 읽는다: 0으로 채운 zip과 없는 파일은 읽지 못한 것, 끝 레코드만 있는 빈 zip은 읽은 것
    #[test]
    fn marks_are_read_from_the_file() {
        let dir = std::env::temp_dir().join(format!("sakadu-marks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let zeros = dir.join("영.zip");
        std::fs::write(&zeros, vec![0u8; 4096]).unwrap();
        let mut empty = vec![0x50, 0x4b, 0x05, 0x06];
        empty.resize(22, 0);
        let ok = dir.join("빈.zip");
        std::fs::write(&ok, &empty).unwrap();
        assert!(marks_missing(&zeros.to_string_lossy()));
        assert!(!marks_missing(&ok.to_string_lossy()));
        assert!(marks_missing(&dir.join("없음.zip").to_string_lossy()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_of_needs_a_file() {
        let dir = std::env::temp_dir().join(format!("sakadu-arch-lib-{}", std::process::id()));
        let fake_dir = dir.join("폴더.zip");
        std::fs::create_dir_all(&fake_dir).unwrap();
        let file = dir.join("묶음.zip");
        std::fs::write(&file, b"PK").unwrap();
        let d = dir.to_string_lossy().to_string();
        let at = |rel: &[&str]| rel.iter().fold(d.clone(), |p, s| util::join(&p, s));
        let zip = at(&["묶음.zip"]);
        assert_eq!(archive_of(&at(&["폴더.zip", "안"])), None);
        assert_eq!(archive_of(&zip), Some(zip.clone()));
        assert_eq!(archive_of(&at(&["묶음.zip", "사진", "a.jpg"])), Some(zip.clone()));
        assert_eq!(archive_of(&d), None);
        assert!(!inside(&zip));
        assert!(inside(&at(&["묶음.zip", "사진"])));
        assert!(!inside(&at(&["폴더.zip", "안"])));
        // 압축 파일 안의 압축 파일은 바깥 것이 먼저다
        assert_eq!(archive_of(&at(&["묶음.zip", "안.zip", "x"])), Some(zip.clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 파일인지 묻는 것은 압축 확장자로 끝나는 조각뿐이고, 맨 앞의 것이 이긴다
    #[test]
    fn archive_of_asks_only_archive_names() {
        let asked = std::cell::RefCell::new(Vec::new());
        let root = if cfg!(windows) { "C:\\자료" } else { "/자료" };
        let path = util::join(&util::join(&util::join(root, "a.zip"), "b.7z"), "c.txt");
        let got = archive_of_with(&path, |p| {
            asked.borrow_mut().push(p.to_string());
            true
        });
        assert_eq!(got, Some(util::join(root, "a.zip")));
        assert_eq!(*asked.borrow(), [util::join(root, "a.zip")]);
    }

    #[test]
    fn inner_paths_use_slashes() {
        let root = if cfg!(windows) { "C:\\자료" } else { "/자료" };
        let zip = util::join(root, "묶음.zip");
        assert_eq!(inner_path(&zip, &zip).as_deref(), Some(""));
        assert_eq!(inner_path(&util::join(&util::join(&zip, "사진"), "여름"), &zip).as_deref(), Some("사진/여름"));
        assert_eq!(inner_path(&util::join(root, "묶음.zipx"), &zip), None);
        assert_eq!(inner_path(root, &zip), None);
    }

    /// 압축 파일 밖을 가리키는 이름은 버리고, 나머지는 "./"·끝 '/'·빈 조각을 뺀 꼴로
    #[test]
    fn entry_names_are_tidied_and_escapes_refused() {
        assert_eq!(entry_path("./사진/a.jpg").as_deref(), Some("사진/a.jpg"));
        assert_eq!(entry_path("사진/").as_deref(), Some("사진"));
        assert_eq!(entry_path("a//b/./c").as_deref(), Some("a/b/c"));
        assert_eq!(entry_path("./"), None);
        assert_eq!(entry_path(""), None);
        for bad in ["../evil", "a/../../evil", "/etc/passwd", "a/.."] {
            assert_eq!(entry_path(bad), None, "{bad}");
        }
        // '..'로 시작할 뿐인 이름은 괜찮다
        assert_eq!(entry_path("..숨김/x").as_deref(), Some("..숨김/x"));
    }

    fn item(path: &str, dir: bool, size: u64) -> Item {
        Item { path: path.to_string(), dir, size, modified: 7 }
    }

    /// 폴더 항목이 따로 없어도 그 안의 항목으로 폴더가 생기고, 맥의 자모로 나뉜 한글 이름은 모아 쓴 꼴로 보이고 찾아진다
    #[test]
    fn children_build_the_folder_tree() {
        // "사진" 폴더를 자모로 나눠 적은 이름 (맥이 만든 zip)
        let nfd = "\u{1109}\u{1161}\u{110c}\u{1175}\u{11ab}";
        let items = [
            item("메모.txt", false, 5),
            item(&format!("{nfd}/바다.jpg"), false, 300),
            item(&format!("{nfd}/여름/노을.jpg"), false, 200),
            item("문서", true, 0),
            item("__MACOSX/._메모.txt", false, 1),
            item(".숨김", false, 1),
        ];
        let top = children(&items, "");
        let seen: Vec<(String, bool, u64)> = top.iter().map(|e| (e.name_string(), e.is_dir(), e.size)).collect();
        let want = [("메모.txt", false, 5), ("사진", true, 0), ("문서", true, 0), ("__MACOSX", true, 0), (".숨김", false, 1)];
        assert_eq!(seen, want.map(|(n, d, s)| (n.to_string(), d, s)));
        assert!(top.iter().filter(|e| e.is_hidden()).map(Entry::name_string).eq(["__MACOSX", ".숨김"]));
        // 모아 쓴 꼴로 쳐도 자모로 적힌 폴더 안이 보인다. 더 깊은 폴더도 폴더로
        let inside: Vec<(String, bool)> = children(&items, "사진").iter().map(|e| (e.name_string(), e.is_dir())).collect();
        assert_eq!(inside, [("바다.jpg".to_string(), false), ("여름".to_string(), true)]);
        assert_eq!(children(&items, "사진/여름").len(), 1);
        assert!(children(&items, "없는 폴더").is_empty());
        assert_eq!(kind_of(&items, "사진"), Some(true));
        assert_eq!(kind_of(&items, "사진/바다.jpg"), Some(false));
        assert_eq!(kind_of(&items, "문서"), Some(true));
        assert_eq!(kind_of(&items, "없음"), None);
        assert_eq!(kind_of(&items, ""), Some(true));
    }

    /// 같은 이름이 두 번 적혔으면(덧붙여 쓴 tar) 한 줄로, 크기는 나중 것. 폴더와 파일이 겹치면 폴더
    #[test]
    fn children_merge_repeated_names() {
        let items = [item("a.txt", false, 1), item("a.txt", false, 9), item("b", false, 3), item("b/c", false, 4)];
        let top: Vec<(String, bool, u64)> = children(&items, "").iter().map(|e| (e.name_string(), e.is_dir(), e.size)).collect();
        assert_eq!(top, [("a.txt".to_string(), false, 9), ("b".to_string(), true, 0)]);
    }

    #[test]
    fn preview_text_lists_folders_first() {
        let items = [item("나.txt", false, 2048), item("가/x", false, 1), item("다.txt", false, 10)];
        let text = archive_text(&children(&items, ""));
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(text, format!("압축 파일 안 · 폴더 1개, 파일 2개\n\n가{sep}\n나.txt · 2.0 KB\n다.txt · 10 B\n"));
    }
}
