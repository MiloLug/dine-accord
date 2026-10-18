//! Icon Composer `.icon` bundles (icon.json + layer images) into catalog renditions.

use crate::car::{self, MultisizeImageEntry, Rendition};
use crate::catalog::load_image_as_bgra;
use crate::icon_json::{Fill, IconJson};
use crate::name_hash::hash_name;

static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
use anyhow::Result;
use image::imageops::FilterType;
use std::fs;
use std::path::{Path, PathBuf};

/// One facet entry in the FACETKEYS tree: (facet_name, element, part, identifier).
type FacetEntry = (String, u16, Option<u16>, u16);

/// Bundle stem, the prefix of every facet name (`<stem>_Assets/<image stem>`).
fn bundle_facet_prefix(icon_path: &Path) -> String {
    icon_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Assets")
        .to_string()
}

/// A group or layer-image name as a facet name component: actool keeps ASCII letters, digits,
/// space, `.`, `-` and `_`, and turns anything else into `_` (`01 · Rear flaps`, `Icon@1x`).
fn facet_component(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || " .-_".contains(c) { c } else { '_' }).collect()
}

/// UUID-shaped string stable per `inputs`, so recompiling yields identical bytes.
fn deterministic_uuid(inputs: &[&str]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h1 = std::collections::hash_map::DefaultHasher::new();
    let mut h2 = std::collections::hash_map::DefaultHasher::new();
    "uuid-lo".hash(&mut h1);
    "uuid-hi".hash(&mut h2);
    for s in inputs {
        s.hash(&mut h1);
        s.hash(&mut h2);
    }
    let lo = h1.finish();
    let hi = h2.finish();
    let bytes: [u8; 16] = {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&lo.to_be_bytes());
        out[8..].copy_from_slice(&hi.to_be_bytes());
        out
    };
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

/// Apple's pre-rendered icon name, `<stem>NxN_<appearance>_<UUID>-<pid>-<hex>.png` (feishin.icon:
/// "feishin16x16…"). CoreUI looks renditions up by attributes, so only the shape matters.
fn pre_rendered_name(
    icon_name: &str,
    point_size: u32,
    scale: u32,
    appearance_name: &str,
) -> String {
    let scale_label = format!("{scale}x");
    let pt_label = format!("{point_size}");
    let uuid = deterministic_uuid(&[icon_name, &pt_label, &scale_label, appearance_name]);
    let tail_uuid =
        deterministic_uuid(&[icon_name, &pt_label, &scale_label, "tail"]);
    let tail_hex: String = tail_uuid.chars().filter(|c| *c != '-').collect();
    let pid = &tail_hex[0..5];
    let hex = &tail_hex[5..21];
    format!(
        "{icon_name}{point_size}x{point_size}_{appearance_name}_{uuid}-{pid}-{hex}.png"
    )
}

/// (point size, scale) of the sized renditions Apple emits for a macOS `.icon`.
const MACOS_ICON_SIZES: &[(u32, u32)] = &[
    (16, 2),
    (32, 2),
    (64, 2),
    (128, 2),
    (256, 2),
    (512, 2),
    (1024, 1),
];

/// Point sizes Apple atlases into ZZZZPackedAsset; the rest are inline.
const ATLAS_POINT_SIZES: &[u32] = &[16, 32, 64];

fn icon_dim2(point_size: u32) -> u16 {
    match point_size {
        16 => 1,
        32 => 2,
        64 => 3,
        128 => 4,
        256 => 5,
        512 => 6,
        1024 => 7,
        _ => 0,
    }
}

pub fn is_icon_bundle(path: &Path) -> bool {
    // Extension only: a broken `.icon` must fail in `document`, not fall through to the
    // xcassets path.
    path.extension().and_then(|s| s.to_str()) == Some("icon")
}

/// What an `.icon` bundle contributes to the catalog; writes `<app-icon>.icns` into `output_dir`.
pub fn document(
    icon_path: &Path,
    output_dir: &Path,
    opts: &crate::compiler::Options,
) -> Result<crate::assemble::Document> {
    let (platform, min_deploy) = (opts.platform.as_str(), opts.min_deploy.as_str());
    let app_icon = opts.app_icon.as_deref();
    let bundle_stem = icon_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    // --app-icon controls the .icns filename and Info.plist value, but
    // Apple's actool always names the in-CAR facet after the bundle stem.
    let icon_name = app_icon.map(|s| s.to_string()).unwrap_or(bundle_stem.clone());

    let icon_json_path = icon_path.join("icon.json");
    let icon_json_text = fs::read_to_string(&icon_json_path).map_err(|e| {
        anyhow::anyhow!("could not read {}: {e}", icon_json_path.display())
    })?;
    let icon_json_value: serde_json::Value = serde_json::from_str(&icon_json_text)?;
    let parsed: IconJson = IconJson::parse(&icon_json_text)?;

    // Apple errors on `{}` but accepts `{"groups": []}`; the serde-default Vec can't tell
    // them apart.
    if icon_json_value.get("groups").is_none() {
        anyhow::bail!("icon.json missing required `groups` field");
    }

    validate_layer_image_refs(icon_path, &icon_json_value)?;

    let source_images = find_all_source_images(icon_path, &icon_json_value);
    if source_images.is_empty() {
        return Ok(crate::assemble::Document::default());
    }
    let facet_prefix = bundle_facet_prefix(icon_path);
    let layer_assets = collect_layer_assets(icon_path, &parsed, &facet_prefix);
    // Apple gives every group a facet, unnamed ones "Group"; repeated names get " 2", " 3", …
    // (ding_icon's unnamed groups, VLC's three "group"s).
    let group_facet_names: Vec<String> = {
        let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        parsed
            .groups
            .iter()
            .map(|g| {
                let base = facet_component(g.name.as_deref().unwrap_or("Group"));
                let n = seen.entry(base.clone()).or_default();
                *n += 1;
                match *n {
                    1 => format!("{facet_prefix}/{base}"),
                    n => format!("{facet_prefix}/{base} {n}"),
                }
            })
            .collect()
    };
    let (color_assets, gradient_assets) =
        fill_assets(&facet_prefix, parsed.fill.as_ref(), &parsed)
            .unwrap_or_else(|| (Vec::new(), Vec::new()));

    let solid_fill = parsed.fill_specializations.is_none()
        && matches!(&parsed.fill, Some(Fill::Structured(v)) if v.get("solid").is_some());
    let solid_fill_color = solid_fill.then(|| solid_fill_facet(&parsed, &color_assets)).flatten();
    if car::is_idiom_platform(platform) {
        return ios_document(
            icon_path,
            output_dir,
            opts,
            &parsed,
            &facet_prefix,
            icon_name == bundle_stem,
            &layer_assets,
            &group_facet_names,
            &color_assets,
            &gradient_assets,
            solid_fill_color.as_deref(),
        );
    }

    // The counter keeps concurrent compiles in one process (parallel tests) from sharing and
    // deleting each other's scratch dir.
    let tmp_seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmpdir = std::env::temp_dir().join(format!(
        "accord_icon_{}_{}",
        std::process::id(),
        tmp_seq
    ));
    fs::create_dir_all(&tmpdir)?;
    let primary_source = &source_images[0];
    let primary_is_svg = is_svg_path(primary_source);

    // (filepath, pixel_size, scale)
    let mut icon_images: Vec<(PathBuf, u32, u32)> = Vec::new();
    if primary_is_svg {
        let svg_data = fs::read(primary_source)?;
        let (sw, sh) = crate::svg_raster::parse_svg_dimensions(&svg_data);
        if sw == 0 || sh == 0 {
            anyhow::bail!(
                "could not determine intrinsic size of {}",
                primary_source.display()
            );
        }
        for (point_size, scale) in MACOS_ICON_SIZES {
            let pixel_size = point_size * scale;
            let bgra =
                crate::svg_raster::rasterize_svg(&svg_data, *point_size, *point_size, *scale)?;
            let mut rgba = bgra.clone();
            for px in rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            let img = image::RgbaImage::from_raw(pixel_size, pixel_size, rgba)
                .ok_or_else(|| anyhow::anyhow!("svg rasterization size mismatch"))?;
            let filename = format!("Icon{pixel_size}x{pixel_size}.png");
            let filepath = tmpdir.join(&filename);
            img.save(&filepath)?;
            icon_images.push((filepath, pixel_size, *scale));
        }
    } else {
        let src_img = crate::color::open(primary_source)?.image.to_rgba8();
        for (point_size, scale) in MACOS_ICON_SIZES {
            let pixel_size = point_size * scale;
            let resized = image::imageops::resize(
                &src_img,
                pixel_size,
                pixel_size,
                FilterType::Lanczos3,
            );
            let filename = format!("Icon{pixel_size}x{pixel_size}.png");
            let filepath = tmpdir.join(&filename);
            resized.save(&filepath)?;
            icon_images.push((filepath, pixel_size, *scale));
        }
    }

    // One layer stack per entry of `icon_images`. Frosted glass is derived from the background,
    // so the stack renderer gets the fill the compositor draws under it.
    let shape = crate::icon_render::Shape::Squircle;
    let light_layers = collect_stack_layers(icon_path, &parsed, crate::icon_effects::Appearance::Light, shape);
    let (light_fill, _) = resolve_background_fills(&parsed, &gradient_assets, &color_assets, shape);
    let mut light_stacks: Vec<Vec<u8>> = Vec::with_capacity(MACOS_ICON_SIZES.len());
    for (point_size, scale) in MACOS_ICON_SIZES {
        light_stacks.push(render_layer_stack(&light_layers, point_size * scale, light_fill.as_ref())?);
    }

    let mut doc = build_icon_car(
        &facet_prefix,
        &icon_images,
        &layer_assets,
        &group_facet_names,
        &color_assets,
        &gradient_assets,
        &parsed.groups,
        &light_stacks,
        light_fill.as_ref(),
        solid_fill_color.as_deref(),
        &[1, 8, 10],
        platform,
        min_deploy,
    )?;
    doc.appearances = car::ICON_APPEARANCES.iter().map(|(n, v)| (n.to_string(), *v)).collect();

    // Apple emits the `.icns` and plist keys only when `--app-icon` equals the bundle stem
    // (case-sensitive); `supported-platforms` and `--standalone-icon-behavior` don't matter.
    if icon_name == bundle_stem {
        let icns_path = output_dir.join(format!("{icon_name}.icns"));
        crate::icns::create_icns(&icon_images, &icns_path)?;
        if icns_path.exists() {
            doc.files.push(fs::canonicalize(&icns_path).unwrap_or(icns_path));
        }
        doc.plist.insert("CFBundleIconFile".into(), icon_name.clone().into());
        doc.plist.insert("CFBundleIconName".into(), icon_name.into());
    }
    let _ = fs::remove_dir_all(&tmpdir);
    Ok(doc)
}

/// iOS keys icon stacks by these appearances (UIAppearanceLight takes the place of Aqua).
const IOS_ICON_APPEARANCES: &[(&str, u16)] = &[
    ("UIAppearanceAny", 0),
    ("UIAppearanceDark", 1),
    ("UIAppearanceLight", 4),
    ("ISAppearanceTintable", 10),
];

/// The iOS form of an `.icon`: the same stacks, plus one full-bleed 1024 pre-render per target
/// device for the default, dark and tinted appearances (iOS applies the mask itself), and the
/// loose home-screen PNGs instead of an `.icns`.
#[allow(clippy::too_many_arguments)]
fn ios_document(
    icon_path: &Path,
    output_dir: &Path,
    opts: &crate::compiler::Options,
    parsed: &IconJson,
    facet_prefix: &str,
    is_app_icon: bool,
    layer_assets: &[LayerAsset],
    group_facet_names: &[String],
    color_assets: &[ColorAsset],
    gradient_assets: &[GradientAsset],
    solid_fill_color: Option<&str>,
) -> Result<crate::assemble::Document> {
    use crate::icon_effects::Appearance;
    use crate::icon_render::Shape;
    let mut doc = build_icon_car(
        facet_prefix,
        &[],
        layer_assets,
        group_facet_names,
        color_assets,
        gradient_assets,
        &parsed.groups,
        &[],
        None,
        solid_fill_color,
        &[1, 4, 10],
        &opts.platform,
        &opts.min_deploy,
    )?;
    doc.appearances = IOS_ICON_APPEARANCES.iter().map(|(n, v)| (n.to_string(), *v)).collect();

    let ident = hash_name(facet_prefix);
    let idioms: Vec<u16> = if opts.target_devices.is_empty() {
        vec![0]
    } else {
        opts.target_devices.iter().map(|d| car::idiom_value(d)).collect()
    };
    let (light, dark) = resolve_background_fills(parsed, gradient_assets, color_assets, Shape::FullBleed);
    let black = flat_fill([0.0; 3], Shape::FullBleed);
    let looks = [
        (0, "UIAppearanceAny", Appearance::Light, light.as_ref()),
        (1, "UIAppearanceDark", Appearance::Dark, dark.as_ref().or(light.as_ref())),
        (10, "ISAppearanceTintable", Appearance::Tinted, Some(&black)),
    ];
    let mut default_render = Vec::new();
    for (appearance, appearance_name, look, fill) in looks {
        let layers = collect_stack_layers(icon_path, parsed, look, Shape::FullBleed);
        let stack = render_layer_stack(&layers, 1024, fill)?;
        let pixels = match fill {
            Some(f) => crate::icon_render::composite_icon(1024, f, &stack, None).unwrap_or(stack),
            None => stack,
        };
        if appearance == 0 {
            default_render = pixels.clone();
        }
        let (pixel_data, width, height, pixel_format) =
            crate::catalog::bgra_to_best_format(pixels, 1024, 1024, false);
        let name = pre_rendered_name(facet_prefix, 1024, 1, appearance_name);
        for &idiom in &idioms {
            doc.renditions.push(Rendition {
                name: name.clone(),
                identifier: ident,
                part: car::PART_ICON,
                width,
                height,
                pixel_data: pixel_data.clone(),
                pixel_format,
                dim2: 1,
                appearance,
                idiom,
                colorspace_id: car::colorspace_for_pixel_format(&pixel_format),
                template_rendering_intent: 0,
                ..Rendition::default()
            });
        }
    }
    for &idiom in &idioms {
        let entries = [MultisizeImageEntry { width: 1024, height: 1024, index: 1 }];
        let mut multisize = car::build_multisize_rendition(facet_prefix, ident, &entries);
        multisize.idiom = idiom;
        doc.renditions.push(multisize);
    }

    if is_app_icon {
        let mut loose = Vec::new();
        for (device, point, suffix) in [("iphone", 60, ""), ("ipad", 76, "~ipad")] {
            if !opts.target_devices.iter().any(|d| d == device) {
                continue;
            }
            let path = output_dir.join(format!("{facet_prefix}{point}x{point}@2x{suffix}.png"));
            save_resized(&default_render, 1024, point * 2, &path)?;
            doc.files.push(fs::canonicalize(&path).unwrap_or(path));
            loose.push(device);
        }
        let name = opts.app_icon.as_deref().unwrap_or(facet_prefix);
        doc.plist.extend(crate::compiler::ios_icon_plist(name, &loose, &[]));
    }
    Ok(doc)
}

/// Write premultiplied BGRA `size`² pixels as a `px`² PNG.
fn save_resized(bgra: &[u8], size: u32, px: u32, path: &Path) -> Result<()> {
    let rgba: Vec<u8> = bgra
        .chunks_exact(4)
        .flat_map(|c| {
            let a = c[3] as u32;
            let un = |v: u8| if a == 0 { 0 } else { ((v as u32 * 255 + a / 2) / a).min(255) as u8 };
            [un(c[2]), un(c[1]), un(c[0]), c[3]]
        })
        .collect();
    let img = image::RgbaImage::from_raw(size, size, rgba)
        .ok_or_else(|| anyhow::anyhow!("render is not {size}x{size}"))?;
    crate::compiler::write_loose_icon(path, &image::imageops::resize(&img, px, px, FilterType::Lanczos3))
}

/// One `<stem>_Assets/<image stem>` asset per distinct layer image found in `Assets/` or the
/// bundle root.
fn collect_layer_assets(
    bundle: &Path,
    json: &IconJson,
    facet_prefix: &str,
) -> Vec<LayerAsset> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (_group, layer) in json.iter_layers() {
        let Some(image_name) = layer.image_name.as_deref() else {
            continue;
        };
        let assets_path = bundle.join("Assets").join(image_name);
        let resolved = if assets_path.exists() {
            assets_path
        } else {
            let root_path = bundle.join(image_name);
            if !root_path.exists() {
                continue;
            }
            root_path
        };
        // Apple names the facet after the image file stem, not the layer `name` (KYA: AppIcon.svg
        // → "AppIcon" for layer "Logo"; tagspaces: Image 2.png → "Image 2").
        let stem = std::path::Path::new(image_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(image_name);
        let facet_name = format!("{facet_prefix}_Assets/{}", facet_component(stem));
        if !seen.insert(facet_name.clone()) {
            continue;
        }
        out.push(LayerAsset {
            facet_name,
            source_path: resolved,
        });
    }
    out
}

/// Apple's actool errors on a layer without `image-name` or whose image is in neither `Assets/`
/// nor the bundle root.
fn validate_layer_image_refs(bundle: &Path, json: &serde_json::Value) -> Result<()> {
    let groups = match json.get("groups").and_then(|v| v.as_array()) {
        Some(g) => g,
        None => return Ok(()),
    };
    for group in groups {
        let Some(layers) = group.get("layers").and_then(|v| v.as_array()) else {
            continue;
        };
        for layer in layers {
            let layer_name = layer
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("(unnamed)");
            let image_name = match layer.get("image-name").and_then(|v| v.as_str()) {
                Some(n) => n,
                None => {
                    anyhow::bail!(
                        "the layer \"{layer_name}\" does not have an image name"
                    );
                }
            };
            let in_assets = bundle.join("Assets").join(image_name);
            let in_root = bundle.join(image_name);
            if !in_assets.exists() && !in_root.exists() {
                anyhow::bail!(
                    "the layer \"{layer_name}\" references an image named \
                     \"{image_name}\" that does not exist"
                );
            }
        }
    }
    Ok(())
}

/// Grey floor an opaque-glass layer's blacks are lifted toward (measured on
/// KYA's cup body, lum ≈ 45/255).
const GLASS_FLOOR: f32 = 45.0 / 255.0;

/// A `layer-color`-shadow glass layer darkens the background by `D·(1 − colour)` per channel
/// (subtractive, not a multiply; overlaps add), `D` constant (`tools/probe_glass_tint.py`).
const GLASS_TINT_D: f32 = 63.5 / 255.0;

/// The "raised glass" edge is a blur, not a bevel (monotonic profile, no rim): Apple's σ ≈ 19 px
/// at 1024 (`tools/probe_glass_relief.py`). 18 because three box passes widen the effective σ.
const GLASS_BLUR_SIGMA: f32 = 18.0;

/// An untinted frosted layer darkens the background under its whole body by a flat fraction
/// (≈12.7/255 on scrumdinger), with no vertical ramp.
const GLASS_RELIEF_STRENGTH: f32 = 0.051;

/// Glass-layer drop shadow at 1024 px, tinted subtractively by `(1 − colour)`; tuned against
/// Apple's renders (`tools/probe_layer_shadow.py`).
const SHADOW_PEAK_LAYERCOLOR: f32 = 0.49;
const SHADOW_PEAK_NEUTRAL: f32 = 0.10;
const SHADOW_SIGMA: f32 = 17.0;
const SHADOW_OFFSET_Y: f32 = 12.0;

/// In-place separable blur of a straight-RGBA buffer (Gaussian approximated by
/// three box passes), blurring in premultiplied space so transparent edges
/// don't bleed dark. `sigma` is in pixels; a no-op below ~1 px.
pub(crate) fn blur_rgba_premul(buf: &mut [u8], w: usize, sigma: f32) {
    let radius = sigma.round() as usize;
    if radius < 1 {
        return;
    }
    let n = w * w;
    let mut ch: [Vec<f32>; 4] =
        [vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]];
    for i in 0..n {
        let a = buf[i * 4 + 3] as f32 / 255.0;
        for c in 0..3 {
            ch[c][i] = (buf[i * 4 + c] as f32 / 255.0) * a;
        }
        ch[3][i] = a;
    }
    let mut tmp = vec![0.0f32; n];
    for c in &mut ch {
        for _ in 0..3 {
            box_blur_h(c, &mut tmp, w, radius);
            box_blur_v(c, &mut tmp, w, radius);
        }
    }
    for i in 0..n {
        let a = ch[3][i].clamp(0.0, 1.0);
        for c in 0..3 {
            let v = if a > 0.0004 { (ch[c][i] / a).clamp(0.0, 1.0) } else { 0.0 };
            buf[i * 4 + c] = (v * 255.0).round() as u8;
        }
        buf[i * 4 + 3] = (a * 255.0).round() as u8;
    }
}

/// One horizontal box-blur pass (radius `r`, edge-clamped) in place; `tmp` is scratch.
fn box_blur_h(src: &mut [f32], tmp: &mut [f32], w: usize, r: usize) {
    let win = (2 * r + 1) as f32;
    for y in 0..w {
        let row = y * w;
        let mut sum = 0.0;
        for k in 0..=r {
            sum += src[row + k.min(w - 1)];
        }
        sum += src[row] * r as f32; // left edge clamp
        for x in 0..w {
            tmp[row + x] = sum / win;
            let add = (x + r + 1).min(w - 1);
            let sub = if x >= r { x - r } else { 0 };
            sum += src[row + add] - src[row + sub];
        }
    }
    src.copy_from_slice(tmp);
}

/// One vertical box-blur pass (radius `r`, edge-clamped).
fn box_blur_v(src: &mut [f32], tmp: &mut [f32], w: usize, r: usize) {
    let win = (2 * r + 1) as f32;
    for x in 0..w {
        let mut sum = 0.0;
        for k in 0..=r {
            sum += src[k.min(w - 1) * w + x];
        }
        sum += src[x] * r as f32;
        for y in 0..w {
            tmp[y * w + x] = sum / win;
            let add = (y + r + 1).min(w - 1);
            let sub = if y >= r { y - r } else { 0 };
            sum += src[add * w + x] - src[sub * w + x];
        }
    }
    src.copy_from_slice(tmp);
}

/// A layer in paint order; `scale`, `tx`, `ty` are in 1024-canvas pixels.
struct StackLayer {
    source: PathBuf,
    /// Frosted glass (glass + translucency enabled): drawn as a faint
    /// see-through relief rather than in its own colour.
    frosted: bool,
    /// Frosted glass whose group shadow is `layer-color`: its colour survives as a subtractive
    /// tint; with `none`/`neutral` it is a neutral relief.
    tinted: bool,
    /// Opaque specular glass (KYA's coffee cup): keeps its colour, blacks lifted to `GLASS_FLOOR`.
    specular: bool,
    /// Drop-shadow peak; 0 = none.
    shadow_strength: f32,
    scale: f32,
    tx: f32,
    ty: f32,
    opacity: f32,
    blend: BlendMode,
    /// Native point size of the source (SVG viewBox / image dimensions), so a
    /// non-1024 / non-square layer keeps its aspect and isn't stretched.
    native_w: u32,
    native_h: u32,
    /// The layer's solid fill, which replaces the artwork's colour inside its coverage.
    fill: Option<[f64; 3]>,
}

/// The visible layers of every group for `appearance`, back to front.
fn collect_stack_layers(
    bundle: &Path,
    parsed: &IconJson,
    appearance: crate::icon_effects::Appearance,
    shape: crate::icon_render::Shape,
) -> Vec<StackLayer> {
    let base = shape.content_scale();
    use crate::icon_effects::resolve_icon_effects;
    let mut out = Vec::new();
    for group in &parsed.groups {
        let eff = resolve_icon_effects(group, appearance);
        // A glass sibling frosts layers that don't opt out (scrumdinger); group
        // translucency alone doesn't (openrct2, WiresharkCapture's fin keep colour).
        let glass_context = eff.layers.iter().any(|l| l.glass);
        let (gscale, gtx, gty) = group
            .position
            .as_ref()
            .map(|p| {
                let t = p.translation_in_points.unwrap_or([0.0, 0.0]);
                (p.scale.unwrap_or(1.0), t[0], t[1])
            })
            .unwrap_or((1.0, 0.0, 0.0));
        for (i, layer) in group.layers.iter().enumerate() {
            if layer.hidden == Some(true) {
                continue;
            }
            let Some(name) = layer.image_name.as_deref() else { continue };
            let assets = crate::catalog::entry_path(&bundle.join("Assets"), name);
            let path =
                if assets.exists() { assets } else { crate::catalog::entry_path(bundle, name) };
            if !path.exists() {
                continue;
            }
            let explicit = eff.layers.get(i).map(|l| l.glass).unwrap_or(false);
            let opted_out =
                layer.glass == Some(false) && layer.glass_specializations.is_none();
            let (lscale, ltx, lty) = layer
                .position
                .as_ref()
                .map(|p| {
                    let t = p.translation_in_points.unwrap_or([0.0, 0.0]);
                    (p.scale.unwrap_or(1.0), t[0], t[1])
                })
                .unwrap_or((1.0, 0.0, 0.0));
            let (opacity, blend) = eff
                .layers
                .get(i)
                .map(|l| (l.opacity, parse_blend(&l.blend_mode)))
                .unwrap_or((1.0, BlendMode::Normal));
            let is_glass = explicit || (glass_context && !opted_out);
            let frosted = is_glass && eff.translucency.enabled;
            let tinted = frosted
                && eff.shadow.kind == crate::icon_effects::ShadowKind::LayerColor;
            let specular = is_glass && !eff.translucency.enabled && eff.specular;
            use crate::icon_effects::ShadowKind;
            let shadow_strength = if is_glass {
                match eff.shadow.kind {
                    ShadowKind::LayerColor => SHADOW_PEAK_LAYERCOLOR * eff.shadow.opacity,
                    ShadowKind::Neutral => SHADOW_PEAK_NEUTRAL * eff.shadow.opacity,
                    ShadowKind::None => 0.0,
                }
            } else {
                0.0
            };
            let (native_w, native_h) = layer_native_size(&path);
            let fill = layer
                .fill_specializations
                .as_ref()
                .and_then(|s| crate::icon_effects::spec_value(s, appearance))
                .or(match layer.fill.as_ref() {
                    Some(Fill::Structured(v)) => Some(v),
                    _ => None,
                })
                .and_then(|v| v.get("solid")?.as_str())
                .and_then(parse_color_spec)
                .and_then(|(_, comps)| comps_to_rgb(&comps));
            out.push(StackLayer {
                source: path,
                frosted,
                tinted,
                specular,
                shadow_strength,
                scale: base * gscale * lscale,
                tx: base * (gscale * ltx + gtx),
                ty: base * (gscale * lty + gty),
                opacity,
                blend,
                native_w,
                native_h,
                fill,
            });
        }
    }
    // icon.json lists groups and layers front to back (index 0 on top).
    out.reverse();
    out
}

fn is_svg_path(path: &Path) -> bool {
    path.to_string_lossy().to_lowercase().ends_with(".svg")
}

/// A layer source at `w`×`h`, straight RGBA.
fn rasterize_layer(path: &Path, w: u32, h: u32) -> Result<Vec<u8>> {
    if is_svg_path(path) {
        // `rasterize_svg` returns premultiplied BGRA.
        let svg = fs::read(path)?;
        let bgra = crate::svg_raster::rasterize_svg(&svg, w, h, 1)?;
        let mut rgba = vec![0u8; bgra.len()];
        for (o, px) in bgra.chunks_exact(4).enumerate() {
            let a = px[3];
            let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
            rgba[o * 4] = un(px[2]);
            rgba[o * 4 + 1] = un(px[1]);
            rgba[o * 4 + 2] = un(px[0]);
            rgba[o * 4 + 3] = a;
        }
        Ok(rgba)
    } else {
        let img = crate::color::open(path)?.image.to_rgba8();
        let resized = image::imageops::resize(&img, w, h, FilterType::Lanczos3);
        Ok(resized.into_raw())
    }
}

/// Native point size of a layer source (SVG size or viewBox, raster dimensions).
fn layer_native_size(path: &Path) -> (u32, u32) {
    if is_svg_path(path) {
        if let Ok(svg) = fs::read(path) {
            let (w, h) = crate::svg_raster::parse_svg_dimensions(&svg);
            if w > 0 && h > 0 {
                return (w, h);
            }
        }
        (1024, 1024)
    } else {
        image::image_dimensions(path).unwrap_or((1024, 1024))
    }
}

/// Separable layer blend modes (icon.json `blend-mode-specializations`). The
/// channel functions are the W3C/PDF separable blends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    SoftLight,
    HardLight,
    Darken,
    Lighten,
}

fn parse_blend(s: &str) -> BlendMode {
    match s {
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "soft-light" => BlendMode::SoftLight,
        "hard-light" => BlendMode::HardLight,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        _ => BlendMode::Normal,
    }
}

/// Blend one channel (backdrop `cb`, source `cs`, both 0..1).
fn blend_channel(mode: BlendMode, cb: f32, cs: f32) -> f32 {
    let hard = |cb: f32, cs: f32| {
        if cs <= 0.5 {
            2.0 * cb * cs
        } else {
            1.0 - 2.0 * (1.0 - cb) * (1.0 - cs)
        }
    };
    match mode {
        BlendMode::Normal => cs,
        BlendMode::Multiply => cb * cs,
        BlendMode::Screen => cb + cs - cb * cs,
        BlendMode::Overlay => hard(cs, cb),
        BlendMode::HardLight => hard(cb, cs),
        BlendMode::Darken => cb.min(cs),
        BlendMode::Lighten => cb.max(cs),
        BlendMode::SoftLight => {
            if cs <= 0.5 {
                cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
            } else {
                let d = if cb <= 0.25 {
                    ((16.0 * cb - 12.0) * cb + 4.0) * cb
                } else {
                    cb.sqrt()
                };
                cb + (2.0 * cs - 1.0) * (d - cb)
            }
        }
    }
}

/// Straight-alpha source-over of `src` onto `dst` with a separable blend mode
/// (W3C compositing). `Normal` reduces to plain "over".
fn composite_blend(dst: &mut [u8], src: &[u8], mode: BlendMode) {
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return;
    }
    for c in 0..3 {
        let cs = src[c] as f32 / 255.0;
        let cb = dst[c] as f32 / 255.0;
        let bl = blend_channel(mode, cb, cs);
        // Co = (1-αb)·αs·Cs + αb·αs·B(Cb,Cs) + (1-αs)·αb·Cb, normalised by αo.
        let co = (1.0 - da) * sa * cs + da * sa * bl + (1.0 - sa) * da * cb;
        dst[c] = (co / oa * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Frosted layer over its backdrop (background plus the layers drawn so far), shaded and blurred.
fn draw_glass(
    rgba: &mut [u8],
    cov: &[u8],
    tint_sub: &[[f32; 3]],
    gradient: Option<&crate::icon_render::GradientFill>,
    pixel_size: u32,
) {
    let w = pixel_size as usize;
    let mut glass_buf = vec![0u8; w * w * 4];
    for y in 0..w {
        for x in 0..w {
            let i = y * w + x;
            if cov[i] == 0 {
                continue;
            }
            let glass_px = match gradient {
                Some(g) => {
                    let bg = g.sample(x as u32, y as u32, pixel_size);
                    let la = rgba[i * 4 + 3] as f32 / 255.0;
                    let mut out = [0u8; 4];
                    for c in 0..3 {
                        let backdrop = rgba[i * 4 + c] as f32 / 255.0 * la + bg[c] as f32 * (1.0 - la);
                        let v = match tint_sub.get(i) {
                            Some(sub) => backdrop - sub[c],
                            None => backdrop * (1.0 - GLASS_RELIEF_STRENGTH),
                        };
                        out[c] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                    out[3] = cov[i];
                    out
                }
                // No gradient (raw-layer fallback): keep the neutral relief.
                None => [0, 0, 0, (cov[i] as f32 * GLASS_RELIEF_STRENGTH).round() as u8],
            };
            glass_buf[i * 4..i * 4 + 4].copy_from_slice(&glass_px);
        }
    }
    blur_rgba_premul(&mut glass_buf, w, GLASS_BLUR_SIGMA * pixel_size as f32 / 1024.0);
    for i in 0..w * w {
        if glass_buf[i * 4 + 3] == 0 {
            continue;
        }
        let g = [glass_buf[i * 4], glass_buf[i * 4 + 1], glass_buf[i * 4 + 2], glass_buf[i * 4 + 3]];
        let mut dst = [rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2], rgba[i * 4 + 3]];
        composite_blend(&mut dst, &g, BlendMode::Normal);
        rgba[i * 4..i * 4 + 4].copy_from_slice(&dst);
    }
}

/// Composite `layers` into premultiplied BGRA at `pixel_size`². Frosted layers share one
/// coverage mask drawn as a tint of `gradient` (see `GLASS_*`), not in their own colour.
fn render_layer_stack(
    layers: &[StackLayer],
    pixel_size: u32,
    gradient: Option<&crate::icon_render::GradientFill>,
) -> Result<Vec<u8>> {
    let w = pixel_size as usize;
    let n = w * w;
    let mut rgba = vec![0u8; n * 4];
    let mut shadow_dark = vec![[0.0f32; 3]; n];
    let mut any_shadow = false;
    let f = pixel_size as f32 / 1024.0;
    // Multi-group icons repeat a source at the same size once per group (hundreds of decodes
    // at 50 groups), and decoding + resizing dominates the render.
    let mut raster_cache: std::collections::HashMap<(PathBuf, u32, u32), Vec<u8>> =
        std::collections::HashMap::new();
    for layer in layers {
        let k = layer.scale * f;
        let rw = ((layer.native_w as f32 * k).round() as u32).max(1);
        let rh = ((layer.native_h as f32 * k).round() as u32).max(1);
        use std::collections::hash_map::Entry;
        let src = match raster_cache.entry((layer.source.clone(), rw, rh)) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => e.insert(rasterize_layer(&layer.source, rw, rh)?),
        };
        let filled;
        let src = match layer.fill {
            Some(rgb) => {
                let px = rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
                filled = src
                    .chunks_exact(4)
                    .flat_map(|p| [px[0], px[1], px[2], p[3]])
                    .collect::<Vec<u8>>();
                &filled
            }
            None => &*src,
        };
        let (rw, rh) = (rw as i64, rh as i64);
        let half = pixel_size as f32 / 2.0;
        let ox = (half + layer.tx * f - rw as f32 / 2.0).round() as i64;
        let oy = (half + layer.ty * f - rh as f32 / 2.0).round() as i64;
        // A frosted layer covers what is below it, so it is drawn when it comes up, not after.
        let mut glass_cov = if layer.frosted { vec![0u8; n] } else { Vec::new() };
        let mut glass_sub = if layer.tinted { vec![[0.0f32; 3]; n] } else { Vec::new() };
        for ly in 0..rh {
            let cy = oy + ly;
            if cy < 0 || cy >= w as i64 {
                continue;
            }
            for lx in 0..rw {
                let cx = ox + lx;
                if cx < 0 || cx >= w as i64 {
                    continue;
                }
                let si = ((ly * rw + lx) * 4) as usize;
                let sa = (src[si + 3] as f32 * layer.opacity).round() as u8;
                if sa == 0 {
                    continue;
                }
                let ci = (cy * w as i64 + cx) as usize;
                if layer.shadow_strength > 0.0 {
                    any_shadow = true;
                    let cov = sa as f32 / 255.0;
                    for c in 0..3 {
                        let col = src[si + c] as f32 / 255.0;
                        shadow_dark[ci][c] += layer.shadow_strength * cov * (1.0 - col);
                    }
                }
                if layer.frosted {
                    glass_cov[ci] = sa;
                    if layer.tinted {
                        let cov = sa as f32 / 255.0;
                        for c in 0..3 {
                            let col = src[si + c] as f32 / 255.0;
                            glass_sub[ci][c] = cov * GLASS_TINT_D * (1.0 - col);
                        }
                    }
                } else {
                    let mut dst =
                        [rgba[ci * 4], rgba[ci * 4 + 1], rgba[ci * 4 + 2], rgba[ci * 4 + 3]];
                    let s = if layer.specular {
                        let lift = |c: u8| {
                            let c = c as f32 / 255.0;
                            ((c + GLASS_FLOOR - c * GLASS_FLOOR) * 255.0).round() as u8
                        };
                        [lift(src[si]), lift(src[si + 1]), lift(src[si + 2]), sa]
                    } else {
                        [src[si], src[si + 1], src[si + 2], sa]
                    };
                    composite_blend(&mut dst, &s, layer.blend);
                    rgba[ci * 4..ci * 4 + 4].copy_from_slice(&dst);
                }
            }
        }
        if layer.frosted {
            draw_glass(&mut rgba, &glass_cov, &glass_sub, gradient, pixel_size);
        }
    }
    // The drop shadow darkens only background pixels, so the casting layer stays on top.
    if any_shadow {
        if let Some(g) = gradient {
            let off = (SHADOW_OFFSET_Y * f).round() as usize;
            let mut chans: [Vec<f32>; 3] =
                [vec![0.0; n], vec![0.0; n], vec![0.0; n]];
            for y in 0..w {
                let sy = y.saturating_sub(off);
                for x in 0..w {
                    for c in 0..3 {
                        chans[c][y * w + x] = shadow_dark[sy * w + x][c];
                    }
                }
            }
            let radius = (SHADOW_SIGMA * f).round() as usize;
            if radius >= 1 {
                let mut tmp = vec![0.0f32; n];
                for c in &mut chans {
                    for _ in 0..3 {
                        box_blur_h(c, &mut tmp, w, radius);
                        box_blur_v(c, &mut tmp, w, radius);
                    }
                }
            }
            for y in 0..w {
                for x in 0..w {
                    let i = y * w + x;
                    if rgba[i * 4 + 3] >= 16 {
                        continue; // a layer/glass already covers this pixel
                    }
                    let dark = [chans[0][i], chans[1][i], chans[2][i]];
                    if dark[0] + dark[1] + dark[2] < 0.004 {
                        continue;
                    }
                    let bg = g.sample(x as u32, y as u32, pixel_size);
                    let mut out = [0u8; 4];
                    for c in 0..3 {
                        let v = (bg[c] as f32 - dark[c]).clamp(0.0, 1.0);
                        out[c] = (v * 255.0).round() as u8;
                    }
                    out[3] = 255;
                    rgba[i * 4..i * 4 + 4].copy_from_slice(&out);
                }
            }
        }
    }
    let mut out = vec![0u8; n * 4];
    for i in 0..n {
        let (r, g, b, a) = (rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2], rgba[i * 4 + 3]);
        let pm = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        out[i * 4] = pm(b);
        out[i * 4 + 1] = pm(g);
        out[i * 4 + 2] = pm(r);
        out[i * 4 + 3] = a;
    }
    Ok(out)
}

fn find_all_source_images(bundle: &Path, json: &serde_json::Value) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(groups) = json.get("groups").and_then(|v| v.as_array()) {
        for group in groups {
            if let Some(layers) = group.get("layers").and_then(|v| v.as_array()) {
                for layer in layers {
                    let Some(name) = layer.get("image-name").and_then(|v| v.as_str())
                    else {
                        continue;
                    };
                    if !seen.insert(name.to_string()) {
                        continue;
                    }
                    let assets_path = crate::catalog::entry_path(&bundle.join("Assets"), name);
                    if assets_path.exists() {
                        out.push(assets_path);
                        continue;
                    }
                    let root_path = crate::catalog::entry_path(bundle, name);
                    if root_path.exists() {
                        out.push(root_path);
                    }
                }
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn build_icon_car(
    icon_name: &str,
    icon_images: &[(PathBuf, u32, u32)],
    layer_assets: &[LayerAsset],
    group_facet_names: &[String],
    color_assets: &[ColorAsset],
    gradient_assets: &[GradientAsset],
    groups: &[crate::icon_json::Group],
    light_stacks: &[Vec<u8>],
    light_fill: Option<&crate::icon_render::GradientFill>,
    solid_fill_color: Option<&str>,
    stack_appearances: &[u16],
    platform: &str,
    min_deploy: &str,
) -> Result<crate::assemble::Document> {
    let ident = hash_name(icon_name);
    let mut renditions: Vec<Rendition> = Vec::new();

    let mut packable: Vec<Rendition> = Vec::new();

    // Apple's margin halo ignores the group `shadow` kind (same for `none`, absent, `layer-color`:
    // element-web, feishin, KYA). `shadow_geometry` is tuned to this spec.
    use crate::icon_effects::{ShadowKind, ShadowSpec};
    let _ = groups;
    let icon_shadow = Some(ShadowSpec { kind: ShadowKind::Neutral, opacity: 0.5 });

    for (idx, (img_path, pixel_size, scale)) in icon_images.iter().enumerate() {
        let point_size = pixel_size / scale;
        let dim2 = icon_dim2(point_size);
        let (layer_bgra, w, h) = match light_stacks.get(idx) {
            Some(stack) if stack.len() == (pixel_size * pixel_size * 4) as usize => {
                (stack.clone(), *pixel_size, *pixel_size)
            }
            _ => {
                let img = load_image_as_bgra(img_path, true)?;
                (img.pixel_data, img.width, img.height)
            }
        };
        let shadow = shadow_params(icon_shadow, *pixel_size);
        let composited = match light_fill {
            Some(f) => crate::icon_render::composite_icon(*pixel_size, f, &layer_bgra, shadow.as_ref())
                .unwrap_or_else(|| layer_bgra.clone()),
            None => layer_bgra.clone(),
        };
        let (pd, w, h, pf) = crate::catalog::bgra_to_best_format(composited, w, h, false);
        let cs_id = car::colorspace_for_pixel_format(&pf);
        let name = pre_rendered_name(icon_name, point_size, *scale, "NSAppearanceNameSystem");
        let rendition = Rendition {
                name,
                identifier: ident,
                element: car::ELEMENT_UNIVERSAL,
                part: car::PART_ICON,
                scale: *scale as u16,
                width: w,
                height: h,
                pixel_data: pd,
                pixel_format: pf,
                layout: car::LAYOUT_ONE_PART_SCALE,
                dim2,
                min_deploy: min_deploy.to_string(),
                platform: platform.to_string(),
                colorspace_id: cs_id,
                // The default -1 sets the rendition flag that makes CUICatalog look for a
                // template variant that doesn't exist.
                template_rendering_intent: 0,
                ..Rendition::default()
        };
        if ATLAS_POINT_SIZES.contains(&point_size) {
            packable.push(rendition);
        } else {
            renditions.push(rendition);
        }
    }

    let mut ms_entries: Vec<MultisizeImageEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (_, pixel_size, scale) in icon_images {
        let pt = pixel_size / scale;
        if seen.insert(pt) {
            ms_entries.push(MultisizeImageEntry {
                width: pt,
                height: pt,
                index: icon_dim2(pt) as u32,
            });
        }
    }
    if !ms_entries.is_empty() {
        renditions.push(car::build_multisize_rendition(icon_name, ident, &ms_entries));
    }

    // Apple's main facet carries PART_ICON (its sized renditions), not the iconstack's
    // PART_ICON_COMPOSER.
    let mut facets: Vec<FacetEntry> = vec![(
        icon_name.to_string(),
        car::ELEMENT_UNIVERSAL,
        Some(car::PART_ICON),
        ident,
    )];
    for gname in group_facet_names {
        let gid = hash_name(gname);
        facets.push((
            gname.clone(),
            car::ELEMENT_UNIVERSAL,
            Some(car::PART_ICON_GROUP),
            gid,
        ));
    }
    for asset in layer_assets {
        let asset_ident = hash_name(&asset.facet_name);
        let is_svg = asset
            .source_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("svg"))
            .unwrap_or(false);
        if is_svg {
            // Apple stores an SVG layer as a vector rendition of the raw SVG, not a bitmap.
            let svg_data = fs::read(&asset.source_path)?;
            let csi = car::build_svg_csi("image.svg", &svg_data);
            renditions.push(Rendition {
                name: "image.svg".to_string(),
                identifier: asset_ident,
                element: car::ELEMENT_UNIVERSAL,
                part: car::PART_REGULAR,
                scale: 1,
                width: 0,
                height: 0,
                pixel_data: Vec::new(),
                pixel_format: *car::PIXELFMT_SVG,
                layout: car::LAYOUT_PDF,
                min_deploy: min_deploy.to_string(),
                platform: platform.to_string(),
                colorspace_id: 0,
                csi_override: Some(csi),
                ..Rendition::default()
            });
            facets.push((
                asset.facet_name.clone(),
                car::ELEMENT_UNIVERSAL,
                Some(car::PART_REGULAR),
                asset_ident,
            ));
            continue;
        }
        // actool stores layer images larger than the 1024-px canvas shrunk to it (wireshark's
        // 2048-px layers); the slices keep the source size.
        let img = crate::catalog::load_image_fitting(&asset.source_path, false, 1024)?;
        // Apple renames every layer image "image.png"; lookups go by attributes, so only byte
        // parity depends on it.
        renditions.push(Rendition {
            name: "image.png".to_string(),
            identifier: asset_ident,
            element: car::ELEMENT_UNIVERSAL,
            part: car::PART_REGULAR,
            scale: 1,
            width: img.width,
            height: img.height,
            slices: Some(image::image_dimensions(&asset.source_path)?),
            colorspace_id: car::colorspace_for_pixel_format(&img.pixel_format),
            pixel_data: img.pixel_data,
            pixel_format: img.pixel_format,
            wide_gamut: img.wide_gamut,
            layout: car::LAYOUT_ONE_PART_SCALE,
            min_deploy: min_deploy.to_string(),
            platform: platform.to_string(),
            // Apple stores layer images non-opaque (CELM ver=0) even when fully opaque.
            force_non_opaque: true,
            ..Rendition::default()
        });
        facets.push((
            asset.facet_name.clone(),
            car::ELEMENT_UNIVERSAL,
            Some(car::PART_REGULAR),
            asset_ident,
        ));
    }

    for color in color_assets {
        let cident = hash_name(&color.facet_name);
        let csi = car::build_icon_color_csi(
            &color.facet_name,
            color.colorspace_id,
            &color.components,
        );
        // Color keys use scale=1 although the CSI scale_factor is 0: CUICatalog filters keys by
        // scale, so scale=0 colors are invisible to colorWithName:.
        renditions.push(Rendition {
            name: color.facet_name.clone(),
            identifier: cident,
            element: car::ELEMENT_UNIVERSAL,
            part: car::PART_COLOR,
            scale: 1,
            width: 0,
            height: 0,
            pixel_data: Vec::new(),
            pixel_format: *b"\0\0\0\0",
            layout: car::LAYOUT_COLOR,
            min_deploy: min_deploy.to_string(),
            platform: platform.to_string(),
            colorspace_id: 0,
            csi_override: Some(csi),
            ..Rendition::default()
        });
        facets.push((
            color.facet_name.clone(),
            car::ELEMENT_UNIVERSAL,
            Some(car::PART_COLOR),
            cident,
        ));
    }

    for grad in gradient_assets {
        let gident = hash_name(&grad.facet_name);
        let stops: Vec<(f32, &str)> = grad
            .stops
            .iter()
            .map(|(p, name)| (*p, name.as_str()))
            .collect();
        let csi = car::build_icon_gradient_csi(
            &grad.facet_name,
            grad.geometry,
            &stops,
            grad.kind,
        );
        // Gradient KEYs follow the same scale=1 convention as Colors.
        renditions.push(Rendition {
            name: grad.facet_name.clone(),
            identifier: gident,
            element: car::ELEMENT_UNIVERSAL,
            part: car::PART_ICON_GRADIENT,
            scale: 1,
            width: 0,
            height: 0,
            pixel_data: Vec::new(),
            pixel_format: *b"\0\0\0\0",
            layout: car::LAYOUT_GRADIENT,
            min_deploy: min_deploy.to_string(),
            platform: platform.to_string(),
            colorspace_id: 0,
            csi_override: Some(csi),
            ..Rendition::default()
        });
        facets.push((
            grad.facet_name.clone(),
            car::ELEMENT_UNIVERSAL,
            Some(car::PART_ICON_GRADIENT),
            gident,
        ));
    }

    // Apple emits an iconstack and IconGroups per non-system appearance.
    let layered = !group_facet_names.is_empty()
        && !layer_assets.is_empty()
        && !gradient_assets.is_empty();
    if layered {
        let main_ident = hash_name(icon_name);
        let stack_name = format!("{icon_name}.iconstack");
        let group_idents: Vec<u16> = group_facet_names.iter().map(|f| hash_name(f)).collect();
        // A solid fill's palette has only the dark gradient; its light fill is the user colour.
        let (light_fill, dark_gradient) = match solid_fill_color {
            Some(color) => ((car::PART_COLOR, hash_name(color)), &gradient_assets[0]),
            None => (
                (car::PART_ICON_GRADIENT, hash_name(&gradient_assets[0].facet_name)),
                gradient_assets.get(1).unwrap_or(&gradient_assets[0]),
            ),
        };
        let dark_fill = (car::PART_ICON_GRADIENT, hash_name(&dark_gradient.facet_name));
        for &appearance in stack_appearances {
            let look = match appearance {
                1 => crate::icon_effects::Appearance::Dark,
                10 => crate::icon_effects::Appearance::Tinted,
                _ => crate::icon_effects::Appearance::Light,
            };
            let (fill_part, fill_ident) = if appearance == 1 { dark_fill } else { light_fill };
            let mut stack_layers = vec![car::LayerRef {
                part: fill_part,
                identifier: fill_ident,
                opacity: 1.0,
                ..Default::default()
            }];
            let mut group_renditions = Vec::new();
            for (group, ident) in groups.iter().zip(&group_idents) {
                let layer_refs = group_layer_refs(group, look, icon_name, layer_assets, color_assets, gradient_assets);
                if layer_refs.is_empty() {
                    continue;
                }
                let eff = crate::icon_effects::resolve_icon_effects(group, look);
                use crate::icon_effects::ShadowKind;
                stack_layers.push(car::LayerRef {
                    part: car::PART_ICON_GROUP,
                    identifier: *ident,
                    opacity: 1.0,
                    kind: 2,
                    value: eff.blur_material.unwrap_or(0.0),
                    effects: Some(car::GroupEffects {
                        translucency: if eff.translucency.enabled { eff.translucency.value } else { 0.0 },
                        shadow_kind: match eff.shadow.kind {
                            ShadowKind::None => 0,
                            ShadowKind::LayerColor => 2,
                            ShadowKind::Neutral => 3,
                        },
                        shadow_opacity: eff.shadow.opacity,
                    }),
                    ..Default::default()
                });
                group_renditions.push((*ident, layer_refs));
            }
            // icon.json lists groups front to back; stacks paint back to front.
            stack_layers[1..].reverse();
            let stack_csi = car::build_iconstack_csi(&stack_name, 1024, &stack_layers);
            renditions.push(Rendition {
                name: stack_name.clone(),
                identifier: main_ident,
                element: car::ELEMENT_UNIVERSAL,
                part: car::PART_ICON_COMPOSER,
                scale: 1,
                appearance,
                width: 1024,
                height: 1024,
                pixel_data: Vec::new(),
                pixel_format: *car::PIXELFMT_DATA,
                layout: car::LAYOUT_ICONSTACK,
                min_deploy: min_deploy.to_string(),
                platform: platform.to_string(),
                colorspace_id: 0,
                csi_override: Some(stack_csi),
                ..Rendition::default()
            });

            for (group_ident, layer_refs) in &group_renditions {
                let group_csi = car::build_icongroup_csi("IconGroup", layer_refs);
                renditions.push(Rendition {
                    name: "IconGroup".to_string(),
                    identifier: *group_ident,
                    element: car::ELEMENT_UNIVERSAL,
                    part: car::PART_ICON_GROUP,
                    scale: 1,
                    appearance,
                    width: 0,
                    height: 0,
                    pixel_data: Vec::new(),
                    pixel_format: *car::PIXELFMT_DATA,
                    layout: car::LAYOUT_ICON_GROUP,
                    min_deploy: min_deploy.to_string(),
                    platform: platform.to_string(),
                    colorspace_id: 0,
                    csi_override: Some(group_csi),
                    ..Rendition::default()
                });
            }
        }
    }

    Ok(crate::assemble::Document {
        renditions: car::gamut_variants(&renditions),
        packable,
        facets: facets
            .into_iter()
            .map(|(name, element, part, identifier)| crate::assemble::Facet { name, element, part, identifier })
            .collect(),
        coreui_version: 975,
        // In .icon catalogs every facet's Appearance mask is -1, even for single-appearance colors.
        wildcard_attributes: vec![7],
        ..Default::default()
    })
}

/// A layer's source image and its `<stem>_Assets/<image stem>` facet.
pub struct LayerAsset {
    pub facet_name: String,
    pub source_path: PathBuf,
}

/// A solid color asset extracted from icon.json `fill` / `fill-specializations`.
struct ColorAsset {
    facet_name: String,
    colorspace_id: u32,
    components: Vec<f64>,
}

/// A gradient asset extracted from icon.json `fill` specs. Stops reference
/// Color assets by facet name (e.g. "icon_Assets/Color-2").
struct GradientAsset {
    facet_name: String,
    geometry: [f32; 4],
    stops: Vec<(f32, String)>,
    /// 0 = single-color (radial-style; Apple emits this for the user
    /// color in `automatic-gradient`), 1 = linear along `geometry`.
    kind: u32,
}

/// Components as device RGB, alpha dropped. p3 passes through unconverted, exact only for grays.
fn comps_to_rgb(comps: &[f64]) -> Option<[f64; 3]> {
    match comps {
        [g, _a] => Some([*g, *g, *g]),
        [r, g, b, _a] => Some([*r, *g, *b]),
        _ => None,
    }
}

fn color_to_rgb(c: &ColorAsset) -> [f64; 3] {
    comps_to_rgb(&c.components).unwrap_or([0.0, 0.0, 0.0])
}

/// Margin drop-shadow parameters at `pixel_size`. Both shadow kinds are drawn black; dark-mode
/// backgrounds make the difference negligible.
fn shadow_params(
    spec: Option<crate::icon_effects::ShadowSpec>,
    pixel_size: u32,
) -> Option<crate::icon_render::ShadowParams> {
    use crate::icon_effects::shadow_geometry::{BLUR_RATIO, OFFSET_Y_RATIO, PEAK_ALPHA};
    use crate::icon_effects::ShadowKind;
    let spec = spec?;
    if spec.kind == ShadowKind::None || spec.opacity <= 0.0 {
        return None;
    }
    let alpha = (2.0 * PEAK_ALPHA * spec.opacity as f64).min(1.0);
    Some(crate::icon_render::ShadowParams {
        color: [0.0, 0.0, 0.0, alpha],
        blur: pixel_size as f64 * BLUR_RATIO,
        offset: [0.0, pixel_size as f64 * OFFSET_Y_RATIO],
    })
}

/// `grad` as a background fill; a single-stop gradient is flat.
fn resolve_gradient_fill(
    grad: &GradientAsset,
    colors: &[ColorAsset],
    shape: crate::icon_render::Shape,
) -> Option<crate::icon_render::GradientFill> {
    let rgb = |facet: &str| -> Option<[f64; 3]> {
        colors.iter().find(|c| c.facet_name == facet).map(color_to_rgb)
    };
    let start_rgb = rgb(&grad.stops.first()?.1)?;
    let stop_rgb = rgb(&grad.stops.last()?.1)?;
    let g = grad.geometry;
    // Apple draws the first stop at the top across the full content box whatever the stored
    // y order or span (feishin's 1.0→0.3 renders like 0→1). Observed gradients are all vertical.
    let (top_y, bot_y) = (1.0f32, 0.0f32);
    // The gradient spans the layers' content box, not the squircle: measured 662 px at 1024
    // (y ≈ 181–843), on x too.
    let inset = |p: f32| 0.5 + (p - 0.5) * shape.content_scale();
    Some(crate::icon_render::GradientFill {
        start_rgb,
        stop_rgb,
        start: [inset(g[0]), inset(top_y)],
        stop: [inset(g[2]), inset(bot_y)],
        shape,
    })
}

fn flat_fill(rgb: [f64; 3], shape: crate::icon_render::Shape) -> crate::icon_render::GradientFill {
    crate::icon_render::GradientFill {
        start_rgb: rgb,
        stop_rgb: rgb,
        start: [0.5, 0.0],
        stop: [0.5, 1.0],
        shape,
    }
}

/// The colour of a `fill: {"solid": …}` icon. Apple's light rendition paints it, not
/// `Gradient-1`, which is the dark-mode background.
fn solid_fill_color(parsed: &IconJson) -> Option<[f64; 3]> {
    let Some(Fill::Structured(v)) = parsed.fill.as_ref() else { return None };
    let spec = v.get("solid").and_then(|x| x.as_str())?;
    let (_cspace, comps) = parse_color_spec(spec)?;
    comps_to_rgb(&comps)
}

/// The (light, dark) background fills drawn under the layer stack.
fn resolve_background_fills(
    parsed: &IconJson,
    gradient_assets: &[GradientAsset],
    color_assets: &[ColorAsset],
    shape: crate::icon_render::Shape,
) -> (Option<crate::icon_render::GradientFill>, Option<crate::icon_render::GradientFill>) {
    let fill = |g: &GradientAsset| resolve_gradient_fill(g, color_assets, shape);
    if let Some(rgb) = solid_fill_color(parsed) {
        return (Some(flat_fill(rgb, shape)), gradient_assets.first().and_then(fill));
    }
    (gradient_assets.first().and_then(fill), gradient_assets.get(1).and_then(fill))
}

/// Apple's palette for `fill: "automatic"`: white anchor, light and dark background gradients.
/// Apple stores components as `f64(f32(v))`.
fn automatic_fill_assets(facet_prefix: &str) -> (Vec<ColorAsset>, Vec<GradientAsset>) {
    let n = |s: &str| format!("{facet_prefix}_Assets/{s}");
    let g = |v: f32| (v as f32) as f64;
    let colors = vec![
        ColorAsset {
            facet_name: n("Color-1"),
            colorspace_id: 6,
            components: vec![g(1.0), g(1.0)],
        },
        ColorAsset {
            facet_name: n("Color-2"),
            colorspace_id: 2,
            components: vec![g(1.0), g(1.0)],
        },
        ColorAsset {
            facet_name: n("Color-3"),
            colorspace_id: 2,
            components: vec![g(0.925), g(1.0)],
        },
        ColorAsset {
            facet_name: n("Color-4"),
            colorspace_id: 2,
            components: vec![g(0.192), g(1.0)],
        },
        ColorAsset {
            facet_name: n("Color-5"),
            colorspace_id: 2,
            components: vec![g(0.078), g(1.0)],
        },
    ];
    let gradients = vec![
        GradientAsset {
            facet_name: n("Gradient-1"),
            geometry: [0.5, 0.0, 0.5, 1.0],
            stops: vec![(0.0, n("Color-2")), (1.0, n("Color-3"))],
            kind: 1,
        },
        GradientAsset {
            facet_name: n("Gradient-2"),
            geometry: [0.5, 0.0, 0.5, 1.0],
            stops: vec![(0.0, n("Color-4")), (1.0, n("Color-5"))],
            kind: 1,
        },
    ];
    (colors, gradients)
}

/// Apple's actool treats an absent `fill` as `"automatic"`.
fn fill_is_automatic(fill: Option<&Fill>) -> bool {
    match fill {
        None => true,
        Some(Fill::Keyword(k)) => k == "automatic",
        Some(Fill::Structured(_)) => false,
    }
}

/// A parsed Apple color-spec string like `srgb:0.97288,0.97288,0.97288,1.0`
/// or `extended-gray:0.84536,1.0`. Returns (colorspace_id, components).
fn parse_color_spec(spec: &str) -> Option<(u32, Vec<f64>)> {
    let (name, rest) = spec.split_once(':')?;
    // Apple rounds to 3 decimals before f32: "0.97288" is stored as f32(0.973).
    let comps: Vec<f64> = rest
        .split(',')
        .map(|s| {
            let f: f64 = s.trim().parse().ok()?;
            let rounded = (f * 1000.0).round() / 1000.0;
            Some(rounded as f32 as f64)
        })
        .collect::<Option<Vec<_>>>()?;
    // Colorspace ids as Apple's RLOC blobs store them.
    let cspace = match name {
        "srgb" => 1,
        "gray" => 2,
        "display-p3" => 3,
        "extended-srgb" => 4,
        "extended-gray" => 6,
        _ => return None,
    };
    Some((cspace, comps))
}

/// Apple's palette for `fill: {"solid": "<spec>"}` (tagspaces): white anchor, the user colour,
/// and only the dark-mode background gradient.
fn solid_fill_assets(
    facet_prefix: &str,
    spec: &str,
) -> Option<(Vec<ColorAsset>, Vec<GradientAsset>)> {
    let (user_cspace, user_components) = parse_color_spec(spec)?;
    let g = |v: f32| v as f64;
    let mut colors = Vec::new();
    palette_add_color(&mut colors, facet_prefix, 6, vec![g(1.0), g(1.0)], 0);
    palette_add_color(&mut colors, facet_prefix, user_cspace, user_components, 0);
    let top = palette_add_color(&mut colors, facet_prefix, 2, vec![g(0.192), g(1.0)], 0);
    let bottom = palette_add_color(&mut colors, facet_prefix, 2, vec![g(0.078), g(1.0)], 0);
    let gradients = vec![GradientAsset {
        facet_name: format!("{facet_prefix}_Assets/Gradient-1"),
        geometry: [0.5, 0.0, 0.5, 1.0],
        stops: vec![(0.0, top), (1.0, bottom)],
        kind: 1,
    }];
    Some((colors, gradients))
}

/// Apple's palette for `fill: {"linear-gradient": [a, b]}` (classhub, recipe-scraper): the
/// automatic layout with the user stops, in their own colorspace, behind Gradient-1.
fn linear_gradient_fill_assets(
    facet_prefix: &str,
    stops_spec: &[&str],
    geometry: [f32; 4],
) -> Option<(Vec<ColorAsset>, Vec<GradientAsset>)> {
    if stops_spec.len() < 2 {
        return None;
    }
    let (cs0, c0) = parse_color_spec(stops_spec[0])?;
    let (cs1, c1) = parse_color_spec(stops_spec[1])?;
    let g = |v: f32| v as f64;
    let mut colors = Vec::new();
    palette_add_color(&mut colors, facet_prefix, 6, vec![g(1.0), g(1.0)], 0);
    let start = palette_add_color(&mut colors, facet_prefix, cs0, c0, 0);
    let end = palette_add_color(&mut colors, facet_prefix, cs1, c1, 0);
    let top = palette_add_color(&mut colors, facet_prefix, 2, vec![g(0.192), g(1.0)], 0);
    let bottom = palette_add_color(&mut colors, facet_prefix, 2, vec![g(0.078), g(1.0)], 0);
    let n = |s: &str| format!("{facet_prefix}_Assets/{s}");
    let gradients = vec![
        GradientAsset { facet_name: n("Gradient-1"), geometry, stops: vec![(0.0, start), (1.0, end)], kind: 1 },
        GradientAsset {
            facet_name: n("Gradient-2"),
            geometry: [0.5, 0.0, 0.5, 1.0],
            stops: vec![(0.0, top), (1.0, bottom)],
            kind: 1,
        },
    ];
    Some((colors, gradients))
}

/// Apple's palette for `fill: {"automatic-gradient": "<spec>"}` (ding_icon): the solid palette
/// plus a single-stop Gradient-1 of the user colour ahead of the dark background.
fn automatic_gradient_fill_assets(
    facet_prefix: &str,
    spec: &str,
) -> Option<(Vec<ColorAsset>, Vec<GradientAsset>)> {
    let (user_cspace, user_components) = parse_color_spec(spec)?;
    let g = |v: f32| v as f64;
    let mut colors = Vec::new();
    palette_add_color(&mut colors, facet_prefix, 6, vec![g(1.0), g(1.0)], 0);
    let user = palette_add_color(&mut colors, facet_prefix, user_cspace, user_components, 0);
    let top = palette_add_color(&mut colors, facet_prefix, 2, vec![g(0.192), g(1.0)], 0);
    let bottom = palette_add_color(&mut colors, facet_prefix, 2, vec![g(0.078), g(1.0)], 0);
    let n = |s: &str| format!("{facet_prefix}_Assets/{s}");
    let gradients = vec![
        GradientAsset { facet_name: n("Gradient-1"), geometry: [0.5, 0.0, 0.5, 1.0], stops: vec![(0.0, user)], kind: 0 },
        GradientAsset {
            facet_name: n("Gradient-2"),
            geometry: [0.5, 0.0, 0.5, 1.0],
            stops: vec![(0.0, top), (1.0, bottom)],
            kind: 1,
        },
    ];
    Some((colors, gradients))
}

/// Apple's palette for `fill: "system-dark"` (KYA): white anchor and two identical dark
/// gradients.
fn system_dark_fill_assets(facet_prefix: &str) -> (Vec<ColorAsset>, Vec<GradientAsset>) {
    let n = |s: &str| format!("{facet_prefix}_Assets/{s}");
    let g = |v: f32| (v as f32) as f64;
    let colors = vec![
        ColorAsset {
            facet_name: n("Color-1"),
            colorspace_id: 6,
            components: vec![g(1.0), g(1.0)],
        },
        ColorAsset {
            facet_name: n("Color-2"),
            colorspace_id: 2,
            components: vec![g(0.192), g(1.0)],
        },
        ColorAsset {
            facet_name: n("Color-3"),
            colorspace_id: 2,
            components: vec![g(0.078), g(1.0)],
        },
    ];
    let gradients = vec![
        GradientAsset {
            facet_name: n("Gradient-1"),
            geometry: [0.5, 0.0, 0.5, 1.0],
            stops: vec![(0.0, n("Color-2")), (1.0, n("Color-3"))],
            kind: 1,
        },
        GradientAsset {
            facet_name: n("Gradient-2"),
            geometry: [0.5, 0.0, 0.5, 1.0],
            stops: vec![(0.0, n("Color-2")), (1.0, n("Color-3"))],
            kind: 1,
        },
    ];
    (colors, gradients)
}


/// (top, bottom) background grays of a fill keyword; `automatic` follows the entry's appearance.
fn keyword_bg_pair(keyword: &str, appearance: Option<&str>) -> Option<(f64, f64)> {
    match keyword {
        "system-light" => Some((1.0, 0.925)),
        "system-dark" => Some((0.192, 0.078)),
        "automatic" => {
            if appearance == Some("dark") {
                Some((0.192, 0.078))
            } else {
                Some((1.0, 0.925))
            }
        }
        _ => None,
    }
}

/// Add a Color unless an equal one exists at `dedup_from` or later; returns its facet name. Apple
/// keeps gradient colours apart from equal base ones (transmission: Color-12, Color-4, both 0.078).
fn palette_add_color(
    colors: &mut Vec<ColorAsset>,
    facet_prefix: &str,
    colorspace_id: u32,
    components: Vec<f64>,
    dedup_from: usize,
) -> String {
    if let Some(existing) = colors[dedup_from.min(colors.len())..]
        .iter()
        .find(|c| c.colorspace_id == colorspace_id && c.components == components)
    {
        return existing.facet_name.clone();
    }
    let idx = colors.len() + 1;
    let name = format!("{facet_prefix}_Assets/Color-{idx}");
    colors.push(ColorAsset {
        facet_name: name.clone(),
        colorspace_id,
        components,
    });
    name
}

/// Add a linear Gradient unless an equal one exists (scrumdinger's redundant layer `automatic`
/// collapses here).
fn palette_add_gradient(
    gradients: &mut Vec<GradientAsset>,
    facet_prefix: &str,
    geometry: [f32; 4],
    stops: Vec<(f32, String)>,
    name: Option<&str>,
) {
    if gradients
        .iter()
        .any(|g| g.kind == 1 && g.geometry == geometry && g.stops == stops)
    {
        return;
    }
    let idx = gradients.len() + 1;
    let name = name.map_or_else(|| format!("Gradient-{idx}"), str::to_string);
    gradients.push(GradientAsset {
        facet_name: format!("{facet_prefix}_Assets/{name}"),
        geometry,
        stops,
        kind: 1,
    });
}

/// A gradient `orientation` as [start.x, start.y, stop.x, stop.y].
fn parse_orientation(obj: &serde_json::Map<String, serde_json::Value>) -> [f32; 4] {
    let default = [0.5f32, 0.0, 0.5, 1.0];
    let Some(orient) = obj.get("orientation") else {
        return default;
    };
    let pt = |key: &str, dx: f32, dy: f32| -> (f32, f32) {
        let p = orient.get(key);
        let x = p
            .and_then(|o| o.get("x"))
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(dx);
        let y = p
            .and_then(|o| o.get("y"))
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(dy);
        (x, y)
    };
    let (sx, sy) = pt("start", 0.5, 0.0);
    let (ex, ey) = pt("stop", 0.5, 1.0);
    [sx, sy, ex, ey]
}

/// Fold one fill `value` (keyword, `linear-gradient` or `solid`) into the palette.
fn process_fill_value(
    value: &serde_json::Value,
    appearance: Option<&str>,
    facet_prefix: &str,
    colors: &mut Vec<ColorAsset>,
    gradients: &mut Vec<GradientAsset>,
    grad_floor: usize,
) {
    let g = |v: f64| (v as f32) as f64;
    if let Some(s) = value.as_str() {
        if let Some((top, bottom)) = keyword_bg_pair(s, appearance) {
            let c0 = palette_add_color(colors, facet_prefix, 2, vec![g(top), g(1.0)], grad_floor);
            let c1 = palette_add_color(colors, facet_prefix, 2, vec![g(bottom), g(1.0)], grad_floor);
            // Named after the system background it resolves to, yet numbered with the others.
            let name = if top < 0.5 { "system-dark" } else { "system-light" };
            palette_add_gradient(
                gradients,
                facet_prefix,
                [0.5, 0.0, 0.5, 1.0],
                vec![(0.0, c0), (1.0, c1)],
                Some(name),
            );
        }
        return;
    }
    let Some(obj) = value.as_object() else {
        return;
    };
    if let Some(arr) = obj.get("linear-gradient").and_then(|x| x.as_array()) {
        let specs: Vec<&str> = arr.iter().filter_map(|x| x.as_str()).collect();
        if specs.len() < 2 {
            return;
        }
        let geometry = parse_orientation(obj);
        let mut names = Vec::with_capacity(specs.len());
        for spec in &specs {
            let Some((cspace, comps)) = parse_color_spec(spec) else {
                return;
            };
            names.push(palette_add_color(colors, facet_prefix, cspace, comps, grad_floor));
        }
        let last = (names.len() - 1) as f32;
        let stops: Vec<(f32, String)> = names
            .into_iter()
            .enumerate()
            .map(|(i, n)| (i as f32 / last, n))
            .collect();
        palette_add_gradient(gradients, facet_prefix, geometry, stops, None);
    } else if let Some(solid) = obj.get("solid").and_then(|x| x.as_str()) {
        // Solids deduplicate against the whole palette (recipe-scraper's layer
        // solid collapses onto its base gradient stop).
        if let Some((cspace, comps)) = parse_color_spec(solid) {
            palette_add_color(colors, facet_prefix, cspace, comps, 0);
        }
    }
}

/// Apple's palette for a top-level `fill-specializations`: the white anchor, then each
/// specialization and every layer's fills in document order (feishin, scrumdinger).
fn fill_specializations_assets(
    facet_prefix: &str,
    json: &IconJson,
) -> (Vec<ColorAsset>, Vec<GradientAsset>) {
    let g = |v: f32| (v as f32) as f64;
    let mut colors = vec![ColorAsset {
        facet_name: format!("{facet_prefix}_Assets/Color-1"),
        colorspace_id: 6,
        components: vec![g(1.0), g(1.0)],
    }];
    let mut gradients: Vec<GradientAsset> = Vec::new();
    // The whole top-spec palette is one fold, so gradient colours deduplicate
    // against everything (floor 0).
    if let Some(specs) = json.fill_specializations.as_ref() {
        for sp in specs {
            let appearance = sp.get("appearance").and_then(|a| a.as_str());
            if let Some(v) = sp.get("value") {
                process_fill_value(v, appearance, facet_prefix, &mut colors, &mut gradients, 0);
            }
        }
    }
    append_layer_fills(facet_prefix, json, &mut colors, &mut gradients, 0);
    (colors, gradients)
}

/// Fold every layer's `fill` and `fill-specializations` into the palette in document order, as
/// Apple emits per-layer gradients (transmission's ArrowLines / OuterEdge).
fn append_layer_fills(
    facet_prefix: &str,
    json: &IconJson,
    colors: &mut Vec<ColorAsset>,
    gradients: &mut Vec<GradientAsset>,
    grad_floor: usize,
) {
    for (_group, layer) in json.iter_layers() {
        if let Some(Fill::Structured(v)) = layer.fill.as_ref() {
            process_fill_value(v, None, facet_prefix, colors, gradients, grad_floor);
        }
        if let Some(specs) = layer.fill_specializations.as_ref() {
            for sp in specs {
                let appearance = sp.get("appearance").and_then(|a| a.as_str());
                if let Some(v) = sp.get("value") {
                    process_fill_value(v, appearance, facet_prefix, colors, gradients, grad_floor);
                }
            }
        }
    }
}

/// The Color/Gradient palette of the icon's fill; `None` for an unrecognized fill shape.
fn fill_assets(
    facet_prefix: &str,
    fill: Option<&Fill>,
    parsed: &IconJson,
) -> Option<(Vec<ColorAsset>, Vec<GradientAsset>)> {
    if parsed
        .fill_specializations
        .as_ref()
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        return Some(fill_specializations_assets(facet_prefix, parsed));
    }
    if fill_is_automatic(fill) {
        let (mut colors, mut gradients) = automatic_fill_assets(facet_prefix);
        let base = colors.len();
            append_layer_fills(facet_prefix, parsed, &mut colors, &mut gradients, base);
        return Some((colors, gradients));
    }
    let fill_val = fill?;
    match fill_val {
        Fill::Keyword(k) if k == "system-dark" => {
            let (mut colors, mut gradients) = system_dark_fill_assets(facet_prefix);
            let base = colors.len();
            append_layer_fills(facet_prefix, parsed, &mut colors, &mut gradients, base);
            Some((colors, gradients))
        }
        Fill::Structured(v) => {
            if let Some(spec) = v.get("solid").and_then(|s| s.as_str()) {
                let (mut colors, mut gradients) = solid_fill_assets(facet_prefix, spec)?;
                let base = colors.len();
            append_layer_fills(facet_prefix, parsed, &mut colors, &mut gradients, base);
                return Some((colors, gradients));
            }
            if let Some(spec) = v.get("automatic-gradient").and_then(|s| s.as_str()) {
                let (mut colors, mut gradients) =
                    automatic_gradient_fill_assets(facet_prefix, spec)?;
                let base = colors.len();
            append_layer_fills(facet_prefix, parsed, &mut colors, &mut gradients, base);
                return Some((colors, gradients));
            }
            if let Some(arr) = v.get("linear-gradient").and_then(|s| s.as_array()) {
                let specs: Vec<&str> = arr.iter().filter_map(|x| x.as_str()).collect();
                let (mut colors, mut gradients) =
                    linear_gradient_fill_assets(facet_prefix, &specs, parse_orientation(v.as_object()?))?;
                let base = colors.len();
            append_layer_fills(facet_prefix, parsed, &mut colors, &mut gradients, base);
                return Some((colors, gradients));
            }
            None
        }
        _ => None,
    }
}

/// The IconGroup rows for a group's visible image layers, resolved for `look`.
fn group_layer_refs(
    group: &crate::icon_json::Group,
    look: crate::icon_effects::Appearance,
    icon_name: &str,
    layer_assets: &[LayerAsset],
    colors: &[ColorAsset],
    gradients: &[GradientAsset],
) -> Vec<car::LayerRef> {
    let eff = crate::icon_effects::resolve_icon_effects(group, look);
    let (gscale, gt) = position_of(group.position.as_ref());
    let mut out = Vec::new();
    for (i, layer) in group.layers.iter().enumerate() {
        if layer.hidden == Some(true) {
            continue;
        }
        let Some(img) = layer.image_name.as_deref() else { continue };
        let stem = Path::new(img).file_stem().and_then(|s| s.to_str()).unwrap_or(img);
        let facet = format!("{icon_name}_Assets/{}", facet_component(stem));
        let Some(asset) = layer_assets.iter().find(|a| a.facet_name == facet) else { continue };
        let (nw, nh) = layer_native_size(&asset.source_path);
        let (lscale, lt) = position_of(layer.position.as_ref());
        let s = (gscale * lscale) as f64;
        let (w, h) = (nw as f64 * s, nh as f64 * s);
        let tx = (gscale * lt[0] + gt[0]) as f64;
        let ty = (gscale * lt[1] + gt[1]) as f64;
        let frame = [((1024.0 - w) / 2.0 + tx).floor(), ((1024.0 - h) / 2.0 + ty).floor(), w.floor(), h.floor()];
        let resolved = |specs: Option<&Vec<serde_json::Value>>| {
            specs.and_then(|s| crate::icon_effects::spec_value(s, look)).cloned()
        };
        let glass = resolved(layer.glass_specializations.as_ref())
            .or(layer.glass.map(serde_json::Value::Bool))
            .and_then(|v| v.as_bool());
        let fill = resolved(layer.fill_specializations.as_ref()).or_else(|| match layer.fill.as_ref() {
            Some(Fill::Structured(v)) => Some(v.clone()),
            _ => None,
        });
        out.push(car::LayerRef {
            part: car::PART_REGULAR,
            identifier: hash_name(&facet),
            frame: frame.map(|v| v as i32),
            opacity: eff.layers.get(i).map_or(1.0, |l| l.opacity),
            kind: (glass != Some(false)) as u32,
            fill: fill.and_then(|v| fill_facet(&v, colors, gradients)).unwrap_or_default(),
            ..Default::default()
        });
    }
    // Back to front, like the stack's groups.
    out.reverse();
    out
}

/// A `position` as (scale, translation in points).
fn position_of(p: Option<&crate::icon_json::Position>) -> (f32, [f32; 2]) {
    p.map(|p| (p.scale.unwrap_or(1.0), p.translation_in_points.unwrap_or([0.0, 0.0])))
        .unwrap_or((1.0, [0.0, 0.0]))
}

/// The palette facet of a top-level `solid` fill's colour.
fn solid_fill_facet(parsed: &IconJson, colors: &[ColorAsset]) -> Option<String> {
    let Some(Fill::Structured(v)) = parsed.fill.as_ref() else { return None };
    fill_facet(v, colors, &[])
}

/// The palette facet a fill value was folded into by [`process_fill_value`].
fn fill_facet(v: &serde_json::Value, colors: &[ColorAsset], gradients: &[GradientAsset]) -> Option<String> {
    let obj = v.as_object()?;
    let color = |spec: &str| {
        let (cs, comps) = parse_color_spec(spec)?;
        colors.iter().find(|c| c.colorspace_id == cs && c.components == comps).map(|c| c.facet_name.clone())
    };
    if let Some(solid) = obj.get("solid").and_then(|x| x.as_str()) {
        return color(solid);
    }
    let names: Vec<String> = obj
        .get("linear-gradient")?
        .as_array()?
        .iter()
        .map(|x| color(x.as_str()?))
        .collect::<Option<_>>()?;
    let geometry = parse_orientation(obj);
    gradients
        .iter()
        .find(|g| g.kind == 1 && g.geometry == geometry && g.stops.iter().map(|(_, n)| n).eq(names.iter()))
        .map(|g| g.facet_name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_rendered_name_matches_apple_pattern() {
        let n = pre_rendered_name("feishin", 16, 2, "NSAppearanceNameSystem");
        assert!(n.starts_with("feishin16x16_NSAppearanceNameSystem_"));
        assert!(n.ends_with(".png"));
        // Pattern: prefix + UUID(8-4-4-4-12) + - + 5 hex + - + 16 hex + .png
        let body = n
            .trim_start_matches("feishin16x16_NSAppearanceNameSystem_")
            .trim_end_matches(".png");
        let parts: Vec<&str> = body.split('-').collect();
        assert_eq!(parts.len(), 7, "expected UUID-PID-HEX shape, got {n:?}");
    }

    #[test]
    fn pre_rendered_name_is_deterministic() {
        let a = pre_rendered_name("Icon", 32, 2, "NSAppearanceNameSystem");
        let b = pre_rendered_name("Icon", 32, 2, "NSAppearanceNameSystem");
        assert_eq!(a, b);
    }

    #[test]
    fn pre_rendered_name_differs_by_size_and_scale() {
        let a = pre_rendered_name("Icon", 32, 1, "NSAppearanceNameSystem");
        let b = pre_rendered_name("Icon", 32, 2, "NSAppearanceNameSystem");
        let c = pre_rendered_name("Icon", 64, 2, "NSAppearanceNameSystem");
        assert_ne!(a, b);
        assert_ne!(b, c);
    }

    fn palette(json: &str) -> (Vec<ColorAsset>, Vec<GradientAsset>) {
        let parsed = crate::icon_json::IconJson::parse(json).unwrap();
        fill_specializations_assets("X", &parsed)
    }

    fn write_solid_png(path: &Path, rgba: [u8; 4]) {
        image::RgbaImage::from_pixel(64, 64, image::Rgba(rgba))
            .save(path)
            .unwrap();
    }

    #[test]
    fn glass_layer_becomes_desaturated_relief() {
        let dir = std::env::temp_dir().join(format!("glass_t_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let red = dir.join("red.png");
        write_solid_png(&red, [255, 0, 0, 255]);
        let layers = vec![StackLayer { source: red, frosted: true, tinted: false, specular: false, shadow_strength: 0.0, scale: 1.0, tx: 0.0, ty: 0.0, opacity: 1.0, blend: BlendMode::Normal, native_w: 64, native_h: 64, fill: None }];
        let out = render_layer_stack(&layers, 64, None).unwrap();
        let i = (32 * 64 + 32) * 4; // centre, premul-first BGRA
        let (b, g, r, a) = (out[i], out[i + 1], out[i + 2], out[i + 3]);
        assert!((5..70).contains(&a), "glass alpha should be low, got {a}");
        assert_eq!((b, g, r), (0, 0, 0), "colour stripped to premultiplied black");
    }

    #[test]
    fn non_glass_layer_keeps_colour() {
        let dir = std::env::temp_dir().join(format!("noglass_t_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let red = dir.join("red2.png");
        write_solid_png(&red, [255, 0, 0, 255]);
        let layers = vec![StackLayer { source: red, frosted: false, tinted: false, specular: false, shadow_strength: 0.0, scale: 1.0, tx: 0.0, ty: 0.0, opacity: 1.0, blend: BlendMode::Normal, native_w: 64, native_h: 64, fill: None }];
        let out = render_layer_stack(&layers, 64, None).unwrap();
        let i = (32 * 64 + 32) * 4; // premul-first BGRA, opaque red
        assert_eq!((out[i], out[i + 1], out[i + 2], out[i + 3]), (0, 0, 255, 255));
    }

    #[test]
    fn tinted_glass_subtracts_and_stacks() {
        let dir = std::env::temp_dir().join(format!("tint_t_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let blue = dir.join("blue.png");
        let red = dir.join("red3.png");
        write_solid_png(&blue, [0, 51, 229, 255]);
        write_solid_png(&red, [229, 25, 0, 255]);
        // Constant grey 0.6 gradient so the sampled background is ≈153 anywhere.
        let grad = crate::icon_render::GradientFill {
            start_rgb: [0.6, 0.6, 0.6],
            stop_rgb: [0.6, 0.6, 0.6],
            start: [0.5, 0.0],
            stop: [0.5, 1.0],
            shape: crate::icon_render::Shape::Squircle,
        };
        // Full-canvas glass, so the sampled centre is past the edge blur.
        let mk = |src: &Path, tinted: bool| StackLayer {
            source: src.to_path_buf(),
            frosted: true,
            tinted,
            specular: false,
            shadow_strength: 0.0,
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
            opacity: 1.0,
            blend: BlendMode::Normal,
            native_w: 1024,
            native_h: 1024,
            fill: None,
        };
        let i = (128 * 256 + 128) * 4; // centre at 256², premul-first BGRA
        // One tinted blue layer over grey 153: out = 153 − 63.5·(1−col).
        // R(col 0) ≈ 89, B(col 0.9) ≈ 147 — darkened, blue ordering preserved.
        let single = render_layer_stack(&[mk(&blue, true)], 256, Some(&grad)).unwrap();
        let (b1, g1, r1) = (single[i], single[i + 1], single[i + 2]);
        assert!(b1 > g1 && g1 > r1, "blue tint ordering B>G>R, got {r1},{g1},{b1}");
        assert!((80..100).contains(&r1) && (140..155).contains(&b1),
            "subtractive tint magnitude off: {r1},{g1},{b1}");
        let stacked =
            render_layer_stack(&[mk(&blue, true), mk(&red, true)], 256, Some(&grad)).unwrap();
        assert!(
            stacked[i] < b1 && stacked[i + 1] < g1 && stacked[i + 2] < r1,
            "overlap must be darker than either single tint on all channels"
        );
        let neutral = render_layer_stack(&[mk(&blue, false)], 256, Some(&grad)).unwrap();
        let (b, g, r) = (neutral[i], neutral[i + 1], neutral[i + 2]);
        assert!(
            b.abs_diff(g) < 6 && g.abs_diff(r) < 6 && g > 130,
            "untinted glass stays grey, got {r},{g},{b}"
        );
    }

    #[test]
    fn layer_keeps_native_aspect() {
        let dir = std::env::temp_dir().join(format!("aspect_t_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let red = dir.join("wide.png");
        write_solid_png(&red, [255, 0, 0, 255]);
        let layers = vec![StackLayer {
            source: red,
            frosted: false,
            tinted: false,
            specular: false,
            shadow_strength: 0.0,
            scale: crate::icon_render::Shape::Squircle.content_scale(),
            tx: 0.0,
            ty: 0.0,
            opacity: 1.0,
            blend: BlendMode::Normal,
            native_w: 256,
            native_h: 128,
            fill: None,
        }];
        let out = render_layer_stack(&layers, 1024, None).unwrap();
        let opaque = |x: usize, y: usize| out[(y * 1024 + x) * 4 + 3] > 0;
        let mut xs = (0, 0);
        let mut ys = (0, 0);
        for x in 0..1024 {
            if opaque(x, 512) {
                if xs.0 == 0 {
                    xs.0 = x;
                }
                xs.1 = x;
            }
        }
        for y in 0..1024 {
            if opaque(512, y) {
                if ys.0 == 0 {
                    ys.0 = y;
                }
                ys.1 = y;
            }
        }
        let aspect = (xs.1 - xs.0) as f32 / (ys.1 - ys.0) as f32;
        assert!((aspect - 2.0).abs() < 0.1, "expected ~2:1, got {aspect:.2}");
    }

    #[test]
    fn blend_channel_math() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(near(blend_channel(BlendMode::Screen, 0.5, 0.5), 0.75));
        assert!(near(blend_channel(BlendMode::Multiply, 0.5, 0.5), 0.25));
        assert!(near(blend_channel(BlendMode::Darken, 0.3, 0.7), 0.3));
        assert!(near(blend_channel(BlendMode::Lighten, 0.3, 0.7), 0.7));
        assert_eq!(blend_channel(BlendMode::Normal, 0.3, 0.7), 0.7);
        // soft-light leaves a 0.5 source as the backdrop (identity).
        assert!(near(blend_channel(BlendMode::SoftLight, 0.4, 0.5), 0.4));
    }

    #[test]
    fn composite_blend_normal_is_over() {
        let mut dst = [0, 0, 255, 255];
        composite_blend(&mut dst, &[255, 0, 0, 255], BlendMode::Normal);
        assert_eq!(dst, [255, 0, 0, 255]);
        // Half-alpha red over opaque blue → halfway.
        let mut dst2 = [0, 0, 255, 255];
        composite_blend(&mut dst2, &[255, 0, 0, 128], BlendMode::Normal);
        assert!((120..=135).contains(&dst2[0]) && (120..=135).contains(&dst2[2]));
    }

    #[test]
    fn composite_blend_screen_lightens() {
        let mut dst = [128, 128, 128, 255];
        composite_blend(&mut dst, &[128, 128, 128, 255], BlendMode::Screen);
        // screen(0.502, 0.502) ≈ 0.752 → ~192
        assert!((185..=198).contains(&dst[0]), "got {}", dst[0]);
    }

    #[test]
    fn fill_specializations_feishin_palette() {
        // Top-level custom p3 gradient + system-dark, then a layer with a
        // default solid, a dark gradient (whose 2nd stop dedups to Color-2),
        // and a tinted solid → Apple's 8 Colors / 3 Gradients.
        let (colors, gradients) = palette(
            r#"{
              "fill-specializations":[
                {"value":{"linear-gradient":[
                    "display-p3:0.87416,0.87416,0.87416,1.0",
                    "display-p3:0.99575,0.99575,0.99575,1.0"],
                  "orientation":{"start":{"x":0.5,"y":1},"stop":{"x":0.5,"y":0.3}}}},
                {"appearance":"dark","value":"system-dark"}
              ],
              "groups":[{"layers":[{"image-name":"f.svg","name":"f",
                "fill-specializations":[
                  {"value":{"solid":"extended-gray:0.0,1.0"}},
                  {"appearance":"dark","value":{"linear-gradient":[
                      "display-p3:0.78674,0.78674,0.78674,1.0",
                      "display-p3:0.87416,0.87416,0.87416,1.0"],
                    "orientation":{"start":{"x":0.5,"y":1},"stop":{"x":0.5,"y":0}}}},
                  {"appearance":"tinted","value":{"solid":"gray:1.0,1.0"}}
                ]}]}]
            }"#,
        );
        assert_eq!(colors.len(), 8, "feishin should have 8 colors");
        assert_eq!(gradients.len(), 3, "feishin should have 3 gradients");
        // Colorspaces: anchor(6), 2×p3(3), 2×gray(2), ext-gray(6), p3(3), gray(2)
        let cs: Vec<u32> = colors.iter().map(|c| c.colorspace_id).collect();
        assert_eq!(cs, vec![6, 3, 3, 2, 2, 6, 3, 2]);
        // The layer dark-gradient's second stop dedups onto Color-2.
        assert_eq!(gradients[2].stops[0].1, "X_Assets/Color-7");
        assert_eq!(gradients[2].stops[1].1, "X_Assets/Color-2");
        // Gradient-1 carries the JSON orientation (top→0.3), not the default.
        assert_eq!(gradients[0].geometry, [0.5, 1.0, 0.5, 0.3]);
    }

    #[test]
    fn fill_specializations_scrumdinger_palette() {
        // system-light + dark automatic, plus a redundant layer dark automatic
        // that dedups entirely → 5 Colors / 2 Gradients (no Gradient-3).
        let (colors, gradients) = palette(
            r#"{
              "fill-specializations":[
                {"value":"system-light"},
                {"appearance":"dark","value":"automatic"}
              ],
              "groups":[{"layers":[{"image-name":"1.png","name":"1",
                "fill-specializations":[{"appearance":"dark","value":"automatic"}]}]}]
            }"#,
        );
        assert_eq!(colors.len(), 5, "scrumdinger should have 5 colors");
        assert_eq!(gradients.len(), 2, "redundant layer automatic must dedup");
        let cs: Vec<u32> = colors.iter().map(|c| c.colorspace_id).collect();
        assert_eq!(cs, vec![6, 2, 2, 2, 2]);
    }
}

