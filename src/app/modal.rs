//! 모달 작업 실행 (App 빌림이 풀린 상태), 메뉴, 입력 상자 색

use super::*;

// ---------- 모달 작업 실행 (App 빌림이 풀린 상태) ----------

pub fn execute(hwnd: HWND, act: Deferred) {
    match act {
        Deferred::Close => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        Deferred::HideToTray => unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
            // 숨어 있는 동안 쓰지 않는 메모리를 돌려준다. 다시 띄우면 필요한 만큼 돌아온다
            let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
        },
        Deferred::Open { pane, paths, dir } => {
            for path in paths {
                open_one(hwnd, pane, &path, dir.as_deref());
            }
        }
        Deferred::ConfirmRename { tab, path, old, new } => {
            let text = format!("확장자를 바꾸면 파일을 열지 못할 수도 있어요.\n\n‘{old}’ → ‘{new}’\n\n이대로 바꿀까요?");
            let yes = unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("이름 바꾸기"), MB_YESNO | MB_ICONWARNING) } == IDYES;
            with_app(|a| if yes { a.start_rename(tab, path, new) } else { a.reopen_rename(tab, &path, &old, &new) });
        }
        Deferred::ConfirmOpen { pane, paths } => {
            let text = format!("파일 {}개를 한꺼번에 열까요?", paths.len());
            if unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Sakadu"), MB_YESNO | MB_ICONQUESTION) } == IDYES {
                for path in paths {
                    let dir = util::parent(&path);
                    open_one(hwnd, pane, &path, dir.as_deref());
                }
            }
        }
        Deferred::DragOut(paths) => {
            let _ = shell::drag_out(hwnd, &paths);
        }
        Deferred::Verb { paths, verb } => {
            let _ = shell::invoke_verb(hwnd, &paths, verb);
        }
        Deferred::Paste { pane, dest } => match shell::clipboard_files() {
            Some((paths, false)) if !paths.is_empty() => shell::spawn_file_op(hwnd, FileOp::Copy { paths, dest }),
            Some((clip, true)) if !clip.is_empty() => {
                // 클립보드는 옮기기가 끝난 뒤에 정리한다 (같은 폴더·취소·실패면 다시 붙여넣을 수 있게)
                let seq = shell::clipboard_seq();
                let paths: Vec<String> = clip.iter().filter(|p| !util::parent(p).is_some_and(|d| util::eq_path(&d, &dest))).cloned().collect();
                let busy = with_app(|a| a.moving.busy(&paths)).unwrap_or(false);
                if busy {
                    with_app(|a| a.flash(pane, "잘라낸 파일을 옮기는 중이에요"));
                } else if paths.is_empty() {
                    with_app(|a| a.flash(pane, "이미 이 폴더에 있어요 · 다른 폴더에 붙여넣으세요"));
                } else if let Some(id) = with_app(|a| a.moving.begin(&paths)) {
                    shell::spawn_cut_paste(hwnd, paths, dest, seq, clip, id);
                }
            }
            _ => {
                with_app(|a| a.flash(pane, "클립보드에 붙여넣을 파일이 없어요"));
            }
        },
        Deferred::Menu { pane, paths, dir, pt } => show_menu(hwnd, MenuRequest { pane, paths, dir, pt }),
        Deferred::PrepareMenu { paths, dir } => prepare_menu(hwnd, paths, dir),
        Deferred::FavMenu { row, pt } => with_menu(|| fav_ui::show_fav_menu(hwnd, row, pt)),
        Deferred::FavSortMenu { pt } => with_menu(|| fav_ui::show_fav_sort_menu(hwnd, pt)),
        Deferred::SideMenu { side, pt } => with_menu(|| sidebar::show_side_menu(hwnd, side, pt)),
        Deferred::PinMenu { index, pt } => with_menu(|| fav_ui::show_pin_menu(hwnd, index, pt)),
        Deferred::RailMenu { index, pt } => with_menu(|| workspace_ui::show_rail_menu(hwnd, index, pt)),
        Deferred::PreviewSync => preview::sync_handler(),
        Deferred::Settings { ctl, index, pt } => settings_ui::run_deferred(hwnd, ctl, index, pt),
        Deferred::PalMenu { pt } => palette::show_pal_menu(hwnd, pt),
        Deferred::MoreMenu { pt } => with_menu(|| sidebar::show_more_menu(hwnd, pt)),
        Deferred::StartMenu { pane, item, loc, pt } => with_menu(|| start::show_start_menu(hwnd, pane, item, loc, pt)),
        Deferred::WorkspaceMenu { pt } => with_menu(|| workspace_ui::show_workspace_menu(hwnd, pt)),
        Deferred::NetFix { tab, share, ask } => {
            let r = crate::listing::net_connect(hwnd, &share, ask);
            with_app(|a| a.after_net_fix(tab, r));
        }
    }
}

/// 팝업 메뉴를 띄우는 동안 표시해 둔다: 메뉴 위로 간 마우스 때문에 떠 있는 패널이 닫히지 않게
/// 파일 하나 열기. 폴더를 가리키는 바로 가기(.lnk)는 Sakadu 안에서 그 폴더로 간다
pub(super) fn open_one(hwnd: HWND, pane: usize, path: &str, dir: Option<&str>) {
    if path.to_lowercase().ends_with(".lnk") {
        if let Some(target) = shell::shortcut_target_dir(path) {
            with_app(|a| a.navigate(pane, Location::Dir(target), None));
            return;
        }
    }
    shell::open_path(hwnd, path, dir);
}

pub(super) fn with_menu(f: impl FnOnce()) {
    with_app(|a| a.menu_open = true);
    f();
    with_app(|a| a.menu_open = false);
}

pub(super) fn remove_search_folder(hwnd: HWND, folders: &crate::folders::Folders, root: &str) {
    let text = format!("이 폴더를 검색에서 뺄까요?\n{root}");
    if unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Sakadu"), MB_YESNO | MB_ICONQUESTION) } != IDYES {
        return;
    }
    let msg = match folders.remove(root) {
        Ok(()) => format!("검색에서 뺐어요: {root}"),
        Err(e) => format!("검색 폴더 목록을 저장하지 못했어요: {e}"),
    };
    with_app(|a| a.flash(a.active, msg));
}

pub(super) struct MenuRequest {
    pane: usize,
    paths: Vec<String>,
    dir: Option<String>,
    pt: POINT,
}

pub(super) struct PreparedMenu {
    paths: Vec<String>,
    dir: Option<String>,
    extended: bool,
    menu: shell::ShellMenu,
    single_dir: bool,
}

thread_local! {
    /// 오른쪽 버튼을 누를 때 미리 만든 셸 메뉴
    static PREPARED_MENU: RefCell<Option<PreparedMenu>> = const { RefCell::new(None) };
    static PREPARING_MENU: Cell<bool> = const { Cell::new(false) };
    /// 준비 도중(셸 확장이 메시지를 돌리는 사이) 버튼을 떼면, 준비가 끝나는 즉시 띄운다
    static SHOW_AFTER_PREPARE: RefCell<Option<MenuRequest>> = const { RefCell::new(None) };
}

pub(super) fn build_menu(hwnd: HWND, paths: &[String], dir: Option<&str>, extended: bool) -> Option<(shell::ShellMenu, bool)> {
    let cm = if paths.is_empty() { shell::background_menu(hwnd, dir?) } else { shell::ui_object(hwnd, paths) }.ok()?;
    let single_dir = paths.len() == 1 && shell::is_dir(&paths[0]);
    let custom: Vec<(u32, &str)> = if single_dir {
        vec![(ID_OPEN_TAB, "새 탭에서 열기"), (ID_ADD_FAV, "즐겨찾기에 추가")]
    } else if paths.len() == 1 {
        vec![(ID_ADD_FAV, "즐겨찾기에 추가")]
    } else if paths.is_empty() {
        vec![(ID_ADD_FAV, "이 폴더를 즐겨찾기에 추가")]
    } else {
        Vec::new()
    };
    let menu = shell::ShellMenu::new(cm, extended, &custom).ok()?;
    Some((menu, single_dir))
}

pub(super) fn prepare_menu(hwnd: HWND, paths: Vec<String>, dir: Option<String>) {
    let extended = key_down(VK_SHIFT);
    let t = std::time::Instant::now();
    PREPARING_MENU.set(true);
    let built = build_menu(hwnd, &paths, dir.as_deref(), extended);
    PREPARING_MENU.set(false);
    crate::diag::log(|| format!("menu: prepared in {:?}", t.elapsed()));
    PREPARED_MENU.with(|m| *m.borrow_mut() = built.map(|(menu, single_dir)| PreparedMenu { paths, dir, extended, menu, single_dir }));
    if let Some(req) = SHOW_AFTER_PREPARE.with(|s| s.borrow_mut().take()) {
        show_menu(hwnd, req);
    }
}

pub(super) fn show_menu(hwnd: HWND, req: MenuRequest) {
    if PREPARING_MENU.get() {
        SHOW_AFTER_PREPARE.with(|s| *s.borrow_mut() = Some(req));
        return;
    }
    let extended = key_down(VK_SHIFT);
    let prepared = PREPARED_MENU
        .with(|m| m.borrow_mut().take())
        .filter(|p| p.paths == req.paths && p.dir == req.dir && p.extended == extended);
    let reused = prepared.is_some();
    let (menu, single_dir) = match prepared {
        Some(p) => (p.menu, p.single_dir),
        None => match build_menu(hwnd, &req.paths, req.dir.as_deref(), extended) {
            Some(b) => b,
            None => return,
        },
    };
    crate::diag::log(|| format!("menu: show (prepared={reused})"));
    let MenuRequest { pane, paths, dir, pt } = req;
    let Some(picked) = menu.track(hwnd, pt) else { return };
    if picked.id == ID_OPEN_TAB {
        with_app(|a| a.new_tab(pane, Some(Location::Dir(paths[0].clone())), false));
        return;
    }
    if picked.id == ID_ADD_FAV {
        if let Some(target) = paths.first().cloned().or(dir) {
            with_app(|a| a.add_favorite_path(&target, None));
        }
        return;
    }
    // 폴더 열기·이름 바꾸기는 탐색기 창 대신 이 앱 안에서 처리
    match picked.verb.as_str() {
        "rename" => {
            with_app(|a| a.begin_rename(pane));
        }
        "open" | "explore" if single_dir => {
            with_app(|a| a.navigate(pane, Location::Dir(util::normalize(&paths[0])), None));
        }
        "opennewwindow" if single_dir => {
            with_app(|a| a.new_tab(pane, Some(Location::Dir(util::normalize(&paths[0]))), true));
        }
        _ => {
            let _ = menu.invoke(hwnd, picked.id, Some(pt), dir.as_deref());
        }
    }
}

/// 입력 상자 색. 입력 상자는 App이 다른 메시지를 처리하는 도중에도 다시 그려지며 색을 물으므로
/// App 밖에 두고 창 프로시저에서 바로 답한다
#[derive(Clone, Copy)]
pub(super) struct EditColors {
    pub(super) text: COLORREF,
    pub(super) field: COLORREF,
    pub(super) field_brush: HBRUSH,
    pub(super) pane: COLORREF,
    pub(super) pane_brush: HBRUSH,
    /// 필터 칩 바탕 (옅은 강조 색)
    pub(super) chip: COLORREF,
    pub(super) chip_brush: HBRUSH,
    pub(super) rename: HWND,
    pub(super) preview_host: HWND,
    pub(super) filter: HWND,
    pub(super) cmd: HWND,
}

thread_local! {
    /// 처리한 키에 뒤따르는 WM_CHAR를 삼켜 삑 소리나 제어 문자가 들어가지 않게 한다
    pub(super) static SUPPRESS_CHAR: Cell<bool> = const { Cell::new(false) };
    pub(super) static EDIT_COLORS: Cell<Option<EditColors>> = const { Cell::new(None) };
}

/// WM_CTLCOLOREDIT 응답
pub fn edit_ctl_color(wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let c = EDIT_COLORS.get()?;
    let hdc = HDC(wp.0 as *mut c_void);
    let target = HWND(lp.0 as *mut c_void);
    let (bg, brush) = if target == c.rename || target == c.preview_host || target == c.cmd {
        (c.pane, c.pane_brush)
    } else if target == c.filter {
        (c.chip, c.chip_brush)
    } else {
        (c.field, c.field_brush)
    };
    unsafe {
        SetTextColor(hdc, c.text);
        SetBkColor(hdc, bg);
    }
    Some(LRESULT(brush.0 as isize))
}

pub unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, id: usize, _data: usize) -> LRESULT {
    unsafe {
        match msg {
            WM_GETDLGCODE => return LRESULT(DefSubclassProc(hwnd, msg, wp, lp).0 | DLGC_WANTALLKEYS as isize),
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                SUPPRESS_CHAR.set(false);
                let repeat = lp.0 & (1 << 30) != 0;
                let handled = with_app(|app| {
                    app.key_repeat = repeat;
                    app.on_edit_key(id as i32, VIRTUAL_KEY(wp.0 as u16))
                })
                .unwrap_or(false);
                // 글자를 만드는 키(Enter·Esc·Tab·Backspace·Space·Ctrl 조합)를 처리했을 때만 뒤따르는 WM_CHAR를 삼킨다.
                // 미뤄 둔 확인 창(이름·열기)의 메시지 루프가 그 글자를 먼저 꺼내 가므로, 창을 띄우기 전에 정해 둔다
                if handled {
                    let vk = VIRTUAL_KEY(wp.0 as u16);
                    SUPPRESS_CHAR.set(matches!(vk, VK_RETURN | VK_ESCAPE | VK_TAB | VK_BACK | VK_SPACE) || key_down(VK_CONTROL));
                }
                if let Ok(parent) = GetParent(hwnd) {
                    crate::run_deferred(parent);
                }
                if handled {
                    return LRESULT(0);
                }
            }
            // 이름 상자에 붙여넣을 때는 첫 줄에서 쓸 수 없는 문자를 뺀다
            WM_PASTE if id as i32 == ID_RENAME && with_app(|app| matches!(app.edit_mode, EditMode::Rename(_))).unwrap_or(false) => {
                if let Some(text) = shell::clipboard_text(hwnd) {
                    let clean: String = text.lines().next().unwrap_or("").chars().filter(|c| !blocked_name_char(*c)).collect();
                    let q = HSTRING::from(clean);
                    SendMessageW(hwnd, EM_REPLACESEL, Some(WPARAM(1)), Some(LPARAM(q.as_ptr() as isize)));
                    return LRESULT(0);
                }
            }
            // 검색 상자에 여러 줄(엑셀 열 등)을 붙여넣으면 줄마다 `|`로 이어 한 번에 찾는다
            WM_PASTE if id as i32 == ID_SEARCH => {
                if let Some(text) = shell::clipboard_text(hwnd).filter(|t| t.trim().contains('\n')) {
                    let q = HSTRING::from(palette::lines_to_query(&text));
                    SendMessageW(hwnd, EM_REPLACESEL, Some(WPARAM(1)), Some(LPARAM(q.as_ptr() as isize)));
                    return LRESULT(0);
                }
            }
            WM_CHAR | WM_SYSCHAR => {
                if SUPPRESS_CHAR.replace(false) || wp.0 == 0x7F {
                    return LRESULT(0);
                }
                if msg == WM_CHAR
                    && id as i32 == ID_RENAME
                    && char::from_u32(wp.0 as u32).is_some_and(blocked_name_char)
                    && with_app(|app| app.rename_char_blocked()).unwrap_or(false)
                {
                    return LRESULT(0);
                }
                // 보기 메뉴가 열려 있으면 글자가 필터 상자로 들어가지 않게 삼킨다
                if id as i32 == ID_FILTER && with_app(|app| app.view_menu.is_some()).unwrap_or(false) {
                    return LRESULT(0);
                }
                // 키보드가 즐겨찾기에 있으면 글자는 필터 대신 줄 찾기에 쓴다
                if msg == WM_CHAR && id as i32 == ID_FILTER && with_app(|app| app.fav_type_char(wp.0 as u16)).unwrap_or(false) {
                    return LRESULT(0);
                }
            }
            // 필터 상자를 직접 누르면 키보드는 칸으로 돌아온다 (즐겨찾기 조작이 필터 글자·Delete를 가로채지 않게).
            // 앱이 스스로 포커스를 줄 때는 App이 빌려져 있어 여기서 아무것도 하지 않는다
            WM_SETFOCUS if id as i32 == ID_FILTER => {
                with_app(|app| {
                    if app.fav_focus {
                        app.fav_focus = false;
                        app.invalidate();
                    }
                });
            }
            // 검색 상자를 직접 누르면 키보드는 검색 쪽으로
            WM_SETFOCUS if id as i32 == ID_SEARCH => {
                with_app(|app| {
                    if !app.pal_focus {
                        app.pal_focus = true;
                        app.invalidate();
                    }
                });
            }
            WM_KILLFOCUS => {
                let r = DefSubclassProc(hwnd, msg, wp, lp);
                // 검색 상자에서 이 창의 다른 곳으로 키보드가 옮겨 갔다 (wParam이 0이면 다른 앱으로: 돌아오면 검색 상자로)
                if id as i32 == ID_SEARCH && wp.0 != 0 {
                    with_app(|app| {
                        if app.pal_focus {
                            app.pal_focus = false;
                            app.invalidate();
                        }
                    });
                }
                with_app(|app| app.on_edit_blur(id as i32));
                if let Ok(parent) = GetParent(hwnd) {
                    crate::run_deferred(parent);
                }
                return r;
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
