//! 사카두 맥 시제품. 지금은 창 없이 화면 모양 후보를 PNG로 그린다:
//! `sakadu-mac --render <데모 폴더> --out <출력 폴더>` → layout1-light.png, layout1-dark.png, layout2-light.png, layout2-dark.png.
//! 창과 마우스·키 입력은 다음 단계(2-2)에서 붙인다

#[cfg(target_os = "macos")]
mod gfx;
#[cfg(target_os = "macos")]
mod scene;

#[cfg(target_os = "macos")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let (Some(demo), Some(out)) = (arg("--render"), arg("--out")) else {
        eprintln!("사카두 맥 시제품: 아직 창이 없습니다 (다음 단계).");
        eprintln!("화면 그림: sakadu-mac --render <데모 폴더> --out <출력 폴더>");
        std::process::exit(2);
    };
    let demo = std::fs::canonicalize(&demo).unwrap_or_else(|e| panic!("데모 폴더 {demo}: {e}"));
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("출력 폴더");
    for shape in [scene::Shape::Current, scene::Shape::MacSidebar] {
        for dark in [false, true] {
            let name = format!("layout{}-{}.png", shape as u8 + 1, if dark { "dark" } else { "light" });
            let path = out.join(&name);
            scene::render(&demo, shape, dark, &path).unwrap_or_else(|e| panic!("{name}: {e}"));
            println!("{}", path.display());
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("sakadu-mac은 macOS에서만 돈다");
}
