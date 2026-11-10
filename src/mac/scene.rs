//! 맥 시제품 화면: 화면 모양 후보 두 가지를 Windows와 같은 치수(app.rs, app/sidebar.rs)로 그린다.
//! 화면 코드를 두 OS가 함께 쓰게 되는 3단계 전까지 쓰는 시제품용 그리기다 (마우스 올림·끌기 같은 상태는 없다)

use crate::gfx::Gfx;
use sakadu::draw::{Color, Font, R};
use sakadu::listing::{self, Kind};
use sakadu::palette::{R_CTRL, R_ROW, Theme, alpha};
use sakadu::pane::{DEFAULT_COLS, Location, Sort, Tab};
use sakadu::util;
use std::path::Path;

#[derive(Clone, Copy)]
pub enum Shape {
    /// 모양 1: 지금 모양. 맨 위 줄 왼쪽에 신호등, 아이콘 막대는 그 아래부터
    Current = 0,
    /// 모양 2: 맥 관례. 사이드바가 창 위까지 올라가고 신호등은 그 안, 아이콘 막대 대신 고르기 단추, 설정은 오른쪽 위
    MacSidebar = 1,
}

const W: f32 = 1280.0;
const H: f32 = 800.0;

// app.rs와 같은 치수
const WORKBAR_H: f32 = 44.0;
const MARGIN: f32 = 8.0;
const GAP: f32 = 8.0;
const HEAD_H: f32 = 44.0;
const PATH_H: f32 = 36.0;
const HEADER_H: f32 = 28.0;
const ROW_H: f32 = 28.0;
const SCROLL_W: f32 = 10.0;
const RADIUS: f32 = 12.0;
const CHEV_W: f32 = 16.0;
const CRUMB_PAD: f32 = 8.0;
// app/sidebar.rs와 같은 치수
const BAR_W: f32 = 44.0;
const BAR_ITEM: f32 = 32.0;
const BAR_PITCH: f32 = 44.0;
const SIDE_W: f32 = 220.0;
const SIDE_HEADER_H: f32 = 44.0;
const SIDE_ROW_H: f32 = 32.0;
const INDENT: f32 = 20.0;
/// 모양 2의 사이드바 너비 (아이콘 막대와 패널을 합친 폭)
const MAC_SIDE_W: f32 = BAR_W + SIDE_W;

// Windows 아이콘 글꼴 번호 (gfx가 SF Symbols로 바꾼다)
const G_BACK: u16 = 0xE72B;
const G_FWD: u16 = 0xE72A;
const G_UP: u16 = 0xE74A;
const G_ADD: u16 = 0xE710;
const G_CLOSE: u16 = 0xE8BB;
const G_CHEVRON: u16 = 0xE76C;
const G_CHEVRON_DOWN: u16 = 0xE70D;
const G_SORT_UP: u16 = 0xE70E;
const G_STAR: u16 = 0xE734;
const G_STAR_FILL: u16 = 0xE735;
const G_NEW_FOLDER: u16 = 0xE8F4;
const G_HIDDEN: u16 = 0xE7B3;
const G_LIST: u16 = 0xE8FD;
const G_SETTINGS: u16 = 0xE713;
const G_MORE: u16 = 0xE712;
const G_PREVIEW: u16 = 0xE90D;
const G_SEARCH: u16 = 0xE721;
const G_PLACES: u16 = 0xE8B7;
const G_PIN_FILL: u16 = 0xE841;
const G_MORE_CRUMB: u16 = 0xE712;

/// 칸 하나: 탭들(첫째가 지금 탭), 지금 탭은 폴더를 읽어 둔다
struct PaneData {
    tabs: Vec<Tab>,
    /// 고른 줄 (보기 순서)
    selected: Option<usize>,
}

/// 즐겨찾기 줄: 폴더·파일, 그룹 머리(접힘)
enum Fav {
    Item { name: &'static str, path: String, dir: bool, grouped: bool },
    Group { name: &'static str, collapsed: bool },
}

pub fn render(demo: &Path, shape: Shape, dark: bool, out: &Path) -> Result<(), String> {
    let g = Gfx::bitmap(W, H, 2.0).ok_or("그림판을 만들지 못함")?;
    let t = if dark { Theme::DARK } else { Theme::LIGHT };
    let d = |rel: &str| util::normalize(&demo.join(rel).to_string_lossy());
    let panes = [pane(&d("업무/2026 4분기"), &[d("업무/문서")], Some(1))?, pane(&d("다운로드"), &[d("팀 공유")], None)?];
    let favs = [
        Fav::Item { name: "다운로드", path: d("다운로드"), dir: true, grouped: false },
        Fav::Item { name: "2026 4분기", path: d("업무/2026 4분기"), dir: true, grouped: false },
        Fav::Item { name: "회의록_11월.md", path: d("업무/2026 4분기/회의록_11월.md"), dir: false, grouped: false },
        Fav::Group { name: "제주살이", collapsed: false },
        Fav::Item { name: "여행 사진", path: d("사진/제주살이"), dir: true, grouped: true },
        Fav::Item { name: "맛집 메모", path: d("사진/제주살이/맛집 메모.md"), dir: false, grouped: true },
        Fav::Item { name: "숙소 안내", path: d("다운로드/제주살이_숙소_안내.zip"), dir: false, grouped: true },
        Fav::Group { name: "프로젝트", collapsed: true },
    ];
    let current_dir = panes[0].tabs[0].loc.dir().unwrap_or_default().to_string();

    g.clear(t.window);
    let panes_l = match shape {
        Shape::Current => {
            paint_bar(&g, &t);
            paint_side_panel(&g, &t, R::new(BAR_W, WORKBAR_H, BAR_W + SIDE_W, H - MARGIN), &favs, &current_dir);
            let ws = paint_workbar(&g, &t, BAR_W + MARGIN + 32.0, false);
            traffic_lights(&g, WORKBAR_H / 2.0);
            let _ = ws;
            BAR_W + MARGIN + SIDE_W
        }
        Shape::MacSidebar => {
            // 사이드바: 창 바탕 위에 아주 옅은 면, 오른쪽에 얇은 선
            g.fill(R::new(0.0, 0.0, MAC_SIDE_W, H), alpha(t.text, if t.dark { 0.03 } else { 0.025 }));
            g.fill(R::new(MAC_SIDE_W - 1.0, 0.0, MAC_SIDE_W, H), alpha(t.text, 0.08));
            traffic_lights(&g, WORKBAR_H / 2.0);
            // 아이콘 막대 대신: 바로가기·즐겨찾기·검색 고르기 단추
            let track = R::new(12.0, WORKBAR_H + 4.0, MAC_SIDE_W - 12.0, WORKBAR_H + 36.0);
            g.fill_round(track, track.h() / 2.0, seg_track(&t));
            let seg_w = (track.w() - 6.0) / 3.0;
            for (i, glyph) in [G_PLACES, G_STAR, G_SEARCH].into_iter().enumerate() {
                let r = R::new(track.l + 3.0 + i as f32 * seg_w, track.t + 3.0, track.l + 3.0 + (i + 1) as f32 * seg_w, track.b - 3.0);
                let on = i == 1;
                if on {
                    seg_on(&g, r, r.h() / 2.0, &t);
                }
                g.glyph(glyph, r, false, if on { t.text } else { t.dim });
            }
            paint_side_panel(&g, &t, R::new(0.0, WORKBAR_H + 44.0, MAC_SIDE_W, H - MARGIN), &favs, &current_dir);
            paint_workbar(&g, &t, MAC_SIDE_W + MARGIN, true);
            MAC_SIDE_W + MARGIN
        }
    };

    // 칸 두 개 (좌우), 왼쪽이 지금 칸
    let area = R::new(panes_l, WORKBAR_H, W - MARGIN, H - MARGIN);
    let xm = area.l + (area.w() - GAP) * 0.5;
    paint_pane(&g, &t, R::new(area.l, area.t, xm, area.b), &panes[0], true, &favs);
    paint_pane(&g, &t, R::new(xm + GAP, area.t, area.r, area.b), &panes[1], false, &favs);
    g.save_png(out)
}

fn pane(dir: &str, others: &[String], selected: Option<usize>) -> Result<PaneData, String> {
    let mut first = Tab::new(1, Location::Dir(dir.to_string()));
    let entries = listing::list_dir(dir).map_err(|e| format!("{dir}: {e}"))?;
    first.set_entries(entries, Sort::default(), false);
    let mut tabs = vec![first];
    tabs.extend(others.iter().enumerate().map(|(i, p)| Tab::new(2 + i as u64, Location::Dir(p.clone()))));
    Ok(PaneData { tabs, selected })
}

/// 맥 창의 닫기·최소화·확대 단추 (실제 창에서는 시스템이 그린다. 표준 크기 12pt, 간격 20pt)
fn traffic_lights(g: &Gfx, cy: f32) {
    for (i, hex) in [0xFF5F57u32, 0xFEBC2E, 0x28C840].into_iter().enumerate() {
        let cx = 20.0 + i as f32 * 20.0;
        let c = Color { r: ((hex >> 16) & 0xFF) as f32 / 255.0, g: ((hex >> 8) & 0xFF) as f32 / 255.0, b: (hex & 0xFF) as f32 / 255.0, a: 1.0 };
        let r = R::new(cx - 6.0, cy - 6.0, cx + 6.0, cy + 6.0);
        g.fill_round(r, 6.0, c);
        g.stroke_round(r.inset(0.25, 0.25), 6.0, Color { r: 0.0, g: 0.0, b: 0.0, a: 0.12 }, 0.5);
    }
}

// ---------- sidebar.rs와 같은 면 ----------

fn well(t: &Theme) -> Color {
    t.sel_inactive
}

fn seg_track(t: &Theme) -> Color {
    if t.dark { Color { r: 0.0, g: 0.0, b: 0.0, a: 0.28 } } else { t.sel_inactive }
}

fn seg_on(g: &Gfx, r: R, radius: f32, t: &Theme) {
    if t.dark {
        g.fill_round(r, radius, t.sel_inactive);
        g.fill(R::new(r.l + radius, r.t + 0.5, r.r - radius, r.t + 1.5), Color { r: 1.0, g: 1.0, b: 1.0, a: 0.10 });
    } else {
        soft_shadow(g, r, radius, 3.0, 0.12, false);
        g.fill_round(r, radius, t.pane);
    }
}

/// app.rs soft_shadow와 같다: 조금씩 내려가며 커지는 아주 옅은 둥근 사각형을 겹친다
fn soft_shadow(g: &Gfx, r: R, radius: f32, depth: f32, strength: f32, dark: bool) {
    const LAYERS: usize = 8;
    let (base, k) = if dark { ((0.0, 0.0, 0.0), 2.5) } else { ((0.06, 0.09, 0.16), 1.0) };
    let ink = Color { r: base.0, g: base.1, b: base.2, a: (strength * k).min(0.7) / LAYERS as f32 };
    for i in 1..=LAYERS {
        let f = i as f32 / LAYERS as f32;
        let spread = depth * (f * 0.5 - 0.25);
        let dy = depth * f * 0.6;
        g.fill_round(R::new(r.l - spread, r.t + dy - spread, r.r + spread, r.b + dy + spread), radius + spread.max(0.0), ink);
    }
    g.fill_round(R::new(r.l - 0.5, r.t + 0.5, r.r + 0.5, r.b + 1.5), radius + 0.5, Color { a: if dark { 0.25 } else { 0.05 }, ..ink });
}

/// 모양 1의 아이콘 막대: 신호등 줄 아래부터 바로가기·즐겨찾기(켜짐)·검색, 맨 아래 설정
fn paint_bar(g: &Gfx, t: &Theme) {
    let item = |y: f32, glyph: u16, on: bool| {
        let l = (BAR_W - BAR_ITEM) / 2.0;
        let r = R::new(l, y, l + BAR_ITEM, y + BAR_ITEM);
        if on {
            seg_on(g, r, R_CTRL, t);
        }
        g.glyph(glyph, r, false, if on { t.text } else { t.dim });
    };
    for (i, (glyph, on)) in [(G_PLACES, false), (G_STAR, true), (G_SEARCH, false)].into_iter().enumerate() {
        item(WORKBAR_H + 6.0 + i as f32 * BAR_PITCH, glyph, on);
    }
    item(H - 8.0 - BAR_ITEM, G_SETTINGS, false);
}

/// 즐겨찾기 패널: 머리 줄(제목, 고정 핀)과 목록. 지금 칸의 폴더 줄은 들어간 면
fn paint_side_panel(g: &Gfx, t: &Theme, pr: R, favs: &[Fav], current_dir: &str) {
    g.text("즐겨찾기", R::new(pr.l + 18.0, pr.t, pr.r - 44.0, pr.t + SIDE_HEADER_H), Font::Strong, t.text);
    let pin = R::new(pr.r - 8.0 - 28.0, pr.t + 8.0, pr.r - 8.0, pr.t + 36.0);
    g.glyph(G_PIN_FILL, pin, false, t.dim);
    let mut y = pr.t + SIDE_HEADER_H + 2.0;
    for f in favs {
        let r = R::new(pr.l + 8.0, y, pr.r - 8.0, y + SIDE_ROW_H);
        match f {
            Fav::Group { name, collapsed } => {
                g.glyph(if *collapsed { G_CHEVRON } else { G_CHEVRON_DOWN }, R::new(r.l + 8.0, r.t, r.l + 24.0, r.b), true, t.faint);
                g.text(name, R::new(r.l + 36.0, r.t, r.r - 8.0, r.b), Font::Normal, t.text);
            }
            Fav::Item { name, path, dir, grouped } => {
                if *dir && util::eq_path(path, current_dir) {
                    g.fill_round(r, R_ROW, well(t));
                }
                let lx = r.l + 36.0 + if *grouped { INDENT } else { 0.0 };
                let _ = dir;
                g.icon_for_path(path, R::new(lx - 26.0, r.t + 8.0, lx - 10.0, r.t + 24.0));
                g.text(name, R::new(lx, r.t, r.r - 8.0, r.b), Font::Normal, t.text);
            }
        }
        y += SIDE_ROW_H;
    }
}

/// 작업 줄: 작업 공간 이름, 칸 나누기 판, 오른쪽 단추(…·숨김·미리 보기, 모양 2는 설정까지)
fn paint_workbar(g: &Gfx, t: &Theme, l: f32, settings_right: bool) -> R {
    let label = "4분기 보고";
    let ws = R::new(l, 6.0, l + (g.measure(label, Font::Normal) + 40.0).clamp(96.0, 220.0), 38.0);
    g.text(label, R::new(ws.l + 10.0, ws.t, ws.r - 24.0, ws.b), Font::Normal, t.text);
    g.glyph(G_CHEVRON_DOWN, R::new(ws.r - 22.0, ws.t + 1.0, ws.r - 8.0, ws.b + 1.0), true, t.faint);
    // 칸 나누기: 한 덩어리 판 위에 지금 고른 것(좌우 두 칸)만 도드라지게
    let pl = ws.r + 8.0;
    let pill = R::new(pl, 6.0, pl + 6.0 + 5.0 * 32.0, 38.0);
    g.fill_round(pill, pill.h() / 2.0, seg_track(t));
    for i in 0..5 {
        let bl = pill.l + 3.0 + i as f32 * 32.0;
        let r = R::new(bl, 9.0, bl + 32.0, 35.0);
        let current = i == 1;
        if current {
            seg_on(g, r, r.h() / 2.0, t);
        }
        draw_layout_icon(g, r, i, if current { t.text } else { t.dim });
    }
    // 오른쪽 단추: 맥은 창 단추가 왼쪽이라 오른쪽 끝부터 바로 놓는다
    let mut r_edge = W - 8.0;
    let mut button = |glyph: u16| {
        let r = R::new(r_edge - 32.0, 6.0, r_edge, 38.0);
        g.glyph(glyph, r, false, t.dim);
        r_edge = r.l - 4.0;
    };
    if settings_right {
        button(G_SETTINGS);
    }
    button(G_PREVIEW);
    button(G_HIDDEN);
    button(G_MORE);
    ws
}

/// app.rs draw_layout_icon과 같다 (0 한 칸, 1 좌우, 2 위아래, 3 세 칸, 4 네 칸)
fn draw_layout_icon(g: &Gfx, r: R, layout: usize, c: Color) {
    let (cx, cy) = ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0);
    let ic = R::new(cx - 9.0, cy - 7.0, cx + 9.0, cy + 7.0);
    g.stroke_round(ic, 2.0, c, 1.3);
    let v = |x: f32, t: f32, b: f32| g.fill(R::new(x - 0.65, t, x + 0.65, b), c);
    let h = |y: f32, l: f32, rr: f32| g.fill(R::new(l, y - 0.65, rr, y + 0.65), c);
    match layout {
        1 => v(cx, ic.t, ic.b),
        2 => h(cy, ic.l, ic.r),
        3 => {
            v(cx, ic.t, ic.b);
            h(cy, cx, ic.r);
        }
        4 => {
            v(cx, ic.t, ic.b);
            h(cy, ic.l, ic.r);
        }
        _ => {}
    }
}

// ---------- 칸 (app.rs paint_pane의 자세히 보기) ----------

fn paint_pane(g: &Gfx, t: &Theme, r: R, p: &PaneData, active: bool, favs: &[Fav]) {
    let tab = &p.tabs[0];
    // 카드: 밝은 테마는 옅은 그림자로 떠 있고 지금 칸은 조금 더 높이 뜬다. 어두운 테마는 얇은 테두리
    if active {
        soft_shadow(g, r, RADIUS, 12.0, 0.10, t.dark);
    } else if !t.dark {
        soft_shadow(g, r, RADIUS, 6.0, 0.06, false);
    }
    g.fill_round(r, RADIUS, t.pane);
    if active {
        g.fill_round(R::new(r.l + RADIUS, r.t + 1.0, r.r - RADIUS, r.t + 3.0), 1.0, t.accent);
    }

    // 칩 줄: 탭 칩(지금 탭은 들어간 면), 새 탭(+), 오른쪽에 항목 수
    let head = R::new(r.l, r.t, r.r, r.t + HEAD_H);
    let count = format!("{}개", tab.view.len());
    let cw = g.measure(&count, Font::Small);
    g.text(&count, R::new(head.r - 8.0 - cw, head.t + 8.0, head.r - 8.0, head.t + 36.0), Font::Small, t.faint);
    let mut x = head.l + 8.0;
    let n = p.tabs.len();
    for (i, tb) in p.tabs.iter().enumerate() {
        let title = tb.loc.title();
        let w = (g.measure(&title, Font::Normal) + 22.0 + if n > 1 { 20.0 } else { 0.0 }).clamp(52.0, 180.0);
        let tr = R::new(x, head.t + 8.0, x + w, head.t + 36.0);
        let current = i == 0;
        if current {
            g.fill_round(tr, tr.h() / 2.0, well(t));
        }
        let closable = n > 1 && current;
        g.text(&title, R::new(tr.l + 12.0, tr.t, tr.r - if closable { 26.0 } else { 10.0 }, tr.b), Font::Normal, t.text);
        if closable {
            g.glyph(G_CLOSE, R::new(tr.r - 24.0, tr.t + 4.0, tr.r - 4.0, tr.b - 4.0), true, t.dim);
        }
        x = tr.r + 2.0;
    }
    g.glyph_font(G_ADD, R::new(x + 2.0, head.t + 8.0, x + 30.0, head.t + 36.0), Font::GlyphMid, t.dim);

    // 경로 줄: 뒤로·앞으로(기록 없음)·위로, 경로 조각, 별·새 폴더·보기
    let path = R::new(r.l, head.b, r.r, head.b + PATH_H);
    let (pt, pb) = (path.t + 4.0, path.b - 4.0);
    let off = alpha(t.dim, 0.45);
    let back = R::new(path.l + 8.0, pt, path.l + 36.0, pb);
    let fwd = R::new(back.r + 2.0, pt, back.r + 30.0, pb);
    let up = R::new(fwd.r + 2.0, pt, fwd.r + 30.0, pb);
    g.glyph_font(G_BACK, back, Font::GlyphMid, off);
    g.glyph_font(G_FWD, fwd, Font::GlyphMid, off);
    g.glyph_font(G_UP, up, Font::GlyphMid, t.dim);
    let view = R::new(path.r - 8.0 - 40.0, pt, path.r - 8.0, pb);
    let new_folder = R::new(view.l - 4.0 - 28.0, pt, view.l - 4.0, pb);
    let star = R::new(new_folder.l - 4.0 - 28.0, pt, new_folder.l - 4.0, pb);
    g.glyph(G_NEW_FOLDER, new_folder, false, t.dim);
    g.glyph(G_LIST, R::new(view.l + 2.0, view.t, view.l + 26.0, view.b), false, t.dim);
    g.glyph(G_CHEVRON_DOWN, R::new(view.r - 16.0, view.t + 1.0, view.r - 4.0, view.b + 1.0), true, t.faint);
    let dir = tab.loc.dir().unwrap_or_default();
    let starred = favs.iter().any(|f| matches!(f, Fav::Item { path, dir: true, .. } if util::eq_path(path, dir)));
    g.glyph(if starred { G_STAR_FILL } else { G_STAR }, star, false, if starred { t.text } else { t.dim });
    paint_crumbs(g, t, R::new(up.r + 8.0, path.t + 4.0, star.l - 4.0, path.b - 4.0), dir);

    // 열 머리글과 목록
    let header = R::new(r.l, path.b, r.r, path.b + HEADER_H);
    let list = R::new(r.l, header.b, r.r, r.b - 6.0);
    let cols = columns(list);
    g.text("이름", R::new(cols.name.0 + 4.0, header.t, cols.name.1, header.b), Font::Normal, t.dim);
    let nw = g.measure("이름", Font::Normal);
    g.glyph(G_SORT_UP, R::new(cols.name.0 + 8.0 + nw, header.t, cols.name.0 + 22.0 + nw, header.b), true, t.faint);
    if let Some((l, rr)) = cols.modified {
        g.text("수정한 날짜", R::new(l + 8.0, header.t, rr - 8.0, header.b), Font::Normal, t.dim);
    }
    if let Some((l, rr)) = cols.size {
        g.text("크기", R::new(l + 8.0, header.t, rr - 8.0, header.b), Font::Right, t.dim);
    }
    g.fill(R::new(r.l + 8.0, header.b - 1.0, r.r - 8.0, header.b), alpha(t.text, 0.07));

    let now = listing::now_filetime();
    g.clip(list);
    for (vi, &ei) in tab.view.iter().enumerate() {
        let e = &tab.entries[ei as usize];
        let y = list.t + vi as f32 * ROW_H;
        if y > list.b {
            break;
        }
        let row = R::new(list.l + 8.0, y + 1.0, list.r - SCROLL_W - 6.0, y + ROW_H - 1.0);
        let selected = p.selected == Some(vi);
        if selected {
            g.fill_round(row, R_ROW, if active { t.sel } else { t.sel_inactive });
        }
        let full = util::join(dir, &e.name_string());
        g.icon_for_path(&full, R::new(cols.name.0 + 4.0, y + (ROW_H - 16.0) / 2.0, cols.name.0 + 20.0, y + (ROW_H + 16.0) / 2.0));
        let fg = if e.is_hidden() { t.faint } else { t.text };
        g.text(&e.name_string(), R::new(cols.name.0 + 28.0, y, cols.name.1 - 8.0, y + ROW_H), Font::Normal, fg);
        if let Some((l, rr)) = cols.modified {
            g.text(&listing::format_time_short(e.modified, now), R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Normal, t.dim);
        }
        if let Some((l, rr)) = cols.size {
            let s = if e.kind == Kind::File { listing::format_size(e.size) } else { String::new() };
            g.text(&s, R::new(l + 8.0, y, rr - 8.0, y + ROW_H), Font::Right, t.dim);
        }
    }
    g.unclip();

    // 어두운 테마는 얇은 테두리
    if t.dark {
        g.stroke_round(r.inset(0.5, 0.5), RADIUS, t.pane_border, 1.0);
    }
}

/// 경로 조각: 뒤에서부터 들어가는 만큼, 앞은 "…"로 줄인다. 마지막 조각만 진하게
fn paint_crumbs(g: &Gfx, t: &Theme, area: R, dir: &str) {
    let segs = util::segments(dir);
    let width = |s: &str| g.measure(s, Font::Normal) + CRUMB_PAD * 1.5;
    let mut shown: Vec<&str> = Vec::new();
    let mut used = 0.0;
    let more_w = 28.0;
    for (label, _) in segs.iter().rev() {
        let w = width(label) + if shown.is_empty() { 0.0 } else { CHEV_W };
        if used + w + more_w + CHEV_W > area.w() && !shown.is_empty() {
            break;
        }
        used += w;
        shown.push(label);
    }
    let cut = shown.len() < segs.len();
    shown.reverse();
    let mut x = area.l;
    if cut {
        g.glyph(G_MORE_CRUMB, R::new(x, area.t, x + more_w, area.b), true, t.dim);
        x += more_w;
        g.glyph(G_CHEVRON, R::new(x, area.t, x + CHEV_W, area.b), true, t.faint);
        x += CHEV_W;
    }
    let n = shown.len();
    for (i, label) in shown.iter().enumerate() {
        let w = width(label);
        let last = i + 1 == n;
        g.text(label, R::new(x + CRUMB_PAD, area.t, x + w - CRUMB_PAD / 2.0, area.b), Font::Normal, if last { t.text } else { t.dim });
        x += w;
        if !last {
            g.glyph(G_CHEVRON, R::new(x, area.t, x + CHEV_W, area.b), true, t.faint);
            x += CHEV_W;
        }
    }
}

struct Cols {
    name: (f32, f32),
    modified: Option<(f32, f32)>,
    size: Option<(f32, f32)>,
}

/// app.rs columns와 같은 규칙 (유형 열은 590 넘는 칸에서만이라 이 그림에서는 빠진다)
fn columns(list: R) -> Cols {
    let w = DEFAULT_COLS;
    let mut x = list.r - SCROLL_W - 6.0;
    let size = (list.w() >= 460.0).then(|| {
        let c = (x - w[2], x);
        x -= w[2];
        c
    });
    if list.w() >= 590.0 {
        x -= w[1];
    }
    let modified = (list.w() >= 300.0).then(|| {
        let c = (x - w[0], x);
        x -= w[0];
        c
    });
    Cols { name: (list.l + 12.0, x.max(list.l + 124.0)), modified, size }
}
