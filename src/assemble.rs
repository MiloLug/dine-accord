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

fn to_packed(r: &Rendition, index: usize, platform: &str) -> PackedImage {
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
        sort_key: sort_key(r, platform),
        index,
    }
}

/// The full key orders actool breaks packing ties with (CoreUI's untrimmed key formats).
const MAC_KEY_ORDER: [u16; 20] = [7, 13, 1, 2, 3, 4, 6, 17, 8, 9, 10, 14, 11, 12, 24, 19, 18, 25, 26, 27];
const IOS_KEY_ORDER: [u16; 21] = [7, 13, 12, 15, 16, 26, 27, 9, 8, 25, 24, 4, 20, 21, 23, 22, 17, 1, 2, 10, 6];

fn locale_id(r: &Rendition) -> u16 {
    if r.locale.is_empty() { 0 } else { crate::name_hash::hash_name(&r.locale) }
}

fn sort_key(r: &Rendition, platform: &str) -> Vec<u16> {
    let order: &[u16] = if car::is_idiom_platform(platform) { &IOS_KEY_ORDER } else { &MAC_KEY_ORDER };
    order
        .iter()
        .map(|a| match a {
            1 => r.element,
            2 => r.part,
            4 => r.direction,
            7 => r.appearance,
            9 => r.dim2,
            12 => r.scale,
            13 => locale_id(r),
            15 => r.idiom,
            16 => r.subtype,
            17 => r.identifier,
            24 => r.display_gamut,
            _ => 0,
        })
        .collect()
}

/// actool packs app icons by their pixels' opacity, other images by whether the source has alpha.
fn packs_opaque(r: &Rendition) -> bool {
    if r.part == car::PART_ICON || r.idiom == 6 {
        car::check_opaque(&r.pixel_data, &r.pixel_format, r.width, r.height)
    } else {
        r.source_opaque
    }
}

/// actool's packing group: scale, opaque, monochrome, subtype, idiom, two size classes, explicit
/// atlas, memory and graphics classes, gamut, compression, deployment target, appearance,
/// localization and archive flag, joined with '.' and ordered as strings.
fn group_key(r: &Rendition) -> String {
    let mono = matches!(&r.pixel_format, b" 8AG" | b"61AG");
    format!(
        "{}.{}.{}.{}.{}.0.0.{}.0.0.{}.0.0.{}.{}.0",
        r.scale,
        packs_opaque(r) as u8,
        mono as u8,
        r.subtype,
        r.idiom,
        r.sprite_atlas_id,
        r.display_gamut,
        r.appearance,
        locale_id(r)
    )
}

/// Atlases are numbered (Dimension1) per group key without its opaque, monochrome and
/// compression fields and without the chunk ordinal.
fn dim1_key(chunk: &str) -> String {
    let fields = chunk.split('.').take(16).enumerate();
    fields.filter(|(i, _)| ![1, 2, 11].contains(i)).map(|(_, f)| f).collect::<Vec<_>>().join(".")
}

/// Atlas and packed-reference renditions for every document's packable images, and the
/// packable images actool leaves unpacked.
fn pack(docs: &[Document], min_deploy: &str, platform: &str) -> (Vec<Rendition>, Vec<Rendition>) {
    let packable: Vec<&Rendition> = docs.iter().flat_map(|d| &d.packable).collect();
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, r) in packable.iter().enumerate() {
        groups.entry(group_key(r)).or_default().push(i);
    }
    let mut unpacked = Vec::new();
    let mut chunks: BTreeMap<String, Vec<PackedImage>> = BTreeMap::new();
    for (key, members) in groups {
        let first = packable[members[0]];
        let explicit = first.sprite_atlas_id != 0;
        if members.len() == 1 && !explicit {
            unpacked.push(first.clone());
            continue;
        }
        let scale = first.scale as u32;
        let limit = if explicit { 0x36_6666 / scale * scale } else { packer::MAX_PACKED_AREA_PER_SCALE * scale };
        let images = members.iter().map(|&i| to_packed(packable[i], i, platform)).collect();
        let (split, rest) = packer::split_group(images, limit);
        unpacked.extend(rest.iter().map(|img| packable[img.index].clone()));
        for (step, imgs) in split {
            chunks.insert(format!("{key}.{step}"), imgs);
        }
    }

    let mut out = Vec::new();
    let mut dim1: BTreeMap<String, u16> = BTreeMap::new();
    for (key, imgs) in chunks {
        let first = packable[imgs[0].index];
        let (scale, sprite_atlas_id) = (first.scale, first.sprite_atlas_id);
        let mut atlas = packer::pack_chunk(imgs);
        let counter = dim1.entry(dim1_key(&key)).or_default();
        let link = car::AtlasKey {
            identifier: sprite_atlas_id,
            dim1: *counter,
            scale,
            idiom: first.idiom,
            subtype: first.subtype,
            appearance: first.appearance,
            display_gamut: first.display_gamut,
        };
        *counter += 1;
        atlas.dim1 = link.dim1 as u32;
        atlas.gamut = link.display_gamut as u32;
        atlas.opaque = packs_opaque(first);
        atlas.render();
        let fmt = atlas.pixel_format;
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
            dim1: link.dim1,
            idiom: link.idiom,
            subtype: link.subtype,
            appearance: link.appearance,
            display_gamut: link.display_gamut,
            width: atlas.width,
            height: atlas.height,
            pixel_format: fmt,
            layout: car::LAYOUT_NAME_LIST,
            csi_override: Some(csi),
            ..Rendition::default()
        });
        for img in &atlas.images {
            let r = packable[img.index];
            let mut flags = (img.template_rendering_intent as u32) << 2;
            if img.is_svg_rasterization {
                flags |= 0x04;
            }
            let csi = car::build_packed_image_csi(&img.name, img.width, img.height, &fmt, img.x, img.y, flags, &link);
            out.push(Rendition {
                name: img.name.clone(),
                identifier: r.identifier,
                part: r.part,
                scale,
                dim2: r.dim2,
                appearance: r.appearance,
                direction: r.direction,
                idiom: r.idiom,
                subtype: r.subtype,
                display_gamut: r.display_gamut,
                width: img.width,
                height: img.height,
                pixel_format: fmt,
                layout: car::LAYOUT_PACKED_IMAGE,
                csi_override: Some(csi),
                ..Rendition::default()
            });
        }
    }
    (out, unpacked)
}

/// One BITMAPKEYS value per facet: for each key attribute, the bitset of values its
/// renditions use (1 when none), or -1 for wildcard attributes.
/// The BITMAPKEYS bit CoreUI gives a device subtype (key attribute 16) on iOS-family platforms.
fn subtype_bit(subtype: u16) -> Option<u32> {
    Some(match subtype {
        0 => 0,
        568 => 1,
        569 => 2,
        570 => 3,
        2732 => 4,
        163 => 5,
        2420 => 7,
        2752 => 8,
        2034 => 9,
        2224 => 10,
        2436 => 11,
        2688 => 12,
        2622 => 13,
        2868 => 14,
        2388 => 15,
        1792 => 16,
        32401 => 17,
        2340 => 18,
        2778 => 19,
        2360 => 20,
        2160 => 21,
        2532 => 22,
        2266 => 25,
        2556 => 29,
        2796 => 30,
        2736 => 31,
        _ => return None,
    })
}

/// The BITMAPKEYS bit of an attribute value: its subtype bit, 1 for any localization, else the
/// value itself below 32.
fn value_bit(attr: u16, value: u16) -> Option<u32> {
    match attr {
        16 => subtype_bit(value),
        13 => Some((value != 0) as u32),
        _ => (value < 32).then_some(value as u32),
    }
}

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
                    let m = rows.iter().filter_map(|r| value_bit(*attr, r[i])).fold(0u32, |m, b| m | 1 << b);
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
    let (packed, unpacked) = pack(docs, min_deploy, platform);
    renditions.extend(unpacked);
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
