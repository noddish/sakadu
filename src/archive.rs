//! 압축 파일(zip·7z 등)을 폴더처럼 읽기. Windows 셸(내장 압축 폴더)이 읽는 형식만 다루고, 안은 읽기만 한다.
//! 경로는 "C:\자료\묶음.zip\폴더"처럼 압축 파일 이름 뒤로 이어 쓴다 (셸이 이 경로를 그대로 알아듣는다).
//! 이름·경로 규칙과 손상 표식은 맥과 함께 쓰는 공통 코드(sakadu::archive_path)에 있다

pub use sakadu::archive_path::{archive_of, inside, is_archive_name};
use crate::listing::{ATTR_DIR, Entry, Kind, filetime_u64};
use crate::util;
use sakadu::archive_path::marks_missing;
use windows::Win32::Foundation::{FILETIME, PROPERTYKEY};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemServices::{SFGAO_FLAGS, SFGAO_FOLDER, SFGAO_STREAM};
use windows::Win32::UI::Shell::*;
use windows::core::{GUID, HSTRING, Interface};

/// 셸 속성: 크기(System.Size)와 수정한 날짜(System.DateModified)
const PKEY_SIZE: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac), pid: 12 };
const PKEY_DATE_MODIFIED: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac), pid: 14 };

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

#[cfg(test)]
mod tests {
    use super::*;

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
