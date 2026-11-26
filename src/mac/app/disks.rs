//! 디스크와 네트워크 공유: 사이드바 '모든 공간'의 디스크 줄(붙거나 떨어지면 다시 읽는다, 남은 용량, 오른쪽 단추의 꺼내기)과
//! 주소 칸·명령 메뉴에 친 smb:// 따위의 공유에 연결하기.
//! 맥에 묻는 일(디스크 목록, 꺼내기, 연결)은 창 쪽이 뒷줄에서 한다 (volumes.rs): 앱은 할 일을 내주고 결과를 받는다.
//! 떨어진 디스크를 보던 탭은 사라진 폴더처럼 홈으로 간다

use super::*;
use sakadu::hangul;

/// 연결을 기다리는 동안 칸에 보이는 알림 (연결이 끝나면 지운다)
const CONNECTING: &str = "연결하는 중…";
/// 꺼내는 동안의 알림
const EJECTING: &str = "꺼내는 중…";

/// 디스크에 일어난 일 (창 쪽이 맥의 알림에서 읽는다)
#[derive(Clone, Debug, PartialEq)]
pub enum VolumeEvent {
    /// 그 자리에 디스크가 붙었다
    Mounted(String),
    /// 그 자리의 디스크가 떨어졌다
    Unmounted(String),
    /// 디스크의 이름이 바뀌어 자리가 from에서 to로 옮겼다
    Renamed { from: String, to: String },
}

/// path가 root이거나 그 아래면 root 뒤에 남는 부분. 한글 이름은 합친 꼴로 견준다 (맥이 알려 주는 자리와 탭의 경로가 다른 꼴일 수 있다)
fn under_composed(path: &str, root: &str) -> Option<String> {
    under(&hangul::compose(path), &hangul::compose(root)).map(str::to_string)
}

impl App {
    /// 디스크가 붙거나 떨어지거나 이름이 바뀌었다 (창 쪽이 맥의 알림을 받아 부른다. 연결한 공유가 붙은 뒤에도 부른다):
    /// 사이드바 '모든 공간'의 바로가기를 places로 바꾸고, 용량을 다시 읽게 한다. 떨어진 디스크를 보던 탭은 홈으로 가고
    /// (방문 기록에서도 뺀다), 이름이 바뀐 디스크를 보던 탭과 검색에 더한 그 디스크 위의 폴더는 새 자리로 따라간다
    pub fn on_volumes(&mut self, places: Vec<(String, String)>, event: Option<VolumeEvent>) {
        // 디스크 메뉴와 누르고 있는 사이드바 줄은 줄 번호로 기억한다: 줄이 바뀌면 다른 디스크를 가리킬 수 있다
        if self.places != places {
            if self.menu.as_ref().is_some_and(|m| matches!(m.kind, MenuKind::Volume(_))) {
                self.menu = None;
            }
            self.side_press = None;
        }
        self.places = places;
        self.volumes_wanted = true;
        if let Some(VolumeEvent::Mounted(at) | VolumeEvent::Unmounted(at)) = &event {
            self.rescan_folders_on(at);
        }
        // 그 디스크를 보던 탭이 있을 때만 다시 읽는다 (타임머신의 스냅숏처럼 아무도 보지 않는 디스크도 수시로 붙고 떨어진다).
        // 방문 기록은 늘 고친다
        let showing = |a: &App, root: &str| a.panes.iter().flat_map(|pane| pane.tabs.iter()).any(|t| t.loc.dir().is_some_and(|d| under_composed(d, root).is_some()));
        match event {
            Some(VolumeEvent::Unmounted(gone)) => {
                let touched = showing(self, &gone);
                let shown = (0..self.layout.count()).find(|p| self.panes[*p].tab().loc.dir().is_some_and(|d| under_composed(d, &gone).is_some()));
                let home = self.home.clone();
                self.tabs_left_where(|d| under_composed(d, &gone).is_some(), &home);
                if let Some(p) = shown {
                    self.flash = Some((p, format!("‘{}’ 디스크가 빠져서 홈으로 왔어요", util::file_name(&gone))));
                }
                if touched {
                    self.refresh_changed();
                }
            }
            Some(VolumeEvent::Renamed { from, to }) => {
                let touched = showing(self, &from);
                self.tabs_moved_with(|d| under_composed(d, &from).map(|rest| format!("{to}{rest}")));
                // 검색에 더한 폴더도 새 자리로: 옛 자리는 이제 없다. 설정 창에서 폴더를 더하고 뺄 때처럼 적고 새 범위로 다시 훑는다
                let folders = &mut self.find.setting.folders;
                let mut moved = false;
                for f in folders.iter_mut() {
                    if let Some(rest) = under_composed(f, &from) {
                        *f = format!("{to}{rest}");
                        moved = true;
                    }
                }
                if moved {
                    // 새 자리에 이미 더해 둔 폴더와 겹치면 하나만 둔다 (폴더 더하기처럼)
                    let mut seen = std::collections::HashSet::new();
                    folders.retain(|f| seen.insert(f.clone()));
                    self.find_scope_changed();
                }
                if touched {
                    self.refresh_changed();
                }
            }
            Some(VolumeEvent::Mounted(_)) | None => {}
        }
    }

    /// 사이드바 줄 i가 꺼낼 수 있는 디스크 줄이면 그 메뉴 (바로가기 places의 번호로 가리킨다)
    pub(super) fn volume_menu(&self, rows: &[SideRow], i: usize) -> Option<MenuKind> {
        let row = rows.get(i).filter(|r| r.kind == SideKind::Place)?;
        if !self.ejectable(row.path.as_deref()?) {
            return None;
        }
        Some(MenuKind::Volume(rows[..i].iter().filter(|r| r.kind == SideKind::Place).count()))
    }

    /// 꺼낼 수 있는 디스크인가: 디스크가 붙은 자리 그 자체다 (시동 디스크로 가는 /Volumes의 바로가기는 아니다).
    /// 용량을 읽을 때 함께 안다 (아직 모르면 아니라고 본다)
    pub(super) fn ejectable(&self, path: &str) -> bool {
        self.volumes.get(path).is_some_and(|s| s.mount)
    }

    /// 디스크 줄 오른쪽에 옅게 보일 글: 남은 용량 (읽어 둔 디스크만)
    pub(super) fn volume_hint(&self, path: &str) -> Option<String> {
        let s = self.volumes.get(path).filter(|s| s.total > 0)?;
        Some(format!("{} 남음", listing::format_size(s.free)))
    }

    /// 바로가기 i의 디스크를 꺼내 달라고 한다 (디스크 메뉴의 '꺼내기')
    pub(super) fn eject_place(&mut self, i: usize) {
        let Some((_, path)) = self.places.get(i) else { return };
        if !self.ejectable(path) {
            return;
        }
        self.ejects.push(path.clone());
        self.flash = Some((self.active, EJECTING.to_string()));
    }

    /// 꺼내 달라는 디스크들 (창 쪽이 뒷줄에서 crate::volumes::eject로 꺼내 on_ejected로 돌려준다)
    pub fn take_ejects(&mut self) -> Vec<String> {
        std::mem::take(&mut self.ejects)
    }

    /// 꺼내기가 끝났다. 꺼냈으면 맥의 알림이 사이드바를 고친다 (on_volumes). 꺼내지 못했으면 알린다
    pub fn on_ejected(&mut self, path: &str, result: Result<(), String>) {
        if self.flash.as_ref().is_some_and(|(_, note)| note == EJECTING) {
            self.flash = None;
        }
        if let Err(why) = result {
            sakadu::diag::log(|| format!("eject {path}: {why}"));
            self.flash = Some((self.active, "꺼내지 못했어요 · 쓰고 있는 파일이 있으면 닫아 주세요".to_string()));
        }
    }

    /// 칸 p에서 친 네트워크 공유 주소에 연결해 달라고 한다 (probe_path가 부른다. probe.text는 다듬은 주소)
    pub(super) fn ask_mount(&mut self, probe: Probe) {
        self.flash = Some((probe.pane, CONNECTING.to_string()));
        self.mount_asks.push(probe);
    }

    /// 연결해 달라는 공유들 (창 쪽이 뒷줄에서 crate::volumes::mount로 연결하고 on_mounted로 돌려준다)
    pub fn take_mount_asks(&mut self) -> Vec<Probe> {
        std::mem::take(&mut self.mount_asks)
    }

    /// 연결이 끝났다: 붙은 자리(Ok)로 간다. 경로 확인과 같은 규칙이라 그 사이 그 칸에서 다른 곳으로 옮겼거나 다른 주소를 쳤으면
    /// 옮기지 않는다 (연결한 디스크는 사이드바에 보인다). 연결하지 못했으면 그 까닭을 알린다
    pub fn on_mounted(&mut self, probe: &Probe, result: Result<String, String>) {
        if self.flash.as_ref().is_some_and(|(p, note)| *p == probe.pane && note == CONNECTING) {
            self.flash = None;
        }
        self.on_probed(probe, result.map(|at| (at, None)).map_err(|why| format!("연결하지 못했어요: {why}")));
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{app, demo, mid, names, noted, side_row};
    use super::*;

    /// 시험용 디스크 줄: 바로가기의 끝에 둔다 (자리는 시험 폴더. 맥에는 묻지 않는다)
    fn with_disk(home: &str, disk: &str) -> Vec<(String, String)> {
        vec![("홈".to_string(), home.to_string()), ("SAKADU".to_string(), disk.to_string())]
    }

    /// 디스크가 떨어지면: 사이드바에서 빠지고, 그 디스크를 보던 탭은 홈으로, 방문 기록에서도 빠진다. 용량을 다시 읽는다.
    /// 이름이 바뀌면 그 디스크를 보던 탭이 새 자리로 따라가고, 검색에 더한 그 디스크 위의 폴더도 새 자리로 고쳐 적고 다시 훑는다
    #[test]
    fn disks_coming_and_going_refresh_the_sidebar_and_the_tabs() {
        let (root, home) = demo();
        let disk = util::join(&home, "다운로드");
        let (mut a, _g) = app(&home);
        a.on_volumes(with_disk(&home, &disk), Some(VolumeEvent::Mounted(disk.clone())));
        assert!(a.take_volume_ask().is_some(), "capacities are read again");
        assert!(a.side_row_center("SAKADU").is_some());
        a.navigate(1, Location::Dir(util::join(&home, "업무")), None);
        a.navigate(1, Location::Dir(disk.clone()), None);
        a.settle();
        // 그 디스크를 보던 탭: 홈으로. 앞의 기록(업무)은 남고 디스크 안은 빠진다
        a.on_volumes(vec![("홈".to_string(), home.clone())], Some(VolumeEvent::Unmounted(disk.clone())));
        a.settle();
        assert_eq!(a.pane(1).tab().loc, Location::Dir(home.clone()));
        assert!(names(&a, 1).contains(&"업무".to_string()), "the home folder is listed");
        assert!(!a.pane(1).tab().back.iter().any(|l| l.dir() == Some(disk.as_str())));
        assert!(noted(&a, "디스크가 빠져서"), "{:?}", a.flash);
        assert!(a.side_row_center("SAKADU").is_none(), "the disk row is gone");
        // 이름이 바뀐 디스크: 그 안을 보던 탭이 새 자리로 (여기서는 폴더 이름을 바꿔 흉내 낸다)
        let renamed = util::join(&home, "새 이름");
        a.on_volumes(with_disk(&home, &util::join(&home, "업무")), None);
        // 검색에 더한 폴더: 그 디스크 위의 것과 이름만 비슷한 옆 폴더 (훑는 범위는 내려받기와 더한 폴더만. 러너의 홈은 훑지 않는다)
        let beside = util::join(&home, "업무 보관");
        std::fs::create_dir(&beside).unwrap();
        a.set_find_roots(vec![util::join(&home, "다운로드")]);
        a.add_find_folder(&util::join(&home, "업무/2026 4분기"));
        a.add_find_folder(&beside);
        a.set_find_on(true);
        a.navigate(0, Location::Dir(util::join(&home, "업무/2026 4분기")), None);
        a.settle();
        std::fs::rename(util::join(&home, "업무"), &renamed).unwrap();
        a.on_volumes(with_disk(&home, &renamed), Some(VolumeEvent::Renamed { from: util::join(&home, "업무"), to: renamed.clone() }));
        // 그 디스크 위의 더한 폴더는 새 자리로 적히고(설정 창에서 더하고 뺄 때처럼 저장한다) 새 범위로 다시 훑는다. 옆 폴더는 그대로
        let moved = util::join(&renamed, "2026 4분기");
        assert_eq!(a.prefs().folders, [moved.clone(), beside.clone()]);
        let opts = a.take_index_start().expect("a new index is asked for");
        assert!(opts.roots.contains(&moved) && !opts.roots.contains(&util::join(&home, "업무/2026 4분기")), "{:?}", opts.roots);
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(util::join(&renamed, "2026 4분기")));
        assert_eq!(names(&a, 0), ["보고서.docx"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 꺼내기는 꺼낼 수 있는 디스크 줄의 오른쪽 단추 메뉴에만 있다 (홈 같은 바로가기, 시동 디스크로 가는 바로가기에는 없다).
    /// 고르면 그 디스크를 꺼내 달라고 하고, 꺼내지 못하면 알린다. 디스크 줄에는 남은 용량이 보인다
    #[test]
    fn eject_is_offered_only_on_disk_rows() {
        let (root, home) = demo();
        let (mut a, g) = app(&home);
        let (usb, link) = ("/Volumes/SAKADU-TEST".to_string(), "/Volumes/Macintosh HD".to_string());
        let mut places = with_disk(&home, &usb);
        places.push(("Macintosh HD".to_string(), link.clone()));
        a.on_volumes(places, Some(VolumeEvent::Mounted(usb.clone())));
        a.settle();
        // 맥에 묻지 않고 용량을 넣는다 (USB는 디스크가 붙은 자리, Macintosh HD는 시동 디스크로 가는 바로가기)
        let space = |mount| Space { free: 3 << 30, total: 16 << 30, mount };
        a.set_volumes([(usb.clone(), space(true)), (link.clone(), space(false))].into());
        assert_eq!(a.volume_hint(&usb).as_deref(), Some("3.0 GB 남음"));
        assert_eq!(a.volume_hint(&home), None);
        let at = |a: &App, label: &str| mid(side_row(a, label));
        for label in ["홈", "Macintosh HD"] {
            let (x, y) = at(&a, label);
            a.right_down(&g, x, y);
            assert_eq!(a.menu_shown(), None, "{label}");
        }
        let (x, y) = at(&a, "SAKADU");
        assert!(a.right_down(&g, x, y));
        assert_eq!(a.menu_shown().map(|(k, _)| k), Some(MenuKind::Volume(1)));
        assert_eq!(a.menu_items(MenuKind::Volume(1)), [("꺼내기".to_string(), false)]);
        a.pick_open_menu(0);
        assert_eq!(a.take_ejects(), std::slice::from_ref(&usb));
        assert!(noted(&a, "꺼내는 중"), "{:?}", a.flash);
        a.on_ejected(&usb, Err("사용 중".to_string()));
        assert!(noted(&a, "꺼내지 못했어요"), "{:?}", a.flash);
        // 메뉴가 떠 있는 동안 디스크 줄이 바뀌면 메뉴를 닫는다 (다른 디스크를 가리키지 않게)
        a.right_down(&g, x, y);
        a.on_volumes(vec![("홈".to_string(), home.clone())], Some(VolumeEvent::Unmounted(usb.clone())));
        assert_eq!(a.menu_shown(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 주소 칸과 명령 메뉴에 친 네트워크 공유 주소는 경로 확인 대신 연결 요청이 된다 (시험에서는 연결하지 않는다).
    /// 붙은 자리를 받으면 그리로 가고, 연결하지 못하면 알린다. 그 사이 다른 곳으로 옮겼으면 옮기지 않는다
    #[test]
    fn share_addresses_ask_to_connect() {
        let (root, home) = demo();
        let (mut a, _g) = app(&home);
        a.begin_address(0);
        a.commit_address("smb://nas.local/사진 폴더");
        assert!(a.take_probes().is_empty(), "a share is not probed as a path");
        let asks = a.take_mount_asks();
        assert_eq!(asks.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["smb://nas.local/%EC%82%AC%EC%A7%84%20%ED%8F%B4%EB%8D%94"]);
        assert!(noted(&a, "연결하는 중"), "{:?}", a.flash);
        // 붙은 자리로 간다 (시험에서는 내려받기 폴더를 붙은 자리로 친다)
        let at = util::join(&home, "다운로드");
        a.on_mounted(&asks[0], Ok(at.clone()));
        a.settle();
        assert_eq!(a.pane(0).tab().loc, Location::Dir(at.clone()));
        assert_eq!(a.flash, None, "the connecting note is gone");
        // 연결하지 못했다: 까닭을 알린다
        a.begin_address(0);
        a.commit_address("afp://nas.local/share");
        let asks = a.take_mount_asks();
        a.on_mounted(&asks[0], Err("서버에 닿지 않아요".to_string()));
        assert!(noted(&a, "연결하지 못했어요: 서버에 닿지 않아요"), "{:?}", a.flash);
        // 연결을 기다리는 사이 다른 곳으로 옮겼다: 늦게 붙어도 그 칸을 옮기지 않는다
        a.begin_address(0);
        a.commit_address("smb://nas.local/share");
        let asks = a.take_mount_asks();
        a.navigate(0, Location::Dir(util::join(&home, "업무")), None);
        a.settle();
        a.on_mounted(&asks[0], Ok(at));
        assert_eq!(a.pane(0).tab().loc, Location::Dir(util::join(&home, "업무")));
        // 명령 메뉴의 '경로로 이동'도 같은 길로 간다
        a.toggle_commands();
        a.cmd_typed("smb://nas.local/share");
        a.cmd_key(Key::Return, Mods::default());
        assert_eq!(a.take_mount_asks().len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
