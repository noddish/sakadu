//! 기본 파일 관리자 등록: 폴더·드라이브 열기와 Win+E를 Sakadu로 연결한다.
//! 현재 사용자(HKCU)만 바꾸고, 원래 기본 동작을 백업해 두었다가 해제할 때 되돌린다.
//! 작업 표시줄·바탕 화면은 계속 탐색기가 맡고, 제어판·휴지통 같은 특수 폴더도 탐색기로 열린다.

use crate::util::wide;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use windows::core::{HSTRING, PCWSTR};

pub const CLASSES: &str = "Software\\Classes";
const VERB: &str = "openinsakadu";
const BACKUP: &str = "SakaduPrevDefault";
/// Win+E가 여는 대상 ("홈/내 PC" 새 창)
const WIN_E: &str = "CLSID\\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";
const STATE: &str = "Software\\Sakadu";

fn set(path: &str, name: &str, data: &str) -> Result<(), String> {
    unsafe {
        let mut key = HKEY::default();
        let r = RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None);
        if r != ERROR_SUCCESS {
            return Err(format!("{path}: {r:?}"));
        }
        let w = wide(data);
        let bytes = std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2);
        let r = RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(key);
        if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("{path}\\{name}: {r:?}")) }
    }
}

fn get(path: &str, name: &str) -> Option<String> {
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(path),
            &HSTRING::from(name),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    (r == ERROR_SUCCESS).then(|| crate::util::from_wide(&buf))
}

fn delete_value(path: &str, name: &str) {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, KEY_SET_VALUE, &mut key) == ERROR_SUCCESS {
            let _ = RegDeleteValueW(key, &HSTRING::from(name));
            let _ = RegCloseKey(key);
        }
    }
}

fn delete_tree(path: &str) -> bool {
    let r = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(path)) };
    r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND
}

fn key_exists(path: &str) -> bool {
    unsafe {
        let mut key = HKEY::default();
        let ok = RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, KEY_READ, &mut key) == ERROR_SUCCESS;
        if ok {
            let _ = RegCloseKey(key);
        }
        ok
    }
}

/// 등록할 실행 파일 위치. %TEMP% 등 지워질 수 있는 곳 대신 사용자별 프로그램 폴더를 쓴다
pub fn install_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Programs").join("Sakadu").join("sakadu.exe")
}

/// 지금 실행 중인 파일을 설치 위치로 복사한다. 그 위치의 파일이 실행 중이면 이름을 비켜 두고 복사
pub fn copy_self() -> Result<PathBuf, String> {
    let target = install_path();
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    if me == target {
        return Ok(target);
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if std::fs::copy(&me, &target).is_err() {
        let old = target.with_extension("old.exe");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&target, &old).map_err(|e| format!("{}: {e}", target.display()))?;
        std::fs::copy(&me, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(target)
}

pub fn is_registered(classes: &str) -> bool {
    get(&format!("{classes}\\Directory\\shell"), "").as_deref() == Some(VERB)
}

/// classes는 보통 CLASSES. 테스트에서는 다른 경로를 넘겨 실제 연결을 건드리지 않는다
pub fn register(classes: &str, exe: &Path) -> Result<(), String> {
    for class in ["Directory", "Drive"] {
        let shell = format!("{classes}\\{class}\\shell");
        let verb = format!("{shell}\\{VERB}");
        // 다시 등록할 때 백업을 우리 값으로 덮어쓰지 않도록, 이미 우리 것이면 기존 백업을 유지
        let prev = if get(&shell, "").as_deref() == Some(VERB) { get(&verb, BACKUP).unwrap_or_default() } else { get(&shell, "").unwrap_or_default() };
        set(&verb, "", "Sakadu에서 열기")?;
        set(&verb, BACKUP, &prev)?;
        set(&format!("{verb}\\command"), "", &format!("\"{}\" \"%1\"", exe.display()))?;
        set(&shell, "", VERB)?;
    }
    // Win+E: 원래 키가 없었으면 해제할 때 통째로 지운다
    let win_e = format!("{classes}\\{WIN_E}");
    if !key_exists(&win_e) {
        set(STATE, &format!("WinEKeyCreated:{classes}"), "1")?;
    }
    let cmd = format!("{win_e}\\shell\\opennewwindow\\command");
    set(&cmd, "", &format!("\"{}\"", exe.display()))?;
    // 비워 두어야 탐색기의 기본 처리 대신 위 명령이 실행된다
    set(&cmd, "DelegateExecute", "")?;
    notify();
    Ok(())
}

pub fn unregister(classes: &str) {
    for class in ["Directory", "Drive"] {
        let shell = format!("{classes}\\{class}\\shell");
        let verb = format!("{shell}\\{VERB}");
        if get(&shell, "").as_deref() == Some(VERB) {
            match get(&verb, BACKUP) {
                Some(p) if !p.is_empty() => {
                    let _ = set(&shell, "", &p);
                }
                _ => delete_value(&shell, ""),
            }
        }
        delete_tree(&verb);
    }
    let win_e = format!("{classes}\\{WIN_E}");
    let created = format!("WinEKeyCreated:{classes}");
    if get(STATE, &created).is_some() {
        delete_tree(&win_e);
        delete_value(STATE, &created);
    } else {
        delete_tree(&format!("{win_e}\\shell\\opennewwindow"));
    }
    notify();
}

fn notify() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// 실제로 폴더를 열 때 쓰일 명령 (확인용)
pub fn effective_command(class: &str) -> Option<String> {
    use windows::Win32::UI::Shell::{ASSOCF_NONE, ASSOCSTR_COMMAND, AssocQueryStringW};
    use windows::core::PWSTR;
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    let hr = unsafe { AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_COMMAND, &HSTRING::from(class), PCWSTR::null(), Some(PWSTR(buf.as_mut_ptr())), &mut len) };
    hr.is_ok().then(|| crate::util::from_wide(&buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실제 연결 대신 HKCU\Software\SakaduTest 아래에서 등록·해제를 왕복한다
    #[test]
    #[ignore]
    fn register_and_restore_in_test_root() {
        let root = "Software\\SakaduTest\\Classes";
        delete_tree("Software\\SakaduTest");
        // 다른 프로그램이 기본 동작을 바꿔 둔 상황을 흉내
        set(&format!("{root}\\Directory\\shell"), "", "someothertool").unwrap();
        let exe = Path::new("C:\\Tools\\sakadu.exe");
        register(root, exe).unwrap();
        assert!(is_registered(root));
        assert_eq!(get(&format!("{root}\\Directory\\shell\\{VERB}\\command"), "").as_deref(), Some("\"C:\\Tools\\sakadu.exe\" \"%1\""));
        assert_eq!(get(&format!("{root}\\{WIN_E}\\shell\\opennewwindow\\command"), "DelegateExecute").as_deref(), Some(""));
        // 다시 등록해도 백업은 원래 값 유지
        register(root, exe).unwrap();
        unregister(root);
        assert!(!is_registered(root));
        assert_eq!(get(&format!("{root}\\Directory\\shell"), "").as_deref(), Some("someothertool"));
        assert_eq!(get(&format!("{root}\\Drive\\shell"), ""), None);
        assert!(!key_exists(&format!("{root}\\{WIN_E}")));
        delete_tree("Software\\SakaduTest");
    }
}
