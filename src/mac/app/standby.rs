//! 메뉴 막대 대기, 어디서든 찾기(전역 단축키), Finder에서 열어 달라고 보낸 것 (Windows의 트레이 대기·전역 단축키·
//! 탐색기 대신 열기와 같은 자리).
//! 앱은 설정 값과 고르는 규칙만 갖는다. 메뉴 막대 아이콘, 전역 단축키 등록(Carbon), 로그인 항목(SMAppService),
//! Finder가 보낸 것을 받는 일은 창 쪽이 한다 (view/standby.rs, view.rs의 앱 위임)

use super::*;

/// 어디서든 찾기의 자동 후보 (차례대로): ⌥⌘Space, ⌃⌥Space, ⌥⌘F (Windows의 Ctrl+Alt+Space, Win+Shift+F, Ctrl+Alt+F처럼).
/// 앱 안 단축키나 맥이 쓰는 키, 다른 앱이 먼저 가져간 키는 건너뛴다
pub const GLOBAL_CANDIDATES: [&str; 3] = ["Alt+Cmd+Space", "Ctrl+Alt+Space", "Alt+Cmd+F"];

/// 메뉴 막대 대기 설정 파일 (설정 폴더의 standby.txt)의 첫 줄
const STANDBY_HEADER: &str = "sakadu-standby 1";

fn standby_path() -> Option<std::path::PathBuf> {
    util::config_dir().map(|d| d.join("standby.txt"))
}

/// standby.txt를 읽는다. 첫 줄이 다르거나 줄이 없으면 꺼짐
fn parse_standby(text: &str) -> bool {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next().map(str::trim) != Some(STANDBY_HEADER) {
        return false;
    }
    lines.any(|line| line.trim() == "on=1")
}

fn standby_text(on: bool) -> String {
    format!("{STANDBY_HEADER}\non={}\n", on as u8)
}

/// Finder에서 보낸 경로 하나를 확인한다 (뒷줄에서): 폴더면 그 폴더, 파일이면 그 위 폴더와 고를 이름.
/// 경로는 받은 글자 그대로 쓴다 (다듬으면 이름 끝의 공백 따위가 잘려 다른 것이 된다)
fn resolve_one(path: &str) -> Result<(String, Option<String>), String> {
    let missing = || format!("경로를 찾을 수 없어요: {path}");
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => Ok((path.to_string(), None)),
        Ok(_) => util::parent(path).map(|dir| (dir, Some(util::file_name(path).to_string()))).ok_or_else(missing),
        Err(_) => Err(missing()),
    }
}

/// Finder에서 한 번에 보낸 경로들을 확인한다 (창 쪽이 뒷줄에서 부른다: 꺼진 네트워크 폴더라도 창이 멈추지 않게)
pub fn resolve_external(paths: &[String]) -> Vec<Result<(String, Option<String>), String>> {
    paths.iter().map(|p| resolve_one(p)).collect()
}

impl App {
    // ---------- 메뉴 막대 대기 ----------

    /// 저장한 설정을 읽는다 (보통 실행)
    pub fn load_standby(&mut self) {
        self.standby = standby_path().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|t| parse_standby(&t));
    }

    /// 메뉴 막대에서 대기하는가
    pub fn standby(&self) -> bool {
        self.standby
    }

    /// 설정 창의 '메뉴 막대에서 대기'. 바로 저장한다 (시험·그림에서는 저장하지 않는다). 아이콘은 창 쪽이 맞춘다
    pub fn set_standby(&mut self, on: bool) {
        self.standby = on;
        if !self.persist {
            return;
        }
        let saved = standby_path().ok_or_else(|| std::io::Error::other("HOME 없음")).and_then(|path| {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, standby_text(on))
        });
        if let Err(e) = saved {
            sakadu::diag::log(|| format!("standby setting: {e}"));
        }
    }

    /// 창을 닫을 때 끝내지 않고 창만 숨기는가 (대기 중이면. Windows의 트레이로 숨기기와 같다)
    pub fn stays_after_close(&self) -> bool {
        self.standby
    }

    /// 창 없이(메뉴 막대 아이콘만) 시작하는가: 대기를 켰고, 로그인할 때 저절로 열렸고, 열어 달라는 폴더가 없을 때
    pub fn starts_hidden(&self, at_login: bool, opening: bool) -> bool {
        self.standby && at_login && !opening
    }

    /// 메뉴 막대 아이콘의 풍선 도움말: 어디서든 찾기 키가 있으면 그 키도 (Windows의 tray_tip처럼)
    pub fn standby_tip(&self) -> String {
        match self.hotkey {
            Some(c) => format!("Sakadu · {}로 찾기", c.glyphs()),
            None => "Sakadu".to_string(),
        }
    }

    // ---------- 어디서든 찾기 ----------

    /// 등록해 볼 어디서든 찾기 키 (차례대로). 정해 둔 키가 있으면 그것만, 없으면 자동 후보 가운데 앱 안 단축키나
    /// 기본 조작에 쓰지 않는 것 (어디서든 키가 먼저 가로채, 그 키의 앱 안 동작을 키로 부를 수 없게 되므로. Windows와 같다)
    pub fn global_choices(&self) -> Vec<Combo> {
        if let Some(c) = self.keys.global {
            return vec![c];
        }
        GLOBAL_CANDIDATES.iter().filter_map(|s| Combo::parse(s)).filter(|c| self.keys.find(*c).is_none() && !fixed_key(*c)).collect()
    }

    /// 어디서든 찾기 키를 정한다 (None이면 자동 후보). 저장하고 앞의 값을 돌려준다. 등록은 창 쪽이 이어서 한다
    pub fn set_global_key(&mut self, c: Option<Combo>) -> Option<Combo> {
        let before = std::mem::replace(&mut self.keys.global, c);
        self.save_keys();
        before
    }

    /// 설정 창의 '어디서든 찾기' 단추 글: 정한 키, 자동으로 고른 키, 등록하지 못했으면 그렇다고
    pub fn global_text(&self) -> String {
        match (self.keys.global, self.hotkey) {
            (Some(c), Some(_)) => c.glyphs(),
            (Some(c), None) => format!("{} · 등록 안 됨", c.glyphs()),
            (None, Some(hk)) => format!("자동 ({})", hk.glyphs()),
            (None, None) => "없음".to_string(),
        }
    }

    // ---------- Finder에서 열기 ----------

    /// Finder에서 열어 달라고 보냈다 ('다음으로 열기', 서비스의 'Sakadu에서 열기', Dock 아이콘에 놓기).
    /// 확인은 창 쪽이 뒷줄에서 resolve_external로 하고 결과를 on_external로 받는다.
    /// 하던 입력·메뉴·명령 메뉴는 그만두고, 검색 입력 칸이 키보드를 가졌으면 칸으로 돌린다 (열린 탭이 바로 보이게)
    pub fn open_external(&mut self, paths: Vec<String>) {
        let paths: Vec<String> = paths.into_iter().filter(|p| p.starts_with('/')).collect();
        if paths.is_empty() {
            return;
        }
        self.edit = Editing::None;
        self.menu = None;
        self.close_commands();
        self.find_blur();
        self.external.push(paths);
    }

    /// 확인해 달라는 경로들 (창 쪽이 뒷줄로 가져간다)
    pub fn take_external(&mut self) -> Vec<Vec<String>> {
        std::mem::take(&mut self.external)
    }

    /// Finder에서 보낸 것을 확인한 결과: 지금 칸에 새 탭으로 연다. 폴더는 그 폴더를, 파일은 그 위 폴더를 열고 그 파일을 고른다.
    /// 같은 폴더의 것은 한 탭에 모은다 (그 폴더의 파일 여럿을 함께 고른다). 찾지 못한 것은 칸에 알린다
    pub fn on_external(&mut self, results: Vec<Result<(String, Option<String>), String>>) {
        let p = self.active;
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for r in results {
            match r {
                Ok((dir, name)) => {
                    let i = match groups.iter().position(|g| g.0 == dir) {
                        Some(i) => i,
                        None => {
                            groups.push((dir, Vec::new()));
                            groups.len() - 1
                        }
                    };
                    if let Some(n) = name
                        && !groups[i].1.contains(&n)
                    {
                        groups[i].1.push(n);
                    }
                }
                Err(msg) => self.flash = Some((p, msg)),
            }
        }
        for (dir, names) in groups {
            self.open_tab(p, Location::Dir(dir), names.first().cloned());
            if names.len() > 1
                && let Some(r) = self.panes[p].tab_mut().restore.as_mut()
            {
                r.selected = names;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{app, demo, noted};
    use super::*;

    /// Finder에서 보낸 것: 폴더는 새 탭으로, 파일은 그 폴더를 새 탭으로 열고 그 파일들을 고른다 (같은 폴더의 것은 한 탭).
    /// 하던 입력은 그만두고, 없는 경로는 칸에 알린다
    #[test]
    fn opening_from_finder_puts_folders_and_files_in_new_tabs() {
        let (_root, home) = demo();
        let (mut a, _g) = app(&home);
        let p = a.active_pane();
        let before = a.pane(p).tabs.len();
        a.begin_address(p);
        let work = util::join(&home, "업무");
        let downloads = util::join(&home, "다운로드");
        a.open_external(vec![downloads.clone(), util::join(&work, "예산.csv"), util::join(&work, "회의록.md"), "상대 경로".to_string()]);
        assert_eq!(a.edit, Editing::None, "typing a path stops");
        a.settle();
        let pane = a.pane(p);
        assert_eq!(pane.tabs.len(), before + 2, "one tab for the folder, one for the two files' folder");
        assert_eq!(pane.tabs[pane.active - 1].loc, Location::Dir(downloads));
        let s = a.seen();
        assert_eq!(s.dir, work);
        assert_eq!(s.selected, ["예산.csv", "회의록.md"]);
        assert_eq!(s.cursor, "예산.csv");
        // 없어진 것: 탭을 열지 않고 칸에 알린다
        a.open_external(vec![util::join(&home, "없는 폴더")]);
        a.settle();
        assert_eq!(a.pane(p).tabs.len(), before + 2);
        assert!(noted(&a, "경로를 찾을 수 없어요"), "{:?}", a.flash);
        // 상대 경로만 오면 아무것도 하지 않는다
        a.open_external(vec!["상대 경로".to_string()]);
        assert!(a.take_external().is_empty());
    }

    /// 메뉴 막대 대기: 파일 형식이 오가고, 켜면 창을 닫아도 남는다. 창 없이 시작하는 것은 로그인할 때 저절로 열렸고 열 것이 없을 때만
    #[test]
    fn standby_setting_round_trips_and_decides_closing() {
        assert!(parse_standby(&standby_text(true)));
        assert!(!parse_standby(&standby_text(false)));
        assert!(!parse_standby("on=1"), "a file without the header is ignored");
        assert!(!parse_standby(""));
        let (_root, home) = demo();
        let (mut a, _g) = app(&home);
        assert!(!a.prefs().standby && !a.stays_after_close(), "off by default");
        assert!(!a.starts_hidden(true, false));
        a.set_standby(true);
        assert!(a.prefs().standby && a.stays_after_close());
        assert!(a.starts_hidden(true, false));
        assert!(!a.starts_hidden(false, false), "opened by hand: show the window");
        assert!(!a.starts_hidden(true, true), "opened to show a folder: show the window");
        a.set_standby(false);
        assert!(!a.stays_after_close());
    }

    /// 어디서든 찾기: 정한 키가 없으면 자동 후보를 차례대로, 앱 안 단축키가 쓰는 후보는 빼고. 정한 키가 있으면 그것만.
    /// 설정 창 단추의 글은 정한 키·자동으로 고른 키·등록하지 못한 것을 가린다
    #[test]
    fn global_key_choices_skip_keys_the_app_uses() {
        let (_root, home) = demo();
        let (mut a, _g) = app(&home);
        let combo = |s: &str| Combo::parse(s).unwrap();
        let all: Vec<Combo> = GLOBAL_CANDIDATES.iter().map(|s| combo(s)).collect();
        for c in &all {
            assert!(c.usable() && !fixed_key(*c) && (c.cmd || c.ctrl), "{}", c.glyphs());
        }
        assert_eq!(a.global_choices(), all);
        assert_eq!(a.global_text(), "없음");
        // ⌥⌘Space를 앱 안 검색 키로 쓰면 자동 후보에서 빠진다
        a.rebind(Action::Search, Some(all[0]));
        assert_eq!(a.global_choices(), all[1..]);
        a.hotkey = Some(all[1]);
        assert_eq!(a.global_text(), "자동 (⌃⌥Space)");
        assert_eq!(a.standby_tip(), "Sakadu · ⌃⌥Space로 찾기");
        // 정한 키는 그것만 등록해 본다 (앱 안 단축키와 겹치면 등록한 뒤 창 쪽이 그쪽을 비운다)
        let mine = combo("Ctrl+Cmd+J");
        assert_eq!(a.set_global_key(Some(mine)), None);
        assert_eq!(a.global_choices(), [mine]);
        a.hotkey = None;
        assert_eq!(a.global_text(), "⌃⌘J · 등록 안 됨");
        a.hotkey = Some(mine);
        assert_eq!(a.global_text(), "⌃⌘J");
        // 기본으로 되돌리면 자동 후보로 돌아간다
        a.reset_keys();
        assert_eq!(a.global_choices(), all);
    }
}
