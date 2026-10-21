# Icon Composer catalogs

What actool writes into `Assets.car` for an Icon Composer `.icon` bundle: the facets, the colour
and gradient palette, the layer images, and the icon stack and group renditions that iOS 27 and
macOS 26 draw the icon from. Which pre-rendered icons and side files come with it is in
[app-icons.md](app-icons.md); what the pre-renders look like is in
[icon-rendering.md](icon-rendering.md); the byte layout of every block and TLV named here is in
[format.md](format.md).

Corpus samples with an `.icon` source: aven, brew-browser, daintree, emacs-liquid-glass, mediainfo,
moddable, readinglist, sentinel, tusk, vlc, zettlr (macOS), openutau and utm-remote (iOS).

## Reading icon.json

`icon.json` lists groups front to back, and each group's layers front to back. A group or layer
property is either a plain field or a `*-specializations` list of `{appearance, value}` entries;
the list replaces the plain field, its entry without an appearance is the light value, and an
appearance without an entry of its own takes that light value. Appearances are light, `dark` and
`tinted`. A layer with `hidden: true` is left out entirely: no IconGroup entry and no palette
entries for its fills (vlc).

## Facets

For a bundle `<stem>.icon`:

| Facet | Part | Holds |
|---|---|---|
| `<stem>` | 220 or 245 | pre-rendered icons (Part 220), the multisize set, the icon stacks (Part 245) |
| `<stem>/<group name>` | 246 | that group's IconGroup renditions |
| `<stem>_Assets/<image stem>` | 181 | one layer image |
| `<stem>_Assets/Color-<n>`, `Gradient-<n>` | 217, 247 | the palette |

- The main facet's part is 220 in aven, daintree, mediainfo, readinglist, sentinel and tusk, and
  245 in brew-browser, emacs-liquid-glass, moddable, openutau, utm-remote, vlc and zettlr. No
  version or input explains the split; two builds of one commit (brave beta and nightly) got one
  each. The renditions under it keep their own parts either way (vlc: a 245 facet with Part 220
  pre-renders; daintree: a 220 facet with Part 245 stacks).
- A group without a name is `Group`; repeated names get ` 2`, ` 3` (daintree: `Group`,
  `Group 2`).
- In group and image facet names, anything other than ASCII letters, digits, space, `.`, `-` and
  `_` becomes `_` (brew-browser's `03 · Barley emblem` is `03 _ Barley emblem`; mediainfo's
  `AppIcon-Cocoa-Default-1024x1024@1x.png` is `AppIcon-Cocoa-Default-1024x1024_1x`).
- Group facets and SVG layer facets hold no bitmap, so an image lookup by those names finds
  nothing, in actool's output as in any other.

## Palette

Colours and gradients get numbered facets in the order they are folded in:

1. `Color-1`, extended gray (1, 1), in every catalog.
2. The background. A top-level `fill` gives its light form, then the dark pair, gray (0.192,
   0.078) as two colours and a linear gradient top to bottom:
   - `automatic` or no fill: the light pair, gray (1.0, 0.925), the same way (daintree);
   - `automatic-gradient`: a one-stop gradient of kind 0 on the given colour (mediainfo, zettlr);
   - `linear-gradient`: a gradient of the two stops along `orientation` (tusk);
   - `solid`: one colour.

   A top-level `fill-specializations` gives each entry in order instead (brew-browser: a light and
   a dark linear gradient). There the keywords `system-light` and `system-dark`, and `automatic`
   by the entry's appearance, resolve to the light and dark pairs (aven).
3. Each visible layer's fill and `fill-specializations`, in document order (zettlr's `Color-5` and
   `Color-6`; moddable's white layer fill is `Color-1`).

Encoding:
- Colour specs `<space>:<components>` map to colorspace srgb 1, gray 2, display-p3 3,
  extended-srgb 4, extended-gray 6. Each component is rounded to 3 decimals and stored as
  `(double)(float)` (zettlr's 0.10980 is stored as 0.10999999940395355).
- A gradient's geometry is `orientation` as `[start.x, start.y, stop.x, stop.y]`, default
  `[0.5, 0, 0.5, 1]`.
- Colours deduplicate on colorspace and components, gradients on geometry and stops. A layer's
  solid fill deduplicates against the whole palette. A layer's gradient stop or keyword colour
  deduplicates against every colour except the keyword gray pairs (1.0, 0.925) and (0.192,
  0.078): vlc's white stop reuses the background's `Color-2`, while the public transmission
  bundle keeps a second 0.078 gray.
- Since Xcode 27 (CoreUI 1006 and 1010) the gradient of the dark gray pair, whether it comes from
  the `system-dark` keyword, a dark `automatic` or a top-level `fill`, is named `system-dark` and
  still uses up its number (vlc: `Gradient-1`, `system-dark`, `Gradient-3`; tusk, utm-remote).
  Earlier builds number it like any other (aven, daintree: `Gradient-2`).

## Layer images

- One facet per image file, named after its stem. An SVG is stored as the SVG itself (layout 9,
  `DWAR` version 1, rendition name `image.svg`); a PNG as an image (`image.png`).
- SVG renditions carry colorspace 0, or 3 when `color-space-for-untagged-svg-colors` is
  `display-p3` (vlc).
- A PNG is marked opaque only when it has no alpha channel (daintree's `background.png`); an RGBA
  PNG is not, whatever its pixels.
- An image larger than the 1024-px canvas is stored at 1024; its slices TLV keeps the source size.
- A wide-gamut PNG gets a DisplayGamut 1 copy as any image does (mediainfo).

## Stacks and groups

Each appearance gets one icon stack (layout 1019, Part 245, under `<stem>`, 1024 × 1024) and one
IconGroup rendition (layout 1020) per group: on macOS DarkAqua 1, Aqua 8 and Tintable 10; on iOS
Dark 1, Light 4 and Tintable 10. Both carry an empty `DWAR`; the stack also carries the UTI
`public.layeredimage`. iOS 27 draws the home-screen icon from these, not from the pre-rendered
images (observed on device).

The stack's entries, in TLVs 1012, 1020 and 1021 alike, are the fill, then the groups back to
front:

| | Fill entry | Group entry |
|---|---|---|
| 1012 key | the light gradient (Aqua, Light, Tintable) or the dark one (Dark); a solid fill's colour facet | the group's facet |
| 1012 frame, opacity | zero frame, 1 | zero frame, the group's opacity for that appearance (aven: 0 where `opacity-specializations` hides the group) |
| 1020 | kind 0, value 0, no name | kind 2, but 0 in zettlr, the only sample with `lighting: combined`; value = `blur-material` since Xcode 27 (vlc 0.5), 0 before (daintree's 0.5) |
| 1021 | all zero | specular flag, translucency, shadow kind, shadow opacity (below) |

1021 group entry:
- The flag is the group's `specular`, absent counting as on (aven and brew-browser say `false`
  and store 0; emacs-liquid-glass, tusk and the rest store 1).
- The translucency is the group's value for that appearance when enabled, 0 when disabled
  (emacs-liquid-glass's `EmacsLG3`, translucent in light only: 0.5 in Aqua, 0 in Dark and
  Tintable; tusk).
- Shadow kind: `none` 0, `layer-color` 2, `neutral` 3. daintree's `layer-color` group, whose
  layer is not glass, is stored as 3. Shadow opacity is copied.

An IconGroup rendition lists the group's visible layers back to front:
- 1012 frame: `w, h = native size × layer scale × group scale`, rounded to the nearest integer,
  and `x, y = (1024 − w, 1024 − h) / 2 + layer translation × group scale + group translation`
  (with the unrounded size), rounded down. sentinel: an 851-px layer at scale 1.33 is 1132 wide;
  vlc: x = 83.98 is stored as 83. y grows downward.
- The native size is the PNG's pixel size, or the SVG's `width` and `height`, or its viewBox when
  those are percentages (vlc: 395 × 437). zettlr's 512-pt SVG at scale 2.76 and translation
  (0, −43) gives (−195, −238, 1413, 1413).
- 1012 key: the layer image's facet; opacity is the layer's for that appearance.
- 1020: kind 1 and the name of the layer's fill facet for that appearance (zettlr:
  `Zettlr_Assets/Color-1`), with no name when the layer has no fill there (readinglist,
  sentinel); kind 0 with no name when the layer says `glass: false` (aven, daintree, mediainfo,
  tusk).
- 1021: one all-zero entry per layer.
