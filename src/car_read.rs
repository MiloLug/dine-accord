//! Reader for compiled asset catalogs (`Assets.car`). BOM structures are big-endian;
//! everything CoreUI writes inside a block is little-endian.

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::Serialize;
use std::sync::OnceLock;

fn be32(b: &[u8], off: usize) -> Result<u32> {
    let s = b.get(off..off + 4).ok_or_else(|| anyhow!("read past end at {off}"))?;
    Ok(u32::from_be_bytes(s.try_into().unwrap()))
}

fn be16(b: &[u8], off: usize) -> Result<u16> {
    let s = b.get(off..off + 2).ok_or_else(|| anyhow!("read past end at {off}"))?;
    Ok(u16::from_be_bytes(s.try_into().unwrap()))
}

fn le32(b: &[u8], off: usize) -> Result<u32> {
    let s = b.get(off..off + 4).ok_or_else(|| anyhow!("read past end at {off}"))?;
    Ok(u32::from_le_bytes(s.try_into().unwrap()))
}

fn le16(b: &[u8], off: usize) -> Result<u16> {
    let s = b.get(off..off + 2).ok_or_else(|| anyhow!("read past end at {off}"))?;
    Ok(u16::from_le_bytes(s.try_into().unwrap()))
}

fn lef32(b: &[u8], off: usize) -> Result<f32> {
    Ok(f32::from_bits(le32(b, off)?))
}

fn lef64(b: &[u8], off: usize) -> Result<f64> {
    let s = b.get(off..off + 8).ok_or_else(|| anyhow!("read past end at {off}"))?;
    Ok(f64::from_le_bytes(s.try_into().unwrap()))
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// FourCCs are stored little-endian, so `ARGB` is `BGRA` on disk.
fn fourcc(v: u32) -> String {
    if v == 0 {
        return String::new();
    }
    String::from_utf8_lossy(&v.to_be_bytes()).into_owned()
}

pub struct Bom<'a> {
    data: &'a [u8],
    pub version: u32,
    blocks: Vec<(u32, u32)>,
    pub vars: Vec<(String, u32)>,
}

impl<'a> Bom<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        ensure!(data.starts_with(b"BOMStore"), "not a BOMStore file");
        let version = be32(data, 8)?;
        let (idx_off, vars_off) = (be32(data, 16)? as usize, be32(data, 24)? as usize);
        let n = be32(data, idx_off)? as usize;
        let blocks = (0..n)
            .map(|i| Ok((be32(data, idx_off + 4 + i * 8)?, be32(data, idx_off + 8 + i * 8)?)))
            .collect::<Result<Vec<_>>>()?;
        let nv = be32(data, vars_off)? as usize;
        let mut vars = Vec::with_capacity(nv);
        let mut p = vars_off + 4;
        for _ in 0..nv {
            let bi = be32(data, p)?;
            let ln = *data.get(p + 4).ok_or_else(|| anyhow!("truncated vars"))? as usize;
            let name = data.get(p + 5..p + 5 + ln).ok_or_else(|| anyhow!("truncated vars"))?;
            vars.push((String::from_utf8_lossy(name).into_owned(), bi));
            p += 5 + ln;
        }
        Ok(Bom { data, version, blocks, vars })
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn block(&self, i: u32) -> Result<&'a [u8]> {
        let &(addr, len) = self.blocks.get(i as usize).ok_or_else(|| anyhow!("block {i} out of range"))?;
        self.data
            .get(addr as usize..(addr + len) as usize)
            .ok_or_else(|| anyhow!("block {i} past end of file"))
    }

    pub fn var(&self, name: &str) -> Option<Result<&'a [u8]>> {
        self.vars.iter().find(|(n, _)| n == name).map(|&(_, bi)| self.block(bi))
    }

    /// Entries of a BOM B-tree, in leaf order. Trees with the inline-key flag (BITMAPKEYS) store
    /// the key itself in the entry instead of a block index; it is returned as 4 big-endian bytes.
    pub fn tree(&self, name: &str) -> Result<Vec<(Vec<u8>, &'a [u8])>> {
        let Some(header) = self.var(name) else { return Ok(Vec::new()) };
        let header = header?;
        ensure!(header.starts_with(b"tree"), "{name} is not a tree");
        let mut node_i = be32(header, 8)?;
        let inline_keys = header.get(20).copied().unwrap_or(0) != 0;
        let mut seen = std::collections::HashSet::new();
        loop {
            let node = self.block(node_i)?;
            if be16(node, 0)? != 0 {
                break;
            }
            ensure!(seen.insert(node_i), "{name}: tree cycle");
            node_i = be32(node, 12)?;
        }
        let mut out = Vec::new();
        while node_i != 0 && seen.insert(node_i) {
            let node = self.block(node_i)?;
            let count = be16(node, 2)? as usize;
            for k in 0..count {
                let vi = be32(node, 12 + 8 * k)?;
                let ki = be32(node, 16 + 8 * k)?;
                let key = if inline_keys { ki.to_be_bytes().to_vec() } else { self.block(ki)?.to_vec() };
                out.push((key, self.block(vi)?));
            }
            node_i = be32(node, 4)?;
        }
        Ok(out)
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CarHeader {
    pub tag: String,
    pub coreui_version: u32,
    pub storage_version: u32,
    pub storage_timestamp: u32,
    pub rendition_count: u32,
    pub main_version_string: String,
    pub version_string: String,
    pub uuid: String,
    pub associated_checksum: u32,
    pub schema_version: u32,
    pub color_space_id: u32,
    pub key_semantics: u32,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Metadata {
    pub thinning_arguments: String,
    pub deployment_platform_version: String,
    pub deployment_platform: String,
    pub authoring_tool: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Facet {
    pub name: String,
    pub hotspot: (i16, i16),
    pub attributes: Vec<(u16, u16)>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct BitmapKey {
    pub key: u32,
    pub masks: Vec<u32>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Rendition {
    /// Attribute values in KEYFORMAT order.
    pub key: Vec<u16>,
    pub name: String,
    pub layout: u16,
    pub version: u32,
    pub flags: u32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub pixel_format: String,
    pub colorspace: u32,
    pub modtime: u32,
    pub tlvs: Vec<Tlv>,
    pub payload: Payload,
    /// Decoded image rows (`bytes_per_row` × height) or raw data bytes.
    #[serde(skip)]
    pub data: Option<Vec<u8>>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Tlv {
    pub tag: u32,
    #[serde(flatten)]
    pub value: TlvValue,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TlvValue {
    Slices { rects: Vec<[u32; 4]> },
    Metrics { words: Vec<u32> },
    BlendOpacity { blend: u32, opacity: f32 },
    Uti { uti: String },
    Orientation { orientation: u32 },
    BytesPerRow { bytes_per_row: u32 },
    Link { rect: [u32; 4], key: Vec<(u16, u16)> },
    Layers { layers: Vec<LayerEntry> },
    LayerFills { entries: Vec<LayerFill> },
    LayerEffects { entries: Vec<LayerEffects> },
    Raw { hex: String },
}

/// One entry of TLV 1021. Field meanings are inferred from icon.json group settings
/// (shadow kinds: 2 layer-color, 3 neutral).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct LayerEffects {
    pub unknown: u32,
    pub translucency_enabled: u32,
    pub translucency: f32,
    pub shadow_kind: u32,
    pub shadow_opacity: f32,
}

/// One entry of the icon stack/group layer list (TLV 1012).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct LayerEntry {
    pub frame: [i32; 4],
    pub opacity: f32,
    pub key: Vec<(u16, u16)>,
}

/// One entry of TLV 1020: a kind word, a float and an optional facet name.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct LayerFill {
    pub kind: u32,
    pub value: f32,
    pub name: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Payload {
    None,
    Celm {
        version: u32,
        compression: u32,
        data_len: u32,
        chunks: Option<u32>,
        decoded_len: Option<usize>,
        error: Option<String>,
    },
    Color { version: u32, colorspace: u32, components: Vec<f64> },
    Gradient { kind: u32, reserved: u32, geometry: [f32; 4], stops: Vec<(f32, String)> },
    Multisize { version: u32, entries: Vec<[u32; 3]> },
    RawData { version: u32, data_len: u32, decoded_len: Option<usize> },
    Unknown { magic: String, len: usize },
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Car {
    pub bom_version: u32,
    pub block_count: usize,
    pub vars: Vec<String>,
    pub header: Option<CarHeader>,
    pub metadata: Option<Metadata>,
    pub keyformat: Vec<u16>,
    pub appearances: Vec<(String, u16)>,
    pub facets: Vec<Facet>,
    pub bitmap_keys: Vec<BitmapKey>,
    pub renditions: Vec<Rendition>,
}

/// Rendition key attribute names (`kCRTheme…Name`), indexed by attribute id.
pub const ATTRIBUTE_NAMES: [&str; 28] = [
    "ThemeLook", "Element", "Part", "Size", "Direction", "Placeholder", "Value", "Appearance",
    "Dimension1", "Dimension2", "State", "Layer", "Scale", "Localization", "PresentationState",
    "Idiom", "Subtype", "Identifier", "PreviousValue", "PreviousState", "SizeClassHorizontal",
    "SizeClassVertical", "MemoryClass", "GraphicsClass", "DisplayGamut", "DeploymentTarget",
    "GlyphWeight", "GlyphSize",
];

pub fn attribute_name(id: u16) -> String {
    ATTRIBUTE_NAMES.get(id as usize).map_or_else(|| format!("Attr{id}"), |s| s.to_string())
}

pub fn layout_name(layout: u16) -> &'static str {
    match layout {
        9 => "Vector",
        10 => "TextEffect",
        11 => "Vector(legacy)",
        12 => "Image",
        20..=34 => "Sliced",
        40 => "AnimationFilmstrip",
        1000 => "Data",
        1001 => "ExternalLink",
        1002 => "LayerStack",
        1003 => "InternalReference",
        1004 => "PackedImage",
        1005 => "NameList",
        1007 => "Texture",
        1008 => "TextureImage",
        1009 => "Color",
        1010 => "MultisizeImageSet",
        1011 => "LayerReference",
        1012 => "ContentRendition",
        1013 => "RecognitionObject",
        1019 => "IconImageStack",
        1020 => "IconGroup",
        1021 => "Gradient",
        _ => "?",
    }
}

impl Car {
    pub fn parse(data: &[u8]) -> Result<Car> {
        let bom = Bom::parse(data)?;
        let header = bom.var("CARHEADER").transpose()?.map(parse_header).transpose()?;
        let metadata = bom.var("EXTENDED_METADATA").transpose()?.map(parse_metadata).transpose()?;
        let keyformat = match bom.var("KEYFORMAT").transpose()? {
            Some(b) => {
                ensure!(b.starts_with(b"tmfk"), "bad KEYFORMAT magic");
                let n = le32(b, 8)? as usize;
                (0..n).map(|i| Ok(le32(b, 12 + 4 * i)? as u16)).collect::<Result<_>>()?
            }
            None => Vec::new(),
        };
        let appearances = bom
            .tree("APPEARANCEKEYS")?
            .into_iter()
            .map(|(k, v)| Ok((cstr(&k), le16(v, 0)?)))
            .collect::<Result<_>>()?;
        let facets = bom
            .tree("FACETKEYS")?
            .into_iter()
            .map(|(k, v)| parse_facet(&k, v))
            .collect::<Result<_>>()?;
        let bitmap_keys = bom
            .tree("BITMAPKEYS")?
            .into_iter()
            .map(|(k, v)| parse_bitmap_key(&k, v))
            .collect::<Result<_>>()?;
        let renditions = bom
            .tree("RENDITIONS")?
            .into_iter()
            .map(|(k, v)| parse_rendition(&k, v, keyformat.len()))
            .collect::<Result<_>>()?;
        Ok(Car {
            bom_version: bom.version,
            block_count: bom.block_count(),
            vars: bom.vars.iter().map(|(n, _)| n.clone()).collect(),
            header,
            metadata,
            keyformat,
            appearances,
            facets,
            bitmap_keys,
            renditions,
        })
    }
}

fn parse_header(h: &[u8]) -> Result<CarHeader> {
    ensure!(h.len() >= 436, "CARHEADER is {} bytes, expected 436", h.len());
    Ok(CarHeader {
        tag: fourcc(le32(h, 0)?),
        coreui_version: le32(h, 4)?,
        storage_version: le32(h, 8)?,
        storage_timestamp: le32(h, 12)?,
        rendition_count: le32(h, 16)?,
        main_version_string: cstr(&h[20..148]),
        version_string: cstr(&h[148..404]),
        uuid: hex(&h[404..420]),
        associated_checksum: le32(h, 420)?,
        schema_version: le32(h, 424)?,
        color_space_id: le32(h, 428)?,
        key_semantics: le32(h, 432)?,
    })
}

fn parse_metadata(m: &[u8]) -> Result<Metadata> {
    ensure!(m.len() >= 1028 && m.starts_with(b"META"), "bad EXTENDED_METADATA");
    Ok(Metadata {
        thinning_arguments: cstr(&m[4..260]),
        deployment_platform_version: cstr(&m[260..516]),
        deployment_platform: cstr(&m[516..772]),
        authoring_tool: cstr(&m[772..1028]),
    })
}

fn parse_facet(k: &[u8], v: &[u8]) -> Result<Facet> {
    let n = le16(v, 4)? as usize;
    let attributes = (0..n).map(|i| Ok((le16(v, 6 + 4 * i)?, le16(v, 8 + 4 * i)?))).collect::<Result<_>>()?;
    Ok(Facet { name: cstr(k), hotspot: (le16(v, 0)? as i16, le16(v, 2)? as i16), attributes })
}

fn parse_bitmap_key(k: &[u8], v: &[u8]) -> Result<BitmapKey> {
    let key = be32(k, 0)?;
    let n = le32(v, 12)? as usize;
    let masks = (0..n).map(|i| le32(v, 16 + 4 * i)).collect::<Result<_>>()?;
    Ok(BitmapKey { key, masks })
}

fn parse_rendition(k: &[u8], v: &[u8], attrs: usize) -> Result<Rendition> {
    ensure!(k.len() >= attrs * 2, "rendition key shorter than KEYFORMAT");
    let key = (0..attrs).map(|i| le16(k, 2 * i)).collect::<Result<_>>()?;
    ensure!(v.starts_with(b"ISTC"), "rendition without CSI header");
    let name = cstr(&v[40..168]);
    let ctx = || format!("rendition {name:?}");
    let tlv_len = le32(v, 168)? as usize;
    let data_len = le32(v, 180)? as usize;
    let tlv_bytes = v.get(184..184 + tlv_len).ok_or_else(|| anyhow!("TLVs past block end")).with_context(ctx)?;
    let data = v
        .get(184 + tlv_len..184 + tlv_len + data_len)
        .ok_or_else(|| anyhow!("payload past block end"))
        .with_context(ctx)?;
    let tlvs = parse_tlvs(tlv_bytes).with_context(ctx)?;
    let bytes_per_row = tlvs.iter().find_map(|t| match t.value {
        TlvValue::BytesPerRow { bytes_per_row } => Some(bytes_per_row),
        _ => None,
    });
    let (payload, decoded) = parse_payload(data, bytes_per_row.map(|b| b as usize * le32(v, 16).unwrap_or(0) as usize));
    let height = le32(v, 16)?;
    Ok(Rendition {
        key,
        layout: le16(v, 36)?,
        version: le32(v, 4)?,
        flags: le32(v, 8)?,
        width: le32(v, 12)?,
        height,
        scale: le32(v, 20)? as f64 / 100.0,
        pixel_format: fourcc(le32(v, 24)?),
        colorspace: le32(v, 28)?,
        modtime: le32(v, 32)?,
        tlvs,
        payload,
        data: decoded,
        name,
    })
}

fn parse_tlvs(mut b: &[u8]) -> Result<Vec<Tlv>> {
    let mut out = Vec::new();
    while b.len() >= 8 {
        let tag = le32(b, 0)?;
        let len = le32(b, 4)? as usize;
        let v = b.get(8..8 + len).ok_or_else(|| anyhow!("TLV {tag} overruns"))?;
        let value = decode_tlv(tag, v).unwrap_or_else(|_| TlvValue::Raw { hex: hex(v) });
        out.push(Tlv { tag, value });
        b = &b[8 + len..];
    }
    ensure!(b.is_empty(), "{} trailing TLV bytes", b.len());
    Ok(out)
}

fn attr_pairs(b: &[u8], off: usize, len: usize) -> Result<Vec<(u16, u16)>> {
    let mut out = Vec::new();
    for i in 0..len / 4 {
        let pair = (le16(b, off + 4 * i)?, le16(b, off + 4 * i + 2)?);
        if pair == (0, 0) {
            break;
        }
        out.push(pair);
    }
    Ok(out)
}

fn decode_tlv(tag: u32, v: &[u8]) -> Result<TlvValue> {
    Ok(match tag {
        1001 => {
            let n = le32(v, 0)? as usize;
            let rects = (0..n)
                .map(|i| Ok([le32(v, 4 + 16 * i)?, le32(v, 8 + 16 * i)?, le32(v, 12 + 16 * i)?, le32(v, 16 + 16 * i)?]))
                .collect::<Result<_>>()?;
            TlvValue::Slices { rects }
        }
        1003 => TlvValue::Metrics { words: (0..v.len() / 4).map(|i| le32(v, 4 * i)).collect::<Result<_>>()? },
        1004 if v.len() == 8 => TlvValue::BlendOpacity { blend: le32(v, 0)?, opacity: lef32(v, 4)? },
        1005 => {
            let n = le32(v, 0)? as usize;
            TlvValue::Uti { uti: cstr(v.get(8..8 + n).ok_or_else(|| anyhow!("short UTI"))?) }
        }
        1006 if v.len() == 4 => TlvValue::Orientation { orientation: le32(v, 0)? },
        1007 if v.len() == 4 => TlvValue::BytesPerRow { bytes_per_row: le32(v, 0)? },
        1010 => {
            ensure!(v.starts_with(b"KLNI"), "INLK magic");
            let rect = [le32(v, 8)?, le32(v, 12)?, le32(v, 16)?, le32(v, 20)?];
            // u16 12, u16 key length, then the pairs wrapped in a leading and a trailing u16 0.
            let key_len = le16(v, 26)? as usize;
            let key = attr_pairs(v, 30, key_len.saturating_sub(4))?;
            TlvValue::Link { rect, key }
        }
        1012 => {
            let n = le32(v, 0)? as usize;
            let layers = (0..n)
                .map(|i| {
                    let o = 4 + 48 * i;
                    Ok(LayerEntry {
                        frame: [le32(v, o + 8)? as i32, le32(v, o + 12)? as i32, le32(v, o + 16)? as i32, le32(v, o + 20)? as i32],
                        opacity: lef32(v, o + 28)?,
                        key: attr_pairs(v, o + 36, le32(v, o + 32)? as usize)?,
                    })
                })
                .collect::<Result<_>>()?;
            TlvValue::Layers { layers }
        }
        1021 => {
            let n = le32(v, 0)? as usize;
            ensure!(v.len() == 8 + 20 * n, "1021 is {} bytes for {n} entries", v.len());
            let entries = (0..n)
                .map(|i| {
                    let o = 4 + 20 * i;
                    Ok(LayerEffects {
                        unknown: le32(v, o)?,
                        translucency_enabled: le32(v, o + 4)?,
                        translucency: lef32(v, o + 8)?,
                        shadow_kind: le32(v, o + 12)?,
                        shadow_opacity: lef32(v, o + 16)?,
                    })
                })
                .collect::<Result<_>>()?;
            TlvValue::LayerEffects { entries }
        }
        1020 => {
            let n = le32(v, 0)? as usize;
            let mut p = 8;
            let mut entries = Vec::with_capacity(n);
            for _ in 0..n {
                let name_len = le32(v, p + 8)? as usize;
                let name = cstr(v.get(p + 12..p + 12 + name_len).ok_or_else(|| anyhow!("short 1020 name"))?);
                entries.push(LayerFill { kind: le32(v, p)?, value: lef32(v, p + 4)?, name });
                p += 12 + name_len;
            }
            ensure!(p == v.len(), "1020 has {} trailing bytes", v.len() - p);
            TlvValue::LayerFills { entries }
        }
        _ => bail!("undecoded"),
    })
}

/// The payload, plus its decoded bytes for images (CELM) and raw data (RAWD).
fn parse_payload(d: &[u8], expected_pixels: Option<usize>) -> (Payload, Option<Vec<u8>>) {
    if d.is_empty() {
        return (Payload::None, None);
    }
    let magic = String::from_utf8_lossy(&d[..d.len().min(4)]).into_owned();
    let r: Result<(Payload, Option<Vec<u8>>)> = match magic.as_str() {
        "MLEC" => Ok(parse_celm(d, expected_pixels)),
        "RLOC" => (|| {
            let n = le32(d, 12)? as usize;
            Ok((
                Payload::Color {
                    version: le32(d, 4)?,
                    colorspace: le32(d, 8)?,
                    components: (0..n).map(|i| lef64(d, 16 + 8 * i)).collect::<Result<_>>()?,
                },
                None,
            ))
        })(),
        "ARGG" => (|| {
            let n = le32(d, 4)? as usize;
            let geometry = [lef32(d, 16)?, lef32(d, 20)?, lef32(d, 24)?, lef32(d, 28)?];
            let mut p = 32;
            let mut stops = Vec::with_capacity(n);
            for _ in 0..n {
                let ln = le32(d, p + 4)? as usize;
                let name = cstr(d.get(p + 8..p + 8 + ln).ok_or_else(|| anyhow!("short stop name"))?);
                stops.push((lef32(d, p)?, name));
                p += 8 + ln;
            }
            Ok((Payload::Gradient { kind: le32(d, 8)?, reserved: le32(d, 12)?, geometry, stops }, None))
        })(),
        "SISM" => (|| {
            let n = le32(d, 8)? as usize;
            let entries = (0..n)
                .map(|i| Ok([le32(d, 12 + 12 * i)?, le32(d, 16 + 12 * i)?, le32(d, 20 + 12 * i)?]))
                .collect::<Result<_>>()?;
            Ok((Payload::Multisize { version: le32(d, 4)?, entries }, None))
        })(),
        "DWAR" => (|| {
            let (version, data_len) = (le32(d, 4)?, le32(d, 8)?);
            let body = d.get(12..12 + data_len as usize).ok_or_else(|| anyhow!("short RAWD"))?;
            let decoded = if body.starts_with(b"bvx") { Some(lzfse_decode(body, None)?) } else { None };
            let decoded_len = decoded.as_ref().map(Vec::len);
            Ok((Payload::RawData { version, data_len, decoded_len }, Some(decoded.unwrap_or_else(|| body.to_vec()))))
        })(),
        _ => Ok((Payload::Unknown { magic: hex(&d[..d.len().min(4)]), len: d.len() }, None)),
    };
    r.unwrap_or_else(|e| (Payload::Unknown { magic: format!("{magic} ({e})"), len: d.len() }, None))
}

/// LZFSE-compressed images are split into `KCBC` chunks of whole rows.
fn parse_celm(d: &[u8], expected: Option<usize>) -> (Payload, Option<Vec<u8>>) {
    let field = |off| le32(d, off).unwrap_or(0);
    let (version, compression, word) = (field(4), field(8), field(12));
    let mut chunks = None;
    let mut data_len = word;
    let r: Result<Option<Vec<u8>>> = (|| {
        let pixels = match compression {
            0 => Some(d.get(16..16 + word as usize).ok_or_else(|| anyhow!("short raw body"))?.to_vec()),
            2 => {
                use std::io::Read;
                let body = d.get(16..16 + word as usize).ok_or_else(|| anyhow!("short zip body"))?;
                let mut out = Vec::new();
                flate2::read::GzDecoder::new(body).read_to_end(&mut out)?;
                Some(out)
            }
            4 if d.get(16..20) == Some(b"KCBC") => {
                chunks = Some(word);
                data_len = (d.len() - 16) as u32;
                let mut p = 16;
                let mut out = Vec::new();
                for _ in 0..word {
                    ensure!(d.get(p..p + 4) == Some(b"KCBC"), "missing KCBC at {p}");
                    let len = le32(d, p + 16)? as usize;
                    let body = d.get(p + 20..p + 20 + len).ok_or_else(|| anyhow!("short KCBC chunk"))?;
                    out.extend(lzfse_decode(body, None)?);
                    p += 20 + len;
                }
                ensure!(p == d.len(), "{} bytes after the last KCBC chunk", d.len() - p);
                Some(out)
            }
            4 => {
                let body = d.get(16..16 + word as usize).ok_or_else(|| anyhow!("short LZFSE body"))?;
                Some(lzfse_decode(body, expected)?)
            }
            _ => None,
        };
        if let (Some(got), Some(want)) = (pixels.as_ref().map(Vec::len), expected) {
            ensure!(got == want, "decoded {got} bytes, rows × bytes-per-row is {want}");
        }
        Ok(pixels)
    })();
    let (pixels, error) = match r {
        Ok(p) => (p, None),
        Err(e) => (None, Some(e.to_string())),
    };
    let decoded_len = pixels.as_ref().map(Vec::len);
    (Payload::Celm { version, compression, data_len, chunks, decoded_len, error }, pixels)
}

fn lzfse_decode(input: &[u8], hint: Option<usize>) -> Result<Vec<u8>> {
    let mut cap = hint.unwrap_or(input.len() * 4).max(1024);
    loop {
        let mut out = vec![0u8; cap];
        match lzfse::decode_buffer(input, &mut out) {
            Ok(n) if n < cap => {
                out.truncate(n);
                return Ok(out);
            }
            Ok(_) | Err(lzfse::Error::BufferTooSmall) => {}
            Err(e) => bail!("lzfse: {e:?}"),
        }
        ensure!(cap < 1 << 30, "lzfse output exceeds 1 GiB");
        cap *= 2;
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// actool names per-build icon renditions `<stem>_<UUID>-<pid>-<timestamp>.png`; these parts
/// differ on every run, so comparisons replace them with `<id>`. The pid is decimal in Apple's
/// output and hex in ours.
pub fn normalize_name(name: &str) -> String {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}-[0-9A-F]+-[0-9A-F]{16}").unwrap()
    })
    .replace_all(name, "<id>")
    .into_owned()
}

/// Structural differences between two catalogs as `path: want X, got Y` lines, ignoring what
/// changes on every actool run: timestamps, version strings, UUIDs, per-build name suffixes,
/// compressed sizes and block layout.
pub fn diff(want: &Car, got: &Car) -> Vec<String> {
    let (a, b) = (summary(want), summary(got));
    let mut out = Vec::new();
    for (k, va) in &a {
        match b.get(k) {
            None => out.push(format!("{k}: want {va}, missing")),
            Some(vb) if vb != va => out.push(format!("{k}: want {va}, got {vb}")),
            _ => {}
        }
    }
    for (k, vb) in &b {
        if !a.contains_key(k) {
            out.push(format!("{k}: unexpected {vb}"));
        }
    }
    out
}

fn summary(car: &Car) -> std::collections::BTreeMap<String, String> {
    use serde_json::{json, Value};
    let mut m = std::collections::BTreeMap::new();
    fn flatten(path: String, v: &Value, m: &mut std::collections::BTreeMap<String, String>) {
        match v {
            Value::Object(o) => o.iter().for_each(|(k, v)| flatten(format!("{path}.{k}"), v, m)),
            Value::Array(a) if a.iter().any(|v| v.is_object() || v.is_array()) => {
                a.iter().enumerate().for_each(|(i, v)| flatten(format!("{path}[{i}]"), v, m))
            }
            _ => {
                m.insert(path, v.to_string());
            }
        }
    }
    let mut vars = car.vars.clone();
    vars.sort();
    flatten("vars".into(), &json!(vars), &mut m);
    if let Some(h) = &car.header {
        let h = json!({"coreui_version": h.coreui_version, "storage_version": h.storage_version,
            "rendition_count": h.rendition_count, "schema_version": h.schema_version,
            "color_space_id": h.color_space_id, "key_semantics": h.key_semantics});
        flatten("header".into(), &h, &mut m);
    }
    if let Some(md) = &car.metadata {
        let md = json!({"platform": md.deployment_platform, "version": md.deployment_platform_version});
        flatten("metadata".into(), &md, &mut m);
    }
    let attrs: Vec<String> = car.keyformat.iter().map(|&a| attribute_name(a)).collect();
    flatten("keyformat".into(), &json!(attrs), &mut m);
    for (name, id) in &car.appearances {
        m.insert(format!("appearance {name}"), id.to_string());
    }
    for f in &car.facets {
        let a: Vec<String> = f.attributes.iter().map(|(k, v)| format!("{}={v}", attribute_name(*k))).collect();
        m.insert(format!("facet {:?}", f.name), format!("{} hotspot {:?}", a.join(" "), f.hotspot));
    }
    for k in &car.bitmap_keys {
        m.insert(format!("bitmapkey {}", k.key), format!("{:x?}", k.masks));
    }
    for r in &car.renditions {
        let key: Vec<String> = car
            .keyformat
            .iter()
            .zip(&r.key)
            .filter(|(_, v)| **v != 0)
            .map(|(a, v)| format!("{}={v}", attribute_name(*a)))
            .collect();
        let payload = match &r.payload {
            Payload::Celm { compression, decoded_len, .. } => json!({"celm": compression, "decoded_len": decoded_len}),
            Payload::RawData { version, data_len, decoded_len } => {
                json!({"rawd": version, "len": decoded_len.unwrap_or(*data_len as usize)})
            }
            p => serde_json::to_value(p).unwrap(),
        };
        let tlvs: serde_json::Map<String, Value> = r
            .tlvs
            .iter()
            .map(|t| (format!("tlv{}", t.tag), serde_json::to_value(&t.value).unwrap()))
            .collect();
        let v = json!({"name": normalize_name(&r.name), "layout": layout_name(r.layout), "format": r.pixel_format,
            "size": [r.width, r.height], "scale": r.scale, "flags": r.flags, "colorspace": r.colorspace,
            "tlvs": tlvs, "payload": payload});
        flatten(format!("rendition [{}]", key.join(" ")), &v, &mut m);
    }
    m
}

impl Rendition {
    /// The decoded image as straight (non-premultiplied) RGBA8, for 8-bit ARGB and GA8 renditions.
    pub fn rgba8(&self) -> Option<Vec<u8>> {
        let data = self.data.as_ref()?;
        let (w, h) = (self.width as usize, self.height as usize);
        let bpp = match self.pixel_format.as_str() {
            "ARGB" => 4,
            "GA8 " => 2,
            _ => return None,
        };
        if w == 0 || h == 0 || data.len() < w * bpp * h {
            return None;
        }
        let stride = data.len() / h;
        let mut out = Vec::with_capacity(w * h * 4);
        for row in data.chunks_exact(stride).take(h) {
            for px in row[..w * bpp].chunks_exact(bpp) {
                // Both formats are premultiplied: ARGB is BGRA in memory, GA8 is gray then alpha.
                let (rgb, a) = if bpp == 4 { ([px[2], px[1], px[0]], px[3]) } else { ([px[0]; 3], px[1]) };
                let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
                out.extend([un(rgb[0]), un(rgb[1]), un(rgb[2]), a]);
            }
        }
        Some(out)
    }
}
