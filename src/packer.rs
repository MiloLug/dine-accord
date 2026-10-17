//! Packing of small images into atlases, as actool does.

pub const MARGIN: u32 = 2;
pub const GAP: u32 = 2;
/// Any value in 17145..=18123 fits Apple's sweep and corpus.
const ATLAS_AREA_PER_SCALE: u32 = 17600;

pub const PART_REGULAR: u32 = 181;
pub const PART_ICON: u32 = 184;

#[derive(Debug, Clone)]
pub struct PackedImage {
    pub name: String,
    pub identifier: u32,
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    pub pixel_data: Vec<u8>,
    pub pixel_format: [u8; 4],
    pub scale: u32,
    pub is_template: bool,
    /// bitmapEncoding: 0=original, 4=automatic, 2=template.
    pub template_rendering_intent: i32,
    pub part: u32,
    pub dim2: u32,
    pub appearance: u32,
    pub direction: u32,
    /// Device idiom (key attribute 15); 0 on macOS.
    pub idiom: u32,
    /// Rasterized from a vector source (SVG / PDF); sets CSI flag bit 0x04.
    pub is_svg_rasterization: bool,
}

impl PackedImage {
    pub fn new(name: String, identifier: u32, width: u32, height: u32) -> Self {
        Self {
            name,
            identifier,
            width,
            height,
            x: 0,
            y: 0,
            pixel_data: Vec::new(),
            pixel_format: *b"BGRA",
            scale: 1,
            is_template: false,
            template_rendering_intent: 4,
            part: PART_REGULAR,
            dim2: 0,
            appearance: 0,
            direction: 0,
            idiom: 0,
            is_svg_rasterization: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub pixel_format: [u8; 4],
    pub scale: u32,
    pub dim1: u32,
    pub images: Vec<PackedImage>,
    pub pixel_data: Vec<u8>,
}

impl Atlas {
    pub fn name(&self) -> String {
        let fmt_idx: u32 = if &self.pixel_format == b"BGRA" { 0 } else { 1 };
        // Every atlas of a (scale, format) has the same name: Dimension1 in the key tells them
        // apart and INLK links resolve by key, so duplicate names are expected.
        format!("ZZZZPackedAsset-{}.0.{}-gamut0", self.scale, fmt_idx)
    }

    fn bpp_for_format(pf: &[u8]) -> usize {
        // "61AG" is GA16.
        if pf == b"BGRA" || pf == b"61AG" {
            4
        } else {
            2
        }
    }

    pub fn bytes_per_row(&self) -> usize {
        let bpp = Self::bpp_for_format(&self.pixel_format);
        let exact = self.width as usize * bpp;
        ((exact + 31) / 32) * 32
    }

    /// Blit the packed images into one buffer with 32-byte-aligned rows (the encoder's stride).
    /// Positions are bottom-up, like actool's packer and the INLK rects.
    pub fn render(&mut self) {
        let bpp = Self::bpp_for_format(&self.pixel_format);
        let bpr = self.bytes_per_row();
        let mut buf = vec![0u8; bpr * self.height as usize];

        for img in &self.images {
            let src_stride = img.width as usize * bpp;
            for row in 0..img.height as usize {
                let src_off = row * src_stride;
                let top = (self.height - img.y - img.height) as usize;
                let dst_off = (top + row) * bpr + img.x as usize * bpp;
                if src_off + src_stride <= img.pixel_data.len() {
                    buf[dst_off..dst_off + src_stride]
                        .copy_from_slice(&img.pixel_data[src_off..src_off + src_stride]);
                }
            }
        }
        self.pixel_data = buf;
    }
}

#[derive(Clone)]
struct Node {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    used: bool,
    right: Option<usize>,
    down: Option<usize>,
}

/// Growing binary-tree packer: blocks go into the first free node (right before down), and when
/// none fits the canvas grows right or down, whichever keeps it closer to square.
#[derive(Clone)]
struct Packer {
    nodes: Vec<Node>,
    root: usize,
}

impl Packer {
    fn new(w: u32, h: u32) -> Self {
        let mut p = Packer { nodes: Vec::new(), root: 0 };
        p.root = p.node(0, 0, w, h);
        p
    }

    fn node(&mut self, x: u32, y: u32, w: u32, h: u32) -> usize {
        self.nodes.push(Node { x, y, w, h, used: false, right: None, down: None });
        self.nodes.len() - 1
    }

    fn find(&self, n: Option<usize>, w: u32, h: u32) -> Option<usize> {
        let node = &self.nodes[n?];
        if node.used {
            self.find(node.right, w, h).or_else(|| self.find(node.down, w, h))
        } else if w <= node.w && h <= node.h {
            n
        } else {
            None
        }
    }

    fn split(&mut self, n: usize, w: u32, h: u32) -> (u32, u32) {
        let Node { x, y, w: nw, h: nh, .. } = self.nodes[n];
        let down = self.node(x, y + h, nw, nh - h);
        let right = self.node(x + w, y, nw - w, h);
        let node = &mut self.nodes[n];
        node.used = true;
        node.down = Some(down);
        node.right = Some(right);
        (x, y)
    }

    fn insert(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if let Some(n) = self.find(Some(self.root), w, h) {
            return Some(self.split(n, w, h));
        }
        let (rw, rh) = (self.nodes[self.root].w, self.nodes[self.root].h);
        let (can_down, can_right) = (w <= rw, h <= rh);
        let should_right = can_right && rh >= rw + w;
        let should_down = can_down && rw >= rh + h;
        let old = self.root;
        if should_right || (!should_down && can_right) {
            let strip = self.node(rw, 0, w, rh);
            self.root = self.node(0, 0, rw + w, rh);
            self.nodes[self.root] = Node { used: true, right: Some(strip), down: Some(old), ..self.nodes[self.root].clone() };
        } else if can_down {
            let strip = self.node(0, rh, rw, h);
            self.root = self.node(0, 0, rw, rh + h);
            self.nodes[self.root] = Node { used: true, right: Some(old), down: Some(strip), ..self.nodes[self.root].clone() };
        } else {
            return None;
        }
        let n = self.find(Some(self.root), w, h)?;
        Some(self.split(n, w, h))
    }

    fn size(&self) -> (u32, u32) {
        (self.nodes[self.root].w + MARGIN, self.nodes[self.root].h + MARGIN)
    }
}

/// actool's atlas split and layout: an atlas takes no more images once their pixel area reaches
/// `ATLAS_AREA_PER_SCALE × scale`. Reproduces every atlas of Apple's iOS app-icon sweep
/// (`tools/atlas_sweep_dataset.json`) and of the corpus' multi-icon catalogs.
pub fn pack_images_split(mut images: Vec<PackedImage>) -> Vec<Atlas> {
    images.sort_by(|a, b| {
        b.width.max(b.height).cmp(&a.width.max(a.height)).then(a.identifier.cmp(&b.identifier))
    });
    let mut packs: Vec<(Packer, Atlas)> = Vec::new();
    for mut img in images {
        let (w, h) = (img.width + GAP, img.height + GAP);
        if let Some((packer, atlas)) = packs.last_mut() {
            let area: u32 = atlas.images.iter().map(|i| i.width * i.height).sum();
            if area < ATLAS_AREA_PER_SCALE * img.scale {
                if let Some((x, y)) = packer.insert(w, h) {
                    (img.x, img.y) = (x + MARGIN, y + MARGIN);
                    atlas.images.push(img);
                    continue;
                }
            }
        }
        let mut packer = Packer::new(w, h);
        packer.insert(w, h);
        (img.x, img.y) = (MARGIN, MARGIN);
        let atlas = Atlas { pixel_format: img.pixel_format, scale: img.scale, images: vec![img], ..Default::default() };
        packs.push((packer, atlas));
    }
    packs
        .into_iter()
        .map(|(packer, mut atlas)| {
            (atlas.width, atlas.height) = packer.size();
            atlas
        })
        .collect()
}

/// Split renditions into pack groups `(pixel_format, scale, indices)` and inline indices.
pub fn group_for_packing(
    renditions: &[crate::car::Rendition],
) -> (Vec<([u8; 4], u16, Vec<usize>)>, Vec<usize>) {
    use crate::car;

    const ICON_INLINE_THRESHOLD: u32 = 256;
    const PACK_MAX_WIDTH: u32 = 262;
    const PACK_MAX_HEIGHT: u32 = 196;
    const PACK_MARGIN: u32 = 4;

    // Apple never mixes idioms in an atlas.
    let mut groups: indexmap::IndexMap<([u8; 4], u16, u16, u16), Vec<usize>> =
        indexmap::IndexMap::new();
    let mut force_inline: Vec<usize> = Vec::new();

    for (i, rend) in renditions.iter().enumerate() {
        match rend.layout {
            car::LAYOUT_MULTISIZE_IMAGE
            | car::LAYOUT_COLOR
            | car::LAYOUT_RAW_DATA
            | car::LAYOUT_METADATA => {
                force_inline.push(i);
                continue;
            }
            _ => {}
        }
        if rend.part == car::PART_ICON && rend.width >= ICON_INLINE_THRESHOLD {
            force_inline.push(i);
            continue;
        }
        // actool stores subtype icons (the synthesized 90pt Plus icon, 1792) inline.
        if rend.subtype != 0 {
            force_inline.push(i);
            continue;
        }
        if rend.csi_override.is_some() {
            force_inline.push(i);
            continue;
        }
        if rend.width >= PACK_MAX_WIDTH - PACK_MARGIN
            || rend.height >= PACK_MAX_HEIGHT - PACK_MARGIN
        {
            force_inline.push(i);
            continue;
        }
        let key = (rend.pixel_format, rend.scale, rend.sprite_atlas_id, rend.idiom);
        groups.entry(key).or_default().push(i);
    }

    let mut pack_groups: Vec<([u8; 4], u16, Vec<usize>)> = Vec::new();
    let mut inline: Vec<usize> = force_inline;

    let mut keys: Vec<_> = groups.keys().cloned().collect();
    keys.sort_by_key(|k| (k.0, k.1, k.2, k.3));
    for k in keys {
        let idxs = groups.shift_remove(&k).unwrap();
        // iOS app icons all share the icon facet's identifier, so they count as distinct by
        // size (dim2).
        let ios_icon_group = idxs.iter().all(|i| {
            let r = &renditions[*i];
            r.part == car::PART_ICON && car::is_idiom_platform(&r.platform)
        });
        let distinct = if ios_icon_group {
            idxs.iter()
                .map(|i| (renditions[*i].identifier, renditions[*i].dim2))
                .collect::<std::collections::HashSet<_>>()
                .len()
        } else {
            idxs.iter()
                .map(|i| renditions[*i].identifier)
                .collect::<std::collections::HashSet<_>>()
                .len()
        };
        if distinct >= 2 {
            pack_groups.push((k.0, k.1, idxs));
        } else {
            inline.extend(idxs);
        }
    }

    (pack_groups, inline)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(name: &str, w: u32, h: u32) -> PackedImage {
        PackedImage::new(name.to_string(), 0, w, h)
    }

    #[test]
    fn empty_packing() {
        let atlases = pack_images_split(vec![]);
        assert!(atlases.is_empty());
    }

    #[test]
    fn single_image_placed_with_margin() {
        let atlases = pack_images_split(vec![img("a", 10, 10)]);
        assert_eq!(atlases.len(), 1);
        let atlas = &atlases[0];
        assert_eq!(atlas.images.len(), 1);
        assert_eq!(atlas.images[0].x, MARGIN);
        assert_eq!(atlas.images[0].y, MARGIN);
        assert_eq!(atlas.width, MARGIN + 10 + MARGIN);
    }

    #[test]
    fn atlas_name_format() {
        let atlas = Atlas {
            scale: 2,
            dim1: 3,
            pixel_format: *b"BGRA",
            ..Default::default()
        };
        assert_eq!(atlas.name(), "ZZZZPackedAsset-2.0.0-gamut0");
        let atlas = Atlas {
            scale: 1,
            dim1: 0,
            pixel_format: *b" 8AG",
            ..Default::default()
        };
        assert_eq!(atlas.name(), "ZZZZPackedAsset-1.0.1-gamut0");
    }

    #[test]
    fn bytes_per_row_alignment() {
        let atlas = Atlas {
            width: 10,
            pixel_format: *b"BGRA",
            ..Default::default()
        };
        assert_eq!(atlas.bytes_per_row(), 64);

        let atlas = Atlas {
            width: 10,
            pixel_format: *b" 8AG",
            ..Default::default()
        };
        assert_eq!(atlas.bytes_per_row(), 32);
    }

    #[test]
    fn render_copies_pixels() {
        let mut atlas = Atlas {
            width: 10,
            height: 10,
            pixel_format: *b"BGRA",
            ..Default::default()
        };
        let mut pi = img("a", 2, 2);
        pi.x = 0;
        pi.y = 0;
        pi.pixel_data = vec![
            1, 2, 3, 4, 5, 6, 7, 8, // row 0
            9, 10, 11, 12, 13, 14, 15, 16, // row 1
        ];
        atlas.images = vec![pi];
        atlas.render();
        // y = 0 is the bottom edge: the image lands in the last two rows, upright.
        let bpr = atlas.bytes_per_row();
        assert_eq!(&atlas.pixel_data[8 * bpr..8 * bpr + 8], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&atlas.pixel_data[9 * bpr..9 * bpr + 8], &[9, 10, 11, 12, 13, 14, 15, 16]);
    }
}
