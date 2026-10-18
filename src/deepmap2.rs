//! Deepmap2, the image codec behind CELM compression 11: tiles stored raw, compressed, as palette
//! indices or as YCoCg with per-row prediction, whichever is smallest. Matches actool byte for byte.

use anyhow::{anyhow, bail, ensure, Result};
use half::f16;

/// The quantisation actool encodes with: chroma divided by `1 << SHIFT`, and float channels
/// scaled by `1 << (FRACTION - 1)`.
const SHIFT: u32 = 1;
const FRACTION: u32 = 10;

const METHOD_RAW: u8 = 1;
const METHOD_PREDICTED: u8 = 2;
const METHOD_PACKED: u8 = 3;
const METHOD_PALETTE: u8 = 4;

/// Above this many bytes, a buffer is compressed with LZFSE rather than raw LZVN.
const LZVN_LIMIT: usize = 0x1000;
/// The largest decoded size of one tile.
const TILE_BYTES: usize = 0x20_0000;
/// actool encodes images of this many pixels or more in row chunks of this many pixels.
const CHUNK_PIXELS: usize = 0x10_0000;

/// The stream's pixel format code for a CSI pixel format.
pub fn format_code(pixel_format: &[u8]) -> Option<u8> {
    match pixel_format {
        b" 8AG" => Some(2),
        b"BGRA" => Some(4),
        b"61AG" => Some(18),
        b"WBGR" => Some(20),
        _ => None,
    }
}

fn pixel_size(format: u8) -> usize {
    match format {
        2 => 2,
        4 => 4,
        18 => 4,
        20 => 8,
        _ => 0,
    }
}

fn is_gray(format: u8) -> bool {
    matches!(format, 2 | 18)
}

/// One tile of an image: `rows` rows of `width` pixels, `stride` bytes apart, from `data`.
#[derive(Clone, Copy)]
struct Tile<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    stride: usize,
}

impl<'a> Tile<'a> {
    fn row(&self, y: usize, ps: usize) -> &'a [u8] {
        &self.data[y * self.stride..y * self.stride + self.width * ps]
    }

    fn sub(&self, x: usize, y: usize, w: usize, h: usize, ps: usize) -> Tile<'a> {
        let start = y * self.stride + x * ps;
        Tile { data: &self.data[start..], width: w, height: h, stride: self.stride }
    }

    fn packed(&self, ps: usize) -> Vec<u8> {
        (0..self.height).flat_map(|y| self.row(y, ps).iter().copied()).collect()
    }
}

#[derive(Clone)]
struct Palette {
    /// 3: 24-bit colours plus an alpha plane; 4: whole 32-bit pixels.
    kind: u32,
    entries: Vec<u32>,
    /// Index of the first entry with each hash.
    first: [u32; 256],
}

fn palette_hash(px: u32, kind: u32) -> usize {
    let h = (px >> 8 ^ px >> 16 ^ px) & 0xff;
    (if kind == 4 { h ^ px >> 24 } else { h }) as usize
}

impl Palette {
    fn hash(&self, px: u32) -> usize {
        palette_hash(px, self.kind)
    }

    fn matches(&self, entry: u32, px: u32) -> bool {
        if self.kind == 4 { entry == px } else { (entry ^ px) & 0xff_ffff == 0 }
    }

    fn index(&self, px: u32) -> Option<u8> {
        let h = self.hash(px);
        let mut i = self.first[h] as usize;
        if i > 0xff {
            return None;
        }
        let entry = |i: usize| self.entries.get(i).copied().unwrap_or(0);
        while self.hash(entry(i)) == h && i < 0xff && !self.matches(entry(i), px) {
            i += 1;
        }
        self.matches(entry(i), px).then_some(i as u8)
    }

    fn build(mut values: Vec<u32>, kind: u32) -> Option<Palette> {
        values.dedup();
        if values.len() > 256 {
            return None;
        }
        qsort(&mut values, |a, b| palette_hash(a, kind) as i32 - palette_hash(b, kind) as i32);
        let mut p = Palette { kind, entries: values, first: [u32::MAX; 256] };
        let mut prev = p.hash(p.entries[0]);
        p.first[prev] = 0;
        for i in 1..p.entries.len() {
            let h = p.hash(p.entries[i]);
            if h != prev {
                p.first[h] = i as u32;
                prev = h;
            }
        }
        Some(p)
    }
}

/// The 24-bit and 32-bit palettes of a BGRA image, each when it has at most 256 colours.
fn build_palettes(img: &Tile) -> (Option<Palette>, Option<Palette>) {
    let mut px: Vec<u32> = (0..img.height)
        .flat_map(|y| img.row(y, 4).chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())))
        .collect();
    qsort(&mut px, |a, b| (a & 0xff_ffff) as i32 - (b & 0xff_ffff) as i32);
    let rgb = Palette::build(px.iter().map(|p| p & 0xff_ffff).collect(), 3);
    qsort(&mut px, |a, b| (a > b) as i32 - (a < b) as i32);
    let rgba = Palette::build(px, 4);
    (rgba, rgb)
}

struct Header {
    format: u8,
    method: u8,
    tile_w: usize,
    tile_h: usize,
    palette: Option<Palette>,
}

impl Header {
    fn new(format: u8, method: u8, width: usize, height: usize, palette: Option<Palette>) -> Header {
        let mut h = Header { format, method, tile_w: width, tile_h: height, palette };
        if method == METHOD_RAW {
            return h;
        }
        h.tile_w = width.min(0x4000);
        let row_bytes = match method {
            METHOD_PALETTE => h.tile_w * if h.palette.as_ref().is_some_and(|p| p.kind == 3) { 2 } else { 1 },
            METHOD_PACKED => pixel_size(format) * h.tile_w,
            _ => predicted_buffer_len(format, h.tile_w, 1),
        };
        h.tile_h = height.min(TILE_BYTES / row_bytes.max(1)).max(1);
        h
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"dmp2");
        out.extend_from_slice(&[self.method, SHIFT as u8, FRACTION as u8, self.format]);
        out.extend_from_slice(&(self.tile_w as u16).to_le_bytes());
        out.extend_from_slice(&(self.tile_h as u16).to_le_bytes());
        if let Some(p) = &self.palette {
            out.extend_from_slice(&(p.entries.len() as u32 | p.kind << 16).to_le_bytes());
            for e in &p.entries {
                out.extend_from_slice(&e.to_le_bytes());
            }
        }
    }

    fn read(d: &[u8]) -> Result<(Header, usize)> {
        ensure!(d.len() >= 12 && &d[..4] == b"dmp2", "not a dmp2 stream");
        let (method, format) = (d[4], d[7]);
        ensure!(d[5] as u32 == SHIFT && d[6] as u32 == FRACTION, "dmp2 parameters {} {}", d[5], d[6]);
        let tile_w = u16::from_le_bytes([d[8], d[9]]) as usize;
        let tile_h = u16::from_le_bytes([d[10], d[11]]) as usize;
        let mut h = Header { format, method, tile_w, tile_h, palette: None };
        if method != METHOD_PALETTE {
            return Ok((h, 12));
        }
        ensure!(d.len() >= 16, "short palette header");
        let word = u32::from_le_bytes(d[12..16].try_into().unwrap());
        let (count, kind) = ((word & 0xffff) as usize, word >> 16);
        ensure!(count <= 256 && (kind == 3 || kind == 4), "bad palette {count}/{kind}");
        let entries: Vec<u32> = d
            .get(16..16 + 4 * count)
            .ok_or_else(|| anyhow!("short palette"))?
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        h.palette = Some(Palette { kind, entries, first: [u32::MAX; 256] });
        Ok((h, 16 + 4 * count))
    }
}

/// Bytes of the predicted method's buffer: alpha plane, predictor bytes, high and low planes.
fn predicted_buffer_len(format: u8, width: usize, height: usize) -> usize {
    predicted_raw_len(format, width, height) + 15 & !15
}

fn predicted_raw_len(format: u8, width: usize, height: usize) -> usize {
    let alpha = if matches!(format, 2 | 4 | 18 | 20) { width * height } else { 0 };
    height + alpha + if is_gray(format) { 2 } else { 6 } * width * height
}

unsafe extern "C" {
    fn lzvn_encode_buffer(dst: *mut u8, dst_size: usize, src: *const u8, src_size: usize, work: *mut u8) -> usize;
    fn lzvn_encode_scratch_size() -> usize;
}

/// Compresses `src` into at most `capacity` bytes, as LZVN below 4 KiB and LZFSE above, the way
/// `compression_encode_buffer` does; `None` when it doesn't fit.
fn compress(src: &[u8], threshold_len: usize, capacity: usize) -> Option<Vec<u8>> {
    let mut out = vec![0u8; capacity];
    let n = if threshold_len >= LZVN_LIMIT {
        lzfse::encode_buffer(src, &mut out).ok()?
    } else {
        let mut work = vec![0u8; unsafe { lzvn_encode_scratch_size() }];
        unsafe { lzvn_encode_buffer(out.as_mut_ptr(), capacity, src.as_ptr(), src.len(), work.as_mut_ptr()) }
    };
    (n > 0).then(|| {
        out.truncate(n);
        out
    })
}

fn decompress(src: &[u8], len: usize, threshold_len: usize) -> Result<Vec<u8>> {
    let mut framed;
    let input = if threshold_len >= LZVN_LIMIT {
        src
    } else {
        // Raw LZVN, framed as an LZFSE `bvxn` block so the LZFSE decoder takes it.
        framed = b"bvxn".to_vec();
        framed.extend_from_slice(&(len as u32).to_le_bytes());
        framed.extend_from_slice(&(src.len() as u32).to_le_bytes());
        framed.extend_from_slice(src);
        framed.extend_from_slice(b"bvx$");
        &framed
    };
    let mut out = vec![0u8; len + 1];
    let n = lzfse::decode_buffer(input, &mut out).map_err(|e| anyhow!("lzfse: {e:?}"))?;
    ensure!(n == len, "decoded {n} bytes, want {len}");
    out.truncate(len);
    Ok(out)
}

/// Encodes `width` × `height` pixels, rows `stride` bytes apart, as a `dmp2` stream.
pub fn encode(pixels: &[u8], stride: usize, pixel_format: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let format = format_code(pixel_format)?;
    let (w, h) = (width as usize, height as usize);
    let ps = pixel_size(format);
    if w == 0 || h == 0 || pixels.len() < (h - 1) * stride + w * ps {
        return None;
    }
    let img = Tile { data: pixels, width: w, height: h, stride };
    let capacity = w * h * ps + 12;
    let worse = capacity + 1;
    let packed = encode_with(&img, format, METHOD_PACKED, capacity);
    let palette = if format == 4 { encode_with(&img, format, METHOD_PALETTE, capacity) } else { None };
    let predicted = encode_with(&img, format, METHOD_PREDICTED, capacity);
    let size = |r: &Option<Vec<u8>>| r.as_ref().map_or(worse, Vec::len);
    let best = size(&packed).min(size(&predicted)).min(size(&palette)).min(capacity);
    if size(&predicted) == best {
        return predicted;
    }
    if size(&packed) == best {
        return packed;
    }
    if size(&palette) == best {
        return palette;
    }
    encode_with(&img, format, METHOD_RAW, capacity)
}

fn encode_with(img: &Tile, format: u8, method: u8, capacity: usize) -> Option<Vec<u8>> {
    if method != METHOD_PALETTE {
        return encode_tiles(img, Header::new(format, method, img.width, img.height, None), capacity);
    }
    let (rgba, rgb) = build_palettes(img);
    let with = |p: Option<Palette>| {
        p.and_then(|p| encode_tiles(img, Header::new(format, method, img.width, img.height, Some(p)), capacity))
    };
    let full = with(rgba);
    let split = with(rgb);
    match (full, split) {
        (Some(f), Some(s)) => Some(if f.len() <= s.len() { f } else { s }),
        (f, s) => f.or(s),
    }
}

fn encode_tiles(img: &Tile, header: Header, capacity: usize) -> Option<Vec<u8>> {
    let ps = pixel_size(header.format);
    let mut out = Vec::new();
    header.write(&mut out);
    if out.len() >= capacity {
        return None;
    }
    let mut room = capacity - out.len();
    if header.method == METHOD_RAW {
        if room < img.width * img.height * ps {
            return None;
        }
        out.extend(img.packed(ps));
        return Some(out);
    }
    if room < 9 || img.width * img.height * ps < 9 {
        return None;
    }
    let single = header.tile_w >= img.width && header.tile_h >= img.height;
    for y in (0..img.height).step_by(header.tile_h) {
        for x in (0..img.width).step_by(header.tile_w) {
            if room < 5 {
                return None;
            }
            let tile = img.sub(x, y, header.tile_w.min(img.width - x), header.tile_h.min(img.height - y), ps);
            let body = match header.method {
                METHOD_PREDICTED => encode_predicted(&tile, header.format, room - 4),
                METHOD_PACKED => {
                    let raw = tile.packed(ps);
                    compress(&raw, raw.len(), room - 4)
                }
                _ => encode_palette(&tile, header.palette.as_ref()?, room - 4),
            }?;
            if !single && body.len() > TILE_BYTES {
                return None;
            }
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend(&body);
            room -= body.len() + 4;
        }
    }
    Some(out)
}

fn encode_palette(tile: &Tile, p: &Palette, capacity: usize) -> Option<Vec<u8>> {
    let n = tile.width * tile.height;
    let mut buf = vec![0u8; if p.kind == 3 { 2 * n } else { n }];
    let indices_at = if p.kind == 3 { n } else { 0 };
    for y in 0..tile.height {
        for (x, c) in tile.row(y, 4).chunks_exact(4).enumerate() {
            let px = u32::from_le_bytes(c.try_into().unwrap());
            let i = y * tile.width + x;
            if p.kind == 3 {
                buf[i] = (px >> 24) as u8;
            }
            buf[indices_at + i] = p.index(px)?;
        }
    }
    compress(&buf, buf.len(), capacity)
}

/// One pixel as Y, Co, Cg, with chroma divided by `1 << SHIFT`.
fn ycocg(c0: i32, c1: i32, c2: i32) -> [i16; 3] {
    let [c0, c1, c2] = [c0 as i16, c1 as i16, c2 as i16];
    let co = c0.wrapping_sub(c2);
    let t = c2.wrapping_add(co / 2);
    let cg = c1.wrapping_sub(t);
    let d = 1i16 << SHIFT;
    [t.wrapping_add(cg / 2), co / d, cg / d]
}

fn half(b: &[u8], i: usize) -> f32 {
    f16::from_le_bytes([b[2 * i], b[2 * i + 1]]).to_f32()
}

fn alpha8(a: f32) -> u8 {
    let v = (a * 255.0) as u32;
    let v = if v >> 31 != 0 { 0 } else { v };
    v.min(255) as u8
}

/// Converts one row to Y/Co/Cg triples plus 8-bit alpha.
fn convert_row(format: u8, row: &[u8], alpha: &mut [u8], out: &mut [i16]) {
    let scale = (1u32 << (FRACTION - 1)) as f32;
    let fixed = |v: f32| (scale * v) as i32;
    for (x, o) in out.chunks_exact_mut(3).enumerate() {
        let (a, v) = match format {
            2 => (row[2 * x + 1], [row[2 * x] as i16, 0, 0]),
            4 => {
                let p = &row[4 * x..4 * x + 4];
                (p[3], ycocg(p[0] as i32, p[1] as i32, p[2] as i32))
            }
            18 => {
                let y = fixed(half(row, 2 * x)) as f32 as i32;
                (alpha8(half(row, 2 * x + 1)), [y as i16, 0, 0])
            }
            _ => {
                let c = |k| half(row, 4 * x + k);
                (alpha8(c(3)), ycocg(fixed(c(0)), fixed(c(1)), fixed(c(2))))
            }
        };
        alpha[x] = a;
        o.copy_from_slice(&v);
    }
}

fn cost(v: i16) -> f32 {
    ((v as i32).abs() as f64 + 1.0).ln() as f32
}

fn predict_left(cur: &[i16], out: &mut [i16]) -> f32 {
    let mut c = 0.0f32;
    for i in 0..cur.len() {
        out[i] = if i < 3 { cur[i] } else { cur[i].wrapping_sub(cur[i - 3]) };
        c += cost(out[i]);
    }
    c
}

fn predict_up(cur: &[i16], prev: &[i16], out: &mut [i16]) -> f32 {
    let mut c = 0.0f32;
    for i in 0..cur.len() {
        out[i] = cur[i].wrapping_sub(prev[i]);
        c += cost(out[i]);
    }
    c
}

fn predict_mean(cur: &[i16], prev: &[i16], out: &mut [i16]) -> f32 {
    let mut c = 0.0f32;
    for i in 0..cur.len() {
        out[i] = if i < 3 {
            cur[i].wrapping_sub(prev[i])
        } else {
            cur[i].wrapping_sub(((cur[i - 3] as i32 + prev[i] as i32 + 1) / 2) as i16)
        };
        c += cost(out[i]);
    }
    c
}

/// Picks left or up for each pixel from its Y channel; unlike PNG's Paeth it never picks up-left.
fn paeth_left(left: i16, up: i16, up_left: i16) -> bool {
    (up as i32 - up_left as i32).unsigned_abs() <= (left as i32 - up_left as i32).unsigned_abs()
}

fn predict_paeth(cur: &[i16], prev: &[i16], out: &mut [i16]) -> f32 {
    let mut c = 0.0f32;
    for i in 0..3.min(cur.len()) {
        out[i] = cur[i].wrapping_sub(prev[i]);
        c += cost(out[i]);
    }
    for p in (3..cur.len()).step_by(3) {
        let from = if paeth_left(cur[p - 3], prev[p], prev[p - 3]) { &cur[p - 3..p] } else { &prev[p..p + 3] };
        for k in 0..3 {
            out[p + k] = cur[p + k].wrapping_sub(from[k]);
        }
        // A pixel's three costs are summed before joining the total; the order decides ties.
        c += cost(out[p]) + cost(out[p + 1]) + cost(out[p + 2]);
    }
    c
}

fn sign_magnitude(v: i16) -> u16 {
    (if v >= 0 { v.wrapping_mul(2) } else { v.wrapping_mul(-2).wrapping_add(1) }) as u16
}

fn encode_predicted(tile: &Tile, format: u8, capacity: usize) -> Option<Vec<u8>> {
    let (w, h) = (tile.width, tile.height);
    let ps = pixel_size(format);
    let channels = if is_gray(format) { 1 } else { 3 };
    let alpha_len = if matches!(format, 2 | 4 | 18 | 20) { w * h } else { 0 };
    let raw_len = predicted_raw_len(format, w, h);
    let mut buf = vec![0u8; raw_len + 15 & !15];
    let (alpha, rest) = buf.split_at_mut(alpha_len);
    let (codes, planes) = rest.split_at_mut(h);
    let (high, low) = planes.split_at_mut(channels * w * h);
    let n = 3 * w;
    let mut prev = vec![0i16; n];
    let mut cur = vec![0i16; n];
    let mut scratch = [vec![0i16; n], vec![0i16; n], vec![0i16; n], vec![0i16; n]];
    let mut dummy_alpha = vec![0u8; w];
    for y in 0..h {
        std::mem::swap(&mut prev, &mut cur);
        let a = if alpha_len > 0 { &mut alpha[y * w..(y + 1) * w] } else { &mut dummy_alpha[..] };
        convert_row(format, tile.row(y, ps), a, &mut cur);
        let none = cur.iter().fold(0.0f32, |c, &v| c + cost(v));
        let [left, up, mean, paeth] = &mut scratch;
        let left_cost = predict_left(&cur, left);
        let (mut best, mut code, mut row): (f32, u8, &[i16]) = (none, 0, &cur);
        if left_cost < best {
            (best, code, row) = (left_cost, 2, left);
        }
        if y > 0 {
            let c = predict_up(&cur, &prev, up);
            if c < best {
                (best, code, row) = (c, 3, up);
            }
            let c = predict_mean(&cur, &prev, mean);
            if c < best {
                (best, code, row) = (c, 4, mean);
            }
            let c = predict_paeth(&cur, &prev, paeth);
            if c < best {
                (code, row) = (1, paeth);
            }
        }
        codes[y] = code;
        for x in 0..w {
            for k in 0..channels {
                let s = sign_magnitude(row[3 * x + k]);
                let i = (y * w + x) * channels + k;
                low[i] = s as u8;
                high[i] = (s >> 8) as u8;
            }
        }
    }
    compress(&buf, raw_len + 15, capacity)
}

/// Decodes a `dmp2` stream of a `width` × `height` image into tightly packed rows.
pub fn decode(d: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let (mut header, mut p) = Header::read(d)?;
    let ps = pixel_size(header.format);
    ensure!(ps > 0, "dmp2 pixel format {}", header.format);
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; w * h * ps];
    if header.method == METHOD_RAW {
        let body = d.get(p..p + out.len()).ok_or_else(|| anyhow!("short raw dmp2"))?;
        out.copy_from_slice(body);
        return Ok(out);
    }
    ensure!(header.tile_w > 0 && header.tile_h > 0, "empty dmp2 tile");
    if let Some(pal) = header.palette.as_mut() {
        for (i, e) in pal.entries.clone().iter().enumerate().rev() {
            pal.first[pal.hash(*e)] = i as u32;
        }
    }
    for y in (0..h).step_by(header.tile_h) {
        for x in (0..w).step_by(header.tile_w) {
            let (tw, th) = (header.tile_w.min(w - x), header.tile_h.min(h - y));
            let len = d.get(p..p + 4).ok_or_else(|| anyhow!("short dmp2 tile"))?;
            let len = u32::from_le_bytes(len.try_into().unwrap()) as usize;
            let body = d.get(p + 4..p + 4 + len).ok_or_else(|| anyhow!("dmp2 tile past end"))?;
            p += 4 + len;
            let pixels = match header.method {
                METHOD_PREDICTED => decode_predicted(body, header.format, tw, th)?,
                METHOD_PACKED => decompress(body, tw * th * ps, tw * th * ps)?,
                METHOD_PALETTE => {
                    let pal = header.palette.as_ref().ok_or_else(|| anyhow!("palette method without palette"))?;
                    decode_palette(body, pal, tw * th)?
                }
                m => bail!("dmp2 method {m}"),
            };
            for row in 0..th {
                let dst = (y + row) * w * ps + x * ps;
                out[dst..dst + tw * ps].copy_from_slice(&pixels[row * tw * ps..(row + 1) * tw * ps]);
            }
        }
    }
    Ok(out)
}

fn decode_palette(body: &[u8], p: &Palette, n: usize) -> Result<Vec<u8>> {
    let len = if p.kind == 3 { 2 * n } else { n };
    let buf = decompress(body, len, len)?;
    let mut out = Vec::with_capacity(4 * n);
    for i in 0..n {
        let index = buf[if p.kind == 3 { n + i } else { i }] as usize;
        let e = *p.entries.get(index).ok_or_else(|| anyhow!("palette index {index} out of range"))?;
        let px = if p.kind == 3 { e & 0xff_ffff | (buf[i] as u32) << 24 } else { e };
        out.extend_from_slice(&px.to_le_bytes());
    }
    Ok(out)
}

fn decode_predicted(body: &[u8], format: u8, w: usize, h: usize) -> Result<Vec<u8>> {
    let raw_len = predicted_raw_len(format, w, h);
    let buf = decompress(body, raw_len + 15 & !15, raw_len + 15)?;
    let ps = pixel_size(format);
    let channels = if is_gray(format) { 1 } else { 3 };
    let alpha_len = if matches!(format, 2 | 4 | 18 | 20) { w * h } else { 0 };
    let (alpha, rest) = buf.split_at(alpha_len);
    let (codes, planes) = rest.split_at(h);
    let (high, low) = planes.split_at(channels * w * h);
    let n = 3 * w;
    let mut prev = vec![0i16; n];
    let mut cur = vec![0i16; n];
    let mut residual = vec![0i16; n];
    let mut out = vec![0u8; w * h * ps];
    for y in 0..h {
        std::mem::swap(&mut prev, &mut cur);
        for x in 0..w {
            for k in 0..3 {
                residual[3 * x + k] = if k < channels {
                    let i = (y * w + x) * channels + k;
                    let s = u16::from_le_bytes([low[i], high[i]]);
                    let m = (s >> 1) as i16;
                    if s & 1 != 0 { m.wrapping_neg() } else { m }
                } else {
                    0
                };
            }
        }
        for i in 0..n {
            cur[i] = match codes[y] {
                0 => residual[i],
                2 if i < 3 => residual[i],
                2 => residual[i].wrapping_add(cur[i - 3]),
                3 => residual[i].wrapping_add(prev[i]),
                4 if i < 3 => residual[i].wrapping_add(prev[i]),
                4 => residual[i].wrapping_add(((cur[i - 3] as i32 + prev[i] as i32 + 1) / 2) as i16),
                1 if i < 3 => residual[i].wrapping_add(prev[i]),
                1 => {
                    let p = i - i % 3;
                    let left = paeth_left(cur[p - 3], prev[p], prev[p - 3]);
                    residual[i].wrapping_add(if left { cur[i - 3] } else { prev[i] })
                }
                c => bail!("dmp2 predictor {c}"),
            };
        }
        let a = if alpha_len > 0 { &alpha[y * w..(y + 1) * w] } else { &[][..] };
        unconvert_row(format, &cur, a, &mut out[y * w * ps..(y + 1) * w * ps]);
    }
    Ok(out)
}

fn unconvert_row(format: u8, v: &[i16], alpha: &[u8], out: &mut [u8]) {
    let scale = 1.0f32 / (1u32 << (FRACTION - 1)) as f32;
    let alpha_f = |a: u8| f16::from_f32((a as f32 * 0.003921569).min(1.0));
    let rgb = |y: i16, co: i16, cg: i16| {
        let co = ((co as u16 as u32) << SHIFT) as i16;
        let cg = ((cg as u16 as u32) << SHIFT) as i16;
        let t = y.wrapping_sub(cg / 2);
        let c2 = t.wrapping_sub(co / 2);
        [c2.wrapping_add(co), cg.wrapping_add(t), c2]
    };
    for x in 0..out.len() / pixel_size(format) {
        let (y, co, cg) = (v[3 * x], v[3 * x + 1], v[3 * x + 2]);
        match format {
            2 => out[2 * x..2 * x + 2].copy_from_slice(&[y as u8, alpha[x]]),
            4 => {
                let c = rgb(y, co, cg).map(|c| c as u8);
                out[4 * x..4 * x + 4].copy_from_slice(&[c[0], c[1], c[2], alpha[x]]);
            }
            18 => {
                out[4 * x..4 * x + 2].copy_from_slice(&f16::from_f32(scale * y as f32).to_le_bytes());
                out[4 * x + 2..4 * x + 4].copy_from_slice(&alpha_f(alpha[x]).to_le_bytes());
            }
            _ => {
                let c = rgb(y, co, cg);
                for k in 0..3 {
                    out[8 * x + 2 * k..8 * x + 2 * k + 2]
                        .copy_from_slice(&f16::from_f32(scale * c[k] as f32).to_le_bytes());
                }
                out[8 * x + 6..8 * x + 8].copy_from_slice(&alpha_f(alpha[x]).to_le_bytes());
            }
        }
    }
}

/// The CELM payload of a deepmap2 rendition: one `dmp2` stream behind a 16-byte header, or, for
/// images of a million pixels or more, `KCBC` chunks of whole rows.
pub fn celm(pixels: &[u8], stride: usize, pixel_format: &[u8], width: u32, height: u32, opaque: bool) -> Option<Vec<u8>> {
    let format = format_code(pixel_format)?;
    let (w, h) = (width as usize, height as usize);
    let rows_per_chunk = if w * h < CHUNK_PIXELS { h } else { (CHUNK_PIXELS / w).max(1) };
    let mut chunks = Vec::new();
    for y in (0..h).step_by(rows_per_chunk.max(1)) {
        let rows = rows_per_chunk.min(h - y);
        let dmp2 = encode(&pixels[y * stride..], stride, pixel_format, width, rows as u32)?;
        let mut c = Vec::with_capacity(16 + dmp2.len());
        for v in [1, format as u32, dmp2.len() as u32, 0] {
            c.extend_from_slice(&v.to_le_bytes());
        }
        c.extend(dmp2);
        chunks.push((rows, c));
    }
    let chunked = chunks.len() > 1;
    let mut out = b"MLEC".to_vec();
    out.extend_from_slice(&(chunked as u32 | (opaque as u32) << 1).to_le_bytes());
    out.extend_from_slice(&11u32.to_le_bytes());
    if !chunked {
        let (_, c) = chunks.pop()?;
        out.extend_from_slice(&(c.len() as u32).to_le_bytes());
        out.extend(c);
        return Some(out);
    }
    out.extend_from_slice(&(chunks.len() as u32).to_le_bytes());
    for (rows, c) in chunks {
        out.extend_from_slice(b"KCBC");
        for v in [0, 0, rows as u32, c.len() as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend(c);
    }
    Some(out)
}

/// Decodes one chunk's 16-byte header and `dmp2` stream.
pub fn decode_chunk(c: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    ensure!(c.len() >= 16, "short deepmap2 chunk");
    let len = u32::from_le_bytes(c[8..12].try_into().unwrap()) as usize;
    decode(c.get(16..16 + len).ok_or_else(|| anyhow!("deepmap2 chunk past end"))?, width, height)
}

/// The sort actool's palette builder runs: the C library's `qsort`, whose order among entries
/// that compare equal decides palette indices.
fn qsort(a: &mut [u32], cmp: impl Fn(u32, u32) -> i32 + Copy) {
    let n = a.len();
    let fls = usize::BITS - n.leading_zeros();
    qsort_depth(a, cmp, 2 * fls as i32 - 2);
}

fn med3(a: &[u32], i: usize, j: usize, k: usize, cmp: impl Fn(u32, u32) -> i32) -> usize {
    if cmp(a[i], a[j]) < 0 {
        if cmp(a[j], a[k]) < 0 { j } else if cmp(a[i], a[k]) < 0 { k } else { i }
    } else if cmp(a[j], a[k]) > 0 {
        j
    } else if cmp(a[i], a[k]) < 0 {
        i
    } else {
        k
    }
}

/// Insertion sort; with a `limit`, gives up (returning false) on the swap after `limit` swaps.
fn isort(a: &mut [u32], cmp: impl Fn(u32, u32) -> i32, limit: usize) -> bool {
    let mut swaps = 0;
    for i in 1..a.len() {
        let mut j = i;
        while j > 0 && cmp(a[j - 1], a[j]) > 0 {
            a.swap(j - 1, j);
            j -= 1;
            if limit != 0 {
                if limit <= swaps {
                    return false;
                }
                swaps += 1;
            }
        }
    }
    true
}

fn vecswap(a: &mut [u32], i: usize, j: usize, n: usize) {
    for k in 0..n {
        a.swap(i + k, j + k);
    }
}

fn qsort_depth(mut a: &mut [u32], cmp: impl Fn(u32, u32) -> i32 + Copy, mut depth: i32) {
    if depth < 1 {
        heapsort(a, cmp);
        return;
    }
    loop {
        let n = a.len();
        if n < 8 {
            isort(a, cmp, 0);
            return;
        }
        let (mut pl, mut pm, mut pn) = (0, n / 2, n - 1);
        if n > 40 {
            let d = n / 8;
            pl = med3(a, pl, pl + d, pl + 2 * d, cmp);
            pm = med3(a, pm - d, pm, pm + d, cmp);
            pn = med3(a, pn - 2 * d, pn - d, pn, cmp);
        }
        pm = med3(a, pl, pm, pn, cmp);
        a.swap(0, pm);
        let level = depth;
        depth -= 1;
        let mut swapped = false;
        let (mut pa, mut pb, mut pc, mut pd) = (1usize, 1usize, n - 1, n - 1);
        loop {
            while pb <= pc {
                let r = cmp(a[pb], a[0]);
                if r > 0 {
                    break;
                }
                if r == 0 {
                    swapped = true;
                    a.swap(pa, pb);
                    pa += 1;
                }
                pb += 1;
            }
            while pb <= pc {
                let r = cmp(a[pc], a[0]);
                if r < 0 {
                    break;
                }
                if r == 0 {
                    swapped = true;
                    a.swap(pc, pd);
                    pd -= 1;
                }
                pc -= 1;
            }
            if pb > pc {
                break;
            }
            a.swap(pb, pc);
            swapped = true;
            pb += 1;
            pc -= 1;
        }
        let d1 = pb - pa;
        let r = pa.min(d1);
        vecswap(a, 0, pb - r, r);
        let d2 = pd - pc;
        let r = d2.min(n - pd - 1);
        vecswap(a, pb, n - r, r);
        if !swapped && isort(a, cmp, n / 4 + 1) {
            return;
        }
        if d1 <= d2 {
            if d1 > 1 {
                qsort_depth(&mut a[..d1], cmp, depth);
            }
            if d2 <= 1 {
                return;
            }
            a = &mut a[n - d2..];
        } else {
            if d2 > 1 {
                qsort_depth(&mut a[n - d2..], cmp, depth);
            }
            if d1 <= 1 {
                return;
            }
            a = &mut a[..d1];
        }
        if level < 2 {
            heapsort(a, cmp);
            return;
        }
    }
}

fn heapsort(a: &mut [u32], cmp: impl Fn(u32, u32) -> i32) {
    let n = a.len();
    if n < 2 {
        return;
    }
    // 1-based heap over `a`.
    let at = |i: usize| i - 1;
    for start in (1..=n / 2).rev() {
        let mut par = start;
        while par * 2 <= n {
            let mut child = par * 2;
            if child < n && cmp(a[at(child)], a[at(child + 1)]) < 0 {
                child += 1;
            }
            if cmp(a[at(child)], a[at(par)]) <= 0 {
                break;
            }
            a.swap(at(par), at(child));
            par = child;
        }
    }
    let mut n = n;
    while n > 1 {
        let k = a[at(n)];
        a[at(n)] = a[at(1)];
        n -= 1;
        let mut par = 1;
        while par * 2 <= n {
            let mut child = par * 2;
            if child < n && cmp(a[at(child)], a[at(child + 1)]) < 0 {
                child += 1;
            }
            a[at(par)] = a[at(child)];
            par = child;
        }
        loop {
            let child = par;
            par = child / 2;
            if child == 1 || cmp(k, a[at(par)]) < 0 {
                a[at(child)] = k;
                break;
            }
            a[at(child)] = a[at(par)];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(pixels: &[u8], pf: &[u8], w: u32, h: u32) -> Vec<u8> {
        let ps = pixel_size(format_code(pf).unwrap());
        let d = encode(pixels, w as usize * ps, pf, w, h).unwrap();
        decode(&d, w, h).unwrap()
    }

    #[test]
    fn gray_and_palette_images_roundtrip_exactly() {
        let ga: Vec<u8> = (0..64 * 48).flat_map(|i| [(i * 7 % 251) as u8, (i % 256) as u8]).collect();
        assert_eq!(roundtrip(&ga, b" 8AG", 64, 48), ga);
        let few: Vec<u8> = (0..40 * 30).flat_map(|i| [[1, 2, 3, 255], [9, 8, 7, 128]][i % 2]).collect();
        assert_eq!(roundtrip(&few, b"BGRA", 40, 30), few);
    }

    #[test]
    fn colour_images_lose_at_most_the_chroma_low_bit() {
        let px: Vec<u8> =
            (0..97 * 61).flat_map(|i| [(i * 13 % 256) as u8, (i * 5 % 256) as u8, (i % 256) as u8, 255]).collect();
        let back = roundtrip(&px, b"BGRA", 97, 61);
        assert!(px.iter().zip(&back).all(|(a, b)| (*a as i32 - *b as i32).abs() <= 2));
    }

    #[test]
    fn qsort_sorts() {
        let mut v: Vec<u32> = (0..300u32).map(|i| i.wrapping_mul(2654435761) % 1000).collect();
        qsort(&mut v, |a, b| (a > b) as i32 - (a < b) as i32);
        assert!(v.windows(2).all(|w| w[0] <= w[1]));
        let mut h: Vec<u32> = (0..100u32).rev().collect();
        heapsort(&mut h, |a, b| (a > b) as i32 - (a < b) as i32);
        assert!(h.windows(2).all(|w| w[0] <= w[1]));
    }
}
