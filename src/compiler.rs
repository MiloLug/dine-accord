//! Asset catalog compiler: the documents of one actool invocation (`.xcassets`, `.icon`) into
//! `Assets.car`, loose files and the partial Info.plist.

use crate::assemble::{self, Document};
use crate::car;
use crate::catalog::{AppIconSet, AssetCatalog};
use crate::icns;
use crate::packer;
use anyhow::Result;
use indexmap::IndexMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Options {
    pub platform: String,
    pub min_deploy: String,
    pub app_icon: Option<String>,
    pub info_plist: Option<PathBuf>,
    pub accent_color: Option<String>,
    pub widget_background_color: Option<String>,
    pub standalone_icon_behavior: String,
    pub target_devices: Vec<String>,
    pub include_all_app_icons: bool,
    pub include_languages: Option<Vec<String>>,
    pub development_region: Option<String>,
    pub plist_localizations: bool,
}

/// Compiles `documents` into `output_dir`; returns the files written.
pub fn compile(documents: &[PathBuf], output_dir: &Path, opts: &Options) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(output_dir)?;
    // Since Xcode 26.1 an `.appiconset` named like an `.icon` document is ignored.
    let icon_names: Vec<String> = documents
        .iter()
        .filter(|d| crate::icon_bundle::is_icon_bundle(d))
        .filter_map(|d| d.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    let docs = documents
        .iter()
        .map(|d| {
            if crate::icon_bundle::is_icon_bundle(d) {
                crate::icon_bundle::document(d, output_dir, opts)
            } else {
                catalog_document(d, output_dir, opts, &icon_names)
            }
        })
        .collect::<Result<Vec<_>>>()?;

    let mut output_files: Vec<PathBuf> = docs.iter().flat_map(|d| d.files.iter().cloned()).collect();
    let car_path = output_dir.join("Assets.car");
    if assemble::write_car(&docs, &car_path, &opts.platform, &opts.min_deploy)? {
        output_files.push(fs::canonicalize(&car_path).unwrap_or(car_path));
    }
    if let Some(path) = &opts.info_plist {
        let mut merged = plist::Dictionary::new();
        for d in &docs {
            merge_plist(&mut merged, &d.plist);
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        plist::Value::Dictionary(merged).to_file_xml(path)?;
        output_files.push(fs::canonicalize(path).unwrap_or(path.clone()));
    }
    Ok(output_files)
}

/// Merges `from` into `into`, recursing into dictionaries present in both.
fn merge_plist(into: &mut plist::Dictionary, from: &plist::Dictionary) {
    for (key, value) in from {
        match (into.get_mut(key), value) {
            (Some(plist::Value::Dictionary(a)), plist::Value::Dictionary(b)) => merge_plist(a, b),
            _ => {
                into.insert(key.clone(), value.clone());
            }
        }
    }
}

fn catalog_document(
    xcassets_path: &Path,
    output_dir: &Path,
    opts: &Options,
    icon_names: &[String],
) -> Result<Document> {
    let platform = opts.platform.as_str();
    let min_deploy = opts.min_deploy.as_str();
    let app_icon = opts.app_icon.as_deref();
    let mut catalog = AssetCatalog::new(
        xcassets_path.to_path_buf(),
        opts.platform.clone(),
        opts.min_deploy.clone(),
        opts.app_icon.clone(),
        opts.include_languages.clone(),
        opts.development_region.clone(),
    );
    catalog.target_devices = opts.target_devices.clone();
    catalog.include_all_app_icons = opts.include_all_app_icons;
    catalog.skipped_app_icons = icon_names.to_vec();
    let (renditions, facets) = catalog.parse()?;
    let renditions = car::gamut_variants(&renditions);
    let mut doc = Document {
        coreui_version: if car::is_idiom_platform(platform) { 975 } else { 972 },
        ..Document::default()
    };

    let deploy_ver: (u32, u32) = {
        let mut parts = min_deploy.split('.');
        let a: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let b: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        (a, b)
    };
    if deploy_ver >= car::min_pack_version(platform) {
        let (pack_groups, _) = packer::group_for_packing(&renditions);
        let packed: std::collections::HashSet<usize> =
            pack_groups.iter().flat_map(|(_, _, idxs)| idxs.iter().copied()).collect();
        for (i, r) in renditions.iter().enumerate() {
            if packed.contains(&i) {
                doc.packable.push(r.clone());
            } else {
                doc.renditions.push(r.clone());
            }
        }
    } else {
        doc.renditions = renditions.clone();
    }

    let mut atlas_sprites: IndexMap<u16, Vec<String>> = IndexMap::new();
    for (name, facet) in &facets {
        if let Some(rend) =
            renditions.iter().find(|r| r.sprite_atlas_id != 0 && r.identifier == facet.identifier)
        {
            let entry = atlas_sprites.entry(rend.sprite_atlas_id).or_default();
            if !entry.contains(name) {
                entry.push(name.clone());
            }
        }
    }
    for (atlas_id, mut sprite_names) in atlas_sprites {
        sprite_names.sort();
        doc.renditions.push(car::Rendition {
            name: "CoreStructuredImage".to_string(),
            identifier: atlas_id,
            element: car::ELEMENT_PACKED,
            part: car::PART_SPRITE_ATLAS,
            layout: car::LAYOUT_METADATA,
            csi_override: Some(car::build_sprite_atlas_metadata_csi(&sprite_names)),
            ..car::Rendition::default()
        });
    }

    doc.facets = facets
        .iter()
        .map(|(name, f)| assemble::Facet {
            name: name.clone(),
            element: f.element,
            part: f.part,
            identifier: f.identifier,
        })
        .collect();
    let has = |a: u16| renditions.iter().any(|r| r.appearance == a);
    if car::is_idiom_platform(platform) {
        // iOS always emits APPEARANCEKEYS, declaring the wildcard appearance
        // (UIAppearanceAny=0) even when no asset has a dark variant.
        doc.appearances.push(("UIAppearanceAny".into(), 0));
        if has(1) {
            doc.appearances.push(("UIAppearanceDark".into(), 1));
        }
        doc.wildcard_attributes.push(7);
    } else if has(1) {
        doc.appearances.push(("NSAppearanceNameDarkAqua".into(), 1));
        doc.appearances.push(("NSAppearanceNameSystem".into(), 0));
    }
    if has(10) {
        doc.appearances.push(("ISAppearanceTintable".into(), 10));
    }

    // Below the platform's JPEG-in-CAR deployment target, actool ships JPEGs as loose files.
    for (imageset_stem, src) in &catalog.loose_jpegs {
        let ext = src
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("jpg");
        let dest = output_dir.join(format!("{imageset_stem}.{ext}"));
        fs::copy(src, &dest)?;
        doc.files.push(fs::canonicalize(&dest).unwrap_or(dest));
    }

    let mut loose_idioms = Vec::new();
    if let Some(icon_name) = app_icon {
        if car::is_idiom_platform(platform) {
            if let Some(set) = catalog.app_icon_sets.iter().find(|s| s.primary) {
                for (idiom, dest) in write_ios_loose_icons(set, output_dir)? {
                    loose_idioms.push(idiom);
                    doc.files.push(fs::canonicalize(&dest).unwrap_or(dest));
                }
            }
        } else if opts.standalone_icon_behavior != "none" {
            let icons = catalog.get_icon_images()?;
            if !icons.is_empty() {
                let icns_path = output_dir.join(format!("{icon_name}.icns"));
                icns::create_icns(&icons, &icns_path)?;
                doc.files.push(fs::canonicalize(&icns_path).unwrap_or(icns_path));
            }
        }
    }

    let locales = if opts.plist_localizations { catalog.get_locales_used() } else { Vec::new() };
    let ios = car::is_idiom_platform(platform);
    doc.plist = info_plist(
        app_icon.filter(|_| !ios),
        opts.accent_color.as_deref(),
        opts.widget_background_color.as_deref(),
        &locales,
    );
    if let (true, Some(icon_name)) = (ios, app_icon) {
        doc.plist.extend(ios_icon_plist(icon_name, &loose_idioms, &catalog.app_icon_sets));
    }
    Ok(doc)
}

fn info_plist(
    app_icon: Option<&str>,
    accent_color: Option<&str>,
    widget_background_color: Option<&str>,
    localizations: &[String],
) -> plist::Dictionary {
    let mut d = plist::Dictionary::new();
    if let Some(name) = app_icon {
        d.insert("CFBundleIconFile".into(), name.into());
        d.insert("CFBundleIconName".into(), name.into());
    }
    if let Some(n) = accent_color {
        d.insert("NSAccentColorName".into(), n.into());
    }
    if let Some(n) = widget_background_color {
        d.insert("NSWidgetBackgroundColorName".into(), n.into());
    }
    if !localizations.is_empty() {
        let locs = localizations.iter().map(|l| plist::Value::from(l.as_str())).collect();
        d.insert("CFBundleLocalizations".into(), plist::Value::Array(locs));
    }
    d
}

/// iOS home-screen ("primary") icon point size per idiom: the size listed in CFBundleIconFiles
/// and emitted as a loose PNG. Other idiom sizes and the marketing icon are CAR-only.
const IOS_PRIMARY_SIZES: [(&str, u32, &str); 2] = [("iphone", 60, ""), ("ipad", 76, "~ipad")];

/// Loose home-screen PNGs actool drops next to the CAR (`AppIcon60x60@2x.png`,
/// `AppIcon76x76@2x~ipad.png`): the idiom's @2x primary icon, or a single-size 1024 icon scaled
/// down. Returns the idioms that got one.
fn write_ios_loose_icons(set: &AppIconSet, output_dir: &Path) -> Result<Vec<(&'static str, PathBuf)>> {
    let mut out = Vec::new();
    for (idiom, primary, suffix) in IOS_PRIMARY_SIZES {
        let mut of_idiom = set.images.iter().filter(|i| i.idiom == idiom && i.appearance == 0);
        let base = format!("{}{primary}x{primary}@2x{suffix}", set.name);
        let dest = if let Some(img) = of_idiom.clone().find(|i| i.point_w == primary && i.scale == 2) {
            let ext = img.path.extension().and_then(|s| s.to_str()).unwrap_or("png");
            let dest = output_dir.join(format!("{base}.{ext}"));
            if ext.eq_ignore_ascii_case("png") {
                write_loose_icon(&dest, &crate::color::open(&img.path)?.image.to_rgba8())?;
            } else {
                fs::copy(&img.path, &dest)?;
            }
            dest
        } else if let Some(img) = of_idiom.find(|i| i.point_w == 1024) {
            let dest = output_dir.join(format!("{base}.png"));
            let px = primary * 2;
            let icon = crate::color::open(&img.path)?.image;
            write_loose_icon(&dest, &icon.resize_exact(px, px, image::imageops::FilterType::Lanczos3).to_rgba8())?;
            dest
        } else {
            continue;
        };
        out.push((idiom, dest));
    }
    Ok(out)
}

/// A loose icon PNG as actool writes it: 8-bit RGBA, an `sRGB` chunk, and an `eXIf` holding the
/// colour space (sRGB) and the pixel size.
pub(crate) fn write_loose_icon(path: &Path, img: &image::RgbaImage) -> Result<()> {
    use byteorder::{BigEndian, WriteBytesExt};
    let (w, h) = img.dimensions();
    // Big-endian TIFF: IFD0 holds only the Exif IFD pointer (offset 26); the Exif IFD holds
    // ColorSpace (SHORT 1, left-justified), PixelXDimension and PixelYDimension (LONG).
    let mut exif = b"MM\0\x2a\0\0\0\x08".to_vec();
    let entries: [&[(u16, u16, u32)]; 2] =
        [&[(0x8769, 4, 26)], &[(0xa001, 3, 1 << 16), (0xa002, 4, w), (0xa003, 4, h)]];
    for ifd in entries {
        exif.write_u16::<BigEndian>(ifd.len() as u16)?;
        for &(tag, kind, value) in ifd {
            exif.write_u16::<BigEndian>(tag)?;
            exif.write_u16::<BigEndian>(kind)?;
            exif.write_u32::<BigEndian>(1)?;
            exif.write_u32::<BigEndian>(value)?;
        }
        exif.write_u32::<BigEndian>(0)?;
    }

    let mut encoder = png::Encoder::new(fs::File::create(path)?, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header()?;
    writer.write_chunk(png::chunk::ChunkType(*b"eXIf"), &exif)?;
    writer.write_image_data(img.as_raw())?;
    Ok(())
}

/// iOS partial Info.plist: a `CFBundleIcons` (iPhone) and `CFBundleIcons~ipad` dict, each with
/// the primary icon's `CFBundleIconName`, `CFBundleIconFiles` when the idiom has a loose icon (the
/// iPad list starts with the iPhone one, its fallback), and the alternate icons by name.
pub(crate) fn ios_icon_plist(name: &str, loose: &[&str], sets: &[AppIconSet]) -> plist::Dictionary {
    let files = |idioms: &[&str]| -> Vec<plist::Value> {
        IOS_PRIMARY_SIZES
            .iter()
            .filter(|(idiom, _, _)| idioms.contains(idiom) && loose.contains(idiom))
            .map(|(_, primary, _)| format!("{name}{primary}x{primary}").into())
            .collect()
    };
    let icons = |files: Vec<plist::Value>| {
        let mut primary = plist::Dictionary::new();
        if !files.is_empty() {
            primary.insert("CFBundleIconFiles".into(), plist::Value::Array(files));
        }
        primary.insert("CFBundleIconName".into(), name.into());
        let mut d = plist::Dictionary::new();
        let alternates: plist::Dictionary = sets
            .iter()
            .filter(|s| !s.primary)
            .map(|s| {
                let mut alt = plist::Dictionary::new();
                alt.insert("CFBundleIconName".into(), s.name.clone().into());
                (s.name.clone(), plist::Value::Dictionary(alt))
            })
            .collect();
        if !alternates.is_empty() {
            d.insert("CFBundleAlternateIcons".into(), alternates.into());
        }
        d.insert("CFBundlePrimaryIcon".into(), primary.into());
        plist::Value::Dictionary(d)
    };
    let mut d = plist::Dictionary::new();
    d.insert("CFBundleIcons".into(), icons(files(&["iphone"])));
    let ipad = if loose.contains(&"ipad") { files(&["iphone", "ipad"]) } else { Vec::new() };
    d.insert("CFBundleIcons~ipad".into(), icons(ipad));
    d
}

