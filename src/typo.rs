//! 글자 단계와 글자 배치 규칙 (스타일 시트 2장 '글자', docs/sakadu-style-sheet.md)
//!
//! - 1~4장은 OS와 상관없는 공통 코드다: 글자 단계, 고정 줄 높이 계산, 파일 이름 가운데 줄임, 날짜 열.
//! - 5장은 Windows(DirectWrite) 도우미로, `src/gfx.rs`가 쓴다. 6장은 macOS(`src/mac/gfx.rs`)를 위한 메모다.
//!
//! 의존성: `unicode-segmentation = "1"`
//! (맥 파일 이름은 한글이 자모로 풀린 NFD로 오는 경우가 많아서,
//!  글자 단위로 자르면 '가'가 'ㄱ' + 'ㅏ'로 쪼개진다. 반드시 글자 덩어리(grapheme) 단위로 자른다.)

use std::borrow::Cow;
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

// ─────────────────────────────────────────────────────────────
// 1. 글자 단계
// ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeStyle {
    /// DIP (맥은 point)
    pub size: f32,
    /// 400 또는 600만 쓴다
    pub weight: u16,
    /// 고정폭 숫자(OpenType `tnum`)
    pub tnum: bool,
    pub align: Align,
}

pub mod style {
    use super::{Align, TypeStyle};

    /// 그대로: 작업 공간 이름, 설정 쪽 제목
    pub const TITLE: TypeStyle = TypeStyle { size: 15.0, weight: 600, tnum: false, align: Align::Left };
    /// 그대로: 파일 이름, 탭, 메뉴, 본문
    pub const NORMAL: TypeStyle = TypeStyle { size: 13.0, weight: 400, tnum: false, align: Align::Left };
    /// 새로: 파일 이름. 모양은 Normal과 같고, 넘치면 middle_ellipsis로 가운데를 줄인다.
    pub const NAME: TypeStyle = NORMAL;
    /// 그대로. 쓰는 곳 추가: 사이드바 그룹 머리('월간', '서비스지원')
    pub const STRONG: TypeStyle = TypeStyle { size: 13.0, weight: 600, tnum: false, align: Align::Left };
    /// 새로: 날짜 열. 색은 `dim`. 지금은 Normal을 쓰고 있다.
    pub const META: TypeStyle = TypeStyle { size: 13.0, weight: 400, tnum: true, align: Align::Left };
    /// 바뀜: 지금의 `Right`(크기 열)에 고정폭 숫자를 켠 것
    pub const META_RIGHT: TypeStyle = TypeStyle { align: Align::Right, ..META };
    /// 바뀜: 개수('11개'), 진행 수치처럼 숫자가 바뀌는 곳은 고정폭 숫자
    pub const SMALL: TypeStyle = TypeStyle { size: 12.0, weight: 400, tnum: true, align: Align::Left };
    /// 그대로: 썸네일 아래 이름
    pub const CAPTION: TypeStyle = TypeStyle { size: 12.0, weight: 400, tnum: false, align: Align::Center };
    /// 바뀜: 11/400 → 12/600. 색은 `faint` 그대로.
    /// 11px 한글은 맑은 고딕으로 대체되며 획이 뭉개지기 쉽고, Windows 지침의 최소 크기(12)보다 작다.
    /// 크기를 키운 대신 굵기로 Small(12/400)과 구분한다.
    pub const LABEL: TypeStyle = TypeStyle { size: 12.0, weight: 600, tnum: false, align: Align::Left };
}

// ─────────────────────────────────────────────────────────────
// 2. 고정 줄 높이와 기준선
// ─────────────────────────────────────────────────────────────

/// 주 글꼴(Segoe UI Variable Text)의 ascent/descent(em 비율)로 고정 줄 높이와 기준선을 낸다.
///
/// DirectWrite 기본 줄 간격은 '내용에 맞춤'이라서, 한글처럼 대체 글꼴(맑은 고딕)이 끼면
/// 그 글꼴의 지표가 줄 높이에 섞인다. 그러면 한글 이름 줄과 영문 이름 줄의 기준선이
/// 1px 안팎으로 어긋나 목록이 출렁여 보인다. 주 글꼴 지표만으로 고정해서 막는다.
///
/// 반환: (줄 높이, 줄 위에서 기준선까지)
pub fn uniform_line(size: f32, ascent_em: f32, descent_em: f32) -> (f32, f32) {
    let asc = ascent_em * size;
    let line = (asc + descent_em * size).ceil();
    (line, asc.round())
}

/// 줄(ROW_H) 안에서 글자 상자를 세로 가운데에 둘 y
pub fn text_top_in_row(row_h: f32, line: f32) -> f32 {
    ((row_h - line) / 2.0).round()
}

// ─────────────────────────────────────────────────────────────
// 3. 파일 이름 가운데 줄임
// ─────────────────────────────────────────────────────────────

pub const ELLIPSIS: &str = "…";
/// 확장자 앞에서 같이 남길 글자 덩어리 수 ('…집계표.xlsx')
const KEEP_BEFORE_EXT: usize = 3;
/// 확장자가 없을 때 끝에서 남길 수 ('…수정본(2)')
const KEEP_NO_EXT: usize = 5;
/// 앞부분을 이보다 적게 남겨야 한다면 가운데 줄임을 포기하고 끝 줄임으로 물러선다.
/// 좁아질수록 앞부분을 먼저 줄여서, 확장자가 최대한 오래 보이게 한다.
const MIN_HEAD: usize = 2;
/// 이보다 긴 '.뒤'는 확장자로 보지 않는다 ('보고서.최종검토버전입니다')
const MAX_EXT: usize = 8;

/// 화면에 보일 파일 이름: 자모로 나뉜 한글(맥에서 만든 NFD 이름)을 완성된 글자로 합친다.
/// Windows 글꼴은 나뉜 자모를 한 글자로 합쳐 그리지 않아 'ㄱㅏ'처럼 풀려 보이기 때문이다.
/// 그리기에만 쓴다: 파일 이름 자체(열기·이름 바꾸기·복사에 쓰는 이름)는 바꾸지 않는다
pub fn display_name(raw: &str) -> Cow<'_, str> {
    crate::hangul::compose(raw)
}

/// 파일 이름 줄임 결과 캐시: (보일 이름, 폭) → (보일 글자, 줄였는지).
/// 보이는 줄만 계산하므로 스크롤할 때 다시 재지 않는다. 글꼴·배율·글자 그리기 방식이 바뀌면 비운다
#[derive(Default)]
pub struct NameCache {
    map: HashMap<(String, i32), (String, bool)>,
}

impl NameCache {
    /// 이만큼 쌓이면 비운다 (아주 큰 폴더를 오래 스크롤해도 메모리가 늘지 않게)
    const MAX: usize = 4096;

    pub fn fit(&mut self, name: &str, max_w: f32, measure: impl Fn(&str) -> f32) -> (String, bool) {
        // 폭은 0.25 DIP 단위로 묶는다 (창 크기를 바꾸는 동안 한 칸 폭마다 따로 쌓이지 않게)
        let key = (name.to_string(), (max_w * 4.0).floor() as i32);
        if let Some(hit) = self.map.get(&key) {
            return hit.clone();
        }
        let out = fit_name(name, max_w, measure);
        if self.map.len() >= Self::MAX {
            self.map.clear();
        }
        self.map.insert(key, out.clone());
        out
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

/// 파일 이름을 폭에 맞춘 글자와 줄였는지 (display_name을 거친 이름을 넘긴다)
pub fn fit_name(name: &str, max_w: f32, measure: impl Fn(&str) -> f32) -> (String, bool) {
    match middle_ellipsis(name, max_w, measure) {
        Cow::Borrowed(s) => (s.to_string(), false),
        Cow::Owned(s) => (s, true),
    }
}

/// 끝까지 남길 꼬리가 시작하는 글자 덩어리 번호
fn tail_start(name: &str, g: &[(usize, &str)]) -> usize {
    let n = g.len();
    if let Some(dot) = name.rfind('.') {
        let ext = &name[dot + 1..];
        let ext_len = ext.graphemes(true).count();
        // dot > 0: '.gitignore' 같은 숨김 파일은 확장자로 보지 않는다
        if dot > 0 && (1..=MAX_EXT).contains(&ext_len) && !ext.contains(' ') {
            let dot_g = g.iter().position(|&(i, _)| i == dot).unwrap_or(n);
            return dot_g.saturating_sub(KEEP_BEFORE_EXT);
        }
    }
    n.saturating_sub(KEEP_NO_EXT)
}

/// 넘치는 파일 이름의 가운데를 줄이고 확장자를 남긴다.
///
/// `measure`는 지금 글자 단계로 문자열 폭(DIP)을 재는 함수다.
/// DirectWrite에서는 `IDWriteTextLayout::GetMetrics().widthIncludingTrailingWhitespace`.
/// 이진 탐색이라 긴 이름도 레이아웃을 열 번 안팎만 만든다. 보이는 줄만 계산하고,
/// (이름, 열 폭) 쌍으로 결과를 캐시해 두면 스크롤할 때 다시 잴 일이 없다.
///
/// 줄였다면 마우스를 올렸을 때 전체 이름을 툴팁으로 보여 준다.
pub fn middle_ellipsis<'a>(name: &'a str, max_w: f32, measure: impl Fn(&str) -> f32) -> Cow<'a, str> {
    if measure(name) <= max_w {
        return Cow::Borrowed(name);
    }
    let g: Vec<(usize, &str)> = name.grapheme_indices(true).collect();
    let n = g.len();
    let byte = |k: usize| if k >= n { name.len() } else { g[k].0 };
    let ts = tail_start(name, &g);
    let tail = &name[byte(ts)..];
    let with_tail = |k: usize| format!("{}{}{}", name[..byte(k)].trim_end(), ELLIPSIS, tail);

    if ts >= MIN_HEAD && measure(&with_tail(MIN_HEAD)) <= max_w {
        // lo는 들어가는 것이 확인된 앞부분 길이
        let (mut lo, mut hi) = (MIN_HEAD, ts);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if measure(&with_tail(mid)) <= max_w {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        return Cow::Owned(with_tail(lo));
    }
    Cow::Owned(end_ellipsis(name, &g, max_w, &measure))
}

/// 꼬리를 지킬 자리조차 없을 때 쓰는 끝 줄임
fn end_ellipsis(name: &str, g: &[(usize, &str)], max_w: f32, measure: &impl Fn(&str) -> f32) -> String {
    let n = g.len();
    let byte = |k: usize| if k >= n { name.len() } else { g[k].0 };
    let cand = |k: usize| format!("{}{}", name[..byte(k)].trim_end(), ELLIPSIS);
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure(&cand(mid)) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    cand(lo)
}

// ─────────────────────────────────────────────────────────────
// 4. 날짜 열
// ─────────────────────────────────────────────────────────────

/// 날짜 열 폭. 고정폭 숫자를 켜면 모든 날짜가 같은 폭이 되므로
/// '0'으로 채운 문자열 하나만 재면 된다. 글자 단계·배율이 바뀔 때만 다시 잰다.
/// 남는 폭은 전부 이름 열에 준다.
pub fn date_col_width(measure_tnum: impl Fn(&str) -> f32, pad: f32) -> f32 {
    (measure_tnum("0000-00-00 00:00") + pad).ceil()
}

#[derive(Clone, Copy, Debug)]
pub struct DateParts {
    pub y: i32,
    pub mo: u32,
    pub d: u32,
    pub h: u32,
    pub mi: u32,
}

/// 열이 좁아도 날짜를 '…'로 자르지 않고 더 짧은 꼴로 바꾼다.
/// 2026-11-03 15:18 → 26-11-03 15:18 → 오늘 15:18 또는 26-11-03 → 11-03
pub fn fit_date(dt: DateParts, today: (i32, u32, u32), avail: f32, measure: impl Fn(&str) -> f32) -> String {
    let is_today = (dt.y, dt.mo, dt.d) == today;
    let yy = dt.y.rem_euclid(100);
    let mut cands = vec![
        format!("{:04}-{:02}-{:02} {:02}:{:02}", dt.y, dt.mo, dt.d, dt.h, dt.mi),
        format!("{:02}-{:02}-{:02} {:02}:{:02}", yy, dt.mo, dt.d, dt.h, dt.mi),
    ];
    if is_today {
        cands.push(format!("오늘 {:02}:{:02}", dt.h, dt.mi));
    } else {
        cands.push(format!("{:02}-{:02}-{:02}", yy, dt.mo, dt.d));
    }
    cands.push(format!("{:02}-{:02}", dt.mo, dt.d));
    let last = cands.len() - 1;
    for (i, c) in cands.iter().enumerate() {
        if i == last || measure(c) <= avail {
            return c.clone();
        }
    }
    unreachable!()
}

// ─────────────────────────────────────────────────────────────
// 5. DirectWrite (Windows) — src/gfx.rs로 옮길 부분
// ─────────────────────────────────────────────────────────────
//
// windows 크레이트 0.62 기준.

#[cfg(windows)]
pub mod dwrite {
    use windows::core::{Interface, Result, PCWSTR};
    use windows::Win32::Graphics::DirectWrite::*;

    /// 고정폭 숫자 묶음. 한 번 만들어 두고 날짜·크기·개수 레이아웃마다 붙인다.
    pub fn tabular_typography(factory: &IDWriteFactory) -> Result<IDWriteTypography> {
        unsafe {
            let t = factory.CreateTypography()?;
            t.AddFontFeature(DWRITE_FONT_FEATURE {
                nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
                parameter: 1,
            })?;
            Ok(t)
        }
    }

    pub fn apply_tabular(layout: &IDWriteTextLayout, t: &IDWriteTypography, len_utf16: u32) -> Result<()> {
        unsafe { layout.SetTypography(t, DWRITE_TEXT_RANGE { startPosition: 0, length: len_utf16 }) }
    }

    /// 주 글꼴 지표 → (ascent_em, descent_em). super::uniform_line에 넘긴다.
    pub fn font_em_metrics(face: &IDWriteFontFace) -> (f32, f32) {
        let mut m = DWRITE_FONT_METRICS::default();
        unsafe { face.GetMetrics(&mut m) };
        let upm = m.designUnitsPerEm as f32;
        (m.ascent as f32 / upm, m.descent as f32 / upm)
    }

    /// 글자 단계마다 TextFormat을 만들 때 한 번 호출한다.
    pub fn fix_line(format: &IDWriteTextFormat, line: f32, baseline: f32) -> Result<()> {
        unsafe { format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, baseline) }
    }

    /// 한글 자모·완성형 범위. 1100–11FF는 맥에서 온 NFD 파일 이름 때문에 꼭 넣는다.
    pub const HANGUL: [DWRITE_UNICODE_RANGE; 5] = [
        DWRITE_UNICODE_RANGE { first: 0x1100, last: 0x11FF },
        DWRITE_UNICODE_RANGE { first: 0x3130, last: 0x318F },
        DWRITE_UNICODE_RANGE { first: 0xA960, last: 0xA97F },
        DWRITE_UNICODE_RANGE { first: 0xAC00, last: 0xD7A3 },
        DWRITE_UNICODE_RANGE { first: 0xD7B0, last: 0xD7FF },
    ];

    /// Segoe UI Variable에는 한글이 없어서 한글은 늘 대체 글꼴로 그려진다.
    /// 시스템에 맡기지 말고 어떤 글꼴을 쓸지 정해 둔다. 나머지는 시스템 대체 규칙을 그대로 잇는다.
    ///
    /// - family: "Malgun Gothic"(기본, 설치 필요 없음) 또는 앱에 넣은 "Pretendard" 등
    /// - scale: 한글이 라틴보다 커 보이면 0.96~0.98처럼 살짝 줄인다. 눈으로 맞출 값.
    ///
    /// 맑은 고딕은 굵기가 350·400·700뿐이라 600을 요청하면 700으로 그려진다.
    /// 한글·영문이 섞인 Title/Strong에서 영문은 600, 한글은 700이 되는 이유다.
    pub fn hangul_fallback(factory: &IDWriteFactory, family: PCWSTR, scale: f32) -> Result<IDWriteFontFallback> {
        unsafe {
            let f2: IDWriteFactory2 = factory.cast()?;
            let b = f2.CreateFontFallbackBuilder()?;
            let names = [family.as_ptr()];
            b.AddMapping(&HANGUL, &names, None::<&IDWriteFontCollection>, PCWSTR::null(), PCWSTR::null(), scale)?;
            b.AddMappings(&f2.GetSystemFontFallback()?)?;
            b.CreateFontFallback()
        }
    }

    /// TextFormat마다 붙인다 (IDWriteTextFormat1, Windows 8.1+)
    pub fn use_fallback(format: &IDWriteTextFormat, fb: &IDWriteFontFallback) -> Result<()> {
        unsafe {
            let f1: IDWriteTextFormat1 = format.cast()?;
            f1.SetFontFallback(fb)
        }
    }
}

// ─────────────────────────────────────────────────────────────
// 6. Core Text (macOS) 메모 — src/mac/gfx.rs
// ─────────────────────────────────────────────────────────────
//
// - 고정폭 숫자: 시스템 글꼴(SF)도 기본 숫자가 비례폭이다.
//   CTFontDescriptor에 kCTFontFeatureSettingsAttribute로
//   { kCTFontOpenTypeFeatureTag: "tnum", kCTFontOpenTypeFeatureValue: 1 }을 넣거나,
//   AppKit 쪽이라면 NSFont::monospacedDigitSystemFontOfSize_weight를 쓴다.
// - 한글은 Apple SD Gothic Neo로 대체된다. 이 글꼴은 SemiBold가 있어서 600이 그대로 600이다.
// - 줄 높이: CTLine의 typographic bounds 대신 주 글꼴의 ascent로 기준선을 고정하고 그린다
//   (2장 uniform_line과 같은 생각).
// - 파일 이름 NFD: 3장 middle_ellipsis는 글자 덩어리 단위라 그대로 쓴다.
//   화면에 그리기 전에 NFC로 바꾸지는 않는다(이름을 바꿔 저장하는 일과 섞이지 않게).

// ─────────────────────────────────────────────────────────────
// 시험
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 가짜 폭: 한글·한자 등 넓은 글자 덩어리 1.0, 그 밖 0.55
    fn w(s: &str) -> f32 {
        s.graphemes(true)
            .map(|g| {
                let c = g.chars().next().unwrap();
                if (c as u32) >= 0x1100 { 1.0 } else { 0.55 }
            })
            .sum()
    }

    #[test]
    fn short_name_unchanged() {
        assert!(matches!(middle_ellipsis("2026", 20.0, w), Cow::Borrowed("2026")));
    }

    #[test]
    fn keeps_extension_and_fits() {
        let name = "20261124 - 프리미어 서비스 고객별 매출 집계표.xlsx";
        let out = middle_ellipsis(name, 16.0, w);
        assert!(out.ends_with("집계표.xlsx"), "{out}");
        assert!(out.contains(ELLIPSIS));
        assert!(w(&out) <= 16.0, "{out} {}", w(&out));
        assert!(out.starts_with("20261124"), "{out}");
    }

    #[test]
    fn xlsm_vs_xlsx_stay_distinct() {
        let a = middle_ellipsis("20261124 - 일일 테크니션 매출보고.xlsm", 12.0, w);
        let b = middle_ellipsis("20261124 - 일일 테크니션 매출보고.xlsx", 12.0, w);
        assert_ne!(a, b);
        assert!(a.ends_with(".xlsm") && b.ends_with(".xlsx"));
    }

    #[test]
    fn no_extension_keeps_tail() {
        let out = middle_ellipsis("2026년 11월 월마감 체크리스트_최종_수정본(2)", 14.0, w);
        assert!(out.ends_with("정본(2)"), "{out}");
        assert!(w(&out) <= 14.0);
    }

    #[test]
    fn hidden_file_is_not_extension() {
        let out = middle_ellipsis(".gitignore_backup_from_old_machine", 10.0, w);
        assert!(out.ends_with("chine"), "{out}");
    }

    #[test]
    fn nfd_hangul_never_splits_syllables() {
        // '가나다라마바사아자차카타파하' 를 자모로 푼 NFD + 확장자
        let syll = "가나다라마바사아자차카타파하";
        let nfd: String = syll
            .chars()
            .flat_map(|c| {
                let s = c as u32 - 0xAC00;
                let (l, v, t) = (s / 588, (s % 588) / 28, s % 28);
                let mut v2 = vec![char::from_u32(0x1100 + l).unwrap(), char::from_u32(0x1161 + v).unwrap()];
                if t > 0 {
                    v2.push(char::from_u32(0x11A7 + t).unwrap());
                }
                v2
            })
            .collect::<String>()
            + ".txt";
        let out = middle_ellipsis(&nfd, 9.0, w);
        for g in out.graphemes(true) {
            let c = g.chars().next().unwrap() as u32;
            let starts_with_vowel_or_final = (0x1161..=0x11FF).contains(&c);
            assert!(!starts_with_vowel_or_final, "쪼개진 자모: {out:?}");
        }
        assert!(out.ends_with(".txt"));
    }

    #[test]
    fn very_narrow_falls_back_to_end() {
        let out = middle_ellipsis("20261124 - 프리미어 서비스 고객별 매출 집계표.xlsx", 4.0, w);
        assert!(out.ends_with(ELLIPSIS), "{out}");
        assert!(w(&out) <= 4.0);
        let none = middle_ellipsis("가나다라", 0.5, w);
        assert_eq!(none, ELLIPSIS);
    }

    #[test]
    fn display_name_joins_jamo_only_for_drawing() {
        let nfd = "\u{1100}\u{1161}\u{11A8}.txt"; // '각'을 자모로 푼 이름
        assert_eq!(display_name(nfd), "각.txt");
        assert!(matches!(display_name("abc.txt"), Cow::Borrowed(_)));
    }

    #[test]
    fn name_cache_remembers_and_reports_cut() {
        let mut c = NameCache::default();
        let calls = std::cell::Cell::new(0);
        let m = |s: &str| {
            calls.set(calls.get() + 1);
            w(s)
        };
        let long = "20261124 - 프리미어 서비스 고객별 매출 집계표.xlsx";
        let (a, cut) = c.fit(long, 16.0, m);
        assert!(cut && a.ends_with("집계표.xlsx"));
        let n = calls.get();
        assert_eq!(c.fit(long, 16.0, m), (a, true));
        assert_eq!(calls.get(), n, "second fit comes from the cache");
        assert_eq!(c.fit("짧은.txt", 16.0, m), ("짧은.txt".to_string(), false));
    }

    #[test]
    fn date_shrinks_instead_of_cutting() {
        let dt = DateParts { y: 2026, mo: 11, d: 3, h: 15, mi: 18 };
        let today = (2026, 11, 25);
        assert_eq!(fit_date(dt, today, 99.0, w), "2026-11-03 15:18");
        assert_eq!(fit_date(dt, today, 8.0, w), "26-11-03 15:18");
        assert_eq!(fit_date(dt, today, 5.0, w), "26-11-03");
        assert_eq!(fit_date(dt, today, 1.0, w), "11-03");
        let now = DateParts { y: 2026, mo: 11, d: 25, h: 9, mi: 6 };
        assert_eq!(fit_date(now, today, 6.0, w), "오늘 09:06");
    }

    #[test]
    fn line_metrics_are_stable() {
        // Segoe UI 계열 지표 근사(ascent 1.08, descent 0.25 em). 실제 값은 font_em_metrics로 읽는다.
        let (line, base) = uniform_line(13.0, 1.08, 0.25);
        assert_eq!((line, base), (18.0, 14.0));
        assert_eq!(text_top_in_row(28.0, line), 5.0);
        assert_eq!(style::META_RIGHT.align, Align::Right);
        const { assert!(style::META_RIGHT.tnum && !style::NORMAL.tnum) };
    }
}
