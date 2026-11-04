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

pub(crate) fn set(path: &str, name: &str, data: &str) -> Result<(), String> {
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

pub(crate) fn get(path: &str, name: &str) -> Option<String> {
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

pub(crate) fn delete_value(path: &str, name: &str) {
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
    // Win+E: 다른 프로그램이 해 둔 연결을 백업해 두었다가 해제할 때 되돌린다.
    // 이미 우리 것이면(다시 등록) 백업을 우리 값으로 덮어쓰지 않는다
    let win_e = format!("{classes}\\{WIN_E}");
    let cmd = format!("{win_e}\\shell\\opennewwindow\\command");
    let ours = format!("\"{}\"", exe.display());
    if !win_e_is_ours(classes) {
        if !key_exists(&win_e) {
            set(STATE, &format!("WinEKeyCreated:{classes}"), "1")?;
        }
        set(STATE, &format!("WinEPrev:{classes}"), &get(&cmd, "").unwrap_or_else(|| NONE.to_string()))?;
        set(STATE, &format!("WinEPrevDelegate:{classes}"), &get(&cmd, "DelegateExecute").unwrap_or_else(|| NONE.to_string()))?;
    }
    set(&cmd, "", &ours)?;
    // 비워 두어야 탐색기의 기본 처리 대신 위 명령이 실행된다
    set(&cmd, "DelegateExecute", "")?;
    set(STATE, &format!("WinECmd:{classes}"), &ours)?;
    notify();
    Ok(())
}

/// 백업에서 "값이 없었음"
const NONE: &str = "\u{1}none";

/// Win+E가 아직 우리가 등록한 그대로인지 (그사이 사용자나 다른 프로그램이 바꿨으면 false)
fn win_e_is_ours(classes: &str) -> bool {
    let cmd = format!("{classes}\\{WIN_E}\\shell\\opennewwindow\\command");
    get(STATE, &format!("WinECmd:{classes}")).is_some_and(|ours| get(&cmd, "").as_deref() == Some(ours.as_str()) && get(&cmd, "DelegateExecute").as_deref() == Some(""))
}

/// 하위 키도 값도 없으면 지운다 (우리가 만든 빈 껍데기만 정리)
fn delete_if_empty(path: &str) {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, KEY_READ, &mut key) != ERROR_SUCCESS {
            return;
        }
        let (mut subkeys, mut values) = (0u32, 0u32);
        let r = RegQueryInfoKeyW(key, None, None, None, Some(&mut subkeys), None, None, Some(&mut values), None, None, None, None);
        let _ = RegCloseKey(key);
        if r == ERROR_SUCCESS && subkeys == 0 && values == 0 {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(path));
        }
    }
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
    let cmd = format!("{win_e}\\shell\\opennewwindow\\command");
    let created = format!("WinEKeyCreated:{classes}");
    let (prev, prev_delegate) = (format!("WinEPrev:{classes}"), format!("WinEPrevDelegate:{classes}"));
    // 그사이 사용자가 Win+E를 다른 프로그램으로 바꿨으면 그 값을 그대로 둔다
    if win_e_is_ours(classes) {
        let restore = |name: &str, backup: Option<String>| match backup {
            Some(v) if v != NONE => {
                let _ = set(&cmd, name, &v);
            }
            _ => delete_value(&cmd, name),
        };
        let had_before = get(STATE, &prev).is_some();
        if had_before {
            restore("", get(STATE, &prev));
            restore("DelegateExecute", get(STATE, &prev_delegate));
        } else {
            // 백업이 없던 예전 버전에서 등록한 경우: 예전처럼 우리 명령만 지운다
            delete_tree(&format!("{win_e}\\shell\\opennewwindow"));
        }
        // 우리가 만든 키만 비어 있으면 정리한다 (다른 프로그램이 그 아래 더 넣었으면 남는다)
        if get(STATE, &created).is_some() {
            for path in [cmd.clone(), format!("{win_e}\\shell\\opennewwindow"), format!("{win_e}\\shell"), win_e.clone()] {
                delete_if_empty(&path);
            }
        }
    }
    for name in [created, prev, prev_delegate, format!("WinECmd:{classes}")] {
        delete_value(STATE, &name);
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

    /// Win+E: 다른 프로그램의 연결은 해제할 때 되돌리고, 등록 뒤 사용자가 바꾼 값은 그대로 둔다
    #[test]
    #[ignore]
    fn win_e_restores_previous_and_keeps_later_choice() {
        let root = "Software\\SakaduTest2\\Classes";
        delete_tree("Software\\SakaduTest2");
        let cmd = format!("{root}\\{WIN_E}\\shell\\opennewwindow\\command");
        let exe = Path::new("C:\\Tools\\sakadu.exe");

        // 1) 다른 파일 관리자가 Win+E를 쓰고 있었다 → 껐다 켜면 그대로 돌아온다
        set(&cmd, "", "\"C:\\Other\\fm.exe\"").unwrap();
        register(root, exe).unwrap();
        assert_eq!(get(&cmd, "").as_deref(), Some("\"C:\\Tools\\sakadu.exe\""));
        register(root, exe).unwrap();
        unregister(root);
        assert_eq!(get(&cmd, "").as_deref(), Some("\"C:\\Other\\fm.exe\""));
        assert_eq!(get(&cmd, "DelegateExecute"), None, "no leftover DelegateExecute");

        // 2) 사카두 등록 뒤 사용자가 다른 프로그램으로 바꿨다 → 해제해도 사용자의 값을 지키고, 폴더 열기도 마찬가지
        register(root, exe).unwrap();
        set(&cmd, "", "\"C:\\New\\fm2.exe\"").unwrap();
        set(&format!("{root}\\Directory\\shell"), "", "usertool").unwrap();
        unregister(root);
        assert_eq!(get(&cmd, "").as_deref(), Some("\"C:\\New\\fm2.exe\""));
        assert_eq!(get(&cmd, "DelegateExecute").as_deref(), Some(""), "user's later values untouched");
        assert_eq!(get(&format!("{root}\\Directory\\shell"), "").as_deref(), Some("usertool"));

        // 3) 원래 연결이 없었다 → 해제하면 키도 남지 않는다
        delete_tree("Software\\SakaduTest2");
        register(root, exe).unwrap();
        unregister(root);
        assert!(!key_exists(&format!("{root}\\{WIN_E}")));
        assert!(get(STATE, &format!("WinECmd:{root}")).is_none(), "state cleaned");
        delete_tree("Software\\SakaduTest2");
    }
}
