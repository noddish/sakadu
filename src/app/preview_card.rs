//! 자동 미리 보기 카드: 지금 칸에서 그림·글 파일 하나를 고르면 다른 칸의 오른쪽 아래에 저절로 뜬다.
//! 자리와 그리기는 공통 코드(ui::preview_card). 그림은 셸 썸네일(목록·미리 보기 창과 같은 캐시), 글은 앞부분을 뒤에서 읽는다.
//! 도구 막대의 눈 단추로 끄고 켠다. 오른쪽 미리 보기 창(Alt+P, preview.rs)이 열려 있으면 그 창이 보여 주므로 카드는 뜨지 않는다.
//! PDF·Office 문서처럼 미리 보기 처리기가 필요한 파일은 카드에 띄우지 않는다 (미리 보기 창에서 본다)

use super::preview::{IMAGE_EXTS, TEXT_EXTS, ext_of, image_px, next_seq, spawn_text};
use super::*;
use crate::thumbs::Request;
use sakadu::ui::preview_card::{CardArt, CardBody, CardGeom, CardHover, CardScene, card_geom, paint_preview_card};

pub(super) const TIMER_CARD: usize = 31;
/// 고른 뒤 카드가 뜨기까지 (키를 누르고 있을 때 파일마다 띄우지 않게)
const SETTLE_MS: u32 = 120;
/// 카드에 보일 글: 줄 수와 한 줄의 글자 수 (그릴 때마다 긴 글을 배치하지 않게 앞부분만 둔다)
const TEXT_LINES: usize = 80;
const TEXT_COLS: usize = 200;

#[derive(Clone, PartialEq)]
struct Target {
    path: String,
    name: String,
    size: u64,
    modified: u64,
    type_name: String,
    /// 글 파일이면 true, 그림이면 false
    text: bool,
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
    missing: &'a mut Vec<Request>,
    drawn: bool,
}

impl CardArt<Gfx> for WinCardArt<'_> {
    fn image(&mut self, g: &mut Gfx, r: R) -> bool {
        self.drawn = g.thumb(&self.key, r, 1.0);
        if !self.drawn && !self.failed {
            // 카드의 그림은 목록보다 먼저 만든다
            self.missing.insert(0, Request { key: self.key.clone(), path: self.path.to_string(), px: self.px });
        }
        self.drawn
    }
}

impl App {
    /// 지금 카드로 보여 줄 파일: 지금 칸에서 하나만 고른 그림·글 파일.
    /// 자동 미리 보기를 껐거나, 미리 보기 창·설정 창이 열려 있으면 없다
    fn card_wanted(&mut self) -> Option<Target> {
        if !self.auto_preview || self.preview.is_some() || self.settings.is_some() {
            return None;
        }
        let App { panes, types, active, .. } = self;
        let tab = panes[*active].tab();
        let e = tab.entry_at(tab.cursor).filter(|e| e.selected && e.kind == Kind::File)?;
        if tab.selection_stats().0 != 1 {
            return None;
        }
        let name = e.name_string();
        let ext = ext_of(&name);
        let text = TEXT_EXTS.contains(&ext.as_str());
        if !text && !IMAGE_EXTS.contains(&ext.as_str()) {
            return None;
        }
        let (type_name, _) = type_info(types, e);
        Some(Target { path: tab.path_of(e)?, name, size: e.size, modified: e.modified, type_name, text })
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
        if t.text {
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

    /// 카드가 보이면 그 자리. 글 파일은 다 읽은 뒤에 보인다
    pub(super) fn card_rects(&self) -> Option<CardGeom> {
        let c = &self.card;
        let t = c.target.as_ref()?;
        let ready = c.started && !c.unreadable && (!t.text || c.text.is_some());
        if !ready || c.dismissed.as_deref() == Some(t.path.as_str()) || !self.auto_preview || self.preview.is_some() || self.settings.is_some() {
            return None;
        }
        card_geom(&self.pane_rects(), self.active, c.large)
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
        self.flash(p, if self.auto_preview { "자동 미리 보기를 켰어요 · 그림이나 글 파일을 고르면 보여요" } else { "자동 미리 보기를 껐어요" });
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
        let App { gfx, card, thumb_failed, .. } = self;
        let Some(target) = &card.target else { return };
        let detail = format!("{} · {}", target.type_name, format_size(target.size));
        let px = image_px(m.body.w().max(m.body.h()), scale);
        let key = thumb_key(&target.path, target.modified, px);
        let failed = thumb_failed.contains(&key);
        let waited = card.since.elapsed().as_millis();
        // 그림이 곧 오면 글자를 보이지 않는다. 늦으면 0.3초 뒤에 '불러오는 중…'
        let note = match () {
            _ if target.text => None,
            _ if failed => Some("미리 볼 수 없어요"),
            _ if waited >= 300 => Some("불러오는 중…"),
            _ => None,
        };
        let body = match &card.text {
            Some(s) if target.text => CardBody::Text(s),
            _ => CardBody::Image,
        };
        let mut art = WinCardArt { key, path: &target.path, px, failed, missing, drawn: false };
        paint_preview_card(gfx, &mut art, &t, &CardScene { geom: &m, name: &target.name, detail: &detail, body, note, large: card.large, hover });
        if !target.text && !art.drawn && !failed && waited < 300 {
            unsafe { SetTimer(Some(hwnd), TIMER_REDRAW, (300 - waited) as u32, None) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
