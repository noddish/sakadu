//! 작업 공간: 이름을 붙여 저장해 둔 화면 (칸 나누기·칸·탭, 사이드 패널, 미리 보기와 고정한 파일, 검색어).
//! 공간마다 색과, 그 공간에만 보이는 고정 목록(폴더·파일)이 있다. 어느 공간에서나 보이는 것은 즐겨찾기(favorites.rs)다.
//! 세션과 같은 줄 형식(session::serialize·parse)을 그대로 써서 %APPDATA%\Sakadu\workspaces.txt 한 파일에 담는다.
//! 색(wscolor=)과 고정(wspin=) 줄은 세션 형식이 모르는 이름이라, 예전 빌드는 건너뛰고 읽는다.
//! 창 위치·화면 설정·최근 위치처럼 작업 공간과 상관없는 값은 들어 있어도 바꿀 때 쓰지 않는다.
//! 작업 공간이 하나라도 있으면 늘 그중 하나가 지금 것이고, 그 작업 공간은 화면을 따라간다:
//! 세션을 저장할 때(창을 닫을 때·설정을 바꿀 때·Windows를 끌 때)와 다른 작업 공간으로 바꿀 때 지금 화면으로 저장된다

use crate::favorites::{self, Favorite};
use crate::pane::Location;
use crate::session::{self, Session};
use crate::util;
use std::path::PathBuf;

const HEADER: &str = "sakadu-workspaces 1";
/// 이름 길이 (작업 줄 단추에 들어갈 만큼)
const NAME_MAX: usize = 40;

/// 공간에 줄 수 있는 색 (palette::ACCENTS의 키). 새 공간에는 이 순서로 아직 쓰지 않은 색을 준다
pub const COLORS: [&str; 6] = ["blue", "purple", "orange", "green", "pink", "teal"];

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub name: String,
    /// 공간 색 (COLORS 가운데 하나). 비어 있으면 설정의 강조 색을 따른다
    pub color: String,
    /// 이 공간에 고정한 폴더·파일 (사이드바 위쪽에 보인다)
    pub pins: Vec<Favorite>,
    pub state: Session,
}

impl Workspace {
    /// 색과 고정한 것이 없는 새 공간
    pub fn new(name: String, state: Session) -> Workspace {
        Workspace { name, color: String::new(), pins: Vec::new(), state }
    }

    /// 고정한 것 가운데 그 경로의 번호
    pub fn pin_index(&self, path: &str) -> Option<usize> {
        self.pins.iter().position(|f| util::eq_path(&f.path, path))
    }

    /// 그 경로를 고정한다 (이미 있으면 그대로). 그 줄의 번호
    pub fn pin(&mut self, path: &str, is_dir: bool) -> usize {
        self.pin_index(path).unwrap_or_else(|| {
            self.pins.push(Favorite { name: favorites::default_name(path), path: path.to_string(), is_dir });
            self.pins.len() - 1
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workspaces {
    pub list: Vec<Workspace>,
    /// 지금 작업 공간. 목록이 비어 있지 않으면 늘 Some
    pub current: Option<usize>,
}

impl Workspaces {
    pub fn current_name(&self) -> Option<&str> {
        self.current().map(|w| w.name.as_str())
    }

    pub fn current(&self) -> Option<&Workspace> {
        self.current.and_then(|c| self.list.get(c))
    }

    pub fn current_mut(&mut self) -> Option<&mut Workspace> {
        self.current.and_then(|c| self.list.get_mut(c))
    }

    /// 새 공간에 줄 색: 아직 쓰지 않은 색. 다 썼으면 가장 적게 쓴 색 (같으면 앞의 것).
    /// 색을 정하지 않은 공간은 설정의 강조 색(default)을 쓰는 것으로 센다 (기본 공간과 같은 색을 주지 않게)
    pub fn next_color(&self, default: &str) -> &'static str {
        let used = |c: &str| self.list.iter().filter(|w| if w.color.is_empty() { default == c } else { w.color == c }).count();
        COLORS.into_iter().min_by_key(|c| used(c)).unwrap_or(COLORS[0])
    }

    /// 공간이 하나도 없으면 지금 화면으로 '기본' 공간을 만든다 (왼쪽 레일에는 늘 지금 공간이 있다).
    /// 색은 비워 둬서 설정의 강조 색을 그대로 따른다. 만들었으면 true
    pub fn ensure_one(&mut self, state: impl FnOnce() -> Session) -> bool {
        if !self.list.is_empty() {
            return false;
        }
        self.list.push(Workspace::new("기본".to_string(), state()));
        self.current = Some(0);
        true
    }

    /// 공간 i를 지운다 (마지막 하나는 지우지 않는다). 지금 공간이 바뀌었으면(지금 것을 지웠으면) true
    pub fn remove(&mut self, i: usize) -> bool {
        if self.list.len() <= 1 || i >= self.list.len() {
            return false;
        }
        self.list.remove(i);
        match self.current {
            Some(c) if c == i => {
                self.current = Some(i.min(self.list.len() - 1));
                true
            }
            Some(c) if c > i => {
                self.current = Some(c - 1);
                false
            }
            _ => false,
        }
    }

    /// 겹치지 않는 이름. 같은 이름이 있으면 뒤에 " 2", " 3"…을 붙인다. skip은 이름을 바꾸는 그 작업 공간
    pub fn unique_name(&self, base: &str, skip: Option<usize>) -> String {
        let taken = |n: &str| self.list.iter().enumerate().any(|(i, w)| Some(i) != skip && w.name.eq_ignore_ascii_case(n));
        if !taken(base) {
            return base.to_string();
        }
        // 번호를 붙여도 길이 제한 안에 들게 앞부분을 줄인다 (저장할 때 잘려 같은 이름이 되지 않게)
        (2..)
            .map(|k| {
                let suffix = format!(" {k}");
                let head: String = base.chars().take(NAME_MAX.saturating_sub(suffix.chars().count())).collect();
                format!("{}{suffix}", head.trim_end())
            })
            .find(|n| !taken(n))
            .unwrap_or_default()
    }
}

/// 이름으로 쓸 수 있게: 앞뒤 공백·줄바꿈을 없애고 길이를 줄인다. 비면 빈 문자열
pub fn clean_name(s: &str) -> String {
    let one_line: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    one_line.trim().chars().take(NAME_MAX).collect::<String>().trim_end().to_string()
}

pub fn file_path() -> Option<PathBuf> {
    crate::util::config_dir().map(|d| d.join("workspaces.txt"))
}

pub fn load() -> Workspaces {
    file_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| parse(&t)).unwrap_or_default()
}

pub fn save(w: &Workspaces) -> std::io::Result<()> {
    let path = file_path().ok_or_else(|| std::io::Error::other("APPDATA 없음"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // 쓰는 도중 종료돼도 기존 파일이 깨지지 않도록 임시 파일에 쓴 뒤 교체
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serialize(w))?;
    std::fs::rename(tmp, path)
}

pub fn serialize(w: &Workspaces) -> String {
    let mut out = format!("{HEADER}\n");
    if let Some(c) = w.current {
        out.push_str(&format!("current={c}\n"));
    }
    for ws in &w.list {
        out.push_str(&format!("workspace={}\n", clean_name(&ws.name)));
        if COLORS.contains(&ws.color.as_str()) {
            out.push_str(&format!("wscolor={}\n", ws.color));
        }
        // 경로를 맨 뒤에 둔다 (경로에 탭이 있어도 되게). 이름의 탭·줄바꿈은 공백으로
        for f in &ws.pins {
            out.push_str(&format!("wspin={}\t{}\t{}\n", if f.is_dir { "dir" } else { "file" }, favorites::clean_name(&f.name), f.path));
        }
        // 세션 형식에서 머리 줄만 뺀다
        let body = session::serialize(&ws.state);
        out.push_str(body.split_once('\n').map_or("", |(_, rest)| rest));
    }
    out
}

pub fn parse(text: &str) -> Option<Workspaces> {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next()?.trim() != HEADER {
        return None;
    }
    let mut current = None;
    // (이름, 색, 고정한 것, 세션 줄)
    let mut blocks: Vec<(String, String, Vec<Favorite>, Vec<&str>)> = Vec::new();
    for line in lines {
        if let Some(name) = line.strip_prefix("workspace=") {
            blocks.push((name.to_string(), String::new(), Vec::new(), Vec::new()));
        } else if let Some((_, color, pins, body)) = blocks.last_mut() {
            if let Some(c) = line.strip_prefix("wscolor=") {
                *color = if COLORS.contains(&c.trim()) { c.trim().to_string() } else { String::new() };
            } else if let Some(p) = line.strip_prefix("wspin=") {
                let mut parts = p.splitn(3, '\t');
                if let (Some(kind), Some(name), Some(path)) = (parts.next(), parts.next(), parts.next()) {
                    // 같은 경로가 두 번 적혀 있으면 처음 것만
                    if !path.trim().is_empty() && !pins.iter().any(|f: &Favorite| util::eq_path(&f.path, path)) {
                        let name = if name.trim().is_empty() { favorites::default_name(path) } else { name.to_string() };
                        pins.push(Favorite { name, path: path.to_string(), is_dir: kind != "file" });
                    }
                }
            } else {
                body.push(line);
            }
        } else if let Some(v) = line.strip_prefix("current=") {
            current = v.trim().parse::<usize>().ok();
        }
    }
    let list: Vec<Workspace> = blocks
        .into_iter()
        .filter_map(|(name, color, pins, body)| {
            let state = session::parse(&format!("{}\n{}", session::HEADER, body.join("\n")))?;
            let name = clean_name(&name);
            (!name.is_empty()).then_some(Workspace { name, color, pins, state })
        })
        .collect();
    let current = if list.is_empty() { None } else { Some(current.filter(|c| *c < list.len()).unwrap_or(0)) };
    Some(Workspaces { list, current })
}

/// 메뉴에 이름과 함께 보일 한 줄 요약: "두 칸 · 리뉴얼 · 검색 ‘견적’ · 고정 미리 보기"
pub fn summary(s: &Session) -> String {
    // session.layout은 app::Layout::ALL의 번호 (한 칸, 좌우 두 칸, 위아래 두 칸, 세 칸, 네 칸)
    let panes = match s.layout {
        0 => "한 칸",
        1 | 2 => "두 칸",
        3 => "세 칸",
        _ => "네 칸",
    };
    let mut parts = vec![panes.to_string()];
    if let Some(p) = s.panes.get(s.active).or(s.panes.first()) {
        if let Some(t) = p.tabs.get(p.active.min(p.tabs.len().saturating_sub(1))) {
            parts.push(Location::from_session(t).title());
        }
    }
    if s.side == "search" && !s.query.trim().is_empty() {
        parts.push(format!("검색 ‘{}’", s.query.trim()));
    }
    if s.preview {
        parts.push(if s.pin.is_some() { "고정 미리 보기" } else { "미리 보기" }.to_string());
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(layout: u8, tabs: &[&str], side: &str, query: &str, pin: Option<&str>) -> Session {
        let mut s = session::parse(&format!("{}\nlayout={layout}\npane=active:0\n{}", session::HEADER, tabs.iter().map(|t| format!("tab={t}")).collect::<Vec<_>>().join("\n"))).unwrap();
        s.side = side.into();
        s.query = query.into();
        s.preview = pin.is_some();
        s.pin = pin.map(String::from);
        s
    }

    #[test]
    fn round_trip_keeps_names_states_and_current() {
        let pin = |name: &str, path: &str, is_dir: bool| Favorite { name: name.into(), path: path.into(), is_dir };
        let w = Workspaces {
            list: vec![
                Workspace {
                    name: "월말 정산".into(),
                    color: "purple".into(),
                    pins: vec![pin("리뉴얼", "D:\\업무\\리뉴얼", true), pin("견적서 = 최종", "D:\\업무\\견적서=최종.pdf", false)],
                    state: state(1, &["D:\\업무\\리뉴얼", "::start"], "search", "견적", Some("D:\\업무\\견적서.pdf")),
                },
                Workspace::new("사진 선별".into(), state(3, &["E:\\사진=2026"], "places", "", None)),
            ],
            current: Some(1),
        };
        assert_eq!(parse(&serialize(&w)), Some(w));
    }

    /// 색과 고정 줄이 없는 예전 파일도 그대로 읽히고, 새 줄이 있는 파일을 예전 빌드(세션 형식만 아는 쪽)가 읽어도 화면은 같다
    #[test]
    fn color_and_pins_are_optional_and_invisible_to_the_session_format() {
        let old = parse("sakadu-workspaces 1\ncurrent=0\nworkspace=하나\nlayout=3\npane=active:0\ntab=C:\\").unwrap();
        assert_eq!((old.list[0].color.as_str(), old.list[0].pins.len(), old.list[0].state.layout), ("", 0, 3));
        let new = "sakadu-workspaces 1\nworkspace=하나\nwscolor=orange\nwspin=dir\t자료\tD:\\자료\nwspin=dir\t\tD:\\자료\nwspin=file\t\tD:\\a.txt\nwscolor=nope\nlayout=3\npane=active:0\ntab=C:\\";
        let w = parse(new).unwrap();
        // 모르는 색은 비우고, 같은 경로는 한 번만, 이름이 비면 파일 이름으로
        assert_eq!(w.list[0].color, "");
        assert_eq!(w.list[0].pins.iter().map(|f| (f.name.as_str(), f.is_dir)).collect::<Vec<_>>(), [("자료", true), ("a.txt", false)]);
        let body = new.split_once("workspace=하나\n").unwrap().1;
        assert_eq!(session::parse(&format!("{}\n{body}", session::HEADER)).unwrap(), w.list[0].state);
    }

    #[test]
    fn new_spaces_get_unused_colors_and_the_last_space_stays() {
        let mut w = Workspaces::default();
        assert!(w.ensure_one(|| state(0, &["C:\\"], "none", "", None)));
        assert_eq!((w.list[0].name.as_str(), w.list[0].color.as_str(), w.current), ("기본", "", Some(0)));
        assert!(!w.ensure_one(|| unreachable!()));
        assert!(!w.remove(0), "the only workspace is not removed");
        // 색 없는 '기본'은 설정의 강조 색으로 센다: 강조 색이 파랑이면 새 공간은 보라부터
        assert_eq!((w.next_color("blue"), w.next_color("system")), ("purple", "blue"));
        for _ in 0..7 {
            let color = w.next_color("system").to_string();
            let mut ws = Workspace::new("새".into(), state(0, &["C:\\"], "none", "", None));
            ws.color = color;
            w.list.push(ws);
        }
        // 여섯 색을 한 번씩 쓴 뒤 처음 색으로 돌아온다
        assert_eq!(w.list[1..].iter().map(|x| x.color.as_str()).collect::<Vec<_>>(), ["blue", "purple", "orange", "green", "pink", "teal", "blue"]);
        // 지금 공간 앞의 것을 지우면 번호가 따라 내려가고, 지금 것을 지우면 그 자리의 다음 공간이 지금 것이 된다
        w.current = Some(3);
        assert!(!w.remove(1));
        assert_eq!((w.current, w.list[2].color.as_str()), (Some(2), "orange"));
        assert!(w.remove(2));
        assert_eq!((w.current, w.list[2].color.as_str()), (Some(2), "green"));
        // 고정: 같은 경로는 한 번만 (대소문자만 다르면 같은 경로)
        let ws = &mut w.list[0];
        assert_eq!((ws.pin("D:\\자료", true), ws.pin("d:\\자료", true), ws.pin("D:\\b.txt", false)), (0, 0, 1));
        assert_eq!(ws.pin_index("D:\\B.TXT"), Some(1));
    }

    #[test]
    fn current_points_into_the_list() {
        let w = parse("sakadu-workspaces 1\ncurrent=9\nworkspace=하나\nlayout=0").unwrap();
        assert_eq!(w.current, Some(0), "an out-of-range current falls back to the first");
        assert_eq!(parse("sakadu-workspaces 1\ncurrent=0").unwrap(), Workspaces::default(), "no workspaces, no current");
        assert_eq!(parse("something else"), None);
    }

    #[test]
    fn names_are_cleaned_and_kept_apart() {
        assert_eq!(clean_name("  월말\n정산  "), "월말 정산");
        let w = Workspaces { list: vec![Workspace::new("리뉴얼".into(), state(0, &["C:\\"], "none", "", None))], current: Some(0) };
        assert_eq!(w.unique_name("리뉴얼", None), "리뉴얼 2");
        assert_eq!(w.unique_name("리뉴얼", Some(0)), "리뉴얼", "renaming to its own name is fine");
        assert_eq!(w.unique_name("사진", None), "사진");
        // 길이 제한(40자)에 걸린 이름도 번호를 붙인 뒤 제한 안에 들고 서로 다르다
        let long = "가".repeat(40);
        let w = Workspaces { list: vec![Workspace::new(long.clone(), state(0, &["C:\\"], "none", "", None))], current: Some(0) };
        let second = w.unique_name(&long, None);
        assert_eq!(second.chars().count(), 40);
        assert_ne!(clean_name(&second), long);
    }

    #[test]
    fn summary_reads_like_the_screen() {
        assert_eq!(summary(&state(1, &["D:\\업무\\리뉴얼"], "search", "견적", Some("D:\\a.pdf"))), "두 칸 · 리뉴얼 · 검색 ‘견적’ · 고정 미리 보기");
        assert_eq!(summary(&state(0, &["::start"], "places", "견적", None)), "한 칸 · 새 탭", "a query of a closed search panel is not shown");
    }
}
