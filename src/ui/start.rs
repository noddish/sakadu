//! 새 탭 시작 화면 ("어디로 이동할까요?"): 고정한 폴더(이 공간에 고정한 것과 즐겨찾기) 칸, 최근 위치, 드라이브(Windows, 내 PC 포함)·위치(맥).
//! 칸과 줄에는 이름만 보인다. 자리(start_layout), 키보드로 옮길 이웃(neighbor), 그리기(paint_start)를 두 앱이 함께 쓴다.
//! 아이콘은 각 앱이 그린다 (StartArt). 최근 위치 목록(세션의 recent)을 고치는 규칙도 여기 있다 (명령 메뉴도 이 목록을 쓴다).
//! 자리는 start_layout 한 곳에서 정해 그리기·누르기·키보드·스크롤이 같이 쓴다

use super::{Canvas, PaneGeom, hover_fill, scroll_metrics, seg_track};
use crate::draw::{Color, Font, R};
use crate::favorites::{Favorite, Favorites};
use crate::palette::{FOCUS_W, R_CTRL, R_ROW, Theme, alpha};
use crate::pane::Location;
use crate::util;

/// 세션에 남기는 최근 위치 수와 시작 화면에 보이는 수
pub const RECENT_KEEP: usize = 20;
pub const RECENT_SHOWN: usize = 8;
/// 내용 좌우 여백과 가장 넓을 때의 폭 (넓은 칸에서 카드가 한없이 늘어나지 않게)
const PAD: f32 = 16.0;
const MAX_W: f32 = 760.0;
const CARD_MIN_W: f32 = 148.0;
/// 칸: 아이콘과 이름 한 줄 (드라이브는 아래에 사용량 막대)
const CARD_H: f32 = 40.0;
const CARD_GAP: f32 = 12.0;
const CARD_R: f32 = R_CTRL;
/// 최근 위치 줄: 파일 목록과 같은 높이
const LINE_H: f32 = 28.0;
const TITLE_H: f32 = 20.0;
/// 시작 화면 영역이 이보다 낮으면 제목을 그리지 않는다
const TITLE_MIN_AREA_H: f32 = 360.0;
const SECTION_H: f32 = 16.0;
const SECTION_GAP: f32 = 24.0;
const NOTE_H: f32 = 20.0;

/// 시작 화면에서 고를 수 있는 것
#[derive(Clone, PartialEq, Debug)]
pub enum StartItem {
    /// 즐겨찾기 폴더 (보이는 이름, 경로)
    Fav { name: String, path: String },
    Recent(String),
    /// 내 PC (Windows)
    Computer,
    /// StartData::drives의 번호
    Drive(usize),
}

/// 시작 화면 배치. 좌표는 지금 스크롤이 반영된 화면 좌표
pub struct StartView {
    pub title: R,
    pub heads: Vec<(&'static str, R)>,
    pub items: Vec<(StartItem, R)>,
    /// 즐겨찾기가 없을 때·찾는 글자와 맞는 게 없을 때의 안내
    pub notes: Vec<(String, R)>,
    /// 스크롤과 상관없는 내용 전체 높이
    pub height: f32,
    pub scroll: f32,
    pub area: R,
}

/// 드라이브(맥은 위치) 칸 하나: 이름과 남은·전체 바이트 (모르면 total 0: 막대 없이)
pub struct StartDrive<'a> {
    pub name: &'a str,
    pub free: u64,
    pub total: u64,
}

/// 시작 화면에 놓을 것 (앱이 자기 상태로 채운다)
pub struct StartData<'a> {
    /// 지금 작업 공간에 고정한 것
    pub pins: &'a [Favorite],
    /// 즐겨찾기 ('모든 공간')
    pub favs: &'a Favorites,
    /// 최근 위치 (최근 것이 앞)
    pub recent: &'a [String],
    /// 내 PC 칸을 둘지 (Windows)
    pub computer: bool,
    pub drives: &'a [StartDrive<'a>],
    /// 드라이브 구역의 이름 (Windows "드라이브", 맥 "위치")
    pub drives_label: &'static str,
}

/// 키보드로 옮길 쪽
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nav {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// 칸 안에서 시작 화면이 놓이는 곳 (경로 줄 아래 전부)
pub fn start_area(g: &PaneGeom) -> R {
    R::new(g.list.l, g.header.t, g.list.r, g.list.b)
}

/// 최근 위치의 보이는 이름 (드라이브 뿌리는 "C:")
pub fn recent_label(path: &str) -> &str {
    let n = util::file_name(path);
    if n.is_empty() { path } else { n }
}

/// 최근 위치·시작 화면에서 두 경로를 같은 폴더로 볼 때 대소문자를 가리지 않을지: Windows는 가리지 않는다.
/// 맥은 글자 그대로 본다. 대소문자를 가리는 볼륨에서는 Work와 work가 다른 폴더다 (맥 앱의 다른 경로 비교와 같다)
const FOLD_CASE: bool = cfg!(windows);

/// 두 경로가 같은 폴더인지 (fold_case: 대소문자를 가리지 않는다. 시험이 두 규칙을 다 보려고 따로 받는다)
fn same_dir(fold_case: bool, a: &str, b: &str) -> bool {
    if fold_case { util::eq_path(a, b) } else { a == b }
}

/// 다녀간 폴더를 최근 위치 맨 앞에 둔다 (같은 폴더는 한 번만, RECENT_KEEP개까지). 폴더가 아니면 그대로 두고 false
pub fn note_recent(list: &mut Vec<String>, loc: &Location) -> bool {
    note_recent_in(FOLD_CASE, list, loc)
}

fn note_recent_in(fold_case: bool, list: &mut Vec<String>, loc: &Location) -> bool {
    let Location::Dir(d) = loc else { return false };
    list.retain(|r| !same_dir(fold_case, r, d));
    list.insert(0, d.clone());
    list.truncate(RECENT_KEEP);
    true
}

/// 최근 위치에서 뺀다 (메뉴·지우기 키, 또는 지워진 폴더)
pub fn forget_recent(list: &mut Vec<String>, path: &str) {
    forget_recent_in(FOLD_CASE, list, path);
}

fn forget_recent_in(fold_case: bool, list: &mut Vec<String>, path: &str) {
    list.retain(|r| !same_dir(fold_case, r, path));
}

/// 카드를 cols개씩 줄지어 놓고 y를 마지막 줄 아래로 옮긴다
fn place_cards(list: Vec<StartItem>, x0: f32, y: &mut f32, cols: usize, card_w: f32, out: &mut Vec<(StartItem, R)>) {
    let rows = list.len().div_ceil(cols);
    for (i, it) in list.into_iter().enumerate() {
        let l = x0 + (i % cols) as f32 * (card_w + CARD_GAP);
        let t = *y + (i / cols) as f32 * (CARD_H + CARD_GAP);
        out.push((it, R::new(l, t, l + card_w, t + CARD_H)));
    }
    if rows > 0 {
        *y += rows as f32 * (CARD_H + CARD_GAP) - CARD_GAP;
    }
}

/// 화살표 방향의 다음 항목: ←→는 앞뒤, ↑↓는 바로 위·아래 줄에서 가로 가운데가 가장 가까운 것, Home·End는 처음·끝
pub fn neighbor(rects: &[R], from: usize, nav: Nav) -> usize {
    let Some(cur) = rects.get(from) else { return 0 };
    match nav {
        Nav::Left => from.saturating_sub(1),
        Nav::Right => (from + 1).min(rects.len() - 1),
        Nav::Home => 0,
        Nav::End => rects.len() - 1,
        Nav::Up | Nav::Down => {
            let down = nav == Nav::Down;
            // 지금 줄과 겹치지 않는 것 중 가장 가까운 줄
            let row_t = rects
                .iter()
                .filter(|r| if down { r.t >= cur.b - 1.0 } else { r.b <= cur.t + 1.0 })
                .map(|r| r.t)
                .reduce(|a, b| if down { a.min(b) } else { a.max(b) });
            let Some(row_t) = row_t else { return from };
            let cx = (cur.l + cur.r) / 2.0;
            let off = |r: &R| ((r.l + r.r) / 2.0 - cx).abs();
            rects
                .iter()
                .enumerate()
                .filter(|(_, r)| (r.t - row_t).abs() < 1.0)
                .min_by(|(_, a), (_, b)| off(a).total_cmp(&off(b)))
                .map_or(from, |(i, _)| i)
        }
    }
}

/// 제목 → 고정한 폴더 카드 → 최근 위치 줄 → 드라이브(위치) 카드. 필터 글자가 있으면 이름에 그 글자가 든 것만.
/// area는 시작 화면 자리(start_area), scroll은 탭의 스크롤
pub fn start_layout(area: R, filter: &str, scroll: f32, d: &StartData) -> StartView {
    let needle = filter.to_lowercase();
    let fits = |label: &str| needle.is_empty() || label.to_lowercase().contains(&needle);
    let w = (area.w() - PAD * 2.0).clamp(0.0, MAX_W);
    let x0 = area.l + PAD;
    let top = area.t - scroll;
    let mut y = top + 16.0;
    let title = if area.h() >= TITLE_MIN_AREA_H { R::new(x0, y, x0 + w, y + TITLE_H) } else { R::new(x0, y, x0 + w, y) };
    if title.h() > 0.0 {
        y += TITLE_H + 12.0;
    }
    let cols = (((w + CARD_GAP) / (CARD_MIN_W + CARD_GAP)).floor() as usize).clamp(1, 4);
    let card_w = ((w - CARD_GAP * (cols - 1) as f32) / cols as f32).max(0.0);
    let mut heads = Vec::new();
    let mut items = Vec::new();
    let mut notes = Vec::new();
    // 이 공간에 고정한 폴더가 먼저, 그 뒤에 '모든 공간'(즐겨찾기)의 폴더. 둘 다에 있으면 한 번만
    let pins = d.pins;
    let fav_dirs = || {
        let common = d.favs.items.iter().chain(d.favs.groups.iter().flat_map(|g| g.items.iter())).filter(|f| !pins.iter().any(|p| same_dir(FOLD_CASE, &p.path, &f.path)));
        pins.iter().chain(common).filter(|f| f.is_dir)
    };

    let favs: Vec<StartItem> = fav_dirs().filter(|f| fits(&f.name)).map(|f| StartItem::Fav { name: f.name.clone(), path: f.path.clone() }).collect();
    if !favs.is_empty() || needle.is_empty() {
        heads.push(("고정한 곳", R::new(x0, y, x0 + w, y + SECTION_H)));
        y += SECTION_H + 4.0;
        if favs.is_empty() {
            notes.push(("경로 오른쪽 책갈피로 고정한 폴더가 여기에 카드로 보여요".to_string(), R::new(x0, y, x0 + w, y + NOTE_H)));
            y += NOTE_H;
        }
        place_cards(favs, x0, &mut y, cols, card_w, &mut items);
        y += SECTION_GAP;
    }

    // 즐겨찾기에 있는 폴더는 위 카드로 이미 보이니 빼고
    let recent: Vec<StartItem> = d
        .recent
        .iter()
        .filter(|r| !fav_dirs().any(|f| same_dir(FOLD_CASE, &f.path, r)))
        .filter(|r| fits(recent_label(r)))
        .take(RECENT_SHOWN)
        .map(|r| StartItem::Recent(r.clone()))
        .collect();
    if !recent.is_empty() {
        heads.push(("최근 위치", R::new(x0, y, x0 + w, y + SECTION_H)));
        y += SECTION_H + 4.0;
        for it in recent {
            items.push((it, R::new(x0, y, x0 + w, y + LINE_H)));
            y += LINE_H;
        }
        y += SECTION_GAP;
    }

    let mut drives = Vec::new();
    if d.computer && fits("내 PC") {
        drives.push(StartItem::Computer);
    }
    drives.extend(d.drives.iter().enumerate().filter(|(_, dr)| fits(dr.name)).map(|(i, _)| StartItem::Drive(i)));
    if !drives.is_empty() {
        heads.push((d.drives_label, R::new(x0, y, x0 + w, y + SECTION_H)));
        y += SECTION_H + 4.0;
        place_cards(drives, x0, &mut y, cols, card_w, &mut items);
        y += SECTION_GAP;
    }

    if items.is_empty() && !needle.is_empty() {
        notes.push((format!("필터 ‘{filter}’에 맞는 위치가 없어요"), R::new(x0, y, x0 + w, y + NOTE_H)));
        y += NOTE_H;
    }
    StartView { title, heads, items, notes, height: y - top, scroll, area }
}

/// 시작 화면의 아이콘을 그리는 쪽 (Windows: 시스템 아이콘 목록, 맥: Finder 아이콘)
pub trait StartArt<G: Canvas> {
    /// 항목의 16×16 아이콘
    fn icon(&mut self, g: &mut G, t: &Theme, item: &StartItem, r: R);
}

/// 그리기. drives: 배치할 때 쓴 드라이브(위치), hovered: 마우스가 올라간 항목, focus: 키보드 초점 테두리를 그릴 항목
pub fn paint_start<G: Canvas>(g: &mut G, art: &mut impl StartArt<G>, t: &Theme, v: &StartView, drives: &[StartDrive], hovered: Option<usize>, focus: Option<usize>) {
    g.clip(v.area);
    if v.title.h() > 0.0 {
        g.text("어디로 이동할까요?", v.title, Font::Title, t.text);
    }
    for (label, r) in &v.heads {
        g.text(label, *r, Font::Label, t.faint);
    }
    for (s, r) in &v.notes {
        g.text(s, *r, Font::Small, t.faint);
    }
    let focus = focus.map(|f| f.min(v.items.len().saturating_sub(1)));
    for (i, (it, r)) in v.items.iter().enumerate() {
        let r = *r;
        if r.b < v.area.t || r.t > v.area.b {
            continue;
        }
        let hot = hovered == Some(i);
        let iy = r.t + (r.h() - 16.0) / 2.0;
        let ic = R::new(r.l + 12.0, iy, r.l + 28.0, iy + 16.0);
        let radius = if let StartItem::Recent(path) = it {
            if hot {
                g.fill_round(r, R_ROW, hover_fill(t));
            }
            art.icon(g, t, it, ic);
            let name = recent_label(path);
            let nw = (g.measure(name, Font::Normal) + 4.0).min(r.w() * 0.6);
            g.text(name, R::new(r.l + 36.0, r.t, r.l + 36.0 + nw, r.b), Font::Normal, t.text);
            // 어디 있는 폴더인지는 마우스를 올리거나 키보드로 고른 줄에만 (길면 가운데를 줄인다)
            if (hot || focus == Some(i))
                && let Some(parent) = util::parent(path)
            {
                g.text(&parent, R::new(r.l + 36.0 + nw + 12.0, r.t, r.r - 8.0, r.b), Font::Path, t.faint);
            }
            R_ROW
        } else {
            // 칸: 옅은 면에 아이콘과 이름 한 줄, 올리면 한 단계 진한 면 (고대비에서는 테두리)
            g.fill_round(r, CARD_R, seg_track(t));
            if hot {
                g.fill_round(r, CARD_R, hover_fill(t));
            }
            if t.high_contrast {
                g.stroke_round(r.inset(0.5, 0.5), CARD_R, t.border, 1.0);
            }
            let name_r = R::new(r.l + 36.0, r.t, r.r - 12.0, r.b);
            match it {
                StartItem::Fav { name, .. } => {
                    art.icon(g, t, it, ic);
                    g.text(name, name_r, Font::Normal, t.text);
                }
                StartItem::Computer => {
                    art.icon(g, t, it, ic);
                    g.text("내 PC", name_r, Font::Normal, t.text);
                }
                StartItem::Drive(di) => {
                    if let Some(d) = drives.get(*di) {
                        art.icon(g, t, it, ic);
                        g.text(d.name, name_r, Font::Normal, t.text);
                        if d.total > 0 {
                            // 쓴 만큼 채운 막대. 거의 찼으면(90% 넘게) 빨간색
                            let used = (1.0 - d.free as f32 / d.total as f32).clamp(0.0, 1.0);
                            let bar = R::new(r.l + 12.0, r.b - 8.0, r.r - 12.0, r.b - 4.0);
                            g.fill_round(bar, 2.0, alpha(t.text, 0.10));
                            let fill = if used > 0.9 { Color { r: 0.80, g: 0.20, b: 0.16, a: 1.0 } } else { t.faint };
                            g.fill_round(R::new(bar.l, bar.t, bar.l + bar.w() * used, bar.b), 2.0, fill);
                        }
                    }
                }
                StartItem::Recent(_) => {}
            }
            CARD_R
        };
        if focus == Some(i) {
            g.stroke_round(r.inset(FOCUS_W / 2.0, FOCUS_W / 2.0), radius, t.focus, FOCUS_W);
        }
    }
    g.unclip();
    // 넘치면 목록처럼 오른쪽에 스크롤 손잡이 (보이기만 한다. 휠·Page Up/Down으로 움직인다)
    if let Some((_, thumb)) = scroll_metrics(v.area, v.height, v.scroll) {
        g.fill_round(thumb, 3.0, t.thumb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 카드 세 개짜리 두 줄(마지막 줄은 하나) 아래에 넓은 줄 두 개
    fn sample() -> Vec<R> {
        let card = |c: f32, row: f32| R::new(c * 110.0, row * 70.0, c * 110.0 + 100.0, row * 70.0 + 60.0);
        vec![card(0.0, 0.0), card(1.0, 0.0), card(2.0, 0.0), card(0.0, 1.0), R::new(0.0, 150.0, 320.0, 182.0), R::new(0.0, 182.0, 320.0, 214.0)]
    }

    #[test]
    fn arrows_move_between_cards_and_rows() {
        let r = sample();
        assert_eq!(neighbor(&r, 1, Nav::Right), 2);
        assert_eq!(neighbor(&r, 5, Nav::Right), 5, "stops at the end");
        assert_eq!(neighbor(&r, 0, Nav::Left), 0, "stops at the start");
        assert_eq!(neighbor(&r, 1, Nav::Down), 3, "the only card below");
        assert_eq!(neighbor(&r, 2, Nav::Down), 3);
        assert_eq!(neighbor(&r, 3, Nav::Down), 4, "down into the rows");
        assert_eq!(neighbor(&r, 4, Nav::Down), 5);
        assert_eq!(neighbor(&r, 5, Nav::Down), 5, "nothing below the last row");
        assert_eq!(neighbor(&r, 4, Nav::Up), 3);
        assert_eq!(neighbor(&r, 3, Nav::Up), 0, "the card straight above");
        assert_eq!(neighbor(&r, 0, Nav::Up), 0);
        assert_eq!(neighbor(&r, 3, Nav::End), 5);
        assert_eq!(neighbor(&r, 4, Nav::Home), 0);
        assert_eq!(neighbor(&[], 0, Nav::Down), 0, "nothing to move to");
    }

    #[test]
    fn recent_label_keeps_drive_roots_readable() {
        if cfg!(windows) {
            assert_eq!(recent_label("C:\\Users\\a\\문서"), "문서");
            assert_eq!(recent_label("D:\\"), "D:");
        } else {
            assert_eq!(recent_label("/Users/a/문서"), "문서");
            assert_eq!(recent_label("/"), "/");
        }
    }

    /// 최근 위치: 다녀간 폴더가 맨 앞, 같은 폴더는 한 번만, RECENT_KEEP개까지. 폴더가 아니면 남기지 않는다.
    /// Windows 규칙(대소문자만 다르면 같은 폴더)으로 본다
    #[test]
    fn recent_list_keeps_the_latest_once() {
        let mut list = Vec::new();
        for i in 0..RECENT_KEEP + 3 {
            assert!(note_recent_in(true, &mut list, &Location::Dir(format!("/r/{i}"))));
        }
        assert_eq!(list.len(), RECENT_KEEP);
        assert_eq!(list[0], format!("/r/{}", RECENT_KEEP + 2));
        assert!(note_recent_in(true, &mut list, &Location::Dir("/R/5".into())));
        assert_eq!(list[0], "/R/5");
        assert_eq!(list.iter().filter(|p| p.eq_ignore_ascii_case("/r/5")).count(), 1);
        assert!(!note_recent_in(true, &mut list, &Location::Start));
        assert_eq!(list[0], "/R/5");
        forget_recent_in(true, &mut list, "/r/5");
        assert!(!list.iter().any(|p| p.eq_ignore_ascii_case("/r/5")));
    }

    /// 맥 규칙: 대소문자를 가리는 볼륨에서는 Work와 work가 다른 폴더라 둘 다 남고, 빼기도 글자 그대로 맞는 것만 뺀다.
    /// 앱이 쓰는 note_recent·forget_recent는 Windows에서 Windows 규칙, 맥에서 맥 규칙을 따른다
    #[test]
    fn recent_list_case_rule_follows_the_platform() {
        let mut list = Vec::new();
        note_recent_in(false, &mut list, &Location::Dir("/x/Work".into()));
        note_recent_in(false, &mut list, &Location::Dir("/x/work".into()));
        assert_eq!(list, ["/x/work", "/x/Work"]);
        note_recent_in(false, &mut list, &Location::Dir("/x/Work".into()));
        assert_eq!(list, ["/x/Work", "/x/work"], "the same spelling still moves to the front once");
        forget_recent_in(false, &mut list, "/x/WORK");
        assert_eq!(list.len(), 2, "no folder spelled that way");
        forget_recent_in(false, &mut list, "/x/work");
        assert_eq!(list, ["/x/Work"]);

        let mut list = Vec::new();
        note_recent(&mut list, &Location::Dir("/x/Work".into()));
        note_recent(&mut list, &Location::Dir("/x/work".into()));
        assert_eq!(list.len(), if cfg!(windows) { 1 } else { 2 });
        forget_recent(&mut list, "/x/WORK");
        assert_eq!(list.len(), if cfg!(windows) { 0 } else { 2 });
    }

    /// 고정한 폴더는 카드로 한 번만, 최근 위치는 고정한 폴더를 빼고, 드라이브 구역 이름은 앱이 정한다. 필터에 맞는 것만
    #[test]
    fn layout_places_pins_recent_and_drives() {
        let f = |name: &str, path: &str, is_dir: bool| Favorite { name: name.into(), path: path.into(), is_dir };
        let pins = [f("업무", "/w/업무", true), f("메모.md", "/w/메모.md", false)];
        let favs = Favorites { items: vec![f("업무", "/w/업무", true), f("사진", "/w/사진", true)], groups: Vec::new() };
        let recent = ["/w/업무".to_string(), "/w/다운로드".to_string()];
        let drives = [StartDrive { name: "홈", free: 0, total: 0 }, StartDrive { name: "외장 디스크", free: 1, total: 4 }];
        let d = StartData { pins: &pins, favs: &favs, recent: &recent, computer: false, drives: &drives, drives_label: "위치" };
        let area = R::new(0.0, 0.0, 700.0, 600.0);
        let v = start_layout(area, "", 0.0, &d);
        let kinds: Vec<&StartItem> = v.items.iter().map(|(it, _)| it).collect();
        assert_eq!(
            kinds,
            [
                &StartItem::Fav { name: "업무".into(), path: "/w/업무".into() },
                &StartItem::Fav { name: "사진".into(), path: "/w/사진".into() },
                &StartItem::Recent("/w/다운로드".into()),
                &StartItem::Drive(0),
                &StartItem::Drive(1),
            ]
        );
        assert_eq!(v.heads.iter().map(|h| h.0).collect::<Vec<_>>(), ["고정한 곳", "최근 위치", "위치"]);
        assert!(v.title.h() > 0.0 && v.height > 0.0 && v.height < area.h());
        // 필터: 이름에 그 글자가 든 것만, 없으면 안내
        let v = start_layout(area, "디스크", 0.0, &d);
        assert_eq!(v.items.iter().map(|(it, _)| it.clone()).collect::<Vec<_>>(), [StartItem::Drive(1)]);
        let v = start_layout(area, "없는 이름", 0.0, &d);
        assert!(v.items.is_empty() && v.notes.len() == 1);
        // 스크롤하면 모두 그만큼 위로
        let a = start_layout(area, "", 0.0, &d);
        let b = start_layout(area, "", 30.0, &d);
        assert_eq!(a.items[2].1.t - 30.0, b.items[2].1.t);
    }

    /// 고정한 폴더와 최근 위치에서 겹친 것 빼기도 같은 폴더 규칙을 따른다: Windows는 대소문자만 다르면 하나로,
    /// 맥은 Work와 work를 다른 폴더로 둘 다 보인다
    #[test]
    fn layout_dedup_follows_the_case_rule() {
        let f = |name: &str, path: &str| Favorite { name: name.into(), path: path.into(), is_dir: true };
        let pins = [f("Work", "/w/Work")];
        let favs = Favorites { items: vec![f("work", "/w/work")], groups: Vec::new() };
        let recent = ["/w/WORK".to_string()];
        let d = StartData { pins: &pins, favs: &favs, recent: &recent, computer: false, drives: &[], drives_label: "위치" };
        let v = start_layout(R::new(0.0, 0.0, 700.0, 600.0), "", 0.0, &d);
        let kinds: Vec<StartItem> = v.items.into_iter().map(|(it, _)| it).collect();
        let work = StartItem::Fav { name: "Work".into(), path: "/w/Work".into() };
        if cfg!(windows) {
            assert_eq!(kinds, [work]);
        } else {
            assert_eq!(kinds, [work, StartItem::Fav { name: "work".into(), path: "/w/work".into() }, StartItem::Recent("/w/WORK".into())]);
        }
    }
}
