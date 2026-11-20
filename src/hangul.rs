//! 한글 합치기: 자모로 나뉜 한글(NFD)을 완성된 글자(NFC)로 바꾼다.
//! 맥이 만든 파일 이름은 자모로 나뉘어 저장되는 일이 많고, 입력한 검색어는 완성된 글자라 그대로는 서로 맞지 않는다.
//! 맥의 검색 인덱스는 이름과 검색어를 모두 이것으로 합쳐 맞춘다. 한글 음절만 다루고 다른 글자는 그대로 둔다.

use std::borrow::Cow;

/// 첫소리 ᄀ (U+1100)부터 19개
const L_BASE: u32 = 0x1100;
const L_COUNT: u32 = 19;
/// 가운뎃소리 ᅡ (U+1161)부터 21개
const V_BASE: u32 = 0x1161;
const V_COUNT: u32 = 21;
/// 끝소리: T_BASE + 1 (ᆨ, U+11A8)부터 27개. T_BASE 자체는 '받침 없음'
const T_BASE: u32 = 0x11A7;
const T_COUNT: u32 = 28;
/// 완성된 글자 가 (U+AC00)부터 11172개
const S_BASE: u32 = 0xAC00;
const S_COUNT: u32 = L_COUNT * V_COUNT * T_COUNT;

/// 자모로 나뉜 한글을 완성된 글자로 합친다. 합칠 것이 없으면 복사하지 않고 그대로 돌려준다
pub fn compose(s: &str) -> Cow<'_, str> {
    if !has_jamo(s) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut held: Option<char> = None;
    for c in s.chars() {
        if let Some(prev) = held {
            if let Some(joined) = join(prev, c) {
                held = Some(joined);
                continue;
            }
            out.push(prev);
        }
        held = Some(c);
    }
    out.extend(held);
    Cow::Owned(out)
}

/// 한글 자모(U+1100~U+11FF)가 들어 있는가. UTF-8로 E1 84~87로 시작하는 세 바이트다
fn has_jamo(s: &str) -> bool {
    s.as_bytes().windows(2).any(|w| w[0] == 0xE1 && (0x84..=0x87).contains(&w[1]))
}

/// 첫소리+가운뎃소리 → 받침 없는 글자, 받침 없는 글자+끝소리 → 받침 있는 글자. 합칠 수 없으면 None
fn join(a: char, b: char) -> Option<char> {
    let (a, b) = (a as u32, b as u32);
    if (L_BASE..L_BASE + L_COUNT).contains(&a) && (V_BASE..V_BASE + V_COUNT).contains(&b) {
        return char::from_u32(S_BASE + ((a - L_BASE) * V_COUNT + (b - V_BASE)) * T_COUNT);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&a) && (a - S_BASE).is_multiple_of(T_COUNT) && (T_BASE + 1..T_BASE + T_COUNT).contains(&b) {
        return char::from_u32(a + (b - T_BASE));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 완성된 글자를 자모로 나눈다 (맥이 저장하는 꼴). 시험에서 나뉜 이름을 만들 때 쓴다
    fn split(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            let u = c as u32;
            if !(S_BASE..S_BASE + S_COUNT).contains(&u) {
                out.push(c);
                continue;
            }
            let i = u - S_BASE;
            out.push(char::from_u32(L_BASE + i / (V_COUNT * T_COUNT)).unwrap());
            out.push(char::from_u32(V_BASE + (i % (V_COUNT * T_COUNT)) / T_COUNT).unwrap());
            if !i.is_multiple_of(T_COUNT) {
                out.push(char::from_u32(T_BASE + i % T_COUNT).unwrap());
            }
        }
        out
    }

    #[test]
    fn joins_split_syllables() {
        let nfd = "\u{1107}\u{1169}\u{1100}\u{1169}\u{1109}\u{1165}.docx";
        assert_eq!(compose(nfd), "보고서.docx");
        // 받침이 있는 글자: 한 = ᄒ ᅡ ᆫ, 글 = ᄀ ᅳ ᆯ
        assert_eq!(compose("\u{1112}\u{1161}\u{11AB}\u{1100}\u{1173}\u{11AF}"), "한글");
        for word in ["주간보고 2026.xlsx", "제주살이 여행 일정", "똠방각하", "ㄱ자 모양", "빈 폴더", "읽ㅎ"] {
            assert_eq!(compose(&split(word)), word, "{word}");
        }
    }

    #[test]
    fn leaves_other_text_alone() {
        for s in ["report.pdf", "보고서.docx", "", "café", "日本語", "ㄱㄴㄷ"] {
            assert!(matches!(compose(s), Cow::Borrowed(_)), "{s} should not be copied");
            assert_eq!(compose(s), s);
        }
    }

    #[test]
    fn mixed_and_partial_input() {
        // 이미 완성된 글자 뒤의 나뉜 글자, 완성된 '하' + 끝소리 ᆫ
        assert_eq!(compose("보고\u{1109}\u{1165}"), "보고서");
        assert_eq!(compose("하\u{11AB}"), "한");
        // 받침이 이미 있는 글자에는 끝소리를 더 붙이지 않는다
        assert_eq!(compose("한\u{11AB}"), "한\u{11AB}");
        // 짝이 없는 첫소리·가운뎃소리는 그대로
        assert_eq!(compose("\u{1100}"), "\u{1100}");
        assert_eq!(compose("a\u{1161}b"), "a\u{1161}b");
        assert_eq!(compose("\u{1100}\u{1100}\u{1161}"), "\u{1100}가");
    }
}
