//! 즐겨찾기: 실제 폴더 이름과 별개인 표시 이름, 사용자가 정한 순서.
//! %APPDATA%\Sakadu\favorites.txt 에 "표시 이름<TAB>경로" 줄로 저장한다.

use crate::util::{self, wide};
use std::cmp::Ordering;
use std::path::PathBuf;
use windows::Win32::UI::Shell::StrCmpLogicalW;
use windows::core::PCWSTR;

#[derive(Debug, Clone, PartialEq)]
pub struct Favorite {
    pub name: String,
    pub path: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SortBy {
    Name,
    NameDesc,
    Path,
}

const HEADER: &str = "sakadu-favorites 1";

fn file_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Sakadu").join("favorites.txt"))
}

pub fn load() -> Vec<Favorite> {
    file_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save(favs: &[Favorite]) -> std::io::Result<()> {
    let path = file_path().ok_or_else(|| std::io::Error::other("APPDATA 없음"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serialize(favs))?;
    std::fs::rename(tmp, path)
}

/// 표시 이름에 저장 형식을 깨는 문자(탭·줄바꿈)가 들어가지 않게 정리
pub fn clean_name(name: &str) -> String {
    name.chars().map(|c| if c == '\t' || c == '\r' || c == '\n' { ' ' } else { c }).collect::<String>().trim().to_string()
}

pub fn serialize(favs: &[Favorite]) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    for f in favs {
        out.push_str(&clean_name(&f.name));
        out.push('\t');
        out.push_str(&f.path);
        out.push('\n');
    }
    out
}

pub fn parse(text: &str) -> Vec<Favorite> {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next().map(str::trim) != Some(HEADER) {
        return Vec::new();
    }
    lines
        .filter_map(|l| l.split_once('\t'))
        .filter(|(_, p)| !p.trim().is_empty())
        .map(|(n, p)| Favorite { name: n.to_string(), path: p.trim().to_string() })
        .collect()
}

/// 새 즐겨찾기의 기본 표시 이름: 폴더 이름(루트면 "C:")
pub fn default_name(path: &str) -> String {
    let n = util::file_name(path);
    if n.is_empty() { path.to_string() } else { n.to_string() }
}

pub fn index_of(favs: &[Favorite], path: &str) -> Option<usize> {
    favs.iter().position(|f| util::eq_path(&f.path, path))
}

/// 탐색기와 같은 자연 정렬 ("2.주간" < "10.월간")
fn natural(a: &str, b: &str) -> Ordering {
    let (wa, wb) = (wide(a), wide(b));
    unsafe { StrCmpLogicalW(PCWSTR(wa.as_ptr()), PCWSTR(wb.as_ptr())) }.cmp(&0)
}

/// 목록 순서를 바꿔 저장한다. 이후 끌어서 세부 순서를 다시 조정할 수 있다
pub fn sort(favs: &mut [Favorite], by: SortBy) {
    match by {
        SortBy::Name => favs.sort_by(|a, b| natural(&a.name, &b.name)),
        SortBy::NameDesc => favs.sort_by(|a, b| natural(&b.name, &a.name)),
        SortBy::Path => favs.sort_by(|a, b| natural(&a.path, &b.path)),
    }
}

/// from 항목을 to 위치(이동 후 기준 인덱스)로 옮긴다
pub fn move_item(favs: &mut Vec<Favorite>, from: usize, to: usize) {
    if from >= favs.len() {
        return;
    }
    let item = favs.remove(from);
    favs.insert(to.min(favs.len()), item);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fav(n: &str, p: &str) -> Favorite {
        Favorite { name: n.into(), path: p.into() }
    }

    #[test]
    fn roundtrip_keeps_custom_names_and_order() {
        let favs = vec![fav("01.일간 업무", "D:\\업무\\일간 업무"), fav("02.주간", "D:\\업무\\주간"), fav("C:", "C:\\")];
        assert_eq!(parse(&serialize(&favs)), favs);
    }

    #[test]
    fn names_cannot_break_the_format() {
        let favs = vec![fav("탭\t이름\n", "C:\\a")];
        assert_eq!(parse(&serialize(&favs)), vec![fav("탭 이름", "C:\\a")]);
    }

    #[test]
    fn natural_sort_by_name() {
        let mut favs = vec![fav("10.기타", "a"), fav("2.주간", "b"), fav("01.일간", "c")];
        sort(&mut favs, SortBy::Name);
        let names: Vec<_> = favs.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["01.일간", "2.주간", "10.기타"]);
        sort(&mut favs, SortBy::NameDesc);
        assert_eq!(favs[0].name, "10.기타");
    }

    #[test]
    fn move_up_and_down() {
        let mut favs = vec![fav("a", "1"), fav("b", "2"), fav("c", "3")];
        move_item(&mut favs, 2, 0);
        assert_eq!(favs.iter().map(|f| f.name.as_str()).collect::<String>(), "cab");
        move_item(&mut favs, 0, 2);
        assert_eq!(favs.iter().map(|f| f.name.as_str()).collect::<String>(), "abc");
    }

    #[test]
    fn default_names() {
        assert_eq!(default_name("D:\\업무\\일간 업무"), "일간 업무");
        assert_eq!(default_name("C:\\"), "C:");
    }
}
