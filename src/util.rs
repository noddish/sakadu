//! 문자열·경로 유틸리티 (Win32 의존 없음)

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// 경로의 루트 길이. "C:\" → 3, "\\server\share\x" → "\\server\share"의 길이
pub fn root_len(path: &str) -> usize {
    let b = path.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return if b.len() >= 3 && b[2] == b'\\' { 3 } else { 2 };
    }
    if let Some(rest) = path.strip_prefix("\\\\") {
        let mut seps = 0;
        for (i, ch) in rest.char_indices() {
            if ch == '\\' {
                seps += 1;
                if seps == 2 {
                    return 2 + i;
                }
            }
        }
        return path.len();
    }
    0
}

/// 사용자 입력 경로 정규화: 따옴표 제거, '/'→'\', 드라이브 문자 대문자, 루트가 아닌 끝 '\' 제거
pub fn normalize(path: &str) -> String {
    let mut s = path.trim().trim_matches('"').replace('/', "\\");
    if s.len() >= 2 && s.as_bytes()[1] == b':' && s.as_bytes()[0].is_ascii_alphabetic() {
        let up = s[..1].to_ascii_uppercase();
        s.replace_range(..1, &up);
        if s.len() == 2 {
            s.push('\\');
        }
    }
    while s.len() > root_len(&s).max(1) && s.ends_with('\\') {
        s.pop();
    }
    s
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') { format!("{dir}{name}") } else { format!("{dir}\\{name}") }
}

/// 상위 폴더. 루트면 None
pub fn parent(path: &str) -> Option<String> {
    let r = root_len(path);
    if r == 0 || path.len() <= r {
        return None;
    }
    let idx = path.rfind('\\')?;
    let root = if r == 2 { format!("{}\\", &path[..2]) } else { path[..r].to_string() };
    if idx < r { Some(root) } else { Some(path[..idx].to_string()) }
}

/// 마지막 구성 요소. 루트면 루트 표시("C:", "\\server\share")
pub fn file_name(path: &str) -> &str {
    let r = root_len(path);
    if path.len() <= r {
        return path.trim_end_matches('\\');
    }
    path[r..].rsplit('\\').find(|s| !s.is_empty()).unwrap_or("")
}

/// 경로 표시줄용 (표시 이름, 전체 경로) 목록
pub fn segments(path: &str) -> Vec<(String, String)> {
    let r = root_len(path);
    let root = &path[..r];
    let mut cur = if root.len() == 2 { format!("{root}\\") } else { root.to_string() };
    let mut out = vec![(root.trim_end_matches('\\').to_string(), cur.clone())];
    for part in path[r..].split('\\').filter(|p| !p.is_empty()) {
        cur = join(&cur, part);
        out.push((part.to_string(), cur.clone()));
    }
    out
}

pub fn eq_path(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

/// 파일 이름에 쓸 수 없는 문자가 있으면 그 문자를 돌려준다
pub fn invalid_name_char(name: &str) -> Option<char> {
    name.chars().find(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (*c as u32) < 0x20)
}

/// 파일·폴더 이름으로 쓸 수 없으면 그 까닭 (쓸 수 없는 문자, Windows 예약 이름)
pub fn name_problem(name: &str) -> Option<String> {
    if let Some(c) = invalid_name_char(name) {
        return Some(if (c as u32) < 0x20 { "이름에 쓸 수 없는 제어 문자가 들어 있어요".to_string() } else { format!("이름에는 {c} 문자를 쓸 수 없어요") });
    }
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let numbered = |p: &str| stem.len() == 4 && stem.starts_with(p) && matches!(stem.as_bytes()[3], b'1'..=b'9');
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT") {
        return Some(format!("‘{stem}’ 이름은 Windows가 따로 써서 쓸 수 없어요"));
    }
    None
}

/// 천 단위 쉼표 (12345 → "12,345")
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_puts_commas_every_three_digits() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(1234567), "1,234,567");
    }

    /// 쓸 수 없는 문자와 Windows 예약 이름은 막고, 비슷하지만 괜찮은 이름은 통과시킨다
    #[test]
    fn name_problem_blocks_reserved_and_invalid() {
        for bad in ["a?b.txt", "보고서:최종.xlsx", "CON", "con.txt", "Nul.docx", "COM1.log", "lpt9"] {
            assert!(name_problem(bad).is_some(), "{bad}");
        }
        for ok in ["보고서.xlsx", "CONTRACT.pdf", "COM0.txt", "COM10.txt", "console", ".gitignore", "aux2"] {
            assert!(name_problem(ok).is_none(), "{ok}");
        }
    }

    #[test]
    fn root_lengths() {
        assert_eq!(root_len("C:\\"), 3);
        assert_eq!(root_len("C:\\Users"), 3);
        assert_eq!(root_len("C:"), 2);
        assert_eq!(root_len("\\\\srv\\share"), 11);
        assert_eq!(root_len("\\\\srv\\share\\dir"), 11);
        assert_eq!(root_len("relative"), 0);
    }

    #[test]
    fn normalizes_input() {
        assert_eq!(normalize("c:/users/"), "C:\\users");
        assert_eq!(normalize("c:"), "C:\\");
        assert_eq!(normalize("\"D:\\Work\\\""), "D:\\Work");
        assert_eq!(normalize("C:\\"), "C:\\");
        assert_eq!(normalize("\\\\srv\\share\\"), "\\\\srv\\share");
    }

    #[test]
    fn parents() {
        assert_eq!(parent("C:\\a\\b").as_deref(), Some("C:\\a"));
        assert_eq!(parent("C:\\a").as_deref(), Some("C:\\"));
        assert_eq!(parent("C:\\"), None);
        assert_eq!(parent("\\\\srv\\share\\x").as_deref(), Some("\\\\srv\\share"));
        assert_eq!(parent("\\\\srv\\share"), None);
    }

    #[test]
    fn names_and_segments() {
        assert_eq!(file_name("C:\\a\\b.txt"), "b.txt");
        assert_eq!(file_name("C:\\"), "C:");
        assert_eq!(join("C:\\", "a"), "C:\\a");
        assert_eq!(join("C:\\a", "b"), "C:\\a\\b");
        let s = segments("C:\\Users\\hong");
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], ("C:".to_string(), "C:\\".to_string()));
        assert_eq!(s[2], ("hong".to_string(), "C:\\Users\\hong".to_string()));
    }

    #[test]
    fn invalid_chars() {
        assert_eq!(invalid_name_char("a?b"), Some('?'));
        assert_eq!(invalid_name_char("보고서 (최종).txt"), None);
    }
}
