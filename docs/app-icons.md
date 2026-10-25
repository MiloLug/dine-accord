# App icons

How actool compiles an app icon, from an `.appiconset` or an Icon Composer `.icon`: the icon
renditions in the catalog, the pre-rendered images it makes from an `.icon`, the files it writes
next to the catalog, and the partial Info.plist keys. Rendition keys are described in
[format.md](format.md), the `.icon` structure in [icon-bundle.md](icon-bundle.md), and the look of
the pre-rendered images in [icon-rendering.md](icon-rendering.md).

Facts about renditions cite the corpus. Facts about loose files and the partial plist come from
actool runs; the corpus holds only catalogs.

## Choosing the app icon

- `--app-icon <name>` names the primary icon set or `.icon` bundle. An `.icon` bundle gets the
  `.icns` and the plist keys only when its file stem equals `<name>` (case-sensitive); otherwise
  the output is `Assets.car` and an empty `<dict/>` plist.
- Since Xcode 26.1, an `.appiconset` named like an `.icon` document of the same invocation is
  ignored (observed).
- `--include-all-app-icons` also compiles the other icon sets, as alternate icons.

## Renditions

- App-icon renditions are Part 220 with flags 0 ([format.md](format.md#renditions)). Small ones
  are packed into atlases (layout 1003 in utm-remote).
- Dimension2 is the rank of the icon's point size among all sizes of the set, counting only the
  target devices, universal and marketing: 20, 29, 38, 40, 60, 76, 83.5, 90, 1024 give 1 to 9 in
  a full iOS set (utm-remote); a single 1024 gives 1. A multisize rendition (`SISM`) lists each
  size with its Dimension2 (83.5 as 83). A macOS `.iconset` folder ranks its files' sizes the
  same way (browseros: a lone 256 pt icon is 1).
- For iPhone Plus models actool adds a 90 pt @2x icon (Idiom phone, Subtype 1792) made from the
  60 pt @3x one (both 180 px), one per appearance, with a multisize set of its own (roothide,
  utm-remote).
- A universal icon is stored once per `--target-device` idiom (phone 1 and pad 2 in roothide and
  utm-remote).

## Pre-rendered `.icon` icons

macOS:
- Sizes 16, 32, 64, 128, 256 and 512 pt at @2x and 1024 pt at @1x, Dimension2 1 to 7 in that
  order, with one multisize set listing them. Xcode 26.0 writes the 1024 pt one at @2x (zettlr).
- 16, 32 and 64 pt are packed into an atlas; the others stay inline (aven).
- Appearance 0 only, named
  `<stem><pt>x<pt>_NSAppearanceNameSystem_<UUID>-<pid>-<timestamp>.png`.
- `ARGB` for readinglist's near-gray icon, whose channels differ by up to 16.
- An icon with wide-gamut colours gets an `RGBW` DisplayGamut 1 copy of every size (zettlr).
- Xcode 26.0 leaves them out with `--enable-icon-stack-fallback-generation NO`
  (emacs-liquid-glass), and the main facet then has Part 245. moddable,
  compiled for macOS 10.14 with `--app-icon main` and `main.icon`, has none either, for a reason
  not known.

iOS:
- One 1024 × 1024 @1x per target-device idiom for each of Any (0), Dark (1) and Tintable (10),
  Dimension2 1, named `<stem>1024x1024_UIAppearanceAny_…` (openutau, utm-remote).
- The Tintable image is `GA8` in openutau, whose pixels are exactly gray, and `ARGB` in
  utm-remote, whose channels differ by up to 2.
- They show on systems before iOS 27 and in the loose PNG; iOS 27 draws the icon from the stack
  ([icon-bundle.md](icon-bundle.md#stacks-and-groups)).

## Loose files

- iOS: `<name>60x60@2x.png` for iPhone and `<name>76x76@2x~ipad.png` for iPad. Each is the
  set's @2x home-screen icon for that idiom, or its 1024 scaled down when the set has a single
  size; for an `.icon`, the Any pre-render scaled down. The PNG is 8-bit RGBA with an `sRGB`
  chunk and an `eXIf` chunk holding the colour space (sRGB) and the pixel size.
- macOS: `<name>.icns` (for an `.icon`, only under the stem rule above). iOS gets no `.icns`.

## Partial Info.plist

- iOS: `CFBundleIcons` (iPhone) and `CFBundleIcons~ipad`, each a dictionary with:
  - `CFBundlePrimaryIcon`: `CFBundleIconName` and, when the idiom has a loose icon,
    `CFBundleIconFiles` (`<name>60x60`; the iPad list starts with the iPhone entry, then
    `<name>76x76`);
  - `CFBundleAlternateIcons` under `--include-all-app-icons`: each other set's name mapped to a
    dictionary with its `CFBundleIconName`.
- macOS: `CFBundleIconFile` and `CFBundleIconName`, both the app icon name.
- `--accent-color` adds `NSAccentColorName`, `--widget-background-color` adds
  `NSWidgetBackgroundColorName`.
