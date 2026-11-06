//! Property tests for the model AST.
//!
//! Run with a fixed seed for reproducible gate evidence:
//!
//! ```text
//! TOKIORI_PROPTEST_SEED=20261107 cargo test -p tokiori-model --test pbt_model
//! ```
//!
//! A failure prints the seed, case index, and generated trace; rerun with the
//! same seed and case budget to reproduce.

use std::cell::Cell;
use std::collections::BTreeMap;

use noprop::{Ratio, TestCaseContext};
use tokiori_model::{
    AttributeType, Cardinality, EntityDef, ModelDocument, RelationDef, RoleDef, canonical_json,
    model_digest, normalize,
};

const CASES: usize = 1_000;

fn sample_identifier(ctx: &mut TestCaseContext) -> String {
    let len = noprop::sample_usize_in(ctx, 1..=8);
    let mut out = String::with_capacity(len);
    out.push(noprop::sample_choice(ctx, &['a', 'b', 'c', 'x', 'y', 'z']));
    for _ in 1..len {
        out.push(noprop::sample_choice(
            ctx,
            &['a', 'b', 'c', 'x', 'y', 'z', '0', '1', '9', '_'],
        ));
    }
    out
}

fn sample_attribute_type(ctx: &mut TestCaseContext) -> AttributeType {
    noprop::sample_choice(
        ctx,
        &[
            AttributeType::Text,
            AttributeType::Integer,
            AttributeType::Boolean,
            AttributeType::Date,
        ],
    )
}

fn sample_attributes(ctx: &mut TestCaseContext, count: usize) -> BTreeMap<String, AttributeType> {
    let mut out = BTreeMap::new();
    for _ in 0..count {
        out.insert(sample_identifier(ctx), sample_attribute_type(ctx));
    }
    out
}

/// Generates a document whose entity names are unique and whose roles only
/// reference declared entities, so the only invalid outcomes are the ones the
/// generator deliberately injects.
fn sample_valid_document(ctx: &mut TestCaseContext) -> ModelDocument {
    let entity_count = noprop::sample_usize_in(ctx, 1..=4);
    let mut entities = Vec::with_capacity(entity_count);
    for i in 0..entity_count {
        let name = format!("e{i}_{}", sample_identifier(ctx));
        let attribute_count = noprop::sample_usize_in(ctx, 0..=3);
        entities.push(EntityDef {
            name,
            attributes: sample_attributes(ctx, attribute_count),
        });
    }

    let relation_count = noprop::sample_usize_in(ctx, 0..=3);
    let mut relations = Vec::with_capacity(relation_count);
    for r in 0..relation_count {
        let role_count = noprop::sample_usize_in(ctx, 1..=3);
        let attribute_count = noprop::sample_usize_in(ctx, 0..=2);
        let mut roles = BTreeMap::new();
        for k in 0..role_count {
            let target = &entities[noprop::sample_usize_in(ctx, 0..entities.len())].name;
            roles.insert(
                format!("r{k}_{}", sample_identifier(ctx)),
                RoleDef {
                    entity: target.clone(),
                    cardinality: noprop::sample_choice(ctx, &[Cardinality::One, Cardinality::Many]),
                },
            );
        }
        relations.push(RelationDef {
            name: format!("rel{r}_{}", sample_identifier(ctx)),
            roles,
            attributes: sample_attributes(ctx, attribute_count),
        });
    }

    ModelDocument {
        model: "organization".to_owned(),
        version: noprop::sample_u32_in(ctx, 1..=9),
        time_zone: noprop::sample_choice(ctx, &["Asia/Tokyo", "UTC", "America/New_York"])
            .to_owned(),
        entities,
        relations,
    }
}

#[test]
fn a_valid_document_survives_a_serialize_parse_round_trip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("TOKIORI_PROPTEST_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(CASES, |ctx| {
        let doc = sample_valid_document(ctx);
        let model = normalize(doc.clone()).expect("generated document must be valid");

        let json = canonical_json(&model);
        let reparsed = tokiori_model::parse_model_str(&json).expect("canonical JSON must parse");
        assert_eq!(reparsed, model, "round trip changed the model\n{json}");

        let normalized_again = normalize(doc).expect("valid");
        assert_eq!(canonical_json(&normalized_again), json);
        assert_eq!(model_digest(&normalized_again), model_digest(&model));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn a_document_with_an_injected_defect_is_always_rejected() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("TOKIORI_PROPTEST_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    let rejected = Cell::new(0usize);
    let accepted = Cell::new(0usize);

    runner.run(CASES, |ctx| {
        let mut doc = sample_valid_document(ctx);
        let defect = noprop::sample_weighted_index(ctx, &[1, 1, 1, 1]);
        let mut injected = true;
        match defect {
            0 => {
                // A role that points at an entity nobody declared.
                if let Some(rel) = doc.relations.first_mut() {
                    if let Some((_, role)) = rel.roles.iter_mut().next() {
                        role.entity = "undeclared_entity".to_owned();
                    } else {
                        injected = false;
                    }
                } else {
                    injected = false;
                }
            }
            1 => {
                // A duplicate entity declaration.
                let clone = doc.entities[0].clone();
                doc.entities.push(clone);
            }
            2 => {
                // A version the engine does not accept.
                doc.version = 0;
            }
            _ => {
                // An attribute name the engine owns.
                doc.entities[0]
                    .attributes
                    .insert("scope_id".to_owned(), AttributeType::Text);
            }
        }

        if !injected {
            ctx.reject_case();
        }

        let outcome = normalize(doc);
        assert!(
            outcome.is_err(),
            "injected defect {defect} was accepted: {outcome:?}"
        );
        rejected.set(rejected.get() + 1);
        Ok(())
    })?;

    assert!(
        rejected.get() > 0,
        "coverage gate: no defective document was generated"
    );
    assert_eq!(accepted.get(), 0);
    Ok(())
}

#[test]
fn normalization_is_independent_of_declaration_order() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("TOKIORI_PROPTEST_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    let shuffled = Cell::new(0usize);

    runner.run(CASES, |ctx| {
        let doc = sample_valid_document(ctx);
        let mut reversed = doc.clone();
        reversed.entities.reverse();
        reversed.relations.reverse();
        if reversed.relations != doc.relations {
            shuffled.set(shuffled.get() + 1);
        }

        let a = normalize(doc).expect("valid");
        let b = normalize(reversed).expect("valid");
        assert_eq!(canonical_json(&a), canonical_json(&b));
        Ok(())
    })?;

    assert!(
        shuffled.get() > 0,
        "coverage gate: no document had a non-trivial declaration order"
    );
    Ok(())
}

#[test]
fn arbitrary_text_never_panics_the_parser() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("TOKIORI_PROPTEST_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    let _ = Ratio::new(1, 2);

    runner.run(CASES, |ctx| {
        let len = noprop::sample_usize_in(ctx, 0..=64);
        let text: String = noprop::sample_ascii_printable_string(ctx, len);
        let _ = tokiori_model::parse_model_str(&text);
        Ok(())
    })?;
    Ok(())
}
