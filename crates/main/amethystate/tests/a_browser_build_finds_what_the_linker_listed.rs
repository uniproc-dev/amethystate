#![cfg(target_arch = "wasm32")]

use amethystate::migration::registry::compiled_steps;
use amethystate::schema::declarations;
use amethystate::{AmeData, amethystate, migrate};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

mod v1 {
    use super::*;

    #[amethystate(prefix = "browser_probe", version = 1)]
    pub struct Probe {
        #[amestate(default = 1u32)]
        pub width: u32,
    }
}

#[amethystate(prefix = "browser_probe", version = 2)]
pub struct Probe {
    #[amestate(default = 1u32)]
    pub width: u32,
}

#[migrate]
fn widen_the_probe(old: AmeData<v1::Probe>) -> amethystate::MigrationResult<AmeData<Probe>> {
    Ok(AmeData::<Probe> {
        width: old.width * 2,
    })
}

#[wasm_bindgen_test]
fn every_declaration_in_the_binary_is_listed() {
    let mut listed: Vec<(&str, u32)> = declarations()
        .iter()
        .filter(|entry| entry.prefix.to_string() == "browser_probe")
        .map(|entry| (entry.struct_name, entry.version))
        .collect();
    listed.sort();

    assert_eq!(listed, [("Probe", 1), ("Probe", 2)]);
}

#[wasm_bindgen_test]
fn every_migrate_step_in_the_binary_is_listed() {
    let listed: Vec<u32> = compiled_steps()
        .iter()
        .filter(|step| step.prefix.path().to_string() == "browser_probe")
        .map(|step| step.target_version)
        .collect();

    assert_eq!(listed, [2]);
}
