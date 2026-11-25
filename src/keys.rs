//! 바꿀 수 있는 단축키: 동작마다 키 조합 하나. 기본값과 다른 것만
//! 설정 폴더의 keys.txt 에 "동작=키" 줄로 저장한다 (빈 값은 단축키 없음).
//! 방향키·Enter·Esc·Tab, 복사·붙여넣기(Ctrl+C/X/V/A) 같은 기본 조작은 바꾸지 않는다.
//! 키는 두 앱 모두 Windows 가상 키 코드로 적는다. 맥은 ⌘(cmd)를 쓰고, 기본 키가 Finder 관례를 따른다 (MAC_DEFAULTS)

use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Combo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// 맥의 ⌘ (Windows에서는 쓰지 않는다)
    pub cmd: bool,
    pub vk: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Search,
    Commands,
    Favorites,
    AddFavorite,
    NewTab,
    CloseTab,
    ReopenTab,
    NewFolder,
    Hidden,
    Preview,
    Undo,
    Layout1,
    Layout2,
    Layout3,
    Layout4,
    Layout5,
    Huge,
    /// 큰 아이콘 (예전 이름: 썸네일)
    Thumbs,
    Medium,
    SmallIcons,
    List,
    Details,
    Address,
    Rename,
    Refresh,
    Settings,
    Help,
}

/// (동작, 저장 이름, 화면 이름, 기본 키)
pub const ACTIONS: [(Action, &str, &str, &str); 27] = [
    (Action::Search, "search", "검색", "Ctrl+F"),
    (Action::Commands, "commands", "명령 메뉴", "Ctrl+K"),
    (Action::Favorites, "favorites", "사이드바 목록으로 이동", "Ctrl+B"),
    (Action::AddFavorite, "add-favorite", "이 공간에 고정", "Ctrl+D"),
    (Action::NewTab, "new-tab", "새 탭", "Ctrl+T"),
    (Action::CloseTab, "close-tab", "탭 닫기", "Ctrl+W"),
    (Action::ReopenTab, "reopen-tab", "닫은 탭 다시 열기", "Ctrl+Shift+T"),
    (Action::NewFolder, "new-folder", "새 폴더", "Ctrl+Shift+N"),
    (Action::Hidden, "hidden", "숨김 항목 보기", "Ctrl+H"),
    (Action::Preview, "preview", "미리 보기 창", "Alt+P"),
    (Action::Undo, "undo", "되돌리기", "Ctrl+Z"),
    (Action::Layout1, "layout-1", "한 칸", "Ctrl+1"),
    (Action::Layout2, "layout-2", "좌우 두 칸", "Ctrl+2"),
    (Action::Layout3, "layout-3", "위아래 두 칸", "Ctrl+3"),
    (Action::Layout4, "layout-4", "세 칸", "Ctrl+4"),
    (Action::Layout5, "layout-5", "네 칸", "Ctrl+5"),
    // 보기 단축키는 탐색기와 같다
    (Action::Huge, "huge-icons", "아주 큰 아이콘 보기", "Ctrl+Shift+1"),
    (Action::Thumbs, "thumbs", "큰 아이콘 보기", "Ctrl+Shift+2"),
    (Action::Medium, "medium-icons", "보통 아이콘 보기", "Ctrl+Shift+3"),
    (Action::SmallIcons, "small-icons", "작은 아이콘 보기", "Ctrl+Shift+4"),
    (Action::List, "list", "목록 보기", "Ctrl+Shift+5"),
    (Action::Details, "details", "자세히 보기", "Ctrl+Shift+6"),
    (Action::Address, "address", "경로 입력", "Ctrl+L"),
    (Action::Rename, "rename", "이름 바꾸기", "F2"),
    (Action::Refresh, "refresh", "새로 고침", "F5"),
    (Action::Settings, "settings", "설정", "Ctrl+,"),
    (Action::Help, "help", "도움말", "F1"),
];

/// 맥의 기본 키 (ACTIONS와 같은 순서). Finder 관례를 따른다: ⌘로 부르고, 이름 바꾸기는 Return.
/// 빈 것은 맥에 아직 그 동작이 없다 (사이드바 목록으로 이동, 명령 메뉴, 미리 보기 창)
pub const MAC_DEFAULTS: [&str; 27] = [
    "Cmd+F",
    "",
    "",
    "Ctrl+Cmd+T",
    "Cmd+T",
    "Cmd+W",
    "Shift+Cmd+T",
    "Shift+Cmd+N",
    "Shift+Cmd+.",
    "",
    "Cmd+Z",
    "Alt+Cmd+1",
    "Alt+Cmd+2",
    "Alt+Cmd+3",
    "Alt+Cmd+4",
    "Alt+Cmd+5",
    "Cmd+1",
    "Cmd+2",
    "Cmd+3",
    "Cmd+4",
    "Cmd+5",
    "Cmd+6",
    "Cmd+L",
    "Return",
    "Cmd+R",
    "Cmd+,",
    "Shift+Cmd+/",
];

/// i번째 동작의 기본 키 (이 앱이 도는 OS의 것)
fn default_key(i: usize) -> &'static str {
    if cfg!(target_os = "macos") { MAC_DEFAULTS[i] } else { ACTIONS[i].3 }
}

/// 키 이름 ↔ 가상 키 코드 (글자·숫자·F키 밖의 것). 지우기·Return·Esc·방향키는 맥에서만 단축키로 쓴다 (Combo::usable)
const NAMED: [(&str, u16); 27] = [
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("Tab", 0x09),
    (";", 0xBA),
    ("=", 0xBB),
    (",", 0xBC),
    ("-", 0xBD),
    (".", 0xBE),
    ("/", 0xBF),
    ("`", 0xC0),
    ("[", 0xDB),
    ("\\", 0xDC),
    ("]", 0xDD),
    ("'", 0xDE),
    ("Pause", 0x13),
    ("Backspace", 0x08),
    ("Return", 0x0D),
    ("Esc", 0x1B),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
];

/// Windows 기본 조작 키라 단축키로 정하지 않는 것 (맥에서는 지우기·방향키 조합을 쓴다)
const WIN_FIXED: [u16; 7] = [0x08, 0x0D, 0x1B, 0x25, 0x26, 0x27, 0x28];

pub fn key_name(vk: u16) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some((vk as u8 as char).to_string()),
        0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),
        _ => NAMED.iter().find(|n| n.1 == vk).map(|n| n.0.to_string()),
    }
}

pub fn key_code(name: &str) -> Option<u16> {
    let b = name.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_digit() || b[0].is_ascii_uppercase()) {
        return Some(b[0] as u16);
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some(0x6F + n);
    }
    NAMED.iter().find(|n| n.0 == name).map(|n| n.1)
}

impl Combo {
    /// 지금 눌린 수정 키와 함께
    #[cfg(windows)]
    pub fn now(vk: u16) -> Combo {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
        let down = |k: u16| unsafe { GetKeyState(k as i32) } < 0;
        Combo { ctrl: down(VK_CONTROL.0), shift: down(VK_SHIFT.0), alt: down(VK_MENU.0), cmd: false, vk }
    }

    /// 저장하는 이름 ("Ctrl+Shift+N", 맥은 "Shift+Cmd+N"). Windows 화면에도 이것을 보인다
    pub fn label(self) -> String {
        let mut s = String::new();
        for (on, name) in [(self.ctrl, "Ctrl+"), (self.shift, "Shift+"), (self.alt, "Alt+"), (self.cmd, "Cmd+")] {
            if on {
                s.push_str(name);
            }
        }
        s.push_str(&key_name(self.vk).unwrap_or_else(|| format!("0x{:02X}", self.vk)));
        s
    }

    /// 맥 화면에 보이는 이름: 수정 키는 ⌃⌥⇧⌘ 순서의 기호, 키는 글자나 기호 ("⇧⌘N", "↩", "⌘↑")
    pub fn glyphs(self) -> String {
        let mut s = String::new();
        for (on, g) in [(self.ctrl, '⌃'), (self.alt, '⌥'), (self.shift, '⇧'), (self.cmd, '⌘')] {
            if on {
                s.push(g);
            }
        }
        let key = match self.vk {
            0x08 => "⌫".to_string(),
            0x0D => "↩".to_string(),
            0x1B => "⎋".to_string(),
            0x09 => "⇥".to_string(),
            0x25 => "←".to_string(),
            0x26 => "↑".to_string(),
            0x27 => "→".to_string(),
            0x28 => "↓".to_string(),
            0x21 => "⇞".to_string(),
            0x22 => "⇟".to_string(),
            0x24 => "↖".to_string(),
            0x23 => "↘".to_string(),
            0x2E => "⌦".to_string(),
            vk => key_name(vk).unwrap_or_else(|| format!("0x{vk:02X}")),
        };
        s.push_str(&key);
        s
    }

    pub fn parse(s: &str) -> Option<Combo> {
        let mut c = Combo { ctrl: false, shift: false, alt: false, cmd: false, vk: 0 };
        // "Ctrl+," 처럼 키 자체가 '+'인 경우는 없으므로 마지막 '+' 뒤가 키다
        let (mods, key) = s.rsplit_once('+').unwrap_or(("", s));
        for m in mods.split('+').filter(|m| !m.is_empty()) {
            match m {
                "Ctrl" => c.ctrl = true,
                "Shift" => c.shift = true,
                "Alt" => c.alt = true,
                "Cmd" => c.cmd = true,
                _ => return None,
            }
        }
        c.vk = key_code(key)?;
        Some(c)
    }

    /// 단축키로 쓸 수 있는가 (이 앱이 도는 OS의 규칙)
    pub fn usable(self) -> bool {
        self.usable_on(cfg!(target_os = "macos"))
    }

    /// 단축키로 쓸 수 있는가. 글자 입력·기본 조작·시스템 키와 겹치지 않게 한다.
    /// - Windows: Ctrl·Alt를 함께 누르거나 F1~F24. Ctrl+A/C/V/X/Tab과 Enter·Esc·방향키·지우기는 기본 조작
    /// - 맥: ⌘·⌃를 함께 누르거나 F키, 또는 수정 키 없는 Return(이름 바꾸기의 기본). 편집(⌘A/C/V/X), 끝내기·가리기·최소화(⌘Q/H/M),
    ///   앱 바꾸기(⌘Tab), Spotlight(⌘Space), 창 바꾸기(⌘`), 화면 찍기(⇧⌘3/4/5)는 시스템이 쓴다
    pub fn usable_on(self, mac: bool) -> bool {
        let f_key = (0x70..=0x87).contains(&self.vk);
        if key_name(self.vk).is_none() {
            return false;
        }
        if !mac {
            let reserved = self.ctrl && !self.alt && !self.shift && matches!(self.vk, 0x41 | 0x43 | 0x56 | 0x58 | 0x09);
            return !self.cmd && (f_key || self.ctrl || self.alt) && !reserved && !WIN_FIXED.contains(&self.vk);
        }
        let plain_return = self.vk == 0x0D && !(self.ctrl || self.alt || self.shift || self.cmd);
        let cmd_only = self.cmd && !self.ctrl && !self.alt;
        let system = (cmd_only && !self.shift && matches!(self.vk, 0x41 | 0x43 | 0x56 | 0x58 | 0x51 | 0x48 | 0x4D | 0x09 | 0x20 | 0xC0))
            || (cmd_only && self.shift && (0x33..=0x35).contains(&self.vk));
        (plain_return || f_key || self.cmd || self.ctrl) && !system
    }
}

pub fn name(a: Action) -> &'static str {
    ACTIONS.iter().find(|x| x.0 == a).map(|x| x.2).unwrap_or("")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    /// ACTIONS와 같은 순서
    keys: Vec<Option<Combo>>,
    /// 어디서든 검색(전역 단축키). None이면 비어 있는 후보를 자동으로 고른다
    pub global: Option<Combo>,
}

impl Default for Keymap {
    fn default() -> Keymap {
        Keymap { keys: (0..ACTIONS.len()).map(|i| Combo::parse(default_key(i))).collect(), global: None }
    }
}

const HEADER: &str = "sakadu-keys 1";

impl Keymap {
    pub fn get(&self, a: Action) -> Option<Combo> {
        ACTIONS.iter().position(|x| x.0 == a).and_then(|i| self.keys[i])
    }

    pub fn find(&self, c: Combo) -> Option<Action> {
        self.keys.iter().position(|k| *k == Some(c)).map(|i| ACTIONS[i].0)
    }

    /// a에 c를 쓴다. 다른 동작이 쓰던 키면 그 동작의 단축키는 비우고 그 동작을 돌려준다
    pub fn set(&mut self, a: Action, c: Option<Combo>) -> Option<Action> {
        let taken = c.and_then(|c| self.find(c)).filter(|&other| other != a);
        if let Some(other) = taken {
            let i = ACTIONS.iter().position(|x| x.0 == other).unwrap_or(0);
            self.keys[i] = None;
        }
        if let Some(i) = ACTIONS.iter().position(|x| x.0 == a) {
            self.keys[i] = c;
        }
        taken
    }

    pub fn serialize(&self) -> String {
        let defaults = Keymap::default();
        let mut out = format!("{HEADER}\n");
        for (i, a) in ACTIONS.iter().enumerate() {
            if self.keys[i] != defaults.keys[i] {
                out.push_str(&format!("{}={}\n", a.1, self.keys[i].map(Combo::label).unwrap_or_default()));
            }
        }
        if let Some(g) = self.global {
            out.push_str(&format!("global={}\n", g.label()));
        }
        out
    }

    pub fn parse(text: &str) -> Keymap {
        let mut map = Keymap::default();
        let mut lines = text.trim_start_matches('\u{feff}').lines();
        if lines.next().map(str::trim) != Some(HEADER) {
            return map;
        }
        let mut explicit = vec![false; ACTIONS.len()];
        for line in lines {
            let Some((key, value)) = line.split_once('=') else { continue };
            let combo = Combo::parse(value.trim());
            if key == "global" {
                map.global = combo;
            } else if let Some(i) = ACTIONS.iter().position(|a| a.1 == key) {
                map.keys[i] = combo;
                explicit[i] = true;
            }
        }
        // 사용자가 정한 키가 새 버전에서 생긴 기본 키와 겹치면 사용자 것을 남긴다
        for j in 0..ACTIONS.len() {
            if !explicit[j] && map.keys[j].is_some() && (0..ACTIONS.len()).any(|i| explicit[i] && map.keys[i] == map.keys[j]) {
                map.keys[j] = None;
            }
        }
        map
    }

    pub fn load() -> Keymap {
        file_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Keymap::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = file_path().ok_or_else(|| std::io::Error::other("APPDATA 없음"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.serialize())?;
        std::fs::rename(tmp, path)
    }
}

fn file_path() -> Option<PathBuf> {
    crate::util::config_dir().map(|d| d.join("keys.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 예전 버전에서 사용자가 지금의 '닫은 탭 다시 열기' 기본 키를 다른 동작에 정해 두었으면 새 기본보다 사용자 것
    #[test]
    fn user_binding_beats_new_default() {
        let reopen = Keymap::default().get(Action::ReopenTab).unwrap();
        let m = Keymap::parse(&format!("{HEADER}\nthumbs={}\n", reopen.label()));
        assert_eq!(m.find(reopen), Some(Action::Thumbs));
        assert_eq!(m.get(Action::ReopenTab), None);
    }

    #[test]
    fn combos_round_trip() {
        for s in ["Ctrl+F", "Ctrl+Shift+N", "Alt+P", "F5", "Ctrl+,", "Ctrl+Shift+2", "Ctrl+Alt+Space", "Ctrl+\\", "Shift+Cmd+N", "Ctrl+Cmd+T", "Cmd+Up", "Return"] {
            assert_eq!(Combo::parse(s).map(Combo::label).as_deref(), Some(s), "{s}");
        }
        assert_eq!(Combo::parse("Ctrl+Hyper+F"), None);
        assert_eq!(Combo::parse("Ctrl+"), None);
    }

    #[test]
    fn only_safe_combos_are_usable() {
        let win = |s: &str| Combo::parse(s).unwrap().usable_on(false);
        assert!(win("Ctrl+K") && win("F3") && win("Alt+P"));
        assert!(!win("K") && !win("Shift+K") && !win("Ctrl+C") && !win("Ctrl+V"));
        // Windows에서는 Enter·방향키·지우기와 ⌘ 조합을 단축키로 받지 않는다 (기본 조작)
        assert!(!win("Ctrl+Return") && !win("Ctrl+Up") && !win("Alt+Backspace") && !win("Cmd+K"));
        let mac = |s: &str| Combo::parse(s).unwrap().usable_on(true);
        assert!(mac("Cmd+K") && mac("Shift+Cmd+N") && mac("Ctrl+Cmd+T") && mac("F5") && mac("Return") && mac("Cmd+Up"));
        // 맥: 수정 키 없는 글자, ⌥만, 편집 키, 시스템 키는 받지 않는다
        assert!(!mac("K") && !mac("Alt+P") && !mac("Cmd+C") && !mac("Cmd+Q") && !mac("Cmd+Space") && !mac("Shift+Cmd+4"));
    }

    /// 맥 기본 키: 모두 읽히고 맥 규칙에 맞으며, 서로 겹치지 않는다
    #[test]
    fn mac_defaults_are_usable_and_distinct() {
        let mut seen = Vec::new();
        for (i, s) in MAC_DEFAULTS.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
            let c = Combo::parse(s).unwrap_or_else(|| panic!("{s}"));
            assert!(c.usable_on(true), "{} {s}", ACTIONS[i].1);
            assert!(!seen.contains(&c), "{s} twice");
            seen.push(c);
        }
        assert_eq!(Combo::parse("Shift+Cmd+N").unwrap().glyphs(), "⇧⌘N");
        assert_eq!(Combo::parse("Ctrl+Alt+Cmd+Up").unwrap().glyphs(), "⌃⌥⌘↑");
        assert_eq!(Combo::parse("Return").unwrap().glyphs(), "↩");
        assert_eq!(Combo::parse("Shift+Cmd+/").unwrap().glyphs(), "⇧⌘/");
    }

    #[test]
    fn rebinding_frees_the_old_owner() {
        let mut map = Keymap::default();
        let ctrl_j = Combo::parse("Ctrl+J").unwrap();
        assert_eq!(map.set(Action::Search, Some(ctrl_j)), None);
        assert_eq!(map.find(ctrl_j), Some(Action::Search));
        // 즐겨찾기(Ctrl+B)를 Ctrl+J로 바꾸면 검색은 단축키가 없어진다
        assert_eq!(map.set(Action::Favorites, Some(ctrl_j)), Some(Action::Search));
        assert_eq!(map.get(Action::Search), None);
        let back = Keymap::parse(&map.serialize());
        assert_eq!(back, map);
        assert_eq!(Keymap::parse(&Keymap::default().serialize()), Keymap::default());
    }
}
