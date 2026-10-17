# accord

A Linux replacement for Apple's `actool`: compiles `.xcassets` and Icon Composer `.icon` bundles
into `Assets.car` and the partial Info.plist, with `actool`-compatible arguments. Built for
[Dine](https://github.com/MiloLug/dine), which builds iOS apps from Linux.

accord is a fork of [viraptor/actool](https://github.com/viraptor/actool). Its changes are not
offered upstream.

```sh
cargo build --release
target/release/accord --compile out --platform iphoneos --minimum-deployment-target 17.0 \
    --app-icon AppIcon --output-partial-info-plist out/partial.plist Assets.xcassets
```

Supported on Linux: images (PNG, JPEG, SVG via resvg, PDF via hayro), colors, data, app icons
(multi-size and single-size, with dark and tinted variants), Icon Composer `.icon` bundles for
macOS and iOS, and any mix of `.xcassets` and `.icon` documents in one catalog, with Apple's atlas
packing. Not yet: the palette-img and deepmap2 encoders (LZFSE is written instead, which CoreUI
reads), wide-gamut (`DisplayGamut` 1) variants, Xcode 27's TLV 1022, and pixel-exact Liquid
Glass pre-renders.

`tests/corpus/` (Git LFS: `git lfs pull` after cloning) checks accord's output against catalogs
compiled by Apple's actool (macOS and iOS, Xcode 26.2 – 27.1); `docs/format.md` collects the
format facts.
