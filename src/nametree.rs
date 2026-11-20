//! 맥의 이름 인덱스 틀: 폴더를 훑어 넣고, 바뀐 폴더를 다시 읽으면 그 차이만 반영한다.
//! 운영 체제 호출은 없다: 폴더를 읽고 변경 알림을 받는 것은 맥 쪽(src/mac/search)이 하고, 여기는 받은 목록으로 인덱스를 맞춘다.
//! 이름 저장, 순위, 검색은 index.rs의 Volume과 search_visible을 그대로 쓴다 (Windows와 같은 검색어 규칙).
//!
//! 칸 번호는 여기서 매긴다: ROOT_RECORD 칸이 '/'이고 그 위 번호를 차례로 쓰며, 지운 칸은 다시 쓴다.
//! 폴더마다 자식 목록(첫 자식, 다음 형제)을 달아 경로로 칸을 찾고, 폴더 하나를 다시 읽었을 때의 차이를 계산한다.
//! 이름은 한글을 합쳐 넣는다 (hangul.rs): 맥 디스크는 나뉜 꼴과 합친 꼴을 같은 이름으로 보므로 여는 데는 지장이 없다.

use crate::hangul;
use crate::index::{Hit, Query, ROOT_RECORD, Volume, search_visible};
use std::collections::HashMap;

const NONE: u32 = u32::MAX;

/// 폴더를 읽어 얻은 항목 하나
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    pub kind: Kind,
}

/// 인덱스에 두는 꼴
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 들어가 훑는 폴더
    Dir,
    /// 파일. 링크와 패키지(.app 등)도 여기에 둔다: 따라가거나 안을 훑지 않는다
    File,
    /// 폴더지만 안을 훑지 않는다 (홈의 라이브러리·휴지통, 훑는 범위 위쪽의 폴더)
    Closed,
}

/// 변경 알림이 온 경로를 어떻게 다룰지
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// 훑은 폴더: 다시 읽어 차이를 반영한다
    Dir(u32),
    /// 인덱스에 아직 없는 경로: 그 위의 훑은 폴더(이 칸)를 다시 읽으면 새 폴더가 들어온다
    Below(u32),
    /// 훑는 범위 밖이거나 파일·패키지·훑지 않는 폴더 안: 할 일이 없다
    Ignore,
}

/// 맥의 이름 인덱스
pub struct NameTree {
    vol: Volume,
    first_child: Vec<u32>,
    next_sibling: Vec<u32>,
    /// 안을 훑지 않는 폴더 (Kind::Closed)
    closed: Vec<bool>,
    /// 지운 칸 (다시 쓴다)
    free: Vec<u32>,
    next_slot: u32,
    /// 훑는 범위의 맨 위 폴더들
    roots: Vec<u32>,
}

impl Default for NameTree {
    fn default() -> Self {
        Self::new()
    }
}

impl NameTree {
    pub fn new() -> NameTree {
        let n = ROOT_RECORD as usize + 1;
        // 0~4 칸은 쓰지 않고, ROOT_RECORD 칸('/')은 훑는 범위로 두기 전까지 훑지 않는 폴더다
        NameTree { vol: Volume::posix(), first_child: vec![NONE; n], next_sibling: vec![NONE; n], closed: vec![true; n], free: Vec::new(), next_slot: ROOT_RECORD + 1, roots: Vec::new() }
    }

    /// 훑는 범위의 맨 위 폴더를 둔다 ("/Users/hong"). 그 위의 폴더들은 훑지 않는 폴더로 만든다.
    /// 그 칸을 돌려준다: 이 칸부터 sync_dir로 채워 훑는다. 이미 있으면 그 칸을 훑는 범위로 바꾼다
    pub fn add_root(&mut self, path: &str) -> u32 {
        let path = hangul::compose(path);
        let parts: Vec<&str> = components(&path).collect();
        let mut cur = ROOT_RECORD;
        for (k, part) in parts.iter().enumerate() {
            cur = match self.child_exact(cur, part) {
                Some(c) => c,
                None => {
                    let kind = if k + 1 == parts.len() { Kind::Dir } else { Kind::Closed };
                    let s = self.insert(cur, part, kind);
                    self.next_sibling[s as usize] = self.first_child[cur as usize];
                    self.first_child[cur as usize] = s;
                    s
                }
            };
        }
        self.closed[cur as usize] = false;
        if !self.roots.contains(&cur) {
            self.roots.push(cur);
        }
        cur
    }

    /// 폴더 dir을 다시 읽은 항목으로 맞춘다: 같은 이름·꼴은 그대로 두고(그 아래도 그대로), 없어진 것은 그 아래까지 빼고, 새것은 넣는다.
    /// 새로 들어온 훑을 폴더(Kind::Dir)의 칸을 돌려준다: 그 폴더를 읽어 다시 이것으로 채운다
    pub fn sync_dir(&mut self, dir: u32, items: &[Item]) -> Vec<u32> {
        let mut old: HashMap<String, u32> = HashMap::new();
        for c in self.children(dir) {
            if let Some(name) = self.vol.name(c) {
                old.insert(name.to_string(), c);
            }
        }
        let mut chain = Vec::with_capacity(items.len());
        let mut fresh = Vec::new();
        for it in items {
            let name = hangul::compose(&it.name);
            if let Some(c) = old.remove(name.as_ref()) {
                match (self.kind(c), it.kind) {
                    (was, now) if was == now => {
                        chain.push(c);
                        continue;
                    }
                    // 훑지 않던 폴더를 이제 훑는다 (그 아래에 다른 훑는 범위가 있으면 그대로 남는다)
                    (Kind::Closed, Kind::Dir) => {
                        self.closed[c as usize] = false;
                        fresh.push(c);
                        chain.push(c);
                        continue;
                    }
                    // 훑던 폴더를 이제 훑지 않는다
                    (Kind::Dir, Kind::Closed) => {
                        self.drop_children(c);
                        self.closed[c as usize] = true;
                        chain.push(c);
                        continue;
                    }
                    // 파일과 폴더가 바뀌었다: 새 항목으로 넣는다
                    _ => self.drop_subtree(c),
                }
            }
            let s = self.insert(dir, &name, it.kind);
            if it.kind == Kind::Dir {
                fresh.push(s);
            }
            chain.push(s);
        }
        for c in old.into_values() {
            self.drop_subtree(c);
        }
        let mut next = NONE;
        for &c in chain.iter().rev() {
            self.next_sibling[c as usize] = next;
            next = c;
        }
        self.first_child[dir as usize] = next;
        fresh
    }

    /// 변경 알림이 온 경로("/Users/hong/문서")를 어떻게 다룰지. 끝의 '/', 한글의 나뉜 꼴, 대소문자만 다른 경로도 받는다
    pub fn locate(&self, path: &str) -> Place {
        let path = hangul::compose(path);
        let mut cur = ROOT_RECORD;
        let mut crawled = self.kind(ROOT_RECORD) == Kind::Dir;
        for part in components(&path) {
            let Some(c) = self.child(cur, part) else {
                return if crawled { Place::Below(cur) } else { Place::Ignore };
            };
            match self.kind(c) {
                Kind::File => return Place::Ignore,
                Kind::Dir => crawled = crawled || self.roots.contains(&c),
                Kind::Closed => crawled = false,
            }
            cur = c;
        }
        if crawled { Place::Dir(cur) } else { Place::Ignore }
    }

    /// 폴더 dir 바로 아래의 훑는 폴더들 (그 아래까지 다시 훑을 때 차례로 읽는다)
    pub fn child_dirs(&self, dir: u32) -> Vec<u32> {
        self.children(dir).into_iter().filter(|&c| self.kind(c) == Kind::Dir).collect()
    }

    /// 칸의 전체 경로 ("/Users/hong/문서")
    pub fn path(&self, slot: u32) -> Option<String> {
        self.vol.path(slot)
    }

    /// 칸의 이름 (한글은 합친 꼴)
    pub fn name(&self, slot: u32) -> Option<&str> {
        self.vol.name(slot)
    }

    /// 이름으로 찾는다 (index::search_visible과 같은 규칙. 검색어는 Query::parse_posix로 만든다)
    pub fn search(&self, q: &Query, limit: usize) -> (Vec<Hit>, usize) {
        search_visible(&[&self.vol], q, limit, &|_, _| true)
    }

    /// 인덱스에 든 이름 수
    pub fn len(&self) -> usize {
        self.vol.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 대략적인 메모리 사용량(바이트)
    pub fn memory(&self) -> usize {
        self.vol.memory() + (self.first_child.capacity() + self.next_sibling.capacity() + self.free.capacity()) * 4 + self.closed.capacity()
    }

    /// 지운 이름이 많이 쌓였으면 저장소를 다시 채운다 (변경을 한 묶음 반영한 뒤에 부른다)
    pub fn tidy(&mut self) {
        self.vol.compact_if_needed();
    }

    fn kind(&self, slot: u32) -> Kind {
        if self.closed[slot as usize] {
            Kind::Closed
        } else if slot == ROOT_RECORD || self.vol.is_dir(slot) {
            Kind::Dir
        } else {
            Kind::File
        }
    }

    fn children(&self, dir: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut c = self.first_child[dir as usize];
        while c != NONE {
            out.push(c);
            c = self.next_sibling[c as usize];
        }
        out
    }

    fn child_exact(&self, dir: u32, name: &str) -> Option<u32> {
        self.children(dir).into_iter().find(|&c| self.vol.name(c) == Some(name))
    }

    /// 이름이 같은 자식. 없으면 대소문자만 다른 자식이 하나뿐일 때 그것 (알림의 경로와 디스크의 대소문자가 다를 때)
    fn child(&self, dir: u32, name: &str) -> Option<u32> {
        if let Some(c) = self.child_exact(dir, name) {
            return Some(c);
        }
        let lower = name.to_lowercase();
        let mut same = self.children(dir).into_iter().filter(|&c| self.vol.name(c).is_some_and(|n| n.to_lowercase() == lower));
        match (same.next(), same.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    }

    fn insert(&mut self, parent: u32, name: &str, kind: Kind) -> u32 {
        let slot = self.free.pop().unwrap_or_else(|| {
            self.next_slot += 1;
            self.next_slot - 1
        });
        let i = slot as usize;
        if self.first_child.len() <= i {
            self.first_child.resize(i + 1, NONE);
            self.next_sibling.resize(i + 1, NONE);
            self.closed.resize(i + 1, false);
        }
        self.first_child[i] = NONE;
        self.next_sibling[i] = NONE;
        self.closed[i] = kind == Kind::Closed;
        self.vol.upsert(slot as u64, parent as u64, name, kind != Kind::File);
        slot
    }

    fn drop_children(&mut self, dir: u32) {
        for c in self.children(dir) {
            self.drop_subtree(c);
        }
        self.first_child[dir as usize] = NONE;
    }

    /// top과 그 아래를 모두 지운다. top을 부모의 자식 목록에서 빼는 것은 부르는 쪽이 한다 (sync_dir이 목록을 다시 잇는다)
    fn drop_subtree(&mut self, top: u32) {
        let mut stack = vec![top];
        while let Some(s) = stack.pop() {
            stack.extend(self.children(s));
            let i = s as usize;
            self.vol.remove(s as u64);
            self.first_child[i] = NONE;
            self.next_sibling[i] = NONE;
            self.closed[i] = false;
            self.free.push(s);
            self.roots.retain(|&r| r != s);
        }
    }
}

/// 경로의 이름 조각들 ("/Users/hong/" → Users, hong)
fn components(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|p| !p.is_empty())
}

/// Finder가 파일 하나로 보이는 패키지의 확장자 (앱, 보관함, 문서 묶음). 그 안은 훑지 않는다
const PACKAGES: [&str; 34] = [
    "app",
    "appex",
    "bundle",
    "framework",
    "plugin",
    "kext",
    "xpc",
    "prefpane",
    "saver",
    "qlgenerator",
    "mdimporter",
    "component",
    "workflow",
    "photoslibrary",
    "photolibrary",
    "musiclibrary",
    "imovielibrary",
    "tvlibrary",
    "theater",
    "fcpbundle",
    "logicx",
    "band",
    "sparsebundle",
    "xcarchive",
    "dsym",
    "rtfd",
    "pkg",
    "mpkg",
    "pages",
    "numbers",
    "key",
    "playground",
    "xcodeproj",
    "xcworkspace",
];

/// 폴더를 읽을 때 항목을 어떻게 둘지. home은 홈 폴더, parent는 읽은 폴더의 경로다.
/// 링크는 따라가지 않고 파일로 둔다. 패키지(앱, 사진 보관함 등)는 Finder처럼 파일 하나로 본다.
/// 홈의 라이브러리(다른 앱의 데이터·캐시)와 휴지통(권한 없이 읽을 수 없다)은 안을 훑지 않는다
pub fn classify(home: &str, parent: &str, name: &str, is_dir: bool, is_link: bool) -> Kind {
    if is_link || !is_dir || is_package(name) {
        return Kind::File;
    }
    if (name == "Library" || name == ".Trash") && hangul::compose(parent).trim_end_matches('/') == hangul::compose(home).trim_end_matches('/') {
        return Kind::Closed;
    }
    Kind::Dir
}

fn is_package(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && PACKAGES.iter().any(|p| p.eq_ignore_ascii_case(ext)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 주간보고 (맥이 저장하는 나뉜 꼴)
    const WEEKLY_NFD: &str = "\u{110C}\u{116E}\u{1100}\u{1161}\u{11AB}\u{1107}\u{1169}\u{1100}\u{1169}";

    fn items(list: &[(&str, Kind)]) -> Vec<Item> {
        list.iter().map(|(name, kind)| Item { name: name.to_string(), kind: *kind }).collect()
    }

    fn found(t: &NameTree, q: &str) -> (Vec<String>, usize) {
        let (hits, total) = t.search(&Query::parse_posix(q), 50);
        (hits.into_iter().map(|h| h.path).collect(), total)
    }

    /// 홈에 문서 폴더, 파일, 라이브러리, 앱이 있고 문서 안에 나뉜 꼴의 한글 이름이 있다
    fn sample() -> (NameTree, u32, u32) {
        let mut t = NameTree::new();
        let home = t.add_root("/Users/hong");
        let fresh = t.sync_dir(home, &items(&[("문서", Kind::Dir), ("메모.txt", Kind::File), ("Library", Kind::Closed), ("Safari.app", Kind::File)]));
        assert_eq!(fresh.len(), 1, "only 문서 is crawled further");
        let docs = fresh[0];
        assert!(t.sync_dir(docs, &items(&[(&format!("{WEEKLY_NFD}.xlsx"), Kind::File), ("회의록.md", Kind::File)])).is_empty());
        (t, home, docs)
    }

    #[test]
    fn crawled_names_are_found_with_composed_hangul() {
        let (t, _, docs) = sample();
        assert_eq!(t.path(docs).as_deref(), Some("/Users/hong/문서"));
        assert_eq!(found(&t, "보고"), (vec!["/Users/hong/문서/주간보고.xlsx".to_string()], 1));
        // 나뉜 꼴로 친 검색어도 (Finder에서 이름을 복사해 붙인 경우)
        assert_eq!(found(&t, WEEKLY_NFD).1, 1);
        // 경로 낱말은 '/'로
        assert_eq!(found(&t, "보고 문서/").1, 1);
        assert_eq!(found(&t, "hong/문서/").1, 2);
        assert_eq!(found(&t, "보고 다른곳/").1, 0);
        // 위쪽 폴더(Users)와 홈, 패키지도 이름으로 찾는다
        assert_eq!(found(&t, "safari").0, ["/Users/hong/Safari.app"]);
        assert_eq!(t.len(), 8, "Users, hong, 문서, 메모, Library, Safari, 주간보고, 회의록");
    }

    #[test]
    fn rescanning_a_folder_applies_only_the_difference() {
        let (mut t, home, docs) = sample();
        // 이름 바꾸기: 옛 이름은 빠지고 새 이름이 들어온다
        assert!(t.sync_dir(docs, &items(&[("최종 보고.xlsx", Kind::File), ("회의록.md", Kind::File)])).is_empty());
        assert_eq!(found(&t, "주간").1, 0);
        assert_eq!(found(&t, "최종").0, ["/Users/hong/문서/최종 보고.xlsx"]);
        assert_eq!(t.len(), 8);
        // 그대로인 폴더는 다시 훑지 않는다 (새 폴더만 돌려준다)
        let fresh = t.sync_dir(home, &items(&[("문서", Kind::Dir), ("메모.txt", Kind::File), ("Library", Kind::Closed), ("Safari.app", Kind::File), ("사진", Kind::Dir)]));
        assert_eq!(fresh.len(), 1);
        assert_eq!(t.path(fresh[0]).as_deref(), Some("/Users/hong/사진"));
        assert_eq!(found(&t, "최종").1, 1, "문서 kept its contents");
    }

    #[test]
    fn a_removed_folder_takes_everything_below_it() {
        let (mut t, home, docs) = sample();
        let before = t.len();
        t.sync_dir(home, &items(&[("메모.txt", Kind::File), ("Library", Kind::Closed), ("Safari.app", Kind::File)]));
        assert_eq!(t.len(), before - 3, "문서 and its two files");
        assert_eq!(found(&t, "보고").1, 0);
        assert_eq!(found(&t, "회의록").1, 0, "no orphans counted");
        assert_eq!(t.path(docs), None);
        // 지운 칸은 다시 쓴다
        let fresh = t.sync_dir(home, &items(&[("메모.txt", Kind::File), ("Library", Kind::Closed), ("Safari.app", Kind::File), ("새 폴더", Kind::Dir)]));
        assert!(t.free.len() < 3 && fresh.len() == 1);
        assert_eq!(t.path(fresh[0]).as_deref(), Some("/Users/hong/새 폴더"));
    }

    #[test]
    fn a_folder_that_became_a_file_and_back() {
        let (mut t, home, _) = sample();
        let mut list = vec![("문서", Kind::File), ("메모.txt", Kind::File), ("Library", Kind::Closed), ("Safari.app", Kind::File)];
        assert!(t.sync_dir(home, &items(&list)).is_empty());
        assert_eq!(found(&t, "보고").1, 0);
        let (hits, _) = t.search(&Query::parse_posix("문서"), 5);
        assert!(!hits[0].is_dir);
        list[0].1 = Kind::Dir;
        let fresh = t.sync_dir(home, &items(&list));
        assert_eq!(fresh.len(), 1, "a new folder to crawl, empty for now");
        assert!(t.child_dirs(fresh[0]).is_empty());
    }

    #[test]
    fn locates_changed_paths() {
        let (mut t, home, docs) = sample();
        assert_eq!(t.locate("/Users/hong/문서"), Place::Dir(docs));
        assert_eq!(t.locate("/Users/hong/문서/"), Place::Dir(docs));
        assert_eq!(t.locate("/users/HONG/문서"), Place::Dir(docs), "case differs from the disk");
        assert_eq!(t.locate("/Users/hong"), Place::Dir(home));
        // 아직 모르는 폴더: 아는 데까지 올라가 다시 읽는다
        assert_eq!(t.locate("/Users/hong/새 폴더/안쪽"), Place::Below(home));
        assert_eq!(t.locate("/Users/hong/문서/새 폴더"), Place::Below(docs));
        // 할 일이 없는 경로: 훑지 않는 라이브러리 안, 파일 아래, 패키지 안, 범위 밖, 범위 위쪽 폴더
        for path in ["/Users/hong/Library/Caches", "/Users/hong/메모.txt/x", "/Users/hong/Safari.app/Contents", "/Volumes/USB", "/Users", "/"] {
            assert_eq!(t.locate(path), Place::Ignore, "{path}");
        }
        // 나뉜 꼴의 경로 (알림은 디스크에 저장된 꼴로 온다)
        let nfd_docs = "/Users/hong/\u{1106}\u{116E}\u{11AB}\u{1109}\u{1165}";
        assert_eq!(t.locate(nfd_docs), Place::Dir(docs));
        // 라이브러리 안의 iCloud Drive를 따로 훑는 범위로 둔다: 홈을 다시 읽어도 남는다
        let icloud = t.add_root("/Users/hong/Library/Mobile Documents");
        assert_eq!(t.locate("/Users/hong/Library/Mobile Documents/com~apple~CloudDocs"), Place::Below(icloud));
        let fresh = t.sync_dir(icloud, &items(&[("com~apple~CloudDocs", Kind::Dir)]));
        t.sync_dir(fresh[0], &items(&[("여행 계획.pages", Kind::File)]));
        t.sync_dir(home, &items(&[("문서", Kind::Dir), ("메모.txt", Kind::File), ("Library", Kind::Closed), ("Safari.app", Kind::File)]));
        assert_eq!(t.locate("/Users/hong/Library/Mobile Documents/com~apple~CloudDocs"), Place::Dir(fresh[0]));
        assert_eq!(found(&t, "여행").0, ["/Users/hong/Library/Mobile Documents/com~apple~CloudDocs/여행 계획.pages"]);
        assert_eq!(t.locate("/Users/hong/Library/Caches"), Place::Ignore, "the rest of Library stays closed");
    }

    #[test]
    fn a_closed_folder_that_becomes_crawled_keeps_roots_below_it() {
        // 홈을 먼저 범위로 두고, 나중에 '/' 전체를 훑는다 (측정할 때): Users는 위쪽 폴더였다가 훑는 폴더가 된다
        let (mut t, _, _) = sample();
        let root = t.add_root("/");
        assert_eq!(root, ROOT_RECORD);
        let fresh = t.sync_dir(root, &items(&[("Users", Kind::Dir), ("Applications", Kind::Dir)]));
        assert_eq!(fresh.len(), 2);
        assert_eq!(t.locate("/Users"), Place::Dir(fresh.iter().copied().find(|&s| t.path(s).as_deref() == Some("/Users")).unwrap()));
        assert_eq!(found(&t, "보고").1, 1, "the home folder below was kept");
        assert_eq!(t.locate("/Volumes/USB"), Place::Below(ROOT_RECORD));
    }

    #[test]
    fn classifies_entries_like_finder() {
        let home = "/Users/hong";
        assert_eq!(classify(home, home, "문서", true, false), Kind::Dir);
        assert_eq!(classify(home, home, "메모.txt", false, false), Kind::File);
        assert_eq!(classify(home, home, "링크", true, true), Kind::File, "links are not followed");
        for package in ["Safari.app", "THING.APP", "사진 보관함.photoslibrary", "백업.sparsebundle", "발표.key", "Project.xcodeproj"] {
            assert_eq!(classify(home, "/Applications", package, true, false), Kind::File, "{package}");
        }
        assert_eq!(classify(home, home, ".app", true, false), Kind::Dir, "a hidden folder named .app is not a package");
        assert_eq!(classify(home, home, "Library", true, false), Kind::Closed);
        assert_eq!(classify(home, "/Users/hong/", ".Trash", true, false), Kind::Closed);
        assert_eq!(classify(home, "/Users/hong/문서", "Library", true, false), Kind::Dir, "only the home folder's Library is closed");
    }

    #[test]
    fn memory_grows_with_names() {
        let (t, _, _) = sample();
        assert!(t.memory() > 0 && !t.is_empty());
    }
}
