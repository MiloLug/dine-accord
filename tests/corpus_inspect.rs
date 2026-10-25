//! `car_read` on every corpus sample: each payload decodes to rows × bytes-per-row, each
//! identifier has a facet, BITMAPKEYS admit every key value the renditions use, and each
//! LOCALIZATIONKEYS entry is the hash of its locale that the rendition keys carry.

mod common;

use accord::car_read::{Car, Payload};
use std::collections::HashSet;

#[test]
fn reader_decodes_every_sample() {
    for s in common::samples() {
        let path = common::root().join("samples").join(&s.name).join("Assets.car");
        let car = Car::parse(&common::read(&path)).unwrap_or_else(|e| panic!("{}: {e:#}", s.name));
        let name = &s.name;
        let header = car.header.as_ref().expect("CARHEADER");
        assert_eq!(header.rendition_count as usize, car.renditions.len(), "{name}: rendition count");

        let id_col = car.keyformat.iter().position(|&a| a == 17).expect("Identifier in KEYFORMAT");
        let facet_ids: HashSet<u16> = car
            .facets
            .iter()
            .filter_map(|f| f.attributes.iter().find(|(a, _)| *a == 17).map(|(_, v)| *v))
            .collect();
        for r in &car.renditions {
            assert_eq!(r.key.len(), car.keyformat.len(), "{name} {}: key length", r.name);
            match &r.payload {
                Payload::Celm { error: Some(e), .. } => panic!("{name} {}: {e}", r.name),
                Payload::Unknown { magic, .. } => panic!("{name} {}: unknown payload {magic}", r.name),
                _ => {}
            }
            let ident = r.key[id_col];
            assert!(ident == 0 || facet_ids.contains(&ident), "{name} {}: identifier {ident} has no facet", r.name);
        }

        if name == "ios/sumup" {
            assert_eq!(car.localizations.len(), 28, "sumup's localized images span 28 locales");
        }
        for (locale, id) in &car.localizations {
            assert_eq!(*id, accord::name_hash::hash_name(locale), "{name}: LOCALIZATIONKEYS {locale}");
        }
        if let Some(col) = car.keyformat.iter().position(|&a| a == 13) {
            let ids: HashSet<u16> = car.localizations.iter().map(|(_, id)| *id).collect();
            for r in car.renditions.iter().filter(|r| r.key[col] != 0) {
                assert!(ids.contains(&r.key[col]), "{name} {}: Localization {} not in LOCALIZATIONKEYS", r.name, r.key[col]);
            }
        }

        for b in &car.bitmap_keys {
            for r in car.renditions.iter().filter(|r| r.key[id_col] as u32 == b.key) {
                for ((&mask, &value), attr) in b.masks.iter().zip(&r.key).zip(&car.keyformat) {
                    assert!(
                        mask == u32::MAX || value >= 32 || mask & (1 << value) != 0,
                        "{name} {}: BITMAPKEYS mask {mask:#x} of attribute {attr} lacks {value}",
                        r.name
                    );
                }
            }
        }
    }
}
