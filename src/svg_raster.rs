//! SVG rasterization with resvg. actool uses CoreSVG, so rasters are close but never
//! byte-identical to Apple's.

use anyhow::{anyhow, Result};
use regex::Regex;
use resvg::{tiny_skia, usvg};
use std::sync::{Arc, OnceLock};

/// Extract (width, height) from SVG root element attributes. Falls back
/// to the viewBox when width/height aren't specified.
pub fn parse_svg_dimensions(svg: &[u8]) -> (u32, u32) {
    let slice = &svg[..svg.len().min(2048)];
    let text = String::from_utf8_lossy(slice);

    let w_re = Regex::new(r#"width="(\d+(?:\.\d+)?)""#).unwrap();
    let h_re = Regex::new(r#"height="(\d+(?:\.\d+)?)""#).unwrap();
    if let (Some(w), Some(h)) = (w_re.captures(&text), h_re.captures(&text)) {
        let wv: f64 = w[1].parse().unwrap_or(0.0);
        let hv: f64 = h[1].parse().unwrap_or(0.0);
        return (wv as u32, hv as u32);
    }

    let vb_re = Regex::new(
        r#"viewBox="[\d.]+\s+[\d.]+\s+([\d.]+)\s+([\d.]+)""#,
    )
    .unwrap();
    if let Some(cap) = vb_re.captures(&text) {
        let wv: f64 = cap[1].parse().unwrap_or(0.0);
        let hv: f64 = cap[2].parse().unwrap_or(0.0);
        return (wv as u32, hv as u32);
    }
    (0, 0)
}

fn options() -> &'static usvg::Options<'static> {
    static OPTS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        usvg::Options { fontdb: Arc::new(db), ..Default::default() }
    })
}

/// Rasterize SVG data scaled to fill `(width*scale) x (height*scale)`, as premultiplied BGRA
/// (CoreGraphics' little-endian premultiplied-first layout).
pub fn rasterize_svg(svg_data: &[u8], width: u32, height: u32, scale: u32) -> Result<Vec<u8>> {
    let tree = usvg::Tree::from_data(svg_data, options())?;
    let (pw, ph) = (width * scale, height * scale);
    let mut pixmap =
        tiny_skia::Pixmap::new(pw, ph).ok_or_else(|| anyhow!("invalid raster size {pw}x{ph}"))?;
    let size = tree.size();
    let t = tiny_skia::Transform::from_scale(pw as f32 / size.width(), ph as f32 / size.height());
    resvg::render(&tree, t, &mut pixmap.as_mut());
    let mut data = pixmap.take();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_width_height() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="48" viewBox="0 0 64 48"></svg>"#;
        assert_eq!(parse_svg_dimensions(svg), (64, 48));
    }

    #[test]
    fn parse_from_viewbox() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"></svg>"#;
        assert_eq!(parse_svg_dimensions(svg), (32, 32));
    }

    #[test]
    fn parse_fractional_width() {
        let svg = br#"<svg width="10.5" height="20.75" viewBox="0 0 10 20"></svg>"#;
        assert_eq!(parse_svg_dimensions(svg), (10, 20));
    }

    #[test]
    fn parse_missing() {
        let svg = br#"<svg></svg>"#;
        assert_eq!(parse_svg_dimensions(svg), (0, 0));
    }

    #[test]
    fn rasterize_simple_rect() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8" viewBox="0 0 8 8"><rect width="8" height="8" fill="#FF0000"/></svg>"##;
        let data = rasterize_svg(svg, 8, 8, 1).unwrap();
        assert_eq!(data.len(), 8 * 8 * 4);
        let center = 4 * 4 + 4 * 4 * 8;
        assert_eq!(&data[center..center + 4], &[0, 0, 0xFF, 0xFF], "BGRA red");
    }

    #[test]
    fn rasterize_scales_to_requested_size() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect x="2" width="2" height="4" fill="#00FF00"/></svg>"##;
        let data = rasterize_svg(svg, 8, 8, 2).unwrap();
        assert_eq!(data.len(), 16 * 16 * 4);
        let px = |x: usize, y: usize| &data[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4];
        assert_eq!(px(2, 8)[3], 0, "left half transparent");
        assert_eq!(px(12, 8), &[0, 0xFF, 0, 0xFF], "right half green");
    }
}
