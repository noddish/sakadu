//! 창 멈춤 기록: 창이 1초 넘게 응답하지 않으면 %LOCALAPPDATA%\Sakadu\slow.txt에 한 줄 남긴다.
//! 언제, 몇 초, 무엇을 하던 중이었는지(단계 이름과 창 메시지 번호)만 적는다. 경로나 파일 이름은 적지 않는다.
//! 다른 스레드가 0.5초마다 창에 빈 메시지(WM_NULL)를 보내 답을 잰다. 메뉴·대화 상자처럼 메시지를 돌리며 기다리는 동안은
//! 답이 오므로 멈춤으로 치지 않는다

use crate::shell::SendHwnd;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SMTO_NORMAL, SendMessageTimeoutW, WM_NULL};

/// 이보다 오래 답이 없으면 적는다
const STALL_MS: u32 = 1000;
/// 기록 파일이 이보다 커지면 뒤쪽 절반만 남긴다
const MAX_BYTES: u64 = 64 * 1024;

static STAGE: Mutex<&'static str> = Mutex::new("");
static MSG: AtomicU32 = AtomicU32::new(0);

/// 창이 지금 처리하는 메시지 (창 프로시저 들머리에서)
pub fn message(msg: u32) {
    MSG.store(msg, Ordering::Relaxed);
}

/// 오래 걸릴 수 있는 일의 이름. 돌려받은 값이 사라지면 앞의 이름으로 돌아간다
pub struct Stage(&'static str);

pub fn stage(name: &'static str) -> Stage {
    Stage(set_stage(name))
}

impl Drop for Stage {
    fn drop(&mut self) {
        set_stage(self.0);
    }
}

fn set_stage(name: &'static str) -> &'static str {
    STAGE.lock().map(|mut s| std::mem::replace(&mut *s, name)).unwrap_or("")
}

fn answers(h: HWND, timeout_ms: u32) -> bool {
    unsafe { SendMessageTimeoutW(h, WM_NULL, WPARAM(0), LPARAM(0), SMTO_NORMAL, timeout_ms, None) }.0 != 0
}

/// 창을 지켜보기 시작한다 (창을 만든 뒤 한 번)
pub fn watch(hwnd: HWND) {
    let h = SendHwnd(hwnd);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            if !unsafe { IsWindow(Some(h.get())) }.as_bool() {
                return;
            }
            let t = Instant::now();
            if answers(h.get(), STALL_MS) {
                continue;
            }
            // 멈춰 있다: 지금 하던 일을 적어 두고 풀릴 때까지 잰다
            let stage = STAGE.lock().map(|s| *s).unwrap_or("");
            let msg = MSG.load(Ordering::Relaxed);
            let answered = answers(h.get(), 120_000);
            let secs = t.elapsed().as_secs_f32();
            if unsafe { IsWindow(Some(h.get())) }.as_bool() {
                record(secs, answered, stage, msg);
            }
        }
    });
}

fn record(secs: f32, answered: bool, stage: &str, msg: u32) {
    let st = unsafe { GetLocalTime() };
    let what = if stage.is_empty() { "-" } else { stage };
    let end = if answered { "" } else { " (아직 응답 없음)" };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}  {secs:.1}초{end}  {what}  메시지 0x{msg:04X}\r\n",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    );
    crate::diag::log(|| format!("stall: {}", line.trim_end()));
    let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Sakadu")) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("slow.txt");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES)
        && let Ok(old) = std::fs::read(&path)
    {
        let tail = &old[old.len() / 2..];
        let start = tail.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1);
        let _ = std::fs::write(&path, &tail[start..]);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}
