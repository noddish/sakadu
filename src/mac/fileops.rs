//! 파일 작업의 실행: 앱(App)이 맡긴 작업 하나를 실제 파일에 한다. 창에서는 뒷줄 하나가 맡긴 순서대로 하나씩 부르고,
//! 시험에서는 App::settle이 그 자리에서 부른다.
//! 대상은 줄 번호가 아니라 경로와 파일 번호로 받아, 실행 직전에 그 경로의 항목이 목록을 읽을 때의 그 항목인지 다시 본다
//! (맥 버전에는 아직 폴더 감시가 없어 목록이 낡았을 수 있다). 덮어쓰지 않고, 파일을 없애는 호출은 두지 않는다:
//! 지우기는 휴지통으로 보내기뿐이고, 만든 폴더를 되돌릴 때도 휴지통으로 보낸다

use sakadu::fileops::{self, Trashed, Undo};
use sakadu::util;

/// 할 일
#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    /// dir 안에 새 폴더를 만든다 (이름은 "새 폴더", 있으면 번호를 붙인다)
    NewFolder { dir: String },
    /// dir 안의 from을 to로 이름을 바꾼다. file_id는 목록에서 본 from의 파일 번호
    Rename { dir: String, from: String, to: String, file_id: u64 },
    /// dir 안의 항목들(이름, 목록에서 본 파일 번호)을 휴지통으로 보낸다
    Trash { dir: String, items: Vec<(String, u64)> },
    /// 앞서 한 작업을 되돌린다
    Undo(Undo),
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
    /// 휴지통으로 보낸 것과, 보내지 못해 그대로 둔 것(이름, 까닭)
    Trashed { moved: Vec<Trashed>, failed: Vec<(String, String)> },
    /// 되돌렸다. left는 휴지통에서 꺼내지 못해 그대로 둔 것(이름, 까닭)
    Undone { left: Vec<(String, String)> },
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

fn other(e: std::io::Error) -> Fail {
    Fail::Other(fileops::reason(&e))
}

/// 그 경로의 항목이 그 파일 번호의 항목인지 (실행 직전에 본다)
fn same_item(path: &str, file_id: u64) -> Result<(), Fail> {
    match fileops::inode(path) {
        Ok(now) if now == file_id => Ok(()),
        Ok(_) => Err(Fail::Changed),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Fail::Changed),
        Err(e) => Err(other(e)),
    }
}

/// 덮어쓰지 않는 이름 바꾸기
fn rename(src: &str, dst: &str) -> Result<(), Fail> {
    fileops::rename_excl(src, dst).map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { Fail::Exists } else { other(e) })
}

/// 작업 하나를 실행한다 (창에서는 뒷줄에서 부른다)
pub fn run(job: &Job) -> Result<Done, Fail> {
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
            same_item(&src, *file_id)?;
            rename(&src, &dst)?;
            Ok(Done::Renamed)
        }
        Job::Trash { dir, items } => {
            // 하나씩 보낸다: 하나가 안 돼도 나머지는 보내고, 안 된 것은 까닭과 함께 돌려준다
            let (mut moved, mut failed) = (Vec::new(), Vec::new());
            for (name, file_id) in items {
                let path = util::join(dir, name);
                guard(&path);
                match trash_item(&path, *file_id) {
                    Ok(trash_path) => moved.push(Trashed { name: name.clone(), file_id: *file_id, trash_path }),
                    Err(why) => failed.push((name.clone(), why)),
                }
            }
            Ok(Done::Trashed { moved, failed })
        }
        // 이름 바꾸기를 되돌린다: 지금 이름(to)의 항목이 그때 그 항목일 때만, 원래 이름이 비어 있을 때만
        Job::Undo(Undo::Rename { dir, from, to, file_id }) => {
            let (now, back) = (util::join(dir, to), util::join(dir, from));
            guard(&now);
            guard(&back);
            same_item(&now, *file_id)?;
            rename(&now, &back)?;
            Ok(Done::Undone { left: Vec::new() })
        }
        // 만든 폴더를 되돌린다: 그 폴더가 그대로이고 비어 있을 때만 휴지통으로 보낸다 (그사이 넣은 것을 함께 보내지 않는다)
        Job::Undo(Undo::NewFolder { dir, name, file_id }) => {
            let path = util::join(dir, name);
            guard(&path);
            same_item(&path, *file_id)?;
            if !fileops::dir_is_empty(&path).map_err(other)? {
                return Err(Fail::Other("폴더에 항목이 들어 있어 그대로 뒀어요".to_string()));
            }
            to_trash(&path, true).map_err(Fail::Other)?;
            Ok(Done::Undone { left: Vec::new() })
        }
        // 휴지통으로 보낸 것을 되돌린다: 하나씩 원래 자리로. 안 되는 것은 휴지통에 그대로 두고 까닭을 돌려준다
        Job::Undo(Undo::Trash { dir, items }) => {
            let mut left = Vec::new();
            for item in items {
                let back = util::join(dir, &item.name);
                guard(&back);
                if let Err(why) = put_back(&item.trash_path, &back, item.file_id) {
                    left.push((item.name.clone(), why));
                }
            }
            Ok(Done::Undone { left })
        }
    }
}

/// 항목 하나를 휴지통으로 보내고 휴지통 안의 경로를 돌려준다. 목록에서 본 그 항목일 때만, 디스크 자체가 아닐 때만
fn trash_item(path: &str, file_id: u64) -> Result<String, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("그 사이 없어졌어요".to_string()),
        Err(e) => return Err(fileops::reason(&e)),
    };
    if std::os::unix::fs::MetadataExt::ino(&meta) != file_id {
        return Err("그 사이 다른 항목으로 바뀌었어요".to_string());
    }
    if fileops::is_mount_point(path) {
        return Err("디스크는 휴지통으로 보낼 수 없어요".to_string());
    }
    to_trash(path, meta.is_dir())
}

/// 휴지통으로 보낸 항목을 원래 자리로 되돌린다: 휴지통 안의 그것이 그때 그 항목일 때만, 원래 자리가 비어 있을 때만.
/// 맥의 개인정보 보호 때문에 앱이 휴지통 안을 건드리지 못할 수 있다: 그때는 그대로 두고 그렇게 알린다
pub fn put_back(trash_path: &str, back: &str, file_id: u64) -> Result<(), String> {
    let by_hand = "휴지통에서 직접 꺼내 주세요";
    if trash_path.is_empty() {
        return Err(format!("휴지통 안의 자리를 몰라요. {by_hand}"));
    }
    match fileops::inode(trash_path) {
        Ok(now) if now == file_id => {}
        Ok(_) => return Err(format!("휴지통의 그 항목이 바뀌었어요. {by_hand}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("휴지통에 더는 없어요".to_string()),
        Err(e) => return Err(format!("{}. {by_hand}", fileops::reason(&e))),
    }
    fileops::rename_excl(trash_path, back).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("원래 자리에 같은 이름이 있어요. {by_hand}"),
        _ => format!("{}. {by_hand}", fileops::reason(&e)),
    })
}

/// 맥의 휴지통으로 보낸다 (Finder의 '휴지통으로 이동'과 같은 곳). 휴지통 안의 경로를 돌려준다 (알려 주지 않으면 빈 글자).
/// is_dir는 항목 자체가 폴더인지: 폴더를 가리키는 링크는 폴더가 아니라고 넘겨야 링크 자체가 간다
pub fn real_trash(path: &str, is_dir: bool) -> Result<String, String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    // 뒷줄에서 부르므로 여기서 만든 맥 객체는 이 안에서 정리한다
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), is_dir);
        let mut out = None;
        match NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error(&url, Some(&mut out)) {
            Ok(()) => Ok(out.and_then(|u| u.path()).map(|p| p.to_string()).unwrap_or_default()),
            Err(e) => Err(e.localizedDescription().to_string()),
        }
    })
}

#[cfg(not(test))]
fn to_trash(path: &str, is_dir: bool) -> Result<String, String> {
    real_trash(path, is_dir)
}

/// 시험의 가짜 휴지통: 임시 폴더 안의 폴더로 옮긴다 (러너의 진짜 휴지통에 시험 파일을 쌓지 않는다).
/// 같은 이름을 여러 번 보내도 겹치지 않게 번호 폴더를 하나씩 만들어 그 안에 둔다
#[cfg(test)]
fn to_trash(path: &str, _is_dir: bool) -> Result<String, String> {
    let bin = fake_trash();
    let why = |e: std::io::Error| fileops::reason(&e);
    std::fs::create_dir_all(&bin).map_err(why)?;
    let slot = fileops::make_dir_numbered(&bin, "칸").map_err(why)?;
    let dst = util::join(&util::join(&bin, &slot), util::file_name(path));
    fileops::rename_excl(path, &dst).map_err(why)?;
    Ok(dst)
}

/// 가짜 휴지통 폴더 (시험 프로세스마다 하나)
#[cfg(test)]
pub fn fake_trash() -> String {
    let temp = std::fs::canonicalize(std::env::temp_dir()).expect("임시 폴더");
    util::join(&util::normalize(&temp.to_string_lossy()), &format!("sakadu-fake-trash-{}", std::process::id()))
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

    fn id(path: &str) -> u64 {
        fileops::inode(path).unwrap()
    }

    /// 이 파일의 시험이 아닌 부분에는 파일을 없애는 호출이 없다 (맥의 '휴지통으로 보내기'만 있다)
    #[test]
    fn engine_has_no_delete_calls() {
        let code = include_str!("fileops.rs").split("#[cfg(test)]").next().unwrap();
        assert!(code.contains("trashItemAtURL"), "the scanned part is the engine");
        for banned in ["remove_file", "remove_dir", "unlink", "rmdir", "removeItem"] {
            assert!(!code.contains(banned), "{banned}");
        }
    }

    #[test]
    fn new_folder_reports_its_name_and_number() {
        let dir = temp("new");
        let Ok(Done::NewFolder { name, file_id }) = run(&Job::NewFolder { dir: dir.clone() }) else { panic!("새 폴더") };
        assert_eq!((name.as_str(), file_id), ("새 폴더", id(&util::join(&dir, "새 폴더"))));
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
        let (a, b) = (id(&p("a.txt")), id(&p("b.txt")));
        let job = |from: &str, to: &str, file_id: u64| Job::Rename { dir: dir.clone(), from: from.into(), to: to.into(), file_id };
        assert_eq!(run(&job("a.txt", "c.txt", b)), Err(Fail::Changed), "another file's number");
        assert_eq!(run(&job("없는 파일.txt", "c.txt", a)), Err(Fail::Changed));
        assert_eq!(run(&job("a.txt", "b.txt", a)), Err(Fail::Exists));
        assert_eq!((std::fs::read(p("a.txt")).unwrap(), std::fs::read(p("b.txt")).unwrap()), (b"a".to_vec(), b"b".to_vec()));
        assert!(std::fs::symlink_metadata(p("c.txt")).is_err());
        assert_eq!(run(&job("a.txt", "c.txt", a)), Ok(Done::Renamed));
        assert_eq!((std::fs::read(p("c.txt")).unwrap(), id(&p("c.txt"))), (b"a".to_vec(), a));
        // 되돌리기도 같은 규칙: 그 항목일 때만, 원래 이름이 비어 있을 때만
        let undo = |file_id: u64| Job::Undo(Undo::Rename { dir: dir.clone(), from: "a.txt".into(), to: "c.txt".into(), file_id });
        assert_eq!(run(&undo(b)), Err(Fail::Changed));
        std::fs::write(p("a.txt"), b"new a").unwrap();
        assert_eq!(run(&undo(a)), Err(Fail::Exists));
        assert_eq!((std::fs::read(p("a.txt")).unwrap(), std::fs::read(p("c.txt")).unwrap()), (b"new a".to_vec(), b"a".to_vec()));
        std::fs::rename(p("a.txt"), p("치운 것.txt")).unwrap();
        assert_eq!(run(&undo(a)), Ok(Done::Undone { left: Vec::new() }));
        assert_eq!((std::fs::read(p("a.txt")).unwrap(), id(&p("a.txt"))), (b"a".to_vec(), a));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 휴지통으로 보내기(시험의 가짜 휴지통): 목록에서 본 그 항목만 가고, 아닌 것은 까닭과 함께 남는다. 되돌리면 원래 자리로 온다
    #[test]
    fn trash_moves_only_the_listed_items_and_undo_puts_them_back() {
        let dir = temp("trash");
        let p = |n: &str| util::join(&dir, n);
        std::fs::write(p("a.txt"), b"a").unwrap();
        std::fs::write(p("b.txt"), b"b").unwrap();
        std::fs::create_dir(p("폴더")).unwrap();
        std::fs::write(util::join(&p("폴더"), "안.txt"), b"in").unwrap();
        let (a, b, d) = (id(&p("a.txt")), id(&p("b.txt")), id(&p("폴더")));
        // b는 목록을 읽은 뒤 다른 파일로 바뀌었다 (파일 번호가 다르다), '없는 것'은 그 사이 없어졌다
        let items = vec![("a.txt".to_string(), a), ("b.txt".to_string(), a), ("폴더".to_string(), d), ("없는 것".to_string(), 7)];
        let Ok(Done::Trashed { moved, failed }) = run(&Job::Trash { dir: dir.clone(), items }) else { panic!("휴지통") };
        assert_eq!(moved.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["a.txt", "폴더"]);
        assert_eq!(failed.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["b.txt", "없는 것"]);
        assert!(failed[0].1.contains("바뀌었어요") && failed[1].1.contains("없어졌어요"), "{failed:?}");
        assert_eq!((std::fs::read(p("b.txt")).unwrap(), id(&p("b.txt"))), (b"b".to_vec(), b), "the changed file stays");
        assert!(std::fs::symlink_metadata(p("a.txt")).is_err() && std::fs::symlink_metadata(p("폴더")).is_err());
        // 휴지통 안에 그대로 있다 (없어진 것이 아니다)
        assert!(moved.iter().all(|t| t.trash_path.starts_with(&fake_trash())));
        assert_eq!(std::fs::read(&moved[0].trash_path).unwrap(), b"a");
        assert_eq!(std::fs::read(util::join(&moved[1].trash_path, "안.txt")).unwrap(), b"in");
        // 그 사이 원래 자리에 같은 이름이 생긴 것은 휴지통에 그대로 두고, 나머지만 되돌린다
        std::fs::write(p("a.txt"), b"new a").unwrap();
        let Ok(Done::Undone { left }) = run(&Job::Undo(Undo::Trash { dir: dir.clone(), items: moved.clone() })) else { panic!("되돌리기") };
        assert_eq!(left.len(), 1);
        assert!(left[0].0 == "a.txt" && left[0].1.contains("같은 이름"), "{left:?}");
        assert_eq!((std::fs::read(p("a.txt")).unwrap(), std::fs::read(&moved[0].trash_path).unwrap()), (b"new a".to_vec(), b"a".to_vec()));
        assert_eq!((std::fs::read(util::join(&p("폴더"), "안.txt")).unwrap(), id(&p("폴더"))), (b"in".to_vec(), d));
        // 한 번 더 되돌리면: 폴더는 휴지통에 더는 없다
        let Ok(Done::Undone { left }) = run(&Job::Undo(Undo::Trash { dir: dir.clone(), items: moved })) else { panic!("되돌리기") };
        assert_eq!(left.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 만든 폴더 되돌리기: 비어 있으면 휴지통으로 보내고, 무엇이 들어 있으면 그대로 둔다
    #[test]
    fn undoing_a_new_folder_only_takes_an_empty_one() {
        let dir = temp("undo-new");
        let Ok(Done::NewFolder { name, file_id }) = run(&Job::NewFolder { dir: dir.clone() }) else { panic!("새 폴더") };
        let path = util::join(&dir, &name);
        let undo = Job::Undo(Undo::NewFolder { dir: dir.clone(), name: name.clone(), file_id });
        std::fs::write(util::join(&path, "넣은 파일.txt"), b"x").unwrap();
        assert!(matches!(run(&undo), Err(Fail::Other(why)) if why.contains("들어 있어")));
        assert_eq!(std::fs::read(util::join(&path, "넣은 파일.txt")).unwrap(), b"x");
        std::fs::rename(util::join(&path, "넣은 파일.txt"), util::join(&dir, "꺼낸 파일.txt")).unwrap();
        assert_eq!(run(&undo), Ok(Done::Undone { left: Vec::new() }));
        assert!(std::fs::symlink_metadata(&path).is_err());
        // 같은 이름의 다른 폴더가 생겼으면 건드리지 않는다
        std::fs::create_dir(&path).unwrap();
        assert_eq!(run(&undo), Err(Fail::Changed));
        assert!(std::fs::metadata(&path).unwrap().is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 맥의 진짜 휴지통: 파일, 속이 있는 폴더, 폴더를 가리키는 링크를 보냈다가 되돌린다.
    /// 링크를 보내면 링크만 가고 가리키던 폴더와 그 안은 그대로다
    #[test]
    fn real_trash_takes_the_item_itself_and_gives_it_back() {
        let dir = temp("real-trash");
        let p = |n: &str| util::join(&dir, n);
        std::fs::write(p("파일.txt"), b"f").unwrap();
        std::fs::create_dir(p("폴더")).unwrap();
        std::fs::write(util::join(&p("폴더"), "안.txt"), b"in").unwrap();
        std::fs::create_dir(p("진짜")).unwrap();
        std::fs::write(util::join(&p("진짜"), "지키는 파일.txt"), b"keep").unwrap();
        std::os::unix::fs::symlink(p("진짜"), p("링크")).unwrap();
        for (name, is_dir) in [("파일.txt", false), ("폴더", true), ("링크", false)] {
            let before = id(&p(name));
            let in_trash = real_trash(&p(name), is_dir).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(std::fs::symlink_metadata(p(name)).is_err(), "{name} left its folder");
            assert!(!in_trash.is_empty() && id(&in_trash) == before, "{name} is in the Trash as the same item: {in_trash}");
            assert_eq!(std::fs::read(util::join(&p("진짜"), "지키는 파일.txt")).unwrap(), b"keep", "the link's target is untouched after {name}");
            put_back(&in_trash, &p(name), before).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(id(&p(name)), before, "{name} is back");
        }
        assert_eq!(std::fs::read(p("파일.txt")).unwrap(), b"f");
        assert_eq!(std::fs::read(util::join(&p("폴더"), "안.txt")).unwrap(), b"in");
        assert!(std::fs::symlink_metadata(p("링크")).unwrap().file_type().is_symlink());
        // 없는 것을 보내면 오류만 난다
        assert!(real_trash(&p("없는 것"), false).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 시험의 안전장치: 임시 폴더 밖의 경로는 실행하기 전에 멈춘다
    #[test]
    #[should_panic(expected = "임시 폴더 밖")]
    fn tests_cannot_touch_paths_outside_the_temp_folder() {
        let _ = run(&Job::NewFolder { dir: "/Users/Shared".to_string() });
    }
}
