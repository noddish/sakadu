//! Sakadu 공통 코드: 운영 체제와 상관없는 칸·탭 상태, 저장 형식(세션·작업 공간·즐겨찾기·단축키), 이름 검색, 표시용 포맷.
//! 실행 파일(main.rs)이 Windows 화면과 셸 연동을 맡고, 여기 모듈은 `crate::이름`으로 그대로 쓴다.
//! Windows API가 필요한 부분은 `#[cfg(windows)]`로 묶어, 다른 OS(macOS 버전 준비)에서도 빌드·테스트되게 한다.

pub mod diag;
pub mod favorites;
pub mod index;
pub mod keys;
pub mod listing;
pub mod natural;
pub mod pane;
pub mod session;
pub mod util;
pub mod workspaces;
