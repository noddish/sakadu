//! html 파일을 화면 그림으로: PC에 있는 Edge를 창 없이 띄워 페이지를 찍는다.
//! 자동 미리 보기 카드와 미리 보기 창만 쓴다 (목록 썸네일은 만들지 않는다: 폴더마다 Edge를 띄우지 않게).
//! 찍은 그림은 경로·수정 시각·크기로 캐시해, 다시 고르거나 크게 보기로 바꿔도 다시 찍지 않는다.
//! 글꼴·스타일·스크립트 같은 바깥 파일은 브라우저에서 연 것처럼 받아 온다(AI로 만든 페이지는 CDN을 많이 쓴다).
//! 인터넷에서 받은 파일(Mark of the Web)은 찍지 않고 코드로 둔다: 탐색기가 그런 파일의 미리 보기를 막는 것과 같은 까닭

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// 찍는 화면 크기. 데스크톱 쪽 모양으로 그려지면서도, 작은 카드·창에 줄여 보일 때 글자가 너무 작아지지 않게 1024 폭
pub const VIEW: (u32, u32) = (1024, 640);
/// 캐시에 남겨 둘 그림 수
const KEEP: usize = 40;
/// Edge가 이 안에 찍지 못하면 끝내고 코드로 보인다
const TIMEOUT: Duration = Duration::from_secs(10);

/// 화면으로 미리 볼 확장자
pub fn is_page_ext(ext: &str) -> bool {
    matches!(ext, "html" | "htm")
}

/// 이 PC의 Edge (없으면 html은 지금처럼 코드로 보인다)
fn edge() -> Option<&'static Path> {
    static EDGE: OnceLock<Option<PathBuf>> = OnceLock::new();
    EDGE.get_or_init(|| {
        let under = |var: &str| std::env::var_os(var).map(|d| PathBuf::from(d).join(r"Microsoft\Edge\Application\msedge.exe"));
        ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"].into_iter().filter_map(under).find(|p| p.is_file())
    })
    .as_deref()
}

/// html을 화면으로 미리 볼 수 있는지 (Edge가 있는지)
pub fn available() -> bool {
    edge().is_some()
}

/// 사카두가 Edge에 쓰는 자리: 따로 쓰는 프로필과 찍은 그림 캐시
fn home() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(r"Sakadu\page-preview"))
}

/// html 파일의 화면 그림(PNG) 경로. 캐시에 있으면 그것, 없으면 Edge로 찍는다. 찍지 못하면 None
pub fn shot(path: &str) -> Option<PathBuf> {
    if from_internet(path) {
        return None;
    }
    let md = std::fs::metadata(path).ok()?;
    let modified = md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let home = home()?;
    let cache = home.join("cache");
    let png = cache.join(format!("{:016x}.png", cache_key(path, modified, md.len())));
    if png.is_file() {
        // 최근에 본 것으로 표시해 오래된 것부터 지운다
        let _ = std::fs::File::options().write(true).open(&png).and_then(|f| f.set_modified(std::time::SystemTime::now()));
        return Some(png);
    }
    // 같은 프로필을 두 Edge가 함께 쓰지 못하니 한 번에 하나씩 찍는다
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    if png.is_file() {
        return Some(png);
    }
    std::fs::create_dir_all(&cache).ok()?;
    let tmp = cache.join(format!("찍는 중-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut cmd = std::process::Command::new(edge()?);
    cmd.args([
        "--headless=new",
        "--disable-gpu",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        "--disable-sync",
        "--disable-background-networking",
        "--disable-component-update",
        "--mute-audio",
        "--hide-scrollbars",
        "--disk-cache-size=52428800",
        // 스크립트가 그리는 페이지도 다 그려지게 (바깥 파일을 받는 동안은 기다린다)
        "--virtual-time-budget=2000",
    ]);
    cmd.arg(format!("--window-size={},{}", VIEW.0, VIEW.1));
    cmd.arg(format!("--user-data-dir={}", home.join("profile").display()));
    cmd.arg(format!("--screenshot={}", tmp.display()));
    // 경로를 그대로 넘기면 Edge가 file 주소로 바꾼다 (빈칸·#·한글도 알맞게)
    cmd.arg(path);
    let done = crate::thumbs::run_hidden(&mut cmd, TIMEOUT);
    if !done || std::fs::metadata(&tmp).map_or(true, |m| m.len() == 0) || std::fs::rename(&tmp, &png).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    prune(&cache);
    Some(png)
}

fn cache_key(path: &str, modified: u128, len: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // 찍는 화면 크기를 바꾸면 예전 그림을 쓰지 않게 함께 넣는다
    (path.to_lowercase(), modified, len, VIEW).hash(&mut h);
    h.finish()
}

/// 캐시는 최근 것 KEEP장만 남긴다
fn prune(cache: &Path) {
    let Ok(rd) = std::fs::read_dir(cache) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= KEEP {
        return;
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, p) in files.into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(p);
    }
}

/// 인터넷에서 받은 파일인지 (Zone.Identifier의 영역이 인터넷(3)·제한된 사이트(4))
fn from_internet(path: &str) -> bool {
    std::fs::read(format!("{path}:Zone.Identifier")).is_ok_and(|b| {
        String::from_utf8_lossy(&b).lines().filter_map(|l| l.trim().trim_matches('\0').strip_prefix("ZoneId=")).any(|z| z.trim().parse::<u32>().is_ok_and(|n| n >= 3))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sakadu-page-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 인터넷에서 받은 표시가 있으면 찍지 않는다 (Edge를 띄우기 전에 멈춘다)
    #[test]
    fn files_from_the_internet_are_not_rendered() {
        let d = temp_dir("motw");
        let f = d.join("받은 페이지.html");
        std::fs::write(&f, "<p>hi</p>").unwrap();
        let p = f.to_string_lossy().into_owned();
        assert!(!from_internet(&p));
        std::fs::write(format!("{p}:Zone.Identifier"), "[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
        assert!(from_internet(&p));
        assert!(shot(&p).is_none());
        std::fs::remove_dir_all(d).unwrap();
    }

    /// 실제 Edge로 찍는다 (Edge가 있는 PC에서 손으로: cargo test -- --ignored page): 요즘 CSS와 스크립트가 그린 것까지,
    /// 빈칸·한글이 든 경로도. 같은 파일은 캐시에서 바로 나온다
    #[test]
    #[ignore]
    fn renders_a_page_with_edge() {
        let d = temp_dir("edge");
        // 사용자의 사카두 캐시 대신 시험 폴더에 (손으로 따로 돌리는 시험이라 다른 시험과 겹치지 않는다)
        unsafe { std::env::set_var("LOCALAPPDATA", d.join("local")) };
        let f = d.join("제주 일정 #1.html");
        std::fs::write(&f, "<!doctype html><meta charset=utf-8><style>body{margin:0;display:grid;background:#3070e0}</style><body><script>document.body.style.background='#e03030'</script>").unwrap();
        let p = f.to_string_lossy().into_owned();
        let png = shot(&p).expect("edge screenshot");
        let start = std::time::Instant::now();
        assert_eq!(shot(&p).as_deref(), Some(png.as_path()), "cached");
        assert!(start.elapsed() < Duration::from_millis(200));
        let _com = crate::shell::ComInit::sta();
        let (w, h, px) = crate::thumbs::image_file(&png.to_string_lossy(), 320).expect("png");
        assert_eq!((w, h), (320, 200), "the {VIEW:?} view, shrunk");
        let mid = ((h / 2 * w + w / 2) * 4) as usize;
        assert!(px[mid + 2] > 180 && px[mid] < 90, "the script painted it red (BGRA): {:?}", &px[mid..mid + 4]);
        let _ = std::fs::remove_file(png);
        std::fs::remove_dir_all(d).unwrap();
    }
}
