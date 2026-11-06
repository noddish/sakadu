//! 세션(창 위치·레이아웃·탭) 저장과 복원. 외부 의존성 없이 줄 단위 텍스트 형식을 쓴다.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct PaneState {
    pub tabs: Vec<String>,
    pub active: usize,
    pub cols: [f32; 3],
    pub sort_key: u8,
    pub sort_desc: bool,
    /// 0 자세히, 1 썸네일
    pub view: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub window: Option<[i32; 4]>,
    pub maximized: bool,
    pub layout: u8,
    pub split: (f32, f32),
    pub active: usize,
    pub show_hidden: bool,
    pub preview: bool,
    /// 새 탭(Ctrl+T·+)을 열 위치: NEW_TAB_SAME이면 지금 탭과 같은 폴더, 아니면 탭 경로 형식("::computer" 또는 폴더)
    pub new_tab: String,
    /// 고정해 둔 왼쪽 패널: "places"(바로가기)·"favorites"(즐겨찾기)·"none"
    pub side: String,
    /// 왼쪽 패널 너비 (DIP)
    pub side_w: f32,
    pub look: Look,
    pub panes: Vec<PaneState>,
}

/// 왼쪽 패널 너비의 기본값과 범위
pub const SIDE_W_DEFAULT: f32 = 220.0;
pub const SIDE_W_RANGE: (f32, f32) = (180.0, 420.0);

/// 화면 설정: 테마("system"·"light"·"dark"), 강조 색(theme::ACCENTS의 키), 글꼴(빈 문자열이면 기본), 크기(%),
/// 글자 표시("soft"·"sharp"·"gray", gfx::TextLook)
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub theme: String,
    pub accent: String,
    pub font: String,
    pub zoom: u32,
    pub text: String,
}

impl Default for Look {
    fn default() -> Look {
        Look { theme: "system".into(), accent: "blue".into(), font: String::new(), zoom: 100, text: "gray".into() }
    }
}

const HEADER: &str = "sakadu-session 1";
pub const NEW_TAB_SAME: &str = "::same";
/// 0.4.0까지의 열 너비 기본값 (수정한 날짜, 유형, 크기)
const OLD_DEFAULT_COLS: [f32; 3] = [140.0, 130.0, 84.0];

pub fn file_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Sakadu").join("session.txt"))
}

pub fn load() -> Option<Session> {
    parse(&std::fs::read_to_string(file_path()?).ok()?)
}

pub fn save(s: &Session) -> std::io::Result<()> {
    let path = file_path().ok_or_else(|| std::io::Error::other("APPDATA 없음"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // 쓰는 도중 종료돼도 기존 세션이 깨지지 않도록 임시 파일에 쓴 뒤 교체
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serialize(s))?;
    std::fs::rename(tmp, path)
}

pub fn serialize(s: &Session) -> String {
    let mut out = String::new();
    out.push_str(HEADER);
    out.push('\n');
    if let Some([x, y, w, h]) = s.window {
        out.push_str(&format!("window={x},{y},{w},{h}\n"));
    }
    out.push_str(&format!("maximized={}\n", s.maximized as u8));
    out.push_str(&format!("layout={}\n", s.layout));
    out.push_str(&format!("split={:.4},{:.4}\n", s.split.0, s.split.1));
    out.push_str(&format!("active={}\n", s.active));
    out.push_str(&format!("hidden={}\n", s.show_hidden as u8));
    out.push_str(&format!("preview={}\n", s.preview as u8));
    out.push_str(&format!("newtab={}\n", s.new_tab));
    out.push_str(&format!("side={}\nsidew={}\n", s.side, s.side_w));
    out.push_str(&format!("theme={}\naccent={}\nfont={}\nzoom={}\ntext={}\n", s.look.theme, s.look.accent, s.look.font, s.look.zoom, s.look.text));
    for p in &s.panes {
        out.push_str(&format!(
            "pane=active:{};cols:{},{},{};sort:{},{};view:{}\n",
            p.active, p.cols[0], p.cols[1], p.cols[2], p.sort_key, p.sort_desc as u8, p.view
        ));
        for t in &p.tabs {
            out.push_str("tab=");
            out.push_str(t);
            out.push('\n');
        }
    }
    out
}

pub fn parse(text: &str) -> Option<Session> {
    // 메모장 등으로 저장하면 붙는 UTF-8 BOM은 무시
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next()?.trim() != HEADER {
        return None;
    }
    let mut s = Session {
        window: None,
        maximized: false,
        layout: 1,
        split: (0.5, 0.5),
        active: 0,
        show_hidden: false,
        preview: false,
        new_tab: "::computer".to_string(),
        // 0.5.x까지는 즐겨찾기 사이드바가 늘 보였으니, 그 세션은 즐겨찾기 패널을 고정한 채로 연다
        side: "favorites".to_string(),
        side_w: SIDE_W_DEFAULT,
        look: Look::default(),
        panes: Vec::new(),
    };
    for line in lines {
        let Some((key, value)) = line.split_once('=') else { continue };
        match key {
            "window" => {
                let v: Vec<i32> = value.split(',').filter_map(|n| n.trim().parse().ok()).collect();
                if v.len() == 4 {
                    s.window = Some([v[0], v[1], v[2], v[3]]);
                }
            }
            "maximized" => s.maximized = value == "1",
            "layout" => s.layout = value.parse().unwrap_or(1),
            "split" => {
                if let Some((a, b)) = value.split_once(',') {
                    s.split = (a.parse().unwrap_or(0.5), b.parse().unwrap_or(0.5));
                }
            }
            "active" => s.active = value.parse().unwrap_or(0),
            "hidden" => s.show_hidden = value == "1",
            "preview" => s.preview = value == "1",
            "newtab" => s.new_tab = value.to_string(),
            "side" => s.side = value.to_string(),
            "sidew" => s.side_w = value.parse::<f32>().unwrap_or(SIDE_W_DEFAULT).clamp(SIDE_W_RANGE.0, SIDE_W_RANGE.1),
            "theme" => s.look.theme = value.to_string(),
            "accent" => s.look.accent = value.to_string(),
            "font" => s.look.font = value.to_string(),
            "zoom" => s.look.zoom = value.parse::<u32>().unwrap_or(100).clamp(80, 200),
            "text" => s.look.text = value.to_string(),
            "pane" => {
                let mut p = PaneState { tabs: Vec::new(), active: 0, cols: crate::pane::DEFAULT_COLS, sort_key: 0, sort_desc: false, view: 0 };
                for part in value.split(';') {
                    let Some((k, v)) = part.split_once(':') else { continue };
                    let nums: Vec<f32> = v.split(',').filter_map(|n| n.parse().ok()).collect();
                    match k {
                        "active" => p.active = v.parse().unwrap_or(0),
                        // 한 번도 너비를 바꾸지 않은 예전 기본값이면 새 기본값으로 (유형 열이 잘리지 않게)
                        "cols" if nums == OLD_DEFAULT_COLS => p.cols = crate::pane::DEFAULT_COLS,
                        "cols" if nums.len() == 3 => p.cols = [nums[0], nums[1], nums[2]],
                        "sort" if nums.len() == 2 => {
                            p.sort_key = nums[0] as u8;
                            p.sort_desc = nums[1] != 0.0;
                        }
                        "view" => p.view = v.parse().unwrap_or(0),
                        _ => {}
                    }
                }
                s.panes.push(p);
            }
            // 경로에 '='나 ','가 있어도 되도록 나머지 전체를 경로로 본다
            "tab" => {
                if let Some(p) = s.panes.last_mut() {
                    p.tabs.push(value.to_string());
                }
            }
            _ => {}
        }
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Session {
            window: Some([10, 20, 1280, 800]),
            maximized: true,
            layout: 4,
            split: (0.4, 0.6),
            active: 2,
            show_hidden: true,
            preview: true,
            new_tab: "D:\\업무".into(),
            side: "places".into(),
            side_w: 260.0,
            look: Look { theme: "dark".into(), accent: "green".into(), font: "맑은 고딕".into(), zoom: 125, text: "soft".into() },
            panes: vec![
                PaneState {
                    tabs: vec!["C:\\a=b,c".into(), "::computer".into()],
                    active: 1,
                    cols: [150.0, 120.0, 90.0],
                    sort_key: 2,
                    sort_desc: true,
                    view: 1,
                },
                PaneState { tabs: vec!["D:\\".into()], active: 0, cols: crate::pane::DEFAULT_COLS, sort_key: 0, sort_desc: false, view: 0 },
            ],
        };
        assert_eq!(parse(&serialize(&s)), Some(s));
    }

    #[test]
    fn untouched_old_column_widths_move_to_new_default() {
        let s = parse("sakadu-session 1\npane=cols:140,130,84\ntab=C:\\\npane=cols:140,131,84\ntab=C:\\\npane=active:0\ntab=C:\\").unwrap();
        assert_eq!(s.panes[0].cols, crate::pane::DEFAULT_COLS);
        assert_eq!(s.panes[1].cols, [140.0, 131.0, 84.0], "a width the user set stays");
        assert_eq!(s.panes[2].cols, crate::pane::DEFAULT_COLS);
    }

    #[test]
    fn old_session_keeps_the_favorites_panel_open() {
        let s = parse("sakadu-session 1\nlayout=2").unwrap();
        assert_eq!(s.side, "favorites");
        assert_eq!(s.side_w, SIDE_W_DEFAULT);
        let s = parse("sakadu-session 1\nside=none\nsidew=9999").unwrap();
        assert_eq!(s.side, "none");
        assert_eq!(s.side_w, SIDE_W_RANGE.1, "an out-of-range width is clamped");
    }

    #[test]
    fn new_tab_defaults_to_this_pc() {
        assert_eq!(parse("sakadu-session 1\nlayout=2").unwrap().new_tab, "::computer");
    }

    #[test]
    fn rejects_unknown_header() {
        assert_eq!(parse("something else\nlayout=2"), None);
    }
}
