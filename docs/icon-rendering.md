# Pre-rendered icon look

actool flattens an Icon Composer icon into plain images: the pre-rendered app icons listed in
[app-icons.md](app-icons.md). This page describes what those images look like, so that a renderer
can reproduce them. The bundle structure and `icon.json` semantics are in
[icon-bundle.md](icon-bundle.md).

The facts come from measuring actool output: public bundles (feishin, scrumdinger,
KeepingYouAwake, tagspaces, Rectangle, transmission) and test bundles that vary one property at a
time, compiled with actool on macOS and compared pixel by pixel (`tools/probe_*.py`, which need
macOS and actool). Numbers marked as a model are a fit to those measurements, not values read from
actool.

## Canvas

- **macOS**: the icon is a squircle on a transparent square, inset 100/1024 of the edge on every
  side, with a drop shadow around it and light along its inner edge. Model: the superellipse
  `|x/a|⁵ + |y/a|⁵ = 1`, which fits actool's alpha mask to about 2 px (the best circular-arc
  rounded rectangle fits to about 7 px).
- **iOS**: a full-bleed square with no clip, no shadow and no edge light.
- Layers and the background gradient span a content box of 824/1024 of the canvas, centred on
  macOS (measured with marker squares, and on tagspaces' positioned layer); on iOS they span the
  whole canvas.

## Background

- A `solid` fill paints its flat colour.
- A gradient runs from its first stop at the top of the content box to its last stop at the
  bottom. `orientation` gives the direction only: feishin's 1.0 to 0.3 covers the whole box, as a
  0 to 1 gradient does.
- Colours interpolate linearly in their components (a black to white probe has its midpoint at
  127).

## Layers

- Layers are drawn back to front, each at its own aspect ratio.
- A layer is drawn at 824/1024 of its own size in points (a 512-pt viewBox gives 413 px at 1024,
  a 1024-pt one 824 px), centred, then scaled by layer scale × group scale and moved by
  (layer translation × group scale + group translation) × 824/1024. Marker sweeps across scale,
  translation, viewBox size and aspect match this to the pixel.
- A layer's `solid` fill colours its shape.
- Model: blend modes (`normal`, `multiply`, `screen`, `overlay`, `soft-light`, `hard-light`,
  `darken`, `lighten`) follow the W3C separable blend definitions, and opacity scales the layer's
  alpha. Both can differ per appearance.

## Glass

A layer with `glass: true` renders as glass; `glass: false` opts out. A layer without the key
renders as glass when another layer of its group is glass; whether a translucent group alone has
the same effect is not settled. The catalog marks every layer without `glass: false` as glass
([icon-bundle.md](icon-bundle.md#stacks-and-groups)).

- **Frosted** (the group's translucency enabled):
  - With the group's shadow `layer-color`, the layer tints what is behind it: every channel
    darkens by `D·(1 − colour)`, with `D = 63.5/255`. D is the same for any background (0.3 to
    0.9), colour, position and translucency value above 0; the value only switches the tint on.
    Overlapping tinted layers add up (a blue and a red group overlap in dark purple).
  - With any other shadow kind, the layer is a neutral relief: the background under its body
    darkens by a flat 12.7/255 (about 5 %), top to bottom, and the layer's inner detail is
    flattened into that tint.
  - The edge of the glass is feathered like a Gaussian of σ ≈ 19 px at 1024, at every size: the
    profile is monotonic with no rim, and its 10 to 90 % width is about 48 px for circles of
    radius 320 and 160.
- **Opaque** (translucency disabled): the layer keeps its colours with blacks lifted toward a
  floor of 45/255 (KeepingYouAwake's black cup renders 45), and is flat otherwise. `specular`
  makes no difference.
- **Layer shadow**: a glass layer whose group shadow is `layer-color` or `neutral` darkens the
  background below it by `(1 − colour)`, peaking at about 0.49 × opacity (`layer-color`) or
  0.10 × opacity (`neutral`). Model: offset 12 px down, Gaussian σ 17 px, at 1024.

## Icon shadow (macOS)

The squircle casts the same shadow whatever the groups' `shadow` settings (`none`, absent and
`layer-color` give the same halo). Measured alpha outside the edge at 1024: 16, 11, 6 above and
37, 31, 22, 12 below, at 4, 8, 14 and 22 px; it fades out about 45 px from the edge. Model: a
black squircle, offset 9/1024 down, Gaussian σ 17/1024 of the edge, alpha 0.245.

## Edge light (macOS)

White light is added in linear light inside the squircle edge, fading to nothing about 16 px in,
brightest at the top left; all four edges brighten. Measured strength at the edge: about 0.34 at
the top and left, 0.23 at the bottom and right. Model: `L0 = 0.286 + 0.083·d`, with `d` the
outward edge normal's projection on the top-left diagonal, falling linearly to 0 at 16.5 px.

## Properties with little or no static effect

- `specular`: true and false give identical images.
- `lighting` (`individual`, `combined`): identical images; overlapping glass layers merge into one
  relief either way.
- `blur-material`: not a backdrop blur (a striped backdrop stays sharp behind it). It changes the
  image by at most 9/255 and raises the relief's edge contrast (1 at 0, 7 at 2), and stops
  changing at 2 (5 and 20 give the same image as 2).
- Detail inside frosted glass: across input luminance 80 to 240, the relief changes by about
  2/255.

## Not captured by the model

- actool's squircle corners are slightly fuller than n = 5, which leaves a 1 px anti-aliasing
  ring.
- At 128 pt (256 px), actool draws the icon 1 px larger (margin 24 instead of 25).
- A thin bright band, about 30 px, along the top edge of the squircle.
- actool flattens a frosted layer's body more than the neutral-relief model does.
