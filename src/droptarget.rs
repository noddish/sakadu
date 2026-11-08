//! 다른 앱이나 다른 패널에서 끌어온 항목을 받는 OLE 드롭 대상.
//! 파일 목록(CF_HDROP)은 앱이 직접 IFileOperation으로 복사·이동해 Ctrl+Z로 되돌릴 수 있게 하고,
//! 그 밖의 데이터나 바로 가기(Alt)·오른쪽 버튼 끌기(메뉴)는 커서 아래 폴더의 셸 드롭 대상에 맡긴다.
//! 기본 동작은 탐색기와 같다: 같은 드라이브(같은 공유) 안은 이동, 다른 곳으로는 복사.
//! Shift는 이동, Ctrl은 복사, Ctrl+Shift·Alt는 바로 가기(셸에 맡김).

use crate::app::DropAt;
use crate::shell::{self, FileOp};
use crate::{util, with_app};
use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, POINT, POINTL};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IDataObject};
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, MK_ALT};
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_RBUTTON, MK_SHIFT, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::{CLSID_DragDropHelper, IDropTargetHelper};
use windows::core::{Ref, Result, implement};

#[implement(IDropTarget)]
pub struct DropTarget {
    hwnd: HWND,
    state: RefCell<State>,
}

#[derive(Default)]
struct State {
    data: Option<IDataObject>,
    /// 끌어온 파일 경로 (파일이 아닌 데이터면 None)
    files: Option<Vec<String>>,
    dir: Option<String>,
    /// 사이드바 즐겨찾기 위: 놓으면 즐겨찾기에 넣는다 (파일은 그대로)
    favorites: bool,
    /// 마지막으로 정한 처리 방식: true면 앱이 직접, false면 셸 드롭 대상
    own: bool,
    target: Option<IDropTarget>,
    helper: Option<IDropTargetHelper>,
}

/// 앱이 직접 처리할지: 파일 목록이고, 바로 가기(Alt, Ctrl+Shift)나 오른쪽 버튼 끌기(셸 메뉴)가 아닐 때
fn handle_ourselves(files: &Option<Vec<String>>, keys: MODIFIERKEYS_FLAGS) -> bool {
    let link = keys.0 & MK_ALT != 0 || (keys.0 & MK_CONTROL.0 != 0 && keys.0 & MK_SHIFT.0 != 0);
    files.is_some() && !link && keys.0 & MK_RBUTTON.0 == 0
}

/// 같은 드라이브나 같은 공유 폴더에 있는가 ("C:", "\\서버\공유"를 대소문자 없이 비교).
/// 경로 글자만 본다: 끄는 동안 디스크·네트워크에 묻지 않게. 연결 드라이브(Z:)와 그 공유의 UNC 경로는 다른 곳으로 보고 복사한다
fn same_volume(a: &str, b: &str) -> bool {
    let ra = a[..util::root_len(a)].trim_end_matches('\\');
    let rb = b[..util::root_len(b)].trim_end_matches('\\');
    !ra.is_empty() && ra.eq_ignore_ascii_case(rb)
}

fn is_inside(folder: &str, path: &str) -> bool {
    let prefix = format!("{}\\", folder.trim_end_matches('\\').to_lowercase());
    path.to_lowercase().starts_with(&prefix)
}

/// 즐겨찾기에 넣을 때의 커서: 바로 가기(링크)처럼 보이게, 안 되면 복사
fn fav_effect(allowed: DROPEFFECT) -> DROPEFFECT {
    if allowed.0 & DROPEFFECT_LINK.0 != 0 {
        DROPEFFECT_LINK
    } else if allowed.0 & DROPEFFECT_COPY.0 != 0 {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    }
}

/// 직접 처리할 때의 동작 (탐색기와 같은 규칙): 수정 키가 없으면 같은 드라이브·공유 안은 이동, 다른 곳으로는 복사.
/// Shift는 이동, Ctrl은 복사. 자기 자신·하위 폴더에 놓기와 같은 폴더로 이동은 막는다
fn own_effect(files: &[String], dest: &str, keys: MODIFIERKEYS_FLAGS, allowed: DROPEFFECT) -> DROPEFFECT {
    if files.iter().any(|f| util::eq_path(f, dest) || is_inside(f, dest)) {
        return DROPEFFECT_NONE;
    }
    let (shift, ctrl) = (keys.0 & MK_SHIFT.0 != 0, keys.0 & MK_CONTROL.0 != 0);
    let want = match (shift, ctrl) {
        (true, false) => DROPEFFECT_MOVE,
        (false, true) => DROPEFFECT_COPY,
        _ if files.iter().all(|f| same_volume(f, dest)) => DROPEFFECT_MOVE,
        _ => DROPEFFECT_COPY,
    };
    let same_dir = files.iter().all(|f| util::parent(f).is_some_and(|p| util::eq_path(&p, dest)));
    // 같은 폴더에 놓기: 이동은 할 일이 없고, 복사는 Ctrl을 눌렀을 때만 ("- 복사본"을 만든다. 탐색기와 같음).
    // 살짝 끌었다 제자리에 놓는 실수로 사본이 생기지 않게 한다
    if same_dir && (want == DROPEFFECT_MOVE || keys.0 & MK_CONTROL.0 == 0) {
        return DROPEFFECT_NONE;
    }
    if allowed.0 & want.0 != 0 {
        want
    } else if allowed.0 & DROPEFFECT_COPY.0 != 0 {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    }
}

impl DropTarget {
    pub fn new(hwnd: HWND) -> DropTarget {
        // 끌고 있는 항목의 미리보기 이미지를 그려 주는 셸 도우미
        let helper = unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) }.ok();
        DropTarget { hwnd, state: RefCell::new(State { helper, ..Default::default() }) }
    }

    /// 커서 아래 대상 폴더와 처리 방식을 정하고 *effect(커서 모양)를 채운다
    fn retarget(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) {
        let found = with_app(|app| app.drop_dir_at(*pt));
        if found.is_none() {
            crate::diag::log(|| "drop: app busy, cannot resolve folder".to_string());
        }
        let (dir, favorites) = match found {
            Some(DropAt::Dir(d)) => (Some(d), false),
            Some(DropAt::Favorites) => (None, true),
            _ => (None, false),
        };
        let mut st = self.state.borrow_mut();
        st.own = handle_ourselves(&st.files, keys);
        st.favorites = favorites;
        unsafe {
            if st.own {
                if let Some(t) = st.target.take() {
                    let _ = t.DragLeave();
                }
                if dir != st.dir {
                    crate::diag::log(|| format!("drop: own target {dir:?}"));
                }
                st.dir = dir;
                *effect = match (&st.dir, &st.files) {
                    (Some(d), Some(f)) => own_effect(f, d, keys, *effect),
                    (None, Some(_)) if favorites => fav_effect(*effect),
                    _ => DROPEFFECT_NONE,
                };
                return;
            }
            if dir != st.dir || st.target.is_none() {
                if let Some(t) = st.target.take() {
                    let _ = t.DragLeave();
                }
                st.dir = dir.clone();
                if let (Some(d), Some(data)) = (dir, st.data.clone()) {
                    match shell::folder_drop_target(self.hwnd, &d) {
                        Ok(t) => {
                            let r = t.DragEnter(&data, keys, *pt, effect);
                            crate::diag::log(|| format!("drop: shell target {d} DragEnter {r:?} effect={}", (*effect).0));
                            if r.is_ok() {
                                st.target = Some(t);
                            }
                        }
                        Err(e) => crate::diag::log(|| format!("drop: no shell target for {d}: {e}")),
                    }
                }
                if st.target.is_none() {
                    *effect = DROPEFFECT_NONE;
                }
            } else if let Some(t) = &st.target {
                if t.DragOver(keys, *pt, effect).is_err() {
                    *effect = DROPEFFECT_NONE;
                }
            }
        }
    }

    fn reset(&self) {
        {
            let mut st = self.state.borrow_mut();
            st.data = None;
            st.files = None;
            st.dir = None;
            st.favorites = false;
            st.target = None;
        }
        with_app(|app| app.clear_drop_hover());
    }
}

fn point(pt: &POINTL) -> POINT {
    POINT { x: pt.x, y: pt.y }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, pdataobj: Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let data = pdataobj.ok()?.clone();
        let files = shell::hdrop_paths(&data).filter(|f| !f.is_empty());
        crate::diag::log(|| format!("drop: DragEnter at {},{} files={:?}", pt.x, pt.y, files.as_ref().map(|f| f.len())));
        {
            let mut st = self.state.borrow_mut();
            st.data = Some(data.clone());
            st.files = files;
            st.dir = None;
            st.target = None;
        }
        self.retarget(keys, pt, effect);
        if let Some(h) = &self.state.borrow().helper {
            unsafe {
                let _ = h.DragEnter(self.hwnd, &data, &point(pt), *effect);
            }
        }
        Ok(())
    }

    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.retarget(keys, pt, effect);
        if let Some(h) = &self.state.borrow().helper {
            unsafe {
                let _ = h.DragOver(&point(pt), *effect);
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        crate::diag::log(|| "drop: DragLeave".to_string());
        if let Some(t) = self.state.borrow_mut().target.take() {
            unsafe {
                let _ = t.DragLeave();
            }
        }
        if let Some(h) = &self.state.borrow().helper {
            unsafe {
                let _ = h.DragLeave();
            }
        }
        self.reset();
        Ok(())
    }

    /// 놓을 때는 버튼이 이미 떼어져 키 상태로 방식을 다시 정할 수 없으므로 마지막 DragOver의 결정을 따른다
    fn Drop(&self, pdataobj: Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let data = pdataobj.ok()?.clone();
        let (own, target, helper, dir, files, favorites) = {
            let mut st = self.state.borrow_mut();
            (st.own, st.target.take(), st.helper.clone(), st.dir.clone(), st.files.take(), st.favorites)
        };
        unsafe {
            if own {
                *effect = match (&dir, &files) {
                    (Some(d), Some(f)) => own_effect(f, d, keys, *effect),
                    (None, Some(_)) if favorites => fav_effect(*effect),
                    _ => DROPEFFECT_NONE,
                };
            }
            if let Some(h) = &helper {
                let _ = h.Drop(&data, &point(pt), *effect);
            }
            match (own, dir, files, target) {
                (true, Some(dest), Some(paths), _) => {
                    let e = *effect;
                    crate::diag::log(|| format!("drop: own {} {} item(s) -> {dest}", if e == DROPEFFECT_MOVE { "move" } else { "copy" }, paths.len()));
                    if e == DROPEFFECT_COPY {
                        shell::spawn_file_op(self.hwnd, FileOp::Copy { paths, dest });
                    } else if e == DROPEFFECT_MOVE {
                        shell::spawn_file_op(self.hwnd, FileOp::Move { paths, dest });
                        // 이미 옮겼으므로 끌어온 쪽이 원본을 지우지 않도록 NONE으로 알린다 (최적화된 이동)
                        *effect = DROPEFFECT_NONE;
                    }
                }
                // 즐겨찾기에 넣기만 하고 파일은 건드리지 않는다
                (true, None, Some(paths), _) if favorites && *effect != DROPEFFECT_NONE => {
                    crate::diag::log(|| format!("drop: add {} item(s) to favorites", paths.len()));
                    with_app(|app| app.add_favorite_paths(&paths));
                }
                (false, _, _, Some(t)) => {
                    let r = t.Drop(&data, keys, *pt, effect);
                    crate::diag::log(|| format!("drop: shell Drop {r:?} effect={}", (*effect).0));
                }
                _ => *effect = DROPEFFECT_NONE,
            }
        }
        self.reset();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: DROPEFFECT = DROPEFFECT(7);

    /// 탐색기 규칙: 같은 드라이브·공유 안은 이동, 다른 곳은 복사. Shift는 이동, Ctrl은 복사
    #[test]
    fn moves_within_a_volume_and_copies_across() {
        let none = MODIFIERKEYS_FLAGS(0);
        let f = vec!["C:\\a\\x.txt".to_string()];
        assert_eq!(own_effect(&f, "C:\\b", none, ALL), DROPEFFECT_MOVE);
        assert_eq!(own_effect(&f, "c:\\b", none, ALL), DROPEFFECT_MOVE);
        assert_eq!(own_effect(&f, "D:\\b", none, ALL), DROPEFFECT_COPY);
        assert_eq!(own_effect(&f, "\\\\nas\\share\\b", none, ALL), DROPEFFECT_COPY);
        assert_eq!(own_effect(&f, "D:\\b", MK_SHIFT, ALL), DROPEFFECT_MOVE);
        assert_eq!(own_effect(&f, "C:\\b", MK_CONTROL, ALL), DROPEFFECT_COPY);
        let n = vec!["\\\\nas\\share\\a\\x.txt".to_string()];
        assert_eq!(own_effect(&n, "\\\\NAS\\Share\\b", none, ALL), DROPEFFECT_MOVE);
        assert_eq!(own_effect(&n, "\\\\nas\\other\\b", none, ALL), DROPEFFECT_COPY);
        assert_eq!(own_effect(&n, "C:\\b", none, ALL), DROPEFFECT_COPY);
        // 다른 드라이브에서 섞여 온 파일은 복사
        let mixed = vec!["C:\\a\\x.txt".to_string(), "D:\\y.txt".to_string()];
        assert_eq!(own_effect(&mixed, "C:\\b", none, ALL), DROPEFFECT_COPY);
    }

    #[test]
    fn refuses_into_itself_or_same_folder_move() {
        let dir = vec!["C:\\a\\폴더".to_string()];
        assert_eq!(own_effect(&dir, "C:\\a\\폴더", MODIFIERKEYS_FLAGS(0), ALL), DROPEFFECT_NONE);
        assert_eq!(own_effect(&dir, "C:\\a\\폴더\\하위", MODIFIERKEYS_FLAGS(0), ALL), DROPEFFECT_NONE);
        // 이름이 앞부분만 같은 다른 폴더는 허용 (같은 드라이브라 이동)
        assert_eq!(own_effect(&dir, "C:\\a\\폴더2", MODIFIERKEYS_FLAGS(0), ALL), DROPEFFECT_MOVE);
        let f = vec!["C:\\a\\x.txt".to_string()];
        assert_eq!(own_effect(&f, "C:\\a", MK_SHIFT, ALL), DROPEFFECT_NONE);
        // 같은 폴더에 그냥 놓으면 아무 일도 없고, Ctrl을 누르고 놓아야 "- 복사본"을 만든다
        assert_eq!(own_effect(&f, "C:\\a", MODIFIERKEYS_FLAGS(0), ALL), DROPEFFECT_NONE);
        assert_eq!(own_effect(&f, "C:\\a", MK_CONTROL, ALL), DROPEFFECT_COPY);
    }

    #[test]
    fn respects_allowed_effects_and_modes() {
        let f = vec!["C:\\a\\x.txt".to_string()];
        assert_eq!(own_effect(&f, "D:\\b", MK_SHIFT, DROPEFFECT_COPY), DROPEFFECT_COPY);
        // 원본이 이동을 허락하지 않으면(읽기 전용 원본 등) 같은 드라이브여도 복사
        assert_eq!(own_effect(&f, "C:\\b", MODIFIERKEYS_FLAGS(0), DROPEFFECT_COPY), DROPEFFECT_COPY);
        // Ctrl+Shift는 바로 가기라 셸에 맡긴다
        assert!(!handle_ourselves(&Some(f.clone()), MODIFIERKEYS_FLAGS(MK_CONTROL.0 | MK_SHIFT.0)));
        assert!(!handle_ourselves(&Some(f.clone()), MODIFIERKEYS_FLAGS(MK_ALT)));
        assert!(!handle_ourselves(&Some(f.clone()), MK_RBUTTON));
        assert!(!handle_ourselves(&None, MODIFIERKEYS_FLAGS(0)));
        assert!(handle_ourselves(&Some(f), MODIFIERKEYS_FLAGS(0)));
    }
}
