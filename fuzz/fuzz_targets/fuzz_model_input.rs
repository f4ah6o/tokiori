//! Arbitrary input must never panic the model parser.
//!
//! A rejected input is a correct outcome: the parser returns an error and the
//! harness only fails on a panic, a hang, or a successful parse that cannot be
//! re-serialized and re-parsed identically.

#![no_main]

use libfuzzer_sys::fuzz_target;
use tokiori_model::{canonical_json, parse_model_str};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(model) = parse_model_str(text) {
        // Anything the parser accepts must survive a round trip; a canonical
        // form that cannot be read back would make stored models unreadable.
        let canonical = canonical_json(&model);
        let reparsed = parse_model_str(&canonical).expect("canonical form must parse");
        assert_eq!(
            model_digest_of(&reparsed),
            model_digest_of(&model),
            "round trip changed the model digest"
        );
    }
});

fn model_digest_of(model: &tokiori_model::Model) -> String {
    tokiori_model::model_digest(model)
}
