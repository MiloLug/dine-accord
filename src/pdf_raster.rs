//! PDF rasterization with hayro. actool uses CoreGraphics, so rasters are close but not
//! byte-identical to Apple's.

use anyhow::{anyhow, Result};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::RenderSettings;
use std::sync::Arc;

pub struct RasterizedPdf {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub bgra: Vec<u8>,
}

/// Rasterize the first page of `pdf_data` at an integer scale factor, as premultiplied BGRA
/// rows. The pixel size is round(page size × scale) of the fractional page box: actool rounds
/// after scaling (iina's 86.25×67.5 page is 173×135 at @2x, not 172×136).
pub fn rasterize_pdf(pdf_data: &[u8], scale: u32) -> Result<RasterizedPdf> {
    let pdf = Pdf::new(Arc::new(pdf_data.to_vec())).map_err(|e| anyhow!("unreadable PDF: {e:?}"))?;
    let page = pdf.pages().first().ok_or_else(|| anyhow!("PDF has no pages"))?;
    let (w, h) = page.render_dimensions();
    let px_w = (w * scale as f32).round() as u16;
    let px_h = (h * scale as f32).round() as u16;
    if px_w == 0 || px_h == 0 {
        return Err(anyhow!("PDF page has zero dimensions"));
    }
    let settings = RenderSettings {
        x_scale: scale as f32,
        y_scale: scale as f32,
        width: Some(px_w),
        height: Some(px_h),
        ..Default::default()
    };
    let pixmap = hayro::render(page, &InterpreterSettings::default(), &settings);
    let bgra = pixmap.data_as_u8_slice().chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect();
    Ok(RasterizedPdf { width: px_w as u32, height: px_h as u32, scale, bgra })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 10×5 pt page filled red.
    const RED_PAGE: &[u8] = b"%PDF-1.4
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 10 5]/Contents 4 0 R>>endobj
4 0 obj<</Length 24>>stream
1 0 0 rg 0 0 10 5 re f
endstream endobj
trailer<</Root 1 0 R>>
%%EOF";

    #[test]
    fn rasterizes_at_scale() {
        let r = rasterize_pdf(RED_PAGE, 2).unwrap();
        assert_eq!((r.width, r.height), (20, 10));
        assert_eq!(&r.bgra[..4], &[0, 0, 255, 255], "BGRA red");
    }
}
