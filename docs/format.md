# `.car` format notes

Facts read from actool output: the corpus in `tests/corpus/corpus.toml` (CoreUI 308 to 1010;
macOS, iOS, tvOS and Mac Catalyst), and other `Assets.car` files committed to public GitHub
repositories, named where cited. `src/car_read.rs` decodes all of it; `accord inspect [--json]` prints it,
`accord diff` compares two catalogs, `accord extract` writes the images as PNG.

## BOM container

- Big-endian: header, block index, vars, tree headers, tree pages, BITMAPKEYS' inline keys.
- Header `BOMStore`, version 1, block count, index and vars offset/length; zero up to 0x200, where
  block 1 starts. Block 0 is (0, 0).
- Blocks are 16-byte aligned and zero-padded. A value block is its key block + 1. Identical
  payloads share one block.
- File order: CARHEADER, RENDITIONS tree + leaf, FACETKEYS, APPEARANCEKEYS, appearance and facet
  key/value pairs, KEYFORMAT, rendition key/value pairs, META, BITMAPKEYS tree + leaf + values,
  vars, index. The index has 256 slots and no free list; the file ends with it.
- Tree header (29 bytes): `tree`, version 1, child, page size, path count, u8 inline-keys flag,
  i32 key size, u32 0. Every tree in the corpus is a single leaf.
  - RENDITIONS: page 4096, key size = key length; the keys repeat after the entry table (4-byte
    gap) and the leaf is `4096 + n × key size` long. Entries sorted by memcmp of the key.
  - FACETKEYS, APPEARANCEKEYS: key size -1, sorted by strcmp.
  - BITMAPKEYS: page 1024, inline flag 1, key size 0; the key is the u32 itself.

## CoreUI blocks (little-endian)

- CARHEADER (436 bytes): `RATC`, coreUI version, storage version 17, timestamp 0, rendition
  count, main version string[128], version string[256], UUID (zero), checksum 0, schema version 2,
  colorspace 1, key semantics (1 macOS, 2 iOS).
- META (1028 bytes): thinning arguments, deployment platform version, platform, authoring tool,
  256 bytes each.
- KEYFORMAT: `tmfk`, 0, count, u32 attribute ids. A rendition key is the u16 values in that order.
  Layouts seen: `[7,13,1,2,3,17,9,11,12]`, the same plus 24 and/or 8 (before 9), and
  `[7,1,2,17,9,10,14,12,19,18]` (Xcode 26.3 brave-origin). The order is a fixed per-platform list
  with the attributes no rendition uses dropped; catalogs from older Xcode keep the whole list:
  macOS `[7,13,1,2,3,4,6,17,8,9,10,14,11,12,24,19,18,25,26,27]`, iOS
  `[7,13,12,15,16,26,27,9,8,25,24,4,20,21,23,22,17,1,2,10,6]`. So DisplayGamut (24) follows Scale
  on macOS and Dimension1 on iOS (`[7,13,12,15,16,9,24,17,1,2]` in osu, sentry, dusklight).
- APPEARANCEKEYS for `.icon`: System 0, DarkAqua 1, Aqua 8, ISAppearanceTintable 10.
- FACETKEYS value: i16 hotspot x, y, u16 count, (u16 id, u16 value) pairs: Element 85, Part,
  Identifier.
- BITMAPKEYS value: 1, 0, 4(n+1), n, then one u32 mask per KEYFORMAT attribute (bitset of the
  values used; `0xFFFFFFFF` unconstrained).
- Parts: 181 image/SVG, 217 color, 218 multisize set, 220 icon image, 245 icon stack, 246 icon
  group, 247 gradient. The packed atlas uses Element 9.

## Renditions

CSI header (184 bytes): `ISTC`, version 1, flags, width, height, scale × 100, pixel format,
colorspace, modtime 0, u16 layout, u16 0, name[128], TLV length, 1, 0, payload length.

TLVs: 1001 slices, 1003 metrics, 1004 blend + opacity, 1005 UTI (u32 length incl. NUL, u32 0,
string; a dataset's `universal-type-identifier`, read by `NSDataAsset.typeIdentifier`), 1006 EXIF orientation,
1007 bytes per row, which must equal the payload's row stride (KCBC/LZFSE: w × bpp
rounded up to 32, so GA8 is 2w-based; deepmap2: w × 4 for 8-bit formats, GA8 too, or w × 8 for
GA16 and RGBW, rounded up to 32), 1010 `INLK`
atlas link, and for icon stacks
(1019) and groups (1020):
- 1012 layers: count, then 48-byte entries (u32 0, u32 0, i32 x, y, w, h in a 1024 canvas,
  u32 0, f32 opacity, u32 16, attribute pairs Element/Part/Identifier), then 4 zero bytes. Stacks
  list fill first, then groups bottom to top (icon.json lists them top to bottom).
- 1020: count, 0, then per entry u32 kind, f32, u32 name length incl. NUL, name.
- 1021: count, then 20-byte entries, then 4 zero bytes. In stacks the group entry is
  (u32 0, translucency enabled, f32 translucency, shadow kind (2 layer-color, 3 neutral), f32
  shadow opacity), matching icon.json; the fill entry is all zero.
- 1022: Xcode 27 only, meaning unknown.

Payloads:
- `MLEC` (CELM): version, compression, then for LZFSE (4) a chunk count and `KCBC` chunks
  (`KCBC`, 0, 0, rows, length, LZFSE stream); otherwise a length and the data. Version bit 0 marks
  chunks, bit 1 an opaque image.
- Compression 11 (deepmap2): a 16-byte header (1, format code, stream length, 0) and a `dmp2`
  stream. Images of 2^20 pixels or more are `KCBC` chunks of 2^20 / w rows, each with its own
  header and stream. Format codes: 2 GA8, 4 ARGB, 18 GA16, 20 RGBW.
- `dmp2` stream: `dmp2`, method, 1, 10, format code, u16 tile width, u16 tile height, then tiles
  row by row, each a u32 length and a body compressed with LZVN (raw stream) below 4 KiB of
  input and LZFSE above. Tiles are at most 16384 wide and 2 MiB decoded. actool encodes with
  methods 3, 4 (ARGB only) and 2, keeps the smallest, and falls back to 1 (raw, untiled) when
  none beats the raw size:
  - 3: the tile's pixels.
  - 4: a palette of at most 256 colours, as `count | kind << 16` and the entries after the header;
    kind 4 stores whole pixels and one index plane, kind 3 24-bit colours plus an alpha plane
    before the index plane. actool tries both kinds and keeps the smaller.
  - 2: each pixel as Y, Co, Cg in 16 bits (YCoCg-R; Co and Cg halved, so this method drops their
    low bit; 16-bit floats scaled by 512 first), and alpha in 8 bits. The body is the alpha
    plane, one predictor byte per row (0 none, 1 left or up chosen per pixel from Y, 2 left,
    3 up, 4 mean of left and up), then the high bytes and the low bytes of the residuals in
    sign-magnitude form (2v, or −2v+1 when negative), padded to 16 bytes. Each row takes the
    predictor with the smallest sum of ln(|residual| + 1).
- Pixel formats: `ARGB` (BGRA bytes, premultiplied), `GA8 `, `GA16`, and `RGBW`: premultiplied
  RGBA, a little-endian f16 per channel, in extended sRGB (colorspace 4), values outside 0..1
  kept.
- `RLOC` color: 1, colorspace (3 display-p3, 6 extended-gray, 2 generated dark greys), count,
  f64 components. icon.json values are stored as `(double)(float)`.
- `ARGG` gradient: stop count, kind (0 automatic, 1 linear), 0, f32 start x, y, stop x, y, then
  per stop f32 location, u32 name length, color facet name.
- `SISM` multisize: 1, count, (w, h, Dimension2 index).
- `DWAR` raw data: version (1 = LZFSE-compressed SVG), length, data. Icon stacks and groups carry
  an empty `DWAR` (version 0, length 0).
- Atlases are named `ZZZZPackedAsset-<scale>.<opaque>.<monochrome>-gamut<DisplayGamut>`; packed
  images are `InternalReference` renditions whose `INLK` gives the rect (2 px gutter) and the
  atlas key (Element, Part, Dimension1, Identifier, Scale, Idiom, Subtype, Appearance,
  DisplayGamut, each when non-zero).
- Per-build names embed `<UUID>-<pid>-<timestamp>`; compare with `car_read::normalize_name`.

## Key attributes

- 24 is DisplayGamut, not an appearance axis. A PNG gets a second rendition at gamut 1 when its
  ICC profile is wide (the xy triangle of its colorants is larger than 0.13447: Display P3 and
  Adobe RGB are, sRGB and Cinema Display Calibrated are not) and some pixel, premultiplied in
  extended sRGB, falls outside -0.025..1.003. A wide profile with in-gamut pixels gets none (a
  16-px Adobe RGB icon in openzfs). The copy is `RGBW` in colorspace 4 with the same flags; the
  gamut-0 rendition holds the same colours clipped to sRGB (openrct2's `Slide.png`, wireshark's
  `Fin.png`, OpenUtauMobile's 16-bit P3 app icon). An image with more than 8 bits a channel whose
  pixels are gray in 8-bit sRGB gets a `GA16` copy instead (premultiplied f16 gray and alpha,
  colorspace 6; readinglist's 16-bit RGBA `eyeglasses.png`), and the macOS icons rendered
  from an `.icon` with wide colours get `RGBW` copies too (Zettlr). JPEGs get none.
- The facet holding a copy has BITMAPKEYS mask 3 for 24, every other facet 1. Copies of small
  images pack into their own `-gamut1` atlases, whose Dimension1 is counted apart from gamut 0;
  their `INLK` key ends with (24, 1) after Scale. A copy that would be alone in its atlas stays
  inline, as any lone image does.
- Dimension2 of an app-icon rendition is the rank of its point size among all sizes of the set
  (target devices, universal and marketing only): 20, 29, 38, 40, 60, 76, 83.5, 90, 1024 give
  1..9 in a full iOS set, 1024 alone gives 1. The synthesized 90pt@2x Plus icon (subtype 1792)
  counts, one per appearance of the 60pt@3x icon.
- BITMAPKEYS masks are -1 for Element, Part and Identifier everywhere, and for Appearance in
  `.icon` and iOS catalogs. Other attributes set bit `value` (below 32), Localization bit 1 for
  any locale, and Subtype a bit per device subtype (iOS: 0→0, 568→1, 569→2, 570→3, 2732→4,
  163→5, 2420→7, 2752→8, 2034→9, 2224→10, 2436→11, 2688→12, 2622→13, 2868→14, 2388→15,
  1792→16, 32401→17, 2340→18, 2778→19, 2360→20, 2160→21, 2532→22, 2266→25, 2556→29, 2796→30,
  2736→31).

## Atlases

Images of at most 18050 × scale pixels pack into atlases, grouped by scale, opacity,
monochrome, subtype, idiom, gamut and appearance (images of every document of an invocation
share them); a group of one stays inline. [atlas-packing.md](atlas-packing.md) has the split,
the layout, the atlas size and the Dimension1 numbering. Rects in `INLK` are bottom-up.

## Icon Composer `.icon`

- Stacks and groups exist once per appearance: DarkAqua 1, Aqua 8, Tintable 10 on macOS; Dark 1,
  Light 4, Tintable 10 on iOS. A stack lists the fill (the light fill for Aqua/Light/Tintable, the
  dark gradient for Dark; a solid fill's colour is a Color facet), then the groups back to front.
- Group entries: TLV 1020 kind 2 with the group's blur material; TLV 1021 shadow kind none 0,
  layer-color 2, neutral 3, and translucency 0 when disabled.
- IconGroup entries, back to front: frame = floor of the layer's native size × layer scale ×
  group scale, centred, offset by translation × group scale (+ group translation); TLV 1020 kind
  1 unless the layer says `glass: false`, name = the layer's fill facet for that appearance.
- Palette: colours deduplicate (a white automatic-gradient colour is Color-1); a keyword fill
  (`system-dark`, dark `automatic`) is a gradient named `system-dark`/`system-light` that still
  takes a gradient number; repeated group names get " 2", " 3".
- Layer images larger than the 1024-px canvas are stored at 1024; the slices TLV keeps the source
  size.
- macOS pre-renders 16–1024 pt (16/32/64 @2x packed) with a squircle, margin and shadow. iOS
  pre-renders one full-bleed 1024 per target device for Any, Dark and Tintable, writes
  `<icon>60x60@2x.png` / `<icon>76x76@2x~ipad.png` and the `CFBundleIcons` plist, and no `.icns`.
- Since Xcode 26.1 an `.appiconset` named like an `.icon` in the same invocation is ignored.
- The main facet's part is 220 or 245 seemingly at random (brave beta vs nightly, same commit).
