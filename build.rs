// 실행 파일에 매니페스트(공용 컨트롤 v6, Per-Monitor V2 DPI, 긴 경로)와 아이콘을 링커로 직접 포함한다.
// 리소스 컴파일러나 추가 크레이트 없이 MSVC 링커 옵션만 사용한다. 아이콘은 .res 파일을 직접 써서 넘긴다.
use std::path::Path;

/// (리소스 번호, 파일). 번호는 src/tray.rs의 ICON_* 상수와 맞춘다
const ICONS: [(u16, &str); 3] = [(1, "assets/sakadu.ico"), (2, "assets/tray-black.ico"), (3, "assets/tray-white.ico")];

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("msvc") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let manifest = Path::new(&dir).join("app.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
        let res = Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons.res");
        std::fs::write(&res, icon_res(Path::new(&dir))).unwrap();
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
    println!("cargo:rerun-if-changed=app.manifest");
    for (_, file) in ICONS {
        println!("cargo:rerun-if-changed={file}");
    }
}

/// .ico 파일들을 아이콘(RT_ICON)과 아이콘 묶음(RT_GROUP_ICON) 리소스로 담은 .res 파일
fn icon_res(root: &Path) -> Vec<u8> {
    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;
    let mut out = Vec::new();
    // .res 파일은 빈 항목 하나로 시작한다
    res_entry(&mut out, 0, 0, 0, &[]);
    let mut image_id = 1u16;
    for (group_id, file) in ICONS {
        let ico = std::fs::read(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        let mut group = ico[..6].to_vec();
        for i in 0..count {
            let e = &ico[6 + i * 16..6 + (i + 1) * 16];
            let size = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
            let offset = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
            res_entry(&mut out, RT_ICON, image_id, 0x1010, &ico[offset..offset + size]);
            // 묶음 항목은 .ico 항목과 같고, 끝의 파일 위치(4바이트)만 아이콘 리소스 번호(2바이트)로 바뀐다
            group.extend_from_slice(&e[..12]);
            group.extend_from_slice(&image_id.to_le_bytes());
            image_id += 1;
        }
        res_entry(&mut out, RT_GROUP_ICON, group_id, 0x1030, &group);
    }
    out
}

/// 번호로 이름 붙인 리소스 하나: 32바이트 머리글 + 데이터, 4바이트 단위로 맞춘다. 언어는 중립(0)
fn res_entry(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(32u32.to_le_bytes());
    for v in [0xFFFF, kind, 0xFFFF, id] {
        out.extend(v.to_le_bytes());
    }
    out.extend(0u32.to_le_bytes()); // DataVersion
    out.extend(flags.to_le_bytes()); // MemoryFlags
    out.extend(0u16.to_le_bytes()); // LanguageId
    out.extend(0u32.to_le_bytes()); // Version
    out.extend(0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}
