//! 맥 시제품 앱: 칸·탭·사이드바·작업 공간 상태, 그리기(paint), 누르기 판정(hit), 동작.
//! 사이드바가 창 위까지 올라가고 신호등은 그 안에 있다. 사이드바 왼쪽에는 작업 공간 레일, 그 옆에는 지금 공간의 이름과
//! 그 공간에 고정한 것, 아래에는 어느 공간에서나 같은 '모든 공간'이 온다.
//! 화면 계산·그리기 도우미·치수와 칸·도구 막대·레일 그리기는 Windows와 함께 쓰는 sakadu::ui를 쓴다
//! (이 앱의 상태로 장면을 채워 넘긴다).
//! 그림·글·문서 파일을 하나 고르면 다른 칸의 오른쪽 아래에 미리 보기 카드가 저절로 뜬다 (도구 막대의 눈으로 끄고 켠다).
//! 문서(PDF·Office·동영상 …)는 Quick Look 썸네일, 한글 문서는 안에 든 미리 보기 그림·글을 보이고, 썸네일이 없으면 카드를 띄우지 않는다.
//! 오른쪽 미리 보기 창(⌥⌘P)은 preview_pane.rs: 열려 있는 동안은 카드 대신 그 창이 보여 준다.
//! 아이콘 보기에서는 그림 파일이 썸네일로 보인다 (뒷줄에서 읽어 둔 것을 그린다).
//! 파일 작업(새 폴더, 이름 바꾸기, 휴지통, 복사, 옮기기, 되돌리기)은 할 일을 큐에 넣어 두면 창 쪽이 뒷줄에서 실행하고 결과를 돌려준다 (fileops.rs).
//! 클립보드도 창 쪽이 다룬다: 앱은 담아 달라는 경로(clip_out)와 붙여넣어 달라는 표시(clip_in)만 남긴다.
//! 끌어서 놓기도 그렇다: 앱은 끌어 낼 항목(take_drag)을 내주고, 끌려 온 것이 어디에 놓일지(drag_over)와 놓기(drop_paths)를 맡는다.
//! 검색(⌘F)은 find.rs: 도구 막대에서 입력 칸이 늘어나고, 결과는 사이드바 자리에 보인다.
//! 명령 메뉴(⌘K)는 commands.rs, 새 탭 시작 화면과 최근 위치는 start.rs (둘 다 Windows와 같은 공통 코드를 쓴다).
//! 사이드바 줄은 누를 때가 아니라 뗄 때 열고, 위아래로 끌면 옮긴다: 고정한 것끼리, 즐겨찾기 항목과 그룹, 그 둘 사이
//! (빈자리·놓을 곳 계산과 미끄러지는 움직임은 Windows와 같은 공통 ui::side_flow). 즐겨찾기 그룹은 '모든 공간' 줄의 단추로 만들고,
//! 그룹 머리의 메뉴로 이름을 바꾸고 풀고 지운다. 끌려 온 파일은 탭 칩 위에 놓으면 그 탭의 폴더로 간다.
//! 창(NSView)과 상관없이 돌아가서, 시험과 그림(--render)은 창 없이 같은 코드를 쓴다

mod commands;
mod find;
mod preview_pane;
mod start;

pub use commands::CMD_PLACEHOLDER;
pub use preview_pane::{PinCheck, check_pin};
pub use start::volume_space;

use crate::fileops::{self, Done, Fail, Job, Op};
use crate::gfx::{Gfx, MONO_LINE_H, Picture};
use sakadu::draw::{Color, Font, R};
use sakadu::favorites::{self, Favorite, Favorites, Row};
use sakadu::fileops::{Undo, UndoStack};
use sakadu::keys::{self, Action, Combo, Keymap};
use sakadu::listing::{self, Entry, Kind};
use sakadu::palette::{self, R_CTRL, R_ROW, System, Theme, alpha};
use sakadu::pane::{DATE_AUTO, DEFAULT_COLS, LoadState, Location, Restore, Sort, SortKey, Tab, ViewMode, child_toward};
use sakadu::session::{self, Look, PaneState, Session};
use sakadu::ui::glyph::*;
use sakadu::ui::menu::{MenuIcon, MenuLine, MenuScene, menu_height, menu_hit, menu_max_scroll, menu_place, menu_reveal, menu_step, menu_width, paint_menu};
use sakadu::ui::paint::{EntryArt, PaneHover, PaneScene, paint_pane_border, paint_pane_head, paint_pane_list};
use sakadu::ui::preview_card::{CardArt, CardBody, CardGeom, CardHover, CardScene, IMAGE_EXTS, TEXT_EXTS, card_geom, paint_preview_card};
use sakadu::ui::rail::{RailGeom, RailHover, RailScene, RailSpace, paint_rail, rail_geom};
use sakadu::ui::side_flow::{DropPlan, DropTo, FlowKind, FlowSlot, SideFlow, drop_plan, group_span, paint_drag_card, paint_drop_area, paint_group_plate};
use sakadu::ui::toolbar::{ToolbarGeom, ToolbarHover, ToolbarScene, ToolbarSpec, can_go_up, paint_toolbar, toolbar_geom};
use sakadu::ui::*;
use sakadu::util;
use sakadu::workspaces::{self, Workspace, Workspaces};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

/// 사이드바 폭: 작업 공간 레일과 그 옆 패널(220)을 합친 폭
const SIDE_W: f32 = BAR_W + 220.0;
/// 맥 창의 신호등 아래 끝 (창에서 읽기 전 기본값, 그림 시험에서도 쓴다)
pub const LIGHTS_BOTTOM: f32 = 28.0;
/// 고정한 것이 없을 때 보이는 안내(두 줄)의 높이
const HINT_H: f32 = 44.0;
/// 고정한 것과 '모든 공간' 사이 간격 (줄이 많아 둘이 이어질 때)
const COMMON_GAP: f32 = 12.0;

/// 카드의 글: 파일 앞쪽 64KB에서 앞 80줄, 한 줄은 200자까지 (그릴 때마다 긴 글을 배치하지 않게 앞부분만 둔다)
const TEXT_READ: u64 = 64 * 1024;
const TEXT_LINES: usize = 80;
const TEXT_COLS: usize = 200;
/// 카드의 그림: 긴 쪽을 이 픽셀 수까지 줄여 읽는다 (작은 카드, 크게 보기)
const PX_SMALL: u32 = 1024;
const PX_LARGE: u32 = 2048;
/// 읽어 둔 그림을 몇 개까지 남겨 두나 (방금 본 그림으로 돌아가면 다시 읽지 않는다)
const PICS_MAX: usize = 6;
/// 그림이 이보다 늦게 오면 카드에 '불러오는 중…'을 보인다 (곧 오면 글자가 깜빡이지 않게)
pub const LOADING_NOTE_MS: u64 = 300;

/// 아이콘 보기의 썸네일을 읽는 픽셀 수의 단계: 그림 자리(pt)에 화면 배율을 곱한 값을 이 가운데 하나로 올려 읽는다
/// (아이콘 크기나 화면 배율이 조금 다를 때마다 다시 읽지 않게). 가장 큰 단계는 레티나 화면의 아주 큰 아이콘(256pt)
const THUMB_PX: [u32; 4] = [64, 128, 256, 512];
/// 썸네일을 한 번에 몇 개까지 뒷줄에서 읽나
const THUMB_JOBS: usize = 2;
/// 읽어 둔 썸네일을 몇 개, 몇 바이트까지 남겨 두나 (넘으면 오래 안 쓴 것부터 버린다)
const THUMBS_MAX: usize = 400;
const THUMB_BYTES: usize = 64 * 1024 * 1024;

/// 마우스가 가리키는 것 (칸 번호 p, 탭·줄 번호 i)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    None,
    /// 레일의 작업 공간 아이콘 (공간 번호)
    Space(usize),
    /// 레일의 새 공간(+)
    SpaceAdd,
    /// 사이드바 맨 위, 지금 공간의 이름 (두 번 누르면 이름을 고친다)
    SideTitle,
    /// 사이드바 목록의 줄 (side_rows의 번호)
    SideRow(usize),
    /// '모든 공간' 이름 줄 오른쪽 끝의 새 그룹 단추 (Windows의 같은 자리 단추)
    NewGroup,
    /// 도구 막대의 칸 나누기 단추, 칸의 경로 줄에 있는 보기 방식 단추 (누르면 메뉴가 열린다)
    LayoutPick,
    ViewPick(usize),
    /// 도구 막대의 눈 (숨김 항목을 보이고 감춘다: ⌘⇧.와 같다)
    HiddenToggle,
    /// 도구 막대의 자동 미리 보기 단추
    PreviewToggle,
    /// 도구 막대의 돋보기 (검색이 닫혀 있을 때), 늘어난 입력 칸, 그 칸의 닫기
    FindOpen,
    FindField,
    FindClose,
    /// 검색 패널 안 (검색하는 동안 사이드바 자리)
    Find(find::FindHit),
    /// 미리 보기 카드의 크게 보기 단추, 닫기 단추, 카드의 그 밖의 곳 (카드 아래 칸을 누르지 않게 카드가 받는다)
    CardExpand,
    CardClose,
    Card,
    /// 오른쪽 미리 보기 창 머리의 고정(핀)·열기 단추, 창의 그 밖의 곳 (아래 칸을 누르지 않게 창이 받는다)
    PreviewPin,
    PreviewOpen,
    PreviewArea,
    Tab(usize, usize),
    TabClose(usize, usize),
    TabAdd(usize),
    /// 도구 막대의 뒤로·앞으로 (지금 칸)
    Back(usize),
    Fwd(usize),
    Crumb(usize, usize),
    /// 경로 줄의 빈 곳 (누르면 경로 입력)
    CrumbEmpty(usize),
    Star(usize),
    /// 열 머리글(자세히 보기)이나 정렬 칩(아이콘·목록 보기): 누르면 그 열로 정렬한다
    Header(usize, SortKey),
    /// 머리글의 열 경계 (resizable의 번호: 0 날짜, 1 유형, 2 크기): 끌어서 폭을 바꾼다
    HeaderResize(usize, usize),
    Row(usize, usize),
    List(usize),
    Pane(usize),
    SplitX,
    SplitY,
    /// 두 나눔선이 만나는 곳 (4칸 가운데, 3칸 T자): 두 선을 함께 끈다
    SplitXY,
    /// 열린 메뉴 안 (고를 수 있는 줄이면 그 번호). 메뉴가 열려 있으면 메뉴 밖은 Hit::None이다
    Menu(Option<usize>),
    /// 시작 화면의 항목 (칸, 항목 번호)
    Start(usize, usize),
    /// 명령 메뉴의 줄, 판의 그 밖의 곳, 판 밖 (누르면 닫힌다). 명령 메뉴가 열려 있으면 모든 것 위에 있다
    CmdRow(usize),
    CmdPanel,
    CmdScrim,
}

/// 마우스 모양: 경계를 끌 수 있는 곳(열 경계, 칸 나눔선)에서는 끄는 방향의 화살표.
/// 두 나눔선이 만나는 곳은 맥에 네 방향 모양이 없어 보통 화살표다
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pointer {
    Arrow,
    LeftRight,
    UpDown,
}

/// 누른 수정 키
#[derive(Clone, Copy, Default, Debug)]
pub struct Mods {
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// 앱이 직접 그리는 메뉴의 종류 (단추 메뉴와 오른쪽 단추 메뉴)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuKind {
    /// 칸 나누기 다섯 가지
    Layout,
    /// 보기 방식 여섯 가지와 맨 아래 '숨김 항목 보기'
    View,
    /// 레일의 공간 아이콘을 오른쪽 단추로 눌렀을 때 (공간 번호): 색 여섯 가지, 이름 바꾸기, 지우기
    Space(usize),
    /// 레일의 + 를 오른쪽 단추로 눌렀을 때: 모든 작업 공간 (창이 낮아 레일에 다 보이지 않는 공간도 여기서 고른다)
    Spaces,
    /// 사이드바의 고정한 줄을 오른쪽 단추로 눌렀을 때 (이 공간에 고정한 것 가운데 번호): 위·아래로 옮기기, 모든 공간에 두기, 고정 풀기
    Pin(usize),
    /// 사이드바 '모든 공간'의 즐겨찾기 항목을 오른쪽 단추로 눌렀을 때 (Favorites::rows의 줄 번호): 이 작업 공간에만 두기, 모든 공간에서 빼기,
    /// 그룹으로 옮기기 (Windows의 '그룹으로 이동' 하위 메뉴를 펼쳐 둔 줄들)
    Fav(usize),
    /// 즐겨찾기 그룹 머리를 오른쪽 단추로 눌렀을 때 (그룹 번호): 이름 바꾸기, 그룹 풀기, 그룹과 안의 즐겨찾기 빼기
    Group(usize),
    /// 그룹과 안의 즐겨찾기를 뺄지 한 번 더 묻는다 (Windows의 확인 창과 같은 물음): 빼기, 그만두기
    GroupDelete(usize),
    /// 목록의 줄을 오른쪽 단추로 눌렀을 때: 고른 항목들의 메뉴 (열기, Finder에서 보기, 이름 바꾸기, 휴지통으로 이동)
    Rows,
    /// 목록의 빈 곳을 오른쪽 단추로 눌렀을 때: 그 폴더의 메뉴 (새 폴더, 새로 고침)
    Blank,
}

/// 열린 메뉴: 종류, 붙인 자리, 줄(열 때 정한다), 가리킨 줄(마우스·키보드), 스크롤
struct OpenMenu {
    kind: MenuKind,
    anchor: R,
    lines: Vec<MenuLine>,
    /// 폭 (열 때 가장 긴 줄로 정한다)
    w: f32,
    sel: Option<usize>,
    scroll: f32,
}

/// 고정한 줄 메뉴의 줄
#[derive(Clone, Copy, PartialEq, Debug)]
enum PinAct {
    Up,
    Down,
    ToCommon,
    Unpin,
}

impl PinAct {
    fn label(self) -> &'static str {
        match self {
            PinAct::Up => "위로 이동",
            PinAct::Down => "아래로 이동",
            PinAct::ToCommon => "모든 공간에 두기",
            PinAct::Unpin => "고정 풀기",
        }
    }
}

/// 즐겨찾기 항목 메뉴의 줄
#[derive(Clone, Copy, PartialEq, Debug)]
enum FavAct {
    /// 이 작업 공간에만 두기
    ToPins,
    /// 모든 공간에서 빼기
    Remove,
    /// 그 그룹으로 (None이면 그룹 밖으로)
    ToGroup(Option<usize>),
    /// 새 그룹을 만들어 넣기
    NewGroup,
}

/// 목록 메뉴(고른 항목들)의 줄
#[derive(Clone, Copy, PartialEq, Debug)]
enum RowAct {
    Open,
    Reveal,
    Rename,
    Cut,
    Copy,
    Duplicate,
    Trash,
}

impl RowAct {
    fn label(self) -> &'static str {
        match self {
            RowAct::Open => "열기",
            RowAct::Reveal => "Finder에서 보기",
            RowAct::Rename => "이름 바꾸기",
            RowAct::Cut => "잘라내기",
            RowAct::Copy => "복사",
            RowAct::Duplicate => "복제",
            RowAct::Trash => "휴지통으로 이동",
        }
    }
}

/// 목록 메뉴를 열 때 적어 둔 대상: 그 탭, 그 폴더, 고른 항목들(이름, 파일 번호, 폴더인지), 누른 자리.
/// 메뉴가 떠 있는 동안 목록이나 고른 것이 바뀌어도, 고른 줄은 이때 적어 둔 항목에 한다
struct ListTarget {
    pane: usize,
    tab: u64,
    dir: String,
    items: Vec<(String, u64, bool)>,
    at: (f32, f32),
}

/// 붙여넣기의 종류
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paste {
    /// 복사한다 (⌘C 뒤 ⌘V)
    Copy,
    /// 복사해 둔 것을 옮긴다 (⌥⌘V)
    Move,
    /// 잘라 둔 것을 옮긴다 (⌘X 뒤 ⌘V). 끝나면 옮겨 간 것을 클립보드에서 뺀다.
    /// 수는 창 쪽이 준 표식(잘라 둘 때의 클립보드 번호)이다: 끝났을 때 그대로 돌려줘서, 그 사이 다른 것을 잘라 뒀으면
    /// 그 새것을 건드리지 않게 한다
    Cut(i64),
    /// 끌어다 놓았다: 같은 디스크면 옮기고, 다른 디스크면 복사한다 (Finder와 같다)
    Drop,
}

/// 끌어 온 것을 놓을 곳
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropAt {
    /// 칸 p가 보이고 있는 폴더
    Pane(usize),
    /// 칸 p의 폴더 줄 vi (그 폴더 안으로)
    Row(usize, usize),
    /// 사이드바의 폴더 줄 (side_rows의 번호)
    Side(usize),
    /// 칸 p의 탭 칩 i (그 탭이 보이는 폴더로. 지금 탭이 아니어도)
    Tab(usize, usize),
}

/// 고른 줄을 누른 것: 누른 채 조금 넘게 움직이면 그 항목들을 끌어 낸다
struct Press {
    pane: usize,
    row: usize,
    x: f32,
    y: f32,
    /// 여럿을 고른 것 가운데 하나를 그냥 눌렀다: 끌지 않고 떼면 그때 그 줄만 고른다
    collapse: bool,
}

/// 줄을 한 번 눌렀을 때 그 줄의 항목: 이어서 두 번째로 누르면(두 번 누르기) 이 항목을 연다.
/// 두 번 사이에 목록이 바뀌어 손 밑에 다른 줄이 와 있을 수 있어서, 줄 번호가 아니라 항목으로 기억한다
struct Clicked {
    pane: usize,
    tab: u64,
    file_id: u64,
    name: String,
}

/// 누른 채 이만큼 넘게 움직여야 끌기로 본다 (누르다가 손이 조금 떨린 것은 끌기가 아니다)
const DRAG_START: f32 = 4.0;

/// 누른 사이드바 줄 (Windows의 FavPress·PinPress, 끌기 시작하면 FavMove·PinMove): 그대로 떼면 그 줄을 열고,
/// 위아래로 조금 넘게 움직이면 그 줄을 카드로 띄워 끌어 옮긴다
#[derive(Clone, Copy, Debug)]
struct SidePress {
    /// side_rows의 번호
    row: usize,
    /// 끌 수 있는 줄이면 무엇인지 (고정한 줄, 즐겨찾기 항목·그룹 머리). 바로가기는 끌지 않는다
    what: Option<FlowKind>,
    /// 누른 y
    y0: f32,
    /// 끄는 중이면 (마우스 y, 카드 위쪽에서 잡은 자리까지)
    moving: Option<(f32, f32)>,
}

/// '모든 공간' 이름 줄의 작은 단추 (새 그룹) 한 변
const MINI: f32 = 24.0;

/// 창 시험(`sakadu-mac --smoke`)이 보는 앱의 상태: 지금 칸의 지금 탭
#[derive(Debug)]
pub struct Seen {
    /// 그 탭의 폴더
    pub dir: String,
    /// 목록의 이름 (보이는 순서)
    pub names: Vec<String>,
    /// 커서 줄의 이름
    pub cursor: String,
    /// 고른 줄의 이름 (보이는 순서)
    pub selected: Vec<String>,
    /// 다 읽었는지
    pub ready: bool,
    /// 이름을 고치는 중인 항목
    pub editing: Option<String>,
    /// 칩 줄에 떠 있는 알림
    pub flash: Option<String>,
    /// 맡긴 파일 작업의 결과를 아직 다 받지 않았다 (그동안은 되돌리기를 받지 않는다)
    pub busy: bool,
    /// 지금 칸의 탭 수
    pub tabs: usize,
    /// 그 탭이 새 탭 시작 화면이다
    pub start: bool,
}

/// 설정 창에 보이는 값. 창 쪽이 이것으로 스위치와 글을 맞춘다 (입력과 뒷줄 결과를 처리할 때마다)
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct Prefs {
    pub hidden: bool,
    pub auto_preview: bool,
    /// 이름으로 찾기를 켰다
    pub search_on: bool,
    pub icloud: bool,
    /// 색인을 훑는 중이다 (켰는데 아직 처음 훑기를 마치지 않았다)
    pub crawling: bool,
    /// 맥이 묻는 폴더(데스크탑·문서·다운로드·iCloud Drive)를 읽지 못했다
    pub denied: bool,
    /// 모양: "system", "light", "dark"
    pub theme: String,
    /// 새 탭(⌘T)을 시작 화면으로 연다 (아니면 지금 폴더)
    pub new_tab_start: bool,
}

/// 진행 중인 복사·옮기기 묶음 (붙여넣기, 복제). 항목을 하나씩 맡기고, 하나가 끝나면 다음 것을 맡긴다:
/// 그래서 몇 개 중 몇 개인지 보이고, 항목 사이에서 그만둘 수 있다
struct Batch {
    /// 붙여넣은 탭과 받는 폴더
    tab: u64,
    dir: String,
    /// 아직 맡기지 않은 것 (전체 경로, 목록에서 본 파일 번호. 모르면 0)
    todo: VecDeque<(String, u64)>,
    total: usize,
    /// 만든 복사본이나 옮긴 것 (받는 폴더에서의 이름, 파일 번호)과 그것의 원래 전체 경로 (같은 순서),
    /// 그 가운데 번호를 붙인 개수, 하지 못한 것 (원래 전체 경로, 까닭)
    made: Vec<(String, u64)>,
    sources: Vec<String>,
    renamed: usize,
    failed: Vec<(String, String)>,
    /// made와 같은 순서로, 그것을 옮겼는지(아니면 복사했다). 끌어다 놓기는 디스크에 따라 항목마다 다를 수 있다
    moved: Vec<bool>,
    /// 복제(같은 폴더 안으로): 번호가 붙는 것이 당연해서 알리지 않는다
    duplicate: bool,
    /// 복사인지, 옮기기인지(잘라 둔 것이면 끝난 뒤 클립보드에서 뺀다), 끌어다 놓기인지
    how: Paste,
    cancelled: bool,
}

/// path가 old이거나 그 아래면, old 뒤에 남는 부분 (빈 글자이거나 '/'로 시작한다)
fn under<'a>(path: &'a str, old: &str) -> Option<&'a str> {
    let rest = path.strip_prefix(old)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

/// 맥 키 (글자 키는 소문자 그대로, 스페이스는 ' ')
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    Esc,
    Return,
    /// 지우기 키(⌫). ⌘와 함께 누르면 휴지통으로 보낸다
    Delete,
    /// F1~F20
    F(u8),
    Char(char),
}

impl Key {
    /// 누르고 있을 때 되풀이해 받아도 되는 키인지. 파일을 만들거나 고치거나 옮기거나 휴지통으로 보내는 키
    /// (Return, ⌫, 새 폴더의 n, 복제의 d, 옮기기의 v)는 한 번 누름에 한 번만 받는다.
    /// 열고 닫는 키(스페이스, 훑어보기의 y)도 누르고 있는 동안 열렸다 닫혔다 하지 않게 한 번만 받는다
    pub fn repeats(self) -> bool {
        !matches!(self, Key::Return | Key::Delete | Key::Char('n' | 'd' | 'v' | ' ' | 'y'))
    }

    /// 단축키 표(keys.rs)의 키 조합으로: 키는 Windows 가상 키 코드로 적는다
    pub fn combo(self, m: Mods) -> Option<Combo> {
        let vk = match self {
            Key::Up => 0x26,
            Key::Down => 0x28,
            Key::Left => 0x25,
            Key::Right => 0x27,
            Key::Home => 0x24,
            Key::End => 0x23,
            Key::PageUp => 0x21,
            Key::PageDown => 0x22,
            Key::Tab => 0x09,
            Key::Esc => 0x1B,
            Key::Return => 0x0D,
            Key::Delete => 0x08,
            Key::F(n) => 0x6F + n as u16,
            Key::Char(' ') => 0x20,
            Key::Char(c) => keys::key_code(&c.to_ascii_uppercase().to_string())?,
        };
        Some(Combo { ctrl: m.ctrl, shift: m.shift, alt: m.alt, cmd: m.cmd, vk })
    }
}

/// 맥에서 도는 동작 (단축키 표와 메뉴 막대, 설정의 단축키에 보인다). 사이드바 목록으로 이동은 맥에 아직 없다
pub fn mac_runs(a: Action) -> bool {
    !matches!(a, Action::Favorites)
}

/// 바꾸지 않는 기본 조작의 키 (App::key가 단축키 표 다음에 받는 것, 편집 메뉴의 실행 복귀). 단축키로 정하지 않는다:
/// 정하면 단축키 표가 먼저 받아 그 조작을 키로 부를 수 없게 된다
pub fn fixed_key(c: Combo) -> bool {
    const FIXED: [&str; 13] = ["Shift+Cmd+G", "Cmd+Down", "Cmd+O", "Cmd+Backspace", "Cmd+D", "Alt+Cmd+V", "Cmd+Up", "Cmd+[", "Cmd+]", "Ctrl+Tab", "Ctrl+Shift+Tab", "Cmd+Y", "Shift+Cmd+Z"];
    FIXED.iter().filter_map(|s| Combo::parse(s)).any(|f| f == c)
}

/// 입력 상자(창 쪽의 글 상자 하나)가 지금 무엇을 고치는지
#[derive(Clone, PartialEq, Debug)]
pub enum Editing {
    None,
    /// 칸 p의 경로 입력
    Address(usize),
    /// 지금 공간의 이름
    Space,
    /// 목록 항목의 이름
    Item(ItemEdit),
    /// 즐겨찾기 그룹 g의 이름 (사이드바의 그 머리 줄에서)
    Group(usize),
}

/// 이름을 고치는 항목. 줄 번호가 아니라 탭·폴더·이름·파일 번호로 적어 둔다: 고치는 동안 목록이 다시 읽히거나 탭이 다른 곳으로
/// 가면, 그 항목이 아직 그 자리에 있는지 다시 본다 (App::item_edit)
#[derive(Clone, PartialEq, Debug)]
pub struct ItemEdit {
    pane: usize,
    tab: u64,
    dir: String,
    name: String,
    file_id: u64,
    is_dir: bool,
    /// 이 고치기의 번호 (입력 상자가 다른 항목으로 옮겨 갔는지 창 쪽이 안다)
    serial: u64,
    /// 확정해서 맡긴 이름 바꾸기의 작업 번호. 그 작업이 끝나기 전에는 다시 확정하지 않는다
    waiting: Option<u64>,
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub view: ViewMode,
    /// 정렬 (칸마다. 머리글이나 정렬 칩을 눌러 바꾼다)
    pub sort: Sort,
    /// 수정한 날짜·유형·크기 열 폭 (칸마다. 날짜가 pane::DATE_AUTO면 내용 폭)
    pub cols: [f32; 3],
}

impl Pane {
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
}

/// 끌고 있는 열 경계: 칸, 선 오른쪽 열, 선 왼쪽에 보이는 열(없으면 이름 칸), 누른 x, 누를 때의 열 폭 (ui::drag_column)
#[derive(Clone, Copy, Debug)]
struct ColDrag {
    pane: usize,
    col: usize,
    prev: Option<usize>,
    start_x: f32,
    start: [f32; 3],
}

/// 폴더 읽기 요청: 창 쪽이 뒷줄에서 읽고 on_listed로 돌려준다
pub struct Request {
    pub tab: u64,
    pub generation: u64,
    pub dir: String,
}

/// 경로 입력 확인 요청: 창 쪽이 뒷줄에서 resolve_input을 돌려 on_probed로 돌려준다 (꺼진 NAS를 확인하는 동안 멈추지 않게)
pub struct Probe {
    pub pane: usize,
    pub tab: u64,
    /// 그 칸의 경로 확인 요청 번호 (나중에 낸 요청이 이긴다)
    pub seq: u64,
    pub text: String,
    /// 확인한 폴더를 새 탭에서 연다 (명령 메뉴의 ⌘Return)
    pub new_tab: bool,
}

/// 주소 입력 해석 (app.rs resolve_input과 같은 규칙): 앞의 ~는 홈, 폴더면 그 폴더, 파일이면 그 폴더에서 그 파일을 고른다
pub fn resolve_input(text: &str, home: &str) -> Result<(String, Option<String>), String> {
    let t = text.trim();
    let missing = || format!("경로를 찾을 수 없어요: {t}");
    let expanded = match t.strip_prefix('~') {
        Some("") => home.to_string(),
        Some(rest) if rest.starts_with('/') => format!("{}{rest}", home.trim_end_matches('/')),
        _ => t.to_string(),
    };
    if !expanded.starts_with('/') {
        return Err(missing());
    }
    let path = util::normalize(&expanded);
    match std::fs::metadata(&path) {
        Ok(m) if m.is_dir() => Ok((path, None)),
        Ok(_) => util::parent(&path).map(|p| (p, Some(util::file_name(&path).to_string()))).ok_or_else(missing),
        Err(_) => Err(missing()),
    }
}

/// 미리 보기 카드에 띄우는 파일(과 썸네일의 파일)을 가리는 값: 경로와 수정한 때·크기 (같은 경로라도 파일이 바뀌면 다시 읽는다)
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct CardKey {
    path: String,
    modified: u64,
    size: u64,
}

/// 카드와 오른쪽 미리 보기 창의 내용 읽기 요청: 창 쪽이 뒷줄에서 load_preview를 돌려 on_previewed로 돌려준다
pub struct PreviewRequest {
    /// 요청 번호 (나중에 낸 요청일수록 크다). 창 쪽이 밀린 그림 읽기를 건너뛸 때 쓴다
    pub seq: u64,
    key: CardKey,
    /// 그림·썸네일이면 긴 쪽을 이 픽셀 수까지 줄여 읽는다. 글 파일이면 0
    pub px: u32,
    /// 무엇으로 읽나: 그림(ImageIO), 글, 문서(Quick Look 썸네일, 한글 문서는 안에 든 미리 보기)
    kind: CardKind,
    /// 오른쪽 미리 보기 창의 것 (아니면 카드의 것). 창은 글을 더 많이 보인다
    pane: bool,
}

/// 뒷줄에서 읽은 카드·창의 내용
pub enum Preview {
    Image(Picture),
    /// 글 파일의 앞부분 (decode_text가 다듬은 것)
    Text(String),
}

/// 요청한 내용을 읽는다 (창에서는 뒷줄에서 부른다). 읽지 못하면 None
pub fn load_preview(req: &PreviewRequest) -> Option<Preview> {
    let path = &req.key.path;
    let lines = if req.pane { preview_pane::PANE_LINES } else { TEXT_LINES };
    match req.kind {
        CardKind::Image => Gfx::load_picture(path, req.px).map(Preview::Image),
        CardKind::Doc if is_hwp(path) => load_hwp(path, req.px, lines),
        // 꾸미지 않은 썸네일: PDF의 첫 쪽, 동영상의 한 장면을 그대로 크게 보인다
        CardKind::Doc => Gfx::load_quick_look(path, req.px, false).map(Preview::Image),
        CardKind::Text => {
            use std::io::Read;
            let mut buf = Vec::new();
            std::fs::File::open(path).ok()?.take(TEXT_READ).read_to_end(&mut buf).ok()?;
            if req.pane { decode_lines(&buf, lines) } else { decode_text(&buf) }.map(Preview::Text)
        }
    }
}

/// 한글 문서(hwp)의 미리 보기: 안에 든 그림(PrvImage: PNG·GIF·BMP)이 있으면 그 그림, 없으면 미리 보기 글(PrvText, UTF-16).
/// 복합 파일은 공통 코드(sakadu::cfb)로 읽는다 (Windows는 COM으로 읽는다)
fn load_hwp(path: &str, px: u32, lines: usize) -> Option<Preview> {
    let mut f = sakadu::cfb::Cfb::open(std::fs::File::open(path).ok()?)?;
    if let Some(pic) = f.stream("PrvImage").and_then(|b| Gfx::load_picture_data(&b, px)) {
        return Some(Preview::Image(pic));
    }
    let b = f.stream("PrvText")?;
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let s = String::from_utf16_lossy(&units);
    let s = s.trim_end_matches('\0').trim();
    (!s.is_empty()).then(|| Preview::Text(tidy(s, lines)))
}

/// 한글 문서인가 (확장자 hwp)
fn is_hwp(path: &str) -> bool {
    ext_of(path.rsplit('/').next().unwrap_or(path)).as_deref() == Some("hwp")
}

/// 글 파일의 앞부분(바이트)을 카드에 보일 글로 바꾼다 (앞 80줄)
fn decode_text(b: &[u8]) -> Option<String> {
    decode_lines(b, TEXT_LINES)
}

/// 글 파일의 앞부분(바이트)을 보일 글로 바꾼다. UTF-8(BOM이 있어도 된다)과 BOM이 있는 UTF-16만 읽는다.
/// 글이 아니면(NUL이 있거나 그 형식의 글자로 읽히지 않으면) None. 앞 lines줄만, 긴 줄은 잘라서, 줄마다 '\n'으로 끝낸다
fn decode_lines(b: &[u8], lines: usize) -> Option<String> {
    let utf8 = |b: &[u8]| match std::str::from_utf8(b) {
        Ok(s) => Some(s.to_string()),
        // 읽기를 자른 자리가 글자 중간이면 그 앞까지
        Err(e) if e.error_len().is_none() => std::str::from_utf8(&b[..e.valid_up_to()]).ok().map(str::to_string),
        Err(_) => None,
    };
    let utf16 = |b: &[u8], big_endian: bool| {
        let mut units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| if big_endian { u16::from_be_bytes(*c) } else { u16::from_le_bytes(*c) }).collect();
        // 읽기를 자른 자리가 두 칸짜리 글자의 앞 칸이면 그 앞까지
        if units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
            units.pop();
        }
        String::from_utf16(&units).ok()
    };
    let s = if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        utf8(rest)?
    } else if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, false)?
    } else if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, true)?
    } else {
        utf8(b)?
    };
    if s.contains('\0') {
        return None;
    }
    Some(tidy(&s, lines))
}

/// 보일 글로 다듬는다: 앞 lines줄만, 탭은 칸 넷, 긴 줄은 TEXT_COLS자까지. 줄마다 '\n'으로 끝낸다
fn tidy(s: &str, lines: usize) -> String {
    let mut out = String::new();
    for line in s.lines().take(lines) {
        out.extend(line.replace('\t', "    ").chars().take(TEXT_COLS));
        out.push('\n');
    }
    out
}

/// 글의 앞 n줄 (줄 끝 '\n'까지)
fn first_lines(s: &str, n: usize) -> &str {
    match s.match_indices('\n').nth(n.saturating_sub(1)) {
        Some((i, _)) => &s[..=i],
        None => s,
    }
}

/// 카드에 무엇을 보이는지 (Windows preview_card.rs와 같은 셋)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CardKind {
    /// 그림 (ImageIO로 읽는다)
    Image,
    /// 글 파일의 앞부분
    Text,
    /// 문서·동영상·음악 따위의 Quick Look 썸네일, 한글 문서의 미리 보기 그림(없으면 미리 보기 글). 없으면 카드를 띄우지 않는다
    Doc,
}

/// Quick Look이 썸네일을 만들지 않는 형식: 압축 파일, 디스크 이미지, 설치 꾸러미·실행 파일·라이브러리, 바로가기, 받는 중인 파일,
/// 데이터베이스, Finder가 쓰는 파일. 이런 파일은 카드를 띄우지 않고 Quick Look에 묻지도 않는다
/// (Windows thumb_provider가 바로 가기·실행 파일을 빼는 것과 같다. 폴더와 앱 꾸러미는 파일이 아니라 처음부터 빠진다)
const NO_THUMBNAIL_EXTS: &[&str] = &[
    "zip", "gz", "tgz", "bz2", "tbz", "tbz2", "xz", "txz", "zst", "lz", "lz4", "lzma", "7z", "rar", "tar", "cab", "arj", "z", "sit", "sitx", "cpio", "xar",
    "dmg", "iso", "img", "sparseimage", "toast", "vmdk", "vdi", "qcow2", "pkg", "mpkg", "exe", "msi", "dll", "so", "dylib", "a", "o", "bin", "dat",
    "class", "jar", "war", "apk", "ipa", "deb", "rpm", "appx", "msix", "lnk", "url", "webloc", "inetloc", "fileloc", "alias", "part", "crdownload",
    "download", "tmp", "temp", "swp", "db", "sqlite", "sqlite3", "ds_store", "localized", "icloud",
];

/// 이름의 확장자 (소문자). Windows와 같게 가린다: 점으로 시작하는 이름(.gitignore)은 점 뒤 전체가 확장자다. 없으면 None
fn ext_of(name: &str) -> Option<String> {
    match name.rsplit_once('.') {
        Some((stem, ext)) if (!stem.is_empty() || name.starts_with('.')) && !ext.is_empty() => Some(ext.to_ascii_lowercase()),
        _ => None,
    }
}

/// 이름으로 본 카드 종류: 글 파일, 그림, 그 밖에 Quick Look이 썸네일을 만들 수 있는 문서. 카드에 띄우지 않는 파일이면 None
fn card_kind(name: &str) -> Option<CardKind> {
    let ext = ext_of(name)?;
    let e = ext.as_str();
    if TEXT_EXTS.contains(&e) {
        Some(CardKind::Text)
    } else if IMAGE_EXTS.contains(&e) {
        Some(CardKind::Image)
    } else if NO_THUMBNAIL_EXTS.contains(&e) {
        None
    } else {
        Some(CardKind::Doc)
    }
}

/// 카드의 이름 아래 글: 종류와 크기 ("PNG 이미지 · 248 KB"). 크기는 목록과 같은 모양으로 적는다
fn card_detail(kind: &str, size: u64) -> String {
    let size = listing::format_size(size);
    if kind.is_empty() { size } else { format!("{kind} · {size}") }
}

/// 지금 고른 파일 가운데 카드에 띄울 것 (App::card_target이 지금 칸에서 찾는다)
struct CardTarget {
    key: CardKey,
    kind: CardKind,
}

/// 미리 보기 카드의 상태. 어느 파일의 카드인지는 그때그때 지금 칸에서 고른 파일로 정하고(card_target),
/// 여기에는 마지막으로 맞춘 파일(key)에 대해 읽은 것과 사용자가 정한 것(닫기, 크게 보기)을 둔다
struct Card {
    /// 마지막으로 맞춘 파일 (sync_card). 고른 파일이 이것과 달라지면 그 파일에 맞춰 새로 읽는다
    key: Option<CardKey>,
    /// 글 파일의 앞부분, 그림이 없는 한글 문서의 미리 보기 글 (읽는 중이면 None)
    text: Option<String>,
    /// 읽지 못했다: 글 파일과 문서면 카드를 띄우지 않고, 그림이면 카드에 '미리 볼 수 없어요'를 보인다
    failed: bool,
    /// 이 파일에 대해 이미 요청한 것: 그림은 그 픽셀 수, 글은 1, 아직이면 0
    asked: u32,
    /// 이 파일에 맞춘 때 (그림이 늦으면 '불러오는 중…'을 보일 때를 잰다)
    since: Instant,
    /// 닫기로 닫은 파일. 다른 파일을 고를 때까지 다시 띄우지 않는다
    closed: Option<String>,
    /// 크게 보기. 카드가 사라질 때까지 이어진다 (파일을 옮겨 다니며 크게 볼 수 있게)
    large: bool,
}

impl Card {
    fn new() -> Card {
        Card { key: None, text: None, failed: false, asked: 0, since: Instant::now(), closed: None, large: false }
    }
}

/// 읽어 둔 그림과 그 그림을 읽을 때 요청한 픽셀 수
struct CachedPic {
    key: CardKey,
    px: u32,
    pic: Picture,
}

impl CachedPic {
    /// 원본을 줄이지 않고 읽었는지: 긴 쪽이 요청한 크기에 못 미치면 원본 그대로라 더 크게 읽을 것이 없다
    /// (요청한 크기에 거의 닿았으면 줄여 읽은 것으로 본다)
    fn full(&self) -> bool {
        self.pic.longest() + 8 < self.px
    }
}

/// 썸네일을 가리는 값: 파일과 읽은 픽셀 수의 단계 (THUMB_PX 가운데 하나)
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct ThumbKey {
    file: CardKey,
    px: u32,
}

/// 썸네일 읽기 요청: 창 쪽이 뒷줄에서 load_thumb를 돌려 on_thumb로 돌려준다
pub struct ThumbRequest {
    key: ThumbKey,
}

/// 요청한 썸네일을 읽는다 (창에서는 뒷줄에서 부른다). 그림 파일은 ImageIO로 줄여 읽고,
/// 그 밖의 파일은 Quick Look에 만들어 달라고 한다 (Windows의 셸 썸네일처럼 PDF·동영상·문서도 보인다). 읽지 못하면 None
pub fn load_thumb(req: &ThumbRequest) -> Option<Picture> {
    let path = &req.key.file.path;
    if is_picture(path) { Gfx::load_picture(path, req.key.px) } else { Gfx::load_quick_look(path, req.key.px, true) }
}

/// 그림 파일인가 (미리 보기 카드와 같은 확장자). 썸네일을 ImageIO로 직접 읽는 파일이다
fn is_picture(path: &str) -> bool {
    card_kind(path.rsplit('/').next().unwrap_or(path)) == Some(CardKind::Image)
}

/// 그림 자리(한 변 size pt)를 scale배 화면에 채울 픽셀 수를 단계로 올린 값. 가장 큰 단계보다 크면 가장 큰 단계
fn thumb_px(size: f32, scale: f32) -> u32 {
    let need = (size * scale).ceil() as u32;
    THUMB_PX.into_iter().find(|px| *px >= need).unwrap_or(THUMB_PX[THUMB_PX.len() - 1])
}

/// 그 항목의 썸네일을 가리는 값. 썸네일은 파일에만 있다 (폴더는 None): 그림 파일은 직접 읽고, 그 밖의 파일은 Quick Look이 만든다.
/// 썸네일이 없는 파일(Quick Look이 만들지 못하는 형식)은 읽지 못한 것으로 적어 두고 아이콘을 그린다
fn thumb_key(tab: &Tab, e: &Entry, size: f32, scale: f32) -> Option<ThumbKey> {
    if e.kind != Kind::File {
        return None;
    }
    Some(ThumbKey { file: CardKey { path: tab.path_of(e)?, modified: e.modified, size: e.size }, px: thumb_px(size, scale) })
}

/// 그림이 차지하는 메모리 (픽셀마다 4바이트)
fn pic_bytes(pic: &Picture) -> usize {
    let (w, h) = pic.size();
    w * h * 4
}

/// 읽어 둔 썸네일 하나
struct Thumb {
    /// 읽지 못한 그림은 None (그릴 때마다 다시 읽지 않게 그것도 적어 둔다)
    pic: Option<Picture>,
    /// 마지막으로 화면에 보인 때 (Thumbs::clock의 값)
    used: u64,
}

/// 아이콘 보기의 썸네일: 읽어 둔 것, 읽으려고 기다리는 것, 뒷줄에서 읽는 중인 것.
/// 기다리는 것은 화면에 맞출 때마다(sync) 지금 보이는 것으로 새로 채운다: 스크롤로 지나갔거나 떠난 폴더·탭의 것은 읽기 전에 빠진다
#[derive(Default)]
struct Thumbs {
    cache: HashMap<ThumbKey, Thumb>,
    /// 읽어 둔 썸네일이 차지하는 메모리 (바이트)
    bytes: usize,
    /// 기다리는 것 (보이는 순서)
    queue: VecDeque<ThumbKey>,
    /// 읽는 중인 것 (THUMB_JOBS개까지)
    loading: Vec<ThumbKey>,
    /// 화면에 맞춘 횟수. used가 이 값인 썸네일은 지금 보이는 것이다
    clock: u64,
}

impl Thumbs {
    /// 읽어 둔 썸네일 (아직 없거나 읽지 못한 그림이면 None)
    fn get(&self, key: &ThumbKey) -> Option<&Picture> {
        self.cache.get(key)?.pic.as_ref()
    }

    /// 지금 보이는 썸네일(보이는 순서)에 맞춘다: 읽어 둔 것은 방금 쓴 것으로 적고,
    /// 기다리는 것은 지금 보이는데 아직 없는 것으로 새로 채운다
    fn sync(&mut self, visible: Vec<ThumbKey>) {
        self.clock += 1;
        self.queue.clear();
        for key in visible {
            if let Some(t) = self.cache.get_mut(&key) {
                t.used = self.clock;
            } else if !self.loading.contains(&key) {
                self.queue.push_back(key);
            }
        }
    }

    /// 기다리는 것 가운데 지금 읽기 시작할 것을 꺼낸다. 읽는 중인 것은 THUMB_JOBS개를 넘지 않는다
    fn start(&mut self) -> Vec<ThumbRequest> {
        let mut out = Vec::new();
        while self.loading.len() < THUMB_JOBS {
            let Some(key) = self.queue.pop_front() else { break };
            // 같은 폴더를 두 칸에서 보고 있으면 같은 그림이 두 번 기다린다
            if !self.loading.contains(&key) {
                self.loading.push(key.clone());
                out.push(ThumbRequest { key });
            }
        }
        out
    }

    /// 다 읽은 썸네일을 넣는다 (읽지 못했으면 None). 남겨 둘 한도를 넘으면 가장 오래 안 쓴 것부터 버린다
    fn finish(&mut self, key: &ThumbKey, pic: Option<Picture>) {
        self.loading.retain(|k| k != key);
        self.bytes += pic.as_ref().map_or(0, pic_bytes);
        if let Some(old) = self.cache.insert(key.clone(), Thumb { pic, used: self.clock }) {
            self.bytes -= old.pic.as_ref().map_or(0, pic_bytes);
        }
        while self.cache.len() > THUMBS_MAX || self.bytes > THUMB_BYTES {
            // 지금 보이는 것은 버리지 않는다 (버리면 곧바로 다시 읽게 된다). 보이는 것만으로 한도를 넘으면 넘긴 채로 둔다
            let Some(oldest) = self.cache.iter().filter(|(_, t)| t.used < self.clock).min_by_key(|(_, t)| t.used).map(|(k, _)| k.clone()) else { break };
            if let Some(t) = self.cache.remove(&oldest) {
                self.bytes -= t.pic.as_ref().map_or(0, pic_bytes);
            }
        }
    }
}

/// 사이드바 줄의 종류
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SideKind {
    /// 이 공간에 고정한 것
    Pin,
    /// 고정한 것이 없을 때의 안내 (누를 수 없다)
    Hint,
    /// '모든 공간' 구역 이름 (누를 수 없다)
    Label,
    /// 어느 공간에나 있는 바로가기 (홈·문서·다운로드·디스크)
    Place,
    /// 즐겨찾기 그룹 머리 (그룹 번호, 접힘)
    Group(usize, bool),
    /// 즐겨찾기 항목
    Fav,
}

/// 사이드바 한 줄. y는 목록 맨 위를 0으로 본 위치 (스크롤 전)
struct SideRow {
    kind: SideKind,
    label: String,
    path: Option<String>,
    is_dir: bool,
    indent: bool,
    y: f32,
    h: f32,
    /// 끌어 옮기기에서 맡는 몫 (공통 ui::side_flow): 고정한 것의 번호, 즐겨찾기 줄, 즐겨찾기 바로 위 줄
    flow: FlowKind,
}

impl SideRow {
    /// 공통 끌기 계산에 넘길 자리
    fn slot(&self) -> FlowSlot {
        FlowSlot { t: self.y, b: self.y + self.h, kind: self.flow }
    }

    /// 움직임을 이어 가는 열쇠: 같은 줄이면 자리가 바뀌어도 같다 (고정한 것·즐겨찾기 항목·바로가기는 경로, 그룹은 이름)
    fn key(&self) -> String {
        let path = self.path.as_deref().unwrap_or_default();
        match self.kind {
            SideKind::Pin => pin_key(path),
            SideKind::Fav => fav_key(path),
            SideKind::Group(..) => format!("g|{}", self.label),
            SideKind::Place => format!("s|{path}"),
            SideKind::Hint => "hint".to_string(),
            SideKind::Label => "label".to_string(),
        }
    }
}

/// 고정한 줄의 움직임 열쇠
fn pin_key(path: &str) -> String {
    format!("p|{path}")
}

/// 즐겨찾기 항목 줄의 움직임 열쇠
fn fav_key(path: &str) -> String {
    format!("f|{path}")
}

/// 사이드바 줄 i를 오른쪽 단추로 눌렀을 때 여는 메뉴: 고정한 줄이면 고정한 것 가운데 번호로, 즐겨찾기 항목이면 즐겨찾기 줄
/// (Favorites::rows) 번호로, 그룹 머리면 그룹 번호로 가리킨다. 바로가기, 안내, 구역 이름에는 메뉴가 없다
fn side_menu(rows: &[SideRow], i: usize) -> Option<MenuKind> {
    let before = |same: fn(SideKind) -> bool| rows[..i].iter().filter(|r| same(r.kind)).count();
    match rows.get(i)?.kind {
        SideKind::Pin => Some(MenuKind::Pin(before(|k| k == SideKind::Pin))),
        SideKind::Fav => Some(MenuKind::Fav(before(|k| matches!(k, SideKind::Group(..) | SideKind::Fav)))),
        SideKind::Group(g, _) => Some(MenuKind::Group(g)),
        _ => None,
    }
}

/// 테마를 정할 때 쓰는 맥 설정 (창 쪽이 읽어 set_system으로 넣어 준 값)
struct Sys {
    dark: bool,
    accent: Option<Color>,
}

impl System for Sys {
    fn prefers_dark(&self) -> bool {
        self.dark
    }
    fn high_contrast(&self) -> Option<[Color; 7]> {
        None
    }
    fn accent(&self, _dark: bool) -> Option<Color> {
        self.accent
    }
}

pub struct App {
    pub w: f32,
    pub h: f32,
    pub theme: Theme,
    panes: Vec<Pane>,
    layout: Layout,
    active: usize,
    split: (f32, f32),
    /// 즐겨찾기: 사이드바 아래 '모든 공간'에 보인다 (어느 공간에서나 같다)
    favs: Favorites,
    show_hidden: bool,
    pub hover: Hit,
    next_id: u64,
    /// 끌고 있는 칸 경계
    drag: Option<Hit>,
    /// 끌고 있는 열 경계 (자세히 보기 머리글)
    col_drag: Option<ColDrag>,
    /// 날짜 열의 내용 폭 (글꼴로 한 번 재어 둔다: ui::date_col_width)
    date_w: std::cell::Cell<Option<f32>>,
    /// '모든 공간'에 늘 있는 줄 (이름, 경로): 홈·문서·다운로드·디스크
    places: Vec<(String, String)>,
    requests: Vec<Request>,
    /// 신호등 아래 끝 (창에서 읽는다). 사이드바(레일과 공간 이름)는 그 아래에서 시작한다
    pub lights_bottom: f32,
    /// 창 없이 그림만 그릴 때 (그림 시험): 신호등과 경로 입력 글자도 직접 그린다 (창에서는 시스템·입력 상자가 그린다)
    pub standalone: bool,
    /// 입력 상자가 지금 고치는 것: 경로, 공간 이름, 항목 이름 (창 쪽 입력 상자가 그 자리에 뜬다)
    pub edit: Editing,
    /// 항목 이름 고치기를 시작한 횟수 (ItemEdit::serial)
    edit_serial: u64,
    /// 맡길 파일 작업 (창 쪽이 뒷줄에서 하나씩 실행하고 on_op_done으로 돌려준다)
    ops: Vec<Op>,
    op_seq: u64,
    /// 방금 만든 새 폴더: (탭, 폴더, 새 폴더의 파일 번호). 그 탭을 다 읽으면 그 줄에서 이름을 고친다
    name_next: Option<(u64, String, u64)>,
    /// 다시 읽어야 하는 탭: 파일 작업이나 폴더 감시로 바뀐 것을 알았는데 지금 보이지 않거나(보이게 될 때 읽는다),
    /// 읽는 중이거나 마우스로 누르고 있어 미뤄 둔 것 (reload_stale)
    stale: HashSet<u64>,
    /// 읽기를 맡기고 아직 결과를 받지 못한 탭
    reading: HashSet<u64>,
    /// 같은 폴더를 다시 읽는 중인 탭 (refresh): 다 읽으면 그때의 고른 것·커서·스크롤을 되살린다
    rereading: HashSet<u64>,
    /// 다 읽었지만 아직 적용하지 않은 다시 읽기 결과 (탭, 세대 번호, 결과): 그 탭의 줄을 마우스로 다루는 동안에는
    /// 목록을 바꾸지 않고, 손을 뗄 때 적용한다 (after_pointer)
    held: Vec<(u64, u64, Result<Vec<Entry>, String>)>,
    /// 창 쪽에 감시해 달라고 내준 폴더: 보이는 칸의 폴더 (take_watch_dirs)
    watched: Vec<String>,
    /// 맡기고 아직 결과를 받지 못한 파일 작업의 수
    ops_running: usize,
    /// 되돌리기 기록: 이름 바꾸기, 새 폴더, 휴지통 (⌘Z)
    undos: UndoStack,
    /// 목록 메뉴를 열 때 적어 둔 대상
    list_target: Option<ListTarget>,
    /// Finder에서 보여 달라는 항목 (창 쪽이 Finder를 앞으로 불러 그 항목을 고른다)
    pub reveal_files: Vec<String>,
    /// 진행 중인 복사 묶음
    batch: Option<Batch>,
    /// 고른 줄을 누르고 있다 (끌기를 시작할지 본다)
    press: Option<Press>,
    /// 끌어 내 달라는 항목들의 경로 (창 쪽이 take_drag로 가져가 맥의 끌기를 시작한다)
    drag_out: Option<Vec<String>>,
    /// 끌려 온 것이 지금 놓일 곳 (그 칸이나 줄을 강조한다)
    drop: Option<DropAt>,
    /// 클립보드에 담아 달라는 항목들의 경로 (창 쪽이 파일 주소로 담는다: Finder에도 붙여넣을 수 있다).
    /// clip_cut이면 잘라낸 것이다: 다음에 붙여넣을 때 복사하지 않고 옮긴다 (그 사이 클립보드가 바뀌지 않았으면. 창 쪽이 기억한다)
    pub clip_out: Option<Vec<String>>,
    pub clip_cut: bool,
    /// 클립보드의 파일들을 붙여넣어 달라는 표시 (창 쪽이 클립보드를 읽어 paste_paths로 넘긴다).
    /// Some(true)면 잘라낸 것이 아니어도 옮긴다 (⌥⌘V)
    pub clip_in: Option<bool>,
    /// 잘라 둔 것을 옮기는 묶음이 끝난 뒤: 그 묶음의 표식(Paste::Cut)과 옮기지 못하고 남은 것들의 경로.
    /// 창 쪽이 클립보드를 이것만 잘라 둔 것으로 고친다 (비어 있으면 모두 옮겨 간 것이라 클립보드를 비운다).
    /// 옮겨 가지 않은 것은 다른 곳에 다시 붙여넣을 수 있다
    pub clip_left: Option<(i64, Vec<String>)>,
    /// 훑어보기(Quick Look) 창을 열어(true) 또는 닫아(false) 달라는 요청 (창 쪽이 가져가 맥의 훑어보기 창을 다룬다)
    pub look_request: Option<bool>,
    /// 훑어보기 창이 화면에 떠 있다 (창 쪽이 키를 넘기기 직전에 그 창에 물어 맞춘다)
    pub look_open: bool,
    /// 칸 p의 칩 줄 오른쪽에 잠깐 보일 알림 (다음 입력 때 지운다)
    flash: Option<(usize, String)>,
    probes: Vec<Probe>,
    /// 칸마다 마지막으로 낸 경로 확인 요청 번호
    probe_seq: [u64; 4],
    /// 칸마다 가장 나중에 낸 경로 확인이 기다리는 자리: (탭, 그 탭의 세대 번호). 그 탭이 다른 곳으로 옮기면 세대 번호가
    /// 달라져 그 확인은 버려진다. 같은 폴더를 다시 읽는 것은 옮긴 것이 아니므로 refresh가 번호를 따라 올린다
    probe_at: [(u64, u64); 4],
    /// 저장한 세션에서 연 탭과 그 폴더. 그 폴더의 첫 읽기가 실패하면 home으로 바꾼다
    /// (그 사이 다른 곳으로 옮겼으면 그곳의 오류는 그대로 보인다)
    restored: std::collections::HashMap<u64, String>,
    /// 홈 폴더 (세션에서 연 폴더가 없을 때)
    home: String,
    /// 사이드바 목록 스크롤 (줄이 창 아래로 넘칠 때)
    side_scroll: f32,
    /// 열어 달라는 파일 (창 쪽이 macOS '열기'로 연다)
    pub open_files: Vec<String>,
    /// 즐겨찾기를 파일에 저장할지 (시험·그림에서는 저장하지 않는다)
    pub persist: bool,
    /// 열린 메뉴 (앱이 맨 위에 그린다)
    menu: Option<OpenMenu>,
    /// Finder '종류' 이름: 확장자별로 한 번만 묻는다 (폴더는 앞에 '/')
    kinds: RefCell<HashMap<String, String>>,
    /// 작업 공간. 늘 하나 이상이고 그중 하나가 지금 것이다.
    /// 지금 공간에 저장된 화면은 다른 공간으로 바꾸거나 파일에 저장할 때 지금 화면으로 맞춘다
    workspaces: Workspaces,
    /// 맥 설정에서 읽은 모양과 강조 색 (창 쪽이 넣어 준다). 색을 정하지 않은 공간은 이 강조 색을 따른다
    dark: bool,
    system_accent: Option<Color>,
    /// 자동 미리 보기: 그림·글 파일을 고르면 카드가 저절로 뜬다 (도구 막대의 눈으로 끄고 켠다. 숨김 항목 보기처럼 작업 공간과 상관없다)
    auto_preview: bool,
    card: Card,
    /// 마지막 누름(두 번 누르기의 첫 누름)이 미리 보기 카드 위였는가. 그랬으면 이어지는 둘째 누름은 버리고, 아니었으면 카드 아래 것이 받는다.
    /// 시작 화면의 칸이나 명령 메뉴를 눌러 다른 곳으로 옮겨 간 누름도 그렇다 (둘째 누름이 새 목록의 항목을 열지 않게)
    press_on_card: bool,
    /// 방금 한 번 누른 줄의 항목 (두 번 누르기의 첫 번째)
    clicked: Option<Clicked>,
    /// 읽어 둔 그림: 최근에 본 몇 개만 남긴다 (맨 뒤가 가장 최근)
    pics: Vec<CachedPic>,
    previews: Vec<PreviewRequest>,
    /// 마지막으로 낸 카드·창 내용 요청의 번호 (고정 확인의 순번도 여기서 받는다)
    preview_seq: u64,
    /// 오른쪽 미리 보기 창 (⌥⌘P). 닫혀 있으면 None
    preview: Option<preview_pane::PreviewPane>,
    /// 고정한 파일을 확인해 달라는 요청 (창 쪽이 뒷줄에서 check_pin을 돌려 on_pin_checked로 돌려준다)
    pin_checks: Vec<PinCheck>,
    /// 창 쪽이 미리 보기 창의 본문에 Quick Look 보기(QLPreviewView)를 띄울 수 있다. 만들지 못했다고 알려 오면 false:
    /// 그 뒤로 문서는 큰 Quick Look 썸네일로 보인다
    look_view_ok: bool,
    /// 아이콘 보기의 썸네일 (그림 파일만)
    thumbs: Thumbs,
    /// 화면 배율 (레티나는 2. 창 쪽이 창에서 읽어 넣는다). 썸네일을 몇 픽셀로 읽을지 정한다
    pub scale: f32,
    /// 검색: 입력, 결과, 자체 색인 (find.rs)
    find: find::Find,
    /// 열어 달라는 주소 (창 쪽이 macOS '열기'로 연다. 시스템 설정의 개인정보 보호 쪽 등)
    pub open_url: Option<String>,
    /// 단축키 (keys.txt. 창 쪽이 처음에 읽어 넣는다. 시험과 그림은 기본값)
    keys: Keymap,
    /// 닫은 탭 (칸, 위치). '닫은 탭 다시 열기'가 가장 나중 것부터 연다
    closed: Vec<(usize, Location)>,
    /// 설정 창을 열어 달라는 표시 (설정·도움말 단축키. 창 쪽이 연다)
    pub open_settings: bool,
    /// 모양: "system"(맥 설정을 따른다), "light", "dark". 창 쪽이 앱의 모양으로 정하고, 세션에 남긴다
    theme_pref: String,
    /// 새 탭(⌘T, +)을 열 곳: session::NEW_TAB_START(시작 화면, 기본) 또는 NEW_TAB_SAME(지금 폴더). 세션에 남긴다 (작업 공간과 상관없다)
    new_tab_pref: String,
    /// 다녀간 폴더 (최근 것이 앞): 시작 화면과 명령 메뉴에 보인다. 세션에 남긴다 (작업 공간과 상관없다)
    recent: Vec<String>,
    /// 시작 화면에서 키보드(화살표·Return)를 썼다: 초점 테두리가 보이고 Return이 연다
    start_keys: bool,
    /// 시작 화면 위치 칸의 디스크 용량 (경로 → 남은·전체 바이트)과, 다시 읽어 달라는 표시 (시작 화면을 열 때)
    volumes: HashMap<String, (u64, u64)>,
    volumes_wanted: bool,
    /// 명령 메뉴(⌘K)와 그 입력 칸의 글
    cmd: Option<sakadu::ui::commands::CmdMenu>,
    cmd_text: String,
    /// 누르고 있는 사이드바 줄 (떼면 열고, 끌면 옮긴다)
    side_press: Option<SidePress>,
    /// 사이드바 줄이 미끄러지는 움직임 (공통 ui::side_flow). 그릴 때 한 걸음씩 나아가므로 그리기(&self) 안에서 고친다
    side_flow: RefCell<SideFlow>,
    /// 움직임을 보일지. 맥 설정의 '동작 줄이기'를 켜면 false: 사이드바 줄이 미끄러지지 않고 바로 옮겨진다 (창 쪽이 그리기 전에 넣는다)
    pub motion: bool,
    /// 마우스가 사이드바 목록 위에 있다: '모든 공간' 줄의 새 그룹 단추가 보인다 (Windows처럼 마우스를 올릴 때만)
    side_hot: bool,
}

impl App {
    /// home: 처음 여는 폴더, places: '모든 공간'에 늘 있는 줄
    pub fn new(w: f32, h: f32, theme: Theme, home: &str, favs: Favorites, places: Vec<(String, String)>) -> App {
        let mut app = App {
            w,
            h,
            theme,
            panes: Vec::new(),
            layout: Layout::TwoV,
            active: 0,
            split: (0.5, 0.5),
            favs,
            show_hidden: false,
            hover: Hit::None,
            next_id: 1,
            drag: None,
            col_drag: None,
            date_w: std::cell::Cell::new(None),
            places,
            requests: Vec::new(),
            lights_bottom: LIGHTS_BOTTOM,
            standalone: false,
            edit: Editing::None,
            edit_serial: 0,
            ops: Vec::new(),
            op_seq: 0,
            name_next: None,
            stale: HashSet::new(),
            reading: HashSet::new(),
            rereading: HashSet::new(),
            held: Vec::new(),
            watched: Vec::new(),
            ops_running: 0,
            undos: UndoStack::default(),
            list_target: None,
            reveal_files: Vec::new(),
            batch: None,
            press: None,
            drag_out: None,
            drop: None,
            clip_out: None,
            clip_cut: false,
            clip_in: None,
            clip_left: None,
            look_request: None,
            look_open: false,
            flash: None,
            probes: Vec::new(),
            probe_seq: [0; 4],
            probe_at: [(0, 0); 4],
            restored: std::collections::HashMap::new(),
            home: home.to_string(),
            side_scroll: 0.0,
            open_files: Vec::new(),
            persist: false,
            menu: None,
            kinds: RefCell::new(HashMap::new()),
            workspaces: Workspaces::default(),
            dark: theme.dark,
            system_accent: None,
            auto_preview: true,
            card: Card::new(),
            press_on_card: false,
            clicked: None,
            pics: Vec::new(),
            previews: Vec::new(),
            preview_seq: 0,
            preview: None,
            pin_checks: Vec::new(),
            look_view_ok: true,
            thumbs: Thumbs::default(),
            scale: 2.0,
            find: find::Find::default(),
            open_url: None,
            keys: Keymap::default(),
            closed: Vec::new(),
            open_settings: false,
            theme_pref: "system".to_string(),
            new_tab_pref: session::NEW_TAB_START.to_string(),
            recent: Vec::new(),
            start_keys: false,
            volumes: HashMap::new(),
            volumes_wanted: false,
            cmd: None,
            cmd_text: String::new(),
            side_press: None,
            side_flow: RefCell::new(SideFlow::default()),
            motion: true,
            side_hot: false,
        };
        for _ in 0..4 {
            let tab = app.new_tab_at(Location::Dir(home.to_string()));
            app.panes.push(Pane { tabs: vec![tab], active: 0, view: ViewMode::Details, sort: Sort::default(), cols: DEFAULT_COLS });
        }
        for p in 0..app.layout.count() {
            app.load(p);
        }
        // 레일에는 늘 지금 공간이 있다: 저장해 둔 공간을 넣기 전에는 지금 화면이 '기본' 공간이다
        app.set_workspaces(Workspaces::default());
        app
    }

    fn new_tab_at(&mut self, loc: Location) -> Tab {
        self.next_id += 1;
        Tab::new(self.next_id, loc)
    }

    /// 창 시험이 보는 것 (Seen)
    pub fn seen(&self) -> Seen {
        let tab = self.panes[self.active].tab();
        let rows = || (0..tab.view.len()).filter_map(|vi| tab.entry_at(vi));
        Seen {
            dir: tab.loc.dir().unwrap_or_default().to_string(),
            names: rows().map(Entry::name_string).collect(),
            cursor: tab.cursor_entry().map(Entry::name_string).unwrap_or_default(),
            selected: rows().filter(|e| e.selected).map(Entry::name_string).collect(),
            ready: tab.state == LoadState::Ready,
            editing: match &self.edit {
                Editing::Item(e) => Some(e.name.clone()),
                _ => None,
            },
            flash: self.flash.as_ref().map(|(_, note)| note.clone()),
            busy: self.ops_running > 0 || self.batch.is_some(),
            tabs: self.panes[self.active].tabs.len(),
            start: tab.loc == Location::Start,
        }
    }

    /// 창 시험: 지금 칸에서 그 이름의 줄 가운데 (뷰 좌표, 위가 0). 그 줄이 목록에 없으면 None
    pub fn row_center(&self, name: &str) -> Option<(f32, f32)> {
        let p = self.active;
        let tab = self.panes[p].tab();
        let vi = (0..tab.view.len()).find(|vi| tab.entry_at(*vi).is_some_and(|e| e.name_string() == name))?;
        let r = Grid::of(self.list_rect(p)?, self.panes[p].view).item(vi, tab.scroll);
        Some(((r.l + r.r) / 2.0, (r.t + r.b) / 2.0))
    }

    /// 창 제목: 지금 칸의 지금 탭
    pub fn title(&self) -> String {
        format!("{} - Sakadu", self.panes[self.active].tab().loc.title())
    }

    #[cfg(test)]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    #[cfg(test)]
    pub fn active(&self) -> usize {
        self.active
    }

    #[cfg(test)]
    pub fn pane(&self, p: usize) -> &Pane {
        &self.panes[p]
    }

    /// 지금 칸의 정렬 (smoke가 머리글을 누른 뒤 본다)
    pub fn active_sort(&self) -> Sort {
        self.panes[self.active].sort
    }

    /// 지금 칸 번호 (메뉴 막대의 뒤로·앞으로·위 폴더)
    pub fn active_pane(&self) -> usize {
        self.active
    }

    /// 칸 p의 지금 탭 (그림 시험에서 고른 줄을 정할 때)
    pub fn tab_mut(&mut self, p: usize) -> &mut Tab {
        self.panes[p].tab_mut()
    }

    #[cfg(test)]
    pub fn favorites(&self) -> &Favorites {
        &self.favs
    }

    /// 작업 공간 (시험과 창 시험이 고정한 것을 본다)
    pub fn workspaces(&self) -> &Workspaces {
        &self.workspaces
    }

    // ---------- 폴더 읽기 ----------

    /// 칸 p의 지금 탭을 (다시) 읽는다. 결과는 on_listed로 받는다
    fn load(&mut self, p: usize) {
        let tab = self.panes[p].tab_mut();
        tab.generation += 1;
        // 새로 읽는다: 같은 폴더를 다시 읽는 것이면 refresh가 다시 적어 둔다
        self.rereading.remove(&tab.id);
        if tab.state != LoadState::Ready {
            tab.state = LoadState::Loading;
        }
        if let Some(dir) = tab.loc.dir() {
            self.requests.push(Request { tab: tab.id, generation: tab.generation, dir: dir.to_string() });
            // 이제 읽는 것이 지금까지의 변화를 모두 담는다. 읽는 사이 또 바뀌면 다 읽은 뒤 한 번 더 읽는다 (reload_stale)
            self.stale.remove(&tab.id);
            self.reading.insert(tab.id);
        } else if tab.loc == Location::Start {
            // 시작 화면은 읽을 목록이 없다. 볼 때마다 위치 칸의 디스크 용량을 새로 읽는다
            tab.entries.clear();
            tab.view.clear();
            tab.state = LoadState::Ready;
            self.volumes_wanted = true;
        }
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// 뒷줄에서 읽은 결과. 그 사이 다른 곳으로 옮겼으면(세대 번호가 다르면) 버린다
    pub fn on_listed(&mut self, tab_id: u64, generation: u64, result: Result<Vec<Entry>, String>) {
        let hidden = self.show_hidden;
        let Some(p) = self.panes.iter().position(|pane| pane.tabs.iter().any(|t| t.id == tab_id)) else { return };
        let sort = self.panes[p].sort;
        let (list, view) = (self.list_rect(p), self.panes[p].view);
        let current = self.panes[p].tabs.iter().any(|t| t.id == tab_id && t.generation == generation);
        if !current {
            return;
        }
        // 다시 읽은 결과가 그 탭의 줄을 마우스로 다루는 동안에 왔다: 지금은 목록을 바꾸지 않는다 (after_pointer가 적용한다).
        // 누른 줄과 놓일 줄은 줄 번호로 기억하므로, 손 밑의 줄이 바뀌면 다른 항목이 골라지거나 다른 폴더에 놓인다
        if self.rereading.contains(&tab_id) && self.pointer_on(tab_id) {
            self.held.push((tab_id, generation, result));
            return;
        }
        self.reading.remove(&tab_id);
        let rereading = self.rereading.remove(&tab_id);
        // 이 탭에서 이름을 고치는 중인 항목 (파일 번호, 이름)
        let editing = match &self.edit {
            Editing::Item(e) if e.tab == tab_id => Some((e.file_id, e.name.clone())),
            _ => None,
        };
        // 세션에서 연 탭인지는 그 탭의 첫 읽기 결과에서만, 그리고 아직 저장한 그 폴더에 있을 때만 본다
        let saved = self.restored.remove(&tab_id);
        let home = self.home.clone();
        let Some(tab) = self.panes[p].tabs.iter_mut().find(|t| t.id == tab_id) else { return };
        let restored = saved.is_some_and(|d| tab.loc.dir() == Some(d.as_str()));
        match result {
            Ok(entries) => {
                // 같은 폴더를 다시 읽었다: 읽기를 시작한 때가 아니라 지금의 고른 것·커서·스크롤을 되살린다
                // (읽는 사이에 고르거나 스크롤한 것을 되돌리지 않는다). 읽은 뒤 고를 것을 따로 정해 둔 때는 그것을 따른다
                if rereading && tab.restore.is_none() {
                    tab.restore = Some(tab.snapshot());
                }
                let restore = tab.set_entries(entries, sort, hidden);
                // 스크롤은 다 읽은 그 탭에 맞춘다 (그 사이 다른 탭으로 옮겼어도 보고 있는 탭은 그대로)
                if let Some(list) = list {
                    clamp_tab(tab, list, view);
                    // 이름을 고치는 중인 줄은 목록이 바뀌어 밀려도 계속 보이게 한다: 화면 밖으로 밀리면 이름 상자가 닫혀
                    // 치던 이름을 잃는다. 그 항목이 없어졌으면 스크롤하지 않는다 (그때는 고치기가 끝난다)
                    let editing_row = editing.is_some_and(|(id, name)| tab.cursor_entry().is_some_and(|e| e.file_id == id && e.name_string() == name));
                    if restore.is_some_and(|r| r.reveal) || editing_row {
                        reveal(tab, list, view);
                    }
                }
            }
            // 세션에서 연 폴더가 없어졌거나 닿지 않으면 home을 연다 (기록은 남기지 않는다)
            Err(_) if restored && tab.loc != Location::Dir(home.clone()) => {
                let gone = std::mem::replace(&mut tab.loc, Location::Dir(home.clone()));
                tab.generation += 1;
                tab.state = LoadState::Loading;
                self.requests.push(Request { tab: tab.id, generation: tab.generation, dir: home });
                self.reading.insert(tab_id);
                // 열지 못한 폴더는 최근 위치에서 뺀다 (시작 화면·명령 메뉴에 남지 않게)
                if let Some(d) = gone.dir() {
                    self.forget_recent(d);
                }
            }
            Err(msg) => {
                // 지워졌거나 폴더가 아닌 것만 최근 위치에서 뺀다 (Windows와 같다). 잠깐 닿지 않는 NAS·권한 없음은 그대로 둔다.
                // 읽기 오류는 io::Error의 글이라 끝에 OS 오류 번호가 붙는다 (ENOENT 2, ENOTDIR 20)
                let gone = msg.ends_with("(os error 2)") || msg.ends_with("(os error 20)");
                tab.entries.clear();
                tab.view.clear();
                tab.state = LoadState::Error(msg);
                if let Some(d) = tab.loc.dir().map(str::to_string).filter(|_| gone) {
                    self.forget_recent(&d);
                }
            }
        }
        self.name_listed(tab_id);
        // 읽는 사이 그 폴더가 또 바뀌었으면 한 번 더 읽는다
        self.reload_stale();
    }

    /// 탭을 다 읽은 뒤: 방금 만든 새 폴더를 기다리던 탭이면 그 폴더의 줄에서 이름 고치기를 시작한다.
    /// 그 폴더는 파일 번호로 찾는다. 찾지 못하면(그 사이 다른 곳으로 옮겼거나, 그 탭이 보이지 않거나, 다른 입력 중이면)
    /// 아무것도 하지 않는다: 이름이 비슷한 줄이나 커서가 놓인 다른 줄에서 시작하지 않는다
    fn name_listed(&mut self, tab_id: u64) {
        let Some((_, dir, file_id)) = self.name_next.take_if(|(t, ..)| *t == tab_id) else { return };
        let Some(p) = (0..self.layout.count()).find(|p| self.panes[*p].tab().id == tab_id) else { return };
        if self.edit != Editing::None || file_id == 0 {
            return;
        }
        let tab = self.panes[p].tab_mut();
        if tab.loc.dir() != Some(dir.as_str()) {
            return;
        }
        let Some(vi) = (0..tab.view.len()).find(|vi| tab.entry_at(*vi).is_some_and(|e| e.file_id == file_id)) else { return };
        tab.select_only(vi);
        self.begin_item_rename(p);
    }

    /// 쌓인 읽기 요청과 파일 작업을 바로 처리한다 (시험·그림용, 창에서는 뒷줄에서 한다)
    pub fn settle(&mut self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        self.settle_find();
        if let Some(paths) = self.take_volume_ask() {
            self.set_volumes(volume_space(&paths));
        }
        loop {
            let (reqs, probes, previews, thumbs, ops) = (self.take_requests(), self.take_probes(), self.take_previews(), self.take_thumbs(), self.take_ops());
            let pins = self.take_pin_checks();
            if reqs.is_empty() && probes.is_empty() && previews.is_empty() && thumbs.is_empty() && ops.is_empty() && pins.is_empty() {
                break;
            }
            // 고정한 파일 확인이 먼저: 바뀌었으면 다음 바퀴에 다시 읽는다
            for c in pins {
                let found = check_pin(&c);
                self.on_pin_checked(&c, found);
            }
            for op in ops {
                let result = fileops::run(&op.job);
                self.on_op_done(&op, result);
            }
            for r in reqs {
                let result = listing::list_dir(&r.dir);
                self.on_listed(r.tab, r.generation, result);
            }
            for p in probes {
                let result = resolve_input(&p.text, &home);
                self.on_probed(&p, result);
            }
            for r in previews {
                let result = load_preview(&r);
                self.on_previewed(&r, result);
            }
            // 썸네일은 한 번에 THUMB_JOBS개씩 나온다: 보이는 것을 다 읽을 때까지 돈다
            for r in thumbs {
                let pic = load_thumb(&r);
                self.on_thumb(&r, pic);
            }
        }
        // 폴더를 옮겼으면 '지금 폴더' 범위의 검색을 다시 한다
        self.settle_find();
    }

    /// 시험·그림·창 시험: 검색이 훑는 범위를 이 폴더들로 (홈 폴더 대신)
    pub fn set_find_roots(&mut self, roots: Vec<String>) {
        self.find.roots = Some(roots);
    }

    // ---------- 자동 미리 보기 카드 ----------

    /// 카드에 띄울 파일: 지금 칸에서 하나만 고른 그림·글·문서 파일 (커서가 그 파일에 있다).
    /// 자동 미리 보기를 껐거나, 오른쪽 미리 보기 창이 열려 있으면(그 창이 보여 준다. Windows와 같다) 없다
    fn card_target(&self) -> Option<CardTarget> {
        if !self.auto_preview || self.preview.is_some() {
            return None;
        }
        let tab = self.panes[self.active].tab();
        let e = tab.cursor_entry().filter(|e| e.selected && e.kind == Kind::File)?;
        let kind = card_kind(&e.name_string())?;
        if tab.selection_stats().0 != 1 {
            return None;
        }
        Some(CardTarget { key: CardKey { path: tab.path_of(e)?, modified: e.modified, size: e.size }, kind })
    }

    /// 카드가 보이면 그 파일과 카드 자리. 닫은 파일이면 없고, 글 파일은 다 읽은 뒤에 보인다 (글로 읽지 못하면 뜨지 않는다).
    /// 문서도 썸네일(한글 문서는 미리 보기 그림이나 글)이 온 뒤에 보인다: 만들지 못하면 뜨지 않는다 (Windows와 같다.
    /// 썸네일이 없는 파일에서 빈 카드가 떴다 사라지지 않게 먼저 띄우지 않는다). 놓을 만한 칸이 없어도 없다
    fn card_shown(&self) -> Option<(CardTarget, CardGeom)> {
        let t = self.card_target()?;
        if self.card.closed.as_deref() == Some(t.key.path.as_str()) {
            return None;
        }
        let read_text = self.card.key.as_ref() == Some(&t.key) && self.card.text.is_some();
        match t.kind {
            CardKind::Text if !read_text => return None,
            CardKind::Doc if !read_text && self.pic(&t.key).is_none() => return None,
            _ => {}
        }
        let m = card_geom(&self.pane_rects(), self.active, self.card.large)?;
        Some((t, m))
    }

    fn card(&self) -> Option<CardGeom> {
        self.card_shown().map(|(_, m)| m)
    }

    /// 읽어 둔 그림
    fn pic(&self, key: &CardKey) -> Option<&CachedPic> {
        self.pics.iter().find(|c| c.key == *key)
    }

    /// 지금 고른 파일에 카드를 맞춘다: 고른 파일이 바뀌었으면 읽은 것을 비우고, 카드에 필요한 읽기를 요청에 쌓는다.
    /// 고른 것이 바뀔 수 있는 입력 뒤마다 take_previews가 이것을 거친다
    fn sync_card(&mut self) {
        let target = self.card_target();
        let key = target.as_ref().map(|t| &t.key);
        if key != self.card.key.as_ref() {
            // 닫아 둔 파일이 아닌 것을 고르면(고른 것이 없어져도) 다음에는 다시 띄운다
            if key.map(|k| k.path.as_str()) != self.card.closed.as_deref() {
                self.card.closed = None;
            }
            self.card.key = key.cloned();
            self.card.text = None;
            self.card.failed = false;
            self.card.asked = 0;
            self.card.since = Instant::now();
        }
        let Some(t) = target else {
            // 카드가 사라지면(띄울 파일이 없거나 자동 미리 보기를 껐으면) 크게 보기도 끝난다
            self.card.large = false;
            return;
        };
        if self.card.failed || self.card.closed.as_deref() == Some(t.key.path.as_str()) {
            return;
        }
        // 방금 본 그림은 남겨 둘 것들 가운데 맨 뒤로
        if let Some(i) = self.pics.iter().position(|c| c.key == t.key) {
            let c = self.pics.remove(i);
            self.pics.push(c);
        }
        // 필요한 것과 이미 가진 것: 글은 1, 그림은 긴 쪽 픽셀 수 (원본 그대로 읽어 둔 그림은 더 읽을 것이 없다).
        // 그림이 없는 한글 문서는 글을 읽었으면 더 읽을 것이 없다
        let (want, have) = match t.kind {
            CardKind::Text => (1, self.card.text.is_some() as u32),
            CardKind::Doc if self.card.text.is_some() => (1, 1),
            _ => (if self.card.large { PX_LARGE } else { PX_SMALL }, self.pic(&t.key).map_or(0, |c| if c.full() { u32::MAX } else { c.px })),
        };
        if have < want && self.card.asked < want {
            self.card.asked = want;
            let seq = self.next_seq();
            let px = if t.kind == CardKind::Text { 0 } else { want };
            self.previews.push(PreviewRequest { seq, key: t.key, px, kind: t.kind, pane: false });
        }
    }

    /// 다음 요청 번호 (카드·창의 내용 요청과 고정 확인이 함께 쓴다. 늦게 온 이전 결과를 가린다)
    fn next_seq(&mut self) -> u64 {
        self.preview_seq += 1;
        self.preview_seq
    }

    /// 지금 고른 파일에 카드와 오른쪽 미리 보기 창을 맞추고, 쌓인 내용 읽기 요청을 내준다.
    /// 창 쪽이 입력 뒤와 폴더를 읽은 뒤마다 불러(pump) 뒷줄에서 읽고 on_previewed로 돌려준다
    pub fn take_previews(&mut self) -> Vec<PreviewRequest> {
        self.sync_preview();
        self.sync_card();
        std::mem::take(&mut self.previews)
    }

    /// 뒷줄에서 읽은 카드·창의 내용. 그 사이 다른 파일을 골랐으면(지금 카드의 파일이 아니면) 버린다
    pub fn on_previewed(&mut self, req: &PreviewRequest, result: Option<Preview>) {
        if req.pane {
            return self.on_preview_loaded(req, result);
        }
        if self.card.key.as_ref() != Some(&req.key) {
            return;
        }
        match result {
            Some(Preview::Image(pic)) => {
                // 같은 파일을 더 크게 읽은 것이 이미 있으면 그대로 둔다 (작게 읽은 것이 늦게 온 경우)
                if self.pic(&req.key).is_some_and(|c| c.px >= req.px) {
                    return;
                }
                self.pics.retain(|c| c.key != req.key);
                self.pics.push(CachedPic { key: req.key.clone(), px: req.px, pic });
                if self.pics.len() > PICS_MAX {
                    self.pics.remove(0);
                }
            }
            Some(Preview::Text(text)) => self.card.text = Some(text),
            // 크게 읽는 것만 실패했으면 읽어 둔 작은 그림을 그대로 보인다
            None => self.card.failed = self.pic(&req.key).is_none(),
        }
    }

    /// 도구 막대의 눈: 자동 미리 보기를 끄고 켠다 (무엇이 바뀌었는지 지금 칸의 칩 줄에 잠깐 알린다)
    pub fn toggle_auto_preview(&mut self) {
        self.auto_preview = !self.auto_preview;
        let note = if self.auto_preview { "자동 미리 보기를 켰어요 · 그림·글·문서 파일을 고르면 보여요" } else { "자동 미리 보기를 껐어요" };
        self.flash = Some((self.active, note.to_string()));
    }

    /// 카드의 닫기: 다른 파일을 고를 때까지 다시 띄우지 않는다
    fn close_card(&mut self) {
        self.card.closed = self.card_target().map(|t| t.key.path);
        self.card.large = false;
    }

    /// 카드를 크게(놓인 칸을 채운다)·작게
    pub fn toggle_card_large(&mut self) {
        self.card.large = !self.card.large;
    }

    // ---------- 아이콘 보기의 썸네일 ----------

    /// 지금 화면에 보이는 파일들의 썸네일 (보이는 순서): 아이콘 보기인 칸마다 지금 탭의 보이는 줄.
    /// 칸 그리기(ui::paint_pane_list)가 썸네일을 그리는 항목과 같다
    fn visible_thumbs(&self) -> Vec<ThumbKey> {
        let mut out = Vec::new();
        for (p, r) in self.pane_rects() {
            let list = geom(r).list;
            let tab = self.panes[p].tab();
            let grid = Grid::of(list, self.panes[p].view);
            let CellKind::Tile(size) = grid.cell else { continue };
            out.extend(grid.visible(list, tab.scroll, tab.view.len()).filter_map(|vi| thumb_key(tab, tab.entry_at(vi)?, size, self.scale)));
        }
        out
    }

    /// 썸네일을 지금 화면에 맞추고(지금 보이는데 아직 없는 것만 기다린다), 지금 읽기 시작할 요청을 내준다.
    /// 읽는 중인 것은 THUMB_JOBS개를 넘지 않는다. 창 쪽이 그린 뒤와 썸네일 하나를 다 읽은 뒤마다 불러,
    /// 뒷줄에서 load_thumb로 읽고 on_thumb로 돌려준다 (메인 줄에서는 그림 파일을 읽지 않는다)
    pub fn take_thumbs(&mut self) -> Vec<ThumbRequest> {
        let visible = self.visible_thumbs();
        self.thumbs.sync(visible);
        self.thumbs.start()
    }

    /// 뒷줄에서 읽은 썸네일 (읽지 못했으면 None: 그것도 적어 두고 다시 요청하지 않는다).
    /// 그 사이 스크롤했거나 다른 폴더로 옮겨 지금은 보이지 않는 것이어도 남겨 둔다
    pub fn on_thumb(&mut self, req: &ThumbRequest, pic: Option<Picture>) {
        self.thumbs.finish(&req.key, pic);
    }

    // ---------- 경로 입력 ----------

    /// 칸 p에서 경로 입력을 시작한다 (폴더나 시작 화면을 보고 있을 때. 시작 화면이면 빈 상자로)
    pub fn begin_address(&mut self, p: usize) {
        let loc = &self.panes[p].tab().loc;
        if loc.dir().is_some() || *loc == Location::Start {
            self.close_commands();
            self.activate_pane(p);
            self.edit = Editing::Address(p);
        }
    }

    /// 입력을 그만둔다 (경로 입력, 공간 이름·항목 이름 고치기). 이미 맡긴 이름 바꾸기는 그대로 끝까지 간다
    pub fn cancel_address(&mut self) {
        self.edit = Editing::None;
    }

    /// 입력 상자의 글을 확정한다: 공간 이름이면 그 이름으로, 항목 이름이면 이름 바꾸기를 맡기고, 경로면 그 경로로 간다.
    /// 경로 확인은 뒷줄에서 하고 결과는 on_probed로 받는다
    pub fn commit_address(&mut self, text: &str) {
        if matches!(self.edit, Editing::Item(_)) {
            return self.commit_item_rename(text);
        }
        match std::mem::replace(&mut self.edit, Editing::None) {
            Editing::Space => self.commit_rename(text),
            Editing::Address(p) => self.probe_path(p, text.to_string(), false),
            Editing::Group(g) => self.commit_group_rename(g, text),
            Editing::None | Editing::Item(_) => {}
        }
    }

    /// 칸 p에서 친 경로를 확인해 달라고 한다 (경로 입력, 명령 메뉴). 확인은 뒷줄에서 하고 결과는 on_probed로 받는다:
    /// 꺼진 네트워크 경로를 확인하는 동안 멈추지 않는다. new_tab이면 확인한 폴더를 새 탭에서 연다
    fn probe_path(&mut self, p: usize, text: String, new_tab: bool) {
        let (tab, generation) = (self.panes[p].tab().id, self.panes[p].tab().generation);
        self.probe_seq[p] += 1;
        self.probe_at[p] = (tab, generation);
        self.probes.push(Probe { pane: p, tab, seq: self.probe_seq[p], text, new_tab });
    }

    /// 그림 시험용: 칸 p에서 text를 입력해 확정한 결과까지 바로 반영한다
    pub fn commit_address_for_render(&mut self, p: usize, text: &str) {
        self.begin_address(p);
        self.commit_address(text);
        self.settle();
    }

    pub fn take_probes(&mut self) -> Vec<Probe> {
        std::mem::take(&mut self.probes)
    }

    /// 경로 확인 결과. 그 사이 그 칸의 지금 탭이 바뀌었거나, 그 탭이 다른 곳으로 옮겼거나, 더 나중에 낸 경로 입력이
    /// 있으면 버린다 (확인이 늦게 끝난 순서대로 옮겨 다니지 않게). 그 사이 같은 폴더를 다시 읽은 것(폴더 감시, ⌘R,
    /// 파일 작업 뒤)은 옮긴 것이 아니라서 버리지 않는다
    pub fn on_probed(&mut self, probe: &Probe, result: Result<(String, Option<String>), String>) {
        let p = probe.pane;
        let tab = self.panes[p].tab();
        if tab.id != probe.tab || self.probe_at[p] != (tab.id, tab.generation) || self.probe_seq[p] != probe.seq {
            return;
        }
        match result {
            Ok((dir, select)) if probe.new_tab => self.open_tab(p, Location::Dir(dir), select),
            Ok((dir, select)) => {
                // 대소문자를 가리는 볼륨에서는 Work와 work가 다른 폴더라 글자 그대로 비교한다
                let here = self.panes[p].tab().loc.dir() == Some(dir.as_str());
                if here && select.is_none() {
                    self.refresh(p);
                } else {
                    self.navigate(p, Location::Dir(dir), select);
                }
            }
            Err(msg) => self.flash = Some((p, msg)),
        }
    }

    /// 입력 상자 자리: 공간 이름이면 사이드바의 이름 자리, 항목 이름이면 그 줄의 이름 자리, 그룹 이름이면 사이드바의 그 머리 줄,
    /// 경로면 칸의 경로 자리 (app.rs addr_rect)
    pub fn address_rect(&self) -> Option<R> {
        match &self.edit {
            Editing::None => None,
            Editing::Space => Some(self.rename_rect()),
            Editing::Address(p) => {
                let r = self.pane_rects().into_iter().find(|(q, _)| q == p)?.1;
                Some(addr_rect(&geom(r)))
            }
            Editing::Item(_) => self.item_edit().map(|(_, r)| r),
            Editing::Group(g) => self.group_edit_rect(*g),
        }
    }

    /// 입력 상자에 처음 넣을 글자: 지금 공간의 이름, 고치는 항목·그룹의 이름, 또는 지금 폴더
    pub fn address_text(&self) -> String {
        match &self.edit {
            Editing::None => String::new(),
            Editing::Space => self.workspaces.current_name().unwrap_or_default().to_string(),
            Editing::Address(p) => self.panes[*p].tab().loc.dir().unwrap_or_default().to_string(),
            Editing::Item(e) => e.name.clone(),
            Editing::Group(g) => self.favs.groups.get(*g).map(|grp| grp.name.clone()).unwrap_or_default(),
        }
    }

    /// 입력 상자에서 미리 골라 둘 앞부분의 길이 (UTF-16 단위). 항목 이름을 고칠 때만 있다: 파일은 확장자 앞까지.
    /// None이면 전체를 골라 둔다
    pub fn address_select(&self) -> Option<usize> {
        match &self.edit {
            Editing::Item(e) => Some(sakadu::fileops::stem_units(&e.name, e.is_dir)),
            _ => None,
        }
    }

    /// 입력 상자의 글자가 상자 왼쪽 끝에서 들어가 시작하는 거리. 항목·그룹 이름은 그려져 있던 이름과 같은 자리에서 시작한다
    pub fn address_pad(&self) -> f32 {
        if matches!(self.edit, Editing::Item(_) | Editing::Group(_)) { 2.0 } else { 6.0 }
    }

    /// 입력 상자가 지금 무엇을 담는지 가리는 값 (바뀌면 창 쪽이 글자를 다시 채운다): 경로를 입력하는 칸, 공간 이름, 항목 이름 고치기의 번호,
    /// 그룹 번호 (항목 고치기의 번호와 겹치지 않게 높은 자리에 둔다)
    pub fn edit_key(&self) -> Option<u64> {
        match &self.edit {
            Editing::None => None,
            Editing::Address(p) => Some(*p as u64),
            Editing::Space => Some(100),
            Editing::Item(e) => Some(1000 + e.serial),
            Editing::Group(g) => Some((1 << 40) + *g as u64),
        }
    }

    /// 입력 상자가 키 입력을 잃었다 (다른 곳을 눌렀다): 입력을 그만둔다. 입력 중이었으면 true
    pub fn end_editing(&mut self) -> bool {
        let was = self.edit != Editing::None;
        self.cancel_address();
        was
    }

    // ---------- 파일 작업: 새 폴더, 이름 바꾸기, 휴지통, 되돌리기 ----------

    fn push_op(&mut self, tab: u64, job: Job) -> u64 {
        self.op_seq += 1;
        self.ops_running += 1;
        self.ops.push(Op { seq: self.op_seq, tab, job });
        self.op_seq
    }

    /// 맡길 파일 작업을 내준다. 창 쪽이 뒷줄에서 fileops::run으로 하나씩 실행하고 on_op_done으로 돌려준다
    pub fn take_ops(&mut self) -> Vec<Op> {
        std::mem::take(&mut self.ops)
    }

    /// 칸 p의 폴더에 새 폴더를 만든다 (⇧⌘N, 도구 막대). 만든 뒤 그 줄에서 이름을 바로 고칠 수 있다
    pub fn new_folder(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        let (Some(dir), LoadState::Ready) = (tab.loc.dir(), &tab.state) else { return };
        let (tab, dir) = (tab.id, dir.to_string());
        self.push_op(tab, Job::NewFolder { dir });
    }

    /// 칸 p에서 고른 항목들을 휴지통으로 보낸다 (⌘⌫). 실제로 고른 것만: 커서만 놓인 줄은 대상이 아니다.
    /// 묻지 않고 보낸다 (잘못 보냈으면 ⌘Z로 되돌린다)
    pub fn trash_selected(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        let Some(dir) = tab.loc.dir().map(str::to_string) else { return };
        let items: Vec<(String, u64)> = (0..tab.view.len()).filter_map(|vi| tab.entry_at(vi)).filter(|e| e.selected).map(|e| (e.name_string(), e.file_id)).collect();
        let tab = tab.id;
        self.trash_items(tab, dir, items);
    }

    /// dir 안의 항목들(이름, 목록에서 본 파일 번호)을 휴지통으로 보내 달라고 맡긴다
    fn trash_items(&mut self, tab: u64, dir: String, items: Vec<(String, u64)>) {
        if !items.is_empty() {
            self.push_op(tab, Job::Trash { dir, items });
        }
    }

    /// 지금 칸에서 고른 항목들을 클립보드에 담아 달라고 한다 (⌘C). 실제로 고른 것만 담는다
    pub fn copy_selected(&mut self) {
        self.clip_selected(false);
    }

    /// 지금 칸에서 고른 항목들을 잘라 둔다 (⌘X): 클립보드에 담고, 다음에 붙여넣을 때 복사하지 않고 옮긴다.
    /// 붙여넣기 전까지 파일은 그대로다
    pub fn cut_selected(&mut self) {
        self.clip_selected(true);
    }

    fn clip_selected(&mut self, cut: bool) {
        if self.edit != Editing::None {
            return;
        }
        let paths = self.panes[self.active].tab().marked_paths();
        self.copy_out(paths, cut);
    }

    fn copy_out(&mut self, paths: Vec<String>, cut: bool) {
        if paths.is_empty() {
            return;
        }
        let note = if cut { format!("{}개를 잘라 뒀어요 · ⌘V로 옮겨요", paths.len()) } else { format!("{}개를 복사해 뒀어요 · ⌘V로 붙여넣어요", paths.len()) };
        self.flash = Some((self.active, note));
        self.clip_out = Some(paths);
        self.clip_cut = cut;
    }

    /// 클립보드의 파일들을 지금 칸에 붙여넣어 달라고 한다 (⌘V). 창 쪽이 클립보드를 읽어 paste_paths로 넘긴다.
    /// force_move면 잘라낸 것이 아니어도 옮긴다 (⌥⌘V: Finder의 '여기로 항목 이동')
    pub fn ask_paste(&mut self, force_move: bool) {
        if self.edit == Editing::None {
            self.flash = None;
            self.clip_in = Some(force_move);
        }
    }

    /// 클립보드에서 읽은 파일들(전체 경로)을 지금 칸의 폴더로 복사하거나 옮긴다. 시작했으면 true
    pub fn paste_paths(&mut self, paths: Vec<String>, how: Paste) -> bool {
        let tab = self.panes[self.active].tab();
        let Some(dir) = tab.loc.dir().map(str::to_string) else { return false };
        if paths.is_empty() {
            self.flash = Some((self.active, "붙여넣을 파일이 클립보드에 없어요".to_string()));
            return false;
        }
        let tab = tab.id;
        self.start_copy(tab, dir, paths.into_iter().map(|path| (path, 0)).collect(), false, how)
    }

    /// 칸 p에서 고른 항목들을 같은 폴더에 복제한다 (⌘D). 이름에 번호가 붙는다
    pub fn duplicate_selected(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        let Some(dir) = tab.loc.dir().map(str::to_string) else { return };
        let items: VecDeque<(String, u64)> = (0..tab.view.len()).filter_map(|vi| tab.entry_at(vi)).filter(|e| e.selected).map(|e| (util::join(&dir, &e.name_string()), e.file_id)).collect();
        let tab = tab.id;
        self.start_copy(tab, dir, items, true, Paste::Copy);
    }

    /// 항목들(전체 경로, 목록에서 본 파일 번호)을 폴더 dir로 복사하거나 옮기기 시작한다. 하나씩 맡긴다. 시작했으면 true
    fn start_copy(&mut self, tab: u64, dir: String, items: VecDeque<(String, u64)>, duplicate: bool, how: Paste) -> bool {
        if items.is_empty() {
            return false;
        }
        if self.batch.is_some() {
            self.flash = Some((self.active, "앞의 복사가 끝난 뒤에 해 주세요".to_string()));
            return false;
        }
        self.batch = Some(Batch {
            tab,
            dir,
            total: items.len(),
            todo: items,
            made: Vec::new(),
            sources: Vec::new(),
            renamed: 0,
            failed: Vec::new(),
            moved: Vec::new(),
            duplicate,
            how,
            cancelled: false,
        });
        self.feed_batch();
        true
    }

    /// 묶음의 다음 항목을 맡긴다. 더 없거나 그만두기로 했으면 묶음을 끝낸다
    fn feed_batch(&mut self) {
        let Some(b) = &mut self.batch else { return };
        let next = if b.cancelled { None } else { b.todo.pop_front() };
        match next {
            Some((src, file_id)) => {
                let (tab, dir, how) = (b.tab, b.dir.clone(), b.how);
                let job = match how {
                    Paste::Copy => Job::Copy { src, dir, file_id },
                    // 끌어다 놓기만: 다른 디스크면 옮기는 대신 복사한다
                    Paste::Move | Paste::Cut(_) | Paste::Drop => Job::Move { src, dir, file_id, or_copy: how == Paste::Drop },
                };
                self.push_op(tab, job);
            }
            None => self.finish_batch(),
        }
    }

    /// 묶음을 끝낸다: 무엇을 했는지 알리고, 만든 복사본(옮긴 것)을 고르고, 되돌리기 기록에 남기고, 다시 읽는다
    fn finish_batch(&mut self) {
        let Some(b) = self.batch.take() else { return };
        // 옮긴 것(원래 전체 경로, 이름, 파일 번호)과 복사한 것(이름, 파일 번호)을 가른다
        let (mut moved, mut copied) = (Vec::new(), Vec::new());
        for (((name, file_id), src), was_moved) in b.made.iter().zip(&b.sources).zip(&b.moved) {
            if *was_moved {
                moved.push((src.clone(), name.clone(), *file_id));
            } else {
                copied.push((name.clone(), *file_id));
            }
        }
        let done = match (moved.len(), copied.len()) {
            (0, 0) => None,
            (m, 0) => Some(format!("{m}개를 옮겼")),
            (0, c) => Some(format!("{c}개를 복사했")),
            (m, c) => Some(format!("{m}개를 옮기고 {c}개를 복사했")),
        };
        let copying = b.how == Paste::Copy;
        let mut note = match (done, b.failed.first().map(|(src, why)| (util::file_name(src), why))) {
            (None, None) => if copying { "복사한 것이 없어요" } else { "옮긴 것이 없어요" }.to_string(),
            (Some(done), None) => format!("{done}어요"),
            (None, Some((name, why))) => format!("{}: {name} · {why}", if copying { "복사하지 못했어요" } else { "옮기지 못했어요" }),
            (Some(done), Some((name, why))) => format!("{done}고 {}개는 하지 못했어요: {name} · {why}", b.failed.len()),
        };
        // 잘라 둔 것 가운데 옮기지 못한 것(하지 못한 것, 그만둬서 하지 않은 것)은 잘라 둔 채로 남긴다
        if let Paste::Cut(mark) = b.how {
            self.clip_left = Some((mark, b.failed.iter().map(|(src, _)| src.clone()).chain(b.todo.iter().map(|(src, _)| src.clone())).collect()));
        }
        if b.renamed > 0 && !b.duplicate {
            note.push_str(&format!(" · 같은 이름이 있던 {}개에는 번호를 붙였어요", b.renamed));
        }
        if !b.todo.is_empty() {
            note.push_str(&format!(" · 그만둬서 {}개는 하지 않았어요", b.todo.len()));
        }
        // 옮겨 간 것은 원래 폴더를 보이는 탭의 고른 것에서 빼고, 옮긴 폴더 안을 보던 탭은 새 경로로 따라가게 한다
        for (src, name, file_id) in &moved {
            if let Some(from) = util::parent(src) {
                self.deselect_file(&from, *file_id);
            }
            self.tabs_moved(src, &util::join(&b.dir, name));
        }
        let pane = self.pane_showing(b.tab, &b.dir);
        let names: Vec<String> = b.made.iter().map(|(name, _)| name.clone()).collect();
        if let Some(p) = pane.filter(|_| !names.is_empty()) {
            self.select_after(p, &names);
        }
        // 되돌리기 기록은 한 번의 붙여넣기·놓기에 하나: 옮긴 것과 복사한 것이 섞였으면 묶어서 함께 되돌린다
        let mut records = Vec::new();
        if !copied.is_empty() {
            records.push(Undo::Copy { dir: b.dir.clone(), items: copied });
        }
        if !moved.is_empty() {
            records.push(Undo::Move { to: b.dir, items: moved });
        }
        if records.len() > 1 {
            self.undos.push(Undo::Group(records));
        } else if let Some(one) = records.pop() {
            self.undos.push(one);
        }
        self.flash = Some((pane.unwrap_or(self.active), note));
        self.refresh_changed();
    }

    /// 끌려 온 것(paths)을 (x, y)에 놓으면 어디로 가는지: 놓을 곳, 받는 폴더, 그 묶음을 맡을 탭.
    /// 폴더 줄 위면 그 폴더 안으로, 그 밖의 목록 자리면 그 칸이 보이는 폴더로, 사이드바의 폴더 줄이면 그 폴더로,
    /// 탭 칩 위면 그 탭이 보이는 폴더로 (지금 탭이 아니어도. Windows와 같다).
    /// 놓을 수 없는 곳(도구 막대, 시작 화면 탭, 파일을 가리키는 사이드바 줄)이나 놓아도 달라질 것이 없는 곳(이미 그 폴더에 있다,
    /// 끌고 있는 폴더 자신이나 그 안)이면 None
    fn drop_target(&self, g: &Gfx, x: f32, y: f32, paths: &[String]) -> Option<(DropAt, String, u64)> {
        // 미리 보기 카드는 없는 것으로 보고 그 아래 칸을 찾는다
        let hit = self.hit_with(g, x, y, false);
        let (at, dir, tab) = match hit {
            Hit::SideRow(i) => {
                let rows = self.side_rows();
                let row = rows.get(i).filter(|row| row.is_dir)?;
                (DropAt::Side(i), row.path.clone()?, self.panes[self.active].tab().id)
            }
            // 탭 칩: 그 탭의 폴더 (닫기 단추 위도 그 탭이다). 묶음도 그 탭이 맡는다
            Hit::Tab(p, i) | Hit::TabClose(p, i) => {
                let tab = self.panes[p].tabs.get(i)?;
                (DropAt::Tab(p, i), tab.loc.dir()?.to_string(), tab.id)
            }
            // 경로 줄은 지금 탭의 것이다
            Hit::Row(p, _) | Hit::List(p) | Hit::Pane(p) | Hit::Crumb(p, _) | Hit::CrumbEmpty(p) | Hit::Star(p) => {
                let tab = self.panes[p].tab();
                let here = tab.loc.dir()?.to_string();
                // 폴더 줄 위면 그 폴더 안으로. 끌고 있는 항목 자신의 줄은 그 칸의 폴더로 본다
                let folder = match hit {
                    Hit::Row(_, vi) => tab.entry_at(vi).filter(|e| e.is_dir()).and_then(|e| tab.path_of(e)).filter(|path| !paths.contains(path)).map(|path| (vi, path)),
                    _ => None,
                };
                match folder {
                    Some((vi, path)) => (DropAt::Row(p, vi), path, tab.id),
                    None => (DropAt::Pane(p), here, tab.id),
                }
            }
            _ => return None,
        };
        // 모두 이미 그 폴더에 있으면 놓아도 달라질 것이 없다 (같은 칸 안에서 조금 끌다 놓은 경우)
        if paths.iter().all(|path| util::parent(path).as_deref() == Some(dir.as_str())) {
            return None;
        }
        // 끌고 있는 폴더 자신이나 그 안으로는 놓지 않는다
        if paths.iter().any(|path| under(&dir, path).is_some()) {
            return None;
        }
        Some((at, dir, tab))
    }

    /// 끌려 온 것이 (x, y) 위에 있다: 놓일 곳을 강조하고 받는 폴더를 돌려준다. 놓을 수 없으면 None (강조도 없다)
    pub fn drag_over(&mut self, g: &Gfx, x: f32, y: f32, paths: &[String]) -> Option<String> {
        let target = self.drop_target(g, x, y, paths);
        self.drop = target.as_ref().map(|(at, ..)| *at);
        // 놓일 곳이 다른 칸으로 옮겨 갔으면, 앞의 칸에서 미뤄 둔 것을 한다
        self.after_pointer();
        target.map(|(_, dir, _)| dir)
    }

    /// 끌려 온 것이 창을 벗어났거나 끌기가 끝났다: 강조를 지운다
    pub fn drag_exit(&mut self) {
        self.drop = None;
        // 끌려 온 것이 위에 있는 동안 미뤄 둔 것
        self.after_pointer();
    }

    /// (x, y)에 놓았다: 그곳의 폴더로 옮기거나 복사하기 시작한다 (how가 Drop이면 같은 디스크는 옮기고 다른 디스크는 복사).
    /// 칸에 놓았으면 그 칸이 지금 칸이 된다. 시작했으면 true
    pub fn drop_paths(&mut self, g: &Gfx, x: f32, y: f32, paths: Vec<String>, how: Paste) -> bool {
        self.drop = None;
        self.flash = None;
        // 놓을 곳은 끄는 동안 보던 그 목록에서 찾는다 (미뤄 둔 읽기 결과와 다시 읽기는 그다음에 한다)
        let started = match self.drop_target(g, x, y, &paths) {
            Some((at, dir, tab)) => {
                if let DropAt::Pane(p) | DropAt::Row(p, _) | DropAt::Tab(p, _) = at {
                    self.activate_pane(p);
                }
                self.edit = Editing::None;
                self.start_copy(tab, dir, paths.into_iter().map(|path| (path, 0)).collect(), false, how)
            }
            None => false,
        };
        self.after_pointer();
        started
    }

    /// 끌어 내 달라는 항목들의 경로를 내준다 (고른 줄을 누른 채 움직였을 때 한 번). 창 쪽이 맥의 끌기를 시작한다
    pub fn take_drag(&mut self) -> Option<Vec<String>> {
        self.drag_out.take()
    }

    /// 진행 중인 복사나 옮기기를 그만둔다 (Esc): 하고 있던 항목은 마치고, 남은 것은 하지 않는다
    pub fn cancel_copy(&mut self) {
        if let Some(b) = &mut self.batch {
            b.cancelled = true;
        }
    }

    /// 복사·옮기기의 진행: (지금 하고 있는 것이 몇 번째인지, 모두 몇 개인지). 하고 있지 않으면 None
    pub fn copy_progress(&self) -> Option<(usize, usize)> {
        self.batch.as_ref().map(|b| ((b.made.len() + b.failed.len() + 1).min(b.total), b.total))
    }

    /// 폴더 dir을 보이고 있는 칸: 그 탭의 칸이 먼저, 아니면 그 폴더를 보이는 다른 칸
    fn pane_showing(&self, tab: u64, dir: &str) -> Option<usize> {
        let shows = |p: &usize| self.panes[*p].tab().loc.dir() == Some(dir);
        (0..self.layout.count()).find(|p| self.panes[*p].tab().id == tab && shows(p)).or_else(|| (0..self.layout.count()).find(shows))
    }

    /// 가장 나중에 한 파일 작업(이름 바꾸기, 새 폴더, 휴지통, 복사, 옮기기)을 되돌린다 (⌘Z).
    /// 이름이나 경로를 치는 중에는 하지 않는다: 그때의 ⌘Z는 입력 상자의 글자에 쓰는 키다
    pub fn undo(&mut self) {
        if self.edit != Editing::None {
            return;
        }
        // 앞의 작업이 끝나야 그 기록이 생긴다: 끝나기 전에 누르면 그보다 앞의 작업이 되돌려진다
        let note = if self.ops_running > 0 {
            Some("앞의 작업이 끝난 뒤에 되돌릴 수 있어요")
        } else if let Some(undo) = self.undos.pop() {
            let tab = self.panes[self.active].tab().id;
            self.push_op(tab, Job::Undo(undo));
            None
        } else {
            Some("되돌릴 작업이 없어요")
        };
        self.flash = note.map(|n| (self.active, n.to_string()));
    }

    /// 칸 p에서 고른 항목의 이름을 고치기 시작한다 (Return). 하나만 골랐을 때만: 여럿이면 알리고, 없으면 아무것도 하지 않는다
    pub fn begin_item_rename(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        let Some(dir) = tab.loc.dir().map(str::to_string) else { return };
        let chosen: Vec<usize> = (0..tab.view.len()).filter(|vi| tab.is_selected(*vi)).take(2).collect();
        let &[vi] = chosen.as_slice() else {
            if chosen.len() > 1 {
                self.flash = Some((p, "이름은 하나씩 바꿀 수 있어요".to_string()));
            }
            return;
        };
        let Some(e) = tab.entry_at(vi) else { return };
        let (tab_id, name, file_id, is_dir) = (tab.id, e.name_string(), e.file_id, e.is_dir());
        // 파일 번호를 모르는 항목은 실행할 때 같은 항목인지 볼 수 없다
        if file_id == 0 {
            self.flash = Some((p, "이 항목은 이름을 바꿀 수 없어요".to_string()));
            return;
        }
        self.activate_pane(p);
        let tab = self.panes[p].tab_mut();
        tab.cursor = vi;
        tab.anchor = vi;
        self.ensure_visible(p);
        // 창이 너무 낮아 그 줄이 다 보이지 않으면 시작하지 않는다
        if self.item_name_rect(p).is_none() {
            return;
        }
        self.edit_serial += 1;
        self.edit = Editing::Item(ItemEdit { pane: p, tab: tab_id, dir, name, file_id, is_dir, serial: self.edit_serial, waiting: None });
    }

    /// 고치는 중인 항목과 그 이름 상자 자리. 그 항목이 그 칸의 지금 탭, 그 폴더의 커서 줄에 그대로 있고 줄이 다 보일 때만 있다
    fn item_edit(&self) -> Option<(&ItemEdit, R)> {
        let Editing::Item(e) = &self.edit else { return None };
        if e.pane >= self.layout.count() {
            return None;
        }
        let tab = self.panes[e.pane].tab();
        if tab.id != e.tab || tab.loc.dir() != Some(e.dir.as_str()) {
            return None;
        }
        let cur = tab.cursor_entry()?;
        if cur.file_id != e.file_id || cur.name_string() != e.name {
            return None;
        }
        Some((e, self.item_name_rect(e.pane)?))
    }

    /// 항목 이름을 고치는 중인데 그 항목이 더는 그 자리에 없으면(탭이 다른 곳으로 갔거나, 다른 탭·칸 나누기로 바뀌었거나,
    /// 목록에서 사라졌거나, 그 줄이 가려졌으면) 고치기를 끝낸다. 창 쪽이 입력 상자를 맞추기 전에 부른다
    pub fn sync_edit(&mut self) {
        if matches!(self.edit, Editing::Item(_)) && self.item_edit().is_none() {
            self.edit = Editing::None;
        }
        // 그룹 이름: 그 그룹이 없어졌거나 머리 줄이 사이드바 목록에 다 보이지 않으면(검색 패널로 바뀌었으면) 끝낸다
        if let Editing::Group(g) = self.edit
            && self.group_edit_rect(g).is_none()
        {
            self.edit = Editing::None;
        }
        // 경로·이름을 고치기 시작했으면 키보드는 그 상자로 갔다 (고치기를 마쳐도 검색 입력 칸으로 돌아가지 않는다)
        if self.edit != Editing::None {
            self.find_blur();
        }
    }

    /// 칸 p의 커서 줄에서 이름이 놓인 자리 (공통 칸 그리기가 이름 상자를 그리는 자리와 같다). 줄이 다 보이지 않으면 None
    fn item_name_rect(&self, p: usize) -> Option<R> {
        let list = self.list_rect(p)?;
        let tab = self.panes[p].tab();
        name_edit_rect(list, self.panes[p].view, self.cols(p), tab.cursor, tab.scroll)
    }

    /// 고친 이름을 확정한다: 이름 바꾸기를 맡기고, 끝날 때까지 상자는 그대로 둔다.
    /// 쓸 수 없는 이름이나 이미 있는 이름이면 상자를 친 그대로 열어 둔 채 알린다
    fn commit_item_rename(&mut self, text: &str) {
        let Some((e, _)) = self.item_edit() else {
            // 고치는 동안 탭이 다른 곳으로 갔거나 목록이 바뀌었다
            self.edit = Editing::None;
            self.flash = Some((self.active, "그 사이 목록이 바뀌어 이름을 바꾸지 않았어요".to_string()));
            return;
        };
        if e.waiting.is_some() {
            return;
        }
        let e = e.clone();
        let new = text.trim();
        if new.is_empty() || new == e.name {
            self.edit = Editing::None;
            return;
        }
        if let Some(problem) = sakadu::fileops::mac_name_problem(new, self.show_hidden) {
            self.flash = Some((e.pane, problem));
            return;
        }
        // 목록에 같은 이름이 보이면 바로 알린다 (목록이 낡았을 수 있어, 실행할 때 한 번 더 걸러진다)
        if self.panes[e.pane].tab().entries.iter().any(|x| x.file_id != e.file_id && x.name_string() == new) {
            self.flash = Some((e.pane, "같은 이름이 이미 있어요".to_string()));
            return;
        }
        let seq = self.push_op(e.tab, Job::Rename { dir: e.dir, from: e.name, to: new.to_string(), file_id: e.file_id });
        if let Editing::Item(cur) = &mut self.edit {
            cur.waiting = Some(seq);
        }
    }

    /// 뒷줄에서 끝낸 파일 작업의 결과
    pub fn on_op_done(&mut self, op: &Op, result: Result<Done, Fail>) {
        self.ops_running = self.ops_running.saturating_sub(1);
        // 파일 작업으로 고정한 파일이 옮겨지거나 지워졌을 수 있다
        self.pin_check();
        // 복사·옮기기 묶음의 한 항목: 결과를 묶음에 적고 다음 항목을 맡긴다 (알림과 다시 읽기는 묶음이 끝날 때 한 번)
        if let Job::Copy { src, .. } | Job::Move { src, .. } = &op.job {
            if let Some(b) = &mut self.batch {
                match result {
                    Ok(done @ (Done::Copied { .. } | Done::Moved { .. })) => {
                        b.moved.push(matches!(done, Done::Moved { .. }));
                        let (Done::Copied { name, file_id, renamed } | Done::Moved { name, file_id, renamed }) = done else { unreachable!() };
                        b.made.push((name, file_id));
                        b.sources.push(src.clone());
                        b.renamed += usize::from(renamed);
                    }
                    Ok(_) => {}
                    Err(fail) => {
                        let why = match fail {
                            Fail::Exists => "같은 이름이 이미 있어요".to_string(),
                            Fail::Changed => "그 사이 없어졌거나 다른 것으로 바뀌었어요".to_string(),
                            Fail::Other(msg) => msg,
                        };
                        b.failed.push((src.clone(), why));
                    }
                }
            }
            return self.feed_batch();
        }
        // 그 작업을 낸 탭이 지금 보이는 칸 (알림도 그 칸에. 보이지 않으면 지금 칸에)
        let pane = (0..self.layout.count()).find(|p| self.panes[*p].tab().id == op.tab);
        let note_at = pane.unwrap_or(self.active);
        // 그 폴더를 보이고 있는 칸: 작업을 낸 탭의 칸이 먼저, 아니면 그 폴더를 보이는 다른 칸 (되돌리기는 어느 칸에서나 한다)
        let here = |a: &App, dir: &str| {
            let shows = |p: &usize| a.panes[*p].tab().loc.dir() == Some(dir);
            pane.filter(shows).or_else(|| (0..a.layout.count()).find(shows))
        };
        // 이 작업이 끝나기를 기다리던 이름 상자
        let waiting = matches!(&self.edit, Editing::Item(e) if e.waiting == Some(op.seq));
        let note = match (&op.job, result) {
            (Job::NewFolder { dir }, Ok(Done::NewFolder { name, file_id })) => {
                // 그 탭이 아직 그 폴더를 보이고 있으면 만든 폴더를 고르고, 다 읽은 뒤 그 줄에서 이름을 고친다
                if let Some(p) = pane.filter(|p| self.panes[*p].tab().loc.dir() == Some(dir.as_str())) {
                    self.select_after(p, std::slice::from_ref(&name));
                    self.name_next = Some((op.tab, dir.clone(), file_id));
                }
                if file_id != 0 {
                    self.undos.push(Undo::NewFolder { dir: dir.clone(), name, file_id });
                }
                None
            }
            (Job::Rename { dir, from, to, file_id }, Ok(_)) => {
                if waiting {
                    self.edit = Editing::None;
                }
                self.deselect_file(dir, *file_id);
                self.tabs_moved(&util::join(dir, from), &util::join(dir, to));
                if let Some(p) = here(self, dir) {
                    self.select_after(p, std::slice::from_ref(to));
                }
                self.undos.push(Undo::Rename { dir: dir.clone(), from: from.clone(), to: to.clone(), file_id: *file_id });
                None
            }
            (Job::Trash { dir, .. }, Ok(Done::Trashed { moved, failed })) => {
                for t in &moved {
                    self.deselect_file(dir, t.file_id);
                    self.tabs_left(&util::join(dir, &t.name), dir);
                }
                // 되돌릴 수 있다고 약속하지 않는다: 맥의 개인정보 보호 때문에 휴지통에서 꺼내지 못할 수 있다
                let note = match (moved.len(), failed.first()) {
                    (n, None) => format!("{n}개를 휴지통으로 옮겼어요"),
                    (0, Some((name, why))) => format!("휴지통으로 옮기지 못했어요: {name} · {why}"),
                    (n, Some((name, why))) => format!("{n}개를 휴지통으로 옮겼고 {}개는 그대로 뒀어요: {name} · {why}", failed.len()),
                };
                if !moved.is_empty() {
                    self.undos.push(Undo::Trash { dir: dir.clone(), items: moved });
                }
                Some(note)
            }
            (Job::Undo(undo), Ok(Done::Undone { left })) => {
                self.after_undo(op.tab, undo, &left);
                Some(match (undo, left.as_slice()) {
                    (_, []) => undo.label(),
                    (Undo::Copy { .. }, [(name, why), ..]) => format!("복사본 {}개는 그대로 뒀어요: {name} · {why}", left.len()),
                    (Undo::Move { .. }, [(path, why), ..]) => format!("{}개는 옮긴 자리에 그대로 있어요: {} · {why}", left.len(), util::file_name(path)),
                    (Undo::Group(_), [(name, why), ..]) => format!("{}개는 되돌리지 못했어요: {} · {why}", left.len(), util::file_name(name)),
                    (_, [(name, why)]) => format!("‘{name}’은(는) 휴지통에 그대로 있어요: {why}"),
                    (_, [(_, why), ..]) => format!("{}개는 휴지통에 그대로 있어요: {why}", left.len()),
                })
            }
            (_, Ok(_)) => None,
            (Job::Rename { .. }, Err(Fail::Exists)) => {
                // 상자는 친 그대로 열어 둔다: 다른 이름으로 다시 확정할 수 있다
                if let Editing::Item(e) = &mut self.edit
                    && waiting
                {
                    e.waiting = None;
                }
                Some("같은 이름이 이미 있어요".to_string())
            }
            (job, Err(fail)) => {
                if waiting {
                    self.edit = Editing::None;
                }
                let what = match job {
                    Job::NewFolder { .. } => "폴더를 만들지 못했어요",
                    Job::Rename { .. } => "이름을 바꾸지 못했어요",
                    Job::Trash { .. } => "휴지통으로 옮기지 못했어요",
                    Job::Copy { .. } => "복사하지 못했어요",
                    Job::Move { .. } => "옮기지 못했어요",
                    Job::Undo(_) => "되돌리지 못했어요",
                };
                let why = match (job, fail) {
                    (Job::Undo(Undo::Rename { .. }), Fail::Exists) => "원래 이름을 다른 항목이 쓰고 있어요".to_string(),
                    (_, Fail::Exists) => "같은 이름이 이미 있어요".to_string(),
                    (_, Fail::Changed) => "그 사이 항목이 바뀌었어요".to_string(),
                    (_, Fail::Other(msg)) => msg,
                };
                Some(format!("{what}: {why}"))
            }
        };
        if let Some(note) = note {
            self.flash = Some((note_at, note));
        }
        // 잘 끝났든 아니든 다시 읽는다 (하지 못한 까닭이 낡은 목록일 수 있다)
        self.refresh_changed();
    }

    /// 되돌리기가 끝난 뒤의 뒷정리: 되돌려진 것을 고르거나 고른 것에서 빼고, 그 안을 보던 탭을 맞는 곳으로 옮긴다.
    /// tab은 되돌리기를 낸 탭, left는 되돌리지 못해 그대로 둔 것 (그것들은 건드리지 않는다)
    fn after_undo(&mut self, tab: u64, undo: &Undo, left: &[(String, String)]) {
        match undo {
            Undo::Rename { dir, from, to, file_id } => {
                self.deselect_file(dir, *file_id);
                self.tabs_moved(&util::join(dir, to), &util::join(dir, from));
                if let Some(p) = self.pane_showing(tab, dir) {
                    self.select_after(p, std::slice::from_ref(from));
                }
            }
            Undo::NewFolder { dir, name, file_id } => {
                self.deselect_file(dir, *file_id);
                self.tabs_left(&util::join(dir, name), dir);
            }
            Undo::Trash { dir, items } => {
                // 원래 자리로 돌아온 것들을 고른다
                let back: Vec<String> = items.iter().filter(|t| !left.iter().any(|(name, _)| *name == t.name)).map(|t| t.name.clone()).collect();
                if let Some(p) = self.pane_showing(tab, dir).filter(|_| !back.is_empty()) {
                    self.select_after(p, &back);
                }
            }
            Undo::Copy { dir, items } => {
                // 휴지통으로 간 복사본만: 고른 것에서 빼고, 그 안을 보던 탭은 남아 있는 위 폴더로 올린다
                for (name, file_id) in items {
                    if left.iter().any(|(kept, _)| kept == name) {
                        continue;
                    }
                    self.deselect_file(dir, *file_id);
                    self.tabs_left(&util::join(dir, name), dir);
                }
            }
            Undo::Move { to, items } => {
                // 원래 자리로 돌아간 것만: 옮긴 곳의 고른 것에서 빼고, 그 안을 보던 탭은 원래 경로로 따라가게 한다
                let mut back = Vec::new();
                for (orig, now, file_id) in items {
                    // 되돌리지 못한 것은 원래 전체 경로로 온다 (다른 폴더에서 온 같은 이름의 항목과 헷갈리지 않는다)
                    if left.iter().any(|(path, _)| path == orig) {
                        continue;
                    }
                    self.deselect_file(to, *file_id);
                    self.tabs_moved(&util::join(to, now), orig);
                    back.push(orig);
                }
                // 한 폴더에서 온 것들이면 그 폴더를 보이는 칸에서 고른다
                if let Some(from) = back.first().and_then(|orig| util::parent(orig))
                    && let Some(p) = self.pane_showing(tab, &from)
                {
                    let names: Vec<String> = back.iter().filter(|orig| util::parent(orig).as_deref() == Some(from.as_str())).map(|orig| util::file_name(orig).to_string()).collect();
                    self.select_after(p, &names);
                }
            }
            Undo::Group(list) => {
                for undo in list {
                    self.after_undo(tab, undo, left);
                }
            }
        }
    }

    /// 칸 p를 다음에 읽은 뒤 그 이름들을 고르고 첫 이름이 보이게 한다 (스크롤은 지금 자리에서)
    fn select_after(&mut self, p: usize, names: &[String]) {
        let tab = self.panes[p].tab_mut();
        tab.restore = Some(Restore { selected: names.to_vec(), cursor: names.first().cloned(), scroll: tab.scroll, cursor_index: tab.cursor, reveal: true, ..Default::default() });
    }

    /// 폴더 old가 new가 됐다 (이름 바꾸기와 그 되돌리기): 그 안을 보던 탭과 방문 기록을 새 경로로 고친다.
    /// 고친 탭은 읽던 것을 버린다 (옛 경로로 읽던 결과가 뒤늦게 와서 '열 수 없어요'가 되지 않게). 다시 읽기는 refresh_changed가 한다
    fn tabs_moved(&mut self, old: &str, new: &str) {
        let fix = |loc: &mut Location| {
            let Location::Dir(d) = loc else { return false };
            let Some(rest) = under(d, old) else { return false };
            *d = format!("{new}{rest}");
            true
        };
        for tab in self.panes.iter_mut().flat_map(|pane| pane.tabs.iter_mut()) {
            for loc in tab.back.iter_mut().chain(tab.fwd.iter_mut()) {
                fix(loc);
            }
            if fix(&mut tab.loc) && tab.generation > 0 {
                tab.generation += 1;
            }
        }
    }

    /// 폴더 gone이 휴지통으로 갔다: 그 안을 보던 탭은 남아 있는 위 폴더 up으로 올리고, 방문 기록에서 그 안의 곳들을 뺀다
    fn tabs_left(&mut self, gone: &str, up: &str) {
        let inside = |loc: &Location| loc.dir().is_some_and(|d| under(d, gone).is_some());
        for tab in self.panes.iter_mut().flat_map(|pane| pane.tabs.iter_mut()) {
            tab.back.retain(|loc| !inside(loc));
            tab.fwd.retain(|loc| !inside(loc));
            if inside(&tab.loc) {
                tab.loc = Location::Dir(up.to_string());
                tab.reset_view_state();
                tab.state = LoadState::Loading;
                if tab.generation > 0 {
                    tab.generation += 1;
                }
            }
        }
    }

    /// 폴더 dir 안의 이름이 바뀌었거나 휴지통으로 간 항목을, 그 폴더를 보이는 모든 탭의 고른 것에서 뺀다: 아직 다시 읽지 않은
    /// 목록에 고른 채 남은 옛 이름이, 나중에 그 이름을 갖게 된 다른 항목의 선택으로 되살아나지 않게.
    /// 파일 번호는 한 볼륨 안에서만 서로 다르므로 그 폴더를 보이는 탭에서만 찾는다 (다른 볼륨의 파일이 같은 번호일 수 있다)
    fn deselect_file(&mut self, dir: &str, file_id: u64) {
        if file_id == 0 {
            return;
        }
        for tab in self.panes.iter_mut().flat_map(|pane| pane.tabs.iter_mut()).filter(|tab| tab.loc.dir() == Some(dir)) {
            for e in tab.entries.iter_mut().filter(|e| e.file_id == file_id) {
                e.selected = false;
            }
        }
    }

    /// 파일 작업 뒤: 보이는 칸은 모두 바로 다시 읽고 (폴더 감시의 알림을 기다리지 않는다: 새 폴더의 이름 고치기처럼
    /// 결과가 곧바로 보여야 한다), 보이지 않는 탭은 보이게 될 때 다시 읽게 적어 둔다
    fn refresh_changed(&mut self) {
        let shown = self.layout.count();
        for (p, pane) in self.panes.iter().enumerate() {
            for (i, tab) in pane.tabs.iter().enumerate() {
                if tab.generation > 0 && !(p < shown && i == pane.active) {
                    self.stale.insert(tab.id);
                }
            }
        }
        for p in 0..shown {
            self.refresh(p);
        }
    }

    /// 보이는 칸 가운데 다 읽은 탭을 다시 읽는다 (창이 다시 앞으로 올 때: 다른 앱에서 바뀐 것이 보이게). 읽는 중인 탭은 그대로 둔다
    pub fn refresh_shown(&mut self) {
        for p in 0..self.layout.count() {
            if self.panes[p].tab().state == LoadState::Ready {
                self.refresh(p);
            }
        }
        // 다른 앱에서 고정한 파일을 고치거나 옮겼을 수 있다
        self.pin_check();
    }

    // ---------- 폴더 감시 ----------

    /// 감시할 폴더가 달라졌으면 새 목록을 내준다 (창 쪽이 감시를 그 목록으로 맞춘다): 보이는 칸의 폴더, 겹치면 한 번.
    /// 읽는 중인 폴더도 넣는다 (감시를 먼저 걸어야 읽는 사이의 변화를 놓치지 않는다). 열지 못한 폴더는 뺀다.
    /// 감시를 그만두는 폴더를 보이던 탭은 다음에 보일 때 다시 읽게 적어 둔다 (보이지 않는 동안 바뀐 것은 알 길이 없다)
    pub fn take_watch_dirs(&mut self) -> Option<Vec<String>> {
        let mut want: Vec<String> = Vec::new();
        for p in 0..self.layout.count() {
            let tab = self.panes[p].tab();
            if let Some(dir) = tab.loc.dir().filter(|_| !matches!(tab.state, LoadState::Error(_)))
                && !want.iter().any(|w| w == dir)
            {
                want.push(dir.to_string());
            }
        }
        if want == self.watched {
            return None;
        }
        for tab in self.panes.iter().flat_map(|pane| pane.tabs.iter()) {
            let dropped = tab.loc.dir().is_some_and(|d| self.watched.iter().any(|w| w == d) && !want.iter().any(|w| w == d));
            if dropped && tab.generation > 0 {
                self.stale.insert(tab.id);
            }
        }
        self.watched = want.clone();
        Some(want)
    }

    /// 감시하던 폴더 dir 바로 안이 바뀌었다 (항목이 생겼거나 없어졌거나 이름이 바뀌었다. 그 폴더 자체가 없어진 것도 이리 온다):
    /// 그 폴더를 보이는 탭을 다시 읽는다 (고른 것과 스크롤은 그대로). 보이지 않는 탭은 보이게 될 때 읽는다
    pub fn on_dir_changed(&mut self, dir: &str) {
        for tab in self.panes.iter().flat_map(|pane| pane.tabs.iter()) {
            if tab.generation > 0 && tab.loc.dir() == Some(dir) {
                self.stale.insert(tab.id);
            }
        }
        // 미뤄 둔 읽기 결과가 남아 있으면 함께 본다 (마우스로 다루던 것이 끝났다는 것을 놓쳤어도 여기서 풀린다)
        self.after_pointer();
        // 고정한 파일이 그 폴더에서 이름이 바뀌었거나 바뀌어 쓰였을 수 있다
        self.pin_check();
    }

    /// 보이는 칸의 탭 가운데 다시 읽기로 해 둔 것을 읽는다. 다음 때는 미루고, 그 일이 끝날 때 다시 부른다:
    /// - 읽는 중일 때: 잇달아 바뀌는 폴더에서 읽던 것을 버리고 새로 읽기만 되풀이하지 않게, 다 읽은 뒤 한 번 더 읽는다
    /// - 그 탭의 줄을 마우스로 다루는 중일 때 (pointer_on): 손 밑의 줄이 바뀌지 않게
    /// - 복사·옮기기 묶음이 그 폴더에 넣고 있을 때: 복사하는 중인 임시 이름이 목록에 보이지 않게 (묶음이 끝나면 읽는다)
    fn reload_stale(&mut self) {
        for p in 0..self.layout.count() {
            let tab = self.panes[p].tab();
            let (id, ready) = (tab.id, tab.state == LoadState::Ready);
            let filling = self.batch.as_ref().is_some_and(|b| tab.loc.dir() == Some(b.dir.as_str()));
            if ready && !filling && !self.reading.contains(&id) && !self.pointer_on(id) && self.stale.remove(&id) {
                self.refresh(p);
            }
        }
    }

    /// 그 탭의 줄을 마우스로 다루는 중인지: 그 탭이 보이는 칸에서 줄을 누르고 있거나, 끌려 온 것이 그 칸의 목록 위에 있다.
    /// 누른 줄과 놓일 줄은 줄 번호로 기억하므로, 그동안에는 그 목록을 바꾸지 않는다
    fn pointer_on(&self, tab_id: u64) -> bool {
        let shows = |p: usize| p < self.layout.count() && self.panes[p].tab().id == tab_id;
        self.press.as_ref().is_some_and(|press| shows(press.pane)) || matches!(self.drop, Some(DropAt::Pane(p) | DropAt::Row(p, _)) if shows(p))
    }

    /// 마우스로 다루던 것이 끝났거나 다른 칸으로 옮겨 갔다: 그동안 미뤄 둔 읽기 결과를 적용하고, 미뤄 둔 다시 읽기를 한다
    fn after_pointer(&mut self) {
        // 아직 다루는 중인 탭의 것은 on_listed가 다시 미뤄 둔다
        for (tab, generation, result) in std::mem::take(&mut self.held) {
            self.on_listed(tab, generation, result);
        }
        self.reload_stale();
    }

    // ---------- 세션 ----------

    /// 끝낼 때 저장할 화면 상태 (맥이 쓰지 않는 값은 기본값)
    pub fn to_session(&self) -> Session {
        let layout = Layout::ALL.iter().position(|l| *l == self.layout).unwrap_or(1) as u8;
        Session {
            window: None,
            maximized: false,
            layout,
            split: self.split,
            active: self.active,
            show_hidden: self.show_hidden,
            preview: self.preview.is_some(),
            auto_preview: self.auto_preview,
            new_tab: self.new_tab_pref.clone(),
            recent: self.recent.clone(),
            side: "main".to_string(),
            side_w: session::SIDE_W_DEFAULT,
            pin: self.preview_pin().map(|(p, _)| p),
            pin_id: self.preview_pin().and_then(|(_, id)| id),
            ws: self.workspaces.current_name().map(str::to_string),
            query: String::new(),
            scope_here: false,
            folded: Vec::new(),
            look: Look { theme: self.theme_pref.clone(), ..Look::default() },
            panes: self
                .panes
                .iter()
                .map(|pane| PaneState {
                    tabs: pane.tabs.iter().map(|t| t.loc.to_session()).collect(),
                    active: pane.active,
                    cols: pane.cols,
                    sort_key: pane.sort.key.to_u8(),
                    sort_desc: pane.sort.desc,
                    view: pane.view.code(),
                })
                .collect(),
        }
    }

    /// 저장한 화면 상태로 연다. 없어진 폴더(와 맥에서 쓰지 않는 '내 PC')는 home으로 바꾼다. 시작 화면 탭은 그대로 연다
    pub fn restore(&mut self, s: &Session, home: &str) {
        self.requests.clear();
        // 탭을 새로 만든다: 옛 탭에 걸어 둔 표시는 버린다
        self.stale.clear();
        self.reading.clear();
        self.rereading.clear();
        self.held.clear();
        self.name_next = None;
        self.layout = Layout::ALL[(s.layout as usize).min(4)];
        self.split = (s.split.0.clamp(0.15, 0.85), s.split.1.clamp(0.15, 0.85));
        self.show_hidden = s.show_hidden;
        self.auto_preview = s.auto_preview;
        self.home = home.to_string();
        for (p, ps) in s.panes.iter().take(self.panes.len()).enumerate() {
            let mut tabs = Vec::new();
            for t in &ps.tabs {
                // 폴더가 아직 있는지는 여기서 보지 않는다 (꺼진 네트워크 폴더면 창이 뜨기 전에 멈춘다).
                // 처음 읽기가 실패하면 on_listed가 home으로 바꾼다
                let dir = match Location::from_session(t) {
                    Location::Start => {
                        tabs.push(self.new_tab_at(Location::Start));
                        continue;
                    }
                    Location::Dir(d) => d,
                    Location::Computer => home.to_string(),
                };
                let tab = self.new_tab_at(Location::Dir(dir.clone()));
                self.restored.insert(tab.id, dir);
                tabs.push(tab);
            }
            if !tabs.is_empty() {
                let active = ps.active.min(tabs.len() - 1);
                // 열 폭은 Windows와 같은 범위로 (저장한 파일을 손으로 고쳐도 열이 사라지거나 넘치지 않게)
                self.panes[p] = Pane {
                    tabs,
                    active,
                    view: ViewMode::from_code(ps.view),
                    sort: Sort { key: SortKey::from_u8(ps.sort_key), desc: ps.sort_desc },
                    cols: ps.cols.map(|c| c.clamp(COL_MIN, COL_MAX)),
                };
            }
        }
        self.active = s.active.min(self.layout.count() - 1);
        for p in 0..self.layout.count() {
            self.load_if_new(p);
        }
        // 오른쪽 미리 보기 창(열림, 고정한 파일)은 작업 공간마다 기억한다 (Windows와 같다)
        self.restore_preview(s.preview, s.pin.as_deref(), s.pin_id);
    }

    // ---------- 작업 공간 ----------

    /// 저장해 둔 작업 공간을 넣는다 (화면은 바꾸지 않는다). 하나도 없으면 지금 화면으로 '기본' 공간을 만든다
    pub fn set_workspaces(&mut self, ws: Workspaces) {
        self.workspaces = ws;
        if self.workspaces.list.is_empty() {
            let state = self.to_session();
            self.workspaces.ensure_one(|| state);
        }
        if self.workspaces.current.is_none_or(|c| c >= self.workspaces.list.len()) {
            self.workspaces.current = Some(0);
        }
        self.end_space_rename();
        self.side_scroll = 0.0;
        self.retheme();
    }

    /// 시작할 때: 저장해 둔 작업 공간과 지난번 화면(세션)으로 연다.
    /// 세션이 지금 공간의 화면이면 그 화면(마지막으로 본 모습)으로, 아니면 그 공간에 저장된 화면으로 연다.
    /// 작업 공간이 없으면(처음 실행했거나 작업 공간이 없던 시제품을 쓰던 경우) 지난번 화면이 '기본' 공간이 된다
    pub fn open_saved(&mut self, ws: Workspaces, session: Option<&Session>, home: &str) {
        // 모양·새 탭 설정·최근 위치는 앱 전체의 것이라 작업 공간을 오갈 때가 아니라 여기서만 읽는다
        // (오른쪽 미리 보기 창은 작업 공간마다라 restore가 읽는다)
        if let Some(s) = session {
            self.set_theme_pref(&s.look.theme);
            self.set_new_tab_start(s.new_tab != session::NEW_TAB_SAME);
            self.recent = s.recent.iter().take(sakadu::ui::start::RECENT_KEEP).cloned().collect();
        }
        if ws.list.is_empty() {
            if let Some(s) = session {
                self.restore(s, home);
            }
            return self.set_workspaces(Workspaces::default());
        }
        self.set_workspaces(ws);
        match session {
            Some(s) if s.ws.as_deref() == self.workspaces.current_name() => self.restore(s, home),
            _ => {
                if let Some(state) = self.workspaces.current().map(|w| w.state.clone()) {
                    self.restore(&state, home);
                }
                // 숨김 항목 보기와 자동 미리 보기는 작업 공간과 상관없다
                if let Some(s) = session {
                    self.show_hidden = s.show_hidden;
                    self.auto_preview = s.auto_preview;
                }
            }
        }
    }

    /// 맥 설정이 바뀌었다 (밝은·어두운 모양, 강조 색): 테마를 다시 정한다
    pub fn set_system(&mut self, dark: bool, accent: Option<Color>) {
        self.dark = dark;
        self.system_accent = accent;
        self.retheme();
    }

    /// 테마를 정한다: 강조 색은 지금 공간의 색, 색을 정하지 않은 공간은 맥 설정의 강조 색
    fn retheme(&mut self) {
        let sys = Sys { dark: self.dark, accent: self.system_accent };
        let key = self.workspaces.current().map(|w| w.color.as_str()).filter(|c| !c.is_empty()).unwrap_or("system");
        self.theme = Theme::resolve(if self.dark { "dark" } else { "light" }, key, &sys);
    }

    /// 레일에 보일 공간의 색: 그 공간의 색. 정하지 않았으면 맥 설정의 강조 색 (읽지 못했으면 기본 파랑)
    fn space_color(&self, w: &Workspace) -> Color {
        let dark = self.theme.dark;
        let sys = Sys { dark, accent: self.system_accent };
        let key = if w.color.is_empty() { "system" } else { w.color.as_str() };
        palette::accent_color(key, dark, &sys).unwrap_or(if dark { Theme::DARK.accent } else { Theme::LIGHT.accent })
    }

    /// 지금 화면을 지금 공간에 적어 둔다
    fn sync_workspace(&mut self) {
        let state = self.to_session();
        if let Some(w) = self.workspaces.current_mut() {
            w.state = state;
        }
    }

    /// 지금 공간에 지금 화면을 적고 작업 공간을 파일에 저장한다 (시험·그림에서는 저장하지 않는다). 저장하지 못했으면 그 오류
    fn try_save_workspaces(&mut self) -> std::io::Result<()> {
        self.sync_workspace();
        if self.persist { workspaces::save(&self.workspaces) } else { Ok(()) }
    }

    /// 지금 공간에 지금 화면을 적고 작업 공간을 파일에 저장한다 (저장하지 못하면 기록만 남긴다)
    pub fn save_workspaces(&mut self) {
        if let Err(e) = self.try_save_workspaces() {
            sakadu::diag::log(|| format!("workspaces: {e}"));
        }
    }

    /// 지금 공간에 저장된 화면(칸 나누기·칸·탭)을 열고, 색을 그 공간 색으로 바꾼다.
    /// 숨김 항목 보기와 자동 미리 보기는 작업 공간과 상관없어 그대로 둔다
    fn open_current_workspace(&mut self) {
        let Some(state) = self.workspaces.current().map(|w| w.state.clone()) else { return };
        let (home, hidden, auto_preview) = (self.home.clone(), self.show_hidden, self.auto_preview);
        // 떠나는 화면의 탭은 없어진다: 그 탭들에 걸어 둔 표시와 하던 입력·메뉴·끌기를 그만둔다
        self.restored.clear();
        self.restore(&state, &home);
        self.show_hidden = hidden;
        self.auto_preview = auto_preview;
        self.edit = Editing::None;
        self.flash = None;
        self.menu = None;
        self.cmd = None;
        self.start_keys = false;
        self.drag = None;
        self.col_drag = None;
        self.side_press = None;
        self.side_scroll = 0.0;
        self.retheme();
    }

    /// 공간 i로 바꾼다. 지금 공간에는 지금 화면을 적어 두고, 고른 공간의 화면을 연다
    pub fn switch_workspace(&mut self, i: usize) {
        if self.workspaces.current == Some(i) || i >= self.workspaces.list.len() {
            return;
        }
        self.sync_workspace();
        self.workspaces.current = Some(i);
        self.open_current_workspace();
        self.save_workspaces();
    }

    /// 지금 화면으로 새 공간을 만들어 그리로 옮기고, 이름을 바로 고칠 수 있게 한다.
    /// 이름은 지금 폴더 이름(겹치면 번호를 붙인다), 색은 아직 쓰지 않은 색
    pub fn new_workspace(&mut self) {
        self.sync_workspace();
        let title = workspaces::clean_name(&self.panes[self.active].tab().loc.title());
        let base = if title.is_empty() { format!("작업 공간 {}", self.workspaces.list.len() + 1) } else { title };
        let mut ws = Workspace::new(self.workspaces.unique_name(&base, None), self.to_session());
        // 색을 정하지 않은 공간은 맥 설정의 강조 색을 쓴다
        ws.color = self.workspaces.next_color("system").to_string();
        self.workspaces.list.push(ws);
        self.workspaces.current = Some(self.workspaces.list.len() - 1);
        self.side_scroll = 0.0;
        self.retheme();
        self.save_workspaces();
        self.begin_rename();
    }

    /// 공간 i를 지운다 (마지막 하나는 지우지 않는다). 지금 공간을 지웠으면 그 자리의 다음 공간을 연다
    pub fn delete_workspace(&mut self, i: usize) {
        if self.workspaces.remove(i) {
            self.open_current_workspace();
        }
        self.end_space_rename();
        self.save_workspaces();
    }

    /// 공간 i의 색을 바꾼다 (workspaces::COLORS의 이름). 지금 공간이면 화면 색도 따라 바뀐다
    pub fn set_workspace_color(&mut self, i: usize, color: &str) {
        if !workspaces::COLORS.contains(&color) {
            return;
        }
        let Some(w) = self.workspaces.list.get_mut(i) else { return };
        w.color = color.to_string();
        self.retheme();
        self.save_workspaces();
    }

    /// 지금 공간의 이름을 고치기 시작한다 (창 쪽 입력 상자가 이름 자리에 뜬다. 확정은 commit_address, 취소는 cancel_address)
    pub fn begin_rename(&mut self) {
        self.edit = Editing::Space;
    }

    /// 공간 이름을 고치던 중이면 그만둔다 (다른 입력은 그대로 둔다)
    fn end_space_rename(&mut self) {
        if self.edit == Editing::Space {
            self.edit = Editing::None;
        }
    }

    /// 고친 이름을 지금 공간에 적는다. 비었거나 그대로면 두고, 다른 공간과 겹치면 번호를 붙인다
    fn commit_rename(&mut self, text: &str) {
        let name = workspaces::clean_name(text);
        let Some(c) = self.workspaces.current else { return };
        if name.is_empty() || self.workspaces.list[c].name == name {
            return;
        }
        self.workspaces.list[c].name = self.workspaces.unique_name(&name, Some(c));
        self.save_workspaces();
    }

    // ---------- 이동 ----------

    /// 칸 p를 loc으로 옮긴다 (기록을 남기고, 폴더면 최근 위치에 둔다). select가 있으면 읽은 뒤 그 이름을 고른다
    pub fn navigate(&mut self, p: usize, loc: Location, select: Option<String>) {
        self.note_recent(&loc);
        self.start_keys = false;
        let tab = self.panes[p].tab_mut();
        tab.nav_push(loc);
        tab.reset_view_state();
        tab.state = LoadState::Loading;
        tab.restore = select.as_deref().map(Restore::select);
        self.load(p);
    }

    pub fn go_up(&mut self, p: usize) {
        let Some(dir) = self.panes[p].tab().loc.dir().map(str::to_string) else { return };
        if let Some(parent) = util::parent(&dir) {
            self.navigate(p, Location::Dir(parent), Some(util::file_name(&dir).to_string()));
        }
    }

    pub fn go_back(&mut self, p: usize) {
        let from = self.panes[p].tab().loc.clone();
        if self.panes[p].tab_mut().nav_back() {
            self.after_history(p, from);
        }
    }

    pub fn go_forward(&mut self, p: usize) {
        let from = self.panes[p].tab().loc.clone();
        if self.panes[p].tab_mut().nav_forward() {
            self.after_history(p, from);
        }
    }

    /// 뒤로·앞으로 뒤: 상위 폴더로 돌아왔으면 원래 있던 하위 폴더를 고른다. 돌아온 폴더는 최근 위치에 둔다
    fn after_history(&mut self, p: usize, from: Location) {
        let loc = self.panes[p].tab().loc.clone();
        self.note_recent(&loc);
        self.start_keys = false;
        let tab = self.panes[p].tab_mut();
        let select = child_toward(&tab.loc, &from);
        tab.reset_view_state();
        tab.state = LoadState::Loading;
        tab.restore = select.as_deref().map(Restore::select);
        self.load(p);
    }

    /// 줄 vi 열기: 폴더면 그리로, 파일이면 macOS '열기'
    pub fn open_row(&mut self, p: usize, vi: usize) {
        let tab = self.panes[p].tab();
        let Some(e) = tab.entry_at(vi) else { return };
        let Some(path) = tab.path_of(e) else { return };
        if e.is_dir() {
            self.navigate(p, Location::Dir(path), None);
        } else {
            self.open_files.push(path);
        }
    }

    /// 열기 키(⌘↓·⌘O): 커서가 폴더면 그리로, 아니면 고른 파일을 연다
    pub fn open_selected(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        let (cursor, folder) = (tab.cursor, tab.cursor_entry().is_some_and(Entry::is_dir));
        if folder {
            return self.open_row(p, cursor);
        }
        let files: Vec<String> = tab.selected_paths();
        self.open_files.extend(files);
    }

    /// 칸 p의 지금 탭을 다시 읽는다 (같은 폴더). 고른 것·커서·스크롤은 다 읽었을 때의 것을 그대로 둔다 (on_listed)
    pub fn refresh(&mut self, p: usize) {
        let before = self.panes[p].tab().generation;
        self.load(p);
        let tab = self.panes[p].tab();
        let (id, now) = (tab.id, tab.generation);
        self.rereading.insert(id);
        // 같은 폴더를 다시 읽는 것은 다른 곳으로 옮긴 것이 아니다: 확인 중인 경로 입력이 버려지지 않게 기다리는 번호를 따라 올린다
        if self.probe_at[p] == (id, before) {
            self.probe_at[p] = (id, now);
        }
    }

    // ---------- 탭·칸 ----------

    /// 새 탭 (지금 탭 바로 뒤): 설정에 따라 시작 화면 또는 지금 폴더
    pub fn new_tab(&mut self, p: usize) {
        let loc = if self.new_tab_starts() { Location::Start } else { self.panes[p].tab().loc.clone() };
        self.open_tab(p, loc, None);
    }

    /// loc을 새 탭으로 연다 (칸 p의 지금 탭 바로 뒤, 그 탭으로 옮겨 간다). 폴더면 최근 위치에 둔다. select가 있으면 읽은 뒤 그 이름을 고른다
    pub(super) fn open_tab(&mut self, p: usize, loc: Location, select: Option<String>) {
        self.note_recent(&loc);
        self.start_keys = false;
        let mut tab = self.new_tab_at(loc);
        tab.restore = select.as_deref().map(Restore::select);
        let pane = &mut self.panes[p];
        pane.active += 1;
        pane.tabs.insert(pane.active, tab);
        self.load(p);
    }

    /// 마지막 탭은 닫지 않는다
    pub fn close_tab(&mut self, p: usize, i: usize) {
        let pane = &mut self.panes[p];
        if pane.tabs.len() <= 1 || i >= pane.tabs.len() {
            return;
        }
        let gone = pane.tabs.remove(i);
        self.stale.remove(&gone.id);
        self.reading.remove(&gone.id);
        self.rereading.remove(&gone.id);
        if pane.active > i || pane.active >= pane.tabs.len() {
            pane.active = pane.active.saturating_sub(1);
        }
        // 다시 열 수 있게 남겨 둔다 (Windows와 같이 스무 개까지)
        self.closed.push((p, gone.loc));
        if self.closed.len() > 20 {
            self.closed.remove(0);
        }
        self.load_if_new(p);
    }

    /// 가장 나중에 닫은 탭을 그 칸에 다시 연다 (그 칸이 지금 보이지 않으면 지금 칸에). 닫은 탭이 없으면 false
    pub fn reopen_tab(&mut self) -> bool {
        let Some((p, loc)) = self.closed.pop() else { return false };
        let p = if p < self.layout.count() { p } else { self.active };
        let tab = self.new_tab_at(loc);
        let pane = &mut self.panes[p];
        pane.active += 1;
        pane.tabs.insert(pane.active, tab);
        self.activate_pane(p);
        self.load(p);
        true
    }

    pub fn activate_tab(&mut self, p: usize, i: usize) {
        if i < self.panes[p].tabs.len() {
            self.panes[p].active = i;
            self.start_keys = false;
            self.load_if_new(p);
        }
    }

    pub fn cycle_tab(&mut self, p: usize, back: bool) {
        let n = self.panes[p].tabs.len();
        let i = self.panes[p].active;
        self.activate_tab(p, if back { (i + n - 1) % n } else { (i + 1) % n });
    }

    /// 한 번도 읽지 않은 탭이면 읽고, 파일 작업 뒤 다시 읽기로 해 둔 탭이면 다시 읽는다
    fn load_if_new(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        if tab.generation == 0 {
            self.load(p);
        } else if self.stale.remove(&tab.id) {
            self.refresh(p);
        }
    }

    pub fn activate_pane(&mut self, p: usize) {
        if p < self.layout.count() {
            self.active = p;
        }
    }

    pub fn cycle_pane(&mut self) {
        self.active = (self.active + 1) % self.layout.count();
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
        self.active = self.active.min(layout.count() - 1);
        for p in 0..layout.count() {
            self.load_if_new(p);
            self.clamp_scroll(p);
        }
    }

    /// 설정 창에 보이는 값
    pub fn prefs(&self) -> Prefs {
        let (search_on, icloud, crawling, denied) = self.find_prefs();
        Prefs { hidden: self.show_hidden, auto_preview: self.auto_preview, search_on, icloud, crawling, denied, theme: self.theme_pref.clone(), new_tab_start: self.new_tab_starts() }
    }

    /// 설정 창의 '숨김 항목 보기' (⌘⇧.와 같은 값)
    pub fn set_hidden(&mut self, on: bool) {
        if self.show_hidden != on {
            self.toggle_hidden();
        }
    }

    /// 설정 창의 '자동 미리 보기' (도구 막대의 눈 단추와 같은 값)
    pub fn set_auto_preview(&mut self, on: bool) {
        if self.auto_preview != on {
            self.toggle_auto_preview();
        }
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        let hidden = self.show_hidden;
        for p in 0..self.panes.len() {
            let sort = self.panes[p].sort;
            for tab in &mut self.panes[p].tabs {
                tab.rebuild_view(sort, hidden);
            }
            self.clamp_scroll(p);
        }
    }

    /// 칸 p의 보기 방식을 바꾼다. 커서 줄이 계속 보이게 스크롤을 맞춘다
    pub fn set_view(&mut self, p: usize, view: ViewMode) {
        self.panes[p].view = view;
        self.clamp_scroll(p);
        self.ensure_visible(p);
    }

    /// 칸 p를 key로 정렬한다 (머리글이나 정렬 칩을 눌렀다). 같은 열을 다시 누르면 순서를 뒤집는다.
    /// 날짜·크기는 큰 값(최신·큰 파일)부터 보는 경우가 많아 내림차순으로 시작한다 (Windows와 같다)
    pub fn toggle_sort(&mut self, p: usize, key: SortKey) {
        let hidden = self.show_hidden;
        let pane = &mut self.panes[p];
        pane.sort = if pane.sort.key == key { Sort { key, desc: !pane.sort.desc } } else { Sort { key, desc: matches!(key, SortKey::Modified | SortKey::Size) } };
        let sort = pane.sort;
        for tab in &mut pane.tabs {
            tab.rebuild_view(sort, hidden);
        }
        self.clamp_scroll(p);
        self.ensure_visible(p);
    }

    /// 날짜 열의 내용 폭 (ui::date_col_width). 글꼴이 바뀌지 않으므로 한 번 재어 둔다
    fn date_col_w(&self, g: &Gfx) -> f32 {
        if let Some(w) = self.date_w.get() {
            return w;
        }
        let w = date_col_width(g, true);
        self.date_w.set(Some(w));
        w
    }

    /// 칸 p의 열 폭: '자동' 날짜 열(pane::DATE_AUTO)은 내용 폭으로. 그리기·누르기·열 끌기·이름 상자가 모두 이것을 쓴다.
    /// 아직 한 번도 재지 않았으면(그리기 전) 기본 폭
    fn cols(&self, p: usize) -> [f32; 3] {
        resolve_cols(self.panes[p].cols, self.date_w.get().unwrap_or(DATE_AUTO))
    }

    /// 메뉴를 연다: 붙일 자리와 줄을 정한다. 줄이 없거나 붙일 자리가 없으면 열지 않는다
    fn open_menu(&mut self, g: &Gfx, kind: MenuKind) {
        let Some(anchor) = self.menu_anchor(kind) else { return };
        let lines = self.menu_lines(kind);
        if lines.is_empty() {
            return;
        }
        let w = menu_width(g, &lines, (self.w - 2.0 * MARGIN).clamp(1.0, 420.0));
        self.menu = Some(OpenMenu { kind, anchor, lines, w, sel: None, scroll: 0.0 });
    }

    /// 그림용: 메뉴를 열고 sel번째 줄을 가리킨 모습. 목록 메뉴는 왼쪽 칸 셋째 줄을, 공간 메뉴는 그 공간 아이콘을 오른쪽 단추로 누른 자리에 연다
    pub fn open_menu_for_render(&mut self, g: &Gfx, kind: MenuKind, sel: usize) {
        let at = match kind {
            MenuKind::Rows => self.list_rect(0).map(|l| (l.l + 80.0, l.t + 2.0 * ROW_H + ROW_H / 2.0)),
            MenuKind::Space(i) => self.rail().spaces.iter().find(|(s, _)| *s == i).map(|(_, r)| ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0)),
            _ => None,
        };
        match at {
            Some((x, y)) => {
                self.right_down(g, x, y);
            }
            None => self.open_menu(g, kind),
        }
        if let Some(m) = self.menu.as_mut() {
            m.sel = Some(sel);
        }
    }

    /// 열린 메뉴의 종류와 붙인 자리
    pub fn menu_shown(&self) -> Option<(MenuKind, R)> {
        self.menu.as_ref().map(|m| (m.kind, m.anchor))
    }

    pub fn close_menu(&mut self) {
        self.menu = None;
    }

    /// 메뉴를 붙일 자리: 단추나 줄, 목록 메뉴는 누른 자리
    fn menu_anchor(&self, kind: MenuKind) -> Option<R> {
        let anchor = match kind {
            MenuKind::Layout => self.toolbar().layout,
            // 보기 방식은 지금 칸의 경로 줄 단추 아래 (좁아서 단추가 없으면 경로 줄 오른쪽 끝 아래)
            MenuKind::View => match self.view_button(self.active) {
                b if b.w() > 0.0 => b,
                _ => {
                    let g = geom(self.pane_rects().into_iter().find(|(p, _)| *p == self.active)?.1);
                    R::new(g.star.l, g.star.t, g.star.r.max(g.crumb.r), g.star.b)
                }
            },
            MenuKind::Space(i) => self.rail().spaces.iter().find(|(s, _)| *s == i)?.1,
            MenuKind::Spaces => self.rail().add,
            // 사이드바 줄의 메뉴는 그 줄에 붙인다
            MenuKind::Pin(_) | MenuKind::Fav(_) | MenuKind::Group(_) => {
                let rows = self.side_rows();
                let i = (0..rows.len()).find(|i| side_menu(&rows, *i) == Some(kind))?;
                self.side_rect(&rows[i])
            }
            // 그룹을 지울지 묻는 메뉴는 그 그룹 머리에 붙인다
            MenuKind::GroupDelete(g) => return self.menu_anchor(MenuKind::Group(g)),
            // 목록 메뉴는 누른 자리에 연다 (메뉴는 붙인 자리의 아래 끝에서 4 내려 놓는다)
            MenuKind::Rows | MenuKind::Blank => {
                let (x, y) = self.list_target.as_ref()?.at;
                R::new(x, y - 4.0, x, y - 4.0)
            }
        };
        Some(anchor)
    }

    /// 메뉴의 줄 (이름, 지금 것 체크, 구분선, 그림): 칸 나누기와 보기 방식은 그 모양, 공간의 색은 색 동그라미
    fn menu_lines(&self, kind: MenuKind) -> Vec<MenuLine> {
        let brk = self.menu_break(kind);
        let icon = |i: usize| match kind {
            MenuKind::Layout => Layout::ALL.get(i).map_or(MenuIcon::None, |l| MenuIcon::Layout(*l)),
            MenuKind::View => ViewMode::ALL.get(i).map_or(MenuIcon::None, |v| MenuIcon::View(*v)),
            MenuKind::Space(_) => workspaces::COLORS.get(i).map_or(MenuIcon::None, |key| MenuIcon::Swatch(self.key_color(key))),
            _ => MenuIcon::None,
        };
        self.menu_items(kind).into_iter().enumerate().map(|(i, (label, on))| MenuLine::new(label).checked(on).icon(icon(i)).sep(brk == Some(i))).collect()
    }

    /// 공간 색 이름(workspaces::COLORS)의 색
    fn key_color(&self, key: &str) -> Color {
        let dark = self.theme.dark;
        let sys = Sys { dark, accent: self.system_accent };
        palette::accent_color(key, dark, &sys).unwrap_or(self.theme.accent)
    }

    /// 열린 메뉴의 자리 (창 안쪽에 들어가게)
    fn menu_rect(&self) -> Option<R> {
        let m = self.menu.as_ref()?;
        let bounds = R::new(MARGIN, MARGIN, (self.w - MARGIN).max(MARGIN + 1.0), (self.h - MARGIN).max(MARGIN + 1.0));
        Some(menu_place(m.anchor, m.w, menu_height(&m.lines), bounds))
    }

    /// 열린 메뉴에서 줄 i를 골랐다: 메뉴를 닫고 그 줄의 일을 한다
    fn pick_open_menu(&mut self, i: usize) {
        if let Some(m) = self.menu.take() {
            self.pick_menu(m.kind, i);
        }
    }

    /// 메뉴가 열려 있는 동안의 키: ↑↓ 줄 옮기기, Home·End, Return·스페이스 고르기, Esc·←→·Tab 닫기. 그 밖의 키는 삼킨다
    pub fn menu_key(&mut self, key: Key) {
        let rect = self.menu_rect();
        let Some(m) = self.menu.as_mut() else { return };
        match key {
            Key::Up | Key::Down => m.sel = menu_step(&m.lines, m.sel, key == Key::Down),
            Key::Home => m.sel = menu_step(&m.lines, None, true),
            Key::End => m.sel = menu_step(&m.lines, None, false),
            Key::Return | Key::Char(' ') => {
                if let Some(i) = m.sel {
                    self.pick_open_menu(i);
                }
                return;
            }
            Key::Esc | Key::Left | Key::Right | Key::Tab => {
                self.close_menu();
                return;
            }
            _ => return,
        }
        if let (Some(r), Some(i)) = (rect, m.sel) {
            m.scroll = menu_reveal(r, &m.lines, m.scroll, i);
        }
    }

    /// 메뉴의 줄: (이름, 지금 것인지). 숨김 항목은 보기 메뉴가 아니라 도구 막대의 눈으로 켜고 끈다.
    /// 공간 메뉴는 색 여섯 가지 다음에 이름 바꾸기(지금 공간만), 지우기(공간이 둘 이상일 때만).
    /// 모든 작업 공간 메뉴는 공간 이름들.
    /// 고정한 줄의 메뉴는 위로·아래로 이동(갈 수 있는 쪽만), 모든 공간에 두기, 고정 풀기.
    /// 즐겨찾기 항목의 메뉴는 이 작업 공간에만 두기, 모든 공간에서 빼기, 그리고 그룹으로 옮기는 줄들 (그룹마다, 지금 그룹에 체크.
    /// 그룹 안 항목이면 그룹에서 빼기. 맨 끝은 새 그룹을 만들어 넣기). 앱이 그리는 메뉴에는 하위 메뉴가 없어 Windows의 '그룹으로 이동'을 펼쳐 둔다.
    /// 그룹 머리의 메뉴는 이름 바꾸기, 그룹 풀기, 그룹과 안의 즐겨찾기 빼기 (즐겨찾기가 들어 있으면 한 번 더 묻는다)
    pub fn menu_items(&self, kind: MenuKind) -> Vec<(String, bool)> {
        use sakadu::keys::{Action, name};
        match kind {
            MenuKind::Layout => {
                let names = [Action::Layout1, Action::Layout2, Action::Layout3, Action::Layout4, Action::Layout5];
                Layout::ALL.iter().zip(names).map(|(l, a)| (name(a).to_string(), *l == self.layout)).collect()
            }
            MenuKind::View => {
                let current = self.panes[self.active].view;
                ViewMode::ALL.iter().map(|v| (v.label().to_string(), *v == current)).collect()
            }
            MenuKind::Space(i) => {
                let Some(w) = self.workspaces.list.get(i) else { return Vec::new() };
                let color_name = |key: &str| palette::ACCENTS.iter().find(|a| a.0 == key).map_or(key, |a| a.1).to_string();
                let mut v: Vec<_> = workspaces::COLORS.iter().map(|key| (color_name(key), w.color == *key)).collect();
                // 이름은 사이드바 맨 위(지금 공간의 이름 자리)에서 고친다
                if self.workspaces.current == Some(i) {
                    v.push(("이름 바꾸기".to_string(), false));
                }
                if self.workspaces.list.len() > 1 {
                    v.push(("이 작업 공간 지우기".to_string(), false));
                }
                v
            }
            MenuKind::Spaces => self.workspaces.list.iter().enumerate().map(|(i, w)| (w.name.clone(), self.workspaces.current == Some(i))).collect(),
            MenuKind::Pin(p) => self.pin_menu(p).into_iter().map(|act| (act.label().to_string(), false)).collect(),
            MenuKind::Fav(row) => {
                let current = match self.favs.rows().get(row) {
                    Some(Row::Item { group, .. }) => *group,
                    _ => None,
                };
                let line = |act: FavAct| match act {
                    FavAct::ToPins => ("이 작업 공간에만 두기".to_string(), false),
                    FavAct::Remove => ("모든 공간에서 빼기".to_string(), false),
                    FavAct::ToGroup(Some(g)) => (format!("‘{}’ 그룹으로 이동", self.favs.groups.get(g).map_or("", |grp| grp.name.as_str())), current == Some(g)),
                    FavAct::ToGroup(None) => ("그룹에서 빼기".to_string(), false),
                    FavAct::NewGroup => ("새 그룹을 만들어 넣기".to_string(), false),
                };
                self.fav_menu(row).into_iter().map(line).collect()
            }
            MenuKind::Group(g) => match self.favs.groups.get(g) {
                Some(grp) => {
                    // 즐겨찾기가 든 그룹은 한 번 더 묻는다 (…)
                    let delete = if grp.items.is_empty() { "그룹과 안의 즐겨찾기 빼기" } else { "그룹과 안의 즐겨찾기 빼기…" };
                    ["이름 바꾸기", "그룹 풀기 (안의 즐겨찾기는 남김)", delete].map(|s| (s.to_string(), false)).to_vec()
                }
                None => Vec::new(),
            },
            MenuKind::GroupDelete(g) => match self.favs.groups.get(g) {
                Some(grp) => vec![(format!("‘{}’ 그룹과 즐겨찾기 {}개 빼기", grp.name, grp.items.len()), false), ("그만두기".to_string(), false)],
                None => Vec::new(),
            },
            MenuKind::Rows => self.row_menu().into_iter().map(|act| (act.label().to_string(), false)).collect(),
            MenuKind::Blank => vec![("새 폴더".to_string(), false), ("붙여넣기".to_string(), false), ("새로 고침".to_string(), false)],
        }
    }

    /// 목록 메뉴(고른 항목들)의 줄: 열기, Finder에서 보기, 이름 바꾸기(하나를 골랐을 때만), 잘라내기, 복사, 복제, 휴지통으로 이동
    fn row_menu(&self) -> Vec<RowAct> {
        match self.list_target.as_ref().map_or(0, |t| t.items.len()) {
            0 => Vec::new(),
            1 => vec![RowAct::Open, RowAct::Reveal, RowAct::Rename, RowAct::Cut, RowAct::Copy, RowAct::Duplicate, RowAct::Trash],
            _ => vec![RowAct::Open, RowAct::Reveal, RowAct::Cut, RowAct::Copy, RowAct::Duplicate, RowAct::Trash],
        }
    }

    /// 고정한 줄 p의 메뉴에 넣을 줄: 맨 위 줄에는 위로, 맨 아래 줄에는 아래로가 없다. 그런 줄이 없으면 비어 있다
    fn pin_menu(&self, p: usize) -> Vec<PinAct> {
        let n = self.pins().len();
        if p >= n {
            return Vec::new();
        }
        let mut v = Vec::new();
        if p > 0 {
            v.push(PinAct::Up);
        }
        if p + 1 < n {
            v.push(PinAct::Down);
        }
        v.extend([PinAct::ToCommon, PinAct::Unpin]);
        v
    }

    /// 즐겨찾기 줄 row(Favorites::rows의 번호)의 메뉴에 넣을 줄: 항목이 아니면 비어 있다.
    /// 그룹으로 옮기는 줄은 그룹이 있을 때만, 그룹에서 빼기는 그룹 안 항목일 때만 있다
    fn fav_menu(&self, row: usize) -> Vec<FavAct> {
        let Some(Row::Item { group, .. }) = self.favs.rows().get(row).copied() else { return Vec::new() };
        let mut v = vec![FavAct::ToPins, FavAct::Remove];
        v.extend((0..self.favs.groups.len()).map(|g| FavAct::ToGroup(Some(g))));
        if group.is_some() {
            v.push(FavAct::ToGroup(None));
        }
        v.push(FavAct::NewGroup);
        v
    }

    /// 메뉴에서 이 번호의 줄 앞에 구분선을 둔다 (보기 방식과 숨김 항목 보기 사이, 색과 그 밖의 줄 사이,
    /// 고정한 줄을 옮기는 줄과 그 밖의 줄 사이, 즐겨찾기 항목을 두고 빼는 줄과 그룹으로 옮기는 줄 사이,
    /// 그룹 이름 바꾸기와 그룹을 없애는 줄 사이)
    pub fn menu_break(&self, kind: MenuKind) -> Option<usize> {
        match kind {
            MenuKind::Layout | MenuKind::View | MenuKind::Spaces | MenuKind::Blank => None,
            MenuKind::Fav(_) => Some(2),
            MenuKind::Group(_) => Some(1),
            // 빼기와 그만두기 사이
            MenuKind::GroupDelete(_) => Some(1),
            // 보는 줄(열기, Finder에서 보기)과 그 밖의 줄(이름 바꾸기, 복사, 복제, 휴지통으로 이동) 사이
            MenuKind::Rows => Some(2),
            MenuKind::Space(_) => Some(workspaces::COLORS.len()),
            // 옮길 곳이 없는 줄(고정한 것이 하나뿐)은 구분선 없이 나머지 줄만
            MenuKind::Pin(p) => Some(self.pin_menu(p).iter().filter(|act| matches!(act, PinAct::Up | PinAct::Down)).count()).filter(|n| *n > 0),
        }
    }

    /// 메뉴에서 i번째 줄을 골랐다
    pub fn pick_menu(&mut self, kind: MenuKind, i: usize) {
        match kind {
            MenuKind::Layout => {
                if let Some(l) = Layout::ALL.get(i) {
                    self.set_layout(*l);
                }
            }
            MenuKind::View => {
                if let Some(v) = ViewMode::ALL.get(i) {
                    self.set_view(self.active, *v);
                }
            }
            MenuKind::Space(w) => {
                if let Some(color) = workspaces::COLORS.get(i) {
                    return self.set_workspace_color(w, color);
                }
                // 색 다음 줄: 이름 바꾸기(지금 공간만 있다), 지우기
                let rename = self.workspaces.current == Some(w);
                match (i - workspaces::COLORS.len(), rename) {
                    (0, true) => self.begin_rename(),
                    (0, false) | (1, true) => self.delete_workspace(w),
                    _ => {}
                }
            }
            MenuKind::Spaces => self.switch_workspace(i),
            MenuKind::Pin(p) => match self.pin_menu(p).get(i) {
                Some(PinAct::Up) => self.pin_nudge(p, true),
                Some(PinAct::Down) => self.pin_nudge(p, false),
                Some(PinAct::ToCommon) => self.pin_to_common(p),
                Some(PinAct::Unpin) => self.unpin(p),
                None => {}
            },
            MenuKind::Fav(row) => {
                let Some(Row::Item { group, index }) = self.favs.rows().get(row).copied() else { return };
                match self.fav_menu(row).get(i) {
                    Some(FavAct::ToPins) => self.fav_to_pins(group, index),
                    Some(FavAct::Remove) => {
                        self.favs.remove_item(group, index);
                        self.save_favorites();
                    }
                    Some(FavAct::ToGroup(to)) => self.fav_move_to_group(group, index, *to),
                    Some(FavAct::NewGroup) => self.fav_move_to_new_group(group, index),
                    None => {}
                }
            }
            MenuKind::Group(g) => match i {
                0 => self.begin_group_rename(g),
                1 => self.fav_ungroup(g),
                2 if self.favs.groups.get(g).is_some_and(|grp| !grp.items.is_empty()) => self.ask_group_delete(g),
                2 => self.fav_delete_group(g),
                _ => {}
            },
            MenuKind::GroupDelete(g) => {
                if i == 0 {
                    self.fav_delete_group(g);
                }
            }
            MenuKind::Rows | MenuKind::Blank => self.pick_list_menu(kind, i),
        }
    }

    /// 목록 메뉴에서 고른 줄을, 메뉴를 열 때 적어 둔 대상에 한다. 그 사이 그 탭이 다른 곳으로 갔으면 아무것도 하지 않는다
    fn pick_list_menu(&mut self, kind: MenuKind, i: usize) {
        let acts = self.row_menu();
        let Some(t) = self.list_target.take() else { return };
        let still = t.pane < self.layout.count() && {
            let tab = self.panes[t.pane].tab();
            tab.id == t.tab && tab.loc.dir() == Some(t.dir.as_str())
        };
        if !still {
            return;
        }
        if kind == MenuKind::Blank {
            match i {
                0 => self.new_folder(t.pane),
                // 붙여넣기는 그 칸에 (창 쪽이 클립보드를 읽어 지금 칸에 붙여넣는다)
                1 => {
                    self.activate_pane(t.pane);
                    self.ask_paste(false);
                }
                2 => self.refresh(t.pane),
                _ => {}
            }
            return;
        }
        let path = |name: &str| util::join(&t.dir, name);
        match acts.get(i) {
            // 폴더 하나면 그리로 들어가고, 아니면 고른 파일들을 macOS '열기'로 연다
            Some(RowAct::Open) => match t.items.as_slice() {
                [(name, _, true)] => self.navigate(t.pane, Location::Dir(path(name)), None),
                items => self.open_files.extend(items.iter().filter(|(_, _, is_dir)| !is_dir).map(|(name, ..)| path(name))),
            },
            Some(RowAct::Reveal) => self.reveal_files.extend(t.items.iter().map(|(name, ..)| path(name))),
            Some(RowAct::Rename) => {
                self.begin_item_rename(t.pane);
                // 지금 고른 것이 메뉴를 열 때의 그 항목이 아니면(그 사이 목록이 바뀌었으면) 시작하지 않는다
                let same = matches!((&self.edit, t.items.as_slice()), (Editing::Item(e), [(_, id, _)]) if e.file_id == *id);
                if !same {
                    self.edit = Editing::None;
                }
            }
            Some(act @ (RowAct::Cut | RowAct::Copy)) => self.copy_out(t.items.iter().map(|(name, ..)| path(name)).collect(), *act == RowAct::Cut),
            Some(RowAct::Duplicate) => {
                self.start_copy(t.tab, t.dir.clone(), t.items.iter().map(|(name, id, _)| (path(name), *id)).collect(), true, Paste::Copy);
            }
            Some(RowAct::Trash) => self.trash_items(t.tab, t.dir.clone(), t.items.iter().map(|(name, id, _)| (name.clone(), *id)).collect()),
            None => {}
        }
    }

    /// 오른쪽 단추로 누른 곳의 메뉴를 열어 달라고 한다: 레일의 공간 아이콘이면 그 공간의 메뉴, + 면 모든 작업 공간 목록,
    /// 사이드바의 고정한 줄이나 즐겨찾기 항목이면 그 줄의 메뉴, 목록의 줄이나 빈 곳이면 목록 메뉴. 그랬으면 true
    pub fn right_down(&mut self, g: &Gfx, x: f32, y: f32) -> bool {
        // 메뉴가 떠 있으면 닫고, 누른 곳의 메뉴를 새로 연다 (맥 메뉴와 같다). 닫기만 했어도 다시 그린다
        let closed = self.menu.take().is_some();
        let hit = self.hit(g, x, y);
        self.ask_menu(g, hit, x, y) || closed
    }

    fn ask_menu(&mut self, g: &Gfx, hit: Hit, x: f32, y: f32) -> bool {
        let kind = match hit {
            Hit::Space(i) => Some(MenuKind::Space(i)),
            // 창이 낮으면 레일에 공간이 다 보이지 않는다: 안 보이는 공간은 이 목록에서 고른다
            Hit::SpaceAdd => Some(MenuKind::Spaces),
            Hit::SideRow(i) => side_menu(&self.side_rows(), i),
            Hit::Row(p, vi) => self.list_menu(p, Some(vi), x, y),
            Hit::List(p) => self.list_menu(p, None, x, y),
            _ => None,
        };
        if let Some(kind) = kind {
            self.open_menu(g, kind);
        }
        kind.is_some()
    }

    /// 목록 메뉴를 준비한다. 고르지 않은 줄을 눌렀으면 그 줄만 고르고(고른 줄이면 고른 것들 그대로), 빈 곳이면 고른 것을 푼다.
    /// 그리고 대상을 지금 적어 둔다: 그 탭, 그 폴더, 고른 항목들의 이름과 파일 번호
    fn list_menu(&mut self, p: usize, row: Option<usize>, x: f32, y: f32) -> Option<MenuKind> {
        let dir = self.panes[p].tab().loc.dir()?.to_string();
        self.activate_pane(p);
        // 하던 입력(경로, 이름)은 그만둔다
        self.edit = Editing::None;
        self.flash = None;
        let tab = self.panes[p].tab_mut();
        match row {
            Some(vi) if tab.is_selected(vi) => tab.cursor = vi,
            Some(vi) => tab.select_only(vi),
            None => tab.clear_selection(),
        }
        let items = (0..tab.view.len()).filter_map(|vi| tab.entry_at(vi)).filter(|e| e.selected).map(|e| (e.name_string(), e.file_id, e.is_dir())).collect();
        self.list_target = Some(ListTarget { pane: p, tab: tab.id, dir, items, at: (x, y) });
        Some(if row.is_some() { MenuKind::Rows } else { MenuKind::Blank })
    }

    // ---------- 고르기 ----------

    pub fn select_all(&mut self) {
        self.panes[self.active].tab_mut().select_all();
    }

    pub fn clear_selection(&mut self) {
        self.panes[self.active].tab_mut().clear_selection();
    }

    /// 커서 이동 (화살표·Home·End·Page). 아이콘 보기에서는 칸 배치를 따라 움직인다 (Windows와 같은 규칙). shift는 범위 고르기
    pub fn move_cursor(&mut self, key: Key, shift: bool) {
        let p = self.active;
        let Some(list) = self.list_rect(p) else { return };
        let grid = Grid::of(list, self.panes[p].view);
        let tab = self.panes[p].tab_mut();
        let n = tab.view.len();
        if n == 0 {
            return;
        }
        let (cur, last) = (tab.cursor as isize, n as isize - 1);
        // 한 줄(목록 보기는 한 열)에 든 개수와 한 화면에 든 개수
        let (line, page) = if grid.horiz {
            (grid.rows as isize, ((list.w() / grid.cell_w).floor() as isize).max(1) * grid.rows as isize)
        } else {
            (grid.cols as isize, ((list.h() / grid.cell_h).floor() as isize).max(1) * grid.cols as isize)
        };
        // 줄을 건너는 이동: 끝 줄이 덜 찼으면 아래 칸 대신 마지막 항목으로
        let jump = |fwd: bool| match fwd {
            false if cur < line => cur,
            false => cur - line,
            true if cur + line > last && cur / line < last / line => last,
            true if cur + line > last => cur,
            true => cur + line,
        };
        // 목록 보기는 위아래가 한 칸, 좌우가 옆 열. 나머지는 좌우가 한 칸, 위아래가 옆 줄
        let (prev, next, back, fwd) = if grid.horiz { (Key::Up, Key::Down, Key::Left, Key::Right) } else { (Key::Left, Key::Right, Key::Up, Key::Down) };
        let to = match key {
            k if k == prev => cur - 1,
            k if k == next => cur + 1,
            k if k == back => jump(false),
            k if k == fwd => jump(true),
            Key::Home => 0,
            Key::End => last,
            Key::PageUp => cur - page,
            Key::PageDown => cur + page,
            _ => return,
        };
        tab.move_cursor(to.clamp(0, last) as usize, shift, false);
        self.ensure_visible(p);
    }

    // ---------- 고정한 것과 즐겨찾기 ----------

    /// 책갈피가 채워져 보이는 폴더인지: 이 공간에 고정했거나 '모든 공간'(즐겨찾기)에 있다
    fn is_marked(&self, dir: &str) -> bool {
        self.workspaces.current().is_some_and(|w| w.pin_index(dir).is_some()) || self.favs.find(dir).is_some()
    }

    /// 경로 줄의 책갈피: 칸 p의 폴더를 이 공간에 고정하거나 푼다
    pub fn toggle_star(&mut self, p: usize) {
        let Some(dir) = self.panes[p].tab().loc.dir().map(str::to_string) else { return };
        self.toggle_pin(&dir, true);
    }

    /// ⌃⌘T: 커서 항목(없으면 지금 폴더)을 이 공간에 고정하거나 푼다
    pub fn toggle_pin_selected(&mut self) {
        let p = self.active;
        let tab = self.panes[p].tab();
        let target = tab.cursor_entry().and_then(|e| tab.path_of(e).map(|path| (path, e.is_dir()))).or_else(|| tab.loc.dir().map(|d| (d.to_string(), true)));
        let Some((path, is_dir)) = target else { return };
        self.toggle_pin(&path, is_dir);
    }

    /// 이 공간에 고정한 것이면 풀고, 아니면 고정한다.
    /// 이 공간에는 없고 '모든 공간'(즐겨찾기)에만 있는 것이면 거기서 뺀다 (채워진 책갈피를 누르면 늘 비워진다)
    fn toggle_pin(&mut self, path: &str, is_dir: bool) {
        let Some(ws) = self.workspaces.current_mut() else { return };
        if let Some(i) = ws.pin_index(path) {
            ws.pins.remove(i);
        } else if let Some((g, i)) = self.favs.find(path) {
            self.favs.remove_item(g, i);
            return self.save_favorites();
        } else {
            ws.pin(path, is_dir);
        }
        self.clamp_side_scroll();
        self.save_workspaces();
    }

    fn save_favorites(&mut self) {
        self.clamp_side_scroll();
        if self.persist
            && let Err(e) = favorites::save(&self.favs)
        {
            sakadu::diag::log(|| format!("favorites: {e}"));
        }
    }

    /// 이 공간에 고정한 것
    fn pins(&self) -> &[Favorite] {
        self.workspaces.current().map(|w| w.pins.as_slice()).unwrap_or_default()
    }

    /// 고정한 것을 한 줄 위·아래로
    fn pin_nudge(&mut self, i: usize, up: bool) {
        let j = if up { i.wrapping_sub(1) } else { i + 1 };
        let Some(ws) = self.workspaces.current_mut().filter(|ws| i < ws.pins.len() && j < ws.pins.len()) else { return };
        ws.pins.swap(i, j);
        self.save_workspaces();
    }

    /// 고정한 줄 from을 slot 번째 줄 앞에 끼워 넣는다 (slot이 줄 수와 같으면 맨 끝). 제자리면 그대로 (Windows pin_move와 같다)
    fn pin_move(&mut self, from: usize, slot: usize) {
        let Some(ws) = self.workspaces.current_mut().filter(|ws| from < ws.pins.len() && slot <= ws.pins.len() && slot != from && slot != from + 1) else { return };
        let f = ws.pins.remove(from);
        ws.pins.insert(if slot > from { slot - 1 } else { slot }, f);
        self.save_workspaces();
    }

    /// 고정한 것 i를 푼다
    fn unpin(&mut self, i: usize) {
        let Some(ws) = self.workspaces.current_mut().filter(|ws| i < ws.pins.len()) else { return };
        ws.pins.remove(i);
        self.clamp_side_scroll();
        self.save_workspaces();
    }

    /// 고정한 것을 '모든 공간'(즐겨찾기)으로 옮긴다: 어느 작업 공간에서나 보이게.
    /// 즐겨찾기 파일에 먼저 저장하고, 저장됐을 때만 이 공간에서 뺀다 (저장하지 못했는데 빼면 다음에 열 때 양쪽 모두에서 사라진다)
    fn pin_to_common(&mut self, i: usize) {
        let Some(f) = self.pins().get(i).cloned() else { return };
        if self.favs.find(&f.path).is_none() {
            let before = self.favs.clone();
            self.favs.push(None, f);
            if self.persist
                && let Err(e) = favorites::save(&self.favs)
            {
                self.favs = before;
                self.flash = Some((self.active, format!("즐겨찾기를 저장하지 못해 옮기지 않았어요: {e}")));
                return;
            }
        }
        self.unpin(i);
    }

    /// 즐겨찾기 항목을 '모든 공간'에서 빼서 지금 작업 공간에만 고정한다.
    /// 작업 공간 파일에 먼저 저장하고, 저장됐을 때만 즐겨찾기에서 뺀다 (pin_to_common과 같은 이유)
    fn fav_to_pins(&mut self, group: Option<usize>, index: usize) {
        let Some(f) = self.favs.item(group, index).cloned() else { return };
        let Some(ws) = self.workspaces.current_mut() else { return };
        if ws.pin_index(&f.path).is_none() {
            ws.pins.push(f);
            if let Err(e) = self.try_save_workspaces() {
                if let Some(ws) = self.workspaces.current_mut() {
                    ws.pins.pop();
                }
                self.flash = Some((self.active, format!("작업 공간을 저장하지 못해 옮기지 않았어요: {e}")));
                return;
            }
        }
        self.favs.remove_item(group, index);
        self.save_favorites();
    }

    /// '모든 공간' 줄의 새 그룹 단추: 맨 끝에 '새 그룹'을 만들고 이름을 바로 고칠 수 있게 한다 (Windows add_fav_group과 같다)
    fn add_fav_group(&mut self) {
        let g = self.favs.add_group("새 그룹".to_string());
        self.save_favorites();
        self.begin_group_rename(g);
    }

    /// 그룹 g의 이름을 그 머리 줄에서 고치기 시작한다 (줄이 보이게 스크롤한다. 확정은 commit_address, 취소는 cancel_address)
    fn begin_group_rename(&mut self, g: usize) {
        if g >= self.favs.groups.len() || self.find_shown() {
            return;
        }
        let rows = self.side_rows();
        if let Some(i) = rows.iter().position(|r| matches!(r.kind, SideKind::Group(h, _) if h == g)) {
            self.side_reveal(&rows[i]);
        }
        self.edit = Editing::Group(g);
    }

    /// 고친 그룹 이름을 적는다. 비었거나 그대로면 둔다
    fn commit_group_rename(&mut self, g: usize, text: &str) {
        let name = favorites::clean_name(text);
        if !name.is_empty() && self.favs.set_row_name(Row::Group(g), name) {
            self.save_favorites();
        }
    }

    /// 그룹 이름을 고치던 중이면 그만둔다 (그룹 번호가 달라지기 전에)
    fn end_group_rename(&mut self) {
        if matches!(self.edit, Editing::Group(_)) {
            self.edit = Editing::None;
        }
    }

    /// 그룹 풀기: 그룹만 없애고 안의 즐겨찾기는 그룹 없음 끝으로 옮긴다
    fn fav_ungroup(&mut self, g: usize) {
        self.end_group_rename();
        self.favs.ungroup(g);
        self.save_favorites();
    }

    /// 즐겨찾기가 든 그룹을 지우기 전에 묻는다: 그 그룹 머리에 '빼기·그만두기' 메뉴를 연다 (Windows의 확인 창).
    /// 메뉴를 고른 자리에는 글자 폭을 잴 그림판이 없어 폭은 글자 수로 어림한다 (한글 한 자 14, 둘레 48)
    fn ask_group_delete(&mut self, g: usize) {
        let kind = MenuKind::GroupDelete(g);
        let Some(anchor) = self.menu_anchor(kind) else { return };
        let lines = self.menu_lines(kind);
        let longest = lines.iter().map(|l| l.label.chars().count()).max().unwrap_or(0) as f32;
        let w = (longest * 14.0 + 48.0).clamp(160.0, (self.w - 2.0 * MARGIN).clamp(1.0, 420.0));
        self.menu = Some(OpenMenu { kind, anchor, lines, w, sel: None, scroll: 0.0 });
    }

    /// 그룹과 안의 즐겨찾기를 함께 지운다 (Windows의 '그룹과 안의 즐겨찾기 빼기'. 즐겨찾기가 들어 있으면 ask_group_delete로 물은 뒤)
    fn fav_delete_group(&mut self, g: usize) {
        self.end_group_rename();
        self.favs.delete_group(g);
        self.save_favorites();
    }

    /// 즐겨찾기 항목을 그룹 to의 끝으로 (None이면 그룹 없음 끝으로)
    fn fav_move_to_group(&mut self, group: Option<usize>, index: usize, to: Option<usize>) {
        let at = self.favs.section(to).len();
        if self.favs.move_item((group, index), to, at).is_some() {
            self.save_favorites();
        }
    }

    /// 즐겨찾기 항목으로 새 그룹을 만든다: 맨 끝에 '새 그룹'을 만들어 그 항목을 넣고, 그룹 이름을 바로 고칠 수 있게 한다
    fn fav_move_to_new_group(&mut self, group: Option<usize>, index: usize) {
        if self.favs.item(group, index).is_none() {
            return;
        }
        self.end_group_rename();
        let g = self.favs.add_group("새 그룹".to_string());
        self.favs.move_item((group, index), Some(g), 0);
        self.save_favorites();
        self.begin_group_rename(g);
    }

    /// 사이드바 목록의 줄. 위에는 이 공간에 고정한 것(없으면 안내), 아래에는 '모든 공간': 구역 이름, 바로가기, 즐겨찾기.
    /// '모든 공간'은 줄이 적으면 목록 아래 끝에 붙고, 많아서 넘치면 고정한 것 바로 아래에 이어져 함께 스크롤된다
    fn side_rows(&self) -> Vec<SideRow> {
        let row = |kind, label: &str, path: Option<&str>, is_dir, indent, y, h, flow| SideRow { kind, label: label.to_string(), path: path.map(str::to_string), is_dir, indent, y, h, flow };
        let mut rows = Vec::new();
        let mut y = 0.0;
        let pins = self.pins();
        for (i, f) in pins.iter().enumerate() {
            rows.push(row(SideKind::Pin, &f.name, Some(&f.path), f.is_dir, false, y, SIDE_ROW_H, FlowKind::Pin(i)));
            y += SIDE_ROW_H;
        }
        if pins.is_empty() {
            rows.push(row(SideKind::Hint, "", None, false, false, y, HINT_H, FlowKind::PinHint));
            y += HINT_H;
        }
        let favs = self.favs.rows();
        let common_h = SIDE_LABEL_H + (self.places.len() + favs.len()) as f32 * SIDE_ROW_H;
        let mut y = side_common_top(y, common_h, self.side_list().h(), COMMON_GAP);
        rows.push(row(SideKind::Label, "모든 공간", None, false, false, y, SIDE_LABEL_H, FlowKind::Other));
        y += SIDE_LABEL_H;
        for (label, path) in &self.places {
            rows.push(row(SideKind::Place, label, Some(path), true, false, y, SIDE_ROW_H, FlowKind::Other));
            y += SIDE_ROW_H;
        }
        // 즐겨찾기 바로 위 줄(마지막 바로가기, 없으면 구역 이름): 즐겨찾기가 없을 때 끌어 온 줄은 그 아래에 놓인다
        if let Some(last) = rows.last_mut() {
            last.flow = FlowKind::FavAnchor;
        }
        for fav in favs {
            let r = match fav {
                Row::Group(g) => self.favs.groups.get(g).map(|grp| row(SideKind::Group(g, grp.collapsed), &grp.name, None, false, false, y, SIDE_ROW_H, FlowKind::Fav(fav))),
                Row::Item { group, index } => self.favs.item(group, index).map(|f| row(SideKind::Fav, &f.name, Some(&f.path), f.is_dir, group.is_some(), y, SIDE_ROW_H, FlowKind::Fav(fav))),
            };
            if let Some(r) = r {
                rows.push(r);
                y += SIDE_ROW_H;
            }
        }
        rows
    }

    /// 사이드바 줄의 가운데 (창 좌표). 그 이름의 줄이 없거나 목록 밖으로 스크롤돼 있으면 None. 창 시험과 그림이 실제로 누르고 끌 때 쓴다
    pub fn side_row_center(&self, label: &str) -> Option<(f32, f32)> {
        let rows = self.side_rows();
        let r = self.side_rect(rows.iter().find(|r| r.label == label && !matches!(r.kind, SideKind::Hint | SideKind::Label))?);
        let list = self.side_list();
        let (x, y) = ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0);
        (y >= list.t && y < list.b).then_some((x, y))
    }

    /// 사이드바 줄이 아직 미끄러지는 중인지 (창 쪽이 곧 다시 그린다)
    pub fn side_moving(&self) -> bool {
        self.side_flow.borrow().moving
    }

    // ---------- 사이드바 끌어 옮기기 (공통 ui::side_flow. Windows side_flow.rs와 같다) ----------

    /// 화면 y를 사이드바 목록 좌표(스크롤 전)로
    fn side_list_y(&self, y: f32) -> f32 {
        y - self.side_list().t + self.side_scroll
    }

    /// 누른 줄에서 잡은 자리 (줄 위쪽에서 마우스까지)
    fn side_grab(&self, row: usize, y: f32) -> f32 {
        let top = self.side_rows().get(row).map_or(0.0, |r| r.y);
        (self.side_list_y(y) - top).clamp(0.0, SIDE_ROW_H)
    }

    /// 끄는 위치 y가 이 공간에 고정한 쪽(true, '모든 공간' 이름 줄보다 위)인지 '모든 공간' 쪽(false)인지. 목록 밖이면 None
    fn side_over_pins(&self, rows: &[SideRow], y: f32) -> Option<bool> {
        let list = self.side_list();
        if y < list.t || y > list.b {
            return None;
        }
        let label = rows.iter().find(|r| r.kind == SideKind::Label)?;
        Some(y < self.side_rect(label).t)
    }

    /// 끄는 동안의 그림과 놓을 곳: 끄는 줄과 그 계획 (빈자리는 카드 가운데가 지난 줄 다음, 그룹은 그룹 사이 틈에만). 끄는 중이 아니면 None
    fn side_drop_plan(&self, rows: &[SideRow]) -> Option<(FlowKind, DropPlan)> {
        let press = self.side_press?;
        let ((y, grab), dragged) = (press.moving?, press.what?);
        let slots: Vec<FlowSlot> = rows.iter().map(SideRow::slot).collect();
        let card_t = self.side_list_y(y) - grab;
        Some((dragged, drop_plan(&slots, dragged, card_t, SIDE_ROW_H, self.side_over_pins(rows, y), &self.favs)))
    }

    /// 끌어서 놓았다: 보이던 빈자리 그대로 옮기고 저장한다. 옮긴 줄은 카드 자리에서 새 자리로 미끄러져 들어간다
    fn side_drop(&mut self, rows: &[SideRow], dragged: FlowKind, p: DropPlan) {
        let key_of = |k: FlowKind| rows.iter().find(|r| r.flow == k).map(SideRow::key).unwrap_or_default();
        let key = match (dragged, p.to) {
            // 고정한 것 안에서: 빼낸 뒤 기준 자리를 pin_move의 옮기기 전 기준으로
            (FlowKind::Pin(i), DropTo::Pin(gap)) => {
                self.pin_move(i, if gap >= i { gap + 1 } else { gap });
                key_of(dragged)
            }
            // '모든 공간' → 이 공간: '이 작업 공간에만 두기'와 같고, 놓은 자리에 끼운다
            (FlowKind::Fav(Row::Item { group, index }), DropTo::Pin(gap)) => {
                let path = self.favs.item(group, index).map(|f| f.path.clone()).unwrap_or_default();
                let before = self.pins().len();
                self.fav_to_pins(group, index);
                if self.pins().len() > before {
                    self.pin_move(before, gap);
                }
                pin_key(&path)
            }
            // 이 공간 → '모든 공간': '모든 공간에 두기'와 같고, 놓은 자리(그룹)에 끼운다
            (FlowKind::Pin(i), DropTo::FavItem(group, index)) => {
                let Some(path) = self.pins().get(i).map(|f| f.path.clone()) else { return };
                let already = self.favs.find(&path).is_some();
                self.pin_to_common(i);
                if !already
                    && let Some(from) = self.favs.find(&path)
                    && self.favs.move_item_to(from, group, index).is_some()
                {
                    self.save_favorites();
                }
                fav_key(&path)
            }
            (FlowKind::Fav(Row::Item { group, index }), DropTo::FavItem(to, at)) => {
                if self.favs.move_item_to((group, index), to, at).is_some() {
                    self.save_favorites();
                }
                key_of(dragged)
            }
            (FlowKind::Fav(Row::Group(g)), DropTo::FavGroup(to)) => {
                if self.favs.move_group(g, to).is_some() {
                    self.save_favorites();
                }
                key_of(dragged)
            }
            _ => return,
        };
        self.side_flow.get_mut().start_at(key, p.card_t);
    }

    /// 그 줄이 목록 안에 다 보이도록 스크롤한다
    fn side_reveal(&mut self, row: &SideRow) {
        let (list, r) = (self.side_list(), self.side_rect(row));
        if r.t < list.t {
            self.side_scroll -= list.t - r.t;
        } else if r.b > list.b {
            self.side_scroll += r.b - list.b;
        }
        self.clamp_side_scroll();
    }

    /// 그룹 g의 이름 상자 자리 (머리 줄의 이름 칸: 이름이 그려지던 자리에서 시작한다).
    /// 그 그룹이 없거나, 검색 패널이 사이드바 자리에 있거나, 줄이 목록에 다 보이지 않으면 None
    fn group_edit_rect(&self, g: usize) -> Option<R> {
        if self.find_shown() {
            return None;
        }
        let rows = self.side_rows();
        let r = self.side_rect(rows.iter().find(|r| matches!(r.kind, SideKind::Group(h, _) if h == g))?);
        let list = self.side_list();
        if r.t < list.t - 1.0 || r.b > list.b + 1.0 {
            return None;
        }
        let lx = r.l + SIDE_LABEL_L;
        Some(R::new(lx - SP_XS, r.t + 3.0, r.r - SP_S, r.b - 3.0))
    }

    /// '모든 공간' 이름 줄 오른쪽 끝의 새 그룹 단추 자리 (화면 좌표. 목록과 함께 스크롤된다)
    fn new_group_rect(&self, rows: &[SideRow]) -> Option<R> {
        let head = self.side_rect(rows.iter().find(|r| r.kind == SideKind::Label)?);
        // 구역 이름 글자는 위쪽 선 아래(SP_XS부터)에 놓인다: 단추도 그 글자 줄의 가운데에
        let t = head.t + SP_XS + (head.h() - SP_XS - MINI) / 2.0;
        Some(R::new(head.r - SP_HAIR - MINI, t, head.r - SP_HAIR, t + MINI))
    }

    fn open_side_row(&mut self, i: usize) {
        let rows = self.side_rows();
        let Some(row) = rows.get(i) else { return };
        if let SideKind::Group(g, collapsed) = row.kind {
            self.favs.set_collapsed(g, !collapsed);
            self.save_favorites();
            return;
        }
        let Some(path) = row.path.clone() else { return };
        if row.is_dir {
            self.navigate(self.active, Location::Dir(path), None);
        } else {
            self.open_files.push(path);
        }
    }

    // ---------- 자리 ----------

    /// 칸들과 오른쪽 미리 보기 창이 놓이는 곳: 사이드바 오른쪽, 도구 막대 아래
    fn content_area(&self) -> R {
        let l = SIDE_W + MARGIN;
        R::new(l, WORKBAR_H, (self.w - MARGIN).max(l + 100.0), (self.h - MARGIN).max(WORKBAR_H + 100.0))
    }

    /// 칸들이 놓이는 곳 (미리 보기 창이 열려 있으면 그 창과 사이 틈만큼 비운다)
    fn area(&self) -> R {
        let c = self.content_area();
        match self.preview_rect() {
            Some(pr) => R::new(c.l, c.t, (pr.l - GAP).max(c.l + 100.0), c.b),
            None => c,
        }
    }

    fn pane_rects(&self) -> Vec<(usize, R)> {
        pane_rects(self.area(), self.split, self.layout)
    }

    fn splitters(&self) -> (Option<R>, Option<R>) {
        splitters(self.area(), self.split, self.layout)
    }

    fn list_rect(&self, p: usize) -> Option<R> {
        self.pane_rects().into_iter().find(|(i, _)| *i == p).map(|(_, r)| geom(r).list)
    }

    /// 칸 p의 탭들의 스크롤을 내용 안으로. 시작 화면은 그 배치의 높이로 (보이는 탭만. 숨은 시작 화면 탭은 보일 때 배치가 맞춘다)
    fn clamp_scroll(&mut self, p: usize) {
        let Some(list) = self.list_rect(p) else { return };
        let view = self.panes[p].view;
        let start_max = self.start_max_scroll(p);
        let pane = &mut self.panes[p];
        for (i, tab) in pane.tabs.iter_mut().enumerate() {
            match (tab.loc == Location::Start, start_max) {
                (true, Some(max)) if i == pane.active => tab.scroll = tab.scroll.clamp(0.0, max),
                (true, _) => {}
                (false, _) => clamp_tab(tab, list, view),
            }
        }
    }

    fn ensure_visible(&mut self, p: usize) {
        let Some(list) = self.list_rect(p) else { return };
        if self.panes[p].tab().loc == Location::Start {
            return self.clamp_scroll(p);
        }
        let view = self.panes[p].view;
        reveal(self.panes[p].tab_mut(), list, view);
    }

    /// 사이드바(레일, 공간 이름)가 시작하는 높이: 도구 막대 아래. 신호등이 더 아래까지 오면 그 아래
    fn side_top(&self) -> f32 {
        WORKBAR_H.max(self.lights_bottom + 8.0)
    }

    /// 작업 공간 레일의 자리 (공간 아이콘들과 새 공간 +)
    fn rail(&self) -> RailGeom {
        rail_geom(self.side_top(), self.h, self.workspaces.list.len(), self.workspaces.current.unwrap_or(0), false)
    }

    /// 레일 옆 맨 위, 지금 공간의 이름 자리
    fn side_title(&self) -> R {
        let t = self.side_top();
        R::new(BAR_W + 8.0, t, SIDE_W - 8.0, t + SIDE_HEADER_H - 4.0)
    }

    /// 이름 입력 상자 자리 (이름 줄 안. 글자가 이름을 그리던 자리에서 시작한다)
    fn rename_rect(&self) -> R {
        let t = self.side_title();
        R::new(t.l + 4.0, t.t + 3.0, t.r, t.b - 3.0)
    }

    /// 사이드바 목록이 보이는 곳 (이름 줄 아래부터 창 아래 여백 앞까지)
    fn side_list(&self) -> R {
        let t = self.side_top() + SIDE_HEADER_H;
        R::new(BAR_W, t, SIDE_W, (self.h - MARGIN).max(t))
    }

    /// 그 줄이 지금 보이는 자리 (스크롤한 만큼 옮긴다)
    fn side_rect(&self, row: &SideRow) -> R {
        let list = self.side_list();
        let t = list.t + row.y - self.side_scroll;
        R::new(list.l + 8.0, t, list.r - 8.0, t + row.h)
    }

    /// 사이드바 스크롤을 줄 수 안으로 (줄이 바뀌거나 창 크기가 바뀐 뒤)
    fn clamp_side_scroll(&mut self) {
        let content = self.side_rows().last().map_or(0.0, |r| r.y + r.h);
        // '모든 공간'을 아래 끝에 붙인 목록은 내용 높이가 목록 높이와 같다 (계산 오차만큼의 스크롤은 없는 것으로 본다)
        let max = content - self.side_list().h();
        self.side_scroll = self.side_scroll.clamp(0.0, if max < 0.5 { 0.0 } else { max });
    }

    /// 도구 막대 단추 자리: 칸 카드의 왼쪽 가장자리와 오른쪽 끝(미리 보기 창이 열려 있으면 그 창의 오른쪽)에 맞춘다.
    /// 오른쪽 끝은 숨김 항목의 눈, 자동 미리 보기, 검색 (창을 열고 닫아도 자리가 그대로다).
    /// 검색은 열면 그 자리에서 입력 칸으로 늘어난다. 위로는 ⌘↑와 경로 조각으로 한다 (Finder와 같다)
    fn toolbar(&self) -> ToolbarGeom {
        let a = self.content_area();
        let search_w = if self.find_shown() { find::FIELD_W } else { 0.0 };
        toolbar_geom(&ToolbarSpec { left: a.l, right: a.r, up: false, hidden: true, preview: true, search: true, search_w })
    }

    /// 칸 p의 경로 줄에 있는 보기 방식 단추 (좁아서 없으면 폭 0)
    pub fn view_button(&self, p: usize) -> R {
        self.pane_rects().into_iter().find(|(q, _)| *q == p).map_or(R::new(0.0, 0.0, 0.0, 0.0), |(_, r)| geom(r).view)
    }

    /// 탭 칩 자리와 새 탭(+) 자리 (app.rs chips와 같다: 오른쪽 항목 수 자리 앞까지, 넘치면 지금 탭을 지킨다)
    fn chips(&self, g: &Gfx, p: usize, gm: &PaneGeom) -> (Vec<R>, R) {
        let pane = &self.panes[p];
        let n = pane.tabs.len();
        let strip = chip_strip(gm, g.measure(&count_text(pane.tab()), Font::Small));
        let natural: Vec<f32> = pane.tabs.iter().map(|t| tab_natural_w(g.measure(&t.loc.title(), Font::Normal), n)).collect();
        let trs = tab_rects(strip, &natural, pane.active);
        let add = add_rect(strip, &trs);
        (trs, add)
    }

    /// 경로 조각 (공통 crumbs, 맥에는 '내 PC'가 없다): (자리, 그 조각의 폴더, 이름). 앞쪽을 접은 "…"도 그 상위 폴더로 간다
    fn crumbs(&self, g: &Gfx, area: R, dir: &str) -> Vec<(R, Option<String>, String)> {
        crumbs(g, area, &Location::Dir(dir.to_string()), None)
            .into_iter()
            .map(|c| (R::new(c.x0, area.t, c.x1, area.b), c.target.dir().map(str::to_string), c.label))
            .collect()
    }

    // ---------- 누르기 판정 ----------

    /// 창을 끌 수 있는 곳인지: 도구 막대 높이 안의 빈 곳 (단추가 없는 자리, 사이드바 위 신호등 옆 포함)
    pub fn drag_zone(&self, g: &Gfx, x: f32, y: f32) -> bool {
        y < WORKBAR_H && self.hit(g, x, y) == Hit::None
    }

    pub fn hit(&self, g: &Gfx, x: f32, y: f32) -> Hit {
        self.hit_with(g, x, y, true)
    }

    /// card가 false면 미리 보기 카드는 없는 것으로 보고 그 아래 것을 찾는다 (두 번 누르기의 둘째 누름: mouse_down)
    fn hit_with(&self, g: &Gfx, x: f32, y: f32, card: bool) -> Hit {
        // 메뉴가 열려 있으면 메뉴만 받는다: 메뉴 밖은 아무것도 아니다 (올림 표시도, 누르기도 메뉴를 닫는 것뿐)
        if let (Some(m), Some(r)) = (self.menu.as_ref(), self.menu_rect()) {
            return if r.contains(x, y) { Hit::Menu(menu_hit(r, &m.lines, m.scroll, x, y)) } else { Hit::None };
        }
        // 명령 메뉴는 모든 것 위에 떠 있다 (판 밖을 누르면 닫힌다)
        if let Some(h) = self.cmd_hit(x, y) {
            return h;
        }
        if x < BAR_W {
            let rail = self.rail();
            if let Some((i, _)) = rail.spaces.iter().find(|(_, r)| r.contains(x, y)) {
                return Hit::Space(*i);
            }
            return if rail.add.contains(x, y) { Hit::SpaceAdd } else { Hit::None };
        }
        if x < SIDE_W {
            // 검색하는 동안은 사이드바 자리가 검색 패널이다
            if self.find_shown() {
                return if y >= self.side_top() { Hit::Find(self.find_hit(g, x, y)) } else { Hit::None };
            }
            if self.side_title().contains(x, y) {
                return Hit::SideTitle;
            }
            if !self.side_list().contains(x, y) {
                return Hit::None;
            }
            // 안내와 구역 이름은 누를 수 없다. 구역 이름 줄 오른쪽 끝에는 새 그룹 단추가 있다
            let rows = self.side_rows();
            if self.new_group_rect(&rows).is_some_and(|r| r.contains(x, y)) {
                return Hit::NewGroup;
            }
            return rows.iter().position(|r| !matches!(r.kind, SideKind::Hint | SideKind::Label) && self.side_rect(r).contains(x, y)).map_or(Hit::None, Hit::SideRow);
        }
        if y < WORKBAR_H {
            // 도구 막대: 뒤로·앞으로는 지금 칸에 대해 움직인다
            let m = self.toolbar();
            let p = self.active;
            let hits = [(m.back, Hit::Back(p)), (m.fwd, Hit::Fwd(p)), (m.layout, Hit::LayoutPick), (m.hidden, Hit::HiddenToggle), (m.preview, Hit::PreviewToggle)];
            if let Some((_, h)) = hits.into_iter().find(|(r, _)| r.contains(x, y)) {
                return h;
            }
            // 검색: 닫혀 있으면 돋보기, 열려 있으면 입력 칸과 그 오른쪽 끝의 닫기
            return match find::field_parts(&m) {
                Some((_, close)) if close.contains(x, y) => Hit::FindClose,
                Some(_) if m.search.contains(x, y) => Hit::FindField,
                None if m.search.contains(x, y) => Hit::FindOpen,
                _ => Hit::None,
            };
        }
        // 오른쪽 미리 보기 창: 머리의 단추, 그 밖은 창이 받는다
        if let Some(h) = self.preview_hit(x, y) {
            return h;
        }
        // 미리 보기 카드는 칸 위에 떠 있다: 카드 안은 어디든 카드가 받는다 (아래 칸을 누르지 않게)
        if let Some(m) = self.card().filter(|m| card && m.card.contains(x, y)) {
            return if m.close.contains(x, y) {
                Hit::CardClose
            } else if m.expand.contains(x, y) {
                Hit::CardExpand
            } else {
                Hit::Card
            };
        }
        if split_cross(self.area(), self.split, self.layout).is_some_and(|r| r.contains(x, y)) {
            return Hit::SplitXY;
        }
        let (sx, sy) = self.splitters();
        if sx.is_some_and(|r| r.contains(x, y)) {
            return Hit::SplitX;
        }
        if sy.is_some_and(|r| r.contains(x, y)) {
            return Hit::SplitY;
        }
        for (p, r) in self.pane_rects() {
            if !r.contains(x, y) {
                continue;
            }
            let gm = geom(r);
            let tab = self.panes[p].tab();
            if gm.head.contains(x, y) {
                let (trs, add) = self.chips(g, p, &gm);
                for (i, tr) in trs.iter().enumerate() {
                    if tr.contains(x, y) {
                        let closable = self.panes[p].tabs.len() > 1 && (i == self.panes[p].active || self.hover == Hit::Tab(p, i) || self.hover == Hit::TabClose(p, i));
                        return if closable && close_rect(*tr).contains(x, y) { Hit::TabClose(p, i) } else { Hit::Tab(p, i) };
                    }
                }
                return if add.contains(x, y) { Hit::TabAdd(p) } else { Hit::Pane(p) };
            }
            // 책갈피와 보기 방식 단추는 폴더에만 있다 (시작 화면에는 없다)
            let start = tab.loc == Location::Start;
            if gm.star.contains(x, y) && !start {
                return Hit::Star(p);
            }
            if gm.view.contains(x, y) && self.edit != Editing::Address(p) && !start {
                return Hit::ViewPick(p);
            }
            if gm.crumb.contains(x, y) {
                if let Some(dir) = tab.loc.dir()
                    && let Some(i) = self.crumbs(g, gm.crumbs, dir).iter().position(|(cr, path, _)| path.is_some() && cr.contains(x, y))
                {
                    return Hit::Crumb(p, i);
                }
                return Hit::CrumbEmpty(p);
            }
            // 시작 화면: 머리글·목록 자리에 카드와 줄
            if start {
                return self.start_hit(p, x, y);
            }
            if gm.header.contains(x, y) {
                return self.header_hit(g, p, &gm, x, y);
            }
            if gm.list.contains(x, y) {
                let grid = Grid::of(gm.list, self.panes[p].view);
                return grid.at(x, y, tab.scroll, tab.view.len()).map_or(Hit::List(p), |vi| Hit::Row(p, vi));
            }
            return Hit::Pane(p);
        }
        Hit::None
    }

    /// 지금 칸에서 key 열 머리글(다른 보기면 그 정렬 칩)의 가운데. smoke가 실제 창에서 누른다
    pub fn header_point(&self, g: &Gfx, key: SortKey) -> Option<(f32, f32)> {
        let p = self.active;
        let gm = geom(self.pane_rects().into_iter().find(|(q, _)| *q == p)?.1);
        let y = (gm.header.t + gm.header.b) / 2.0;
        let mid = |(l, r): (f32, f32)| ((l + r) / 2.0, y);
        if self.panes[p].view != ViewMode::Details {
            return sort_chips(g, gm.header, self.panes[p].sort.key).into_iter().find(|(k, _, _)| *k == key).map(|(_, _, c)| mid((c.l, c.r)));
        }
        self.date_col_w(g);
        let cols = columns(gm.list, self.cols(p));
        match key {
            SortKey::Name => Some(cols.name),
            SortKey::Modified => cols.modified,
            SortKey::Type => cols.kind,
            SortKey::Size => cols.size,
        }
        .map(mid)
    }

    /// 칸 p의 머리글 줄에서 가리키는 것 (Windows layout.rs와 같은 판정): 자세히 보기는 열 경계(±4)와 열, 다른 보기는 정렬 칩
    fn header_hit(&self, g: &Gfx, p: usize, gm: &PaneGeom, x: f32, y: f32) -> Hit {
        let pane = &self.panes[p];
        if pane.view != ViewMode::Details {
            return match sort_chips(g, gm.header, pane.sort.key).into_iter().find(|(_, _, r)| r.contains(x, y)) {
                Some((key, _, _)) => Hit::Header(p, key),
                None => Hit::Pane(p),
            };
        }
        self.date_col_w(g);
        let cols = columns(gm.list, self.cols(p));
        for (ci, c) in cols.resizable().iter().enumerate() {
            if let Some((l, _)) = c
                && (x - l).abs() <= 4.0
            {
                return Hit::HeaderResize(p, ci);
            }
        }
        let key = if cols.size.is_some_and(|(l, _)| x >= l) {
            SortKey::Size
        } else if cols.kind.is_some_and(|(l, _)| x >= l) {
            SortKey::Type
        } else if cols.modified.is_some_and(|(l, _)| x >= l) {
            SortKey::Modified
        } else {
            SortKey::Name
        };
        Hit::Header(p, key)
    }

    // ---------- 마우스·키 ----------

    /// 누름. clicks는 연속으로 누른 횟수 (2면 두 번 누르기)
    pub fn mouse_down(&mut self, g: &Gfx, x: f32, y: f32, mods: Mods, clicks: usize) {
        // 메뉴가 열려 있으면: 줄을 누르면 고르고, 메뉴 밖을 누르면 닫기만 한다 (그 누름은 뒤로 넘기지 않는다)
        if self.menu.is_some() {
            match self.hit(g, x, y) {
                Hit::Menu(Some(i)) => self.pick_open_menu(i),
                Hit::Menu(None) => {}
                _ => self.close_menu(),
            }
            return;
        }
        // 명령 메뉴가 열려 있으면: 줄을 누르면 열고, 판 밖을 누르면 닫기만 한다. 이어지는 두 번째 누름(두 번 누르기)은 버린다:
        // 방금 연 목록의 항목을 열지 않게
        if self.cmd.is_some() {
            let hit = self.hit(g, x, y);
            self.press_on_card = true;
            self.cmd_press(hit, mods);
            return;
        }
        let mut hit = self.hit(g, x, y);
        // 두 번 누르기는 첫 누름이 닿은 쪽의 것이다 (미리 보기 카드는 누름 한 번에 뜨고, 작아지고, 닫혀서 둘째 누름 때는 그 자리에 다른 것이 있다).
        // - 첫 누름이 카드였으면 둘째 누름은 버린다: 카드가 작아지거나 닫혀 드러난 줄을 열지 않는다
        // - 첫 누름이 카드가 아니었으면 둘째 누름은 카드 아래 것이 받는다: 그 누름으로 뜬 카드가 가로채면 파일이 열리지 않는다
        // 시작 화면의 칸·줄도 누름 한 번에 옮겨 가므로, 그 둘째 누름은 버린다 (새로 연 목록의 항목을 열지 않게. Windows와 같다)
        let on_card = matches!(hit, Hit::Card | Hit::CardExpand | Hit::CardClose);
        if clicks < 2 {
            self.press_on_card = on_card || matches!(hit, Hit::Start(..));
        } else if self.press_on_card {
            return;
        } else if on_card {
            hit = self.hit_with(g, x, y, false);
        }
        // 입력 상자 밖을 누르면 입력(경로, 공간 이름, 항목·그룹 이름)을 그만둔다. 알림은 다음 입력 때 지운다
        self.edit = Editing::None;
        self.flash = None;
        self.press = None;
        self.side_press = None;
        // 마우스를 쓰면 시작 화면의 키보드 초점 테두리는 감춘다
        self.start_keys = false;
        // 검색(돋보기, 입력 칸, 검색 패널)은 검색이 받는다. 다른 곳을 누르면 키보드만 검색 입력 칸에서 떠난다
        if self.find_press(hit, clicks) {
            return;
        }
        // 두 번 누르기의 첫 번째에 누른 항목 (한 번 누르기면 아래에서 새로 적는다)
        let first = self.clicked.take();
        // ⌃-누르기는 오른쪽 단추와 같다
        if mods.ctrl && self.ask_menu(g, hit, x, y) {
            return;
        }
        match hit {
            Hit::Tab(p, _)
            | Hit::TabClose(p, _)
            | Hit::TabAdd(p)
            | Hit::Back(p)
            | Hit::Fwd(p)
            | Hit::Crumb(p, _)
            | Hit::CrumbEmpty(p)
            | Hit::Star(p)
            | Hit::ViewPick(p)
            | Hit::Header(p, _)
            | Hit::HeaderResize(p, _)
            | Hit::Row(p, _)
            | Hit::List(p)
            | Hit::Start(p, _)
            | Hit::Pane(p) => self.activate_pane(p),
            _ => {}
        }
        // 두 번 누르기는 처음 누른 그 항목을 연다. 두 번 사이에 목록이 바뀌어(폴더 감시) 손 밑에 다른 줄이나 빈 곳이 와 있어도
        // 그것을 열지 않는다. 처음 누른 항목이 그 사이 없어졌으면 아무것도 열지 않는다
        if clicks >= 2
            && let Some(first) = first
            && let Hit::Row(p, _) | Hit::List(p) = hit
            && first.pane == p
        {
            let tab = self.panes[p].tab();
            let row = (0..tab.view.len()).find(|vi| tab.entry_at(*vi).is_some_and(|e| e.file_id == first.file_id && e.name_string() == first.name));
            if let Some(vi) = row.filter(|_| tab.id == first.tab) {
                self.open_row(p, vi);
            }
            return;
        }
        match hit {
            Hit::CrumbEmpty(p) => self.begin_address(p),
            Hit::Space(i) => self.switch_workspace(i),
            Hit::SpaceAdd => self.new_workspace(),
            Hit::SideTitle => {
                if clicks >= 2 {
                    self.begin_rename();
                }
            }
            // 사이드바 줄은 누를 때가 아니라 뗄 때 연다: 그 사이 위아래로 끌면 그 줄을 옮긴다 (mouse_dragged, mouse_up).
            // 두 번째 누름(두 번 누르기)은 버린다: 첫 누름을 뗄 때 이미 열었다 (파일을 두 번 열거나 그룹이 접혔다 펼쳐지지 않게)
            Hit::SideRow(i) => {
                if clicks < 2 {
                    let what = self.side_rows().get(i).map(|r| r.flow).filter(|k| matches!(k, FlowKind::Pin(_) | FlowKind::Fav(_)));
                    self.side_press = Some(SidePress { row: i, what, y0: y, moving: None });
                }
            }
            Hit::NewGroup => {
                if clicks < 2 {
                    self.add_fav_group();
                }
            }
            Hit::LayoutPick => self.open_menu(g, MenuKind::Layout),
            Hit::ViewPick(_) => self.open_menu(g, MenuKind::View),
            Hit::HiddenToggle => self.toggle_hidden(),
            Hit::PreviewToggle => self.toggle_auto_preview(),
            // 카드는 단추가 아닌 곳을 눌러도 커지고 작아진다
            Hit::CardExpand | Hit::Card => self.toggle_card_large(),
            Hit::CardClose => self.close_card(),
            Hit::PreviewPin => self.toggle_preview_pin(),
            Hit::PreviewOpen => self.open_preview_target(),
            Hit::PreviewArea => {}
            Hit::Tab(p, i) => self.activate_tab(p, i),
            Hit::TabClose(p, i) => self.close_tab(p, i),
            Hit::TabAdd(p) => self.new_tab(p),
            Hit::Back(p) => self.go_back(p),
            Hit::Fwd(p) => self.go_forward(p),
            Hit::Star(p) => self.toggle_star(p),
            Hit::Crumb(p, i) => {
                let Some(r) = self.pane_rects().into_iter().find(|(q, _)| *q == p).map(|(_, r)| geom(r).crumbs) else { return };
                let Some(dir) = self.panes[p].tab().loc.dir().map(str::to_string) else { return };
                if let Some((_, Some(path), _)) = self.crumbs(g, r, &dir).get(i).cloned() {
                    if util::eq_path(&path, &dir) {
                        self.refresh(p);
                    } else {
                        let select = child_toward(&Location::Dir(path.clone()), &Location::Dir(dir));
                        self.navigate(p, Location::Dir(path), select);
                    }
                }
            }
            Hit::Row(p, vi) => {
                if clicks >= 2 {
                    return self.open_row(p, vi);
                }
                let tab = self.panes[p].tab_mut();
                // 여럿을 고른 것 가운데 하나를 그냥 누르면 고른 것을 바로 풀지 않는다: 그대로 끌어 낼 수 있게 두고,
                // 끌지 않고 떼면 그때 그 줄만 고른다 (Finder와 같다)
                let keep = !mods.cmd && !mods.shift && tab.is_selected(vi) && tab.selection_stats().0 > 1;
                match (mods.cmd, mods.shift) {
                    (true, true) => tab.select_range(vi, true),
                    (true, false) => tab.toggle(vi),
                    (false, true) => tab.select_range(vi, false),
                    (false, false) if keep => {
                        tab.cursor = vi;
                        tab.anchor = vi;
                    }
                    (false, false) => tab.select_only(vi),
                }
                let clicked = tab.entry_at(vi).map(|e| Clicked { pane: p, tab: tab.id, file_id: e.file_id, name: e.name_string() });
                // 고른 줄을 누른 채 움직이면 고른 항목들을 끌어 낸다
                if tab.is_selected(vi) {
                    self.press = Some(Press { pane: p, row: vi, x, y, collapse: keep });
                }
                self.clicked = clicked;
            }
            Hit::List(p) => {
                if !mods.cmd {
                    self.panes[p].tab_mut().clear_selection();
                }
            }
            Hit::SplitX | Hit::SplitY | Hit::SplitXY => self.drag = Some(hit),
            Hit::Header(p, key) => self.toggle_sort(p, key),
            Hit::HeaderResize(p, col) => {
                // 선 왼쪽에 보이는 열 (좁은 칸에서는 숨은 열이 있다). 없으면 이름 칸.
                // 끌기는 보이는 폭에서 시작한다 ('자동' 날짜 열은 내용 폭). 끌고 나면 그 폭으로 정해진다
                let Some(list) = self.list_rect(p) else { return };
                let start = self.cols(p);
                let shown = columns(list, start).resizable();
                let prev = (0..col).rev().find(|&i| shown[i].is_some());
                self.col_drag = Some(ColDrag { pane: p, col, prev, start_x: x, start });
            }
            // 시작 화면의 칸·줄: 그곳으로 (⌘-누르기는 새 탭에서)
            Hit::Start(p, i) => {
                if let Some(loc) = self.start_target(p, i) {
                    self.start_go(p, loc, mods.cmd);
                }
            }
            // 검색은 위에서 받았다
            Hit::FindOpen | Hit::FindField | Hit::FindClose | Hit::Find(_) => {}
            // 명령 메뉴도 위에서 받았다
            Hit::CmdRow(_) | Hit::CmdPanel | Hit::CmdScrim => {}
            Hit::None | Hit::Pane(_) | Hit::Menu(_) => {}
        }
    }

    pub fn mouse_dragged(&mut self, x: f32, y: f32) {
        // 사이드바 줄: 위아래로 조금 넘게 움직이면 끌기 시작. 줄에서 잡은 자리를 기억해 카드가 그 자리를 잡은 채 따라오게 한다.
        // 끌 수 없는 줄(바로가기)은 그만큼 움직이면 누르기를 그만둔다 (떼도 열지 않는다)
        if let Some(mut sp) = self.side_press {
            if sp.moving.is_none() && (y - sp.y0).abs() <= DRAG_START {
                return;
            }
            if sp.what.is_none() {
                self.side_press = None;
                return;
            }
            let grab = sp.moving.map_or_else(|| self.side_grab(sp.row, sp.y0), |(_, grab)| grab);
            sp.moving = Some((y, grab));
            self.side_press = Some(sp);
            // 목록 끝에 가까이 끌면 자동 스크롤
            let list = self.side_list();
            if y < list.t + 12.0 {
                self.side_scroll -= 8.0;
            } else if y > list.b - 12.0 {
                self.side_scroll += 8.0;
            }
            self.clamp_side_scroll();
            return;
        }
        // 고른 줄을 누른 채 조금 넘게 움직였다: 고른 항목들을 끌어 내 달라고 한다 (맥의 끌기는 창 쪽이 시작한다)
        if let Some(press) = self.press.as_ref().filter(|press| (x - press.x).hypot(y - press.y) > DRAG_START) {
            let paths = self.panes[press.pane].tab().marked_paths();
            self.press = None;
            if !paths.is_empty() {
                self.drag_out = Some(paths);
            }
            // 이제 누르고 있는 것이 아니다: 누르는 동안 미뤄 둔 것을 한다. 끌려 나가는 것은 위에서 정했으므로 목록이 바뀌어도 된다.
            // 맥의 끌기가 어떻게 끝나든(창 밖에서 취소되면 떼기도 놓기도 오지 않는다) 미뤄 둔 결과가 남지 않게 여기서 한다
            self.after_pointer();
            return;
        }
        // 열 경계: 끄는 선만 따라가고 다른 선은 그대로 (ui::drag_column)
        if let Some(d) = self.col_drag {
            self.panes[d.pane].cols = drag_column(d.start, d.col, d.prev, d.start_x - x);
            return;
        }
        let a = self.area();
        match self.drag {
            Some(Hit::SplitX) => self.split.0 = ((x - GAP / 2.0 - a.l) / (a.w() - GAP)).clamp(0.15, 0.85),
            Some(Hit::SplitY) => self.split.1 = ((y - GAP / 2.0 - a.t) / (a.h() - GAP)).clamp(0.15, 0.85),
            Some(Hit::SplitXY) => {
                self.split = (((x - GAP / 2.0 - a.l) / (a.w() - GAP)).clamp(0.15, 0.85), ((y - GAP / 2.0 - a.t) / (a.h() - GAP)).clamp(0.15, 0.85));
            }
            _ => return,
        }
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
    }

    /// 마우스가 창을 떠났다: 가리키는 것이 없다
    pub fn mouse_left(&mut self) {
        self.hover = Hit::None;
        self.side_hot = false;
    }

    /// 지금 보일 마우스 모양: 끄는 동안은 끄는 것의, 아니면 마우스 아래 것의
    pub fn pointer(&self) -> Pointer {
        if self.col_drag.is_some() {
            return Pointer::LeftRight;
        }
        match self.drag.unwrap_or(self.hover) {
            Hit::HeaderResize(..) | Hit::SplitX => Pointer::LeftRight,
            Hit::SplitY => Pointer::UpDown,
            _ => Pointer::Arrow,
        }
    }

    pub fn mouse_up(&mut self) {
        // 사이드바 줄: 그대로 떼면 열고(그룹 머리는 접고 펼치고), 끌었으면 보이던 빈자리 그대로 옮긴다
        if self.side_press.is_some() {
            let rows = self.side_rows();
            let plan = self.side_drop_plan(&rows);
            if let Some(sp) = self.side_press.take() {
                match plan {
                    Some((dragged, p)) => {
                        self.side_drop(&rows, dragged, p);
                        // 줄이 바뀌었으니 마우스 아래 줄은 다음에 움직일 때 다시 본다 (그 사이 엉뚱한 줄이 강조되지 않게)
                        self.hover = Hit::None;
                    }
                    None if sp.moving.is_none() => self.open_side_row(sp.row),
                    None => {}
                }
            }
        }
        self.drag = None;
        self.col_drag = None;
        // 여럿을 고른 것 가운데 하나를 누르고 끌지 않은 채 뗐다: 그 줄만 고른다
        if let Some(press) = self.press.take().filter(|press| press.collapse) {
            let tab = self.panes[press.pane].tab_mut();
            if press.row < tab.view.len() {
                tab.select_only(press.row);
            }
        }
        // 누르고 있는 동안 미뤄 둔 것
        self.after_pointer();
    }

    /// 마우스 올림이 바뀌었으면 true (다시 그린다)
    pub fn mouse_moved(&mut self, g: &Gfx, x: f32, y: f32) -> bool {
        let h = self.hit(g, x, y);
        let mut changed = h != self.hover;
        self.hover = h;
        // 사이드바 목록 위: '모든 공간' 줄의 새 그룹 단추가 보인다
        let hot = !self.find_shown() && self.side_list().contains(x, y);
        changed |= hot != self.side_hot;
        self.side_hot = hot;
        // 메뉴: 마우스 아래 줄을 가리킨다 (키보드로 가리킨 줄과 같은 표시)
        if let Some(m) = self.menu.as_mut()
            && let Hit::Menu(Some(i)) = h
            && m.sel != Some(i)
        {
            m.sel = Some(i);
            changed = true;
        }
        // 명령 메뉴: 마우스 아래 줄을 고른다 (키보드로 고른 줄과 같은 표시. Windows와 같다)
        if let Some(c) = self.cmd.as_mut()
            && let Hit::CmdRow(i) = h
            && c.sel != i
        {
            c.sel = i;
            changed = true;
        }
        changed
    }

    /// 스크롤 (dy > 0이면 아래 항목이 보이게). 마우스 아래 칸을 움직인다
    pub fn scroll(&mut self, g: &Gfx, x: f32, y: f32, dy: f32) {
        // 메뉴가 열려 있으면 메뉴 위에서만 메뉴를 스크롤한다 (뒤의 칸은 움직이지 않는다)
        if self.menu.is_some() {
            if let Some(r) = self.menu_rect().filter(|r| r.contains(x, y))
                && let Some(m) = self.menu.as_mut()
            {
                m.scroll = (m.scroll + dy).clamp(0.0, menu_max_scroll(r, &m.lines));
            }
            return;
        }
        // 명령 메뉴가 열려 있으면 판 위에서만 줄 목록을 스크롤한다
        if self.cmd.is_some() {
            if self.hit(g, x, y) != Hit::CmdScrim {
                self.cmd_wheel(dy);
            }
            return;
        }
        if self.find_scroll(x, dy) {
            return;
        }
        // 오른쪽 미리 보기 창 위: 글을 스크롤한다 (뒤의 칸은 움직이지 않는다)
        if self.preview_wheel(x, y, dy) {
            return;
        }
        if x < SIDE_W {
            self.side_scroll += dy;
            return self.clamp_side_scroll();
        }
        let p = match self.hit(g, x, y) {
            Hit::Row(p, _) | Hit::List(p) | Hit::Pane(p) | Hit::Start(p, _) => p,
            _ => return,
        };
        // 이름을 고치는 칸은 스크롤하지 않는다 (트랙패드에 손이 스쳐 그 줄이 밀려나면 치던 이름을 잃는다)
        if matches!(&self.edit, Editing::Item(e) if e.pane == p) {
            return;
        }
        self.panes[p].tab_mut().scroll += dy;
        self.clamp_scroll(p);
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.w = w;
        self.h = h;
        for p in 0..self.layout.count() {
            self.clamp_scroll(p);
        }
        self.clamp_side_scroll();
    }

    /// Finder 관례 키. 처리했으면 true
    pub fn key(&mut self, key: Key, m: Mods) -> bool {
        // 메뉴가 열려 있으면 키는 메뉴가 받는다
        if self.menu.is_some() {
            self.menu_key(key);
            return true;
        }
        let p = self.active;
        self.flash = None;
        // 명령 메뉴가 열려 있으면 키는 명령 메뉴가 받는다 (보통은 입력 칸이 받아 넘긴다). 명령 메뉴 키를 다시 누르면 닫는다
        if self.cmd.is_some() {
            if key.combo(m).and_then(|c| self.keys.find(c)) == Some(Action::Commands) {
                self.close_commands();
            } else {
                self.cmd_key(key, m);
            }
            return true;
        }
        // 시작 화면의 키 (화살표, Return, ⌫): 단축키 표보다 먼저 받는다 (Return은 이름 바꾸기의 키다)
        if self.panes[p].tab().loc == Location::Start && self.start_key(p, key, m) {
            return true;
        }
        // 바꿀 수 있는 단축키 (keys.txt): 새 탭, 새 폴더, 보기 방식, 칸 나누기, 이름 바꾸기(Return) 같은 것
        if let Some(a) = key.combo(m).and_then(|c| self.keys.find(c)).filter(|a| mac_runs(*a)) {
            return self.run_action(a);
        }
        // 바꾸지 않는 기본 조작 (Finder와 같다)
        match (key, m.cmd, m.ctrl, m.alt) {
            // 경로 입력의 다른 키: ⌘⇧G(Finder '폴더로 이동')
            (Key::Char('g'), true, false, false) if m.shift => self.begin_address(p),
            (Key::Up | Key::Down | Key::Left | Key::Right, false, false, false) | (Key::Home | Key::End | Key::PageUp | Key::PageDown, false, false, false) => self.move_cursor(key, m.shift),
            (Key::Down, true, false, false) | (Key::Char('o'), true, false, false) => self.open_selected(p),
            // ⌘⌫ 휴지통 (Finder와 같다). 되돌리기(⌘Z)는 편집 메뉴로 온다
            (Key::Delete, true, false, false) => self.trash_selected(p),
            // 복제. 복사(⌘C)와 붙여넣기(⌘V)는 되돌리기처럼 편집 메뉴로 온다
            (Key::Char('d'), true, false, false) => self.duplicate_selected(p),
            // ⌥⌘V: 복사해 둔 것을 복사하지 않고 여기로 옮긴다 (Finder와 같다)
            (Key::Char('v'), true, false, true) => self.ask_paste(true),
            (Key::Up, true, false, false) => self.go_up(p),
            (Key::Char('['), true, false, false) => self.go_back(p),
            (Key::Char(']'), true, false, false) => self.go_forward(p),
            (Key::Tab, false, true, false) => self.cycle_tab(p, m.shift),
            (Key::Tab, false, false, false) => self.cycle_pane(),
            (Key::Char('a'), true, false, false) => self.select_all(),
            // 스페이스: 훑어보기 창이 떠 있으면 닫고, 미리 보기 카드가 떠 있으면 카드를 크게·작게, 아니면 훑어보기를 연다.
            // ⌘Y는 카드와 상관없이 훑어보기를 열고 닫는다 (Finder와 같다)
            (Key::Char(' '), false, false, false) if !self.look_open && self.card().is_some() => self.toggle_card_large(),
            (Key::Char(' '), false, false, false) | (Key::Char('y'), true, false, false) => return self.toggle_look(),
            // Esc: 복사 중이면 그만두고, 아니면 고른 것을 푼다
            (Key::Esc, false, false, false) if self.batch.is_some() => self.cancel_copy(),
            (Key::Esc, false, false, false) => self.clear_selection(),
            _ => return false,
        }
        true
    }

    /// 누르고 있을 때 되풀이해 받아도 되는 키인지. 단축키 표의 동작(새 탭, 새 폴더, 칸 나누기 …)은 한 번 누름에 한 번만 한다
    pub fn key_repeats(&self, key: Key, m: Mods) -> bool {
        match key.combo(m).and_then(|c| self.keys.find(c)).filter(|a| mac_runs(*a)) {
            Some(_) => false,
            None => key.repeats(),
        }
    }

    /// 단축키 표의 동작을 한다 (키, 메뉴 막대). 한 것이 있으면 true
    pub fn run_action(&mut self, a: Action) -> bool {
        let p = self.active;
        self.flash = None;
        // 명령 메뉴가 떠 있는 동안 메뉴 막대로 다른 동작을 하면 명령 메뉴는 닫는다 (그 동작의 입력이 키보드를 받게)
        if a != Action::Commands {
            self.close_commands();
        }
        match a {
            Action::Commands => self.toggle_commands(),
            Action::Search => self.open_find(),
            Action::AddFavorite => self.toggle_pin_selected(),
            Action::NewTab => self.new_tab(p),
            Action::CloseTab => {
                let i = self.panes[p].active;
                self.close_tab(p, i)
            }
            Action::ReopenTab => return self.reopen_tab(),
            Action::NewFolder => self.new_folder(p),
            Action::Hidden => self.toggle_hidden(),
            Action::Preview => self.toggle_preview(),
            Action::Undo => self.undo(),
            Action::Layout1 | Action::Layout2 | Action::Layout3 | Action::Layout4 | Action::Layout5 => {
                let i = [Action::Layout1, Action::Layout2, Action::Layout3, Action::Layout4, Action::Layout5].iter().position(|x| *x == a).unwrap_or(1);
                self.set_layout(Layout::ALL[i])
            }
            Action::Huge => self.set_view(p, ViewMode::Huge),
            Action::Thumbs => self.set_view(p, ViewMode::Large),
            Action::Medium => self.set_view(p, ViewMode::Medium),
            Action::SmallIcons => self.set_view(p, ViewMode::Small),
            Action::List => self.set_view(p, ViewMode::List),
            Action::Details => self.set_view(p, ViewMode::Details),
            Action::Address => self.begin_address(p),
            Action::Rename => self.begin_item_rename(p),
            Action::Refresh => self.refresh(p),
            // 설정 창에 단축키와 도움말이 있다
            Action::Settings | Action::Help => self.open_settings = true,
            Action::Favorites => return false,
        }
        true
    }

    /// 단축키 표 (메뉴 막대와 설정 창이 본다)
    pub fn keymap(&self) -> &Keymap {
        &self.keys
    }

    /// 창 쪽이 읽은 keys.txt를 넣는다
    pub fn set_keymap(&mut self, keys: Keymap) {
        self.keys = keys;
    }

    /// 동작 a의 키를 c로 바꾼다 (None이면 비운다). 다른 동작이 쓰던 키면 그 동작의 키는 비우고 그 동작을 돌려준다. 저장도 한다
    pub fn rebind(&mut self, a: Action, c: Option<Combo>) -> Option<Action> {
        let taken = self.keys.set(a, c);
        self.save_keys();
        taken
    }

    /// 모든 단축키를 기본으로
    pub fn reset_keys(&mut self) {
        self.keys = Keymap::default();
        self.save_keys();
    }

    fn save_keys(&self) {
        if self.persist
            && let Err(e) = self.keys.save()
        {
            sakadu::diag::log(|| format!("keys: {e}"));
        }
    }

    /// 모양 설정 ("system", "light", "dark")
    pub fn theme_pref(&self) -> &str {
        &self.theme_pref
    }

    pub fn set_theme_pref(&mut self, pref: &str) {
        self.theme_pref = if matches!(pref, "light" | "dark") { pref.to_string() } else { "system".to_string() };
    }

    // ---------- 훑어보기 ----------

    /// 훑어보기(Quick Look) 창에 보여 줄 것: 지금 칸에서 고른 항목들(보이는 순서. 고른 것이 없으면 커서 항목)과,
    /// 그 가운데 커서 항목의 번호 (커서 항목이 그 안에 없으면 0)
    pub fn look_items(&self) -> (Vec<String>, usize) {
        let tab = self.panes[self.active].tab();
        let items = tab.selected_paths();
        let cursor = tab.cursor_entry().and_then(|e| tab.path_of(e));
        let at = cursor.and_then(|c| items.iter().position(|p| *p == c)).unwrap_or(0);
        (items, at)
    }

    /// 훑어보기 창을 열어 달라고(떠 있으면 닫아 달라고) 한다. 보여 줄 것이 없으면 아무것도 하지 않고 false
    fn toggle_look(&mut self) -> bool {
        if self.look_open {
            self.look_request = Some(false);
        } else if self.look_items().0.is_empty() {
            return false;
        } else {
            self.look_request = Some(true);
        }
        true
    }

    /// 훑어보기 창을 열지 못했다 (창 쪽이 알려 준다)
    pub fn look_failed(&mut self) {
        self.flash = Some((self.active, "훑어보기를 열 수 없어요".to_string()));
    }

    // ---------- 그리기 ----------

    pub fn paint(&self, g: &mut Gfx) {
        let t = &self.theme;
        g.clear(t.window);
        self.paint_sidebar(g, t);
        self.paint_workbar(g, t);
        for (p, r) in self.pane_rects() {
            self.paint_pane(g, t, p, r);
        }
        if matches!(self.hover, Hit::SplitX | Hit::SplitY | Hit::SplitXY) || self.drag.is_some() {
            let (sx, sy) = self.splitters();
            // 가운데를 끌면 두 선을 함께 밝힌다
            let lines = match self.drag.unwrap_or(self.hover) {
                Hit::SplitX => [sx, None],
                Hit::SplitXY => [sx, sy],
                _ => [sy, None],
            };
            for s in lines.into_iter().flatten() {
                let line = if s.w() < s.h() { R::new(s.l + GAP / 2.0 - 1.0, s.t + 12.0, s.l + GAP / 2.0 + 1.0, s.b - 12.0) } else { R::new(s.l + 12.0, s.t + GAP / 2.0 - 1.0, s.r - 12.0, s.t + GAP / 2.0 + 1.0) };
                g.fill_round(line, 1.0, alpha(t.accent, 0.6));
            }
        }
        self.paint_card(g, t);
        self.paint_preview(g, t);
        // 메뉴는 맨 위에
        if let (Some(m), Some(r)) = (self.menu.as_ref(), self.menu_rect()) {
            paint_menu(g, t, &MenuScene { rect: r, lines: &m.lines, sel: m.sel, scroll: m.scroll, card: true });
        }
        // 명령 메뉴는 창 전체를 덮는다 (메뉴와 함께 떠 있지 않는다)
        self.paint_cmd(g, t);
    }

    /// 미리 보기 카드: 칸들 위에 그린다. 자리와 그리기는 공통 코드, 그림과 글은 이 앱이 읽어 둔 것
    fn paint_card(&self, g: &mut Gfx, t: &Theme) {
        let Some((target, m)) = self.card_shown() else { return };
        let tab = self.panes[self.active].tab();
        let Some(e) = tab.cursor_entry() else { return };
        let name = e.name_string();
        let detail = card_detail(&self.art().type_name(tab, e), e.size);
        // 글(글 파일, 그림이 없는 한글 문서의 미리 보기 글)은 카드에 보이는 줄까지만 넘긴다 (그릴 때마다 80줄을 다 배치하지 않게)
        let mine = self.card.key.as_ref() == Some(&target.key);
        let body = match &self.card.text {
            Some(s) if mine => CardBody::Text(first_lines(s, (m.body.h() / MONO_LINE_H).ceil() as usize)),
            _ => CardBody::Image,
        };
        let hover = match self.hover {
            Hit::CardExpand => CardHover::Expand,
            Hit::CardClose => CardHover::Close,
            _ => CardHover::None,
        };
        let pic = self.pic(&target.key).map(|c| &c.pic);
        // 문서의 썸네일은 흰 종이라 밝은 카드에 묻히지 않게 가는 테두리를 두른다
        let frame = (target.kind == CardKind::Doc).then(|| alpha(t.text, 0.14));
        paint_preview_card(g, &mut CardPic { pic, frame }, t, &CardScene { geom: &m, name: &name, detail: &detail, body, note: self.card_note(&target), large: self.card.large, hover });
    }

    /// 그림이 아직 없을 때 카드에 보일 글: 읽지 못했으면 그렇다고 알리고, 읽는 중이면 0.3초가 지난 뒤부터 알린다
    /// (그림이 곧 오면 글자를 보이지 않는다). 문서는 읽지 못하면 카드가 뜨지 않는다
    fn card_note(&self, target: &CardTarget) -> Option<&'static str> {
        if target.kind == CardKind::Text || self.card.key.as_ref() != Some(&target.key) || self.card.text.is_some() || self.pic(&target.key).is_some() {
            None
        } else if self.card.failed {
            Some("미리 볼 수 없어요")
        } else if self.card.since.elapsed().as_millis() >= LOADING_NOTE_MS as u128 {
            Some("불러오는 중…")
        } else {
            None
        }
    }

    /// 사이드바: 왼쪽에 작업 공간 레일(공통 그리기), 그 옆에 지금 공간의 이름과 고정한 것, 아래에 '모든 공간'
    fn paint_sidebar(&self, g: &Gfx, t: &Theme) {
        if self.standalone {
            traffic_lights(g, WORKBAR_H / 2.0);
        }
        // 레일: 지금 공간의 칸 나누기 그림은 지금 화면을 따른다 (다른 공간은 저장된 화면)
        let rail = self.rail();
        let current = self.workspaces.current.unwrap_or(0);
        let saved_layout = |w: &Workspace| Layout::ALL[(w.state.layout as usize).min(4)];
        let spaces: Vec<RailSpace> =
            self.workspaces.list.iter().enumerate().map(|(i, w)| RailSpace { color: self.space_color(w), layout: if i == current { self.layout } else { saved_layout(w) } }).collect();
        let hover = match self.hover {
            Hit::Space(i) => RailHover::Space(i),
            Hit::SpaceAdd => RailHover::Add,
            _ => RailHover::None,
        };
        paint_rail(g, t, &RailScene { geom: &rail, spaces: &spaces, current, hover, drop: None });
        // 검색하는 동안은 레일 옆이 검색 패널이다
        if self.find_shown() {
            return self.paint_find(g, t);
        }

        // 지금 공간의 이름. 고치는 중이면 입력 상자의 바탕과 테두리만 (글자는 창 쪽 입력 상자가 그린다)
        if self.edit == Editing::Space {
            let er = self.rename_rect();
            g.fill_round(er, R_ROW, t.pane);
            g.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
        } else if let Some(name) = self.workspaces.current_name() {
            let title = self.side_title();
            // 이름은 아래 줄의 아이콘과 같은 세로선에서 시작한다 (Windows 사이드바와 같은 줄 규칙: 여백 8, 아이콘 16, 간격 8)
            g.text(name, R::new(title.l + SP_S, title.t, title.r - SP_S, title.b), Font::Title, t.text);
        }

        let dir = self.panes[self.active].tab().loc.dir();
        let list = self.side_list();
        let rows = self.side_rows();
        let slots: Vec<FlowSlot> = rows.iter().map(SideRow::slot).collect();
        // 끄는 중: 놓일 빈자리와 줄마다 보일 자리 (공통 ui::side_flow). 줄은 지금 자리에서 그 자리로 미끄러져 간다
        let plan = self.side_drop_plan(&rows);
        let targets: Vec<Option<f32>> = match &plan {
            Some((_, p)) => p.targets.clone(),
            None => rows.iter().map(|r| Some(r.y)).collect(),
        };
        let keys: Vec<String> = rows.iter().map(SideRow::key).collect();
        let shown = self.side_flow.borrow_mut().step(&keys, &targets, Instant::now(), self.motion);
        // 목록 좌표를 화면 y로 옮기는 양. 줄의 왼쪽·오른쪽은 side_rect와 같다
        let dy = list.t - self.side_scroll;
        let (row_l, row_r) = (list.l + SP_S, list.r - SP_S);
        g.clip(list);
        // 그룹 판: 끌어 온 줄이 들어갈 그룹(진하게), 끄는 중이 아니면 마우스가 올라간 그룹 머리 줄의 그룹(옅게). Windows와 같다
        let group_box = match &plan {
            Some((_, p)) => p.gap_group.map(|gi| (gi, true)),
            None => match self.hover {
                Hit::SideRow(i) => match rows.get(i).map(|r| r.kind) {
                    Some(SideKind::Group(gi, _)) => Some((gi, false)),
                    _ => None,
                },
                _ => None,
            },
        };
        if let Some((gi, strong)) = group_box {
            let gap = plan.as_ref().filter(|_| strong).map(|(_, p)| (p.gap_t, p.card_h));
            if let Some((top, bot)) = group_span(&slots, &shown, gi, gap) {
                paint_group_plate(g, t, R::new(row_l - SP_XS, top + dy - SP_XS, row_r + SP_XS, bot + dy + SP_XS), strong);
            }
        }
        for (i, row) in rows.iter().enumerate() {
            // 끄는 줄은 카드로 따로 그린다
            let Some(vy) = shown[i] else { continue };
            let r = R::new(row_l, vy + dy, row_r, vy + dy + row.h);
            if r.b < list.t || r.t > list.b {
                continue;
            }
            // 끌려 온 것이 놓일 폴더 줄은 마우스를 올린 것처럼 보인다
            let hovered = self.hover == Hit::SideRow(i) || self.drop == Some(DropAt::Side(i));
            match row.kind {
                SideKind::Hint => {
                    let (l, w) = (r.l + SP_S, r.r - SP_S);
                    g.text("이 공간에 고정한 곳이 없어요", R::new(l, r.t + SP_XS, w, r.t + SP_XS + 16.0), Font::Small, t.faint);
                    g.text("책갈피나 ⌃⌘T로 고정해 보세요", R::new(l, r.t + SP_XS + 16.0, w, r.t + SP_XS + 32.0), Font::Small, t.faint);
                }
                // 구역 이름: 위에 얇은 선, 그 아래 작은 글자. 마우스가 목록 위에 있으면 오른쪽 끝에 새 그룹 단추
                SideKind::Label => {
                    g.fill(R::new(r.l + SP_S, r.t, r.r - SP_S, r.t + 1.0), t.border);
                    g.text(&row.label, R::new(r.l + SP_S, r.t + SP_XS, r.r - SP_S, r.b), Font::Label, t.faint);
                    if let Some(b) = self.new_group_rect(&rows).filter(|_| self.side_hot || self.hover == Hit::NewGroup) {
                        let over = self.hover == Hit::NewGroup;
                        if over {
                            g.fill_round(b, R_CTRL, hover_fill(t));
                        }
                        g.glyph(G_NEW_FOLDER, b, false, if over { t.text } else { t.dim });
                    }
                }
                SideKind::Group(gi, collapsed) => {
                    side_row_bg(g, t, r, false, hovered);
                    g.glyph(if collapsed { G_CHEVRON } else { G_CHEVRON_DOWN }, R::new(r.l + SP_S, r.t, r.l + SP_S + ICON, r.b), true, t.faint);
                    let lx = r.l + SIDE_LABEL_L;
                    if self.edit == Editing::Group(gi) {
                        // 이름을 고치는 중: 입력 상자의 바탕과 테두리만 (글자는 창 쪽 입력 상자가 그린다. 자리는 group_edit_rect와 같다)
                        let er = R::new(lx - SP_XS, r.t + 3.0, r.r - SP_S, r.b - 3.0);
                        g.fill_round(er, R_ROW, t.pane);
                        g.stroke_round(er.inset(0.75, 0.75), R_ROW, t.accent, 1.5);
                    } else {
                        // 그룹 머리는 굵게 (안의 항목과 구분)
                        g.text(&row.label, R::new(lx, r.t, r.r - SP_S, r.b), Font::Strong, t.text);
                    }
                }
                // 고정한 것, 바로가기, 즐겨찾기 항목: 지금 칸의 폴더면 떠 있는 카드
                SideKind::Pin | SideKind::Place | SideKind::Fav => {
                    let here = row.is_dir && row.path.as_deref().zip(dir).is_some_and(|(a, b)| util::eq_path(a, b));
                    side_row_bg(g, t, r, here, hovered);
                    let lx = r.l + SIDE_LABEL_L + if row.indent { INDENT } else { 0.0 };
                    if let Some(path) = &row.path {
                        let it = r.t + (r.h() - ICON) / 2.0;
                        g.icon_for_path(path, R::new(lx - SP_S - ICON, it, lx - SP_S, it + ICON));
                    }
                    g.text(&row.label, R::new(lx, r.t, r.r - SP_S, r.b), Font::Normal, t.text);
                }
            }
        }
        if let Some((dragged, p)) = &plan {
            // 고정한 것이 비어 있는데 그쪽으로 끌어 오면 안내 자리를 강조한다 (빈자리를 보일 줄이 없다)
            if matches!(p.to, DropTo::Pin(_))
                && let Some(hint) = rows.iter().find(|r| r.kind == SideKind::Hint)
            {
                let r = self.side_rect(hint);
                paint_drop_area(g, t, R::new(r.l - SP_HAIR, r.t - SP_HAIR, r.r + SP_HAIR, r.b + SP_HAIR));
            }
            // 끄는 줄: 흰 카드로 떠서 마우스를 따라온다 (아이콘과 이름. 그룹이면 안의 항목 수도)
            let cr = R::new(row_l, p.card_t + dy, row_r, p.card_t + p.card_h + dy);
            paint_drag_card(g, t, cr);
            let label = R::new(cr.l + SIDE_LABEL_L, cr.t, cr.r - SP_S, cr.b);
            match dragged {
                FlowKind::Fav(Row::Group(gi)) => {
                    g.glyph(G_CHEVRON, R::new(cr.l + SP_S, cr.t, cr.l + SP_S + ICON, cr.b), true, t.faint);
                    let name = self.favs.groups.get(*gi).map(|grp| format!("{} · {}", grp.name, grp.items.len())).unwrap_or_default();
                    g.text(&name, label, Font::Strong, t.text);
                }
                _ => {
                    if let Some(row) = rows.iter().find(|r| r.flow == *dragged) {
                        if let Some(path) = &row.path {
                            let it = cr.t + (cr.h() - ICON) / 2.0;
                            g.icon_for_path(path, R::new(cr.l + SP_S, it, cr.l + SP_S + ICON, it + ICON));
                        }
                        g.text(&row.label, label, Font::Normal, t.text);
                    }
                }
            }
        }
        g.unclip();
    }

    /// 칸 영역 위 도구 막대: 뒤로·앞으로, 칸 나누기 단추, 숨김 항목의 눈, 자동 미리 보기 (공통 그리기)
    fn paint_workbar(&self, g: &Gfx, t: &Theme) {
        let m = self.toolbar();
        let p = self.active;
        let tab = self.panes[p].tab();
        let hover = match self.hover {
            Hit::Back(_) => ToolbarHover::Back,
            Hit::Fwd(_) => ToolbarHover::Fwd,
            Hit::LayoutPick => ToolbarHover::Layout,
            Hit::HiddenToggle => ToolbarHover::Hidden,
            Hit::PreviewToggle => ToolbarHover::Preview,
            Hit::FindOpen => ToolbarHover::Search,
            Hit::FindClose => ToolbarHover::SearchClose,
            _ => ToolbarHover::None,
        };
        paint_toolbar(
            g,
            t,
            &ToolbarScene {
                geom: &m,
                layout: self.layout,
                can_back: !tab.back.is_empty(),
                can_fwd: !tab.fwd.is_empty(),
                can_up: can_go_up(&tab.loc),
                hidden_on: self.show_hidden,
                preview_on: self.auto_preview,
                search_on: self.find_shown(),
                search_focus: self.find_wants_keys(),
                layout_open: self.menu_shown().is_some_and(|(k, _)| k == MenuKind::Layout),
                hover,
            },
        );
        self.paint_find_text(g, t);
    }

    /// 칸 하나: 이 앱의 상태로 장면을 채워 공통 칸 그리기에 넘긴다 (맥에 아직 없는 필터는 비워 둔다)
    fn paint_pane(&self, g: &mut Gfx, t: &Theme, p: usize, r: R) {
        let pane = &self.panes[p];
        let tab = pane.tab();
        let active = p == self.active;
        let gm = geom(r);
        let (tabs, add) = self.chips(g, p, &gm);
        // 칩 줄 오른쪽: 항목 수·고른 개수, 잠깐 뜨는 알림이면 그 글을 진하게
        let flash = self.flash.as_ref().filter(|(fp, _)| *fp == p).map(|(_, m)| m.clone());
        // 복사 중이면 그 자리에 진행을 보인다 (받는 폴더를 보이는 칸, 없으면 지금 칸. 잠깐 뜨는 알림이 있으면 그것이 먼저)
        let copying = self.batch.as_ref().filter(|b| self.pane_showing(b.tab, &b.dir).unwrap_or(self.active) == p).and(self.copy_progress());
        // 끌어다 놓은 것은 디스크에 따라 옮기거나 복사한다: 방금 끝낸 항목이 복사였으면 복사하는 중으로 적는다
        let doing = if self.batch.as_ref().is_some_and(|b| b.how != Paste::Copy && b.moved.last() != Some(&false)) { "옮기는 중" } else { "복사하는 중" };
        let flash = flash.or_else(|| copying.map(|(at, total)| format!("{doing} · {total}개 중 {at}개")));
        let count_hot = flash.is_some();
        let count_text = flash.unwrap_or_else(|| count_text(tab));
        let cw = g.measure(&count_text, Font::Small).min(gm.head.w() * 0.6);
        let count_rect = R::new(gm.head.r - SP_S - cw, gm.head.t + SP_S, gm.head.r - SP_S, gm.head.b - SP_S);
        let editing = self.edit == Editing::Address(p);
        // 이 칸에서 이름을 고치는 항목 (커서 줄에 이름 상자가 뜬다)
        let naming = self.item_edit().filter(|(e, _)| e.pane == p);
        let dir = tab.loc.dir();
        let start = tab.loc == Location::Start;
        // 날짜 열 폭을 아직 재지 않았으면 여기서 잰다 (cols가 쓴다)
        self.date_col_w(g);
        let scene = PaneScene {
            r,
            tabs: &pane.tabs,
            current: pane.active,
            sort: pane.sort,
            cols: self.cols(p),
            view: pane.view,
            active,
            multi: self.layout.count() > 1,
            keyboard_here: active,
            // 끌려 온 것이 탭 칩 위에 있으면 그 칩을 마우스를 올린 것처럼 보인다 (칸 테두리도 강조한다: 그 칸의 탭으로 간다)
            hover: match self.drop {
                Some(DropAt::Tab(q, i)) if q == p => PaneHover::Tab(i),
                _ => pane_hover(self.hover, p),
            },
            chips: tabs,
            add,
            count_text,
            count_rect,
            count_hot,
            filter: None,
            starred: dir.map(|d| self.is_marked(d)),
            // 보기 메뉴는 지금 칸에 대해 열린다. 시작 화면에는 보기 방식 단추가 없다
            view_button: (!start).then(|| active && self.menu_shown().is_some_and(|(k, _)| k == MenuKind::View)),
            root: None,
            address: editing,
            // 창에서는 입력 상자가 글자를 그린다. 그림 시험에서만 직접 그린다
            address_text: dir.filter(|_| editing && self.standalone),
            renaming: naming.is_some(),
            dragged_tab: None,
            tab_drop: None,
            drop_on_pane: matches!(self.drop, Some(DropAt::Pane(q) | DropAt::Tab(q, _)) if q == p),
            drop_row: match self.drop {
                Some(DropAt::Row(q, vi)) if q == p => Some(vi),
                _ => None,
            },
            marquee: None,
            scroll_drag: false,
            hint: list_hint(tab),
            net_button: None,
            loading: None,
            short_dates: true,
            now: listing::now_filetime(),
        };
        paint_pane_head(g, t, &scene);
        // 시작 화면: 머리글·목록 자리에 카드와 줄
        if self.paint_start_view(g, t, p) {
            return paint_pane_border(g, t, &scene);
        }
        paint_pane_list(g, &mut self.art(), t, &scene);
        // 창에서는 입력 상자가 고치는 이름을 그린다. 그림 시험에서만 직접 그린다: 이름과, 미리 골라 둔 앞부분의 바탕
        if let (true, Some((e, r))) = (self.standalone, naming) {
            let stem: String = String::from_utf16_lossy(&e.name.encode_utf16().take(sakadu::fileops::stem_units(&e.name, e.is_dir)).collect::<Vec<_>>());
            let x = r.l + 4.0;
            g.fill(R::new(x, r.t + 3.0, (x + g.measure(&stem, Font::Normal)).min(r.r - 4.0), r.b - 3.0), alpha(t.accent, 0.28));
            g.text(&e.name, R::new(x, r.t, r.r - 4.0, r.b), Font::Normal, t.text);
        }
        paint_pane_border(g, t, &scene);
    }

    fn art(&self) -> MacArt<'_> {
        MacArt { kinds: &self.kinds, thumbs: &self.thumbs, scale: self.scale }
    }
}

/// 칸 p 안에서 마우스가 올라간 곳 (공통 칸 그리기에 넘긴다)
fn pane_hover(h: Hit, p: usize) -> PaneHover {
    match h {
        Hit::Tab(q, i) if q == p => PaneHover::Tab(i),
        Hit::TabClose(q, i) if q == p => PaneHover::TabClose(i),
        Hit::TabAdd(q) if q == p => PaneHover::TabAdd,
        Hit::Star(q) if q == p => PaneHover::Star,
        Hit::ViewPick(q) if q == p => PaneHover::ViewPick,
        Hit::Crumb(q, i) if q == p => PaneHover::Crumb(i),
        Hit::CrumbEmpty(q) if q == p => PaneHover::CrumbEmpty,
        Hit::Row(q, vi) if q == p => PaneHover::Row(vi),
        Hit::Header(q, key) if q == p => PaneHover::Header(key),
        _ => PaneHover::None,
    }
}

/// 빈 목록 안내 (Windows와 같은 글, 다시 시도는 ⌘R)
fn list_hint(tab: &Tab) -> Option<String> {
    match &tab.state {
        LoadState::Loading if tab.entries.is_empty() => Some(format!("불러오는 중…\n{}", tab.loc.title())),
        LoadState::Error(msg) => Some(format!("폴더를 열 수 없어요\n{msg}\n⌘R을 누르면 다시 시도해요")),
        LoadState::Ready if tab.entries.is_empty() => Some("빈 폴더예요".to_string()),
        LoadState::Ready if tab.view.is_empty() => Some("숨김 항목만 있어요".to_string()),
        _ => None,
    }
}

/// 칸 그리기의 맥 갈고리: Finder 아이콘, 읽어 둔 썸네일, '종류' 이름
struct MacArt<'a> {
    kinds: &'a RefCell<HashMap<String, String>>,
    thumbs: &'a Thumbs,
    scale: f32,
}

impl EntryArt<Gfx> for MacArt<'_> {
    fn icon(&mut self, g: &mut Gfx, tab: &Tab, e: &Entry, r: R) {
        if let Some(path) = tab.path_of(e) {
            g.icon_for_path(&path, r);
        }
    }

    /// 파일이면 읽어 둔 썸네일을 그린다. 아직 없으면(읽는 중이거나, 읽지 못했거나 썸네일이 없는 파일이면) false: 파일 아이콘이 대신 보인다.
    /// 여기서는 파일을 읽지 않는다: 없는 것은 App::take_thumbs가 지금 보이는 것 가운데서 찾아 뒷줄에 맡긴다
    fn thumbnail(&mut self, g: &mut Gfx, tab: &Tab, e: &Entry, bx: R, size: f32, opacity: f32) -> bool {
        let Some(pic) = thumb_key(tab, e, size, self.scale).and_then(|k| self.thumbs.get(&k)) else { return false };
        g.picture_alpha(pic, bx, opacity);
        true
    }

    fn type_name(&mut self, tab: &Tab, e: &Entry) -> String {
        finder_kind(self.kinds, &e.name_string(), e.kind == Kind::Dir, || tab.path_of(e))
    }
}

/// Finder '종류' 이름 ("PDF 문서", "폴더"). 확장자가 같으면 같은 종류로 보고 kinds에 한 번만 묻는다
/// (확장자 없는 파일은 파일마다 다를 수 있어 매번 묻는다). path는 물어야 할 때만 부른다
fn finder_kind(kinds: &RefCell<HashMap<String, String>>, name: &str, dir: bool, path: impl FnOnce() -> Option<String>) -> String {
    let ext = name.rsplit_once('.').map(|(_, x)| x.to_lowercase()).filter(|x| !x.is_empty());
    let key = ext.as_ref().map(|x| if dir { format!("/{x}") } else { x.clone() });
    let key = key.or_else(|| dir.then(|| "/".to_string()));
    if let Some(k) = &key
        && let Some(v) = kinds.borrow().get(k)
    {
        return v.clone();
    }
    let v = path().and_then(|p| Gfx::type_description(&p)).unwrap_or_default();
    if let Some(k) = key {
        kinds.borrow_mut().insert(k, v.clone());
    }
    v
}

/// 카드 그리기의 맥 갈고리: 읽어 둔 그림을 그린다 (아직 없으면 그리지 않는다). frame이 있으면 그림 둘레에 그 색 테두리
struct CardPic<'a> {
    pic: Option<&'a Picture>,
    frame: Option<Color>,
}

impl CardArt<Gfx> for CardPic<'_> {
    fn image(&mut self, g: &mut Gfx, r: R) -> bool {
        let Some(pic) = self.pic else { return false };
        g.picture(pic, r);
        if let Some(c) = self.frame {
            g.stroke_round(Gfx::fit(pic, r).inset(-0.5, -0.5), 2.0, c, 1.0);
        }
        true
    }
}

/// 칩 줄 오른쪽 글: 항목 수, 고른 것이 있으면 고른 개수·크기. 시작 화면은 비워 둔다 (Windows와 같다)
fn count_text(tab: &Tab) -> String {
    if tab.loc == Location::Start {
        return String::new();
    }
    let (n, size) = tab.selection_stats();
    if n > 0 {
        let s = if size > 0 { format!(" · {}", listing::format_size(size)) } else { String::new() };
        format!("{}개 중 {}개 선택{}", tab.view.len(), n, s)
    } else {
        format!("{}개", tab.view.len())
    }
}

/// 탭의 스크롤을 내용 길이 안으로 (보기 방식의 칸 배치에 맞춰)
fn clamp_tab(tab: &mut Tab, list: R, view: ViewMode) {
    let grid = Grid::of(list, view);
    let max = (grid.content(tab.view.len()) - grid.page(list)).max(0.0);
    tab.scroll = tab.scroll.clamp(0.0, max);
}

/// 커서 항목이 보이게 탭을 스크롤한다
fn reveal(tab: &mut Tab, list: R, view: ViewMode) {
    let grid = Grid::of(list, view);
    // 스크롤 0 기준 위치 (스크롤 방향으로)
    let it = grid.item(tab.cursor, 0.0);
    let (top, bottom) = if grid.horiz { (it.l - list.l, it.r - list.l + 4.0) } else { (it.t - list.t, it.b - list.t) };
    let page = grid.page(list);
    if top < tab.scroll {
        tab.scroll = top;
    } else if bottom > tab.scroll + page {
        tab.scroll = bottom - page;
    }
    clamp_tab(tab, list, view);
}

/// 맥 창의 닫기·최소화·확대 단추 (그림 시험용. 실제 창에서는 시스템이 그린다)
fn traffic_lights(g: &Gfx, cy: f32) {
    for (i, hex) in [0xFF5F57u32, 0xFEBC2E, 0x28C840].into_iter().enumerate() {
        let cx = 20.0 + i as f32 * 20.0;
        let c = Color { r: ((hex >> 16) & 0xFF) as f32 / 255.0, g: ((hex >> 8) & 0xFF) as f32 / 255.0, b: (hex & 0xFF) as f32 / 255.0, a: 1.0 };
        let r = R::new(cx - 6.0, cy - 6.0, cx + 6.0, cy + 6.0);
        g.fill_round(r, 6.0, c);
        g.stroke_round(r.inset(0.25, 0.25), 6.0, Color { r: 0.0, g: 0.0, b: 0.0, a: 0.12 }, 0.5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sakadu::favorites::Group;

    /// 시험용 폴더: 하위 폴더 둘, 파일 셋, 숨김 파일 하나
    fn demo() -> (std::path::PathBuf, String) {
        let root = std::env::temp_dir().join(format!("sakadu-mac-app-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("업무").join("2026 4분기")).unwrap();
        std::fs::create_dir_all(root.join("다운로드")).unwrap();
        for f in ["업무/예산.csv", "업무/회의록.md", "업무/2026 4분기/보고서.docx", "다운로드/사진.jpg", ".숨김"] {
            std::fs::write(root.join(f), b"x").unwrap();
        }
        let path = util::normalize(&std::fs::canonicalize(&root).unwrap().to_string_lossy());
        (root, path)
    }

    fn app(home: &str) -> (App, Gfx) {
        let mut a = App::new(1280.0, 800.0, Theme::LIGHT, home, Favorites::default(), Vec::new());
        // 새 탭은 지금 폴더로 (시작 화면은 따로 시험한다)
        a.set_new_tab_start(false);
        a.settle();
        (a, Gfx::bitmap(8.0, 8.0, 1.0).unwrap())
    }

    /// 칸 p의 줄 vi 가운데 좌표
    fn row_point(a: &App, p: usize, vi: usize) -> (f32, f32) {
        let l = a.list_rect(p).unwrap();
        (l.l + 60.0, l.t + vi as f32 * ROW_H + ROW_H / 2.0 - a.pane(p).tab().scroll)
    }

    fn names(a: &App, p: usize) -> Vec<String> {
        let t = a.pane(p).tab();
        t.view.iter().map(|&i| t.entries[i as usize].name_string()).collect()
    }

    /// 사이드바에서 그 이름을 가진 줄의 자리
    fn side_row(a: &App, label: &str) -> R {
        let rows = a.side_rows();
        a.side_rect(rows.iter().find(|r| r.label == label).unwrap_or_else(|| panic!("no sidebar row named {label}")))
    }

    /// 지금 공간에 고정한 것의 이름
    fn pin_names(a: &App) -> Vec<String> {
        a.workspaces().current().unwrap().pins.iter().map(|f| f.name.clone()).collect()
    }

    /// 열린 메뉴(종류와 붙인 자리)를 보고 닫는다
    fn take_menu(a: &mut App) -> Option<(MenuKind, R)> {
        let m = a.menu_shown();
        a.close_menu();
        m
    }

    fn mid(r: R) -> (f32, f32) {
        ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0)
    }

    #[test]
    fn opens_folders_and_goes_back_up_with_the_child_selected() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        assert_eq!(names(&a, 0), ["다운로드", "업무"], "folders first, hidden file not shown");
        let (x, y) = row_point(&a, 0, 1);
        a.mouse_down(&g, x, y, Mods::default(), 2);
        a.settle();
        assert_eq!(names(&a, 0), ["2026 4분기", "예산.csv", "회의록.md"]);
        assert_eq!(a.title(), "업무 - Sakadu");
        a.go_up(0);
        a.settle();
        let t = a.pane(0).tab();
        assert_eq!(t.cursor_entry().unwrap().name_string(), "업무", "the folder we left is selected");
        a.go_back(0);
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(util::join(&home, "업무")));
        a.go_back(0);
        a.settle();
        assert_eq!(a.pane(0).tab().cursor_entry().unwrap().name_string(), "업무");
        // 파일을 두 번 누르면 macOS '열기'로 넘긴다
        a.navigate(0, Location::Dir(util::join(&home, "업무")), None);
        a.settle();
        let (x, y) = row_point(&a, 0, 1);
        a.mouse_down(&g, x, y, Mods::default(), 2);
        assert_eq!(a.open_files, [util::join(&util::join(&home, "업무"), "예산.csv")]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selection_follows_cmd_and_shift() {
        let (root, home) = demo();
        let (mut a, g) = app(&util::join(&home, "업무"));
        let click = |a: &mut App, vi: usize, m: Mods| {
            let (x, y) = row_point(a, 0, vi);
            a.mouse_down(&g, x, y, m, 1);
        };
        click(&mut a, 0, Mods::default());
        click(&mut a, 2, Mods { shift: true, ..Default::default() });
        assert_eq!(a.pane(0).tab().selection_stats().0, 3);
        click(&mut a, 1, Mods { cmd: true, ..Default::default() });
        assert_eq!(a.pane(0).tab().selection_mask(), [true, false, true]);
        assert!(a.key(Key::Esc, Mods::default()));
        assert_eq!(a.pane(0).tab().selection_stats().0, 0);
        assert!(a.key(Key::Char('a'), Mods { cmd: true, ..Default::default() }));
        assert_eq!(a.pane(0).tab().selection_stats().0, 3);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tabs_layouts_hidden_and_stars() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        assert!(a.key(Key::Char('t'), Mods { cmd: true, ..Default::default() }));
        a.settle();
        assert_eq!((a.pane(0).tabs.len(), a.pane(0).active), (2, 1));
        assert!(a.key(Key::Char('w'), Mods { cmd: true, ..Default::default() }));
        assert_eq!(a.pane(0).tabs.len(), 1);
        a.key(Key::Char('w'), Mods { cmd: true, ..Default::default() });
        assert_eq!(a.pane(0).tabs.len(), 1, "the last tab stays");
        a.key(Key::Char('4'), Mods { cmd: true, alt: true, ..Default::default() });
        a.settle();
        assert_eq!(a.layout(), Layout::Three);
        a.key(Key::Char('5'), Mods { cmd: true, alt: true, ..Default::default() });
        a.settle();
        assert_eq!(a.layout(), Layout::Four);
        assert!(names(&a, 3).contains(&"업무".to_string()), "a pane shown for the first time is loaded");
        a.key(Key::Tab, Mods::default());
        assert_eq!(a.active(), 1);
        a.key(Key::Char('.'), Mods { cmd: true, shift: true, ..Default::default() });
        assert!(names(&a, 0).contains(&".숨김".to_string()));
        // 책갈피: 지금 칸 폴더를 이 공간에 고정하고, 사이드바의 그 줄을 누르면 그 폴더로
        a.navigate(1, Location::Dir(util::join(&home, "업무")), None);
        a.settle();
        a.toggle_star(1);
        assert_eq!(pin_names(&a), ["업무"]);
        assert!(a.favorites().items.is_empty(), "the bookmark pins to this workspace, not to favorites");
        a.navigate(1, Location::Dir(util::join(&home, "다운로드")), None);
        a.settle();
        let r = side_row(&a, "업무");
        a.mouse_down(&g, r.l + 40.0, r.t + 10.0, Mods::default(), 1);
        a.mouse_up();
        a.settle();
        assert_eq!(a.pane(1).tab().loc, Location::Dir(util::join(&home, "업무")));
        a.toggle_star(1);
        assert!(pin_names(&a).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn split_drag_and_scroll_stay_in_range() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let (sx, _) = a.splitters();
        let s = sx.unwrap();
        a.mouse_down(&g, s.l + 2.0, s.t + 50.0, Mods::default(), 1);
        a.mouse_dragged(10_000.0, 0.0);
        a.mouse_up();
        assert!((a.split.0 - 0.85).abs() < 1e-4);
        // 4칸의 가운데를 끌면 두 선이 함께 움직인다
        a.set_layout(Layout::Four);
        let c = split_cross(a.area(), a.split, a.layout).unwrap();
        let (cx, cy) = ((c.l + c.r) / 2.0, (c.t + c.b) / 2.0);
        assert_eq!(a.hit(&g, cx, cy), Hit::SplitXY);
        a.mouse_down(&g, cx, cy, Mods::default(), 1);
        a.mouse_dragged(-10_000.0, -10_000.0);
        a.mouse_up();
        assert!((a.split.0 - 0.15).abs() < 1e-4 && (a.split.1 - 0.15).abs() < 1e-4, "{:?}", a.split);
        a.set_layout(Layout::TwoV);
        let (x, y) = row_point(&a, 0, 0);
        a.scroll(&g, x, y, 500.0);
        assert_eq!(a.pane(0).tab().scroll, 0.0, "a short list does not scroll");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 단축키 표: 기본 키(Finder 관례)로 동작을 하고, 키를 바꾸면 새 키로만 하며, 기본 조작 키는 단축키로 정하지 않는다
    #[test]
    fn keymap_runs_actions_and_can_be_rebound() {
        let (root, home) = demo();
        let (mut a, _) = app(&util::join(&home, "업무"));
        let cmd = Mods { cmd: true, ..Default::default() };
        let cmd_shift = Mods { cmd: true, shift: true, ..Default::default() };
        // ⌘T 새 탭, ⌘W 닫기, ⇧⌘T 닫은 탭 다시 열기
        assert!(a.key(Key::Char('t'), cmd));
        a.settle();
        assert_eq!(a.pane(0).tabs.len(), 2);
        a.navigate(0, Location::Dir(util::join(&home, "다운로드")), None);
        a.settle();
        assert!(a.key(Key::Char('w'), cmd));
        assert_eq!(a.pane(0).tabs.len(), 1);
        assert!(a.key(Key::Char('t'), cmd_shift));
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(util::join(&home, "다운로드")), "the closed tab comes back");
        assert!(!a.key(Key::Char('t'), cmd_shift) || a.pane(0).tabs.len() == 2, "nothing more to reopen");
        // ⌘2 큰 아이콘, ⌘6 자세히, ⌥⌘1 한 칸
        assert!(a.key(Key::Char('2'), cmd));
        assert_eq!(a.pane(0).view, ViewMode::Large);
        assert!(a.key(Key::Char('6'), cmd));
        assert_eq!(a.pane(0).view, ViewMode::Details);
        assert!(a.key(Key::Char('1'), Mods { cmd: true, alt: true, ..Default::default() }));
        assert_eq!(a.layout(), Layout::One);
        // Return 이름 바꾸기 (줄을 하나 고른 뒤. 다시 연 '다운로드' 탭에는 파일이 하나뿐이다)
        a.tab_mut(0).select_only(0);
        assert!(a.key(Key::Return, Mods::default()));
        assert!(matches!(a.edit, Editing::Item(_)));
        a.edit = Editing::None;
        // 새 탭을 ⌘J로 바꾸면 ⌘J로만 연다 (⌘T는 이제 아무것도 아니다). 다른 동작이 쓰던 키면 그 동작은 비운다
        let cmd_j = Combo::parse("Cmd+J").unwrap();
        assert_eq!(a.rebind(Action::NewTab, Some(cmd_j)), None);
        let tabs = a.pane(0).tabs.len();
        assert!(!a.key(Key::Char('t'), cmd));
        assert!(a.key(Key::Char('j'), cmd));
        assert_eq!(a.pane(0).tabs.len(), tabs + 1);
        assert_eq!(a.rebind(Action::Refresh, Some(cmd_j)), Some(Action::NewTab));
        assert_eq!(a.keymap().get(Action::NewTab), None);
        a.reset_keys();
        assert_eq!(a.keymap().get(Action::NewTab), Combo::parse("Cmd+T"));
        // 누르고 있어도 동작은 한 번 (새 폴더), 화살표는 되풀이한다
        assert!(!a.key_repeats(Key::Char('n'), cmd_shift));
        assert!(a.key_repeats(Key::Down, Mods::default()));
        // 기본 조작(⌘O 열기, ⌘[ 뒤로, ⇧⌘Z 실행 복귀)은 단축키로 정하지 않는다
        for s in ["Cmd+O", "Cmd+[", "Shift+Cmd+Z", "Cmd+Up"] {
            assert!(fixed_key(Combo::parse(s).unwrap()), "{s}");
        }
        assert!(!fixed_key(cmd_j));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 모양 설정은 세션에 남고 다음에 열 때 돌아온다. 작업 공간을 오가도 바뀌지 않는다 (앱 전체의 것)
    #[test]
    fn theme_preference_is_saved_and_survives_workspace_switches() {
        let (root, home) = demo();
        let (mut a, _) = app(&home);
        assert_eq!(a.theme_pref(), "system");
        a.set_theme_pref("dark");
        assert_eq!(a.prefs().theme, "dark");
        let s = a.to_session();
        assert_eq!(s.look.theme, "dark");
        let (mut b, _) = app(&home);
        b.open_saved(Workspaces::default(), Some(&s), &home);
        assert_eq!(b.theme_pref(), "dark");
        b.new_workspace();
        b.edit = Editing::None;
        b.set_theme_pref("light");
        b.switch_workspace(0);
        assert_eq!(b.theme_pref(), "light", "switching workspaces keeps the app's look");
        b.set_theme_pref("sepia");
        assert_eq!(b.theme_pref(), "system", "an unknown value means follow the Mac");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 최근 위치: 다녀간 폴더가 맨 앞에(뒤로 가기도), 같은 폴더는 한 번만. 열지 못한 폴더는 빠진다.
    /// 세션에 남아 다음에 열 때 돌아오고, 작업 공간을 오가도 그대로다 (앱 전체의 것)
    #[test]
    fn recent_places_are_recorded_saved_and_restored() {
        let (root, home) = demo();
        let (mut a, _) = app(&home);
        assert!(a.recent.is_empty(), "opening the window records nothing");
        let work = util::join(&home, "업무");
        let down = util::join(&home, "다운로드");
        a.navigate(0, Location::Dir(work.clone()), None);
        a.navigate(0, Location::Dir(down.clone()), None);
        a.settle();
        assert_eq!(a.recent, [down.clone(), work.clone()]);
        a.go_back(0);
        a.settle();
        assert_eq!(a.recent, [work.clone(), down.clone()], "going back puts that folder first again, once");
        let gone = util::join(&home, "없는 곳");
        a.navigate(1, Location::Dir(gone.clone()), None);
        assert_eq!(a.recent[0], gone);
        a.settle();
        assert!(!a.recent.contains(&gone), "a folder that is not there is dropped");
        let s = session::parse(&session::serialize(&a.to_session())).expect("session");
        assert_eq!(s.recent, [work.clone(), down.clone()]);
        let (mut b, _) = app(&home);
        b.open_saved(Workspaces::default(), Some(&s), &home);
        assert_eq!(b.recent, [work.clone(), down.clone()]);
        b.new_workspace();
        b.edit = Editing::None;
        b.switch_workspace(0);
        assert_eq!(b.recent, [work, down], "switching workspaces keeps the list");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// ⌘T는 '새 탭' 설정이 시작 화면이면(기본) 시작 화면 탭을 연다: 고정한 폴더 카드, 최근 위치(고정한 것은 빼고), 위치.
    /// 칸을 누르면 그 폴더로 가고 뒤로 가면 시작 화면이다. 화살표는 처음에 테두리만 보이고, 다음부터 옮기며, Return으로 연다.
    /// 설정과 시작 화면 탭은 세션에 남는다. '지금 폴더'로 바꾸면 ⌘T가 지금 폴더를 연다
    #[test]
    fn cmd_t_opens_the_start_screen_and_its_cards_navigate() {
        use sakadu::ui::start::StartItem;
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let down = util::join(&home, "다운로드");
        let mut a = App::new(1280.0, 800.0, Theme::LIGHT, &home, Favorites::default(), vec![("다운로드".to_string(), down.clone())]);
        let g = Gfx::bitmap(8.0, 8.0, 1.0).unwrap();
        a.settle();
        assert!(a.prefs().new_tab_start, "the start screen is the default");
        a.navigate(1, Location::Dir(down.clone()), None);
        a.navigate(0, Location::Dir(work.clone()), None);
        a.settle();
        a.toggle_star(0);
        let cmd = Mods { cmd: true, ..Default::default() };
        assert!(a.key(Key::Char('t'), cmd));
        a.settle();
        assert_eq!((a.pane(0).tabs.len(), a.pane(0).tab().loc.clone()), (2, Location::Start));
        assert!(a.seen().start);
        assert_eq!(a.title(), "새 탭 - Sakadu");
        let v = a.start_view(0).expect("the start tab is laid out");
        let items: Vec<StartItem> = v.items.iter().map(|(it, _)| it.clone()).collect();
        assert_eq!(items, [StartItem::Fav { name: "업무".into(), path: work.clone() }, StartItem::Recent(down.clone()), StartItem::Drive(0)]);
        // 세션: 새 탭 설정과 시작 화면 탭
        let s = session::parse(&session::serialize(&a.to_session())).expect("session");
        assert_eq!((s.new_tab.as_str(), s.panes[0].tabs[1].as_str()), (session::NEW_TAB_START, "::start"));
        let (mut b, _) = app(&home);
        b.open_saved(Workspaces::default(), Some(&s), &home);
        b.settle();
        assert_eq!(b.pane(0).tab().loc, Location::Start, "the start tab comes back as a start tab");
        assert!(b.prefs().new_tab_start && b.pane(0).tab().state == LoadState::Ready);
        // 고정한 폴더 칸을 누르면 그리로, 뒤로 가면 시작 화면
        let (x, y) = mid(v.items[0].1);
        assert_eq!(a.hit(&g, x, y), Hit::Start(0, 0));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(work.clone()));
        // 둘째 누름(두 번 누르기)은 새로 연 목록의 항목을 열지 않는다
        a.mouse_down(&g, x, y, Mods::default(), 2);
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(work.clone()));
        a.go_back(0);
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Start);
        // 화살표: 처음 ↓는 첫 칸에 테두리만, 다음 ↓는 아래 줄(최근 위치), Return은 그 폴더로. Return은 이름 바꾸기가 아니다
        assert!(a.key(Key::Down, Mods::default()));
        assert_eq!((a.start_keys, a.pane(0).tab().cursor), (true, 0));
        assert!(a.key(Key::Down, Mods::default()));
        assert_eq!(a.pane(0).tab().cursor, 1);
        assert!(a.key(Key::Return, Mods::default()));
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(down.clone()));
        assert_eq!(a.edit, Editing::None);
        // ⌫는 테두리가 있는 최근 위치 줄을 목록에서 뺀다
        a.go_back(0);
        a.settle();
        a.key(Key::Down, Mods::default());
        a.key(Key::Down, Mods::default());
        assert!(a.key(Key::Delete, Mods::default()));
        assert!(!a.recent.contains(&down), "{:?}", a.recent);
        // '지금 폴더'면 ⌘T가 지금 폴더를 연다
        a.set_new_tab_start(false);
        a.activate_pane(1);
        a.key(Key::Char('t'), cmd);
        assert_eq!(a.pane(1).tab().loc, Location::Dir(down));
        assert_eq!(a.to_session().new_tab, session::NEW_TAB_SAME);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// ⌘K 명령 메뉴: 치기 전에는 열린 탭·최근 위치·명령(명령 메뉴 자신은 빼고). 이름 앞부분이 맞는 것이 먼저이고 초성으로도 찾으며,
    /// 자모로 나뉜(맥이 만든 NFD) 폴더 이름도 찾는다. ↓와 Return으로 고른 명령을 하고, 줄을 누르면 그것을 열고, 판 밖을 누르면 닫는다.
    /// 경로를 치면 맨 위 줄로 그 경로에 간다 (⌘Return은 새 탭)
    #[test]
    fn command_menu_ranks_runs_and_goes_to_paths() {
        let (root, home) = demo();
        // '보고서'를 자모로 나눈 이름 (ᄇ ᅩ ᄀ ᅩ ᄉ ᅥ)
        let nfd = "\u{1107}\u{1169}\u{1100}\u{1169}\u{1109}\u{1165}";
        std::fs::create_dir_all(std::path::Path::new(&home).join(nfd)).unwrap();
        let (mut a, g) = app(&home);
        let report = util::join(&home, nfd);
        a.navigate(1, Location::Dir(report.clone()), None);
        a.settle();
        let cmd = Mods { cmd: true, ..Default::default() };
        assert!(a.key(Key::Char('k'), cmd));
        let (text, names, sel) = a.cmd_seen().expect("cmd-K opens the command menu");
        assert_eq!((text.as_str(), sel), ("", 0));
        assert!(names.contains(&"새 탭".to_string()) && names.contains(&nfd.to_string()));
        assert!(!names.contains(&"명령 메뉴".to_string()) && !names.contains(&"사이드바 목록으로 이동".to_string()), "only actions the Mac runs");
        assert!(names.contains(&"미리 보기 창".to_string()));
        // 완성된 글자로도, 초성으로도 나뉜 이름을 찾는다 (탭이 먼저, 그다음 최근 위치, 마지막에 디스크에서 찾기)
        a.cmd_typed("보고");
        assert_eq!(a.cmd_seen().unwrap().1, [nfd, nfd, "‘보고’ 디스크에서 찾기"]);
        a.cmd_typed("ㅂㄱㅅ");
        assert_eq!(a.cmd_seen().unwrap().1, [nfd, nfd, "‘ㅂㄱㅅ’ 디스크에서 찾기"]);
        // 줄을 누르면 그 탭으로 (오른쪽 칸)
        a.activate_pane(0);
        let list = sakadu::ui::commands::cmd_list(a.cmd_rect().unwrap());
        let (x, y) = (list.l + 60.0, list.t + sakadu::ui::commands::CMD_ROW_H / 2.0);
        assert_eq!(a.hit(&g, x, y), Hit::CmdRow(0));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert!(a.cmd_seen().is_none(), "opening a line closes the menu");
        assert_eq!(a.active(), 1);
        // 앞부분이 맞는 것이 먼저: '보기'는 모두 가운데가 맞아 표의 순서대로. ↓ 두 번과 Return은 '큰 아이콘 보기'
        a.run_action(Action::Commands);
        a.cmd_typed("보기");
        let names = a.cmd_seen().unwrap().1;
        assert_eq!(names[..3], ["숨김 항목 보기", "아주 큰 아이콘 보기", "큰 아이콘 보기"]);
        a.key(Key::Down, Mods::default());
        a.cmd_key(Key::Down, Mods::default());
        assert_eq!(a.cmd_seen().unwrap().2, 2);
        assert!(a.cmd_key(Key::Return, Mods::default()));
        assert_eq!((a.pane(1).view, a.cmd_seen().is_none()), (ViewMode::Large, true));
        // ↑는 맨 위에서 맨 아래로, Esc와 판 밖 누르기는 닫는다
        a.toggle_commands();
        a.cmd_key(Key::Up, Mods::default());
        let (_, names, sel) = a.cmd_seen().unwrap();
        assert_eq!(sel, names.len() - 1);
        a.cmd_key(Key::Esc, Mods::default());
        assert!(a.cmd_seen().is_none());
        a.toggle_commands();
        a.mouse_down(&g, 2.0, 2.0, Mods::default(), 1);
        assert!(a.cmd_seen().is_none(), "a press outside the panel closes it");
        // 경로: 맨 위 줄이 그 경로로 가는 줄이다. Return은 이 탭에서, ⌘Return은 새 탭에서 (경로 입력과 같이 뒷줄에서 확인한다)
        let work = util::join(&home, "업무");
        a.toggle_commands();
        a.cmd_typed(&work);
        assert_eq!(a.cmd_seen().unwrap().1[0], work);
        a.cmd_key(Key::Return, Mods::default());
        a.settle();
        assert_eq!(a.pane(1).tab().loc, Location::Dir(work.clone()));
        let tabs = a.pane(1).tabs.len();
        let down = util::join(&home, "다운로드");
        a.toggle_commands();
        a.cmd_typed(&down);
        a.cmd_key(Key::Return, cmd);
        a.settle();
        assert_eq!((a.pane(1).tabs.len(), a.pane(1).tab().loc.clone()), (tabs + 1, Location::Dir(down)));
        // 경로·이름을 고치는 중에는 열지 않는다
        a.begin_address(1);
        a.run_action(Action::Commands);
        assert!(a.cmd_seen().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 머리글을 누르면 그 칸만 정렬이 바뀌고(크기·날짜는 큰 값부터, 다시 누르면 뒤집기), 열 경계를 끌면 그 선이 따라오고,
    /// 둘 다 세션에 남아 다시 열 때(작업 공간을 오갈 때도) 돌아온다
    #[test]
    fn header_sorts_and_resizes_each_pane() {
        let (root, home) = demo();
        let dir = util::join(&home, "업무");
        std::fs::write(std::path::Path::new(&dir).join("회의록.md"), b"xyz").unwrap();
        let (mut a, mut g) = app(&dir);
        a.paint(&mut g);
        let gm = geom(a.pane_rects()[0].1);
        let y = (gm.header.t + gm.header.b) / 2.0;
        let cols = columns(gm.list, a.cols(0));
        let (sl, sr) = cols.size.expect("a two-pane window shows the size column");
        assert_eq!(a.hit(&g, (sl + sr) / 2.0, y), Hit::Header(0, SortKey::Size));
        a.mouse_down(&g, (sl + sr) / 2.0, y, Mods::default(), 1);
        assert_eq!(names(&a, 0), ["2026 4분기", "회의록.md", "예산.csv"], "biggest file first, folders stay on top");
        assert_eq!(names(&a, 1), ["2026 4분기", "예산.csv", "회의록.md"], "the other pane keeps its own order");
        a.mouse_down(&g, (sl + sr) / 2.0, y, Mods::default(), 1);
        assert_eq!(names(&a, 0), ["2026 4분기", "예산.csv", "회의록.md"], "a second click flips the order");
        // 크기 열의 왼쪽 경계를 왼쪽으로 20 끌면 그 경계가 따라오고 크기 열이 20 넓어진다.
        // 두 칸 창에서는 유형 열이 숨어 왼쪽 이웃이 날짜 열이라, 날짜 열이 가장 좁은 폭(COL_MIN)까지 내줄 수 있는 만큼 안에서 끈다
        let start = a.cols(0);
        let edge = cols.resizable()[2].unwrap().0;
        assert_eq!(a.hit(&g, edge + 2.0, y), Hit::HeaderResize(0, 2));
        a.mouse_moved(&g, edge + 2.0, y);
        assert_eq!(a.pointer(), Pointer::LeftRight);
        a.mouse_down(&g, edge + 2.0, y, Mods::default(), 1);
        a.mouse_dragged(edge - 18.0, y);
        a.mouse_up();
        assert_eq!(a.pane(0).cols[2], start[2] + 20.0);
        let moved = columns(gm.list, a.cols(0)).resizable()[2].unwrap().0;
        assert!((moved - (edge - 20.0)).abs() < 0.5, "the edge follows the pointer: {edge} -> {moved}");
        // 더 끌어도 왼쪽 이웃 열은 COL_MIN보다 좁아지지 않는다 (선이 거기서 멈춘다)
        a.mouse_down(&g, moved + 2.0, y, Mods::default(), 1);
        a.mouse_dragged(moved - 400.0, y);
        a.mouse_up();
        assert!(a.cols(0).iter().all(|w| *w >= COL_MIN), "{:?}", a.cols(0));
        assert_eq!(a.pane(1).cols, DEFAULT_COLS, "the other pane keeps its columns");
        a.mouse_moved(&g, (sl + sr) / 2.0, gm.list.t + 10.0);
        assert_eq!(a.pointer(), Pointer::Arrow);
        // 세션: 정렬과 열 폭이 칸마다 남는다
        let s = a.to_session();
        assert_eq!((s.panes[0].sort_key, s.panes[0].sort_desc), (SortKey::Size.to_u8(), false));
        assert_eq!(s.panes[0].cols, a.pane(0).cols);
        assert_eq!(s.panes[1].cols[0], DATE_AUTO, "an untouched date column stays automatic");
        let (mut b, _) = app(&dir);
        b.restore(&s, &dir);
        b.settle();
        assert_eq!(b.pane(0).sort, Sort { key: SortKey::Size, desc: false });
        assert_eq!(b.pane(0).cols, a.pane(0).cols);
        assert_eq!(names(&b, 0), ["2026 4분기", "예산.csv", "회의록.md"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 탭이 넘쳐도 지금 탭은 보이고 닫을 수 있다 (나머지는 줄거나 가려지고 ⌃Tab으로 닿는다)
    #[test]
    fn active_tab_stays_visible_when_tabs_overflow() {
        let (root, home) = demo();
        let long = util::join(&home, "업무");
        let (mut a, g) = app(&long);
        for _ in 0..12 {
            a.new_tab(0);
        }
        a.settle();
        let gm = geom(a.pane_rects()[0].1);
        let head = gm.head;
        let (trs, add) = a.chips(&g, 0, &gm);
        let cur = trs[a.pane(0).active];
        assert!(cur.w() >= 52.0 && cur.r <= add.l, "{cur:?} {add:?}");
        assert!(add.r <= head.r, "the + button stays inside the pane");
        let (x, y) = (close_rect(cur).l + 4.0, cur.t + 8.0);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!(a.pane(0).tabs.len(), 12, "the active tab's close button works");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 즐겨찾기가 사이드바 아래로 넘치면 스크롤해서 마지막 줄까지 누를 수 있다
    #[test]
    fn sidebar_scrolls_to_rows_below_the_window() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let favs: Vec<Favorite> = (0..40).map(|i| Favorite { name: format!("폴더 {i}"), path: util::join(&home, "다운로드"), is_dir: true }).collect();
        a.favs = Favorites { items: favs, groups: Vec::new() };
        a.resize(1280.0, 420.0);
        let last = side_row(&a, "폴더 39");
        assert!(last.t > a.side_list().b, "the last row starts below the window");
        a.scroll(&g, 100.0, 300.0, 100_000.0);
        let last = side_row(&a, "폴더 39");
        assert!(last.b <= a.side_list().b + 0.5 && last.t >= a.side_list().t, "{last:?}");
        a.mouse_down(&g, last.l + 40.0, last.t + 10.0, Mods::default(), 1);
        a.mouse_up();
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(util::join(&home, "다운로드")));
        // 이름 줄 위로 밀려난 줄은 누를 수 없다 (그 자리는 공간 이름이다)
        let first = side_row(&a, "폴더 0");
        assert!(first.b < a.side_list().t);
        assert_eq!(a.hit(&g, first.l + 40.0, a.side_list().t - 6.0), Hit::SideTitle);
        // 줄이 줄면 스크롤도 줄 수 안으로
        a.favs.items.truncate(3);
        a.resize(1280.0, 420.0);
        assert_eq!(a.side_scroll, 0.0);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 위로 간 뒤 다 읽기 전에 다른 탭으로 옮겨도, 떠난 폴더가 보이게 하는 스크롤은 그 탭에만 적용된다
    #[test]
    fn reveal_scroll_stays_with_the_listed_tab() {
        let (root, home) = demo();
        let long = util::join(&home, "긴 폴더");
        for i in 0..100 {
            std::fs::create_dir_all(util::join(&long, &format!("sub_{i:03}"))).unwrap();
        }
        let (mut a, _g) = app(&long);
        a.new_tab(0);
        a.settle();
        // 보고 있을 탭: 커서는 아래쪽, 스크롤은 맨 위
        a.tab_mut(0).cursor = 80;
        a.tab_mut(0).scroll = 0.0;
        a.activate_tab(0, 0);
        a.navigate(0, Location::Dir(util::join(&long, "sub_090")), None);
        a.settle();
        a.go_up(0);
        a.activate_tab(0, 1);
        a.settle();
        assert_eq!(a.pane(0).tabs[1].scroll, 0.0, "the tab being viewed did not jump");
        let listed = &a.pane(0).tabs[0];
        assert_eq!(listed.cursor_entry().unwrap().name_string(), "sub_090");
        assert!(listed.scroll > 0.0, "the folder we left is scrolled into view in its own tab");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 주소 입력 해석: 폴더, 파일(그 폴더에서 고름), ~, 없는 경로·상대 경로
    #[test]
    fn resolve_input_like_windows() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        assert_eq!(resolve_input(&format!("  {work}/ "), "/x"), Ok((work.clone(), None)));
        assert_eq!(resolve_input(&util::join(&work, "예산.csv"), "/x"), Ok((work.clone(), Some("예산.csv".to_string()))));
        assert_eq!(resolve_input("~", &home), Ok((home.clone(), None)));
        assert_eq!(resolve_input("~/업무", &home), Ok((work.clone(), None)));
        assert!(resolve_input(&util::join(&home, "없는 폴더"), "/x").is_err());
        assert!(resolve_input("업무", &home).is_err(), "relative paths are not guessed");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 경로 줄 빈 곳을 누르면 입력, Return이면 그 경로로, 없는 경로면 칩 줄 알림, 다른 곳을 누르면 취소
    #[test]
    fn address_edit_commits_cancels_and_flashes() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        a.resize(3600.0, 800.0);
        let gm = geom(a.pane_rects()[1].1);
        let last = a.crumbs(&g, gm.crumbs, &home).last().unwrap().0;
        let (x, y) = (last.r + CHEV_W + 8.0, gm.crumb.t + 10.0);
        assert!(x < gm.crumb.r, "an empty spot after the last crumb");
        assert_eq!(a.hit(&g, x, y), Hit::CrumbEmpty(1));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!((a.edit.clone(), a.active()), (Editing::Address(1), 1));
        assert_eq!(a.address_text(), home);
        assert!(a.address_rect().is_some());
        a.commit_address(&util::join(&util::join(&home, "업무"), "회의록.md"));
        a.settle();
        assert_eq!(a.edit, Editing::None);
        assert_eq!(a.pane(1).tab().loc, Location::Dir(util::join(&home, "업무")));
        assert_eq!(a.pane(1).tab().cursor_entry().unwrap().name_string(), "회의록.md");
        // 없는 경로: 그대로 두고 알림
        a.key(Key::Char('l'), Mods { cmd: true, ..Default::default() });
        assert_eq!(a.edit, Editing::Address(1));
        a.commit_address("/없는/경로");
        a.settle();
        assert!(a.flash.as_ref().is_some_and(|(p, m)| *p == 1 && m.contains("찾을 수 없어요")));
        assert_eq!(a.pane(1).tab().loc, Location::Dir(util::join(&home, "업무")));
        // 늦게 끝난 확인은 버린다: 경로 입력 두 번이면 나중 것이, 확인 중에 다른 곳으로 옮기면 그 곳이 이긴다
        a.begin_address(1);
        a.commit_address(&util::join(&home, "다운로드"));
        let first = a.take_probes().pop().unwrap();
        a.begin_address(1);
        a.commit_address(&home);
        let second = a.take_probes().pop().unwrap();
        a.on_probed(&second, resolve_input(&second.text, &home));
        a.on_probed(&first, resolve_input(&first.text, &home));
        a.settle();
        assert_eq!(a.pane(1).tab().loc, Location::Dir(home.clone()), "the later address wins");
        a.begin_address(1);
        a.commit_address(&util::join(&home, "다운로드"));
        let slow = a.take_probes().pop().unwrap();
        a.navigate(1, Location::Dir(util::join(&home, "업무")), None);
        a.on_probed(&slow, resolve_input(&slow.text, &home));
        a.settle();
        assert_eq!(a.pane(1).tab().loc, Location::Dir(util::join(&home, "업무")), "navigation after the address wins");
        // 대소문자만 다른 경로는 같은 폴더로 보지 않고 그 경로로 간다 (대소문자를 가리는 볼륨에서는 다른 폴더)
        let work = util::join(&home, "업무");
        let other_case: String = work.chars().map(|c| if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c }).collect();
        if other_case != work && resolve_input(&other_case, &home).is_ok() {
            a.begin_address(1);
            a.commit_address(&other_case);
            a.settle();
            assert_eq!(a.pane(1).tab().loc, Location::Dir(other_case.clone()));
            assert_eq!(a.pane(1).tab().back.last(), Some(&Location::Dir(work)));
        }
        // ⌘⇧G로 열고, 목록을 누르면 취소된다 (알림도 지운다)
        a.key(Key::Char('g'), Mods { cmd: true, shift: true, ..Default::default() });
        assert_eq!(a.edit, Editing::Address(1));
        let l = a.list_rect(0).unwrap();
        a.mouse_down(&g, l.l + 60.0, l.b - 10.0, Mods::default(), 1);
        assert_eq!((a.edit.clone(), a.flash.is_none()), (Editing::None, true));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 세션: 화면 상태를 저장했다가 그대로 연다. 없어진 폴더는 홈으로
    #[test]
    fn session_round_trip() {
        let (root, home) = demo();
        let (mut a, _g) = app(&home);
        a.navigate(0, Location::Dir(util::join(&home, "업무")), None);
        a.new_tab(1);
        a.navigate(1, Location::Dir(util::join(&home, "다운로드")), None);
        a.set_layout(Layout::Three);
        a.activate_pane(2);
        a.toggle_hidden();
        a.split = (0.3, 0.6);
        a.settle();
        let mut s = a.to_session();
        // 다시 읽은 세션 글자로 연다 (파일 형식까지)
        s = session::parse(&session::serialize(&s)).expect("session");
        assert_eq!(s.ws.as_deref(), Some("기본"), "the session names the workspace it shows");
        s.panes[2].tabs = vec![util::join(&home, "지운 폴더")];
        let (mut b, _g) = app(&home);
        b.restore(&s, &home);
        b.settle();
        assert_eq!((b.layout(), b.active(), b.show_hidden, b.split), (Layout::Three, 2, true, (0.3, 0.6)));
        assert_eq!(b.pane(0).tab().loc, Location::Dir(util::join(&home, "업무")));
        assert_eq!(b.pane(1).tabs.len(), 2);
        assert_eq!(b.pane(1).active, 1);
        assert_eq!(b.pane(1).tab().loc, Location::Dir(util::join(&home, "다운로드")));
        assert_eq!(b.pane(2).tab().loc, Location::Dir(home.clone()), "a missing folder opens home after its listing fails");
        assert!(b.pane(2).tab().back.is_empty(), "the fallback leaves no history");
        // 첫 읽기 전에 다른 곳으로 옮겼으면 그곳의 오류는 홈으로 바꾸지 않고 보인다
        let mut s2 = s.clone();
        s2.panes[0].tabs = vec![util::join(&home, "지운 폴더")];
        let (mut c, _g) = app(&home);
        c.restore(&s2, &home);
        let gone = util::join(&home, "없는 곳");
        c.navigate(0, Location::Dir(gone.clone()), None);
        c.settle();
        assert_eq!(c.pane(0).tab().loc, Location::Dir(gone));
        assert!(matches!(c.pane(0).tab().state, LoadState::Error(_)));
        assert!(names(&b, 0).contains(&"예산.csv".to_string()), "visible panes are loaded");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn crumbs_walk_up_and_select_the_child() {
        let (root, home) = demo();
        let (mut a, g) = app(&util::join(&util::join(&home, "업무"), "2026 4분기"));
        let gm = geom(a.pane_rects()[0].1);
        let cs = a.crumbs(&g, gm.crumbs, a.pane(0).tab().loc.dir().unwrap());
        let i = cs.iter().position(|(_, _, l)| l == "업무").unwrap();
        let (cr, _, _) = &cs[i];
        a.mouse_down(&g, cr.l + 4.0, cr.t + 8.0, Mods::default(), 1);
        a.settle();
        assert_eq!(a.pane(0).tab().cursor_entry().unwrap().name_string(), "2026 4분기");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn toolbar_buttons_work_on_the_active_pane_and_ask_for_menus() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let m = a.toolbar();
        let mid = |r: R| ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0);
        // 뒤로·앞으로는 지금 칸의 것
        let (x, y) = mid(m.back);
        assert_eq!(a.hit(&g, x, y), Hit::Back(a.active()));
        // 칸 나누기 단추: 메뉴를 열어 달라고 하고, 단추 자리를 알려 준다
        let (x, y) = mid(m.layout);
        assert_eq!(a.hit(&g, x, y), Hit::LayoutPick);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!(take_menu(&mut a).map(|(k, r)| (k, r.l)), Some((MenuKind::Layout, m.layout.l)));
        assert!(take_menu(&mut a).is_none(), "the request is handed over once");
        assert_eq!(a.menu_items(MenuKind::Layout).iter().filter(|(_, on)| *on).count(), 1);
        a.pick_menu(MenuKind::Layout, 4);
        assert_eq!(a.layout(), Layout::Four);
        // 보기 방식 단추는 칸마다 경로 줄에 있다: 누른 칸이 지금 칸이 되고, 그 칸의 단추 아래에 여섯 가지가 열린다
        let p = 1 - a.active();
        let b = a.view_button(p);
        assert!(b.w() > 0.0);
        let (x, y) = mid(b);
        assert_eq!(a.hit(&g, x, y), Hit::ViewPick(p));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!(a.active(), p);
        assert_eq!(take_menu(&mut a), Some((MenuKind::View, b)));
        assert_eq!(a.menu_items(MenuKind::View).len(), ViewMode::ALL.len());
        a.pick_menu(MenuKind::View, 2);
        assert_eq!(a.pane(p).view, ViewMode::Medium);
        // 숨김 항목은 도구 막대의 눈으로 켜고 끈다
        let (x, y) = mid(a.toolbar().hidden);
        assert_eq!(a.hit(&g, x, y), Hit::HiddenToggle);
        let hidden = a.prefs().hidden;
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_ne!(a.prefs().hidden, hidden);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 메뉴는 앱이 그린다: 단추 아래 창 안에 뜨고, 줄을 누르거나 ↓·Return으로 고른다. Esc와 메뉴 밖 누르기는 닫기만 한다
    /// (그 누름은 뒤로 가지 않고, 메뉴가 떠 있는 동안 메뉴 밖의 것은 가리키지 않는다). 같은 단추를 다시 누르면 닫힌다
    #[test]
    fn menus_are_drawn_by_the_app_and_take_the_mouse_and_keys() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let (rx, ry) = row_point(&a, 0, 1);
        a.mouse_down(&g, rx, ry, Mods::default(), 1);
        a.mouse_up();
        let picked = a.seen().selected.clone();
        assert_eq!(picked.len(), 1);
        let b = a.view_button(a.active());
        let (x, y) = mid(b);
        let open = |a: &mut App| {
            a.mouse_down(&g, x, y, Mods::default(), 1);
            a.mouse_up();
        };
        open(&mut a);
        let r = a.menu_rect().expect("the view menu is open");
        assert!(r.t >= b.b && r.l >= MARGIN && r.r <= a.w - MARGIN, "below the button, inside the window: {r:?}");
        // 메뉴 밖(다른 줄)을 누르면 닫기만 한다
        let (ox, oy) = row_point(&a, 0, 2);
        assert_eq!(a.hit(&g, ox, oy), Hit::None, "nothing under the open menu is pointed at");
        a.mouse_down(&g, ox, oy, Mods::default(), 1);
        a.mouse_up();
        assert!(a.menu_shown().is_none());
        assert_eq!(a.seen().selected, picked, "the press only closed the menu");
        // 마우스로: 올린 줄을 가리키고, 누르면 고르고 닫힌다
        open(&mut a);
        let r = a.menu_rect().unwrap();
        let rows = sakadu::ui::menu::menu_rows(r, &a.menu.as_ref().unwrap().lines, 0.0);
        let (mx, my) = mid(rows[3]);
        assert_eq!(a.hit(&g, mx, my), Hit::Menu(Some(3)));
        assert!(a.mouse_moved(&g, mx, my));
        a.mouse_down(&g, mx, my, Mods::default(), 1);
        assert!(a.menu_shown().is_none());
        assert_eq!(a.pane(a.active()).view, ViewMode::ALL[3]);
        // 키로: ↓ 두 번, Return
        open(&mut a);
        assert!(a.key(Key::Down, Mods::default()) && a.key(Key::Down, Mods::default()));
        assert!(a.key(Key::Return, Mods::default()));
        assert!(a.menu_shown().is_none());
        assert_eq!(a.pane(a.active()).view, ViewMode::ALL[1]);
        // Esc는 닫기만, 같은 단추를 다시 누르면 닫힌다
        open(&mut a);
        assert!(a.key(Key::Esc, Mods::default()) && a.menu_shown().is_none());
        open(&mut a);
        open(&mut a);
        assert!(a.menu_shown().is_none());
        assert_eq!(a.pane(a.active()).view, ViewMode::ALL[1]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 긴 메뉴(작업 공간이 많을 때)는 창 안에 들어가게 높이를 줄이고 스크롤한다. 키로 끝 줄을 가리키면 그 줄이 보인다
    #[test]
    fn long_menus_stay_inside_the_window_and_scroll() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        for _ in 0..24 {
            a.new_workspace();
            a.cancel_address();
        }
        a.resize(900.0, 400.0);
        a.switch_workspace(0);
        a.settle();
        let (x, y) = mid(a.rail().add);
        assert!(a.right_down(&g, x, y));
        let r = a.menu_rect().unwrap();
        assert!(r.t >= MARGIN && r.b <= a.h - MARGIN + 0.01, "{r:?}");
        let lines = a.menu.as_ref().unwrap().lines.clone();
        assert_eq!(lines.len(), 25);
        let max = sakadu::ui::menu::menu_max_scroll(r, &lines);
        assert!(max > 50.0);
        let (cx, cy) = mid(r);
        a.scroll(&g, cx, cy, 50.0);
        assert_eq!(a.menu.as_ref().unwrap().scroll, 50.0);
        assert!(a.key(Key::End, Mods::default()));
        assert_eq!(a.menu.as_ref().unwrap().scroll, max);
        assert!(a.key(Key::Return, Mods::default()));
        a.settle();
        assert_eq!(a.workspaces().current, Some(24));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn icon_view_moves_along_the_grid_and_is_saved_in_the_session() {
        let (root, home) = demo();
        let (mut a, _g) = app(&util::join(&home, "업무"));
        a.set_view(0, ViewMode::Medium);
        let n = a.pane(0).tab().view.len();
        assert_eq!(n, 3);
        // 아이콘 보기: 좌우가 한 칸. 한 줄에 다 들어가 있으면 위아래로는 움직이지 않는다
        a.key(Key::Home, Mods::default());
        a.key(Key::Right, Mods::default());
        assert_eq!(a.pane(0).tab().cursor, 1);
        a.key(Key::Down, Mods::default());
        assert_eq!(a.pane(0).tab().cursor, 1);
        a.key(Key::End, Mods::default());
        assert_eq!(a.pane(0).tab().cursor, n - 1);
        // 목록 보기: 위아래가 한 칸
        a.set_view(0, ViewMode::List);
        a.key(Key::Up, Mods::default());
        assert_eq!(a.pane(0).tab().cursor, n - 2);
        // 보기 방식은 세션에 남는다
        let s = a.to_session();
        let (mut b, _g) = app(&home);
        b.restore(&s, &home);
        assert_eq!(b.pane(0).view, ViewMode::List);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 레일: +로 지금 화면에서 새 공간을 만들고, 다른 공간의 아이콘을 누르면 그 공간의 칸 나누기와 탭이 돌아온다
    #[test]
    fn rail_makes_workspaces_and_switching_restores_their_screens() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        assert_eq!((a.workspaces().list.len(), a.workspaces().current_name()), (1, Some("기본")));
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        a.navigate(0, Location::Dir(work.clone()), None);
        a.settle();
        // +: 이름은 지금 폴더, 색은 아직 쓰지 않은 색. 이름을 바로 고칠 수 있다
        let (x, y) = mid(a.rail().add);
        assert_eq!(a.hit(&g, x, y), Hit::SpaceAdd);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!(a.workspaces().current, Some(1));
        let w = a.workspaces().current().unwrap();
        assert_eq!((w.name.as_str(), w.color.as_str()), ("업무", "blue"));
        assert!(a.edit == Editing::Space && a.address_rect().is_some());
        assert_eq!((a.address_text().as_str(), a.address_select()), ("업무", None));
        a.commit_address("  월말 정산 ");
        assert_eq!((a.edit.clone(), a.workspaces().current_name()), (Editing::None, Some("월말 정산")));
        // 새 공간에서 화면을 바꾼다: 세 칸, 첫 칸에 탭 둘
        a.set_layout(Layout::Three);
        a.new_tab(0);
        a.navigate(0, Location::Dir(downloads.clone()), None);
        a.settle();
        // 첫 공간의 아이콘: 그 공간의 화면(두 칸, 탭 하나, 업무 폴더)이 돌아오고 폴더를 다시 읽는다
        let (x, y) = mid(a.rail().spaces[0].1);
        assert_eq!(a.hit(&g, x, y), Hit::Space(0));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.settle();
        assert_eq!((a.workspaces().current, a.layout(), a.pane(0).tabs.len()), (Some(0), Layout::TwoV, 1));
        assert_eq!(a.pane(0).tab().loc, Location::Dir(work.clone()));
        assert!(names(&a, 0).contains(&"예산.csv".to_string()));
        // 둘째 공간으로 돌아가면 떠날 때의 화면 그대로다
        a.switch_workspace(1);
        a.settle();
        assert_eq!((a.layout(), a.pane(0).tabs.len(), a.pane(0).active), (Layout::Three, 2, 1));
        assert_eq!(a.pane(0).tab().loc, Location::Dir(downloads));
        // 이름: 두 번 누르면 고치고, 다른 곳을 누르면 그만둔다. 다른 공간과 같은 이름에는 번호가 붙는다
        let (x, y) = mid(a.side_title());
        assert_eq!(a.hit(&g, x, y), Hit::SideTitle);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!(a.edit, Editing::None);
        a.mouse_down(&g, x, y, Mods::default(), 2);
        assert_eq!(a.edit, Editing::Space);
        let l = a.list_rect(0).unwrap();
        a.mouse_down(&g, l.l + 60.0, l.b - 10.0, Mods::default(), 1);
        assert_eq!((a.edit.clone(), a.workspaces().current_name()), (Editing::None, Some("월말 정산")));
        a.begin_rename();
        a.commit_address("기본");
        assert_eq!(a.workspaces().current_name(), Some("기본 2"));
        a.begin_rename();
        assert!(a.end_editing() && a.edit == Editing::None && !a.end_editing());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 공간 메뉴(오른쪽 단추): 색을 바꾸고 지운다. 마지막 하나는 지울 수 없다
    #[test]
    fn workspace_menu_sets_the_color_and_keeps_the_last_workspace() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        // 공간이 하나뿐일 때: 색 여섯 가지와 이름 바꾸기. 지우기는 없다
        let items = a.menu_items(MenuKind::Space(0));
        assert_eq!(items.len(), workspaces::COLORS.len() + 1);
        assert!(items.iter().all(|(_, on)| !on), "no colour is chosen yet: it follows the system accent");
        a.delete_workspace(0);
        assert_eq!(a.workspaces().list.len(), 1);
        // 오른쪽 단추: 메뉴를 열어 달라고 하고, 아이콘 자리를 알려 준다. ⌃-누르기도 같다
        let tile = a.rail().spaces[0].1;
        let (x, y) = mid(tile);
        assert!(a.right_down(&g, x, y));
        assert_eq!(take_menu(&mut a).map(|(k, r)| (k, r.t)), Some((MenuKind::Space(0), tile.t)));
        a.mouse_down(&g, x, y, Mods { ctrl: true, ..Default::default() }, 1);
        assert_eq!(take_menu(&mut a).map(|(k, _)| k), Some(MenuKind::Space(0)));
        // 목록의 빈 곳에도 메뉴가 있다 (새 폴더, 새로 고침)
        assert!(a.right_down(&g, a.list_rect(0).unwrap().l + 60.0, 300.0));
        assert_eq!(take_menu(&mut a).map(|(k, _)| k), Some(MenuKind::Blank));
        // 색: 고르면 그 공간의 색이 되고, 지금 공간이면 강조 색도 바뀐다
        let before = a.theme.accent;
        a.pick_menu(MenuKind::Space(0), 2);
        assert_eq!(a.workspaces().list[0].color, "orange");
        assert_eq!(a.menu_items(MenuKind::Space(0)).iter().position(|(_, on)| *on), Some(2));
        let orange = a.theme.accent;
        assert_ne!(orange, before);
        // 이름 바꾸기는 지금 공간의 메뉴에만 있다
        a.pick_menu(MenuKind::Space(0), workspaces::COLORS.len());
        assert_eq!(a.edit, Editing::Space);
        a.cancel_address();
        // 공간이 둘이면 지우기가 생긴다. 새 공간은 다른 색을 받는다
        a.new_workspace();
        a.cancel_address();
        a.set_layout(Layout::One);
        assert_ne!(a.theme.accent, orange, "the new workspace has its own colour");
        assert_eq!(a.menu_items(MenuKind::Space(0)).len(), workspaces::COLORS.len() + 1, "another workspace: colours and delete, no rename");
        let items = a.menu_items(MenuKind::Space(1));
        assert_eq!((items.len(), items.last().unwrap().0.as_str()), (workspaces::COLORS.len() + 2, "이 작업 공간 지우기"));
        assert_eq!(a.menu_break(MenuKind::Space(1)), Some(workspaces::COLORS.len()));
        // 지금 공간을 지우면 남은 공간의 화면과 색으로 바뀐다
        a.pick_menu(MenuKind::Space(1), items.len() - 1);
        a.settle();
        assert_eq!((a.workspaces().list.len(), a.workspaces().current, a.layout()), (1, Some(0), Layout::TwoV));
        assert_eq!(a.theme.accent, orange);
        // 지금 것이 아닌 공간을 지우면 화면은 그대로다
        a.new_workspace();
        a.cancel_address();
        a.set_layout(Layout::Four);
        a.pick_menu(MenuKind::Space(0), workspaces::COLORS.len());
        assert_eq!((a.workspaces().list.len(), a.workspaces().current, a.layout()), (1, Some(0), Layout::Four));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// + 를 오른쪽 단추로 누르면 모든 작업 공간이 나온다: 창이 낮아 레일에 다 보이지 않는 공간도 고를 수 있다
    #[test]
    fn add_menu_lists_every_workspace_even_those_the_rail_hides() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        for _ in 0..7 {
            a.new_workspace();
            a.cancel_address();
        }
        // 낮은 창: 레일에 여덟 개가 다 들어가지 않는다
        a.resize(900.0, 300.0);
        a.switch_workspace(0);
        a.settle();
        let rail = a.rail();
        assert!(rail.spaces.len() < 8, "{} tiles fit", rail.spaces.len());
        let hidden = (0..8).find(|i| !rail.spaces.iter().any(|(s, _)| s == i)).unwrap();
        // + 를 오른쪽 단추로: 메뉴를 + 자리에 열어 달라고 한다. 모든 공간의 이름이 있고 지금 것에 체크
        let (x, y) = mid(rail.add);
        assert!(a.right_down(&g, x, y));
        assert_eq!(take_menu(&mut a).map(|(k, r)| (k, r.t)), Some((MenuKind::Spaces, rail.add.t)));
        let items = a.menu_items(MenuKind::Spaces);
        assert_eq!((items.len(), items.iter().position(|(_, on)| *on)), (8, Some(0)));
        assert_eq!(a.menu_break(MenuKind::Spaces), None);
        // 안 보이던 공간을 고르면 그 공간으로 바뀌고, 레일도 그 공간이 보이는 범위로 옮겨진다
        a.pick_menu(MenuKind::Spaces, hidden);
        a.settle();
        assert_eq!(a.workspaces().current, Some(hidden));
        assert!(a.rail().spaces.iter().any(|(s, _)| *s == hidden));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 책갈피와 ⌃⌘T는 지금 공간에만 고정한다: 다른 공간에서는 보이지 않고, 돌아오면 그대로 있다
    #[test]
    fn pins_belong_to_the_current_workspace() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        a.navigate(0, Location::Dir(work.clone()), None);
        a.settle();
        assert!(!a.is_marked(&work));
        a.toggle_star(0);
        assert!(a.is_marked(&work) && a.favorites().items.is_empty());
        assert_eq!(pin_names(&a), ["업무"]);
        // ⌃⌘T: 커서 항목을 고정하고, 다시 누르면 푼다
        let (x, y) = row_point(&a, 0, 1);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        let ctrl_cmd = Mods { cmd: true, ctrl: true, ..Default::default() };
        assert!(a.key(Key::Char('t'), ctrl_cmd));
        assert_eq!(pin_names(&a), ["업무", "예산.csv"]);
        assert!(!a.workspaces().current().unwrap().pins[1].is_dir);
        // 고정한 파일 줄을 누르면 macOS '열기'로 넘긴다 (누를 때가 아니라 뗄 때)
        let r = side_row(&a, "예산.csv");
        a.mouse_down(&g, r.l + 40.0, r.t + 10.0, Mods::default(), 1);
        assert!(a.open_files.is_empty(), "a press alone does not open the row");
        a.mouse_up();
        assert_eq!(a.open_files, [util::join(&work, "예산.csv")]);
        let (x, y) = row_point(&a, 0, 1);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert!(a.key(Key::Char('t'), ctrl_cmd));
        assert_eq!(pin_names(&a), ["업무"]);
        // 새 공간에는 고정한 것이 없고, 원래 공간으로 돌아오면 그대로 있다
        a.new_workspace();
        a.cancel_address();
        assert!(pin_names(&a).is_empty() && !a.is_marked(&work));
        a.switch_workspace(0);
        a.settle();
        assert_eq!(pin_names(&a), ["업무"]);
        // '모든 공간'(즐겨찾기)에만 있는 폴더: 책갈피가 채워져 보이고, 누르면 거기서 빠진다 (이 공간에 고정되지는 않는다)
        a.favs.push(None, Favorite { name: "다운로드".into(), path: downloads.clone(), is_dir: true });
        a.navigate(0, Location::Dir(downloads.clone()), None);
        a.settle();
        assert!(a.is_marked(&downloads));
        a.toggle_star(0);
        assert!(a.favorites().items.is_empty() && !a.is_marked(&downloads));
        assert_eq!(pin_names(&a), ["업무"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 메뉴의 줄 이름들
    fn menu_labels(a: &App, kind: MenuKind) -> Vec<String> {
        a.menu_items(kind).into_iter().map(|(label, _)| label).collect()
    }

    /// 고정한 줄의 메뉴(오른쪽 단추, ⌃-누르기): 위·아래로 옮기고 푼다. 맨 위 줄에는 위로가, 맨 아래 줄에는 아래로가 없다
    #[test]
    fn pin_row_menu_moves_and_unpins() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        for (path, is_dir) in [(work.clone(), true), (downloads.clone(), true), (util::join(&work, "예산.csv"), false)] {
            a.toggle_pin(&path, is_dir);
        }
        assert_eq!(pin_names(&a), ["업무", "다운로드", "예산.csv"]);
        // 첫 줄·가운데 줄·마지막 줄: 갈 수 있는 쪽만 있고, 옮기는 줄과 나머지 줄 사이에 구분선이 온다
        assert_eq!(menu_labels(&a, MenuKind::Pin(0)), ["아래로 이동", "모든 공간에 두기", "고정 풀기"]);
        assert_eq!(menu_labels(&a, MenuKind::Pin(1)), ["위로 이동", "아래로 이동", "모든 공간에 두기", "고정 풀기"]);
        assert_eq!(menu_labels(&a, MenuKind::Pin(2)), ["위로 이동", "모든 공간에 두기", "고정 풀기"]);
        assert_eq!([0, 1, 2].map(|p| a.menu_break(MenuKind::Pin(p))), [Some(1), Some(2), Some(1)]);
        assert!(a.menu_items(MenuKind::Pin(1)).iter().all(|(_, on)| !on), "nothing is checked");
        assert!(a.menu_items(MenuKind::Pin(3)).is_empty(), "there is no fourth pin");
        // 오른쪽 단추: 메뉴를 그 줄에 열어 달라고 한다. ⌃-누르기도 같고, 그 줄을 열지는 않는다
        let row = side_row(&a, "다운로드");
        let (x, y) = mid(row);
        assert_eq!(a.hit(&g, x, y), Hit::SideRow(1));
        assert!(a.right_down(&g, x, y));
        assert_eq!(take_menu(&mut a), Some((MenuKind::Pin(1), row)));
        assert!(take_menu(&mut a).is_none(), "the request is handed over once");
        a.mouse_down(&g, x, y, Mods { ctrl: true, ..Default::default() }, 1);
        assert_eq!(take_menu(&mut a).map(|(k, _)| k), Some(MenuKind::Pin(1)));
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(home.clone()), "the control-click did not open the row");
        // 위로, 아래로 (첫 줄의 메뉴는 '아래로 이동'이 첫 줄이다)
        a.pick_menu(MenuKind::Pin(1), 0);
        assert_eq!(pin_names(&a), ["다운로드", "업무", "예산.csv"]);
        a.pick_menu(MenuKind::Pin(0), 0);
        assert_eq!(pin_names(&a), ["업무", "다운로드", "예산.csv"]);
        a.pick_menu(MenuKind::Pin(1), 1);
        assert_eq!(pin_names(&a), ["업무", "예산.csv", "다운로드"]);
        // 고정 풀기: 마지막 줄 메뉴의 셋째 줄
        a.pick_menu(MenuKind::Pin(2), 2);
        assert_eq!(pin_names(&a), ["업무", "예산.csv"]);
        // 없는 줄을 고르면 아무 일도 없다
        a.pick_menu(MenuKind::Pin(1), 9);
        a.pick_menu(MenuKind::Pin(7), 0);
        assert_eq!(pin_names(&a), ["업무", "예산.csv"]);
        // 하나만 남으면 옮기는 줄도 구분선도 없다
        a.pick_menu(MenuKind::Pin(0), 2);
        assert_eq!(pin_names(&a), ["예산.csv"]);
        assert_eq!(menu_labels(&a, MenuKind::Pin(0)), ["모든 공간에 두기", "고정 풀기"]);
        assert_eq!(a.menu_break(MenuKind::Pin(0)), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 고정한 줄은 '모든 공간'에 둘 수 있고, '모든 공간'의 즐겨찾기 항목은 이 작업 공간에만 두거나 뺄 수 있다.
    /// 바로가기에는 메뉴가 없고, 그룹 머리에는 그룹 메뉴가 있다
    #[test]
    fn rows_move_between_the_workspace_and_the_common_block() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        a.places = vec![("홈".to_string(), home.clone())];
        a.toggle_pin(&work, true);
        a.toggle_pin(&downloads, true);
        // 모든 공간에 두기 (첫 줄 메뉴의 둘째 줄): 이 공간에서 빠지고 즐겨찾기의 그룹 없는 항목 끝으로 간다
        a.pick_menu(MenuKind::Pin(0), 1);
        assert_eq!(pin_names(&a), ["다운로드"]);
        assert_eq!(a.favorites().items, [Favorite { name: "업무".into(), path: work.clone(), is_dir: true }]);
        assert!(a.is_marked(&work) && a.flash.is_none());
        // 그 줄을 오른쪽 단추로 누르면 즐겨찾기 항목의 메뉴를 그 줄에 열어 달라고 한다
        let row = side_row(&a, "업무");
        let (x, y) = mid(row);
        assert!(a.right_down(&g, x, y));
        assert_eq!(take_menu(&mut a), Some((MenuKind::Fav(0), row)));
        // 그룹이 없으면 그룹으로 옮기는 줄은 새 그룹뿐이다 (구분선 다음)
        assert_eq!(menu_labels(&a, MenuKind::Fav(0)), ["이 작업 공간에만 두기", "모든 공간에서 빼기", "새 그룹을 만들어 넣기"]);
        assert_eq!(a.menu_break(MenuKind::Fav(0)), Some(2));
        // 이 작업 공간에만 두기: 즐겨찾기에서 빠지고 이 공간에 고정한 것의 끝으로 돌아온다
        a.pick_menu(MenuKind::Fav(0), 0);
        assert!(a.favorites().items.is_empty() && a.flash.is_none());
        assert_eq!(pin_names(&a), ["다운로드", "업무"]);
        // 이미 양쪽에 있는 것은 한쪽에서 빠지기만 한다 (겹쳐 들어가지 않는다)
        a.favs.push(None, Favorite { name: "업무".into(), path: work.clone(), is_dir: true });
        a.pick_menu(MenuKind::Pin(1), 1);
        assert_eq!((pin_names(&a), a.favorites().items.len()), (vec!["다운로드".to_string()], 1));
        a.workspaces.current_mut().unwrap().pin(&work, true);
        a.pick_menu(MenuKind::Fav(0), 0);
        assert_eq!((pin_names(&a), a.favorites().items.len()), (vec!["다운로드".to_string(), "업무".to_string()], 0));
        // 그룹 안의 항목: 번호는 즐겨찾기 줄(그룹 머리도 한 줄)로 센다. 바로가기에는 메뉴가 없고, 그룹 머리는 그룹 메뉴를 연다
        let budget = Favorite { name: "예산".into(), path: util::join(&work, "예산.csv"), is_dir: false };
        a.favs.groups.push(Group { name: "묶음".into(), collapsed: false, items: vec![budget] });
        let (x, y) = mid(side_row(&a, "홈"));
        assert!(matches!(a.hit(&g, x, y), Hit::SideRow(_)));
        assert!(!a.right_down(&g, x, y));
        assert!(take_menu(&mut a).is_none());
        let head = side_row(&a, "묶음");
        let (x, y) = mid(head);
        assert!(a.right_down(&g, x, y));
        assert_eq!(take_menu(&mut a), Some((MenuKind::Group(0), head)));
        assert!(a.menu_items(MenuKind::Fav(0)).is_empty(), "the group header has no item menu");
        let item = side_row(&a, "예산");
        let (x, y) = mid(item);
        assert!(a.right_down(&g, x, y));
        assert_eq!(take_menu(&mut a), Some((MenuKind::Fav(1), item)));
        // 모든 공간에서 빼기: 즐겨찾기에서만 없어진다
        a.pick_menu(MenuKind::Fav(1), 1);
        assert!(a.favorites().groups[0].items.is_empty());
        assert_eq!(pin_names(&a), ["다운로드", "업무"]);
        // 없는 줄을 고르면 아무 일도 없다 (그룹 머리의 번호, 없는 번호)
        a.pick_menu(MenuKind::Fav(0), 1);
        a.pick_menu(MenuKind::Fav(9), 0);
        assert_eq!((a.favorites().groups.len(), pin_names(&a).len()), (1, 2));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 사이드바의 그 줄을 누르고, 아래로 dy만큼(위로는 음수) 두 번에 나눠 끌고, 뗀다 (실제 마우스 입력의 길)
    fn drag_side(a: &mut App, g: &Gfx, label: &str, dy: f32) {
        let (x, y) = mid(side_row(a, label));
        a.mouse_down(g, x, y, Mods::default(), 1);
        a.mouse_dragged(x, y + dy / 2.0);
        a.mouse_dragged(x, y + dy);
        a.mouse_up();
    }

    /// 저장하는 작업 공간 파일의 글자로 다시 읽은 지금 공간의 고정한 것 (시험은 파일에 쓰지 않는다: 쓰는 그 내용을 본다)
    fn saved_pins(a: &App) -> Vec<String> {
        let w = workspaces::parse(&workspaces::serialize(a.workspaces())).unwrap();
        w.current().unwrap().pins.iter().map(|f| f.name.clone()).collect()
    }

    /// 저장하는 즐겨찾기 파일의 글자로 다시 읽은 즐겨찾기
    fn saved_favs(a: &App) -> Favorites {
        favorites::parse(&favorites::serialize(a.favorites()))
    }

    fn names_of(items: &[Favorite]) -> Vec<String> {
        items.iter().map(|f| f.name.clone()).collect()
    }

    /// 고정한 줄을 끌어 다른 고정한 줄 아래에 놓으면 그 자리로 옮겨지고 작업 공간에 저장된다. 끄는 동안 줄은 카드로 뜨고
    /// 다른 줄이 빈자리를 내준다. 목록 밖에서 놓으면 고정한 것 안에 남는다
    #[test]
    fn dragging_a_pin_below_another_reorders_and_saves() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let work = util::join(&home, "업무");
        for (path, is_dir) in [(work.clone(), true), (util::join(&home, "다운로드"), true), (util::join(&work, "예산.csv"), false)] {
            a.toggle_pin(&path, is_dir);
        }
        assert_eq!(pin_names(&a), ["업무", "다운로드", "예산.csv"]);
        // 끄는 동안: 첫 줄은 감춰지고(카드), 둘째 줄이 첫 자리로 올라와 그 아래에 빈자리가 열린다
        let (x, y) = mid(side_row(&a, "업무"));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.mouse_dragged(x, y + 2.0);
        assert!(a.side_drop_plan(&a.side_rows()).is_none(), "a small move is still a click");
        a.mouse_dragged(x, y + SIDE_ROW_H - 2.0);
        let (dragged, p) = a.side_drop_plan(&a.side_rows()).unwrap();
        assert_eq!((dragged, p.to, p.gap_t, p.gap_group), (FlowKind::Pin(0), DropTo::Pin(1), SIDE_ROW_H, None));
        assert_eq!(p.targets[..3], [None, Some(0.0), Some(2.0 * SIDE_ROW_H)]);
        a.mouse_up();
        assert_eq!(pin_names(&a), ["다운로드", "업무", "예산.csv"]);
        assert_eq!(saved_pins(&a), ["다운로드", "업무", "예산.csv"]);
        assert!(a.side_moving(), "the dropped row slides in from where the card was");
        assert_eq!(a.pane(0).tab().loc, Location::Dir(home.clone()), "a drag does not open the row");
        // 맨 아래로, 그리고 목록 밖(창 아래)으로 끌어 놓아도 고정한 것 안의 맨 끝
        drag_side(&mut a, &g, "다운로드", 3.0 * SIDE_ROW_H);
        assert_eq!(pin_names(&a), ["업무", "예산.csv", "다운로드"]);
        drag_side(&mut a, &g, "업무", 2000.0);
        assert_eq!(saved_pins(&a), ["예산.csv", "다운로드", "업무"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 즐겨찾기 항목을 펼친 그룹 안으로 끌면 그 그룹에 들어가고(끄는 동안 그룹이 판으로 묶인다), 맨 위로 끌어내면 그룹 밖으로 나온다.
    /// 그룹 머리를 끌면 그룹이 통째로 옮겨진다
    #[test]
    fn favorites_are_dragged_into_and_out_of_groups() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let work = util::join(&home, "업무");
        let f = |name: &str, path: String, is_dir| Favorite { name: name.into(), path, is_dir };
        a.places = vec![("홈".to_string(), home.clone())];
        a.favs = Favorites {
            items: vec![f("업무", work.clone(), true)],
            groups: vec![
                Group { name: "묶음".into(), collapsed: false, items: vec![f("예산", util::join(&work, "예산.csv"), false)] },
                Group { name: "둘째".into(), collapsed: false, items: vec![f("회의록", util::join(&work, "회의록.md"), false)] },
            ],
        };
        // '업무'를 '예산' 아래로: 묶음의 끝 (끄는 동안 빈자리가 묶음 안에 있어 묶음을 판으로 묶는다)
        let (x, y) = mid(side_row(&a, "업무"));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.mouse_dragged(x, y + 40.0);
        let (_, p) = a.side_drop_plan(&a.side_rows()).unwrap();
        assert_eq!((p.to, p.gap_group), (DropTo::FavItem(Some(0), 1), Some(0)));
        a.mouse_up();
        assert!(a.favorites().items.is_empty());
        assert_eq!(names_of(&a.favorites().groups[0].items), ["예산", "업무"]);
        assert_eq!(saved_favs(&a), a.favorites().clone(), "the favorites file is written");
        // 맨 위(그룹 머리 위)로 끌어내면 그룹 없는 맨 앞
        drag_side(&mut a, &g, "업무", -60.0);
        assert_eq!(names_of(&a.favorites().items), ["업무"]);
        assert_eq!(names_of(&a.favorites().groups[0].items), ["예산"]);
        // 그룹 머리 '둘째'를 '묶음' 위로: 안의 항목과 함께 옮겨진다
        drag_side(&mut a, &g, "둘째", -2.0 * SIDE_ROW_H - 4.0);
        let groups: Vec<&str> = a.favorites().groups.iter().map(|grp| grp.name.as_str()).collect();
        assert_eq!(groups, ["둘째", "묶음"]);
        assert_eq!(names_of(&a.favorites().groups[0].items), ["회의록"]);
        assert_eq!(saved_favs(&a), a.favorites().clone());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 즐겨찾기 항목을 고정한 쪽으로 끌면 이 공간에만 두고(놓은 자리에 끼운다), 고정한 줄을 '모든 공간' 쪽으로 끌면 모든 공간에 둔다
    /// (놓은 자리의 그룹에 넣는다). 메뉴의 '이 작업 공간에만 두기'·'모든 공간에 두기'와 같다
    #[test]
    fn rows_are_dragged_between_the_workspace_and_the_common_block() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        a.places = vec![("홈".to_string(), home.clone())];
        a.toggle_pin(&downloads, true);
        a.favs = Favorites {
            items: vec![Favorite { name: "업무".into(), path: work.clone(), is_dir: true }],
            groups: vec![Group { name: "묶음".into(), collapsed: false, items: vec![Favorite { name: "예산".into(), path: util::join(&work, "예산.csv"), is_dir: false }] }],
        };
        // '업무'를 고정한 첫 줄 자리로: 고정한 것의 맨 앞에 들어가고 즐겨찾기에서 빠진다
        let (from, to) = (mid(side_row(&a, "업무")), mid(side_row(&a, "다운로드")));
        drag_side(&mut a, &g, "업무", to.1 - from.1);
        assert_eq!(pin_names(&a), ["업무", "다운로드"]);
        assert_eq!(saved_pins(&a), ["업무", "다운로드"]);
        assert!(a.favorites().items.is_empty());
        // 고정한 '다운로드'를 '예산' 줄 자리로: 모든 공간의 묶음 맨 앞에 들어가고 이 공간에서 빠진다
        let (from, to) = (mid(side_row(&a, "다운로드")), mid(side_row(&a, "예산")));
        drag_side(&mut a, &g, "다운로드", to.1 - from.1);
        assert_eq!(pin_names(&a), ["업무"]);
        assert_eq!(names_of(&a.favorites().groups[0].items), ["다운로드", "예산"]);
        assert_eq!(saved_favs(&a), a.favorites().clone());
        // 고정한 것이 하나 남았을 때 그 줄을 '모든 공간'으로 끌면 안내가 다시 보인다
        let (from, to) = (mid(side_row(&a, "업무")), mid(side_row(&a, "홈")));
        drag_side(&mut a, &g, "업무", to.1 - from.1 + SIDE_ROW_H);
        assert!(pin_names(&a).is_empty() && a.side_rows()[0].kind == SideKind::Hint);
        assert_eq!(names_of(&a.favorites().items), ["업무"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 사이드바 줄은 누를 때가 아니라 뗄 때 연다. 4보다 적게 움직였으면 그대로 연다. 그룹 머리는 접고 펼친다.
    /// 바로가기는 끌 수 없어 조금 넘게 움직이면 열지 않는다. 두 번 누르기의 둘째 누름은 버린다
    #[test]
    fn a_click_without_moving_still_opens_the_row() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        a.places = vec![("다운로드".to_string(), downloads.clone())];
        a.toggle_pin(&work, true);
        a.favs.groups.push(Group { name: "묶음".into(), collapsed: false, items: vec![Favorite { name: "예산".into(), path: util::join(&work, "예산.csv"), is_dir: false }] });
        let (x, y) = mid(side_row(&a, "업무"));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.mouse_dragged(x + 6.0, y + 3.0);
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(home.clone()), "nothing opens while the button is down");
        a.mouse_up();
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(work.clone()));
        assert_eq!(pin_names(&a), ["업무"]);
        // 두 번 누르기: 첫 누름을 뗄 때 접히고, 둘째 누름은 버린다
        let (x, y) = mid(side_row(&a, "묶음"));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.mouse_up();
        a.mouse_down(&g, x, y, Mods::default(), 2);
        a.mouse_up();
        assert!(a.favorites().groups[0].collapsed);
        // 바로가기를 끌다 놓으면 열지 않는다
        let (x, y) = mid(side_row(&a, "다운로드"));
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.mouse_dragged(x, y - 20.0);
        a.mouse_up();
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(work.clone()));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 그룹 머리의 메뉴: 이름 바꾸기(그 줄에서 입력 상자), 그룹 풀기(안의 것은 남는다), 그룹 지우기(안의 것도 지운다).
    /// '모든 공간' 줄의 새 그룹 단추와 항목 메뉴의 그룹 줄들로 그룹을 만들고 옮긴다
    #[test]
    fn groups_are_made_renamed_ungrouped_and_deleted_from_the_menus() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let work = util::join(&home, "업무");
        let f = |name: &str, file: &str| Favorite { name: name.into(), path: util::join(&work, file), is_dir: false };
        a.favs.groups = vec![
            Group { name: "묶음".into(), collapsed: false, items: vec![f("예산", "예산.csv")] },
            Group { name: "둘째".into(), collapsed: false, items: vec![f("회의록", "회의록.md")] },
        ];
        assert_eq!(menu_labels(&a, MenuKind::Group(0)), ["이름 바꾸기", "그룹 풀기 (안의 즐겨찾기는 남김)", "그룹과 안의 즐겨찾기 빼기…"]);
        assert_eq!(a.menu_break(MenuKind::Group(0)), Some(1));
        assert!(a.menu_items(MenuKind::Group(5)).is_empty());
        // 이름 바꾸기: 그 머리 줄의 이름 자리에 입력 상자가 뜨고, 확정하면 바뀐다
        a.pick_menu(MenuKind::Group(0), 0);
        assert_eq!(a.edit, Editing::Group(0));
        let (head, rect) = (side_row(&a, "묶음"), a.address_rect().unwrap());
        assert!(rect.t >= head.t && rect.b <= head.b && rect.l > head.l, "{rect:?} {head:?}");
        assert_eq!((a.address_text().as_str(), a.address_select(), a.address_pad()), ("묶음", None, 2.0));
        a.commit_address("월말 정산");
        assert_eq!((a.edit.clone(), a.favorites().groups[0].name.as_str()), (Editing::None, "월말 정산"));
        // 비운 이름은 그대로 둔다
        a.pick_menu(MenuKind::Group(0), 0);
        a.commit_address("  ");
        assert_eq!(a.favorites().groups[0].name, "월말 정산");
        // 새 그룹 단추: 맨 끝에 '새 그룹'을 만들고 이름을 바로 고친다
        let rows = a.side_rows();
        let b = a.new_group_rect(&rows).unwrap();
        let (x, y) = mid(b);
        assert_eq!(a.hit(&g, x, y), Hit::NewGroup);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.mouse_up();
        assert_eq!((a.favorites().groups.len(), a.edit.clone()), (3, Editing::Group(2)));
        assert_eq!(a.address_text(), "새 그룹");
        a.commit_address("셋째");
        // 그룹 풀기: 안의 즐겨찾기는 그룹 없음 끝으로 남는다
        a.pick_menu(MenuKind::Group(0), 1);
        assert_eq!(names_of(&a.favorites().items), ["예산"]);
        assert_eq!(a.favorites().groups.iter().map(|grp| grp.name.as_str()).collect::<Vec<_>>(), ["둘째", "셋째"]);
        // 그룹 지우기: 즐겨찾기가 든 그룹은 그 머리에 한 번 더 묻고(Windows의 확인 창), '빼기'를 골라야 안의 즐겨찾기와 함께 없어진다
        a.pick_menu(MenuKind::Group(0), 2);
        assert_eq!(a.menu_shown().map(|(k, _)| k), Some(MenuKind::GroupDelete(0)));
        assert_eq!(menu_labels(&a, MenuKind::GroupDelete(0)), ["‘둘째’ 그룹과 즐겨찾기 1개 빼기", "그만두기"]);
        a.close_menu();
        a.pick_menu(MenuKind::GroupDelete(0), 1);
        assert_eq!(a.favorites().groups.len(), 2, "그만두기 keeps the group");
        a.pick_menu(MenuKind::GroupDelete(0), 0);
        assert_eq!(a.favorites().groups.iter().map(|grp| grp.name.as_str()).collect::<Vec<_>>(), ["셋째"]);
        // 빈 그룹은 묻지 않고 지운다
        assert_eq!(menu_labels(&a, MenuKind::Group(0))[2], "그룹과 안의 즐겨찾기 빼기");
        assert!(a.favorites().find(&util::join(&work, "회의록.md")).is_none());
        assert_eq!(saved_favs(&a), a.favorites().clone());
        // 항목 메뉴의 그룹 줄들: 그룹마다 하나(지금 그룹에 체크), 그룹 안이면 그룹에서 빼기, 맨 끝은 새 그룹
        assert_eq!(menu_labels(&a, MenuKind::Fav(0)), ["이 작업 공간에만 두기", "모든 공간에서 빼기", "‘셋째’ 그룹으로 이동", "새 그룹을 만들어 넣기"]);
        a.pick_menu(MenuKind::Fav(0), 2);
        assert!(a.favorites().items.is_empty());
        assert_eq!(names_of(&a.favorites().groups[0].items), ["예산"]);
        // 이제 '예산'은 셋째 머리 다음 줄 (줄 번호 1)
        let items = a.menu_items(MenuKind::Fav(1));
        assert_eq!(items[2], ("‘셋째’ 그룹으로 이동".to_string(), true));
        assert_eq!(items[3].0, "그룹에서 빼기");
        a.pick_menu(MenuKind::Fav(1), 3);
        assert_eq!(names_of(&a.favorites().items), ["예산"]);
        // 새 그룹을 만들어 넣기: 그 항목이 든 그룹을 맨 끝에 만들고 이름을 고친다
        a.pick_menu(MenuKind::Fav(0), 3);
        assert_eq!(a.favorites().groups.len(), 2);
        assert_eq!((a.favorites().groups[1].name.as_str(), names_of(&a.favorites().groups[1].items)), ("새 그룹", vec!["예산".to_string()]));
        assert_eq!(a.edit, Editing::Group(1));
        // 그 그룹이 없어지면 이름 고치기도 끝난다
        a.favs.groups.truncate(1);
        a.sync_edit();
        assert_eq!(a.edit, Editing::None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 탭 칩 위에 놓으면 그 탭이 보이는 폴더로 간다 (지금 탭이 아니어도). 지금 탭은 그대로고 그 칸이 지금 칸이 된다
    #[test]
    fn dropping_on_a_tab_goes_into_that_tabs_folder() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, g) = app(&work);
        a.navigate(1, Location::Dir(downloads.clone()), None);
        a.new_tab(1);
        a.navigate(1, Location::Dir(home.clone()), None);
        a.settle();
        let src = util::join(&work, "예산.csv");
        let (tabs, _) = a.chips(&g, 1, &geom(a.pane_rects()[1].1));
        let (x, y) = mid(tabs[0]);
        assert_eq!(a.drag_over(&g, x, y, std::slice::from_ref(&src)), Some(downloads.clone()));
        assert!(a.drop_paths(&g, x, y, vec![src.clone()], Paste::Drop));
        assert_eq!((a.drop, a.active(), a.pane(1).active), (None, 1, 1));
        a.settle();
        assert!(noted(&a, "1개를 옮겼어요"), "{:?}", a.flash);
        assert!(std::fs::metadata(util::join(&downloads, "예산.csv")).is_ok() && std::fs::metadata(&src).is_err());
        assert_eq!(a.pane(1).tab().loc, Location::Dir(home.clone()), "the current tab stays");
        // 시작 화면 탭에는 놓지 않는다
        a.set_new_tab_start(true);
        a.new_tab(0);
        a.settle();
        let (tabs, _) = a.chips(&g, 0, &geom(a.pane_rects()[0].1));
        let (x, y) = mid(tabs[1]);
        assert_eq!(a.drag_over(&g, x, y, &[util::join(&downloads, "예산.csv")]), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// '모든 공간'은 줄이 적으면 사이드바 아래 끝에 붙고, 고정한 것이 많아 넘치면 그 아래에 이어져 함께 스크롤된다
    #[test]
    fn common_block_sits_at_the_bottom_until_the_list_overflows() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let downloads = util::join(&home, "다운로드");
        a.places = vec![("홈".to_string(), home.clone()), ("다운로드".to_string(), downloads.clone())];
        a.resize(1280.0, 800.0);
        let list = a.side_list();
        // 고정한 것이 없으면 맨 위에 안내, '모든 공간'은 맨 아래
        let kinds: Vec<SideKind> = a.side_rows().iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [SideKind::Hint, SideKind::Label, SideKind::Place, SideKind::Place]);
        let hint = a.side_rect(&a.side_rows()[0]);
        let (label, last) = (side_row(&a, "모든 공간"), side_row(&a, "다운로드"));
        assert_eq!(hint.t, list.t);
        assert!((last.b - list.b).abs() < 0.5, "{last:?} {list:?}");
        assert!(label.t > hint.b + 100.0, "the pins and the common block are apart");
        assert_eq!(a.side_scroll, 0.0);
        // 안내와 구역 이름은 누를 수 없고, 바로가기 줄은 그 폴더를 연다
        assert_eq!(a.hit(&g, hint.l + 20.0, hint.t + 10.0), Hit::None);
        assert_eq!(a.hit(&g, label.l + 20.0, label.t + 10.0), Hit::None);
        a.mouse_down(&g, last.l + 40.0, last.t + 10.0, Mods::default(), 1);
        a.mouse_up();
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(downloads));
        // 고정한 것이 많고 창이 낮으면: '모든 공간'이 고정한 것 바로 아래에 이어지고, 끝까지 스크롤하면 마지막 줄이 보인다
        for i in 0..40 {
            a.workspaces.current_mut().unwrap().pin(&util::join(&home, &format!("폴더 {i}")), true);
        }
        a.resize(1280.0, 420.0);
        let (pin, label) = (side_row(&a, "폴더 39"), side_row(&a, "모든 공간"));
        assert!((label.t - pin.b - COMMON_GAP).abs() < 0.5);
        assert!(side_row(&a, "다운로드").t > a.side_list().b);
        a.scroll(&g, 100.0, 300.0, 100_000.0);
        let last = side_row(&a, "다운로드");
        assert!((last.b - a.side_list().b).abs() < 0.5 && a.side_scroll > 0.0);
        // 고정한 것이 줄면 스크롤이 풀리고 '모든 공간'은 다시 아래 끝으로
        a.workspaces.current_mut().unwrap().pins.truncate(2);
        a.resize(1280.0, 420.0);
        assert_eq!(a.side_scroll, 0.0);
        assert!((side_row(&a, "다운로드").b - a.side_list().b).abs() < 0.5);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 작업 공간 파일: 앱의 공간(이름·색·고정한 것·화면)을 파일 글자로 적었다가 읽으면 그대로고, 그것으로 다시 열 수 있다
    #[test]
    fn workspaces_survive_the_file_format_and_reopen() {
        let (root, home) = demo();
        let (mut a, _g) = app(&home);
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        a.navigate(0, Location::Dir(work.clone()), None);
        a.settle();
        a.toggle_star(0);
        a.new_workspace();
        a.commit_address("사진 정리");
        a.set_workspace_color(1, "purple");
        a.set_layout(Layout::Four);
        a.navigate(1, Location::Dir(downloads.clone()), None);
        a.settle();
        a.save_workspaces();
        let saved = workspaces::parse(&workspaces::serialize(a.workspaces())).expect("workspaces");
        assert_eq!(&saved, a.workspaces());
        let session = session::parse(&session::serialize(&a.to_session())).expect("session");
        assert_eq!(session.ws.as_deref(), Some("사진 정리"));
        // 다시 연 앱: 세션이 지금 공간의 화면이면 그 화면으로 열고, 색도 그 공간의 색이다
        let (mut b, _g) = app(&home);
        b.open_saved(saved.clone(), Some(&session), &home);
        b.settle();
        assert_eq!((b.workspaces().current_name(), b.layout()), (Some("사진 정리"), Layout::Four));
        assert_eq!(b.pane(1).tab().loc, Location::Dir(downloads));
        assert_eq!(b.theme.accent, a.theme.accent);
        assert!(pin_names(&b).is_empty());
        b.switch_workspace(0);
        b.settle();
        assert_eq!((b.layout(), pin_names(&b)), (Layout::TwoV, vec!["업무".to_string()]));
        assert_eq!(b.pane(0).tab().loc, Location::Dir(work));
        // 세션이 다른 공간의 화면이면 지금 공간에 저장된 화면으로 연다
        let mut other = session.clone();
        other.ws = Some("기본".to_string());
        other.layout = 0;
        let (mut c, _g) = app(&home);
        c.open_saved(saved, Some(&other), &home);
        assert_eq!((c.workspaces().current_name(), c.layout()), (Some("사진 정리"), Layout::Four));
        // 작업 공간 파일이 없으면 지난번 화면이 '기본' 공간이 된다
        let (mut d, _g) = app(&home);
        d.open_saved(Workspaces::default(), Some(&session), &home);
        assert_eq!((d.workspaces().list.len(), d.workspaces().current_name(), d.layout()), (1, Some("기본"), Layout::Four));
        assert_eq!(d.workspaces().list[0].state.layout, 4, "the new workspace starts from that screen");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 칸 p에서 고른 줄의 이름들(보이는 순서)과 커서 줄의 이름
    fn picked(a: &App, p: usize) -> (Vec<String>, String) {
        let t = a.pane(p).tab();
        let chosen = (0..t.view.len()).filter_map(|vi| t.entry_at(vi)).filter(|e| e.selected).map(Entry::name_string).collect();
        (chosen, t.cursor_entry().map(Entry::name_string).unwrap_or_default())
    }

    /// 새 폴더(⇧⌘N): 만들고, 다 읽으면 그 줄에서 이름을 고친다. 같은 이름이 있으면 번호가 붙는다
    #[test]
    fn new_folder_is_made_selected_and_renamed_in_place() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, _g) = app(&work);
        let shift_cmd = Mods { cmd: true, shift: true, ..Default::default() };
        assert!(a.key(Key::Char('n'), shift_cmd));
        a.settle();
        assert!(std::fs::metadata(util::join(&work, "새 폴더")).unwrap().is_dir());
        assert_eq!(names(&a, 0), ["2026 4분기", "새 폴더", "예산.csv", "회의록.md"]);
        assert_eq!(picked(&a, 0), (vec!["새 폴더".to_string()], "새 폴더".to_string()));
        assert!(names(&a, 1).contains(&"새 폴더".to_string()), "the other pane on the same folder was read again");
        // 이름 상자가 그 줄에 떠 있다. 폴더는 이름 전체를 골라 둔다
        assert!(matches!(&a.edit, Editing::Item(e) if e.name == "새 폴더" && e.is_dir));
        let (list, rect) = (a.list_rect(0).unwrap(), a.address_rect().unwrap());
        assert!(rect.t >= list.t && rect.b <= list.b && rect.l >= list.l && rect.r <= list.r, "{rect:?} in {list:?}");
        assert_eq!((a.address_text().as_str(), a.address_select()), ("새 폴더", Some(4)));
        // 확정: 끝날 때까지 상자는 그대로 있고, 끝나면 새 이름이 골라져 있다
        a.commit_address("  자료 모음 ");
        assert!(matches!(&a.edit, Editing::Item(e) if e.waiting.is_some()));
        a.settle();
        assert_eq!(a.edit, Editing::None);
        assert!(std::fs::metadata(util::join(&work, "자료 모음")).unwrap().is_dir());
        assert!(std::fs::symlink_metadata(util::join(&work, "새 폴더")).is_err());
        assert_eq!(picked(&a, 0), (vec!["자료 모음".to_string()], "자료 모음".to_string()));
        // 두 번 더 만들면 둘째에는 번호가 붙고, 이름 상자는 나중에 만든 폴더에 있다
        a.key(Key::Char('n'), shift_cmd);
        a.settle();
        assert!(matches!(&a.edit, Editing::Item(e) if e.name == "새 폴더"));
        a.cancel_address();
        a.key(Key::Char('n'), shift_cmd);
        a.settle();
        assert!(matches!(&a.edit, Editing::Item(e) if e.name == "새 폴더 2"));
        // Esc: 폴더는 만든 이름 그대로 남는다
        a.cancel_address();
        assert_eq!(a.edit, Editing::None);
        assert!(std::fs::metadata(util::join(&work, "새 폴더")).unwrap().is_dir() && std::fs::metadata(util::join(&work, "새 폴더 2")).unwrap().is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 이름 바꾸기(Return): 하나만 골랐을 때 그 줄에서. 쓸 수 없는 이름과 이미 있는 이름은 상자를 연 채로 알리고 아무것도 바꾸지 않는다
    #[test]
    fn return_renames_the_one_selected_item() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        let flashed = |a: &App, word: &str| a.flash.as_ref().is_some_and(|(p, m)| *p == 0 && m.contains(word));
        // 고른 것이 없으면 아무 일도 없다
        assert!(a.key(Key::Return, Mods::default()));
        assert_eq!(a.edit, Editing::None);
        click_name(&mut a, &g, 0, "예산.csv", Mods::default());
        a.key(Key::Return, Mods::default());
        assert!(matches!(&a.edit, Editing::Item(e) if e.name == "예산.csv" && !e.is_dir));
        assert_eq!((a.address_text().as_str(), a.address_select()), ("예산.csv", Some(2)), "the extension is left out of the selection");
        // 그대로 두면 아무 일 없이 끝난다
        a.commit_address("예산.csv");
        assert_eq!((a.edit.clone(), a.take_ops().len()), (Editing::None, 0));
        // 쓸 수 없는 이름, 숨김이 되는 이름, 목록에 이미 있는 이름: 상자는 그대로, 알림만
        a.key(Key::Return, Mods::default());
        a.commit_address("예산/최종.csv");
        assert!(matches!(a.edit, Editing::Item(_)) && flashed(&a, "/"), "{:?}", a.flash);
        a.commit_address(".예산");
        assert!(matches!(a.edit, Editing::Item(_)) && flashed(&a, "숨김"), "{:?}", a.flash);
        a.commit_address("회의록.md");
        assert!(matches!(a.edit, Editing::Item(_)) && flashed(&a, "이미 있어요"), "{:?}", a.flash);
        assert_eq!(a.take_ops().len(), 0, "nothing was asked of the disk");
        // 목록에는 아직 없지만 디스크에는 있는 이름 (목록이 낡았다): 실행할 때 걸리고, 두 파일 모두 그대로다
        std::fs::write(util::join(&work, "몰래 생긴 파일.csv"), b"other").unwrap();
        a.flash = None;
        a.commit_address("몰래 생긴 파일.csv");
        a.settle();
        assert!(matches!(&a.edit, Editing::Item(e) if e.waiting.is_none()), "the box stays open for another name: {:?}", a.edit);
        assert!(flashed(&a, "이미 있어요"), "{:?}", a.flash);
        assert_eq!(std::fs::read(util::join(&work, "몰래 생긴 파일.csv")).unwrap(), b"other");
        assert_eq!(std::fs::read(util::join(&work, "예산.csv")).unwrap(), b"x");
        assert!(names(&a, 0).contains(&"몰래 생긴 파일.csv".to_string()), "the list was read again");
        // 다른 이름으로 확정: 파일이 바뀌고 그 이름이 골라진다
        a.commit_address("예산 최종.csv");
        a.settle();
        assert_eq!(a.edit, Editing::None);
        assert_eq!(std::fs::read(util::join(&work, "예산 최종.csv")).unwrap(), b"x");
        assert!(std::fs::symlink_metadata(util::join(&work, "예산.csv")).is_err());
        assert_eq!(picked(&a, 0), (vec!["예산 최종.csv".to_string()], "예산 최종.csv".to_string()));
        // 여럿을 골랐으면 고치지 않고 알린다
        a.select_all();
        a.key(Key::Return, Mods::default());
        assert!(a.edit == Editing::None && flashed(&a, "하나씩"), "{:?}", a.flash);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 목록이 낡았을 때: 고친 이름의 파일이 그 사이 다른 파일로 바뀌었으면 이름을 바꾸지 않는다 (파일 번호가 다르다)
    #[test]
    fn rename_is_refused_when_another_file_took_the_name() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        click_name(&mut a, &g, 0, "회의록.md", Mods::default());
        a.key(Key::Return, Mods::default());
        // 앱 밖에서: 원래 파일은 다른 이름이 되고, 같은 이름의 새 파일이 생겼다
        std::fs::rename(util::join(&work, "회의록.md"), util::join(&work, "옮긴 회의록.md")).unwrap();
        std::fs::write(util::join(&work, "회의록.md"), b"new").unwrap();
        a.commit_address("11월 회의록.md");
        a.settle();
        assert_eq!(a.edit, Editing::None);
        assert!(a.flash.as_ref().is_some_and(|(_, m)| m.contains("바뀌었어요")), "{:?}", a.flash);
        assert_eq!(std::fs::read(util::join(&work, "회의록.md")).unwrap(), b"new");
        assert_eq!(std::fs::read(util::join(&work, "옮긴 회의록.md")).unwrap(), b"x");
        assert!(std::fs::symlink_metadata(util::join(&work, "11월 회의록.md")).is_err());
        assert!(names(&a, 0).contains(&"옮긴 회의록.md".to_string()), "the list was read again");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 이름을 고치는 중에 그 탭이 다른 곳으로 가거나 그 줄이 보이지 않게 되면 고치기가 끝나고, 확정해도 아무것도 바꾸지 않는다
    #[test]
    fn rename_ends_when_its_row_is_no_longer_there() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        let begin = |a: &mut App, p: usize| {
            click_name(a, &g, p, "예산.csv", Mods::default());
            a.key(Key::Return, Mods::default());
            assert!(matches!(&a.edit, Editing::Item(e) if e.pane == p));
        };
        // 늦게 끝난 경로 확인이 그 탭을 다른 폴더로 옮겼다: 그 폴더의 줄에는 상자가 뜨지 않고, 확정해도 바뀌지 않는다
        begin(&mut a, 0);
        a.navigate(0, Location::Dir(home.clone()), None);
        a.settle();
        assert!(a.address_rect().is_none());
        a.commit_address("바뀐 이름.csv");
        a.settle();
        assert!(a.edit == Editing::None && a.flash.as_ref().is_some_and(|(_, m)| m.contains("목록이 바뀌어")), "{:?}", a.flash);
        assert_eq!(std::fs::read(util::join(&work, "예산.csv")).unwrap(), b"x");
        assert!(std::fs::symlink_metadata(util::join(&work, "바뀐 이름.csv")).is_err() && std::fs::symlink_metadata(util::join(&home, "바뀐 이름.csv")).is_err());
        // 그 칸의 탭이 바뀌었다
        a.navigate(0, Location::Dir(work.clone()), None);
        a.settle();
        begin(&mut a, 0);
        a.new_tab(0);
        a.sync_edit();
        assert_eq!(a.edit, Editing::None);
        // 칸 나누기가 바뀌어 그 칸이 보이지 않는다
        a.settle();
        begin(&mut a, 1);
        a.set_layout(Layout::One);
        a.sync_edit();
        assert_eq!(a.edit, Editing::None);
        // 그 항목이 목록에서 사라졌다 (앱 밖에서 지워졌다)
        a.set_layout(Layout::TwoV);
        a.settle();
        begin(&mut a, 1);
        std::fs::remove_file(util::join(&work, "예산.csv")).unwrap();
        a.refresh(1);
        a.settle();
        a.sync_edit();
        assert_eq!(a.edit, Editing::None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 이름을 고치는 칸은 스크롤해도 움직이지 않는다 (그 줄이 밀려나 치던 이름을 잃지 않게). 그만두면 다시 움직인다
    #[test]
    fn list_does_not_scroll_while_a_name_is_edited() {
        let (root, home) = demo();
        let long = util::join(&home, "긴 폴더");
        for i in 0..100 {
            std::fs::create_dir_all(util::join(&long, &format!("sub_{i:03}"))).unwrap();
        }
        let (mut a, g) = app(&long);
        let (x, y) = row_point(&a, 0, 2);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.key(Key::Return, Mods::default());
        assert!(matches!(a.edit, Editing::Item(_)));
        a.scroll(&g, x, y, 300.0);
        assert_eq!(a.pane(0).tab().scroll, 0.0);
        // 다른 칸은 움직인다
        let (x1, y1) = row_point(&a, 1, 2);
        a.scroll(&g, x1, y1, 300.0);
        assert!(a.pane(1).tab().scroll > 0.0);
        a.cancel_address();
        a.scroll(&g, x, y, 300.0);
        assert!(a.pane(0).tab().scroll > 0.0);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 이름을 바꾼 항목은 다른 칸에서도 고른 것에서 빠지고, 다른 항목의 선택은 그대로 남는다. 보이지 않던 탭은 보일 때 다시 읽는다
    #[test]
    fn other_panes_and_hidden_tabs_follow_a_rename() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        // 오른쪽 칸(같은 폴더)에서 두 파일을 골라 둔다
        click_name(&mut a, &g, 1, "예산.csv", Mods::default());
        click_name(&mut a, &g, 1, "회의록.md", Mods { cmd: true, ..Default::default() });
        assert_eq!(picked(&a, 1).0, ["예산.csv", "회의록.md"]);
        // 왼쪽 칸에는 새 탭을 열어, 처음 탭이 보이지 않게 한다
        a.new_tab(0);
        a.settle();
        click_name(&mut a, &g, 0, "예산.csv", Mods::default());
        a.key(Key::Return, Mods::default());
        a.commit_address("예산 최종.csv");
        a.settle();
        assert_eq!(picked(&a, 0).0, ["예산 최종.csv"]);
        assert_eq!(picked(&a, 1).0, ["회의록.md"], "the renamed file is no longer selected there, the other one still is");
        assert!(names(&a, 1).contains(&"예산 최종.csv".to_string()), "the other pane was read again");
        // 보이지 않던 탭은 그대로 두었다가, 보이게 될 때 다시 읽는다
        assert!(!a.pane(0).tabs[0].entries.iter().any(|e| e.name_string() == "예산 최종.csv"));
        a.activate_tab(0, 0);
        a.settle();
        assert!(names(&a, 0).contains(&"예산 최종.csv".to_string()));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 파일 번호는 한 볼륨 안에서만 서로 다르다: 이름이 바뀐 항목을 고른 것에서 뺄 때, 다른 폴더를 보이는 탭의 항목은
    /// 번호가 같아도 건드리지 않는다
    #[test]
    fn a_rename_does_not_deselect_items_of_other_folders() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, g) = app(&work);
        a.navigate(1, Location::Dir(downloads), None);
        a.settle();
        click_name(&mut a, &g, 1, "사진.jpg", Mods::default());
        click_name(&mut a, &g, 0, "예산.csv", Mods::default());
        // 다른 볼륨의 파일이 우연히 같은 번호인 경우를 흉내 낸다
        let id = a.pane(0).tab().cursor_entry().unwrap().file_id;
        a.tab_mut(1).entries.iter_mut().for_each(|e| e.file_id = id);
        a.key(Key::Return, Mods::default());
        a.commit_address("예산 최종.csv");
        a.settle();
        assert_eq!(picked(&a, 0).0, ["예산 최종.csv"]);
        assert_eq!(picked(&a, 1).0, ["사진.jpg"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 새 폴더(⌘⇧N, 빈 곳 메뉴): 만들고 고른 채 이름을 고친다. 파일을 만들거나 고치는 키는 누르고 있어도 되풀이하지 않는다
    #[test]
    fn new_folder_is_made_from_the_key() {
        let (root, home) = demo();
        let (mut a, _g) = app(&home);
        a.key(Key::Char('n'), Mods { cmd: true, shift: true, ..Mods::default() });
        a.settle();
        assert!(std::fs::metadata(util::join(&home, "새 폴더")).unwrap().is_dir());
        assert!(matches!(&a.edit, Editing::Item(e) if e.name == "새 폴더"));
        assert!(!Key::Return.repeats() && !Key::Char('n').repeats() && Key::Down.repeats() && Key::Char('t').repeats());
        // 만들 수 없는 곳(없어진 폴더): 알림만 뜬다
        a.cancel_address();
        let gone = util::join(&home, "다운로드");
        a.navigate(0, Location::Dir(gone.clone()), None);
        a.settle();
        std::fs::remove_dir_all(&gone).unwrap();
        a.new_folder(0);
        a.settle();
        assert!(a.edit == Editing::None && a.flash.as_ref().is_some_and(|(_, m)| m.contains("폴더를 만들지 못했어요")), "{:?}", a.flash);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 잠깐 뜨는 알림 글에 그 말이 들어 있는지
    fn noted(a: &App, word: &str) -> bool {
        a.flash.as_ref().is_some_and(|(_, m)| m.contains(word))
    }

    /// 휴지통(⌘⌫): 실제로 고른 것만 간다. ⌘Z로 되돌리면 원래 자리로 오고 다시 골라진다
    #[test]
    fn trash_takes_the_selected_items_and_undo_brings_them_back() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        let cmd = Mods { cmd: true, ..Default::default() };
        // 고른 것이 없으면(커서만 놓여 있으면) 아무 일도 없다
        assert!(a.key(Key::Delete, cmd));
        a.settle();
        assert_eq!(names(&a, 0), ["2026 4분기", "예산.csv", "회의록.md"]);
        // ⌘ 없이 누른 ⌫는 아무것도 하지 않는다
        click_name(&mut a, &g, 0, "예산.csv", Mods::default());
        assert!(!a.key(Key::Delete, Mods::default()));
        click_name(&mut a, &g, 0, "회의록.md", cmd);
        a.key(Key::Delete, cmd);
        a.settle();
        assert_eq!(names(&a, 0), ["2026 4분기"]);
        assert_eq!(names(&a, 1), ["2026 4분기"], "the other pane on the same folder follows");
        assert!(noted(&a, "2개를 휴지통으로 옮겼어요"), "{:?}", a.flash);
        assert!(picked(&a, 0).0.is_empty());
        // 없어진 것이 아니다: 되돌리면 그 파일들이 그대로 돌아오고 골라진다
        a.undo();
        a.settle();
        assert_eq!(names(&a, 0), ["2026 4분기", "예산.csv", "회의록.md"]);
        assert_eq!(picked(&a, 0).0, ["예산.csv", "회의록.md"]);
        assert_eq!((std::fs::read(util::join(&work, "예산.csv")).unwrap(), std::fs::read(util::join(&work, "회의록.md")).unwrap()), (b"x".to_vec(), b"x".to_vec()));
        assert!(noted(&a, "2개를 휴지통에서 되돌려 놓았어요"), "{:?}", a.flash);
        // 더 되돌릴 것이 없다
        a.undo();
        assert!(noted(&a, "되돌릴 작업이 없어요"), "{:?}", a.flash);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 낡은 목록에서 휴지통: 그 사이 다른 파일로 바뀐 것은 보내지 않고 그대로 둔다. 보낸 것만 되돌리기 기록에 남는다
    #[test]
    fn trash_leaves_an_item_that_changed_under_the_list() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        let cmd = Mods { cmd: true, ..Default::default() };
        click_name(&mut a, &g, 0, "예산.csv", Mods::default());
        click_name(&mut a, &g, 0, "회의록.md", cmd);
        // 앱 밖에서: 회의록은 다른 이름이 되고, 같은 이름의 새 파일이 생겼다
        std::fs::rename(util::join(&work, "회의록.md"), util::join(&work, "옮긴 회의록.md")).unwrap();
        std::fs::write(util::join(&work, "회의록.md"), b"new").unwrap();
        a.key(Key::Delete, cmd);
        a.settle();
        assert_eq!(names(&a, 0), ["2026 4분기", "옮긴 회의록.md", "회의록.md"]);
        assert_eq!(std::fs::read(util::join(&work, "회의록.md")).unwrap(), b"new");
        assert_eq!(std::fs::read(util::join(&work, "옮긴 회의록.md")).unwrap(), b"x");
        assert!(noted(&a, "1개를 휴지통으로 옮겼고 1개는 그대로 뒀어요"), "{:?}", a.flash);
        // 되돌리면 보낸 것 하나만 돌아온다
        a.undo();
        a.settle();
        assert_eq!(names(&a, 0), ["2026 4분기", "예산.csv", "옮긴 회의록.md", "회의록.md"]);
        assert_eq!(picked(&a, 0).0, ["예산.csv"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 되돌리기(⌘Z): 나중에 한 것부터. 이름 바꾸기는 원래 이름으로, 만든 폴더는 비어 있을 때만 휴지통으로. 이름을 치는 중에는 하지 않는다
    #[test]
    fn undo_reverts_new_folder_and_rename_in_order() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        let folder = util::join(&work, "새 폴더");
        // 이름 바꾸기, 새 폴더 순으로 한다
        click_name(&mut a, &g, 0, "예산.csv", Mods::default());
        a.key(Key::Return, Mods::default());
        a.commit_address("예산 최종.csv");
        a.settle();
        a.new_folder(0);
        a.settle();
        // 이름 상자가 떠 있는 동안의 ⌘Z는 파일 작업을 되돌리지 않는다
        assert!(matches!(a.edit, Editing::Item(_)));
        a.undo();
        a.settle();
        assert!(std::fs::metadata(&folder).unwrap().is_dir() && matches!(a.edit, Editing::Item(_)));
        a.cancel_address();
        // 폴더에 무엇을 넣었으면 그대로 둔다 (넣은 것을 함께 휴지통으로 보내지 않는다)
        std::fs::write(util::join(&folder, "넣은 파일.txt"), b"x").unwrap();
        a.undo();
        a.settle();
        assert!(noted(&a, "되돌리지 못했어요") && noted(&a, "들어 있어"), "{:?}", a.flash);
        assert_eq!(std::fs::read(util::join(&folder, "넣은 파일.txt")).unwrap(), b"x");
        // 그다음 ⌘Z는 그 앞의 작업(이름 바꾸기)을 되돌린다
        a.undo();
        a.settle();
        assert_eq!(std::fs::read(util::join(&work, "예산.csv")).unwrap(), b"x");
        assert!(std::fs::symlink_metadata(util::join(&work, "예산 최종.csv")).is_err());
        assert_eq!(picked(&a, 0).0, ["예산.csv"]);
        assert!(noted(&a, "예산.csv"), "{:?}", a.flash);
        // 새 폴더를 하나 더 만들고 비어 있는 채로 되돌리면 휴지통으로 간다
        a.new_folder(0);
        a.settle();
        a.cancel_address();
        assert!(std::fs::metadata(util::join(&work, "새 폴더 2")).unwrap().is_dir());
        a.undo();
        a.settle();
        assert!(std::fs::symlink_metadata(util::join(&work, "새 폴더 2")).is_err());
        assert!(!names(&a, 0).contains(&"새 폴더 2".to_string()) && names(&a, 0).contains(&"새 폴더".to_string()));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 되돌릴 자리에 다른 것이 생겼으면 덮어쓰지 않는다: 이름 바꾸기는 그대로 두고, 휴지통의 것은 휴지통에 둔다
    #[test]
    fn undo_never_replaces_what_took_the_old_name() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        let cmd = Mods { cmd: true, ..Default::default() };
        // 이름을 바꾼 뒤, 원래 이름으로 새 파일이 생겼다
        click_name(&mut a, &g, 0, "예산.csv", Mods::default());
        a.key(Key::Return, Mods::default());
        a.commit_address("예산 최종.csv");
        a.settle();
        std::fs::write(util::join(&work, "예산.csv"), b"new").unwrap();
        a.undo();
        a.settle();
        assert!(noted(&a, "원래 이름을 다른 항목이 쓰고 있어요"), "{:?}", a.flash);
        assert_eq!((std::fs::read(util::join(&work, "예산.csv")).unwrap(), std::fs::read(util::join(&work, "예산 최종.csv")).unwrap()), (b"new".to_vec(), b"x".to_vec()));
        // 휴지통으로 보낸 뒤, 원래 이름으로 새 파일이 생겼다
        click_name(&mut a, &g, 0, "회의록.md", Mods::default());
        a.key(Key::Delete, cmd);
        a.settle();
        std::fs::write(util::join(&work, "회의록.md"), b"new").unwrap();
        a.undo();
        a.settle();
        assert!(noted(&a, "휴지통에 그대로 있어요") && noted(&a, "직접 꺼내"), "{:?}", a.flash);
        assert_eq!(std::fs::read(util::join(&work, "회의록.md")).unwrap(), b"new");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 폴더 이름을 바꾸면 그 안을 보던 탭이 새 경로로 따라오고, 휴지통으로 보내면 남아 있는 위 폴더로 올라간다
    #[test]
    fn tabs_inside_a_renamed_or_trashed_folder_follow() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let quarter = util::join(&work, "2026 4분기");
        let (mut a, g) = app(&work);
        // 오른쪽 칸은 그 폴더 안에 들어가 있고, 왼쪽 칸의 보이지 않는 탭도 그 안에 있다
        a.navigate(1, Location::Dir(quarter.clone()), None);
        a.navigate(0, Location::Dir(quarter.clone()), None);
        a.settle();
        a.new_tab(0);
        a.navigate(0, Location::Dir(work.clone()), None);
        a.settle();
        click_name(&mut a, &g, 0, "2026 4분기", Mods::default());
        a.key(Key::Return, Mods::default());
        a.commit_address("2027 1분기");
        a.settle();
        let moved = util::join(&work, "2027 1분기");
        assert_eq!(a.pane(1).tab().loc, Location::Dir(moved.clone()));
        assert_eq!(names(&a, 1), ["보고서.docx"], "the tab reads the folder under its new name");
        assert_eq!(a.pane(0).tabs[0].loc, Location::Dir(moved.clone()), "a hidden tab follows too");
        assert_eq!(a.pane(0).tab().back.last(), Some(&Location::Dir(moved.clone())), "and so does the history");
        // 되돌리면 탭도 원래 경로로 돌아온다
        a.undo();
        a.settle();
        assert_eq!(a.pane(1).tab().loc, Location::Dir(quarter.clone()));
        assert_eq!(names(&a, 1), ["보고서.docx"]);
        // 휴지통으로 보내면 그 안을 보던 탭은 위 폴더로 올라가고, 그 안으로 돌아가는 기록은 빠진다
        click_name(&mut a, &g, 0, "2026 4분기", Mods::default());
        a.key(Key::Delete, Mods { cmd: true, ..Default::default() });
        a.settle();
        assert_eq!(a.pane(1).tab().loc, Location::Dir(work.clone()));
        assert!(matches!(a.pane(1).tab().state, LoadState::Ready) && !names(&a, 1).contains(&"2026 4분기".to_string()));
        assert_eq!(a.pane(0).tabs[0].loc, Location::Dir(work.clone()));
        assert!(!a.pane(0).tab().back.contains(&Location::Dir(quarter.clone())));
        // 보이지 않던 탭은 보이게 될 때 위 폴더를 읽는다
        a.activate_tab(0, 0);
        a.settle();
        assert!(matches!(a.pane(0).tab().state, LoadState::Ready) && names(&a, 0).contains(&"예산.csv".to_string()));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 목록의 오른쪽 단추 메뉴: 고르지 않은 줄이면 그 줄만 고르고, 메뉴를 연 때의 항목에 한다. 빈 곳은 새 폴더와 새로 고침
    #[test]
    fn list_menu_acts_on_the_rows_it_was_opened_for() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        let menu = |a: &mut App, name: &str| {
            let vi = names(a, 0).iter().position(|n| n == name).unwrap();
            let (x, y) = row_point(a, 0, vi);
            assert!(a.right_down(&g, x, y));
            let (kind, at) = take_menu(a).unwrap();
            assert_eq!((kind, at.l, at.b + 4.0), (MenuKind::Rows, x, y), "the menu opens where the row was pressed");
        };
        // 고르지 않은 줄을 오른쪽 단추로: 그 줄만 골라진다
        click_name(&mut a, &g, 0, "2026 4분기", Mods::default());
        menu(&mut a, "예산.csv");
        assert_eq!(picked(&a, 0).0, ["예산.csv"]);
        assert_eq!(menu_labels(&a, MenuKind::Rows), ["열기", "Finder에서 보기", "이름 바꾸기", "잘라내기", "복사", "복제", "휴지통으로 이동"]);
        assert_eq!(a.menu_break(MenuKind::Rows), Some(2));
        // Finder에서 보기, 열기(파일은 macOS '열기'로 넘긴다)
        a.pick_menu(MenuKind::Rows, 1);
        assert_eq!(a.reveal_files, [util::join(&work, "예산.csv")]);
        a.pick_menu(MenuKind::Rows, 0);
        assert!(a.open_files.is_empty(), "a menu acts once: its target is used up");
        menu(&mut a, "예산.csv");
        a.pick_menu(MenuKind::Rows, 0);
        assert_eq!(a.open_files, [util::join(&work, "예산.csv")]);
        // 이름 바꾸기: 그 줄에서 시작한다
        menu(&mut a, "예산.csv");
        a.pick_menu(MenuKind::Rows, 2);
        assert!(matches!(&a.edit, Editing::Item(e) if e.name == "예산.csv"));
        // 이름을 고치는 중에 오른쪽 단추를 누르면 고치기는 끝난다
        menu(&mut a, "예산.csv");
        assert_eq!(a.edit, Editing::None);
        // 여럿을 고른 채 그중 하나를 오른쪽 단추로: 고른 것 모두의 메뉴 (이름 바꾸기는 없다)
        click_name(&mut a, &g, 0, "예산.csv", Mods::default());
        click_name(&mut a, &g, 0, "회의록.md", Mods { cmd: true, ..Default::default() });
        menu(&mut a, "회의록.md");
        assert_eq!(picked(&a, 0).0, ["예산.csv", "회의록.md"]);
        assert_eq!(menu_labels(&a, MenuKind::Rows), ["열기", "Finder에서 보기", "잘라내기", "복사", "복제", "휴지통으로 이동"]);
        // 메뉴가 떠 있는 동안 고른 것이 바뀌어도, 메뉴를 열 때의 두 항목만 휴지통으로 간다
        a.select_all();
        a.pick_menu(MenuKind::Rows, 5);
        a.settle();
        assert_eq!(names(&a, 0), ["2026 4분기"]);
        // 폴더 하나의 열기는 그리로 들어간다
        menu(&mut a, "2026 4분기");
        a.pick_menu(MenuKind::Rows, 0);
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(util::join(&work, "2026 4분기")));
        a.go_back(0);
        a.settle();
        // 메뉴를 연 뒤 그 탭이 다른 곳으로 갔으면 아무것도 하지 않는다
        menu(&mut a, "2026 4분기");
        a.navigate(0, Location::Dir(home.clone()), None);
        a.settle();
        a.pick_menu(MenuKind::Rows, 6);
        a.settle();
        assert!(std::fs::metadata(util::join(&work, "2026 4분기")).unwrap().is_dir());
        // 빈 곳: 고른 것을 풀고 그 폴더의 메뉴를 연다. ⌃-누르기도 같다
        click_name(&mut a, &g, 0, "업무", Mods::default());
        let l = a.list_rect(0).unwrap();
        a.mouse_down(&g, l.l + 60.0, l.b - 10.0, Mods { ctrl: true, ..Default::default() }, 1);
        assert_eq!(take_menu(&mut a).map(|(k, _)| k), Some(MenuKind::Blank));
        assert!(picked(&a, 0).0.is_empty());
        assert_eq!(menu_labels(&a, MenuKind::Blank), ["새 폴더", "붙여넣기", "새로 고침"]);
        a.pick_menu(MenuKind::Blank, 0);
        a.settle();
        assert!(std::fs::metadata(util::join(&home, "새 폴더")).unwrap().is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 복사(⌘C)와 붙여넣기(⌘V): 고른 것을 클립보드에 담아 달라고 하고, 받은 경로들을 지금 칸의 폴더로 복사한다.
    /// 원본은 그대로이고, 같은 이름이 있으면 덮어쓰지 않고 번호를 붙인다. 되돌리면 만든 복사본만 휴지통으로 간다
    #[test]
    fn copy_and_paste_between_panes() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, g) = app(&work);
        a.navigate(1, Location::Dir(downloads.clone()), None);
        a.settle();
        // 고른 것이 없으면 담지 않는다
        a.copy_selected();
        assert!(a.clip_out.is_none());
        click_name(&mut a, &g, 0, "2026 4분기", Mods::default());
        click_name(&mut a, &g, 0, "예산.csv", Mods { cmd: true, ..Default::default() });
        a.copy_selected();
        let clip = a.clip_out.take().unwrap();
        assert_eq!(clip, [util::join(&work, "2026 4분기"), util::join(&work, "예산.csv")]);
        assert!(noted(&a, "2개를 복사해 뒀어요"), "{:?}", a.flash);
        // 오른쪽 칸에 붙여넣는다: 붙여넣어 달라고 하면 창이 클립보드를 읽어 넘긴다
        a.activate_pane(1);
        a.ask_paste(false);
        assert_eq!(a.clip_in.take(), Some(false));
        a.paste_paths(clip.clone(), Paste::Copy);
        assert_eq!(a.copy_progress(), Some((1, 2)), "one item at a time");
        a.settle();
        assert_eq!(a.copy_progress(), None);
        assert_eq!(names(&a, 1), ["2026 4분기", "사진.jpg", "예산.csv"]);
        assert_eq!(picked(&a, 1).0, ["2026 4분기", "예산.csv"], "the copies are selected");
        assert!(noted(&a, "2개를 복사했어요") && !noted(&a, "번호"), "{:?}", a.flash);
        assert_eq!(std::fs::read(util::join(&util::join(&downloads, "2026 4분기"), "보고서.docx")).unwrap(), b"x", "folders come whole");
        // 원본은 그대로다
        assert_eq!(names(&a, 0), ["2026 4분기", "예산.csv", "회의록.md"]);
        assert_eq!(std::fs::read(util::join(&work, "예산.csv")).unwrap(), b"x");
        // 한 번 더 붙여넣으면 덮어쓰지 않고 번호를 붙이고, 그렇게 했다고 알린다
        std::fs::write(util::join(&downloads, "예산.csv"), b"mine").unwrap();
        a.paste_paths(clip.clone(), Paste::Copy);
        a.settle();
        assert_eq!(names(&a, 1).len(), 5);
        assert!(names(&a, 1).contains(&"2026 4분기 2".to_string()) && names(&a, 1).contains(&"예산 2.csv".to_string()));
        assert_eq!(std::fs::read(util::join(&downloads, "예산.csv")).unwrap(), b"mine", "what was there is not replaced");
        assert_eq!(std::fs::read(util::join(&downloads, "예산 2.csv")).unwrap(), b"x");
        assert!(noted(&a, "2개를 복사했어요") && noted(&a, "2개에는 번호를 붙였어요"), "{:?}", a.flash);
        // 되돌리기: 방금 만든 복사본 둘만 휴지통으로 간다. 그 복사본 안을 보던 칸은 위 폴더로 올라간다
        a.navigate(0, Location::Dir(util::join(&downloads, "2026 4분기 2")), None);
        a.settle();
        assert_eq!(names(&a, 0), ["보고서.docx"]);
        a.undo();
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(downloads.clone()));
        assert!(matches!(a.pane(0).tab().state, LoadState::Ready));
        assert_eq!(names(&a, 1), ["2026 4분기", "사진.jpg", "예산.csv"]);
        assert_eq!(std::fs::read(util::join(&downloads, "예산.csv")).unwrap(), b"mine");
        assert!(noted(&a, "복사본 2개를 휴지통으로 보냈어요"), "{:?}", a.flash);
        // 클립보드에 파일이 없으면 알림만 뜬다
        a.paste_paths(Vec::new(), Paste::Copy);
        assert!(noted(&a, "붙여넣을 파일이") && a.copy_progress().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 복제(⌘D)는 같은 폴더에 번호를 붙여 만든다. 복사 중에 Esc를 누르면 하던 것만 마치고 그만둔다
    #[test]
    fn duplicate_numbers_the_copy_and_escape_stops_a_batch() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, g) = app(&work);
        click_name(&mut a, &g, 0, "예산.csv", Mods::default());
        assert!(a.key(Key::Char('d'), Mods { cmd: true, ..Default::default() }));
        a.settle();
        assert_eq!(std::fs::read(util::join(&work, "예산 2.csv")).unwrap(), b"x");
        assert_eq!(picked(&a, 0).0, ["예산 2.csv"]);
        assert!(noted(&a, "1개를 복사했어요") && !noted(&a, "번호"), "{:?}", a.flash);
        // 넷을 다른 폴더에 붙여넣다가 그만둔다: 맡긴 하나(맨 위의 폴더)는 마치고 나머지 셋은 하지 않는다
        a.select_all();
        a.copy_selected();
        let clip = a.clip_out.take().unwrap();
        assert_eq!((clip.len(), clip[0].as_str()), (4, util::join(&work, "2026 4분기").as_str()));
        a.navigate(1, Location::Dir(downloads.clone()), None);
        a.settle();
        a.activate_pane(1);
        a.paste_paths(clip, Paste::Copy);
        assert_eq!(a.copy_progress(), Some((1, 4)));
        // 복사 중에는 다른 붙여넣기를 받지 않는다
        a.paste_paths(vec![util::join(&work, "회의록.md")], Paste::Copy);
        assert!(noted(&a, "앞의 복사가 끝난 뒤"), "{:?}", a.flash);
        assert!(a.key(Key::Esc, Mods::default()));
        a.settle();
        assert_eq!(a.copy_progress(), None);
        assert_eq!(names(&a, 1), ["2026 4분기", "사진.jpg"]);
        assert!(noted(&a, "1개를 복사했어요") && noted(&a, "그만둬서 3개는 하지 않았어요"), "{:?}", a.flash);
        // 폴더를 자기 안에 붙여넣으려 하면 하지 못했다고 알리고 아무것도 생기지 않는다
        let quarter = util::join(&work, "2026 4분기");
        a.navigate(1, Location::Dir(quarter.clone()), None);
        a.settle();
        a.paste_paths(vec![quarter.clone()], Paste::Copy);
        a.settle();
        assert!(noted(&a, "복사하지 못했어요") && noted(&a, "자기 안"), "{:?}", a.flash);
        assert_eq!(names(&a, 1), ["보고서.docx"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 잘라내기(⌘X)와 붙여넣기(⌘V): 붙여넣을 때 옮겨진다. 같은 파일이 그대로 가고, 옮긴 폴더 안을 보던 칸은 따라온다.
    /// 되돌리면 원래 자리로 온다. 옮기는 곳의 것을 덮어쓰지 않는다
    #[test]
    fn cut_and_paste_moves_and_undo_moves_back() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let quarter = util::join(&work, "2026 4분기");
        let (mut a, g) = app(&work);
        a.navigate(1, Location::Dir(downloads.clone()), None);
        a.settle();
        // 세 번째 칸은 옮길 폴더 안을 보고 있다
        a.set_layout(Layout::Three);
        a.navigate(2, Location::Dir(quarter.clone()), None);
        a.settle();
        click_name(&mut a, &g, 0, "2026 4분기", Mods::default());
        click_name(&mut a, &g, 0, "예산.csv", Mods { cmd: true, ..Default::default() });
        let file = a.pane(0).tab().entries.iter().find(|e| e.name_string() == "예산.csv").unwrap().file_id;
        a.cut_selected();
        let clip = a.clip_out.take().unwrap();
        assert_eq!(clip, [quarter.clone(), util::join(&work, "예산.csv")]);
        assert!(std::mem::take(&mut a.clip_cut) && noted(&a, "2개를 잘라 뒀어요"), "{:?}", a.flash);
        // 잘라 두기만 해서는 아무것도 옮겨지지 않는다
        assert!(std::fs::metadata(&quarter).unwrap().is_dir() && std::fs::metadata(util::join(&work, "예산.csv")).is_ok());
        // 같은 폴더에 붙여넣으면 옮길 것이 없다. 옮기지 못한 것은 잘라 둔 채로 남아, 다른 곳에 다시 붙여넣을 수 있다
        assert!(a.paste_paths(clip.clone(), Paste::Cut(7)));
        assert_eq!(a.clip_left, None, "nothing is decided until the batch ends");
        a.settle();
        assert!(noted(&a, "옮기지 못했어요") && noted(&a, "이미 이 폴더에 있어요"), "{:?}", a.flash);
        assert_eq!(names(&a, 0), ["2026 4분기", "예산.csv", "회의록.md"]);
        assert_eq!(a.clip_left.take(), Some((7, clip.clone())), "with the mark of the cut it belongs to");
        // 다른 칸에 붙여넣는다: 하나씩 옮겨진다. 모두 옮겨 갔으면 잘라 둔 것은 남지 않는다
        a.activate_pane(1);
        assert!(a.paste_paths(clip.clone(), Paste::Cut(7)));
        assert_eq!(a.copy_progress(), Some((1, 2)));
        a.settle();
        assert_eq!(a.clip_left.take(), Some((7, Vec::new())));
        assert_eq!(names(&a, 1), ["2026 4분기", "사진.jpg", "예산.csv"]);
        assert_eq!(picked(&a, 1).0, ["2026 4분기", "예산.csv"], "what was moved is selected");
        assert_eq!(names(&a, 0), ["회의록.md"]);
        assert!(noted(&a, "2개를 옮겼어요") && !noted(&a, "번호"), "{:?}", a.flash);
        assert_eq!(sakadu::fileops::inode(&util::join(&downloads, "예산.csv")).unwrap(), file, "the same file, not a copy");
        // 옮긴 폴더 안을 보던 칸은 새 경로로 따라온다
        let moved = util::join(&downloads, "2026 4분기");
        assert_eq!(a.pane(2).tab().loc, Location::Dir(moved));
        assert_eq!(names(&a, 2), ["보고서.docx"]);
        // 되돌리기: 둘 다 원래 자리로 가고 거기서 골라진다. 그 칸도 원래 경로로 돌아온다
        a.undo();
        a.settle();
        assert_eq!(names(&a, 0), ["2026 4분기", "예산.csv", "회의록.md"]);
        assert_eq!(picked(&a, 0).0, ["2026 4분기", "예산.csv"]);
        assert_eq!(names(&a, 1), ["사진.jpg"]);
        assert_eq!(a.pane(2).tab().loc, Location::Dir(quarter.clone()));
        assert!(noted(&a, "2개를 원래 자리로 되돌려 놓았어요"), "{:?}", a.flash);
        // ⌥⌘V: 복사해 둔 것을 복사하지 않고 옮겨 달라고 한다
        assert!(a.key(Key::Char('v'), Mods { cmd: true, alt: true, ..Default::default() }));
        assert_eq!(a.clip_in.take(), Some(true));
        // 옮기는 곳에 같은 이름이 있으면 덮어쓰지 않고 번호를 붙인다
        std::fs::write(util::join(&downloads, "예산.csv"), b"mine").unwrap();
        a.activate_pane(1);
        assert!(a.paste_paths(vec![util::join(&work, "예산.csv")], Paste::Move));
        a.settle();
        assert_eq!(a.clip_left, None, "items that were only copied stay on the clipboard as they are");
        assert_eq!(std::fs::read(util::join(&downloads, "예산.csv")).unwrap(), b"mine");
        assert_eq!(std::fs::read(util::join(&downloads, "예산 2.csv")).unwrap(), b"x");
        assert!(noted(&a, "1개를 옮겼어요") && noted(&a, "1개에는 번호를 붙였어요"), "{:?}", a.flash);
        // 되돌릴 자리에 다른 것이 생겼으면 옮긴 자리에 그대로 둔다
        std::fs::write(util::join(&work, "예산.csv"), b"newcomer").unwrap();
        a.undo();
        a.settle();
        assert!(noted(&a, "옮긴 자리에 그대로 있어요"), "{:?}", a.flash);
        assert_eq!((std::fs::read(util::join(&work, "예산.csv")).unwrap(), std::fs::read(util::join(&downloads, "예산 2.csv")).unwrap()), (b"newcomer".to_vec(), b"x".to_vec()));
        // 잘라 둔 것 가운데 일부만 옮겨졌으면, 옮기지 못한 것만 잘라 둔 채로 남는다
        let (stay, go) = (util::join(&downloads, "사진.jpg"), util::join(&work, "회의록.md"));
        assert!(a.paste_paths(vec![stay.clone(), go], Paste::Cut(9)));
        a.settle();
        assert!(noted(&a, "1개를 옮겼고 1개는 하지 못했어요"), "{:?}", a.flash);
        assert_eq!(a.clip_left.take(), Some((9, vec![stay])));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 서로 다른 폴더에서 온 같은 이름의 폴더를 옮겼다가 되돌릴 때, 하나만 되돌려지면 그 하나의 안을 보던 칸만 따라간다
    #[test]
    fn undo_of_a_move_tells_same_named_items_apart() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (one, two) = (util::join(&work, "자료"), util::join(&downloads, "자료"));
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(util::join(&one, "하나.txt"), b"1").unwrap();
        std::fs::write(util::join(&two, "둘.txt"), b"2").unwrap();
        let (mut a, _g) = app(&home);
        // 두 폴더를 홈으로 옮긴다: 둘째에는 번호가 붙는다
        assert!(a.paste_paths(vec![one.clone(), two.clone()], Paste::Move));
        a.settle();
        let (moved_one, moved_two) = (util::join(&home, "자료"), util::join(&home, "자료 2"));
        assert!(noted(&a, "2개를 옮겼어요") && noted(&a, "1개에는 번호를 붙였어요"), "{:?}", a.flash);
        // 오른쪽 칸은 옮겨 온 둘째 폴더 안을 보고 있다
        a.navigate(1, Location::Dir(moved_two.clone()), None);
        a.settle();
        assert_eq!(names(&a, 1), ["둘.txt"]);
        // 첫째의 원래 자리에 다른 것이 생겼다: 되돌리면 첫째는 그대로 남고 둘째만 돌아간다
        std::fs::create_dir(&one).unwrap();
        a.undo();
        a.settle();
        assert!(noted(&a, "1개는 옮긴 자리에 그대로 있어요"), "{:?}", a.flash);
        assert_eq!(std::fs::read(util::join(&moved_one, "하나.txt")).unwrap(), b"1");
        assert_eq!(std::fs::read(util::join(&two, "둘.txt")).unwrap(), b"2");
        // 돌아간 둘째의 안을 보던 칸은 원래 경로로 따라간다 (이름이 같은 첫째 때문에 빠지지 않는다)
        assert_eq!(a.pane(1).tab().loc, Location::Dir(two.clone()));
        assert_eq!(names(&a, 1), ["둘.txt"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 끌어다 놓기의 놓을 곳: 폴더 줄이면 그 폴더 안, 그 밖의 목록 자리면 그 칸의 폴더, 사이드바의 폴더 줄이면 그 폴더.
    /// 놓아도 달라질 것이 없는 곳과 놓을 수 없는 곳은 강조하지 않는다
    #[test]
    fn drop_targets_are_folders_and_panes() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, g) = app(&work);
        a.navigate(1, Location::Dir(downloads.clone()), None);
        a.settle();
        a.toggle_pin(&downloads, true);
        let dragged = vec![util::join(&work, "예산.csv")];
        let at = |a: &App, p: usize, name: &str| {
            let vi = names(a, p).iter().position(|n| n == name).unwrap();
            row_point(a, p, vi)
        };
        // 같은 칸의 빈 곳이나 다른 파일 줄: 이미 그 폴더에 있어서 놓을 곳이 아니다
        let l0 = a.list_rect(0).unwrap();
        assert_eq!(a.drag_over(&g, l0.l + 60.0, l0.b - 10.0, &dragged), None);
        assert_eq!(a.drop, None);
        let (x, y) = at(&a, 0, "회의록.md");
        assert_eq!(a.drag_over(&g, x, y, &dragged), None);
        // 같은 칸의 폴더 줄: 그 폴더 안으로. 그 줄이 강조된다
        let (x, y) = at(&a, 0, "2026 4분기");
        assert_eq!(a.drag_over(&g, x, y, &dragged), Some(util::join(&work, "2026 4분기")));
        let vi = names(&a, 0).iter().position(|n| n == "2026 4분기").unwrap();
        assert_eq!(a.drop, Some(DropAt::Row(0, vi)));
        // 다른 칸: 빈 곳이든 파일 줄이든 그 칸이 보이는 폴더로. 그 칸이 강조된다
        let l1 = a.list_rect(1).unwrap();
        assert_eq!(a.drag_over(&g, l1.l + 60.0, l1.b - 10.0, &dragged), Some(downloads.clone()));
        assert_eq!(a.drop, Some(DropAt::Pane(1)));
        let (x, y) = at(&a, 1, "사진.jpg");
        assert_eq!(a.drag_over(&g, x, y, &dragged), Some(downloads.clone()));
        assert_eq!(a.drop, Some(DropAt::Pane(1)));
        // 사이드바의 고정한 폴더 줄
        let (x, y) = mid(side_row(&a, "다운로드"));
        assert_eq!(a.drag_over(&g, x, y, &dragged), Some(downloads.clone()));
        assert!(matches!(a.drop, Some(DropAt::Side(_))), "{:?}", a.drop);
        // 도구 막대 위는 놓을 곳이 아니고, 창을 벗어나면 강조가 지워진다
        let (x, y) = mid(a.toolbar().layout);
        assert_eq!(a.drag_over(&g, x, y, &dragged), None);
        assert_eq!(a.drop, None);
        // 탭 칩 위: 그 탭이 보이는 폴더로 (지금 탭이 아니어도). 그 칩이 강조된다. 그 폴더에 이미 있으면 놓을 곳이 아니다
        a.new_tab(1);
        a.navigate(1, Location::Dir(home.clone()), None);
        a.settle();
        let gm = geom(a.pane_rects()[1].1);
        let (tabs, _) = a.chips(&g, 1, &gm);
        let (x, y) = mid(tabs[0]);
        assert_eq!(a.drag_over(&g, x, y, &dragged), Some(downloads.clone()));
        assert_eq!(a.drop, Some(DropAt::Tab(1, 0)));
        let (x, y) = mid(tabs[1]);
        assert_eq!(a.drag_over(&g, x, y, &dragged), Some(home.clone()));
        assert_eq!(a.drop, Some(DropAt::Tab(1, 1)));
        let (x, y) = mid(a.chips(&g, 0, &geom(a.pane_rects()[0].1)).0[0]);
        assert_eq!(a.drag_over(&g, x, y, &dragged), None, "the left pane's tab already shows the file's folder");
        a.drag_over(&g, l1.l + 60.0, l1.b - 10.0, &dragged);
        a.drag_exit();
        assert_eq!(a.drop, None);
        // 끌고 있는 폴더 자신의 줄이나 그 폴더 안으로는 놓지 않는다
        let folder = vec![util::join(&work, "2026 4분기")];
        let (x, y) = at(&a, 0, "2026 4분기");
        assert_eq!(a.drag_over(&g, x, y, &folder), None);
        a.navigate(1, Location::Dir(folder[0].clone()), None);
        a.settle();
        let l1 = a.list_rect(1).unwrap();
        assert_eq!(a.drag_over(&g, l1.l + 60.0, l1.b - 10.0, &folder), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 놓기: 같은 디스크면 옮겨지고(같은 파일이 간다), ⌥를 누르고 끌면 복사된다. 폴더 줄에 놓으면 그 폴더 안으로 간다
    #[test]
    fn dropping_moves_on_the_same_disk_and_option_copies() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, g) = app(&work);
        a.navigate(1, Location::Dir(downloads.clone()), None);
        a.settle();
        let src = util::join(&work, "예산.csv");
        let file = sakadu::fileops::inode(&src).unwrap();
        let l1 = a.list_rect(1).unwrap();
        // 다른 칸의 빈 곳에 놓는다: 같은 디스크라 옮겨지고, 그 칸이 지금 칸이 된다
        a.drag_over(&g, l1.l + 60.0, l1.b - 10.0, std::slice::from_ref(&src));
        assert!(a.drop_paths(&g, l1.l + 60.0, l1.b - 10.0, vec![src.clone()], Paste::Drop));
        assert_eq!((a.drop, a.active()), (None, 1));
        a.settle();
        assert!(noted(&a, "1개를 옮겼어요"), "{:?}", a.flash);
        assert_eq!(sakadu::fileops::inode(&util::join(&downloads, "예산.csv")).unwrap(), file, "the same file, not a copy");
        assert_eq!(names(&a, 0), ["2026 4분기", "회의록.md"]);
        assert_eq!(picked(&a, 1).0, ["예산.csv"]);
        // 되돌리면 원래 자리로 간다
        a.undo();
        a.settle();
        assert_eq!(sakadu::fileops::inode(&src).unwrap(), file);
        assert_eq!(names(&a, 1), ["사진.jpg"]);
        // ⌥를 누르고 끌어 놓으면 복사한다: 원본이 남는다
        assert!(a.drop_paths(&g, l1.l + 60.0, l1.b - 10.0, vec![src.clone()], Paste::Copy));
        a.settle();
        assert!(noted(&a, "1개를 복사했어요"), "{:?}", a.flash);
        assert_eq!(sakadu::fileops::inode(&src).unwrap(), file);
        assert_ne!(sakadu::fileops::inode(&util::join(&downloads, "예산.csv")).unwrap(), file);
        // 폴더 줄에 놓으면 그 폴더 안으로 간다
        let vi = names(&a, 0).iter().position(|n| n == "2026 4분기").unwrap();
        let (x, y) = row_point(&a, 0, vi);
        assert!(a.drop_paths(&g, x, y, vec![util::join(&work, "회의록.md")], Paste::Drop));
        a.settle();
        assert_eq!(std::fs::read(util::join(&util::join(&work, "2026 4분기"), "회의록.md")).unwrap(), b"x");
        assert!(!names(&a, 0).contains(&"회의록.md".to_string()));
        // 놓을 수 없는 곳에 놓으면 아무 일도 없다
        let (x, y) = mid(a.toolbar().layout);
        assert!(!a.drop_paths(&g, x, y, vec![src.clone()], Paste::Drop));
        assert_eq!(a.copy_progress(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 한 번의 놓기가 옮긴 것과 복사한 것으로 갈렸어도(다른 디스크의 것이 섞였을 때) ⌘Z 한 번으로 모두 되돌린다
    #[test]
    fn one_drop_is_one_undo_even_when_it_moved_and_copied() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, _g) = app(&work);
        a.navigate(1, Location::Dir(downloads.clone()), None);
        a.settle();
        a.activate_pane(1);
        // 하나는 복사하고 하나는 옮긴 뒤, 그 두 기록을 한 번의 놓기가 남기는 것처럼 묶는다 (시험에는 디스크가 하나뿐이다)
        assert!(a.paste_paths(vec![util::join(&work, "예산.csv")], Paste::Copy));
        a.settle();
        assert!(a.paste_paths(vec![util::join(&work, "회의록.md")], Paste::Move));
        a.settle();
        let (moved, copied) = (a.undos.pop().unwrap(), a.undos.pop().unwrap());
        assert!(matches!(moved, Undo::Move { .. }) && matches!(copied, Undo::Copy { .. }));
        a.undos.push(Undo::Group(vec![copied, moved]));
        assert_eq!(names(&a, 1), ["사진.jpg", "예산.csv", "회의록.md"]);
        a.undo();
        a.settle();
        assert_eq!(names(&a, 1), ["사진.jpg"], "the copy went to the Trash and the moved file went back");
        assert_eq!(names(&a, 0), ["2026 4분기", "예산.csv", "회의록.md"]);
        assert!(noted(&a, "복사본 1개") && noted(&a, "원래 자리"), "{:?}", a.flash);
        a.undo();
        assert!(noted(&a, "되돌릴 작업이 없어요"), "{:?}", a.flash);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 끌어 내기: 고른 줄을 누른 채 조금 넘게 움직이면 고른 항목들을 내준다. 여럿을 고른 것 가운데 하나를 눌러도
    /// 고른 것이 그대로이고, 끌지 않고 떼면 그 줄만 남는다
    #[test]
    fn dragging_selected_rows_hands_out_their_paths() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        let at = |a: &App, name: &str| {
            let vi = names(a, 0).iter().position(|n| n == name).unwrap();
            row_point(a, 0, vi)
        };
        // 누른 채 조금만 움직인 것은 끌기가 아니다
        let (x, y) = at(&a, "예산.csv");
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.mouse_dragged(x + 2.0, y + 1.0);
        assert_eq!(a.take_drag(), None);
        // 더 움직이면 그 항목을 끌어 낸다 (한 번만 내준다)
        a.mouse_dragged(x + 12.0, y + 6.0);
        assert_eq!(a.take_drag(), Some(vec![util::join(&work, "예산.csv")]));
        a.mouse_dragged(x + 30.0, y + 6.0);
        assert_eq!(a.take_drag(), None);
        a.mouse_up();
        assert_eq!(picked(&a, 0).0, ["예산.csv"]);
        // 여럿을 고른 것 가운데 하나를 누른다: 고른 것이 그대로이고, 끌면 모두 끌려 나온다
        a.select_all();
        let (x, y) = at(&a, "회의록.md");
        a.mouse_down(&g, x, y, Mods::default(), 1);
        assert_eq!(a.pane(0).tab().selection_stats().0, 3);
        a.mouse_dragged(x + 12.0, y);
        assert_eq!(a.take_drag().map(|paths| paths.len()), Some(3));
        a.mouse_up();
        assert_eq!(a.pane(0).tab().selection_stats().0, 3, "a drag keeps the selection");
        // 끌지 않고 떼면 그 줄만 고른다
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.mouse_up();
        assert_eq!(picked(&a, 0).0, ["회의록.md"]);
        // 빈 곳을 누른 채 움직이는 것은 끌기가 아니다
        let l = a.list_rect(0).unwrap();
        a.mouse_down(&g, l.l + 60.0, l.b - 10.0, Mods::default(), 1);
        a.mouse_dragged(l.l + 90.0, l.b - 10.0);
        assert_eq!(a.take_drag(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 목록 메뉴의 복사·복제, 빈 곳 메뉴의 붙여넣기
    #[test]
    fn list_menu_copies_duplicates_and_pastes() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        let menu = |a: &mut App, name: &str| {
            let vi = names(a, 0).iter().position(|n| n == name).unwrap();
            let (x, y) = row_point(a, 0, vi);
            assert!(a.right_down(&g, x, y));
            assert_eq!(take_menu(a).map(|(k, _)| k), Some(MenuKind::Rows));
        };
        menu(&mut a, "예산.csv");
        a.pick_menu(MenuKind::Rows, 4);
        assert_eq!((a.clip_out.take(), a.clip_cut), (Some(vec![util::join(&work, "예산.csv")]), false));
        menu(&mut a, "예산.csv");
        a.pick_menu(MenuKind::Rows, 3);
        assert_eq!((a.clip_out.take(), a.clip_cut), (Some(vec![util::join(&work, "예산.csv")]), true), "cut marks the clipboard as a move");
        menu(&mut a, "예산.csv");
        a.pick_menu(MenuKind::Rows, 5);
        a.settle();
        assert_eq!(std::fs::read(util::join(&work, "예산 2.csv")).unwrap(), b"x");
        // 빈 곳의 붙여넣기: 그 칸을 지금 칸으로 하고 붙여넣어 달라고 한다
        let l = a.list_rect(1).unwrap();
        assert!(a.right_down(&g, l.l + 60.0, l.b - 10.0));
        assert_eq!(take_menu(&mut a).map(|(k, _)| k), Some(MenuKind::Blank));
        a.pick_menu(MenuKind::Blank, 1);
        assert!(a.clip_in == Some(false) && a.active() == 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 폴더 감시: 보이는 폴더가 바뀌었다고 하면 다시 읽고, 고른 것은 그대로 둔다. 보이지 않는 탭은 보일 때 읽는다
    #[test]
    fn a_changed_folder_is_read_again_with_the_selection_kept() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, g) = app(&work);
        // 왼쪽 칸: 처음 탭(업무)은 숨기고 새 탭에서 다운로드를 본다. 오른쪽 칸은 업무를 본다
        a.new_tab(0);
        a.navigate(0, Location::Dir(downloads.clone()), None);
        a.settle();
        click_name(&mut a, &g, 1, "회의록.md", Mods::default());
        std::fs::write(util::join(&work, "새 파일.txt"), b"x").unwrap();
        std::fs::write(util::join(&downloads, "받은 것.zip"), b"x").unwrap();
        a.on_dir_changed(&work);
        a.settle();
        assert_eq!(names(&a, 1), ["2026 4분기", "새 파일.txt", "예산.csv", "회의록.md"]);
        assert_eq!(picked(&a, 1), (vec!["회의록.md".to_string()], "회의록.md".to_string()), "selection and cursor stay");
        assert_eq!(names(&a, 0), ["사진.jpg"], "another folder is not read");
        // 숨어 있던 탭은 그대로 두었다가 보이게 될 때 읽는다
        assert!(!a.pane(0).tabs[0].entries.iter().any(|e| e.name_string() == "새 파일.txt"));
        a.activate_tab(0, 0);
        a.settle();
        assert!(names(&a, 0).contains(&"새 파일.txt".to_string()));
        // 아무 탭도 보이지 않는 폴더의 알림은 아무 일도 하지 않는다
        a.on_dir_changed(&downloads);
        a.on_dir_changed("/없는/폴더");
        assert!(a.take_requests().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 읽는 중에 그 폴더가 또 바뀌면 읽던 것을 버리지 않고, 다 읽은 뒤 한 번 더 읽는다
    /// (잇달아 바뀌는 폴더에서 읽던 것을 버리기만 되풀이하면 목록이 끝내 바뀌지 않는다)
    #[test]
    fn a_change_while_reading_is_read_once_more_afterwards() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, _g) = app(&work);
        a.set_layout(Layout::ALL[0]);
        std::fs::write(util::join(&work, "첫째.txt"), b"x").unwrap();
        a.on_dir_changed(&work);
        let reqs = a.take_requests();
        assert_eq!(reqs.len(), 1);
        // 읽는 사이에 또 바뀌었다: 새로 읽기를 맡기지 않는다
        let listed = listing::list_dir(&reqs[0].dir);
        std::fs::write(util::join(&work, "둘째.txt"), b"x").unwrap();
        a.on_dir_changed(&work);
        a.on_dir_changed(&work);
        assert!(a.take_requests().is_empty(), "the read in flight is not thrown away");
        // 읽던 것이 오면 그것을 보이고, 한 번 더 읽는다
        a.on_listed(reqs[0].tab, reqs[0].generation, listed);
        assert!(names(&a, 0).contains(&"첫째.txt".to_string()) && !names(&a, 0).contains(&"둘째.txt".to_string()));
        let again = a.take_requests();
        assert_eq!(again.len(), 1, "one more read for the change that came meanwhile");
        a.on_listed(again[0].tab, again[0].generation, listing::list_dir(&again[0].dir));
        assert!(names(&a, 0).contains(&"둘째.txt".to_string()));
        assert!(a.take_requests().is_empty(), "and then it rests");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 마우스로 줄을 누르고 있는 동안과 끌려 온 것이 놓일 곳 위에 있는 동안에는 다시 읽지 않는다 (손 밑의 줄이 바뀌지 않게).
    /// 떼거나 끌기가 끝나면 읽는다
    #[test]
    fn a_change_waits_while_a_row_is_pressed_or_a_drop_hovers() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, g) = app(&work);
        a.navigate(1, Location::Dir(downloads.clone()), None);
        a.settle();
        // 여럿을 고르고 그 가운데 하나를 누른 채로 있다
        a.select_all();
        let vi = names(&a, 0).iter().position(|n| n == "회의록.md").unwrap();
        let (x, y) = row_point(&a, 0, vi);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        std::fs::write(util::join(&work, "가.txt"), b"x").unwrap();
        a.on_dir_changed(&work);
        assert!(a.take_requests().is_empty(), "not while the mouse is down on a row");
        // 떼면: 누른 그 줄만 고르고(목록은 누를 때 그대로였다), 그다음에 읽는다
        a.mouse_up();
        assert_eq!(picked(&a, 0).0, ["회의록.md"]);
        a.settle();
        assert_eq!(names(&a, 0), ["2026 4분기", "가.txt", "예산.csv", "회의록.md"]);
        assert_eq!(picked(&a, 0).0, ["회의록.md"]);
        // 끌려 온 것이 왼쪽 칸의 폴더 줄 위에 있다
        let dragged = vec![util::join(&downloads, "사진.jpg")];
        let (x, y) = row_point(&a, 0, 0);
        assert_eq!(a.drag_over(&g, x, y, &dragged), Some(util::join(&work, "2026 4분기")));
        std::fs::write(util::join(&work, "0 맨 위.txt"), b"x").unwrap();
        a.on_dir_changed(&work);
        assert!(a.take_requests().is_empty(), "not while a drop hovers");
        // 창 밖으로 나가면 읽는다
        a.drag_exit();
        a.settle();
        assert!(names(&a, 0).contains(&"0 맨 위.txt".to_string()));
        // 다시 들어와 놓는다: 놓을 곳은 끄는 동안 보던 목록에서 찾고, 미뤄 둔 읽기는 그다음에 한다
        assert_eq!(a.drag_over(&g, x, y, &dragged), Some(util::join(&work, "2026 4분기")));
        std::fs::write(util::join(&work, "나.txt"), b"x").unwrap();
        a.on_dir_changed(&work);
        assert!(a.drop_paths(&g, x, y, dragged, Paste::Drop));
        a.settle();
        assert!(std::fs::metadata(util::join(&util::join(&work, "2026 4분기"), "사진.jpg")).is_ok(), "it went into the folder under the pointer");
        assert!(names(&a, 0).contains(&"나.txt".to_string()));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 다시 읽은 결과가 마우스로 줄을 누르고 있는 동안이나 끌려 온 것이 그 목록 위에 있는 동안에 오면, 그 목록은 손을 뗄 때까지
    /// 그대로 둔다: 뗀 줄은 누른 그 줄이고, 놓은 것은 강조돼 있던 그 폴더로 간다
    #[test]
    fn a_read_that_ends_under_the_pointer_is_applied_afterwards() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, g) = app(&work);
        let listed = |a: &mut App, reqs: &[Request]| {
            for r in reqs {
                a.on_listed(r.tab, r.generation, listing::list_dir(&r.dir));
            }
        };
        // 두 칸 모두 업무를 본다. 맨 위에 올 폴더가 생겨 줄이 하나씩 밀리게 된다
        a.select_all();
        std::fs::create_dir(util::join(&work, "0 맨 위")).unwrap();
        a.on_dir_changed(&work);
        let reqs = a.take_requests();
        assert_eq!(reqs.len(), 2);
        // 결과가 오기 전에 왼쪽 칸에서 회의록.md를 누른다 (여럿을 고른 것 가운데 하나)
        let vi = names(&a, 0).iter().position(|n| n == "회의록.md").unwrap();
        let (x, y) = row_point(&a, 0, vi);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        listed(&mut a, &reqs);
        assert!(!names(&a, 0).contains(&"0 맨 위".to_string()), "the list under the pressed row is left alone");
        assert_eq!(names(&a, 1)[0], "0 맨 위", "the other pane is not held back");
        a.mouse_up();
        assert_eq!(picked(&a, 0).0, ["회의록.md"], "the row that was pressed, not the one that slid under the pointer");
        assert_eq!(names(&a, 0)[0], "0 맨 위", "and then the list is brought up to date");
        assert!(a.take_requests().is_empty());
        // 끌려 온 것이 오른쪽 칸의 폴더 줄 '2026 4분기' 위에 있는 동안, 그 줄 자리에 올 폴더가 생긴다
        let dragged = vec![util::join(&downloads, "사진.jpg")];
        let (x, y) = row_point(&a, 1, 1);
        std::fs::create_dir(util::join(&work, "1 사이")).unwrap();
        a.on_dir_changed(&work);
        let reqs = a.take_requests();
        assert_eq!(reqs.len(), 2);
        assert_eq!(a.drag_over(&g, x, y, &dragged), Some(util::join(&work, "2026 4분기")));
        listed(&mut a, &reqs);
        assert_eq!(names(&a, 1)[1], "2026 4분기", "the list under the hovering drop is left alone");
        assert_eq!(names(&a, 0)[1], "1 사이");
        // 놓는다: 강조돼 있던 폴더로 가고, 그다음에 목록이 맞춰진다
        assert!(a.drop_paths(&g, x, y, dragged, Paste::Drop));
        a.settle();
        assert!(std::fs::metadata(util::join(&util::join(&work, "2026 4분기"), "사진.jpg")).is_ok(), "it went into the folder that was highlighted");
        assert!(std::fs::metadata(util::join(&util::join(&work, "1 사이"), "사진.jpg")).is_err());
        assert_eq!(names(&a, 1)[1], "1 사이");
        // 끌려 온 것이 다른 칸으로 옮겨 가면, 앞의 칸에서 미뤄 둔 것은 그때 적용한다
        let dragged = vec![util::join(&downloads, "없는 파일")];
        let (x, y) = row_point(&a, 0, 0);
        std::fs::create_dir(util::join(&work, "00 더 위")).unwrap();
        a.on_dir_changed(&work);
        let reqs = a.take_requests();
        assert!(a.drag_over(&g, x, y, &dragged).is_some());
        listed(&mut a, &reqs);
        assert!(!names(&a, 0).contains(&"00 더 위".to_string()) && names(&a, 1).contains(&"00 더 위".to_string()));
        let (x1, y1) = row_point(&a, 1, 0);
        assert!(a.drag_over(&g, x1, y1, &dragged).is_some());
        assert!(names(&a, 0).contains(&"00 더 위".to_string()));
        a.drag_exit();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 두 번 누르기는 처음 누른 그 항목을 연다: 두 번 사이에 목록이 바뀌어 손 밑에 다른 줄이 와 있어도 그것을 열지 않는다.
    /// 처음 누른 항목이 그 사이 없어졌으면 아무것도 열지 않는다
    #[test]
    fn a_double_click_opens_the_item_first_clicked() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        a.set_layout(Layout::ALL[0]);
        // 읽는 사이에 예산.csv를 누른다. 읽은 결과(맨 위에 폴더가 하나 생겼다)는 떼는 순간 적용되어 줄이 하나씩 밀린다
        std::fs::create_dir(util::join(&work, "0 맨 위")).unwrap();
        a.on_dir_changed(&work);
        let reqs = a.take_requests();
        let vi = names(&a, 0).iter().position(|n| n == "예산.csv").unwrap();
        let (x, y) = row_point(&a, 0, vi);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.on_listed(reqs[0].tab, reqs[0].generation, listing::list_dir(&reqs[0].dir));
        a.mouse_up();
        assert_eq!(names(&a, 0)[vi], "2026 4분기", "a folder is under the pointer now");
        // 둘째 누름: 손 밑에 온 폴더가 아니라 처음 누른 파일이 열린다
        a.mouse_down(&g, x, y, Mods::default(), 2);
        a.mouse_up();
        assert_eq!(a.open_files, [util::join(&work, "예산.csv")]);
        assert_eq!(a.pane(0).tab().loc, Location::Dir(work.clone()), "the folder that slid under the pointer is not opened");
        // 처음 누른 항목이 두 번 사이에 없어졌으면: 그 자리에 무엇이 와 있든 아무것도 열지 않는다
        a.clear_selection();
        let vi = names(&a, 0).iter().position(|n| n == "2026 4분기").unwrap();
        let (x, y) = row_point(&a, 0, vi);
        std::fs::rename(util::join(&work, "2026 4분기"), util::join(&home, "옮긴 폴더")).unwrap();
        a.on_dir_changed(&work);
        let reqs = a.take_requests();
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.on_listed(reqs[0].tab, reqs[0].generation, listing::list_dir(&reqs[0].dir));
        a.mouse_up();
        assert_eq!(names(&a, 0)[vi], "예산.csv");
        a.mouse_down(&g, x, y, Mods::default(), 2);
        a.mouse_up();
        assert_eq!(a.open_files.len(), 1, "what was clicked is gone, and what took its place is not opened");
        assert_eq!(a.pane(0).tab().loc, Location::Dir(work.clone()));
        // 목록이 그대로면 전과 같다: 폴더를 두 번 누르면 그 폴더로 들어간다
        let vi = names(&a, 0).iter().position(|n| n == "0 맨 위").unwrap();
        let (x, y) = row_point(&a, 0, vi);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.mouse_up();
        a.mouse_down(&g, x, y, Mods::default(), 2);
        a.mouse_up();
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(util::join(&work, "0 맨 위")));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 이름을 고치는 중에 목록이 저절로 바뀌어 그 줄이 화면 밖으로 밀리면, 그 줄이 보이게 스크롤한다:
    /// 이름 상자가 닫혀 치던 이름을 잃는 일이 없다
    #[test]
    fn a_name_being_edited_stays_in_view_when_the_list_changes() {
        let (root, home) = demo();
        let long = util::join(&home, "긴 폴더");
        for i in 0..100 {
            std::fs::create_dir_all(util::join(&long, &format!("sub_{i:03}"))).unwrap();
        }
        let (mut a, g) = app(&long);
        a.set_layout(Layout::ALL[0]);
        // 화면에 다 보이는 줄 가운데 아래쪽 줄(맨 아래에서 둘째)에서 이름을 고친다
        let last = (a.list_rect(0).unwrap().h() / ROW_H) as usize - 2;
        let name = format!("sub_{last:03}");
        click_name(&mut a, &g, 0, &name, Mods::default());
        a.key(Key::Return, Mods::default());
        assert!(matches!(&a.edit, Editing::Item(e) if e.name == name));
        assert_eq!(a.pane(0).tab().scroll, 0.0);
        // 그 줄보다 앞에 올 폴더가 여럿 생긴다: 그 줄은 다섯 줄 아래로 밀린다
        for i in 0..5 {
            std::fs::create_dir(util::join(&long, &format!("a_{i}"))).unwrap();
        }
        a.on_dir_changed(&long);
        a.settle();
        a.sync_edit();
        assert!(matches!(&a.edit, Editing::Item(e) if e.name == name), "the name is still being edited");
        assert!(a.pane(0).tab().scroll > 0.0, "the list scrolled to keep the row in view");
        let (list, rect) = (a.list_rect(0).unwrap(), a.address_rect().unwrap());
        assert!(rect.t >= list.t && rect.b <= list.b, "{rect:?} in {list:?}");
        // 고치던 항목이 없어지면 따라가지 않고 고치기가 끝난다
        let scroll = a.pane(0).tab().scroll;
        std::fs::rename(util::join(&long, &name), util::join(&home, "옮긴 것")).unwrap();
        a.on_dir_changed(&long);
        a.settle();
        a.sync_edit();
        assert_eq!(a.edit, Editing::None);
        assert_eq!(a.pane(0).tab().scroll, scroll);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 누르고 있던 줄을 끌어 내기 시작하면, 누르는 동안 미뤄 둔 읽기 결과를 그때 적용한다: 끌기가 창 밖에서 취소돼도
    /// 그 탭이 읽는 중으로 남아 다시는 저절로 읽히지 않는 일이 없다
    #[test]
    fn starting_a_drag_applies_what_was_held_while_pressing() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        a.set_layout(Layout::ALL[0]);
        std::fs::write(util::join(&work, "가.txt"), b"x").unwrap();
        a.on_dir_changed(&work);
        let reqs = a.take_requests();
        assert_eq!(reqs.len(), 1);
        // 읽는 사이에 예산.csv를 누르고, 누르고 있는 동안 결과가 온다
        let vi = names(&a, 0).iter().position(|n| n == "예산.csv").unwrap();
        let (x, y) = row_point(&a, 0, vi);
        a.mouse_down(&g, x, y, Mods::default(), 1);
        a.on_listed(reqs[0].tab, reqs[0].generation, listing::list_dir(&reqs[0].dir));
        assert!(!names(&a, 0).contains(&"가.txt".to_string()));
        // 끌기 시작: 끌려 나가는 것은 누른 그 파일이고, 미뤄 둔 결과는 이때 적용된다
        a.mouse_dragged(x + 12.0, y + 6.0);
        assert_eq!(a.take_drag(), Some(vec![util::join(&work, "예산.csv")]));
        assert!(names(&a, 0).contains(&"가.txt".to_string()));
        assert_eq!(picked(&a, 0).0, ["예산.csv"]);
        // 떼기도 놓기도 오지 않았어도(창 밖에서 취소) 그 뒤의 변화는 다시 읽힌다
        std::fs::write(util::join(&work, "나.txt"), b"x").unwrap();
        a.on_dir_changed(&work);
        assert_eq!(a.take_requests().len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 다시 읽는 사이에 고른 것을 바꾸면, 다 읽었을 때 그 바꾼 것이 남는다 (읽기를 시작한 때의 선택으로 되돌리지 않는다:
    /// 되돌리면 방금 고른 것과 다른 파일에 다음 작업이 간다)
    #[test]
    fn a_read_keeps_what_is_selected_when_it_ends() {
        let (root, home) = demo();
        let work = util::join(&home, "업무");
        let (mut a, g) = app(&work);
        a.set_layout(Layout::ALL[0]);
        click_name(&mut a, &g, 0, "회의록.md", Mods::default());
        std::fs::write(util::join(&work, "새 파일.txt"), b"x").unwrap();
        a.on_dir_changed(&work);
        let reqs = a.take_requests();
        assert_eq!(reqs.len(), 1);
        // 읽는 사이에 다른 파일을 고른다
        click_name(&mut a, &g, 0, "예산.csv", Mods::default());
        a.on_listed(reqs[0].tab, reqs[0].generation, listing::list_dir(&reqs[0].dir));
        assert!(names(&a, 0).contains(&"새 파일.txt".to_string()));
        assert_eq!(picked(&a, 0), (vec!["예산.csv".to_string()], "예산.csv".to_string()));
        // ⌘R로 다시 읽을 때도 같다
        a.refresh(0);
        let reqs = a.take_requests();
        a.select_all();
        a.on_listed(reqs[0].tab, reqs[0].generation, listing::list_dir(&reqs[0].dir));
        assert_eq!(a.pane(0).tab().selection_stats().0, 4);
        // 읽은 뒤 고를 것을 정해 둔 작업(새 폴더)은 그것을 따른다
        a.clear_selection();
        assert!(a.key(Key::Char('n'), Mods { cmd: true, shift: true, ..Default::default() }));
        a.settle();
        assert_eq!(picked(&a, 0).0, ["새 폴더"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 경로를 입력해 확인하는 사이에 지금 폴더가 다시 읽혀도(폴더 감시) 그 입력은 버려지지 않는다.
    /// 그 사이 다른 곳으로 옮겼으면 전과 같이 버린다
    #[test]
    fn a_read_does_not_drop_an_address_that_is_being_checked() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let quarter = util::join(&work, "2026 4분기");
        let (mut a, _g) = app(&work);
        a.begin_address(0);
        a.commit_address(&downloads);
        let probes = a.take_probes();
        assert_eq!(probes.len(), 1);
        // 확인이 끝나기 전에 지금 폴더가 바뀌어 다시 읽힌다 (두 번)
        std::fs::write(util::join(&work, "새 파일.txt"), b"x").unwrap();
        a.on_dir_changed(&work);
        a.settle();
        a.on_dir_changed(&work);
        a.settle();
        assert!(names(&a, 0).contains(&"새 파일.txt".to_string()));
        a.on_probed(&probes[0], resolve_input(&probes[0].text, &home));
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(downloads.clone()), "the typed folder is opened");
        // 확인하는 사이에 다른 곳으로 옮겼으면 그 입력은 버린다 (옮긴 뒤에 다시 읽혀도)
        a.begin_address(0);
        a.commit_address(&work);
        let probes = a.take_probes();
        a.navigate(0, Location::Dir(quarter.clone()), None);
        a.settle();
        a.on_dir_changed(&quarter);
        a.settle();
        a.on_probed(&probes[0], resolve_input(&probes[0].text, &home));
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(quarter.clone()), "the address typed before moving on is dropped");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 복사 묶음이 넣고 있는 폴더는 묶음이 끝날 때까지 다시 읽지 않는다 (복사하는 중인 임시 이름이 목록에 보이지 않게)
    #[test]
    fn a_folder_being_pasted_into_is_read_when_the_batch_ends() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, _g) = app(&work);
        a.navigate(1, Location::Dir(downloads.clone()), None);
        a.settle();
        a.activate_pane(1);
        assert!(a.paste_paths(vec![util::join(&work, "예산.csv"), util::join(&work, "회의록.md")], Paste::Copy));
        assert_eq!(a.copy_progress(), Some((1, 2)));
        a.on_dir_changed(&downloads);
        assert!(a.take_requests().is_empty(), "not in the middle of the batch");
        // 원본 쪽 폴더는 묶음과 상관없이 읽는다
        a.on_dir_changed(&work);
        assert_eq!(a.take_requests().len(), 1);
        a.settle();
        assert_eq!(a.copy_progress(), None);
        assert_eq!(names(&a, 1), ["사진.jpg", "예산.csv", "회의록.md"]);
        assert!(a.take_requests().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 감시할 폴더: 보이는 칸의 폴더를 한 번씩. 감시에서 빠지는 폴더를 보이던 탭은 다음에 보일 때 다시 읽는다
    #[test]
    fn watch_list_follows_the_visible_folders() {
        let (root, home) = demo();
        let (work, downloads) = (util::join(&home, "업무"), util::join(&home, "다운로드"));
        let (mut a, _g) = app(&work);
        // 두 칸이 같은 폴더를 본다: 한 번만
        assert_eq!(a.take_watch_dirs(), Some(vec![work.clone()]));
        assert_eq!(a.take_watch_dirs(), None, "nothing changed");
        // 읽기 시작한 폴더는 다 읽기 전부터 감시한다
        a.navigate(1, Location::Dir(downloads.clone()), None);
        assert_eq!(a.take_watch_dirs(), Some(vec![work.clone(), downloads.clone()]));
        a.settle();
        assert_eq!(a.take_watch_dirs(), None);
        // 왼쪽 칸에서 새 탭으로 다른 폴더에 간다: 업무는 이제 아무 칸에도 보이지 않아 감시에서 빠진다
        a.new_tab(0);
        a.navigate(0, Location::Dir(util::join(&work, "2026 4분기")), None);
        a.settle();
        assert_eq!(a.take_watch_dirs(), Some(vec![util::join(&work, "2026 4분기"), downloads.clone()]));
        // 그동안 업무에 생긴 것은 알림이 오지 않는다. 그 탭이 다시 보일 때 읽는다
        std::fs::write(util::join(&work, "몰래.txt"), b"x").unwrap();
        a.activate_tab(0, 0);
        assert_eq!(a.take_watch_dirs(), Some(vec![work.clone(), downloads.clone()]));
        a.settle();
        assert!(names(&a, 0).contains(&"몰래.txt".to_string()));
        // 칸을 하나로 줄이면 가려진 칸의 폴더는 빠지고, 다시 넓히면 그 칸을 다시 읽는다
        a.set_layout(Layout::ALL[0]);
        assert_eq!(a.take_watch_dirs(), Some(vec![work.clone()]));
        std::fs::write(util::join(&downloads, "나중.zip"), b"x").unwrap();
        a.set_layout(Layout::ALL[1]);
        a.settle();
        assert!(names(&a, 1).contains(&"나중.zip".to_string()));
        assert_eq!(a.take_watch_dirs(), Some(vec![work.clone(), downloads.clone()]));
        // 열지 못한 폴더는 감시하지 않는다
        std::fs::rename(&downloads, util::join(&home, "옮김")).unwrap();
        a.refresh(1);
        a.settle();
        assert!(matches!(a.pane(1).tab().state, LoadState::Error(_)));
        assert_eq!(a.take_watch_dirs(), Some(vec![work.clone()]));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 훑어보기(Quick Look): 스페이스와 ⌘Y가 창을 열고 닫아 달라고 하고, 보여 줄 것은 고른 항목들이다.
    /// 미리 보기 카드가 떠 있으면 스페이스는 지금처럼 카드를 크게 하고, ⌘Y는 카드와 상관없이 훑어보기다
    #[test]
    fn space_and_cmd_y_ask_for_quick_look() {
        let (root, dir) = preview_demo();
        let (mut a, g) = app(&dir);
        let (none, cmd) = (Mods::default(), Mods { cmd: true, ..Default::default() });
        // 카드가 뜨지 않는 파일(압축 파일): 스페이스가 훑어보기를 열어 달라고 한다. 보여 줄 것은 그 파일
        click_name(&mut a, &g, 0, "보관함.zip", none);
        a.settle();
        assert!(a.card().is_none());
        assert!(a.key(Key::Char(' '), none));
        assert_eq!(a.look_request.take(), Some(true));
        assert_eq!(a.look_items(), (vec![util::join(&dir, "보관함.zip")], 0));
        // 창이 떠 있으면(창 쪽이 알려 준다) 스페이스도 ⌘Y도 닫아 달라고 한다
        a.look_open = true;
        assert!(a.key(Key::Char(' '), none));
        assert_eq!(a.look_request.take(), Some(false));
        assert!(a.key(Key::Char('y'), cmd));
        assert_eq!(a.look_request.take(), Some(false));
        // 카드가 뜬 그림: 창이 떠 있으면 스페이스는 카드보다 창을 먼저 닫는다
        click_name(&mut a, &g, 0, "노을.bmp", none);
        a.settle();
        assert!(a.card().is_some());
        assert!(a.key(Key::Char(' '), none));
        assert_eq!(a.look_request.take(), Some(false));
        assert!(!a.card.large);
        // 창이 닫혀 있으면: 스페이스는 카드를 크게 하고, ⌘Y는 훑어보기를 연다
        a.look_open = false;
        assert!(a.key(Key::Char(' '), none));
        assert!(a.card.large && a.look_request.is_none());
        assert!(a.key(Key::Char('y'), cmd));
        assert_eq!(a.look_request.take(), Some(true));
        a.key(Key::Char(' '), none);
        assert!(!a.card.large);
        // 여럿을 고르면 모두 보여 주고, 커서가 있는 것부터 보여 준다
        click_name(&mut a, &g, 0, "메모.txt", cmd);
        assert_eq!(a.look_items(), (vec![util::join(&dir, "노을.bmp"), util::join(&dir, "메모.txt")], 1));
        // 아무것도 고르지 않았으면 커서 항목을 보여 준다
        a.clear_selection();
        assert_eq!(a.look_items(), (vec![util::join(&dir, "메모.txt")], 0));
        // 빈 폴더: 보여 줄 것이 없으면 아무 일도 하지 않는다
        a.navigate(0, Location::Dir(util::join(&dir, "폴더")), None);
        a.settle();
        assert!(!a.key(Key::Char(' '), none) && !a.key(Key::Char('y'), cmd));
        assert_eq!(a.look_request, None);
        // 스페이스와 ⌘Y는 누르고 있어도 한 번만 받는다 (열렸다 닫혔다 하지 않게)
        assert!(!Key::Char(' ').repeats() && !Key::Char('y').repeats() && Key::Down.repeats());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 미리 보기 시험용 폴더: 하위 폴더, 읽히는 그림 둘, 그림이 아닌 .jpg, 글 파일, 글이 아닌 .txt, 카드에 띄우지 않는 파일(압축 파일)
    fn preview_demo() -> (std::path::PathBuf, String) {
        let (root, home) = demo();
        let dir = root.join("미리 보기");
        std::fs::create_dir_all(dir.join("폴더")).unwrap();
        for name in ["노을.bmp", "바다.bmp"] {
            std::fs::write(dir.join(name), crate::render::demo_picture()).unwrap();
        }
        std::fs::write(dir.join("깨진 그림.jpg"), b"x").unwrap();
        std::fs::write(dir.join("메모.txt"), "첫 줄\n\t둘째 줄\n").unwrap();
        std::fs::write(dir.join("글 아님.txt"), [b'a', 0, 0xFF, b'b']).unwrap();
        std::fs::write(dir.join("보관함.zip"), b"x").unwrap();
        (root, util::join(&home, "미리 보기"))
    }

    /// 칸 p에서 그 이름의 줄을 누른다 (자세히 보기): 눌렀다 뗀다.
    /// 여럿을 고른 것 가운데 하나를 누르면 뗄 때 그 줄만 남으므로, 떼기까지 해야 누른 것이다
    fn click_name(a: &mut App, g: &Gfx, p: usize, name: &str, m: Mods) {
        let vi = names(a, p).iter().position(|n| n == name).unwrap_or_else(|| panic!("no row named {name}"));
        let (x, y) = row_point(a, p, vi);
        a.mouse_down(g, x, y, m, 1);
        a.mouse_up();
    }

    fn pane_rect(a: &App, p: usize) -> R {
        a.pane_rects().into_iter().find(|(i, _)| *i == p).unwrap().1
    }

    /// 그림이나 글 파일을 하나 고르면 카드가 뜬다. 폴더, 그 밖의 파일, 여러 개를 고른 때에는 뜨지 않는다
    #[test]
    fn card_shows_for_one_selected_image_or_text_file() {
        let (root, dir) = preview_demo();
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        assert!(a.card().is_none(), "nothing is selected");
        // 그림: 카드는 바로 뜨고 그림은 읽은 뒤에 들어온다
        click_name(&mut a, &g, 0, "노을.bmp", none);
        assert!(a.card().is_some() && a.pics.is_empty());
        a.settle();
        assert!(a.card().is_some() && !a.card.failed);
        assert_eq!(a.pics.len(), 1, "the picture was read");
        assert_eq!(a.pics[0].pic.size(), (480, 320), "a picture smaller than the limit is read as it is");
        let (target, _) = a.card_shown().unwrap();
        assert_eq!(a.card_note(&target), None, "no note over a picture that was read");
        // 글 파일: 다 읽은 뒤에 뜬다
        click_name(&mut a, &g, 0, "메모.txt", none);
        assert!(a.card().is_none(), "the text card waits for the text");
        a.settle();
        assert!(a.card().is_some());
        assert_eq!(a.card.text.as_deref(), Some("첫 줄\n    둘째 줄\n"));
        // 글로 읽히지 않는 파일은 카드를 띄우지 않는다
        click_name(&mut a, &g, 0, "글 아님.txt", none);
        a.settle();
        assert!(a.card().is_none() && a.card.failed);
        // 읽히지 않는 그림: 카드는 뜨고, 미리 볼 수 없다고 알린다
        click_name(&mut a, &g, 0, "깨진 그림.jpg", none);
        a.settle();
        let (target, _) = a.card_shown().expect("the card of an unreadable picture still shows");
        assert_eq!(a.card_note(&target), Some("미리 볼 수 없어요"));
        // 폴더와 카드에 띄우지 않는 파일 (Quick Look에 묻지도 않는다)
        for name in ["폴더", "보관함.zip"] {
            click_name(&mut a, &g, 0, name, none);
            assert!(a.take_previews().is_empty(), "{name}");
            a.settle();
            assert!(a.card().is_none(), "{name}");
        }
        // 여러 개를 고르면 뜨지 않고, 하나만 고르면 다시 뜬다
        click_name(&mut a, &g, 0, "노을.bmp", none);
        click_name(&mut a, &g, 0, "바다.bmp", Mods { cmd: true, ..Default::default() });
        a.settle();
        assert!(a.card().is_none());
        click_name(&mut a, &g, 0, "바다.bmp", none);
        a.settle();
        assert!(a.card().is_some());
        // 이름으로 가리는 종류와 이름 아래 글: 글, 그림, 그 밖에 Quick Look이 썸네일을 만들 수 있는 문서(한글 문서, SVG 포함).
        // 압축 파일·디스크 이미지·바로가기 같은 것과 확장자 없는 파일은 카드가 없다
        assert_eq!((card_kind("사진.JPG"), card_kind("메모.md"), card_kind(".gitignore")), (Some(CardKind::Image), Some(CardKind::Text), Some(CardKind::Text)));
        for name in ["보고서.docx", "발표.key", "영상.MOV", "그림.svg", "회의.hwp", "책.pdf", "노래.mp3"] {
            assert_eq!(card_kind(name), Some(CardKind::Doc), "{name}");
        }
        for name in ["보관함.zip", "설치.dmg", "바로가기.webloc", ".DS_Store", "txt", "이름.", "Makefile"] {
            assert_eq!(card_kind(name), None, "{name}");
        }
        assert_eq!(card_detail("PNG 이미지", 248 * 1024), "PNG 이미지 · 248 KB");
        assert_eq!(card_detail("", 512), "512 B");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 카드는 지금 칸이 아닌 칸의 오른쪽 아래에 놓이고, 지금 칸이 바뀌면 자리를 옮긴다.
    /// 한 칸뿐이면 그 칸에 놓이고, 좁은 창에서는 뜨지 않는다
    #[test]
    fn card_sits_in_another_pane_and_follows_the_active_pane() {
        use sakadu::ui::preview_card::{CARD_MARGIN, CARD_W};
        let (root, dir) = preview_demo();
        let (mut a, g) = app(&dir);
        click_name(&mut a, &g, 0, "노을.bmp", Mods::default());
        a.settle();
        let (m, host) = (a.card().unwrap(), pane_rect(&a, 1));
        assert_eq!(m.host, 1);
        assert_eq!((m.card.r, m.card.b, m.card.w()), (host.r - CARD_MARGIN, host.b - CARD_MARGIN, CARD_W));
        // 오른쪽 칸에서 고르면 그 칸이 지금 칸이 되고, 카드는 왼쪽 칸으로 간다
        click_name(&mut a, &g, 1, "바다.bmp", Mods::default());
        a.settle();
        assert_eq!((a.active(), a.card().unwrap().host), (1, 0));
        // Tab으로 지금 칸을 바꾸면 그 칸에서 고른 파일의 카드가 다른 칸에 뜬다
        a.key(Key::Tab, Mods::default());
        a.settle();
        assert_eq!((a.active(), a.card().unwrap().host), (0, 1));
        assert_eq!(a.card_shown().unwrap().0.key.path, util::join(&dir, "노을.bmp"));
        // 고른 것이 없는 칸으로 가면 카드가 없다
        a.set_layout(Layout::Three);
        a.settle();
        a.activate_pane(2);
        a.settle();
        assert!(a.card().is_none());
        // 한 칸: 그 칸의 오른쪽 아래에 놓이고 왼쪽에 이름이 보일 자리가 남는다. 좁은 창에서는 뜨지 않는다
        a.activate_pane(0);
        a.set_layout(Layout::One);
        a.settle();
        let (m, only) = (a.card().unwrap(), pane_rect(&a, 0));
        assert_eq!((m.host, m.card.r), (0, only.r - CARD_MARGIN));
        assert!(m.card.l - only.l >= 200.0);
        a.resize(700.0, 420.0);
        assert!(a.card().is_none(), "too narrow to keep the names visible");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 한 칸: 파일을 두 번 누르면, 첫 누름으로 뜬 카드가 그 자리를 덮어도 파일이 열린다 (카드가 둘째 누름을 가로채지 않는다)
    #[test]
    fn double_click_opens_the_file_even_when_its_card_covers_the_spot() {
        let (root, dir) = preview_demo();
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        a.set_layout(Layout::One);
        // 낮은 창: 카드(칸의 오른쪽 아래)가 목록의 위쪽 줄들을 덮는다
        a.resize(1100.0, 360.0);
        a.settle();
        let vi = names(&a, 0).iter().position(|n| n == "노을.bmp").unwrap();
        let (_, y) = row_point(&a, 0, vi);
        let x = pane_rect(&a, 0).r - 100.0;
        assert_eq!(a.hit(&g, x, y), Hit::Row(0, vi));
        a.mouse_down(&g, x, y, none, 1);
        a.mouse_up();
        a.settle();
        let m = a.card().expect("the card shows in the only pane");
        assert!(m.card.contains(x, y), "the card covers the clicked spot: {:?}", m.card);
        assert_eq!(a.hit(&g, x, y), Hit::Card);
        // 둘째 누름(두 번 누르기): 카드가 아니라 그 아래 파일이 받아 파일이 열린다
        a.mouse_down(&g, x, y, none, 2);
        a.mouse_up();
        a.settle();
        assert!(a.open_files.iter().any(|p| p.ends_with("노을.bmp")), "{:?}", a.open_files);
        assert!(!a.card.large, "the card was not enlarged");
        // 카드를 두 번 누르면(첫 누름이 카드 위) 첫 누름만 듣는다: 카드가 커지고, 둘째 누름은 버린다
        let opened = a.open_files.len();
        a.mouse_down(&g, x, y, none, 1);
        assert!(a.card.large);
        a.mouse_down(&g, x, y, none, 2);
        assert!(a.card.large, "the second click of a double-click on the card is dropped");
        // 큰 카드의 왼쪽(작은 카드 밖)을 두 번 누른다: 첫 누름으로 카드가 작아져 그 자리에 다른 파일의 줄이 드러나도,
        // 둘째 누름은 그 줄을 고르거나 열지 않는다
        let big = a.card().unwrap().card;
        let other = (1..names(&a, 0).len()).find(|i| *i != vi).unwrap();
        let (_, y2) = row_point(&a, 0, other);
        let x2 = big.l + 20.0;
        assert!(big.contains(x2, y2));
        a.mouse_down(&g, x2, y2, none, 1);
        assert!(!a.card.large);
        assert_eq!(a.hit(&g, x2, y2), Hit::Row(0, other), "the small card no longer covers the spot");
        a.mouse_down(&g, x2, y2, none, 2);
        a.settle();
        assert_eq!(a.open_files.len(), opened, "clicks that began on the card open nothing");
        assert_eq!(a.pane(0).tab().cursor, vi, "and select nothing");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 카드의 단추: 크게 보기는 단추·카드·스페이스로 켜고 끄며, 파일을 옮겨 다녀도 이어지다가 카드가 사라지면 끝난다.
    /// 닫으면 다른 파일을 고를 때까지 뜨지 않는다. 카드를 누른 것은 아래 칸으로 가지 않는다
    #[test]
    fn card_buttons_enlarge_and_close() {
        use sakadu::ui::preview_card::CARD_MARGIN;
        let (root, dir) = preview_demo();
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        click_name(&mut a, &g, 0, "노을.bmp", none);
        a.settle();
        let small = a.card().unwrap();
        // 크게 보기 단추: 카드가 놓인 칸을 여백만 남기고 채운다
        let (x, y) = mid(small.expand);
        assert_eq!(a.hit(&g, x, y), Hit::CardExpand);
        a.mouse_down(&g, x, y, none, 1);
        a.settle();
        let large = a.card().unwrap();
        assert_eq!(large.card, pane_rect(&a, 1).inset(CARD_MARGIN, CARD_MARGIN));
        assert_eq!(a.active(), 0, "a click on the card does not reach the pane under it");
        // 카드의 그림 자리를 눌러도, 스페이스를 눌러도 바뀐다
        let (x, y) = mid(large.body);
        assert_eq!(a.hit(&g, x, y), Hit::Card);
        a.mouse_down(&g, x, y, none, 1);
        assert_eq!((a.card().unwrap().card, a.active()), (small.card, 0));
        assert!(a.key(Key::Char(' '), none));
        assert_eq!(a.card().unwrap().card, large.card);
        // 크게 본 채로 다른 그림·글 파일로 옮겨도 그대로 크다
        click_name(&mut a, &g, 0, "바다.bmp", none);
        a.settle();
        assert_eq!(a.card().unwrap().card, large.card);
        click_name(&mut a, &g, 0, "메모.txt", none);
        a.settle();
        assert_eq!(a.card().unwrap().card, large.card);
        // 카드가 사라지면(폴더를 골랐다) 크게 보기도 끝난다. 카드가 없으면 스페이스는 훑어보기를 열어 달라고 한다
        click_name(&mut a, &g, 0, "폴더", none);
        a.settle();
        assert!(a.card().is_none() && a.key(Key::Char(' '), none));
        assert_eq!(a.look_request.take(), Some(true));
        click_name(&mut a, &g, 0, "노을.bmp", none);
        a.settle();
        assert_eq!(a.card().unwrap().card, small.card);
        // 닫기: 카드가 사라지고 크게 보기도 끝난다. 같은 파일을 다시 눌러도 뜨지 않는다
        a.key(Key::Char(' '), none);
        let (x, y) = mid(a.card().unwrap().close);
        assert_eq!(a.hit(&g, x, y), Hit::CardClose);
        a.mouse_down(&g, x, y, none, 1);
        a.settle();
        assert!(a.card().is_none());
        assert_eq!(a.hit(&g, x, y), Hit::List(1), "nothing floats over the pane any more");
        click_name(&mut a, &g, 0, "노을.bmp", none);
        a.settle();
        assert!(a.card().is_none(), "the same file stays closed");
        // 다른 파일을 고르면 그 파일의 카드가 작게 뜨고, 닫았던 파일로 돌아와도 다시 뜬다
        click_name(&mut a, &g, 0, "바다.bmp", none);
        a.settle();
        assert_eq!(a.card().unwrap().card, small.card);
        click_name(&mut a, &g, 0, "노을.bmp", none);
        a.settle();
        assert!(a.card().is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 도구 막대의 눈: 자동 미리 보기를 끄면 카드가 뜨지 않는다. 세션에 남고, 작업 공간을 바꿔도 그대로다
    #[test]
    fn eye_button_turns_the_card_off_and_the_setting_is_kept() {
        let (root, dir) = preview_demo();
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        click_name(&mut a, &g, 0, "노을.bmp", none);
        a.settle();
        a.key(Key::Char(' '), none);
        assert!(a.card().is_some() && a.card.large);
        // 눈을 누르면 꺼진다: 카드가 사라지고 크게 보기도 끝난다. 고른 것은 그대로다
        let (x, y) = mid(a.toolbar().preview);
        assert_eq!(a.hit(&g, x, y), Hit::PreviewToggle);
        assert!(!a.drag_zone(&g, x, y), "the button does not drag the window");
        a.mouse_down(&g, x, y, none, 1);
        a.settle();
        assert!(!a.auto_preview && a.card().is_none() && !a.card.large);
        assert!(a.flash.as_ref().is_some_and(|(_, m)| m.contains("껐어요")));
        assert_eq!(a.pane(0).tab().selection_stats().0, 1);
        click_name(&mut a, &g, 0, "바다.bmp", none);
        assert!(a.card().is_none() && a.take_previews().is_empty(), "nothing is read while it is off");
        // 세션(파일 형식까지)에 남아 다음에도 꺼진 채로 열린다
        let s = session::parse(&session::serialize(&a.to_session())).expect("session");
        assert!(!s.auto_preview);
        let (mut b, _g) = app(&dir);
        assert!(b.auto_preview, "on unless it was turned off");
        b.restore(&s, &dir);
        assert!(!b.auto_preview);
        // 작업 공간을 바꿔도 그대로다: 그 공간에 저장된 화면의 값이 아니라 지금 값을 따른다
        a.new_workspace();
        a.cancel_address();
        a.mouse_down(&g, x, y, none, 1);
        assert!(a.auto_preview);
        a.switch_workspace(0);
        a.settle();
        assert!(a.auto_preview, "workspace 0 was left with the preview off, but the setting is not per workspace");
        a.mouse_down(&g, x, y, none, 1);
        a.switch_workspace(1);
        a.settle();
        assert!(!a.auto_preview);
        // 다시 켜면 고른 파일의 카드가 뜬다
        a.mouse_down(&g, x, y, none, 1);
        assert!(a.flash.as_ref().is_some_and(|(_, m)| m.contains("켰어요")));
        click_name(&mut a, &g, 0, "노을.bmp", none);
        a.settle();
        assert!(a.auto_preview && a.card().is_some());
        // 지난번 화면이 다른 공간의 것이어도 자동 미리 보기는 지난번 값으로 연다
        let mut last = a.to_session();
        last.ws = Some("없는 공간".to_string());
        last.auto_preview = false;
        let (mut c, _g) = app(&dir);
        c.open_saved(a.workspaces().clone(), Some(&last), &dir);
        assert!(!c.auto_preview);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 읽기 요청과 결과: 그 사이 다른 파일을 골랐으면 늦게 온 결과는 버린다. 크게 보면 더 큰 그림을 한 번 더 읽고,
    /// 읽어 둔 그림은 몇 개만 남긴다 (읽은 결과를 직접 넣어 본다)
    #[test]
    fn card_requests_follow_the_selected_file() {
        let (root, dir) = preview_demo();
        for i in 0..8 {
            std::fs::write(root.join("미리 보기").join(format!("사진 {i}.png")), b"x").unwrap();
        }
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        let blank = |w, h| Some(Preview::Image(Picture::blank(w, h).unwrap()));
        click_name(&mut a, &g, 0, "사진 0.png", none);
        let first = a.take_previews();
        assert_eq!((first.len(), first[0].px), (1, PX_SMALL));
        assert!(a.take_previews().is_empty(), "asked once");
        // 그 사이 다른 파일을 골랐다: 늦게 온 결과는 버린다
        click_name(&mut a, &g, 0, "사진 1.png", none);
        let second = a.take_previews();
        assert!(second[0].seq > first[0].seq);
        a.on_previewed(&first[0], blank(1024, 600));
        assert!(a.pics.is_empty());
        a.on_previewed(&second[0], blank(1024, 600));
        assert_eq!(a.pics.len(), 1);
        assert!(a.take_previews().is_empty());
        // 크게 보기: 줄여 읽은 그림이면 더 크게 한 번 더 읽는다. 그것이 실패해도 읽어 둔 그림은 남는다
        a.key(Key::Char(' '), none);
        let larger = a.take_previews();
        assert_eq!((larger.len(), larger[0].px), (1, PX_LARGE));
        a.on_previewed(&larger[0], None);
        assert!(!a.card.failed && a.pics.len() == 1);
        a.on_previewed(&larger[0], blank(2048, 1200));
        assert_eq!((a.pics.len(), a.pics[0].px), (1, PX_LARGE));
        // 작게 읽은 것이 늦게 와도 큰 그림을 덮지 않는다
        a.on_previewed(&second[0], blank(1024, 600));
        assert_eq!(a.pics[0].pic.longest(), 2048);
        a.key(Key::Char(' '), none);
        // 원본이 작아 줄이지 않고 읽은 그림은 크게 봐도 다시 읽지 않는다
        click_name(&mut a, &g, 0, "사진 2.png", none);
        let small = a.take_previews();
        a.on_previewed(&small[0], blank(300, 200));
        a.key(Key::Char(' '), none);
        assert!(a.card.large && a.take_previews().is_empty());
        a.key(Key::Char(' '), none);
        // 읽지 못한 그림은 다시 요청하지 않는다
        click_name(&mut a, &g, 0, "사진 3.png", none);
        let bad = a.take_previews();
        a.on_previewed(&bad[0], None);
        assert!(a.card.failed && a.take_previews().is_empty());
        // 읽어 둔 그림은 PICS_MAX개까지만 남는다
        for i in 0..8 {
            click_name(&mut a, &g, 0, &format!("사진 {i}.png"), none);
            for r in a.take_previews() {
                a.on_previewed(&r, blank(64, 64));
            }
        }
        assert_eq!(a.pics.len(), PICS_MAX);
        assert_eq!(a.pics.last().unwrap().key.path, util::join(&dir, "사진 7.png"));
        // 읽어 둔 그림으로 돌아가면 다시 읽지 않고, 방금 본 그림이 가장 늦게까지 남는다
        click_name(&mut a, &g, 0, "사진 6.png", none);
        assert!(a.take_previews().is_empty());
        assert_eq!(a.pics.last().unwrap().key.path, util::join(&dir, "사진 6.png"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 썸네일 시험용 폴더 '사진첩': 그림 파일 n개(내용은 그림이 아니다. 시험이 읽은 결과를 직접 넣는다),
    /// 이름이 그림 같은 하위 폴더, 그림이 아닌 파일 둘
    fn thumbs_demo(n: usize) -> (std::path::PathBuf, String) {
        let (root, home) = demo();
        let dir = root.join("사진첩");
        std::fs::create_dir_all(dir.join("폴더.png")).unwrap();
        for i in 0..n {
            std::fs::write(dir.join(format!("사진 {i:03}.jpg")), b"x").unwrap();
        }
        std::fs::write(dir.join("메모.txt"), "글").unwrap();
        std::fs::write(dir.join("보고서.docx"), b"x").unwrap();
        (root, util::join(&home, "사진첩"))
    }

    /// 쌓인 폴더 읽기만 바로 처리한다 (settle과 달리 썸네일은 읽지 않는다)
    fn list_now(a: &mut App) {
        for r in a.take_requests() {
            let result = listing::list_dir(&r.dir);
            a.on_listed(r.tab, r.generation, result);
        }
    }

    /// 아이콘 보기의 썸네일: 보이는 파일만, 한 번에 THUMB_JOBS개까지만 읽는다. 폴더는 요청하지 않는다.
    /// 그림 파일은 직접 읽고, 그 밖의 파일(글, 문서)은 Quick Look에 맡긴다
    #[test]
    fn thumbnails_are_read_a_few_at_a_time_for_visible_files_only() {
        let (root, dir) = thumbs_demo(7);
        let (mut a, _g) = app(&dir);
        assert!(a.take_thumbs().is_empty(), "the details view shows no thumbnails");
        a.set_view(0, ViewMode::Large);
        // 보이는 파일은 아홉(그림 일곱, 글 파일, 문서)인데 둘만 읽기 시작하고, 나머지는 기다린다
        let mut running = a.take_thumbs();
        assert_eq!((running.len(), a.thumbs.queue.len()), (THUMB_JOBS, 9 - THUMB_JOBS));
        assert!(a.take_thumbs().is_empty(), "nothing more starts while two are being read");
        // 하나를 다 읽을 때마다 다음 하나가 시작된다. 읽는 중인 것은 한도를 넘지 않는다
        let mut asked: Vec<String> = Vec::new();
        while let Some(r) = running.pop() {
            asked.push(r.key.file.path.clone());
            a.on_thumb(&r, Picture::blank(8, 8));
            running.extend(a.take_thumbs());
            assert!(running.len() <= THUMB_JOBS, "{} at once", running.len());
            assert_eq!(a.thumbs.loading.len(), running.len());
        }
        // 요청한 것은 파일 아홉이다. 폴더는 이름이 그림 같아도 요청하지 않는다
        asked.sort();
        let mut files: Vec<String> = (0..7).map(|i| util::join(&dir, &format!("사진 {i:03}.jpg"))).collect();
        files.extend(["메모.txt", "보고서.docx"].map(|name| util::join(&dir, name)));
        files.sort();
        assert_eq!(asked, files);
        let t = a.pane(0).tab();
        let folder = t.entries.iter().find(|e| e.name_string() == "폴더.png").expect("no entry named 폴더.png");
        assert!(thumb_key(t, folder, THUMB, a.scale).is_none());
        // 그림 파일은 직접 읽고, 글 파일과 문서는 Quick Look에 맡긴다 (확장자의 대소문자는 가리지 않는다)
        assert!(is_picture(&util::join(&dir, "사진 000.jpg")) && is_picture("/사진/큰 사진.JPG"));
        assert!(!is_picture(&util::join(&dir, "메모.txt")) && !is_picture(&util::join(&dir, "보고서.docx")) && !is_picture("/사진.png/설명"));
        // 다 읽은 뒤에는 요청이 없다
        assert!(a.take_thumbs().is_empty() && a.thumbs.queue.is_empty());
        assert_eq!(a.thumbs.cache.len(), 9);
        // 같은 폴더를 두 칸에서 아이콘으로 봐도 파일마다 한 번만 읽는다
        let (mut b, _g) = app(&dir);
        b.set_view(0, ViewMode::Large);
        b.set_view(1, ViewMode::Large);
        let mut read = 0;
        loop {
            let reqs = b.take_thumbs();
            if reqs.is_empty() {
                break;
            }
            assert!(reqs.len() <= THUMB_JOBS);
            for r in reqs {
                read += 1;
                b.on_thumb(&r, Picture::blank(8, 8));
            }
        }
        assert_eq!(read, 9);
        // 읽을 픽셀 수의 단계: 그림 자리 × 화면 배율을 올린 값. 가장 큰 단계를 넘지 않는다
        let px = [(48.0, 1.0), (48.0, 2.0), (96.0, 1.0), (96.0, 2.0), (256.0, 1.0), (256.0, 2.0), (256.0, 3.0)].map(|(size, scale)| thumb_px(size, scale));
        assert_eq!(px, [64, 128, 128, 256, 256, 512, 512]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 기다리는 썸네일은 지금 보이는 것을 따른다: 스크롤로 지나간 것, 떠난 폴더와 가려진 탭의 것은 읽기 전에 빠진다.
    /// 이미 읽기 시작한 것은 늦게 와도 남겨 둔다
    #[test]
    fn queued_thumbnails_follow_what_is_visible_now() {
        let (root, dir) = thumbs_demo(200);
        let (mut a, g) = app(&dir);
        a.set_view(0, ViewMode::Medium);
        let first = a.take_thumbs();
        assert_eq!(first.len(), THUMB_JOBS);
        let waiting: Vec<ThumbKey> = a.thumbs.queue.iter().cloned().collect();
        assert!(!waiting.is_empty() && waiting.len() < 100, "only the visible part of the folder waits: {}", waiting.len());
        // 맨 아래로 스크롤: 위쪽에서 기다리던 것은 빠지고, 지금 보이는 아래쪽 것이 기다린다
        let l = a.list_rect(0).unwrap();
        a.scroll(&g, l.l + 60.0, l.t + 60.0, 1.0e6);
        assert!(a.pane(0).tab().scroll > 0.0);
        assert!(a.take_thumbs().is_empty(), "two are still being read");
        assert!(!a.thumbs.queue.is_empty() && a.thumbs.queue.iter().all(|k| !waiting.contains(k)), "nothing that scrolled away still waits");
        assert!(a.thumbs.queue.iter().any(|k| k.file.path.ends_with("사진 199.jpg")));
        // 읽던 것이 끝나면 지금 보이는 것을 읽기 시작한다. 지나간 것의 결과도 남겨 둔다
        for r in &first {
            a.on_thumb(r, Picture::blank(8, 8));
        }
        let next = a.take_thumbs();
        assert_eq!(next.len(), THUMB_JOBS);
        assert!(next.iter().all(|r| !waiting.contains(&r.key) && first.iter().all(|f| f.key != r.key)));
        assert!(first.iter().all(|r| a.thumbs.get(&r.key).is_some()), "a result that came late is kept");
        // 다른 폴더로 옮기면 기다리던 것이 모두 빠진다 (폴더를 읽기 전에도, 읽은 뒤에도)
        assert!(!a.thumbs.queue.is_empty());
        a.navigate(0, Location::Dir(util::join(&dir, "폴더.png")), None);
        assert!(a.take_thumbs().is_empty() && a.thumbs.queue.is_empty());
        list_now(&mut a);
        for r in &next {
            a.on_thumb(r, Picture::blank(8, 8));
        }
        assert!(a.take_thumbs().is_empty() && a.thumbs.queue.is_empty() && a.thumbs.loading.is_empty());
        // 돌아오면 읽어 둔 것은 다시 읽지 않고 나머지만 읽는다
        a.go_back(0);
        list_now(&mut a);
        let again = a.take_thumbs();
        assert_eq!(again.len(), THUMB_JOBS);
        assert!(again.iter().all(|r| first.iter().all(|f| f.key != r.key)), "what was read is not read again");
        // 다른 탭이 앞에 오면 가려진 탭의 것은 기다리지 않고, 그 탭으로 돌아오면 다시 기다린다
        assert!(!a.thumbs.queue.is_empty());
        a.new_tab(0);
        assert!(a.take_thumbs().is_empty() && a.thumbs.queue.is_empty());
        a.activate_tab(0, 0);
        assert!(a.take_thumbs().is_empty() && !a.thumbs.queue.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 읽어 둔 썸네일은 한도(개수, 바이트)까지만 남기고, 넘으면 가장 오래 안 쓴 것부터 버린다. 지금 보이는 것은 버리지 않는다
    #[test]
    fn thumbnail_cache_drops_the_least_recently_used() {
        let key = |i: usize| ThumbKey { file: CardKey { path: format!("/사진/{i}.jpg"), modified: 1, size: 1 }, px: 128 };
        // i번째 그림 하나만 보이는 화면: 한 변이 side 픽셀인 썸네일을 읽어 넣는다 (그 앞의 것들은 화면에 없다)
        let show = |t: &mut Thumbs, i: usize, side: usize| {
            t.sync(vec![key(i)]);
            for r in t.start() {
                t.finish(&r.key, Picture::blank(side, side));
            }
        };
        let mut t = Thumbs::default();
        for i in 0..THUMBS_MAX {
            show(&mut t, i, 4);
        }
        assert_eq!((t.cache.len(), t.bytes), (THUMBS_MAX, THUMBS_MAX * 4 * 4 * 4));
        // 0번을 다시 본다 (읽어 둔 것이라 다시 읽지 않는다): 가장 최근에 쓴 것이 된다
        t.sync(vec![key(0)]);
        assert!(t.start().is_empty());
        // 새 것 둘이 들어오면 가장 오래 안 쓴 1번과 2번이 빠지고, 0번과 3번은 남는다
        show(&mut t, THUMBS_MAX, 4);
        show(&mut t, THUMBS_MAX + 1, 4);
        assert_eq!((t.cache.len(), t.bytes), (THUMBS_MAX, THUMBS_MAX * 4 * 4 * 4));
        assert!(t.get(&key(0)).is_some() && t.get(&key(1)).is_none() && t.get(&key(2)).is_none() && t.get(&key(3)).is_some());
        assert!(t.get(&key(THUMBS_MAX)).is_some() && t.get(&key(THUMBS_MAX + 1)).is_some());
        // 읽지 못한 그림도 한 자리를 차지하지만 메모리는 쓰지 않는다
        t.sync(vec![key(9000)]);
        for r in t.start() {
            t.finish(&r.key, None);
        }
        assert_eq!((t.cache.len(), t.bytes), (THUMBS_MAX, (THUMBS_MAX - 1) * 4 * 4 * 4));
        // 바이트 한도: 큰 썸네일은 개수가 한도에 못 미쳐도 오래 안 쓴 것부터 빠진다
        let mut t = Thumbs::default();
        let fit = THUMB_BYTES / (1024 * 1024 * 4);
        assert!(fit < THUMBS_MAX);
        for i in 0..fit + 3 {
            show(&mut t, i, 1024);
        }
        assert_eq!((t.cache.len(), t.bytes), (fit, fit * 1024 * 1024 * 4));
        assert!(t.bytes <= THUMB_BYTES && t.get(&key(2)).is_none() && t.get(&key(3)).is_some());
        // 지금 보이는 것이 한도보다 많으면 한도를 넘겨 둔다: 보이는 것을 버리면 곧바로 다시 읽게 되어 끝나지 않는다
        let mut t = Thumbs::default();
        let all: Vec<ThumbKey> = (0..THUMBS_MAX + 5).map(key).collect();
        let mut read = 0;
        loop {
            t.sync(all.clone());
            let reqs = t.start();
            if reqs.is_empty() {
                break;
            }
            assert!(reqs.len() <= THUMB_JOBS);
            for r in reqs {
                read += 1;
                t.finish(&r.key, Picture::blank(4, 4));
            }
        }
        assert_eq!((read, t.cache.len()), (THUMBS_MAX + 5, THUMBS_MAX + 5));
        // 화면이 바뀌면 다음 것을 넣을 때 한도 안으로 돌아온다
        show(&mut t, 9000, 4);
        assert_eq!(t.cache.len(), THUMBS_MAX);
        assert!(t.get(&key(9000)).is_some());
    }

    /// 읽지 못한 그림은 적어 두고 다시 요청하지 않는다. 파일이 바뀌면(크기, 수정한 때) 다른 그림으로 보고 다시 읽는다
    #[test]
    fn a_picture_that_could_not_be_read_is_not_asked_for_again() {
        let (root, dir) = thumbs_demo(1);
        // 그림 하나만 남긴다 (글 파일과 문서도 썸네일을 요청한다)
        for name in ["메모.txt", "보고서.docx"] {
            std::fs::remove_file(root.join("사진첩").join(name)).unwrap();
        }
        let (mut a, _g) = app(&dir);
        a.set_view(0, ViewMode::Large);
        let reqs = a.take_thumbs();
        assert_eq!(reqs.len(), 1);
        a.on_thumb(&reqs[0], None);
        assert!(a.take_thumbs().is_empty() && a.thumbs.queue.is_empty(), "not asked for again");
        assert!(a.take_thumbs().is_empty(), "nor on the paint after that");
        assert!(a.thumbs.get(&reqs[0].key).is_none() && a.thumbs.cache.len() == 1, "the failure is remembered, there is no picture");
        // 파일이 바뀌었다: 다시 읽은 목록의 그 파일은 다른 그림이다.
        // settle은 실제로 읽는다. 내용이 그림이 아니라 이번에도 읽지 못하고, 그 뒤로는 요청이 없다
        std::fs::write(root.join("사진첩").join("사진 000.jpg"), b"not a picture either").unwrap();
        a.refresh(0);
        list_now(&mut a);
        a.settle();
        assert_eq!(a.thumbs.cache.len(), 2, "the changed file was tried once more");
        assert!(a.thumbs.cache.values().all(|t| t.pic.is_none()));
        assert!(a.take_thumbs().is_empty() && a.thumbs.loading.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 한 쪽짜리 PDF (w×h pt, 둘레를 남긴 파란 사각형 하나. 그림 시험·창 시험과 같이 쓴다)
    fn tiny_pdf(w: u32, h: u32) -> Vec<u8> {
        crate::render::tiny_pdf(w, h)
    }

    /// 그림이 아닌 파일의 썸네일은 Quick Look이 실제로 만든다 (맥 러너에서): PDF는 그 쪽의 내용으로 요청한 크기 안에 온다.
    /// 썸네일을 만드는 확장이 없는 형식은 아이콘을 받지 않고 None이다 (그런 파일은 사카두가 아이콘을 그린다)
    #[test]
    fn quick_look_makes_thumbnails_of_other_files() {
        let (root, _home) = demo();
        let request = |name: &str, px: u32| ThumbRequest { key: ThumbKey { file: CardKey { path: util::normalize(&root.join(name).to_string_lossy()), modified: 1, size: 1 }, px } };
        std::fs::write(root.join("세로 문서.pdf"), tiny_pdf(200, 300)).unwrap();
        let req = request("세로 문서.pdf", 256);
        assert!(!is_picture(&req.key.file.path));
        let pic = load_thumb(&req).expect("Quick Look made no thumbnail of a PDF");
        // Finder처럼 아이콘 모양으로 꾸며 받아 정사각형 바탕에 종이가 놓여 온다 (러너에서 256×256): 모양 대신 크기와 내용을 본다
        assert!(pic.longest() > 0 && pic.longest() <= 256, "within 256 px, got {:?}", pic.size());
        // 한가운데는 PDF의 파란 사각형이다: 파일 아이콘이 아니라 그 파일의 내용으로 만든 썸네일
        let [r, g, b, a] = pic.center_color().expect("could not read the thumbnail's pixels");
        assert!(b > 180 && r < 90 && g < 120 && a > 200, "the middle of the PDF thumbnail should be its blue square, got {:?}", [r, g, b, a]);
        std::fs::write(root.join("모르는 형식.sakadu-test"), [0u8, 1, 2, 3, 0xFF]).unwrap();
        let none = load_thumb(&request("모르는 형식.sakadu-test", 256));
        assert!(none.is_none(), "an unknown format gave {:?} (an icon instead of no thumbnail?)", none.map(|p| p.size()));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 문서의 카드: PDF를 고르면 Quick Look이 만든 첫 쪽이 카드에 크게 보인다 (맥 러너에서 실제로 만든다).
    /// 아이콘처럼 꾸미지 않아 쪽의 모양(세로로 긴 종이) 그대로이고, 한가운데는 PDF의 파란 사각형이다.
    /// 썸네일을 만들지 못하는 형식은 읽어 본 뒤에도 카드가 뜨지 않고, 다시 묻지 않는다
    #[test]
    fn doc_card_shows_a_plain_quick_look_thumbnail() {
        let (root, dir) = preview_demo();
        std::fs::write(root.join("미리 보기").join("세로 문서.pdf"), tiny_pdf(200, 300)).unwrap();
        std::fs::write(root.join("미리 보기").join("모르는 형식.sakadu-test"), [0u8, 1, 2, 3, 0xFF]).unwrap();
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        click_name(&mut a, &g, 0, "세로 문서.pdf", none);
        assert!(a.card().is_none(), "a document card waits for its thumbnail");
        let reqs = a.take_previews();
        assert_eq!(reqs.iter().map(|r| (r.kind, r.px, r.pane)).collect::<Vec<_>>(), [(CardKind::Doc, PX_SMALL, false)]);
        for r in &reqs {
            let result = load_preview(r);
            a.on_previewed(r, result);
        }
        assert!(a.card().is_some(), "Quick Look made no thumbnail of the PDF");
        let pic = &a.pics.last().expect("the thumbnail is kept").pic;
        let (w, h) = pic.size();
        assert!(w < h && h <= PX_SMALL as usize, "the page as it is, not a square icon canvas: {:?}", (w, h));
        let [r, gr, b, al] = pic.center_color().expect("could not read the thumbnail's pixels");
        assert!(b > 180 && r < 90 && gr < 120 && al > 200, "the middle of the thumbnail should be the PDF's blue square, got {:?}", [r, gr, b, al]);
        let (target, _) = a.card_shown().unwrap();
        assert_eq!(a.card_note(&target), None);
        let mut canvas = Gfx::bitmap(1280.0, 800.0, 1.0).unwrap();
        a.paint(&mut canvas);
        // 썸네일을 만들지 못하는 형식
        click_name(&mut a, &g, 0, "모르는 형식.sakadu-test", none);
        a.settle();
        assert!(a.card().is_none() && a.card.failed);
        assert!(a.take_previews().is_empty(), "not asked for again");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 문서의 카드는 썸네일이 온 뒤에 뜬다: 만들지 못했으면 뜨지 않는다. 크게 보면 더 크게 한 번 더 읽고,
    /// 그것이 실패해도 읽어 둔 썸네일은 남는다 (읽은 결과를 직접 넣어 본다)
    #[test]
    fn doc_card_waits_for_its_thumbnail_and_hides_without_one() {
        let (root, dir) = preview_demo();
        for name in ["문서 1.pdf", "문서 2.key"] {
            std::fs::write(root.join("미리 보기").join(name), b"x").unwrap();
        }
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        click_name(&mut a, &g, 0, "문서 1.pdf", none);
        let first = a.take_previews();
        assert_eq!((first.len(), first[0].kind, first[0].px), (1, CardKind::Doc, PX_SMALL));
        assert!(a.card().is_none(), "nothing to show yet");
        a.on_previewed(&first[0], None);
        assert!(a.card().is_none() && a.card.failed && a.take_previews().is_empty());
        click_name(&mut a, &g, 0, "문서 2.key", none);
        let second = a.take_previews();
        a.on_previewed(&second[0], Some(Preview::Image(Picture::blank(768, 1024).unwrap())));
        let small = a.card().expect("the card shows once the thumbnail is in");
        a.key(Key::Char(' '), none);
        let larger = a.take_previews();
        assert_eq!((larger.len(), larger[0].px), (1, PX_LARGE));
        a.on_previewed(&larger[0], None);
        assert!(a.card().is_some_and(|m| m.card != small.card) && !a.card.failed, "the small thumbnail stays in the large card");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 한글 문서: 안에 든 미리 보기 그림(PrvImage)이 카드와 오른쪽 창에 보이고, 그림이 없으면 미리 보기 글(PrvText)이 보인다.
    /// 복합 파일이 아니면 카드가 뜨지 않고 창은 정보만 보인다. 시험 파일은 공통 코드의 시험용 만들기(cfb::build)로 만든다
    #[test]
    fn hwp_shows_its_preview_picture_or_text() {
        let (root, dir) = preview_demo();
        let text: Vec<u8> = "회의 자료\r\n\t둘째 줄\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let picture = crate::render::demo_picture();
        let at = root.join("미리 보기");
        let head: &[u8] = b"HWP Document File";
        std::fs::write(at.join("그림 있음.hwp"), sakadu::cfb::build(&[("FileHeader", head), ("PrvImage", &picture), ("PrvText", &text)])).unwrap();
        std::fs::write(at.join("글만.hwp"), sakadu::cfb::build(&[("FileHeader", head), ("PrvText", &text)])).unwrap();
        std::fs::write(at.join("망가짐.hwp"), b"not a compound file").unwrap();
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        click_name(&mut a, &g, 0, "그림 있음.hwp", none);
        a.settle();
        assert!(a.card().is_some());
        assert_eq!(a.pics.last().map(|c| c.pic.size()), Some((480, 320)), "the picture inside the document");
        click_name(&mut a, &g, 0, "글만.hwp", none);
        a.settle();
        assert!(a.card().is_some());
        assert_eq!(a.card.text.as_deref(), Some("회의 자료\n    둘째 줄\n"));
        // 글을 읽었으면 크게 봐도 다시 읽지 않는다
        a.key(Key::Char(' '), none);
        assert!(a.card.large && a.take_previews().is_empty());
        a.key(Key::Char(' '), none);
        click_name(&mut a, &g, 0, "망가짐.hwp", none);
        a.settle();
        assert!(a.card().is_none() && a.card.failed);
        // 오른쪽 창도 같은 것을 보인다 (한글 문서는 Quick Look 보기 대신 안에 든 미리 보기)
        a.run_action(Action::Preview);
        click_name(&mut a, &g, 0, "그림 있음.hwp", none);
        a.settle();
        assert_eq!(a.preview_body_seen(), Some(("picture", Some((480, 320)))));
        click_name(&mut a, &g, 0, "글만.hwp", none);
        a.settle();
        assert_eq!(a.preview_seen(), Some((Some("글만.hwp".to_string()), Some("회의 자료\n    둘째 줄\n".to_string()))));
        click_name(&mut a, &g, 0, "망가짐.hwp", none);
        a.settle();
        assert_eq!(a.preview_body_seen(), Some(("info", None)));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 오른쪽 미리 보기 창: ⌥⌘P와 메뉴 막대의 동작으로 열고 닫는다. 열리면 칸들이 그만큼 좁아지고(도구 막대 단추는 제자리),
    /// 오른쪽 미리 보기 창이 열렸는지는 작업 공간마다 기억한다 (Windows와 같다)
    #[test]
    fn preview_pane_is_remembered_per_workspace() {
        let (root, home) = demo();
        let (mut a, _) = app(&home);
        a.toggle_preview();
        assert!(a.preview.is_some());
        a.new_workspace();
        a.edit = Editing::None;
        assert!(a.preview.is_some(), "a new workspace starts from the current screen");
        a.toggle_preview();
        a.switch_workspace(0);
        assert!(a.preview.is_some(), "the first workspace had it open");
        a.switch_workspace(1);
        assert!(a.preview.is_none(), "the second one had it closed");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 지금 칸에서 고른 것을 따라 보인다: 그림, 글, 폴더·압축 파일·글이 아닌 것은 정보, 문서는 창 쪽의 Quick Look 보기
    /// (앱이 그린 메뉴가 떠 있으면 감춘다. 그 보기를 만들 수 없으면 큰 썸네일). 열려 있는 동안 자동 미리 보기 카드는 뜨지 않는다
    #[test]
    fn preview_pane_opens_with_alt_cmd_p_and_follows_the_selection() {
        let (root, dir) = preview_demo();
        std::fs::write(root.join("미리 보기").join("책.pdf"), tiny_pdf(200, 300)).unwrap();
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        let alt_cmd = Mods { cmd: true, alt: true, ..Default::default() };
        assert_eq!(a.keymap().get(Action::Preview), Combo::parse("Alt+Cmd+P"));
        assert!(mac_runs(Action::Preview));
        click_name(&mut a, &g, 0, "노을.bmp", none);
        a.settle();
        assert!(a.card().is_some());
        let (wide, tools) = (pane_rect(&a, 1), a.toolbar());
        assert!(a.key(Key::Char('p'), alt_cmd));
        let pr = a.preview_rect().expect("alt-cmd-P opens the pane");
        assert!(a.card().is_none(), "the card gives way to the pane");
        assert_eq!((pane_rect(&a, 1).r, pr.r, pr.w(), pr.t), (pr.l - GAP, wide.r, 340.0, wide.t));
        assert_eq!((a.toolbar().search, a.toolbar().preview), (tools.search, tools.preview), "the toolbar buttons stay put");
        a.settle();
        assert_eq!(a.preview_seen().map(|s| s.0), Some(Some("노을.bmp".to_string())));
        assert_eq!(a.preview_body_seen(), Some(("picture", Some((480, 320)))));
        click_name(&mut a, &g, 0, "메모.txt", none);
        a.settle();
        assert_eq!(a.preview_seen(), Some((Some("메모.txt".to_string()), Some("첫 줄\n    둘째 줄\n".to_string()))));
        for (name, shows) in [("폴더", "info"), ("보관함.zip", "info"), ("깨진 그림.jpg", "broken"), ("글 아님.txt", "info"), ("책.pdf", "look")] {
            click_name(&mut a, &g, 0, name, none);
            a.settle();
            assert_eq!(a.preview_body_seen().map(|s| s.0), Some(shows), "{name}");
        }
        // 창 쪽이 본문 자리에 Quick Look 보기를 띄운다. 앱이 그린 메뉴가 떠 있으면 그 메뉴를 가리지 않게 감춘다
        let body = a.preview_body().unwrap();
        let book = util::join(&dir, "책.pdf");
        assert!(a.look_view().is_some_and(|(r, p, _)| r == body && p == book), "{:?}", a.look_view());
        a.open_menu(&g, MenuKind::Layout);
        assert!(a.look_view().is_none(), "an app-drawn menu is not covered");
        a.close_menu();
        assert!(a.look_view().is_some());
        // 창 위를 누른 것은 아래 칸으로 가지 않는다
        let (pin, open) = a.preview_buttons().unwrap();
        assert_eq!((a.hit(&g, mid(pin).0, mid(pin).1), a.hit(&g, mid(open).0, mid(open).1)), (Hit::PreviewPin, Hit::PreviewOpen));
        assert_eq!(a.hit(&g, body.l + 10.0, body.t + 10.0), Hit::PreviewArea);
        // 열기: 파일은 기본 앱으로
        let opened = a.open_files.len();
        a.mouse_down(&g, mid(open).0, mid(open).1, none, 1);
        assert_eq!(&a.open_files[opened..], std::slice::from_ref(&book));
        // Quick Look 보기를 만들 수 없으면 큰 Quick Look 썸네일 (쪽 모양 그대로)
        a.look_view_missing();
        a.settle();
        assert!(a.look_view().is_none());
        assert!(matches!(a.preview_body_seen(), Some(("picture", Some((w, h)))) if w < h), "{:?}", a.preview_body_seen());
        let mut canvas = Gfx::bitmap(1280.0, 800.0, 1.0).unwrap();
        a.paint(&mut canvas);
        // 아무것도 고르지 않으면 안내만
        a.clear_selection();
        a.settle();
        assert_eq!(a.preview_seen(), Some((None, None)));
        a.paint(&mut canvas);
        // 메뉴 막대의 동작(보기 > 미리 보기 창)으로 닫는다: 칸들이 다시 넓어지고 카드가 돌아온다
        assert!(a.run_action(Action::Preview));
        assert!(a.preview_rect().is_none() && a.preview_seen().is_none() && a.look_view().is_none());
        assert_eq!(pane_rect(&a, 1), wide);
        click_name(&mut a, &g, 0, "노을.bmp", none);
        a.settle();
        assert!(a.card().is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 창의 글은 스크롤된다: 창 위의 스크롤은 글만 움직이고(끝에서 멈춘다) 뒤의 칸은 그대로다. 창은 카드보다 많은 줄을 보이고,
    /// 다른 파일을 고르면 맨 위부터 보인다
    #[test]
    fn preview_pane_scrolls_long_text() {
        let (root, dir) = preview_demo();
        let long: String = (0..1000).map(|i| format!("줄 {i}\n")).collect();
        std::fs::write(root.join("미리 보기").join("긴 글.txt"), long).unwrap();
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        a.run_action(Action::Preview);
        click_name(&mut a, &g, 0, "긴 글.txt", none);
        a.settle();
        let text = a.preview_seen().and_then(|s| s.1).expect("the text is read");
        assert_eq!(text.lines().count(), preview_pane::PANE_LINES, "more lines than the card's {TEXT_LINES}");
        let body = a.preview_body().unwrap();
        let (x, y) = mid(body);
        a.scroll(&g, x, y, 100.0);
        assert_eq!(a.preview_scroll(), 100.0);
        a.scroll(&g, x, y, 1.0e6);
        assert_eq!(a.preview_scroll(), preview_pane::PANE_LINES as f32 * MONO_LINE_H + 20.0 - body.h(), "it stops at the last line");
        let mut canvas = Gfx::bitmap(1280.0, 800.0, 1.0).unwrap();
        a.paint(&mut canvas);
        a.scroll(&g, x, y, -1.0e6);
        assert_eq!(a.preview_scroll(), 0.0);
        assert_eq!(a.pane(0).tab().scroll, 0.0, "the pane under it does not move");
        // 칸 위의 스크롤은 창의 글을 움직이지 않는다
        let (lx, ly) = row_point(&a, 0, 0);
        a.scroll(&g, lx, ly, 50.0);
        assert_eq!(a.preview_scroll(), 0.0);
        a.scroll(&g, x, y, 300.0);
        click_name(&mut a, &g, 0, "메모.txt", none);
        a.settle();
        assert_eq!(a.preview_scroll(), 0.0, "another file starts at the top");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 핀으로 고정하면 다른 것을 고르거나 다른 폴더로 가도 그 파일을 계속 보인다. 열림과 고정한 파일(경로, 디스크·파일 번호)은
    /// 세션에 남아 다음에 그대로 연다 (확인한 뒤에 읽는다). 같은 폴더에서 이름이 바뀌면 따라가고, 고쳐지면 다시 읽고,
    /// 지워지면 고정을 풀고 알린다. 핀을 다시 누르면 풀려 다시 고른 것을 따라간다
    #[test]
    fn pinned_file_stays_through_navigation_and_sessions() {
        use std::os::unix::fs::MetadataExt;
        let (root, dir) = preview_demo();
        let (mut a, g) = app(&dir);
        let none = Mods::default();
        a.run_action(Action::Preview);
        click_name(&mut a, &g, 0, "메모.txt", none);
        a.settle();
        let (pin, _) = a.preview_buttons().unwrap();
        a.mouse_down(&g, mid(pin).0, mid(pin).1, none, 1);
        a.mouse_up();
        a.settle();
        let memo = util::join(&dir, "메모.txt");
        let m = std::fs::metadata(&memo).unwrap();
        let id = (m.dev() as u32, m.ino());
        assert_eq!(a.preview_pin(), Some((memo.clone(), Some(id))));
        // 다른 파일을 골라도, 다른 폴더로 가도 그대로
        let shows_memo = Some((Some("메모.txt".to_string()), Some("첫 줄\n    둘째 줄\n".to_string())));
        click_name(&mut a, &g, 0, "노을.bmp", none);
        a.navigate(0, Location::Dir(util::join(&dir, "폴더")), None);
        a.settle();
        assert_eq!(a.preview_seen(), shows_memo);
        // 세션(파일 형식까지): 다음에 열면 창이 열리고 그 파일이 고정되어 있다. 확인하기 전에는 읽지 않는다
        let s = session::parse(&session::serialize(&a.to_session())).expect("session");
        assert!(s.preview);
        assert_eq!((s.pin.as_deref(), s.pin_id), (Some(memo.as_str()), Some(id)));
        let (mut b, _g) = app(&dir);
        b.open_saved(a.workspaces().clone(), Some(&s), &dir);
        assert_eq!(b.preview_body_seen(), Some(("waiting", None)), "nothing is read before the check");
        b.settle();
        assert_eq!((b.preview_seen(), b.preview_pin()), (shows_memo.clone(), Some((memo.clone(), Some(id)))));
        // 핀을 다시 누르면 풀린다: 지금 칸에서 고른 것이 없으니 안내만 보인다
        b.toggle_preview_pin();
        b.settle();
        assert_eq!((b.preview_pin(), b.preview_seen()), (None, Some((None, None))));
        // 창을 닫고 끝냈으면 다음에도 닫혀 있다
        b.run_action(Action::Preview);
        let closed = session::parse(&session::serialize(&b.to_session())).unwrap();
        assert!(!closed.preview && closed.pin.is_none());
        // 같은 폴더에서 이름이 바뀌면 따라간다 (폴더가 바뀐 알림 뒤에 확인한다)
        let renamed = util::join(&dir, "메모 새 이름.txt");
        std::fs::rename(&memo, &renamed).unwrap();
        a.on_dir_changed(&dir);
        a.settle();
        assert_eq!(a.preview_pin(), Some((renamed.clone(), Some(id))));
        assert_eq!(a.preview_seen().and_then(|s| s.0).as_deref(), Some("메모 새 이름.txt"));
        // 고친 내용은 다시 읽는다 (창이 다시 앞으로 올 때 확인한다)
        std::fs::write(&renamed, "고친 줄\n").unwrap();
        a.refresh_shown();
        a.settle();
        assert_eq!(a.preview_seen().and_then(|s| s.1).as_deref(), Some("고친 줄\n"));
        // 지워지면 고정을 풀고 알린다
        std::fs::remove_file(&renamed).unwrap();
        a.refresh_shown();
        a.settle();
        assert!(a.preview_pin().is_none());
        assert!(a.flash.as_ref().is_some_and(|(_, m)| m.contains("고정을 풀었어요")), "{:?}", a.flash);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// settle(시험·그림)은 보이는 썸네일을 바로 읽는다: 그림은 그 아이콘 크기에 맞게 줄여 읽고, 읽지 못한 것은 그렇게 적어 둔다
    #[test]
    fn settle_reads_the_visible_thumbnails() {
        let (root, dir) = preview_demo();
        let (mut a, _g) = app(&dir);
        assert!(a.thumbs.cache.is_empty(), "nothing is read for the details view");
        a.set_view(0, ViewMode::Large);
        a.settle();
        assert!(a.take_thumbs().is_empty(), "everything visible was read");
        let key = |name: &str| {
            let t = a.pane(0).tab();
            let e = t.entries.iter().find(|e| e.name_string() == name).unwrap_or_else(|| panic!("no entry named {name}"));
            thumb_key(t, e, THUMB, a.scale)
        };
        // 큰 아이콘(96pt)을 2배 화면에 그릴 192픽셀을 단계로 올린 256픽셀까지 줄여 읽는다 (원본은 480×320)
        for name in ["노을.bmp", "바다.bmp"] {
            let k = key(name).unwrap_or_else(|| panic!("{name} is a picture"));
            assert_eq!(k.px, 256);
            let pic = a.thumbs.get(&k).unwrap_or_else(|| panic!("{name} was not read"));
            assert!((129..=256).contains(&pic.longest()), "{name}: {:?}", pic.size());
        }
        assert!(a.thumbs.get(&key("깨진 그림.jpg").unwrap()).is_none());
        // 그림이 아닌 파일 셋은 Quick Look에 맡겼다 (만들었는지는 형식에 달렸다. PDF는 아래 시험이 본다). 폴더는 썸네일이 없다
        assert!(key("폴더").is_none());
        for name in ["메모.txt", "글 아님.txt", "보관함.zip"] {
            let k = key(name).unwrap_or_else(|| panic!("{name} has no thumbnail key"));
            assert!(a.thumbs.cache.contains_key(&k), "{name} was not asked for");
        }
        assert_eq!(a.thumbs.cache.len(), 6, "two pictures, one that could not be read, three other files");
        // 아이콘 크기를 바꾸면 그 크기의 단계로 다시 읽는다 (보통 아이콘 48pt는 128픽셀)
        a.set_view(0, ViewMode::Medium);
        let reqs = a.take_thumbs();
        assert_eq!(reqs.len(), THUMB_JOBS);
        assert!(reqs.iter().all(|r| r.key.px == 128));
        // 자세히 보기로 돌아가면 기다리는 것이 없다
        a.set_view(0, ViewMode::Details);
        assert!(a.take_thumbs().is_empty() && a.thumbs.queue.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 글 파일 앞부분 읽기: UTF-8과 BOM이 있는 UTF-16은 읽고, 글이 아닌 것은 버린다. 앞 80줄만, 긴 줄은 자른다
    #[test]
    fn decode_text_reads_utf8_and_utf16_with_a_bom() {
        assert_eq!(decode_text("가나\r\n\t다".as_bytes()).as_deref(), Some("가나\n    다\n"));
        assert_eq!(decode_text(b"\xEF\xBB\xBFabc").as_deref(), Some("abc\n"));
        assert_eq!(decode_text(b"").as_deref(), Some(""));
        let utf16 = |big_endian: bool| -> Vec<u8> {
            let bom: [u8; 2] = if big_endian { [0xFE, 0xFF] } else { [0xFF, 0xFE] };
            bom.into_iter().chain("한글 😀\nok".encode_utf16().flat_map(|u| if big_endian { u.to_be_bytes() } else { u.to_le_bytes() })).collect()
        };
        assert_eq!(decode_text(&utf16(false)).as_deref(), Some("한글 😀\nok\n"));
        assert_eq!(decode_text(&utf16(true)).as_deref(), Some("한글 😀\nok\n"));
        // 글이 아닌 것: NUL이 있는 것, UTF-8이 아닌 바이트, BOM 없는 UTF-16
        assert_eq!(decode_text(b"ab\0cd"), None);
        assert_eq!(decode_text(&[b'a', 0xFF, b'b']), None);
        assert_eq!(decode_text(&"abc".encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>()), None);
        // 읽기를 자른 자리가 글자 중간이면 그 앞까지 읽는다 (UTF-16은 두 칸짜리 글자의 앞 칸까지 읽은 경우)
        let cut = "가나다".as_bytes();
        assert_eq!(decode_text(&cut[..cut.len() - 1]).as_deref(), Some("가나\n"));
        assert_eq!(decode_text(&utf16(false)[..2 + 3 * 2 + 2]).as_deref(), Some("한글 \n"));
        // 앞 80줄만, 긴 줄은 200자까지. 줄마다 '\n'으로 끝난다
        let long: String = (0..TEXT_LINES + 20).map(|i| if i == 1 { "가".repeat(TEXT_COLS + 50) + "\n" } else { format!("줄 {i}\n") }).collect();
        let s = decode_text(long.as_bytes()).unwrap();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!((lines.len(), lines[1].chars().count(), lines[TEXT_LINES - 1]), (TEXT_LINES, TEXT_COLS, format!("줄 {}", TEXT_LINES - 1).as_str()));
        assert!(s.ends_with('\n'));
        // 카드에는 보이는 줄까지만 넘긴다
        assert_eq!(first_lines("a\nb\nc\n", 2), "a\nb\n");
        assert_eq!(first_lines("a\nb\n", 5), "a\nb\n");
        assert_eq!(first_lines("", 3), "");
    }
}
