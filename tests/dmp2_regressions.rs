//! Regression tests for DMP2 (deepmap2) compression selection.

use accord::{car::compress_data, deepmap2};

fn pixel_data(w: u32, h: u32, bpp: u32) -> Vec<u8> {
    let colors: [u8; 4] = [0xAA, 0xBB, 0xCC, 0xFF];
    (0..(w * h) as usize)
        .flat_map(|_| colors[..bpp as usize].iter().copied())
        .collect()
}

#[test]
fn dmp2_used_for_atlas_at_11_0() {
    let data = pixel_data(64, 64, 4);
    let out = compress_data(&data, b"BGRA", 64, 64, "11.0", "macosx", true, true);
    assert_eq!(&out[..4], b"MLEC");
    let ver = u32::from_le_bytes(out[4..8].try_into().unwrap());
    let comp = u32::from_le_bytes(out[8..12].try_into().unwrap());
    assert_eq!(comp, 11, "expected DMP2");
    assert_eq!(ver, 2, "expected CELM ver=2 for opaque DMP2");
}

#[test]
fn lzfse_used_for_standalone_at_11_0() {
    let data = pixel_data(64, 64, 4);
    let out = compress_data(&data, b"BGRA", 64, 64, "11.0", "macosx", false, false);
    let comp = u32::from_le_bytes(out[8..12].try_into().unwrap());
    assert_ne!(comp, 11, "standalone images should not use DMP2");
}

#[test]
fn no_dmp2_at_10_15() {
    let data = pixel_data(64, 64, 4);
    let out = compress_data(&data, b"BGRA", 64, 64, "10.15", "macosx", true, false);
    let comp = u32::from_le_bytes(out[8..12].try_into().unwrap());
    assert_ne!(comp, 11, "DMP2 should not be used for target < 11.0");
}

#[test]
fn no_dmp2_at_10_11() {
    let data = pixel_data(64, 64, 4);
    let out = compress_data(&data, b"BGRA", 64, 64, "10.11", "macosx", true, false);
    let comp = u32::from_le_bytes(out[8..12].try_into().unwrap());
    assert_ne!(comp, 11);
}

#[test]
fn dmp2_for_ga8_atlas() {
    let data = pixel_data(64, 64, 2);
    let out = compress_data(&data, b" 8AG", 64, 64, "11.0", "macosx", true, false);
    let comp = u32::from_le_bytes(out[8..12].try_into().unwrap());
    assert_eq!(comp, 11, "GA8 atlas should use DMP2 at 11.0+");
}

#[test]
fn small_data_skips_dmp2() {
    let data = vec![0u8; 256];
    let out = compress_data(&data, b"BGRA", 8, 8, "11.0", "macosx", true, false);
    let comp = u32::from_le_bytes(out[8..12].try_into().unwrap());
    assert_ne!(comp, 11, "data <= 256 bytes should skip DMP2");
}

mod common;

/// Re-encoding the pixels of every 8-bit deepmap2 payload in the corpus gives Apple's bytes; tvOS
/// ScummVM's catalog comes from an older encoder. Float payloads can't round-trip this way,
/// since decoding rounds their 8-bit alpha to f16.
#[test]
fn reencoding_apple_payloads_reproduces_them() {
    let mut checked = 0;
    for s in common::samples().into_iter().filter(|s| s.name != "tvos/scummvm") {
        let path = common::root().join("samples").join(&s.name).join("Assets.car");
        let data = common::read(&path);
        let bom = accord::car_read::Bom::parse(&data).unwrap();
        for (_, csi) in bom.tree("RENDITIONS").unwrap() {
            let le = |o: usize| u32::from_le_bytes(csi[o..o + 4].try_into().unwrap());
            let p = 184 + le(168) as usize;
            if csi.len() < p + 16 || &csi[p..p + 4] != b"MLEC" || le(p + 8) != 11 {
                continue;
            }
            let pf: &[u8] = match &csi[24..28] {
                b"BGRA" => b"BGRA",
                b" 8AG" => b" 8AG",
                _ => continue,
            };
            let (w, h, celm) = (le(12), le(16), &csi[p..p + le(180) as usize]);
            let mut rows = Vec::new();
            if &celm[16..20] == b"KCBC" {
                let mut q = 16;
                for _ in 0..le(p + 12) {
                    let n = u32::from_le_bytes(celm[q + 12..q + 16].try_into().unwrap());
                    let len = u32::from_le_bytes(celm[q + 16..q + 20].try_into().unwrap()) as usize;
                    rows.extend(deepmap2::decode_chunk(&celm[q + 20..q + 20 + len], w, n).unwrap());
                    q += 20 + len;
                }
            } else {
                rows = deepmap2::decode_chunk(&celm[16..], w, h).unwrap();
            }
            let stride = rows.len() / h as usize;
            let ours = deepmap2::celm(&rows, stride, pf, w, h, le(p + 4) & 2 != 0).unwrap();
            assert!(ours == celm, "{}: {w}x{h} payload differs", s.name);
            checked += 1;
        }
    }
    assert!(checked >= 60, "only {checked} payloads checked");
}
