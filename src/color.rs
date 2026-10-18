//! Source images in sRGB, plus the wide-gamut copy actool keeps for a colour image whose ICC
//! profile is wide and whose pixels leave sRGB (`docs/format.md`, key attribute 24).

use anyhow::Result;
use half::f16;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage};
use moxcms::{ColorProfile, DataColorSpace, ToneCurveEvaluator, ToneReprCurve, Xyzd};
use std::path::Path;

pub struct Source {
    /// The pixels in sRGB.
    pub image: DynamicImage,
    /// Premultiplied RGBA in extended sRGB, a little-endian f16 per channel.
    pub wide_gamut: Option<Vec<u8>>,
}

pub fn open(path: &Path) -> Result<Source> {
    open_fitting(path, u32::MAX)
}

/// `open`, the image first shrunk to fit `max_px`: actool shrinks `.icon` layers before its gamut
/// test, which then sees the shrunk pixels.
pub fn open_fitting(path: &Path, max_px: u32) -> Result<Source> {
    let mut decoder = ImageReader::open(path)?.with_guessed_format()?.into_decoder()?;
    let icc = decoder.icc_profile()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    if image.width().max(image.height()) > max_px {
        image = image.resize(max_px, max_px, image::imageops::FilterType::Lanczos3);
    }
    // Gray images keep their values: an RGB profile on a gray PNG (wireshark's GEARS) is ignored.
    let colour = !matches!(image, DynamicImage::ImageLuma8(_) | DynamicImage::ImageLumaA8(_)
        | DynamicImage::ImageLuma16(_) | DynamicImage::ImageLumaA16(_));
    let profile = icc.and_then(|icc| ColorProfile::new_from_slice(&icc).ok());
    match profile.and_then(|p| Shaper::new(&p).map(|s| (s, is_wide_gamut(&p)))) {
        Some((shaper, wide)) if colour => Ok(shaper.convert(&image, wide)),
        _ => Ok(Source { image, wide_gamut: None }),
    }
}

/// actool's wide-gamut profile test: the xy triangle of the colorants is larger than 0.13447.
fn is_wide_gamut(p: &ColorProfile) -> bool {
    let xy = |c: Xyzd| {
        let s = c.x + c.y + c.z;
        (s != 0.0).then(|| (c.x / s, c.y / s))
    };
    let (Some(r), Some(g), Some(b)) = (xy(p.red_colorant), xy(p.green_colorant), xy(p.blue_colorant))
    else {
        return false;
    };
    let side = |a: (f64, f64), b: (f64, f64)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let (rg, rb, gb) = (side(r, g), side(r, b), side(g, b));
    let s = (rg + rb + gb) / 2.0;
    (s * (s - rg) * (s - rb) * (s - gb)).sqrt() > 0.13447
}

/// A matrix/TRC RGB profile, as a conversion to sRGB.
struct Shaper {
    trc: [Box<dyn ToneCurveEvaluator + Send + Sync>; 3],
    to_srgb: [[f64; 3]; 3],
}

impl Shaper {
    /// None for profiles that are not matrix/TRC RGB, and for sRGB itself (no conversion).
    fn new(p: &ColorProfile) -> Option<Shaper> {
        if p.color_space != DataColorSpace::Rgb {
            return None;
        }
        let curve = |t: &Option<ToneReprCurve>| t.as_ref()?.make_linear_evaluator().ok();
        let trc = [curve(&p.red_trc)?, curve(&p.green_trc)?, curve(&p.blue_trc)?];
        let srgb = ColorProfile::new_srgb();
        let to_srgb = p.transform_matrix(&srgb).v;
        let identity = (0..3).all(|i| (0..3).all(|j| (to_srgb[i][j] - (i == j) as u8 as f64).abs() < 2e-3));
        let srgb_trc = (0..=32).all(|i| {
            let x = i as f32 / 32.0;
            trc.iter().all(|t| (t.evaluate_value(x) - srgb_decode(x)).abs() < 2e-3)
        });
        (!(identity && srgb_trc)).then_some(Shaper { trc, to_srgb })
    }

    /// The image in 8-bit sRGB, and its f16 copy when `wide` and a pixel leaves sRGB.
    fn convert(&self, image: &DynamicImage, wide: bool) -> Source {
        let rgba = image.to_rgba32f();
        let (w, h) = rgba.dimensions();
        let mut out = Vec::with_capacity(rgba.len());
        let mut deep = Vec::with_capacity(if wide { rgba.len() * 2 } else { 0 });
        let mut outside = false;
        for px in rgba.pixels() {
            let lin: [f64; 3] = std::array::from_fn(|c| self.trc[c].evaluate_value(px[c]) as f64);
            let enc: [f32; 3] = std::array::from_fn(|c| {
                srgb_encode(self.to_srgb[c].iter().zip(lin).map(|(m, l)| m * l).sum::<f64>() as f32)
            });
            let a = px[3];
            out.extend(enc.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
            out.push((a.clamp(0.0, 1.0) * 255.0).round() as u8);
            if wide {
                for v in enc {
                    let v = f16::from_f32(v * a);
                    // actool's out-of-sRGB test, on premultiplied extended-sRGB f16 values.
                    outside |= !(-0.025..=1.003).contains(&v.to_f64());
                    deep.extend(v.to_le_bytes());
                }
                deep.extend(f16::from_f32(a).to_le_bytes());
            }
        }
        let image = DynamicImage::ImageRgba8(RgbaImage::from_raw(w, h, out).expect("rgba8 size"));
        Source { image, wide_gamut: outside.then_some(deep) }
    }
}

fn srgb_decode(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// The sRGB curve, mirrored below zero as in extended sRGB.
fn srgb_encode(l: f32) -> f32 {
    let a = l.abs();
    let e = if a <= 0.0031308 { a * 12.92 } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 };
    e.copysign(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_gamut_threshold_separates_p3_from_srgb() {
        assert!(is_wide_gamut(&ColorProfile::new_display_p3()));
        assert!(!is_wide_gamut(&ColorProfile::new_srgb()));
    }

    #[test]
    fn srgb_profile_needs_no_conversion() {
        assert!(Shaper::new(&ColorProfile::new_srgb()).is_none());
        assert!(Shaper::new(&ColorProfile::new_display_p3()).is_some());
    }

    #[test]
    fn extended_encoding_is_odd() {
        assert_eq!(srgb_encode(-0.5), -srgb_encode(0.5));
        assert!((srgb_encode(srgb_decode(0.7)) - 0.7).abs() < 1e-5);
    }
}
