//! 폴더 목록 불러오기와 그 결과 반영

use super::*;

impl App {
    // ---------- 목록 불러오기 ----------

    pub(super) fn load(&mut self, p: usize, keep: bool) {
        let t = self.panes[p].active;
        self.load_at(p, t, keep);
    }

    /// keep이면 지금 목록을 보여 둔 채 새로 읽고, 선택·스크롤을 되살린다
    pub(super) fn load_at(&mut self, p: usize, t: usize, keep: bool) {
        let tab = &mut self.panes[p].tabs[t];
        tab.generation += 1;
        if keep {
            if tab.restore.is_none() {
                tab.restore = Some(tab.snapshot());
            }
            // 열지 못했던 폴더를 다시 읽을 때는 다시 "불러오는 중"으로 (오류 글이 그대로 남아 아무 일도 없는 것처럼 보이지 않게)
            if !matches!(tab.state, LoadState::Ready) {
                tab.state = LoadState::Loading;
            }
        } else {
            tab.state = LoadState::Loading;
        }
        if tab.loc != Location::Start {
            tab.loading_since = Some(std::time::Instant::now());
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_LOADING, LOADING_SHOW_MS as u32 + 20, None);
            }
        }
        spawn_listing(self.hwnd, tab.id, tab.generation, tab.loc.clone());
    }

    pub(super) fn find_tab(&self, id: u64) -> Option<(usize, usize)> {
        self.panes.iter().enumerate().find_map(|(p, pane)| pane.tabs.iter().position(|t| t.id == id).map(|t| (p, t)))
    }

    pub(super) fn on_listed(&mut self, l: Listed) {
        let Some((p, t)) = self.find_tab(l.tab) else { return };
        let (sort, hidden) = (self.panes[p].sort, self.show_hidden);
        let is_active = t == self.panes[p].active;
        let tab = &mut self.panes[p].tabs[t];
        if tab.generation != l.generation {
            return;
        }
        let waited = tab.loading_since.take().is_some();
        let ok = l.result.is_ok();
        let mut went_offline = false;
        let restore = match l.result {
            Ok(entries) => {
                if let Some(dir) = tab.loc.dir() {
                    self.folders.observe(dir, &entries);
                    // 네트워크 폴더는 마지막으로 본 목록을 기억해 둔다 (연결이 끊기면 흐리게 보여 준다)
                    if !is_local_path(dir) && entries.len() <= 20_000 {
                        if self.net_cache.len() >= 64 {
                            self.net_cache.clear();
                        }
                        self.net_cache.insert(dir.to_lowercase(), entries.clone());
                    }
                }
                tab.offline = false;
                tab.net_fix = None;
                tab.set_entries(entries, sort, hidden)
            }
            Err(msg) => {
                // 로그인이 필요하면 단추를 보이고, 닿지 못한 것이면 기억한 목록을 흐리게
                let cached = match (&l.net, tab.loc.dir()) {
                    (Some(f), _) if f.ask => None,
                    (_, Some(dir)) if !is_local_path(dir) => self.net_cache.get(&dir.to_lowercase()).cloned(),
                    _ => None,
                };
                tab.net_fix = l.net.clone();
                match cached {
                    Some(entries) => {
                        went_offline = !tab.offline && is_active;
                        tab.offline = true;
                        tab.set_entries(entries, sort, hidden)
                    }
                    None => {
                        tab.offline = false;
                        tab.entries.clear();
                        tab.view.clear();
                        tab.state = LoadState::Error(msg);
                        None
                    }
                }
            }
        };
        // 지워졌거나 옮겨진 폴더는 시작 화면의 최근 위치에서 뺀다 (잠깐 못 연 것은 그대로)
        if l.gone {
            if let Some(d) = self.panes[p].tabs[t].loc.dir().map(str::to_string) {
                self.forget_recent(&d);
            }
        }
        if self.panes[p].active == t {
            self.clamp_scroll(p);
            if p == self.active {
                self.layout_edits();
            }
            if let Some(r) = restore {
                if r.reveal {
                    self.ensure_visible(p);
                }
                // 고르려던 항목(새 폴더)을 실제로 찾았을 때만: 못 찾았는데 열면 엉뚱한 항목의 이름이 바뀐다
                let found = self.panes[p].tab().cursor_entry().map(|e| e.name_string().to_lowercase()) == r.cursor;
                if r.rename && p == self.active && found && self.edit_mode == EditMode::None {
                    self.begin_rename(p);
                }
            }
            // 이름을 바꾸는 중에 목록이 다시 오면(폴더 감시, 앞선 작업의 고르기) 상자를 바꾸던 항목에 그대로 둔다
            if self.edit_mode == EditMode::Rename(p) {
                if let Some(key) = self.rename.as_ref().filter(|c| c.tab == l.tab).map(|c| c.old.to_lowercase()) {
                    let tab = self.panes[p].tab_mut();
                    match tab.view.iter().position(|&i| tab.entries[i as usize].name_string().to_lowercase() == key) {
                        Some(vi) => {
                            tab.cursor = vi;
                            tab.anchor = vi;
                            let i = tab.view[vi] as usize;
                            tab.entries[i].selected = true;
                        }
                        None => {
                            self.cancel_edit();
                            self.flash(p, "바꾸던 항목이 다른 곳에서 지워졌거나 옮겨졌어요");
                        }
                    }
                }
            }
            if self.edit_mode == EditMode::Rename(p) {
                self.layout_edits();
            }
            // 다 읽은 폴더를 이제 감시한다 (읽기 전에는 꺼진 NAS를 기다리며 멈출 수 있어 미뤄 두었다)
            if ok {
                self.sync_watches();
            }
            if waited {
                self.refresh_cursor();
            }
        }
        if self.panes[p].tabs.get(t).is_some_and(|tab| tab.offline) {
            if went_offline {
                self.flash(p, "연결이 끊겨 마지막으로 본 목록을 보여 줘요 · 다시 연결되면 저절로 새로 고쳐요");
            }
            unsafe { SetTimer(Some(self.hwnd), TIMER_OFFLINE, 10_000, None) };
        }
        self.invalidate();
    }

    /// 네트워크 연결(로그인 창)이 끝났다: 되면 그 탭을 다시 읽고, 안 됐으면 까닭을 알린다 (취소는 조용히)
    pub(super) fn after_net_fix(&mut self, tab: u64, r: u32) {
        let Some((p, t)) = self.find_tab(tab) else { return };
        match r {
            0 => self.load_at(p, t, true),
            1223 => {}
            e => {
                let msg = windows::core::Error::from_hresult(windows::Win32::Foundation::WIN32_ERROR(e).to_hresult()).message();
                self.flash(p, format!("연결하지 못했어요 · {msg}"));
            }
        }
        self.invalidate();
    }

    /// 열지 못한 네트워크 폴더의 로그인·다시 연결 단추 (목록 자리 안내 글 아래)
    pub(super) fn net_button(&self, p: usize) -> Option<(R, &'static str)> {
        let tab = self.panes[p].tab();
        let f = tab.net_fix.as_ref().filter(|_| matches!(tab.state, LoadState::Error(_)))?;
        let list = geom(self.pane_rect(p)?).list;
        let label = if f.ask { "로그인" } else { "다시 연결" };
        let w = self.gfx.measure(label, Font::Normal) + 32.0;
        Some((R::new(list.l + 24.0, list.t + 120.0, list.l + 24.0 + w, list.t + 152.0), label))
    }

    pub(super) fn net_fix_click(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        if let Some(f) = tab.net_fix.clone() {
            self.pending.push(Deferred::NetFix { tab: tab.id, share: f.share, ask: f.ask });
        }
    }

    pub(super) fn on_icons(&mut self, ic: IconsReady) {
        let Some((p, t)) = self.find_tab(ic.tab) else { return };
        let tab = &mut self.panes[p].tabs[t];
        if tab.generation != ic.generation {
            return;
        }
        for (i, icon) in ic.icons {
            if let Some(e) = tab.entries.get_mut(i as usize) {
                e.icon = icon;
            }
        }
        self.invalidate();
    }

    pub(super) fn sync_watches(&mut self) {
        for p in 0..4 {
            // 다 읽은 폴더만 감시한다. 읽는 중이거나 열지 못한 폴더(꺼진 NAS)는 등록하는 동안 네트워크를 기다리며 창이 멈출 수 있다
            let want = (p < self.layout.count())
                .then(|| {
                    let tab = self.panes[p].tab();
                    tab.loc
                        .dir()
                        .filter(|d| matches!(tab.state, LoadState::Ready) || (matches!(tab.state, LoadState::Loading) && is_local_path(d)))
                        .map(String::from)
                })
                .flatten();
            if want.as_deref() == self.watches[p].as_ref().map(|(_, d)| d.as_str()) {
                continue;
            }
            if let Some((id, _)) = self.watches[p].take() {
                if id != 0 {
                    shell::unwatch(id);
                }
            }
            if let Some(d) = want {
                // 등록에 실패해도(id 0) 기억해 두어, 같은 폴더를 다른 칸·탭을 오갈 때마다 다시 시도하지 않는다 (F5면 다시)
                let id = shell::watch(self.hwnd, &d, WM_APP_NOTIFY + p as u32);
                self.watches[p] = Some((id, d));
            }
        }
    }
}
