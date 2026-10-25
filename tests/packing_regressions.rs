//! Which imagesets are packed into atlases and which stay inline.

use accord::compiler;
use image::{ImageBuffer, Rgba};
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_tmp(name: &str) -> PathBuf {
    let dir = PathBuf::from("tmp").join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `channels` "LA" writes grey RGBA pixels, which the compiler stores as GA8.
fn make_imageset(parent: &Path, name: &str, channels: &str) -> anyhow::Result<()> {
    let iset = parent.join(format!("{name}.imageset"));
    fs::create_dir_all(&iset)?;
    for (suffix, size, _scale) in [("", 16u32, "1x"), ("@2x", 32u32, "2x")] {
        let filename = format!("{name}{suffix}.png");
        let img = match channels {
            "RGBA" => {
                let buf: ImageBuffer<Rgba<u8>, Vec<u8>> =
                    ImageBuffer::from_pixel(size, size, Rgba([255, 0, 0, 255]));
                buf
            }
            "LA" => {
                let buf: ImageBuffer<Rgba<u8>, Vec<u8>> =
                    ImageBuffer::from_pixel(size, size, Rgba([128, 128, 128, 255]));
                buf
            }
            _ => unreachable!(),
        };
        img.save(iset.join(&filename))?;
    }
    let contents = serde_json::json!({
        "images": [
            {"filename": format!("{name}.png"), "idiom": "mac", "scale": "1x"},
            {"filename": format!("{name}@2x.png"), "idiom": "mac", "scale": "2x"},
        ],
        "info": {"author": "xcode", "version": 1},
    });
    fs::write(iset.join("Contents.json"), serde_json::to_string(&contents)?)?;
    Ok(())
}

fn make_catalog(tmpdir: &Path, imagesets: &[(&str, &str)]) -> anyhow::Result<PathBuf> {
    let catalog = tmpdir.join("Test.xcassets");
    fs::create_dir_all(&catalog)?;
    fs::write(
        catalog.join("Contents.json"),
        r#"{"info":{"author":"xcode","version":1}}"#,
    )?;
    for (name, channels) in imagesets {
        make_imageset(&catalog, name, channels)?;
    }
    Ok(catalog)
}

/// Parse the CAR and return a map of rendition name -> layout code.
fn parse_car_layouts(car_path: &Path) -> std::collections::HashMap<String, u16> {
    let data = fs::read(car_path).expect("read car");
    let mut map = std::collections::HashMap::new();
    let mut pos = 0usize;
    while let Some(idx) = data[pos..].windows(4).position(|w| w == b"ISTC") {
        pos += idx;
        if pos + 168 > data.len() {
            break;
        }
        let layout = u16::from_le_bytes(data[pos + 36..pos + 38].try_into().unwrap());
        let name_end = data[pos + 40..pos + 168]
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(128);
        let name = String::from_utf8_lossy(&data[pos + 40..pos + 40 + name_end]).to_string();
        map.insert(name, layout);
        pos += 4;
    }
    map
}

#[test]
fn lone_ga8_stored_inline() {
    let tmp = workspace_tmp("packing_lone_ga8");
    let catalog = make_catalog(
        &tmp,
        &[("A", "RGBA"), ("B", "RGBA"), ("C", "RGBA"), ("Mono", "LA")],
    )
    .expect("make catalog");
    let out = tmp.join("out");
    compiler::compile(&[(&catalog).to_path_buf()], &out, &compiler::Options { platform: ("macosx").to_string(), min_deploy: ("11.0").to_string(), app_icon: None, info_plist: None, accent_color: None, widget_background_color: None, standalone_icon_behavior: ("default").to_string(), target_devices: Vec::new(), include_all_app_icons: false, include_languages: None, development_region: None, plist_localizations: true, ..Default::default() })
    .expect("compile");
    let layouts = parse_car_layouts(&out.join("Assets.car"));
    assert_eq!(layouts.get("Mono.png").copied(), Some(12));
    assert_eq!(layouts.get("Mono@2x.png").copied(), Some(12));
    assert_eq!(layouts.get("A.png").copied(), Some(1003));
}

#[test]
fn single_imageset_stored_inline() {
    let tmp = workspace_tmp("packing_single");
    let catalog = make_catalog(&tmp, &[("Solo", "RGBA")]).expect("make");
    let out = tmp.join("out");
    compiler::compile(&[(&catalog).to_path_buf()], &out, &compiler::Options { platform: ("macosx").to_string(), min_deploy: ("11.0").to_string(), app_icon: None, info_plist: None, accent_color: None, widget_background_color: None, standalone_icon_behavior: ("default").to_string(), target_devices: Vec::new(), include_all_app_icons: false, include_languages: None, development_region: None, plist_localizations: true, ..Default::default() })
    .expect("compile");
    let layouts = parse_car_layouts(&out.join("Assets.car"));
    assert_eq!(layouts.get("Solo.png").copied(), Some(12));
    // No atlas renditions
    assert!(!layouts.keys().any(|n| n.starts_with("ZZZZ")));
}

#[test]
fn all_same_format_packed() {
    let tmp = workspace_tmp("packing_same_format");
    let catalog =
        make_catalog(&tmp, &[("X", "LA"), ("Y", "LA"), ("Z", "LA")]).expect("make");
    let out = tmp.join("out");
    compiler::compile(&[(&catalog).to_path_buf()], &out, &compiler::Options { platform: ("macosx").to_string(), min_deploy: ("11.0").to_string(), app_icon: None, info_plist: None, accent_color: None, widget_background_color: None, standalone_icon_behavior: ("default").to_string(), target_devices: Vec::new(), include_all_app_icons: false, include_languages: None, development_region: None, plist_localizations: true, ..Default::default() })
    .expect("compile");
    let layouts = parse_car_layouts(&out.join("Assets.car"));
    for name in ["X.png", "Y.png", "Z.png"] {
        assert_eq!(
            layouts.get(name).copied(),
            Some(1003),
            "{name} should be packed"
        );
    }
}

#[test]
fn two_formats_one_each_both_inline() {
    let tmp = workspace_tmp("packing_one_each");
    let catalog =
        make_catalog(&tmp, &[("Color", "RGBA"), ("Gray", "LA")]).expect("make");
    let out = tmp.join("out");
    compiler::compile(&[(&catalog).to_path_buf()], &out, &compiler::Options { platform: ("macosx").to_string(), min_deploy: ("11.0").to_string(), app_icon: None, info_plist: None, accent_color: None, widget_background_color: None, standalone_icon_behavior: ("default").to_string(), target_devices: Vec::new(), include_all_app_icons: false, include_languages: None, development_region: None, plist_localizations: true, ..Default::default() })
    .expect("compile");
    let layouts = parse_car_layouts(&out.join("Assets.car"));
    assert_eq!(layouts.get("Color.png").copied(), Some(12));
    assert_eq!(layouts.get("Gray.png").copied(), Some(12));
}
