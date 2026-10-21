# Image payloads

How actool encodes the pixels of an image rendition: the CELM container, its compressions, the
deepmap2 codec and the pixel formats. Where a payload sits in the file, and the payloads that are
not images (colours, gradients, multisize sets, raw data), is in [format.md](format.md).

Sources are as in [format.md](format.md): the corpus (`tests/corpus/corpus.toml`) and public
catalogs, named where cited.

## Pixel formats

The CSI header's pixel format names the payload:

| Format | Content |
|---|---|
| `ARGB` | BGRA bytes, premultiplied, 8 bits a channel |
| `GA8 ` | gray and alpha, 8 bits each, premultiplied |
| `GA16` | gray and alpha, little-endian f16 each, premultiplied |
| `RGBW` | premultiplied RGBA, a little-endian f16 per channel, in extended sRGB (colorspace 4); values outside 0..1 kept |
| `RGB5` | seen in sumup (LZFSE CELM), layout unknown |
| `JPEG`, `HEIF` | the file's bytes in a version 0 `DWAR`, layout 12 (roothide, sumup) |
| `PDF `, `SVG ` | vector sources in a `DWAR`, layout 9: PDF as version 0, SVG LZFSE-compressed as version 1 |

JPEGs are stored this way from macOS 10.10, iOS 9.0 and watchOS 2.0 on; below that actool writes
them next to the catalog as loose files (observed).

## CELM

`MLEC` (CELM), u32 version, u32 compression, then the compressed data. Version bit 0 marks a
chunked payload, bit 1 an opaque image.

Compressions:

| Id | Encoding | Seen in |
|---|---|---|
| 0 | raw | sparkle-test-app |
| 1 | RLE | swiftgen (one GA8 atlas) |
| 2 | gzip | sparkle-test-app (CoreUI 691, macOS 10.9) |
| 3 | not decoded | pxcore2 (CoreUI 308) |
| 4 | LZFSE, chunked as `KCBC` or whole | almost every catalog |
| 5 | not decoded | sumup (CoreUI 975, iOS 16.0) |
| 6 | not decoded | scummvm (CoreUI 802, tvOS 9.0) |
| 8 | not decoded | swiftgen (CoreUI 676, iOS 11.0), scummvm |
| 10 | not decoded | moddable (CoreUI 969, macOS 10.14), scummvm |
| 11 | deepmap2 | most catalogs with images, iOS 11.0 and macOS 11.0 targets upward |

RLE (1) data is three u32 (3 in swiftgen's GA8 atlas, the width, the height), a u32 offset per
row counted from the start of the RLE data, then the rows. Identical rows share one offset. A row
is a series of runs (`u16 count, u16 0x8000`, one pixel) and literals (`u16 count, u16 0`, count
pixels) that adds up to the width; rows are tight, and the TLV 1007 stride pads them.

LZFSE (4) payloads come in two forms:
- chunked (version bit 0 set): u32 chunk count, then per chunk `KCBC`, u32 0, u32 0, u32 row
  count, u32 length, LZFSE stream. In current catalogs each chunk holds a third of the height,
  rounded down, plus one chunk for the remainder; acextract (CoreUI 374) and a few small images
  (kdrive, utm-remote's 64-px atlases) have 2;
- whole (bit 0 clear): u32 length, then one LZFSE stream (kdrive, acextract).

Which compression a rendition gets depends on the actool build, the deployment target and the
rendition:
- App icons (part 220) and icon atlases use LZFSE in every current catalog (gzip in
  sparkle-test-app), never deepmap2.
- Other images and atlases use deepmap2 in current catalogs (daintree, mediainfo, readinglist and
  sentinel at macOS 26.0; roothide, utm-remote and openutau at iOS 15.0 to 26.0). sumup's images
  from HEIC sources use compression 5, and its `RGB5` images LZFSE.
- Below macOS 11, CoreUI 969 builds use LZFSE (kdrive, macOS 10.13) or compression 10 (moddable,
  macOS 10.14).
- Older builds store an image once per deployment-target class, each copy with its own
  compression: swiftgen's `Apple-icon.png` has DeploymentTarget (25) 0, 3 and 5, compressed 4, 8
  and 11; scummvm does the same through attribute 5 (4; 8 or 10; 11). This is where compressions
  6, 8 and 10 appear.

## deepmap2

Compression 11. The CELM data is a 16-byte header (u32 1, u32 format code, u32 stream length,
u32 0) and a `dmp2` stream. An image of 2^20 pixels or more is split into `KCBC` chunks of
`2^20 / width` rows, each with its own header and stream.

Format codes: 2 `GA8 `, 4 `ARGB`, 18 `GA16`, 20 `RGBW`.

The `dmp2` stream: `dmp2`, u8 method, u8 1, u8 10, u8 format code, u16 tile width, u16 tile
height, then the tiles row by row, each a u32 length and a body compressed with LZVN (a raw stream,
no LZFSE block header) when the input is under 4 KiB and with LZFSE above. Tiles are at most
16384 pixels wide and 2 MiB decoded.

Methods:
- **1**, raw: the pixels, untiled.
- **3**, packed: each tile's pixels.
- **4**, palette (`ARGB` only): at most 256 colours, stored as `count | kind << 16` and the entries
  after the header. Kind 4 stores whole pixels and one index plane; kind 3 stores 24-bit colours,
  then an alpha plane before the index plane.
- **2**, predicted:
  - Each pixel becomes Y, Co, Cg in 16 bits (YCoCg-R with Co and Cg halved, so the method drops
    their low bit; f16 values are scaled by 512 first), plus alpha in 8 bits. Gray formats store
    only Y.
  - The body is the alpha plane, one predictor byte per row, then the high bytes and the low
    bytes of the residuals in sign-magnitude form (2v, or −2v+1 when negative), padded to 16
    bytes.
  - Predictors: 0 none, 1 left or up chosen per pixel from Y, 2 left, 3 up, 4 mean of left and
    up. Each row takes the predictor with the smallest sum of ln(|residual| + 1).

actool encodes with methods 2, 3 and 4 (both palette kinds, the smaller kept, kind 4 on a tie) and
keeps the smallest result; on equal sizes method 2 wins over 3 and 3 over 4. A result must fit in
the raw size plus 12 bytes; when none does, it writes method 1. Re-encoding every 8-bit deepmap2
payload in the corpus with these rules gives the same bytes, except scummvm's (CoreUI 802, an
older encoder).

## Row stride

TLV 1007 (bytes per row) equals the payload's row stride:
- LZFSE and `KCBC`: `width × bytes per pixel`, rounded up to 32. `GA8` is 2 bytes a pixel.
- deepmap2: `width × 4` for the 8-bit formats (`GA8` included) or `width × 8` for `GA16` and
  `RGBW`, rounded up to 32.
