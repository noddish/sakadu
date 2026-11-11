//! 진단 로그. `--log` 실행 옵션이나 SAKADU_LOG 환경 변수가 있을 때만 %TEMP%\sakadu.log(맥은 $TMPDIR/sakadu.log)에 기록한다 (문제 재현용)

use std::io::Write;
use std::sync::OnceLock;
use std::time::Instant;

fn start() -> Option<Instant> {
    static START: OnceLock<Option<Instant>> = OnceLock::new();
    *START.get_or_init(|| {
        let on = std::env::var_os("SAKADU_LOG").is_some() || std::env::args().skip(1).any(|a| a.eq_ignore_ascii_case("--log"));
        on.then(Instant::now)
    })
}

pub fn enabled() -> bool {
    start().is_some()
}

/// 꺼져 있으면 메시지를 만들지도 않는다
pub fn log(msg: impl FnOnce() -> String) {
    let Some(t0) = start() else { return };
    // 맥에는 TEMP가 없어 그때만 시스템 임시 폴더($TMPDIR)를 쓴다 (Windows는 지금처럼 TEMP)
    let dir = std::env::var_os("TEMP").map(std::path::PathBuf::from).or_else(|| cfg!(not(windows)).then(std::env::temp_dir));
    let Some(dir) = dir else { return };
    let path = dir.join("sakadu.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{:>9.3}] {}", t0.elapsed().as_secs_f64(), msg());
    }
}
