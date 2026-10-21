# Atlas packing

How actool packs small images into `ZZZZPackedAsset` atlases. These rules reproduce the splits and
positions of the 59 iOS app-icon configurations recorded in `tools/atlas_sweep_dataset.json`, and
roothide's atlases, sizes included. The atlas and `INLK` renditions are described in
[format.md](format.md#atlases).

## Which images pack

Packing starts at deployment target macOS 10.11, iOS and tvOS 9.0, watchOS 2.0 (observed;
sparkle-test-app at macOS 10.9 has no atlas). A raster image rendition (PNG, or a PDF/SVG
rasterized at a scale; never a JPEG) packs when its pixel area is at most 18050 × scale. The
images of every document in one actool run share the atlases. Images are grouped by a key of
'.'-joined fields:

`scale.opaque.monochrome.subtype.idiom.0.0.explicit.0.0.gamut.0.0.appearance.localization.0`

- *opaque*: app icons (and `ios-marketing` images) whose pixels are all opaque; other images whose
  source file has no alpha channel. Opaque groups get atlases named `…-<scale>.1.<mono>-…`.
- *monochrome*: gray images (`GA8`), named `…-<scale>.<opaque>.1-…`.
- *explicit*: the sprite atlas of a `.spriteatlas`; those atlases are `ZZZZExplicitlyPackedAsset`.
  No corpus sample has a sprite atlas, so the sprite-atlas rules on this page are unverified.

A group of one image stays inline, except in a sprite atlas.

## Splitting a group

The images are ordered by pixel area, largest first; equal areas by their rendition key in the
platform's full key order (on iOS: Appearance, Localization, Scale, Idiom, Subtype, …,
Dimension2, Dimension1, …, Identifier, Element, Part; on macOS: Appearance, Localization,
Element, Part, …, Identifier, Dimension1, Dimension2, …, Scale, DisplayGamut).

From the first unplaced image, images are added while their summed area stays below
18050 × scale (sprite atlases: 0x366666 / scale × scale). At the image that reaches it:

1. the images before it are laid out, and the layout's empty nodes counted (see below);
2. if that many more images remain, the images before it plus that many are laid out too; when
   both layouts enclose the same area, the chunk extends that far;
3. the chunk ends at (and includes) that image, or the extended end; the next chunk starts after.

An image that reaches the limit on its own (the first of a chunk) is left inline. Chunks are
numbered by this step count, including steps that made no chunk, and the group key plus `.<n>`
names the chunk.

## Layout

Each chunk is laid out with a growing binary-tree packer, every image padded to (w + 2, h + 2):

- the canvas starts as the first image's padded size;
- an image goes into the first free node that fits, searching a used node's right child before
  its down child; the node splits into a down node (full width, below the image) and a right node
  (image height, beside it);
- when nothing fits, the canvas grows right if `h ≤ height` and `height ≥ width + w`, else down
  if `w ≤ width` and `width ≥ height + h`, else right if `h ≤ height`, else down;
- an image's position is its node's origin plus 2.

Empty nodes, for the split above, are counted from the root's children: a node holding no image
counts 1 and is not descended into (so an earlier canvas that the packer grew around counts once).

The atlas is the enclosing size plus 1, rounded up to even, in each direction.

## Numbering

Chunk keys are visited in string order. Dimension1 counts atlases per chunk key without its
opaque, monochrome and compression fields and without the chunk number, so an opaque and a
non-opaque atlas of one idiom are Dimension1 0 and 1.
