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

/// 두 경로가 같은 파일인지 (대소문자·짧은 이름 차이는 무시). 어느 쪽이든 없으면 false
fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => crate::util::eq_path(&a.to_string_lossy(), &b.to_string_lossy()),
        _ => false,
    }
}

/// 두 실행 파일이 같은 빌드인지: 같은 파일이거나 내용이 같다. 어느 쪽이든 읽지 못하면 false
pub fn same_build(a: &Path, b: &Path) -> bool {
    if same_path(a, b) {
        return true;
    }
    let len = |p: &Path| std::fs::metadata(p).map(|m| m.len()).ok();
    if len(a).is_none() || len(a) != len(b) {
        return false;
    }
    matches!((std::fs::read(a), std::fs::read(b)), (Ok(x), Ok(y)) if x == y)
}

/// 이 프로세스가 설치 위치의 사본인지 (로그인할 때·폴더를 열 때 실행된 것)
pub fn is_install_copy(me: &Path) -> bool {
    same_path(me, &install_path())
}

/// 설치 위치의 사본을 쓰는 설정(트레이 대기, 기본 파일 관리자)이 하나라도 켜져 있는지
pub fn install_copy_in_use() -> bool {
    crate::tray::is_enabled(crate::tray::RUN_KEY) || is_registered(CLASSES)
}

/// 설치 위치의 사본을 지금 실행한 버전으로 맞춘다. 사본을 쓰는 설정이 꺼져 있거나 이미 같으면 그대로 두고 false
pub fn sync_install_copy() -> Result<bool, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    if !install_copy_in_use() || same_build(&me, &install_path()) {
        return Ok(false);
    }
    copy_self().map(|_| true)
}

/// 지금 실행 중인 파일을 설치 위치로 복사한다. 이미 같은 빌드면 그대로 둔다.
/// 옆에 다 복사한 뒤 바꿔 넣어서, 도중에 실패해도 등록된 파일이 반쯤 쓰인 채로 남지 않는다
pub fn copy_self() -> Result<PathBuf, String> {
    let target = install_path();
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    if same_build(&me, &target) {
        return Ok(target);
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let new = target.with_extension("new.exe");
    if let Err(e) = std::fs::copy(&me, &new) {
        let _ = std::fs::remove_file(&new);
        return Err(format!("{}: {e}", new.display()));
    }
    // 백신 검사나 방금 끝난 이전 프로세스 때문에 잠깐 잠겨 있을 수 있어 몇 번 다시 해 본다
    let mut tries = 0;
    loop {
        match replace_file(&new, &target) {
            Ok(()) => {
                // 지난번에 비켜 둔 파일. 아직 실행 중이면 지워지지 않고 다음에 다시 지운다
                let _ = std::fs::remove_file(target.with_extension("old.exe"));
                return Ok(target);
            }
            Err(_) if tries < 10 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&new);
                return Err(format!("{}: {e}", target.display()));
            }
        }
    }
}

/// new를 target 자리에 넣는다. 실행 중인 파일은 덮어쓰거나 지울 수 없지만 이름은 바꿀 수 있으므로,
/// 그때는 target을 sakadu.old.exe로 비켜 두고 넣는다
fn replace_file(new: &Path, target: &Path) -> std::io::Result<()> {
    let Err(e) = std::fs::rename(new, target) else { return Ok(()) };
    if !target.exists() {
        return Err(e);
    }
    let old = target.with_extension("old.exe");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(target, &old)?;
    std::fs::rename(new, target).inspect_err(|_| {
        let _ = std::fs::rename(&old, target);
    })
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

/// Win+E가 아직 우리가 등록한 그대로인지 (그사이 사용자나 다른 프로그램이 바꿨으면 false).
/// 등록한 명령을 기록해 두지 않던 예전 버전(0.3.0까지)의 등록은 명령이 sakadu.exe를 가리키는지로 알아본다.
/// 알아보지 못하면 해제해도 Win+E가 계속 Sakadu를 열고, 다시 등록할 때 우리 명령을 "원래 값"으로 백업하게 된다
fn win_e_is_ours(classes: &str) -> bool {
    let cmd = format!("{classes}\\{WIN_E}\\shell\\opennewwindow\\command");
    if get(&cmd, "DelegateExecute").as_deref() != Some("") {
        return false;
    }
    let now = get(&cmd, "");
    match get(STATE, &format!("WinECmd:{classes}")) {
        Some(ours) => now.as_deref() == Some(ours.as_str()),
        None => now.is_some_and(|c| is_sakadu_command(&c)),
    }
}

/// 예전 버전이 Win+E에 써 둔 명령: 따옴표로 감싼 sakadu.exe 경로
fn is_sakadu_command(cmd: &str) -> bool {
    crate::util::file_name(cmd.trim().trim_matches('"')).eq_ignore_ascii_case("sakadu.exe")
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

        // 4) 예전 버전(0.3.0까지)이 등록해 둔 상태: 우리 명령을 기록해 두지 않았다 → 해제하면 Win+E에서 빠진다
        let legacy = |root: &str| {
            for class in ["Directory", "Drive"] {
                let shell = format!("{root}\\{class}\\shell");
                set(&format!("{shell}\\{VERB}\\command"), "", "\"C:\\Old\\sakadu.exe\" \"%1\"").unwrap();
                set(&shell, "", VERB).unwrap();
            }
            set(STATE, &format!("WinEKeyCreated:{root}"), "1").unwrap();
            set(&cmd, "", "\"C:\\Old\\sakadu.exe\"").unwrap();
            set(&cmd, "DelegateExecute", "").unwrap();
        };
        delete_tree("Software\\SakaduTest2");
        legacy(root);
        unregister(root);
        assert!(!key_exists(&format!("{root}\\{WIN_E}")), "legacy Win+E removed");
        assert!(!is_registered(root));
        // 예전 등록 위에 다시 등록해도 우리 명령을 원래 값으로 백업하지 않는다
        delete_tree("Software\\SakaduTest2");
        legacy(root);
        register(root, exe).unwrap();
        assert!(get(STATE, &format!("WinEPrev:{root}")).is_none(), "own command not backed up");
        unregister(root);
        assert!(!key_exists(&format!("{root}\\{WIN_E}")));
        // 예전 등록 뒤 다른 프로그램이 Win+E를 가져갔으면 그대로 둔다
        delete_tree("Software\\SakaduTest2");
        legacy(root);
        set(&cmd, "", "\"C:\\Other\\fm.exe\"").unwrap();
        unregister(root);
        assert_eq!(get(&cmd, "").as_deref(), Some("\"C:\\Other\\fm.exe\""));
        delete_tree("Software\\SakaduTest2");
        delete_value(STATE, &format!("WinEKeyCreated:{root}"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sakadu-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn same_build_compares_contents() {
        let d = temp_dir("same-build");
        let (a, b, c) = (d.join("a.exe"), d.join("b.exe"), d.join("c.exe"));
        std::fs::write(&a, b"build 1").unwrap();
        std::fs::write(&b, b"build 1").unwrap();
        std::fs::write(&c, b"build 2").unwrap();
        assert!(same_build(&a, &b), "same contents elsewhere");
        assert!(!same_build(&a, &c), "same size, different contents");
        assert!(same_build(&a, &d.join(".").join("A.EXE")), "same file spelled differently");
        assert!(!same_build(&a, &d.join("missing.exe")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn replace_file_swaps_in_new_build() {
        let d = temp_dir("replace");
        let (new, target) = (d.join("sakadu.new.exe"), d.join("sakadu.exe"));
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&new, b"new").unwrap();
        replace_file(&new, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!new.exists());
        // 처음 넣을 때는 자리가 비어 있다
        std::fs::remove_file(&target).unwrap();
        std::fs::write(&new, b"first").unwrap();
        replace_file(&new, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 설치 사본이 실행 중이어도 바꿔 넣는다 (실행 중인 exe는 덮어쓰거나 지울 수 없다)
    #[test]
    #[ignore]
    fn replace_file_while_target_runs() {
        let d = temp_dir("running");
        let (new, target) = (d.join("sakadu.new.exe"), d.join("sakadu.exe"));
        // 실행 중인 사본 흉내: 시스템의 ping을 복사해 몇 초 돌린다
        let ping = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32").join("PING.EXE");
        std::fs::copy(&ping, &target).unwrap();
        let mut child = std::process::Command::new(&target).args(["-n", "5", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(std::fs::OpenOptions::new().write(true).open(&target).is_err(), "running exe can't be overwritten");
        std::fs::write(&new, b"new build").unwrap();
        let r = replace_file(&new, &target);
        let _ = child.kill();
        let _ = child.wait();
        r.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new build");
        assert!(target.with_extension("old.exe").exists(), "running file moved aside");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn recognizes_old_sakadu_command() {
        assert!(is_sakadu_command("\"C:\\Users\\a\\AppData\\Local\\Programs\\Sakadu\\sakadu.exe\""));
        assert!(is_sakadu_command("\"D:\\Tools\\SAKADU.EXE\""));
        assert!(!is_sakadu_command("\"C:\\Other\\fm.exe\""));
        assert!(!is_sakadu_command("\"C:\\Tools\\not-sakadu.exe\""));
    }
}
