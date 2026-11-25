//! 명령 메뉴 (Windows Ctrl+K, 맥 ⌘K): 열린 탭·고정한 것·즐겨찾기·최근 위치·작업 공간으로 가기와 명령 실행을 한 상자에서.
//! 줄 고르기(이름·초성 맞추기와 순서), 판의 자리, 그리기를 두 앱이 함께 쓴다. 입력 상자와 고른 줄을 여는 일은 각 앱이 한다.
//! 디스크 전체에서 찾기는 검색 패널이 맡고, 여기서는 마지막 줄로만 이어 준다

use super::glyph::*;
use super::{Canvas, MARGIN, RADIUS, WORKBAR_H, soft_shadow};
use crate::draw::{Color, Font, R};
use crate::favorites::{Favorite, Favorites};
use crate::keys::{ACTIONS, Action};
use crate::palette::{R_ROW, Theme, alpha};
use crate::pane::Tab;
use crate::util;
use crate::workspaces::Workspaces;

pub const CMD_W: f32 = 560.0;
pub const CMD_INPUT_H: f32 = 44.0;
/// 줄 높이는 파일 목록과 같다
pub const CMD_ROW_H: f32 = 28.0;
/// 한 번에 보이는 줄 수 (넘치면 스크롤)
pub const CMD_ROWS: usize = 12;
/// 치기 전에 보여 줄 최근 위치 수
pub const RECENT_SHOWN: usize = 5;
const G_RUN: u16 = 0xE76C;
const G_WORKSPACE: u16 = 0xE8FD;

#[derive(Clone, PartialEq, Debug)]
pub enum CmdItem {
    Tab { pane: usize, tab: usize, title: String, path: Option<String> },
    Fav { group: Option<usize>, index: usize, name: String, path: String, is_dir: bool },
    /// 지금 작업 공간에 고정한 것 (pins 번호)
    Pin { index: usize, name: String, path: String, is_dir: bool },
    Recent(String),
    Workspace(usize, String),
    Command(Action),
    /// 친 글자가 경로처럼 보이면 그 경로로
    GoPath(String),
    /// 마지막 줄: 검색 패널에서 디스크 전체로 찾기
    SearchDisk(String),
}

/// 열린 명령 메뉴: 줄, 고른 줄, 목록 스크롤
pub struct CmdMenu {
    pub items: Vec<CmdItem>,
    pub sel: usize,
    pub scroll: f32,
}

/// 열 곳: 지금 칸, 옆 칸(Windows Alt+Enter, 맥 ⌥Return), 새 탭(Ctrl+Enter, ⌘Return)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CmdOpen {
    Here,
    Other,
    NewTab,
}

/// 고른 줄 옮기기: ↑↓는 끝에서 반대쪽 끝으로 넘어가고, Page Up/Down은 끝에서 멈춘다
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CmdStep {
    Up,
    Down,
    PageUp,
    PageDown,
}

/// 명령 메뉴 판에서 누른 곳 (판 밖은 닫기)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CmdHit {
    Scrim,
    Panel,
    Row(usize),
}

/// 한글 음절을 초성으로 (다운로드 → ㄷㅇㄹㄷ). 한글이 아닌 글자는 소문자로 그대로
pub fn chosung(s: &str) -> String {
    const CHO: [char; 19] = ['ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ'];
    s.chars()
        .map(|c| match c as u32 {
            0xAC00..=0xD7A3 => CHO[((c as u32 - 0xAC00) / 588) as usize],
            _ => c.to_ascii_lowercase(),
        })
        .collect()
}

/// 이름이 검색어에 맞는 정도 (0: 앞부분, 1: 가운데, 2: 초성). 맞지 않으면 None. 검색어는 소문자.
/// 자모로 나뉜 한글 이름(맥이 만든 NFD)은 합친 꼴로 맞춘다 (나뉜 것이 없으면 그대로)
pub fn score(name: &str, q: &str) -> Option<u8> {
    if q.is_empty() {
        return Some(0);
    }
    let name = crate::hangul::compose(name);
    let n = name.to_lowercase();
    if n.starts_with(q) {
        Some(0)
    } else if n.contains(q) {
        Some(1)
    } else if q.chars().all(|c| ('ㄱ'..='ㅎ').contains(&c) || c == ' ') && chosung(&name).contains(q) {
        Some(2)
    } else {
        None
    }
}

/// 경로처럼 보이는 입력 (이 앱이 도는 OS의 규칙)
pub fn looks_like_path(s: &str) -> bool {
    looks_like_path_on(s, cfg!(not(windows)))
}

/// 경로처럼 보이는 입력. Windows: C:\…, \\서버\…, %환경 변수%. 맥: /…, ~, ~/… (홈), 네트워크 공유 주소(smb:// 따위: 연결해서 연다)
pub fn looks_like_path_on(s: &str, unix: bool) -> bool {
    if unix {
        return s.starts_with('/') || s == "~" || s.starts_with("~/") || crate::util::share_url(s).is_some();
    }
    let b = s.as_bytes();
    s.starts_with("\\\\") || s.starts_with('%') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// 명령 메뉴의 줄을 만드는 데 쓰는 앱의 것
pub struct CmdSource<'a> {
    /// 보이는 칸의 탭 (칸 번호 순서)
    pub panes: Vec<&'a [Tab]>,
    /// 지금 작업 공간에 고정한 것
    pub pins: &'a [Favorite],
    /// 즐겨찾기 ('모든 공간')
    pub favs: &'a Favorites,
    /// 최근 위치 (최근 것이 앞)
    pub recent: &'a [String],
    pub workspaces: &'a Workspaces,
    /// 이 앱에서 도는 동작인지 (명령 메뉴 자체는 늘 뺀다)
    pub runs: fn(Action) -> bool,
}

/// 보여 줄 줄: 경로 → 열린 탭 → 고정한 것·즐겨찾기 → 최근 위치 → 작업 공간 → 명령 → 디스크에서 찾기.
/// 친 글자가 있으면 이름(또는 초성)이 맞는 것만, 같은 묶음 안에서는 앞부분이 맞는 것부터
pub fn cmd_items(src: &CmdSource, text: &str) -> Vec<CmdItem> {
    let raw = text.trim();
    let q = crate::hangul::compose(raw).to_lowercase();
    let mut out: Vec<(u8, u8, CmdItem)> = Vec::new();
    if looks_like_path(raw) {
        out.push((0, 0, CmdItem::GoPath(raw.to_string())));
    }
    for (p, tabs) in src.panes.iter().enumerate() {
        for (i, tab) in tabs.iter().enumerate() {
            let title = tab.loc.title();
            if let Some(s) = score(&title, &q) {
                out.push((1, s, CmdItem::Tab { pane: p, tab: i, title, path: tab.loc.dir().map(str::to_string) }));
            }
        }
    }
    for (index, f) in src.pins.iter().enumerate() {
        if let Some(s) = score(&f.name, &q).or_else(|| score(util::file_name(&f.path), &q)) {
            out.push((2, s, CmdItem::Pin { index, name: f.name.clone(), path: f.path.clone(), is_dir: f.is_dir }));
        }
    }
    let sections = std::iter::once(None).chain((0..src.favs.groups.len()).map(Some));
    for group in sections {
        for (index, f) in src.favs.section(group).iter().enumerate() {
            if let Some(s) = score(&f.name, &q).or_else(|| score(util::file_name(&f.path), &q)) {
                out.push((2, s, CmdItem::Fav { group, index, name: f.name.clone(), path: f.path.clone(), is_dir: f.is_dir }));
            }
        }
    }
    let recent = src.recent.iter().take(if q.is_empty() { RECENT_SHOWN } else { usize::MAX });
    for path in recent {
        let name = util::file_name(path);
        if let Some(s) = score(if name.is_empty() { path } else { name }, &q) {
            out.push((3, s, CmdItem::Recent(path.clone())));
        }
    }
    for (i, w) in src.workspaces.list.iter().enumerate().filter(|(i, _)| Some(*i) != src.workspaces.current) {
        if let Some(s) = score(&w.name, &q) {
            out.push((4, s, CmdItem::Workspace(i, w.name.clone())));
        }
    }
    for (a, _, name, _) in ACTIONS.iter().filter(|x| x.0 != Action::Commands && (src.runs)(x.0)) {
        if let Some(s) = score(name, &q) {
            out.push((5, s, CmdItem::Command(*a)));
        }
    }
    if !raw.is_empty() && !looks_like_path(raw) {
        out.push((6, 0, CmdItem::SearchDisk(raw.to_string())));
    }
    out.sort_by_key(|(group, s, _)| (*group, *s));
    out.into_iter().map(|(_, _, it)| it).collect()
}

/// 고른 줄 sel에서 step만큼 옮긴 줄 (n: 줄 수)
pub fn cmd_step(sel: usize, n: usize, step: CmdStep) -> usize {
    let page = CMD_ROWS.saturating_sub(1).max(1);
    match step {
        CmdStep::Up => {
            if sel == 0 {
                n.saturating_sub(1)
            } else {
                sel - 1
            }
        }
        CmdStep::Down => {
            if sel + 1 >= n {
                0
            } else {
                sel + 1
            }
        }
        CmdStep::PageUp => sel.saturating_sub(page),
        CmdStep::PageDown => (sel + page).min(n.saturating_sub(1)),
    }
}

/// 판 자리: 창(w×h) 가운데, 위에서 20%. n은 줄 수 (판 높이가 줄 수를 따른다)
pub fn cmd_rect(w: f32, h: f32, n: usize) -> R {
    let cw = CMD_W.min(w - 96.0).max(280.0);
    let rows = n.clamp(1, CMD_ROWS) as f32;
    let ch = CMD_INPUT_H + 1.0 + 8.0 + rows * CMD_ROW_H + 8.0;
    let top = (h * 0.2).max(WORKBAR_H + 8.0).min((h - ch - MARGIN).max(MARGIN));
    let l = (w - cw) / 2.0;
    R::new(l, top, l + cw, top + ch)
}

/// 입력 상자 자리 (돋보기 오른쪽). r은 판 자리
pub fn cmd_input_rect(r: R) -> R {
    R::new(r.l + 44.0, r.t, r.r - 16.0, r.t + CMD_INPUT_H)
}

/// 줄 목록 자리. r은 판 자리
pub fn cmd_list(r: R) -> R {
    R::new(r.l + 8.0, r.t + CMD_INPUT_H + 1.0 + 8.0, r.r - 8.0, r.b - 8.0)
}

impl CmdMenu {
    pub fn new(items: Vec<CmdItem>) -> CmdMenu {
        CmdMenu { items, sel: 0, scroll: 0.0 }
    }

    /// 입력이 바뀌어 줄을 새로 받았다: 맨 위 줄을 고른다
    pub fn set_items(&mut self, items: Vec<CmdItem>) {
        self.items = items;
        self.sel = 0;
        self.scroll = 0.0;
    }

    /// 창(w×h)에서 판 자리
    pub fn rect(&self, w: f32, h: f32) -> R {
        cmd_rect(w, h, self.items.len())
    }

    /// 판 자리 r에서 (x, y)를 눌렀다
    pub fn hit(&self, r: R, x: f32, y: f32) -> CmdHit {
        if !r.contains(x, y) {
            return CmdHit::Scrim;
        }
        let list = cmd_list(r);
        if list.contains(x, y) {
            let i = ((y - list.t + self.scroll) / CMD_ROW_H).floor();
            if i >= 0.0 && (i as usize) < self.items.len() {
                return CmdHit::Row(i as usize);
            }
        }
        CmdHit::Panel
    }

    /// 목록 스크롤 (list_h: 목록 높이)
    pub fn wheel(&mut self, dy: f32, list_h: f32) {
        let max = (self.items.len() as f32 * CMD_ROW_H - list_h).max(0.0);
        self.scroll = (self.scroll + dy).clamp(0.0, max);
    }

    /// 고른 줄이 보이게 스크롤한다
    pub fn reveal(&mut self, list_h: f32) {
        let y = self.sel as f32 * CMD_ROW_H;
        if y < self.scroll {
            self.scroll = y;
        } else if y + CMD_ROW_H > self.scroll + list_h {
            self.scroll = y + CMD_ROW_H - list_h;
        }
    }
}

/// 줄에 보일 것: (이름, 그 뒤의 흐린 위치, 고른 줄 오른쪽의 종류). multi: 칸이 여럿이면 탭 줄의 종류에 칸 번호를 쓴다
pub fn cmd_row_text(item: &CmdItem, multi: bool) -> (String, String, String) {
    match item {
        CmdItem::Tab { pane, title, path, .. } => (title.clone(), path.clone().unwrap_or_default(), if multi { format!("{}번 칸 탭", pane + 1) } else { "열린 탭".to_string() }),
        CmdItem::Fav { name, path, .. } => (name.clone(), util::parent(path).unwrap_or_default(), "모든 공간".to_string()),
        CmdItem::Pin { name, path, .. } => (name.clone(), util::parent(path).unwrap_or_default(), "이 공간에 고정".to_string()),
        CmdItem::Recent(path) => {
            let n = util::file_name(path);
            (if n.is_empty() { path.clone() } else { n.to_string() }, util::parent(path).unwrap_or_default(), "최근 위치".to_string())
        }
        CmdItem::Workspace(_, name) => (name.clone(), String::new(), "작업 공간".to_string()),
        CmdItem::Command(a) => (crate::keys::name(*a).to_string(), String::new(), String::new()),
        CmdItem::GoPath(p) => (p.clone(), String::new(), "경로로 이동".to_string()),
        CmdItem::SearchDisk(q) => (format!("‘{q}’ 디스크에서 찾기"), String::new(), "검색".to_string()),
    }
}

/// 줄의 아이콘을 그리는 쪽 (Windows: 시스템 아이콘 목록, 맥: Finder 아이콘)
pub trait CmdArt<G: Canvas> {
    /// 줄 i(item)의 16×16 아이콘. 작업 공간·디스크에서 찾기·명령 줄은 공통 코드가 글자 아이콘으로 그려 여기로 오지 않는다
    fn icon(&mut self, g: &mut G, i: usize, item: &CmdItem, r: R);
}

/// 판을 그리는 데 필요한 것
pub struct CmdScene<'a> {
    pub menu: &'a CmdMenu,
    /// 판 자리 (cmd_rect)
    pub rect: R,
    /// 칸이 여럿인지 (탭 줄의 종류에 칸 번호)
    pub multi: bool,
}

/// 판 뒤의 창(w×h)을 옅게 가린다
pub fn paint_cmd_scrim(g: &impl Canvas, t: &Theme, w: f32, h: f32) {
    g.fill(R::new(0.0, 0.0, w, h), Color { r: 0.0, g: 0.0, b: 0.0, a: if t.dark { 0.40 } else { 0.16 } });
}

/// 판과 줄. 입력 글자는 앱의 입력 상자가 cmd_input_rect 자리에서 그린다 (Windows는 이 앞뒤에 떠오르는 효과를 건다)
pub fn paint_commands<G: Canvas>(g: &mut G, art: &mut impl CmdArt<G>, t: &Theme, s: &CmdScene) {
    let (r, c) = (s.rect, s.menu);
    let list = cmd_list(r);
    if !t.high_contrast {
        soft_shadow(g, r, RADIUS, 16.0, 0.18, t.dark);
    }
    g.fill_round(r, RADIUS, t.pane);
    g.stroke_round(r.inset(0.5, 0.5), RADIUS, t.pane_border, 1.0);
    // 입력 줄: 돋보기와 (입력 상자는 앱의 것), 아래 가는 선
    g.glyph(G_SEARCH, R::new(r.l + 16.0, r.t, r.l + 32.0, r.t + CMD_INPUT_H), false, t.dim);
    g.fill(R::new(r.l, r.t + CMD_INPUT_H, r.r, r.t + CMD_INPUT_H + 1.0), if t.high_contrast { t.border } else { alpha(t.text, 0.07) });

    g.clip(list);
    if c.items.is_empty() {
        g.text("맞는 것이 없어요", R::new(list.l + 12.0, list.t, list.r - 12.0, list.t + CMD_ROW_H), Font::Normal, t.faint);
    }
    let first = (c.scroll / CMD_ROW_H).floor().max(0.0) as usize;
    let count = (list.h() / CMD_ROW_H).ceil() as usize + 1;
    for i in first..(first + count).min(c.items.len()) {
        let y = list.t + i as f32 * CMD_ROW_H - c.scroll;
        let row = R::new(list.l, y, list.r, y + CMD_ROW_H);
        // 고른 줄: 파일 목록의 고른 줄과 같은 옅은 강조 색
        if i == c.sel {
            g.fill_round(row, R_ROW, t.sel);
            if t.high_contrast {
                g.stroke_round(row.inset(0.75, 0.75), R_ROW, t.focus, 1.5);
            }
        }
        let ir = R::new(row.l + 12.0, y + 6.0, row.l + 28.0, y + 22.0);
        let item = &c.items[i];
        match item {
            CmdItem::Workspace(..) => g.glyph(G_WORKSPACE, ir, false, t.dim),
            CmdItem::SearchDisk(_) => g.glyph_font(G_SEARCH, ir, Font::GlyphMid, t.dim),
            CmdItem::Command(_) => g.glyph(G_RUN, ir, true, t.dim),
            _ => art.icon(g, i, item, ir),
        }
        // 이름, 그 뒤에 흐린 위치. 오른쪽의 종류는 고른 줄에만 보인다 (단축키는 보이지 않는다)
        let (name, detail, kind) = cmd_row_text(item, s.multi);
        let right = if i == c.sel && !kind.is_empty() {
            let kw = g.measure(&kind, Font::Small);
            g.text(&kind, R::new(row.r - 12.0 - kw, row.t, row.r - 12.0, row.b), Font::Small, t.dim);
            row.r - 12.0 - kw - 12.0
        } else {
            row.r - 12.0
        };
        let nw = g.measure(&name, Font::Normal).min((right - row.l - 40.0) * 0.6).max(0.0);
        g.text(&name, R::new(row.l + 40.0, row.t, (row.l + 40.0 + nw + 2.0).min(right), row.b), Font::Normal, t.text);
        if !detail.is_empty() && row.l + 40.0 + nw + 12.0 < right - 24.0 {
            g.text(&detail, R::new(row.l + 40.0 + nw + 12.0, row.t, right, row.b), Font::Path, t.faint);
        }
    }
    g.unclip();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::Location;
    use crate::workspaces::Workspace;

    /// 초성으로도 찾는다 (ㄷㅇㄹㄷ → 다운로드), 앞부분이 맞는 것이 먼저
    #[test]
    fn matches_by_prefix_middle_and_chosung() {
        assert_eq!(chosung("다운로드"), "ㄷㅇㄹㄷ");
        assert_eq!(chosung("2026 견적"), "2026 ㄱㅈ");
        assert_eq!(score("다운로드", "다운"), Some(0));
        assert_eq!(score("내 다운로드", "다운"), Some(1));
        assert_eq!(score("다운로드", "ㄷㅇㄹ"), Some(2));
        assert_eq!(score("Downloads", "load"), Some(1));
        assert_eq!(score("문서", "ㄷㅇ"), None);
        assert_eq!(score("문서", ""), Some(0));
    }

    /// 자모로 나뉜 이름(맥이 만든 NFD)도 완성된 글자와 초성으로 찾는다
    #[test]
    fn decomposed_names_match_like_composed_ones() {
        // 보고서 = ᄇ ᅩ ᄀ ᅩ ᄉ ᅥ
        let nfd = "\u{1107}\u{1169}\u{1100}\u{1169}\u{1109}\u{1165}";
        assert_eq!(score(nfd, "보고"), Some(0));
        assert_eq!(score(&format!("주간 {nfd}"), "보고"), Some(1));
        assert_eq!(score(nfd, "ㅂㄱㅅ"), Some(2));
        assert_eq!(score(nfd, "ㅅㅂ"), None);
    }

    #[test]
    fn recognizes_paths() {
        let win = |s: &str| looks_like_path_on(s, false);
        assert!(win("C:\\Users"));
        assert!(win("\\\\nas\\자료"));
        assert!(win("%USERPROFILE%"));
        assert!(!win("견적서") && !win("/Users"));
        let mac = |s: &str| looks_like_path_on(s, true);
        assert!(mac("/Users/a/문서") && mac("~") && mac("~/Downloads"));
        assert!(!mac("견적서") && !mac("~a") && !mac("C:\\Users"));
        // 맥에서는 네트워크 공유 주소도 (연결해서 연다). Windows의 주소 칸은 받지 않는다
        assert!(mac("smb://nas.local/사진") && mac("afp://nas/share") && !mac("smb://") && !mac("https://example.com"));
        assert!(!win("smb://nas.local/사진"));
    }

    /// 줄의 순서: 경로 → 탭 → 고정한 것·즐겨찾기 → 최근 위치 → 작업 공간 → 명령 → 디스크에서 찾기. 묶음 안에서는 앞부분이 맞는 것부터
    #[test]
    fn items_are_grouped_and_ranked() {
        // 이 OS의 경로 ("a/b" → C:\a\b 또는 /a/b)
        let p = |s: &str| if cfg!(windows) { format!("C:\\{}", s.replace('/', "\\")) } else { format!("/{s}") };
        let tabs = [Tab::new(1, Location::Dir(p("a/내 다운로드"))), Tab::new(2, Location::Dir(p("a/다운로드")))];
        let pins = [Favorite { name: "다운 받은 것".into(), path: p("b/받은 것"), is_dir: true }];
        let favs = Favorites::default();
        let recent = [p("c/다운로드 보관"), p("c/문서")];
        let ws = Workspaces { list: vec![Workspace::new("업무".into(), crate::session::parse("sakadu-session 1\n").unwrap()), Workspace::new("다운로드 정리".into(), crate::session::parse("sakadu-session 1\n").unwrap())], current: Some(0) };
        let src = CmdSource { panes: vec![&tabs], pins: &pins, favs: &favs, recent: &recent, workspaces: &ws, runs: |a| a != Action::Search };
        let items = cmd_items(&src, "다운");
        let names: Vec<String> = items.iter().map(|it| cmd_row_text(it, false).0).collect();
        assert_eq!(names, ["다운로드", "내 다운로드", "다운 받은 것", "다운로드 보관", "다운로드 정리", "‘다운’ 디스크에서 찾기"]);
        // 아무것도 치지 않으면 모든 탭·고정한 것·최근 위치·다른 작업 공간·명령 (도는 명령만, 명령 메뉴 자신은 빼고)
        let all = cmd_items(&src, "");
        assert!(all.contains(&CmdItem::Command(Action::NewTab)) && !all.contains(&CmdItem::Command(Action::Search)) && !all.contains(&CmdItem::Command(Action::Commands)));
        assert!(!all.iter().any(|it| matches!(it, CmdItem::SearchDisk(_) | CmdItem::GoPath(_))));
        assert!(!all.iter().any(|it| matches!(it, CmdItem::Workspace(0, _))), "the current workspace is not listed");
        // 경로처럼 보이면 맨 위에 그 경로로 가는 줄, 디스크에서 찾기는 없다
        let path = if cfg!(windows) { "C:\\Users" } else { "/Users" };
        let items = cmd_items(&src, path);
        assert_eq!(items.first(), Some(&CmdItem::GoPath(path.to_string())));
        assert!(!items.iter().any(|it| matches!(it, CmdItem::SearchDisk(_))));
    }

    #[test]
    fn steps_wrap_at_the_ends_and_pages_stop() {
        assert_eq!(cmd_step(0, 5, CmdStep::Up), 4);
        assert_eq!(cmd_step(4, 5, CmdStep::Down), 0);
        assert_eq!(cmd_step(2, 5, CmdStep::Down), 3);
        assert_eq!(cmd_step(3, 40, CmdStep::PageUp), 0);
        assert_eq!(cmd_step(3, 40, CmdStep::PageDown), 3 + CMD_ROWS - 1);
        assert_eq!(cmd_step(38, 40, CmdStep::PageDown), 39);
        assert_eq!(cmd_step(0, 0, CmdStep::Down), 0);
    }

    /// 판은 줄 수만큼 자라고(열두 줄까지), 줄을 누르면 그 줄, 판 밖은 닫기
    #[test]
    fn panel_grows_with_rows_and_hits_rows() {
        let mut m = CmdMenu::new((0..30).map(|i| CmdItem::Recent(format!("/r/{i}"))).collect());
        let r = m.rect(1280.0, 800.0);
        assert_eq!(r.h(), CMD_INPUT_H + 1.0 + 8.0 + CMD_ROWS as f32 * CMD_ROW_H + 8.0);
        assert_eq!((r.l + r.r) / 2.0, 640.0);
        let list = cmd_list(r);
        assert_eq!(m.hit(r, list.l + 10.0, list.t + CMD_ROW_H * 2.5), CmdHit::Row(2));
        assert_eq!(m.hit(r, r.l + 4.0, r.t + 4.0), CmdHit::Panel);
        assert_eq!(m.hit(r, 2.0, 2.0), CmdHit::Scrim);
        m.wheel(10_000.0, list.h());
        assert_eq!(m.scroll, 30.0 * CMD_ROW_H - list.h());
        assert_eq!(m.hit(r, list.l + 10.0, list.t + 1.0), CmdHit::Row(30 - CMD_ROWS));
        m.sel = 0;
        m.reveal(list.h());
        assert_eq!(m.scroll, 0.0);
        m.set_items(vec![CmdItem::SearchDisk("x".into())]);
        assert_eq!(m.rect(1280.0, 800.0).h(), CMD_INPUT_H + 1.0 + 8.0 + CMD_ROW_H + 8.0);
    }
}
