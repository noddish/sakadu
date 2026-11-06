//! 작업 공간: 이름을 붙여 저장해 둔 화면 (칸 나누기·칸·탭, 사이드 패널, 미리 보기와 고정한 파일, 검색어).
//! 세션과 같은 줄 형식(session::serialize·parse)을 그대로 써서 %APPDATA%\Sakadu\workspaces.txt 한 파일에 담는다.
//! 창 위치·화면 설정·최근 위치처럼 작업 공간과 상관없는 값은 들어 있어도 바꿀 때 쓰지 않는다.
//! 작업 공간이 하나라도 있으면 늘 그중 하나가 지금 것이고, 그 작업 공간은 화면을 따라간다:
//! 세션을 저장할 때(창을 닫을 때·설정을 바꿀 때·Windows를 끌 때)와 다른 작업 공간으로 바꿀 때 지금 화면으로 저장된다

use crate::pane::Location;
use crate::session::{self, Session};
use std::path::PathBuf;

const HEADER: &str = "sakadu-workspaces 1";
/// 이름 길이 (작업 줄 단추에 들어갈 만큼)
const NAME_MAX: usize = 40;

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub name: String,
    pub state: Session,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workspaces {
    pub list: Vec<Workspace>,
    /// 지금 작업 공간. 목록이 비어 있지 않으면 늘 Some
    pub current: Option<usize>,
}

impl Workspaces {
    pub fn current_name(&self) -> Option<&str> {
        self.current.and_then(|c| self.list.get(c)).map(|w| w.name.as_str())
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
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Sakadu").join("workspaces.txt"))
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
    let mut blocks: Vec<(String, Vec<&str>)> = Vec::new();
    for line in lines {
        if let Some(name) = line.strip_prefix("workspace=") {
            blocks.push((name.to_string(), Vec::new()));
        } else if let Some((_, body)) = blocks.last_mut() {
            body.push(line);
        } else if let Some(v) = line.strip_prefix("current=") {
            current = v.trim().parse::<usize>().ok();
        }
    }
    let list: Vec<Workspace> = blocks
        .into_iter()
        .filter_map(|(name, body)| {
            let state = session::parse(&format!("{}\n{}", session::HEADER, body.join("\n")))?;
            let name = clean_name(&name);
            (!name.is_empty()).then_some(Workspace { name, state })
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
        let w = Workspaces {
            list: vec![
                Workspace { name: "월말 정산".into(), state: state(1, &["D:\\업무\\리뉴얼", "::start"], "search", "견적", Some("D:\\업무\\견적서.pdf")) },
                Workspace { name: "사진 선별".into(), state: state(3, &["E:\\사진=2026"], "places", "", None) },
            ],
            current: Some(1),
        };
        assert_eq!(parse(&serialize(&w)), Some(w));
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
        let w = Workspaces { list: vec![Workspace { name: "리뉴얼".into(), state: state(0, &["C:\\"], "none", "", None) }], current: Some(0) };
        assert_eq!(w.unique_name("리뉴얼", None), "리뉴얼 2");
        assert_eq!(w.unique_name("리뉴얼", Some(0)), "리뉴얼", "renaming to its own name is fine");
        assert_eq!(w.unique_name("사진", None), "사진");
        // 길이 제한(40자)에 걸린 이름도 번호를 붙인 뒤 제한 안에 들고 서로 다르다
        let long = "가".repeat(40);
        let w = Workspaces { list: vec![Workspace { name: long.clone(), state: state(0, &["C:\\"], "none", "", None) }], current: Some(0) };
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
