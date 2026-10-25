//! From macOS 10.10 (iOS 9.0, watchOS 2.0) actool stores JPEGs in the CAR as DWAR-wrapped `GEPJ`
//! data with layout 12; below that it copies them out as loose files.

use accord::{car, compiler};
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_tmp(name: &str) -> PathBuf {
    let dir = PathBuf::from("tmp").join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_jpeg_catalog(tmpdir: &Path) -> PathBuf {
    let catalog = tmpdir.join("Test.xcassets");
    fs::create_dir_all(&catalog).unwrap();
    fs::write(
        catalog.join("Contents.json"),
        r#"{"info":{"author":"xcode","version":1}}"#,
    )
    .unwrap();
    let iset = catalog.join("Photo.imageset");
    fs::create_dir_all(&iset).unwrap();
    // Not decodable: passthrough only reads the size from the SOFn marker.
    let buf: Vec<u8> = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xC0, // SOF0
        0x00, 0x0B, // segment len
        0x08, // precision
        0x00, 0x20, 0x00, 0x20, // 32x32
        0x01, // 1 component
        0x01, 0x11, 0x00, // component spec
        0xFF, 0xD9, // EOI
    ];
    fs::write(iset.join("photo.jpg"), &buf).unwrap();
    let contents = r#"{"images":[{"filename":"photo.jpg","idiom":"mac","scale":"1x"}],"info":{"author":"xcode","version":1}}"#;
    fs::write(iset.join("Contents.json"), contents).unwrap();
    catalog
}

fn parse_pixfmts(car_path: &Path) -> Vec<[u8; 4]> {
    let data = fs::read(car_path).unwrap();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(idx) = data[pos..].windows(4).position(|w| w == b"ISTC") {
        pos += idx;
        if pos + 28 > data.len() {
            break;
        }
        out.push(data[pos + 24..pos + 28].try_into().unwrap());
        pos += 4;
    }
    out
}

#[test]
fn jpeg_stored_in_car_at_11_0() {
    let tmp = workspace_tmp("jpeg_11_0");
    let catalog = make_jpeg_catalog(&tmp);
    let out = tmp.join("out");
    compiler::compile(&[(&catalog).to_path_buf()], &out, &compiler::Options { platform: ("macosx").to_string(), min_deploy: ("11.0").to_string(), app_icon: None, info_plist: None, accent_color: None, widget_background_color: None, standalone_icon_behavior: ("default").to_string(), target_devices: Vec::new(), include_all_app_icons: false, include_languages: None, development_region: None, plist_localizations: true, ..Default::default() })
    .expect("compile");
    let car = out.join("Assets.car");
    assert!(car.exists(), "Assets.car missing");
    let fmts = parse_pixfmts(&car);
    assert!(fmts.iter().any(|f| f == b"GEPJ"), "GEPJ rendition missing");
    assert!(!out.join("Photo.jpg").exists());
}

#[test]
fn jpeg_as_loose_file_at_10_9() {
    let tmp = workspace_tmp("jpeg_10_9");
    let catalog = make_jpeg_catalog(&tmp);
    let out = tmp.join("out");
    compiler::compile(&[(&catalog).to_path_buf()], &out, &compiler::Options { platform: ("macosx").to_string(), min_deploy: ("10.9").to_string(), app_icon: None, info_plist: None, accent_color: None, widget_background_color: None, standalone_icon_behavior: ("default").to_string(), target_devices: Vec::new(), include_all_app_icons: false, include_languages: None, development_region: None, plist_localizations: true, ..Default::default() })
    .expect("compile");
    assert!(out.join("Photo.jpg").exists(), "loose Photo.jpg missing");
    // No CAR for a catalog that contains only JPEGs at pre-10.10 targets.
    assert!(!out.join("Assets.car").exists(), "no CAR expected");
}

#[test]
fn jpeg_dimensions_from_sofn() {
    let data = [
        0xFFu8, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x20, 0x00, 0x20, 0x01, 0x01, 0x11,
        0x00, 0xFF, 0xD9,
    ];
    assert_eq!(car::jpeg_dimensions(&data), (32, 32));
}

#[test]
fn jpeg_dimensions_non_jpeg_returns_zero() {
    assert_eq!(car::jpeg_dimensions(b"not a jpeg"), (0, 0));
    assert_eq!(car::jpeg_dimensions(&[]), (0, 0));
}
