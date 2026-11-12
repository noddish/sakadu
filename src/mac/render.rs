//! 그림 시험: 창 없이 App을 몇 가지 상태로 만들어 PNG로 그린다 (CI에서 맥 화면을 확인하는 용도)

use crate::app::{App, Hit};
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

/// demo 폴더(CI가 만드는 한글 이름 데모)로 상태 여덟 가지 × 밝은·어두운 테마를 out에 그린다.
/// 작업 공간은 셋: 업무(파랑, 두 칸), 사진 정리(보라, 네 칸), 개인(주황, 세 칸). 공간마다 고정한 것이 다르고,
/// 아래 '모든 공간'(바로가기와 즐겨찾기)은 어느 공간에서나 같다
pub fn render_all(demo: &Path, out: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let root = util::normalize(&demo.to_string_lossy());
    let d = |rel: &str| rel.split('/').fold(root.clone(), |p, s| util::join(&p, s));
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

        // 아이콘 보기: 왼쪽은 보통 아이콘, 오른쪽은 목록
        let mut a = base(favs.clone(), true);
        a.set_view(0, sakadu::pane::ViewMode::Medium);
        a.set_view(1, sakadu::pane::ViewMode::List);
        shots.push(("icons", a));

        // 경로 입력 중 (오른쪽 칸), 왼쪽 칸 칩 줄에는 없는 경로 알림
        let mut a = base(favs.clone(), true);
        a.commit_address_for_render(0, "/없는/경로");
        a.begin_address(1);
        shots.push(("address", a));

        for (name, a) in shots {
            let mut g = Gfx::bitmap(W, H, 2.0).ok_or("그림판을 만들지 못함")?;
            a.paint(&mut g);
            let path = out.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }));
            g.save_png(&path)?;
            written.push(path);
        }
    }
    Ok(written)
}
