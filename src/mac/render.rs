//! 그림 시험: 창 없이 App을 몇 가지 상태로 만들어 PNG로 그린다 (CI에서 맥 화면을 확인하는 용도)

use crate::app::{App, Hit, Key, MenuKind, Mods, Space};
use sakadu::ui::Layout;
use crate::gfx::Gfx;
use sakadu::favorites::{Favorite, Favorites, Group};
use sakadu::palette::Theme;
use sakadu::pane::Location;
use sakadu::util;
use sakadu::workspaces::{Workspace, Workspaces};
use std::path::Path;

const W: f32 = 1280.0;
const H: f32 = 800.0;
/// 그림판의 배율 (레티나처럼 2배로 그린다). 썸네일도 이 배율에 맞춰 읽는다
const SCALE: f32 = 2.0;

/// 미리 보기 카드 장면에 쓰는 데모 파일 (사진/제주살이 안에 만든다)
const DEMO_PICTURE: &str = "협재 노을.bmp";
const DEMO_NOTE: &str = "여행 일정.txt";
const DEMO_PDF: &str = "숙소 안내.pdf";
/// 압축 파일 장면의 zip (업무/문서 안에 만든다. 그 폴더의 안은 다른 장면에 보이지 않는다)과 그 안의 폴더
const DEMO_ZIP: &str = "여행 자료.zip";
const DEMO_ZIP_DIR: &str = "여행 자료";
const DEMO_NOTE_TEXT: &str = "제주살이 여행 일정\n\n1일차  제주공항 → 협재 해변 → 숙소\n2일차  성산 일출봉 (일출 06:42) → 우도\n3일차  한라산 영실 코스, 오후에는 쉬기\n4일차  동문시장에서 장 보기 → 공항\n\n챙길 것\n- 등산화, 바람막이\n- 보조 배터리\n- 숙소 안내 메일 (다운로드 폴더)\n";

/// 24비트 BMP 파일 내용 (압축 없음). pixel은 왼쪽 위를 (0, 0), 오른쪽 아래를 (1, 1)로 본 자리의 색 [빨강, 초록, 파랑]
pub fn bmp(w: u32, h: u32, pixel: impl Fn(f32, f32) -> [u8; 3]) -> Vec<u8> {
    // 한 줄의 길이는 4의 배수로 맞춘다
    let row = (w as usize * 3).div_ceil(4) * 4;
    let pixels = row * h as usize;
    // 파일 머리 14바이트와 그림 정보 40바이트: 크기, 그림이 시작하는 곳(54), 가로·세로, 색 면 1개·24비트, 압축 없음, 인치당 72점
    let mut out = b"BM".to_vec();
    for v in [54 + pixels as u32, 0, 54, 40, w, h, 1 | (24 << 16), 0, pixels as u32, 2835, 2835, 0, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    // 줄은 아래에서 위로, 색은 파랑·초록·빨강 순서
    for y in (0..h).rev() {
        for x in 0..w {
            let [r, g, b] = pixel((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            out.extend_from_slice(&[b, g, r]);
        }
        out.resize(54 + row * (h - y) as usize, 0);
    }
    out
}

/// 데모 그림: 바다 위로 지는 해 (480×320 BMP. 맥이 바로 읽는 형식 가운데 손으로 쓰기 가장 쉽다)
pub fn demo_picture() -> Vec<u8> {
    const HORIZON: f32 = 0.62;
    let mix = |a: [f32; 3], b: [f32; 3], k: f32| [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * k.clamp(0.0, 1.0));
    bmp(480, 320, |x, y| {
        // 하늘은 위가 짙은 파랑이고 수평선 쪽이 주황, 바다는 아래로 갈수록 어둡다
        let base = if y < HORIZON {
            mix([0.15, 0.24, 0.46], [0.98, 0.64, 0.36], (y / HORIZON).powf(1.6))
        } else {
            mix([0.22, 0.34, 0.47], [0.06, 0.12, 0.23], (y - HORIZON) / (1.0 - HORIZON))
        };
        // 해와 그 둘레의 빛 (바다에는 옅게 비친다). 가로가 세로의 1.5배라 거리를 잴 때 가로를 그만큼 늘린다
        let d = (((x - 0.68) * 1.5).powi(2) + (y - 0.52).powi(2)).sqrt();
        let glow = (1.0 - d / 0.4).max(0.0).powi(2) * if y < HORIZON { 0.7 } else { 0.3 };
        let c = if y < HORIZON && d < 0.075 { [1.0, 0.95, 0.80] } else { mix(base, [1.0, 0.84, 0.52], glow) };
        c.map(|v| (v * 255.0) as u8)
    })
}

/// 한 쪽짜리 PDF (w×h pt, 쪽의 내용은 PDF 그리기 명령 content). 맥이 읽는 최소한의 모양: 개체 넷, 상호 참조표, 끝 표시
pub fn pdf(w: u32, h: u32, content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents 4 0 R >>"),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    // 상호 참조표의 줄은 저마다 20바이트다
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    out
}

/// 한 쪽짜리 PDF: 둘레 20pt를 남긴 파란 사각형 하나 (시험이 썸네일 한가운데의 색을 본다)
pub fn tiny_pdf(w: u32, h: u32) -> Vec<u8> {
    pdf(w, h, &format!("0 0 1 rg 20 20 {} {} re f\n", w - 40, h - 40))
}

/// 데모 PDF: 안내문처럼 보이는 A5 한 쪽 (머리 띠, 사진 자리, 글줄. 글꼴 없이 사각형으로만 그린다)
fn demo_pdf() -> Vec<u8> {
    let mut c = String::from("0.16 0.36 0.85 rg 36 540 348 34 re f\n0.98 0.64 0.36 rg 36 330 348 180 re f\n0.15 0.24 0.46 rg 36 330 348 60 re f\n0.72 g\n");
    for (i, w) in [330, 348, 290, 340, 210, 320, 348, 260].into_iter().enumerate() {
        c.push_str(&format!("36 {} {w} 7 re f\n", 296 - i as i32 * 24));
    }
    pdf(420, 595, &c)
}

/// 미리 보기 카드 장면에 쓸 그림·글·PDF 파일을 dir에 만든다 (CI가 만드는 데모 파일은 내용이 비어 있어 미리 볼 수 없다).
/// 수정한 날짜는 고정한다 (목록의 짧은 날짜가 늘 같게)
fn write_demo_previews(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // 2026-10-17 09:20 (UTC)
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_792_228_800);
    for (name, data) in [(DEMO_PICTURE, demo_picture()), (DEMO_NOTE, DEMO_NOTE_TEXT.as_bytes().to_vec()), (DEMO_PDF, demo_pdf())] {
        let path = dir.join(name);
        std::fs::write(&path, data)?;
        std::fs::File::options().write(true).open(&path)?.set_modified(when)?;
    }
    Ok(())
}

/// 압축 파일 장면의 zip을 dir에 만든다: 여행 자료/(일정 글, 노을 그림, 숙소 안내 PDF, 영수증/렌터카.txt)를 맥의 ditto로 묶는다
/// (Finder의 압축하기와 같은 도구). 이미 있으면 그대로 쓴다. 만들지 못하면(ditto가 없거나 실패) None: 그 장면은 건너뛴다
fn write_demo_zip(dir: &Path) -> Option<String> {
    let zip = dir.join(DEMO_ZIP);
    if !zip.is_file() {
        // 묶을 것은 임시 폴더에 만들었다가 묶은 뒤 치운다
        let stage = std::env::temp_dir().join(format!("sakadu-render-zip-{}", std::process::id()));
        let made = ditto_demo(&stage.join(DEMO_ZIP_DIR), &zip);
        let _ = std::fs::remove_dir_all(&stage);
        if !matches!(made, Ok(true)) {
            eprintln!("압축 파일 장면을 건너뜀: zip을 만들지 못함 ({made:?})");
            return None;
        }
    }
    Some(util::normalize(&zip.to_string_lossy()))
}

/// 폴더 src에 압축 파일 장면의 내용을 만들고 zip으로 묶는다. ditto가 잘 끝났으면 true
fn ditto_demo(src: &Path, zip: &Path) -> std::io::Result<bool> {
    if let Some(dir) = zip.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_demo_previews(src)?;
    std::fs::create_dir_all(src.join("영수증"))?;
    std::fs::write(src.join("영수증").join("렌터카.txt"), "렌터카 3일 · 186,000원\n")?;
    let status = std::process::Command::new("/usr/bin/ditto").args(["-c", "-k", "--norsrc", "--keepParent"]).arg(src).arg(zip).status()?;
    Ok(status.success())
}

/// demo 폴더(CI가 만드는 한글 이름 데모)로 상태 스물여섯 가지 × 밝은·어두운 테마를 out에 그린다.
/// 작업 공간은 셋: 업무(파랑, 두 칸), 사진 정리(보라, 네 칸), 개인(주황, 세 칸). 공간마다 고정한 것이 다르고,
/// 아래 '모든 공간'(바로가기와 즐겨찾기)은 어느 공간에서나 같다.
/// 미리 보기 카드와 창 장면에 쓸 그림·글·PDF 파일은 demo의 사진/제주살이에, 압축 파일 장면의 zip은 업무/문서에 직접 만든다
pub fn render_all(demo: &Path, out: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let root = util::normalize(&demo.to_string_lossy());
    let d = |rel: &str| rel.split('/').fold(root.clone(), |p, s| util::join(&p, s));
    write_demo_previews(Path::new(&d("사진/제주살이"))).map_err(|e| format!("데모 그림을 만들지 못함: {e}"))?;
    let demo_zip = write_demo_zip(Path::new(&d("업무/문서")));
    let item = |name: &str, rel: &str, is_dir: bool| Favorite { name: name.into(), path: d(rel), is_dir };
    // 즐겨찾기('모든 공간'의 바로가기 아래): 펼친 그룹 하나와 접은 그룹 하나
    let favs = Favorites {
        items: Vec::new(),
        groups: vec![
            Group { name: "제주살이".into(), collapsed: false, items: vec![item("여행 사진", "사진/제주살이", true), item("맛집 메모", "사진/제주살이/맛집 메모.md", false)] },
            Group { name: "프로젝트".into(), collapsed: true, items: Vec::new() },
        ],
    };
    let places = vec![("홈".to_string(), root.clone()), ("문서".to_string(), d("업무")), ("다운로드".to_string(), d("다운로드")), ("사진".to_string(), d("사진"))];
    let mut written = Vec::new();
    for dark in [false, true] {
        let theme = if dark { Theme::DARK } else { Theme::LIGHT };
        let mut shots: Vec<(&str, App)> = Vec::new();
        // spaces: 작업 공간 셋을 넣는다 (지금 것은 업무). 넣지 않으면 고정한 것이 없는 '기본' 공간 하나
        let base = |favs: Favorites, spaces: bool| {
            let mut a = App::new(W, H, theme, &root, favs, places.clone());
            a.standalone = true;
            a.scale = SCALE;
            // 새 탭은 지금 폴더로 (시작 화면은 'start' 장면에서 따로 본다)
            a.set_new_tab_start(false);
            a.navigate(0, Location::Dir(d("업무/2026 4분기")), None);
            a.navigate(1, Location::Dir(d("다운로드")), None);
            a.settle();
            if spaces {
                // 다른 공간의 화면은 지금 화면에서 칸 나누기만 바꾼 것 (0 한 칸, 1 좌우 두 칸, 3 세 칸, 4 네 칸)
                let space = |name: &str, color: &str, layout: u8, pins: Vec<Favorite>| {
                    let mut state = a.to_session();
                    state.layout = layout;
                    Workspace { name: name.into(), color: color.into(), pins, state }
                };
                let list = vec![
                    space("업무", "blue", 1, vec![item("2026 4분기", "업무/2026 4분기", true), item("회의록_11월.md", "업무/2026 4분기/회의록_11월.md", false), item("팀 공유", "팀 공유", true)]),
                    space("사진 정리", "purple", 4, vec![item("제주살이", "사진/제주살이", true), item("사진", "사진", true)]),
                    space("개인", "orange", 3, vec![item("다운로드", "다운로드", true), item("숙소 안내", "다운로드/제주살이_숙소_안내.zip", false)]),
                ];
                a.set_workspaces(Workspaces { list, current: Some(0) });
            }
            a
        };

        // 기본: 업무 공간(파랑). 좌우 두 칸, 왼쪽이 지금 칸, 한 줄 고름, 오른쪽 칸 줄에 마우스
        let mut a = base(favs.clone(), true);
        a.tab_mut(0).select_only(1);
        a.hover = Hit::Row(1, 2);
        shots.push(("basic", a));

        // 네 칸: 사진 정리 공간(보라)으로 바꾼 화면. 아래 두 칸은 사진·팀 공유, 지금 칸은 오른쪽 아래
        let mut a = base(favs.clone(), true);
        a.switch_workspace(1);
        a.navigate(2, Location::Dir(d("사진/제주살이")), None);
        a.navigate(3, Location::Dir(d("팀 공유")), None);
        a.settle();
        a.activate_pane(3);
        shots.push(("four", a));

        // 세 칸: 개인 공간(주황)으로 바꾼 화면. 하위 폴더에서 위로 올라와 떠난 폴더가 골라진 상태, 탭 두 개
        let mut a = base(favs.clone(), true);
        a.switch_workspace(2);
        a.settle();
        a.new_tab(0);
        a.go_up(0);
        a.settle();
        shots.push(("places", a));

        // 숨김 항목 보기
        let mut a = base(favs.clone(), true);
        a.toggle_hidden();
        a.activate_pane(1);
        shots.push(("hidden", a));

        // 처음 실행한 모습: '기본' 공간 하나, 고정한 것과 즐겨찾기가 없다. 빈 폴더, 한 칸
        let mut a = base(Favorites::default(), false);
        a.set_layout(Layout::One);
        a.navigate(0, Location::Dir(d("업무/빈 폴더")), None);
        a.settle();
        shots.push(("empty", a));

        // 넓은 한 칸: 유형 열(Finder '종류' 이름)이 보인다
        let mut a = base(favs.clone(), true);
        a.set_layout(Layout::One);
        a.navigate(0, Location::Dir(d("다운로드")), None);
        a.settle();
        shots.push(("one", a));

        // 정렬: 왼쪽 칸은 수정한 날짜 최신순(머리글에 화살표), 오른쪽 칸은 아이콘 보기에서 크기 칩으로 정렬하고 이름 칩에 마우스
        let mut a = base(favs.clone(), true);
        a.toggle_sort(0, sakadu::pane::SortKey::Modified);
        a.set_view(1, sakadu::pane::ViewMode::Medium);
        a.toggle_sort(1, sakadu::pane::SortKey::Size);
        a.hover = Hit::Header(1, sakadu::pane::SortKey::Name);
        a.settle();
        shots.push(("sorted", a));

        // 아이콘 보기: 왼쪽은 보통 아이콘(사진 폴더. 그림 파일은 썸네일로 보인다), 오른쪽은 목록.
        // 썸네일은 settle이 바로 읽는다: 그림은 직접, 메모(.md) 같은 그림이 아닌 파일은 Quick Look이 만든다.
        // CI가 만드는 내용 없는 jpg는 읽히지 않아 파일 아이콘으로 남는다
        let mut a = base(favs.clone(), true);
        a.navigate(0, Location::Dir(d("사진/제주살이")), None);
        a.set_view(0, sakadu::pane::ViewMode::Medium);
        a.set_view(1, sakadu::pane::ViewMode::List);
        a.settle();
        shots.push(("icons", a));

        // 경로 입력 중 (오른쪽 칸), 왼쪽 칸 칩 줄에는 없는 경로 알림
        let mut a = base(favs.clone(), true);
        a.commit_address_for_render(0, "/없는/경로");
        a.begin_address(1);
        shots.push(("address", a));

        // 이름 바꾸기: 왼쪽 칸에서 파일 하나를 고르고 Return을 누른 모습 (그 줄에 이름 상자, 확장자 앞까지 골라 둔다)
        let mut a = base(favs.clone(), true);
        a.tab_mut(0).select_only(1);
        a.begin_item_rename(0);
        shots.push(("rename", a));

        // 자동 미리 보기: 왼쪽 칸에서 그림을 하나 고르면 오른쪽 칸 아래에 카드가 뜬다 (그림은 settle이 바로 읽는다)
        let preview = |file: &str| {
            let mut a = base(favs.clone(), true);
            a.navigate(0, Location::Dir(d("사진/제주살이")), Some(file.to_string()));
            a.settle();
            a
        };
        shots.push(("preview", preview(DEMO_PICTURE)));

        // 크게 보기: 카드가 놓인 칸을 채운다
        let mut a = preview(DEMO_PICTURE);
        a.toggle_card_large();
        a.settle();
        shots.push(("preview-large", a));

        // 글 파일의 카드: 앞부분을 보여 준다
        shots.push(("preview-text", preview(DEMO_NOTE)));

        // 문서의 카드: PDF를 고르면 Quick Look이 만든 첫 쪽을 크게 보인다 (아이콘처럼 꾸미지 않고 가는 테두리만)
        shots.push(("preview-doc", preview(DEMO_PDF)));

        // 오른쪽 미리 보기 창(⌥⌘P): 글 파일을 고정해 두고 지금 칸은 다른 폴더로 옮긴 모습. 핀이 켜져 있고 칸들은 그만큼 좁다
        let mut a = preview(DEMO_NOTE);
        a.run_action(sakadu::keys::Action::Preview);
        a.settle();
        a.toggle_preview_pin();
        a.settle();
        a.navigate(0, Location::Dir(d("업무/2026 4분기")), None);
        a.settle();
        shots.push(("pane", a));

        // 압축 파일 안: 왼쪽 칸은 zip 안의 폴더 (새 폴더를 만들려 하면 읽기만 한다는 알림), 오른쪽 칸은 그 zip을 고른 폴더와
        // 오른쪽 미리 보기 창에 보이는 zip 안의 목록
        if let Some(zip) = &demo_zip {
            let mut a = base(favs.clone(), true);
            a.navigate(0, Location::Dir(util::join(zip, DEMO_ZIP_DIR)), None);
            a.navigate(1, Location::Dir(d("업무/문서")), Some(DEMO_ZIP.to_string()));
            a.settle();
            a.activate_pane(1);
            a.run_action(sakadu::keys::Action::Preview);
            a.settle();
            a.new_folder(0);
            shots.push(("archive", a));
        }

        // 검색을 처음 연 모습: 꺼져 있어 사이드바 자리에 '검색 켜기' 카드가 보인다
        let mut a = base(favs.clone(), true);
        a.open_find();
        shots.push(("find-intro", a));

        // 검색: 데모 폴더만 훑은 색인에서 '제주'를 찾고 둘째 결과를 고른 모습 (아래 줄에 그 위치)
        let mut a = base(favs.clone(), true);
        a.set_find_roots(vec![root.clone()]);
        a.open_find();
        a.enable_find();
        a.find_typed("제주");
        a.settle();
        a.find_key(Key::Down, false);
        shots.push(("find", a));

        // 새 탭 시작 화면: 왼쪽 칸에 시작 화면 탭을 연 모습. 고정한 폴더 카드, 오른쪽 칸에서 다녀간 최근 위치, 위치 카드
        let mut a = base(favs.clone(), true);
        for rel in ["사진", "업무", "다운로드"] {
            a.navigate(1, Location::Dir(d(rel)), None);
        }
        a.set_new_tab_start(true);
        a.new_tab(0);
        a.settle();
        a.hover = Hit::Start(0, 1);
        shots.push(("start", a));

        // 명령 메뉴(⌘K): 초성 'ㄷㅇ'을 친 모습 (다운로드 탭과 최근 위치, 닫은 탭 다시 열기·도움말 명령, 디스크에서 찾기)
        let mut a = base(favs.clone(), true);
        a.toggle_commands();
        a.cmd_typed("ㄷㅇ");
        shots.push(("commands", a));

        // 메뉴 (앱이 그린다): 보기 방식 단추의 메뉴, 파일 줄을 오른쪽 단추로 누른 메뉴, 작업 공간 아이콘의 색 메뉴,
        // 사이드바 즐겨찾기 그룹 머리의 메뉴(이름 바꾸기, 그룹 풀기, 그룹 지우기). 한 줄을 가리킨 모습
        let measure = Gfx::bitmap(8.0, 8.0, 1.0).ok_or("그림판을 만들지 못함")?;
        for (name, kind, sel) in [("menu-view", MenuKind::View, 1), ("menu-rows", MenuKind::Rows, 3), ("menu-space", MenuKind::Space(1), 2), ("menu-group", MenuKind::Group(0), 1)] {
            let mut a = base(favs.clone(), true);
            a.open_menu_for_render(&measure, kind, sel);
            shots.push((name, a));
        }

        // 디스크: '모든 공간'의 디스크 줄 오른쪽에 남은 용량이 옅게 보이고, 꺼낼 수 있는 디스크 줄을 오른쪽 단추로 누르면 '꺼내기' 메뉴.
        // 디스크 자리는 러너에 늘 있는 곳(시동 디스크로 가는 /Volumes의 바로가기, 데이터 볼륨)을 쓰고 용량은 고정한 값을 넣는다
        let mut a = base(favs.clone(), true);
        let (usb, link) = ("/System/Volumes/Data".to_string(), "/Volumes/Macintosh HD".to_string());
        let mut disks = places.clone();
        disks.extend([("Macintosh HD".to_string(), link.clone()), ("SAKADU".to_string(), usb.clone())]);
        a.on_volumes(disks, None);
        let gb = |n: u64| n << 30;
        a.set_volumes([(link, Space { free: gb(182), total: gb(494), mount: false }), (usb, Space { free: gb(23), total: gb(64), mount: true })].into());
        let (x, y) = a.side_row_center("SAKADU").ok_or("사이드바에 'SAKADU' 줄이 없음")?;
        a.right_down(&measure, x, y);
        shots.push(("disks", a));

        // 사이드바 끌기 한가운데: '제주살이' 그룹의 '맛집 메모'를 누른 채 한 줄 위로 끈 모습. 그 줄은 흰 카드로 떠 있고,
        // 그룹 머리 바로 아래에 빈자리가 열리며 그룹이 판으로 묶인다 (처음 그리는 그림이라 줄은 이미 제자리에 있다)
        let mut a = base(favs.clone(), true);
        let (x, y) = a.side_row_center("맛집 메모").ok_or("사이드바에 '맛집 메모' 줄이 없음")?;
        a.mouse_down(&measure, x, y, Mods::default(), 1);
        a.mouse_dragged(x, y - 30.0);
        shots.push(("side-drag", a));

        for (name, a) in shots {
            let mut g = Gfx::bitmap(W, H, SCALE).ok_or("그림판을 만들지 못함")?;
            a.paint(&mut g);
            let path = out.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }));
            g.save_png(&path)?;
            written.push(path);
        }
    }
    Ok(written)
}
