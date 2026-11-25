//! 화면 계산과 그리기 도우미: Windows와 맥이 함께 쓴다 (치수·규칙이 한 벌이라 두 화면이 어긋나지 않는다).
//! 그리는 방법은 `Canvas`로 받는다 (Windows Direct2D `gfx.rs`, 맥 Core Graphics `mac/gfx.rs`)

use crate::draw::{Color, Font, R};
use crate::pane::{Location, SortKey, ViewMode};
use crate::palette::{Theme, alpha, mix};
use crate::util;

pub mod commands;
pub mod menu;
pub mod paint;
pub mod preview_card;
pub mod rail;
pub mod side_flow;
pub mod start;
pub mod toolbar;

/// 공통 화면 코드가 쓰는 그리기 (각 OS의 Gfx가 같은 이름의 메서드로 구현한다)
pub trait Canvas {
    fn fill(&self, r: R, c: Color);
    fn fill_round(&self, r: R, radius: f32, c: Color);
    fn stroke_round(&self, r: R, radius: f32, c: Color, width: f32);
    fn text(&self, s: &str, r: R, f: Font, c: Color);
    /// 파일 이름처럼 UTF-16으로 가진 글자 (Windows는 바꾸지 않고 바로 그린다)
    fn text_w(&self, s: &[u16], r: R, f: Font, c: Color) {
        self.text(&String::from_utf16_lossy(s), r, f, c)
    }
    fn measure(&self, s: &str, f: Font) -> f32;
    /// 파일 이름을 폭 max_w에 맞춘 글자와 줄였는지: 자모로 나뉜 한글은 합쳐 보이고(typo::display_name),
    /// 넘치면 가운데를 줄여 확장자를 남긴다(typo::middle_ellipsis). 그리는 쪽이 결과를 캐시해도 된다
    fn fit_name(&self, name: &[u16], max_w: f32) -> (String, bool) {
        let raw = String::from_utf16_lossy(name);
        let shown = crate::typo::display_name(&raw);
        crate::typo::fit_name(&shown, max_w, |s| self.measure(s, Font::Name))
    }
    /// 아이콘 글자 (Segoe 번호, 맥은 SF Symbols로 바꿔 그린다). small은 10px, 아니면 16px
    fn glyph(&self, code: u16, r: R, small: bool, c: Color);
    fn glyph_font(&self, code: u16, r: R, f: Font, c: Color);
    /// r 밖에는 그리지 않는다 (unclip까지)
    fn clip(&self, r: R);
    fn unclip(&self);
}

/// 아이콘 글자 번호 (Segoe Fluent Icons / Segoe MDL2 Assets)
pub mod glyph {
    pub const G_BACK: u16 = 0xE72B;
    pub const G_FWD: u16 = 0xE72A;
    pub const G_UP: u16 = 0xE74A;
    /// 도구 막대의 뒤로: 왼쪽 꺾쇠 (앞으로는 G_CHEVRON, 위로는 G_SORT_UP)
    pub const G_CHEVRON_LEFT: u16 = 0xE76B;
    pub const G_ADD: u16 = 0xE710;
    pub const G_CLOSE: u16 = 0xE8BB;
    pub const G_CHEVRON: u16 = 0xE76C;
    pub const G_CHECK: u16 = 0xE73E;
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
    /// 책갈피(즐겨찾기), 빈 것과 채운 것. Segoe 아이콘 글꼴에 없는 모양이라 글꼴에 없는 번호를 쓴다:
    /// Windows는 이 번호를 받으면 직접 그리고, 맥은 SF Symbols로 그린다
    pub const G_BOOKMARK: u16 = 0xFFF0;
    pub const G_BOOKMARK_FILL: u16 = 0xFFF1;
    /// 눈 (자동 미리 보기)
    pub const G_EYE: u16 = 0xE890;
    /// 감은 눈 (숨김 항목을 숨겨 둔 상태)
    pub const G_EYE_OFF: u16 = 0xED1A;
    /// 자동 미리 보기 단추 (Segoe의 '미리 보기')
    pub const G_AUTO_PREVIEW: u16 = 0xE8A1;
    /// 크게 보기·작게 보기 (미리 보기 카드)
    pub const G_EXPAND: u16 = 0xE740;
    pub const G_SHRINK: u16 = 0xE73F;
}
use glyph::*;

// ---------- 디자인 단계 (docs/design-rules.md 2장). 그리기 코드는 숫자 대신 이 이름을 쓴다 ----------

/// 간격: 붙어 있는 것 사이 (선택 줄 틈, 둥근 판의 바깥 띠)
pub const SP_HAIR: f32 = 2.0;
/// 간격: 한 덩어리 안
pub const SP_XS: f32 = 4.0;
/// 간격: 같은 묶음의 이웃 (아이콘과 글자, 줄 안쪽 여백)
pub const SP_S: f32 = 8.0;
/// 간격: 줄·카드 안 글 시작
pub const SP_M: f32 = 12.0;
/// 간격: 서로 다른 묶음 사이
pub const SP_L: f32 = 16.0;
/// 간격: 큰 묶음 사이 (구역 이름 위)
pub const SP_XL: f32 = 24.0;
/// 기본 아이콘 크기
pub const ICON: f32 = 16.0;
/// 투명도: 마우스를 올린 겹침
pub const A_HOVER: f32 = 0.06;
/// 투명도: 누르는 중·끌어 놓을 자리
pub const A_PRESS: f32 = 0.10;
/// 투명도: 꺼진 것, 연결이 끊긴 목록 덮개
pub const A_DISABLED: f32 = 0.40;
/// 칸 안 목록의 오른쪽 여백: 스크롤 막대(10)와 틈 6. 줄 바탕·구분선·열이 여기서 끝난다
pub const LIST_R_INSET: f32 = SP_L;
/// 그림자: 칸·카드 (깊이, 세기)
pub const SHADOW_S: (f32, f32) = (6.0, 0.06);
/// 그림자: 떠 있는 판 (메뉴, 끄는 카드, 떠 있는 사이드바)
pub const SHADOW_M: (f32, f32) = (16.0, 0.12);

/// 칸 영역 위 작업 줄 (칸 나누기·숨김·미리 보기·창 단추). 빈 곳은 창을 끄는 제목 줄 역할도 한다
/// 창 위쪽 도구 막대 높이 (ui::toolbar)
pub const WORKBAR_H: f32 = 52.0;
pub const MARGIN: f32 = 8.0;
pub const GAP: f32 = 8.0;
/// 칸 맨 위 칩 줄 (탭 칩, 새 탭, 오른쪽에 항목 수 또는 필터 칩)
pub const HEAD_H: f32 = 44.0;
/// 칩 줄 아래 경로 줄 (경로, 책갈피)
pub const PATH_H: f32 = 28.0;
pub const HEADER_H: f32 = 28.0;
pub const ROW_H: f32 = 28.0;
pub const SCROLL_W: f32 = 10.0;
/// 칸 카드의 모서리
pub const RADIUS: f32 = 14.0;
pub const CHEV_W: f32 = 16.0;
pub const CRUMB_PAD: f32 = 8.0;

// 사이드바의 치수
/// 작업 공간 레일 너비. 패널이나 칸 영역은 그 오른쪽부터
pub const BAR_W: f32 = 44.0;
pub const SIDE_ROW_H: f32 = 28.0;
/// 사이드바 줄: 줄 왼쪽에서 이름이 시작하는 x (여백 8 + 아이콘 16 + 간격 8)
pub const SIDE_LABEL_L: f32 = SP_S + ICON + SP_S;
/// 사이드바 줄의 모서리
pub const R_SIDE_ROW: f32 = crate::palette::R_ROW;
/// 그룹 안 항목의 들여쓰기
pub const INDENT: f32 = 20.0;
/// 사이드바 패널 머리 줄 (작업 공간 이름, 핀). 이름 줄 28에 아래 여백 4
pub const SIDE_HEADER_H: f32 = 32.0;
/// 사이드바 구역 이름 줄 ("모든 공간")
pub const SIDE_LABEL_H: f32 = 28.0;

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

/// 가로·세로 나눔선이 만나는 곳 (4칸은 가운데, 3칸은 T자). 여기를 끌면 두 선이 함께 움직인다.
/// 잡기 쉽게 둘레를 4씩 넓힌다 (칸 카드의 둥근 모서리 바깥이라 칸의 것을 가리지 않는다)
pub fn split_cross(a: R, split: (f32, f32), layout: Layout) -> Option<R> {
    matches!(layout, Layout::Three | Layout::Four).then(|| {
        let (xm, ym) = split_points(a, split);
        R::new(xm - 4.0, ym - 4.0, xm + GAP + 4.0, ym + GAP + 4.0)
    })
}

/// 열 너비의 범위
pub const COL_MIN: f32 = 60.0;
pub const COL_MAX: f32 = 400.0;

/// 열 머리글의 경계를 d만큼 왼쪽으로 끌었다 (오른쪽이면 음수). 열은 오른쪽 끝에 붙어 있어(columns), 끄는 선만 따라가고
/// 다른 선은 그대로 두려면 선 오른쪽 열(col)이 넓어지는 만큼 왼쪽 열(prev)이 좁아져야 한다 (탐색기처럼 그 선만 움직인다).
/// 왼쪽에 다른 열이 없으면(이름과 사이의 선) 이름 칸이 받는다. 열은 COL_MIN~COL_MAX
pub fn drag_column(start: [f32; 3], col: usize, prev: Option<usize>, d: f32) -> [f32; 3] {
    let mut w = start;
    match prev {
        None => w[col] = (start[col] + d).clamp(COL_MIN, COL_MAX),
        Some(p) => {
            let lo = (COL_MIN - start[col]).max(start[p] - COL_MAX);
            let hi = (COL_MAX - start[col]).min(start[p] - COL_MIN);
            let d = d.clamp(lo, hi.max(lo));
            w[col] = start[col] + d;
            w[p] = start[p] - d;
        }
    }
    w
}

pub struct PaneGeom {
    /// 칩 줄 (탭 칩, 새 탭, 오른쪽 항목 수·필터 칩)
    pub head: R,
    /// 경로 글자 자리 (주소 입력 상자도 여기에 놓인다)
    pub crumb: R,
    /// 경로 조각을 그리는 곳 (오른쪽 끝 책갈피를 뺀 자리)
    pub crumbs: R,
    /// 경로 오른쪽 끝: 즐겨찾기 책갈피
    pub star: R,
    /// 책갈피 앞: 보기 방식 단추 (모양 그림과 작은 아래 화살표). 폭 0이면 없다
    pub view: R,
    pub header: R,
    pub list: R,
}

/// 칸 안 자리. 경로 줄에는 경로 조각, 보기 방식 단추, 책갈피가 있다 (뒤로·앞으로·칸 나누기는 도구 막대: ui::toolbar)
pub fn geom(r: R) -> PaneGeom {
    let head = R::new(r.l, r.t, r.r, r.t + HEAD_H);
    let path = R::new(r.l, head.b, r.r, head.b + PATH_H);
    let (t, b) = (path.t + SP_HAIR, path.b - SP_HAIR);
    // 경로 글자는 목록의 아이콘과 같은 선(16)에서 시작한다: 조각 바탕은 줄 바탕과 같은 선(8), 글자는 그 안쪽 여백(CRUMB_PAD)만큼 뒤
    let l = path.l + SP_L - CRUMB_PAD;
    // 책갈피: 24×24 단추. 그림의 오른쪽 끝이 목록의 오른쪽 여백(16)과 맞는다
    let mut star = R::new(path.r - SP_M - SP_XL, t, path.r - SP_M, b);
    let mut view = R::new(star.l - SP_XS - VIEW_W, t, star.l - SP_XS, b);
    // 좁은 칸: 경로가 PATH_MIN_W보다 좁아지면 보기 방식 단추, 그다음 책갈피를 뺀다 (폭 0이면 그리지도 누르지도 않는다. 단축키로는 된다)
    if view.l - SP_XS - l < PATH_MIN_W {
        view = R::new(star.l, t, star.l, b);
    }
    if star.l - SP_XS - l < PATH_MIN_W {
        star = R::new(path.r - SP_M, t, path.r - SP_M, b);
        view = R::new(star.l, t, star.l, b);
    }
    let right = if view.w() > 0.0 { view.l } else { star.l };
    let crumb = R::new(l, t, (right - SP_XS).max(l), b);
    let crumbs = crumb;
    let header = R::new(r.l, path.b, r.r, path.b + HEADER_H);
    // 아래쪽은 둥근 모서리에 항목이 닿지 않게 조금 띄운다
    let list = R::new(r.l, header.b, r.r, (r.b - SP_S).max(header.b));
    PaneGeom { head, crumb, crumbs, star, view, header, list }
}

/// 경로 줄의 보기 방식 단추 폭
pub const VIEW_W: f32 = 44.0;

/// 경로 입력 상자 자리: 경로 자리가 너무 좁으면 책갈피 자리까지 덮는다
pub fn addr_rect(g: &PaneGeom) -> R {
    if g.crumb.w() >= ADDR_MIN_W { g.crumb } else { R::new(g.crumb.l, g.crumb.t, g.star.r.max(g.crumb.r), g.crumb.b) }
}

/// 경로 입력 상자의 가장 좁은 폭
pub const ADDR_MIN_W: f32 = 80.0;

/// 경로 조각에 남길 가장 좁은 폭. 이보다 좁아지면 책갈피를 덜어 낸다
pub const PATH_MIN_W: f32 = 96.0;

/// 필터 칩 자리 (칩 줄 오른쪽 끝)
pub fn filter_chip(g: &PaneGeom) -> R {
    let w = (g.head.w() * 0.4).clamp(150.0, 240.0).min((g.head.w() - 22.0).max(0.0));
    R::new(g.head.r - SP_S - w, g.head.t + SP_S, g.head.r - SP_S, g.head.b - SP_S)
}

/// 칩 줄에서 탭 칩과 새 탭(+)이 놓일 곳: 왼쪽 여백 8, 오른쪽은 항목 수·필터 칩 자리(right)를 뺀 곳
pub fn chip_strip(g: &PaneGeom, right: f32) -> R {
    let l = g.head.l + SP_S;
    R::new(l, g.head.t + SP_S, (g.head.r - SP_S - right - SP_S).max(l + 100.0), g.head.b - SP_S)
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
    let mut x = list.r - LIST_R_INSET;
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
    // 이름 열: 아이콘(줄 바탕 8 + 안쪽 여백 8 = 16)보다 4 앞. 머리글 '이름'은 이 열의 8 안쪽, 곧 아이콘과 같은 16에서 시작한다
    Cols { name: (list.l + SP_M, x.max(list.l + 124.0)), modified, kind, size }
}

/// 자세히 보기가 아닌 보기의 정렬 버튼 (기준, 이름, 자리). 머리글 줄 왼쪽부터 나란히 둔다.
/// 열이 없는 격자 위에 자세히 보기의 열 머리글이 떠 있지 않게 한다. 지금 기준에만 화살표 자리가 붙는다
pub fn sort_chips(gfx: &impl Canvas, header: R, current: SortKey) -> Vec<(SortKey, &'static str, R)> {
    const KEYS: [(SortKey, &str); 4] = [(SortKey::Name, "이름"), (SortKey::Modified, "수정한 날짜"), (SortKey::Type, "유형"), (SortKey::Size, "크기")];
    let mut x = header.l + SP_S;
    KEYS.iter()
        .map(|&(key, label)| {
            let w = gfx.measure(label, Font::Normal) + if key == current { SP_M * 2.0 + ICON } else { SP_M * 2.0 };
            let r = R::new(x, header.t + SP_XS, x + w, header.b - SP_XS);
            x = r.r + SP_XS;
            (key, label, r)
        })
        // 좁은 칸에서는 다 들어가는 칩만 (끝에서 잘린 칩은 무엇인지 읽히지 않는다)
        .take_while(|(_, _, r)| r.r <= header.r - SP_S)
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

/// 탭 칩의 이름에 맞춘 폭: 글자 + 양쪽 안쪽 여백 12 (+ 탭이 여럿이면 닫기 단추 자리 20). 그리기(paint_pane_head)의 글자 자리와 같다
pub fn tab_natural_w(text_w: f32, tabs: usize) -> f32 {
    (text_w + SP_M * 2.0 + if tabs > 1 { 20.0 } else { 0.0 }).clamp(52.0, 180.0)
}

/// 탭 칩 안 오른쪽 끝의 닫기 단추
pub fn close_rect(tr: R) -> R {
    R::new(tr.r - SP_XL, tr.t + SP_XS, tr.r - SP_XS, tr.b - SP_XS)
}

pub fn add_rect(strip: R, tabs: &[R]) -> R {
    let l = tabs.iter().map(|r| r.r + SP_XS).fold(strip.l, f32::max);
    R::new(l, strip.t, l + 28.0, strip.b)
}

/// 큰 아이콘 보기의 그림 영역(DIP). 보통·아주 큰 아이콘은 VIEW_THUMBS
pub const THUMB: f32 = 96.0;
/// 타일 높이에서 그림을 뺀 나머지: 위 8 + 그림 아래 8 + 이름 두 줄(16×2) + 아래 4
pub const TILE_EXTRA: f32 = SP_S + SP_S + 32.0 + SP_XS;
/// 자세히 보기 줄: 이름 열 왼쪽에서 아이콘(4)·이름(4 + 16 + 8)까지. 줄 바탕(8)에서 보면 아이콘 8, 이름 32
pub const ROW_ICON_L: f32 = SP_XS;
pub const ROW_LABEL_L: f32 = SP_XS + ICON + SP_S;
/// 작은 아이콘·목록 칸: 칸 왼쪽(=줄 바탕)에서 아이콘 8, 이름 32 (자세히 보기와 같은 선)
pub const ICON_ICON_L: f32 = SP_S;
pub const ICON_LABEL_L: f32 = SP_S + ICON + SP_S;
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
            let usable = (list.w() - SP_S - LIST_R_INSET + SP_XS * 2.0).max(min_w);
            let cols = ((usable / min_w).floor() as usize).max(1);
            // 남는 폭을 칸에 고르게 나눠 오른쪽이 비지 않게
            // 타일 바탕은 칸 안쪽 4에서 시작해 줄 바탕과 같은 선(8)에 놓인다. 높이: 위 8 + 그림 + 8 + 이름 두 줄(16×2) + 4
            Grid { ox: list.l + SP_XS, oy: list.t + SP_XS, cols, rows: 0, cell_w: usable / cols as f32, cell_h: thumb + TILE_EXTRA, cell: CellKind::Tile(thumb), horiz: false }
        };
        match view {
            // 줄 바탕은 머리글 구분선과 같은 8px 안쪽에서 시작한다 (칸 테두리에 붙어 보이지 않게)
            ViewMode::Details => Grid { ox: list.l + SP_S, oy: list.t, cols: 1, rows: 0, cell_w: list.w() - SP_S - LIST_R_INSET, cell_h: ROW_H, cell: CellKind::Row, horiz: false },
            ViewMode::Huge => tile(256.0, 280.0),
            ViewMode::Large => tile(THUMB, 124.0),
            ViewMode::Medium => tile(48.0, 88.0),
            ViewMode::Small => {
                // 칸 사이 틈(4)까지 칸 폭에 넣는다: 마지막 칸의 바탕이 줄 바탕과 같은 오른쪽 끝(16)에서 끝난다
                let usable = (list.w() - SP_S - LIST_R_INSET + SP_XS).max(SMALL_W);
                let cols = ((usable / SMALL_W).floor() as usize).max(1);
                Grid { ox: list.l + SP_S, oy: list.t + SP_HAIR, cols, rows: 0, cell_w: usable / cols as f32, cell_h: ROW_H, cell: CellKind::Icon, horiz: false }
            }
            ViewMode::List => {
                // 아래에 가로 스크롤 막대 자리를 남긴다
                let rows = (((list.h() - 6.0 - SCROLL_W - 4.0) / ROW_H).floor() as usize).max(1);
                Grid { ox: list.l + SP_S, oy: list.t + SP_HAIR, cols: 0, rows, cell_w: LIST_W, cell_h: ROW_H, cell: CellKind::Icon, horiz: true }
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

/// 날짜 열 머리글
pub const DATE_HEADER: &str = "수정한 날짜";

/// 날짜 열의 내용 폭: 글자(날짜 단계 Meta, 고정폭 숫자) + 양쪽 여백. 숫자 폭이 모두 같아 0으로 채운 꼴만 재면 된다.
/// 짧은 꼴(short)은 그 후보 가운데 가장 넓은 것으로 ('오늘 00:00', '00월 00일', '0000-00-00' 등).
/// 머리글(정렬 화살표 자리 포함)보다 좁게 하지는 않는다 (탐색기의 열 자동 맞춤과 같다)
pub fn date_col_width(g: &impl Canvas, short: bool) -> f32 {
    let measure = |s: &str| g.measure(s, Font::Meta);
    let pad = SP_S * 2.0;
    let content = if short {
        (["방금", "오늘 00:00", "어제 00:00", "00월 00일", "0000-00-00"].iter().map(|s| measure(s)).fold(0.0, f32::max) + pad).ceil()
    } else {
        crate::typo::date_col_width(measure, pad)
    };
    let header = (g.measure(DATE_HEADER, Font::Normal) + SP_XS + ICON + pad).ceil();
    content.max(header).clamp(COL_MIN, COL_MAX)
}

/// 저장된 열 폭에서 '자동' 날짜 열(pane::DATE_AUTO)을 내용 폭(date_w)으로 바꾼다
pub fn resolve_cols(cols: [f32; 3], date_w: f32) -> [f32; 3] {
    if cols[0] == crate::pane::DATE_AUTO { [date_w, cols[1], cols[2]] } else { cols }
}

/// 자세히 보기 줄에서 파일 이름 글자의 자리 (이름 열, 줄 it)
pub fn row_name_rect(name_col: (f32, f32), it: R) -> R {
    R::new(name_col.0 + ROW_LABEL_L, it.t, name_col.1 - SP_S, it.b)
}

/// 작은 아이콘·목록 보기 칸에서 파일 이름 글자의 자리
pub fn icon_name_rect(it: R) -> R {
    R::new(it.l + ICON_LABEL_L, it.t, it.r - SP_S - SP_XS, it.b)
}

/// 목록의 줄 vi에서 파일 이름 글자의 자리 (그리기와 같은 자리). 큰 아이콘 보기(그림 아래 두 줄)는 None
pub fn name_rect(list: R, view: ViewMode, cols: [f32; 3], vi: usize, scroll: f32) -> Option<R> {
    let grid = Grid::of(list, view);
    let it = grid.item(vi, scroll);
    match grid.cell {
        CellKind::Row => Some(row_name_rect(columns(list, cols).name, it)),
        CellKind::Icon => Some(icon_name_rect(it)),
        CellKind::Tile(_) => None,
    }
}

/// GDI식 글자 그리기는 글자 상자의 양 끝을 픽셀에 맞추며 1px까지 좁아질 수 있다: 이름은 그만큼 덜 넓다고 보고 맞춘다
pub const NAME_FIT_SLACK: f32 = 1.0;

/// 목록의 줄 vi에서 이름을 고치는 상자의 자리 (paint_pane_list가 이름 상자를 그리는 자리와 같다).
/// 그 줄이 목록 안에 다 보이지 않으면 None
pub fn name_edit_rect(list: R, view: ViewMode, cols: [f32; 3], vi: usize, scroll: f32) -> Option<R> {
    let grid = Grid::of(list, view);
    let it = grid.item(vi, scroll);
    if it.t < list.t - 1.0 || it.b > list.b + 1.0 || it.l < list.l - 1.0 || it.r > list.r + 1.0 {
        return None;
    }
    Some(match grid.cell {
        // 그림 아래 이름 자리
        CellKind::Tile(thumb) => {
            let y = it.t + SP_S + thumb + SP_S;
            R::new(it.l + SP_XS, y, it.r - SP_S, y + SP_XL)
        }
        CellKind::Icon => R::new(it.l + ICON_LABEL_L - SP_XS, it.t + SP_HAIR, it.r - SP_S, it.b - SP_HAIR),
        CellKind::Row => {
            let name = columns(list, cols).name;
            R::new(name.0 + ROW_LABEL_L - SP_XS, it.t + SP_HAIR, name.1 - SP_XS, it.b - SP_HAIR)
        }
    })
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
    let avail = area.w() - SP_XS;
    let mut start = items.len() - 1;
    let mut total = widths[start];
    // "…"까지 넣으면 지금 폴더 이름이 60보다 좁아지는 칸이면 "…" 없이 지금 폴더만 (잘려도 이름이 보이게)
    let room_with_ellipsis = avail - (24.0 + CHEV_W);
    if start > 0 && room_with_ellipsis < widths[start].min(60.0) {
        let w = widths[start].min(avail.max(0.0));
        let (label, target) = items.swap_remove(start);
        return vec![Crumb { x0: area.l, x1: area.l + w, label, target }];
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
    let mut x = area.l;
    if start > 0 {
        out.push(Crumb { x0: x, x1: x + 24.0, label: "…".into(), target: items[start - 1].1.clone() });
        x += 24.0 + CHEV_W;
    }
    for (i, (label, target)) in items.into_iter().enumerate().skip(start) {
        let w = widths[i].min((area.r - SP_XS - x).max(0.0));
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

/// 단추판과 고르기 판(도구 막대 단추, 칸 나누기, 검색 범위, 설정의 고르기)의 바탕: 창 바탕과 같은 빛에서 한 단계 진하게
/// (어두운 테마는 밝게). 그래서 바탕에 묻히지도, 다른 빛으로 떠 보이지도 않는다
pub fn seg_track(t: &Theme) -> Color {
    if t.high_contrast { t.window } else { t.control }
}

/// 고르기 판에서 고른 칸·켜진 단추: 밝은 테마는 흰 면 + 아주 작은 그림자, 어두운 테마는 한 단계 밝은 면 + 위쪽 빛 선 (떠 보이게)
pub fn seg_on(g: &impl Canvas, r: R, radius: f32, t: &Theme) {
    if t.high_contrast {
        g.fill_round(r, radius, t.pane);
        g.stroke_round(r.inset(0.5, 0.5), radius, t.border, 1.0);
    } else if t.dark {
        g.fill_round(r, radius, mix(t.control, t.text, 0.12));
        g.fill(R::new(r.l + radius, r.t + 0.5, r.r - radius, r.t + 1.5), Color { r: 1.0, g: 1.0, b: 1.0, a: 0.10 });
    } else {
        soft_shadow(g, r, radius, 3.0, 0.12, false);
        g.fill_round(r, radius, t.pane);
    }
}

/// 사이드바 줄의 바탕: 지금 위치는 떠 있는 카드(고르기 판의 고른 칸과 같은 모습), 마우스를 올리면 옅은 면
pub fn side_row_bg(g: &impl Canvas, t: &Theme, r: R, current: bool, hovered: bool) {
    if current {
        seg_on(g, r, R_SIDE_ROW, t);
    } else if hovered {
        g.fill_round(r, R_SIDE_ROW, hover_fill(t));
    }
}

/// 사이드바에서 '모든 공간' 구역이 시작하는 y (목록 좌표). 위쪽 고정 목록이 짧으면 구역을 목록 아래 끝에 붙이고,
/// 길어서 넘치면 고정 목록 바로 아래(사이 간격 gap)에 이어 놓아 함께 스크롤한다
pub fn side_common_top(pins_h: f32, common_h: f32, list_h: f32, gap: f32) -> f32 {
    (pins_h + gap).max(list_h - common_h)
}

/// 줄의 마우스 올림 바탕: 창 바탕에서도 보이도록 반투명으로 겹친다
pub fn hover_fill(t: &Theme) -> Color {
    if t.high_contrast {
        t.hover
    } else if t.dark {
        Color { r: 1.0, g: 1.0, b: 1.0, a: 0.06 }
    } else {
        // 글자 색을 옅게 겹친다: 어느 공간 색 위에서도 빛을 바꾸지 않고 한 단계만 진해진다
        alpha(t.text, 0.06)
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

    /// '자동' 날짜 열(예전 기본값)만 내용 폭으로 바뀌고, 사용자가 끌어 정한 폭은 그대로
    #[test]
    fn auto_date_column_takes_the_content_width() {
        assert_eq!(resolve_cols(crate::pane::DEFAULT_COLS, 118.0), [118.0, 180.0, 84.0]);
        assert_eq!(resolve_cols([150.0, 180.0, 84.0], 118.0), [150.0, 180.0, 84.0]);
    }

    /// 이름 상자 자리: 자세히 보기에서는 그 줄의 이름 열 안, 아이콘 보기에서는 칸 안의 그림 아래. 줄이 다 보이지 않으면 없다
    #[test]
    fn name_edit_rect_sits_on_the_row_and_goes_away_when_it_is_cut_off() {
        let list = R::new(0.0, 100.0, 600.0, 400.0);
        let cols = [140.0, 180.0, 84.0];
        let r = name_edit_rect(list, ViewMode::Details, cols, 2, 0.0).unwrap();
        let name = columns(list, cols).name;
        assert_eq!((r.t, r.b), (100.0 + 2.0 * ROW_H + 2.0, 100.0 + 3.0 * ROW_H - 2.0));
        assert_eq!((r.l, r.r), (name.0 + 24.0, name.1 - 4.0));
        // 위로 반쯤 밀려난 줄, 목록 아래로 넘어간 줄
        assert!(name_edit_rect(list, ViewMode::Details, cols, 2, 2.0 * ROW_H + 10.0).is_none());
        assert!(name_edit_rect(list, ViewMode::Details, cols, 50, 0.0).is_none());
        for view in [ViewMode::Medium, ViewMode::Large, ViewMode::Small, ViewMode::List] {
            let it = Grid::of(list, view).item(0, 0.0);
            let r = name_edit_rect(list, view, cols, 0, 0.0).unwrap();
            assert!(r.l >= it.l && r.r <= it.r && r.t > it.t && r.b <= it.b && r.w() > 40.0, "{view:?}: {r:?} in {it:?}");
        }
    }

    /// 4칸의 가운데와 3칸의 T자에서 두 나눔선이 만난다. 1·2칸에는 없다
    #[test]
    fn split_cross_sits_where_both_splitters_meet() {
        let a = R::new(0.0, 0.0, 1000.0, 800.0);
        for layout in [Layout::Three, Layout::Four] {
            let c = split_cross(a, (0.5, 0.5), layout).unwrap();
            let (sx, sy) = splitters(a, (0.5, 0.5), layout);
            let (sx, sy) = (sx.unwrap(), sy.unwrap());
            let mid = |r: R| ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0);
            let (cx, cy) = mid(c);
            assert!(sx.contains(cx, cy) && (sy.contains(cx, cy) || (layout == Layout::Three && sy.l - cx <= GAP)), "{layout:?}");
            assert!(c.w() <= 20.0 && c.h() <= 20.0, "small target: {c:?}");
        }
        assert!(split_cross(a, (0.5, 0.5), Layout::TwoV).is_none() && split_cross(a, (0.5, 0.5), Layout::One).is_none());
    }

    /// 열 경계를 끌면 그 선만 움직인다: 선 왼쪽 열들의 왼쪽 끝은 그대로다. 열은 60 아래로 줄지 않는다
    #[test]
    fn dragging_a_column_edge_moves_only_that_line() {
        let list = R::new(0.0, 0.0, 900.0, 400.0);
        let start = [140.0, 130.0, 84.0];
        let lefts = |w: [f32; 3]| {
            let c = columns(list, w);
            (c.modified.unwrap().0, c.kind.unwrap().0, c.size.unwrap().0)
        };
        let (m0, k0, s0) = lefts(start);
        // 유형 열의 왼쪽 선(날짜와 사이)을 20 왼쪽으로: 그 선만 20 움직인다
        let w = drag_column(start, 1, Some(0), 20.0);
        assert_eq!(w, [120.0, 150.0, 84.0]);
        assert_eq!(lefts(w), (m0, k0 - 20.0, s0));
        // 크기 열의 왼쪽 선을 오른쪽으로 끝까지: 크기 열은 60에서 멈추고 유형 열이 그만큼 넓어진다
        let w = drag_column(start, 2, Some(1), -500.0);
        assert_eq!(w, [140.0, 154.0, 60.0]);
        assert_eq!(lefts(w).0, m0);
        // 날짜 열의 왼쪽 선(이름과 사이): 날짜 열이 넓어지고 이름 칸이 받는다
        let w = drag_column(start, 0, None, 30.0);
        assert_eq!(w, [170.0, 130.0, 84.0]);
        assert_eq!((lefts(w).1, lefts(w).2), (k0, s0));
    }

    /// 모든 보기에서 칸 가운데를 누르면 그 항목이 잡히고, 보이는 범위에 들어 있다. 경로 줄은 경로와 책갈피가 겹치지 않는다
    #[test]
    fn every_view_maps_items_and_clicks_consistently() {
        let g = geom(R::new(0.0, 0.0, 700.0, 520.0));
        assert!(g.star.w() > 0.0 && g.crumb.r <= g.star.l && g.star.r <= g.head.r);
        // 보기 방식 단추는 경로와 책갈피 사이, 경로 줄 높이 안
        assert!(g.view.w() == VIEW_W && g.crumb.r <= g.view.l && g.view.r <= g.star.l, "{:?} {:?} {:?}", g.crumb, g.view, g.star);
        assert!(g.view.t >= g.head.b && g.view.b <= g.header.t);
        // 경로 글자와 목록 아이콘이 같은 선에서 시작한다
        assert_eq!(g.crumb.l + CRUMB_PAD, columns(g.list, [140.0, 130.0, 84.0]).name.0 + 4.0);
        // 좁은 칸: 보기 방식 단추가 먼저, 그다음 책갈피가 빠지고, 경로는 칸 밖으로 나가지 않는다
        for w in [100.0, 150.0, 200.0, 240.0, 280.0, 330.0] {
            let g = geom(R::new(0.0, 0.0, w, 400.0));
            assert!(g.star.w() == 0.0 || g.crumb.r <= g.star.l, "w={w}");
            assert!(g.view.w() == 0.0 || (g.crumb.r <= g.view.l && g.view.r <= g.star.l), "w={w}");
            assert!(g.view.w() == 0.0 || g.star.w() > 0.0, "w={w}: the view button goes before the bookmark");
            assert!(g.crumb.l >= 0.0 && g.crumb.r <= w, "w={w}");
            assert!(addr_rect(&g).r <= w, "w={w}");
        }
        assert!(geom(R::new(0.0, 0.0, 120.0, 400.0)).star.w() == 0.0);
        let g = geom(R::new(0.0, 0.0, 190.0, 400.0));
        assert!(g.view.w() == 0.0 && g.star.w() > 0.0, "a narrow pane keeps the bookmark and drops the view button");
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
