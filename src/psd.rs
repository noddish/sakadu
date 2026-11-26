//! 포토샵 파일(PSD, 큰 문서 PSB)의 미리 보기 그림 읽기. 레이어는 건너뛰고 파일 끝의 합친 그림(composite)을 읽되,
//! 읽으면서 줄여 원래 크기의 그림은 만들지 않는다 (수백 MB짜리 파일도 있어 구역은 읽지 않고 건너뛴다).
//! 합친 그림을 못 읽으면(ZIP 압축, Lab·32비트 같은 색 방식, 잘린 파일) 이미지 리소스의 JPEG 썸네일을 준다.
//! 운영 체제 API 없이 Windows와 맥이 같이 쓴다. 망가진 파일에서도 패닉하지 않는다: 길이는 모두 파일 크기와 견주고,
//! 큰 수의 셈은 checked로 한다. 형식은 Adobe "Photoshop File Formats Specification"을 따랐고, 수는 모두 빅 엔디언이다

use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// 머리의 색 방식
const BITMAP: u16 = 0;
const GRAY: u16 = 1;
const INDEXED: u16 = 2;
const RGB: u16 = 3;
const CMYK: u16 = 4;
const DUOTONE: u16 = 8;
/// 한 변의 최대 길이 (PSB의 한도. PSD는 30000이지만 같은 한도로 너그럽게 본다)
const MAX_SIDE: u32 = 300_000;
/// 채널 수의 한도 (명세)
const MAX_CHANNELS: u16 = 56;
/// 줄인 그림의 화소 수 한도. max_side를 아주 크게 줘도 메모리를 끝없이 잡지 않게 (16384×16384)
const MAX_PIXELS: usize = 1 << 28;
/// 썸네일 리소스의 크기 한도 (보통 수십 KB)
const THUMB_MAX: u64 = 16 << 20;
/// 리소스 블록의 서명. 8BIM 말고도 다른 프로그램이 남긴 서명이 섞여 있을 수 있다
const SIGNATURES: [[u8; 4]; 5] = [*b"8BIM", *b"MeSa", *b"AgHg", *b"PHUT", *b"DCSR"];
/// 썸네일 리소스: 1036은 RGB, 옛 판(포토샵 4)의 1033은 빨강과 파랑이 바뀐 BGR
const THUMB: u16 = 1036;
const THUMB_BGR: u16 = 1033;

/// 포토샵 파일에서 얻은 미리 보기 그림
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    /// 합친 그림. 곱하지 않은(straight) RGBA 8비트, 위 줄부터
    Pixels { w: u32, h: u32, rgba: Vec<u8> },
    /// 이미지 리소스의 JPEG 썸네일. bgr이면 옛 리소스 1033의 것이라 빨강과 파랑을 바꿔 그려야 한다
    Jpeg { data: Vec<u8>, bgr: bool },
}

/// 파일에서 미리 보기 그림을 읽는다. 긴 변이 max_side 이하가 되게 읽으면서 줄인다
pub fn read(path: &Path, max_side: u32) -> Option<Picture> {
    read_from(std::fs::File::open(path).ok()?, max_side)
}

/// read와 같되 아무 읽기에서나 (시험이 쓴다)
pub fn read_from<R: Read + Seek>(r: R, max_side: u32) -> Option<Picture> {
    let mut src = Src::new(r)?;
    let hd = header(&mut src)?;
    // 색 방식 데이터: 인덱스 색이면 팔레트 (빨강 256개, 초록 256개, 파랑 256개)
    let color_len = u64::from(src.u32()?);
    let color_end = src.pos.checked_add(color_len)?;
    let mut palette = None;
    if hd.mode == INDEXED && color_len >= 768 {
        let mut p = vec![0u8; 768];
        src.fill(&mut p)?;
        palette = Some(p);
    }
    src.go(color_end)?;
    // 이미지 리소스에서는 썸네일만 찾아 둔다. 구역이 잘렸으면 찾은 데까지
    let res_len = u64::from(src.u32()?);
    let res_end = src.pos.checked_add(res_len)?;
    let res_in_file = res_end.min(src.len);
    let thumb = thumbnail(&mut src, res_in_file);
    let pixels = src.go(res_end).and_then(|()| merged(&mut src, &hd, palette.as_deref(), max_side));
    match pixels {
        // 한 가지 색뿐인 합친 그림은 "호환성 최대화" 없이 저장한 파일의 빈 그림일 수 있다: 썸네일이 있으면 그것이 낫다
        Some((w, h, rgba)) if thumb.is_none() || !uniform(&rgba) => Some(Picture::Pixels { w, h, rgba }),
        _ => thumb,
    }
}

struct Header {
    psb: bool,
    channels: u16,
    w: u32,
    h: u32,
    depth: u16,
    mode: u16,
}

/// 버퍼를 둔 읽기와 지금 자리. 앞으로 건너뛸 때 버퍼 안이면 버퍼를 버리지 않는다(seek_relative)
struct Src<R> {
    r: BufReader<R>,
    pos: u64,
    len: u64,
}

impl<R: Read + Seek> Src<R> {
    fn new(mut r: R) -> Option<Src<R>> {
        let len = r.seek(SeekFrom::End(0)).ok()?;
        r.seek(SeekFrom::Start(0)).ok()?;
        Some(Src { r: BufReader::with_capacity(64 << 10, r), pos: 0, len })
    }

    /// buf를 채운다. 파일 끝을 넘으면 읽지 않고 None
    fn fill(&mut self, buf: &mut [u8]) -> Option<()> {
        if buf.len() as u64 > self.len.saturating_sub(self.pos) {
            return None;
        }
        if self.r.read_exact(buf).is_err() {
            // 읽는 도중 끊겼다 (파일이 바뀌었거나 읽기 오류): 자리를 믿을 수 없으니 뒤의 읽기도 모두 실패하게 한다
            self.len = 0;
            return None;
        }
        self.pos += buf.len() as u64;
        Some(())
    }

    fn u8(&mut self) -> Option<u8> {
        let mut b = [0u8; 1];
        self.fill(&mut b)?;
        Some(b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        let mut b = [0u8; 2];
        self.fill(&mut b)?;
        Some(u16::from_be_bytes(b))
    }

    fn u32(&mut self) -> Option<u32> {
        let mut b = [0u8; 4];
        self.fill(&mut b)?;
        Some(u32::from_be_bytes(b))
    }

    fn u64(&mut self) -> Option<u64> {
        let mut b = [0u8; 8];
        self.fill(&mut b)?;
        Some(u64::from_be_bytes(b))
    }

    /// 파일 안의 자리 at으로 옮긴다. 파일 끝을 넘으면 None
    fn go(&mut self, at: u64) -> Option<()> {
        if at > self.len {
            return None;
        }
        let d = if at >= self.pos { i64::try_from(at - self.pos).ok()? } else { -i64::try_from(self.pos - at).ok()? };
        if self.r.seek_relative(d).is_err() {
            self.len = 0;
            return None;
        }
        self.pos = at;
        Some(())
    }

    fn skip(&mut self, n: u64) -> Option<()> {
        self.go(self.pos.checked_add(n)?)
    }
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// 26바이트 머리. 포토샵 파일이 아니거나 크기·채널 수가 한도 밖이면 None
fn header<R: Read + Seek>(src: &mut Src<R>) -> Option<Header> {
    let mut b = [0u8; 26];
    src.fill(&mut b)?;
    let psb = match (&b[..4], be16(&b, 4)) {
        (b"8BPS", 1) => false,
        (b"8BPS", 2) => true,
        _ => return None,
    };
    let hd = Header { psb, channels: be16(&b, 12), h: be32(&b, 14), w: be32(&b, 18), depth: be16(&b, 22), mode: be16(&b, 24) };
    let side = 1..=MAX_SIDE;
    (side.contains(&hd.w) && side.contains(&hd.h) && (1..=MAX_CHANNELS).contains(&hd.channels)).then_some(hd)
}

/// 이미지 리소스 구역(끝 end)에서 JPEG 썸네일. 1036을 1033보다 먼저 고른다. 블록이 망가졌으면 찾은 데까지
fn thumbnail<R: Read + Seek>(src: &mut Src<R>, end: u64) -> Option<Picture> {
    let mut old = None;
    while let Some((id, at, size)) = resource(src, end) {
        if (id == THUMB || (id == THUMB_BGR && old.is_none())) && let Some(data) = jpeg(src, size) {
            if id == THUMB {
                return Some(Picture::Jpeg { data, bgr: false });
            }
            old = Some(Picture::Jpeg { data, bgr: true });
        }
        // 데이터는 짝수 길이로 채워져 있다
        if src.go(at + size.next_multiple_of(2)).is_none() {
            break;
        }
    }
    old
}

/// 다음 리소스 블록의 (번호, 데이터 자리, 데이터 크기). 구역 끝이거나 블록이 구역 밖으로 나가면 None
fn resource<R: Read + Seek>(src: &mut Src<R>, end: u64) -> Option<(u16, u64, u64)> {
    if src.pos >= end {
        return None;
    }
    let mut sig = [0u8; 4];
    src.fill(&mut sig)?;
    if !SIGNATURES.contains(&sig) {
        return None;
    }
    let id = src.u16()?;
    // 파스칼 문자열 이름: 길이 바이트까지 짝수 길이로 채운다
    let name = u64::from(src.u8()?);
    src.skip((name + 1).next_multiple_of(2) - 1)?;
    let size = u64::from(src.u32()?);
    let at = src.pos;
    (at.checked_add(size)? <= end).then_some((id, at, size))
}

/// 썸네일 리소스 데이터(28바이트 머리 뒤에 JPEG). 형식이 JPEG(1)이 아니거나 JPEG 시작 표시가 없으면 None
fn jpeg<R: Read + Seek>(src: &mut Src<R>, size: u64) -> Option<Vec<u8>> {
    if !(28..=THUMB_MAX).contains(&size) {
        return None;
    }
    let mut head = [0u8; 28];
    src.fill(&mut head)?;
    if be32(&head, 0) != 1 {
        return None;
    }
    // 압축된 크기가 적혀 있으면 그만큼 (리소스 끝에 군더더기가 붙은 파일이 있다)
    let rest = size - 28;
    let n = match u64::from(be32(&head, 20)) {
        0 => rest,
        c => c.min(rest),
    };
    let mut data = vec![0u8; n as usize];
    src.fill(&mut data)?;
    data.starts_with(&[0xFF, 0xD8]).then_some(data)
}

/// 레이어 구역을 건너뛰고 합친 그림을 줄여 가며 읽어 (너비, 높이, RGBA). 못 읽는 방식이거나 잘렸으면 None
fn merged<R: Read + Seek>(src: &mut Src<R>, hd: &Header, palette: Option<&[u8]>, max_side: u32) -> Option<(u32, u32, Vec<u8>)> {
    let layers = if hd.psb { src.u64()? } else { u64::from(src.u32()?) };
    src.skip(layers)?;
    // 합친 그림에 쓰는 색 채널 수와, 그 뒤 채널을 투명도로 볼 수 있는 방식인지. 16비트는 위 바이트만 쓴다
    let (color, may_alpha) = match (hd.mode, hd.depth) {
        (BITMAP, 1) => (1, false),
        (GRAY | DUOTONE, 8 | 16) => (1, true),
        (INDEXED, 8) if palette.is_some() => (1, false),
        (RGB, 8 | 16) => (3, true),
        (CMYK, 8 | 16) => (4, false),
        _ => return None,
    };
    let channels = usize::from(hd.channels);
    if channels < color {
        return None;
    }
    // 색 채널 뒤에 채널이 더 있으면 그 첫째를 합친 그림의 투명도로 본다 (가정). 포토샵은 투명한 곳이 있을 때 그 자리에
    // 투명도를 쓰지만, 저장해 둔 선택 영역(알파 채널)도 같은 자리에 오고 둘을 가려 적는 곳은 레이어 구역 안이라 보지 않는다
    let alpha = may_alpha && channels > color;
    let used = color + usize::from(alpha);
    // 0 그대로, 1 RLE(PackBits). 2·3은 ZIP이라 읽지 않는다
    let compression = src.u16()?;
    if compression > 1 {
        return None;
    }
    let (w, h) = (hd.w as usize, hd.h as usize);
    // k줄·k칸마다 하나씩 남긴다 (가장 가까운 화소). 긴 변이 max_side 이하가 된다
    let k = w.max(h).div_ceil(max_side.max(1) as usize);
    let (ow, oh) = (w.div_ceil(k), h.div_ceil(k));
    let pixels = ow.checked_mul(oh).filter(|n| *n <= MAX_PIXELS)?;
    let row_bytes = match hd.depth {
        1 => w.div_ceil(8),
        16 => w * 2,
        _ => w,
    };
    // 남길 줄마다 (파일 안 자리, 바이트 수). 채널마다 모든 줄이 차례로 오므로(평면) 자리는 앞으로만 간다
    let mut rows: Vec<(u64, u64)> = Vec::with_capacity(used * oh);
    let start = src.pos;
    if compression == 0 {
        for c in 0..used {
            for y in (0..h).step_by(k) {
                let at = (c as u64 * h as u64 + y as u64).checked_mul(row_bytes as u64).and_then(|o| start.checked_add(o))?;
                rows.push((at, row_bytes as u64));
            }
        }
    } else {
        // 모든 채널의 줄마다 묶인 바이트 수의 표가 먼저 온다 (PSD는 u16, PSB는 u32). 표를 더해 가며 자리를 알아 내고,
        // 남기지 않는 줄은 풀지 않고 건너뛴다
        let wide = if hd.psb { 4 } else { 2 };
        let table = (channels as u64).checked_mul(h as u64)?.checked_mul(wide)?;
        let mut at = start.checked_add(table)?;
        if at > src.len {
            return None;
        }
        // 한 줄이 이보다 길게 묶일 수는 없다 (PackBits는 128바이트마다 1바이트를 더할 뿐이다. 넉넉히 두 배)
        let most = 2 * row_bytes as u64 + 2;
        for _ in 0..used {
            for y in 0..h {
                let n = if hd.psb { u64::from(src.u32()?) } else { u64::from(src.u16()?) };
                if y.is_multiple_of(k) {
                    if n > most {
                        return None;
                    }
                    rows.push((at, n));
                }
                at = at.checked_add(n)?;
            }
        }
    }
    // 마지막 줄까지 파일 안에 있어야 한다. 잘린 파일은 여기서 그쳐 큰 버퍼를 잡지 않는다
    let &(last, n) = rows.last()?;
    if last.checked_add(n)? > src.len {
        return None;
    }
    let mut out = vec![0u8; pixels * 4];
    let mut row = vec![0u8; row_bytes];
    let mut packed = Vec::new();
    for (i, &(at, n)) in rows.iter().enumerate() {
        let (c, oy) = (i / oh, i % oh);
        src.go(at)?;
        if compression == 0 {
            src.fill(&mut row)?;
        } else {
            packed.resize(n as usize, 0);
            src.fill(&mut packed)?;
            unpack(&packed, &mut row);
        }
        for (ox, px) in out[oy * ow * 4..(oy + 1) * ow * 4].as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = ox * k;
            let v = match hd.depth {
                // 비트맵은 1이 검정
                1 => if (row[x / 8] >> (7 - x % 8)) & 1 == 1 { 0 } else { 255 },
                16 => row[x * 2],
                _ => row[x],
            };
            match (hd.mode, palette) {
                (INDEXED, Some(p)) => {
                    let v = usize::from(v);
                    px[..3].copy_from_slice(&[p[v], p[256 + v], p[512 + v]]);
                }
                (RGB | CMYK, _) => px[c] = v,
                _ if c == 0 => px[..3].fill(v),
                _ => px[3] = v,
            }
        }
    }
    for px in out.as_chunks_mut::<4>().0 {
        if hd.mode == CMYK {
            // 저장된 값은 거꾸로라(0이 잉크 가득) 흰 종이에 남는 빛의 비율로 곱한다
            let black = u32::from(px[3]);
            for v in &mut px[..3] {
                *v = ((u32::from(*v) * black + 127) / 255) as u8;
            }
            px[3] = 255;
        } else if !alpha {
            px[3] = 255;
        } else if (1..255).contains(&px[3]) {
            // 투명도가 있는 합친 그림은 흰 바탕에 섞여 저장된다 (s = c·a + 255·(1−a)). 곱하지 않은 색으로 되돌린다
            let a = u32::from(px[3]);
            for v in &mut px[..3] {
                *v = (((u32::from(*v) + a).saturating_sub(255) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some((ow as u32, oh as u32, out))
}

/// PackBits로 묶인 한 줄 packed를 out에 푼다. 모자라면 뒤를 0으로 두고 넘치면 버린다 (줄마다 따로 묶여 다음 줄로 번지지 않는다)
fn unpack(packed: &[u8], out: &mut [u8]) {
    let (mut i, mut o) = (0, 0);
    while i < packed.len() && o < out.len() {
        let n = packed[i] as i8;
        i += 1;
        if n >= 0 {
            // n+1바이트를 그대로
            let len = n as usize + 1;
            let take = len.min(packed.len() - i).min(out.len() - o);
            out[o..o + take].copy_from_slice(&packed[i..i + take]);
            i += len;
            o += take;
        } else if n != -128 {
            // 다음 바이트를 1−n번 (−128은 아무것도 아니다)
            let Some(&b) = packed.get(i) else { break };
            i += 1;
            let take = (usize::from(n.unsigned_abs()) + 1).min(out.len() - o);
            out[o..o + take].fill(b);
            o += take;
        }
    }
    out[o..].fill(0);
}

/// 모든 화소가 같은 색인가
fn uniform(rgba: &[u8]) -> bool {
    let px = rgba.as_chunks::<4>().0;
    px.first().is_none_or(|first| px.iter().all(|p| p == first))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// 시험 파일의 재료. planes는 채널마다 깊이에 맞춘 줄들을 위 줄부터 이어 놓은 것
    #[derive(Clone)]
    struct Doc {
        psb: bool,
        mode: u16,
        depth: u16,
        w: u32,
        h: u32,
        planes: Vec<Vec<u8>>,
        color: Vec<u8>,
        res: Vec<(u16, Vec<u8>)>,
        layers: Vec<u8>,
        compression: u16,
    }

    fn doc(mode: u16, depth: u16, w: u32, h: u32, planes: Vec<Vec<u8>>) -> Doc {
        Doc { psb: false, mode, depth, w, h, planes, color: Vec::new(), res: Vec::new(), layers: Vec::new(), compression: 0 }
    }

    /// PackBits로 한 줄을 묶는다: 같은 바이트가 둘 이상 이어지면 되풀이, 아니면 그대로 (둘 다 128바이트까지)
    fn pack(row: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < row.len() {
            let mut run = 1;
            while i + run < row.len() && run < 128 && row[i + run] == row[i] {
                run += 1;
            }
            if run >= 2 {
                out.push((257 - run) as u8);
                out.push(row[i]);
                i += run;
            } else {
                let start = i;
                while i < row.len() && i - start < 128 && !(i + 1 < row.len() && row[i] == row[i + 1]) {
                    i += 1;
                }
                out.push((i - start - 1) as u8);
                out.extend_from_slice(&row[start..i]);
            }
        }
        out
    }

    fn block(res: &mut Vec<u8>, id: u16, name: &[u8], data: &[u8]) {
        res.extend(b"8BIM");
        res.extend(id.to_be_bytes());
        res.push(name.len() as u8);
        res.extend(name);
        if name.len().is_multiple_of(2) {
            res.push(0);
        }
        res.extend((data.len() as u32).to_be_bytes());
        res.extend(data);
        if !data.len().is_multiple_of(2) {
            res.push(0);
        }
    }

    /// 파일과, 그 안에서 합친 그림 구역이 시작하는 자리
    fn build_parts(d: &Doc) -> (Vec<u8>, usize) {
        let mut f = b"8BPS".to_vec();
        f.extend(if d.psb { 2u16 } else { 1 }.to_be_bytes());
        f.extend([0u8; 6]);
        f.extend((d.planes.len() as u16).to_be_bytes());
        f.extend(d.h.to_be_bytes());
        f.extend(d.w.to_be_bytes());
        f.extend(d.depth.to_be_bytes());
        f.extend(d.mode.to_be_bytes());
        f.extend((d.color.len() as u32).to_be_bytes());
        f.extend(&d.color);
        // 리소스: 늘 맨 앞에 이름(채움 바이트가 붙는)과 홀수 길이 데이터를 가진 다른 블록을 둔다
        let mut res = Vec::new();
        block(&mut res, 1005, b"ab", &[1, 2, 3, 4, 5]);
        for (id, data) in &d.res {
            block(&mut res, *id, b"", data);
        }
        f.extend((res.len() as u32).to_be_bytes());
        f.extend(res);
        if d.psb {
            f.extend((d.layers.len() as u64).to_be_bytes());
        } else {
            f.extend((d.layers.len() as u32).to_be_bytes());
        }
        f.extend(&d.layers);
        let image = f.len();
        f.extend(d.compression.to_be_bytes());
        let w = d.w as usize;
        let row_bytes = match d.depth {
            1 => w.div_ceil(8),
            16 => w * 2,
            _ => w,
        };
        if d.compression == 1 {
            let rows: Vec<Vec<u8>> = d.planes.iter().flat_map(|p| p.chunks(row_bytes).map(pack)).collect();
            for r in &rows {
                if d.psb {
                    f.extend((r.len() as u32).to_be_bytes());
                } else {
                    f.extend((r.len() as u16).to_be_bytes());
                }
            }
            rows.iter().for_each(|r| f.extend(r));
        } else {
            d.planes.iter().for_each(|p| f.extend(p));
        }
        (f, image)
    }

    fn build(d: &Doc) -> Vec<u8> {
        build_parts(d).0
    }

    fn open(b: &[u8], max_side: u32) -> Option<Picture> {
        read_from(Cursor::new(b), max_side)
    }

    fn pixels(p: Option<Picture>) -> (u32, u32, Vec<u8>) {
        match p {
            Some(Picture::Pixels { w, h, rgba }) => (w, h, rgba),
            Some(Picture::Jpeg { .. }) => panic!("got the thumbnail, not the composite"),
            None => panic!("got nothing"),
        }
    }

    /// 화소마다 f(x, y)가 준 채널 값들로 8비트 평면들을 만든다
    fn planes8<const N: usize>(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; N]) -> Vec<Vec<u8>> {
        let mut planes = vec![Vec::with_capacity(w * h); N];
        for y in 0..h {
            for x in 0..w {
                for (c, v) in f(x, y).into_iter().enumerate() {
                    planes[c].push(v);
                }
            }
        }
        planes
    }

    fn rgba_of(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).flat_map(|(x, y)| f(x, y)).collect()
    }

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0xFF, 0xD9];

    /// 썸네일 리소스: 28바이트 머리(형식 1 = JPEG, 너비, 높이, 줄 바이트, 전체 크기, 압축된 크기, 비트 수, 평면 수)와 JPEG
    fn thumb(jpeg: &[u8]) -> Vec<u8> {
        let mut r = Vec::new();
        for v in [1u32, 4, 4, 12, 48, jpeg.len() as u32] {
            r.extend(v.to_be_bytes());
        }
        r.extend(24u16.to_be_bytes());
        r.extend(1u16.to_be_bytes());
        r.extend(jpeg);
        r
    }

    fn thumb_of(jpeg: &[u8], bgr: bool) -> Option<Picture> {
        Some(Picture::Jpeg { data: jpeg.to_vec(), bgr })
    }

    #[test]
    fn reads_rgb_8bit_raw() {
        let px = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [10, 20, 30], [200, 100, 50], [1, 2, 3]];
        let mut d = doc(RGB, 8, 3, 2, planes8(3, 2, |x, y| px[y * 3 + x]));
        // 레이어 구역은 내용과 상관없이 건너뛴다
        d.layers = vec![0xAB; 37];
        let (w, h, rgba) = pixels(open(&build(&d), 100));
        assert_eq!((w, h), (3, 2));
        assert_eq!(rgba, px.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect::<Vec<u8>>());
    }

    #[test]
    fn unpacks_packbits() {
        // 되풀이(FE: 3번, FD: 4번, F7: 10번)와 그대로(02: 3바이트, 03: 4바이트)가 섞인 줄
        let packed = [0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7, 0xAA];
        let mut want = vec![0xAA; 3];
        want.extend([0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22]);
        want.extend([0xAA; 10]);
        let mut out = vec![0x11; 24];
        unpack(&packed, &mut out);
        assert_eq!(out, want);
        // 0x80은 건너뛴다, 모자라면 0, 넘치면 버린다, 되풀이할 바이트가 없다
        let mut out = [9u8; 4];
        unpack(&[0x80, 0x00, 0x41, 0x01, 0x42], &mut out);
        assert_eq!(out, [0x41, 0x42, 0, 0]);
        let mut out = [9u8; 2];
        unpack(&[0xFD, 0x07], &mut out);
        assert_eq!(out, [7, 7]);
        let mut out = [9u8; 3];
        unpack(&[0x05, 1, 2, 3, 4, 5, 6], &mut out);
        assert_eq!(out, [1, 2, 3]);
        let mut out = [9u8; 2];
        unpack(&[0xFE], &mut out);
        assert_eq!(out, [0, 0]);
        // 시험의 묶기: 128바이트씩 나뉜 되풀이 셋(6바이트)과 그대로 셋(300 + 3바이트)
        let same = [7u8; 300];
        let ramp: Vec<u8> = (0..300).map(|x| x as u8).collect();
        assert_eq!((pack(&same).len(), pack(&ramp).len()), (6, 303));
        for row in [same.to_vec(), ramp, (0..300).map(|x| (x / 5 * 37 % 251) as u8).collect()] {
            let mut out = vec![0; row.len()];
            unpack(&pack(&row), &mut out);
            assert_eq!(out, row);
        }
    }

    /// 한 줄 안에 되풀이와 그대로가 섞이고, 둘 다 128바이트를 넘어 나뉘는 줄들
    fn rle_rows(x: usize, y: usize) -> u8 {
        match y {
            0 => 7,
            1 => x as u8,
            2 => (x / 9 * 31) as u8,
            3 => if x < 150 { 200 } else { (x * 7) as u8 },
            _ => (x * x + y) as u8,
        }
    }

    #[test]
    fn reads_rgb_8bit_rle() {
        let f = |x, y| {
            let v = rle_rows(x, y);
            [v, v ^ 0x55, 255 - v]
        };
        let mut d = doc(RGB, 8, 300, 5, planes8(300, 5, f));
        d.compression = 1;
        let (w, h, rgba) = pixels(open(&build(&d), 1000));
        assert_eq!((w, h), (300, 5));
        assert_eq!(rgba, rgba_of(300, 5, |x, y| {
            let [r, g, b] = f(x, y);
            [r, g, b, 255]
        }));
    }

    #[test]
    fn reads_grayscale_16bit_high_byte() {
        let values: [u16; 8] = [0x0000, 0x12FF, 0xFF00, 0x8001, 0x7FFF, 0xFFFF, 0x0100, 0xA5A5];
        let plane: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        for compression in [0, 1] {
            let mut d = doc(GRAY, 16, 4, 2, vec![plane.clone()]);
            d.compression = compression;
            let (w, h, rgba) = pixels(open(&build(&d), 100));
            assert_eq!((w, h), (4, 2));
            let want: Vec<u8> = values.iter().flat_map(|v| {
                let g = (v >> 8) as u8;
                [g, g, g, 255]
            }).collect();
            assert_eq!(rgba, want);
        }
        // 듀오톤은 첫 채널을 회색으로
        let d = doc(DUOTONE, 8, 2, 1, vec![vec![30, 220]]);
        assert_eq!(pixels(open(&build(&d), 100)).2, [30, 30, 30, 255, 220, 220, 220, 255]);
    }

    #[test]
    fn reads_indexed_with_palette() {
        let mut d = doc(INDEXED, 8, 4, 1, vec![vec![0, 1, 128, 255]]);
        d.color = (0..256).map(|i| i as u8).chain((0..256).map(|i| 255 - i as u8)).chain((0..256).map(|i| (i / 2) as u8)).collect();
        let rgba = pixels(open(&build(&d), 100)).2;
        assert_eq!(rgba, [0, 255, 0, 255, 1, 254, 0, 255, 128, 127, 64, 255, 255, 0, 127, 255]);
        // 팔레트가 없으면 읽지 못한다
        d.color.truncate(100);
        assert_eq!(open(&build(&d), 100), None);
    }

    #[test]
    fn reads_cmyk_inverted() {
        // 저장된 값은 0이 잉크 가득: 흰 종이, 검정 가득, 시안 가득, 섞인 색. 다섯째 채널(별색이나 투명도)은 쓰지 않는다
        let px = [[255, 255, 255, 255, 9], [255, 255, 255, 0, 9], [0, 255, 255, 255, 9], [200, 100, 50, 128, 9]];
        let d = doc(CMYK, 8, 4, 1, planes8(4, 1, |x, _| px[x]));
        let rgba = pixels(open(&build(&d), 100)).2;
        assert_eq!(rgba, [255, 255, 255, 255, 0, 0, 0, 255, 0, 255, 255, 255, 100, 50, 25, 255]);
    }

    #[test]
    fn reads_psb_with_wide_lengths() {
        let f = |x: usize, y: usize| {
            let v = rle_rows(x, y);
            [v, 255 - v, v / 2]
        };
        let mut d = doc(RGB, 8, 200, 5, planes8(200, 5, f));
        d.compression = 1;
        d.layers = vec![0xCD; 50];
        let psd = open(&build(&d), 1000);
        d.psb = true;
        let file = build(&d);
        assert_eq!(be16(&file, 4), 2);
        let psb = open(&file, 1000);
        assert_eq!(psb, psd);
        assert_eq!(pixels(psb).2, rgba_of(200, 5, |x, y| {
            let [r, g, b] = f(x, y);
            [r, g, b, 255]
        }));
        // 그대로 저장한 PSB도
        d.compression = 0;
        assert_eq!(open(&build(&d), 1000), psd);
    }

    #[test]
    fn fourth_rgb_channel_is_alpha_unblended_from_white() {
        // 다 보임, 다 투명, 반쯤 (191 → 128, 255 → 255, 흰 바탕보다 어두운 100 → 0), 4분의 1
        let px = [[10, 20, 30, 255], [255, 255, 255, 0], [191, 255, 100, 128], [255, 223, 150, 64]];
        let d = doc(RGB, 8, 4, 1, planes8(4, 1, |x, _| px[x]));
        let rgba = pixels(open(&build(&d), 100)).2;
        // 223: ((223 + 64 − 255)·255 + 32) / 64 = 128, 150: 150 + 64 < 255 → 0
        assert_eq!(rgba, [10, 20, 30, 255, 255, 255, 255, 0, 128, 255, 0, 128, 255, 128, 0, 64]);
        // 회색의 둘째 채널도 같다
        let d = doc(GRAY, 8, 2, 1, vec![vec![191, 40], vec![128, 255]]);
        assert_eq!(pixels(open(&build(&d), 100)).2, [128, 128, 128, 128, 40, 40, 40, 255]);
    }

    #[test]
    fn shrinks_while_decoding() {
        let f = |x: usize, y: usize| [x as u8, y as u8, (x * 2 + y) as u8];
        for compression in [0, 1] {
            let mut d = doc(RGB, 8, 100, 40, planes8(100, 40, f));
            d.compression = compression;
            let file = build(&d);
            // 4칸·4줄마다 하나
            let (w, h, rgba) = pixels(open(&file, 25));
            assert_eq!((w, h), (25, 10));
            assert_eq!(rgba, rgba_of(25, 10, |x, y| {
                let [r, g, b] = f(x * 4, y * 4);
                [r, g, b, 255]
            }));
            // max_side가 긴 변을 나누어떨어뜨리지 않으면 k를 올린다: 100 / 30 → 4, 그래서 30이 아니라 25칸
            let (w, h, rgba) = pixels(open(&file, 30));
            assert_eq!((w, h), (25, 10));
            assert_eq!(&rgba[4 * 3..4 * 4], &[12, 0, 24, 255]);
            // 이미 작으면 그대로, max_side 0은 1로
            assert_eq!(pixels(open(&file, 100)).0, 100);
            assert_eq!(pixels(open(&file, 0)), (1, 1, vec![0, 0, 0, 255]));
        }
        // 크기가 k로 나누어떨어지지 않는 그림: 101×41을 25로 → k = 5, 21×9
        let d = doc(RGB, 8, 101, 41, planes8(101, 41, f));
        let (w, h, rgba) = pixels(open(&build(&d), 25));
        assert_eq!((w, h), (21, 9));
        let at = (8 * 21 + 20) * 4;
        assert_eq!(&rgba[at..at + 4], &[100, 40, 240, 255]);
    }

    #[test]
    fn blank_composite_gives_the_thumbnail() {
        let white = planes8(8, 8, |_, _| [255, 255, 255]);
        let mut d = doc(RGB, 8, 8, 8, white);
        d.res = vec![(THUMB, thumb(JPEG))];
        assert_eq!(open(&build(&d), 100), thumb_of(JPEG, false));
        // 옛 썸네일은 BGR, 둘 다 있으면 1036
        let mut old = JPEG.to_vec();
        old.push(1);
        d.res = vec![(THUMB_BGR, thumb(&old))];
        assert_eq!(open(&build(&d), 100), thumb_of(&old, true));
        d.res = vec![(THUMB_BGR, thumb(&old)), (THUMB, thumb(JPEG))];
        assert_eq!(open(&build(&d), 100), thumb_of(JPEG, false));
        // JPEG가 아닌 썸네일(형식 0)은 없는 셈
        let mut raw = thumb(JPEG);
        raw[3] = 0;
        d.res = vec![(THUMB, raw)];
        assert_eq!(pixels(open(&build(&d), 100)).0, 8);
        // 썸네일이 없으면 빈 그림이라도 그대로, 색이 있으면 썸네일보다 합친 그림
        d.res.clear();
        assert_eq!(pixels(open(&build(&d), 100)).2, vec![255; 8 * 8 * 4]);
        d.planes[0][5] = 0;
        d.res = vec![(THUMB, thumb(JPEG))];
        assert_eq!(pixels(open(&build(&d), 100)).2[5 * 4], 0);
    }

    #[test]
    fn unsupported_data_gives_the_thumbnail_or_none() {
        let rgb = planes8(4, 4, |x, y| [x as u8, y as u8, 1]);
        // ZIP 압축 (2, 3)
        let mut d = doc(RGB, 8, 4, 4, rgb.clone());
        d.compression = 2;
        d.res = vec![(THUMB, thumb(JPEG))];
        assert_eq!(open(&build(&d), 100), thumb_of(JPEG, false));
        d.compression = 3;
        d.res.clear();
        assert_eq!(open(&build(&d), 100), None);
        // Lab, 다중 채널, 32비트
        let mut d = doc(9, 8, 4, 4, rgb.clone());
        d.res = vec![(THUMB, thumb(JPEG))];
        assert_eq!(open(&build(&d), 100), thumb_of(JPEG, false));
        assert_eq!(open(&build(&doc(7, 8, 4, 4, rgb.clone())), 100), None);
        let wide: Vec<Vec<u8>> = rgb.iter().map(|p| p.repeat(4)).collect();
        assert_eq!(open(&build(&doc(RGB, 32, 4, 4, wide)), 100), None);
        // 채널이 모자란 RGB
        assert_eq!(open(&build(&doc(RGB, 8, 4, 4, rgb[..2].to_vec())), 100), None);
    }

    #[test]
    fn reads_bitmap_one_bit() {
        // 한 줄 10칸 = 2바이트 (뒤 6비트는 채움). 1이 검정
        let d = doc(BITMAP, 1, 10, 2, vec![vec![0b1010_0000, 0b1100_0000, 0b0000_0001, 0b0111_1111]]);
        let rgba = pixels(open(&build(&d), 100)).2;
        let gray: Vec<u8> = rgba.chunks(4).map(|p| p[0]).collect();
        assert_eq!(gray, [0, 255, 0, 255, 255, 255, 255, 255, 0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 255, 0]);
    }

    #[test]
    fn truncated_image_data_gives_the_thumbnail_or_none() {
        let mut d = doc(RGB, 8, 30, 6, planes8(30, 6, |x, y| [rle_rows(x, y), 3, 4]));
        d.compression = 1;
        d.res = vec![(THUMB, thumb(JPEG))];
        let (file, image) = build_parts(&d);
        assert!(matches!(open(&file, 100), Some(Picture::Pixels { .. })));
        for cut in image..file.len() {
            assert_eq!(open(&file[..cut], 100), thumb_of(JPEG, false), "cut at {cut}");
        }
        d.res.clear();
        let (file, image) = build_parts(&d);
        for cut in image..file.len() {
            assert_eq!(open(&file[..cut], 100), None, "cut at {cut}");
        }
    }

    #[test]
    fn refuses_what_is_not_a_psd() {
        let d = doc(RGB, 8, 2, 2, planes8(2, 2, |_, _| [1, 2, 3]));
        let file = build(&d);
        let with = |at: usize, v: &[u8]| {
            let mut f = file.clone();
            f[at..at + v.len()].copy_from_slice(v);
            open(&f, 100)
        };
        assert_eq!(open(b"", 100), None);
        assert_eq!(open(b"8BPS", 100), None);
        assert_eq!(with(0, b"8BPX"), None);
        assert_eq!(with(4, &3u16.to_be_bytes()), None);
        // 채널 0, 57 / 높이 0 / 너비가 한도 밖
        assert_eq!(with(12, &0u16.to_be_bytes()), None);
        assert_eq!(with(12, &57u16.to_be_bytes()), None);
        assert_eq!(with(14, &0u32.to_be_bytes()), None);
        assert_eq!(with(18, &300_001u32.to_be_bytes()), None);
        // 아주 큰 그림이라고 적힌 작은 파일: 큰 버퍼를 잡지 않고 그친다
        let mut d = doc(RGB, 8, 300_000, 300_000, planes8(2, 2, |_, _| [1, 2, 3]));
        assert_eq!(open(&build(&d), 256), None);
        assert_eq!(open(&build(&d), u32::MAX), None);
        d.compression = 1;
        assert_eq!(open(&build(&d), 256), None);
        assert_eq!(read(Path::new("없는 파일.psd"), 100), None);
    }

    /// 잘라 보고 아무렇게나 망가뜨린 파일 수천 개: 어느 것도 패닉하지 않는다
    #[test]
    fn damage_never_panics() {
        let mut rgb = doc(RGB, 8, 30, 6, planes8(30, 6, |x, y| [rle_rows(x, y), 3, 4, (x * 9) as u8]));
        rgb.compression = 1;
        rgb.res = vec![(THUMB_BGR, thumb(JPEG)), (THUMB, thumb(JPEG))];
        let mut psb = rgb.clone();
        psb.psb = true;
        psb.layers = vec![1; 20];
        let mut gray = doc(GRAY, 16, 7, 5, vec![(0..70).map(|i| (i * 13) as u8).collect()]);
        gray.compression = 1;
        let mut indexed = doc(INDEXED, 8, 9, 3, vec![(0..27).map(|i| (i * 40) as u8).collect()]);
        indexed.color = (0..768).map(|i| i as u8).collect();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for d in [rgb, psb, gray, indexed] {
            let file = build(&d);
            assert!(matches!(open(&file, 100), Some(Picture::Pixels { .. })), "the undamaged file decodes");
            for cut in 0..file.len() {
                let _ = open(&file[..cut], 100);
            }
            for round in 0..2000 {
                let mut f = file.clone();
                for _ in 0..1 + rand() % 8 {
                    let at = rand() as usize % f.len();
                    f[at] = rand() as u8;
                }
                if round % 5 == 0 {
                    f.truncate(rand() as usize % f.len());
                }
                let _ = open(&f, 1 + (rand() % 40) as u32);
            }
        }
    }
}
