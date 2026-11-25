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
    // 손상된 압축 파일은 셸이 오류 없이 빈 목록을 준다. 맨 위가 비었는데 파일은 크면 읽지 못한 것으로 본다
    // (항목이 하나도 없는 zip은 22바이트다)
    if out.is_empty() && util::eq_path(path, archive) && std::fs::metadata(archive).is_ok_and(|m| m.len() > 1024) {
        return Err("압축 파일을 읽지 못했어요 · 손상됐거나 이 PC가 읽지 못하는 형식이에요".to_string());
    }
    Ok(out)
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
