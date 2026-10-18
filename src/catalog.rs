//! Parses an `.xcassets` catalog into renditions and facets.

use crate::car::{self, MultisizeImageEntry, Rendition};
use crate::name_hash::hash_name;
use crate::svg_raster;
use anyhow::{anyhow, Result};
use image::DynamicImage;
use indexmap::IndexMap;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const ICON_DIM2_POINTS: &[(u32, u16)] = &[
    (16, 1),
    (32, 2),
    (128, 3),
    (256, 4),
    (512, 5),
];

fn icon_dim2(point_w: u32) -> u16 {
    ICON_DIM2_POINTS
        .iter()
        .find(|(p, _)| *p == point_w)
        .map(|(_, d)| *d)
        .unwrap_or(0)
}

fn premultiply_bgra(mut buf: Vec<u8>) -> Vec<u8> {
    for chunk in buf.chunks_exact_mut(4) {
        let a = chunk[3];
        if a == 255 {
            continue;
        }
        if a == 0 {
            chunk[0] = 0;
            chunk[1] = 0;
            chunk[2] = 0;
        } else {
            chunk[0] = ((chunk[0] as u32 * a as u32 + 127) / 255) as u8;
            chunk[1] = ((chunk[1] as u32 * a as u32 + 127) / 255) as u8;
            chunk[2] = ((chunk[2] as u32 * a as u32 + 127) / 255) as u8;
        }
    }
    buf
}

fn premultiply_ga8(mut buf: Vec<u8>) -> Vec<u8> {
    for chunk in buf.chunks_exact_mut(2) {
        let a = chunk[1];
        if a == 255 {
            continue;
        }
        if a == 0 {
            chunk[0] = 0;
        } else {
            chunk[0] = ((chunk[0] as u32 * a as u32 + 127) / 255) as u8;
        }
    }
    buf
}

pub fn bgra_to_best_format(
    bgra_data: Vec<u8>,
    width: u32,
    height: u32,
    force_bgra: bool,
) -> (Vec<u8>, u32, u32, [u8; 4]) {
    if !force_bgra {
        let is_gray = bgra_data
            .chunks_exact(4)
            .all(|c| c[0] == c[1] && c[1] == c[2]);
        if is_gray {
            let mut ga = Vec::with_capacity((width * height * 2) as usize);
            for chunk in bgra_data.chunks_exact(4) {
                ga.push(chunk[0]);
                ga.push(chunk[3]);
            }
            return (ga, width, height, *b" 8AG");
        }
    }
    (bgra_data, width, height, *b"BGRA")
}

pub struct LoadedImage {
    pub pixel_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub pixel_format: [u8; 4],
    /// `color::Source::wide_gamut`.
    pub wide_gamut: Option<Vec<u8>>,
}

pub fn load_image_as_bgra(path: &Path, force_bgra: bool) -> Result<LoadedImage> {
    load_image_fitting(path, force_bgra, u32::MAX)
}

/// `load_image_as_bgra` through `color::open_fitting`.
pub fn load_image_fitting(path: &Path, force_bgra: bool, max_px: u32) -> Result<LoadedImage> {
    let crate::color::Source { image: img, wide_gamut } = crate::color::open_fitting(path, max_px)?;
    let (width, height) = (img.width(), img.height());
    let (pixel_data, pixel_format) = match &img {
        DynamicImage::ImageLuma8(luma) if !force_bgra => {
            // Grayscale → add alpha channel (fully opaque, no premultiply needed).
            let mut ga = Vec::with_capacity((width * height * 2) as usize);
            for &px in luma.as_raw() {
                ga.push(px);
                ga.push(0xFF);
            }
            (ga, *b" 8AG")
        }
        DynamicImage::ImageLumaA8(la) if !force_bgra => (premultiply_ga8(la.as_raw().clone()), *b" 8AG"),
        _ => {
            // Grayscale-compatible colour (R==G==B everywhere) is stored as GA8, whatever the
            // PNG's colour type (LiveContainer's RGB AppIconGrey).
            let rgba = img.to_rgba8();
            let raw = rgba.as_raw();
            if !force_bgra && raw.chunks_exact(4).all(|c| c[0] == c[1] && c[1] == c[2]) {
                let ga = raw.chunks_exact(4).flat_map(|c| [c[0], c[3]]).collect();
                (premultiply_ga8(ga), *b" 8AG")
            } else {
                let bgra = raw.chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect();
                (premultiply_bgra(bgra), *b"BGRA")
            }
        }
    };
    Ok(LoadedImage { pixel_data, width, height, pixel_format, wide_gamut })
}

fn parse_color_component(value: &str) -> f64 {
    if value.starts_with("0x") || value.starts_with("0X") {
        return i64::from_str_radix(&value[2..], 16).unwrap_or(0) as f64 / 255.0;
    }
    if value.chars().all(|c| c.is_ascii_digit() || c == '+' || c == '-') {
        let i: i64 = value.parse().unwrap_or(0);
        if i > 1 || i < 0 {
            return i as f64 / 255.0;
        }
        return i as f64;
    }
    let f: f64 = value.parse().unwrap_or(0.0);
    if f > 1.0 {
        return f / 255.0;
    }
    // Cast through f32 to match Apple's single-precision parsing.
    f as f32 as f64
}

fn colorspace_id_for_name(name: &str) -> u32 {
    match name {
        "srgb" => 1,
        "display-p3" => 3,
        "extended-srgb" => 4,
        "extended-linear-srgb" => 7,
        "gray-gamma-22" => 2,
        _ => 1,
    }
}

fn min_ga8_version(platform: &str) -> (u32, u32) {
    match platform {
        "macosx" => (10, 11),
        "iphoneos" | "appletvos" => (9, 0),
        "watchos" => (2, 0),
        _ => (10, 11),
    }
}

/// Deployment target from which actool embeds JPEGs in the CAR as DWAR raw data; below it,
/// it writes a loose file.
fn min_jpeg_car_version(platform: &str) -> (u32, u32) {
    match platform {
        "macosx" => (10, 10),
        "iphoneos" | "appletvos" => (9, 0),
        "watchos" => (2, 0),
        _ => (10, 10),
    }
}

pub struct AssetCatalog {
    pub path: PathBuf,
    pub platform: String,
    pub min_deploy: String,
    pub app_icon: Option<String>,
    pub include_languages: Option<Vec<String>>,
    pub development_region: Option<String>,
    identifiers: IndexMap<String, u16>,
    locales_used: HashSet<String>,
    force_bgra: bool,
    /// `(imageset_stem, source_path)` of JPEGs written as loose files (deployment target below
    /// `min_jpeg_car_version`).
    pub loose_jpegs: Vec<(String, PathBuf)>,
    jpeg_in_car: bool,
    /// actool `--target-device` values; a universal iOS app icon is stored once for each.
    pub target_devices: Vec<String>,
    pub include_all_app_icons: bool,
    /// App icon sets shadowed by an `.icon` document of the same name.
    pub skipped_app_icons: Vec<String>,
    pub app_icon_sets: Vec<AppIconSet>,
}

#[derive(Debug, Clone)]
pub struct Facet {
    pub element: u16,
    pub part: Option<u16>,
    pub identifier: u16,
}

/// One image entry from an `.appiconset`, retaining the metadata needed to
/// build the iOS partial plist and loose home-screen PNGs.
#[derive(Debug, Clone)]
pub struct IconImage {
    pub idiom: String,
    pub point_w: u32,
    pub scale: u32,
    pub appearance: u16,
    pub path: PathBuf,
}

/// A compiled `.appiconset`: the `--app-icon` one, or an alternate under
/// `--include-all-app-icons`.
#[derive(Debug, Clone)]
pub struct AppIconSet {
    pub name: String,
    pub primary: bool,
    pub images: Vec<IconImage>,
}

impl AssetCatalog {
    pub fn new(
        path: PathBuf,
        platform: String,
        min_deploy: String,
        app_icon: Option<String>,
        include_languages: Option<Vec<String>>,
        development_region: Option<String>,
    ) -> Self {
        let deploy = {
            let mut parts = min_deploy.split('.');
            let a: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let b: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            (a, b)
        };
        let force_bgra = deploy < min_ga8_version(&platform);
        let jpeg_in_car = deploy >= min_jpeg_car_version(&platform);
        Self {
            path,
            platform,
            min_deploy,
            app_icon,
            include_languages,
            development_region,
            identifiers: IndexMap::new(),
            locales_used: HashSet::new(),
            force_bgra,
            loose_jpegs: Vec::new(),
            jpeg_in_car,
            target_devices: Vec::new(),
            include_all_app_icons: false,
            skipped_app_icons: Vec::new(),
            app_icon_sets: Vec::new(),
        }
    }

    pub fn get_locales_used(&self) -> Vec<String> {
        let mut v: Vec<_> = self.locales_used.iter().cloned().collect();
        v.sort();
        v
    }

    fn raster_scales(&self) -> &'static [u16] {
        if car::is_idiom_platform(&self.platform) { &[1, 2, 3] } else { &[1, 2] }
    }

    fn get_identifier(&mut self, name: &str) -> u16 {
        if let Some(v) = self.identifiers.get(name) {
            return *v;
        }
        let id = hash_name(name);
        self.identifiers.insert(name.to_string(), id);
        id
    }

    fn should_include_locale(&self, locale: &str) -> bool {
        let Some(filter) = &self.include_languages else {
            return true;
        };
        if let Some(dev) = &self.development_region {
            if locale == dev {
                return true;
            }
        }
        filter.iter().any(|s| s == locale)
    }

    pub fn parse(&mut self) -> Result<(Vec<Rendition>, IndexMap<String, Facet>)> {
        if !self.path.exists() {
            return Err(anyhow!("Asset catalog not found: {}", self.path.display()));
        }
        let mut renditions = Vec::new();
        let mut facets = IndexMap::new();
        let root = self.path.clone();
        self.parse_directory(&root, &mut renditions, &mut facets, "")?;
        Ok((renditions, facets))
    }

    fn parse_directory(
        &mut self,
        dir: &Path,
        renditions: &mut Vec<Rendition>,
        facets: &mut IndexMap<String, Facet>,
        namespace: &str,
    ) -> Result<()> {
        let mut entries: Vec<PathBuf> = match fs::read_dir(dir) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => return Ok(()),
        };
        entries.sort();
        for item in entries {
            let ext = item
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            match ext.as_str() {
                "imageset" => self.parse_imageset(&item, renditions, facets, namespace)?,
                "appiconset" => self.parse_appiconset(&item, renditions, facets)?,
                "iconset" => self.parse_iconset(&item, renditions, facets, namespace)?,
                "colorset" => self.parse_colorset(&item, renditions, facets, namespace)?,
                "dataset" => self.parse_dataset(&item, renditions, facets, namespace)?,
                "spriteatlas" => self.parse_spriteatlas(&item, renditions, facets)?,
                "imagestack" => self.parse_imagestack(&item, renditions, facets)?,
                "" if item.is_dir() => {
                    let mut child_ns = namespace.to_string();
                    let group_json = item.join("Contents.json");
                    if group_json.exists() {
                        if let Ok(v) = read_json(&group_json) {
                            if v.get("properties")
                                .and_then(|p| p.get("provides-namespace"))
                                .and_then(|x| x.as_bool())
                                .unwrap_or(false)
                            {
                                let name =
                                    item.file_name().unwrap_or_default().to_string_lossy();
                                child_ns = if namespace.is_empty() {
                                    format!("{name}/")
                                } else {
                                    format!("{namespace}{name}/")
                                };
                            }
                        }
                    }
                    self.parse_directory(&item, renditions, facets, &child_ns)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn parse_imageset(
        &mut self,
        item: &Path,
        renditions: &mut Vec<Rendition>,
        facets: &mut IndexMap<String, Facet>,
        namespace: &str,
    ) -> Result<()> {
        let name = item.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let facet_name = format!("{namespace}{name}");
        let ident = self.get_identifier(&facet_name);
        let count_before = renditions.len();

        let contents_path = item.join("Contents.json");
        if !contents_path.exists() {
            return Ok(());
        }
        let contents = read_json(&contents_path)?;
        let intent_str = contents
            .get("properties")
            .and_then(|p| p.get("template-rendering-intent"))
            .and_then(|v| v.as_str());
        let template_intent: i32 = match intent_str {
            Some("original") => 0,
            Some("template") => 2,
            _ => 4,
        };

        let images = contents
            .get("images")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for img_info in &images {
            let Some(filename) = img_info.get("filename").and_then(|v| v.as_str()) else {
                continue;
            };
            let img_path = entry_path(item, filename);
            if !img_path.exists() {
                continue;
            }
            let scale = parse_scale(img_info);
            let idiom = img_info
                .get("idiom")
                .and_then(|v| v.as_str())
                .unwrap_or("universal");
            // `ios-marketing` is an app-icon-only idiom; imagesets drop it.
            let idiom_applies = match self.platform.as_str() {
                "macosx" => idiom == "mac" || idiom == "universal",
                "iphoneos" | "iphonesimulator" => {
                    matches!(idiom, "iphone" | "ipad" | "universal")
                }
                _ => true,
            };
            if !idiom_applies {
                continue;
            }
            if self.platform == "macosx" && scale > 2 {
                continue;
            }
            // macOS keys omit the idiom attribute (15).
            let idiom_num = if car::is_idiom_platform(&self.platform) {
                car::idiom_value(idiom)
            } else {
                0
            };

            let locale = img_info
                .get("locale")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !locale.is_empty() && !self.should_include_locale(&locale) {
                continue;
            }
            if !locale.is_empty() {
                self.locales_used.insert(locale.clone());
            }

            let direction = img_info
                .get("language-direction")
                .and_then(|v| v.as_str())
                .map(|s| match s {
                    "left-to-right" => car::DIRECTION_LTR,
                    "right-to-left" => car::DIRECTION_RTL,
                    _ => car::DIRECTION_DEFAULT,
                })
                .unwrap_or(car::DIRECTION_DEFAULT);
            let appearance = appearance_from_json(img_info);

            let img_rend_start = renditions.len();
            let lower_filename = filename.to_ascii_lowercase();
            if lower_filename.ends_with(".pdf") {
                // Apple stores the PDF re-serialized through CoreGraphics (a
                // compact, normalized rewrite — iina's 167 KB design PDFs come
                // back ~3.8 KB); accord stores the source.
                let pdf_data = fs::read(&img_path)?;
                let csi = car::build_pdf_csi(filename, &pdf_data);
                let mut rend = Rendition {
                    name: filename.to_string(),
                    identifier: ident,
                    element: car::ELEMENT_UNIVERSAL,
                    part: car::PART_REGULAR,
                    scale: 0,
                    appearance,
                    direction,
                    layout: car::LAYOUT_PDF,
                    pixel_format: *car::PIXELFMT_PDF,
                    template_rendering_intent: template_intent,
                    min_deploy: self.min_deploy.clone(),
                    platform: self.platform.clone(),
                    ..Rendition::default()
                };
                rend.csi_override = Some(csi);
                renditions.push(rend);

                // actool also emits a raster of a PDF imageset at each of the platform's scales.
                for &scale in self.raster_scales() {
                    let scale = scale as u32;
                    let raster = crate::pdf_raster::rasterize_pdf(&pdf_data, scale)?;
                    // Chroma-free rasters become ` 8AG`, like Apple's PDF rasterization.
                    let (pd, pw, ph, pf) = bgra_to_best_format(
                        raster.bgra,
                        raster.width,
                        raster.height,
                        self.force_bgra,
                    );
                    let cs_id = if &pf == b" 8AG" { 2 } else { 1 };
                    renditions.push(Rendition {
                        name: filename.to_string(),
                        identifier: ident,
                        element: car::ELEMENT_UNIVERSAL,
                        part: car::PART_REGULAR,
                        scale: scale as u16,
                        appearance,
                        direction,
                        layout: car::LAYOUT_ONE_PART_SCALE,
                        pixel_format: pf,
                        width: pw,
                        height: ph,
                        pixel_data: pd,
                        template_rendering_intent: template_intent,
                        is_svg_rasterization: true,
                        colorspace_id: cs_id,
                        min_deploy: self.min_deploy.clone(),
                        platform: self.platform.clone(),
                        ..Rendition::default()
                    });
                }
                stamp_idiom(renditions, img_rend_start, idiom_num);
                continue;
            }

            if lower_filename.ends_with(".jpg") || lower_filename.ends_with(".jpeg") {
                if self.jpeg_in_car {
                    let jpeg_data = fs::read(&img_path)?;
                    let csi = car::build_jpeg_csi(filename, &jpeg_data);
                    let mut rend = Rendition {
                        name: filename.to_string(),
                        identifier: ident,
                        element: car::ELEMENT_UNIVERSAL,
                        part: car::PART_REGULAR,
                        scale: scale as u16,
                        appearance,
                        direction,
                        layout: car::LAYOUT_ONE_PART_SCALE,
                        pixel_format: *car::PIXELFMT_JPEG,
                        template_rendering_intent: template_intent,
                        locale: locale.clone(),
                        colorspace_id: 0,
                        min_deploy: self.min_deploy.clone(),
                        platform: self.platform.clone(),
                        ..Rendition::default()
                    };
                    rend.csi_override = Some(csi);
                    renditions.push(rend);
                } else {
                    self.loose_jpegs.push((name.clone(), img_path.clone()));
                }
                stamp_idiom(renditions, img_rend_start, idiom_num);
                continue;
            }

            if lower_filename.ends_with(".svg") {
                let svg_data = fs::read(&img_path)?;
                let csi = car::build_svg_csi(filename, &svg_data);
                let mut vec_rend = Rendition {
                    name: filename.to_string(),
                    identifier: ident,
                    element: car::ELEMENT_UNIVERSAL,
                    part: car::PART_REGULAR,
                    scale: 1,
                    appearance,
                    direction,
                    layout: car::LAYOUT_PDF,
                    pixel_format: *car::PIXELFMT_SVG,
                    min_deploy: self.min_deploy.clone(),
                    platform: self.platform.clone(),
                    ..Rendition::default()
                };
                vec_rend.csi_override = Some(csi);
                renditions.push(vec_rend);

                let (svg_w, svg_h) = svg_raster::parse_svg_dimensions(&svg_data);
                if svg_w > 0 && svg_h > 0 {
                    for &raster_scale in self.raster_scales() {
                        let pixel_data = svg_raster::rasterize_svg(
                            &svg_data,
                            svg_w,
                            svg_h,
                            raster_scale as u32,
                        )?;
                        let pw = svg_w * raster_scale as u32;
                        let ph = svg_h * raster_scale as u32;
                        let (pd, pw, ph, pf) =
                            bgra_to_best_format(pixel_data, pw, ph, self.force_bgra);
                        let cs_id = if &pf == b" 8AG" { 2 } else { 1 };
                        renditions.push(Rendition {
                            name: filename.to_string(),
                            identifier: ident,
                            element: car::ELEMENT_UNIVERSAL,
                            part: car::PART_REGULAR,
                            scale: raster_scale,
                            width: pw,
                            height: ph,
                            pixel_data: pd,
                            pixel_format: pf,
                            appearance,
                            direction,
                            layout: car::LAYOUT_ONE_PART_SCALE,
                            template_rendering_intent: template_intent,
                            is_svg_rasterization: true,
                            locale: locale.clone(),
                            colorspace_id: cs_id,
                            min_deploy: self.min_deploy.clone(),
                            platform: self.platform.clone(),
                            ..Rendition::default()
                        });
                    }
                }
                stamp_idiom(renditions, img_rend_start, idiom_num);
                continue;
            }

            let img = load_image_as_bgra(&img_path, self.force_bgra)?;
            renditions.push(Rendition {
                name: filename.to_string(),
                identifier: ident,
                element: car::ELEMENT_UNIVERSAL,
                part: car::PART_REGULAR,
                scale: scale as u16,
                width: img.width,
                height: img.height,
                colorspace_id: car::colorspace_for_pixel_format(&img.pixel_format),
                pixel_data: img.pixel_data,
                pixel_format: img.pixel_format,
                wide_gamut: img.wide_gamut,
                appearance,
                direction,
                layout: car::LAYOUT_ONE_PART_SCALE,
                template_rendering_intent: template_intent,
                locale,
                min_deploy: self.min_deploy.clone(),
                platform: self.platform.clone(),
                ..Rendition::default()
            });
            stamp_idiom(renditions, img_rend_start, idiom_num);
        }
        if count_before < renditions.len() {
            facets.insert(
                facet_name,
                Facet {
                    element: car::ELEMENT_UNIVERSAL,
                    part: Some(car::PART_REGULAR),
                    identifier: ident,
                },
            );
        }
        Ok(())
    }

    fn parse_appiconset(
        &mut self,
        item: &Path,
        renditions: &mut Vec<Rendition>,
        facets: &mut IndexMap<String, Facet>,
    ) -> Result<()> {
        let name = item.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let primary = self.app_icon.as_deref() == Some(name.as_str());
        if (!primary && !self.include_all_app_icons) || self.skipped_app_icons.contains(&name) {
            return Ok(());
        }
        let ident = self.get_identifier(&name);
        let contents_path = item.join("Contents.json");
        if !contents_path.exists() {
            return Ok(());
        }
        let contents = read_json(&contents_path)?;
        let images = contents
            .get("images")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut icons: Vec<IconImage> = Vec::new();
        for img_info in &images {
            let Some(filename) = img_info.get("filename").and_then(|v| v.as_str()) else {
                continue;
            };
            let path = entry_path(item, filename);
            if !path.exists() {
                continue;
            }
            let img_platform = img_info.get("platform").and_then(|v| v.as_str()).unwrap_or("");
            if !img_platform.is_empty() && img_platform != json_platform(&self.platform) {
                continue;
            }
            let size_str = img_info.get("size").and_then(|v| v.as_str()).unwrap_or("");
            // Floor handles fractional iPad sizes ("83.5x83.5" -> 83).
            let point_w: u32 = size_str
                .split_once('x')
                .and_then(|(w, _)| w.parse::<f64>().ok())
                .map(|f| f.floor() as u32)
                .unwrap_or(0);
            let idiom = img_info.get("idiom").and_then(|v| v.as_str()).unwrap_or("universal");
            // iOS takes universal, marketing and the target devices' icons; watch, mac and
            // other idioms are other platforms' and take no Dimension2 slot.
            if car::is_idiom_platform(&self.platform)
                && !matches!(idiom, "universal" | "ios-marketing")
                && !(if self.target_devices.is_empty() {
                    matches!(idiom, "iphone" | "ipad")
                } else {
                    self.target_devices.iter().any(|d| d == idiom)
                })
            {
                continue;
            }
            // A universal (single-size) iOS icon is stored once per target device.
            let idioms: Vec<String> = if idiom == "universal" && car::is_idiom_platform(&self.platform) {
                if self.target_devices.is_empty() {
                    vec![idiom.to_string()]
                } else {
                    self.target_devices.clone()
                }
            } else {
                vec![idiom.to_string()]
            };
            for idiom in idioms {
                icons.push(IconImage {
                    idiom,
                    point_w,
                    scale: parse_scale(img_info),
                    appearance: appearance_from_json(img_info),
                    path: path.clone(),
                });
            }
        }
        if icons.is_empty() {
            return Ok(());
        }

        // iPhone Plus/Max: actool synthesizes a 90pt@2x icon (subtype 1792) from the 60pt@3x one
        // (both 180 px), with its own phone/subtype-1792 multisize facet.
        let synth_90 = car::is_idiom_platform(&self.platform)
            && icons.iter().any(|i| i.idiom == "iphone" && i.point_w == 60 && i.scale == 3);
        // Dimension2 is a size's rank among all sizes of the set: a full iOS set maps
        // 20, 29, 40, 60, 76, 83.5, 90, 1024 to 1..8, a single-size set 1024 to 1.
        let mut sizes: Vec<u32> = icons.iter().map(|i| i.point_w).collect();
        if synth_90 {
            sizes.push(90);
        }
        sizes.sort_unstable();
        sizes.dedup();
        let dim2_of = |w: u32| sizes.iter().position(|&s| s == w).map_or(0, |p| p as u16 + 1);

        let appicon_start = renditions.len();
        let mut icon_renditions: Vec<(u32, u16, u16)> = Vec::new(); // (point_w, idiom, subtype)
        for icon in &icons {
            let filename = icon.path.file_name().unwrap_or_default().to_string_lossy().to_string();
            // Per-device icon lookups on iOS resolve by idiom.
            let idiom_num = if car::is_idiom_platform(&self.platform) { car::idiom_value(&icon.idiom) } else { 0 };
            let icon_rend_start = renditions.len();
            let dim2 = dim2_of(icon.point_w);

            if filename.to_ascii_lowercase().ends_with(".svg") {
                let svg_data = fs::read(&icon.path)?;
                let csi = car::build_svg_csi(&filename, &svg_data);
                let mut vec_rend = Rendition {
                    name: filename.clone(),
                    identifier: ident,
                    element: car::ELEMENT_UNIVERSAL,
                    part: car::PART_ICON,
                    scale: 1,
                    dim2,
                    appearance: icon.appearance,
                    layout: car::LAYOUT_PDF,
                    pixel_format: *car::PIXELFMT_SVG,
                    min_deploy: self.min_deploy.clone(),
                    platform: self.platform.clone(),
                    ..Rendition::default()
                };
                vec_rend.csi_override = Some(csi);
                renditions.push(vec_rend);

                if icon.point_w > 0 {
                    let (mut svg_w, mut svg_h) = svg_raster::parse_svg_dimensions(&svg_data);
                    if svg_w == 0 {
                        svg_w = icon.point_w;
                        svg_h = icon.point_w;
                    }
                    for &raster_scale in self.raster_scales() {
                        let pixel_data =
                            svg_raster::rasterize_svg(&svg_data, svg_w, svg_h, raster_scale as u32)?;
                        let pw = svg_w * raster_scale as u32;
                        let ph = svg_h * raster_scale as u32;
                        let (pd, pw, ph, pf) = bgra_to_best_format(pixel_data, pw, ph, self.force_bgra);
                        let cs_id = if &pf == b" 8AG" { 2 } else { 1 };
                        renditions.push(Rendition {
                            name: filename.clone(),
                            identifier: ident,
                            element: car::ELEMENT_UNIVERSAL,
                            part: car::PART_ICON,
                            scale: raster_scale,
                            width: pw,
                            height: ph,
                            pixel_data: pd,
                            pixel_format: pf,
                            layout: car::LAYOUT_ONE_PART_SCALE,
                            dim2,
                            appearance: icon.appearance,
                            template_rendering_intent: 0,
                            is_svg_rasterization: true,
                            colorspace_id: cs_id,
                            min_deploy: self.min_deploy.clone(),
                            platform: self.platform.clone(),
                            ..Rendition::default()
                        });
                        icon_renditions.push((icon.point_w, idiom_num, 0));
                    }
                }
                stamp_idiom(renditions, icon_rend_start, idiom_num);
                continue;
            }

            let img = load_image_as_bgra(&icon.path, self.force_bgra)?;
            renditions.push(Rendition {
                name: filename,
                identifier: ident,
                element: car::ELEMENT_UNIVERSAL,
                part: car::PART_ICON,
                scale: icon.scale as u16,
                width: img.width,
                height: img.height,
                colorspace_id: car::colorspace_for_pixel_format(&img.pixel_format),
                pixel_data: img.pixel_data,
                pixel_format: img.pixel_format,
                wide_gamut: img.wide_gamut,
                layout: car::LAYOUT_ONE_PART_SCALE,
                dim2,
                appearance: icon.appearance,
                template_rendering_intent: 0,
                min_deploy: self.min_deploy.clone(),
                platform: self.platform.clone(),
                ..Rendition::default()
            });
            stamp_idiom(renditions, icon_rend_start, idiom_num);
            icon_renditions.push((icon.point_w, idiom_num, 0));
        }

        if synth_90 {
            // One per appearance the 60pt@3x icon has.
            let sources: Vec<Rendition> = renditions[appicon_start..]
                .iter()
                .filter(|r| {
                    r.idiom == car::idiom_value("iphone")
                        && r.scale == 3
                        && r.dim2 == dim2_of(60)
                        && r.part == car::PART_ICON
                })
                .cloned()
                .collect();
            for mut synth in sources {
                synth.scale = 2;
                synth.subtype = 1792;
                synth.dim2 = dim2_of(90);
                renditions.push(synth);
                icon_renditions.push((90, car::idiom_value("iphone"), 1792));
            }
        }

        // One MultiSized rendition per (idiom, subtype), key attributes 15 and 16; macOS has a
        // single one with both 0.
        let mut sizes_by_group: IndexMap<(u16, u16), Vec<u32>> = IndexMap::new();
        for (point_w, idiom, subtype) in &icon_renditions {
            let sizes = sizes_by_group.entry((*idiom, *subtype)).or_default();
            if !sizes.contains(point_w) {
                sizes.push(*point_w);
            }
        }
        let mut groups: Vec<(u16, u16)> = sizes_by_group.keys().copied().collect();
        groups.sort_unstable();
        for (idiom, subtype) in groups {
            let mut sizes = sizes_by_group.shift_remove(&(idiom, subtype)).unwrap();
            sizes.sort_unstable();
            let ms_entries: Vec<MultisizeImageEntry> = sizes
                .iter()
                .map(|w| MultisizeImageEntry { width: *w, height: *w, index: dim2_of(*w) as u32 })
                .collect();
            let mut ms_rend = car::build_multisize_rendition(&name, ident, &ms_entries);
            ms_rend.idiom = idiom;
            ms_rend.subtype = subtype;
            renditions.push(ms_rend);
        }

        facets.insert(
            name.clone(),
            Facet {
                element: car::ELEMENT_UNIVERSAL,
                part: Some(car::PART_ICON),
                identifier: ident,
            },
        );
        self.app_icon_sets.push(AppIconSet { name, primary, images: icons });
        Ok(())
    }

    fn parse_iconset(
        &mut self,
        item: &Path,
        renditions: &mut Vec<Rendition>,
        facets: &mut IndexMap<String, Facet>,
        namespace: &str,
    ) -> Result<()> {
        let name = item.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let facet_name = format!("{namespace}{name}");
        let ident = self.get_identifier(&facet_name);
        let re = regex::Regex::new(r"^icon_(\d+)x(\d+)(?:@(\d+)x)?\.png$").unwrap();
        let mut icon_renditions: Vec<u32> = Vec::new();
        let mut entries: Vec<PathBuf> = match fs::read_dir(item) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => return Ok(()),
        };
        entries.sort();
        for img_file in entries {
            if !img_file.is_file() {
                continue;
            }
            let fname = img_file.file_name().unwrap().to_string_lossy().to_string();
            let Some(cap) = re.captures(&fname) else {
                continue;
            };
            let point_w: u32 = cap[1].parse().unwrap_or(0);
            let scale: u32 = cap
                .get(3)
                .map(|m| m.as_str().parse().unwrap_or(1))
                .unwrap_or(1);
            if self.platform == "macosx" && scale > 2 {
                continue;
            }
            let img = load_image_as_bgra(&img_file, self.force_bgra)?;
            let dim2 = icon_dim2(point_w);
            renditions.push(Rendition {
                name: fname,
                identifier: ident,
                element: car::ELEMENT_UNIVERSAL,
                part: car::PART_ICON,
                scale: scale as u16,
                width: img.width,
                height: img.height,
                colorspace_id: car::colorspace_for_pixel_format(&img.pixel_format),
                pixel_data: img.pixel_data,
                pixel_format: img.pixel_format,
                wide_gamut: img.wide_gamut,
                layout: car::LAYOUT_ONE_PART_SCALE,
                dim2,
                template_rendering_intent: 0,
                min_deploy: self.min_deploy.clone(),
                platform: self.platform.clone(),
                ..Rendition::default()
            });
            icon_renditions.push(point_w);
        }
        if icon_renditions.is_empty() {
            return Ok(());
        }

        let mut ms_entries: Vec<MultisizeImageEntry> = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();
        for pw in &icon_renditions {
            if seen.insert(*pw) {
                ms_entries.push(MultisizeImageEntry {
                    width: *pw,
                    height: *pw,
                    index: icon_dim2(*pw) as u32,
                });
            }
        }
        // Apple orders by point size; the file sort above is lexical (128 before 16).
        ms_entries.sort_by_key(|e| e.width);
        let ms_rend = car::build_multisize_rendition(&name, ident, &ms_entries);
        renditions.push(ms_rend);

        facets.insert(
            facet_name,
            Facet {
                element: car::ELEMENT_UNIVERSAL,
                part: Some(car::PART_ICON),
                identifier: ident,
            },
        );
        Ok(())
    }

    fn parse_colorset(
        &mut self,
        item: &Path,
        renditions: &mut Vec<Rendition>,
        facets: &mut IndexMap<String, Facet>,
        namespace: &str,
    ) -> Result<()> {
        let name = item.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let facet_name = format!("{namespace}{name}");
        let ident = self.get_identifier(&facet_name);
        let contents_path = item.join("Contents.json");
        if !contents_path.exists() {
            return Ok(());
        }
        let contents = read_json(&contents_path)?;
        let mut added = false;
        if let Some(arr) = contents.get("colors").and_then(|v| v.as_array()) {
            for entry in arr {
                let color = entry.get("color").cloned().unwrap_or(Value::Null);
                let components = color.get("components").cloned().unwrap_or(Value::Null);
                if components.is_null() {
                    continue;
                }
                let r = parse_color_component(
                    components.get("red").and_then(|v| v.as_str()).unwrap_or("0"),
                );
                let g = parse_color_component(
                    components.get("green").and_then(|v| v.as_str()).unwrap_or("0"),
                );
                let b = parse_color_component(
                    components.get("blue").and_then(|v| v.as_str()).unwrap_or("0"),
                );
                let a = parse_color_component(
                    components.get("alpha").and_then(|v| v.as_str()).unwrap_or("1"),
                );
                let cs_name = color
                    .get("color-space")
                    .and_then(|v| v.as_str())
                    .unwrap_or("srgb");
                let cs_id = colorspace_id_for_name(cs_name);
                let appearance = appearance_from_json(entry);

                let csi = car::build_color_csi(&name, r, g, b, a, cs_id);
                let mut rend = Rendition {
                    name: name.clone(),
                    identifier: ident,
                    element: car::ELEMENT_UNIVERSAL,
                    part: car::PART_COLOR,
                    scale: 1,
                    appearance,
                    layout: car::LAYOUT_COLOR,
                    pixel_format: [0, 0, 0, 0],
                    colorspace_id: 0,
                    min_deploy: self.min_deploy.clone(),
                    platform: self.platform.clone(),
                    ..Rendition::default()
                };
                rend.csi_override = Some(csi);
                renditions.push(rend);
                added = true;
            }
        }
        if added {
            facets.insert(
                facet_name,
                Facet {
                    element: car::ELEMENT_UNIVERSAL,
                    part: Some(car::PART_COLOR),
                    identifier: ident,
                },
            );
        }
        Ok(())
    }

    fn parse_dataset(
        &mut self,
        item: &Path,
        renditions: &mut Vec<Rendition>,
        facets: &mut IndexMap<String, Facet>,
        namespace: &str,
    ) -> Result<()> {
        let name = item.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let facet_name = format!("{namespace}{name}");
        let ident = self.get_identifier(&facet_name);
        let contents_path = item.join("Contents.json");
        if !contents_path.exists() {
            return Ok(());
        }
        let contents = read_json(&contents_path)?;
        if let Some(arr) = contents.get("data").and_then(|v| v.as_array()) {
            for entry in arr {
                let Some(filename) = entry.get("filename").and_then(|v| v.as_str()) else {
                    continue;
                };
                let p = entry_path(item, filename);
                if !p.exists() {
                    continue;
                }
                let raw = fs::read(&p)?;
                let uti = entry.get("universal-type-identifier").and_then(|v| v.as_str());
                let csi = car::build_data_csi(&raw, uti);
                let mut rend = Rendition {
                    name: "CoreStructuredImage".to_string(),
                    identifier: ident,
                    element: car::ELEMENT_UNIVERSAL,
                    part: car::PART_REGULAR,
                    scale: 1,
                    layout: car::LAYOUT_RAW_DATA,
                    pixel_format: *car::PIXELFMT_DATA,
                    min_deploy: self.min_deploy.clone(),
                    platform: self.platform.clone(),
                    ..Rendition::default()
                };
                rend.csi_override = Some(csi);
                renditions.push(rend);
            }
        }
        facets.insert(
            facet_name,
            Facet {
                element: car::ELEMENT_UNIVERSAL,
                part: Some(car::PART_REGULAR),
                identifier: ident,
            },
        );
        Ok(())
    }

    fn parse_spriteatlas(
        &mut self,
        item: &Path,
        renditions: &mut Vec<Rendition>,
        facets: &mut IndexMap<String, Facet>,
    ) -> Result<()> {
        let atlas_name = item.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let atlas_ident = self.get_identifier(&atlas_name);
        facets.insert(
            atlas_name.clone(),
            Facet {
                element: car::ELEMENT_PACKED,
                part: None,
                identifier: atlas_ident,
            },
        );
        let mut entries: Vec<PathBuf> = match fs::read_dir(item) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => return Ok(()),
        };
        entries.sort();
        for sprite_item in entries {
            if sprite_item.extension().and_then(|s| s.to_str()) != Some("imageset") {
                continue;
            }
            let sprite_name =
                sprite_item.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let full = format!("{atlas_name}/{sprite_name}");
            let sprite_ident = self.get_identifier(&full);
            let contents_path = sprite_item.join("Contents.json");
            if !contents_path.exists() {
                continue;
            }
            let contents = read_json(&contents_path)?;
            if let Some(arr) = contents.get("images").and_then(|v| v.as_array()) {
                for img_info in arr {
                    let Some(filename) = img_info.get("filename").and_then(|v| v.as_str())
                    else {
                        continue;
                    };
                    let img_path = entry_path(&sprite_item, filename);
                    if !img_path.exists() {
                        continue;
                    }
                    let scale = parse_scale(img_info);
                    let img = load_image_as_bgra(&img_path, self.force_bgra)?;
                    renditions.push(Rendition {
                        name: filename.to_string(),
                        identifier: sprite_ident,
                        element: car::ELEMENT_UNIVERSAL,
                        part: car::PART_REGULAR,
                        scale: scale as u16,
                        width: img.width,
                        height: img.height,
                        colorspace_id: car::colorspace_for_pixel_format(&img.pixel_format),
                        pixel_data: img.pixel_data,
                        pixel_format: img.pixel_format,
                        wide_gamut: img.wide_gamut,
                        layout: car::LAYOUT_ONE_PART_SCALE,
                        sprite_atlas_id: atlas_ident,
                        min_deploy: self.min_deploy.clone(),
                        platform: self.platform.clone(),
                        ..Rendition::default()
                    });
                }
            }
            facets.insert(
                full,
                Facet {
                    element: car::ELEMENT_UNIVERSAL,
                    part: Some(car::PART_REGULAR),
                    identifier: sprite_ident,
                },
            );
        }
        Ok(())
    }

    fn parse_imagestack(
        &mut self,
        item: &Path,
        renditions: &mut Vec<Rendition>,
        facets: &mut IndexMap<String, Facet>,
    ) -> Result<()> {
        let stack_name = item.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let contents_path = item.join("Contents.json");
        if !contents_path.exists() {
            return Ok(());
        }
        let contents = read_json(&contents_path)?;
        if let Some(layers) = contents.get("layers").and_then(|v| v.as_array()) {
            for layer in layers {
                let Some(layer_filename) =
                    layer.get("filename").and_then(|v| v.as_str())
                else {
                    continue;
                };
                let layer_path = entry_path(item, layer_filename);
                if !layer_path.exists() {
                    continue;
                }
                let layer_name = Path::new(layer_filename)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let content_imageset = layer_path.join("Content.imageset");
                if !content_imageset.exists() {
                    continue;
                }
                let full = format!("{stack_name}/{layer_name}/Content");
                let layer_ident = self.get_identifier(&full);
                let img_contents_path = content_imageset.join("Contents.json");
                if !img_contents_path.exists() {
                    continue;
                }
                let img_contents = read_json(&img_contents_path)?;
                if let Some(arr) = img_contents.get("images").and_then(|v| v.as_array())
                {
                    for img_info in arr {
                        let Some(filename) =
                            img_info.get("filename").and_then(|v| v.as_str())
                        else {
                            continue;
                        };
                        let p = entry_path(&content_imageset, filename);
                        if !p.exists() {
                            continue;
                        }
                        let scale = parse_scale(img_info);
                        let idiom = img_info
                            .get("idiom")
                            .and_then(|v| v.as_str())
                            .unwrap_or("universal");
                        if self.platform == "macosx"
                            && idiom != "mac"
                            && idiom != "universal"
                        {
                            continue;
                        }
                        let img = load_image_as_bgra(&p, false)?;
                        renditions.push(Rendition {
                            name: filename.to_string(),
                            identifier: layer_ident,
                            element: car::ELEMENT_UNIVERSAL,
                            part: car::PART_REGULAR,
                            scale: scale as u16,
                            width: img.width,
                            height: img.height,
                            colorspace_id: car::colorspace_for_pixel_format(&img.pixel_format),
                            pixel_data: img.pixel_data,
                            pixel_format: img.pixel_format,
                            wide_gamut: img.wide_gamut,
                            layout: car::LAYOUT_ONE_PART_SCALE,
                            min_deploy: self.min_deploy.clone(),
                            platform: self.platform.clone(),
                            ..Rendition::default()
                        });
                    }
                }
                facets.insert(
                    full,
                    Facet {
                        element: car::ELEMENT_UNIVERSAL,
                        part: Some(car::PART_REGULAR),
                        identifier: layer_ident,
                    },
                );
            }
        }
        Ok(())
    }

    pub fn get_icon_images(&self) -> Result<Vec<(PathBuf, u32, u32)>> {
        let app_icon = match &self.app_icon {
            Some(v) => v.clone(),
            None => return Ok(Vec::new()),
        };
        let icon_dir = self.path.join(format!("{app_icon}.appiconset"));
        if !icon_dir.exists() {
            return Ok(Vec::new());
        }
        let contents_path = icon_dir.join("Contents.json");
        if !contents_path.exists() {
            return Ok(Vec::new());
        }
        let contents = read_json(&contents_path)?;
        let mut result = Vec::new();
        if let Some(arr) = contents.get("images").and_then(|v| v.as_array()) {
            for img_info in arr {
                let Some(filename) = img_info.get("filename").and_then(|v| v.as_str())
                else {
                    continue;
                };
                if filename.to_ascii_lowercase().ends_with(".svg") {
                    continue;
                }
                let p = entry_path(&icon_dir, filename);
                if !p.exists() {
                    continue;
                }
                let scale = parse_scale(img_info);
                let size_str = img_info.get("size").and_then(|v| v.as_str()).unwrap_or("");
                let Some((w_part, _)) = size_str.split_once('x') else {
                    continue;
                };
                let point_w: u32 = w_part.parse().unwrap_or(0);
                let pixel_size = point_w * scale;
                result.push((p, pixel_size, scale));
            }
        }
        Ok(result)
    }
}

/// Stamp `idiom` on the renditions appended since `start`; one image entry can expand into several.
fn stamp_idiom(renditions: &mut [Rendition], start: usize, idiom: u16) {
    if idiom == 0 {
        return;
    }
    for r in &mut renditions[start..] {
        r.idiom = idiom;
    }
}

fn parse_scale(img_info: &Value) -> u32 {
    let s = img_info
        .get("scale")
        .and_then(|v| v.as_str())
        .unwrap_or("1x");
    s.trim_end_matches('x').parse().unwrap_or(1)
}

fn appearance_from_json(v: &Value) -> u16 {
    if let Some(arr) = v.get("appearances").and_then(|x| x.as_array()) {
        for app in arr {
            if app.get("appearance").and_then(|s| s.as_str()) == Some("luminosity") {
                match app.get("value").and_then(|s| s.as_str()) {
                    Some("dark") => return 1,
                    Some("tinted") => return 10,
                    _ => {}
                }
            }
        }
    }
    0
}

/// The platform name Contents.json uses for an SDK platform.
fn json_platform(platform: &str) -> &str {
    match platform {
        "iphoneos" | "iphonesimulator" => "ios",
        "macosx" => "macos",
        "appletvos" | "appletvsimulator" => "tvos",
        "watchos" | "watchsimulator" => "watchos",
        "xros" | "xrsimulator" => "visionos",
        other => other,
    }
}

/// `dir/name`, or the entry whose name matches `name` ignoring case: actool runs on macOS's
/// case-insensitive file system, where `Contents.json` names need not match the file's case.
pub(crate) fn entry_path(dir: &Path, name: &str) -> PathBuf {
    let exact = dir.join(name);
    if exact.exists() {
        return exact;
    }
    let want = name.to_lowercase();
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().to_lowercase() == want)
        .map_or(exact, |e| e.path())
}

fn read_json(path: &Path) -> Result<Value> {
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_color_integer_normalised() {
        assert!((parse_color_component("128") - 128.0 / 255.0).abs() < 1e-10);
        assert_eq!(parse_color_component("0"), 0.0);
        assert_eq!(parse_color_component("1"), 1.0);
    }

    #[test]
    fn parse_color_hex() {
        assert!((parse_color_component("0x80") - 128.0 / 255.0).abs() < 1e-10);
    }

    #[test]
    fn parse_color_float_f32_roundtrip() {
        let v = parse_color_component("0.5");
        let f: f64 = 0.5f32 as f64;
        assert_eq!(v, f);
    }

    #[test]
    fn premultiply_bgra_zero_alpha() {
        let data = vec![0xFF, 0xFF, 0xFF, 0x00];
        let out = premultiply_bgra(data);
        assert_eq!(out, vec![0, 0, 0, 0]);
    }

    #[test]
    fn premultiply_bgra_half_alpha() {
        let data = vec![0xFF, 0xFF, 0xFF, 0x80];
        let out = premultiply_bgra(data);
        assert_eq!(out, vec![128, 128, 128, 128]);
    }

    #[test]
    fn bgra_grayscale_detected() {
        let data = vec![0x80, 0x80, 0x80, 0xFF, 0x80, 0x80, 0x80, 0xFF];
        let (out, w, h, fmt) = bgra_to_best_format(data, 2, 1, false);
        assert_eq!(&fmt, b" 8AG");
        assert_eq!(out, vec![0x80, 0xFF, 0x80, 0xFF]);
        assert_eq!(w, 2);
        assert_eq!(h, 1);
    }

    #[test]
    fn bgra_nongray_kept() {
        let data = vec![0xFF, 0x00, 0x00, 0xFF];
        let (_, _, _, fmt) = bgra_to_best_format(data, 1, 1, false);
        assert_eq!(&fmt, b"BGRA");
    }
}
