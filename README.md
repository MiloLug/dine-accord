# accord

accord compiles Xcode asset catalogs (`.xcassets`) and Icon Composer icons (`.icon`) into
`Assets.car` on Linux. It takes the same arguments as Apple's macOS-only `actool`, so a build can
call `accord` in its place.

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

`Assets.car` goes into the app bundle with the loose home-screen icons; `partial.plist` holds the
`CFBundleIcons` keys to merge into the app's `Info.plist`.

## Install

Needs Linux and Rust 1.89 or later. There are no prebuilt binaries.

```sh
cargo install --git https://github.com/MiloLug/dine-accord
```

`accord --version` prints the version plist of the `actool` it mirrors (26.3, build 24506).

## Usage

macOS app icon from an Icon Composer bundle (writes `Assets.car`, `AppIcon.icns`, `partial.plist`):

```sh
accord --compile out --platform macosx --minimum-deployment-target 11.0 \
    --app-icon AppIcon --output-partial-info-plist out/partial.plist AppIcon.icon
```

As with `actool`, the `.icns` and plist keys appear only when the bundle name equals `--app-icon`.

Several documents in one catalog:

```sh
accord --compile out --platform iphoneos --minimum-deployment-target 17.0 \
    --app-icon AppIcon --target-device iphone --output-partial-info-plist out/partial.plist \
    AppIcon.icon Assets.xcassets
```

`--accent-color AccentColor` adds `NSAccentColorName`; `--include-all-app-icons` also compiles the
other app icon sets as `CFBundleAlternateIcons`.

### Reading a compiled catalog

These work on any `.car`, including Apple's:

| Command | What it does |
|---|---|
| `accord inspect [--json] Assets.car` | prints the header, key format, facets and every rendition |
| `accord diff apple.car ours.car` | one line per structural difference; exits 1 if any |
| `accord extract Assets.car png/` | writes the 8-bit image renditions as PNG |

### Output and options

accord prints the `actool` compilation-results plist and exits 0; errors exit 1, or 2 when
`--compile` or the document is missing. `accord --help` lists every option. Differences from
`actool`:

- `--warnings`, `--errors`, `--notices`, `--compress-pngs`, `--skip-app-store-deployment`,
  `--enable-on-demand-resources`, `--alternate-app-icon`, `--force-png` and
  `--enable-icon-stack-fallback-generation` are ignored; pre-rendered icons are always generated.
- `--print-contents` prints nothing.
- `--generate-objc-asset-symbols` with `--bundle-identifier` writes only the symbols header.
- Options `--help` doesn't list, such as `--launch-image`, exit 2.

## What compiles

- Image sets (PNG, JPEG, SVG, PDF), colour sets, data sets.
- App icons, multi-size or single 1024, with dark and tinted variants.
- Icon Composer `.icon` bundles for macOS and iOS.
- Atlases packed the way `actool` packs them.
- ICC-tagged PNGs, converted to sRGB, plus the 16-bit copy P3 screens load when a wide-gamut image
  has colours sRGB can't show.

Known differences from `actool`:

- Below macOS 11 and iOS 14, LZFSE where `actool` uses palette-img or deepmap compression;
  CoreUI reads both.
- No wide-gamut copies for 16-bit gray images or for icons rendered from an `.icon`.
- Some `.icon` palette entries are named or merged differently.
- Alternate iOS app icon sets don't match `actool`'s rendition set yet.
- `.icon` pre-renders approximate Liquid Glass; iOS 27 draws the icon from the layer stack anyway.

## Testing against Apple's output

`tests/corpus/` holds catalogs compiled by Apple's `actool` from open-source projects, picked so
each covers something the others don't, many with their inputs. `tests/corpus_parity.rs` compiles
those inputs and diffs the result with Apple's; known differences live in
`tests/corpus/expectations/`, and a new one fails the test.

```sh
git lfs pull
cargo test --release
```

## More

- [`docs/format.md`](docs/format.md): `.car` layout, payloads, key attributes.
- [`docs/atlas-packing.md`](docs/atlas-packing.md): how `actool` packs atlases.
- [`docs/icon-bundle-parity.md`](docs/icon-bundle-parity.md): `.icon` output compared with Apple's.
- [`docs/icon-shading.md`](docs/icon-shading.md): `.icon` shading effects.

accord is a fork of [viraptor/actool](https://github.com/viraptor/actool); its changes are not
offered upstream.

## License

MIT, see [`LICENSE`](LICENSE). Corpus files keep their projects' licences, listed in
[`tests/corpus/README.md`](tests/corpus/README.md).
