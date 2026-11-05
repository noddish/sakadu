//! 왼쪽 사이드바: 창 전체 높이. 맨 위 검색 상자, 즐겨찾기, 아래쪽 설정·도움말.
//! 도구 막대 줄을 없애고 그 단추들을 쓰임에 맞는 자리로 옮긴 것이다(칸 나누기·숨김·미리 보기는 작업 줄에)

use super::*;

/// 사이드바 너비 (DIP). 칸 영역은 그 오른쪽 MARGIN부터
pub(super) const SIDEBAR_W: f32 = 232.0;
const G_HELP: u16 = 0xE897;

/// 창 바탕 위에 한 단계 들어간 면 (검색 상자, 칸 나누기 판)
pub(super) fn well(t: &Theme) -> Color {
    if t.high_contrast {
        t.window
    } else {
        t.sel_inactive
    }
}

/// 사이드바 줄의 마우스 올림 바탕: 창 바탕에서도 보이도록 반투명으로 겹친다
fn hover_fill(t: &Theme) -> Color {
    if t.high_contrast {
        t.hover
    } else if t.dark {
        Color { r: 1.0, g: 1.0, b: 1.0, a: 0.06 }
    } else {
        Color { r: 0.06, g: 0.09, b: 0.16, a: 0.05 }
    }
}

impl App {
    /// 맨 위 검색 상자 (누르면 검색 팔레트)
    pub(super) fn search_button(&self) -> R {
        R::new(12.0, 10.0, SIDEBAR_W - 12.0, 44.0)
    }

    /// 즐겨찾기를 여는 줄
    pub(super) fn fav_button(&self) -> R {
        R::new(8.0, 60.0, SIDEBAR_W - 8.0, 90.0)
    }

    /// 아래쪽: 설정, 그 아래 도움말
    pub(super) fn settings_button(&self) -> R {
        let b = self.h - 8.0;
        R::new(8.0, b - 60.0, SIDEBAR_W - 8.0, b - 30.0)
    }

    pub(super) fn help_button(&self) -> R {
        let b = self.h - 8.0;
        R::new(8.0, b - 30.0, SIDEBAR_W - 8.0, b)
    }

    pub(super) fn sidebar_hit(&self, x: f32, y: f32) -> Hit {
        if self.search_button().contains(x, y) {
            Hit::SearchButton
        } else if self.fav_button().contains(x, y) {
            Hit::FavButton
        } else if self.settings_button().contains(x, y) {
            Hit::SettingsButton
        } else if self.help_button().contains(x, y) {
            Hit::HelpButton
        } else {
            Hit::None
        }
    }

    pub(super) fn paint_sidebar(&self) {
        let t = self.theme;
        let g = &self.gfx;

        // 검색 상자. 팔레트가 열려 있으면 눌린 모양
        let sb = self.search_button();
        let on = self.pal_open;
        g.fill_round(sb, 8.0, if on { t.accent_soft } else { well(&t) });
        if t.high_contrast {
            g.stroke_round(sb.inset(0.5, 0.5), 8.0, t.border, 1.0);
        }
        let fg = if on { t.accent } else { t.dim };
        g.glyph(G_SEARCH, R::new(sb.l + 10.0, sb.t, sb.l + 28.0, sb.b), true, fg);
        g.text("검색", R::new(sb.l + 36.0, sb.t, sb.r - 8.0, sb.b), Font::Normal, if on { t.accent } else { t.faint });
        // 바꾼 검색 단축키를 그대로 보여 준다 (비워 두었으면 없음)
        if let Some(c) = self.keys.get(crate::keys::Action::Search) {
            g.text(&c.label(), R::new(sb.l + 36.0, sb.t, sb.r - 10.0, sb.b), Font::Right, t.faint);
        }

        let row = |r: R, glyph: u16, label: &str, on: bool, hovered: bool| {
            if on {
                g.fill_round(r, 6.0, well(&t));
            } else if hovered {
                g.fill_round(r, 6.0, hover_fill(&t));
            }
            g.glyph(glyph, R::new(r.l + 8.0, r.t, r.l + 28.0, r.b), true, if on { t.accent } else { t.dim });
            g.text(label, R::new(r.l + 36.0, r.t, r.r - 8.0, r.b), Font::Normal, t.text);
        };
        row(self.fav_button(), G_STAR, "즐겨찾기", self.fav_open, self.hover == Hit::FavButton);

        // 아래쪽: 구분선, 설정, 도움말
        let s = self.settings_button();
        g.fill(R::new(s.l + 8.0, s.t - 6.0, s.r - 8.0, s.t - 5.0), t.border);
        row(s, G_SETTINGS, "설정", false, self.hover == Hit::SettingsButton);
        row(self.help_button(), G_HELP, "도움말", false, self.hover == Hit::HelpButton);
    }
}
