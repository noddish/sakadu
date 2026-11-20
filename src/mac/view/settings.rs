//! 설정 창: 앱 메뉴의 '설정…'(⌘,)으로 여는, 본 창과 따로 뜨는 창. 맥 기본 조작(스위치·단추)을 시스템 설정처럼
//! 묶음 카드 안에 줄로 놓는다. 줄에는 이름만 쓴다.
//! 값은 App이 갖고(App::prefs, set_…), 이 창은 그것을 보이고 바꾼다. 본 창에서 바뀐 값(⌘⇧., 눈 단추, 훑기 상태)은
//! 입력과 뒷줄 결과를 처리할 때마다 refresh로 따라온다

use super::*;
use crate::app::Prefs;
use objc2_app_kit::{NSAccessibility, NSAppearance, NSAppearanceCustomization, NSBox, NSBoxType, NSButton, NSSwitch, NSTitlePosition};

/// 창 안쪽 폭, 둘레 여백, 묶음 제목 높이, 묶음 사이, 줄 높이, 카드 안쪽 여백
const W: f64 = 460.0;
const MARGIN: f64 = 20.0;
const TITLE_H: f64 = 18.0;
const GAP: f64 = 18.0;
const LINE_H: f64 = 36.0;
const INSET: f64 = 14.0;
const GITHUB: &str = "https://github.com/noddish/sakadu";
/// Esc
const ESC: u16 = 0x35;

thread_local! {
    /// 설정 창과 그 내용 (처음 열 때 만들고, 닫아도 두었다가 다시 연다)
    static SETTINGS: RefCell<Option<(Retained<NSWindow>, Retained<SettingsPane>)>> = const { RefCell::new(None) };
}

/// 줄의 조작 (조작의 tag)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctl {
    Hidden = 1,
    Preview,
    Search,
    ICloud,
    Allow,
    GitHub,
    Folder,
}

impl Ctl {
    const ALL: [Ctl; 7] = [Ctl::Hidden, Ctl::Preview, Ctl::Search, Ctl::ICloud, Ctl::Allow, Ctl::GitHub, Ctl::Folder];

    fn from_tag(tag: NSInteger) -> Option<Ctl> {
        Ctl::ALL.into_iter().find(|c| *c as NSInteger == tag)
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
    /// '이름으로 찾기' 옆의 '훑는 중'
    crawling: Retained<NSTextField>,
}

#[derive(Default)]
pub struct PaneIvars {
    parts: OnceCell<Parts>,
    /// 지난번에 놓을 때 '읽지 못한 폴더' 줄을 보였는지 (바뀌면 다시 놓고 창 높이를 맞춘다)
    denied: std::cell::Cell<Option<bool>>,
}

define_class!(
    /// 설정 창의 내용. 위아래를 뒤집어 쓰고(왼쪽 위가 0), 조작들이 보내는 동작을 받고, Esc와 ⌘W로 창을 닫는다
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
            if e.keyCode() == ESC {
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

        /// ⌘W: 이 창을 닫는다 (본 창의 ⌘W는 탭 닫기라 메뉴에 두지 않았다)
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, e: &NSEvent) -> bool {
            let flags = e.modifierFlags() & NSEventModifierFlags::DeviceIndependentFlagsMask;
            let key = self.window().is_some_and(|w| w.isKeyWindow());
            if key && flags == NSEventModifierFlags::Command && e.charactersIgnoringModifiers().is_some_and(|c| c.to_string() == "w") {
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

        /// 단추를 눌렀다
        #[unsafe(method(press:))]
        fn press(&self, sender: &NSButton) {
            match Ctl::from_tag(sender.tag()) {
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
        let cards: Vec<Retained<NSBox>> = (0..3).map(|_| card()).collect();
        let label = |text: &str| {
            let l = NSTextField::labelWithString(&NSString::from_str(text), mtm);
            pane.addSubview(&l);
            l
        };
        let titles: Vec<Retained<NSTextField>> = ["일반", "검색", "정보"]
            .iter()
            .map(|t| {
                let l = label(t);
                l.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
                l
            })
            .collect();
        let crawling = label("훑는 중");
        crawling.setTextColor(Some(&NSColor::secondaryLabelColor()));
        crawling.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        crawling.setHidden(true);
        let target: &AnyObject = &pane;
        let mut switches = Vec::new();
        let mut lines = Vec::new();
        let version = format!("Sakadu {}", env!("CARGO_PKG_VERSION"));
        let rows: [(usize, Ctl, &str, Option<&str>); 7] = [
            (0, Ctl::Hidden, "숨김 항목 보기", None),
            (0, Ctl::Preview, "자동 미리 보기", None),
            (1, Ctl::Search, "이름으로 찾기", None),
            (1, Ctl::ICloud, "iCloud Drive도 찾기", None),
            (1, Ctl::Allow, "읽지 못한 폴더가 있어요", Some("설정에서 허용")),
            (2, Ctl::GitHub, version.as_str(), Some("GitHub")),
            (2, Ctl::Folder, "설정 폴더", Some("Finder에서 보기")),
        ];
        for (group, ctl, name, button) in rows {
            let divider = NSBox::new(mtm);
            divider.setBoxType(NSBoxType::Separator);
            pane.addSubview(&divider);
            let text = label(name);
            let control: Retained<NSControl> = match button {
                // SAFETY: 이 뷰가 press:를 받는다 (define_class에 있다)
                Some(title) => unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(target), Some(sel!(press:)), mtm) }.into_super(),
                None => {
                    let s = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), NSRect::ZERO);
                    // SAFETY: 이 뷰가 toggle:을 받는다 (define_class에 있다)
                    unsafe {
                        s.setTarget(Some(target));
                        s.setAction(Some(sel!(toggle:)));
                    }
                    // 스위치는 옆의 글을 이름으로 읽지 않으므로 줄 이름을 붙여 둔다 (VoiceOver)
                    s.setAccessibilityLabel(Some(&NSString::from_str(name)));
                    switches.push((ctl, s.clone()));
                    s.into_super()
                }
            };
            control.setTag(ctl as NSInteger);
            pane.addSubview(&control);
            lines.push(Line { group, ctl, label: text, control, divider });
        }
        let _ = pane.ivars().parts.set(Parts { titles, cards, lines, switches, crawling });
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
                line.divider.setFrame(rect(l + INSET, y, r - l - 2.0 * INSET, 1.0));
                let c = line.control.intrinsicContentSize();
                let cx = r - INSET - c.width;
                line.control.setFrame(rect(cx, y + (LINE_H - c.height) / 2.0, c.width, c.height));
                let t = line.label.intrinsicContentSize();
                let tw = t.width.min(cx - 8.0 - (l + INSET));
                line.label.setFrame(rect(l + INSET, y + (LINE_H - t.height) / 2.0, tw, t.height));
                if line.ctl == Ctl::Search {
                    let s = p.crawling.intrinsicContentSize();
                    p.crawling.setFrame(rect(l + INSET + tw + 8.0, y + (LINE_H - s.height) / 2.0, s.width, s.height));
                }
                y += LINE_H;
                first = false;
            }
            card.setFrame(rect(l, top, r - l, y - top));
        }
        y + MARGIN
    }

    /// App의 값으로 스위치와 글을 맞춘다. 읽지 못한 폴더 줄이 생기거나 없어지면 다시 놓고 창 높이를 맞춘다
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
        p.crawling.setHidden(!prefs.crawling);
        if self.ivars().denied.get() != Some(prefs.denied) {
            self.ivars().denied.set(Some(prefs.denied));
            let h = self.layout(prefs.denied);
            fit_height(window, h);
        }
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

/// 설정 폴더(작업 공간·세션·즐겨찾기·검색 설정)를 Finder에서 고른다. 아직 없으면 만든다
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
                window.setContentView(Some(&pane));
                (window, pane)
            })
            .clone()
    });
    let fresh = !window.isVisible();
    // 처음 놓기: 지금 값으로 줄을 놓고 높이를 맞춘다
    pane.ivars().denied.set(None);
    if let Some(prefs) = with_app(|a| a.prefs()) {
        pane.show(&prefs, &window);
    }
    if fresh {
        window.center();
        window.makeFirstResponder(Some(&pane));
    }
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
    if let Some((window, _)) = SETTINGS.with(|s| s.borrow().clone()) {
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

/// 설정 창의 모양을 밝게·어둡게로 정한다 (그림을 찍을 때만)
pub(super) fn set_dark(dark: bool) {
    if let Some(w) = shown() {
        let name = if dark { unsafe { NSAppearanceNameDarkAqua } } else { unsafe { NSAppearanceNameAqua } };
        w.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
    }
}
