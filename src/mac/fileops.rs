//! 파일 작업의 실행: 앱(App)이 맡긴 작업 하나를 실제 파일에 한다. 창에서는 뒷줄 하나가 맡긴 순서대로 하나씩 부르고,
//! 시험에서는 App::settle이 그 자리에서 부른다.
//! 대상은 줄 번호가 아니라 경로와 파일 번호로 받아, 실행 직전에 그 경로의 항목이 목록을 읽을 때의 그 항목인지 다시 본다
//! (폴더 감시의 알림은 조금 늦게 오고, 감시하지 못하는 폴더도 있어 목록이 낡았을 수 있다). 덮어쓰지 않고, 파일을 없애는 호출은 두지 않는다:
//! 지우기는 휴지통으로 보내기뿐이고, 만든 폴더나 복사본을 되돌릴 때도, 하다 만 복사를 치울 때도 휴지통으로 보낸다.
//! 다른 디스크로 옮기기도 그렇다: 복사하고, 복사본을 원본과 견줘 본 뒤, 원본을 휴지통으로 보낸다

use sakadu::fileops::{self, Crossed, Trashed, Undo};
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
    /// src(전체 경로)를 폴더 dir 안으로 복사한다. 같은 이름이 있으면 덮어쓰지 않고 번호를 붙인다.
    /// file_id는 목록에서 본 src의 파일 번호 (클립보드로 받아 모르면 0: 그때는 그 경로에 지금 있는 것을 복사한다)
    Copy { src: String, dir: String, file_id: u64 },
    /// src(전체 경로)를 폴더 dir 안으로 옮긴다. 같은 이름이 있으면 덮어쓰지 않고 번호를 붙인다. 다른 디스크면 복사한 뒤
    /// 원본을 휴지통으로 보낸다. file_id는 Copy와 같다. or_copy면 다른 디스크일 때 옮기는 대신 복사만 한다
    /// (끌어다 놓기: Finder처럼 원본이 그대로 남는다)
    Move { src: String, dir: String, file_id: u64, or_copy: bool },
    /// 앞서 한 작업을 되돌린다
    Undo(Undo),
    /// 압축 파일 archive 안의 항목들(전체 경로)을 새 임시 폴더에 꺼낸다 (압축 파일은 그대로다). then은 꺼낸 뒤 할 일
    Extract { archive: String, items: Vec<String>, then: Then },
}

/// 압축 파일 안에서 꺼낸 뒤 할 일
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Then {
    /// macOS '열기'로 연다
    Open,
    /// 클립보드에 담는다 (복사한 것으로)
    Copy,
    /// 끌어 낸다 (아직 누르고 있으면 다음 움직임에서 맥의 끌기를 시작한다)
    Drag,
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
    /// 되돌렸다. left는 되돌리지 못해 그대로 둔 것(이름, 까닭): 휴지통에서 꺼내지 못한 항목, 휴지통으로 보내지 못한 복사본.
    /// 옮기기를 되돌릴 때만 이름 대신 원래 전체 경로다
    Undone { left: Vec<(String, String)> },
    /// 복사했다: 복사본의 이름과 파일 번호. renamed는 같은 이름이 있어 번호를 붙였는지
    Copied { name: String, file_id: u64, renamed: bool },
    /// 옮겼다: 옮긴 곳에서의 이름과 파일 번호(옮겨도 그대로다). renamed는 Copied와 같다
    Moved { name: String, file_id: u64, renamed: bool },
    /// 다른 디스크로 옮겼다: 복사본의 이름과 파일 번호(복사본은 다른 파일이다), renamed는 Copied와 같다.
    /// trashed는 휴지통으로 간 원본. 원본을 휴지통으로 보내지 못했으면 Err(까닭): 그때 복사본은 남고 원본도 그대로다
    Crossed { name: String, file_id: u64, renamed: bool, trashed: Result<Trashed, String> },
    /// 꺼냈다: 꺼낸 것들의 전체 경로 (맡긴 항목 차례대로)
    Extracted { paths: Vec<String> },
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
        Job::Copy { src, dir, file_id } => {
            guard(src);
            guard(dir);
            copy_item(src, dir, *file_id)
        }
        Job::Move { src, dir, file_id, or_copy } => {
            guard(src);
            guard(dir);
            move_item(src, dir, *file_id, *or_copy)
        }
        // 압축 파일은 읽기만 하고, 꺼낸 것은 임시 폴더 안의 새 폴더에만 쓴다 (crate::archive::extract)
        Job::Extract { archive, items, .. } => {
            guard(archive);
            crate::archive::extract(archive, items).map(|paths| Done::Extracted { paths }).map_err(Fail::Other)
        }
        // 옮기기를 되돌린다: 옮긴 곳의 그것이 그때 그 항목일 때만, 원래 자리가 비어 있을 때만 하나씩 되돌린다.
        // 되돌리지 못한 것은 원래 전체 경로로 돌려준다 (서로 다른 폴더에서 온 같은 이름의 항목을 가릴 수 있게)
        Job::Undo(Undo::Move { to, items }) => {
            let mut left = Vec::new();
            for (orig, now, file_id) in items {
                let cur = util::join(to, now);
                guard(&cur);
                guard(orig);
                if let Err(fail) = same_item(&cur, *file_id).and_then(|()| rename(&cur, orig)) {
                    let why = match fail {
                        Fail::Exists => "원래 자리에 같은 이름이 있어요".to_string(),
                        Fail::Changed => "그 사이 없어졌거나 다른 것으로 바뀌었어요".to_string(),
                        Fail::Other(msg) => msg,
                    };
                    left.push((orig.clone(), why));
                }
            }
            Ok(Done::Undone { left })
        }
        // 다른 디스크로 옮긴 것을 되돌린다: 원본을 휴지통에서 원래 자리로 꺼내고, 꺼낸 것만 그 복사본을 휴지통으로 보낸다
        // (원본이 돌아오지 못하면 복사본이 남은 하나다). 되돌리지 못한 것은 옮기기처럼 원래 전체 경로로 돌려준다
        Job::Undo(Undo::Across { to, items }) => {
            let mut left = Vec::new();
            for Crossed { orig, trashed, name, file_id } in items {
                let copy = util::join(to, name);
                guard(orig);
                guard(&copy);
                if let Err(why) = put_back(&trashed.trash_path, orig, trashed.file_id) {
                    left.push((orig.clone(), why));
                } else if let Err(why) = trash_item(&copy, *file_id) {
                    left.push((orig.clone(), format!("원본은 되돌렸지만 복사본이 남았어요 · {why}")));
                }
            }
            Ok(Done::Undone { left })
        }
        // 한 번에 한 일의 기록들을 함께 되돌린다 (끌어다 놓은 것의 옮긴 것과 복사한 것). 되돌리지 못한 것은 모아서 돌려준다
        Job::Undo(Undo::Group(list)) => {
            let mut left = Vec::new();
            for undo in list {
                match run(&Job::Undo(undo.clone())) {
                    Ok(Done::Undone { left: more }) => left.extend(more),
                    Ok(_) => {}
                    Err(fail) => {
                        let why = match fail {
                            Fail::Exists => "같은 이름이 이미 있어요".to_string(),
                            Fail::Changed => "그 사이 없어졌거나 다른 것으로 바뀌었어요".to_string(),
                            Fail::Other(msg) => msg,
                        };
                        left.push((undo.label(), why));
                    }
                }
            }
            Ok(Done::Undone { left })
        }
        // 복사를 되돌린다: 복사본이 그때 만든 그것일 때만 하나씩 휴지통으로 보낸다
        Job::Undo(Undo::Copy { dir, items }) => {
            let mut left = Vec::new();
            for (name, file_id) in items {
                let path = util::join(dir, name);
                guard(&path);
                if let Err(why) = trash_item(&path, *file_id) {
                    left.push((name.clone(), why));
                }
            }
            Ok(Done::Undone { left })
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

/// dir이 폴더 folder 자신이거나 그 안인지 (링크와 대소문자를 풀어 실제 경로로 본다). 받는 폴더를 읽지 못하면 아니라고 본다
/// (그때는 복사가 제 오류로 실패한다)
fn inside(dir: &str, folder: &str) -> bool {
    match (std::fs::canonicalize(dir), std::fs::canonicalize(folder)) {
        (Ok(d), Ok(f)) => d.starts_with(&f),
        _ => false,
    }
}

/// 항목 하나를 폴더 dir 안으로 복사한다. 받는 쪽에 있는 것은 무엇도 덮어쓰지 않는다: 임시 이름으로 복사한 뒤,
/// 비어 있는 이름(그 이름, 없으면 번호를 붙인 이름)으로 덮어쓰지 않는 이름 바꾸기를 한다.
/// 링크는 링크 자체를 복사하고, 확장 속성(다운로드한 파일의 격리 표시 등)은 맥이 함께 복사한다
fn copy_item(src: &str, dir: &str, file_id: u64) -> Result<Done, Fail> {
    let meta = match std::fs::symlink_metadata(src) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Fail::Changed),
        Err(e) => return Err(other(e)),
    };
    if file_id != 0 && std::os::unix::fs::MetadataExt::ino(&meta) != file_id {
        return Err(Fail::Changed);
    }
    let is_dir = meta.is_dir();
    // 폴더를 자기 자신이나 자기 안으로 복사하면 끝없이 자기를 복사하게 된다
    if is_dir && inside(dir, src) {
        return Err(Fail::Other("폴더를 자기 안에 넣을 수 없어요".to_string()));
    }
    let temp = stage_copy(src, dir, is_dir)?;
    let (name, file_id, renamed) = place_copy(&temp, dir, util::file_name(src), is_dir)?;
    Ok(Done::Copied { name, file_id, renamed })
}

/// src를 폴더 dir 안의 임시 이름으로 복사하고 그 경로를 돌려준다. 같은 임시 이름이 남아 있으면(앞서 하다 만 것)
/// 건드리지 않고 번호를 붙인다. 하다 만 것은 휴지통으로 보낸다
fn stage_copy(src: &str, dir: &str, is_dir: bool) -> Result<String, Fail> {
    let name = util::file_name(src);
    for n in 1..1000 {
        let candidate = util::join(dir, &fileops::staging_name(name, n));
        if std::fs::symlink_metadata(&candidate).is_ok() {
            continue;
        }
        match real_copy(src, &candidate, is_dir) {
            Ok(()) => return Ok(candidate),
            Err(None) => {}
            Err(Some(why)) => {
                // 하다 만 것이 남았으면 휴지통으로 보낸다 (복사를 시작하기 직전에 없던 이름이라 우리가 만든 것이다)
                if std::fs::symlink_metadata(&candidate).is_ok() {
                    let _ = to_trash(&candidate, is_dir);
                }
                return Err(Fail::Other(why));
            }
        }
    }
    Err(Fail::Other("임시 이름을 정하지 못했어요".to_string()))
}

/// 임시 이름의 복사본 temp를 비어 있는 이름(name, 없으면 번호를 붙인 이름)으로 덮어쓰지 않고 바꾼다.
/// (그 이름, 파일 번호, 번호를 붙였는지)를 돌려준다. 하지 못하면 temp를 휴지통으로 보낸다
fn place_copy(temp: &str, dir: &str, name: &str, is_dir: bool) -> Result<(String, u64, bool), Fail> {
    for n in 1..10_000 {
        let final_name = fileops::numbered_name(name, n, is_dir);
        let dst = util::join(dir, &final_name);
        match fileops::rename_excl(temp, &dst) {
            Ok(()) => return Ok((final_name, fileops::inode(&dst).unwrap_or(0), n > 1)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                let _ = to_trash(temp, is_dir);
                return Err(other(e));
            }
        }
    }
    let _ = to_trash(temp, is_dir);
    Err(Fail::Exists)
}

/// 다른 디스크로 옮긴다: 임시 이름으로 복사하고, 복사본을 원본과 견줘 본 뒤(크기, 폴더는 속까지), 제 이름을 붙이고,
/// 원본(파일 번호 id)을 휴지통으로 보낸다. 견줘 본 것이 다르면 복사본을 휴지통으로 보내고 원본은 그대로 둔다.
/// 원본을 휴지통으로 보내지 못하면(읽기만 되는 디스크 따위) 복사본은 남기고 그 까닭을 함께 돌려준다
fn move_across(src: &str, dir: &str, id: u64, is_dir: bool) -> Result<Done, Fail> {
    let temp = stage_copy(src, dir, is_dir)?;
    if let Err(why) = same_tree(src, &temp) {
        let _ = to_trash(&temp, is_dir);
        return Err(Fail::Other(format!("복사본이 원본과 달라 옮기지 않았어요 · {why}")));
    }
    let name = util::file_name(src);
    let (made, file_id, renamed) = place_copy(&temp, dir, name, is_dir)?;
    let trashed = trash_item(src, id).map(|trash_path| Trashed { name: name.to_string(), file_id: id, trash_path });
    Ok(Done::Crossed { name: made, file_id, renamed, trashed })
}

/// 복사본 copy가 원본 src와 같은 모양인지: 같은 종류(파일, 폴더, 링크)이고, 파일은 크기가 같고, 폴더는 원본의 항목이 모두
/// 같은 모양으로 있다 (복사본 쪽에 더 있는 것은 따지지 않는다: 그 사이 Finder가 .DS_Store를 만들 수 있다).
/// 이름은 경로로 찾는다 (디스크에 따라 한글 이름을 나뉜 꼴로 둘 수 있다). 다르면 Err(어디가 다른지)
fn same_tree(src: &str, copy: &str) -> Result<(), String> {
    let mut todo = vec![(src.to_string(), copy.to_string())];
    while let Some((a, b)) = todo.pop() {
        let name = || util::file_name(&a).to_string();
        let (ma, mb) = match (std::fs::symlink_metadata(&a), std::fs::symlink_metadata(&b)) {
            (Ok(ma), Ok(mb)) => (ma, mb),
            (Err(e), _) => return Err(format!("{} · {}", name(), fileops::reason(&e))),
            (_, Err(_)) => return Err(format!("{}이(가) 복사되지 않았어요", name())),
        };
        let (ta, tb) = (ma.file_type(), mb.file_type());
        if ta.is_dir() != tb.is_dir() || ta.is_symlink() != tb.is_symlink() {
            return Err(format!("{}의 종류가 달라요", name()));
        }
        if ta.is_file() && ma.len() != mb.len() {
            return Err(format!("{}의 크기가 달라요", name()));
        }
        if ta.is_dir() {
            for entry in std::fs::read_dir(&a).map_err(|e| format!("{} · {}", name(), fileops::reason(&e)))? {
                let entry = entry.map_err(|e| format!("{} · {}", name(), fileops::reason(&e)))?;
                let child = entry.file_name().to_string_lossy().into_owned();
                todo.push((util::join(&a, &child), util::join(&b, &child)));
            }
        }
    }
    Ok(())
}

/// 두 경로가 같은 폴더인지 (링크와 대소문자를 풀어 실제 경로로 본다. 읽지 못하면 글자 그대로 비교한다)
fn same_dir(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// 항목 하나를 폴더 dir 안으로 옮긴다: 같은 디스크 안에서는 덮어쓰지 않는 이름 바꾸기 한 번이다. 같은 이름이 있으면 번호를 붙인다.
/// 옮겨도 같은 파일이라 파일 번호와 확장 속성은 그대로다.
/// 다른 디스크인지는 미리 묻지 않고 이름 바꾸기가 그렇게 실패하는 것으로 안다: 그때 or_copy면 복사만 하고(원본은 그대로 둔다),
/// 아니면 복사한 뒤 원본을 휴지통으로 보낸다 (move_across)
fn move_item(src: &str, dir: &str, file_id: u64, or_copy: bool) -> Result<Done, Fail> {
    let meta = match std::fs::symlink_metadata(src) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Fail::Changed),
        Err(e) => return Err(other(e)),
    };
    let id = std::os::unix::fs::MetadataExt::ino(&meta);
    if file_id != 0 && id != file_id {
        return Err(Fail::Changed);
    }
    let is_dir = meta.is_dir();
    let stay = |why: &str| Err(Fail::Other(why.to_string()));
    let Some(parent) = util::parent(src) else { return stay("이 항목은 옮길 수 없어요") };
    if same_dir(&parent, dir) {
        return stay("이미 이 폴더에 있어요");
    }
    if is_dir && inside(dir, src) {
        return stay("폴더를 자기 안에 넣을 수 없어요");
    }
    if fileops::is_mount_point(src) {
        return stay("디스크는 옮길 수 없어요");
    }
    let name = util::file_name(src);
    for n in 1..10_000 {
        let final_name = fileops::numbered_name(name, n, is_dir);
        match fileops::rename_excl(src, &util::join(dir, &final_name)) {
            Ok(()) => return Ok(Done::Moved { name: final_name, file_id: id, renamed: n > 1 }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices && or_copy => return copy_item(src, dir, file_id),
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => return move_across(src, dir, id, is_dir),
            Err(e) => return Err(other(e)),
        }
    }
    Err(Fail::Exists)
}

/// 맥의 복사(Finder가 쓰는 것과 같은 호출): 폴더는 속까지, 확장 속성과 권한도 함께, 링크는 링크 자체를.
/// 받는 이름이 이미 있으면 Err(None), 그 밖의 오류는 맥이 준 글과 함께 Err(Some)
pub fn real_copy(src: &str, dst: &str, is_dir: bool) -> Result<(), Option<String>> {
    use objc2_foundation::{NSFileManager, NSFileWriteFileExistsError, NSString, NSURL};
    objc2::rc::autoreleasepool(|_| {
        let url = |p: &str| NSURL::fileURLWithPath_isDirectory(&NSString::from_str(p), is_dir);
        NSFileManager::defaultManager()
            .copyItemAtURL_toURL_error(&url(src), &url(dst))
            .map_err(|e| (e.code() != NSFileWriteFileExistsError).then(|| e.localizedDescription().to_string()))
    })
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
/// 같은 이름을 여러 번 보내도 겹치지 않게 번호 폴더를 하나씩 만들어 그 안에 둔다.
/// 진짜 휴지통처럼 디스크마다 따로 둔다: 시험용으로 허락한 다른 디스크의 것은 그 디스크의 가짜 휴지통으로 간다
#[cfg(test)]
fn to_trash(path: &str, _is_dir: bool) -> Result<String, String> {
    if TRASH_FAILS.get() {
        return Err("시험: 휴지통으로 보내지 못했어요".to_string());
    }
    let far = other_volume().filter(|o| std::path::Path::new(path).starts_with(o));
    let bin = far.map_or_else(fake_trash, |o| util::join(&o, &format!("sakadu-fake-trash-{}", std::process::id())));
    let why = |e: std::io::Error| fileops::reason(&e);
    std::fs::create_dir_all(&bin).map_err(why)?;
    let slot = fileops::make_dir_numbered(&bin, "칸").map_err(why)?;
    let dst = util::join(&util::join(&bin, &slot), util::file_name(path));
    fileops::rename_excl(path, &dst).map_err(why)?;
    Ok(dst)
}

#[cfg(test)]
thread_local! {
    /// 시험: 켜 두면 이 줄에서 휴지통으로 보내기가 실패한다 (원본을 휴지통으로 보내지 못한 경우를 본다. 시험마다 제 줄에서 돈다)
    pub static TRASH_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 가짜 휴지통 폴더 (시험 프로세스마다 하나)
#[cfg(test)]
pub fn fake_trash() -> String {
    let temp = std::fs::canonicalize(std::env::temp_dir()).expect("임시 폴더");
    util::join(&util::normalize(&temp.to_string_lossy()), &format!("sakadu-fake-trash-{}", std::process::id()))
}

/// 시험에서는 임시 폴더 밖의 경로를 건드리려 하면 바로 멈춘다 (시험이 잘못 짜여도 실제 파일에 닿지 않게).
/// 다른 디스크와 주고받는 시험을 위해 폴더 하나를 더 허락할 수 있다 (CI가 SAKADU_TEST_OTHER_VOLUME으로 알려 준다)
#[cfg(test)]
fn guard(path: &str) {
    let temp = std::fs::canonicalize(std::env::temp_dir()).expect("임시 폴더");
    let path = std::path::Path::new(path);
    assert!(path.starts_with(&temp) || other_volume().is_some_and(|other| path.starts_with(other)), "시험이 임시 폴더 밖을 건드리려 함: {}", path.display());
}

/// 시험용으로 허락한, 임시 폴더와 다른 디스크에 있는 폴더 (없으면 None: 그런 시험은 건너뛴다)
#[cfg(test)]
pub fn other_volume() -> Option<String> {
    let dir = std::fs::canonicalize(std::env::var_os("SAKADU_TEST_OTHER_VOLUME")?).ok()?;
    Some(util::normalize(&dir.to_string_lossy()))
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

    /// 그 경로의 확장 속성 값 (맥의 xattr 명령으로 읽는다. 없으면 None)
    fn xattr(path: &str, key: &str) -> Option<String> {
        let out = std::process::Command::new("xattr").args(["-p", key, path]).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn set_xattr(path: &str, key: &str, value: &str) {
        let ok = std::process::Command::new("xattr").args(["-w", key, value, path]).status().is_ok_and(|s| s.success());
        assert!(ok, "xattr -w {key} {path}");
    }

    fn names(dir: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    /// 복사: 받는 쪽의 것을 덮어쓰지 않고 번호를 붙인다. 내용과 확장 속성(격리 표시 포함)이 그대로 오고, 임시 이름은 남지 않는다
    #[test]
    fn copy_keeps_contents_and_attributes_and_never_replaces() {
        let dir = temp("copy");
        let (from, to) = (util::join(&dir, "원본"), util::join(&dir, "받는 곳"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        let src = util::join(&from, "보고서.docx");
        std::fs::write(&src, b"report").unwrap();
        set_xattr(&src, "com.apple.quarantine", "0081;00000000;Safari;");
        set_xattr(&src, "com.example.note", "사카두");
        let copy = |file_id: u64| run(&Job::Copy { src: src.clone(), dir: to.clone(), file_id });
        assert_eq!(copy(id(&src)), Ok(Done::Copied { name: "보고서.docx".into(), file_id: id(&util::join(&to, "보고서.docx")), renamed: false }));
        let made = util::join(&to, "보고서.docx");
        assert_eq!(std::fs::read(&made).unwrap(), b"report");
        assert_ne!(id(&made), id(&src), "a copy is its own file");
        assert_eq!(xattr(&made, "com.apple.quarantine").as_deref(), Some("0081;00000000;Safari;"), "the quarantine mark comes along");
        assert_eq!(xattr(&made, "com.example.note").as_deref(), Some("사카두"));
        // 같은 이름이 있으면 번호를 붙인다: 먼저 있던 것은 그대로다
        std::fs::write(&made, b"already here").unwrap();
        assert!(matches!(copy(0), Ok(Done::Copied { name, renamed: true, .. }) if name == "보고서 2.docx"));
        assert!(matches!(copy(0), Ok(Done::Copied { name, renamed: true, .. }) if name == "보고서 3.docx"));
        assert_eq!(std::fs::read(&made).unwrap(), b"already here");
        assert_eq!(std::fs::read(util::join(&to, "보고서 2.docx")).unwrap(), b"report");
        assert_eq!(names(&to), ["보고서 2.docx", "보고서 3.docx", "보고서.docx"], "no temporary name is left");
        // 목록에서 본 것이 아니면(파일 번호가 다르면) 복사하지 않는다. 없어진 것도 마찬가지
        assert_eq!(copy(id(&made)), Err(Fail::Changed));
        assert_eq!(run(&Job::Copy { src: util::join(&from, "없는 것"), dir: to.clone(), file_id: 0 }), Err(Fail::Changed));
        // 받는 폴더가 없으면 오류만 나고 아무것도 생기지 않는다
        assert!(matches!(run(&Job::Copy { src: src.clone(), dir: util::join(&dir, "없는 곳"), file_id: 0 }), Err(Fail::Other(_))));
        assert_eq!(names(&dir), ["받는 곳", "원본"]);
        // 원본은 그대로다
        assert_eq!((std::fs::read(&src).unwrap(), names(&from)), (b"report".to_vec(), vec!["보고서.docx".to_string()]));
        // 길이 한도에 닿는 이름도 복사된다 (임시 이름이 한도를 넘지 않는다). 이름 끝의 공백도 그대로 간다
        for name in ["a".repeat(fileops::NAME_MAX), format!("{}.txt", "가".repeat(83)), "끝에 공백 ".to_string()] {
            let long = util::join(&from, &name);
            std::fs::write(&long, b"long").unwrap();
            assert!(matches!(run(&Job::Copy { src: long.clone(), dir: to.clone(), file_id: 0 }), Ok(Done::Copied { name: made, renamed: false, .. }) if made == name), "{} bytes", name.len());
            assert_eq!(std::fs::read(util::join(&to, &name)).unwrap(), b"long");
            // 한 번 더: 번호를 붙인 이름도 길이 한도 안에 들어 복사된다
            let Ok(Done::Copied { name: second, renamed: true, .. }) = run(&Job::Copy { src: long, dir: to.clone(), file_id: 0 }) else { panic!("{name}") };
            assert!(second != name && second.len() <= fileops::NAME_MAX, "{second}");
            assert_eq!(std::fs::read(util::join(&to, &second)).unwrap(), b"long");
        }
        assert!(!names(&to).iter().any(|n| n.contains("sakadu-copying")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 폴더는 속까지 복사하고, 링크는 가리키는 것이 아니라 링크 자체를 복사한다. 폴더를 자기 안으로는 복사하지 않는다
    #[test]
    fn copy_takes_folders_whole_and_links_as_links() {
        let dir = temp("copy-tree");
        let p = |n: &str| util::join(&dir, n);
        std::fs::create_dir_all(util::join(&p("자료"), "안쪽")).unwrap();
        std::fs::write(util::join(&util::join(&p("자료"), "안쪽"), "깊은 파일.txt"), b"deep").unwrap();
        std::fs::write(util::join(&p("자료"), "메모.txt"), b"memo").unwrap();
        std::os::unix::fs::symlink(p("자료"), util::join(&p("자료"), "자기를 가리키는 링크")).unwrap();
        std::os::unix::fs::symlink(p("자료"), p("링크")).unwrap();
        std::fs::create_dir(p("받는 곳")).unwrap();
        // 폴더: 속까지. 그 안의 링크도 링크로 온다 (가리키는 폴더를 따라 들어가지 않는다)
        assert!(matches!(run(&Job::Copy { src: p("자료"), dir: p("받는 곳"), file_id: 0 }), Ok(Done::Copied { name, renamed: false, .. }) if name == "자료"));
        let made = util::join(&p("받는 곳"), "자료");
        assert_eq!(std::fs::read(util::join(&util::join(&made, "안쪽"), "깊은 파일.txt")).unwrap(), b"deep");
        assert_eq!(std::fs::read(util::join(&made, "메모.txt")).unwrap(), b"memo");
        assert!(std::fs::symlink_metadata(util::join(&made, "자기를 가리키는 링크")).unwrap().file_type().is_symlink());
        // 링크: 링크 자체가 복사된다
        assert!(matches!(run(&Job::Copy { src: p("링크"), dir: p("받는 곳"), file_id: 0 }), Ok(Done::Copied { name, .. }) if name == "링크"));
        let link = util::join(&p("받는 곳"), "링크");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(&link).unwrap().to_string_lossy(), p("자료"));
        // 같은 폴더 안으로 복제: 번호가 붙는다
        assert!(matches!(run(&Job::Copy { src: p("자료"), dir: dir.clone(), file_id: 0 }), Ok(Done::Copied { name, renamed: true, .. }) if name == "자료 2"));
        // 폴더를 자기 자신이나 자기 안으로는 복사하지 않는다. 링크를 거쳐 들어가도 마찬가지
        for into in [p("자료"), util::join(&p("자료"), "안쪽"), util::join(&p("링크"), "안쪽")] {
            assert!(matches!(run(&Job::Copy { src: p("자료"), dir: into.clone(), file_id: 0 }), Err(Fail::Other(why)) if why.contains("자기 안")), "{into}");
        }
        assert_eq!(names(&p("자료")), ["메모.txt", "안쪽", "자기를 가리키는 링크"], "nothing was added to the source");
        // 복사 되돌리기: 그때 만든 복사본만 휴지통으로 간다. 그 이름의 다른 것으로 바뀌었으면 그대로 둔다
        let twin = p("자료 2");
        let undo = |file_id: u64| run(&Job::Undo(Undo::Copy { dir: dir.clone(), items: vec![("자료 2".to_string(), file_id)] }));
        assert!(matches!(undo(id(&p("자료"))), Ok(Done::Undone { left }) if left.len() == 1));
        assert!(std::fs::metadata(&twin).unwrap().is_dir());
        assert_eq!(undo(id(&twin)), Ok(Done::Undone { left: Vec::new() }));
        assert!(std::fs::symlink_metadata(&twin).is_err());
        assert_eq!(std::fs::read(util::join(&p("자료"), "메모.txt")).unwrap(), b"memo", "the original is untouched");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 옮기기: 같은 파일이 그대로 간다 (파일 번호와 확장 속성). 받는 쪽의 것을 덮어쓰지 않고 번호를 붙이고,
    /// 같은 폴더로, 자기 안으로는 옮기지 않는다. 되돌리면 원래 자리로 오되, 그 자리에 다른 것이 생겼으면 그대로 둔다
    #[test]
    fn move_keeps_the_same_file_and_never_replaces() {
        let dir = temp("move");
        let p = |n: &str| util::join(&dir, n);
        let (from, to) = (p("원본"), p("받는 곳"));
        std::fs::create_dir_all(util::join(&from, "자료")).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(util::join(&util::join(&from, "자료"), "안.txt"), b"in").unwrap();
        let src = util::join(&from, "보고서.docx");
        std::fs::write(&src, b"report").unwrap();
        set_xattr(&src, "com.example.note", "사카두");
        let file = id(&src);
        let job = |src: &str, dir: &str, file_id: u64| run(&Job::Move { src: src.to_string(), dir: dir.to_string(), file_id, or_copy: false });
        // 목록에서 본 것이 아니면 옮기지 않는다
        assert_eq!(job(&src, &to, id(&from)), Err(Fail::Changed));
        // 같은 폴더로, 자기 자신 안으로는 옮기지 않는다
        assert!(matches!(job(&src, &from, file), Err(Fail::Other(why)) if why.contains("이미 이 폴더")));
        let folder = util::join(&from, "자료");
        assert!(matches!(job(&folder, &folder, 0), Err(Fail::Other(why)) if why.contains("자기 안")));
        assert_eq!(names(&from), ["보고서.docx", "자료"]);
        // 옮긴다: 같은 파일이 그대로 간다
        assert_eq!(job(&src, &to, file), Ok(Done::Moved { name: "보고서.docx".into(), file_id: file, renamed: false }));
        let moved = util::join(&to, "보고서.docx");
        assert_eq!((id(&moved), std::fs::read(&moved).unwrap(), xattr(&moved, "com.example.note")), (file, b"report".to_vec(), Some("사카두".to_string())));
        assert!(std::fs::symlink_metadata(&src).is_err());
        // 폴더는 속까지 그대로. 받는 쪽에 같은 이름이 있으면 번호를 붙이고, 먼저 있던 것은 그대로다
        std::fs::create_dir(util::join(&to, "자료")).unwrap();
        std::fs::write(util::join(&util::join(&to, "자료"), "먼저 있던 것.txt"), b"old").unwrap();
        let folder_id = id(&folder);
        assert_eq!(job(&folder, &to, 0), Ok(Done::Moved { name: "자료 2".into(), file_id: folder_id, renamed: true }));
        assert_eq!(std::fs::read(util::join(&util::join(&to, "자료 2"), "안.txt")).unwrap(), b"in");
        assert_eq!(names(&util::join(&to, "자료")), ["먼저 있던 것.txt"]);
        assert!(names(&from).is_empty());
        // 되돌리기: 원래 자리에 다른 것이 생긴 것은 그대로 두고, 나머지만 되돌린다
        std::fs::write(&src, b"newcomer").unwrap();
        let items = vec![(src.clone(), "보고서.docx".to_string(), file), (folder.clone(), "자료 2".to_string(), folder_id)];
        let Ok(Done::Undone { left }) = run(&Job::Undo(Undo::Move { to: to.clone(), items: items.clone() })) else { panic!("되돌리기") };
        assert_eq!(left.len(), 1);
        assert!(left[0].0 == src && left[0].1.contains("같은 이름"), "{left:?}");
        assert_eq!((std::fs::read(&src).unwrap(), std::fs::read(&moved).unwrap()), (b"newcomer".to_vec(), b"report".to_vec()));
        assert_eq!((id(&folder), std::fs::read(util::join(&folder, "안.txt")).unwrap()), (folder_id, b"in".to_vec()));
        // 한 번 더 되돌리면: 폴더는 옮긴 곳에 더는 없다
        let Ok(Done::Undone { left }) = run(&Job::Undo(Undo::Move { to: to.clone(), items })) else { panic!("되돌리기") };
        assert_eq!(left.len(), 2);
        // 한 번의 놓기가 옮긴 것과 복사한 것으로 갈렸을 때: 묶은 기록 하나로 둘 다 되돌린다
        let (a, b) = (util::join(&from, "옮길 것.txt"), util::join(&from, "복사할 것.txt"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let Ok(Done::Moved { name: moved_name, file_id: moved_id, .. }) = job(&a, &to, 0) else { panic!("옮기기") };
        let Ok(Done::Copied { name: copied_name, file_id: copied_id, .. }) = run(&Job::Copy { src: b.clone(), dir: to.clone(), file_id: 0 }) else { panic!("복사") };
        let group = Undo::Group(vec![Undo::Copy { dir: to.clone(), items: vec![(copied_name.clone(), copied_id)] }, Undo::Move { to: to.clone(), items: vec![(a.clone(), moved_name, moved_id)] }]);
        assert_eq!(run(&Job::Undo(group)), Ok(Done::Undone { left: Vec::new() }));
        assert_eq!((std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()), (b"a".to_vec(), b"b".to_vec()));
        assert!(std::fs::symlink_metadata(util::join(&to, &copied_name)).is_err() && std::fs::symlink_metadata(util::join(&to, "옮길 것.txt")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 다른 디스크로: 옮기기는 복사한 뒤 원본을 휴지통으로 보내고(되돌리면 원본이 같은 파일로 돌아오고 복사본은 휴지통으로),
    /// 끌어다 놓기(or_copy)는 복사만 한다 (원본은 그대로 남는다). 원본을 휴지통으로 보낼 수 없으면 복사본을 남기고 까닭을 돌려준다.
    /// 임시 폴더와 다른 디스크의 폴더를 CI가 알려 줄 때만 돈다 (디스크가 하나뿐이면 볼 것이 없다)
    #[test]
    fn another_volume_is_moved_by_copy_then_trash() {
        use std::os::unix::fs::MetadataExt;
        let Some(other) = other_volume() else { return };
        let dir = temp("volumes");
        let far = util::join(&other, &format!("sakadu-other-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&far);
        std::fs::create_dir_all(&far).unwrap();
        // 같은 디스크였다면 이 시험은 볼 것이 없다
        if std::fs::metadata(&dir).unwrap().dev() == std::fs::metadata(&far).unwrap().dev() {
            std::fs::remove_dir_all(&far).unwrap();
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let src = util::join(&dir, "보고서.docx");
        std::fs::write(&src, b"report").unwrap();
        set_xattr(&src, "com.example.note", "사카두");
        let file = id(&src);
        let move_to = |src: &str, or_copy: bool| run(&Job::Move { src: src.to_string(), dir: far.clone(), file_id: 0, or_copy });
        // 끌어다 놓기: 복사만 된다. 원본은 그대로다
        assert!(matches!(move_to(&src, true), Ok(Done::Copied { name, renamed: false, .. }) if name == "보고서.docx"));
        assert_eq!((std::fs::read(&src).unwrap(), std::fs::read(util::join(&far, "보고서.docx")).unwrap()), (b"report".to_vec(), b"report".to_vec()));
        // 옮기기: 같은 이름이 있어 번호가 붙은 복사본이 생기고, 원본은 같은 파일 그대로 휴지통으로 간다
        let Ok(Done::Crossed { name, file_id, renamed: true, trashed: Ok(t) }) = move_to(&src, false) else { panic!("다른 디스크로 옮기기") };
        assert_eq!(name, "보고서 2.docx");
        let made = util::join(&far, &name);
        assert_eq!((id(&made), std::fs::read(&made).unwrap(), xattr(&made, "com.example.note")), (file_id, b"report".to_vec(), Some("사카두".to_string())));
        assert!(std::fs::symlink_metadata(&src).is_err(), "the original left its folder");
        assert_eq!((t.name.as_str(), t.file_id, id(&t.trash_path)), ("보고서.docx", file, file), "the original is in the Trash");
        // 폴더도 속까지 옮겨진다
        let folder = util::join(&dir, "자료");
        std::fs::create_dir_all(util::join(&folder, "안쪽")).unwrap();
        std::fs::write(util::join(&util::join(&folder, "안쪽"), "깊은 파일.txt"), b"deep").unwrap();
        let Ok(Done::Crossed { name: folder_name, file_id: folder_copy, renamed: false, trashed: Ok(folder_trashed) }) = move_to(&folder, false) else { panic!("폴더") };
        assert_eq!(std::fs::read(util::join(&util::join(&util::join(&far, &folder_name), "안쪽"), "깊은 파일.txt")).unwrap(), b"deep");
        assert!(std::fs::symlink_metadata(&folder).is_err());
        assert!(!names(&far).iter().any(|n| n.contains("sakadu-copying")), "no temporary name is left");
        // 되돌리기: 원본이 같은 파일로 돌아오고, 복사본은 그 디스크의 휴지통으로 간다
        let items = vec![
            Crossed { orig: src.clone(), trashed: t, name: name.clone(), file_id },
            Crossed { orig: folder.clone(), trashed: folder_trashed, name: folder_name, file_id: folder_copy },
        ];
        assert_eq!(run(&Job::Undo(Undo::Across { to: far.clone(), items })), Ok(Done::Undone { left: Vec::new() }));
        assert_eq!((id(&src), std::fs::read(&src).unwrap()), (file, b"report".to_vec()));
        assert_eq!(std::fs::read(util::join(&util::join(&folder, "안쪽"), "깊은 파일.txt")).unwrap(), b"deep");
        assert_eq!(names(&far), ["보고서.docx"], "only the plain copy from the drop is left");
        std::fs::remove_dir_all(&far).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 다른 디스크로 옮기기의 속 (디스크가 하나여도 볼 수 있게 바로 부른다): 복사본을 견줘 본 뒤 원본을 휴지통으로 보낸다.
    /// 원본을 휴지통으로 보내지 못하면 복사본을 남기고 까닭을 돌려준다 (원본도 그대로다)
    #[test]
    fn moving_across_copies_then_trashes_and_keeps_the_copy_if_the_trash_fails() {
        let dir = temp("across");
        let (from, to) = (util::join(&dir, "원래 곳"), util::join(&dir, "받는 곳"));
        std::fs::create_dir_all(util::join(&from, "자료")).unwrap();
        std::fs::create_dir(&to).unwrap();
        std::fs::write(util::join(&util::join(&from, "자료"), "안.txt"), b"in").unwrap();
        std::fs::write(util::join(&to, "자료"), b"same name").unwrap();
        let folder = util::join(&from, "자료");
        let folder_id = id(&folder);
        let Ok(Done::Crossed { name, file_id, renamed: true, trashed: Ok(t) }) = move_across(&folder, &to, folder_id, true) else { panic!("옮기기") };
        assert_eq!(name, "자료 2");
        assert_eq!((id(&util::join(&to, &name)), std::fs::read(util::join(&util::join(&to, &name), "안.txt")).unwrap()), (file_id, b"in".to_vec()));
        assert_ne!(file_id, folder_id, "the copy is its own folder");
        assert!(std::fs::symlink_metadata(&folder).is_err() && t.trash_path.starts_with(&fake_trash()) && id(&t.trash_path) == folder_id);
        assert_eq!(std::fs::read(util::join(&to, "자료")).unwrap(), b"same name", "what was there stays");
        // 되돌리면 원본이 같은 폴더로 돌아오고 복사본은 휴지통으로 간다
        let undo = Undo::Across { to: to.clone(), items: vec![Crossed { orig: folder.clone(), trashed: t, name: name.clone(), file_id }] };
        assert_eq!(run(&Job::Undo(undo)), Ok(Done::Undone { left: Vec::new() }));
        assert_eq!(id(&folder), folder_id);
        assert_eq!(names(&to), ["자료"]);
        // 원본을 휴지통으로 보내지 못하면: 복사본은 남고 원본도 그대로, 까닭을 함께 돌려준다
        let memo = util::join(&from, "메모.txt");
        std::fs::write(&memo, b"memo").unwrap();
        TRASH_FAILS.set(true);
        let result = move_across(&memo, &to, id(&memo), false);
        TRASH_FAILS.set(false);
        let Ok(Done::Crossed { name, renamed: false, trashed: Err(why), .. }) = result else { panic!("{result:?}") };
        assert!(why.contains("휴지통"), "{why}");
        assert_eq!((std::fs::read(&memo).unwrap(), std::fs::read(util::join(&to, &name)).unwrap()), (b"memo".to_vec(), b"memo".to_vec()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 다른 디스크로 옮긴 것 되돌리기 (한 디스크 안에서 그 기록을 손으로 만들어 본다): 원본을 휴지통에서 꺼낸 것만 복사본을
    /// 휴지통으로 보낸다. 원본을 꺼내지 못하면 복사본은 그대로 두고, 원본은 돌아왔는데 복사본이 그 사이 바뀌었으면 그것도 그대로 둔다
    #[test]
    fn undoing_a_move_across_disks_puts_the_original_back_first() {
        let dir = temp("undo-across");
        let (from, to) = (util::join(&dir, "원래 곳"), util::join(&dir, "받는 곳"));
        std::fs::create_dir(&from).unwrap();
        std::fs::create_dir(&to).unwrap();
        let make = |name: &str| {
            let orig = util::join(&from, name);
            std::fs::write(&orig, name.as_bytes()).unwrap();
            let orig_id = id(&orig);
            let copy = util::join(&to, name);
            std::fs::write(&copy, name.as_bytes()).unwrap();
            let trash_path = to_trash(&orig, false).unwrap();
            Crossed { orig, trashed: Trashed { name: name.into(), file_id: orig_id, trash_path }, name: name.into(), file_id: id(&copy) }
        };
        let (a, b, c) = (make("a.txt"), make("b.txt"), make("c.txt"));
        // b는 휴지통에서 이미 비웠다. c의 복사본은 그 사이 다른 파일로 바뀌었다
        std::fs::remove_file(&b.trashed.trash_path).unwrap();
        let c_copy = util::join(&to, "c.txt");
        std::fs::write(util::join(&to, "c.new"), b"new").unwrap();
        std::fs::rename(util::join(&to, "c.new"), &c_copy).unwrap();
        let Ok(Done::Undone { left }) = run(&Job::Undo(Undo::Across { to: to.clone(), items: vec![a.clone(), b.clone(), c.clone()] })) else { panic!("되돌리기") };
        assert_eq!(left.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(), [b.orig.as_str(), c.orig.as_str()]);
        assert!(left[1].1.contains("복사본이 남았어요"), "{left:?}");
        // a: 원본이 같은 파일로 돌아오고 복사본은 휴지통으로
        assert_eq!(id(&a.orig), a.trashed.file_id);
        assert!(std::fs::symlink_metadata(util::join(&to, "a.txt")).is_err());
        // b: 원본을 꺼내지 못해 복사본이 남은 하나다
        assert!(std::fs::symlink_metadata(&b.orig).is_err());
        assert_eq!(std::fs::read(util::join(&to, "b.txt")).unwrap(), b"b.txt");
        // c: 원본은 돌아왔고, 바뀐 복사본은 그대로
        assert_eq!((std::fs::read(&c.orig).unwrap(), std::fs::read(&c_copy).unwrap()), (b"c.txt".to_vec(), b"new".to_vec()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 다른 디스크로 옮기기 전에 복사본을 견줘 보는 것: 종류와 크기가 같고 폴더는 속까지 다 있어야 같다. 복사본에 더 있는 것은 괜찮다
    #[test]
    fn copy_check_compares_kinds_and_sizes_through_folders() {
        let dir = temp("same-tree");
        let (src, copy) = (util::join(&dir, "원본"), util::join(&dir, "복사본"));
        std::fs::create_dir_all(util::join(&src, "안쪽")).unwrap();
        std::fs::write(util::join(&util::join(&src, "안쪽"), "a.txt"), b"abc").unwrap();
        std::os::unix::fs::symlink("안쪽", util::join(&src, "링크")).unwrap();
        real_copy(&src, &copy, true).unwrap();
        assert_eq!(same_tree(&src, &copy), Ok(()));
        std::fs::write(util::join(&copy, ".DS_Store"), b"x").unwrap();
        assert_eq!(same_tree(&src, &copy), Ok(()), "something extra in the copy is fine");
        let deep = util::join(&util::join(&copy, "안쪽"), "a.txt");
        std::fs::write(&deep, b"ab").unwrap();
        assert!(same_tree(&src, &copy).is_err_and(|why| why.contains("a.txt") && why.contains("크기")));
        std::fs::write(&deep, b"abc").unwrap();
        std::fs::remove_file(util::join(&copy, "링크")).unwrap();
        assert!(same_tree(&src, &copy).is_err_and(|why| why.contains("링크") && why.contains("복사되지")));
        std::fs::create_dir(util::join(&copy, "링크")).unwrap();
        assert!(same_tree(&src, &copy).is_err_and(|why| why.contains("링크") && why.contains("종류")));
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
