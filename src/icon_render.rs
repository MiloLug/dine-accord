//! Icon Composer-style compositing with tiny-skia: a `.icon`'s pre-rendered sizes are the layers
//! over the fill, clipped to the squircle. Apple's glass, specular and shadow are approximated.

use resvg::tiny_skia::{
    self, Color, FillRule, GradientStop, LinearGradient, Mask, Paint, PathBuilder, Pixmap,
    PixmapPaint, Point, Rect, SpreadMode, Transform,
};

/// How an icon fills its canvas: macOS insets a squircle with a margin; iOS pre-renders fill
/// the whole square and the system applies the mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Squircle,
    FullBleed,
}

impl Shape {
    fn margin_ratio(self) -> f64 {
        match self {
            Shape::Squircle => MARGIN_RATIO,
            Shape::FullBleed => 0.0,
        }
    }

    /// The fraction of the canvas edge the 1024-point layer canvas spans: the squircle's content
    /// box (824/1024, measured on tagspaces' positioned layer) or the whole square.
    pub fn content_scale(self) -> f32 {
        match self {
            Shape::Squircle => 824.0 / 1024.0,
            Shape::FullBleed => 1.0,
        }
    }
}

/// A two-stop linear gradient background. RGB components are device-RGB 0..1;
/// `start`/`stop` are normalized icon coordinates `[x, y]`, y = 1 at the top
/// edge (icon.json `orientation`).
pub struct GradientFill {
    pub start_rgb: [f64; 3],
    pub stop_rgb: [f64; 3],
    pub start: [f32; 2],
    pub stop: [f32; 2],
    pub shape: Shape,
}

impl GradientFill {
    /// The background colour (device-RGB 0..1) at canvas pixel `(x, y)` of a `pixel_size²`
    /// rendition, which a frosted-glass layer tints.
    pub fn sample(&self, x: u32, y: u32, pixel_size: u32) -> [f64; 3] {
        let margin = pixel_size as f64 * self.shape.margin_ratio();
        let content = (pixel_size as f64 - 2.0 * margin).max(1.0);
        let nx = (x as f64 + 0.5 - margin) / content;
        // Gradient y runs bottom-up (y = 1 is the top edge, as `composite_icon` draws it).
        let ny = 1.0 - (y as f64 + 0.5 - margin) / content;
        let (sx, sy) = (self.start[0] as f64, self.start[1] as f64);
        let (ex, ey) = (self.stop[0] as f64, self.stop[1] as f64);
        let (dx, dy) = (ex - sx, ey - sy);
        let len2 = dx * dx + dy * dy;
        let t = if len2 <= 0.0 {
            0.0
        } else {
            (((nx - sx) * dx + (ny - sy) * dy) / len2).clamp(0.0, 1.0)
        };
        [
            self.start_rgb[0] + (self.stop_rgb[0] - self.start_rgb[0]) * t,
            self.start_rgb[1] + (self.stop_rgb[1] - self.start_rgb[1]) * t,
            self.start_rgb[2] + (self.stop_rgb[2] - self.start_rgb[2]) * t,
        ]
    }
}

/// A drop shadow cast by the icon squircle. Drawn before the icon is clipped,
/// so it bleeds into the surrounding margin. See `icon_effects::shadow_geometry`
/// for the measured defaults.
pub struct ShadowParams {
    /// Straight (non-premultiplied) device-RGB colour + alpha.
    pub color: [f64; 4],
    /// Blur in pixels, in CoreGraphics' shadow convention (about two Gaussian sigmas).
    pub blur: f64,
    /// Offset in pixels `(x, y)`; positive `y` is downward on screen.
    pub offset: [f64; 2],
}

/// Squircle inset as a fraction of the canvas edge, measured from Apple's 1024 px output.
const MARGIN_RATIO: f64 = 100.0 / 1024.0;

/// The icon shape is a superellipse |x/a|ⁿ + |y/a|ⁿ = 1: n = 5 fits scrumdinger's 1024 px alpha
/// mask to ≈2 px, the best circular-arc rounded rect only to ≈7 px.
const SQUIRCLE_N: f64 = 5.0;

/// The icon-shape squircle as a polygon inscribed in the square at `origin` with edge `edge`,
/// sampled densely enough to be sub-pixel at the rendition size.
fn squircle_path(origin: f64, edge: f64) -> Option<tiny_skia::Path> {
    let a = edge / 2.0;
    let c = origin + a;
    // Curvature concentrates near the corners; sample by edge length so even
    // small renditions stay smooth there.
    let steps = (edge as usize).clamp(256, 2048);
    let e = 2.0 / SQUIRCLE_N;
    let mut pb = PathBuilder::new();
    for i in 0..steps {
        let t = (i as f64) / (steps as f64) * std::f64::consts::TAU;
        let (st, ct) = t.sin_cos();
        let x = (c + a * ct.signum() * ct.abs().powf(e)) as f32;
        let y = (c + a * st.signum() * st.abs().powf(e)) as f32;
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.close();
    pb.finish()
}

fn bgra_swap(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

/// The squircle's drop shadow: its silhouette blurred, offset and tinted, as premultiplied RGBA.
fn shadow_pixmap(size: u32, path: &tiny_skia::Path, sh: &ShadowParams) -> Option<Pixmap> {
    let mut sil = Pixmap::new(size, size)?;
    let mut white = Paint::default();
    white.set_color_rgba8(255, 255, 255, 255);
    white.anti_alias = true;
    let t = Transform::from_translate(sh.offset[0] as f32, sh.offset[1] as f32);
    sil.fill_path(path, &white, FillRule::Winding, t, None);
    let mut rgba = sil.take();
    crate::icon_bundle::blur_rgba_premul(&mut rgba, size as usize, (sh.blur / 2.0) as f32);
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as f64 / 255.0 * sh.color[3];
        for c in 0..3 {
            px[c] = (sh.color[c] * a * 255.0).round() as u8;
        }
        px[3] = (a * 255.0).round() as u8;
    }
    Pixmap::from_vec(rgba, tiny_skia::IntSize::from_wh(size, size)?)
}

/// Composite a sized rendition: the gradient background and the supplied layer
/// (canvas-sized premultiplied-first BGRA), both clipped to the icon squircle,
/// optionally preceded by a drop shadow. Returns premultiplied-first BGRA of
/// `pixel_size²`, or `None` for a layer of the wrong size.
pub fn composite_icon(
    pixel_size: u32,
    gradient: &GradientFill,
    layer_bgra: &[u8],
    shadow: Option<&ShadowParams>,
) -> Option<Vec<u8>> {
    let size = pixel_size as usize;
    if layer_bgra.len() != size * size * 4 {
        return None;
    }
    let squircle = gradient.shape == Shape::Squircle;
    let margin = pixel_size as f64 * gradient.shape.margin_ratio();
    let content = pixel_size as f64 - 2.0 * margin;
    let path = squircle_path(margin, content)?;
    let mut canvas = Pixmap::new(pixel_size, pixel_size)?;

    // Like actool: the shadow comes from filling the squircle white with a shadow set, so the
    // white shows through the gradient's anti-aliased edge.
    if let Some(sh) = shadow.filter(|_| squircle) {
        let shadow = shadow_pixmap(pixel_size, &path, sh)?;
        canvas.draw_pixmap(0, 0, shadow.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        let mut white = Paint::default();
        white.set_color_rgba8(255, 255, 255, 255);
        white.anti_alias = true;
        canvas.fill_path(&path, &white, FillRule::Winding, Transform::identity(), None);
    }

    let mut clip = Mask::new(pixel_size, pixel_size)?;
    clip.fill_path(&path, FillRule::Winding, true, Transform::identity());
    let clip = squircle.then_some(&clip);

    // icon.json orientation has y = 1 at the top edge; pixmap rows run top-down.
    let to_px = |p: [f32; 2]| {
        Point::from_xy(
            (margin + p[0] as f64 * content) as f32,
            (margin + (1.0 - p[1] as f64) * content) as f32,
        )
    };
    let rgb = |c: [f64; 3]| Color::from_rgba(c[0] as f32, c[1] as f32, c[2] as f32, 1.0).unwrap();
    let mut paint = Paint::default();
    paint.shader = LinearGradient::new(
        to_px(gradient.start),
        to_px(gradient.stop),
        vec![GradientStop::new(0.0, rgb(gradient.start_rgb)), GradientStop::new(1.0, rgb(gradient.stop_rgb))],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap_or(tiny_skia::Shader::SolidColor(rgb(gradient.start_rgb)));
    let full = Rect::from_xywh(0.0, 0.0, pixel_size as f32, pixel_size as f32)?;
    canvas.fill_rect(full, &paint, Transform::identity(), clip);

    let mut layer = layer_bgra.to_vec();
    bgra_swap(&mut layer);
    let layer = Pixmap::from_vec(layer, tiny_skia::IntSize::from_wh(pixel_size, pixel_size)?)?;
    canvas.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), clip);

    let mut data = canvas.take();
    bgra_swap(&mut data);
    if squircle {
        apply_icon_lighting(&mut data, size);
    }
    Some(data)
}

#[inline]
fn srgb_to_lin(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
fn lin_to_srgb(c: f64) -> f64 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Approximate signed distance (px) to the icon superellipse, negative inside: the first-order
/// `(g−1)/|∇g|` (no closed form exists), exact on the boundary. Must match `squircle_path`, or
/// the highlight lands off the corners.
#[inline]
fn squircle_sdf(px: f64, py: f64, center: f64, half: f64, n: f64) -> f64 {
    let nx = ((px - center).abs() / half).max(1e-9);
    let ny = ((py - center).abs() / half).max(1e-9);
    let denom = nx.powf(n) + ny.powf(n);
    let g = denom.powf(1.0 / n);
    // |∇g| in pixel units (chain rule through the /half normalization).
    let pow = denom.powf(1.0 / n - 1.0);
    let dgx = pow * nx.powf(n - 1.0) / half;
    let dgy = pow * ny.powf(n - 1.0) / half;
    let gl = (dgx * dgx + dgy * dgy).sqrt().max(1e-12);
    (g - 1.0) / gl
}

/// Apple's icon-frame lighting: white light added in linear space along the squircle's inner
/// edge, brightest top-left, fading linearly over ~16 px (`tools/probe_icon_lighting.py`:
/// L0 ≈ 0.34 top/left, ≈ 0.23 bottom/right). Only the opaque interior, where RGB is straight.
fn apply_icon_lighting(data: &mut [u8], size: usize) {
    let sz = size as f64;
    let center = sz / 2.0;
    let half = (sz - 2.0 * sz * MARGIN_RATIO) / 2.0;
    let depth = 16.5 * sz / 1024.0;
    if depth < 1.0 {
        return;
    }
    const L0_BASE: f64 = 0.286;
    const L0_DIR: f64 = 0.083;
    let sqrt2 = std::f64::consts::SQRT_2;
    for y in 0..size {
        for x in 0..size {
            let i = (y * size + x) * 4;
            if data[i + 3] < 250 {
                continue;
            }
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let dist = -squircle_sdf(px, py, center, half, SQUIRCLE_N);
            if dist < 0.0 || dist >= depth {
                continue;
            }
            // Outward normal via central difference of the SDF.
            let gx = squircle_sdf(px + 1.0, py, center, half, SQUIRCLE_N)
                - squircle_sdf(px - 1.0, py, center, half, SQUIRCLE_N);
            let gy = squircle_sdf(px, py + 1.0, center, half, SQUIRCLE_N)
                - squircle_sdf(px, py - 1.0, center, half, SQUIRCLE_N);
            let gl = (gx * gx + gy * gy).sqrt().max(1e-6);
            let dir = (-gx / gl - gy / gl) / sqrt2; // top-left → +1
            let l0 = L0_BASE + L0_DIR * dir;
            let dl = l0 * (1.0 - dist / depth);
            for c in 0..3 {
                let lin = srgb_to_lin(data[i + c] as f64 / 255.0) + dl;
                data[i + c] = (lin_to_srgb(lin.min(1.0)) * 255.0).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_masks_to_squircle() {
        let size = 256u32;
        // Fully transparent layer → output is just the gradient, squircle-clipped.
        let layer = vec![0u8; (size * size * 4) as usize];
        let fill = GradientFill {
            start_rgb: [0.5, 0.5, 0.5],
            stop_rgb: [0.9, 0.9, 0.9],
            start: [0.5, 0.0],
            stop: [0.5, 1.0],
            shape: Shape::Squircle,
        };
        let out = composite_icon(size, &fill, &layer, None).unwrap();
        assert_eq!(out.len(), (size * size * 4) as usize);
        let alpha = |x: u32, y: u32| out[((y * size + x) * 4 + 3) as usize];
        assert_eq!(alpha(2, 2), 0, "corner must be transparent");
        assert_eq!(alpha(size / 2, size / 2), 255, "center must be opaque");
    }

    #[test]
    fn icon_lighting_brightens_inner_edge() {
        let size = 256u32;
        let layer = vec![0u8; (size * size * 4) as usize];
        let fill = GradientFill {
            start_rgb: [0.55, 0.55, 0.55],
            stop_rgb: [0.55, 0.55, 0.55],
            start: [0.5, 0.0],
            stop: [0.5, 1.0],
            shape: Shape::Squircle,
        };
        let out = composite_icon(size, &fill, &layer, None).unwrap();
        let g = |x: u32, y: u32| out[((y * size + x) * 4 + 1) as usize] as i32;
        let margin = (size as f64 * MARGIN_RATIO).round() as u32;
        let centre = g(size / 2, size / 2);
        let top_edge = g(size / 2, margin + 1);
        let bot_edge = g(size / 2, size - margin - 2);
        assert!(top_edge > centre + 5, "top inner edge should brighten ({top_edge} vs {centre})");
        assert!(bot_edge > centre + 3, "bottom inner edge should brighten ({bot_edge} vs {centre})");
        assert!(top_edge >= bot_edge, "top should be ≥ bottom (light from top)");
    }
}
