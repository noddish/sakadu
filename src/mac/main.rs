//! 사카두 맥 시제품 (모양 2: 사이드바가 창 위까지, 신호등은 그 안).
//! 인자 없이 실행하면 창을 띄운다. CI 확인용으로 두 가지가 더 있다:
//! `--render <데모 폴더> --out <출력 폴더>`는 창 없이 여러 상태를 PNG로 그리고,
//! `--snapshot <PNG> --start <폴더>`는 실제 창을 열어 창 틀(신호등 포함)까지 그린 뒤 끝낸다

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod gfx;
#[cfg(target_os = "macos")]
mod render;
#[cfg(target_os = "macos")]
mod view;

#[cfg(target_os = "macos")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if let (Some(demo), Some(out)) = (arg("--render"), arg("--out")) {
        let demo = std::fs::canonicalize(&demo).unwrap_or_else(|e| panic!("데모 폴더 {demo}: {e}"));
        let out = std::path::PathBuf::from(out);
        std::fs::create_dir_all(&out).expect("출력 폴더");
        for path in render::render_all(&demo, &out).unwrap_or_else(|e| panic!("그림: {e}")) {
            println!("{}", path.display());
        }
        return;
    }
    if let Some(out) = arg("--snapshot") {
        let start = arg("--start").unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
        let start = std::fs::canonicalize(&start).map(|p| p.to_string_lossy().into_owned()).unwrap_or(start);
        view::snapshot(std::path::Path::new(&out), start).unwrap_or_else(|e| panic!("창 그림: {e}"));
        return;
    }
    view::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("sakadu-mac은 macOS에서만 돈다");
}
