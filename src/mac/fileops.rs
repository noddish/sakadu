//! 파일 작업의 실행: 앱(App)이 맡긴 작업 하나를 실제 파일에 한다. 창에서는 뒷줄 하나가 맡긴 순서대로 하나씩 부르고,
//! 시험에서는 App::settle이 그 자리에서 부른다.
//! 대상은 줄 번호가 아니라 경로와 파일 번호로 받아, 실행 직전에 그 경로의 항목이 목록을 읽을 때의 그 항목인지 다시 본다
//! (맥 버전에는 아직 폴더 감시가 없어 목록이 낡았을 수 있다). 덮어쓰지 않고, 파일을 없애는 호출은 두지 않는다

use sakadu::{fileops, util};

/// 할 일
#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    /// dir 안에 새 폴더를 만든다 (이름은 "새 폴더", 있으면 번호를 붙인다)
    NewFolder { dir: String },
    /// dir 안의 from을 to로 이름을 바꾼다. file_id는 목록에서 본 from의 파일 번호
    Rename { dir: String, from: String, to: String, file_id: u64 },
}

/// 맡긴 작업: 작업 번호, 그 작업을 낸 탭, 할 일
#[derive(Debug, Clone)]
pub struct Op {
    pub seq: u64,
    pub tab: u64,
    pub job: Job,
}

/// 잘 끝난 결과
#[derive(Debug, PartialEq)]
pub enum Done {
    /// 만든 폴더의 이름과 파일 번호 (번호를 읽지 못했으면 0)
    NewFolder { name: String, file_id: u64 },
    Renamed,
}

/// 하지 못한 까닭. 어느 경우든 파일은 바뀌지 않았다
#[derive(Debug, PartialEq)]
pub enum Fail {
    /// 바꿀 이름이 이미 있다
    Exists,
    /// 그 경로의 항목이 목록에서 본 것이 아니다: 그 사이 없어졌거나 다른 것으로 바뀌었다
    Changed,
    /// 그 밖의 오류 (알림에 쓸 말)
    Other(String),
}

/// 작업 하나를 실행한다 (창에서는 뒷줄에서 부른다)
pub fn run(job: &Job) -> Result<Done, Fail> {
    let other = |e: std::io::Error| Fail::Other(fileops::reason(&e));
    match job {
        Job::NewFolder { dir } => {
            guard(dir);
            let name = fileops::make_dir_numbered(dir, fileops::NEW_FOLDER).map_err(other)?;
            // 번호를 읽지 못해도 폴더는 만들어졌다 (0이면 앱이 이름 고치기를 시작하지 않을 뿐이다)
            let file_id = fileops::inode(&util::join(dir, &name)).unwrap_or(0);
            Ok(Done::NewFolder { name, file_id })
        }
        Job::Rename { dir, from, to, file_id } => {
            let (src, dst) = (util::join(dir, from), util::join(dir, to));
            guard(&src);
            guard(&dst);
            match fileops::inode(&src) {
                Ok(now) if now == *file_id => {}
                Ok(_) => return Err(Fail::Changed),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Fail::Changed),
                Err(e) => return Err(other(e)),
            }
            fileops::rename_excl(&src, &dst).map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { Fail::Exists } else { other(e) })?;
            Ok(Done::Renamed)
        }
    }
}

/// 시험에서는 임시 폴더 밖의 경로를 건드리려 하면 바로 멈춘다 (시험이 잘못 짜여도 실제 파일에 닿지 않게)
#[cfg(test)]
fn guard(path: &str) {
    let temp = std::fs::canonicalize(std::env::temp_dir()).expect("임시 폴더");
    assert!(std::path::Path::new(path).starts_with(&temp), "시험이 임시 폴더 밖을 건드리려 함: {path}");
}

#[cfg(not(test))]
fn guard(_path: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("sakadu-mac-ops-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        util::normalize(&std::fs::canonicalize(&dir).unwrap().to_string_lossy())
    }

    /// 이 파일의 시험이 아닌 부분에는 파일을 없애는 호출이 없다
    #[test]
    fn engine_has_no_delete_calls() {
        let code = include_str!("fileops.rs").split("#[cfg(test)]").next().unwrap();
        for banned in ["remove_file", "remove_dir", "unlink", "rmdir", "removeItem"] {
            assert!(!code.contains(banned), "{banned}");
        }
    }

    #[test]
    fn new_folder_reports_its_name_and_number() {
        let dir = temp("new");
        let Ok(Done::NewFolder { name, file_id }) = run(&Job::NewFolder { dir: dir.clone() }) else { panic!("새 폴더") };
        assert_eq!((name.as_str(), file_id), ("새 폴더", fileops::inode(&util::join(&dir, "새 폴더")).unwrap()));
        assert!(matches!(run(&Job::NewFolder { dir: dir.clone() }), Ok(Done::NewFolder { name, .. }) if name == "새 폴더 2"));
        assert!(matches!(run(&Job::NewFolder { dir: util::join(&dir, "없는 곳") }), Err(Fail::Other(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 이름 바꾸기는 목록에서 본 그 항목일 때만 한다: 파일 번호가 다르거나 없어졌으면 그대로 두고, 있는 이름 위로도 가지 않는다
    #[test]
    fn rename_checks_the_item_and_never_replaces() {
        let dir = temp("rename");
        let p = |n: &str| util::join(&dir, n);
        std::fs::write(p("a.txt"), b"a").unwrap();
        std::fs::write(p("b.txt"), b"b").unwrap();
        let (a, b) = (fileops::inode(&p("a.txt")).unwrap(), fileops::inode(&p("b.txt")).unwrap());
        let job = |from: &str, to: &str, file_id: u64| Job::Rename { dir: dir.clone(), from: from.into(), to: to.into(), file_id };
        assert_eq!(run(&job("a.txt", "c.txt", b)), Err(Fail::Changed), "another file's number");
        assert_eq!(run(&job("없는 파일.txt", "c.txt", a)), Err(Fail::Changed));
        assert_eq!(run(&job("a.txt", "b.txt", a)), Err(Fail::Exists));
        assert_eq!((std::fs::read(p("a.txt")).unwrap(), std::fs::read(p("b.txt")).unwrap()), (b"a".to_vec(), b"b".to_vec()));
        assert!(std::fs::symlink_metadata(p("c.txt")).is_err());
        assert_eq!(run(&job("a.txt", "c.txt", a)), Ok(Done::Renamed));
        assert_eq!((std::fs::read(p("c.txt")).unwrap(), fileops::inode(&p("c.txt")).unwrap()), (b"a".to_vec(), a));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 시험의 안전장치: 임시 폴더 밖의 경로는 실행하기 전에 멈춘다
    #[test]
    #[should_panic(expected = "임시 폴더 밖")]
    fn tests_cannot_touch_paths_outside_the_temp_folder() {
        let _ = run(&Job::NewFolder { dir: "/Users/Shared".to_string() });
    }
}
