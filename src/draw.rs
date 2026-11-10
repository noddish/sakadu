//! 그리기 기본 타입: 사각형, 글꼴 종류, 색. 그리는 방법(Windows Direct2D, 맥 Core Graphics)과 상관없이 같은 값을 쓴다.
//! 좌표는 모두 논리 단위(Windows DIP, 맥 pt)다

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct R {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
}

impl R {
    pub const fn new(l: f32, t: f32, r: f32, b: f32) -> R {
        R { l, t, r, b }
    }
    pub fn w(&self) -> f32 {
        self.r - self.l
    }
    pub fn h(&self) -> f32 {
        self.b - self.t
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }
    pub fn inset(&self, dx: f32, dy: f32) -> R {
        R::new(self.l + dx, self.t + dy, self.r - dx, self.b - dy)
    }
}

#[derive(Clone, Copy)]
pub enum Font {
    Normal = 0,
    Right,
    Strong,
    Glyph,
    GlyphSmall,
    Hint,
    /// 경로용: 넘치면 중간을 줄이고 마지막 폴더 이름을 남긴다 ("C:\Users\…\일간 업무")
    Path,
    /// 썸네일 아래 이름: 가운데 정렬, 두 줄까지 줄바꿈 후 말줄임
    Caption,
    /// 텍스트 파일 미리 보기: 고정폭, 줄바꿈, 위쪽 정렬
    Mono,
    /// 설정 창 쪽 제목
    Title,
    /// 설명·작은 안내
    Small,
    /// 이동 화살표(뒤로·앞으로·위로)용 아이콘: 도구 아이콘(Glyph, 16px)과 장식(GlyphSmall, 10px) 사이
    GlyphMid,
}

/// 색 (각 성분 0~1, 알파는 미리 곱하지 않음). Direct2D `D2D1_COLOR_F`와 같은 배치라 Windows에서는 그대로 바꿔 넘긴다
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}
