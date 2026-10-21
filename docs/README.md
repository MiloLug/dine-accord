# accord internals

How accord is put together: the modules that turn `.xcassets` and `.icon` inputs into
`Assets.car`, which spec each part follows, its tests, and where its output still differs from
actool's. The specs describe actool's output and nothing of accord; the README at the repo root
covers installing and running it.

| Spec | Covers |
|---|---|
| [format.md](format.md) | BOM container, CoreUI blocks, renditions, TLVs, key attributes |
| [codecs.md](codecs.md) | image payloads: CELM, LZFSE, deepmap2, pixel formats |
| [atlas-packing.md](atlas-packing.md) | which images actool packs and how it lays out atlases |
| [app-icons.md](app-icons.md) | app-icon renditions, pre-rendered icons, loose files, partial plist |
| [icon-bundle.md](icon-bundle.md) | Icon Composer `.icon` bundles: facets, palette, stacks, groups |
| [icon-rendering.md](icon-rendering.md) | what actool's pre-rendered `.icon` images look like |

## Pipeline

One `accord --compile` run:

1. `main.rs` parses the actool-compatible command line, or runs `inspect`, `diff`, `extract` or
   the asset-symbols mode.
2. `compiler.rs` handles the invocation: each `.icon` goes to `icon_bundle.rs`, everything else
   to `catalog.rs`; it also writes the loose iOS icons, the `.icns` and the partial plist.
3. `catalog.rs` parses an `.xcassets` into renditions. `color.rs` converts ICC-tagged images to
   sRGB and makes the DisplayGamut copies; `svg_raster.rs` and `pdf_raster.rs` rasterise
   vectors.
4. `icon_bundle.rs` turns an `.icon` into its palette, stacks, groups, layer images and
   pre-rendered icons, with `icon_json.rs` (the `icon.json` model), `icon_effects.rs`
   (properties resolved per appearance) and `icon_render.rs` (squircle, shadow, gradient,
   lighting).
5. `assemble.rs` builds one catalog from every document of the run: atlases through
   `packer.rs`, the key format, the CSIs, BITMAPKEYS and the trees.
6. `car.rs` writes the CoreUI blocks, CSI headers and TLVs and picks each payload's compression;
   `deepmap2.rs` is the deepmap2 codec; `bom.rs` writes the BOMStore file.

`icns.rs` writes `.icns` files, `symbols.rs` the Objective-C and Swift asset symbols, and
`name_hash.rs` the identifier hash. `car_read.rs` reads any `.car` for `inspect`, `diff` (rendition
names compared without their per-build part) and `extract` (8-bit images only; `RGBW` and `GA16`
are skipped).

## Parts

### Container and blocks

`bom.rs`, `car.rs`, `assemble.rs` · spec [format.md](format.md) · tests `bom_parity.rs`,
`car_parity.rs`, `corpus_inspect.rs`.

Differs from actool:
- CoreUI version 975 for iOS catalogs and catalogs with an `.icon`, 972 otherwise; actool stamps
  its own build. Storage 17 and schema 2 always; version string
  `IBCocoaTouchImageCatalogTool-17.0`, authoring tool `actool`.
- Block 1 starts at 0x20 and a tree entry's key block follows its value block.
- APPEARANCEKEYS is always variable-key, also with a single entry. LOCALIZATIONKEYS is never
  written.
- tvOS, watchOS and Catalyst get key semantics 1 and the macOS key format.
- The iOS key format has no Direction (4), so the left-to-right and right-to-left variants of an
  image share a key.

### Image payloads

`car.rs` (choice), `deepmap2.rs` · spec [codecs.md](codecs.md) · tests `dmp2_regressions.rs`.

accord uses deepmap2 from macOS 11, iOS and tvOS 14 and watchOS 7 for payloads over 256 bytes,
except app icons and atlases made only of `ARGB` icons; otherwise chunked LZFSE from macOS 10.11,
iOS and tvOS 9 and watchOS 2; below that gzip (4 KiB and up), RLE or raw. Simulator and visionOS
targets use the macOS thresholds. Below macOS 11 and iOS 14, accord writes LZFSE where actool may
use deepmap2 or another compression. accord's RLE header starts with the bytes per pixel where
actool's has 3 (swiftgen). `car_read.rs` decodes raw, RLE, gzip, LZFSE and deepmap2.

### Atlases

`packer.rs`, `assemble.rs` · spec [atlas-packing.md](atlas-packing.md) · tests
`packer_parity.rs`, `packing_regressions.rs`.

The sweep configurations and roothide's atlases match actool's; corpus atlases that differ hold
different images (pre-rendered `.icon` icons). accord counts `GA16` images as monochrome, and
writes sprite atlases (`.spriteatlas`, Part 127, TLV 1013 names); no corpus sample confirms
either.

### Asset catalogs

`catalog.rs`, `color.rs`, `svg_raster.rs`, `pdf_raster.rs` · spec [format.md](format.md) ·
tests `e2e_compile.rs`, `display_gamut.rs`, `jpeg_regressions.rs`.

Differs from actool: PDFs are stored as given (actool rewrites them); JPEGs keep their EXIF
`YCbCrPositioning` tag.

### App icons

`compiler.rs` (loose PNGs, plist), `catalog.rs` (renditions), `icns.rs` · spec
[app-icons.md](app-icons.md) · tests `ios_parity.rs`, `e2e_compile.rs`.

Without `--app-icon`, an `.icon` bundle's name is taken as the app icon, so the `.icns` and plist
keys are written; what actool does there is unverified. `.icon` pre-renders are always generated:
`--enable-icon-stack-fallback-generation NO` is ignored.

### Icon Composer bundles

`icon_json.rs`, `icon_effects.rs`, `icon_bundle.rs` · spec [icon-bundle.md](icon-bundle.md) ·
tests `icon_bundle_e2e.rs`, `icon_bundle_robustness.rs`.

Differs from actool:
- The main facet is always Part 220.
- Gradients are numbered, except keyword entries in `fill-specializations`, which are named
  `system-light` and `system-dark` whatever the build. actool numbers them before Xcode 27; from
  Xcode 27 it names the dark gray-pair gradient `system-dark` whatever its source (tusk's comes
  from a top-level `fill`).
- The stack's 1020 group value is the `blur-material`, as from Xcode 27, in a catalog stamped 975
  (daintree's actool writes 0).
- TLV 1022 is not written; actool writes it from CoreUI 1006 on.
- The 1024 pt macOS pre-render is @1x, as from Xcode 26.1.
- Pre-renders that come out exactly gray are `GA8` where actool's pixels are near gray and stay
  `ARGB` (readinglist; utm-remote's Tintable icon). Pre-renders get no `RGBW` copies.
- The `.icns` holds the first layer image, not the rendered icon.
- PDF layers are not read.

### Pre-rendered icon look

`icon_render.rs`, `icon_bundle.rs` (layer stack), `svg_raster.rs` · spec
[icon-rendering.md](icon-rendering.md) · no pixel tests.

Rendering uses resvg and tiny-skia. The glass edge blur is three box passes, so its σ is set
below the measured one.

## Testing against actool

- `tests/corpus/` holds catalogs compiled by actool and, where public, their inputs
  ([tests/corpus/README.md](../tests/corpus/README.md)). `tests/corpus_parity.rs` compiles each
  pair and compares the result with actool's through `car_read::diff`; the differences it may
  show are listed in `tests/corpus/expectations/<sample>.txt`. A new difference fails the test, a
  vanished one prints a hint, and `ACCORD_UPDATE_EXPECTATIONS=1` rewrites the files.
- `tools/compare_car.py A.car B.car --no-pixels` is a structural diff that runs anywhere. Its
  pixel comparison and the `tools/probe_*.py` scripts need macOS, actool and a
  `tools/extract_pixels` helper that is not in the repo. `tools/sweep_atlas_geometry.py` recorded
  `tools/atlas_sweep_dataset.json`, which `packer_parity.rs` reads.
