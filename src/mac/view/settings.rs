//! 설정 창: 앱 메뉴의 '설정…'(⌘,)으로 여는, 본 창과 따로 뜨는 창. 맥 기본 조작(스위치·단추·팝업)을 시스템 설정처럼
//! 묶음 카드 안에 줄로 놓는다. 줄에는 이름만 쓴다. 내용이 창보다 길면 스크롤한다.
//! 묶음: 일반(숨김 항목, 자동 미리 보기, 모양), 단축키(동작마다 키 단추, 기본으로), 검색, 도움말(바꾸지 않는 기본 키), 정보.
//! 값은 App이 갖고(App::prefs, keymap, set_…), 이 창은 그것을 보이고 바꾼다. 본 창에서 바뀐 값(⌘⇧., 눈 단추, 훑기 상태)은
//! 입력과 뒷줄 결과를 처리할 때마다 refresh로 따라온다.
//! 단축키 단추를 누르면 다음에 누르는 키를 그 동작의 키로 받는다 (Esc는 그만두기, ⌫는 비우기)

use super::*;
use crate::app::{Prefs, fixed_key, mac_runs};
use objc2_app_kit::{NSAccessibility, NSAppearance, NSAppearanceCustomization, NSBox, NSBoxType, NSButton, NSPopUpButton, NSScrollView, NSSwitch, NSTitlePosition};
use sakadu::keys::{ACTIONS, Action, name};

/// 창 안쪽 폭, 둘레 여백, 묶음 제목 높이, 묶음 사이, 줄 높이(단축키·도움말은 낮게), 카드 안쪽 여백, 창 내용의 가장 큰 높이
const W: f64 = 460.0;
const MARGIN: f64 = 20.0;
const TITLE_H: f64 = 18.0;
const GAP: f64 = 18.0;
const LINE_H: f64 = 36.0;
const SHORT_H: f64 = 30.0;
const INSET: f64 = 14.0;
const MAX_H: f64 = 640.0;
const GITHUB: &str = "https://github.com/noddish/sakadu";
/// Esc, 지우기(⌫)
const ESC: u16 = 0x35;
const BACKSPACE: u16 = 0x33;
/// 모양 팝업의 줄 (보이는 이름, 저장하는 값)
const THEMES: [(&str, &str); 3] = [("시스템 설정", "system"), ("밝게", "light"), ("어둡게", "dark")];
/// 도움말: 바꾸지 않는 기본 키 (Finder와 같다)
const HELP: [(&str, &str); 11] = [
    ("뒤로 · 앞으로", "⌘[   ⌘]"),
    ("위 폴더", "⌘↑"),
    ("열기", "⌘↓   ⌘O"),
    ("폴더로 이동", "⇧⌘G"),
    ("휴지통으로 보내기", "⌘⌫"),
    ("복제", "⌘D"),
    ("복사하기 · 붙여넣기", "⌘C   ⌘V"),
    ("붙여넣는 대신 옮기기", "⌥⌘V"),
    ("훑어보기", "스페이스   ⌘Y"),
    ("다음 칸 · 다음 탭", "Tab   ⌃Tab"),
    ("모두 선택", "⌘A"),
];

thread_local! {
    /// 설정 창과 그 내용 (처음 열 때 만들고, 닫아도 두었다가 다시 연다)
    static SETTINGS: RefCell<Option<(Retained<NSWindow>, Retained<SettingsPane>)>> = const { RefCell::new(None) };
}

/// 줄의 조작 (조작의 tag로 찾는다)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctl {
    Hidden,
    Preview,
    Theme,
    /// 단축키 표(ACTIONS)의 i번째 동작의 키 단추
    Key(usize),
    KeysReset,
    Search,
    ICloud,
    Allow,
    /// 도움말 줄 (조작 없이 키 글만)
    Help(usize),
    GitHub,
    Folder,
}

impl Ctl {
    const FIXED: [Ctl; 9] = [Ctl::Hidden, Ctl::Preview, Ctl::Theme, Ctl::KeysReset, Ctl::Search, Ctl::ICloud, Ctl::Allow, Ctl::GitHub, Ctl::Folder];

    fn tag(self) -> NSInteger {
        match self {
            Ctl::Hidden => 1,
            Ctl::Preview => 2,
            Ctl::Search => 3,
            Ctl::ICloud => 4,
            Ctl::Allow => 5,
            Ctl::GitHub => 6,
            Ctl::Folder => 7,
            Ctl::Theme => 8,
            Ctl::KeysReset => 9,
            Ctl::Key(i) => 100 + i as NSInteger,
            Ctl::Help(i) => 200 + i as NSInteger,
        }
    }

    fn from_tag(tag: NSInteger) -> Option<Ctl> {
        match tag {
            100..=199 => Some(Ctl::Key((tag - 100) as usize)),
            _ => Ctl::FIXED.into_iter().find(|c| c.tag() == tag),
        }
    }

    /// 줄 높이: 단축키와 도움말은 줄이 많아 낮게
    fn height(self) -> f64 {
        if matches!(self, Ctl::Key(_) | Ctl::Help(_)) { SHORT_H } else { LINE_H }
    }
}

/// 줄 하나: 몇 번째 묶음인지, 이름, 오른쪽 조작, 위 줄과 가르는 선
struct Line {
    group: usize,
    ctl: Ctl,
    label: Retained<NSTextField>,
    control: Retained<NSControl>,
    divider: Retained<NSBox>,
}

struct Parts {
    titles: Vec<Retained<NSTextField>>,
    cards: Vec<Retained<NSBox>>,
    lines: Vec<Line>,
    switches: Vec<(Ctl, Retained<NSSwitch>)>,
    /// 단축키 단추 (ACTIONS의 번호, 단추)
    keys: Vec<(usize, Retained<NSButton>)>,
    theme: Retained<NSPopUpButton>,
    /// '이름으로 찾기' 옆의 '훑는 중'
    crawling: Retained<NSTextField>,
    /// 단축키를 바꾼 결과 (다른 동작에서 빼 온 키, 쓸 수 없는 키). '기본으로' 줄의 이름 자리에 보인다
    keys_note: Retained<NSTextField>,
}

#[derive(Default)]
pub struct PaneIvars {
    parts: OnceCell<Parts>,
    /// 지난번에 놓을 때 '읽지 못한 폴더' 줄을 보였는지 (바뀌면 다시 놓고 창 높이를 맞춘다)
    denied: std::cell::Cell<Option<bool>>,
    /// 키를 받는 중인 단축키 단추 (ACTIONS의 번호)
    capture: std::cell::Cell<Option<usize>>,
}

define_class!(
    /// 설정 창의 내용. 위아래를 뒤집어 쓰고(왼쪽 위가 0), 조작들이 보내는 동작을 받고, Esc와 ⌘W로 창을 닫는다.
    /// 단축키 단추를 누른 뒤에는 다음 키를 그 동작의 키로 받는다
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SakaduSettingsPane"]
    #[ivars = PaneIvars]
    pub struct SettingsPane;

    impl SettingsPane {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, e: &NSEvent) {
            if self.ivars().capture.get().is_some() {
                self.capture_key(e);
            } else if e.keyCode() == ESC {
                close();
            } else {
                let _: () = unsafe { msg_send![super(self), keyDown: e] };
            }
        }

        /// 스위치에 키보드가 있을 때의 Esc
        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: Option<&AnyObject>) {
            close();
        }

        /// 단축키를 받는 중이면 ⌘ 조합도 메뉴보다 먼저 여기서 받는다. 아니면 ⌘W: 이 창을 닫는다 (본 창의 ⌘W는 탭 닫기라 메뉴에 두지 않았다)
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, e: &NSEvent) -> bool {
            let key = self.window().is_some_and(|w| w.isKeyWindow());
            let flags = e.modifierFlags() & NSEventModifierFlags::DeviceIndependentFlagsMask;
            if key && self.ivars().capture.get().is_some() {
                self.capture_key(e);
                true
            } else if key && flags == NSEventModifierFlags::Command && e.charactersIgnoringModifiers().is_some_and(|c| c.to_string() == "w") {
                close();
                true
            } else {
                unsafe { msg_send![super(self), performKeyEquivalent: e] }
            }
        }

        /// 스위치를 바꿨다
        #[unsafe(method(toggle:))]
        fn toggle(&self, sender: &NSSwitch) {
            let on = sender.state() == NSControlStateValueOn;
            match Ctl::from_tag(sender.tag()) {
                Some(Ctl::Hidden) => with_app(|a| a.set_hidden(on)),
                Some(Ctl::Preview) => with_app(|a| a.set_auto_preview(on)),
                Some(Ctl::Search) => with_app(|a| a.set_find_on(on)),
                Some(Ctl::ICloud) => with_app(|a| a.set_find_icloud(on)),
                _ => None,
            };
            after_change();
        }

        /// 모양을 골랐다
        #[unsafe(method(theme:))]
        fn theme(&self, sender: &NSPopUpButton) {
            let Some((_, value)) = usize::try_from(sender.indexOfSelectedItem()).ok().and_then(|i| THEMES.get(i)) else { return };
            with_app(|a| a.set_theme_pref(value));
            apply_theme(self.mtm());
            after_change();
        }

        /// 단추를 눌렀다
        #[unsafe(method(press:))]
        fn press(&self, sender: &NSButton) {
            match Ctl::from_tag(sender.tag()) {
                Some(Ctl::Key(i)) => {
                    // 같은 단추를 다시 누르면 그만둔다
                    let again = self.ivars().capture.get() == Some(i);
                    self.ivars().capture.set((!again).then_some(i));
                    self.note("");
                    if let Some(w) = self.window() {
                        w.makeFirstResponder(Some(self));
                    }
                    self.show_keys();
                }
                Some(Ctl::KeysReset) => {
                    self.ivars().capture.set(None);
                    with_app(App::reset_keys);
                    self.note("모든 단축키를 기본으로 되돌렸어요");
                    self.keys_changed();
                }
                Some(Ctl::Allow) => {
                    with_app(App::ask_folder_access);
                    after_change();
                }
                Some(Ctl::GitHub) => open_link(GITHUB),
                Some(Ctl::Folder) => reveal_config(),
                _ => {}
            }
        }
    }
);

impl SettingsPane {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PaneIvars::default());
        let pane: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(W, 300.0))] };
        // 카드를 먼저 붙인다: 나중에 붙인 줄의 글과 조작이 그 위에 그려진다
        let card = || {
            let b = NSBox::new(mtm);
            b.setBoxType(NSBoxType::Custom);
            b.setTitlePosition(NSTitlePosition::NoTitle);
            b.setCornerRadius(10.0);
            b.setBorderWidth(1.0);
            b.setBorderColor(&NSColor::separatorColor());
            b.setFillColor(&NSColor::controlBackgroundColor());
            pane.addSubview(&b);
            b
        };
        let groups = ["일반", "단축키", "검색", "도움말", "정보"];
        let cards: Vec<Retained<NSBox>> = groups.iter().map(|_| card()).collect();
        let label = |text: &str| {
            let l = NSTextField::labelWithString(&NSString::from_str(text), mtm);
            pane.addSubview(&l);
            l
        };
        let titles: Vec<Retained<NSTextField>> = groups
            .iter()
            .map(|t| {
                let l = label(t);
                l.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
                l
            })
            .collect();
        let quiet = |l: &NSTextField| {
            l.setTextColor(Some(&NSColor::secondaryLabelColor()));
            l.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        };
        let crawling = label("훑는 중");
        quiet(&crawling);
        crawling.setHidden(true);
        let target: &AnyObject = &pane;
        let button = |title: &str| {
            // SAFETY: 이 뷰가 press:를 받는다 (define_class에 있다)
            unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(target), Some(sel!(press:)), mtm) }
        };
        let theme = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
        theme.addItemsWithTitles(&NSArray::from_retained_slice(&THEMES.map(|t| NSString::from_str(t.0))));
        // SAFETY: 이 뷰가 theme:을 받는다 (define_class에 있다)
        unsafe {
            theme.setTarget(Some(target));
            theme.setAction(Some(sel!(theme:)));
        }
        theme.setAccessibilityLabel(Some(&NSString::from_str("모양")));
        let mut switches = Vec::new();
        let mut keys = Vec::new();
        let mut lines = Vec::new();
        let version = format!("Sakadu {}", env!("CARGO_PKG_VERSION"));
        // (묶음, 조작, 이름). 조작은 아래에서 만든다
        let mut rows: Vec<(usize, Ctl, String)> = vec![(0, Ctl::Hidden, "숨김 항목 보기".into()), (0, Ctl::Preview, "자동 미리 보기".into()), (0, Ctl::Theme, "모양".into())];
        for (i, (a, ..)) in ACTIONS.iter().enumerate().filter(|(_, x)| mac_runs(x.0)) {
            rows.push((1, Ctl::Key(i), name(*a).to_string()));
        }
        rows.push((1, Ctl::KeysReset, String::new()));
        rows.extend([(2, Ctl::Search, "이름으로 찾기".into()), (2, Ctl::ICloud, "iCloud Drive도 찾기".into()), (2, Ctl::Allow, "읽지 못한 폴더가 있어요".into())]);
        for (i, (what, _)) in HELP.iter().enumerate() {
            rows.push((3, Ctl::Help(i), what.to_string()));
        }
        rows.extend([(4, Ctl::GitHub, version), (4, Ctl::Folder, "설정 폴더".into())]);
        let mut keys_note = None;
        for (group, ctl, text) in rows {
            let divider = NSBox::new(mtm);
            divider.setBoxType(NSBoxType::Separator);
            pane.addSubview(&divider);
            let name_field = label(&text);
            let control: Retained<NSControl> = match ctl {
                Ctl::Hidden | Ctl::Preview | Ctl::Search | Ctl::ICloud => {
                    let s = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), NSRect::ZERO);
                    // SAFETY: 이 뷰가 toggle:을 받는다 (define_class에 있다)
                    unsafe {
                        s.setTarget(Some(target));
                        s.setAction(Some(sel!(toggle:)));
                    }
                    // 스위치는 옆의 글을 이름으로 읽지 않으므로 줄 이름을 붙여 둔다 (VoiceOver)
                    s.setAccessibilityLabel(Some(&NSString::from_str(&text)));
                    switches.push((ctl, s.clone()));
                    s.into_super()
                }
                Ctl::Theme => theme.clone().into_super().into_super(),
                Ctl::Key(i) => {
                    let b = button("");
                    b.setAccessibilityLabel(Some(&NSString::from_str(&format!("{text} 단축키"))));
                    keys.push((i, b.clone()));
                    b.into_super()
                }
                Ctl::KeysReset => {
                    quiet(&name_field);
                    keys_note = Some(name_field.clone());
                    button("기본으로").into_super()
                }
                Ctl::Allow => button("설정에서 허용").into_super(),
                Ctl::GitHub => button("GitHub").into_super(),
                Ctl::Folder => button("Finder에서 보기").into_super(),
                Ctl::Help(i) => {
                    let k = NSTextField::labelWithString(&NSString::from_str(HELP[i].1), mtm);
                    k.setTextColor(Some(&NSColor::secondaryLabelColor()));
                    k.into_super()
                }
            };
            control.setTag(ctl.tag());
            pane.addSubview(&control);
            lines.push(Line { group, ctl, label: name_field, control, divider });
        }
        let keys_note = keys_note.expect("'기본으로' 줄은 늘 있다");
        let _ = pane.ivars().parts.set(Parts { titles, cards, lines, switches, keys, theme, crawling, keys_note });
        pane.show_keys();
        pane
    }

    fn parts(&self) -> &Parts {
        self.ivars().parts.get().expect("설정 창의 줄은 만들 때 채운다")
    }

    /// 줄을 놓는다. 읽지 못한 폴더 줄은 denied일 때만 보인다. 내용의 높이를 돌려준다
    fn layout(&self, denied: bool) -> f64 {
        let p = self.parts();
        let (l, r) = (MARGIN, W - MARGIN);
        let rect = |x: f64, y: f64, w: f64, h: f64| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        let mut y = MARGIN;
        for (g, card) in p.cards.iter().enumerate() {
            if g > 0 {
                y += GAP;
            }
            p.titles[g].setFrame(rect(l + 4.0, y, r - l - 4.0, TITLE_H));
            y += TITLE_H + 6.0;
            let top = y;
            let mut first = true;
            for line in p.lines.iter().filter(|line| line.group == g) {
                let shown = line.ctl != Ctl::Allow || denied;
                line.label.setHidden(!shown);
                line.control.setHidden(!shown);
                line.divider.setHidden(!shown || first);
                if !shown {
                    continue;
                }
                let h = line.ctl.height();
                line.divider.setFrame(rect(l + INSET, y, r - l - 2.0 * INSET, 1.0));
                let c = line.control.intrinsicContentSize();
                // 단축키 단추는 키 글이 바뀌어도 같은 폭으로
                let cw = if matches!(line.ctl, Ctl::Key(_)) { c.width.max(120.0) } else { c.width };
                let cx = r - INSET - cw;
                line.control.setFrame(rect(cx, y + (h - c.height) / 2.0, cw, c.height));
                let t = line.label.intrinsicContentSize();
                let tw = t.width.min(cx - 8.0 - (l + INSET));
                line.label.setFrame(rect(l + INSET, y + (h - t.height) / 2.0, tw, t.height));
                if line.ctl == Ctl::Search {
                    let s = p.crawling.intrinsicContentSize();
                    p.crawling.setFrame(rect(l + INSET + tw + 8.0, y + (h - s.height) / 2.0, s.width, s.height));
                }
                y += h;
                first = false;
            }
            card.setFrame(rect(l, top, r - l, y - top));
        }
        y + MARGIN
    }

    /// App의 값으로 스위치·팝업·단축키 단추를 맞춘다. 읽지 못한 폴더 줄이 생기거나 없어지면 다시 놓고 창 높이를 맞춘다
    fn show(&self, prefs: &Prefs, window: &NSWindow) {
        let p = self.parts();
        for (ctl, s) in &p.switches {
            let on = match ctl {
                Ctl::Hidden => prefs.hidden,
                Ctl::Preview => prefs.auto_preview,
                Ctl::Search => prefs.search_on,
                Ctl::ICloud => prefs.icloud,
                _ => false,
            };
            let state = if on { NSControlStateValueOn } else { NSControlStateValueOff };
            if s.state() != state {
                s.setState(state);
            }
        }
        let theme = THEMES.iter().position(|t| t.1 == prefs.theme).unwrap_or(0) as NSInteger;
        if p.theme.indexOfSelectedItem() != theme {
            p.theme.selectItemAtIndex(theme);
        }
        p.crawling.setHidden(!prefs.crawling);
        self.show_keys();
        if self.ivars().denied.get() != Some(prefs.denied) {
            self.ivars().denied.set(Some(prefs.denied));
            let h = self.layout(prefs.denied);
            self.setFrameSize(NSSize::new(W, h));
            fit_height(window, h.min(MAX_H));
        }
    }

    /// 단축키 단추의 글: 지금 키(맥 기호), 없으면 '없음', 키를 받는 중이면 '키를 누르세요'
    fn show_keys(&self) {
        let keys = with_app(|a| a.keymap().clone()).unwrap_or_default();
        let capture = self.ivars().capture.get();
        for (i, b) in &self.parts().keys {
            let text = if capture == Some(*i) { "키를 누르세요".to_string() } else { keys.get(ACTIONS[*i].0).map_or_else(|| "없음".to_string(), |c| c.glyphs()) };
            if b.title().to_string() != text {
                b.setTitle(&NSString::from_str(&text));
            }
        }
    }

    /// '기본으로' 줄의 이름 자리에 알림
    fn note(&self, text: &str) {
        let p = self.parts();
        p.keys_note.setStringValue(&NSString::from_str(text));
        let s = p.keys_note.intrinsicContentSize();
        let f = p.keys_note.frame();
        p.keys_note.setFrameSize(NSSize::new(s.width.min(W - 2.0 * MARGIN - 2.0 * INSET - 100.0), f.size.height.max(s.height)));
    }

    /// 단축키를 받는 중에 누른 키: Esc는 그만두기, ⌫는 비우기, 그 밖은 그 동작의 키로 (쓸 수 없는 키면 알린다)
    fn capture_key(&self, e: &NSEvent) {
        let Some(i) = self.ivars().capture.take() else { return };
        let a = ACTIONS[i].0;
        let m = mods(e);
        let plain = !(m.cmd || m.ctrl || m.alt || m.shift);
        let note = match e.keyCode() {
            ESC if plain => String::new(),
            BACKSPACE if plain => {
                with_app(|app| app.rebind(a, None));
                format!("‘{}’의 단축키를 비웠어요", name(a))
            }
            code => match key_of(code).and_then(|k| k.combo(m)) {
                Some(c) if fixed_key(c) => format!("{}는 기본 조작에 쓰는 키예요 · 도움말을 보세요", c.glyphs()),
                Some(c) if c.usable() => match with_app(|app| app.rebind(a, Some(c))).flatten() {
                    Some(other) => format!("‘{}’에 쓰던 키라서 그쪽 단축키는 비웠어요", name(other)),
                    None => String::new(),
                },
                _ => "이 키는 쓸 수 없어요 · ⌘나 ⌃와 함께 눌러 보세요".to_string(),
            },
        };
        self.note(&note);
        self.keys_changed();
    }

    /// 단축키가 바뀌었다: 단추 글과 메뉴 막대의 키를 맞춘다
    fn keys_changed(&self) {
        self.show_keys();
        let mtm = self.mtm();
        main_menu(mtm, &NSApplication::sharedApplication(mtm));
    }
}

/// 창의 내용 높이를 h로: 위 끝은 그대로 두고 아래로 늘이거나 줄인다
fn fit_height(window: &NSWindow, h: f64) {
    let content = window.contentRectForFrameRect(window.frame());
    if (content.size.height - h).abs() < 0.5 && (content.size.width - W).abs() < 0.5 {
        return;
    }
    let top = content.origin.y + content.size.height;
    let rect = NSRect::new(NSPoint::new(content.origin.x, top - h), NSSize::new(W, h));
    window.setFrame_display(window.frameRectForContentRect(rect), true);
}

/// 값을 바꾼 뒤: 색인을 시작하거나 열 주소를 열고, 본 창을 다시 그리고, 이 창을 맞춘다
fn after_change() {
    pump();
    redraw();
}

fn open_link(link: &str) {
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(link))
        && !NSWorkspace::sharedWorkspace().openURL(&url)
    {
        sakadu::diag::log(|| format!("open failed: {link}"));
    }
}

/// 설정 폴더(작업 공간·세션·즐겨찾기·검색 설정·단축키)를 Finder에서 고른다. 아직 없으면 만든다
fn reveal_config() {
    let Some(dir) = sakadu::util::config_dir() else { return };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        sakadu::diag::log(|| format!("config dir: {e}"));
        return;
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
    NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
}

/// 설정 창을 연다 (앱 메뉴의 '설정…', ⌘,). 이미 떠 있으면 앞으로 가져온다. 처음 열 때와 닫았다가 다시 열 때는 화면 가운데에 놓는다
pub(super) fn open(mtm: MainThreadMarker) {
    let (window, pane) = SETTINGS.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| {
                let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
                let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(W, 300.0));
                let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false) };
                unsafe { window.setReleasedWhenClosed(false) };
                window.setTitle(&NSString::from_str("설정"));
                let pane = SettingsPane::new(mtm);
                // 내용이 창보다 길면 스크롤한다 (위아래를 뒤집은 내용이라 맨 위부터 보인다)
                let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
                scroll.setHasVerticalScroller(true);
                scroll.setAutohidesScrollers(true);
                scroll.setDrawsBackground(false);
                scroll.setDocumentView(Some(&pane));
                window.setContentView(Some(&scroll));
                (window, pane)
            })
            .clone()
    });
    let fresh = !window.isVisible();
    // 처음 놓기: 지금 값으로 줄을 놓고 높이를 맞춘다
    pane.ivars().denied.set(None);
    pane.ivars().capture.set(None);
    if let Some(prefs) = with_app(|a| a.prefs()) {
        pane.show(&prefs, &window);
    }
    if fresh {
        window.center();
        pane.scrollPoint(NSPoint::new(0.0, 0.0));
    }
    window.makeFirstResponder(Some(&pane));
    window.makeKeyAndOrderFront(None);
}

/// 설정 창이 떠 있으면 App의 지금 값에 맞춘다 (입력과 뒷줄 결과를 처리할 때마다 부른다)
pub(super) fn refresh() {
    let Some((window, pane)) = SETTINGS.with(|s| s.borrow().clone()) else { return };
    if !window.isVisible() {
        return;
    }
    if let Some(prefs) = with_app(|a| a.prefs()) {
        pane.show(&prefs, &window);
    }
}

/// 설정 창을 닫는다 (Esc, ⌘W, 본 창을 닫을 때)
pub(super) fn close() {
    if let Some((window, pane)) = SETTINGS.with(|s| s.borrow().clone()) {
        pane.ivars().capture.set(None);
        window.close();
    }
}

/// 설정 창 (창 시험과 그림이 본다). 떠 있지 않으면 None
pub(super) fn shown() -> Option<Retained<NSWindow>> {
    SETTINGS.with(|s| s.borrow().as_ref().map(|(w, _)| w.clone())).filter(|w| w.isVisible())
}

/// 설정 창의 스위치 (창 시험이 누른다)
pub(super) fn switch(name: &str) -> Option<Retained<NSSwitch>> {
    let ctl = match name {
        "hidden" => Ctl::Hidden,
        "preview" => Ctl::Preview,
        "search" => Ctl::Search,
        "icloud" => Ctl::ICloud,
        _ => return None,
    };
    SETTINGS.with(|s| s.borrow().as_ref().and_then(|(_, pane)| pane.parts().switches.iter().find(|(c, _)| *c == ctl).map(|(_, s)| s.clone())))
}

/// 설정 창에서 동작 a의 단축키 단추 (창 시험이 누르고 글을 본다)
pub(super) fn key_button(a: Action) -> Option<Retained<NSButton>> {
    let i = ACTIONS.iter().position(|x| x.0 == a)?;
    SETTINGS.with(|s| s.borrow().as_ref().and_then(|(_, pane)| pane.parts().keys.iter().find(|(k, _)| *k == i).map(|(_, b)| b.clone())))
}

/// 설정 창의 모양을 밝게·어둡게로 정한다 (그림을 찍을 때만)
pub(super) fn set_dark(dark: bool) {
    if let Some(w) = shown() {
        let name = if dark { unsafe { NSAppearanceNameDarkAqua } } else { unsafe { NSAppearanceNameAqua } };
        w.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
    }
}
