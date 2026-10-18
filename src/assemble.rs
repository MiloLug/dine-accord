//! The one `Assets.car` of an actool invocation, from all its documents: one key format, and
//! atlases packed across documents (wireshark's two icons fill one atlas and spill into a second).

use crate::bom::BomWriter;
use crate::car::{self, Rendition};
use crate::packer::{self, PackedImage};
use anyhow::Result;
use byteorder::{LittleEndian, WriteBytesExt};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub struct Facet {
    pub name: String,
    pub element: u16,
    pub part: Option<u16>,
    pub identifier: u16,
}

/// What one input document contributes.
#[derive(Default)]
pub struct Document {
    /// Stored as they are.
    pub renditions: Vec<Rendition>,
    /// Packed into atlases shared with the other documents.
    pub packable: Vec<Rendition>,
    pub facets: Vec<Facet>,
    pub appearances: Vec<(String, u16)>,
    /// CARHEADER version this document's renditions need.
    pub coreui_version: u32,
    /// Key attributes whose BITMAPKEYS mask is -1 even when only one value is used.
    pub wildcard_attributes: Vec<u16>,
    pub plist: plist::Dictionary,
    /// Files written next to the catalog (loose icons, `.icns`, JPEGs).
    pub files: Vec<PathBuf>,
}

/// Atlases are grouped by (scale, BGRA before other formats, sprite atlas, idiom, appearance,
/// format, display gamut).
type PackKey = (u16, u8, u16, u16, u16, [u8; 4], u16);

fn to_packed(r: &Rendition) -> PackedImage {
    let intent = match r.template_rendering_intent {
        i if i >= 0 => i,
        _ if r.is_template => 2,
        _ => 4,
    };
    PackedImage {
        name: r.name.clone(),
        identifier: r.identifier as u32,
        width: r.width,
        height: r.height,
        x: 0,
        y: 0,
        pixel_data: r.pixel_data.clone(),
        pixel_format: r.pixel_format,
        scale: r.scale as u32,
        is_template: r.is_template,
        template_rendering_intent: intent,
        part: r.part as u32,
        dim2: r.dim2 as u32,
        appearance: r.appearance as u32,
        direction: r.direction as u32,
        idiom: r.idiom as u32,
        is_svg_rasterization: r.is_svg_rasterization,
    }
}

/// Atlas and packed-reference renditions for every document's packable images.
fn pack(docs: &[Document], min_deploy: &str, platform: &str) -> Vec<Rendition> {
    let mut groups: BTreeMap<PackKey, Vec<&Rendition>> = BTreeMap::new();
    for r in docs.iter().flat_map(|d| &d.packable) {
        let rank = (&r.pixel_format != b"BGRA") as u8;
        groups
            .entry((r.scale, rank, r.sprite_atlas_id, r.idiom, r.appearance, r.pixel_format, r.display_gamut))
            .or_default()
            .push(r);
    }
    let mut out = Vec::new();
    let mut dim1: BTreeMap<(u16, u16, u16, u16), u16> = BTreeMap::new();
    for ((scale, _, sprite_atlas_id, idiom, appearance, fmt, gamut), members) in groups {
        let images = members.iter().map(|r| to_packed(r)).collect();
        for mut atlas in packer::pack_images_split(images) {
            let counter = dim1.entry((scale, idiom, appearance, gamut)).or_default();
            atlas.dim1 = *counter as u32;
            atlas.gamut = gamut as u32;
            atlas.render();
            let all_icons = atlas.images.iter().all(|i| i.part == car::PART_ICON as u32);
            let name = if sprite_atlas_id != 0 {
                atlas.name().replace("ZZZZPackedAsset", "ZZZZExplicitlyPackedAsset")
            } else {
                atlas.name()
            };
            let csi = car::build_packed_asset_csi(
                &name,
                atlas.width,
                atlas.height,
                scale,
                &fmt,
                &atlas.pixel_data,
                min_deploy,
                platform,
                &fmt == b"BGRA" && all_icons,
            );
            out.push(Rendition {
                name,
                identifier: sprite_atlas_id,
                element: car::ELEMENT_PACKED,
                part: car::PART_REGULAR,
                scale,
                dim1: *counter,
                idiom,
                appearance,
                display_gamut: gamut,
                width: atlas.width,
                height: atlas.height,
                pixel_format: fmt,
                layout: car::LAYOUT_NAME_LIST,
                csi_override: Some(csi),
                ..Rendition::default()
            });
            for img in &atlas.images {
                let mut flags = (img.template_rendering_intent as u32) << 2;
                if img.is_svg_rasterization {
                    flags |= 0x04;
                }
                let csi = car::build_packed_image_csi(
                    &img.name,
                    img.width,
                    img.height,
                    scale,
                    &fmt,
                    img.x,
                    img.y,
                    sprite_atlas_id,
                    *counter,
                    flags,
                    img.idiom as u16,
                    appearance,
                    gamut,
                );
                out.push(Rendition {
                    name: img.name.clone(),
                    identifier: img.identifier as u16,
                    part: img.part as u16,
                    scale,
                    dim2: img.dim2 as u16,
                    appearance: img.appearance as u16,
                    direction: img.direction as u16,
                    idiom: img.idiom as u16,
                    display_gamut: gamut,
                    width: img.width,
                    height: img.height,
                    pixel_format: fmt,
                    layout: car::LAYOUT_PACKED_IMAGE,
                    csi_override: Some(csi),
                    ..Rendition::default()
                });
            }
            *counter += 1;
        }
    }
    out
}

/// One BITMAPKEYS value per facet: for each key attribute, the bitset of values its
/// renditions use (1 when none), or -1 for wildcard attributes.
fn bitmap_keys(
    facets: &BTreeMap<String, &Facet>,
    entries: &[(Vec<u8>, Vec<u8>)],
    keyformat: &[u16],
    wildcards: &BTreeSet<u16>,
) -> Vec<(u32, Vec<u8>)> {
    let id_pos = keyformat.iter().position(|&a| a == 17);
    let mut by_ident: BTreeMap<u16, Vec<Vec<u16>>> = BTreeMap::new();
    for (key, _) in entries {
        let vals: Vec<u16> = key.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        if let Some(&ident) = id_pos.and_then(|p| vals.get(p)) {
            by_ident.entry(ident).or_default().push(vals);
        }
    }
    let idents: BTreeSet<u16> = facets.values().map(|f| f.identifier).filter(|&i| i != 0).collect();
    idents
        .into_iter()
        .map(|ident| {
            let rows = by_ident.get(&ident).map(Vec::as_slice).unwrap_or_default();
            let mut value = Vec::new();
            value.write_u32::<LittleEndian>(1).unwrap();
            value.write_u32::<LittleEndian>(0).unwrap();
            value.write_u32::<LittleEndian>(4 + 4 * keyformat.len() as u32).unwrap();
            value.write_u32::<LittleEndian>(keyformat.len() as u32).unwrap();
            for (i, attr) in keyformat.iter().enumerate() {
                let mask = if wildcards.contains(attr) {
                    u32::MAX
                } else {
                    let m = rows.iter().filter(|r| r[i] < 32).fold(0u32, |m, r| m | 1 << r[i]);
                    if m == 0 { 1 } else { m }
                };
                value.write_u32::<LittleEndian>(mask).unwrap();
            }
            (ident as u32, value)
        })
        .collect()
}

/// Writes `car_path` unless no document has renditions; returns whether it did.
pub fn write_car(docs: &[Document], car_path: &Path, platform: &str, min_deploy: &str) -> Result<bool> {
    let mut renditions: Vec<Rendition> = docs.iter().flat_map(|d| d.renditions.iter().cloned()).collect();
    let packed = pack(docs, min_deploy, platform);
    let atlas_count = packed.iter().filter(|r| r.element == car::ELEMENT_PACKED).count();
    let atlas_scales: BTreeSet<(u16, u16)> = packed
        .iter()
        .filter(|r| r.element == car::ELEMENT_PACKED)
        .map(|r| (r.scale, r.display_gamut))
        .collect();
    renditions.extend(packed);
    if renditions.is_empty() {
        return Ok(false);
    }

    // A Dimension1 column whenever a scale has more than one atlas of one gamut, even if every
    // atlas of that scale belongs to a different idiom and so carries Dimension1 = 0.
    let uses_dim1 = atlas_count > atlas_scales.len();
    let keyformat = if car::is_idiom_platform(platform) {
        car::compute_keyformat_ios(&renditions, uses_dim1)
    } else {
        car::compute_keyformat(&renditions, uses_dim1)
    };
    let mut entries: Vec<(Vec<u8>, Vec<u8>)> = renditions
        .iter_mut()
        .map(|r| {
            r.keyformat = keyformat.clone();
            r.min_deploy = min_deploy.to_string();
            r.platform = platform.to_string();
            (r.build_rendition_key(), r.build_csi())
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let coreui = docs.iter().map(|d| d.coreui_version).max().unwrap_or(972);
    let key_semantics = if car::is_idiom_platform(platform) { 2 } else { 1 };
    let mut bom = BomWriter::new();
    bom.add_named_block("CARHEADER", car::make_carheader_full(entries.len() as u32, coreui, key_semantics));
    bom.set_inline_key_size(Some(keyformat.len() * 2));
    bom.add_tree("RENDITIONS", &entries, 4096);
    bom.set_inline_key_size(None);

    let facets: BTreeMap<String, &Facet> = docs.iter().flat_map(|d| &d.facets).map(|f| (f.name.clone(), f)).collect();
    let facet_entries: Vec<(Vec<u8>, Vec<u8>)> = facets
        .iter()
        .map(|(name, f)| (name.as_bytes().to_vec(), car::make_facetkey_value(f.element, f.part, f.identifier)))
        .collect();
    bom.add_tree("FACETKEYS", &facet_entries, 4096);

    let appearances: BTreeMap<&str, u16> =
        docs.iter().flat_map(|d| &d.appearances).map(|(n, v)| (n.as_str(), *v)).collect();
    if !appearances.is_empty() {
        let entries: Vec<(Vec<u8>, Vec<u8>)> =
            appearances.iter().map(|(n, v)| (n.as_bytes().to_vec(), v.to_le_bytes().to_vec())).collect();
        bom.add_tree("APPEARANCEKEYS", &entries, 4096);
    }

    bom.add_named_block("KEYFORMAT", car::make_keyformat(&keyformat));
    bom.add_named_block("EXTENDED_METADATA", car::make_extended_metadata(platform, min_deploy));
    let wildcards: BTreeSet<u16> =
        [1, 2, 17].into_iter().chain(docs.iter().flat_map(|d| d.wildcard_attributes.iter().copied())).collect();
    bom.add_raw_key_tree("BITMAPKEYS", &bitmap_keys(&facets, &entries, &keyformat, &wildcards), 1024);
    bom.write(car_path)?;
    Ok(true)
}
