//! Ctrl+Z 되돌리기. 이 앱이 실행한 파일 작업의 실제 결과를 셸 진행 알림(IFileOperationProgressSink)으로
//! 기록해 두고 반대로 실행한다. 탐색기의 되돌리기 기록은 탐색기 프로세스 안에만 있어 쓸 수 없다.

use crate::util;
use std::cell::RefCell;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::*;
use windows::core::{HRESULT, PCWSTR, Ref, Result, implement};

/// 셸이 알려 준 작업 결과 하나
#[derive(Debug, Clone, PartialEq)]
pub enum Done {
    Renamed { from: String, to: String },
    Moved { from: String, to: String },
    Copied { to: String },
    /// 휴지통으로 보냄. item은 $Recycle.Bin 안 실제 파일 경로 (복원할 때 휴지통 항목을 찾는 데 씀)
    Recycled { from: String, item: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Undo {
    Rename { now: String, name: String },
    /// (지금 위치, 원래 위치)
    Move(Vec<(String, String)>),
    /// 만들어진 복사본
    Copy(Vec<String>),
    /// (휴지통 항목, 원래 경로)
    Recycle(Vec<(String, String)>),
    Create(String),
}

impl Undo {
    pub fn from_done(done: &[Done]) -> Option<Undo> {
        let mut moves = Vec::new();
        let mut copies = Vec::new();
        let mut recycled = Vec::new();
        for d in done {
            match d {
                Done::Renamed { from, to } => return Some(Undo::Rename { now: to.clone(), name: util::file_name(from).to_string() }),
                Done::Moved { from, to } => moves.push((to.clone(), from.clone())),
                Done::Copied { to } => copies.push(to.clone()),
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
            Undo::Rename { now, name } => format!("이름 바꾸기 되돌림: ‘{}’ → ‘{name}’", util::file_name(now)),
            Undo::Move(v) => format!("이동 {}개 되돌림 (원래 자리로)", v.len()),
            Undo::Copy(v) => format!("복사 {}개 되돌림 (복사본은 휴지통으로)", v.len()),
            Undo::Recycle(v) => format!("삭제 {}개 되돌림 (휴지통에서 복원)", v.len()),
            Undo::Create(p) => format!("‘{}’ 만들기 되돌림 (휴지통으로)", util::file_name(p)),
        }
    }
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

    #[test]
    fn groups_results_into_one_undo() {
        let d = vec![Done::Moved { from: "C:\\a\\x".into(), to: "D:\\x".into() }, Done::Moved { from: "C:\\a\\y".into(), to: "D:\\y".into() }];
        assert_eq!(Undo::from_done(&d), Some(Undo::Move(vec![("D:\\x".into(), "C:\\a\\x".into()), ("D:\\y".into(), "C:\\a\\y".into())])));
        let r = vec![Done::Renamed { from: "C:\\a\\old.txt".into(), to: "C:\\a\\new.txt".into() }];
        assert_eq!(Undo::from_done(&r), Some(Undo::Rename { now: "C:\\a\\new.txt".into(), name: "old.txt".into() }));
        assert_eq!(Undo::from_done(&[]), None);
    }
}
