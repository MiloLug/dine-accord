# `.car` format notes

Facts read from actool output (the corpus in `tests/corpus/corpus.toml`: Xcode 26.2 – 27.1,
CoreUI 972 – 1010, macOS and iOS). `src/car_read.rs` decodes all of it; `accord inspect [--json]`
prints it, `accord diff` compares two catalogs, `accord extract` writes the images as PNG.

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
  `[7,1,2,17,9,10,14,12,19,18]` (Xcode 26.3 brave-origin).
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
rounded up to 32, so GA8 is 2w-based; deepmap2: 4 × round_up(w, 8) for GA8 too), 1010 `INLK`
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
  (`KCBC`, 0, 0, rows, length, LZFSE stream); otherwise a length and the data. Compression 11 is
  deepmap2 (`dmp2`), not decoded yet.
- `RLOC` color: 1, colorspace (3 display-p3, 6 extended-gray, 2 generated dark greys), count,
  f64 components. icon.json values are stored as `(double)(float)`.
- `ARGG` gradient: stop count, kind (0 automatic, 1 linear), 0, f32 start x, y, stop x, y, then
  per stop f32 location, u32 name length, color facet name.
- `SISM` multisize: 1, count, (w, h, Dimension2 index).
- `DWAR` raw data: version (1 = LZFSE-compressed SVG), length, data. Icon stacks and groups carry
  an empty `DWAR` (version 0, length 0).
- Atlases are named `ZZZZPackedAsset-<scale>.0.<n>-gamut0`; small sizes are `InternalReference`
  renditions whose `INLK` gives the rect (2 px gutter) and the atlas key.
- Per-build names embed `<UUID>-<pid>-<timestamp>`; compare with `car_read::normalize_name`.

## Key attributes

- 24 is DisplayGamut: atlases are named `…-gamut0`/`gamut1`, and a wide-gamut (P3 / 16-bit) layer
  PNG gets a second rendition at gamut 1 in `RGBW`. It is not an appearance axis.
- Dimension2 of an app-icon rendition is the rank of its point size among all sizes of the set
  (target devices, universal and marketing only): 20, 29, 38, 40, 60, 76, 83.5, 90, 1024 give
  1..9 in a full iOS set, 1024 alone gives 1. The synthesized 90pt@2x Plus icon (subtype 1792)
  counts, one per appearance of the 60pt@3x icon.
- BITMAPKEYS masks are -1 for Element, Part and Identifier everywhere, and for Appearance in
  `.icon` and iOS catalogs.

## Atlases

actool packs small images per (scale, idiom, appearance, format) with a growing binary-tree
packer (Jake Gordon's): images sorted largest first, ties by identifier, each padded by 2 px; a
block goes into the first free node (right before down), otherwise the canvas grows right or
down, whichever keeps it squarer. An atlas takes no more images once the pixel area it holds
reaches about 17600 × scale (any value in 17145–18123 fits). Rects in `INLK` are bottom-up, and
Dimension1 counts atlases per (scale, idiom, appearance). Images of every document of an
invocation share atlases. This reproduces all 68 atlases of upstream's iOS sweep and the corpus.

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
- macOS pre-renders 16–1024 pt (16/32/64 @2x packed) with a squircle, margin and shadow. iOS
  pre-renders one full-bleed 1024 per target device for Any, Dark and Tintable, writes
  `<icon>60x60@2x.png` / `<icon>76x76@2x~ipad.png` and the `CFBundleIcons` plist, and no `.icns`.
- Since Xcode 26.1 an `.appiconset` named like an `.icon` in the same invocation is ignored.
- The main facet's part is 220 or 245 seemingly at random (brave beta vs nightly, same commit).
