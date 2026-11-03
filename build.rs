// 실행 파일에 매니페스트(공용 컨트롤 v6, Per-Monitor V2 DPI, 긴 경로)를 링커로 직접 포함한다.
// 리소스 컴파일러나 추가 크레이트 없이 MSVC 링커 옵션만 사용한다.
fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("msvc") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let manifest = std::path::Path::new(&dir).join("app.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
    }
    println!("cargo:rerun-if-changed=app.manifest");
}
