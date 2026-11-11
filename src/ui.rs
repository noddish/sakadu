//! 화면 계산과 그리기 도우미: Windows와 맥이 함께 쓴다 (치수·규칙이 한 벌이라 두 화면이 어긋나지 않는다).
//! 그리는 방법은 `Canvas`로 받는다 (Windows Direct2D `gfx.rs`, 맥 Core Graphics `mac/gfx.rs`)

use crate::draw::{Color, Font, R};
use crate::pane::{Location, SortKey, ViewMode};
use crate::palette::Theme;
use crate::util;

/// 공통 화면 코드가 쓰는 그리기 (각 OS의 Gfx가 같은 이름의 메서드로 구현한다)
pub trait Canvas {
    fn fill(&self, r: R, c: Color);
    fn fill_round(&self, r: R, radius: f32, c: Color);
    fn stroke_round(&self, r: R, radius: f32, c: Color, width: f32);
    fn text(&self, s: &str, r: R, f: Font, c: Color);
    fn measure(&self, s: &str, f: Font) -> f32;
    /// 아이콘 글자 (Segoe 번호, 맥은 SF Symbols로 바꿔 그린다). small은 10px, 아니면 16px
    fn glyph(&self, code: u16, r: R, small: bool, c: Color);
    fn glyph_font(&self, code: u16, r: R, f: Font, c: Color);
}

/// 아이콘 글자 번호 (Segoe Fluent Icons / Segoe MDL2 Assets)
pub mod glyph {
    pub const G_BACK: u16 = 0xE72B;
    pub const G_FWD: u16 = 0xE72A;
    pub const G_UP: u16 = 0xE74A;
    pub const G_ADD: u16 = 0xE710;
    pub const G_CLOSE: u16 = 0xE8BB;
    pub const G_CHEVRON: u16 = 0xE76C;
    pub const G_STAR_FILL: u16 = 0xE735;
    pub const G_SORT_UP: u16 = 0xE70E;
    pub const G_SORT_DOWN: u16 = 0xE70D;
    pub const G_NEW_FOLDER: u16 = 0xE8F4;
    pub const G_HIDDEN: u16 = 0xE7B3;
    pub const G_FILTER: u16 = 0xE71C;
    pub const G_LIST: u16 = 0xE8FD;
    pub const G_GRID: u16 = 0xF0E2;
    pub const G_CHEVRON_DOWN: u16 = 0xE70D;
    pub const G_SETTINGS: u16 = 0xE713;
    pub const G_MORE: u16 = 0xE712;
    pub const G_STAR: u16 = 0xE734;
    pub const G_SEARCH: u16 = 0xE721;
    pub const G_PREVIEW: u16 = 0xE90D;
    pub const G_PLACES: u16 = 0xE8B7;
    pub const G_PIN: u16 = 0xE718;
    /// 채운 핀의 안쪽 (바늘이 없어 G_PIN을 겹쳐 그린다: draw_pin)
    pub const G_PIN_FILL: u16 = 0xE841;
}
use glyph::*;

/// 칸 영역 위 작업 줄 (칸 나누기·숨김·미리 보기·창 단추). 빈 곳은 창을 끄는 제목 줄 역할도 한다
pub const WORKBAR_H: f32 = 44.0;
pub const MARGIN: f32 = 8.0;
pub const GAP: f32 = 8.0;
/// 칸 맨 위 칩 줄 (탭 칩, 새 탭, 오른쪽에 항목 수 또는 필터 칩)
pub const HEAD_H: f32 = 44.0;
/// 칩 줄 아래 경로 줄 (뒤로·앞으로·위로, 경로, 별, 새 폴더, 보기)
pub const PATH_H: f32 = 36.0;
pub const HEADER_H: f32 = 28.0;
pub const ROW_H: f32 = 28.0;
pub const SCROLL_W: f32 = 10.0;
/// 칸 카드의 모서리
pub const RADIUS: f32 = 12.0;
pub const CHEV_W: f32 = 16.0;
pub const CRUMB_PAD: f32 = 8.0;

// app/sidebar.rs의 치수
/// 아이콘 막대 너비. 패널이나 칸 영역은 그 오른쪽부터
pub const BAR_W: f32 = 44.0;
pub const SIDE_ROW_H: f32 = 32.0;
/// 그룹 안 항목의 들여쓰기
pub const INDENT: f32 = 20.0;
/// 사이드바 패널 머리 줄 (제목, 작은 단추, 핀)
pub const SIDE_HEADER_H: f32 = 44.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    One,
    TwoV,
    TwoH,
    Three,
    Four,
}

impl Layout {
    pub const ALL: [Layout; 5] = [Layout::One, Layout::TwoV, Layout::TwoH, Layout::Three, Layout::Four];

    pub fn count(self) -> usize {
        match self {
            Layout::One => 1,
            Layout::TwoV | Layout::TwoH => 2,
            Layout::Three => 3,
            Layout::Four => 4,
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&l| l == self).unwrap_or(0)
    }
}

pub fn split_points(a: R, split: (f32, f32)) -> (f32, f32) {
    (a.l + (a.w() - GAP) * split.0, a.t + (a.h() - GAP) * split.1)
}

pub fn pane_rects(a: R, split: (f32, f32), layout: Layout) -> Vec<(usize, R)> {
    let (xm, ym) = split_points(a, split);
    match layout {
        Layout::One => vec![(0, a)],
        Layout::TwoV => vec![(0, R::new(a.l, a.t, xm, a.b)), (1, R::new(xm + GAP, a.t, a.r, a.b))],
        Layout::TwoH => vec![(0, R::new(a.l, a.t, a.r, ym)), (1, R::new(a.l, ym + GAP, a.r, a.b))],
        Layout::Three => vec![
            (0, R::new(a.l, a.t, xm, a.b)),
            (1, R::new(xm + GAP, a.t, a.r, ym)),
            (2, R::new(xm + GAP, ym + GAP, a.r, a.b)),
        ],
        Layout::Four => vec![
            (0, R::new(a.l, a.t, xm, ym)),
            (1, R::new(xm + GAP, a.t, a.r, ym)),
            (2, R::new(a.l, ym + GAP, xm, a.b)),
            (3, R::new(xm + GAP, ym + GAP, a.r, a.b)),
        ],
    }
}

pub fn splitters(a: R, split: (f32, f32), layout: Layout) -> (Option<R>, Option<R>) {
    let (xm, ym) = split_points(a, split);
    let sx = matches!(layout, Layout::TwoV | Layout::Three | Layout::Four).then(|| R::new(xm, a.t, xm + GAP, a.b));
    let sy = match layout {
        Layout::TwoH | Layout::Four => Some(R::new(a.l, ym, a.r, ym + GAP)),
        Layout::Three => Some(R::new(xm + GAP, ym, a.r, ym + GAP)),
        _ => None,
    };
    (sx, sy)
}

pub struct PaneGeom {
    /// 칩 줄 (탭 칩, 새 탭, 오른쪽 항목 수·필터 칩)
    pub head: R,
    pub back: R,
    pub fwd: R,
    pub up: R,
    /// 경로 글자 자리 (주소 입력 상자도 여기에 놓인다)
    pub crumb: R,
    /// 경로 조각을 그리는 곳 (오른쪽 끝 별을 뺀 자리)
    pub crumbs: R,
    /// 경로 오른쪽 끝: 즐겨찾기 별
    pub star: R,
    pub new_folder: R,
    /// 보기 방식 버튼 (지금 보기 그림 + 아래 화살표)
    pub view: R,
    pub header: R,
    pub list: R,
}

pub fn geom(r: R) -> PaneGeom {
    let head = R::new(r.l, r.t, r.r, r.t + HEAD_H);
    let path = R::new(r.l, head.b, r.r, head.b + PATH_H);
    let (t, b) = (path.t + 4.0, path.b - 4.0);
    let view = R::new(path.r - 8.0 - 40.0, t, path.r - 8.0, b);
    let new_folder = R::new(view.l - 4.0 - 28.0, t, view.l - 4.0, b);
    // 별은 새 폴더 옆의 단추 (다른 경로 줄 단추와 같은 28×28)
    let mut star = R::new(new_folder.l - 4.0 - 28.0, t, new_folder.l - 4.0, b);
    // 좁은 칸: 경로가 PATH_MIN_W보다 좁아지면 뒤로·앞으로를 빼고(위로만 남김), 그래도 좁으면 별도 뺀다.
    // 빠진 단추는 폭 0 (그리지도 누르지도 않는다). 뒤로·앞으로는 Alt+←→, 별은 Ctrl+D로 된다
    let compact = star.l - 4.0 - (path.l + 8.0 + 28.0 * 3.0 + 4.0 + 8.0) < PATH_MIN_W;
    let hidden = |x: f32| R::new(x, t, x, b);
    let (back, fwd, up) = if compact {
        let up = R::new(path.l + 8.0, t, path.l + 36.0, b);
        (hidden(up.l), hidden(up.l), up)
    } else {
        let back = R::new(path.l + 8.0, t, path.l + 36.0, b);
        let fwd = R::new(back.r + 2.0, t, back.r + 30.0, b);
        (back, fwd, R::new(fwd.r + 2.0, t, fwd.r + 30.0, b))
    };
    if star.l - 4.0 - (up.r + 8.0) < PATH_MIN_W {
        star = hidden(new_folder.l - 4.0);
    }
    let crumb = R::new(up.r + 8.0, path.t + 4.0, (star.l - 4.0).max(up.r + 8.0), path.b - 4.0);
    let crumbs = crumb;
    let header = R::new(r.l, path.b, r.r, path.b + HEADER_H);
    // 아래쪽은 둥근 모서리에 항목이 닿지 않게 조금 띄운다
    let list = R::new(r.l, header.b, r.r, (r.b - 6.0).max(header.b));
    PaneGeom { head, back, fwd, up, crumb, crumbs, star, new_folder, view, header, list }
}

/// 경로 입력 상자 자리: 경로 자리가 너무 좁으면 새 폴더·보기 단추 자리까지 덮는다
pub fn addr_rect(g: &PaneGeom) -> R {
    if g.crumb.w() >= ADDR_MIN_W { g.crumb } else { R::new(g.up.r + 8.0, g.crumb.t, g.view.r, g.crumb.b) }
}

/// 경로 입력 상자의 가장 좁은 폭
pub const ADDR_MIN_W: f32 = 80.0;

/// 경로 조각에 남길 가장 좁은 폭. 이보다 좁아지면 경로 줄 단추를 덜어 낸다
pub const PATH_MIN_W: f32 = 96.0;

/// 필터 칩 자리 (칩 줄 오른쪽 끝)
pub fn filter_chip(g: &PaneGeom) -> R {
    let w = (g.head.w() * 0.4).clamp(150.0, 240.0).min((g.head.w() - 22.0).max(0.0));
    R::new(g.head.r - 8.0 - w, g.head.t + 8.0, g.head.r - 8.0, g.head.t + 36.0)
}

/// 칩 줄에서 탭 칩과 새 탭(+)이 놓일 곳: 왼쪽 여백 8, 오른쪽은 항목 수·필터 칩 자리(right)를 뺀 곳
pub fn chip_strip(g: &PaneGeom, right: f32) -> R {
    let l = g.head.l + 8.0;
    R::new(l, g.head.t + 8.0, (g.head.r - 8.0 - right - 8.0).max(l + 100.0), g.head.t + 36.0)
}

pub struct Cols {
    pub name: (f32, f32),
    pub modified: Option<(f32, f32)>,
    pub kind: Option<(f32, f32)>,
    pub size: Option<(f32, f32)>,
}

impl Cols {
    /// 너비 조절 가능한 열(수정한 날짜, 유형, 크기)의 범위
    pub fn resizable(&self) -> [Option<(f32, f32)>; 3] {
        [self.modified, self.kind, self.size]
    }
}

/// 좁은 칸에서는 유형 → 크기 → 날짜 순으로 열을 숨긴다 (보고서는 날짜로 찾으니 날짜를 가장 오래 남긴다)
pub fn columns(list: R, w: [f32; 3]) -> Cols {
    let mut x = list.r - SCROLL_W - 6.0;
    let width = list.w();
    let size = if width >= 460.0 {
        let c = (x - w[2], x);
        x -= w[2];
        Some(c)
    } else {
        None
    };
    let kind = if width >= 590.0 {
        let c = (x - w[1], x);
        x -= w[1];
        Some(c)
    } else {
        None
    };
    let modified = if width >= 300.0 {
        let c = (x - w[0], x);
        x -= w[0];
        Some(c)
    } else {
        None
    };
    Cols { name: (list.l + 12.0, x.max(list.l + 124.0)), modified, kind, size }
}

/// 자세히 보기가 아닌 보기의 정렬 버튼 (기준, 이름, 자리). 머리글 줄 왼쪽부터 나란히 둔다.
/// 열이 없는 격자 위에 자세히 보기의 열 머리글이 떠 있지 않게 한다. 지금 기준에만 화살표 자리가 붙는다
pub fn sort_chips(gfx: &impl Canvas, header: R, current: SortKey) -> Vec<(SortKey, &'static str, R)> {
    const KEYS: [(SortKey, &str); 4] = [(SortKey::Name, "이름"), (SortKey::Modified, "수정한 날짜"), (SortKey::Type, "유형"), (SortKey::Size, "크기")];
    let mut x = header.l + 8.0;
    KEYS.iter()
        .map(|&(key, label)| {
            let w = gfx.measure(label, Font::Normal) + if key == current { 40.0 } else { 24.0 };
            let r = R::new(x, header.t + 4.0, x + w, header.b - 4.0);
            x = r.r + 4.0;
            (key, label, r)
        })
        // 좁은 칸에서는 다 들어가는 칩만 (끝에서 잘린 칩은 무엇인지 읽히지 않는다)
        .take_while(|(_, _, r)| r.r <= header.r - 8.0)
        .collect()
}

/// 줄어든 탭 칩의 가장 좁은 폭
pub const CHIP_MIN: f32 = 64.0;

/// 탭 칩 자리. 다 들어가면 칩마다 이름 길이(natural)대로 둔다. 넘치면 지금 탭(active)은 제 폭을 지키고
/// 나머지가 남는 폭을 고르게 나눈다(짧은 이름은 늘리지 않는다). 그래도 넘치면 지금 탭이 보이는 만큼만 보이고
/// 나머지는 폭 0으로 양 끝에 둔다(그리기·누르기에서 빠짐). 새 탭(+)은 늘 보이는 칩 바로 뒤
pub fn tab_rects(strip: R, natural: &[f32], active: usize) -> Vec<R> {
    const GAP_X: f32 = 2.0;
    let n = natural.len();
    let avail = strip.w() - 32.0;
    let total = natural.iter().sum::<f32>() + GAP_X * n.saturating_sub(1) as f32;
    let place = |widths: &mut dyn Iterator<Item = (usize, f32)>, first: usize, shown: usize| -> Vec<R> {
        let mut out = vec![R::new(strip.l, strip.t, strip.l, strip.b); n];
        let mut x = strip.l;
        for (i, w) in widths {
            out[i] = R::new(x, strip.t, x + w, strip.b);
            x += w + GAP_X;
        }
        // 가려진 칩: 앞쪽은 첫 칩 왼쪽 끝, 뒤쪽은 마지막 칩 오른쪽 끝에 폭 0으로
        let end = (x - GAP_X).max(strip.l);
        for (i, r) in out.iter_mut().enumerate() {
            if i >= first + shown {
                *r = R::new(end, strip.t, end, strip.b);
            }
        }
        out
    };
    if n == 0 {
        return Vec::new();
    }
    if total <= avail {
        return place(&mut natural.iter().copied().enumerate(), 0, n);
    }
    let active = active.min(n - 1);
    // 지금 탭은 제 폭 (다른 칩 하나 놓을 자리는 남기고)
    let cur_w = natural[active].min((avail - CHIP_MIN - GAP_X).max(CHIP_MIN)).min(avail.max(0.0));
    let rest = avail - cur_w - GAP_X;
    // 나머지 칩을 가장 좁게 줄였을 때 몇 개 들어가나 (나눗셈 오차로 하나 덜 들어간다고 보지 않게 조금 여유를 둔다)
    let others = (((rest + GAP_X) / (CHIP_MIN + GAP_X) + 0.001).floor().max(0.0) as usize).min(n - 1);
    let shown = others + 1;
    let first = (active + 1).saturating_sub(shown).min(n - shown);
    let w_other = if others > 0 { ((rest + GAP_X) / others as f32 - GAP_X).min(180.0) } else { 0.0 };
    place(&mut (first..first + shown).map(|i| (i, if i == active { cur_w } else { natural[i].min(w_other) })), first, shown)
}

/// 탭 줄의 x에 놓았을 때 들어갈 자리. 보이는 탭만 보고 정한다:
/// 가려진 탭(폭 0)까지 세면 마지막으로 보이는 탭 바로 뒤에 놓아도 맨 끝으로 가 버린다
pub fn drop_index(trs: &[R], x: f32) -> usize {
    let visible: Vec<usize> = (0..trs.len()).filter(|&i| trs[i].w() > 0.0).collect();
    match visible.iter().rev().find(|&&i| (trs[i].l + trs[i].r) / 2.0 < x) {
        Some(&i) => i + 1,
        None => visible.first().copied().unwrap_or(0),
    }
}

/// 탭 칩 안 오른쪽 끝의 닫기 단추
pub fn close_rect(tr: R) -> R {
    R::new(tr.r - 24.0, tr.t + 4.0, tr.r - 4.0, tr.b - 4.0)
}

pub fn add_rect(strip: R, tabs: &[R]) -> R {
    let l = tabs.iter().map(|r| r.r + 4.0).fold(strip.l, f32::max);
    R::new(l, strip.t, l + 28.0, strip.b)
}

/// 큰 아이콘 보기의 그림 영역(DIP). 보통·아주 큰 아이콘은 VIEW_THUMBS
pub const THUMB: f32 = 96.0;
/// 작은 아이콘 보기 칸 너비(남는 폭은 칸에 나눈다), 목록 보기 열 너비
pub const SMALL_W: f32 = 240.0;
pub const LIST_W: f32 = 260.0;

/// 한 칸에 무엇을 그리나
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CellKind {
    /// 자세히: 한 줄에 이름·날짜·유형·크기
    Row,
    /// 아이콘 보기: 그림(이 크기의 정사각형, DIP) 아래 이름 두 줄
    Tile(f32),
    /// 작은 아이콘·목록: 작은 아이콘 옆에 이름
    Icon,
}

/// 목록 배치. 그리기·클릭·스크롤·키 이동·네모 고르기가 모두 이것을 쓴다.
/// 목록 보기(horiz)만 위에서 아래로 채운 뒤 옆 열로 넘어가고 가로로 스크롤한다. 나머지는 왼쪽에서 오른쪽으로 채우고 세로로 스크롤
#[derive(Clone, Copy)]
pub struct Grid {
    pub ox: f32,
    pub oy: f32,
    /// 한 줄의 칸 수 (세로 스크롤)
    pub cols: usize,
    /// 한 열의 칸 수 (가로 스크롤)
    pub rows: usize,
    pub cell_w: f32,
    pub cell_h: f32,
    pub cell: CellKind,
    pub horiz: bool,
}

impl Grid {
    pub fn of(list: R, view: ViewMode) -> Grid {
        let tile = |thumb: f32, min_w: f32| {
            let usable = (list.w() - SCROLL_W - 14.0).max(min_w);
            let cols = ((usable / min_w).floor() as usize).max(1);
            // 남는 폭을 칸에 고르게 나눠 오른쪽이 비지 않게
            Grid { ox: list.l + 4.0, oy: list.t + 4.0, cols, rows: 0, cell_w: usable / cols as f32, cell_h: thumb + 52.0, cell: CellKind::Tile(thumb), horiz: false }
        };
        match view {
            // 줄 바탕은 머리글 구분선과 같은 8px 안쪽에서 시작한다 (칸 테두리에 붙어 보이지 않게)
            ViewMode::Details => Grid { ox: list.l + 8.0, oy: list.t, cols: 1, rows: 0, cell_w: list.w() - SCROLL_W - 14.0, cell_h: ROW_H, cell: CellKind::Row, horiz: false },
            ViewMode::Huge => tile(256.0, 280.0),
            ViewMode::Large => tile(THUMB, 124.0),
            ViewMode::Medium => tile(48.0, 88.0),
            ViewMode::Small => {
                let usable = (list.w() - SCROLL_W - 14.0).max(SMALL_W);
                let cols = ((usable / SMALL_W).floor() as usize).max(1);
                Grid { ox: list.l + 8.0, oy: list.t + 2.0, cols, rows: 0, cell_w: usable / cols as f32, cell_h: ROW_H, cell: CellKind::Icon, horiz: false }
            }
            ViewMode::List => {
                // 아래에 가로 스크롤 막대 자리를 남긴다
                let rows = (((list.h() - 6.0 - SCROLL_W - 4.0) / ROW_H).floor() as usize).max(1);
                Grid { ox: list.l + 8.0, oy: list.t + 2.0, cols: 0, rows, cell_w: LIST_W, cell_h: ROW_H, cell: CellKind::Icon, horiz: true }
            }
        }
    }

    /// 스크롤 방향으로 잰 전체 내용 길이
    pub fn content(&self, n: usize) -> f32 {
        let pad = if matches!(self.cell, CellKind::Tile(_)) { 12.0 } else if self.cell == CellKind::Icon { 4.0 } else { 0.0 };
        if self.horiz { n.div_ceil(self.rows) as f32 * self.cell_w + pad + 8.0 } else { n.div_ceil(self.cols) as f32 * self.cell_h + pad }
    }

    /// 스크롤 방향으로 잰 보이는 길이
    pub fn page(&self, list: R) -> f32 {
        if self.horiz { list.w() } else { list.h() }
    }

    /// 칸 위치 (열, 줄)
    pub fn pos(&self, vi: usize) -> (usize, usize) {
        if self.horiz { (vi / self.rows, vi % self.rows) } else { (vi % self.cols, vi / self.cols) }
    }

    pub fn item(&self, vi: usize, scroll: f32) -> R {
        let (c, r) = self.pos(vi);
        let (sx, sy) = if self.horiz { (scroll, 0.0) } else { (0.0, scroll) };
        let x = self.ox + c as f32 * self.cell_w - sx;
        let y = self.oy + r as f32 * self.cell_h - sy;
        R::new(x, y, x + self.cell_w, y + self.cell_h)
    }

    pub fn at(&self, x: f32, y: f32, scroll: f32, n: usize) -> Option<usize> {
        let (sx, sy) = if self.horiz { (scroll, 0.0) } else { (0.0, scroll) };
        let (fx, fy) = ((x - self.ox + sx) / self.cell_w, (y - self.oy + sy) / self.cell_h);
        if fx < 0.0 || fy < 0.0 {
            return None;
        }
        let (c, r) = (fx as usize, fy as usize);
        let vi = if self.horiz {
            if r >= self.rows {
                return None;
            }
            c * self.rows + r
        } else {
            if c >= self.cols {
                return None;
            }
            r * self.cols + c
        };
        (vi < n).then_some(vi)
    }

    /// 화면에 보이는 항목 범위
    pub fn visible(&self, list: R, scroll: f32, n: usize) -> std::ops::Range<usize> {
        if self.horiz {
            let first = ((scroll - (self.ox - list.l)) / self.cell_w).floor().max(0.0) as usize;
            let cols = (list.w() / self.cell_w).ceil() as usize + 1;
            return (first * self.rows).min(n)..((first + cols) * self.rows).min(n);
        }
        let first_row = ((scroll - (self.oy - list.t)) / self.cell_h).floor().max(0.0) as usize;
        let rows = (list.h() / self.cell_h).ceil() as usize + 1;
        (first_row * self.cols).min(n)..((first_row + rows) * self.cols).min(n)
    }

    /// 스크롤 막대 (트랙, 손잡이). 넘치지 않으면 None
    pub fn bar(&self, list: R, n: usize, scroll: f32) -> Option<(R, R)> {
        let content = self.content(n);
        if !self.horiz {
            return scroll_metrics(list, content, scroll);
        }
        let view = list.w();
        if content <= view || view <= 0.0 {
            return None;
        }
        let track = R::new(list.l + 2.0, list.b - SCROLL_W - 2.0, list.r - 2.0, list.b - 2.0);
        let tw = (view / content * track.w()).max(24.0).min(track.w());
        let x = track.l + (track.w() - tw) * (scroll / (content - view)).clamp(0.0, 1.0);
        Some((track, R::new(x, track.t + 2.0, x + tw, track.b - 2.0)))
    }

    /// 스크롤 방향 좌표 (가로 스크롤이면 x)
    pub fn along(&self, x: f32, y: f32) -> f32 {
        if self.horiz { x } else { y }
    }

    /// 손잡이의 스크롤 방향 시작·길이, 트랙의 시작·길이
    pub fn bar_span(&self, track: R, thumb: R) -> (f32, f32, f32, f32) {
        if self.horiz { (thumb.l, thumb.w(), track.l, track.w()) } else { (thumb.t, thumb.h(), track.t, track.h()) }
    }
}

/// 목록이 넘칠 때 (트랙, 손잡이). content는 전체 내용 높이
pub fn scroll_metrics(list: R, content: f32, scroll: f32) -> Option<(R, R)> {
    let view = list.h();
    if content <= view || view <= 0.0 {
        return None;
    }
    let track = R::new(list.r - SCROLL_W - 2.0, list.t + 2.0, list.r - 2.0, list.b - 2.0);
    let th = (view / content * track.h()).max(24.0).min(track.h());
    let y = track.t + (track.h() - th) * (scroll / (content - view)).clamp(0.0, 1.0);
    Some((track, R::new(track.l + 2.0, y, track.r - 2.0, y + th)))
}

pub struct Crumb {
    pub x0: f32,
    pub x1: f32,
    pub label: String,
    pub target: Location,
}

/// 경로 표시줄 조각. 공간이 모자라면 앞쪽을 "…"로 접는다. root는 맨 앞 조각 이름 (Windows "내 PC", 맥은 없음)
pub fn crumbs(gfx: &impl Canvas, area: R, loc: &Location, root: Option<&str>) -> Vec<Crumb> {
    // 시작 화면은 "새 탭" 한 조각
    let mut items = match loc {
        Location::Start => vec![(loc.title(), Location::Start)],
        _ => root.map(|r| vec![(r.to_string(), Location::Computer)]).unwrap_or_default(),
    };
    if let Location::Dir(path) = loc {
        for (label, full) in util::segments(path) {
            items.push((label, Location::Dir(full)));
        }
    }
    if items.is_empty() {
        return Vec::new();
    }
    let widths: Vec<f32> = items.iter().map(|(l, _)| gfx.measure(l, Font::Normal) + CRUMB_PAD * 2.0).collect();
    let avail = area.w() - 8.0;
    let mut start = items.len() - 1;
    let mut total = widths[start];
    // "…"까지 넣으면 지금 폴더 이름이 60보다 좁아지는 칸이면 "…" 없이 지금 폴더만 (잘려도 이름이 보이게)
    let room_with_ellipsis = avail - (24.0 + CHEV_W);
    if start > 0 && room_with_ellipsis < widths[start].min(60.0) {
        let w = widths[start].min(avail.max(0.0));
        let (label, target) = items.swap_remove(start);
        return vec![Crumb { x0: area.l + 4.0, x1: area.l + 4.0 + w, label, target }];
    }
    while start > 0 {
        let ellipsis = if start - 1 > 0 { 24.0 + CHEV_W } else { 0.0 };
        if total + CHEV_W + widths[start - 1] + ellipsis > avail {
            break;
        }
        start -= 1;
        total += CHEV_W + widths[start];
    }
    let mut out = Vec::new();
    let mut x = area.l + 4.0;
    if start > 0 {
        out.push(Crumb { x0: x, x1: x + 24.0, label: "…".into(), target: items[start - 1].1.clone() });
        x += 24.0 + CHEV_W;
    }
    for (i, (label, target)) in items.into_iter().enumerate().skip(start) {
        let w = widths[i].min((area.r - 4.0 - x).max(0.0));
        out.push(Crumb { x0: x, x1: x + w, label, target });
        x += w + CHEV_W;
    }
    out
}

/// 아래로 부드럽게 번지는 그림자. 흐림 효과 대신 조금씩 내려가며 커지는 아주 옅은 둥근 사각형을 겹쳐,
/// 카드 가장자리에서 멀어질수록 옅어지게 한다. depth: 아래로 번지는 길이(DIP), strength: 가장자리 바로 밑의 진하기
pub fn soft_shadow(g: &impl Canvas, r: R, radius: f32, depth: f32, strength: f32, dark: bool) {
    const LAYERS: usize = 8;
    let (base, k) = if dark { ((0.0, 0.0, 0.0), 2.5) } else { ((0.06, 0.09, 0.16), 1.0) };
    let ink = Color { r: base.0, g: base.1, b: base.2, a: (strength * k).min(0.7) / LAYERS as f32 };
    for i in 1..=LAYERS {
        let f = i as f32 / LAYERS as f32;
        let spread = depth * (f * 0.5 - 0.25);
        let dy = depth * f * 0.6;
        g.fill_round(R::new(r.l - spread, r.t + dy - spread, r.r + spread, r.b + dy + spread), radius + spread.max(0.0), ink);
    }
    // 가장자리에 붙은 얇은 그림자 (카드가 바닥에 닿은 곳)
    g.fill_round(R::new(r.l - 0.5, r.t + 0.5, r.r + 0.5, r.b + 1.5), radius + 0.5, Color { a: if dark { 0.25 } else { 0.05 }, ..ink });
}

pub fn draw_layout_icon(g: &impl Canvas, r: R, layout: Layout, c: Color) {
    let (cx, cy) = ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0);
    let ic = R::new(cx - 9.0, cy - 7.0, cx + 9.0, cy + 7.0);
    g.stroke_round(ic, 2.0, c, 1.3);
    let v = |x: f32, t: f32, b: f32| g.fill(R::new(x - 0.65, t, x + 0.65, b), c);
    let h = |y: f32, l: f32, rr: f32| g.fill(R::new(l, y - 0.65, rr, y + 0.65), c);
    match layout {
        Layout::One => {}
        Layout::TwoV => v(cx, ic.t, ic.b),
        Layout::TwoH => h(cy, ic.l, ic.r),
        Layout::Three => {
            v(cx, ic.t, ic.b);
            h(cy, cx, ic.r);
        }
        Layout::Four => {
            v(cx, ic.t, ic.b);
            h(cy, ic.l, ic.r);
        }
    }
}

/// 창 바탕 위에 한 단계 들어간 면 (칸 나누기 판, 지금 위치 줄)
pub fn well(t: &Theme) -> Color {
    if t.high_contrast { t.window } else { t.sel_inactive }
}

/// 고르기 판(칸 나누기, 검색 범위, 설정의 고르기)의 들어간 바탕. 어두운 테마는 바탕보다 어둡게 겹친다
pub fn seg_track(t: &Theme) -> Color {
    if t.high_contrast {
        t.window
    } else if t.dark {
        Color { r: 0.0, g: 0.0, b: 0.0, a: 0.28 }
    } else {
        t.sel_inactive
    }
}

/// 고르기 판에서 고른 칸·켜진 단추: 밝은 테마는 흰 면 + 아주 작은 그림자, 어두운 테마는 한 단계 밝은 면 + 위쪽 빛 선 (떠 보이게)
pub fn seg_on(g: &impl Canvas, r: R, radius: f32, t: &Theme) {
    if t.high_contrast {
        g.fill_round(r, radius, t.pane);
        g.stroke_round(r.inset(0.5, 0.5), radius, t.border, 1.0);
    } else if t.dark {
        g.fill_round(r, radius, t.sel_inactive);
        g.fill(R::new(r.l + radius, r.t + 0.5, r.r - radius, r.t + 1.5), Color { r: 1.0, g: 1.0, b: 1.0, a: 0.10 });
    } else {
        soft_shadow(g, r, radius, 3.0, 0.12, false);
        g.fill_round(r, radius, t.pane);
    }
}

/// 줄의 마우스 올림 바탕: 창 바탕에서도 보이도록 반투명으로 겹친다
pub fn hover_fill(t: &Theme) -> Color {
    if t.high_contrast {
        t.hover
    } else if t.dark {
        Color { r: 1.0, g: 1.0, b: 1.0, a: 0.06 }
    } else {
        Color { r: 0.06, g: 0.09, b: 0.16, a: 0.05 }
    }
}

/// 핀 그림. 고정했으면 채운 핀(E841)에 빈 핀(E718)을 겹쳐 바늘까지 그린다 (E841만으로는 바늘 없는 덩어리)
pub fn draw_pin(g: &impl Canvas, r: R, pinned: bool, c: Color) {
    if pinned {
        g.glyph(G_PIN_FILL, r, false, c);
    }
    g.glyph(G_PIN, r, false, c);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 모든 보기에서 칸 가운데를 누르면 그 항목이 잡히고, 보이는 범위에 들어 있고, 경로 줄 오른쪽 보기 버튼과 겹치지 않는다
    #[test]
    fn every_view_maps_items_and_clicks_consistently() {
        let g = geom(R::new(0.0, 0.0, 700.0, 520.0));
        assert!(g.new_folder.l > g.crumb.r && g.view.l > g.new_folder.r && g.view.r <= g.head.r);
        assert!(g.crumb.r <= g.star.l && g.star.r <= g.new_folder.l && g.back.w() > 0.0);
        // 좁은 칸: 뒤로·앞으로부터 빠지고, 경로는 단추와 겹치지 않는다
        for w in [150.0, 200.0, 240.0, 280.0, 330.0] {
            let g = geom(R::new(0.0, 0.0, w, 400.0));
            assert!(g.crumb.r <= g.new_folder.l - 4.0 + 0.01, "w={w}");
            assert!(g.star.w() == 0.0 || g.crumb.r <= g.star.l, "w={w}");
            assert!(g.crumb.l >= g.up.r, "w={w}");
        }
        assert!(geom(R::new(0.0, 0.0, 240.0, 400.0)).back.w() == 0.0);
        let cols = |w: f32| columns(R::new(0.0, 0.0, w, 400.0), [140.0, 130.0, 84.0]);
        assert!(cols(420.0).modified.is_some() && cols(420.0).size.is_none(), "date outlives size");
        assert!(cols(600.0).kind.is_some() && cols(600.0).size.is_some());
        assert!(cols(280.0).modified.is_none() && cols(280.0).name.1 > cols(280.0).name.0);
        let n = 137;
        for view in ViewMode::ALL {
            let grid = Grid::of(g.list, view);
            let page = grid.page(g.list);
            let max = (grid.content(n) - page).max(0.0);
            for scroll in [0.0, max / 2.0, max] {
                let shown = grid.visible(g.list, scroll, n);
                for vi in 0..n {
                    let it = grid.item(vi, scroll);
                    let (cx, cy) = ((it.l + it.r) / 2.0, (it.t + it.b) / 2.0);
                    if !g.list.contains(cx, cy) {
                        continue;
                    }
                    assert_eq!(grid.at(cx, cy, scroll, n), Some(vi), "{view:?} scroll={scroll} vi={vi}");
                    assert!(shown.contains(&vi), "{view:?} scroll={scroll} vi={vi} not in {shown:?}");
                }
            }
            // 마지막 항목까지 스크롤하면 끝이 보인다
            let last = grid.item(n - 1, max);
            if grid.horiz {
                assert!(last.r <= g.list.r + 0.5 && last.b <= g.list.b - SCROLL_W, "{view:?} {last:?}");
            } else {
                assert!(last.b <= g.list.b + 0.5, "{view:?} {last:?}");
            }
        }
    }

    /// 목록 보기는 위에서 아래로 채운 뒤 옆 열로 넘어간다 (탐색기와 같다)
    #[test]
    fn list_view_fills_columns_top_to_bottom() {
        let g = geom(R::new(0.0, 0.0, 700.0, 520.0));
        let grid = Grid::of(g.list, ViewMode::List);
        assert!(grid.horiz && grid.rows > 1);
        let (a, b, c) = (grid.item(0, 0.0), grid.item(1, 0.0), grid.item(grid.rows, 0.0));
        assert_eq!(a.l, b.l);
        assert!(b.t > a.t);
        assert_eq!(c.t, a.t);
        assert!(c.l > a.l);
        // 가로로 스크롤하면 왼쪽으로 밀린다
        assert_eq!(grid.item(0, 50.0).l, a.l - 50.0);
    }

    /// 좁은 칸에 탭이 많아도 지금 탭과 새 탭(+)은 칩 줄 안(오른쪽 항목 수 자리 앞)에 보이고 누를 수 있다
    #[test]
    fn many_tabs_keep_active_and_add_reachable() {
        let g = geom(R::new(0.0, 0.0, 380.0, 600.0));
        let strip = chip_strip(&g, 60.0);
        for (n, active) in [(3, 1), (12, 0), (12, 6), (12, 11), (40, 39)] {
            let trs = tab_rects(strip, &vec![120.0; n], active);
            let a = trs[active];
            assert!(a.w() > 0.0 && a.l >= strip.l && a.r <= strip.r, "active tab visible: n={n} active={active} {a:?}");
            let add = add_rect(strip, &trs);
            assert!(add.r <= strip.r, "add button inside strip: n={n} {add:?}");
            let (x, y) = ((a.l + a.r) / 2.0, (a.t + a.b) / 2.0);
            assert_eq!(trs.iter().filter(|r| r.contains(x, y)).count(), 1, "one tab under the cursor");
        }
        // 다 들어가면 칩마다 이름 길이대로 차례로
        let trs = tab_rects(strip, &[60.0, 90.0], 0);
        assert!(trs.windows(2).all(|w| w[0].r < w[1].l));
        assert_eq!((trs[0].w(), trs[1].w()), (60.0, 90.0));
        // 넘치면 지금 탭은 제 폭을 지키고 (이름이 잘리지 않게), 나머지가 줄어든다
        let natural = [150.0, 150.0, 150.0, 150.0, 150.0, 150.0];
        let trs = tab_rects(strip, &natural, 3);
        assert_eq!(trs[3].w(), 150.0, "{trs:?}");
        assert!(trs.iter().enumerate().all(|(i, r)| i == 3 || r.w() == 0.0 || (r.w() >= CHIP_MIN && r.w() < 150.0)), "{trs:?}");
        assert!(trs.iter().filter(|r| r.w() > 0.0).all(|r| r.r <= strip.r - 32.0 + 0.5), "{trs:?}");
        // 짧은 이름의 칩은 늘리지 않는다
        let trs = tab_rects(strip, &[150.0, 70.0, 150.0, 150.0], 0);
        assert!(trs[1].w() <= 70.0);
        // 필터 칩은 칩 줄 안, 칩 자리와 겹치지 않는다
        let chip = filter_chip(&g);
        assert!(chip.r <= g.head.r && chip_strip(&g, chip.w()).r <= chip.l);
        // 아주 좁은 칸에서도 필터 칩은 칸 왼쪽 여백 밖으로 나가지 않는다
        for w in [60.0, 120.0, 204.0] {
            let g = geom(R::new(100.0, 0.0, 100.0 + w, 600.0));
            let chip = filter_chip(&g);
            assert!(chip.l >= g.head.l + 8.0 - 0.01 && chip.r <= g.head.r, "w={w} {chip:?}");
        }
    }

    /// 회귀: 탭이 가려져 있어도 놓은 자리(보이는 탭 사이)에 들어간다
    #[test]
    fn drop_position_counts_only_visible_tabs() {
        let g = geom(R::new(0.0, 0.0, 380.0, 600.0));
        let strip = chip_strip(&g, 60.0);
        let trs = tab_rects(strip, &[120.0; 12], 6);
        let visible: Vec<usize> = (0..12).filter(|&i| trs[i].w() > 0.0).collect();
        let (first, last) = (visible[0], *visible.last().unwrap());
        assert!(first > 0 && last < 11, "both sides hidden: {visible:?}");
        // 마지막으로 보이는 탭 바로 뒤 → 그 탭 다음 자리 (맨 끝이 아니라)
        assert_eq!(drop_index(&trs, trs[last].r + 1.0), last + 1);
        // 첫 보이는 탭 앞 → 그 탭 자리 (맨 앞이 아니라)
        assert_eq!(drop_index(&trs, trs[first].l + 1.0), first);
        // 보이는 두 탭 사이
        assert_eq!(drop_index(&trs, trs[first + 1].l), first + 1);
        // 다 보일 때는 예전과 같다
        let all = tab_rects(strip, &[60.0; 3], 0);
        assert_eq!((drop_index(&all, 0.0), drop_index(&all, all[2].r + 1.0)), (0, 3));
    }
}
