//! Ctrl+Z 되돌리기. 이 앱이 실행한 파일 작업의 실제 결과를 셸 진행 알림(IFileOperationProgressSink)으로
//! 기록해 두고 반대로 실행한다. 탐색기의 되돌리기 기록은 탐색기 프로세스 안에만 있어 쓸 수 없다.

use crate::util;
use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::*;
use windows::core::{HRESULT, HSTRING, PCWSTR, Ref, Result, implement};

/// 셸이 알려 준 작업 결과 하나
#[derive(Debug, Clone, PartialEq)]
pub enum Done {
    Renamed { from: String, to: String },
    Moved { from: String, to: String },
    Copied { to: String },
    /// 휴지통으로 보냄. item은 $Recycle.Bin 안 실제 파일 경로 (복원할 때 휴지통 항목을 찾는 데 씀)
    Recycled { from: String, item: String },
}

impl Done {
    /// 작업한 원래 항목 경로 (복사는 만들어진 쪽)
    pub fn source(&self) -> &str {
        match self {
            Done::Renamed { from, .. } | Done::Moved { from, .. } | Done::Recycled { from, .. } => from,
            Done::Copied { to } => to,
        }
    }

    /// 기록 기준 경로. 폴더 안 항목까지 하나씩 알려 오면 맨 위 항목만 남기는 데 쓴다
    fn key(&self) -> &str {
        match self {
            Done::Renamed { to, .. } | Done::Moved { to, .. } | Done::Copied { to } => to,
            Done::Recycled { from, .. } => from,
        }
    }
}

/// 되돌릴 때 "작업 직후의 바로 그 항목인지" 확인하는 표식.
/// id는 볼륨·파일 번호라 같은 이름의 다른 파일로 바뀌면 달라지고,
/// tree는 내용(폴더면 안에 든 모든 항목의 이름·크기·시각)이라 새 자료가 들어오면 달라진다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stamp {
    id: (u32, u64),
    tree: u64,
}

/// path의 표식. deep이면 내용까지 본다 (복사본·새 폴더처럼 되돌릴 때 지우는 항목)
pub fn stamp(path: &str, deep: bool) -> Option<Stamp> {
    let info = unsafe {
        let h = CreateFileW(
            &HSTRING::from(path),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
        .ok()?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        let r = GetFileInformationByHandle(h, &mut info);
        let _ = CloseHandle(h);
        r.ok()?;
        info
    };
    let id = (info.dwVolumeSerialNumber, (info.nFileIndexHigh as u64) << 32 | info.nFileIndexLow as u64);
    let dir = info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    let link = info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
    let tree = if !deep {
        0
    } else if dir && !link {
        tree_hash(std::path::Path::new(path))?
    } else {
        let mut h = DefaultHasher::new();
        ((info.nFileSizeHigh as u64) << 32 | info.nFileSizeLow as u64).hash(&mut h);
        (info.ftLastWriteTime.dwHighDateTime, info.ftLastWriteTime.dwLowDateTime).hash(&mut h);
        h.finish()
    };
    Some(Stamp { id, tree })
}

/// 폴더 안 모든 항목(이름·크기·만든/고친 시각)을 이름순으로 훑은 값. 연결(정션·심볼릭 링크) 안으로는 들어가지 않는다
fn tree_hash(root: &std::path::Path) -> Option<u64> {
    let mut h = DefaultHasher::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut items: Vec<std::fs::DirEntry> = std::fs::read_dir(&dir).ok()?.collect::<std::io::Result<_>>().ok()?;
        items.sort_by_key(|e| e.file_name());
        for e in items {
            let md = e.metadata().ok()?;
            e.file_name().hash(&mut h);
            md.len().hash(&mut h);
            md.modified().ok().hash(&mut h);
            md.created().ok().hash(&mut h);
            let sub = md.is_dir() && !md.file_type().is_symlink();
            sub.hash(&mut h);
            if sub {
                stack.push(e.path());
            }
        }
        // 폴더 경계: 항목이 다른 폴더로 옮겨 가도 값이 달라지게
        u8::MAX.hash(&mut h);
    }
    Some(h.finish())
}

/// 같은 작업에서 폴더와 그 안 항목이 함께 알려 오면 맨 위 항목만 남긴다
fn top_level(done: &[Done]) -> Vec<&Done> {
    let keys: HashSet<String> = done.iter().map(|d| d.key().to_lowercase()).collect();
    done.iter()
        .filter(|d| {
            let mut p = util::parent(d.key());
            while let Some(dir) = p {
                if keys.contains(&dir.to_lowercase()) {
                    return false;
                }
                p = util::parent(&dir);
            }
            true
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Undo {
    Rename { now: String, name: String, stamp: Stamp },
    /// (지금 위치, 원래 위치, 옮긴 직후 표식)
    Move(Vec<(String, String, Stamp)>),
    /// (만들어진 복사본, 복사 직후 표식)
    Copy(Vec<(String, Stamp)>),
    /// (휴지통 항목, 원래 경로)
    Recycle(Vec<(String, String)>),
    /// 새 폴더 (만든 직후 표식)
    Create(String, Stamp),
}

/// Ctrl+Z를 실행한 결과
#[derive(Debug, Default)]
pub struct Undone {
    pub done: usize,
    /// 그사이 바뀌었거나 없어져서 건드리지 않은 항목
    pub changed: Vec<String>,
    /// 실패하거나 취소돼 처리하지 못한 항목. 다시 Ctrl+Z로 시도할 수 있게 기록으로 되돌려 놓는다
    pub retry: Option<Undo>,
}

impl Undo {
    /// 셸이 알려 준 결과로 되돌리기 기록을 만든다. 작업 직후 표식을 뜨지 못한 항목은 되돌릴 때 확인할 수 없으므로 빼 둔다
    pub fn from_done(done: &[Done], stamp: impl Fn(&str, bool) -> Option<Stamp>) -> Option<Undo> {
        let mut moves = Vec::new();
        let mut copies = Vec::new();
        let mut recycled = Vec::new();
        for d in top_level(done) {
            match d {
                Done::Renamed { from, to } => {
                    return stamp(to, false).map(|s| Undo::Rename { now: to.clone(), name: util::file_name(from).to_string(), stamp: s });
                }
                Done::Moved { from, to } => moves.extend(stamp(to, false).map(|s| (to.clone(), from.clone(), s))),
                Done::Copied { to } => copies.extend(stamp(to, true).map(|s| (to.clone(), s))),
                Done::Recycled { from, item } => recycled.push((item.clone(), from.clone())),
            }
        }
        if !moves.is_empty() {
            Some(Undo::Move(moves))
        } else if !copies.is_empty() {
            Some(Undo::Copy(copies))
        } else if !recycled.is_empty() {
            Some(Undo::Recycle(recycled))
        } else {
            None
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Undo::Rename { now, name, .. } => format!("이름 바꾸기 되돌림: ‘{}’ → ‘{name}’", util::file_name(now)),
            Undo::Move(v) => format!("이동 {}개 되돌림 (원래 자리로)", v.len()),
            Undo::Copy(v) => format!("복사 {}개 되돌림 (복사본은 휴지통으로)", v.len()),
            Undo::Recycle(v) => format!("삭제 {}개 되돌림 (휴지통에서 복원)", v.len()),
            Undo::Create(p, _) => format!("‘{}’ 만들기 되돌림 (휴지통으로)", util::file_name(p)),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Undo::Rename { .. } | Undo::Create(..) => 1,
            Undo::Move(v) => v.len(),
            Undo::Copy(v) => v.len(),
            Undo::Recycle(v) => v.len(),
        }
    }

    /// 작업 직후 그대로인 항목만 남긴 기록과, 그사이 바뀐 항목(경로)으로 나눈다.
    /// 경로만 보고 지우면 다른 앱이 넣어 둔 자료나 같은 이름으로 바뀐 파일까지 휴지통으로 가기 때문이다
    pub fn verified(&self, stamp: impl Fn(&str, bool) -> Option<Stamp>) -> (Option<Undo>, Vec<String>) {
        let mut changed = Vec::new();
        let mut same = |p: &str, deep: bool, s: &Stamp| {
            let ok = stamp(p, deep).as_ref() == Some(s);
            if !ok {
                changed.push(p.to_string());
            }
            ok
        };
        let todo = match self {
            Undo::Rename { now, stamp: s, .. } => same(now, false, s).then(|| self.clone()),
            Undo::Create(p, s) => same(p, true, s).then(|| self.clone()),
            Undo::Move(v) => {
                let keep: Vec<_> = v.iter().filter(|(now, _, s)| same(now, false, s)).cloned().collect();
                (!keep.is_empty()).then_some(Undo::Move(keep))
            }
            Undo::Copy(v) => {
                let keep: Vec<_> = v.iter().filter(|(p, s)| same(p, true, s)).cloned().collect();
                (!keep.is_empty()).then_some(Undo::Copy(keep))
            }
            // 휴지통 복원은 아무것도 지우지 않는다. 원래 자리에 같은 이름이 있으면 셸이 물어본다
            Undo::Recycle(_) => Some(self.clone()),
        };
        (todo, changed)
    }

    /// handled(처리된 항목 경로)에 없는 항목만 남긴 기록
    pub fn left_after(&self, handled: &[String]) -> Option<Undo> {
        let done = |p: &str| handled.iter().any(|h| util::eq_path(h, p));
        let left = match self {
            Undo::Rename { now, .. } | Undo::Create(now, _) => (!done(now)).then(|| self.clone()),
            Undo::Move(v) => Some(Undo::Move(v.iter().filter(|(now, ..)| !done(now)).cloned().collect())),
            Undo::Copy(v) => Some(Undo::Copy(v.iter().filter(|(p, _)| !done(p)).cloned().collect())),
            Undo::Recycle(v) => Some(Undo::Recycle(v.iter().filter(|(bin, _)| !done(bin)).cloned().collect())),
        };
        left.filter(|u| u.len() > 0)
    }
}

/// 잘라내기 목록 중 실제로 옮겨지지 않은 파일 (클립보드 순서 유지).
/// 같은 폴더라 보내지 않았거나, 취소·실패·건너뛴 파일은 다시 붙여넣을 수 있게 남긴다
pub fn not_moved(clip: &[String], done: &[Done]) -> Vec<String> {
    let moved: HashSet<String> = done.iter().filter(|d| matches!(d, Done::Moved { .. })).map(|d| d.source().to_lowercase()).collect();
    clip.iter().filter(|p| !moved.contains(&p.to_lowercase())).cloned().collect()
}

fn name_of(item: &IShellItem, kind: SIGDN) -> Option<String> {
    unsafe {
        let p = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// IFileOperation 진행 알림을 받아 결과만 모은다
#[implement(IFileOperationProgressSink)]
pub struct Sink {
    pub done: RefCell<Vec<Done>>,
}

impl Sink {
    pub fn new() -> Sink {
        Sink { done: RefCell::new(Vec::new()) }
    }

    fn record(&self, hr: HRESULT, item: Ref<IShellItem>, created: Ref<IShellItem>, make: impl FnOnce(String, String) -> Done) {
        if hr.is_err() {
            return;
        }
        let (Ok(item), Ok(created)) = (item.ok(), created.ok()) else { return };
        if let (Some(from), Some(to)) = (name_of(item, SIGDN_FILESYSPATH), name_of(created, SIGDN_FILESYSPATH)) {
            self.done.borrow_mut().push(make(from, to));
        }
    }
}

impl IFileOperationProgressSink_Impl for Sink_Impl {
    fn StartOperations(&self) -> Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, _hr: HRESULT) -> Result<()> {
        Ok(())
    }
    fn PreRenameItem(&self, _f: u32, _i: Ref<IShellItem>, _n: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostRenameItem(&self, _f: u32, item: Ref<IShellItem>, _n: &PCWSTR, hr: HRESULT, created: Ref<IShellItem>) -> Result<()> {
        self.record(hr, item, created, |from, to| Done::Renamed { from, to });
        Ok(())
    }
    fn PreMoveItem(&self, _f: u32, _i: Ref<IShellItem>, _d: Ref<IShellItem>, _n: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostMoveItem(&self, _f: u32, item: Ref<IShellItem>, _d: Ref<IShellItem>, _n: &PCWSTR, hr: HRESULT, created: Ref<IShellItem>) -> Result<()> {
        self.record(hr, item, created, |from, to| Done::Moved { from, to });
        Ok(())
    }
    fn PreCopyItem(&self, _f: u32, _i: Ref<IShellItem>, _d: Ref<IShellItem>, _n: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostCopyItem(&self, _f: u32, item: Ref<IShellItem>, _d: Ref<IShellItem>, _n: &PCWSTR, hr: HRESULT, created: Ref<IShellItem>) -> Result<()> {
        self.record(hr, item, created, |_, to| Done::Copied { to });
        Ok(())
    }
    fn PreDeleteItem(&self, _f: u32, _i: Ref<IShellItem>) -> Result<()> {
        Ok(())
    }
    fn PostDeleteItem(&self, _f: u32, item: Ref<IShellItem>, hr: HRESULT, created: Ref<IShellItem>) -> Result<()> {
        // 영구 삭제는 created가 없어 기록되지 않는다 (되돌릴 수 없음)
        if hr.is_ok() {
            if let (Ok(item), Ok(created)) = (item.ok(), created.ok()) {
                if let (Some(from), Some(bin)) = (name_of(item, SIGDN_FILESYSPATH), name_of(created, SIGDN_FILESYSPATH)) {
                    self.done.borrow_mut().push(Done::Recycled { from, item: bin });
                }
            }
        }
        Ok(())
    }
    fn PreNewItem(&self, _f: u32, _d: Ref<IShellItem>, _n: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostNewItem(&self, _f: u32, _d: Ref<IShellItem>, _n: &PCWSTR, _t: &PCWSTR, _a: u32, _hr: HRESULT, _i: Ref<IShellItem>) -> Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _total: u32, _done: u32) -> Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any_stamp(_: &str, _: bool) -> Option<Stamp> {
        Some(Stamp::default())
    }

    #[test]
    fn groups_results_into_one_undo() {
        let s = Stamp::default();
        let d = vec![Done::Moved { from: "C:\\a\\x".into(), to: "D:\\x".into() }, Done::Moved { from: "C:\\a\\y".into(), to: "D:\\y".into() }];
        assert_eq!(Undo::from_done(&d, any_stamp), Some(Undo::Move(vec![("D:\\x".into(), "C:\\a\\x".into(), s), ("D:\\y".into(), "C:\\a\\y".into(), s)])));
        let r = vec![Done::Renamed { from: "C:\\a\\old.txt".into(), to: "C:\\a\\new.txt".into() }];
        assert_eq!(Undo::from_done(&r, any_stamp), Some(Undo::Rename { now: "C:\\a\\new.txt".into(), name: "old.txt".into(), stamp: s }));
        assert_eq!(Undo::from_done(&[], any_stamp), None);
        // 폴더 안 항목까지 알려 와도 폴더 하나만 기록한다
        let nested = vec![Done::Copied { to: "D:\\f".into() }, Done::Copied { to: "D:\\f\\a.txt".into() }, Done::Copied { to: "D:\\g.txt".into() }];
        assert_eq!(Undo::from_done(&nested, any_stamp), Some(Undo::Copy(vec![("D:\\f".into(), s), ("D:\\g.txt".into(), s)])));
        // 표식을 뜨지 못한 복사본은 기록하지 않는다 (나중에 확인할 수 없음)
        assert_eq!(Undo::from_done(&nested, |_, _| None), None);
    }

    #[test]
    fn failed_items_stay_for_retry() {
        let s = Stamp::default();
        let u = Undo::Copy(vec![("D:\\a".into(), s), ("D:\\b".into(), s), ("D:\\c".into(), s)]);
        assert_eq!(u.left_after(&["d:\\A".into(), "D:\\c".into()]), Some(Undo::Copy(vec![("D:\\b".into(), s)])));
        assert_eq!(u.left_after(&["D:\\a".into(), "D:\\b".into(), "D:\\c".into()]), None);
    }

    #[test]
    fn cut_keeps_files_that_did_not_move() {
        let clip: Vec<String> = vec!["C:\\s\\a".into(), "C:\\s\\b".into(), "C:\\s\\c".into()];
        let done = vec![Done::Moved { from: "C:\\S\\A".into(), to: "D:\\a".into() }, Done::Copied { to: "D:\\b".into() }];
        assert_eq!(not_moved(&clip, &done), ["C:\\s\\b", "C:\\s\\c"]);
        assert!(not_moved(&clip[..1], &done).is_empty());
    }

    /// 회귀: 되돌리기는 작업 직후 그대로인 항목만 지운다.
    /// 새 폴더에 다른 앱이 자료를 넣었거나, 복사본이 같은 이름의 다른 파일로 바뀌었으면 건드리지 않는다
    #[test]
    fn undo_leaves_items_changed_since() {
        let base = std::env::temp_dir().join(format!("sakadu-undo-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let p = |n: &str| base.join(n).to_string_lossy().into_owned();
        std::fs::create_dir(base.join("새 폴더")).unwrap();
        std::fs::create_dir(base.join("빈 폴더")).unwrap();
        std::fs::write(base.join("copy1.txt"), b"one").unwrap();
        std::fs::write(base.join("copy2.txt"), b"two").unwrap();
        std::fs::create_dir_all(base.join("copied\\sub")).unwrap();
        std::fs::write(base.join("copied\\sub\\a.txt"), b"a").unwrap();

        let created = Undo::Create(p("새 폴더"), stamp(&p("새 폴더"), true).unwrap());
        let untouched = Undo::Create(p("빈 폴더"), stamp(&p("빈 폴더"), true).unwrap());
        let copies = Undo::Copy(["copy1.txt", "copy2.txt", "copied"].iter().map(|n| (p(n), stamp(&p(n), true).unwrap())).collect());

        // 다른 앱이 새 폴더에 자료를 넣고, 복사본 하나를 같은 이름의 다른 파일로 바꾸고, 복사한 폴더 깊은 곳에 파일을 더한다
        std::fs::write(base.join("새 폴더\\받은 자료.txt"), b"keep me").unwrap();
        std::fs::remove_file(base.join("copy2.txt")).unwrap();
        std::fs::write(base.join("copy2.txt"), b"different").unwrap();
        std::fs::write(base.join("copied\\sub\\new.txt"), b"new").unwrap();

        let (todo, changed) = created.verified(stamp);
        assert_eq!(todo, None);
        assert_eq!(changed, [p("새 폴더")]);
        let (todo, changed) = untouched.verified(stamp);
        assert_eq!(todo, Some(untouched.clone()));
        assert!(changed.is_empty());
        let (todo, changed) = copies.verified(stamp);
        assert_eq!(todo.map(|u| u.len()), Some(1));
        assert_eq!(changed, [p("copy2.txt"), p("copied")]);
        // 없어진 항목도 건드리지 않는다
        std::fs::remove_dir(base.join("빈 폴더")).unwrap();
        assert_eq!(untouched.verified(stamp).0, None);

        let _ = std::fs::remove_dir_all(&base);
    }
}
