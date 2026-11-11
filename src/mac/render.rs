//! 그림 시험: 창 없이 App을 몇 가지 상태로 만들어 PNG로 그린다 (CI에서 맥 화면을 확인하는 용도)

use crate::app::{App, Hit, Side};
use sakadu::ui::Layout;
use crate::gfx::Gfx;
use sakadu::favorites::{Favorite, Favorites, Group};
use sakadu::palette::Theme;
use sakadu::pane::Location;
use sakadu::util;
use std::path::Path;

const W: f32 = 1280.0;
const H: f32 = 800.0;

/// demo 폴더(CI가 만드는 한글 이름 데모)로 상태 여섯 가지 × 밝은·어두운 테마를 out에 그린다
pub fn render_all(demo: &Path, out: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let root = util::normalize(&demo.to_string_lossy());
    let d = |rel: &str| rel.split('/').fold(root.clone(), |p, s| util::join(&p, s));
    let favs = Favorites {
        items: vec![
            Favorite { name: "다운로드".into(), path: d("다운로드"), is_dir: true },
            Favorite { name: "2026 4분기".into(), path: d("업무/2026 4분기"), is_dir: true },
            Favorite { name: "회의록_11월.md".into(), path: d("업무/2026 4분기/회의록_11월.md"), is_dir: false },
        ],
        groups: vec![
            Group {
                name: "제주살이".into(),
                collapsed: false,
                items: vec![
                    Favorite { name: "여행 사진".into(), path: d("사진/제주살이"), is_dir: true },
                    Favorite { name: "맛집 메모".into(), path: d("사진/제주살이/맛집 메모.md"), is_dir: false },
                    Favorite { name: "숙소 안내".into(), path: d("다운로드/제주살이_숙소_안내.zip"), is_dir: false },
                ],
            },
            Group { name: "프로젝트".into(), collapsed: true, items: Vec::new() },
        ],
    };
    let places = vec![("홈".to_string(), root.clone()), ("문서".to_string(), d("업무")), ("다운로드".to_string(), d("다운로드")), ("사진".to_string(), d("사진"))];
    let mut written = Vec::new();
    for dark in [false, true] {
        let theme = if dark { Theme::DARK } else { Theme::LIGHT };
        let mut shots: Vec<(&str, App)> = Vec::new();
        let base = |favs: Favorites| {
            let mut a = App::new(W, H, theme, &root, favs, places.clone());
            a.standalone = true;
            a.navigate(0, Location::Dir(d("업무/2026 4분기")), None);
            a.navigate(1, Location::Dir(d("다운로드")), None);
            a.settle();
            a
        };

        // 기본: 좌우 두 칸, 왼쪽이 지금 칸, 한 줄 고름, 오른쪽 칸 줄에 마우스
        let mut a = base(favs.clone());
        a.tab_mut(0).select_only(1);
        a.hover = Hit::Row(1, 2);
        shots.push(("basic", a));

        // 네 칸: 아래 두 칸은 사진·팀 공유, 지금 칸은 오른쪽 아래
        let mut a = base(favs.clone());
        a.set_layout(Layout::Four);
        a.navigate(2, Location::Dir(d("사진/제주살이")), None);
        a.navigate(3, Location::Dir(d("팀 공유")), None);
        a.settle();
        a.activate_pane(3);
        shots.push(("four", a));

        // 바로가기 사이드바, 하위 폴더에서 위로 올라와 떠난 폴더가 골라진 상태, 탭 두 개
        let mut a = base(favs.clone());
        a.set_side(Side::Places);
        a.new_tab(0);
        a.go_up(0);
        a.settle();
        shots.push(("places", a));

        // 숨김 항목 보기
        let mut a = base(favs.clone());
        a.toggle_hidden();
        a.activate_pane(1);
        shots.push(("hidden", a));

        // 빈 즐겨찾기와 빈 폴더, 한 칸
        let mut a = base(Favorites::default());
        a.set_layout(Layout::One);
        a.navigate(0, Location::Dir(d("업무/빈 폴더")), None);
        a.settle();
        shots.push(("empty", a));

        // 경로 입력 중 (오른쪽 칸), 왼쪽 칸 칩 줄에는 없는 경로 알림
        let mut a = base(favs.clone());
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
