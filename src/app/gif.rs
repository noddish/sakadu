//! 움직이는 gif: 자동 미리 보기 카드나 미리 보기 창에 보이는 gif 하나를 움직인다.
//! 프레임은 뒤 스레드에서 읽어 보이는 크기로 줄이고(thumbs::gif_frames), 프레임마다 정해진 시간이 지나면 타이머로 넘긴다.
//! 부담을 막으려고: 400장이나 48MB(줄인 크기 기준)를 넘으면 첫 장만 보인다. 사카두 창이 앞에 있고 보일 때만 움직인다
//! (다른 앱으로 가면 멈췄다가 돌아오면 이어 간다). Windows의 '애니메이션 효과' 설정은 따르지 않는다: 화면 효과를 줄이는
//! 설정이지 그림 내용을 멈추는 설정이 아니고, 브라우저도 이 설정과 상관없이 gif를 움직인다

use super::*;
use crate::WM_APP_PREVIEW;

pub(super) const TIMER_GIF: usize = 32;
const MAX_FRAMES: usize = 400;
const MAX_BYTES: usize = 48 << 20;

pub(super) struct GifPlay {
    /// 보이는 gif의 썸네일 키 (px|수정 시각|경로)
    key: String,
    /// 프레임마다 머무는 시간(ms). 비어 있으면 아직 읽는 중이거나 움직이지 않는 gif (첫 장만)
    delays: Vec<u32>,
    at: usize,
    ticking: bool,
}

impl App {
    /// 움직여도 되는 때: 창이 앞에 있고 보인다
    fn gif_may_play(&self) -> bool {
        self.window_active && unsafe { IsWindowVisible(self.hwnd).as_bool() && !IsIconic(self.hwnd).as_bool() }
    }

    /// 그리기 전에: 보이는 gif가 바뀌었으면 읽기 시작하고, 사라졌으면 버린다. 멈춰 있다가 다시 움직여도 되면 이어 간다
    pub(super) fn sync_gif(&mut self) {
        let want = self.card_gif().or_else(|| self.pane_gif());
        // 그리기 장치를 새로 만들었으면(화면 배율 변경 등) 올려 둔 프레임이 없어졌다: 다시 읽는다
        let lost = self.gif.as_ref().is_some_and(|g| !g.delays.is_empty() && self.gfx.anim_key() != Some(g.key.as_str()));
        let same = !lost && matches!((&self.gif, &want), (Some(g), Some((key, ..))) if g.key == *key);
        if same {
            self.resume_gif();
            return;
        }
        if self.gif.is_none() && want.is_none() {
            return;
        }
        self.stop_gif();
        if let Some((key, path, px)) = want {
            spawn_decode(self.hwnd, key.clone(), path, px);
            self.gif = Some(GifPlay { key, delays: Vec::new(), at: 0, ticking: false });
        }
    }

    fn resume_gif(&mut self) {
        let may = self.gif_may_play();        let hwnd = self.hwnd;
        let Some(g) = self.gif.as_mut() else { return };
        if may && !g.ticking && !g.delays.is_empty() {
            g.ticking = true;
            unsafe { SetTimer(Some(hwnd), TIMER_GIF, g.delays[g.at], None) };
        }
    }

    fn stop_gif(&mut self) {
        if self.gif.take().is_some() {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_GIF);
            }
            self.gfx.clear_anim();
        }
    }

    /// 타이머: 다음 프레임으로. 창이 뒤로 갔으면 멈춘다 (돌아와 다시 그릴 때 sync_gif가 이어 간다)
    pub(super) fn on_gif_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_GIF);
        }
        let may = self.gif_may_play();
        let hwnd = self.hwnd;
        let Some(g) = self.gif.as_mut() else { return };
        g.ticking = false;
        if !may || g.delays.is_empty() {
            return;
        }
        g.at = (g.at + 1) % g.delays.len();
        g.ticking = true;
        unsafe { SetTimer(Some(hwnd), TIMER_GIF, g.delays[g.at], None) };
        self.invalidate();
    }

    /// 뒤에서 읽은 프레임: 지금 보이는 gif의 것이면 올리고 움직이기 시작한다. 움직이지 않는 gif면 첫 장(썸네일) 그대로
    pub(super) fn on_gif_frames(&mut self, key: String, frames: Option<crate::thumbs::GifFrames>) {
        if self.gif.as_ref().is_none_or(|g| g.key != key) {
            return;
        }
        let Some((frames, delays)) = frames else { return };
        if !self.gfx.set_anim(key, &frames) {
            return;
        }
        if let Some(g) = self.gif.as_mut() {
            g.delays = delays;
            g.at = 0;
        }
        self.resume_gif();
        self.invalidate();
    }

    /// 움직이는 중인 gif: (썸네일 키, 지금 프레임 번호)
    pub(super) fn gif_now(&self) -> Option<(String, usize)> {
        self.gif.as_ref().filter(|g| !g.delays.is_empty()).map(|g| (g.key.clone(), g.at))
    }
}

fn spawn_decode(hwnd: HWND, key: String, path: String, px: u32) {
    let owner = shell::SendHwnd(hwnd);
    std::thread::spawn(move || {
        let _com = shell::ComInit::sta();
        let frames = crate::thumbs::gif_frames(&path, px, MAX_FRAMES, MAX_BYTES);
        post(owner.get(), WM_APP_PREVIEW, Box::new(PreviewMsg::Gif { key, frames }));
    });
}
