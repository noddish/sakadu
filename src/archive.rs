//! 압축 파일(zip·7z 등)을 폴더처럼 읽기. Windows 셸(내장 압축 폴더)이 읽는 형식만 다루고, 안은 읽기만 한다.
//! 경로는 "C:\자료\묶음.zip\폴더"처럼 압축 파일 이름 뒤로 이어 쓴다 (셸이 이 경로를 그대로 알아듣는다)

use crate::listing::{ATTR_DIR, Entry, Kind, filetime_u64};
use crate::util;
use windows::Win32::Foundation::{FILETIME, PROPERTYKEY};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemServices::{SFGAO_FLAGS, SFGAO_FOLDER, SFGAO_STREAM};
use windows::Win32::UI::Shell::*;
use windows::core::{GUID, HSTRING, Interface};

/// 셸 속성: 크기(System.Size)와 수정한 날짜(System.DateModified)
const PKEY_SIZE: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac), pid: 12 };
const PKEY_DATE_MODIFIED: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac), pid: 14 };

/// 폴더처럼 들어가 보는 압축 형식 (확장자, 소문자). 셸이 못 읽는 PC에서는 기본 프로그램으로 연다
const EXTS: &[&str] = &["zip", "7z", "rar", "tar", "tgz", "gz", "tbz2", "bz2", "txz", "xz", "tzst", "zst", "cab"];

/// 이름이 압축 파일 확장자로 끝나는지
pub fn is_archive_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => EXTS.iter().any(|x| x.eq_ignore_ascii_case(ext)),
        _ => false,
    }
}

/// 경로가 압축 파일 자체이거나 그 안이면, 그 압축 파일의 경로 (맨 앞의 압축 파일).
/// 이름이 압축 확장자인 폴더("자료.zip"이라는 폴더)는 그냥 폴더다
pub fn archive_of(path: &str) -> Option<String> {
    let mut end = 0;
    for part in path.split('\\') {
        let start = end;
        end = start + part.len();
        if start > 0 || !part.is_empty() {
            let prefix = &path[..end];
            if is_archive_name(part) && std::fs::metadata(prefix).is_ok_and(|m| m.is_file()) {
                return Some(prefix.to_string());
            }
        }
        end += 1;
    }
    None
}

/// 경로가 압축 파일 '안'의 항목인지 (압축 파일 자체는 아니다). 압축 파일 자체는 보통 파일처럼 옮기고 끈다
pub fn inside(path: &str) -> bool {
    archive_of(path).is_some_and(|a| !util::eq_path(&a, path))
}

/// 열 때 폴더처럼 들어갈 압축 파일인지: 압축 파일 이름이고, 다른 압축 파일 안에 있지 않고, 셸이 그 형식을 읽는다.
/// 압축 파일 안의 압축 파일은 파일처럼 셸의 '열기'로 (풀어서 기본 프로그램으로 연다)
pub fn enterable(path: &str) -> bool {
    is_archive_name(util::file_name(path)) && !inside(path) && opens_as_folder(path)
}

fn message(e: windows::core::Error) -> String {
    let m = e.message();
    if m.trim().is_empty() { format!("오류 {:#010x}", e.code().0) } else { m.trim().to_string() }
}

/// 셸이 이 파일을 폴더로 열 수 있는지 (그 형식을 읽는 압축 폴더가 이 PC에 있는지). 파일 내용은 읽지 않는다
pub fn opens_as_folder(path: &str) -> bool {
    shell_item(path).and_then(|item| unsafe { item.GetAttributes(SFGAO_FOLDER) }.ok()).is_some_and(|a| a.contains(SFGAO_FOLDER))
}

/// 압축 파일 안의 항목이 폴더인지 (없으면 None). 압축 파일 안의 압축 파일은 파일이다
pub fn is_folder(path: &str) -> Option<bool> {
    let attrs = unsafe { shell_item(path)?.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM) }.ok()?;
    Some(folder_attrs(attrs))
}

fn shell_item(path: &str) -> Option<IShellItem> {
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None) }.ok()
}

/// 폴더(SFGAO_FOLDER)이면서 파일(SFGAO_STREAM)인 것은 압축 파일 안의 압축 파일이다. 파일로 다룬다
fn folder_attrs(attrs: SFGAO_FLAGS) -> bool {
    attrs.contains(SFGAO_FOLDER) && !attrs.contains(SFGAO_STREAM)
}

/// 압축 파일 안의 한 폴더 목록. archive는 그 압축 파일 (archive_of). 크기와 수정한 날짜는 셸이 알려 주는 대로 (모르면 0)
pub fn list(path: &str, archive: &str) -> Result<Vec<Entry>, String> {
    let folder = unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path), None) }.map_err(message)?;
    let all: IEnumShellItems = unsafe { folder.BindToHandler(None, &BHID_EnumItems) }.map_err(message)?;
    let mut out = Vec::new();
    loop {
        let mut one = [None];
        let mut got = 0u32;
        if unsafe { all.Next(&mut one, Some(&mut got)) }.is_err() || got == 0 {
            break;
        }
        let Some(item) = one[0].take() else { break };
        let Ok(raw) = (unsafe { item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING) }) else { continue };
        let name = unsafe { raw.as_wide() }.to_vec();
        unsafe { CoTaskMemFree(Some(raw.0 as *const _)) };
        if name.is_empty() {
            continue;
        }
        let dir = folder_attrs(unsafe { item.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM) }.unwrap_or_default());
        let mut e = Entry::new(&name, if dir { Kind::Dir } else { Kind::File });
        if dir {
            e.attrs = ATTR_DIR;
        }
        if let Ok(two) = item.cast::<IShellItem2>() {
            if !dir {
                e.size = unsafe { two.GetUInt64(&PKEY_SIZE) }.unwrap_or(0);
            }
            e.modified = unsafe { two.GetFileTime(&PKEY_DATE_MODIFIED) }.map(|ft: FILETIME| filetime_u64(&ft)).unwrap_or(0);
        }
        out.push(e);
    }
    // 손상된 압축 파일은 셸이 오류 없이 빈 목록을 준다. 맨 위가 비었으면 형식의 표식으로 정말 빈 것인지 가린다
    if out.is_empty() && util::eq_path(path, archive) && marks_missing(archive) {
        return Err("압축 파일을 읽지 못했어요 · 손상됐거나 이 PC가 읽지 못하는 형식이에요".to_string());
    }
    Ok(out)
}

/// zip의 끝 레코드는 마지막 22바이트에 주석(최대 65,535바이트)이 붙은 자리에 있다
const ZIP_TAIL: u64 = 22 + 65_535;

/// 압축 파일에 그 형식의 표식이 없는지 (파일을 읽지 못해도 없다고 본다)
fn marks_missing(archive: &str) -> bool {
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

/// 셸이 빈 목록을 준 압축 파일이 정말 빈 것인지, 손상되거나 다른 형식이라 읽지 못한 것인지 표식으로 가린다.
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

    #[test]
    fn archive_of_needs_a_file() {
        let dir = std::env::temp_dir().join(format!("sakadu-arch-{}", std::process::id()));
        let fake_dir = dir.join("폴더.zip");
        std::fs::create_dir_all(&fake_dir).unwrap();
        let file = dir.join("묶음.zip");
        std::fs::write(&file, b"PK").unwrap();
        let d = dir.to_string_lossy().to_string();
        assert_eq!(archive_of(&format!("{d}\\폴더.zip\\안")), None);
        assert_eq!(archive_of(&format!("{d}\\묶음.zip")), Some(format!("{d}\\묶음.zip")));
        assert_eq!(archive_of(&format!("{d}\\묶음.zip\\사진\\a.jpg")), Some(format!("{d}\\묶음.zip")));
        assert_eq!(archive_of(&d), None);
        assert!(!inside(&format!("{d}\\묶음.zip")));
        assert!(inside(&format!("{d}\\묶음.zip\\사진")));
        assert!(!inside(&format!("{d}\\폴더.zip\\안")));
        // 압축 파일 안의 압축 파일은 들어가지 않고 연다 (셸에 묻기 전에 가린다)
        assert!(!enterable(&format!("{d}\\묶음.zip\\안.zip")));
        assert!(!enterable(&format!("{d}\\묶음.zip\\사진")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
