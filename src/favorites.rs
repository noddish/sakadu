//! 즐겨찾기: 폴더·파일, 실제 이름과 별개인 표시 이름, 사용자가 정한 순서와 그룹.
//! %APPDATA%\Sakadu\favorites.txt 에 줄마다 "item<TAB>표시 이름<TAB>경로<TAB>dir|file" 또는
//! "group<TAB>그룹 이름<TAB>open|closed"로 저장한다. 그룹 줄 아래 항목이 그 그룹에 들고, 첫 그룹 앞은 그룹 없음이다.

use crate::util::{self, wide};
use std::cmp::Ordering;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Favorite {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub collapsed: bool,
    pub items: Vec<Favorite>,
}

/// 그룹 없는 항목이 먼저, 그 뒤에 그룹들
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Favorites {
    pub items: Vec<Favorite>,
    pub groups: Vec<Group>,
}

/// 사이드바에 보이는 한 줄. group이 None이면 그룹 없는 항목
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Row {
    Item { group: Option<usize>, index: usize },
    Group(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SortBy {
    Name,
    NameDesc,
    Path,
}

const HEADER_V1: &str = "sakadu-favorites 1";
const HEADER: &str = "sakadu-favorites 2";

impl Favorites {
    /// 사이드바 목록의 줄: 그룹 없는 항목, 그다음 그룹마다 머리 줄과 (펼친 그룹이면) 그 안 항목
    pub fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = (0..self.items.len()).map(|index| Row::Item { group: None, index }).collect();
        for (g, group) in self.groups.iter().enumerate() {
            rows.push(Row::Group(g));
            if !group.collapsed {
                rows.extend((0..group.items.len()).map(|index| Row::Item { group: Some(g), index }));
            }
        }
        rows
    }

    /// 그룹을 접거나 펼친다. 바뀌었으면 true
    pub fn set_collapsed(&mut self, g: usize, collapsed: bool) -> bool {
        match self.groups.get_mut(g) {
            Some(group) if group.collapsed != collapsed => {
                group.collapsed = collapsed;
                true
            }
            _ => false,
        }
    }

    pub fn section(&self, group: Option<usize>) -> &[Favorite] {
        match group {
            Some(g) => self.groups.get(g).map(|g| g.items.as_slice()).unwrap_or(&[]),
            None => &self.items,
        }
    }

    fn section_mut(&mut self, group: Option<usize>) -> Option<&mut Vec<Favorite>> {
        match group {
            Some(g) => self.groups.get_mut(g).map(|g| &mut g.items),
            None => Some(&mut self.items),
        }
    }

    pub fn item(&self, group: Option<usize>, index: usize) -> Option<&Favorite> {
        self.section(group).get(index)
    }

    /// 줄의 표시 이름 (항목이면 즐겨찾기 이름, 그룹이면 그룹 이름)
    pub fn row_name(&self, row: Row) -> Option<&str> {
        match row {
            Row::Item { group, index } => self.item(group, index).map(|f| f.name.as_str()),
            Row::Group(g) => self.groups.get(g).map(|g| g.name.as_str()),
        }
    }

    pub fn set_row_name(&mut self, row: Row, name: String) -> bool {
        let slot = match row {
            Row::Item { group, index } => self.section_mut(group).and_then(|s| s.get_mut(index)).map(|f| &mut f.name),
            Row::Group(g) => self.groups.get_mut(g).map(|g| &mut g.name),
        };
        match slot {
            Some(n) if *n != name => {
                *n = name;
                true
            }
            _ => false,
        }
    }

    /// 같은 경로가 이미 있으면 그 위치
    pub fn find(&self, path: &str) -> Option<(Option<usize>, usize)> {
        if let Some(i) = self.items.iter().position(|f| util::eq_path(&f.path, path)) {
            return Some((None, i));
        }
        self.groups.iter().enumerate().find_map(|(g, grp)| grp.items.iter().position(|f| util::eq_path(&f.path, path)).map(|i| (Some(g), i)))
    }

    /// 보이는 줄 가운데 이 항목·그룹의 번호
    pub fn row_index(&self, row: Row) -> Option<usize> {
        self.rows().iter().position(|r| *r == row)
    }

    /// 그 섹션 끝에 넣고 위치를 돌려준다. 접힌 그룹이면 펼친다
    pub fn push(&mut self, group: Option<usize>, fav: Favorite) -> (Option<usize>, usize) {
        let group = group.filter(|&g| g < self.groups.len());
        if let Some(g) = group {
            self.groups[g].collapsed = false;
        }
        let section = self.section_mut(group).expect("checked above");
        section.push(fav);
        (group, section.len() - 1)
    }

    pub fn remove_item(&mut self, group: Option<usize>, index: usize) -> Option<Favorite> {
        let section = self.section_mut(group)?;
        (index < section.len()).then(|| section.remove(index))
    }

    /// 항목을 다른 자리로. to_index는 옮기기 전 기준(같은 섹션 안에서 뒤로 옮기면 하나 당겨진다). 새 위치를 돌려준다
    pub fn move_item(&mut self, from: (Option<usize>, usize), to_group: Option<usize>, to_index: usize) -> Option<(Option<usize>, usize)> {
        if to_group.is_some_and(|g| g >= self.groups.len()) {
            return None;
        }
        let fav = self.remove_item(from.0, from.1)?;
        let to_index = if from.0 == to_group && to_index > from.1 { to_index - 1 } else { to_index };
        if let Some(g) = to_group {
            self.groups[g].collapsed = false;
        }
        let section = self.section_mut(to_group)?;
        let at = to_index.min(section.len());
        section.insert(at, fav);
        Some((to_group, at))
    }

    pub fn move_group(&mut self, from: usize, to: usize) -> Option<usize> {
        if from >= self.groups.len() {
            return None;
        }
        let g = self.groups.remove(from);
        let to = to.min(self.groups.len());
        self.groups.insert(to, g);
        Some(to)
    }

    pub fn add_group(&mut self, name: String) -> usize {
        self.groups.push(Group { name, collapsed: false, items: Vec::new() });
        self.groups.len() - 1
    }

    /// 그룹만 없애고 안의 항목은 그룹 없음 끝으로 옮긴다
    pub fn ungroup(&mut self, g: usize) {
        if g < self.groups.len() {
            let group = self.groups.remove(g);
            self.items.extend(group.items);
        }
    }

    /// 그룹과 안의 항목을 함께 지운다
    pub fn delete_group(&mut self, g: usize) {
        if g < self.groups.len() {
            self.groups.remove(g);
        }
    }

    /// 항목을 row 줄 위에 놓으면 들어갈 (그룹, 위치). 위치는 옮기기 전 기준.
    /// 그룹 머리 줄 위면 그 그룹 끝으로, 항목 줄이면 그 줄의 위(below가 false)나 아래 자리로
    pub fn item_drop_target(&self, row: Row, below: bool) -> (Option<usize>, usize) {
        match row {
            Row::Group(g) => (Some(g), self.section(Some(g)).len()),
            Row::Item { group, index } => (group, index + usize::from(below)),
        }
    }

    /// 그룹을 row 줄 위에 놓으면 들어갈 그룹 순서 (옮긴 뒤 기준). 그룹은 그룹끼리만 자리를 바꾼다:
    /// 그룹 머리 줄의 위쪽 절반이면 그 그룹 앞, 그 밖에 그 그룹에 속한 줄이면 그 그룹 뒤, 그룹 없는 항목 위면 맨 앞
    pub fn group_drop_target(&self, row: Row, below: bool, dragged: usize) -> usize {
        let slot = self.group_drop_slot(row, below);
        if dragged < slot { slot - 1 } else { slot }
    }

    /// 그룹을 놓을 틈 (옮기기 전 기준 0..=그룹 수, 0이면 첫 그룹 앞)
    pub fn group_drop_slot(&self, row: Row, below: bool) -> usize {
        let slot = match row {
            Row::Item { group: None, .. } => 0,
            Row::Group(g) if !below => g,
            Row::Group(g) | Row::Item { group: Some(g), .. } => g + 1,
        };
        slot.min(self.groups.len())
    }

    /// Ctrl+↑↓: 항목은 섹션 안에서 한 칸, 섹션 끝이면 이웃 섹션으로. 그룹은 이웃 그룹과 자리를 바꾼다
    pub fn nudge(&mut self, row: Row, up: bool) -> Option<Row> {
        match row {
            Row::Group(g) => {
                let to = if up { g.checked_sub(1)? } else { g + 1 };
                (to < self.groups.len()).then(|| Row::Group(self.move_group(g, to).unwrap_or(g)))
            }
            Row::Item { group, index } => {
                let len = self.section(group).len();
                let (to_group, to_index) = match (up, group) {
                    (true, _) if index > 0 => (group, index - 1),
                    (true, None) => return None,
                    (true, Some(0)) => (None, self.items.len()),
                    (true, Some(g)) => (Some(g - 1), self.groups[g - 1].items.len()),
                    (false, _) if index + 1 < len => (group, index + 2),
                    (false, None) if self.groups.is_empty() => return None,
                    (false, None) => (Some(0), 0),
                    (false, Some(g)) if g + 1 < self.groups.len() => (Some(g + 1), 0),
                    (false, Some(_)) => return None,
                };
                let (group, index) = self.move_item((group, index), to_group, to_index)?;
                Some(Row::Item { group, index })
            }
        }
    }

    /// 섹션마다 항목을 정렬하고 그룹도 이름순으로 둔다
    pub fn sort(&mut self, by: SortBy) {
        sort_section(&mut self.items, by);
        for g in &mut self.groups {
            sort_section(&mut g.items, by);
        }
        match by {
            SortBy::NameDesc => self.groups.sort_by(|a, b| natural(&b.name, &a.name)),
            _ => self.groups.sort_by(|a, b| natural(&a.name, &b.name)),
        }
    }
}

fn sort_section(favs: &mut [Favorite], by: SortBy) {
    match by {
        SortBy::Name => favs.sort_by(|a, b| natural(&a.name, &b.name)),
        SortBy::NameDesc => favs.sort_by(|a, b| natural(&b.name, &a.name)),
        SortBy::Path => favs.sort_by(|a, b| natural(&a.path, &b.path)),
    }
}

fn file_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Sakadu").join("favorites.txt"))
}

pub fn load() -> Favorites {
    file_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save(favs: &Favorites) -> std::io::Result<()> {
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

pub fn serialize(favs: &Favorites) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    let item = |out: &mut String, f: &Favorite| {
        out.push_str(&format!("item\t{}\t{}\t{}\n", clean_name(&f.name), f.path, if f.is_dir { "dir" } else { "file" }));
    };
    for f in &favs.items {
        item(&mut out, f);
    }
    for g in &favs.groups {
        out.push_str(&format!("group\t{}\t{}\n", clean_name(&g.name), if g.collapsed { "closed" } else { "open" }));
        for f in &g.items {
            item(&mut out, f);
        }
    }
    out
}

pub fn parse(text: &str) -> Favorites {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    let mut favs = Favorites::default();
    match lines.next().map(str::trim) {
        // 예전 형식: "표시 이름<TAB>경로" 줄, 폴더만 있었다
        Some(HEADER_V1) => {
            favs.items = lines
                .filter_map(|l| l.split_once('\t'))
                .filter(|(_, p)| !p.trim().is_empty())
                .map(|(n, p)| Favorite { name: n.to_string(), path: p.trim().to_string(), is_dir: true })
                .collect();
        }
        Some(HEADER) => {
            for line in lines {
                let parts: Vec<&str> = line.split('\t').collect();
                match parts.as_slice() {
                    ["group", name, state, ..] => {
                        favs.groups.push(Group { name: name.to_string(), collapsed: *state == "closed", items: Vec::new() });
                    }
                    ["item", name, path, kind, ..] if !path.trim().is_empty() => {
                        let f = Favorite { name: name.to_string(), path: path.trim().to_string(), is_dir: *kind != "file" };
                        match favs.groups.last_mut() {
                            Some(g) => g.items.push(f),
                            None => favs.items.push(f),
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    favs
}

/// 새 즐겨찾기의 기본 표시 이름: 폴더·파일 이름(루트면 "C:")
pub fn default_name(path: &str) -> String {
    let n = util::file_name(path);
    if n.is_empty() { path.to_string() } else { n.to_string() }
}

/// 탐색기와 같은 자연 정렬 ("2.주간" < "10.월간")
pub fn natural(a: &str, b: &str) -> Ordering {
    crate::natural::cmp_z(&wide(a), &wide(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fav(n: &str, p: &str) -> Favorite {
        Favorite { name: n.into(), path: p.into(), is_dir: true }
    }

    fn file(n: &str, p: &str) -> Favorite {
        Favorite { name: n.into(), path: p.into(), is_dir: false }
    }

    fn sample() -> Favorites {
        Favorites {
            items: vec![fav("01.일간 업무", "D:\\업무\\일간 업무"), file("견적서", "D:\\업무\\견적서.xlsx")],
            groups: vec![
                Group { name: "거래처".into(), collapsed: false, items: vec![fav("A사", "D:\\거래처\\A"), fav("B사", "D:\\거래처\\B")] },
                Group { name: "보관".into(), collapsed: true, items: vec![fav("2025", "E:\\2025")] },
            ],
        }
    }

    fn names(f: &Favorites) -> String {
        f.rows().iter().map(|r| f.row_name(*r).unwrap_or("?")).collect::<Vec<_>>().join(",")
    }

    #[test]
    fn roundtrip_keeps_groups_files_and_order() {
        let favs = sample();
        assert_eq!(parse(&serialize(&favs)), favs);
    }

    #[test]
    fn reads_old_format_as_folders() {
        let old = "sakadu-favorites 1\n01.일간 업무\tD:\\업무\\일간 업무\nC:\tC:\\\n";
        let favs = parse(old);
        assert_eq!(favs.items, vec![fav("01.일간 업무", "D:\\업무\\일간 업무"), fav("C:", "C:\\")]);
        assert!(favs.groups.is_empty());
    }

    #[test]
    fn names_cannot_break_the_format() {
        let favs = Favorites { items: vec![fav("탭\t이름\n", "C:\\a")], groups: vec![Group { name: "그\t룹".into(), collapsed: false, items: Vec::new() }] };
        let back = parse(&serialize(&favs));
        assert_eq!(back.items, vec![fav("탭 이름", "C:\\a")]);
        assert_eq!(back.groups[0].name, "그 룹");
    }

    #[test]
    fn rows_open_groups_in_place() {
        // 펼친 그룹은 머리 줄 아래에 항목이 이어지고, 접힌 그룹은 머리 줄만
        assert_eq!(names(&sample()), "01.일간 업무,견적서,거래처,A사,B사,보관");
        let mut f = sample();
        assert!(f.set_collapsed(1, false));
        assert!(!f.set_collapsed(1, false), "no change");
        assert_eq!(names(&f), "01.일간 업무,견적서,거래처,A사,B사,보관,2025");
        assert_eq!(f.row_index(Row::Item { group: Some(1), index: 0 }), Some(6));
    }

    #[test]
    fn drop_targets_follow_the_line_under_the_cursor() {
        let f = sample();
        let item = |group, index| Row::Item { group, index };
        // 항목 줄의 위·아래 절반
        assert_eq!(f.item_drop_target(item(None, 0), false), (None, 0));
        assert_eq!(f.item_drop_target(item(None, 0), true), (None, 1));
        assert_eq!(f.item_drop_target(item(Some(0), 1), true), (Some(0), 2));
        // 그룹 머리 줄 위에 놓으면 그 그룹 끝으로 (접힌 그룹도)
        assert_eq!(f.item_drop_target(Row::Group(0), false), (Some(0), 2));
        assert_eq!(f.item_drop_target(Row::Group(1), true), (Some(1), 1));
        // 그룹은 그룹끼리: 보관을 거래처 앞으로, 거래처를 맨 뒤로
        assert_eq!(f.group_drop_target(Row::Group(0), false, 1), 0);
        assert_eq!(f.group_drop_target(item(None, 1), true, 1), 0, "groups never go above loose items");
        assert_eq!(f.group_drop_target(Row::Group(1), true, 0), 1);
        // 펼친 그룹의 항목 줄 위는 그 그룹 뒤
        assert_eq!(f.group_drop_slot(item(Some(0), 0), false), 1);
    }

    #[test]
    fn moves_between_groups() {
        let mut f = sample();
        // 견적서를 거래처 맨 앞으로
        assert_eq!(f.move_item((None, 1), Some(0), 0), Some((Some(0), 0)));
        assert_eq!(f.groups[0].items[0].name, "견적서");
        // 같은 섹션에서 뒤로: 옮기기 전 기준 위치 3(B사 뒤)
        assert_eq!(f.move_item((Some(0), 0), Some(0), 3), Some((Some(0), 2)));
        let names: Vec<_> = f.groups[0].items.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["A사", "B사", "견적서"]);
        f.move_item((Some(0), 2), Some(1), 0);
        assert_eq!(f.find("D:\\업무\\견적서.xlsx"), Some((Some(1), 0)));
    }

    #[test]
    fn nudge_crosses_section_edges() {
        let mut f = sample();
        f.groups[1].collapsed = false;
        let r = f.nudge(Row::Item { group: None, index: 1 }, false).unwrap();
        assert_eq!(r, Row::Item { group: Some(0), index: 0 });
        let r = f.nudge(r, true).unwrap();
        assert_eq!(r, Row::Item { group: None, index: 1 });
        assert_eq!(f.nudge(Row::Item { group: None, index: 0 }, true), None);
        assert_eq!(f.nudge(Row::Group(0), false), Some(Row::Group(1)));
        assert_eq!(f.groups[0].name, "보관");
    }

    #[test]
    fn ungroup_keeps_items_and_delete_drops_them() {
        let mut f = sample();
        f.ungroup(0);
        assert_eq!(f.items.len(), 4);
        assert_eq!(f.groups.len(), 1);
        f.delete_group(0);
        assert!(f.groups.is_empty(), "the group and its item are gone");
        assert_eq!(f.items.len(), 4);
    }

    #[test]
    fn natural_sort_within_sections() {
        let mut f = Favorites {
            items: vec![fav("10.기타", "a"), fav("2.주간", "b"), fav("01.일간", "c")],
            groups: vec![Group { name: "나".into(), collapsed: false, items: vec![] }, Group { name: "가".into(), collapsed: false, items: vec![] }],
        };
        f.sort(SortBy::Name);
        let names: Vec<_> = f.items.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["01.일간", "2.주간", "10.기타"]);
        assert_eq!(f.groups[0].name, "가");
        f.sort(SortBy::NameDesc);
        assert_eq!(f.items[0].name, "10.기타");
    }

    #[test]
    fn default_names() {
        assert_eq!(default_name("D:\\업무\\일간 업무"), "일간 업무");
        assert_eq!(default_name("D:\\업무\\견적서.xlsx"), "견적서.xlsx");
        assert_eq!(default_name("C:\\"), "C:");
    }
}
