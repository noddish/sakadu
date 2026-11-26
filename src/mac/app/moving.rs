//! 움직이는 gif (미리 보기 카드와 오른쪽 미리 보기 창). 뒷줄에서 장들을 다 풀어 두고(Gfx::load_frames. 한도를 넘으면 첫 장만),
//! 장마다 적힌 시간만큼 머물렀다 다음 장으로 넘어간다. 창 쪽이 그리기 전에 step_moving을 불러 장을 넘기고, 다음 장의 때에
//! 그 그림 자리만 다시 그린다. 창이 key이고 보이며 맥 설정의 '동작 줄이기'가 꺼져 있을 때만 움직인다 (아니면 지금 장에 머문다).
//! 장들은 고른 것이 바뀌거나 카드·창이 닫히면 함께 버려진다 (카드의 Card::moving, 창 본문의 Body::Moving)

use super::*;
use crate::gfx::Frames;
use std::time::Duration;

/// 움직이는 gif 하나: 장들, 지금 보이는 장, 다음 장으로 넘어갈 때
pub(super) struct Moving {
    frames: Frames,
    /// 장들을 읽을 때 요청한 픽셀 수 (더 크게 읽을지 정한다)
    px: u32,
    shown: usize,
    /// 다음 장으로 넘어갈 때. 멈춰 있으면 None: 다시 움직이면 지금 장을 처음부터 보인다
    due: Option<Instant>,
}

impl Moving {
    /// 장이 둘 이상이고 장마다 머무는 시간이 있는 것만 받는다 (load_preview가 그런 것만 Preview::Moving으로 준다)
    pub(super) fn new(frames: Frames, px: u32) -> Moving {
        debug_assert!(frames.pics.len() > 1 && frames.delays.len() == frames.pics.len());
        Moving { frames, px, shown: 0, due: None }
    }

    /// 지금 보이는 장
    pub(super) fn pic(&self) -> &Picture {
        &self.frames.pics[self.shown]
    }

    /// 이미 가진 크기 (CachedPic::full과 같은 셈): 원본 그대로 읽었으면 더 크게 읽을 것이 없다
    pub(super) fn have(&self) -> u32 {
        if self.pic().longest() + 8 < self.px { u32::MAX } else { self.px }
    }

    /// live면 때가 된 장을 넘기고 다음 장까지 남은 시간을 준다. 아니면 지금 장에 머물고 None
    fn step(&mut self, now: Instant, live: bool) -> Option<Duration> {
        if !live {
            self.due = None;
            return None;
        }
        let due = *self.due.get_or_insert(now + self.frames.delays[self.shown]);
        if now < due {
            return Some(due - now);
        }
        self.shown = (self.shown + 1) % self.frames.pics.len();
        let delay = self.frames.delays[self.shown];
        // 한 장 넘게 늦었으면(창이 바빴다) 밀린 장을 몰아 넘기지 않고 지금부터 센다
        let next = if now - due < delay { due + delay } else { now + delay };
        self.due = Some(next);
        Some(next - now)
    }
}

impl App {
    /// 지금 보이는 움직이는 gif와 그 자리: 오른쪽 창이 열려 있으면 창 본문의 것, 아니면 보이는 카드의 것 (둘은 함께 뜨지 않는다)
    fn moving_shown(&self) -> Option<(&Moving, R)> {
        if self.preview.is_some() {
            return self.pane_moving();
        }
        let (target, m) = self.card_shown()?;
        let mv = self.card.moving.as_ref().filter(|_| self.card.key.as_ref() == Some(&target.key))?;
        Some((mv, m.body))
    }

    /// 창 쪽이 그리기 전에 부른다: 보이는 움직이는 gif를 때가 됐으면 다음 장으로 넘긴다.
    /// live: 창이 key이고 보인다 ('동작 줄이기'는 여기서 본다). 움직이는 중이면 다음 장까지 남은 시간
    pub fn step_moving(&mut self, now: Instant, live: bool) -> Option<Duration> {
        let live = live && self.motion;
        if self.preview.is_some() {
            return self.pane_moving_mut()?.step(now, live);
        }
        let (target, _) = self.card_shown()?;
        if self.card.key.as_ref() != Some(&target.key) {
            return None;
        }
        self.card.moving.as_mut()?.step(now, live)
    }

    /// 움직이는 gif가 보이는 자리 (창 쪽이 다음 장의 때에 그 자리만 다시 그린다)
    pub fn moving_rect(&self) -> Option<R> {
        self.moving_shown().map(|(_, r)| r)
    }

    /// 창 시험: 보이는 움직이는 gif의 지금 장 번호
    pub fn moving_frame(&self) -> Option<usize> {
        self.moving_shown().map(|(m, _)| m.shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::{FRAMES_BYTES_MAX, FRAMES_MAX, frame_delay_of, frames_fit};

    fn frames(n: usize, ms: &[u64]) -> Frames {
        Frames { pics: (0..n).map(|_| Picture::blank(4, 4).unwrap()).collect(), delays: ms.iter().map(|m| Duration::from_millis(*m)).collect() }
    }

    /// 장마다 머무는 시간: 20ms보다 짧거나(0이 흔하다) 적혀 있지 않으면 100ms, 그 밖에는 적힌 그대로
    #[test]
    fn short_or_missing_delays_become_100_ms() {
        let ms = |s: Option<f64>| frame_delay_of(s).as_millis();
        assert_eq!([ms(None), ms(Some(0.0)), ms(Some(0.019)), ms(Some(-1.0)), ms(Some(f64::NAN))], [100; 5]);
        assert_eq!([ms(Some(0.02)), ms(Some(0.05)), ms(Some(1.5))], [20, 50, 1500]);
        assert_eq!(ms(Some(1e300)), 655_350, "no longer than a gif can say");
    }

    /// 다 풀어 두는 한도: 둘 이상 FRAMES_MAX장 이하, 보이는 크기로 푼 장들이 FRAMES_BYTES_MAX 이하. 넘으면 첫 장만
    #[test]
    fn frames_are_kept_only_within_the_limits() {
        assert!(!frames_fit(1, 10, 10), "a still picture");
        assert!(frames_fit(2, 10, 10) && frames_fit(FRAMES_MAX, 10, 10));
        assert!(!frames_fit(FRAMES_MAX + 1, 1, 1), "too many frames");
        // 512×512 한 장은 1MB: 48장까지
        assert_eq!(FRAMES_BYTES_MAX, 48 * 512 * 512 * 4);
        assert!(frames_fit(48, 512, 512) && !frames_fit(49, 512, 512));
        assert!(!frames_fit(3, usize::MAX, usize::MAX), "no overflow");
    }

    /// 장 넘기기: 적힌 시간이 지나면 다음 장, 마지막 다음은 첫 장. 움직이지 않으면 그 장에 머물고, 다시 움직이면 그 장을 처음부터 보인다.
    /// 많이 늦었으면 밀린 장을 몰아 넘기지 않는다
    #[test]
    fn moving_steps_through_frames_by_their_delays() {
        let ms = Duration::from_millis;
        let t0 = Instant::now();
        let mut m = Moving::new(frames(3, &[50, 100, 200]), 1024);
        assert_eq!((m.step(t0, true), m.shown), (Some(ms(50)), 0));
        assert_eq!((m.step(t0 + ms(30), true), m.shown), (Some(ms(20)), 0), "not yet");
        assert_eq!((m.step(t0 + ms(50), true), m.shown), (Some(ms(100)), 1));
        assert_eq!((m.step(t0 + ms(160), true), m.shown), (Some(ms(190)), 2), "10 ms late: the next one keeps the beat");
        assert_eq!((m.step(t0 + ms(400), false), m.shown), (None, 2), "paused on the frame it shows");
        assert_eq!((m.step(t0 + ms(5000), true), m.shown), (Some(ms(200)), 2), "resumes with that frame from the start");
        assert_eq!((m.step(t0 + ms(5200), true), m.shown), (Some(ms(50)), 0), "round to the first frame");
        assert_eq!((m.step(t0 + ms(9000), true), m.shown), (Some(ms(100)), 1), "far behind: one step, counted from now");
        // 원본 그대로 읽은 작은 장들은 더 크게 읽을 것이 없다
        assert_eq!(m.have(), u32::MAX);
        assert_eq!(Moving::new(frames(2, &[100, 100]), 4).have(), 4);
    }
}
