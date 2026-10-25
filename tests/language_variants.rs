//! Localized and direction-specific image variants: LOCALIZATIONKEYS and the Direction key
//! attribute.

use accord::car_read::Car;
use accord::compiler;
use accord::name_hash::hash_name;
use std::path::Path;

fn write_png(path: &Path, rgba: [u8; 4]) {
    image::RgbaImage::from_pixel(40, 40, image::Rgba(rgba)).save(path).unwrap();
}

/// A 10×5 pt page filled red.
const RED_PDF: &[u8] = b"%PDF-1.4
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 10 5]/Contents 4 0 R>>endobj
4 0 obj<</Length 24>>stream
1 0 0 rg 0 0 10 5 re f
endstream endobj
trailer<</Root 1 0 R>>
%%EOF";

/// Two imagesets whose images carry the given extra Contents.json fields, compiled for iOS. Two
/// sets give each variant a packing group of two, so the images land in atlases.
fn compile_imageset(images: &[(&str, &str)]) -> Car {
    let dir = tempfile::TempDir::new().unwrap();
    for set_name in ["Flag", "Flag2"] {
        let set = dir.path().join(format!("A.xcassets/{set_name}.imageset"));
        std::fs::create_dir_all(&set).unwrap();
        let mut entries = Vec::new();
        for (i, (file, extra)) in images.iter().enumerate() {
            if file.ends_with(".pdf") {
                std::fs::write(set.join(file), RED_PDF).unwrap();
            } else {
                write_png(&set.join(file), [40 * i as u8, 0, 255, 255]);
            }
            entries.push(format!(r#"{{"idiom":"universal","scale":"2x","filename":"{file}"{extra}}}"#));
        }
        let contents = format!(r#"{{"images":[{}],"info":{{"author":"xcode","version":1}}}}"#, entries.join(","));
        std::fs::write(set.join("Contents.json"), contents).unwrap();
    }
    let out = dir.path().join("out");
    let opts = compiler::Options {
        platform: "iphoneos".into(),
        min_deploy: "17.0".into(),
        standalone_icon_behavior: "default".into(),
        ..Default::default()
    };
    compiler::compile(&[dir.path().join("A.xcassets")], &out, &opts).unwrap();
    Car::parse(&std::fs::read(out.join("Assets.car")).unwrap()).unwrap()
}

fn assert_unique_keys(car: &Car) {
    let mut keys: Vec<&Vec<u16>> = car.renditions.iter().map(|r| &r.key).collect();
    keys.sort();
    let n = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), n, "rendition keys collide");
}

fn attribute_values(car: &Car, attribute: u16) -> Vec<u16> {
    let column = car.keyformat.iter().position(|&a| a == attribute);
    let mut v: Vec<u16> = car.renditions.iter().map(|r| column.map_or(0, |c| r.key[c])).collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn localized_images_are_listed_in_localizationkeys() {
    let car = compile_imageset(&[
        ("any.png", ""),
        ("de.png", r#","locale":"de""#),
        ("frca.png", r#","locale":"fr-CA""#),
    ]);
    assert_eq!(
        car.localizations,
        [("de".to_string(), hash_name("de")), ("fr-CA".to_string(), hash_name("fr-CA"))]
    );
    let mut ids: Vec<u16> = car.localizations.iter().map(|(_, id)| *id).collect();
    ids.insert(0, 0);
    ids.sort();
    assert_eq!(attribute_values(&car, 13), ids, "Localization key values");
    assert!(car.tree_key_sizes.contains(&("LOCALIZATIONKEYS".to_string(), -1)));
    assert!(car.renditions.iter().any(|r| r.name.starts_with("ZZZZPackedAsset")), "images are packed");
    assert_unique_keys(&car);
}

#[test]
fn localized_pdf_renditions_keep_their_locale() {
    let car = compile_imageset(&[("any.pdf", ""), ("de.pdf", r#","locale":"de""#)]);
    let column = car.keyformat.iter().position(|&a| a == 13).expect("Localization in KEYFORMAT");
    for r in &car.renditions {
        match r.name.as_str() {
            "de.pdf" => assert_eq!(r.key[column], hash_name("de"), "{r:?}"),
            "any.pdf" => assert_eq!(r.key[column], 0),
            _ => {}
        }
    }
    assert!(car.renditions.iter().filter(|r| r.name == "de.pdf").count() > 2, "vector and raster renditions");
    assert_unique_keys(&car);
}

#[test]
fn equal_length_locales_make_a_fixed_key_tree() {
    let car = compile_imageset(&[("de.png", r#","locale":"de""#), ("fr.png", r#","locale":"fr""#)]);
    assert!(car.tree_key_sizes.contains(&("LOCALIZATIONKEYS".to_string(), 2)));
}

#[test]
fn unlocalized_catalog_has_no_localizationkeys() {
    let car = compile_imageset(&[("any.png", "")]);
    assert!(!car.vars.iter().any(|v| v == "LOCALIZATIONKEYS"));
}

fn direction_renditions(car: &Car) -> Vec<(u16, u32)> {
    let column = car.keyformat.iter().position(|&a| a == 4).expect("Direction in KEYFORMAT");
    car.renditions.iter().filter(|r| r.key[column] != 0).map(|r| (r.key[column], r.flags)).collect()
}

#[test]
fn both_directions_get_their_own_keys() {
    let car = compile_imageset(&[
        ("l.png", r#","language-direction":"left-to-right""#),
        ("r.png", r#","language-direction":"right-to-left""#),
    ]);
    let direction = car.keyformat.iter().position(|&a| a == 4).expect("Direction in KEYFORMAT");
    assert_eq!(car.keyformat[direction + 1], 17, "Direction sits right before Identifier");
    let found = direction_renditions(&car);
    assert!(found.iter().any(|&(d, _)| d == 5) && found.iter().any(|&(d, _)| d == 4));
    assert!(found.iter().all(|&(_, flags)| flags & 0x40 == 0), "a set with both directions is not mirrored");
    assert_unique_keys(&car);
}

#[test]
fn single_direction_is_flippable() {
    let car = compile_imageset(&[("l.png", r#","language-direction":"left-to-right""#)]);
    let found = direction_renditions(&car);
    assert!(!found.is_empty());
    assert!(found.iter().all(|&(d, flags)| d == 5 && flags & 0x40 != 0), "{found:?}");
}

#[test]
fn no_direction_no_column() {
    let car = compile_imageset(&[("any.png", "")]);
    assert!(!car.keyformat.contains(&4));
}
