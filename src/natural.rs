//! 이름 자연 정렬: 이어진 숫자는 값으로 비교하고("file2" < "file10") 대소문자는 가리지 않는다.
//! Windows에서는 탐색기와 순서가 같도록 StrCmpLogicalW를 그대로 쓰고, 다른 OS에서는 같은 규칙을 직접 따른다.

use std::cmp::Ordering;

/// a, b는 끝에 NUL이 있는 UTF-16 (listing::Entry::name_z, util::wide)
pub fn cmp_z(a: &[u16], b: &[u16]) -> Ordering {
    debug_assert!(a.last() == Some(&0) && b.last() == Some(&0));
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::StrCmpLogicalW;
        use windows::core::PCWSTR;
        unsafe { StrCmpLogicalW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr())) }.cmp(&0)
    }
    #[cfg(not(windows))]
    {
        portable(until_nul(a), until_nul(b))
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn until_nul(w: &[u16]) -> &[u16] {
    &w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())]
}

/// StrCmpLogicalW의 큰 규칙만 따른다: 이어진 숫자는 값으로(앞의 0은 무시), 나머지 글자는 대소문자 없이 하나씩.
/// 기호의 순서처럼 Windows 언어별 비교에 맡기던 세세한 부분은 다를 수 있다
#[cfg_attr(windows, allow(dead_code))]
fn portable(a: &[u16], b: &[u16]) -> Ordering {
    let chars = |w: &[u16]| char::decode_utf16(w.iter().copied()).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect::<Vec<_>>();
    let (a, b) = (chars(a), chars(b));
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let ord = if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let trim = |d: &[char]| -> Vec<char> { d.iter().copied().skip_while(|&c| c == '0').collect() };
            let (na, nb) = (trim(&a[si..i]), trim(&b[sj..j]));
            na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb))
        } else {
            let ord = a[i].to_lowercase().cmp(b[j].to_lowercase());
            i += 1;
            j += 1;
            ord
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::wide;

    fn cmp(a: &str, b: &str) -> Ordering {
        cmp_z(&wide(a), &wide(b))
    }

    const SAMPLE: [&str; 14] = [
        "file10.txt", "File3.txt", "file2.txt", "file1.txt", "10.월간", "2.주간", "보고서 10", "보고서 2", "나", "가나", "abcd", "abc", "B", "a",
    ];

    #[test]
    fn numbers_by_value_and_case_ignored() {
        assert_eq!(cmp("file2", "file10"), Ordering::Less);
        assert_eq!(cmp("2.주간", "10.월간"), Ordering::Less);
        assert_eq!(cmp("a", "B"), Ordering::Less);
        assert_eq!(cmp("Readme", "README"), Ordering::Equal);
        assert_eq!(cmp("abc", "abcd"), Ordering::Less);
        let mut v = SAMPLE.to_vec();
        v.sort_by(|a, b| cmp(a, b));
        assert_eq!(
            v,
            ["2.주간", "10.월간", "a", "abc", "abcd", "B", "file1.txt", "file2.txt", "File3.txt", "file10.txt", "가나", "나", "보고서 2", "보고서 10"]
        );
    }

    /// 다른 OS에서 쓰는 직접 구현이 탐색기 순서(StrCmpLogicalW)와 같은지
    #[cfg(windows)]
    #[test]
    fn portable_matches_explorer_order() {
        for a in SAMPLE {
            for b in SAMPLE {
                let (wa, wb) = (wide(a), wide(b));
                assert_eq!(portable(until_nul(&wa), until_nul(&wb)), cmp_z(&wa, &wb), "{a} vs {b}");
            }
        }
    }
}
