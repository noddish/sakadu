//! 새 탭 시작 화면 ("어디로 이동할까요?", Windows와 같은 화면): 고정한 폴더 카드, 최근 위치, 위치(사이드바 '모든 공간'의 바로가기와 디스크).
//! 자리·이웃·그리기는 sakadu::ui::start. 여기는 맥의 아이콘(Finder), 디스크 용량, 키·누르기, 최근 위치 목록.
//! 새 탭(⌘T·+)이 시작 화면으로 열릴지 지금 폴더로 열릴지는 설정의 '새 탭'(세션의 new_tab)이 정한다.
//! 키보드 초점 테두리는 화살표·Return으로 키보드를 쓴 뒤에만 보인다 (탭을 열자마자 Return으로 엉뚱한 곳에 가지 않게)

use super::*;
use sakadu::ui::start::{self as shared, Nav, StartArt, StartData, StartDrive, StartItem, StartView, neighbor, paint_start, start_area};

/// 시작 화면 아이콘: 그 경로의 Finder 아이콘
struct MacStartArt<'a> {
    places: &'a [(String, String)],
}

impl StartArt<Gfx> for MacStartArt<'_> {
    fn icon(&mut self, g: &mut Gfx, _t: &Theme, item: &StartItem, r: R) {
        let path = match item {
            StartItem::Fav { path, .. } | StartItem::Recent(path) => Some(path.as_str()),
            StartItem::Drive(i) => self.places.get(*i).map(|(_, path)| path.as_str()),
            StartItem::Computer => None,
        };
        if let Some(path) = path {
            g.icon_for_path(path, r);
        }
    }
}

/// 디스크 자리 하나: 남은·전체 바이트와, 그 자리가 디스크가 붙은 곳 그 자체인지 (바로가기가 아니다: 꺼낼 수 있다)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Space {
    pub free: u64,
    pub total: u64,
    pub mount: bool,
}

/// 디스크 자리들의 용량. 붙어 있는 디스크의 정보를 한 번에 읽고 꺼진 네트워크 디스크를 기다리지 않는다 (MNT_NOWAIT).
/// /Volumes 아래의 바로가기(시동 디스크 'Macintosh HD' → /)는 그 대상으로 찾는다. 디스크의 자리가 아닌 경로는 빠진다
pub fn volume_space(paths: &[String]) -> HashMap<String, Space> {
    if paths.is_empty() {
        return HashMap::new();
    }
    let mounts: HashMap<String, (u64, u64)> = crate::volumes::mounts().into_iter().map(|m| (m.on, (m.free, m.total))).collect();
    paths
        .iter()
        .filter_map(|path| {
            if let Some(&(free, total)) = mounts.get(path) {
                return Some((path.clone(), Space { free, total, mount: true }));
            }
            let target = std::fs::read_link(path).ok()?.to_string_lossy().into_owned();
            mounts.get(&target).map(|&(free, total)| (path.clone(), Space { free, total, mount: false }))
        })
        .collect()
}

impl App {
    /// 위치 칸에 보일 것: 사이드바 '모든 공간'의 바로가기와 디스크 (용량은 읽어 둔 디스크만)
    fn start_drives(&self) -> Vec<StartDrive<'_>> {
        self.places
            .iter()
            .map(|(name, path)| {
                let space = self.volumes.get(path).copied().unwrap_or_default();
                StartDrive { name, free: space.free, total: space.total }
            })
            .collect()
    }

    /// 칸 p의 시작 화면 배치. 지금 탭이 시작 화면이 아니거나 칸이 안 보이면 None
    pub(super) fn start_view(&self, p: usize) -> Option<StartView> {
        let tab = self.panes[p].tab();
        if tab.loc != Location::Start {
            return None;
        }
        let r = self.pane_rects().into_iter().find(|(q, _)| *q == p)?.1;
        let area = start_area(&geom(r));
        let v = self.start_layout(area, tab.scroll);
        // 고정한 것·최근 위치가 줄어 스크롤이 끝을 넘었으면 끝에 맞춘 배치로 (그리기·누르기가 같은 자리를 쓰게)
        let max = (v.height - area.h()).max(0.0);
        Some(if tab.scroll > max { self.start_layout(area, max) } else { v })
    }

    /// 고정한 폴더 카드 → 최근 위치 줄 → 위치 카드 (맥에는 아직 목록 필터가 없어 다 보인다)
    fn start_layout(&self, area: R, scroll: f32) -> StartView {
        let drives = self.start_drives();
        shared::start_layout(area, "", scroll, &StartData { pins: self.pins(), favs: &self.favs, recent: &self.recent, computer: false, drives: &drives, drives_label: "위치" })
    }

    /// 시작 화면의 스크롤 끝. 시작 화면이 아니면 None
    pub(super) fn start_max_scroll(&self, p: usize) -> Option<f32> {
        self.start_view(p).map(|v| (v.height - v.area.h()).max(0.0))
    }

    /// 칸 p의 시작 화면 안 (x, y)
    pub(super) fn start_hit(&self, p: usize, x: f32, y: f32) -> Hit {
        let Some(v) = self.start_view(p) else { return Hit::Pane(p) };
        match v.items.iter().position(|(_, r)| r.contains(x, y)) {
            Some(i) if v.area.contains(x, y) => Hit::Start(p, i),
            _ => Hit::Pane(p),
        }
    }

    fn start_location(&self, it: &StartItem) -> Option<Location> {
        match it {
            StartItem::Fav { path, .. } | StartItem::Recent(path) => Some(Location::Dir(path.clone())),
            StartItem::Drive(i) => self.places.get(*i).map(|(_, path)| Location::Dir(path.clone())),
            StartItem::Computer => None,
        }
    }

    /// 시작 화면 항목 i의 위치
    pub(super) fn start_target(&self, p: usize, i: usize) -> Option<Location> {
        let v = self.start_view(p)?;
        v.items.get(i).and_then(|(it, _)| self.start_location(it))
    }

    /// 시작 화면에서 고른 위치로 간다: new_tab이면 새 탭에서, 아니면 이 탭에서 (시작 화면은 뒤로 가기 기록에 남는다)
    pub(super) fn start_go(&mut self, p: usize, loc: Location, new_tab: bool) {
        self.start_keys = false;
        if new_tab {
            self.open_tab(p, loc, None);
        } else {
            self.navigate(p, loc, None);
        }
    }

    /// 시작 화면에서 누른 키: 화살표·Home·End로 항목 옮기기, Page Up/Down 스크롤, Return 열기(⌘Return은 새 탭),
    /// ⌫ 최근 위치에서 빼기. 처리했으면 true (나머지는 보통 키 처리로)
    pub(super) fn start_key(&mut self, p: usize, key: Key, m: Mods) -> bool {
        if m.alt || m.ctrl || m.shift {
            return false;
        }
        let Some(v) = self.start_view(p) else { return false };
        let n = v.items.len();
        let cur = self.panes[p].tab().cursor.min(n.saturating_sub(1));
        let nav = match key {
            Key::Left => Some(Nav::Left),
            Key::Right => Some(Nav::Right),
            Key::Up => Some(Nav::Up),
            Key::Down => Some(Nav::Down),
            Key::Home => Some(Nav::Home),
            Key::End => Some(Nav::End),
            _ => None,
        };
        match key {
            _ if nav.is_some() && !m.cmd => {
                if n == 0 {
                    return true;
                }
                // 처음 누른 화살표는 테두리만 보인다 (어디에 있는지 먼저 보이게)
                let rects: Vec<R> = v.items.iter().map(|(_, r)| *r).collect();
                let next = match nav {
                    Some(nav) if self.start_keys => neighbor(&rects, cur, nav),
                    _ => cur,
                };
                self.start_keys = true;
                self.panes[p].tab_mut().cursor = next;
                self.start_reveal(p);
            }
            Key::PageUp | Key::PageDown if !m.cmd => {
                let page = (v.area.h() - 40.0).max(40.0);
                self.panes[p].tab_mut().scroll += if key == Key::PageUp { -page } else { page };
                self.clamp_scroll(p);
            }
            // 테두리가 보일 때만 연다 (아니면 테두리부터 보인다): 탭을 열자마자 Return으로 엉뚱한 곳에 가지 않게
            Key::Return => match v.items.get(cur).and_then(|(it, _)| self.start_location(it)) {
                Some(loc) if self.start_keys => self.start_go(p, loc, m.cmd),
                _ => self.start_keys = n > 0,
            },
            // 최근 위치 줄이면 목록에서 뺀다. 다른 것은 지우지 않는다 (고정한 것·위치는 여기서 지울 것이 아니다)
            Key::Delete if !m.cmd => {
                if let (true, Some((StartItem::Recent(path), _))) = (self.start_keys, v.items.get(cur)) {
                    let path = path.clone();
                    self.forget_recent(&path);
                }
            }
            _ => return false,
        }
        true
    }

    /// 키보드로 옮긴 항목이 보이게 스크롤한다. 첫 항목이면 제목까지 보이게 맨 위로
    fn start_reveal(&mut self, p: usize) {
        let Some(v) = self.start_view(p) else { return };
        let cur = self.panes[p].tab().cursor;
        let Some((_, r)) = v.items.get(cur) else { return };
        let tab = self.panes[p].tab_mut();
        if cur == 0 {
            tab.scroll = 0.0;
        } else if r.t - 8.0 < v.area.t {
            tab.scroll -= v.area.t - (r.t - 8.0);
        } else if r.b + 8.0 > v.area.b {
            tab.scroll += r.b + 8.0 - v.area.b;
        }
        self.clamp_scroll(p);
    }

    /// 칸 p가 시작 화면이면 머리글·목록 자리에 카드와 줄을 그린다. 그렸으면 true
    pub(super) fn paint_start_view(&self, g: &mut Gfx, t: &Theme, p: usize) -> bool {
        let Some(v) = self.start_view(p) else { return false };
        let hovered = match self.hover {
            Hit::Start(q, i) if q == p => Some(i),
            _ => None,
        };
        let focus = (p == self.active && self.start_keys).then(|| self.panes[p].tab().cursor);
        paint_start(g, &mut MacStartArt { places: &self.places }, t, &v, &self.start_drives(), hovered, focus);
        true
    }

    // ---------- 최근 위치 ----------

    /// 다녀간 폴더를 최근 위치 맨 앞에 둔다 (같은 폴더는 한 번만)
    pub(super) fn note_recent(&mut self, loc: &Location) {
        shared::note_recent(&mut self.recent, loc);
    }

    /// 최근 위치에서 뺀다 (시작 화면의 ⌫, 또는 열지 못한 폴더)
    pub(super) fn forget_recent(&mut self, path: &str) {
        shared::forget_recent(&mut self.recent, path);
    }

    // ---------- 디스크 용량 ----------

    /// 위치 칸과 사이드바 디스크 줄의 용량을 읽어 달라는 경로들 (창을 열 때, 창이 앞으로 올 때, 디스크가 붙거나 떨어질 때,
    /// 시작 화면을 열 때). 창 쪽이 뒷줄에서 volume_space로 읽어 set_volumes로 넘긴다
    pub fn take_volume_ask(&mut self) -> Option<Vec<String>> {
        if !std::mem::take(&mut self.volumes_wanted) {
            return None;
        }
        Some(self.places.iter().map(|(_, path)| path.clone()).filter(|path| path.starts_with("/Volumes/")).collect())
    }

    pub fn set_volumes(&mut self, space: HashMap<String, Space>) {
        self.volumes = space;
    }

    // ---------- 새 탭 설정 ----------

    /// 설정 창의 '새 탭': 시작 화면(true) 또는 지금 폴더
    pub fn set_new_tab_start(&mut self, start: bool) {
        self.new_tab_pref = if start { session::NEW_TAB_START } else { session::NEW_TAB_SAME }.to_string();
    }

    /// 새 탭이 시작 화면으로 열리는가 (지금 폴더가 아니면 시작 화면이다: 맥에는 '내 PC'나 정해 둔 폴더가 없다)
    pub(super) fn new_tab_starts(&self) -> bool {
        self.new_tab_pref != session::NEW_TAB_SAME
    }
}
