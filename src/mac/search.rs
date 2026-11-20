//! 맥 검색: 홈 폴더 같은 범위를 훑어 파일·폴더 이름을 메모리에 두고(디스크에 저장하지 않는다), 맥의 변경 기록(FSEvents)으로 최신으로 둔다.
//! 글자를 칠 때마다 이 색인에서 바로 찾는다. 이름 저장·순위·검색어 규칙은 Windows 검색과 같다 (index.rs, nametree.rs).
//!
//! 줄 나눔
//! - 폴더를 읽는 줄 몇 개: 낮은 우선순위로 폴더를 읽기만 한다 (색인을 잠그지 않는다)
//! - 주인 줄 하나: 읽은 결과와 변경 알림을 색인에 반영하고, 다음에 읽을 폴더를 정한다 (색인을 고치는 것은 이 줄뿐이다)
//! - 찾는 쪽: 읽기 잠금으로 찾는다
//!
//! 변경 기록은 훑기 전에 듣기 시작한다: 훑는 동안 바뀐 것은 모아 두었다가 처음 훑기를 마친 뒤 반영한다
//! (폴더를 다시 읽는 것이라 여러 번 해도 같다). 맥이 접근을 묻는 폴더(데스크탑·문서·다운로드, iCloud Drive)는
//! 나머지를 다 훑은 뒤 하나씩 읽는다: 묻는 창이 한꺼번에 뜨지 않고, 답을 기다리는 동안에도 다른 폴더는 찾을 수 있다

mod crawl;
mod fsevents;

use crawl::{Listed, Rules};
use fsevents::Change;
use sakadu::hangul;
use sakadu::index::Query;
use sakadu::nametree::{NameTree, Place};
use sakadu::query::{self, Merged};
use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

/// 변경 알림을 모으는 시간(초)
const LATENCY: f64 = 0.3;
/// 훑는 동안 '색인이 바뀌었다'고 알리는 가장 짧은 간격 (검색 화면이 같은 검색어로 다시 찾는다)
const NOTICE_EVERY: Duration = Duration::from_millis(250);
/// '지금 폴더'로 좁혀 찾을 때 더 넉넉히 찾는 배수 (폴더 밖 결과를 걸러낸 뒤 남는 것이 있도록. Windows 검색과 같다)
const SCOPE_WIDEN: usize = 10;

pub struct Options {
    /// 훑는 범위의 맨 위 폴더들 (디스크의 실제 경로)
    pub roots: Vec<String>,
    /// 홈 폴더: 그 바로 아래의 라이브러리·휴지통은 훑지 않는다
    pub home: String,
    /// 맥이 접근을 묻는 폴더 (데스크탑·문서·다운로드, iCloud Drive): 나머지를 다 훑은 뒤 하나씩 읽는다
    pub protected: Vec<String>,
    /// 폴더를 읽는 줄 수
    pub threads: usize,
    /// 패키지(.app 등)를 Finder처럼 파일 하나로 본다. 측정할 때만 꺼서 안까지 훑는다
    pub packages_are_files: bool,
    /// 색인에 둘 이름 수의 상한 (측정할 때). 넘으면 더 훑지 않는다
    pub max_entries: usize,
    /// 변경 기록을 받아 최신으로 둔다
    pub watch: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    /// 처음 훑기를 마쳤다 (그 뒤로는 바뀐 것을 반영한다)
    pub ready: bool,
    /// 색인에 든 이름 수
    pub entries: usize,
    /// 읽은 폴더 수 (다시 읽은 것 포함)
    pub dirs_listed: usize,
    /// 권한이 없어 읽지 못한 폴더
    pub denied: Vec<String>,
    /// 이 맥에 내려받지 않아서 읽지 않은 폴더 수 (iCloud)
    pub not_local: usize,
    pub errors: usize,
    /// 색인의 대략적인 메모리(바이트)
    pub memory: usize,
    /// 처음 훑기에 걸린 시간
    pub crawl_time: Option<Duration>,
    /// 받은 변경 알림 수
    pub changes: usize,
    /// 이름 수의 상한에 닿아 더 훑지 않았다
    pub capped: bool,
}

/// 찾은 결과: 위에서부터 limit개 (폴더 여부, 경로. 경로의 한글은 합친 꼴), 맞는 것 전체 수, 걸린 시간
pub struct Found {
    pub hits: Vec<(bool, String)>,
    pub total: usize,
    pub took: Duration,
}

/// 처음 정한 검색 범위: 홈 폴더(라이브러리·휴지통은 훑지 않는다)와 응용 프로그램 폴더 둘. iCloud Drive는 고른 사람만.
/// 맥이 접근을 묻는 폴더(데스크탑·문서·다운로드, iCloud Drive)도 함께 돌려준다: 나머지를 다 훑은 뒤 하나씩 읽는다
pub fn default_scope(home: &str, icloud: bool) -> (Vec<String>, Vec<String>) {
    let mut roots = vec![home.to_string(), "/Applications".to_string(), "/System/Applications".to_string()];
    let mut protected: Vec<String> = ["Desktop", "Documents", "Downloads"].iter().map(|d| sakadu::util::join(home, d)).collect();
    if icloud {
        let drive = icloud_drive(home);
        roots.push(drive.clone());
        protected.push(drive);
    }
    (roots, protected)
}

/// iCloud Drive 폴더 (홈의 라이브러리 안)
pub fn icloud_drive(home: &str) -> String {
    sakadu::util::join(home, "Library/Mobile Documents/com~apple~CloudDocs")
}

/// 검색 설정 (설정 폴더의 search.txt): 켰는지, iCloud Drive도 찾는지. 기본은 꺼짐
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Setting {
    pub on: bool,
    pub icloud: bool,
}

impl Setting {
    const HEADER: &str = "sakadu-search 1";

    fn path() -> Option<std::path::PathBuf> {
        sakadu::util::config_dir().map(|d| d.join("search.txt"))
    }

    pub fn load() -> Setting {
        Setting::path().and_then(|p| std::fs::read_to_string(p).ok()).map_or_else(Setting::default, |t| Setting::parse(&t))
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Setting::path().ok_or_else(|| std::io::Error::other("HOME 없음"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.text())
    }

    fn parse(text: &str) -> Setting {
        let mut lines = text.lines();
        let mut s = Setting::default();
        if lines.next().map(str::trim) != Some(Setting::HEADER) {
            return s;
        }
        for line in lines {
            match line.split_once('=') {
                Some(("on", v)) => s.on = v == "1",
                Some(("icloud", v)) => s.icloud = v == "1",
                _ => {}
            }
        }
        s
    }

    fn text(&self) -> String {
        format!("{}\non={}\nicloud={}\n", Setting::HEADER, self.on as u8, self.icloud as u8)
    }
}

/// 찾는 쪽 손잡이: 다른 줄에서도 찾을 수 있다 (검색 화면은 입력할 때마다 뒷줄에서 찾는다). 색인이 멈춰도 마지막 모습으로 찾는다
#[derive(Clone)]
pub struct Finder {
    shared: Arc<Shared>,
}

impl Finder {
    /// 이름으로 찾는다. 검색어 규칙은 Windows 검색과 같다 (sakadu::query): '|'로 나눈 조각은 또는, 띄어 쓰면 모두 포함,
    /// '/'가 든 낱말은 경로에서. scope가 있으면 그 폴더 아래만 남긴다
    pub fn find(&self, text: &str, scope: Option<&str>, limit: usize) -> Found {
        let started = Instant::now();
        let each = if scope.is_some() { limit * SCOPE_WIDEN } else { limit };
        let mut merged = Merged::exact();
        let mut total = 0;
        let scope = {
            let tree = read(&self.shared.tree);
            // 범위는 색인에 든 그 폴더의 경로로 바꿔 쓴다: 한글은 합친 꼴, 대소문자는 디스크 그대로
            // (대소문자를 가리지 않는 디스크에서 다르게 친 경로도 그 폴더이고, 가리는 디스크에서 Work와 work는 다른 폴더다)
            let scope = scope.map(|d| {
                let d = hangul::compose(d).into_owned();
                match tree.locate(&d) {
                    Place::Dir(slot) => tree.path(slot).unwrap_or(d),
                    _ => d,
                }
            });
            // 범위의 경로 조건은 검색어를 조각으로 나눈 뒤에 덧붙인다 (폴더 이름에 든 '|'가 '또는'이 되지 않게)
            let terms = scope.as_deref().map(|d| query::scope_terms(d, '/'));
            for piece in query::unique_pieces(text) {
                let piece = match &terms {
                    Some(t) => format!("{piece} {t}"),
                    None => piece.to_string(),
                };
                let (hits, n) = tree.search(&Query::parse_posix(&piece), each);
                total += n;
                for h in hits {
                    merged.add(h.score, h.is_dir, h.path);
                }
            }
            scope
        };
        let mut hits = merged.into_sorted(each);
        if let Some(dir) = &scope {
            hits.retain(|(_, p)| query::under_exact(p, dir, '/'));
            total = hits.len();
        }
        hits.truncate(limit);
        Found { hits, total, took: started.elapsed() }
    }

    /// dir이 훑는 범위 안인가 (밖이면 '지금 폴더'로 찾아도 나오지 않는다: 홈의 라이브러리, 패키지 안, 다른 디스크)
    pub fn covers(&self, dir: &str) -> bool {
        !matches!(read(&self.shared.tree).locate(dir), Place::Ignore)
    }
}

/// 읽을 폴더 하나. deep이면 그 아래 폴더도 모두 다시 읽는다 (알림을 놓쳤을 때)
struct Job {
    slot: u32,
    /// 디스크의 실제 경로 (색인의 이름은 한글을 합친 꼴이라 디스크와 다를 수 있다)
    path: String,
    deep: bool,
}

enum Msg {
    Listed(Job, Listed),
    Changes(Vec<Change>),
    /// 할 일이 없어지면 답한다
    Idle(Sender<()>),
    Stop,
}

struct Shared {
    tree: RwLock<NameTree>,
    status: Mutex<Status>,
    work: Mutex<Work>,
    work_ready: Condvar,
}

#[derive(Default)]
struct Work {
    queue: VecDeque<Job>,
    stop: bool,
}

/// 검색 색인. 없애면 훑기와 변경 받기를 멈춘다
pub struct Search {
    shared: Arc<Shared>,
    tx: Sender<Msg>,
    stream: Option<fsevents::Stream>,
}

impl Search {
    /// 훑기를 시작한다 (바로 돌아온다). 색인이 바뀔 때마다(훑는 동안은 NOTICE_EVERY마다) 주인 줄에서 on_change를 부른다
    pub fn start(opts: Options, on_change: impl Fn() + Send + 'static) -> Search {
        let (tx, rx) = channel();
        // 변경 기록부터 듣는다: 훑는 사이에 바뀐 것도 놓치지 않는다
        let stream = if opts.watch {
            let tx = tx.clone();
            fsevents::Stream::start(&opts.roots, LATENCY, move |changes| {
                let _ = tx.send(Msg::Changes(changes));
            })
        } else {
            None
        };
        let mut tree = NameTree::new();
        let first: Vec<Job> = opts.roots.iter().map(|r| Job { slot: tree.add_root(r), path: r.clone(), deep: false }).collect();
        let shared = Arc::new(Shared { tree: RwLock::new(tree), status: Mutex::new(Status::default()), work: Mutex::new(Work::default()), work_ready: Condvar::new() });
        let rules = Arc::new(Rules { home: opts.home, packages_are_files: opts.packages_are_files, mounts: crawl::mount_points() });
        for _ in 0..opts.threads.max(1) {
            let (shared, tx, rules) = (shared.clone(), tx.clone(), rules.clone());
            std::thread::spawn(move || read_folders(&shared, &tx, &rules));
        }
        let owner = Owner::new(shared.clone(), opts.protected, opts.max_entries, Box::new(on_change));
        std::thread::spawn(move || owner.run(rx, first));
        Search { shared, tx, stream }
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    pub fn finder(&self) -> Finder {
        Finder { shared: self.shared.clone() }
    }

    /// 이름으로 찾는다 (Finder::find, 범위 없이)
    pub fn search(&self, text: &str, limit: usize) -> Found {
        self.finder().find(text, None, limit)
    }

    /// 할 일(읽을 폴더, 반영할 알림)이 없어질 때까지 기다린다. 맥이 아직 넘기지 않은 변경 알림도 먼저 넘겨받는다.
    /// limit까지만 기다린다. 할 일이 없어졌으면 true
    pub fn settle(&self, limit: Duration) -> bool {
        if let Some(stream) = &self.stream {
            stream.flush();
        }
        let (tx, rx) = channel();
        self.tx.send(Msg::Idle(tx)).is_ok() && rx.recv_timeout(limit).is_ok()
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        // 알림부터 끊고 주인 줄을 끝낸다. 폴더를 읽는 줄은 기다리지 않는다: 권한 창의 답을 기다리는 줄이 있을 수 있다
        // (주인 줄이 멈추라고 알리면 지금 읽는 폴더까지만 읽고 끝난다)
        self.stream = None;
        let _ = self.tx.send(Msg::Stop);
    }
}

/// 폴더를 읽는 줄: 일을 받아 폴더를 읽고 결과를 주인 줄에 넘긴다
fn read_folders(shared: &Shared, tx: &Sender<Msg>, rules: &Rules) {
    crawl::prepare_thread();
    loop {
        let job = {
            let mut work = lock(&shared.work);
            loop {
                if work.stop {
                    return;
                }
                if let Some(job) = work.queue.pop_front() {
                    break job;
                }
                work = shared.work_ready.wait(work).unwrap_or_else(|e| e.into_inner());
            }
        };
        let listed = crawl::list(&job.path, rules);
        if tx.send(Msg::Listed(job, listed)).is_err() {
            return;
        }
    }
}

/// 주인 줄: 색인을 고치는 유일한 줄
struct Owner {
    shared: Arc<Shared>,
    protected: HashSet<String>,
    /// 처음 훑기에서 미뤄 둔 폴더 (맥이 접근을 묻는 폴더)
    later: VecDeque<Job>,
    /// 읽으라고 넘겼지만 아직 결과가 오지 않은 폴더 (한글을 합친 경로) → 결과가 온 뒤에 한 번 더 읽을지 (Some(그 아래까지))
    busy: HashMap<String, Option<bool>>,
    /// 읽으라고 넘겼지만 아직 결과가 오지 않은 폴더 수
    in_flight: usize,
    ready: bool,
    /// 처음 훑기를 마치기 전에 온 변경 알림
    held: Vec<Change>,
    /// 할 일이 없어지기를 기다리는 쪽
    waiting: Vec<Sender<()>>,
    max_entries: usize,
    started: Instant,
    last_notice: Option<Instant>,
    on_change: Box<dyn Fn() + Send>,
}

impl Owner {
    fn new(shared: Arc<Shared>, protected: Vec<String>, max_entries: usize, on_change: Box<dyn Fn() + Send>) -> Owner {
        Owner {
            shared,
            protected: protected.into_iter().collect(),
            later: VecDeque::new(),
            busy: HashMap::new(),
            in_flight: 0,
            ready: false,
            held: Vec::new(),
            waiting: Vec::new(),
            max_entries,
            started: Instant::now(),
            last_notice: None,
            on_change,
        }
    }

    fn run(mut self, rx: Receiver<Msg>, first: Vec<Job>) {
        self.push(first);
        loop {
            if self.in_flight == 0 {
                self.out_of_work();
            }
            let Ok(msg) = rx.recv() else { break };
            match msg {
                Msg::Listed(job, listed) => self.listed(job, listed),
                Msg::Changes(changes) if self.ready => {
                    lock(&self.shared.status).changes += changes.len();
                    let jobs = self.jobs_for(&changes);
                    self.push(jobs);
                }
                Msg::Changes(changes) => self.held.extend(changes),
                Msg::Idle(answer) => self.waiting.push(answer),
                Msg::Stop => break,
            }
        }
        let mut work = lock(&self.shared.work);
        work.stop = true;
        work.queue.clear();
        drop(work);
        self.shared.work_ready.notify_all();
    }

    /// 폴더를 하나 읽었다: 색인에 반영하고 이어서 읽을 폴더를 넘긴다. 읽는 사이에 그 폴더의 알림이 또 왔으면 한 번 더 읽는다
    fn listed(&mut self, job: Job, listed: Listed) {
        self.in_flight -= 1;
        let again = self.busy.remove(&*hangul::compose(&job.path)).flatten();
        let again = again.map(|deep| Job { slot: job.slot, path: job.path.clone(), deep });
        let mut next = self.apply(job, listed);
        next.extend(again);
        self.push(next);
        self.notice(false);
    }

    /// 읽을 폴더가 떨어졌다: 미뤄 둔 폴더를 하나 읽거나, 처음 훑기를 마친다. 그래도 할 일이 없으면 기다리던 쪽에 답한다
    fn out_of_work(&mut self) {
        if let Some(job) = self.later.pop_front() {
            self.send(vec![job]);
            return;
        }
        if !self.ready {
            self.ready = true;
            let held = std::mem::take(&mut self.held);
            {
                let mut status = lock(&self.shared.status);
                status.ready = true;
                status.crawl_time = Some(self.started.elapsed());
                status.changes += held.len();
            }
            let jobs = self.jobs_for(&held);
            self.push(jobs);
            self.notice(true);
            if self.in_flight > 0 {
                return;
            }
        }
        for answer in self.waiting.drain(..) {
            let _ = answer.send(());
        }
    }

    /// 읽을 폴더를 넘긴다. 처음 훑기 동안 맥이 접근을 묻는 폴더는 미뤄 둔다
    fn push(&mut self, jobs: Vec<Job>) {
        let (later, now): (Vec<Job>, Vec<Job>) = jobs.into_iter().partition(|j| !self.ready && self.protected.contains(&j.path));
        self.later.extend(later);
        self.send(now);
    }

    /// 한 폴더는 한 번에 한 줄만 읽는다: 이미 읽는 중이면 그 결과가 온 뒤에 한 번 더 읽는다.
    /// 함께 읽으면 늦게 시작한 쪽이 먼저 끝날 수 있고, 그러면 먼저 시작한 쪽의 옛 목록이 나중에 반영되어 색인이 옛 모습으로 남는다
    fn send(&mut self, jobs: Vec<Job>) {
        let mut now = Vec::new();
        for job in jobs {
            match self.busy.entry(hangul::compose(&job.path).into_owned()) {
                Entry::Occupied(mut again) => {
                    let deep = job.deep || *again.get() == Some(true);
                    again.insert(Some(deep));
                }
                Entry::Vacant(free) => {
                    free.insert(None);
                    now.push(job);
                }
            }
        }
        if now.is_empty() {
            return;
        }
        self.in_flight += now.len();
        lock(&self.shared.work).queue.extend(now);
        self.shared.work_ready.notify_all();
    }

    /// 읽은 폴더를 색인에 반영하고, 이어서 읽을 폴더를 돌려준다
    fn apply(&self, job: Job, listed: Listed) -> Vec<Job> {
        let mut tree = write(&self.shared.tree);
        let mut status = lock(&self.shared.status);
        status.dirs_listed += 1;
        let Some(slot) = current_slot(&tree, &job) else { return Vec::new() };
        let next = match listed {
            Listed::Items(items) => {
                let fresh = tree.sync_dir(slot, &items);
                let dirs = if job.deep { tree.child_dirs(slot) } else { fresh };
                if tree.len() >= self.max_entries {
                    status.capped = true;
                    Vec::new()
                } else {
                    // 다음에 읽을 폴더의 실제 경로는 디스크의 이름으로 만든다
                    let on_disk: HashMap<String, &str> = items
                        .iter()
                        .filter_map(|it| match hangul::compose(&it.name) {
                            Cow::Owned(composed) => Some((composed, it.name.as_str())),
                            Cow::Borrowed(_) => None,
                        })
                        .collect();
                    dirs.into_iter()
                        .filter_map(|s| {
                            let name = tree.name(s)?;
                            let disk = on_disk.get(name).copied().unwrap_or(name);
                            Some(Job { slot: s, path: sakadu::util::join(&job.path, disk), deep: job.deep })
                        })
                        .collect()
                }
            }
            Listed::Denied => {
                if !status.denied.contains(&job.path) {
                    status.denied.push(job.path.clone());
                }
                Vec::new()
            }
            Listed::NotLocal => {
                status.not_local += 1;
                Vec::new()
            }
            // 폴더가 없어졌다: 그 안을 뺀다. 폴더 자체는 그 위 폴더를 다시 읽을 때 빠진다.
            // 훑는 범위의 맨 위 폴더는 그 위를 읽지 않으니, 여기서 빼지 않으면 옮겨진 뒤에도 그 안이 검색에 남는다
            Listed::Gone => {
                tree.sync_dir(slot, &[]);
                Vec::new()
            }
            Listed::Failed => {
                status.errors += 1;
                Vec::new()
            }
        };
        tree.tidy();
        status.entries = tree.len();
        status.memory = tree.memory();
        next
    }

    /// 변경 알림을 다시 읽을 폴더로 바꾼다. 같은 폴더는 한 번만, 알림을 놓쳤다는 표시가 있으면 그 아래까지
    fn jobs_for(&self, changes: &[Change]) -> Vec<Job> {
        let tree = read(&self.shared.tree);
        let mut jobs: Vec<Job> = Vec::new();
        let mut at: HashMap<u32, usize> = HashMap::new();
        for c in changes {
            let lost = fsevents::MUST_SCAN_SUB_DIRS | fsevents::USER_DROPPED | fsevents::KERNEL_DROPPED | fsevents::ROOT_CHANGED;
            let deep = c.flags & lost != 0;
            let path = trim_slash(&c.path);
            let (slot, real) = match tree.locate(path) {
                Place::Dir(s) => (s, path.to_string()),
                // 알림의 경로에서 그 폴더까지의 앞부분이 디스크의 실제 경로다
                Place::Below(s) => (s, leading(path, tree.path(s).map_or(0, |p| p.split('/').filter(|x| !x.is_empty()).count()))),
                Place::Ignore => continue,
            };
            match at.get(&slot) {
                Some(&k) => jobs[k].deep |= deep,
                None => {
                    at.insert(slot, jobs.len());
                    jobs.push(Job { slot, path: real, deep });
                }
            }
        }
        jobs
    }

    /// 색인이 바뀌었다고 알린다 (훑는 동안은 너무 자주 알리지 않는다)
    fn notice(&mut self, now: bool) {
        let t = Instant::now();
        if now || self.in_flight == 0 || self.last_notice.is_none_or(|last| t - last >= NOTICE_EVERY) {
            self.last_notice = Some(t);
            (self.on_change)();
        }
    }
}

/// 그 일의 폴더가 지금 색인의 어느 칸인가: 읽는 사이에 색인이 바뀌어 그 칸이 다른 것이 됐으면 경로로 다시 찾는다.
/// 같은 경로라도 그 칸이 이제 폴더가 아니면 버린다: 읽는 사이에 그 폴더가 같은 이름의 파일로 바뀌면 빈 칸을 다시 써서
/// 같은 칸이 그 파일이 될 수 있고, 그 칸에 옛 폴더의 목록을 붙이면 파일 아래에 없는 항목이 남는다
fn current_slot(tree: &NameTree, job: &Job) -> Option<u32> {
    if tree.is_dir(job.slot) && tree.path(job.slot).is_some_and(|p| p == hangul::compose(&job.path)) {
        return Some(job.slot);
    }
    match tree.locate(&job.path) {
        Place::Dir(s) => Some(s),
        _ => None,
    }
}

/// 끝의 '/'를 뗀다 ('/' 자체는 그대로)
fn trim_slash(path: &str) -> &str {
    let t = path.trim_end_matches('/');
    if t.is_empty() { "/" } else { t }
}

/// 경로의 앞 n조각 ("/a/b/c", 2 → "/a/b")
fn leading(path: &str, n: usize) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).take(n).collect();
    format!("/{}", parts.join("/"))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

/// `--index-bench [--threads N] [--no-skip] [--max-entries N] [--time-limit 초] 폴더…`: 그 폴더들을 훑어 숫자만 찍는다.
/// 파일·폴더 이름은 찍지 않는다. --no-skip은 패키지(.app 등) 안까지 훑는다
pub fn bench(args: &[String]) {
    let (mut threads, mut packages_are_files, mut max_entries, mut limit) = (3usize, true, usize::MAX, 120u64);
    let mut roots = Vec::new();
    let mut rest = args.iter().skip_while(|a| *a != "--index-bench").skip(1);
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--threads" => threads = rest.next().and_then(|v| v.parse().ok()).unwrap_or(threads),
            "--no-skip" => packages_are_files = false,
            "--max-entries" => max_entries = rest.next().and_then(|v| v.parse().ok()).unwrap_or(max_entries),
            "--time-limit" => limit = rest.next().and_then(|v| v.parse().ok()).unwrap_or(limit),
            path => roots.push(std::fs::canonicalize(path).map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned())),
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let started = Instant::now();
    let search = Search::start(Options { roots: roots.clone(), home, protected: Vec::new(), threads, packages_are_files, max_entries, watch: false }, || {});
    let finished = search.settle(Duration::from_secs(limit));
    let took = started.elapsed();
    let st = search.status();
    let inside = if packages_are_files { "packages as files" } else { "inside packages" };
    let end = if finished && st.ready { "finished" } else { "STOPPED at the time limit" };
    println!("index bench: {} | {threads} threads | {inside} | {end}", roots.join(" "));
    let capped = if st.capped { " · capped" } else { "" };
    println!("  entries {} · folders read {} · denied {} · not local {} · errors {}{capped}", st.entries, st.dirs_listed, st.denied.len(), st.not_local, st.errors);
    let crawl = st.crawl_time.unwrap_or(took).as_secs_f64();
    println!("  crawl {crawl:.2} s · {:.0} entries/s · index {} MB · process peak {} MB", st.entries as f64 / crawl.max(0.001), st.memory >> 20, peak_memory() >> 20);
    if roots.iter().any(|r| r == "/") {
        println!("  entries under /System/Volumes/Data (should be 0): {}", search.search("/system/volumes/data/", 1).total);
    }
    for q in ["a", "report", "plist", "info.plist", "contents/", "보고", "zzz_nothing"] {
        let (mut best, mut total, mut top) = (Duration::MAX, 0, 0);
        for _ in 0..7 {
            let found = search.search(q, 200);
            best = best.min(found.took);
            (total, top) = (found.total, found.hits.len());
        }
        println!("  query {q:>12}: {total:>9} matches, top {top:>3} · best of 7 {:.2} ms", best.as_secs_f64() * 1000.0);
    }
    change_latency();
    // 권한 창의 답을 기다리는 줄이 있어도 끝낸다
    std::process::exit(0);
}

/// 새 파일이 검색에 보이기까지의 시간 (임시 폴더에서, 변경 기록을 따로 재촉하지 않고)
fn change_latency() {
    let dir = std::env::temp_dir().join(format!("sakadu-index-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let Ok(dir) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::canonicalize(&dir)) else { return };
    let opts = Options {
        roots: vec![dir.to_string_lossy().into_owned()],
        home: String::new(),
        protected: Vec::new(),
        threads: 2,
        packages_are_files: true,
        max_entries: usize::MAX,
        watch: true,
    };
    let search = Search::start(opts, || {});
    search.settle(Duration::from_secs(10));
    let started = Instant::now();
    let _ = std::fs::write(dir.join("sakadu-latency-probe.txt"), b"x");
    let mut seen = None;
    while started.elapsed() < Duration::from_secs(10) {
        if search.search("sakadu-latency-probe", 1).total == 1 {
            seen = Some(started.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let changes = search.status().changes;
    match seen {
        Some(d) => println!("  a new file shows up in search after {} ms ({changes} change notices)", d.as_millis()),
        None => println!("  a new file did not show up within 10 s ({changes} change notices)"),
    }
    drop(search);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 이 프로세스가 쓴 가장 많은 메모리(바이트). 맥의 ru_maxrss는 바이트다
fn peak_memory() -> usize {
    // SAFETY: rusage는 숫자만 든 구조체라 0으로 채워도 된다. 이 프로세스의 값을 읽기만 한다
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    usage.ru_maxrss.max(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use sakadu::nametree::{Item, Kind};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    /// 넉넉히 기다리는 시간
    const SOON: Duration = Duration::from_secs(20);
    /// 주간보고 (맥이 저장하는 나뉜 꼴)
    const WEEKLY_NFD: &str = "\u{110C}\u{116E}\u{1100}\u{1161}\u{11AB}\u{1107}\u{1169}\u{1100}\u{1169}";

    fn temp(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sakadu-search-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::canonicalize(&root).unwrap()
    }

    fn text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn start(root: &Path, protected: Vec<String>, watch: bool) -> Search {
        let opts = Options { roots: vec![text(root)], home: text(root), protected, threads: 2, packages_are_files: true, max_entries: usize::MAX, watch };
        Search::start(opts, || {})
    }

    fn total(s: &Search, q: &str) -> usize {
        s.search(q, 50).total
    }

    /// 바뀐 것이 색인에 보일 때까지 기다린다 (맥은 변경 알림을 조금 늦게 줄 수 있다). 걸린 시간을 돌려준다
    fn eventually(s: &Search, what: &str, ok: impl Fn(&Search) -> bool) -> Duration {
        let started = Instant::now();
        while started.elapsed() < SOON {
            assert!(s.settle(SOON), "{what}: the index did not settle");
            if ok(s) {
                return started.elapsed();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("{what}: not seen within {SOON:?}");
    }

    /// 대소문자를 가리는 디스크에서는 대소문자만 다른 두 파일을 둘 다 찾는다 (CI가 그런 디스크에서도 돌린다. 보통 디스크에서는 한 파일이다)
    #[test]
    fn search_keeps_names_that_differ_only_in_case() {
        let root = temp("case");
        std::fs::write(root.join("Report.txt"), b"x").unwrap();
        std::fs::write(root.join("report.txt"), b"x").unwrap();
        let on_disk = std::fs::read_dir(&root).unwrap().count();
        let s = start(&root, Vec::new(), false);
        assert!(s.settle(SOON));
        let found = s.search("report", 10);
        assert_eq!(found.hits.len(), on_disk, "{:?}", found.hits);
        assert_eq!(found.total, on_disk);
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// '지금 폴더'의 이름에 '|'가 들어도 '또는'이 되지 않는다 (그 폴더 아래의 다른 파일이 걸려 나오지 않는다)
    #[test]
    fn search_scope_with_a_bar_in_its_name_stays_one_condition() {
        let root = temp("bar");
        let dir = root.join("가|나");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("보고서.txt"), b"x").unwrap();
        std::fs::write(dir.join("사진.jpg"), b"x").unwrap();
        std::fs::write(root.join("보고서 밖.txt"), b"x").unwrap();
        let s = start(&root, Vec::new(), false);
        assert!(s.settle(SOON));
        let here = s.finder().find("보고서", Some(&text(&dir)), 50);
        assert_eq!(here.hits, [(false, format!("{}/보고서.txt", text(&dir)))]);
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// '지금 폴더'는 대소문자를 가리는 디스크에서 Work와 work를 다른 폴더로 본다.
    /// 가리지 않는 디스크(보통)에서는 다르게 친 경로도 그 폴더라 그 아래를 찾는다
    #[test]
    fn search_scope_follows_the_case_of_the_disk() {
        let root = temp("scope-case");
        std::fs::create_dir_all(root.join("Work")).unwrap();
        std::fs::write(root.join("Work/메모 큰.txt"), b"x").unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();
        std::fs::write(root.join("work/메모 작은.txt"), b"x").unwrap();
        let sensitive = std::fs::read_dir(&root).unwrap().count() == 2;
        let s = start(&root, Vec::new(), false);
        assert!(s.settle(SOON));
        let mut here: Vec<String> = s.finder().find("메모", Some(&text(&root.join("work"))), 50).hits.into_iter().map(|h| h.1).collect();
        here.sort();
        if sensitive {
            assert_eq!(here, [format!("{}/work/메모 작은.txt", text(&root))]);
        } else {
            assert_eq!(here, [format!("{}/Work/메모 작은.txt", text(&root)), format!("{}/Work/메모 큰.txt", text(&root))]);
        }
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn search_crawls_a_tree_like_finder() {
        let root = temp("tree");
        let docs = root.join("문서");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join(format!("{WEEKLY_NFD}.xlsx")), b"x").unwrap();
        std::fs::write(docs.join("회의록.md"), b"x").unwrap();
        std::fs::create_dir_all(root.join("Safari.app/Contents/MacOS")).unwrap();
        std::fs::write(root.join("Safari.app/Contents/MacOS/Safari"), b"x").unwrap();
        std::os::unix::fs::symlink(&root, root.join("링크")).unwrap();
        std::fs::create_dir_all(root.join("잠김")).unwrap();
        std::fs::write(root.join("잠김/비밀.txt"), b"x").unwrap();
        std::fs::set_permissions(root.join("잠김"), std::fs::Permissions::from_mode(0o000)).unwrap();
        std::fs::create_dir_all(root.join("Library")).unwrap();
        std::fs::write(root.join("Library/캐시.txt"), b"x").unwrap();
        std::fs::create_dir_all(root.join("깊은/a/b/c/d")).unwrap();
        std::fs::write(root.join("깊은/a/b/c/d/끝.txt"), b"x").unwrap();

        let s = start(&root, Vec::new(), false);
        assert!(s.settle(SOON));
        let st = s.status();
        assert!(st.ready && st.crawl_time.is_some());
        // 나뉜 꼴로 저장된 한글 이름을 합친 꼴로 찾는다
        let found = s.search("보고", 10);
        assert_eq!(found.total, 1);
        assert!(found.hits[0].1.ends_with("/문서/주간보고.xlsx"), "{}", found.hits[0].1);
        // 맥 디스크는 합친 꼴의 경로로도 그 파일을 연다: 색인의 경로로 파일을 열 수 있다
        assert!(std::fs::metadata(&found.hits[0].1).is_ok(), "the composed path opens the decomposed name");
        // 패키지 안은 훑지 않는다
        assert_eq!(total(&s, "safari"), 1);
        // 링크는 따라가지 않는다 (자기 위 폴더를 가리켜도 훑기가 끝난다)
        let link = s.search("링크", 5);
        assert_eq!(link.total, 1);
        assert!(!link.hits[0].0);
        // 권한이 없는 폴더는 읽지 못했다고 알린다
        assert_eq!(total(&s, "비밀"), 0);
        assert!(st.denied.iter().any(|d| d.ends_with("/잠김")), "{:?}", st.denied);
        // 홈의 라이브러리 안은 훑지 않는다 (라이브러리 자체는 찾는다)
        assert_eq!(total(&s, "캐시"), 0);
        assert_eq!(total(&s, "library"), 1);
        assert_eq!(total(&s, "끝"), 1);
        assert_eq!(total(&s, "보고 문서/"), 1);
        // 또는(|), 지금 폴더로 좁히기 (그 폴더를 나뉜 꼴로 줘도), 범위 밖 폴더
        assert_eq!(total(&s, "회의록 | 끝 | 회의록"), 2);
        let f = s.finder();
        let docs_nfd = text(&root.join("\u{1106}\u{116E}\u{11AB}\u{1109}\u{1165}"));
        let here = f.find("md | xlsx | txt", Some(&docs_nfd), 50);
        assert_eq!(here.total, 2, "{:?}", here.hits);
        assert!(here.hits.iter().all(|(_, p)| p.contains("/문서/")));
        assert!(f.covers(&text(&docs)) && f.covers(&text(&root.join("문서/새 폴더"))));
        assert!(!f.covers(&text(&root.join("Library"))) && !f.covers(&text(&root.join("Safari.app/Contents"))) && !f.covers("/private"));
        drop(s);
        std::fs::set_permissions(root.join("잠김"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn search_follows_changes_on_disk() {
        let root = temp("changes");
        let outside = temp("changes-outside");
        std::fs::write(root.join("처음.txt"), b"x").unwrap();
        let s = start(&root, Vec::new(), true);
        assert!(s.settle(SOON));
        assert_eq!(total(&s, "처음"), 1);

        std::fs::write(root.join("새 파일.txt"), b"x").unwrap();
        let took = eventually(&s, "a new file", |s| total(s, "새 파일") == 1);
        println!("a new file showed up after {} ms", took.as_millis());
        std::fs::rename(root.join("새 파일.txt"), root.join("바뀐 이름.txt")).unwrap();
        eventually(&s, "a renamed file", |s| total(s, "새 파일") == 0 && total(s, "바뀐 이름") == 1);
        // 밖에서 만든 폴더를 안으로 옮기면 그 안까지 훑는다
        std::fs::create_dir_all(outside.join("가져온 폴더/안쪽")).unwrap();
        std::fs::write(outside.join("가져온 폴더/안쪽/깊은 파일.txt"), b"x").unwrap();
        std::fs::rename(outside.join("가져온 폴더"), root.join("가져온 폴더")).unwrap();
        eventually(&s, "a folder moved in", |s| total(s, "깊은 파일") == 1);
        // 다시 밖으로 옮기면 그 아래까지 빠진다
        std::fs::rename(root.join("가져온 폴더"), outside.join("돌려보낸 폴더")).unwrap();
        eventually(&s, "a folder moved out", |s| total(s, "깊은 파일") == 0 && total(s, "가져온") == 0);
        // 폴더째 지운다
        std::fs::create_dir_all(root.join("지울 폴더/속")).unwrap();
        std::fs::write(root.join("지울 폴더/속/지울 파일.txt"), b"x").unwrap();
        eventually(&s, "a new nested folder", |s| total(s, "지울 파일") == 1);
        std::fs::remove_dir_all(root.join("지울 폴더")).unwrap();
        eventually(&s, "a removed folder", |s| total(s, "지울") == 0);
        // 한꺼번에 1000개
        std::fs::create_dir(root.join("묶음")).unwrap();
        for k in 0..1000 {
            std::fs::write(root.join(format!("묶음/묶음-{k:04}.txt")), b"x").unwrap();
        }
        eventually(&s, "a thousand new files", |s| total(s, "묶음-") == 1000);
        assert!(s.status().changes > 0);
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    /// 훑는 범위의 맨 위 폴더가 옮겨지면 그 안이 검색에서 빠지고, 돌아오면 다시 보인다
    #[test]
    fn search_forgets_a_root_that_moves_away() {
        let parent = temp("root-moves");
        let root = parent.join("범위");
        std::fs::create_dir_all(root.join("속")).unwrap();
        std::fs::write(root.join("속/떠날 파일.txt"), b"x").unwrap();
        let s = start(&root, Vec::new(), true);
        assert!(s.settle(SOON));
        assert_eq!(total(&s, "떠날"), 1);
        std::fs::rename(&root, parent.join("옮긴 범위")).unwrap();
        eventually(&s, "a root moved away", |s| total(s, "떠날") == 0);
        // 돌아온 뒤 그 안이 바뀌면 다시 훑는다
        std::fs::rename(parent.join("옮긴 범위"), &root).unwrap();
        std::fs::write(root.join("돌아옴.txt"), b"x").unwrap();
        eventually(&s, "a root moved back", |s| total(s, "떠날") == 1 && total(s, "돌아옴") == 1);
        drop(s);
        std::fs::remove_dir_all(parent).unwrap();
    }

    /// 한 폴더는 한 번에 한 줄만 읽는다: 읽는 중에 그 폴더의 알림이 또 오면, 그 결과가 온 뒤에 한 번 더 읽는다
    #[test]
    fn search_reads_a_folder_again_only_after_the_last_read_ends() {
        let mut tree = NameTree::new();
        let root = "/sakadu-search-test/범위";
        let slot = tree.add_root(root);
        let shared = Arc::new(Shared { tree: RwLock::new(tree), status: Mutex::new(Status::default()), work: Mutex::new(Work::default()), work_ready: Condvar::new() });
        let mut owner = Owner::new(shared.clone(), Vec::new(), usize::MAX, Box::new(|| {}));
        owner.ready = true;
        let job = |path: &str, deep| Job { slot, path: path.to_string(), deep };
        owner.send(vec![job(root, false)]);
        let first = lock(&shared.work).queue.pop_front().unwrap();
        // 읽는 사이에 알림이 두 번 더 온다 (하나는 그 아래까지). 다른 폴더는 바로 읽는다
        owner.send(vec![job(root, false)]);
        owner.send(vec![job(root, true), job("/sakadu-search-test/다른 곳", false)]);
        assert_eq!(owner.in_flight, 2);
        assert_eq!(lock(&shared.work).queue.iter().map(|j| j.path.as_str()).collect::<Vec<_>>(), ["/sakadu-search-test/다른 곳"]);
        lock(&shared.work).queue.clear();
        owner.listed(first, Listed::Items(Vec::new()));
        let queue = lock(&shared.work).queue.drain(..).collect::<Vec<_>>();
        assert_eq!((queue.len(), queue[0].path.as_str(), queue[0].deep), (1, root, true));
        assert_eq!(owner.in_flight, 2);
        // 다시 읽은 결과가 오면 더 읽지 않는다
        owner.listed(job(root, true), Listed::Items(Vec::new()));
        assert!(lock(&shared.work).queue.is_empty());
        assert_eq!(owner.in_flight, 1);
    }

    /// 읽는 사이에 그 폴더가 같은 이름의 파일로 바뀌었으면, 늦게 온 그 폴더의 목록은 버린다 (파일 아래에 없는 항목이 붙지 않게)
    #[test]
    fn search_drops_a_listing_of_a_folder_that_became_a_file() {
        let mut tree = NameTree::new();
        let root = "/sakadu-search-test/범위";
        let r = tree.add_root(root);
        let d = tree.sync_dir(r, &[Item { name: "폴더".into(), kind: Kind::Dir }])[0];
        let shared = Arc::new(Shared { tree: RwLock::new(tree), status: Mutex::new(Status::default()), work: Mutex::new(Work::default()), work_ready: Condvar::new() });
        let mut owner = Owner::new(shared.clone(), Vec::new(), usize::MAX, Box::new(|| {}));
        owner.ready = true;
        owner.send(vec![Job { slot: d, path: format!("{root}/폴더"), deep: false }]);
        let job = lock(&shared.work).queue.pop_front().unwrap();
        // 그 폴더를 읽는 사이에 다른 줄이 위 폴더를 다시 읽었더니 같은 이름이 파일이 되었다 (빈 칸을 다시 써 같은 칸일 수 있다)
        write(&shared.tree).sync_dir(r, &[Item { name: "폴더".into(), kind: Kind::File }]);
        owner.listed(job, Listed::Items(vec![Item { name: "유령.txt".into(), kind: Kind::File }]));
        let tree = read(&shared.tree);
        assert_eq!(tree.search(&Query::parse_posix("유령"), 10).1, 0);
        assert_eq!(tree.search(&Query::parse_posix("폴더"), 10).1, 1, "the file that replaced the folder stays");
        assert!(lock(&shared.work).queue.is_empty());
    }

    #[test]
    fn search_reads_protected_folders_last_but_reads_them() {
        let root = temp("protected");
        std::fs::create_dir_all(root.join("문서")).unwrap();
        std::fs::write(root.join("문서/보호된 파일.txt"), b"x").unwrap();
        std::fs::write(root.join("보통 파일.txt"), b"x").unwrap();
        let s = start(&root, vec![text(&root.join("문서"))], false);
        assert!(s.settle(SOON));
        assert_eq!((total(&s, "보호된"), total(&s, "보통")), (1, 1));
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 홈의 라이브러리 안에 있는 iCloud Drive는 따로 고르면 그 안만 훑는다 (라이브러리의 다른 곳은 훑지 않는다)
    #[test]
    fn search_reads_icloud_drive_inside_the_closed_library() {
        let home = temp("icloud");
        let drive = icloud_drive(&text(&home));
        std::fs::create_dir_all(&drive).unwrap();
        std::fs::write(Path::new(&drive).join("클라우드 메모.txt"), b"x").unwrap();
        std::fs::create_dir_all(home.join("Library/Caches")).unwrap();
        std::fs::write(home.join("Library/Caches/캐시.txt"), b"x").unwrap();
        let opts = Options { roots: vec![text(&home), drive.clone()], home: text(&home), protected: vec![drive.clone()], threads: 2, packages_are_files: true, max_entries: usize::MAX, watch: true };
        let s = Search::start(opts, || {});
        assert!(s.settle(SOON));
        assert_eq!((total(&s, "클라우드"), total(&s, "캐시")), (1, 0));
        std::fs::write(Path::new(&drive).join("새 클라우드 파일.txt"), b"x").unwrap();
        eventually(&s, "a new file in iCloud Drive", |s| total(s, "새 클라우드") == 1);
        drop(s);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn search_setting_is_off_unless_turned_on() {
        assert_eq!(Setting::parse(""), Setting::default());
        assert_eq!(Setting::parse("other\non=1"), Setting::default());
        let s = Setting { on: true, icloud: true };
        assert_eq!(Setting::parse(&s.text()), s);
        assert_eq!(Setting::parse("sakadu-search 1\non=1\n"), Setting { on: true, icloud: false });
        let (roots, protected) = default_scope("/Users/hong", false);
        assert_eq!(roots, ["/Users/hong", "/Applications", "/System/Applications"]);
        assert_eq!(protected, ["/Users/hong/Desktop", "/Users/hong/Documents", "/Users/hong/Downloads"]);
        let (roots, protected) = default_scope("/Users/hong", true);
        assert_eq!(roots.last().map(String::as_str), Some("/Users/hong/Library/Mobile Documents/com~apple~CloudDocs"));
        assert_eq!(protected.len(), 4);
    }

    #[test]
    fn search_threads_do_not_download_icloud_files() {
        let policy = std::thread::spawn(|| {
            crawl::prepare_thread();
            crawl::dataless_policy()
        })
        .join()
        .unwrap();
        assert_eq!(policy, crawl::IOPOL_MATERIALIZE_DATALESS_FILES_OFF);
    }

    #[test]
    fn search_does_not_cross_into_other_disks() {
        let mounts = crawl::mount_points();
        assert!(mounts.contains("/System/Volumes/Data"), "{mounts:?}");
        assert!(!mounts.contains("/"));
        let root = temp("mounts");
        std::fs::create_dir_all(root.join("붙은 디스크/안")).unwrap();
        let rules = Rules { home: String::new(), packages_are_files: true, mounts: [text(&root.join("붙은 디스크"))].into() };
        let Listed::Items(items) = crawl::list(&text(&root), &rules) else { panic!("the folder could not be read") };
        assert_eq!(items, [Item { name: "붙은 디스크".to_string(), kind: Kind::Closed }]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
