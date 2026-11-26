//! 자동 미리 보기 카드: 지금 칸에서 파일 하나를 고르면 다른 칸의 오른쪽 아래에 저절로 뜬다.
//! 자리와 그리기는 공통 코드(ui::preview_card). 그림은 셸 썸네일(목록·미리 보기 창과 같은 캐시), 글은 앞부분을 뒤에서 읽는다.
//! PDF·Office 문서·동영상·음악처럼 셸이 썸네일을 만들 수 있는 파일과 한글 문서(안에 든 미리 보기 그림), 포토샵·일러스트레이터
//! (사카두가 직접 그린다)는 그 썸네일을 크게 보인다. html은 Edge로 찍은 화면을 보이고, 찍지 못하면 코드를 보인다.
//! 썸네일이 없으면 카드를 띄우지 않는다. 도구 막대의 자동 미리 보기 단추로 끄고 켠다.
//! 오른쪽 미리 보기 창(Alt+P, preview.rs)이 열려 있으면 그 창이 보여 주므로 카드는 뜨지 않는다

use super::preview::{DRAWN_EXTS, IMAGE_EXTS, TEXT_EXTS, ext_of, image_px, next_seq, spawn_text, thumb_provider};
use super::*;
use crate::thumbs::Request;
use sakadu::ui::preview_card::{CardArt, CardBody, CardGeom, CardHover, CardScene, card_geom, paint_preview_card};

pub(super) const TIMER_CARD: usize = 31;
/// 고른 뒤 카드가 뜨기까지 (키를 누르고 있을 때 파일마다 띄우지 않게)
const SETTLE_MS: u32 = 120;
/// 카드에 보일 글: 줄 수와 한 줄의 글자 수 (그릴 때마다 긴 글을 배치하지 않게 앞부분만 둔다)
const TEXT_LINES: usize = 80;
const TEXT_COLS: usize = 200;

/// 카드에 무엇을 보이는지
#[derive(Clone, Copy, PartialEq, Debug)]
enum CardKind {
    /// 그림 (셸 썸네일, SVG는 직접 그린다)
    Image,
    /// 글 파일의 앞부분
    Text,
    /// 문서·동영상·음악의 썸네일, 한글 문서의 미리 보기 그림. 없으면 카드를 띄우지 않는다
    Doc,
    /// html을 Edge로 찍은 화면 (page.rs). 찍지 못하면 글 카드로 바꾼다
    Page,
}

#[derive(Clone, PartialEq)]
struct Target {
    path: String,
    name: String,
    size: u64,
    modified: u64,
    type_name: String,
    kind: CardKind,
}

/// 확장자로 카드에 보일 것을 정한다. 보일 것이 없으면 None
fn card_kind(ext: &str) -> Option<CardKind> {
    if crate::page::is_page_ext(ext) && crate::page::available() {
        Some(CardKind::Page)
    } else if TEXT_EXTS.contains(&ext) {
        Some(CardKind::Text)
    } else if IMAGE_EXTS.contains(&ext) || ext == "svg" {
        Some(CardKind::Image)
    } else if DRAWN_EXTS.contains(&ext) || thumb_provider(ext) {
        Some(CardKind::Doc)
    } else {
        None
    }
}

pub(super) struct Card {
    target: Option<Target>,
    /// 잠깐 기다린 뒤 띄우기 시작했다
    started: bool,
    /// 글 파일의 앞부분 (읽는 중이면 None)
    text: Option<String>,
    /// 글로 읽지 못했다 (이진 파일이거나 열지 못했다): 카드를 띄우지 않는다
    unreadable: bool,
    /// 늦게 도착한 이전 글을 버리기 위한 순번
    seq: u64,
    /// 닫기로 닫은 파일. 다른 파일을 고를 때까지 다시 띄우지 않는다
    dismissed: Option<String>,
    /// 크게 보기. 카드가 사라질 때까지 이어진다 (파일을 옮겨 다니며 크게 볼 수 있게)
    large: bool,
    since: std::time::Instant,
}

impl Card {
    pub(super) fn new() -> Card {
        Card { target: None, started: false, text: None, unreadable: false, seq: next_seq(), dismissed: None, large: false, since: std::time::Instant::now() }
    }
}

/// 카드에 보일 글: 앞쪽 몇 줄만, 긴 줄은 잘라서
fn snippet(text: &str) -> String {
    text.lines().take(TEXT_LINES).map(|l| l.chars().take(TEXT_COLS).collect::<String>()).collect::<Vec<_>>().join("\n")
}

/// 카드의 그림: 셸 썸네일 캐시에 있으면 그리고, 없으면 만들어 달라고 한다
struct WinCardArt<'a> {
    key: String,
    path: &'a str,
    px: u32,
    /// 셸이 그림을 주지 못했다 (다시 요청하지 않는다)
    failed: bool,
    /// 움직이는 gif면 지금 그릴 프레임 번호 (gif.rs)
    frame: Option<usize>,
    /// 썸네일만 (문서: 큰 아이콘을 받지 않는다)
    only_thumb: bool,
    missing: &'a mut Vec<Request>,
    drawn: bool,
}

impl CardArt<Gfx> for WinCardArt<'_> {
    fn image(&mut self, g: &mut Gfx, r: R) -> bool {
        self.drawn = self.frame.is_some_and(|i| g.anim_frame(&self.key, i, r, 1.0)) || g.thumb(&self.key, r, 1.0);
        if !self.drawn && !self.failed {
            // 카드의 그림은 목록보다 먼저 만든다
            self.missing.insert(0, Request { key: self.key.clone(), path: self.path.to_string(), px: self.px, only_thumb: self.only_thumb });
        }
        self.drawn
    }
}

impl App {
    /// 지금 카드로 보여 줄 파일: 지금 칸에서 하나만 고른 파일 가운데 그림·글·문서(썸네일) 파일.
    /// 자동 미리 보기를 껐거나, 미리 보기 창·설정 창이 열려 있으면 없다
    fn card_wanted(&mut self) -> Option<Target> {
        if !self.auto_preview || self.preview.is_some() || self.settings.is_some() {
            return None;
        }
        // html 화면을 찍지 못했는지 보려고 카드 그림의 크기를 미리 잰다
        let px = card_geom(&self.pane_rects(), self.active, self.card.large).map(|m| image_px(m.body.w().max(m.body.h()), self.scale()));
        let App { panes, types, active, thumb_failed, .. } = self;
        let tab = panes[*active].tab();
        let e = tab.entry_at(tab.cursor).filter(|e| e.selected && e.kind == Kind::File)?;
        // 압축 파일 안의 파일은 미리 보지 않는다
        if tab.selection_stats().0 != 1 || tab.in_archive {
            return None;
        }
        let name = e.name_string();
        let path = tab.path_of(e)?;
        let mut kind = card_kind(&ext_of(&name))?;
        // 찍지 못한 html(인터넷에서 받은 파일, 실패, 시간 초과)은 지금처럼 코드를 보인다
        if kind == CardKind::Page && px.is_some_and(|px| thumb_failed.contains(&thumb_key(&path, e.modified, px))) {
            kind = CardKind::Text;
        }
        let (type_name, _) = type_info(types, e);
        Some(Target { path, name, size: e.size, modified: e.modified, type_name, kind })
    }

    /// 그리기 전에: 고른 것이 바뀌었으면 카드를 새 파일에 맞춘다 (조금 기다렸다 띄운다)
    pub(super) fn sync_card(&mut self) {
        let want = self.card_wanted();
        if want == self.card.target {
            return;
        }
        let hwnd = self.hwnd;
        let c = &mut self.card;
        // 닫아 둔 파일이 아닌 것을 고르면(고른 것이 없어져도) 다음에는 다시 띄운다
        if want.as_ref().map(|t| &t.path) != c.dismissed.as_ref() {
            c.dismissed = None;
        }
        c.seq = next_seq();
        c.started = false;
        c.text = None;
        c.unreadable = false;
        c.since = std::time::Instant::now();
        c.target = want;
        unsafe {
            if c.target.is_some() {
                SetTimer(Some(hwnd), TIMER_CARD, SETTLE_MS, None);
            } else {
                let _ = KillTimer(Some(hwnd), TIMER_CARD);
                c.large = false;
            }
        }
    }

    /// 잠깐 기다린 뒤: 카드를 띄우고, 글 파일이면 앞부분을 읽기 시작한다
    pub(super) fn start_card(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_CARD);
        }
        let hwnd = self.hwnd;
        let c = &mut self.card;
        let Some(t) = &c.target else { return };
        c.started = true;
        c.since = std::time::Instant::now();
        if t.kind == CardKind::Text {
            spawn_text(hwnd, c.seq, t.path.clone());
        }
        self.invalidate();
    }

    /// 뒤에서 읽은 글이 이 카드의 것이면 받는다 (순번으로 가린다). 받았으면 true
    pub(super) fn card_text(&mut self, seq: u64, text: &Option<String>) -> bool {
        if seq != self.card.seq {
            return false;
        }
        match text {
            Some(s) => self.card.text = Some(snippet(s)),
            None => self.card.unreadable = true,
        }
        self.refresh_hover();
        self.invalidate();
        true
    }

    /// 카드가 보이면 그 자리. 글 파일은 다 읽은 뒤에 보인다. 문서는 썸네일을 만들지 못했으면 띄우지 않는다
    pub(super) fn card_rects(&self) -> Option<CardGeom> {
        let c = &self.card;
        let t = c.target.as_ref()?;
        let ready = c.started && !c.unreadable && (t.kind != CardKind::Text || c.text.is_some());
        if !ready || c.dismissed.as_deref() == Some(t.path.as_str()) || !self.auto_preview || self.preview.is_some() || self.settings.is_some() {
            return None;
        }
        let m = card_geom(&self.pane_rects(), self.active, c.large)?;
        let px = image_px(m.body.w().max(m.body.h()), self.scale());
        if matches!(t.kind, CardKind::Doc | CardKind::Page) && self.thumb_failed.contains(&thumb_key(&t.path, t.modified, px)) {
            return None;
        }
        Some(m)
    }

    /// 카드에 보이는 gif: (썸네일 키, 경로, 긴 변 px). gif.rs가 움직인다
    pub(super) fn card_gif(&self) -> Option<(String, String, u32)> {
        let m = self.card_rects()?;
        let t = self.card.target.as_ref()?;
        if t.kind != CardKind::Image || ext_of(&t.name) != "gif" {
            return None;
        }
        let px = image_px(m.body.w().max(m.body.h()), self.scale());
        Some((thumb_key(&t.path, t.modified, px), t.path.clone(), px))
    }

    /// 카드에서 누를 수 있는 것. 카드의 그 밖의 곳도 Hit::Card로 잡아, 아래 칸을 누르지 않게 한다
    pub(super) fn card_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let m = self.card_rects()?;
        if m.close.contains(x, y) {
            Some(Hit::CardClose)
        } else if m.expand.contains(x, y) {
            Some(Hit::CardExpand)
        } else {
            m.card.contains(x, y).then_some(Hit::Card)
        }
    }

    /// 카드의 닫기: 다른 파일을 고를 때까지 다시 띄우지 않는다
    pub(super) fn close_card(&mut self) {
        self.card.dismissed = self.card.target.as_ref().map(|t| t.path.clone());
        self.card.large = false;
        self.refresh_hover();
        self.invalidate();
    }

    /// 카드를 크게(놓인 칸을 채운다)·작게
    pub(super) fn toggle_card_large(&mut self) {
        self.card.large = !self.card.large;
        self.refresh_hover();
        self.invalidate();
    }

    /// 도구 막대의 눈: 자동 미리 보기를 끄고 켠다
    pub(super) fn toggle_auto_preview(&mut self) {
        self.auto_preview = !self.auto_preview;
        let p = self.active;
        self.flash(p, if self.auto_preview { "자동 미리 보기를 켰어요 · 그림·글·문서 파일을 고르면 보여요" } else { "자동 미리 보기를 껐어요" });
        self.save_session();
        self.invalidate();
    }

    pub(super) fn card_tip(&self) -> Option<(R, String)> {
        let m = self.card_rects()?;
        match self.hover {
            Hit::CardExpand => Some((m.expand, if self.card.large { "작게 보기" } else { "크게 보기" }.to_string())),
            Hit::CardClose => Some((m.close, "닫기 · 다른 파일을 고르면 다시 보여요".to_string())),
            _ => None,
        }
    }

    pub(super) fn paint_card(&mut self, missing: &mut Vec<Request>) {
        let Some(m) = self.card_rects() else { return };
        let t = self.theme;
        let scale = self.scale();
        let hover = match self.hover {
            Hit::CardExpand => CardHover::Expand,
            Hit::CardClose => CardHover::Close,
            _ => CardHover::None,
        };
        let hwnd = self.hwnd;
        let anim = self.gif_now();
        let App { gfx, card, thumb_failed, .. } = self;
        let Some(target) = &card.target else { return };
        let detail = format!("{} · {}", target.type_name, format_size(target.size));
        let px = image_px(m.body.w().max(m.body.h()), scale);
        let key = thumb_key(&target.path, target.modified, px);
        let frame = anim.filter(|(k, _)| *k == key).map(|(_, i)| i);
        let failed = thumb_failed.contains(&key);
        let waited = card.since.elapsed().as_millis();
        // 그림이 곧 오면 글자를 보이지 않는다. 늦으면 0.3초 뒤에 '불러오는 중…'
        let text = target.kind == CardKind::Text;
        let note = match () {
            _ if text => None,
            _ if failed => Some("미리 볼 수 없어요"),
            _ if waited >= 300 => Some("불러오는 중…"),
            _ => None,
        };
        let body = match &card.text {
            Some(s) if text => CardBody::Text(s),
            _ => CardBody::Image,
        };
        let only_thumb = matches!(target.kind, CardKind::Doc | CardKind::Page);
        let mut art = WinCardArt { key, path: &target.path, px, failed, frame, only_thumb, missing, drawn: false };
        paint_preview_card(gfx, &mut art, &t, &CardScene { geom: &m, name: &target.name, detail: &detail, body, note, large: card.large, hover });
        if !text && !art.drawn && !failed && waited < 300 {
            unsafe { SetTimer(Some(hwnd), TIMER_REDRAW, (300 - waited) as u32, None) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 카드에 보일 것: 글·그림은 그대로, SVG는 그림, 한글 문서와 썸네일이 있는 문서·동영상은 썸네일. 실행 파일·바로 가기는 없다
    #[test]
    fn card_kinds_follow_the_extension() {
        assert_eq!(card_kind("txt"), Some(CardKind::Text));
        assert_eq!(card_kind("jsonl"), Some(CardKind::Text));
        assert_eq!(card_kind("png"), Some(CardKind::Image));
        assert_eq!(card_kind("svg"), Some(CardKind::Image));
        assert_eq!(card_kind("hwp"), Some(CardKind::Doc));
        // 포토샵·일러스트레이터는 사카두가 직접 그리는 문서
        assert_eq!((card_kind("psd"), card_kind("psb"), card_kind("ai")), (Some(CardKind::Doc), Some(CardKind::Doc), Some(CardKind::Doc)));
        // html은 Edge가 있으면 화면, 없으면 지금처럼 코드
        let page = if crate::page::available() { CardKind::Page } else { CardKind::Text };
        assert_eq!((card_kind("html"), card_kind("htm")), (Some(page), Some(page)));
        assert_eq!(card_kind("exe"), None);
        assert_eq!(card_kind("lnk"), None);
        // 이 PC에 등록된 썸네일 처리기를 따른다: Windows에 늘 있는 동영상 썸네일
        assert_eq!(card_kind("mp4"), Some(CardKind::Doc));
    }

    #[test]
    fn snippet_keeps_the_first_lines_and_cuts_long_ones() {
        let long = "가".repeat(TEXT_COLS + 50);
        let text = (0..TEXT_LINES + 20).map(|i| if i == 1 { long.clone() } else { format!("줄 {i}") }).collect::<Vec<_>>().join("\r\n");
        let s = snippet(&text);
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), TEXT_LINES);
        assert_eq!((lines[0], lines[1].chars().count(), lines[TEXT_LINES - 1]), ("줄 0", TEXT_COLS, format!("줄 {}", TEXT_LINES - 1).as_str()));
    }
}
