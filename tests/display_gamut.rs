//! Wide-gamut (DisplayGamut 1) copies of images whose ICC profile is wide and whose colours leave
//! sRGB.

use accord::car_read::{Car, Rendition};
use accord::compiler;
use moxcms::{ColorProfile, Xyzd};
use std::fs;
use std::path::{Path, PathBuf};

const DISPLAY_GAMUT: u16 = 24;

fn workspace_tmp(name: &str) -> PathBuf {
    let dir = PathBuf::from("tmp").join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_png(path: &Path, size: u32, rgba: [u8; 4], icc: &[u8]) {
    let mut info = png::Info::with_size(size, size);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Eight;
    info.icc_profile = Some(icc.to_vec().into());
    let file = fs::File::create(path).unwrap();
    let mut writer = png::Encoder::with_info(file, info).unwrap().write_header().unwrap();
    writer.write_image_data(&rgba.repeat((size * size) as usize)).unwrap();
}

/// A catalog of one-image imagesets, `(name, edge in pixels, RGBA, ICC profile)`.
fn make_catalog(dir: &Path, sets: &[(&str, u32, [u8; 4], &[u8])]) -> PathBuf {
    let catalog = dir.join("Test.xcassets");
    fs::create_dir_all(&catalog).unwrap();
    fs::write(catalog.join("Contents.json"), r#"{"info":{"author":"xcode","version":1}}"#).unwrap();
    for (name, size, rgba, icc) in sets {
        let iset = catalog.join(format!("{name}.imageset"));
        fs::create_dir_all(&iset).unwrap();
        write_png(&iset.join(format!("{name}.png")), *size, *rgba, icc);
        let contents = serde_json::json!({
            "images": [{"filename": format!("{name}.png"), "idiom": "universal", "scale": "1x"}],
            "info": {"author": "xcode", "version": 1},
        });
        fs::write(iset.join("Contents.json"), contents.to_string()).unwrap();
    }
    catalog
}

fn compile(catalog: &Path, out: &Path, platform: &str) -> Car {
    let opts = compiler::Options {
        platform: platform.to_string(),
        min_deploy: "17.0".to_string(),
        app_icon: None,
        info_plist: None,
        accent_color: None,
        widget_background_color: None,
        standalone_icon_behavior: "default".to_string(),
        target_devices: if platform == "iphoneos" { vec!["iphone".to_string()] } else { Vec::new() },
        include_all_app_icons: false,
        include_languages: None,
        development_region: None,
        plist_localizations: true,
    };
    compiler::compile(&[catalog.to_path_buf()], out, &opts).unwrap();
    Car::parse(&fs::read(out.join("Assets.car")).unwrap()).unwrap()
}

fn gamut(car: &Car, r: &Rendition) -> u16 {
    car.keyformat.iter().position(|&a| a == DISPLAY_GAMUT).map_or(0, |i| r.key[i])
}

fn named<'a>(car: &'a Car, name: &str) -> Vec<&'a Rendition> {
    car.renditions.iter().filter(|r| r.name == name).collect()
}

fn f16s(px: &[u8]) -> Vec<f32> {
    px.chunks_exact(2).map(|b| half::f16::from_le_bytes([b[0], b[1]]).to_f32()).collect()
}

fn p3() -> Vec<u8> {
    ColorProfile::new_display_p3().encode().unwrap()
}

#[test]
fn p3_red_gets_a_wide_gamut_copy() {
    let dir = workspace_tmp("display_gamut_p3_red");
    let car = compile(&make_catalog(&dir, &[("red", 300, [255, 0, 0, 255], &p3())]), &dir.join("out"), "macosx");

    assert_eq!(car.keyformat.last(), Some(&DISPLAY_GAMUT));
    let rs = named(&car, "red.png");
    assert_eq!(rs.len(), 2);
    let (srgb, wide) = if gamut(&car, rs[0]) == 0 { (rs[0], rs[1]) } else { (rs[1], rs[0]) };
    assert_eq!(gamut(&car, wide), 1);
    assert_eq!((srgb.pixel_format.as_str(), srgb.colorspace), ("ARGB", 1));
    assert_eq!((wide.pixel_format.as_str(), wide.colorspace), ("RGBW", 4));

    // P3 red in extended sRGB is about (1.093, -0.227, -0.150).
    let px = f16s(&wide.data.as_ref().unwrap()[..8]);
    assert!(px[0] > 1.08 && px[1] < -0.2 && px[2] < -0.1 && px[3] == 1.0, "{px:?}");
    assert_eq!(&srgb.rgba8().unwrap()[..4], &[255, 0, 0, 255]);

    let ident = car.keyformat.iter().position(|&a| a == 17).map(|i| rs[0].key[i]).unwrap();
    let masks = &car.bitmap_keys.iter().find(|k| k.key == ident as u32).unwrap().masks;
    assert_eq!(masks.last(), Some(&0b11));
}

#[test]
fn p3_file_with_srgb_colours_gets_no_copy() {
    let dir = workspace_tmp("display_gamut_inside");
    let car = compile(&make_catalog(&dir, &[("muted", 300, [140, 100, 90, 255], &p3())]), &dir.join("out"), "macosx");

    assert!(!car.keyformat.contains(&DISPLAY_GAMUT));
    let rs = named(&car, "muted.png");
    assert_eq!(rs.len(), 1);
    // Converted to sRGB, which needs a stronger red for the same colour.
    let px = rs[0].rgba8().unwrap();
    assert!(px[0] > 140 && px[3] == 255, "{:?}", &px[..4]);
}

#[test]
fn narrow_profile_gets_no_copy() {
    // Colorants of wireshark's "Cinema Display Calibrated", a gamut about as large as sRGB's.
    let mut cinema = ColorProfile::new_srgb();
    cinema.red_colorant = Xyzd { x: 0.4614, y: 0.2428, z: 0.0099 };
    cinema.green_colorant = Xyzd { x: 0.3533, y: 0.6787, z: 0.0859 };
    cinema.blue_colorant = Xyzd { x: 0.1495, y: 0.0790, z: 0.7290 };
    let dir = workspace_tmp("display_gamut_narrow");
    let icc = cinema.encode().unwrap();
    let car = compile(&make_catalog(&dir, &[("green", 300, [0, 255, 0, 255], &icc)]), &dir.join("out"), "macosx");

    assert!(!car.keyformat.contains(&DISPLAY_GAMUT));
    assert_eq!(named(&car, "green.png").len(), 1);
}

#[test]
fn ios_key_puts_display_gamut_before_identifier() {
    let dir = workspace_tmp("display_gamut_ios");
    let car = compile(&make_catalog(&dir, &[("red", 300, [255, 0, 0, 255], &p3())]), &dir.join("out"), "iphoneos");

    assert_eq!(car.keyformat, [7, 13, 12, 15, 16, DISPLAY_GAMUT, 17, 1, 2]);
    assert_eq!(named(&car, "red.png").len(), 2);
}

#[test]
fn small_wide_images_pack_into_gamut1_atlases() {
    let dir = workspace_tmp("display_gamut_packed");
    let icc = p3();
    let car = compile(
        &make_catalog(&dir, &[("a", 16, [255, 0, 0, 255], &icc), ("b", 20, [0, 255, 0, 255], &icc)]),
        &dir.join("out"),
        "macosx",
    );

    let mut atlases: Vec<(&str, u16, &str)> = car
        .renditions
        .iter()
        .filter(|r| r.name.starts_with("ZZZZPackedAsset"))
        .map(|r| (r.name.as_str(), gamut(&car, r), r.pixel_format.as_str()))
        .collect();
    atlases.sort();
    assert_eq!(
        atlases,
        [("ZZZZPackedAsset-1.0.0-gamut0", 0, "ARGB"), ("ZZZZPackedAsset-1.0.0-gamut1", 1, "RGBW")]
    );
    let refs = named(&car, "a.png");
    let mut gamuts: Vec<u16> = refs.iter().map(|r| gamut(&car, r)).collect();
    gamuts.sort();
    assert_eq!(gamuts, [0, 1]);
    assert!(refs.iter().all(|r| r.layout == 1003));
}
