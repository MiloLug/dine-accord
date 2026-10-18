//! Packing of small images into atlases, as actool does.

pub const MARGIN: u32 = 2;
pub const GAP: u32 = 2;

pub const PART_REGULAR: u32 = 181;

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
    /// Key attribute values in the platform's full key order, which break area ties.
    pub sort_key: Vec<u16>,
    /// The caller's index of the rendition.
    pub index: usize,
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
            sort_key: Vec::new(),
            index: 0,
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
    /// DisplayGamut of the images.
    pub gamut: u32,
    pub opaque: bool,
    pub images: Vec<PackedImage>,
    pub pixel_data: Vec<u8>,
}

impl Atlas {
    pub fn name(&self) -> String {
        let monochrome = matches!(&self.pixel_format, b" 8AG" | b"61AG") as u32;
        // Atlases sharing these fields share a name: Dimension1 in the key tells them apart and
        // INLK links resolve by key.
        format!("ZZZZPackedAsset-{}.{}.{}-gamut{}", self.scale, self.opaque as u32, monochrome, self.gamut)
    }

    fn bpp_for_format(pf: &[u8]) -> usize {
        crate::car::bytes_per_pixel(pf) as usize
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
    /// Holds an image (set on the node an image was placed in).
    fit: bool,
    right: Option<usize>,
    down: Option<usize>,
}

/// actool's growing binary-tree packer: blocks go into the first free node (right before down),
/// and when none fits the canvas grows right or down, whichever keeps it closer to square.
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
        self.nodes.push(Node { x, y, w, h, used: false, fit: false, right: None, down: None });
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

    fn split(&mut self, n: usize, w: u32, h: u32) -> usize {
        let Node { x, y, w: nw, h: nh, .. } = self.nodes[n];
        let down = self.node(x, y + h, nw, nh - h);
        let right = self.node(x + w, y, nw - w, h);
        let node = &mut self.nodes[n];
        node.used = true;
        node.down = Some(down);
        node.right = Some(right);
        n
    }

    fn grow(&mut self, w: u32, h: u32) -> Option<usize> {
        let (rw, rh) = (self.nodes[self.root].w, self.nodes[self.root].h);
        let (can_down, can_right) = (w <= rw, h <= rh);
        let should_right = can_right && rh >= rw + w;
        let should_down = can_down && rw >= rh + h;
        let old = self.root;
        if should_right || (!should_down && can_right) {
            let strip = self.node(rw, 0, w, rh);
            self.root = self.node(0, 0, rw + w, rh);
            let root = &mut self.nodes[self.root];
            (root.used, root.right, root.down) = (true, Some(strip), Some(old));
        } else {
            let strip = self.node(0, rh, rw, h);
            self.root = self.node(0, 0, rw, rh + h);
            let root = &mut self.nodes[self.root];
            (root.used, root.right, root.down) = (true, Some(old), Some(strip));
        }
        let n = self.find(Some(self.root), w, h)?;
        Some(self.split(n, w, h))
    }

    fn insert(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let n = match self.find(Some(self.root), w, h) {
            Some(n) => self.split(n, w, h),
            None => self.grow(w, h)?,
        };
        self.nodes[n].fit = true;
        Some((self.nodes[n].x, self.nodes[n].y))
    }

    /// Nodes under the root that hold no image, not counting below such a node (so a grown
    /// canvas's old root counts as one).
    fn empty_nodes(&self) -> usize {
        fn count(p: &Packer, n: usize) -> usize {
            let node = &p.nodes[n];
            if !node.fit {
                return 1;
            }
            node.down.map_or(0, |d| count(p, d)) + node.right.map_or(0, |r| count(p, r))
        }
        let root = &self.nodes[self.root];
        root.down.map_or(0, |d| count(self, d)) + root.right.map_or(0, |r| count(self, r))
    }

    fn enclosing(&self) -> (u32, u32) {
        (self.nodes[self.root].w, self.nodes[self.root].h)
    }
}

/// Packs `images` in order, each padded by `GAP`; returns the packer and each image's
/// position, or `None` for an image that fits nowhere.
fn layout(images: &[PackedImage]) -> (Packer, Vec<Option<(u32, u32)>>) {
    let mut packer = Packer::new(images[0].width + GAP, images[0].height + GAP);
    let fits = images.iter().map(|i| packer.insert(i.width + GAP, i.height + GAP)).collect();
    (packer, fits)
}

/// Images whose pixel area exceeds this × scale stay out of atlases.
pub const MAX_PACKED_AREA_PER_SCALE: u32 = 18050;

/// actool's split of one packing group, largest area first (ties by rendition key): a chunk
/// takes images until their total area reaches `limit`, keeping the image that crosses it; when the images before it leave empty nodes and as many more images
/// pack into the same canvas, it takes those too. Returns each chunk's ordinal, counting the
/// steps that made none (an image whose own area crosses the limit stays unpacked).
pub fn split_group(mut images: Vec<PackedImage>, limit: u32) -> (Vec<(usize, Vec<PackedImage>)>, Vec<PackedImage>) {
    images.sort_by(|a, b| (b.width * b.height).cmp(&(a.width * a.height)).then_with(|| a.sort_key.cmp(&b.sort_key)));
    let n = images.len();
    let limit = limit as f64;
    let mut chunks = Vec::new();
    let mut ranges = Vec::new();
    let (mut start, mut step) = (0, 0);
    while start < n {
        let mut end = n;
        let mut area = 0.0;
        for i in start..n {
            area += (images[i].width * images[i].height) as f64;
            if area >= limit {
                end = i;
                if i > start {
                    let (first, _) = layout(&images[start..i]);
                    let extended = i + first.empty_nodes();
                    if extended < n {
                        let (more, _) = layout(&images[start..extended]);
                        let (a, b) = (first.enclosing(), more.enclosing());
                        if a.0 * a.1 == b.0 * b.1 {
                            end = extended;
                        }
                    }
                }
                break;
            }
        }
        if end > start {
            let len = if end == n { n - start } else { end - start + 1 };
            ranges.push((step, start..start + len));
        }
        start = end + 1;
        step += 1;
    }
    let mut taken = vec![false; n];
    for (step, r) in &ranges {
        r.clone().for_each(|i| taken[i] = true);
        chunks.push((*step, images[r.clone()].to_vec()));
    }
    let rest = images.into_iter().zip(taken).filter(|(_, t)| !t).map(|(i, _)| i).collect();
    (chunks, rest)
}

/// Lays out one chunk: images at their packed positions plus `MARGIN`, the canvas one pixel wider
/// and taller than the padded layout, rounded up to even.
pub fn pack_chunk(images: Vec<PackedImage>) -> Atlas {
    let (packer, fits) = layout(&images);
    let even = |v: u32| (v + 1).next_multiple_of(2);
    let (w, h) = packer.enclosing();
    let mut atlas = Atlas {
        width: even(w),
        height: even(h),
        pixel_format: images[0].pixel_format,
        scale: images[0].scale,
        ..Default::default()
    };
    for (mut img, fit) in images.into_iter().zip(fits) {
        if let Some((x, y)) = fit {
            (img.x, img.y) = (x + MARGIN, y + MARGIN);
            atlas.images.push(img);
        }
    }
    atlas
}

/// Whether actool packs a rendition: a raster image, not a JPEG, of at most
/// `MAX_PACKED_AREA_PER_SCALE × scale` pixels.
pub fn can_be_packed(r: &crate::car::Rendition) -> bool {
    use crate::car;
    r.layout == car::LAYOUT_ONE_PART_SCALE
        && r.csi_override.is_none()
        && r.width > 0
        && r.height > 0
        && !r.pixel_data.is_empty()
        && r.width * r.height <= MAX_PACKED_AREA_PER_SCALE * r.scale as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(name: &str, w: u32, h: u32) -> PackedImage {
        PackedImage::new(name.to_string(), 0, w, h)
    }

    #[test]
    fn empty_group_has_no_chunks() {
        let (chunks, rest) = split_group(vec![], 18050);
        assert!(chunks.is_empty() && rest.is_empty());
    }

    #[test]
    fn single_image_placed_with_margin() {
        let atlas = pack_chunk(vec![img("a", 10, 10)]);
        assert_eq!(atlas.images.len(), 1);
        assert_eq!((atlas.images[0].x, atlas.images[0].y), (MARGIN, MARGIN));
        // 12-pixel padded layout, plus one, rounded up to even.
        assert_eq!((atlas.width, atlas.height), (14, 14));
    }

    #[test]
    fn image_crossing_the_limit_alone_stays_unpacked() {
        let (chunks, rest) = split_group(vec![img("big", 100, 100), img("a", 10, 10), img("b", 10, 10)], 9000);
        assert_eq!(rest.len(), 1);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].0, 1);
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
