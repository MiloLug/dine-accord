# `.car` format

`Assets.car` is a BOMStore file whose named blocks hold CoreUI's catalog: a header, the rendition
tree, the facet and appearance names, the key format, metadata and the bitmap-key index. This page
describes the container, the blocks and the renditions as actool writes them. Image payload
encodings are in [codecs.md](codecs.md), atlas packing in [atlas-packing.md](atlas-packing.md),
Icon Composer catalogs in [icon-bundle.md](icon-bundle.md), app icons and the files written next
to the catalog in [app-icons.md](app-icons.md).

Every fact here is read from actool output: the corpus listed in `tests/corpus/corpus.toml` (CoreUI
308 to 1010; macOS, iOS, tvOS and Mac Catalyst) and other `Assets.car` files committed to public
GitHub repositories, named where cited. `accord inspect --json` prints all of it for any catalog.

## BOM container

- Big-endian: header, block index, vars, tree headers, tree pages, BITMAPKEYS' inline keys.
- Header `BOMStore`, version 1, block count, index offset and length, vars offset and length;
  zero up to 0x200, where block 1 starts. Block 0 is (0, 0).
- Blocks are 16-byte aligned and zero-padded. A tree entry's value block is its key block + 1.
  Byte-identical payloads still get a block each.
- Named blocks (vars), in this order: CARHEADER, RENDITIONS, FACETKEYS, APPEARANCEKEYS,
  KEYFORMAT, EXTENDED_METADATA, BITMAPKEYS. APPEARANCEKEYS is missing when no rendition has an
  appearance (browseros, kdrive, sparkle-test-app); LOCALIZATIONKEYS follows it in localized
  catalogs (sumup); KEYFORMATWORKAROUND follows EXTENDED_METADATA in scummvm (tvOS, CoreUI 802).
  CoreUI 308 and 374 catalogs (pxcore2, acextract) use other sets.
- File order: CARHEADER, RENDITIONS tree and leaf, FACETKEYS, APPEARANCEKEYS, appearance and
  facet key/value pairs, KEYFORMAT, rendition key/value pairs, EXTENDED_METADATA, BITMAPKEYS tree,
  leaf and values, vars, index. In current catalogs the file ends with the index.
- The index has at least 256 slots and more when the file has more blocks (roothide 536, sumup
  432), no free list, and in current catalogs 20 zero bytes after the last slot.

### Trees

Tree header (29 bytes): `tree`, version 1, child block, page size, path count, u8 inline-keys
flag, i32 key size, u32 0. Every tree in the corpus is a single leaf: u16 1 (leaf), u16 entry
count, u32 forward and backward links, then one (u32 value block, u32 key block) pair per entry.

- RENDITIONS: page 4096, key size = the rendition key's length. The keys repeat after the entry
  table, after a 4-byte zero gap, and the leaf is `4096 + n × key size` bytes. Entries are sorted
  by memcmp of the key. Without that inline-key region the catalog loads, but lookups by name
  return nothing (observed).
- FACETKEYS, APPEARANCEKEYS: key size -1 (keys of any length), sorted by strcmp. A tree with one
  entry is written fixed-key instead: key size = that key's length, the key inlined after the gap,
  leaf `4096 + key length` (APPEARANCEKEYS holding only `UIAppearanceAny` in roothide, sumup,
  swiftyvk and scummvm: 15 and 4111; kdrive's single facet: 11 and 4107).
- BITMAPKEYS: page 1024, inline-keys flag 1, key size 0; the key is the u32 itself.

## CoreUI blocks

Little-endian.

- **CARHEADER** (436 bytes): `RATC`, CoreUI version, storage version, timestamp 0, rendition
  count, main version string[128], version string[256], UUID (zero), checksum 0, schema version,
  colorspace 1, key semantics.
  - The CoreUI version is that of the actool build: 308 to 1010 across the corpus, 969 and up in
    catalogs with an Icon Composer icon. The main version string repeats it (`@(#)PROGRAM:CoreUI
    PROJECT:CoreUI-975 [LAR]`); the version string names the Xcode build (`Xcode 26.6 (17F113)
    via AssetCatalogAgent-AssetRuntime`).
  - Storage 17 and schema 2 in current catalogs; schema 5 in browseros, kdrive and
    sparkle-test-app; storage 8 and schema 4 in pxcore2; storage 11 in acextract.
  - Key semantics 1 on macOS; 2 on iOS, tvOS (scummvm) and Mac Catalyst (swiftyvk).
- **EXTENDED_METADATA** (1028 bytes): `META`, then thinning arguments, deployment platform
  version, platform, authoring tool, 256 bytes each. Platforms seen: `macosx`, `ios`, `atv`,
  `macosx-ios` (Catalyst). On macOS, every deployment target from 11.0 to 26.0 gives the same
  catalog apart from this version field (observed). CoreUI 308 and 374 catalogs have no metadata
  block.
- **KEYFORMAT**: `tmfk`, 0, count, u32 attribute ids. A rendition key is the u16 values in that
  order. The order is a fixed per-platform list:
  - macOS `[7,13,1,2,3,4,6,17,8,9,10,14,11,12,24,19,18,25,26,27]`;
  - iOS and Catalyst `[7,13,12,15,16,26,27,9,8,25,24,4,20,21,23,22,17,1,2,10,6]`.

  Xcode 12 keeps the whole iOS list (swiftgen). Current Xcode drops the attributes no rendition
  uses, except that macOS keeps Size (3) and Layer (11): `[7,13,1,2,3,17,9,11,12]` (aven, vlc),
  `[7,13,1,2,3,17,11,12]` (emacs-liquid-glass), `[7,13,1,2,3,17,9,11,12,24]` (mediainfo),
  `[7,13,12,15,16,9,8,17,1,2]` (roothide), `[7,13,12,15,16,9,24,17,1,2]` (openutau),
  `[7,13,12,15,16,17,1,2]` (swiftyvk). Other layouts seen: `[7,1,2,17,9,10,14,12,19,18]` (Xcode
  26.3, brave-origin), kdrive's macOS list plus 25, 26, 27, and older orders in sparkle-test-app,
  scummvm, acextract and pxcore2.
- **APPEARANCEKEYS**: name to u16 id.
  - macOS: `NSAppearanceNameSystem` 0, `NSAppearanceNameDarkAqua` 1, `NSAppearanceNameAqua` 8,
    `ISAppearanceTintable` 10. A catalog with an Icon Composer icon lists all four.
  - iOS: `UIAppearanceAny` 0 always; `UIAppearanceDark` 1, `UIAppearanceLight` 4 and
    `ISAppearanceTintable` 10 when used (all three with an Icon Composer icon: openutau,
    utm-remote).
- **FACETKEYS** value: i16 hotspot x, y, u16 count, then (u16 attribute, u16 value) pairs:
  Element 85, Part, Identifier.
- **BITMAPKEYS**: one entry per facet identifier, and every facet must have one; with the tree
  empty no facet resolves (observed). The value is `1, 0, 4(n+1), n`, then one u32 mask per
  KEYFORMAT attribute (`16 + 4n` bytes for n attributes): the bitset of the values that facet's
  renditions use, or `0xFFFFFFFF` when unconstrained. See [Key attributes](#key-attributes).

## Renditions

CSI header (184 bytes): `ISTC`, version 1, flags, width, height, scale × 100, pixel format,
colorspace, modtime 0, u16 layout, u16 0, name[128], TLV length, 1, 0, payload length.

Flags: the image's template mode times 4 (2 template gives 8, 4 automatic gives 16), plus 4 when
the rendition comes from a vector source (SVG and PDF renditions carry 4, images rasterised
from them 20: sumup's `.svg` and `.pdf` images). App-icon renditions, atlases, colours,
gradients and Icon Composer stacks carry 0. An app-icon rendition with flags 16 is not found by
name (observed). swiftyvk also sets 0x100, meaning unknown.

Layouts: 9 vector, 12 image, 1000 data, 1003 internal reference (a packed image), 1004 packed
atlas, 1009 colour, 1010 multisize image set, 1019 icon stack, 1020 icon group, 1021 gradient.

Parts: 181 image or SVG, 217 colour, 218 multisize set, 220 icon image, 245 icon stack, 246 icon
group, 247 gradient. The packed atlas uses Element 9.

TLVs:

| Tag | Content |
|---|---|
| 1001 | slices |
| 1003 | metrics |
| 1004 | blend mode and opacity |
| 1005 | UTI: u32 length including NUL, u32 0, string. A data set's `universal-type-identifier`, read by `NSDataAsset.typeIdentifier`; Icon Composer stacks carry `public.layeredimage` |
| 1006 | EXIF orientation |
| 1007 | bytes per row, equal to the payload's row stride ([codecs.md](codecs.md#row-stride)) |
| 1010 | `INLK` atlas link (see [Atlases](#atlases)) |
| 1012 | layer list: count, then 48-byte entries (u32 0, u32 0, i32 x, y, w, h in a 1024 canvas, u32 0, f32 opacity, u32 16, then Element, Part and Identifier as (u16, u16) pairs), then 4 zero bytes |
| 1013 | a list of names, on swiftgen's layout 1005 renditions |
| 1020 | group materials: count, 0, then per entry u32 kind, f32 value, u32 name length including NUL, name |
| 1021 | group effects: count, then 20-byte entries (u32 0, u32 translucency enabled, f32 translucency, u32 shadow kind, f32 shadow opacity), then 4 zero bytes |
| 1022 | Xcode 27 only, meaning unknown |

What TLVs 1012, 1020 and 1021 hold in an Icon Composer catalog is in
[icon-bundle.md](icon-bundle.md#stacks-and-groups).

Payloads other than images:
- `RLOC` colour: 1, colorspace (1 sRGB, 2 gray gamma 2.2, 3 display-p3, 4 extended sRGB, 6
  extended gray), component count (2 for the gray spaces, 4 for the RGB ones), f64 components.
- `ARGG` gradient: stop count, kind (0 automatic, 1 linear), 0, f32 start x, y, stop x, y, then
  per stop f32 location, u32 name length, colour facet name.
- `SISM` multisize set: 1, count, then (width, height, Dimension2 index) per size.
- `DWAR` raw data: version (1 = LZFSE-compressed SVG), length, data. Icon Composer stacks and
  groups carry an empty `DWAR` (version 0, length 0).

Names of renditions that actool renders itself embed `<UUID>-<pid>-<timestamp>`, so two builds of
the same input differ in those bytes.

## Key attributes

| Id | Attribute | Id | Attribute |
|---|---|---|---|
| 1 | Element | 12 | Scale |
| 2 | Part | 13 | Localization |
| 3 | Size | 15 | Idiom |
| 4 | Direction | 16 | Subtype |
| 7 | Appearance | 17 | Identifier |
| 8 | Dimension1 | 24 | DisplayGamut |
| 9 | Dimension2 | 25 | DeploymentTarget |
| 11 | Layer | | |

- Idiom: universal 0, phone 1, pad 2, tv 3, car 4, watch 5, marketing 6 (scummvm has tv and
  marketing; the iOS samples 0 to 2).
  Dimension2 of app icons is in [app-icons.md](app-icons.md#renditions).
- Colour and gradient renditions have Scale 1 in their key and scale 0 in the CSI header (every
  corpus colour). Colour lookups match on the key's scale.
- DisplayGamut (24) is not an appearance axis. It sits after Scale on macOS and after
  DeploymentTarget on iOS.
  - A PNG gets a second rendition at gamut 1 when its ICC profile is wide (the xy triangle of its
    colorants is larger than 0.13447: Display P3 and Adobe RGB are, sRGB and Cinema Display
    Calibrated are not) and some pixel, premultiplied in extended sRGB, falls outside
    -0.025..1.003. A wide profile with in-gamut pixels gets none (a 16-px Adobe RGB icon in
    openzfs).
  - The copy is `RGBW` in colorspace 4 with the same flags; the gamut-0 rendition holds the same
    colours clipped to sRGB (openrct2's `Slide.png`, wireshark's `Fin.png`, openutau's 16-bit P3
    app icon).
  - An image with more than 8 bits a channel whose pixels are gray in 8-bit sRGB gets a `GA16`
    copy instead: premultiplied f16 gray and alpha, colorspace 6 (readinglist's 16-bit RGBA
    `eyeglasses.png`). JPEGs get none.
  - The facet holding a copy has BITMAPKEYS mask 3 for attribute 24, every other facet 1. Copies
    of small images pack into their own `-gamut1` atlases (see [Atlases](#atlases)).
- BITMAPKEYS masks are -1 for Element, Part, Identifier and, wherever the key format has it,
  Appearance (every corpus catalog). Other attributes set bit `value` (below 32),
  Localization bit 1 for any locale, and Subtype a bit per device subtype (iOS: 0→0, 568→1,
  569→2, 570→3, 2732→4, 163→5, 2420→7, 2752→8, 2034→9, 2224→10, 2436→11, 2688→12, 2622→13,
  2868→14, 2388→15, 1792→16, 32401→17, 2340→18, 2778→19, 2360→20, 2160→21, 2532→22, 2266→25,
  2556→29, 2796→30, 2736→31).

## Atlases

Small images are packed into atlases; which ones, how they are grouped, split and laid out, and how
Dimension1 is numbered is in [atlas-packing.md](atlas-packing.md).

- An atlas is a layout 1004 rendition with Element 9, named
  `ZZZZPackedAsset-<scale>.<opaque>.<monochrome>-gamut<DisplayGamut>`.
- Each packed image is a layout 1003 rendition whose `INLK` TLV gives its rect in the atlas,
  bottom-up, with a 2 px gutter around it, and the atlas key: Element, Part, Dimension1,
  Identifier, Scale, Idiom, Subtype, Appearance, DisplayGamut, each only when non-zero (roothide's
  `Oval.png`: rect (2, 44, 66, 64), key Element 9, Part 181, Scale 1). A gamut-1 copy's key ends
  with DisplayGamut 1 after Scale.
