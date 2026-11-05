//! 설정 창: 왼쪽에 분류, 오른쪽에 "제목·설명 + 조작" 줄을 카드로 묶어 보여 준다.
//! 창 전체를 덮고 Esc나 ←로 닫는다. F1은 도움말 쪽을 바로 연다

use super::*;
use crate::keys::{ACTIONS, Action, Combo, Keymap};
use crate::theme::ACCENTS;

const SIDE_W: f32 = 220.0;
/// 창이 좁으면(크기 150% 등) 분류를 아이콘만으로 줄인다
const SIDE_W_COMPACT: f32 = 60.0;
const CONTENT_W: f32 = 680.0;
/// 줄 제목에 남겨 둘 최소 폭. 조작이 이보다 넓게 차지하면 조작을 제목 아래로 내린다
const LABEL_MIN_W: f32 = 140.0;
const ZOOMS: [u32; 5] = [90, 100, 110, 125, 150];
const THEMES: [(&str, &str); 3] = [("system", "시스템"), ("light", "밝게"), ("dark", "어둡게")];
/// 글자 표시 (gfx::TextLook의 키)
const TEXT_LOOKS: [(&str, &str); 3] = [("soft", "부드럽게"), ("sharp", "선명하게 (컬러)"), ("gray", "선명하게 (회색)")];
const G_GEAR: u16 = 0xE713;
const G_COLOR: u16 = 0xE790;
const G_KEYBOARD: u16 = 0xE765;
const G_HELP: u16 = 0xE897;
const G_INFO: u16 = 0xE946;
const G_CHEVRON_DOWN: u16 = 0xE70D;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    General,
    Look,
    Search,
    Keys,
    Help,
    About,
}

const PAGES: [(Page, &str, u16); 6] = [
    (Page::General, "일반", G_GEAR),
    (Page::Look, "화면", G_COLOR),
    (Page::Search, "검색", G_SEARCH),
    (Page::Keys, "단축키", G_KEYBOARD),
    (Page::Help, "도움말", G_HELP),
    (Page::About, "정보", G_INFO),
];

/// 설정 줄의 조작
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ctl {
    NewTab,
    Hidden,
    Preview,
    DefaultFm,
    Tray,
    Theme,
    Accent,
    Font,
    Zoom,
    Text,
    Service,
    Global,
    Key(Action),
    KeysReset,
    FolderRemove,
    FolderAdd,
    GitHub,
    Releases,
    DataFolder,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SetHit {
    Panel,
    Back,
    Nav(Page),
    /// 조작과 그 안의 몇 번째 칸(테마·색·크기 버튼, 폴더 줄)
    Ctl(Ctl, usize),
}

pub struct Settings {
    pub page: Page,
    scroll: f32,
    /// 단축키를 바꾸는 중인 줄 (다음에 누르는 키를 받는다)
    capture: Option<Ctl>,
    /// 마지막 안내 (겹친 단축키 등)
    note: Option<String>,
    /// 키보드 초점 (Tab·방향키로 옮기고 Enter·Space로 누른다). 마우스로 누르면 지운다
    focus: Option<SetHit>,
    // 레지스트리·서비스 상태와 넣을 수 있는 폴더는 열 때와 바꾼 뒤에만 읽는다
    default_fm: bool,
    tray: bool,
    service: bool,
    candidates: Vec<String>,
}

/// 화면에 놓인 것들. 그리기와 누른 곳 찾기가 같은 배치를 쓴다
enum Item {
    Nav { r: R, label: &'static str, glyph: u16, on: bool, hit: SetHit },
    Back(R),
    Title(R, String),
    Header(R, String),
    Card(R),
    Divider(R),
    Label { title: R, desc: R, t: String, d: String },
    Toggle { r: R, on: bool, hit: SetHit },
    Seg { r: R, label: String, on: bool, hit: SetHit },
    SegFrame(R),
    Swatch { r: R, color: Color, on: bool, hit: SetHit },
    Button { r: R, label: String, chevron: bool, hit: SetHit },
    Key { r: R, label: String, capturing: bool, empty: bool, hit: SetHit },
    Note(R, String),
}

/// 줄 오른쪽의 조작
enum Widget {
    None,
    Toggle(bool, Ctl),
    Seg(Vec<String>, usize, Ctl),
    Swatches(Vec<Color>, usize),
    Drop(String, Ctl),
    Button(String, Ctl, usize),
    Key(String, bool, Ctl),
}

struct Row {
    title: String,
    desc: String,
    widget: Widget,
}

struct Card {
    header: String,
    rows: Vec<Row>,
}

fn row(title: impl Into<String>, desc: impl Into<String>, widget: Widget) -> Row {
    Row { title: title.into(), desc: desc.into(), widget }
}

impl App {
    pub(super) fn toggle_settings(&mut self, page: Page) {
        match &self.settings {
            Some(s) if s.page == page => self.close_settings(),
            _ => self.open_settings(page),
        }
    }

    pub(super) fn open_settings(&mut self, page: Page) {
        self.close_palette();
        self.fav_blur();
        if self.edit_mode != EditMode::None {
            self.cancel_edit();
        }
        let s = Settings {
            page,
            scroll: 0.0,
            capture: None,
            note: None,
            focus: None,
            default_fm: false,
            tray: false,
            service: false,
            candidates: Vec::new(),
        };
        self.settings = Some(s);
        self.refresh_settings();
        self.hover = Hit::None;
        self.layout_edits();
        self.focus();
        self.invalidate();
    }

    pub(super) fn close_settings(&mut self) {
        if self.settings.take().is_some() {
            self.layout_edits();
            self.focus();
            self.invalidate();
        }
    }

    /// 레지스트리·서비스 상태, 넣을 수 있는 폴더를 다시 읽는다 (설정을 바꾼 뒤, 창이 다시 활성화될 때)
    pub(super) fn refresh_settings(&mut self) {
        if self.settings.is_none() {
            return;
        }
        let default_fm = crate::defaultfm::is_registered(crate::defaultfm::CLASSES);
        let tray = crate::tray::is_enabled(crate::tray::RUN_KEY);
        let service = crate::service::is_installed();
        let candidates: Vec<String> =
            self.visible_dirs().into_iter().filter(|d| self.folders.covering(d).is_none() && crate::folders::outside_disk_index(d)).collect();
        if let Some(s) = &mut self.settings {
            (s.default_fm, s.tray, s.service, s.candidates) = (default_fm, tray, service, candidates);
        }
        self.invalidate();
    }

    fn settings_note(&mut self, note: impl Into<String>) {
        if let Some(s) = &mut self.settings {
            s.note = Some(note.into());
        }
        self.invalidate();
    }

    // ---------- 쪽마다 줄 ----------

    fn key_label(&self, a: Action) -> String {
        self.keys.get(a).map(Combo::label).unwrap_or_else(|| "없음".to_string())
    }

    fn settings_cards(&self, s: &Settings) -> (&'static str, Vec<Card>) {
        match s.page {
            Page::General => {
                let new_tab = match self.new_tab_home.as_str() {
                    "::computer" | "" => "내 PC".to_string(),
                    session::NEW_TAB_SAME => "지금 탭과 같은 폴더".to_string(),
                    p => util::file_name(p).to_string(),
                };
                (
                    "일반",
                    vec![
                        Card {
                            header: "기본".into(),
                            rows: vec![
                                row("새 탭을 열 위치", "새 탭(+)을 열 때 처음 보여 줄 곳", Widget::Drop(new_tab, Ctl::NewTab)),
                                row("숨김 항목 보기", "숨김 파일과 폴더도 목록에 보여요", Widget::Toggle(self.show_hidden, Ctl::Hidden)),
                                row("미리 보기 창", "고른 파일을 오른쪽에서 바로 봐요", Widget::Toggle(self.preview.is_some(), Ctl::Preview)),
                            ],
                        },
                        Card {
                            header: "Windows와 함께".into(),
                            rows: vec![
                                row("기본 파일 관리자로 사용", "폴더를 열 때와 Win+E를 Sakadu로 열어요", Widget::Toggle(s.default_fm, Ctl::DefaultFm)),
                                row(
                                    "트레이에서 대기",
                                    "창을 닫아도 단축키로 바로 검색하고, 로그인할 때 창 없이 시작해요",
                                    Widget::Toggle(s.tray, Ctl::Tray),
                                ),
                            ],
                        },
                    ],
                )
            }
            Page::Look => {
                let theme = THEMES.iter().position(|t| t.0 == self.look.theme).unwrap_or(0);
                let accent = ACCENTS.iter().position(|a| a.0 == self.look.accent).unwrap_or(1);
                let colors = ACCENTS.iter().map(|a| crate::theme::accent_color(a.0, self.theme.dark).unwrap_or(self.theme.accent)).collect();
                let zoom = ZOOMS.iter().position(|z| *z == self.look.zoom).unwrap_or(1);
                let font = if self.look.font.is_empty() { "기본 (Segoe UI)".to_string() } else { self.look.font.clone() };
                let text = TEXT_LOOKS.iter().position(|t| t.0 == self.look.text).unwrap_or(2);
                (
                    "화면",
                    vec![Card {
                        header: String::new(),
                        rows: vec![
                            row("테마", "시스템은 Windows의 앱 모드를 따라가요", Widget::Seg(THEMES.iter().map(|t| t.1.to_string()).collect(), theme, Ctl::Theme)),
                            row("강조 색", ACCENTS[accent].1, Widget::Swatches(colors, accent)),
                            row("글꼴", "목록·메뉴·검색 창에 쓰는 글꼴", Widget::Drop(font, Ctl::Font)),
                            row("글자 표시", "흐리면 선명하게, 색 번짐이 거슬리면 회색", Widget::Seg(TEXT_LOOKS.iter().map(|t| t.1.to_string()).collect(), text, Ctl::Text)),
                            row("크기", "화면 전체를 크게 또는 작게", Widget::Seg(ZOOMS.iter().map(|z| format!("{z}%")).collect(), zoom, Ctl::Zoom)),
                        ],
                    }],
                )
            }
            Page::Search => {
                let mut folders: Vec<Row> = self
                    .folders
                    .list()
                    .into_iter()
                    .enumerate()
                    .map(|(i, (path, state))| {
                        let state = match state {
                            crate::folders::State::Scanning => "훑는 중".to_string(),
                            crate::folders::State::Ready(n) => format!("항목 {n}개"),
                            crate::folders::State::Offline => "연결 안 됨 · 5분마다 다시 시도해요".to_string(),
                        };
                        row(path, state, Widget::Button("빼기".into(), Ctl::FolderRemove, i))
                    })
                    .collect();
                folders.extend(s.candidates.iter().enumerate().map(|(i, p)| row(p.as_str(), "열어 둔 폴더 · 아직 검색 범위 밖", Widget::Button("넣기".into(), Ctl::FolderAdd, i))));
                if folders.is_empty() {
                    folders.push(row("넣은 폴더가 없어요", "공유폴더나 USB를 아무 칸에나 열어 두면 여기서 넣을 수 있어요", Widget::None));
                }
                let service = if s.service {
                    row("검색 서비스", "설치됨 · 이 PC의 NTFS 디스크 전체를 이름으로 바로 찾아요", Widget::Button("제거…".into(), Ctl::Service, 0))
                } else {
                    row("검색 서비스", "설치하면 디스크 전체를 바로 찾아요 · 관리자 승인이 한 번 필요해요", Widget::Button("설치…".into(), Ctl::Service, 0))
                };
                ("검색", vec![Card { header: "디스크 검색".into(), rows: vec![service] }, Card { header: "검색에 넣은 폴더 (공유폴더·USB)".into(), rows: folders }])
            }
            Page::Keys => {
                let capturing = |c: Ctl| s.capture == Some(c);
                // 정한 키라도 Windows에 등록되지 않았으면(다른 프로그램이 먼저 씀) 동작하는 것처럼 보이지 않게 한다
                let (global, global_desc) = match (self.keys.global, &self.hotkey) {
                    (Some(c), Some(_)) => (c.label(), "다른 앱을 쓰다가도 바로 · 트레이 대기를 켜 두면 창을 닫아도 돼요".to_string()),
                    (Some(c), None) => (format!("{} · 등록 안 됨", c.label()), "다른 프로그램이 이 키를 쓰고 있어 지금은 동작하지 않아요 · 눌러서 다른 키로".to_string()),
                    (None, Some(hk)) => (format!("자동 ({hk})"), "다른 앱을 쓰다가도 바로 · 트레이 대기를 켜 두면 창을 닫아도 돼요".to_string()),
                    (None, None) => ("없음".to_string(), "후보 키를 모두 다른 프로그램이 쓰고 있어요 · 눌러서 직접 정하기".to_string()),
                };
                let mut actions: Vec<Row> = ACTIONS
                    .iter()
                    .map(|(a, _, name, _)| row(*name, "", Widget::Key(self.key_label(*a), capturing(Ctl::Key(*a)), Ctl::Key(*a))))
                    .collect();
                actions.push(row("기본값으로 되돌리기", "바꾼 단축키를 모두 처음대로", Widget::Button("되돌리기".into(), Ctl::KeysReset, 0)));
                (
                    "단축키",
                    vec![
                        Card {
                            header: "어디서든".into(),
                            rows: vec![row("검색 창 띄우기", global_desc, Widget::Key(global, capturing(Ctl::Global), Ctl::Global))],
                        },
                        Card { header: "Sakadu 안에서 · 줄을 누르고 새 키를 누르세요 (Esc 취소 · Backspace 지우기)".into(), rows: actions },
                    ],
                )
            }
            Page::Help => {
                let k = |a: Action| self.keys.get(a).map(Combo::label).unwrap_or_else(|| "단축키 없음".into());
                let tip = |t: &str, d: String| row(t, d, Widget::None);
                (
                    "도움말",
                    vec![
                        Card {
                            header: format!("검색 ({})", k(Action::Search)),
                            rows: vec![
                                tip("여러 낱말", "띄어 쓰면 모두 들어간 것만 찾아요 · 견적 2026".into()),
                                tip("또는", "사이에 |를 넣으면 그중 하나라도 · 견적 | 계약서 ·여러 줄을 붙여넣으면 한 번에".into()),
                                tip("경로로 찾기", "끝에 \\를 붙이면 경로에서 찾아요 · 업무\\".into()),
                                tip("여러 개 다루기", "Ctrl·Shift+클릭으로 고르고 Enter 모두 열기 · Ctrl+C 파일 복사".into()),
                                tip("위치 열기", "Ctrl+Enter · 새 탭에서 그 폴더를 열고 파일을 골라 둬요".into()),
                            ],
                        },
                        Card {
                            header: "목록과 탭".into(),
                            rows: vec![
                                tip("걸러 보기", "목록에서 글자를 치면 지금 폴더에서 바로 걸러요 · Esc로 풀기".into()),
                                tip("여러 개 고르기", "빈 곳이나 날짜 칸에서 끌면 네모 상자로 · Ctrl을 누르면 더하기".into()),
                                tip("탭 옮기기", "탭을 끌어 순서를 바꾸거나 다른 칸으로 · Ctrl+Tab 다음 탭".into()),
                                tip("칸 이동", format!("Tab · Shift+Tab, 칸 나누기는 {}", layout_keys(&self.keys))),
                                tip("되돌리기", format!("{} · 이름 바꾸기·이동·복사·휴지통 삭제를 마지막 것부터", k(Action::Undo))),
                            ],
                        },
                        Card {
                            header: format!("즐겨찾기 ({})", k(Action::Favorites)),
                            rows: vec![
                                tip("추가", format!("{} · 고른 폴더·파일, 없으면 지금 폴더 · 경로 오른쪽 별 · 파일을 끌어 놓기", k(Action::AddFavorite))),
                                tip("그룹", "그룹 줄을 누르거나 →·←로 펼치고 접어요 · 끌어서 순서를 바꾸거나 그룹에 넣기".into()),
                            ],
                        },
                    ],
                )
            }
            Page::About => (
                "정보",
                vec![Card {
                    header: String::new(),
                    rows: vec![
                        row(format!("Sakadu {}", env!("CARGO_PKG_VERSION")), "가볍고 빠른 Windows 멀티뷰 파일 관리자", Widget::Button("GitHub".into(), Ctl::GitHub, 0)),
                        row("새 버전", "새 버전이 나왔는지 릴리스 페이지에서 확인해요", Widget::Button("릴리스 열기".into(), Ctl::Releases, 0)),
                        row("설정 파일", "%APPDATA%\\Sakadu · 세션·즐겨찾기·단축키", Widget::Button("폴더 열기".into(), Ctl::DataFolder, 0)),
                        row("라이선스", "GPL-3.0-or-later", Widget::None),
                    ],
                }],
            ),
        }
    }

    // ---------- 배치 ----------

    /// 왼쪽 분류의 폭 (좁은 창에서는 아이콘만)
    fn side_w(&self) -> f32 {
        if self.w < 640.0 { SIDE_W_COMPACT } else { SIDE_W }
    }

    fn settings_items(&self) -> (Vec<Item>, f32) {
        let mut items = Vec::new();
        let Some(s) = &self.settings else { return (items, 0.0) };
        let g = &self.gfx;
        let side = self.side_w();
        // 왼쪽 분류
        items.push(Item::Back(R::new(10.0, 12.0, side - 10.0, 44.0)));
        for (i, (page, label, glyph)) in PAGES.iter().enumerate() {
            let y = 60.0 + i as f32 * 38.0;
            items.push(Item::Nav { r: R::new(10.0, y, side - 10.0, y + 34.0), label, glyph: *glyph, on: s.page == *page, hit: SetHit::Nav(*page) });
        }
        // 오른쪽 내용: 창이 좁아도(크기 150% 등) 오른쪽으로 넘치지 않게 여백부터 줄인다
        let avail = self.w - side;
        let margin = if avail > 480.0 { 24.0 } else { 12.0 };
        let w = CONTENT_W.min(avail - 2.0 * margin).max(200.0);
        let l = side + ((avail - w) / 2.0).max(margin);
        let r = l + w;
        let mut y = 32.0 - s.scroll;
        let (title, cards) = self.settings_cards(s);
        items.push(Item::Title(R::new(l + 4.0, y, r, y + 36.0), title.to_string()));
        y += 52.0;
        if let Some(note) = &s.note {
            items.push(Item::Note(R::new(l, y, r, y + 34.0), note.clone()));
            y += 46.0;
        }
        for card in cards {
            if !card.header.is_empty() {
                items.push(Item::Header(R::new(l + 4.0, y, r, y + 26.0), card.header));
                y += 30.0;
            }
            let top = y;
            let mut parts = Vec::new();
            for (i, rw) in card.rows.into_iter().enumerate() {
                let label_h = if rw.desc.is_empty() { 48.0 } else { 62.0 };
                if i > 0 {
                    parts.push(Item::Divider(R::new(l + 16.0, y, r - 16.0, y + 1.0)));
                }
                let cr = r - 16.0;
                let tl = l + 16.0;
                // 조작이 제목 자리를 너무 많이 먹으면 제목 아래 줄로 내린다
                let (ww, wh) = widget_size(g, &rw.widget);
                let stacked = ww > 0.0 && (cr - tl) - ww - 14.0 < LABEL_MIN_W;
                let h = if stacked { label_h + wh + 10.0 } else { label_h };
                let cy = if stacked { y + label_h + wh / 2.0 - 4.0 } else { y + h / 2.0 };
                // 오른쪽 조작부터 놓고 남은 폭을 글자에 준다
                let mut wl = cr;
                match rw.widget {
                    Widget::None => {}
                    Widget::Toggle(on, ctl) => {
                        let tr = R::new(cr - 40.0, cy - 11.0, cr, cy + 11.0);
                        wl = tr.l;
                        parts.push(Item::Toggle { r: tr, on, hit: SetHit::Ctl(ctl, 0) });
                    }
                    Widget::Seg(labels, on, ctl) => {
                        let widths: Vec<f32> = labels.iter().map(|t| g.measure(t, Font::Normal) + 26.0).collect();
                        let mut x = cr - widths.iter().sum::<f32>();
                        wl = x;
                        parts.push(Item::SegFrame(R::new(x, cy - 15.0, cr, cy + 15.0)));
                        for (k, (label, sw)) in labels.into_iter().zip(widths).enumerate() {
                            parts.push(Item::Seg { r: R::new(x, cy - 15.0, x + sw, cy + 15.0), label, on: k == on, hit: SetHit::Ctl(ctl, k) });
                            x += sw;
                        }
                    }
                    Widget::Swatches(colors, on) => {
                        let n = colors.len() as f32;
                        let mut x = cr - n * 24.0 - (n - 1.0) * 10.0;
                        wl = x;
                        for (k, color) in colors.into_iter().enumerate() {
                            parts.push(Item::Swatch { r: R::new(x, cy - 12.0, x + 24.0, cy + 12.0), color, on: k == on, hit: SetHit::Ctl(Ctl::Accent, k) });
                            x += 34.0;
                        }
                    }
                    Widget::Drop(label, ctl) => {
                        let bw = (g.measure(&label, Font::Normal) + 44.0).clamp(110.0, 260.0);
                        let br = R::new(cr - bw, cy - 15.0, cr, cy + 15.0);
                        wl = br.l;
                        parts.push(Item::Button { r: br, label, chevron: true, hit: SetHit::Ctl(ctl, 0) });
                    }
                    Widget::Button(label, ctl, k) => {
                        let bw = g.measure(&label, Font::Normal) + 30.0;
                        let br = R::new(cr - bw, cy - 15.0, cr, cy + 15.0);
                        wl = br.l;
                        parts.push(Item::Button { r: br, label, chevron: false, hit: SetHit::Ctl(ctl, k) });
                    }
                    Widget::Key(label, capturing, ctl) => {
                        let shown = if capturing { "키를 누르세요…".to_string() } else { label.clone() };
                        let bw = (g.measure(&shown, Font::Normal) + 28.0).max(96.0);
                        let br = R::new(cr - bw, cy - 14.0, cr, cy + 14.0);
                        wl = br.l;
                        parts.push(Item::Key { r: br, label: shown, capturing, empty: label == "없음", hit: SetHit::Ctl(ctl, 0) });
                    }
                }
                let tr = if stacked { cr } else { (wl - 14.0).max(tl + 60.0) };
                let (title, desc) = if rw.desc.is_empty() {
                    (R::new(tl, y, tr, y + label_h), R::new(tl, y, tl, y))
                } else {
                    (R::new(tl, y + 10.0, tr, y + 32.0), R::new(tl, y + 32.0, tr, y + 52.0))
                };
                parts.push(Item::Label { title, desc, t: rw.title, d: rw.desc });
                y += h;
            }
            items.push(Item::Card(R::new(l, top, r, y)));
            items.extend(parts);
            y += 22.0;
        }
        (items, y + s.scroll + 20.0)
    }

    pub(super) fn settings_hit(&self, x: f32, y: f32) -> SetHit {
        let (items, _) = self.settings_items();
        let side = self.side_w();
        let content = x >= side;
        for item in items.iter().rev() {
            let (r, hit) = match item {
                Item::Nav { r, hit, .. }
                | Item::Toggle { r, hit, .. }
                | Item::Seg { r, hit, .. }
                | Item::Swatch { r, hit, .. }
                | Item::Button { r, hit, .. }
                | Item::Key { r, hit, .. } => (*r, *hit),
                Item::Back(r) => (*r, SetHit::Back),
                _ => continue,
            };
            // 내용 쪽은 스크롤되어 왼쪽 분류 아래로 들어간 것을 누르지 않게
            let in_side = r.r <= side;
            if r.contains(x, y) && in_side != content {
                return hit;
            }
        }
        SetHit::Panel
    }

    pub(super) fn settings_wheel(&mut self, dy: f32) {
        let (_, total) = self.settings_items();
        let max = (total - self.h).max(0.0);
        if let Some(s) = &mut self.settings {
            s.scroll = (s.scroll + dy).clamp(0.0, max);
        }
        self.invalidate();
    }

    // ---------- 누르기 ----------

    pub(super) fn settings_click(&mut self, hit: SetHit, x: f32, y: f32) {
        let Some(s) = &mut self.settings else { return };
        s.focus = None;
        // 단축키를 받는 중에 다른 곳을 누르면 그만둔다
        if s.capture.is_some() && !matches!(hit, SetHit::Ctl(Ctl::Key(_) | Ctl::Global, _)) {
            s.capture = None;
        }
        match hit {
            SetHit::Back => self.close_settings(),
            SetHit::Nav(page) => {
                s.page = page;
                s.scroll = 0.0;
                s.capture = None;
                s.note = None;
                self.refresh_settings();
            }
            SetHit::Panel => {}
            SetHit::Ctl(ctl, i) => self.settings_ctl(ctl, i, x, y),
        }
        self.invalidate();
    }

    fn settings_ctl(&mut self, ctl: Ctl, i: usize, x: f32, y: f32) {
        match ctl {
            Ctl::Hidden => self.toggle_hidden(),
            Ctl::Preview => self.toggle_preview(),
            Ctl::Theme => {
                self.look.theme = THEMES[i.min(THEMES.len() - 1)].0.to_string();
                self.apply_look(LookPart::Colors);
            }
            Ctl::Accent => {
                self.look.accent = ACCENTS[i.min(ACCENTS.len() - 1)].0.to_string();
                self.apply_look(LookPart::Colors);
            }
            Ctl::Zoom => {
                self.look.zoom = ZOOMS[i.min(ZOOMS.len() - 1)];
                self.apply_look(LookPart::Zoom);
            }
            Ctl::Text => {
                self.look.text = TEXT_LOOKS[i.min(TEXT_LOOKS.len() - 1)].0.to_string();
                self.apply_look(LookPart::Text);
            }
            Ctl::Tray => {
                let on = self.settings.as_ref().is_some_and(|s| s.tray);
                let msg = self.set_tray_option(!on);
                self.refresh_settings();
                self.settings_note(msg);
            }
            Ctl::FolderAdd => {
                let Some(path) = self.settings.as_ref().and_then(|s| s.candidates.get(i).cloned()) else { return };
                let msg = match self.folders.add(&path) {
                    Ok(()) => format!("검색에 넣었어요: {path}"),
                    Err(e) => format!("검색 폴더 목록을 저장하지 못했어요: {e}"),
                };
                self.refresh_settings();
                self.settings_note(msg);
            }
            Ctl::Key(_) | Ctl::Global => {
                if let Some(s) = &mut self.settings {
                    s.capture = if s.capture == Some(ctl) { None } else { Some(ctl) };
                    s.note = None;
                }
            }
            Ctl::KeysReset => {
                self.keys = Keymap::default();
                let saved = self.keys.save();
                self.register_global_hotkey();
                self.settings_note(match saved {
                    Ok(()) => "단축키를 모두 처음대로 되돌렸어요".to_string(),
                    Err(e) => format!("단축키를 저장하지 못했어요: {e}"),
                });
            }
            // 메뉴·확인 창·다른 프로그램을 띄우는 것은 App 빌림 밖에서
            Ctl::NewTab | Ctl::Font | Ctl::DefaultFm | Ctl::Service | Ctl::FolderRemove | Ctl::GitHub | Ctl::Releases | Ctl::DataFolder => {
                let pt = self.to_screen(x, y);
                self.pending.push(Deferred::Settings { ctl, index: i, pt });
            }
        }
    }

    /// 트레이 대기를 켜거나 끈다. 결과 안내 문구를 돌려준다
    pub(super) fn set_tray_option(&mut self, on: bool) -> String {
        if !on {
            crate::tray::disable(crate::tray::RUN_KEY);
            self.set_tray(false);
            return "트레이 대기를 껐어요. 이제 창을 닫으면 Sakadu가 끝나요".to_string();
        }
        // 기본 파일 관리자와 같은 위치에 복사해 두고 그 파일을 로그인할 때 실행한다
        let r = crate::defaultfm::copy_self().and_then(|exe| crate::tray::enable(crate::tray::RUN_KEY, &exe));
        crate::diag::log(|| format!("tray: enable -> {r:?}"));
        match r {
            Ok(()) => {
                self.set_tray(true);
                "이제 창을 닫아도 트레이에서 기다리고, 로그인할 때도 창 없이 시작해요".to_string()
            }
            Err(e) => format!("시작 프로그램에 등록하지 못했어요: {e}"),
        }
    }

    /// 설정 창이 열려 있을 때의 키. 단축키를 받는 중이면 그 키를 새 단축키로
    pub(super) fn settings_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let Some(s) = &self.settings else { return false };
        if let Some(target) = s.capture {
            // 수정 키만 누른 동안은 기다린다
            if matches!(vk, VK_CONTROL | VK_SHIFT | VK_MENU | VK_LWIN | VK_RWIN | VK_LCONTROL | VK_RCONTROL | VK_LSHIFT | VK_RSHIFT | VK_LMENU | VK_RMENU) {
                return true;
            }
            let combo = Combo::now(vk.0);
            let plain = !combo.ctrl && !combo.alt && !combo.shift;
            if vk == VK_ESCAPE && plain {
                self.settings.as_mut().unwrap().capture = None;
            } else if matches!(vk, VK_BACK | VK_DELETE) && plain {
                self.assign_key(target, None);
            } else if !combo.usable() || (target == Ctl::Global && !combo.ctrl && !combo.alt) {
                self.settings_note("Ctrl이나 Alt와 함께 누르거나, F1~F12 같은 기능 키를 써 주세요 (어디서든 검색은 Ctrl·Alt 조합만)");
            } else {
                self.assign_key(target, Some(combo));
            }
            self.invalidate();
            return true;
        }
        // 바꿀 수 있는 단축키(설정·도움말)를 먼저 본다: Ctrl+Tab 같은 조합으로 정해 둘 수도 있다
        if let Some(a @ (Action::Settings | Action::Help)) = self.keys.find(Combo::now(vk.0)) {
            self.run_action(a);
            return true;
        }
        match vk {
            VK_ESCAPE => self.close_settings(),
            VK_RETURN | VK_SPACE => {
                self.settings_press_focus();
            }
            VK_TAB | VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT => {
                self.settings_nav_key(vk);
            }
            // 도움말처럼 긴 쪽을 키보드로 끝까지 읽을 수 있게
            VK_PRIOR => self.settings_wheel(-(self.h - 80.0).max(40.0)),
            VK_NEXT => self.settings_wheel((self.h - 80.0).max(40.0)),
            VK_HOME => self.settings_wheel(-f32::MAX / 2.0),
            VK_END => self.settings_wheel(f32::MAX / 2.0),
            _ => {}
        }
        true
    }

    fn assign_key(&mut self, target: Ctl, combo: Option<Combo>) {
        if let Some(s) = &mut self.settings {
            s.capture = None;
        }
        let note = match target {
            Ctl::Global => {
                let before = self.keys.global;
                self.keys.global = combo;
                match combo {
                    None => {
                        self.register_global_hotkey();
                        None
                    }
                    Some(c) if self.register_global_hotkey() => {
                        // 어디서든 키는 Windows가 먼저 가로채, 같은 키의 Sakadu 안 동작은 키로 부를 수 없게 된다.
                        // 겹친 동작을 알려 주고 그쪽 단축키는 비운다 (Sakadu 안에서 키를 겹쳐 정할 때와 같은 방식)
                        self.keys.find(c).map(|a| {
                            self.keys.set(a, None);
                            format!("{}는 ‘{}’에 쓰던 키예요. 이제 어디서든 검색 창을 띄우고, ‘{1}’ 단축키는 비웠어요", c.label(), crate::keys::name(a))
                        })
                    }
                    Some(c) => {
                        // 다른 프로그램이 이미 쓰는 키: 예전 키로 되돌린다
                        self.keys.global = before;
                        self.register_global_hotkey();
                        Some(format!("{}는 다른 프로그램이 이미 쓰고 있어 등록하지 못했어요", c.label()))
                    }
                }
            }
            // 어디서든 검색 키와 같은 키는 Sakadu 안에 들어오지 않으므로 받지 않는다
            Ctl::Key(_) if combo.is_some_and(|c| self.keys.global == Some(c) || self.hotkey.as_deref() == Some(c.label().as_str())) => {
                Some(format!("{}는 어디서든 검색 창을 띄우는 키라서 여기에 쓸 수 없어요", combo.map(Combo::label).unwrap_or_default()))
            }
            Ctl::Key(a) => self.keys.set(a, combo).map(|other| format!("‘{}’에 쓰던 키라서 그쪽 단축키는 비웠어요", crate::keys::name(other))),
            _ => None,
        };
        if let Err(e) = self.keys.save() {
            self.settings_note(format!("단축키를 저장하지 못했어요: {e}"));
            return;
        }
        if let Some(n) = note {
            self.settings_note(n);
        } else if let Some(s) = &mut self.settings {
            s.note = None;
        }
    }

    // ---------- 그리기 ----------

    pub(super) fn paint_settings(&self) {
        let t = self.theme;
        let g = &self.gfx;
        let hover = self.hover;
        let hot = |h: SetHit| hover == Hit::Set(h);
        let (items, _) = self.settings_items();
        g.fill(R::new(0.0, 0.0, self.w, self.h), t.window);
        let side = self.side_w();
        g.fill(R::new(0.0, 0.0, side, self.h), t.strip);
        g.fill(R::new(side - 1.0, 0.0, side, self.h), t.border);
        // 내용 먼저(스크롤되어 위로 넘어가도 분류를 가리지 않게 잘라서), 분류는 그 위에
        g.clip(R::new(side, 0.0, self.w, self.h));
        for item in &items {
            match item {
                Item::Title(r, text) => g.text(text, *r, Font::Title, t.text),
                Item::Header(r, text) => g.text(text, *r, Font::Small, t.dim),
                Item::Note(r, text) => {
                    g.fill_round(*r, 8.0, t.accent_soft);
                    g.text(text, R::new(r.l + 14.0, r.t, r.r - 10.0, r.b), Font::Normal, t.text);
                }
                Item::Card(r) => {
                    g.fill_round(*r, 10.0, t.pane);
                    g.stroke_round(r.inset(0.5, 0.5), 10.0, t.border, 1.0);
                }
                Item::Divider(r) => g.fill(*r, t.border),
                Item::Label { title, desc, t: tt, d } => {
                    g.text(tt, *title, Font::Normal, t.text);
                    if !d.is_empty() {
                        g.text(d, *desc, Font::Small, t.dim);
                    }
                }
                Item::Toggle { r, on, hit } => {
                    let knob = 8.0;
                    if *on {
                        g.fill_round(*r, 11.0, if hot(*hit) { alpha(t.accent, 0.85) } else { t.accent });
                        g.fill_round(R::new(r.r - 11.0 - knob, r.t + 3.0, r.r - 11.0 + knob, r.b - 3.0), knob, Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 });
                    } else {
                        g.fill_round(*r, 11.0, if hot(*hit) { t.hover } else { t.field });
                        g.stroke_round(r.inset(0.5, 0.5), 11.0, t.thumb, 1.0);
                        g.fill_round(R::new(r.l + 11.0 - knob, r.t + 3.0, r.l + 11.0 + knob, r.b - 3.0), knob, t.dim);
                    }
                }
                Item::SegFrame(r) => {
                    g.fill_round(*r, 7.0, t.pane);
                    g.stroke_round(r.inset(0.5, 0.5), 7.0, t.border, 1.0);
                }
                Item::Seg { r, label, on, hit } => {
                    if *on {
                        g.fill_round(r.inset(2.0, 2.0), 5.0, t.accent_soft);
                    } else if hot(*hit) {
                        g.fill_round(r.inset(2.0, 2.0), 5.0, t.hover);
                    }
                    g.text(label, *r, Font::Hint, if *on { t.accent } else { t.text });
                }
                Item::Swatch { r, color, on, hit } => {
                    if *on || hot(*hit) {
                        g.stroke_round(r.inset(-3.0, -3.0), 15.0, if *on { t.text } else { t.thumb }, 2.0);
                    }
                    g.fill_round(*r, 12.0, *color);
                }
                Item::Button { r, label, chevron, hit } => {
                    g.fill_round(*r, 7.0, if hot(*hit) { t.hover } else { t.pane });
                    g.stroke_round(r.inset(0.5, 0.5), 7.0, t.border, 1.0);
                    if *chevron {
                        g.text(label, R::new(r.l + 12.0, r.t, r.r - 26.0, r.b), Font::Normal, t.text);
                        g.glyph(G_CHEVRON_DOWN, R::new(r.r - 26.0, r.t, r.r - 8.0, r.b), true, t.dim);
                    } else {
                        g.text(label, *r, Font::Hint, t.text);
                    }
                }
                Item::Key { r, label, capturing, empty, hit } => {
                    g.fill_round(*r, 6.0, if hot(*hit) { t.hover } else { t.field });
                    if *capturing {
                        g.stroke_round(r.inset(0.75, 0.75), 6.0, t.accent, 1.5);
                    } else {
                        g.stroke_round(r.inset(0.5, 0.5), 6.0, t.border, 1.0);
                    }
                    g.text(label, *r, Font::Hint, if *capturing { t.accent } else if *empty { t.faint } else { t.text });
                }
                Item::Nav { .. } | Item::Back(_) => {}
            }
        }
        g.unclip();
        // 좁은 창에서는 분류를 아이콘만 (글자는 마우스를 올리지 않아도 제목에 보인다)
        let compact = self.side_w() < SIDE_W;
        for item in &items {
            match item {
                Item::Back(r) => {
                    if hot(SetHit::Back) {
                        g.fill_round(*r, 7.0, t.hover);
                    }
                    if compact {
                        g.glyph(G_BACK, *r, false, t.text);
                    } else {
                        g.glyph(G_BACK, R::new(r.l + 6.0, r.t, r.l + 30.0, r.b), false, t.text);
                        g.text("설정", R::new(r.l + 36.0, r.t, r.r, r.b), Font::Strong, t.text);
                    }
                }
                Item::Nav { r, label, glyph, on, hit } => {
                    if *on {
                        g.fill_round(*r, 7.0, t.sel_inactive);
                    } else if hot(*hit) {
                        g.fill_round(*r, 7.0, t.hover);
                    }
                    // 아이콘은 글자와 어울리는 보통 크기(14px). 작은 크기는 글자 옆에서 흐릿하게 보인다
                    let fg = if *on { t.accent } else { t.dim };
                    if compact {
                        g.glyph(*glyph, *r, false, fg);
                    } else {
                        g.glyph(*glyph, R::new(r.l + 8.0, r.t, r.l + 30.0, r.b), false, fg);
                        g.text(label, R::new(r.l + 38.0, r.t, r.r - 6.0, r.b), if *on { Font::Strong } else { Font::Normal }, t.text);
                    }
                }
                _ => {}
            }
        }
        // 키보드 초점 테두리
        if let Some(r) = self.settings.as_ref().and_then(|s| s.focus).and_then(|f| self.focus_stops().into_iter().find(|(h, _)| *h == f)).map(|(_, r)| r) {
            g.stroke_round(r.inset(-3.0, -3.0), 8.0, t.accent, 2.0);
        }
    }

    // ---------- 키보드로 옮겨 다니기 ----------

    /// Tab으로 멈추는 자리(그리는 순서). 테마·색·크기처럼 묶인 버튼은 한 자리로 치고,
    /// 초점이 그 묶음 안에 있으면 그 칸, 아니면 지금 고른 칸에 멈춘다
    fn focus_stops(&self) -> Vec<(SetHit, R)> {
        let (items, _) = self.settings_items();
        let focus = self.settings.as_ref().and_then(|s| s.focus);
        let group = |item: &Item| match item {
            Item::Seg { r, on, hit: h @ SetHit::Ctl(c, _), .. } | Item::Swatch { r, on, hit: h @ SetHit::Ctl(c, _), .. } => Some((*c, *h, *r, *on)),
            _ => None,
        };
        let mut stops = Vec::new();
        let mut seen: Vec<Ctl> = Vec::new();
        for item in &items {
            match item {
                Item::Back(r) => stops.push((SetHit::Back, *r)),
                Item::Nav { r, hit, .. } | Item::Toggle { r, hit, .. } | Item::Button { r, hit, .. } | Item::Key { r, hit, .. } => stops.push((*hit, *r)),
                _ => {
                    let Some((ctl, ..)) = group(item) else { continue };
                    if seen.contains(&ctl) {
                        continue;
                    }
                    seen.push(ctl);
                    let members: Vec<(SetHit, R, bool)> = items.iter().filter_map(group).filter(|(c, ..)| *c == ctl).map(|(_, h, r, on)| (h, r, on)).collect();
                    let pick = members
                        .iter()
                        .find(|(h, ..)| Some(*h) == focus)
                        .or_else(|| members.iter().find(|(.., on)| *on))
                        .or(members.first());
                    if let Some((h, r, _)) = pick {
                        stops.push((*h, *r));
                    }
                }
            }
        }
        // 누를 것이 없는 쪽(도움말)은 내용 전체를 한 자리로 둔다. 여기서 ↑↓로 내용을 읽어 내려간다
        let side = self.side_w();
        if !stops.iter().any(|(_, r)| r.r > side) {
            stops.push((SetHit::Panel, R::new(side + 12.0, 16.0, self.w - 12.0, self.h - 16.0)));
        }
        stops
    }

    /// 방향키·Tab 처리. 처리했으면 true
    fn settings_nav_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let shift = key_down(VK_SHIFT);
        let stops = self.focus_stops();
        let side = self.side_w();
        let Some(s) = &self.settings else { return false };
        let page = s.page;
        let at = s.focus.and_then(|f| stops.iter().position(|(h, _)| *h == f));
        let in_side = |i: usize| stops[i].1.r <= side;
        let first_content = stops.iter().position(|(_, r)| r.r > side);
        let next = match (vk, at) {
            (VK_TAB, None) => if shift { stops.len().checked_sub(1) } else { first_content },
            (VK_TAB, Some(i)) => Some(if shift { (i + stops.len() - 1) % stops.len() } else { (i + 1) % stops.len() }),
            // 분류에서 위아래는 쪽을 바로 바꾼다
            (VK_UP | VK_DOWN, None) => stops.iter().position(|(h, _)| *h == SetHit::Nav(page)),
            (VK_UP | VK_DOWN, Some(i)) if matches!(stops[i].0, SetHit::Nav(_)) => {
                let k = PAGES.iter().position(|(p, ..)| *p == page).unwrap_or(0);
                let k = if vk == VK_UP { k.saturating_sub(1) } else { (k + 1).min(PAGES.len() - 1) };
                self.settings_click(SetHit::Nav(PAGES[k].0), 0.0, 0.0);
                if let Some(s) = &mut self.settings {
                    s.focus = Some(SetHit::Nav(PAGES[k].0));
                }
                self.invalidate();
                return true;
            }
            (VK_UP | VK_DOWN, Some(i)) if stops[i].0 == SetHit::Panel => {
                self.settings_wheel(if vk == VK_UP { -48.0 } else { 48.0 });
                return true;
            }
            (VK_UP, Some(i)) => (i > 0 && !in_side(i - 1)).then(|| i - 1).or(Some(i)),
            (VK_DOWN, Some(i)) => Some((i + 1).min(stops.len() - 1)),
            // 묶음 안에서는 옆 칸으로, 내용에서 ←는 분류로, 분류에서 →는 내용으로
            (VK_LEFT | VK_RIGHT, Some(i)) if matches!(stops[i].0, SetHit::Ctl(Ctl::Theme | Ctl::Accent | Ctl::Zoom | Ctl::Text, _)) => {
                let SetHit::Ctl(ctl, k) = stops[i].0 else { return true };
                let n = match ctl {
                    Ctl::Theme => THEMES.len(),
                    Ctl::Accent => ACCENTS.len(),
                    Ctl::Text => TEXT_LOOKS.len(),
                    _ => ZOOMS.len(),
                };
                let k = if vk == VK_LEFT { k.checked_sub(1) } else { (k + 1 < n).then_some(k + 1) };
                match k {
                    Some(k) => {
                        if let Some(s) = &mut self.settings {
                            s.focus = Some(SetHit::Ctl(ctl, k));
                        }
                        self.invalidate();
                        return true;
                    }
                    None if vk == VK_LEFT => stops.iter().position(|(h, _)| *h == SetHit::Nav(page)),
                    None => Some(i),
                }
            }
            (VK_LEFT, _) => stops.iter().position(|(h, _)| *h == SetHit::Nav(page)),
            (VK_RIGHT, Some(i)) if in_side(i) => first_content,
            (VK_RIGHT, _) => at,
            _ => return false,
        };
        if let Some((hit, r)) = next.and_then(|i| stops.get(i).copied()) {
            if let Some(s) = &mut self.settings {
                s.focus = Some(hit);
            }
            if r.r > side {
                self.settings_scroll_into_view(r);
            }
        }
        self.invalidate();
        true
    }

    fn settings_scroll_into_view(&mut self, r: R) {
        let (_, total) = self.settings_items();
        let max = (total - self.h).max(0.0);
        let h = self.h;
        if let Some(s) = &mut self.settings {
            if r.t < 16.0 {
                s.scroll = (s.scroll - (16.0 - r.t)).clamp(0.0, max);
            } else if r.b > h - 16.0 {
                s.scroll = (s.scroll + (r.b - (h - 16.0))).clamp(0.0, max);
            }
        }
    }

    /// Enter·Space: 초점이 있는 조작을 누른다
    fn settings_press_focus(&mut self) -> bool {
        let Some(f) = self.settings.as_ref().and_then(|s| s.focus) else { return false };
        let Some((_, r)) = self.focus_stops().into_iter().find(|(h, _)| *h == f) else { return false };
        let (x, y) = ((r.l + r.r) / 2.0, r.b);
        match f {
            SetHit::Ctl(ctl, i) => {
                if let Some(s) = &mut self.settings {
                    s.capture = None;
                }
                self.settings_ctl(ctl, i, x, y);
            }
            other => self.settings_click(other, x, y),
        }
        // 누른 뒤에도 초점은 그 자리에 (쪽을 바꿨으면 그 쪽 분류에)
        if let Some(s) = &mut self.settings {
            s.focus = Some(match f {
                SetHit::Nav(_) => SetHit::Nav(s.page),
                other => other,
            });
        }
        self.invalidate();
        true
    }
}

/// 칸 나누기 단축키들. 처음 그대로면 "Ctrl+1~5"로 줄인다
fn layout_keys(keys: &Keymap) -> String {
    let all: Vec<String> =
        [Action::Layout1, Action::Layout2, Action::Layout3, Action::Layout4, Action::Layout5].iter().map(|a| keys.get(*a).map(Combo::label).unwrap_or_else(|| "없음".into())).collect();
    if all.iter().enumerate().all(|(i, l)| *l == format!("Ctrl+{}", i + 1)) { "Ctrl+1~5".to_string() } else { all.join(" · ") }
}

/// 줄 오른쪽 조작이 차지하는 (폭, 높이). settings_items의 배치와 같아야 한다
fn widget_size(g: &Gfx, w: &Widget) -> (f32, f32) {
    match w {
        Widget::None => (0.0, 0.0),
        Widget::Toggle(..) => (40.0, 22.0),
        Widget::Seg(labels, ..) => (labels.iter().map(|t| g.measure(t, Font::Normal) + 26.0).sum(), 30.0),
        Widget::Swatches(colors, _) => {
            let n = colors.len() as f32;
            (n * 24.0 + (n - 1.0).max(0.0) * 10.0, 24.0)
        }
        Widget::Drop(label, _) => ((g.measure(label, Font::Normal) + 44.0).clamp(110.0, 260.0), 30.0),
        Widget::Button(label, ..) => (g.measure(label, Font::Normal) + 30.0, 30.0),
        Widget::Key(label, capturing, _) => {
            let shown = if *capturing { "키를 누르세요…" } else { label.as_str() };
            ((g.measure(shown, Font::Normal) + 28.0).max(96.0), 28.0)
        }
    }
}

// ---------- 메뉴·확인 창·다른 프로그램 (App 빌림 밖) ----------

pub(super) fn run_deferred(hwnd: HWND, ctl: Ctl, index: usize, pt: POINT) {
    match ctl {
        Ctl::NewTab => new_tab_menu(hwnd, pt),
        Ctl::Font => font_menu(hwnd, pt),
        Ctl::DefaultFm => toggle_default_fm(hwnd),
        Ctl::Service => {
            // 서비스 설치·제거는 관리자 권한이 필요하므로 UAC를 거쳐 따로 실행한다
            let installed = crate::service::is_installed();
            if let Ok(exe) = std::env::current_exe() {
                let arg = if installed { w!("--uninstall-service") } else { w!("--install-service") };
                unsafe {
                    windows::Win32::UI::Shell::ShellExecuteW(Some(hwnd), w!("runas"), &HSTRING::from(exe.as_os_str()), arg, PCWSTR::null(), SW_SHOWNORMAL);
                }
            }
        }
        Ctl::FolderRemove => {
            let Some((folders, path)) = with_app(|a| a.folders.list().get(index).map(|(p, _)| (a.folders.clone(), p.clone()))).flatten() else { return };
            remove_search_folder(hwnd, &folders, &path);
        }
        Ctl::GitHub => open_url(hwnd, "https://github.com/noddish/sakadu"),
        Ctl::Releases => open_url(hwnd, "https://github.com/noddish/sakadu/releases/latest"),
        Ctl::DataFolder => {
            if let Some(dir) = std::env::var_os("APPDATA").map(|d| std::path::PathBuf::from(d).join("Sakadu")) {
                let _ = std::fs::create_dir_all(&dir);
                open_url(hwnd, &dir.to_string_lossy());
            }
        }
        _ => {}
    }
    with_app(|a| a.refresh_settings());
}

fn open_url(hwnd: HWND, target: &str) {
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(Some(hwnd), w!("open"), &HSTRING::from(target), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// 항목들을 체크 표시와 함께 띄우고 고른 번호(1부터, 없으면 0)
fn pick_menu(hwnd: HWND, pt: POINT, items: &[(String, bool)]) -> usize {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        for (i, (label, checked)) in items.iter().enumerate() {
            let flags = if label.is_empty() { MF_SEPARATOR } else { MF_STRING | if *checked { MF_CHECKED } else { MF_UNCHECKED } };
            let _ = AppendMenuW(menu, flags, i + 1, &HSTRING::from(label.as_str()));
        }
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        cmd
    }
}

fn new_tab_menu(hwnd: HWND, pt: POINT) {
    let Some((home, dir)) = with_app(|a| (a.new_tab_home.clone(), a.panes[a.active].tab().loc.dir().map(str::to_string))) else { return };
    let custom = Some(home.as_str()).filter(|h| *h != "::computer" && *h != session::NEW_TAB_SAME && !h.is_empty()).map(str::to_string);
    let mut items = vec![("내 PC".to_string(), custom.is_none() && home != session::NEW_TAB_SAME), ("지금 탭과 같은 폴더".to_string(), home == session::NEW_TAB_SAME)];
    if let Some(c) = &custom {
        items.push((c.clone(), true));
    }
    if let Some(d) = &dir {
        items.push((String::new(), false));
        items.push((format!("지금 폴더로 정하기 ({d})"), false));
    }
    let cmd = pick_menu(hwnd, pt, &items);
    // 정해 둔 폴더 줄은 이미 고른 것이라 바꿀 것이 없다
    let home = match cmd {
        1 => "::computer".to_string(),
        2 => session::NEW_TAB_SAME.to_string(),
        n if n == items.len() && dir.is_some() => dir.unwrap_or_default(),
        _ => return,
    };
    with_app(|a| {
        a.new_tab_home = home;
        a.save_session();
        a.invalidate();
    });
}

/// 자주 쓰는 한글 글꼴 중 설치된 것 + 설치된 글꼴 전체 하위 메뉴
fn font_menu(hwnd: HWND, pt: POINT) {
    const COMMON: [&str; 9] = ["맑은 고딕", "Pretendard", "Pretendard Variable", "SUIT", "Noto Sans KR", "나눔고딕", "나눔스퀘어", "Spoqa Han Sans Neo", "IBM Plex Sans KR"];
    let Some((current, all)) = with_app(|a| (a.look.font.clone(), a.gfx.installed_fonts())) else { return };
    let common: Vec<&str> = COMMON.iter().copied().filter(|f| all.iter().any(|a| a == f)).collect();
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let check = |on: bool| MF_STRING | if on { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, check(current.is_empty()), 1, w!("기본 (Segoe UI)"));
        for (i, f) in common.iter().enumerate() {
            let _ = AppendMenuW(menu, check(current == *f), 100 + i, &HSTRING::from(*f));
        }
        if let Ok(sub) = CreatePopupMenu() {
            for (i, f) in all.iter().enumerate() {
                let _ = AppendMenuW(sub, check(current == *f), 1000 + i, &HSTRING::from(f.as_str()));
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("설치된 글꼴 전체"));
        }
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        cmd
    };
    let font = match cmd {
        1 => String::new(),
        100..=999 => match common.get(cmd - 100) {
            Some(f) => f.to_string(),
            None => return,
        },
        1000.. => match all.get(cmd - 1000) {
            Some(f) => f.clone(),
            None => return,
        },
        _ => return,
    };
    with_app(|a| {
        a.look.font = font;
        a.apply_look(LookPart::Font);
    });
}

fn toggle_default_fm(hwnd: HWND) {
    use crate::defaultfm;
    if defaultfm::is_registered(defaultfm::CLASSES) {
        defaultfm::unregister(defaultfm::CLASSES);
        with_app(|a| a.settings_note("기본 파일 관리자 설정을 해제했어요. 폴더는 다시 탐색기로 열려요"));
        return;
    }
    let text = "폴더·드라이브를 열 때와 Win+E를 Sakadu로 엽니다.\n\
                바탕 화면·작업 표시줄은 그대로 탐색기가 맡고, 제어판·휴지통 같은 특수 폴더도 탐색기로 열립니다.\n\
                현재 사용자에게만 적용되며, 설정에서 언제든 되돌릴 수 있습니다.\n\
                Win+E는 탐색기가 이전 설정을 기억하고 있어서, 다시 로그인하거나 PC를 다시 시작한 뒤부터 바뀔 수 있습니다.\n\n계속할까요?";
    if unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("Sakadu"), MB_YESNO | MB_ICONQUESTION) } != IDYES {
        return;
    }
    let r = defaultfm::copy_self().and_then(|exe| defaultfm::register(defaultfm::CLASSES, &exe));
    crate::diag::log(|| format!("default file manager: register -> {r:?}, folder opens with {:?}", defaultfm::effective_command("Directory")));
    let msg = match r {
        Ok(()) => "이제 폴더가 Sakadu로 열려요. Win+E는 다시 로그인한 뒤부터 바뀔 수 있어요".to_string(),
        Err(e) => format!("기본 파일 관리자로 등록하지 못했어요: {e}"),
    };
    with_app(|a| a.settings_note(msg));
}
