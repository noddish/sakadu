//! 압축 파일(zip·7z·tar.gz 등)을 폴더처럼 읽기 (맥). 맥에 들어 있는 libarchive(/usr/lib/libarchive.2.dylib, bsdtar가 쓰는 것)를
//! 처음 쓸 때 연다. 실행 파일에 링크하지 않아, 열지 못하거나 함수가 없으면 압축 파일은 그냥 기본 앱으로 열린다
//! (Windows에서 셸이 읽지 못하는 형식처럼). 이름·경로 규칙과 폴더 목록 엮기는 Windows와 함께 쓰는 sakadu::archive_path.
//! 안은 읽기만 한다. 안의 파일을 열거나 복사·끌어 내려면 임시 폴더($TMPDIR/Sakadu/archive-…)에 꺼낸 것을 쓴다.
//! 꺼낼 때 압축 파일 밖을 가리키는 이름(절대 경로, "..")은 버리고, 링크는 만들지 않는다 (링크를 따라 밖에 쓰지 않게).
//! 인터넷에서 받은 압축 파일의 격리 표시(com.apple.quarantine)는 꺼낸 것에도 붙인다 (Finder의 압축 풀기처럼).
//! 읽기와 꺼내기는 뒷줄에서 한다 (폴더 읽기 요청, 파일 작업). 메인 줄에서는 들어갈 수 있는지(readable)만 본다

use sakadu::archive_path::{self, Item};
use sakadu::listing::{Entry, filetime_of};
use sakadu::{hangul, util};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

const LIB_PATH: &CStr = c"/usr/lib/libarchive.2.dylib";

/// libarchive 3의 돌려주는 값 (archive.h)
const ARCHIVE_EOF: c_int = 1;
const ARCHIVE_OK: c_int = 0;
const ARCHIVE_WARN: c_int = -20;

/// 항목의 종류 (archive_entry.h의 AE_IF*)
const AE_IFMT: u32 = 0o170000;
const AE_IFREG: u32 = 0o100000;
const AE_IFDIR: u32 = 0o040000;

/// 읽지 못한 압축 파일 (Windows와 같은 말, 'PC' 대신 'Mac')
pub const UNREADABLE: &str = "압축 파일을 읽지 못했어요 · 손상됐거나 이 Mac이 읽지 못하는 형식이에요";
/// 압축 파일 안에 없는 폴더를 열려고 했다 (경로를 쳤거나, 그 사이 압축 파일이 바뀌었다)
const NO_FOLDER: &str = "압축 파일 안에 그 폴더가 없어요";

/// 격리 표시 (인터넷에서 받은 파일에 맥이 붙인다)
const QUARANTINE: &CStr = c"com.apple.quarantine";

/// struct archive*, struct archive_entry* (안을 보지 않는 포인터)
type Ar = *mut c_void;
type En = *mut c_void;

/// 쓰는 libarchive 함수들 (C 선언 그대로. mode_t는 맥에서 16비트, la_int64_t·time_t는 64비트)
struct Lib {
    read_new: unsafe extern "C" fn() -> Ar,
    filter_all: unsafe extern "C" fn(Ar) -> c_int,
    format_all: unsafe extern "C" fn(Ar) -> c_int,
    open_filename: unsafe extern "C" fn(Ar, *const c_char, usize) -> c_int,
    next_header: unsafe extern "C" fn(Ar, *mut En) -> c_int,
    pathname: unsafe extern "C" fn(En) -> *const c_char,
    /// 없을 수도 있다 (옛 libarchive)
    pathname_utf8: Option<unsafe extern "C" fn(En) -> *const c_char>,
    filetype: unsafe extern "C" fn(En) -> libc::mode_t,
    perm: Option<unsafe extern "C" fn(En) -> libc::mode_t>,
    size: unsafe extern "C" fn(En) -> i64,
    mtime: unsafe extern "C" fn(En) -> libc::time_t,
    hardlink: Option<unsafe extern "C" fn(En) -> *const c_char>,
    read_data: unsafe extern "C" fn(Ar, *mut c_void, usize) -> isize,
    data_skip: unsafe extern "C" fn(Ar) -> c_int,
    read_free: unsafe extern "C" fn(Ar) -> c_int,
    error_string: unsafe extern "C" fn(Ar) -> *const c_char,
}

/// libarchive (처음 쓸 때 한 번 연다. 뒷줄 여럿이 함께 불러도 한 번만). 열지 못했으면 None
fn lib() -> Option<&'static Lib> {
    static LOADED: OnceLock<Option<Lib>> = OnceLock::new();
    LOADED
        .get_or_init(|| {
            // SAFETY: NUL로 끝나는 고정 경로다. 받은 핸들은 닫지 않는다 (앱이 끝날 때까지 쓴다)
            let h = unsafe { libc::dlopen(LIB_PATH.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
            if h.is_null() {
                sakadu::diag::log(|| "archive: libarchive did not load".to_string());
                return None;
            }
            // SAFETY: 아래의 형은 libarchive 3의 그 함수들의 C 선언과 같다
            let got = unsafe { resolve(h) };
            if got.is_none() {
                sakadu::diag::log(|| "archive: a libarchive function is missing".to_string());
            }
            got
        })
        .as_ref()
}

/// SAFETY: h는 libarchive의 핸들이어야 한다 (기호마다 Lib의 그 칸의 형으로 읽는다)
unsafe fn resolve(h: *mut c_void) -> Option<Lib> {
    // SAFETY: 칸마다 그 함수의 C 선언과 같은 형이다
    unsafe {
        Some(Lib {
            read_new: sym(h, c"archive_read_new")?,
            filter_all: sym(h, c"archive_read_support_filter_all")?,
            format_all: sym(h, c"archive_read_support_format_all")?,
            open_filename: sym(h, c"archive_read_open_filename")?,
            next_header: sym(h, c"archive_read_next_header")?,
            pathname: sym(h, c"archive_entry_pathname")?,
            pathname_utf8: sym(h, c"archive_entry_pathname_utf8"),
            filetype: sym(h, c"archive_entry_filetype")?,
            perm: sym(h, c"archive_entry_perm"),
            size: sym(h, c"archive_entry_size")?,
            mtime: sym(h, c"archive_entry_mtime")?,
            hardlink: sym(h, c"archive_entry_hardlink"),
            read_data: sym(h, c"archive_read_data")?,
            data_skip: sym(h, c"archive_read_data_skip")?,
            read_free: sym(h, c"archive_read_free")?,
            error_string: sym(h, c"archive_error_string")?,
        })
    }
}

/// 기호 하나를 함수 포인터로. 없으면 None
/// SAFETY: T는 그 기호의 함수 포인터 형이어야 한다
unsafe fn sym<T: Copy>(h: *mut c_void, name: &CStr) -> Option<T> {
    debug_assert_eq!(size_of::<T>(), size_of::<*mut c_void>());
    // SAFETY: NUL로 끝나는 이름이다
    let p = unsafe { libc::dlsym(h, name.as_ptr()) };
    // SAFETY: 맥에서 함수 포인터와 데이터 포인터는 크기가 같다. T가 맞는 형인지는 부르는 쪽이 보장한다
    (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) })
}

/// 이 줄(스레드)의 글자 집합을 UTF-8로 두는 동안. libarchive는 항목 이름을 지금 로케일의 글자로 바꾸는데, 앱의 로케일은 "C"라
/// 한글 같은 이름을 바꾸지 못하고 버린다 (이름이 비어 온다). 이 값이 사라질 때 원래대로 되돌린다
struct Utf8Ctype(libc::locale_t);

impl Utf8Ctype {
    fn set() -> Option<Utf8Ctype> {
        static LOCALE: OnceLock<usize> = OnceLock::new();
        let loc = *LOCALE.get_or_init(|| {
            for name in [c"UTF-8", c"en_US.UTF-8"] {
                // SAFETY: NUL로 끝나는 이름. 만든 로케일은 놓지 않는다 (앱이 끝날 때까지 여러 줄이 쓴다)
                let l = unsafe { libc::newlocale(libc::LC_CTYPE_MASK, name.as_ptr(), std::ptr::null_mut()) };
                if !l.is_null() {
                    return l as usize;
                }
            }
            sakadu::diag::log(|| "archive: no UTF-8 locale".to_string());
            0
        });
        // SAFETY: 위에서 만든 로케일을 이 줄에만 쓴다
        (loc != 0).then(|| Utf8Ctype(unsafe { libc::uselocale(loc as libc::locale_t) }))
    }
}

impl Drop for Utf8Ctype {
    fn drop(&mut self) {
        // SAFETY: uselocale이 돌려준 원래 로케일로 되돌린다
        unsafe { libc::uselocale(self.0) };
    }
}

/// 항목 머리 (이름은 libarchive가 준 그대로. 다듬기 전)
struct Header {
    name: Option<String>,
    kind: u32,
    perm: u32,
    size: u64,
    mtime: i64,
    hardlink: bool,
}

impl Header {
    /// 수정한 때 (FILETIME 단위, 모르면 0)
    fn modified(&self) -> u64 {
        if self.mtime > 0 { filetime_of(std::time::UNIX_EPOCH + std::time::Duration::from_secs(self.mtime as u64)) } else { 0 }
    }
}

/// NUL로 끝나는 글을 복사한다 (UTF-8이 아닌 바이트는 바꿔 읽는다)
/// SAFETY: p는 NULL이거나 NUL로 끝나는 글이어야 한다
unsafe fn text(p: *const c_char) -> Option<String> {
    // SAFETY: 부르는 쪽이 보장한다
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

/// 압축 파일 하나를 처음부터 차례로 읽는 것 (libarchive의 읽기 개체). 한 줄에서만 쓴다
struct Reader {
    lib: &'static Lib,
    a: Ar,
    /// 놓을 때(read_free)까지 UTF-8로 둔다. 필드는 drop 뒤에 사라지므로 놓은 다음에 되돌려진다
    _ctype: Option<Utf8Ctype>,
}

impl Reader {
    fn open(path: &str) -> Result<Reader, String> {
        let lib = lib().ok_or("libarchive를 열지 못했어요")?;
        let c = CString::new(path).map_err(|_| "경로에 쓸 수 없는 글자가 있어요".to_string())?;
        let ctype = Utf8Ctype::set();
        // SAFETY: 읽기 개체를 만들어 모든 압축·묶음 형식을 켜고 파일을 연다. 열지 못하면 그 개체를 바로 놓는다
        unsafe {
            let a = (lib.read_new)();
            if a.is_null() {
                return Err("libarchive가 읽기를 시작하지 못했어요".to_string());
            }
            (lib.filter_all)(a);
            (lib.format_all)(a);
            if (lib.open_filename)(a, c.as_ptr(), 64 * 1024) < ARCHIVE_WARN {
                let why = text((lib.error_string)(a)).unwrap_or_default();
                (lib.read_free)(a);
                return Err(why);
            }
            Ok(Reader { lib, a, _ctype: ctype })
        }
    }

    /// 다음 항목의 머리 (끝이면 None)
    fn next(&mut self) -> Result<Option<Header>, String> {
        let lib = self.lib;
        let mut e: En = std::ptr::null_mut();
        // SAFETY: 연 읽기 개체. 받은 항목은 다음 next_header까지 쓸 수 있고, 그 안에서 다 복사해 둔다
        unsafe {
            let r = (lib.next_header)(self.a, &mut e);
            if r == ARCHIVE_EOF {
                return Ok(None);
            }
            if (r != ARCHIVE_OK && r != ARCHIVE_WARN) || e.is_null() {
                return Err(self.error());
            }
            let name = lib.pathname_utf8.and_then(|f| text(f(e))).or_else(|| text((lib.pathname)(e)));
            Ok(Some(Header {
                name,
                kind: u32::from((lib.filetype)(e)) & AE_IFMT,
                perm: lib.perm.map_or(0, |f| u32::from(f(e))),
                size: (lib.size)(e).max(0) as u64,
                mtime: (lib.mtime)(e),
                hardlink: lib.hardlink.is_some_and(|f| !f(e).is_null()),
            }))
        }
    }

    /// 지금 항목의 내용을 out에 쓴다
    fn copy_data(&mut self, out: &mut impl std::io::Write) -> Result<(), String> {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            // SAFETY: buf의 길이까지만 쓴다
            let n = unsafe { (self.lib.read_data)(self.a, buf.as_mut_ptr().cast(), buf.len()) };
            if n == 0 {
                return Ok(());
            }
            if n < 0 {
                return Err(self.error());
            }
            out.write_all(&buf[..n as usize]).map_err(|e| sakadu::fileops::reason(&e))?;
        }
    }

    /// 지금 항목의 내용을 건너뛴다
    fn skip(&mut self) {
        // SAFETY: 연 읽기 개체
        unsafe { (self.lib.data_skip)(self.a) };
    }

    fn error(&self) -> String {
        // SAFETY: 연 읽기 개체의 마지막 오류 글 (없으면 NULL)
        unsafe { text((self.lib.error_string)(self.a)) }.unwrap_or_else(|| "알 수 없는 오류".to_string())
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // SAFETY: open에서 만든 개체를 한 번만 놓는다
        unsafe { (self.lib.read_free)(self.a) };
    }
}

/// 압축 파일을 읽을 수 있는지: libarchive가 그 형식을 알아보고 첫 항목을 읽는다 (빈 압축 파일도 읽은 것이다).
/// 압축 파일을 열 때 메인 줄에서 부른다: 파일의 앞부분(zip은 끝의 목차)만 읽는다
pub fn readable(path: &str) -> bool {
    Reader::open(path).and_then(|mut r| r.next()).is_ok()
}

/// 열 때 폴더처럼 들어갈 압축 파일인지 (Windows archive::enterable과 같은 규칙): 압축 파일 이름의 파일이고,
/// 다른 압축 파일 안에 있지 않고, libarchive가 읽는다. 압축 파일 안의 압축 파일은 꺼내서 기본 앱으로 연다
pub fn enterable(path: &str) -> bool {
    archive_path::is_archive_name(util::file_name(path)) && std::fs::metadata(path).is_ok_and(|m| m.is_file()) && !archive_path::inside(path) && readable(path)
}

/// 압축 파일 안의 모든 항목 (적힌 차례대로). 밖을 가리키는 이름과 이름을 읽지 못한 항목은 뺀다
fn items(archive: &str) -> Result<Vec<Item>, String> {
    let mut r = Reader::open(archive)?;
    let mut out = Vec::new();
    while let Some(h) = r.next()? {
        let Some(path) = h.name.as_deref().and_then(archive_path::entry_path) else { continue };
        let dir = h.kind == AE_IFDIR;
        out.push(Item { path, dir, size: if dir { 0 } else { h.size }, modified: h.modified() });
        r.skip();
    }
    Ok(out)
}

/// 압축 파일 archive 안의 한 폴더 path의 목록 (path가 archive면 맨 위). 크기와 수정한 날짜는 압축 파일에 적힌 대로 (모르면 0).
/// 읽지 못하면 Windows처럼 '읽지 못했어요'. 맨 위가 비었는데 그 형식의 표식도 없으면(0으로 채운 zip 따위) 읽지 못한 것이다
pub fn list(path: &str, archive: &str) -> Result<Vec<Entry>, String> {
    let inner = archive_path::inner_path(path, archive).ok_or(NO_FOLDER)?;
    let all = items(archive).map_err(|e| {
        sakadu::diag::log(|| format!("archive: read failed: {e}"));
        UNREADABLE.to_string()
    })?;
    let out = archive_path::children(&all, &inner);
    if out.is_empty() {
        if inner.is_empty() && archive_path::marks_missing(archive) {
            return Err(UNREADABLE.to_string());
        }
        if !inner.is_empty() && archive_path::kind_of(&all, &inner) != Some(true) {
            return Err(NO_FOLDER.to_string());
        }
    }
    Ok(out)
}

/// 압축 파일 안의 경로가 폴더인지 (없으면 None. 압축 파일을 읽지 못해도 None). 경로 입력에서 뒷줄로 부른다
pub fn is_folder(path: &str) -> Option<bool> {
    let archive = archive_path::archive_of(path)?;
    let inner = archive_path::inner_path(path, &archive)?;
    archive_path::kind_of(&items(&archive).ok()?, &inner)
}

/// 꺼낸 것을 두는 곳: $TMPDIR/Sakadu (맥이 오래 쓰지 않은 것을 치운다). 꺼낼 때마다 그 안에 새 폴더를 만든다
pub fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("Sakadu")
}

/// 꺼낼 새 폴더: cache_dir 안의 archive-<프로세스>-<번호>. 남아 있는 것과 겹치지 않는 이름으로 새로 만든다
fn fresh_dir() -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let base = cache_dir();
    std::fs::create_dir_all(&base).map_err(|e| sakadu::fileops::reason(&e))?;
    loop {
        let dir = base.join(format!("archive-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(sakadu::fileops::reason(&e)),
        }
    }
}

/// 압축 파일 archive 안의 항목들(전체 경로, 같은 압축 파일 안)을 새 임시 폴더에 꺼낸다. 폴더는 속까지.
/// 꺼낸 것들의 전체 경로를 항목 차례대로 돌려준다. 하나라도 찾지 못하거나 쓰지 못하면 Err (알림에 쓸 말).
/// 파일 작업 줄에서 부른다 (큰 파일은 오래 걸린다)
pub fn extract(archive: &str, paths: &[String]) -> Result<Vec<String>, String> {
    // 꺼낼 것: (안쪽 경로, 꺼낸 이름). 이름은 목록에 보이는 꼴(한글을 모아 쓴 꼴)로. 다 맞는지 본 뒤에 폴더를 만든다
    let mut wants = Vec::new();
    for p in paths {
        let inner = archive_path::inner_path(p, archive).and_then(|i| archive_path::entry_path(&i));
        let Some(inner) = inner else { return Err(format!("압축 파일 안의 항목이 아니에요: {}", util::file_name(p))) };
        let name = hangul::compose(inner.rsplit('/').next().unwrap_or(&inner)).into_owned();
        wants.push((inner, name));
    }
    let dest = fresh_dir()?;
    extract_into(archive, &wants, &dest)
}

fn extract_into(archive: &str, wants: &[(String, String)], dest: &Path) -> Result<Vec<String>, String> {
    let quarantine = quarantine_of(archive);
    let mut made = vec![false; wants.len()];
    let mut written: Vec<PathBuf> = Vec::new();
    let mut places = Places::default();
    let unreadable = |e: String| {
        sakadu::diag::log(|| format!("archive: open failed: {e}"));
        UNREADABLE.to_string()
    };
    // 목록처럼 폴더가 이긴다: 같은 경로에 파일과 폴더(폴더 항목이나 그 안의 항목)가 함께 적혀 있으면 목록에는 폴더만 보이므로
    // 그 파일은 꺼내지 않는다 (파일이 먼저 적혀 있어도 그 자리를 폴더에 남긴다). 어느 경로가 폴더인지는 먼저 한 번 훑어 안다
    let dirs = folder_paths(archive).map_err(unreadable)?;
    let mut r = Reader::open(archive).map_err(unreadable)?;
    while let Some(h) = r.next()? {
        // 밖을 가리키는 이름은 꺼내지 않는다. 남은 조각에는 ".."도 빈 조각도 없어 dest 밖으로 나가지 않는다
        let Some(path) = h.name.as_deref().and_then(archive_path::entry_path) else { continue };
        if h.kind != AE_IFDIR && dirs.contains(&*hangul::compose(&path)) {
            r.skip();
            continue;
        }
        let Some((i, rest)) = wants.iter().enumerate().find_map(|(i, (inner, _))| archive_path::rest_under(&path, inner).map(|rest| (i, rest))) else {
            r.skip();
            continue;
        };
        // 자리는 만드는 것만 받는다 (만들지 않는 링크가 이름을 차지하지 않게)
        let mut place = |dir: bool| dest.join(places.place(i, &wants[i].1, &rest, dir));
        match h.kind {
            AE_IFDIR => {
                create_dirs(&place(true), &mut written)?;
                made[i] = true;
            }
            // 보통 파일 (종류를 적지 않는 형식은 0). 하드 링크 항목은 내용이 없다 (가리키는 항목의 내용을 쓴다): 만들지 않는다
            0 | AE_IFREG if !h.hardlink => {
                let out = place(false);
                if let Some(parent) = out.parent() {
                    create_dirs(parent, &mut written)?;
                }
                write_file(&mut r, &out, &h).map_err(|why| format!("{} · {why}", wants[i].1))?;
                written.push(out);
                made[i] = true;
            }
            // 링크·장치 따위는 만들지 않는다 (링크를 따라 밖에 쓰지 않게)
            _ => r.skip(),
        }
    }
    if let Some(i) = made.iter().position(|m| !m) {
        return Err(format!("압축 파일에서 꺼낼 것을 찾지 못했어요: {}", wants[i].1));
    }
    if let Some(q) = &quarantine {
        for p in &written {
            set_quarantine(p, q);
        }
    }
    Ok(wants.iter().enumerate().map(|(i, (_, name))| dest.join(places.top(i, name)).to_string_lossy().into_owned()).collect())
}

/// 압축 파일 안에서 폴더인 경로들 (한글은 모아 쓴 꼴): 폴더 항목과 모든 항목의 위 폴더들. 목록(archive_path::children)이
/// 폴더로 보이는 것과 같다
fn folder_paths(archive: &str) -> Result<std::collections::HashSet<String>, String> {
    let mut out = std::collections::HashSet::new();
    for it in items(archive)? {
        let path = hangul::compose(&it.path).into_owned();
        // 폴더 항목은 그 자신부터, 아니면 위 폴더부터
        let mut p = if it.dir { path.as_str() } else { path.rfind('/').map_or("", |i| &path[..i]) };
        // 이미 넣은 경로면 그 위 폴더들도 이미 들어 있다
        while !p.is_empty() && out.insert(p.to_string()) {
            p = p.rfind('/').map_or("", |i| &p[..i]);
        }
    }
    Ok(out)
}

/// 꺼낸 폴더 안의 자리 정하기. 압축 파일 안의 경로(한글은 모아 쓴 꼴, 목록에 보이는 그대로)마다 자리를 하나씩 준다.
/// 맥 디스크는 보통 대소문자를 가리지 않아 README와 readme가 한 파일이 된다: 목록에는 둘 다 보이니, 뒤에 나온 것에
/// 복사본처럼 번호를 붙여("readme 2") 둘 다 꺼낸다 (꺼낸 것을 옮겨 놓을 디스크도 그럴 수 있어 늘 대소문자를 접어 본다).
/// 같은 경로가 다시 나오면(tar에 덧붙인 새 판) 같은 자리에 덮어쓴다: 목록에도 하나로 보인다
#[derive(Default)]
struct Places {
    /// (꺼낼 것의 번호, 그 꺼낸 이름부터의 안쪽 경로) → 꺼낸 폴더 안의 상대 경로
    at: std::collections::HashMap<(usize, String), PathBuf>,
    /// 이미 준 자리 (상대 경로를 소문자로 접은 것)
    taken: std::collections::HashSet<String>,
}

impl Places {
    /// 꺼낼 것 i(꺼낸 이름 top) 아래 rest("" 이면 그 자신)의 자리. 위 폴더들도 차례로 자리를 받는다. dir: 그 항목이 폴더인지
    fn place(&mut self, i: usize, top: &str, rest: &str, dir: bool) -> PathBuf {
        let parts: Vec<&str> = std::iter::once(top).chain(rest.split('/').filter(|s| !s.is_empty())).collect();
        let mut key = String::new();
        let mut placed = PathBuf::new();
        for (k, part) in parts.iter().enumerate() {
            if k > 0 {
                key.push('/');
            }
            key.push_str(part);
            placed = match self.at.get(&(i, key.clone())) {
                Some(p) => p.clone(),
                None => {
                    let p = self.free(&placed, part, dir || k + 1 < parts.len());
                    self.at.insert((i, key.clone()), p.clone());
                    p
                }
            };
        }
        placed
    }

    /// 꺼낼 것 i의 꺼낸 자리 (그 이름이 다른 꺼낼 것과 겹쳐 번호를 붙였으면 그 이름)
    fn top(&self, i: usize, name: &str) -> PathBuf {
        self.at.get(&(i, name.to_string())).cloned().unwrap_or_else(|| PathBuf::from(name))
    }

    /// parent 안에서 아직 주지 않은 이름: name, 대소문자만 달라도 겹치면 "name 2", "name 3"… (파일은 확장자 앞에)
    fn free(&mut self, parent: &Path, name: &str, dir: bool) -> PathBuf {
        let mut n = 1;
        loop {
            let p = parent.join(sakadu::fileops::numbered_name(name, n, dir));
            if self.taken.insert(p.to_string_lossy().to_lowercase()) {
                return p;
            }
            n += 1;
        }
    }
}

/// 폴더를 (위 폴더까지) 만들고, 새로 만든 폴더들을 적어 둔다 (격리 표시를 붙일 것. 꺼내는 폴더 dest는 이미 있다)
fn create_dirs(dir: &Path, written: &mut Vec<PathBuf>) -> Result<(), String> {
    let missing: Vec<PathBuf> = dir.ancestors().take_while(|d| !d.is_dir()).map(Path::to_path_buf).collect();
    if missing.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| sakadu::fileops::reason(&e))?;
    written.extend(missing);
    Ok(())
}

/// 지금 항목의 내용을 새 파일 out에 쓴다. 링크를 따라가지 않고, 수정한 때와 실행 권한은 압축 파일에 적힌 대로
fn write_file(r: &mut Reader, out: &Path, h: &Header) -> Result<(), String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let reason = |e: std::io::Error| sakadu::fileops::reason(&e);
    let mut f = std::fs::File::options().write(true).create(true).truncate(true).custom_flags(libc::O_NOFOLLOW).open(out).map_err(reason)?;
    r.copy_data(&mut f)?;
    if h.mtime > 0 {
        let _ = f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(h.mtime as u64));
    }
    if h.perm & 0o111 != 0 {
        let _ = f.set_permissions(std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// 압축 파일의 격리 표시 (없으면 None)
fn quarantine_of(archive: &str) -> Option<Vec<u8>> {
    let c = CString::new(archive).ok()?;
    let mut buf = vec![0u8; 1024];
    // SAFETY: NUL로 끝나는 경로와 이름. buf의 길이까지만 쓴다
    let n = unsafe { libc::getxattr(c.as_ptr(), QUARANTINE.as_ptr(), buf.as_mut_ptr().cast(), buf.len(), 0, 0) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(buf)
}

/// 꺼낸 것에 격리 표시를 붙인다 (붙이지 못해도 그대로 둔다). 링크는 따라가지 않는다
fn set_quarantine(path: &Path, value: &[u8]) {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return };
    // SAFETY: NUL로 끝나는 경로와 이름, value의 길이만큼 읽는다
    unsafe { libc::setxattr(c.as_ptr(), QUARANTINE.as_ptr(), value.as_ptr().cast(), value.len(), 0, libc::XATTR_NOFOLLOW) };
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 시험 폴더 (임시 폴더 안. 실제 경로로)
    pub(crate) fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("sakadu-mac-archive-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        util::normalize(&std::fs::canonicalize(&dir).unwrap().to_string_lossy())
    }

    /// dir 안에 폴더 "자료"를 만들고 ditto로 "자료.zip"에 묶는다 (Finder의 압축하기와 같은 도구. 자료 폴더째 든다):
    /// 자료/메모.txt, 자료/사진/바다.bmp, 자료/빈 폴더/. 만든 zip의 경로
    pub(crate) fn make_zip(dir: &str) -> String {
        let src = Path::new(dir).join("원본").join("자료");
        std::fs::create_dir_all(src.join("사진")).unwrap();
        std::fs::create_dir_all(src.join("빈 폴더")).unwrap();
        std::fs::write(src.join("메모.txt"), "압축 안의 메모\n").unwrap();
        std::fs::write(src.join("사진").join("바다.bmp"), [7u8; 300]).unwrap();
        let zip = util::join(dir, "자료.zip");
        ditto(&src, &zip);
        zip
    }

    /// 폴더 src를 그 폴더째 zip으로 묶는다 (Finder의 압축하기와 같은 도구. 확장 속성·리소스 포크는 넣지 않는다)
    pub(crate) fn ditto(src: &Path, zip: &str) {
        let ok = std::process::Command::new("/usr/bin/ditto").args(["-c", "-k", "--norsrc", "--keepParent"]).arg(src).arg(zip).status().unwrap().success();
        assert!(ok, "ditto made {zip}");
    }

    /// tar 머리 하나 (ustar, 512바이트): 이름, 종류('0' 파일, '2' 링크, '5' 폴더), 크기, 링크 대상
    fn tar_header(name: &str, kind: u8, size: usize, link: &str) -> [u8; 512] {
        let mut h = [0u8; 512];
        let put = |h: &mut [u8; 512], at: usize, s: &[u8]| h[at..at + s.len()].copy_from_slice(s);
        put(&mut h, 0, name.as_bytes());
        put(&mut h, 100, b"0000644\0");
        put(&mut h, 108, b"0000000\0");
        put(&mut h, 116, b"0000000\0");
        put(&mut h, 124, format!("{size:011o}\0").as_bytes());
        put(&mut h, 136, b"15100000000\0");
        h[156] = kind;
        put(&mut h, 157, link.as_bytes());
        put(&mut h, 257, b"ustar\0");
        put(&mut h, 263, b"00");
        // 검사 합: 그 칸을 빈칸으로 놓고 모든 바이트를 더한다
        put(&mut h, 148, b"        ");
        let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
        put(&mut h, 148, format!("{sum:06o}\0 ").as_bytes());
        h
    }

    /// 손으로 만든 tar: 밖을 가리키는 이름("../밖.txt", 절대 경로), 시험 폴더 밖을 가리키는 링크와 그 링크 아래의 파일, 보통 파일.
    /// outside는 tar 머리의 이름 칸(100바이트)에 들어가게 짧아야 한다
    fn make_evil_tar(dir: &str, outside: &str) -> String {
        let mut tar = Vec::new();
        let file = |tar: &mut Vec<u8>, name: &str, body: &[u8]| {
            tar.extend(tar_header(name, b'0', body.len(), ""));
            tar.extend(body);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        };
        file(&mut tar, "../밖.txt", b"evil");
        file(&mut tar, &format!("{outside}/절대.txt"), b"evil");
        tar.extend(tar_header("링크", b'2', 0, outside));
        file(&mut tar, "링크/링크 안.txt", b"evil");
        file(&mut tar, "괜찮음.txt", b"fine");
        tar.resize(tar.len() + 1024, 0);
        let path = util::join(dir, "수상한.tar");
        std::fs::write(&path, tar).unwrap();
        path
    }

    /// 숨김이 아닌 항목의 (이름, 폴더인지), 이름 차례로 (맥 도구가 붙일 수 있는 "._" 파일과 __MACOSX는 뺀다)
    fn names(entries: &[Entry]) -> Vec<(String, bool)> {
        let mut v: Vec<(String, bool)> = entries.iter().filter(|e| !e.is_hidden()).map(|e| (e.name_string(), e.is_dir())).collect();
        v.sort();
        v
    }

    #[test]
    fn lists_a_zip_made_by_ditto() {
        let dir = temp("list");
        let zip = make_zip(&dir);
        assert!(readable(&zip) && enterable(&zip));
        assert_eq!(names(&list(&zip, &zip).unwrap()), [("자료".to_string(), true)]);
        let top = util::join(&zip, "자료");
        let inside = list(&top, &zip).unwrap();
        let want = [("메모.txt", false), ("빈 폴더", true), ("사진", true)].map(|(n, d)| (n.to_string(), d));
        assert_eq!(names(&inside), want);
        let memo = inside.iter().find(|e| e.name_string() == "메모.txt").unwrap();
        assert_eq!(memo.size, "압축 안의 메모\n".len() as u64);
        assert!(memo.modified > 0, "the zip keeps the time");
        assert_eq!(names(&list(&util::join(&top, "사진"), &zip).unwrap()), [("바다.bmp".to_string(), false)]);
        assert!(list(&util::join(&top, "빈 폴더"), &zip).unwrap().is_empty());
        assert_eq!(list(&util::join(&top, "없음"), &zip).unwrap_err(), NO_FOLDER);
        assert_eq!(is_folder(&util::join(&top, "사진")), Some(true));
        assert_eq!(is_folder(&util::join(&top, "메모.txt")), Some(false));
        assert_eq!(is_folder(&util::join(&top, "없음")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 폴더 항목 없이 파일만 적힌 tar.gz: 위 폴더들이 목록에 폴더로 생긴다
    #[test]
    fn folders_appear_from_the_paths_inside() {
        let dir = temp("implicit");
        let src = Path::new(&dir).join("원본");
        std::fs::create_dir_all(src.join("가").join("나")).unwrap();
        std::fs::write(src.join("가").join("나").join("다.txt"), b"x").unwrap();
        let tgz = util::join(&dir, "묶음.tar.gz");
        // 확장 속성을 "._" 파일로 넣지 않게 (맥의 tar)
        let ok = std::process::Command::new("/usr/bin/tar")
            .env("COPYFILE_DISABLE", "1")
            .arg("-czf")
            .arg(&tgz)
            .arg("-C")
            .arg(&src)
            .arg("가/나/다.txt")
            .status()
            .unwrap()
            .success();
        assert!(ok, "tar made the archive");
        assert_eq!(names(&list(&tgz, &tgz).unwrap()), [("가".to_string(), true)]);
        assert_eq!(names(&list(&util::join(&tgz, "가"), &tgz).unwrap()), [("나".to_string(), true)]);
        assert_eq!(names(&list(&util::join(&util::join(&tgz, "가"), "나"), &tgz).unwrap()), [("다.txt".to_string(), false)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 아무 바이트나 든 zip은 읽지 못한다 (열면 기본 앱으로). 0으로 채운 zip은 빈 묶음처럼 읽히지만 표식이 없어 '읽지 못했어요'
    #[test]
    fn broken_zips_are_unreadable() {
        let dir = temp("broken");
        let junk = util::join(&dir, "가짜.zip");
        std::fs::write(&junk, (0..4096u32).map(|i| (i * 7919 % 251) as u8 + 1).collect::<Vec<_>>()).unwrap();
        assert!(!readable(&junk) && !enterable(&junk));
        assert!(list(&junk, &junk).unwrap_err().contains("읽지 못했어요"));
        let zeros = util::join(&dir, "영.zip");
        std::fs::write(&zeros, vec![0u8; 8192]).unwrap();
        assert_eq!(list(&zeros, &zeros).unwrap_err(), UNREADABLE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_files_and_folders_to_a_fresh_temp_folder() {
        let dir = temp("extract");
        let zip = make_zip(&dir);
        let top = util::join(&zip, "자료");
        let got = extract(&zip, &[util::join(&top, "메모.txt"), util::join(&top, "사진")]).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|p| Path::new(p).starts_with(cache_dir())), "{got:?}");
        assert!(got[0].ends_with("/메모.txt") && got[1].ends_with("/사진"), "{got:?}");
        assert_eq!(std::fs::read_to_string(&got[0]).unwrap(), "압축 안의 메모\n");
        assert_eq!(std::fs::read(Path::new(&got[1]).join("바다.bmp")).unwrap(), [7u8; 300]);
        // 꺼낼 때마다 새 폴더
        let again = extract(&zip, &[util::join(&top, "메모.txt")]).unwrap();
        assert_ne!(Path::new(&again[0]).parent(), Path::new(&got[0]).parent());
        assert!(extract(&zip, &[util::join(&top, "없음.txt")]).unwrap_err().contains("찾지 못했어요"));
        for p in got.iter().chain(&again) {
            let _ = std::fs::remove_dir_all(Path::new(p).parent().unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 밖을 가리키는 이름과 링크는 목록에 없고 꺼내지도 않는다: 링크 아래의 파일은 링크를 따라가지 않고 꺼낸 폴더 안의 보통 폴더에 쓴다
    #[test]
    fn names_that_escape_are_refused() {
        let dir = temp("evil");
        // 밖: tar 머리에 들어가게 짧은 경로 (시험이 만들고 지운다)
        let outside = format!("/tmp/sakadu-o-{}", std::process::id());
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        let tar = make_evil_tar(&dir, &outside);
        let top = names(&list(&tar, &tar).unwrap());
        assert!(top.contains(&("괜찮음.txt".to_string(), false)), "{top:?}");
        assert!(!top.iter().any(|(n, _)| n.contains("밖") || n.contains("절대")), "{top:?}");
        let got = extract(&tar, &[util::join(&tar, "괜찮음.txt"), util::join(&tar, "링크")]).unwrap();
        assert_eq!(std::fs::read(&got[0]).unwrap(), b"fine");
        let link = std::fs::symlink_metadata(&got[1]).unwrap();
        assert!(link.is_dir() && !link.file_type().is_symlink(), "the link is not made");
        assert!(Path::new(&got[1]).join("링크 안.txt").is_file());
        assert!(extract(&tar, &[util::join(&tar, "../밖.txt")]).is_err());
        // 시험 폴더 밖(위 폴더, 링크가 가리키던 곳)에는 아무것도 생기지 않았다
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        let parent = Path::new(&got[0]).parent().unwrap();
        assert!(!parent.parent().unwrap().join("밖.txt").exists());
        let _ = std::fs::remove_dir_all(parent);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// 같은 경로에 파일과 폴더가 함께 적혀 있으면(파일을 폴더로 바꾼 뒤 덧붙인 tar) 목록처럼 폴더가 이긴다: 파일이 먼저 적혀 있어도,
    /// 폴더 항목이 먼저 적혀 있어도 그 이름은 폴더가 받고, 목록에 보이지 않는 그 파일은 꺼내지 않는다
    #[test]
    fn a_folder_wins_over_a_file_with_the_same_path() {
        let dir = temp("folder-wins");
        let mut tar = Vec::new();
        let mut add = |name: &str, kind: u8, body: &[u8]| {
            tar.extend(tar_header(name, kind, body.len(), ""));
            tar.extend(body);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        };
        // foo: 파일이 먼저, 그다음 그 안의 파일. baz: 폴더 항목과 그 안의 파일이 먼저, 그다음 같은 이름의 파일
        add("폴더/foo", b'0', b"file first");
        add("폴더/foo/bar.txt", b'0', b"bar");
        add("폴더/baz/", b'5', b"");
        add("폴더/baz/qux.txt", b'0', b"qux");
        add("폴더/baz", b'0', b"file last");
        tar.resize(tar.len() + 1024, 0);
        let path = util::join(&dir, "섞임.tar");
        std::fs::write(&path, tar).unwrap();
        let top = util::join(&path, "폴더");
        assert_eq!(names(&list(&top, &path).unwrap()), [("baz".to_string(), true), ("foo".to_string(), true)]);
        let got = extract(&path, std::slice::from_ref(&top)).unwrap();
        let out = Path::new(&got[0]);
        assert_eq!(std::fs::read(out.join("foo").join("bar.txt")).unwrap(), b"bar");
        assert_eq!(std::fs::read(out.join("baz").join("qux.txt")).unwrap(), b"qux");
        let mut inside: Vec<String> = std::fs::read_dir(out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        inside.sort();
        assert_eq!(inside, ["baz", "foo"], "the shadowed files are not extracted");
        // 그 폴더 하나만 골라도 폴더다
        let one = extract(&path, &[util::join(&top, "foo"), util::join(&top, "baz")]).unwrap();
        assert!(Path::new(&one[0]).is_dir() && Path::new(&one[1]).is_dir(), "{one:?}");
        assert_eq!(std::fs::read(Path::new(&one[0]).join("bar.txt")).unwrap(), b"bar");
        for p in got.iter().chain(&one) {
            let _ = std::fs::remove_dir_all(Path::new(p).parent().unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 대소문자만 다른 항목(README와 readme, Sub와 sub)은 맥 디스크에서 한 자리가 되므로 뒤에 나온 것에 번호를 붙여 모두 꺼낸다
    /// (목록에도 모두 보인다). 고른 것끼리 겹쳐도 그렇다. 같은 경로가 다시 나오면(tar에 덧붙인 새 판) 뒤의 것으로 덮어쓴다
    #[test]
    fn names_that_differ_only_in_case_are_all_extracted() {
        let dir = temp("case");
        let entries: [(&str, &[u8]); 6] =
            [("폴더/README", b"upper"), ("폴더/readme", b"lower"), ("폴더/메모.txt", b"old"), ("폴더/메모.txt", b"new"), ("폴더/Sub/a.txt", b"a"), ("폴더/sub/b.txt", b"b")];
        let mut tar = Vec::new();
        for (name, body) in entries {
            tar.extend(tar_header(name, b'0', body.len(), ""));
            tar.extend(body);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        }
        tar.resize(tar.len() + 1024, 0);
        let path = util::join(&dir, "겹침.tar");
        std::fs::write(&path, tar).unwrap();
        let top = util::join(&path, "폴더");
        let shown = names(&list(&top, &path).unwrap());
        assert_eq!(shown.len(), 5, "README, readme, Sub, sub, 메모.txt: {shown:?}");
        let got = extract(&path, std::slice::from_ref(&top)).unwrap();
        let out = Path::new(&got[0]);
        let read = |p: &str| std::fs::read(out.join(p)).unwrap();
        assert_eq!(read("README"), b"upper");
        assert_eq!(read("readme 2"), b"lower");
        assert_eq!(read("메모.txt"), b"new", "the later copy of the same path wins");
        assert_eq!(read("Sub/a.txt"), b"a");
        assert_eq!(read("sub 2/b.txt"), b"b");
        let two = extract(&path, &[util::join(&top, "README"), util::join(&top, "readme")]).unwrap();
        assert!(two[0].ends_with("/README") && two[1].ends_with("/readme 2"), "{two:?}");
        assert_eq!(std::fs::read(&two[0]).unwrap(), b"upper");
        assert_eq!(std::fs::read(&two[1]).unwrap(), b"lower");
        for p in got.iter().chain(&two) {
            let _ = std::fs::remove_dir_all(Path::new(p).parent().unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
