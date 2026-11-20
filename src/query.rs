//! 검색어 규칙 (Windows와 맥이 함께 쓴다). 한 조각을 이름과 경로에 맞춰 보는 것은 index::Query가 한다.
//! - `|`로 나눈 조각은 '또는'이다. 조각 안에 띄어 쓴 낱말은 모두 이름에 들어 있어야 한다
//! - 경로 구분자(Windows '\', 맥 '/')가 든 낱말은 전체 경로에서 찾는다
//! - 지금 폴더로 좁힐 때는 그 폴더의 경로를 조각마다 경로 조건으로 덧붙여 찾고(순위를 매기고 자르기 전에 걸러진다),
//!   받은 결과를 정확한 범위로 한 번 더 거른다
//! - 조각마다 찾은 결과는 하나로 합친다: 같은 항목은 가장 높은 점수로 한 번만 남는다

use std::collections::{HashMap, HashSet};

/// 검색어를 `|`로 나눈 조각들(또는). 조각 안의 띄어 쓴 낱말은 모두 포함해야 한다
pub fn pieces(q: &str) -> Vec<&str> {
    q.split('|').map(str::trim).filter(|p| !p.is_empty()).collect()
}

/// 같은 검색어 조각은 한 번만 찾는다 (엑셀 열에는 같은 값이 자주 되풀이된다).
/// 대소문자·띄어쓰기 차이만 있는 조각은 찾는 결과가 같으므로 같은 것으로 본다
pub fn unique_pieces(q: &str) -> Vec<&str> {
    let mut seen = HashSet::new();
    pieces(q).into_iter().filter(|p| seen.insert(p.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase())).collect()
}

/// 여러 줄 텍스트(엑셀 열 등)를 한 줄 검색어로: 줄마다 `|`로 잇는다
pub fn lines_to_query(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" | ")
}

/// 지금 폴더 범위를 검색어 조각마다 경로 조건으로 덧붙인다. 구분자(sep)가 든 낱말은 전체 경로에서 찾으며
/// 순위를 매기고 자르기 전에 거르므로, 다른 곳의 일치 항목이 많아도 이 폴더 아래 항목을 놓치지 않는다.
/// 경로를 띄어쓰기로 나눈 조각 가운데 구분자가 든 것만 쓴다(구분자가 없으면 이름 조건이 된다). 이 폴더 아래 경로는 이 조각들을
/// 늘 모두 품으므로 범위 안의 결과를 잃지 않고, 정확한 범위는 받은 뒤 under()로 거른다
pub fn scoped_query(q: &str, dir: &str, sep: char) -> String {
    let dir = format!("{}{sep}", dir.trim_end_matches(sep));
    let terms = dir.split_whitespace().filter(|w| w.contains(sep)).collect::<Vec<_>>().join(" ");
    pieces(q).iter().map(|p| format!("{p} {terms}")).collect::<Vec<_>>().join(" | ")
}

/// path가 dir 안에 있는가 (대소문자 무시)
pub fn under(path: &str, dir: &str, sep: char) -> bool {
    let (p, d) = (path.to_lowercase(), dir.trim_end_matches(sep).to_lowercase());
    p.len() > d.len() && p.starts_with(&d) && p[d.len()..].starts_with(sep)
}

/// 보이는 결과가 지금 것인지 가르는 열쇠: 검색어와 범위. 범위를 바꾸면 같은 검색어여도 이전 결과로는 열지 않는다
pub fn result_key(text: &str, scope: Option<&str>) -> String {
    match scope {
        Some(d) => format!("{text}\u{1F}{}", d.to_lowercase()),
        None => text.to_string(),
    }
}

/// 조각마다 찾은 결과를 합친다. 여러 조각에 걸린 항목은 가장 높은 점수로 한 번만 남는다
#[derive(Default)]
pub struct Merged {
    /// 소문자 경로 → all의 번호
    seen: HashMap<String, usize>,
    /// (점수, 폴더 여부, 경로)
    all: Vec<(i32, bool, String)>,
}

impl Merged {
    pub fn add(&mut self, score: i32, is_dir: bool, path: String) {
        let key = path.to_lowercase();
        match self.seen.get(&key) {
            Some(&i) => self.all[i].0 = self.all[i].0.max(score),
            None => {
                self.seen.insert(key, self.all.len());
                self.all.push((score, is_dir, path));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.all.len()
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// 점수가 높은 것부터(같으면 짧은 경로, 그다음 자연 정렬) limit개: (폴더 여부, 경로)
    pub fn into_sorted(mut self, limit: usize) -> Vec<(bool, String)> {
        self.all.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.len().cmp(&b.2.len())).then_with(|| crate::favorites::natural(&a.2, &b.2)));
        self.all.truncate(limit);
        self.all.into_iter().map(|(_, d, p)| (d, p)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 지금 폴더 범위는 검색어 조각마다 경로 조건(구분자가 든 낱말)으로 붙는다. 띄어쓰기 뒤 구분자가 없는 조각은 빼고(이름 조건이 되므로)
    /// 정확한 범위는 받은 뒤 거른다
    #[test]
    fn scope_becomes_path_terms() {
        assert_eq!(scoped_query("보고서", r"C:\업무\월간", '\\'), r"보고서 C:\업무\월간\");
        assert_eq!(scoped_query("보고서", r"C:\업무\월간\", '\\'), r"보고서 C:\업무\월간\");
        assert_eq!(scoped_query("a | b c", r"C:\Sakadu 데모\사진", '\\'), r"a C:\Sakadu 데모\사진\ | b c C:\Sakadu 데모\사진\");
        assert_eq!(scoped_query("x", r"C:\My Big Folder\월간", '\\'), r"x C:\My Folder\월간\");
        assert_eq!(scoped_query("x", r"\\nas\공유\팀", '\\'), r"x \\nas\공유\팀\");
        assert_eq!(scoped_query("x", r"C:\", '\\'), r"x C:\");
        // 맥 경로
        assert_eq!(scoped_query("보고 | 회의", "/Users/hong/My Docs", '/'), "보고 /Users/hong/My Docs/ | 회의 /Users/hong/My Docs/");
        assert_eq!(scoped_query("x", "/", '/'), "x /");
        // 범위가 다르면 같은 검색어여도 다른 결과로 본다
        assert_ne!(result_key("보고", Some(r"C:\a")), result_key("보고", None));
        assert_ne!(result_key("보고", Some(r"C:\a")), result_key("보고", Some(r"C:\b")));
        assert_eq!(result_key("보고", Some(r"C:\A")), result_key("보고", Some(r"c:\a")));
    }

    #[test]
    fn pieces_split_on_bar() {
        assert_eq!(pieces("견적 2026 | 계약서 |  | 0420"), vec!["견적 2026", "계약서", "0420"]);
        assert_eq!(pieces("업무\\ 보고"), vec!["업무\\ 보고"]);
        assert!(pieces(" | ").is_empty());
    }

    #[test]
    fn repeated_pieces_are_searched_once() {
        assert_eq!(unique_pieces("견적 | 계약 | 견적 |  견적  | 계약서 | 견적"), vec!["견적", "계약", "계약서"]);
        assert_eq!(unique_pieces("견적  2026 | 견적 2026 | ABC | abc"), vec!["견적  2026", "ABC"]);
    }

    #[test]
    fn scope_keeps_only_paths_under_the_folder() {
        assert!(under("C:\\업무\\견적.xlsx", "c:\\업무", '\\'));
        assert!(under("C:\\OneDrive - Personal\\a b\\x.txt", "C:\\OneDrive - Personal\\a b", '\\'));
        assert!(under("C:\\업무\\2026\\견적.xlsx", "C:\\업무\\", '\\'));
        assert!(!under("C:\\업무2\\견적.xlsx", "C:\\업무", '\\'));
        assert!(!under("C:\\업무", "C:\\업무", '\\'));
        assert!(under("/Users/hong/문서/a.txt", "/users/hong/문서", '/'));
        assert!(under("/Users/hong/a.txt", "/", '/'));
        assert!(!under("/Users/hong/문서2/a.txt", "/Users/hong/문서", '/'));
    }

    #[test]
    fn pasted_lines_become_alternatives() {
        let excel = "20260420_160559\r\n20260407_091344\r\n\r\n  견적서 \r\n";
        assert_eq!(lines_to_query(excel), "20260420_160559 | 20260407_091344 | 견적서");
    }

    /// 여러 조각에 걸린 항목은 가장 높은 점수로 한 번만, 순서는 점수 · 짧은 경로 · 자연 정렬
    #[test]
    fn merged_keeps_each_item_once_with_its_best_score() {
        let mut m = Merged::default();
        m.add(10, false, "/a/보고서 10.txt".into());
        m.add(10, false, "/a/보고서 9.txt".into());
        m.add(5, true, "/b/긴 폴더 이름".into());
        m.add(30, true, "/B/긴 폴더 이름".into());
        m.add(10, false, "/a/b/c/보고서.txt".into());
        assert_eq!(m.len(), 4);
        let out = m.into_sorted(3);
        assert_eq!(out, vec![(true, "/b/긴 폴더 이름".to_string()), (false, "/a/보고서 9.txt".to_string()), (false, "/a/보고서 10.txt".to_string())]);
    }
}
