//! The packer against Apple's iOS app-icon atlases (`tools/atlas_sweep_dataset.json`: per input
//! set, each atlas's images and their bottom-up positions as actool wrote them; the recorded
//! W/H are the images' extent plus 2, not the atlas size).

use accord::packer::{pack_chunk, split_group, PackedImage, MAX_PACKED_AREA_PER_SCALE};

#[derive(serde::Deserialize)]
struct Config {
    scale: u32,
    input: Vec<u32>,
    atlases: Vec<Atlas>,
}

#[derive(serde::Deserialize)]
struct Atlas {
    imgs: Vec<[u32; 4]>,
}

fn layout(imgs: impl Iterator<Item = [u32; 4]>) -> Vec<[u32; 4]> {
    let mut imgs: Vec<_> = imgs.collect();
    imgs.sort();
    imgs
}

#[test]
fn matches_apple_sweep() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/atlas_sweep_dataset.json");
    let configs: Vec<Config> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (mut matched, mut layouts_matched, mut total) = (0, 0, 0);
    for c in &configs {
        let mut want: Vec<_> = c.atlases.iter().map(|a| layout(a.imgs.iter().copied())).collect();
        want.sort();
        let pack = |sizes: &[u32]| {
            // Sizes are distinct, so the dimension index stands in for the rendition key.
            let imgs = sizes
                .iter()
                .map(|&s| PackedImage { scale: c.scale, sort_key: vec![s as u16], ..PackedImage::new(String::new(), 0, s, s) })
                .collect();
            let (chunks, _) = split_group(imgs, MAX_PACKED_AREA_PER_SCALE * c.scale);
            let mut got: Vec<_> = chunks
                .into_iter()
                .map(|(_, imgs)| pack_chunk(imgs))
                .map(|a| layout(a.images.iter().map(|i| [i.width, i.height, i.x, i.y])))
                .collect();
            got.sort();
            got
        };
        matched += (pack(&c.input) == want) as usize;
        // Within an atlas the layout must always match, whatever the split.
        for a in &c.atlases {
            total += 1;
            let sizes: Vec<u32> = a.imgs.iter().map(|i| i[0]).collect();
            layouts_matched += (pack(&sizes) == vec![layout(a.imgs.iter().copied())]) as usize;
        }
    }
    assert_eq!(layouts_matched, total, "atlas layouts");
    assert_eq!(matched, configs.len(), "atlas splits");
}
