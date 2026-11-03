//! 바꿀 수 있는 단축키: 동작마다 키 조합 하나. 기본값과 다른 것만
//! %APPDATA%\Sakadu\keys.txt 에 "동작=키" 줄로 저장한다 (빈 값은 단축키 없음).
//! 방향키·Enter·Esc·Tab, 복사·붙여넣기(Ctrl+C/X/V/A) 같은 기본 조작은 바꾸지 않는다.

use std::path::PathBuf;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Combo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub vk: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Search,
    Favorites,
    AddFavorite,
    NewTab,
    CloseTab,
    NewFolder,
    Hidden,
    Preview,
    Undo,
    Layout1,
    Layout2,
    Layout3,
    Layout4,
    Layout5,
    Thumbs,
    Details,
    Address,
    Rename,
    Refresh,
    Settings,
    Help,
}

/// (동작, 저장 이름, 화면 이름, 기본 키)
pub const ACTIONS: [(Action, &str, &str, &str); 21] = [
    (Action::Search, "search", "디스크 검색", "Ctrl+F"),
    (Action::Favorites, "favorites", "즐겨찾기 열기", "Ctrl+B"),
    (Action::AddFavorite, "add-favorite", "즐겨찾기에 추가", "Ctrl+D"),
    (Action::NewTab, "new-tab", "새 탭", "Ctrl+T"),
    (Action::CloseTab, "close-tab", "탭 닫기", "Ctrl+W"),
    (Action::NewFolder, "new-folder", "새 폴더", "Ctrl+Shift+N"),
    (Action::Hidden, "hidden", "숨김 항목 보기", "Ctrl+H"),
    (Action::Preview, "preview", "미리 보기 창", "Alt+P"),
    (Action::Undo, "undo", "되돌리기", "Ctrl+Z"),
    (Action::Layout1, "layout-1", "한 칸", "Ctrl+1"),
    (Action::Layout2, "layout-2", "좌우 두 칸", "Ctrl+2"),
    (Action::Layout3, "layout-3", "위아래 두 칸", "Ctrl+3"),
    (Action::Layout4, "layout-4", "세 칸", "Ctrl+4"),
    (Action::Layout5, "layout-5", "네 칸", "Ctrl+5"),
    (Action::Thumbs, "thumbs", "썸네일 보기", "Ctrl+Shift+2"),
    (Action::Details, "details", "자세히 보기", "Ctrl+Shift+6"),
    (Action::Address, "address", "경로 입력", "Ctrl+L"),
    (Action::Rename, "rename", "이름 바꾸기", "F2"),
    (Action::Refresh, "refresh", "새로 고침", "F5"),
    (Action::Settings, "settings", "설정", "Ctrl+,"),
    (Action::Help, "help", "도움말", "F1"),
];

/// 키 이름 ↔ 가상 키 코드 (글자·숫자·F키 밖의 것)
const NAMED: [(&str, u16); 20] = [
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
];

fn key_name(vk: u16) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some((vk as u8 as char).to_string()),
        0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),
        _ => NAMED.iter().find(|n| n.1 == vk).map(|n| n.0.to_string()),
    }
}

fn key_code(name: &str) -> Option<u16> {
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
    pub fn now(vk: u16) -> Combo {
        let down = |k: u16| unsafe { GetKeyState(k as i32) } < 0;
        Combo { ctrl: down(VK_CONTROL.0), shift: down(VK_SHIFT.0), alt: down(VK_MENU.0), vk }
    }

    pub fn label(self) -> String {
        let mut s = String::new();
        for (on, name) in [(self.ctrl, "Ctrl+"), (self.shift, "Shift+"), (self.alt, "Alt+")] {
            if on {
                s.push_str(name);
            }
        }
        s.push_str(&key_name(self.vk).unwrap_or_else(|| format!("0x{:02X}", self.vk)));
        s
    }

    pub fn parse(s: &str) -> Option<Combo> {
        let mut c = Combo { ctrl: false, shift: false, alt: false, vk: 0 };
        // "Ctrl+," 처럼 키 자체가 '+'인 경우는 없으므로 마지막 '+' 뒤가 키다
        let (mods, key) = s.rsplit_once('+').unwrap_or(("", s));
        for m in mods.split('+').filter(|m| !m.is_empty()) {
            match m {
                "Ctrl" => c.ctrl = true,
                "Shift" => c.shift = true,
                "Alt" => c.alt = true,
                _ => return None,
            }
        }
        c.vk = key_code(key)?;
        Some(c)
    }

    /// 단축키로 쓸 수 있는가: Ctrl·Alt를 함께 누르거나 F1~F24. 글자 입력·기본 조작과 겹치지 않게 한다
    pub fn usable(self) -> bool {
        let f_key = (0x70..=0x87).contains(&self.vk);
        let reserved = self.ctrl && !self.alt && !self.shift && matches!(self.vk, 0x41 | 0x43 | 0x56 | 0x58 | 0x09);
        key_name(self.vk).is_some() && (f_key || self.ctrl || self.alt) && !reserved
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
        Keymap { keys: ACTIONS.iter().map(|a| Combo::parse(a.3)).collect(), global: None }
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
        for line in lines {
            let Some((key, value)) = line.split_once('=') else { continue };
            let combo = Combo::parse(value.trim());
            if key == "global" {
                map.global = combo;
            } else if let Some(i) = ACTIONS.iter().position(|a| a.1 == key) {
                map.keys[i] = combo;
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
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Sakadu").join("keys.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_round_trip() {
        for s in ["Ctrl+F", "Ctrl+Shift+N", "Alt+P", "F5", "Ctrl+,", "Ctrl+Shift+2", "Ctrl+Alt+Space", "Ctrl+\\"] {
            assert_eq!(Combo::parse(s).map(Combo::label).as_deref(), Some(s), "{s}");
        }
        assert_eq!(Combo::parse("Ctrl+Hyper+F"), None);
        assert_eq!(Combo::parse("Ctrl+"), None);
    }

    #[test]
    fn only_safe_combos_are_usable() {
        assert!(Combo::parse("Ctrl+K").unwrap().usable());
        assert!(Combo::parse("F3").unwrap().usable());
        assert!(Combo::parse("Alt+P").unwrap().usable());
        assert!(!Combo::parse("K").unwrap().usable());
        assert!(!Combo::parse("Shift+K").unwrap().usable());
        assert!(!Combo::parse("Ctrl+C").unwrap().usable());
        assert!(!Combo::parse("Ctrl+V").unwrap().usable());
    }

    #[test]
    fn rebinding_frees_the_old_owner() {
        let mut map = Keymap::default();
        let ctrl_k = Combo::parse("Ctrl+K").unwrap();
        assert_eq!(map.set(Action::Search, Some(ctrl_k)), None);
        assert_eq!(map.find(ctrl_k), Some(Action::Search));
        // 즐겨찾기(Ctrl+B)를 Ctrl+K로 바꾸면 검색은 단축키가 없어진다
        assert_eq!(map.set(Action::Favorites, Some(ctrl_k)), Some(Action::Search));
        assert_eq!(map.get(Action::Search), None);
        let back = Keymap::parse(&map.serialize());
        assert_eq!(back, map);
        assert_eq!(Keymap::parse(&Keymap::default().serialize()), Keymap::default());
    }
}
