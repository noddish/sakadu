//! Ctrl+K 명령 메뉴: 열린 탭·즐겨찾기·최근 위치·작업 공간으로 가기와 명령 실행을 한 상자에서.
//! 디스크 전체에서 찾기는 검색 패널(Ctrl+F)이 맡고, 여기서는 마지막 줄로만 이어 준다

use super::*;
use crate::keys::{ACTIONS, Action};

const CMD_W: f32 = 560.0;
const CMD_INPUT_H: f32 = 48.0;
const CMD_ROW_H: f32 = 36.0;
const CMD_ROWS: usize = 9;
const CMD_FOOT_H: f32 = 32.0;
/// 치기 전에 보여 줄 최근 위치 수
const RECENT_SHOWN: usize = 5;
pub(super) const CMD_FADE_MS: f32 = 160.0;
const G_RUN: u16 = 0xE76C;
const G_WORKSPACE: u16 = 0xE8FD;

#[derive(Clone, PartialEq, Debug)]
pub(super) enum CmdItem {
    Tab { pane: usize, tab: usize, title: String, path: Option<String> },
    Fav { group: Option<usize>, index: usize, name: String, path: String, is_dir: bool },
    Recent(String),
    Workspace(usize, String),
    Command(Action),
    /// 친 글자가 경로처럼 보이면 그 경로로
    GoPath(String),
    /// 마지막 줄: 검색 패널에서 디스크 전체로 찾기
    SearchDisk(String),
}

pub(super) struct CmdMenu {
    pub(super) items: Vec<CmdItem>,
    pub(super) sel: usize,
    pub(super) scroll: f32,
}

/// 열 곳: 지금 칸, 옆 칸(Alt+Enter), 새 탭(Ctrl+Enter)
#[derive(Clone, Copy, PartialEq)]
enum CmdOpen {
    Here,
    Other,
    NewTab,
}

/// 한글 음절을 초성으로 (다운로드 → ㄷㅇㄹㄷ). 한글이 아닌 글자는 소문자로 그대로
fn chosung(s: &str) -> String {
    const CHO: [char; 19] = ['ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ'];
    s.chars()
        .map(|c| match c as u32 {
            0xAC00..=0xD7A3 => CHO[((c as u32 - 0xAC00) / 588) as usize],
            _ => c.to_ascii_lowercase(),
        })
        .collect()
}

/// 이름이 검색어에 맞는 정도 (0: 앞부분, 1: 가운데, 2: 초성). 맞지 않으면 None. 검색어는 소문자
fn score(name: &str, q: &str) -> Option<u8> {
    if q.is_empty() {
        return Some(0);
    }
    let n = name.to_lowercase();
    if n.starts_with(q) {
        Some(0)
    } else if n.contains(q) {
        Some(1)
    } else if q.chars().all(|c| ('ㄱ'..='ㅎ').contains(&c) || c == ' ') && chosung(name).contains(q) {
        Some(2)
    } else {
        None
    }
}

/// 경로처럼 보이는 입력 (C:\…, \\서버\…, %환경 변수%)
fn looks_like_path(s: &str) -> bool {
    let b = s.as_bytes();
    s.starts_with("\\\\") || s.starts_with('%') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

impl App {
    /// Ctrl+K: 열려 있으면 닫는다. 이름 바꾸기·경로 입력 중에는 열지 않는다
    pub(super) fn toggle_commands(&mut self) {
        if self.cmd.is_some() {
            return self.close_commands();
        }
        if self.edit_mode != EditMode::None || self.settings.is_some() {
            return;
        }
        if self.view_menu.is_some() {
            self.close_view_menu();
        }
        unsafe {
            let _ = SetWindowTextW(self.cmd_edit, &HSTRING::new());
        }
        self.cmd = Some(CmdMenu { items: self.cmd_items(""), sel: 0, scroll: 0.0 });
        self.start_anim(|a| &mut a.cmd_at);
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    pub(super) fn close_commands(&mut self) {
        if self.cmd.take().is_some() {
            self.layout_edits();
            self.focus();
            self.invalidate();
        }
    }

    /// 입력이 바뀌었다
    pub(super) fn cmd_rebuild(&mut self) {
        if self.cmd.is_none() {
            return;
        }
        let items = self.cmd_items(&edit_text(self.cmd_edit));
        if let Some(c) = self.cmd.as_mut() {
            c.items = items;
            c.sel = 0;
            c.scroll = 0.0;
        }
        self.layout_edits();
        self.invalidate();
    }

    /// 보여 줄 줄: 경로 → 열린 탭 → 즐겨찾기 → 최근 위치 → 작업 공간 → 명령 → 디스크에서 찾기.
    /// 친 글자가 있으면 이름(또는 초성)이 맞는 것만, 같은 묶음 안에서는 앞부분이 맞는 것부터
    fn cmd_items(&self, text: &str) -> Vec<CmdItem> {
        let raw = text.trim();
        let q = raw.to_lowercase();
        let mut out: Vec<(u8, u8, CmdItem)> = Vec::new();
        if looks_like_path(raw) {
            out.push((0, 0, CmdItem::GoPath(raw.to_string())));
        }
        for p in 0..self.layout.count() {
            for (i, tab) in self.panes[p].tabs.iter().enumerate() {
                let title = tab.loc.title();
                if let Some(s) = score(&title, &q) {
                    out.push((1, s, CmdItem::Tab { pane: p, tab: i, title, path: tab.loc.dir().map(str::to_string) }));
                }
            }
        }
        let sections = std::iter::once(None).chain((0..self.favs.groups.len()).map(Some));
        for group in sections {
            for (index, f) in self.favs.section(group).iter().enumerate() {
                if let Some(s) = score(&f.name, &q).or_else(|| score(util::file_name(&f.path), &q)) {
                    out.push((2, s, CmdItem::Fav { group, index, name: f.name.clone(), path: f.path.clone(), is_dir: f.is_dir }));
                }
            }
        }
        let recent = self.recent.iter().take(if q.is_empty() { RECENT_SHOWN } else { usize::MAX });
        for path in recent {
            let name = util::file_name(path);
            if let Some(s) = score(if name.is_empty() { path } else { name }, &q) {
                out.push((3, s, CmdItem::Recent(path.clone())));
            }
        }
        for (i, w) in self.workspaces.list.iter().enumerate().filter(|(i, _)| Some(*i) != self.workspaces.current) {
            if let Some(s) = score(&w.name, &q) {
                out.push((4, s, CmdItem::Workspace(i, w.name.clone())));
            }
        }
        for (a, _, name, _) in ACTIONS.iter().filter(|x| x.0 != Action::Commands) {
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

    /// 카드 자리: 창 가운데, 위에서 20%
    pub(super) fn cmd_rect(&self) -> Option<R> {
        let c = self.cmd.as_ref()?;
        let w = CMD_W.min(self.w - 96.0).max(280.0);
        let rows = c.items.len().clamp(1, CMD_ROWS) as f32;
        let h = CMD_INPUT_H + 1.0 + 8.0 + rows * CMD_ROW_H + 8.0 + CMD_FOOT_H;
        let top = (self.h * 0.2).max(WORKBAR_H + 8.0).min((self.h - h - MARGIN).max(MARGIN));
        let l = (self.w - w) / 2.0;
        Some(R::new(l, top, l + w, top + h))
    }

    /// 입력 상자 자리 (돋보기 오른쪽)
    pub(super) fn cmd_input_rect(&self) -> Option<R> {
        let r = self.cmd_rect()?;
        Some(R::new(r.l + 44.0, r.t, r.r - 16.0, r.t + CMD_INPUT_H))
    }

    fn cmd_list(&self) -> Option<R> {
        let r = self.cmd_rect()?;
        Some(R::new(r.l + 8.0, r.t + CMD_INPUT_H + 1.0 + 8.0, r.r - 8.0, r.b - 8.0 - CMD_FOOT_H))
    }

    /// 명령 메뉴가 열려 있으면 무엇을 눌렀는지 (카드 밖은 닫기)
    pub(super) fn cmd_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let r = self.cmd_rect()?;
        if !r.contains(x, y) {
            return Some(Hit::CmdScrim);
        }
        let list = self.cmd_list()?;
        let c = self.cmd.as_ref()?;
        if list.contains(x, y) {
            let i = ((y - list.t + c.scroll) / CMD_ROW_H).floor();
            if i >= 0.0 && (i as usize) < c.items.len() {
                return Some(Hit::CmdRow(i as usize));
            }
        }
        Some(Hit::CmdPanel)
    }

    pub(super) fn cmd_click(&mut self, hit: Hit) {
        match hit {
            Hit::CmdRow(i) => {
                let how = if key_down(VK_CONTROL) { CmdOpen::NewTab } else if key_down(VK_MENU) { CmdOpen::Other } else { CmdOpen::Here };
                self.cmd_run(i, how);
            }
            Hit::CmdPanel => {}
            _ => self.close_commands(),
        }
    }

    pub(super) fn cmd_hover(&mut self, hit: Hit) {
        if let (Hit::CmdRow(i), Some(c)) = (hit, self.cmd.as_mut()) {
            c.sel = i;
        }
    }

    pub(super) fn cmd_wheel(&mut self, dy: f32) {
        let Some(list) = self.cmd_list() else { return };
        if let Some(c) = self.cmd.as_mut() {
            let max = (c.items.len() as f32 * CMD_ROW_H - list.h()).max(0.0);
            c.scroll = (c.scroll + dy).clamp(0.0, max);
        }
        self.invalidate();
    }

    fn cmd_ensure_visible(&mut self) {
        let Some(list) = self.cmd_list() else { return };
        if let Some(c) = self.cmd.as_mut() {
            let y = c.sel as f32 * CMD_ROW_H;
            if y < c.scroll {
                c.scroll = y;
            } else if y + CMD_ROW_H > c.scroll + list.h() {
                c.scroll = y + CMD_ROW_H - list.h();
            }
        }
    }

    /// 명령 메뉴 입력 상자에서 누른 키
    pub(super) fn cmd_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let Some(n) = self.cmd.as_ref().map(|c| c.items.len()) else { return false };
        let sel = self.cmd.as_ref().map_or(0, |c| c.sel);
        let page = CMD_ROWS.saturating_sub(1).max(1);
        let to = match vk {
            VK_UP => Some(if sel == 0 { n.saturating_sub(1) } else { sel - 1 }),
            VK_DOWN => Some(if sel + 1 >= n { 0 } else { sel + 1 }),
            VK_PRIOR => Some(sel.saturating_sub(page)),
            VK_NEXT => Some((sel + page).min(n.saturating_sub(1))),
            VK_RETURN => {
                let how = if key_down(VK_CONTROL) { CmdOpen::NewTab } else if key_down(VK_MENU) { CmdOpen::Other } else { CmdOpen::Here };
                self.cmd_run(sel, how);
                return true;
            }
            VK_ESCAPE => {
                self.close_commands();
                return true;
            }
            VK_TAB => return true,
            // Ctrl+K를 다시 누르면 닫는다
            _ if key_down(VK_CONTROL) && vk.0 == b'K' as u16 => {
                self.close_commands();
                return true;
            }
            _ => return false,
        };
        if let (Some(i), Some(c)) = (to, self.cmd.as_mut()) {
            c.sel = i;
        }
        self.cmd_ensure_visible();
        self.invalidate();
        true
    }

    /// 고른 줄을 연다·실행한다
    fn cmd_run(&mut self, i: usize, how: CmdOpen) {
        let Some(item) = self.cmd.as_ref().and_then(|c| c.items.get(i)).cloned() else { return };
        self.close_commands();
        // 옆 칸: 칸이 여럿이면 지금 칸 다음 칸에서 연다
        if how == CmdOpen::Other && self.layout.count() > 1 && !matches!(item, CmdItem::Tab { .. } | CmdItem::Command(_) | CmdItem::Workspace(..) | CmdItem::SearchDisk(_)) {
            let next = (self.active + 1) % self.layout.count();
            self.activate_pane(next);
        }
        let new_tab = how == CmdOpen::NewTab;
        match item {
            CmdItem::Tab { pane, tab, .. } => {
                if pane < self.layout.count() && tab < self.panes[pane].tabs.len() {
                    self.activate_pane(pane);
                    self.activate_tab(pane, tab);
                }
            }
            CmdItem::Fav { group, index, .. } => self.fav_open_entry(group, index, new_tab),
            CmdItem::Recent(path) => {
                let p = self.active;
                let tab = self.panes[p].tab();
                let then = ProbeThen::Favorite { tab: tab.id, from: tab.loc.clone(), new_tab, is_dir: Some(true) };
                self.probe(p, path, then);
            }
            CmdItem::GoPath(text) => {
                let p = self.active;
                let tab = self.panes[p].tab();
                let then = if new_tab { ProbeThen::Favorite { tab: tab.id, from: tab.loc.clone(), new_tab, is_dir: None } } else { ProbeThen::Address { tab: tab.id, from: tab.loc.clone() } };
                self.probe(p, text, then);
            }
            CmdItem::Workspace(i, _) => self.switch_workspace(i),
            CmdItem::Command(a) => self.run_action(a),
            CmdItem::SearchDisk(q) => {
                if !self.pal_shown() {
                    self.open_palette();
                }
                unsafe {
                    let _ = SetWindowTextW(self.search_edit, &HSTRING::from(q.as_str()));
                }
                self.pal_query_changed();
            }
        }
        self.invalidate();
    }

    pub(super) fn paint_commands(&mut self) {
        let (Some(r), Some(list)) = (self.cmd_rect(), self.cmd_list()) else { return };
        let t = self.theme;
        let (w, h) = (self.w, self.h);
        let multi = self.layout.count() > 1;
        let folder_icon = self.type_icon("", true);
        // 줄마다 아이콘 (즐겨찾기 파일은 형식 아이콘)
        let items = self.cmd.as_ref().map(|c| c.items.clone()).unwrap_or_default();
        let icons: Vec<i32> = items
            .iter()
            .map(|it| match it {
                CmdItem::Fav { path, is_dir: false, .. } => {
                    let ext = util::file_name(path).rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                    self.type_icon(&ext, false)
                }
                CmdItem::Tab { .. } | CmdItem::Fav { .. } | CmdItem::Recent(_) | CmdItem::GoPath(_) => folder_icon,
                _ => -1,
            })
            .collect();
        let keys: Vec<Option<String>> = self.cmd.as_ref().map_or_else(Vec::new, |c| {
            c.items.iter().map(|it| if let CmdItem::Command(a) = it { self.keys.get(*a).map(|k| k.label()) } else { None }).collect()
        });
        let fade = anim_progress(self.cmd_at, CMD_FADE_MS);
        let typed = !edit_text(self.cmd_edit).trim().is_empty();
        let App { gfx: g, cmd, .. } = self;
        let Some(c) = cmd.as_ref() else { return };

        // 뒤를 옅게 가린다
        g.fill(R::new(0.0, 0.0, w, h), Color { r: 0.0, g: 0.0, b: 0.0, a: if t.dark { 0.40 } else { 0.16 } });
        let pushed = fade.is_some_and(|f| g.push_fade(f, 0.0, -(1.0 - f) * 6.0));
        if !t.high_contrast {
            soft_shadow(g, r, 12.0, 16.0, 0.18, t.dark);
        }
        g.fill_round(r, 12.0, t.pane);
        g.stroke_round(r.inset(0.5, 0.5), 12.0, t.pane_border, 1.0);
        // 입력 줄: 돋보기와 (입력 상자는 자식 창), 아래 구분선
        g.glyph_font(palette::G_SEARCH, R::new(r.l + 16.0, r.t, r.l + 36.0, r.t + CMD_INPUT_H), Font::GlyphMid, t.dim);
        g.fill(R::new(r.l, r.t + CMD_INPUT_H, r.r, r.t + CMD_INPUT_H + 1.0), t.border);

        g.clip(list);
        if c.items.is_empty() {
            g.text("맞는 것이 없어요", R::new(list.l + 12.0, list.t, list.r - 12.0, list.t + CMD_ROW_H), Font::Normal, t.faint);
        }
        let first = (c.scroll / CMD_ROW_H).floor().max(0.0) as usize;
        let count = (list.h() / CMD_ROW_H).ceil() as usize + 1;
        for i in first..(first + count).min(c.items.len()) {
            let y = list.t + i as f32 * CMD_ROW_H - c.scroll;
            let row = R::new(list.l, y, list.r, y + CMD_ROW_H);
            if i == c.sel {
                g.fill_round(row, R_ROW, t.sel_inactive);
                if t.high_contrast {
                    g.stroke_round(row.inset(0.75, 0.75), R_ROW, t.focus, 1.5);
                }
            }
            let ir = R::new(row.l + 12.0, y + 10.0, row.l + 28.0, y + 26.0);
            match (&c.items[i], icons.get(i).copied().unwrap_or(-1)) {
                (CmdItem::Workspace(..), _) => g.glyph(G_WORKSPACE, ir, false, t.dim),
                (CmdItem::SearchDisk(_), _) => g.glyph_font(palette::G_SEARCH, ir, Font::GlyphMid, t.dim),
                (CmdItem::Command(_), _) => g.glyph(G_RUN, ir, true, t.dim),
                (_, icon) if icon >= 0 => g.icon(icon, ir),
                _ => {}
            }
            // 이름, 그 뒤에 흐린 위치, 오른쪽에 종류(명령이면 단축키)
            let (name, detail, kind): (String, String, String) = match &c.items[i] {
                CmdItem::Tab { pane, title, path, .. } => (title.clone(), path.clone().unwrap_or_default(), if multi { format!("{}번 칸 탭", pane + 1) } else { "열린 탭".to_string() }),
                CmdItem::Fav { name, path, .. } => (name.clone(), util::parent(path).unwrap_or_default(), "즐겨찾기".to_string()),
                CmdItem::Recent(path) => {
                    let n = util::file_name(path);
                    (if n.is_empty() { path.clone() } else { n.to_string() }, util::parent(path).unwrap_or_default(), "최근 위치".to_string())
                }
                CmdItem::Workspace(_, name) => (name.clone(), String::new(), "작업 공간".to_string()),
                CmdItem::Command(a) => (crate::keys::name(*a).to_string(), String::new(), String::new()),
                CmdItem::GoPath(p) => (p.clone(), String::new(), "경로로 이동".to_string()),
                CmdItem::SearchDisk(q) => (format!("‘{q}’ 디스크에서 찾기"), String::new(), "검색".to_string()),
            };
            let right = match keys.get(i).cloned().flatten() {
                Some(k) => {
                    let kw = g.measure(&k, Font::Small) + 10.0;
                    palette::kbd(g, &t, row.r - 10.0 - kw, (row.t + row.b) / 2.0, &k);
                    row.r - 10.0 - kw - 8.0
                }
                None if !kind.is_empty() => {
                    let kw = g.measure(&kind, Font::Small);
                    g.text(&kind, R::new(row.r - 12.0 - kw, row.t, row.r - 12.0, row.b), Font::Small, t.faint);
                    row.r - 12.0 - kw - 12.0
                }
                None => row.r - 12.0,
            };
            let nw = g.measure(&name, Font::Normal).min((right - row.l - 40.0) * 0.6).max(0.0);
            g.text(&name, R::new(row.l + 40.0, row.t, (row.l + 40.0 + nw + 2.0).min(right), row.b), Font::Normal, t.text);
            if !detail.is_empty() && row.l + 40.0 + nw + 12.0 < right - 24.0 {
                g.text(&detail, R::new(row.l + 40.0 + nw + 12.0, row.t, right, row.b), Font::Path, t.faint);
            }
        }
        g.unclip();

        // 아래: 키 안내
        let foot = R::new(r.l + 16.0, r.b - CMD_FOOT_H, r.r - 16.0, r.b);
        g.fill(R::new(r.l, foot.t, r.r, foot.t + 1.0), t.border);
        let cy = (foot.t + foot.b) / 2.0;
        let mut x = foot.l;
        for (k, label) in [("Enter", if typed { "열기·실행" } else { "열기" }), ("Alt+Enter", "옆 칸"), ("Ctrl+Enter", "새 탭")] {
            x = palette::kbd(g, &t, x, cy, k) + 6.0;
            let lw = g.measure(label, Font::Small);
            if x + lw > foot.r {
                break;
            }
            g.text(label, R::new(x, foot.t, x + lw + 2.0, foot.b), Font::Small, t.dim);
            x += lw + 16.0;
        }
        if pushed {
            g.pop_fade();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn recognizes_paths() {
        assert!(looks_like_path("C:\\Users"));
        assert!(looks_like_path("\\\\nas\\자료"));
        assert!(looks_like_path("%USERPROFILE%"));
        assert!(!looks_like_path("견적서"));
    }
}
