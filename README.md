# accord

accord compiles Xcode asset catalogs (`.xcassets`) and Icon Composer icons (`.icon`) into
`Assets.car` and a partial Info.plist on Linux. It takes the arguments of Apple's `actool`, which
runs only on macOS, so a build that calls `actool` can call `accord` in its place.

```sh
accord --compile out --platform iphoneos --minimum-deployment-target 17.0 \
    --app-icon AppIcon --target-device iphone --target-device ipad \
    --output-partial-info-plist out/partial.plist Assets.xcassets
```

```console
$ ls -1 out
AppIcon60x60@2x.png
AppIcon76x76@2x~ipad.png
Assets.car
partial.plist
```

`Assets.car` is the compiled catalog CoreUI loads, the PNGs are the loose home-screen icons iOS
expects next to it, and `partial.plist` holds the `CFBundleIcons` keys to merge into the app's
`Info.plist`. Xcode passes the app bundle as `--compile` and does the merge itself; a build
without Xcode does both.

## Install

Requirements: Linux and a Rust toolchain (accord builds with rustc 1.89). There are no prebuilt
binaries.

```sh
cargo install --git https://github.com/MiloLug/dine-accord
```

This puts `accord` in `~/.cargo/bin`. From a checkout, `cargo install --path .` does the same, and
`cargo build --release` builds `target/release/accord` without installing it.

`accord --version` prints the same version plist as the `actool` it mirrors
(`short-bundle-version` 26.3, `bundle-version` 24506) and exits 0.

## Usage

### macOS app icon from an Icon Composer bundle

```sh
accord --compile out --platform macosx --minimum-deployment-target 11.0 \
    --app-icon AppIcon --output-partial-info-plist out/partial.plist AppIcon.icon
```

This writes `Assets.car`, `AppIcon.icns` and a `partial.plist` with `CFBundleIconFile` and
`CFBundleIconName`. As with `actool`, the `.icns` and those keys appear only when the bundle's file
stem equals `--app-icon`, case-sensitive; otherwise the output is `Assets.car` and an empty
`<dict/>` plist.

### Several documents in one catalog

Pass any number of `.icon` and `.xcassets` paths; they compile into one `Assets.car`:

```sh
accord --compile out --platform iphoneos --minimum-deployment-target 17.0 \
    --app-icon AppIcon --target-device iphone --output-partial-info-plist out/partial.plist \
    AppIcon.icon Assets.xcassets
```

### Accent colour and alternate icons

`--accent-color AccentColor` adds `NSAccentColorName` to the partial plist.
`--include-all-app-icons` also compiles the catalog's other app icon sets and lists them under
`CFBundleAlternateIcons`.

### Reading a compiled catalog

These commands read any `.car`, including ones compiled by Apple's `actool`:

| Command | What it does |
|---|---|
| `accord inspect Assets.car` | prints the header, key format, appearances, facets and every rendition with its key |
| `accord inspect --json Assets.car` | the same as JSON |
| `accord diff apple.car ours.car` | prints one line per structural difference, the first file being the reference; exits 1 when there are any, 0 when there are none |
| `accord extract Assets.car png/` | writes each image rendition as a PNG named after the rendition and its key (`AppIcon1024.png.Scale1.Idiom1.Dimension21.png`) and prints each path |

`extract` decodes raw, gzip and LZFSE images. Renditions Apple compressed with deepmap2 or
palette-img are skipped.

### Output and options

On success accord prints the `com.apple.actool.compilation-results` plist listing the files it
wrote (`--output-format human-readable-text` gives a plain list) and exits 0. Errors go to stderr
with exit code 1, or 2 when `--compile` or the document path is missing.

`accord --help` lists every option; names and defaults follow `actool`. Where accord differs:

- `--warnings`, `--errors`, `--notices`, `--compress-pngs`, `--skip-app-store-deployment`,
  `--enable-on-demand-resources` and `--alternate-app-icon` are accepted and ignored.
- `--print-contents` prints nothing.
- `--generate-objc-asset-symbols FILE` with `--bundle-identifier ID` writes only the Objective-C
  symbols header (plus the index from `--generate-asset-symbol-index FILE`), no catalog. It still
  requires `--compile`.
- `--platform` is tested with `macosx` (the default) and `iphoneos`.

## What compiles

- Image sets: PNG, JPEG, SVG (rasterized with resvg) and PDF (rasterized with hayro).
- Colour sets and data sets.
- App icons, multi-size and single-size, with dark and tinted variants. A single 1024 icon is
  scaled down for the loose PNGs.
- Icon Composer `.icon` bundles for macOS and iOS.
- Small images packed into atlases the way `actool` packs them.

Where the output differs from `actool`:

- Where Apple's `actool` compresses an image with deepmap2 or palette-img, accord on Linux writes
  LZFSE, which CoreUI reads.
- No wide-gamut (`DisplayGamut` 1) variants.
- No Xcode 27 TLV 1022 (its meaning is unknown).
- The pre-rendered images of an `.icon` (the 1024 rendition and the loose PNGs) approximate Liquid
  Glass and are not pixel-identical to Apple's. iOS 27 renders the home-screen icon from the layer
  stack in the catalog, so the approximation shows on older systems and in the loose PNGs only.

## Testing against Apple's output

`tests/corpus/` holds `Assets.car` files compiled by Apple's `actool` (Xcode 26.2 to 27.1, macOS
and iOS) from open-source projects, and for seven of them the inputs. `tests/corpus_parity.rs`
compiles each of those with the recorded arguments and diffs the result against Apple's; the
remaining differences are listed in `tests/corpus/expectations/`, and a new difference fails the
test. The corpus binaries are in Git LFS:

```sh
git lfs pull
cargo test --release
```

Byte-identical output is not reachable for `.icon`: Apple's `actool` embeds a random UUID in the
rendition names, so two of its own runs on one bundle differ.

## Format notes

- [`docs/format.md`](docs/format.md): the `.car` layout, rendition payloads and key attributes.
- [`docs/atlas-packing.md`](docs/atlas-packing.md): how `actool` groups and packs atlases.
- [`docs/icon-bundle-parity.md`](docs/icon-bundle-parity.md): `.icon` output compared with Apple's.
- [`docs/icon-shading.md`](docs/icon-shading.md): the `.icon` shading effects and their parameters.

## Provenance

accord is a fork of [viraptor/actool](https://github.com/viraptor/actool). Its changes are not
offered upstream.

## License

MIT, see [`LICENSE`](LICENSE). The corpus files stay under their projects' licences, listed in
[`tests/corpus/README.md`](tests/corpus/README.md).
