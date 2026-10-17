//! The packer against Apple's iOS app-icon atlases (`tools/atlas_sweep_dataset.json`: per input
//! set, each atlas's size and its images' bottom-up positions as actool wrote them).

use accord::packer::{pack_images_split, PackedImage};

#[derive(serde::Deserialize)]
struct Config {
    scale: u32,
    input: Vec<u32>,
    atlases: Vec<Atlas>,
}

#[derive(serde::Deserialize)]
#[allow(non_snake_case)]
struct Atlas {
    W: u32,
    H: u32,
    imgs: Vec<[u32; 4]>,
}

fn layout(w: u32, h: u32, imgs: impl Iterator<Item = [u32; 4]>) -> (u32, u32, Vec<[u32; 4]>) {
    let mut imgs: Vec<_> = imgs.collect();
    imgs.sort();
    (w, h, imgs)
}

#[test]
fn matches_apple_sweep() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/atlas_sweep_dataset.json");
    let configs: Vec<Config> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (mut matched, mut layouts_matched, mut total) = (0, 0, 0);
    for c in &configs {
        let mut want: Vec<_> = c.atlases.iter().map(|a| layout(a.W, a.H, a.imgs.iter().copied())).collect();
        want.sort();
        let pack = |sizes: &[u32]| {
            let imgs = sizes
                .iter()
                .map(|&s| PackedImage { scale: c.scale, ..PackedImage::new(String::new(), 0, s, s) })
                .collect();
            let mut got: Vec<_> = pack_images_split(imgs)
                .iter()
                .map(|a| layout(a.width, a.height, a.images.iter().map(|i| [i.width, i.height, i.x, i.y])))
                .collect();
            got.sort();
            got
        };
        matched += (pack(&c.input) == want) as usize;
        // Within an atlas the layout must always match, whatever the split.
        for a in &c.atlases {
            total += 1;
            let sizes: Vec<u32> = a.imgs.iter().map(|i| i[0]).collect();
            layouts_matched += (pack(&sizes) == vec![layout(a.W, a.H, a.imgs.iter().copied())]) as usize;
        }
    }
    assert_eq!(layouts_matched, total, "atlas layouts");
    assert_eq!(matched, configs.len(), "atlas splits");
}
