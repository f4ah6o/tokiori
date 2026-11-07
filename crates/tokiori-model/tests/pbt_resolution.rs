//! Property tests for [`resolve`].
//!
//! The reference below is written independently and naively: it scans every
//! version, keeps those known at the revision, and sorts them by revision. If
//! the real implementation and the reference ever disagree, one of them is
//! wrong about the boundary or the ordering.

use chrono::NaiveDate;
use noprop::TestCaseContext;
use tokiori_model::{FactVersion, Resolution, ValidTime, resolve_cloned};

const CASES: usize = 1_000;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// The obvious implementation, used only as an oracle.
fn reference(versions: &[FactVersion<u32>], valid_at: NaiveDate, known: u64) -> Resolution<u32> {
    let visible: Vec<&FactVersion<u32>> = versions
        .iter()
        .filter(|v| {
            let started = v.recorded_from <= known;
            let not_yet_replaced = match v.recorded_to {
                None => true,
                Some(to) => known <= to,
            };
            started && not_yet_replaced
        })
        .collect();

    let mut matching: Vec<&&FactVersion<u32>> = visible
        .iter()
        .filter(|v| valid_at >= v.valid.from && v.valid.until.is_none_or(|u| valid_at < u))
        .collect();
    matching.sort_by_key(|v| v.recorded_from);

    if let Some(best) = matching.last() {
        return Resolution::Found(best.value);
    }
    let earliest = versions.iter().map(|v| v.recorded_from).min();
    match earliest {
        Some(earliest) if known < earliest => Resolution::HistoryUnavailable {
            earliest_known: earliest,
        },
        None => Resolution::HistoryUnavailable { earliest_known: 0 },
        Some(_) => Resolution::NotFound,
    }
}

/// Builds a version list whose recorded intervals are consistent: a version is
/// closed exactly when a later revision records a replacement.
fn sample_versions(ctx: &mut TestCaseContext) -> Vec<FactVersion<u32>> {
    let count = noprop::sample_usize_in(ctx, 0..=6);
    let mut versions = Vec::with_capacity(count);
    let mut revision = noprop::sample_u64_in(ctx, 1..=5);
    for index in 0..count {
        let start_day = u32::try_from(noprop::sample_usize_in(ctx, 0..=28)).expect("small");
        let length = u32::try_from(noprop::sample_usize_in(ctx, 0..=10)).expect("small");
        let from = day(2027, 7, 1 + start_day.min(27));
        let valid = if length == 0 || start_day + length > 28 {
            ValidTime::unbounded_from(from)
        } else {
            ValidTime::bounded(from, day(2027, 7, 1 + start_day + length)).expect("non-empty")
        };
        versions.push(FactVersion {
            valid,
            recorded_from: revision,
            recorded_to: None,
            value: u32::try_from(index).expect("few versions"),
        });
        revision += noprop::sample_u64_in(ctx, 1..=3);
    }
    versions
}

fn sample_query(ctx: &mut TestCaseContext) -> (NaiveDate, u64) {
    let offset = u32::try_from(noprop::sample_usize_in(ctx, 0..=30)).expect("small");
    let revision = noprop::sample_u64_in(ctx, 1..=25);
    (day(2027, 7, 1 + offset.min(30)), revision)
}

#[test]
fn resolve_agrees_with_the_reference_for_any_version_list() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("TOKIORI_PROPTEST_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(CASES, |ctx| {
        let versions = sample_versions(ctx);
        let (valid_at, known) = sample_query(ctx);

        let actual = resolve_cloned(&versions, valid_at, known);
        let expected = reference(&versions, valid_at, known);
        assert_eq!(
            actual, expected,
            "versions={versions:?} valid_at={valid_at} known={known}"
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn adding_a_newer_revision_never_changes_an_older_answer() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("TOKIORI_PROPTEST_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(CASES, |ctx| {
        let mut versions = sample_versions(ctx);
        let (valid_at, _) = sample_query(ctx);

        // Record a correction that is strictly newer than anything present.
        let next = versions.iter().map(|v| v.recorded_from).max().unwrap_or(0) + 1;
        versions.push(FactVersion {
            valid: ValidTime::unbounded_from(day(2027, 7, 1)),
            recorded_from: next,
            recorded_to: None,
            value: u32::MAX,
        });

        // The claim is about revisions that predate the correction: asking at
        // or after `next` is *supposed* to see it. With no versions at all
        // there is no earlier revision to ask about.
        if next <= 1 {
            return Ok(());
        }
        let known = noprop::sample_u64_in(ctx, 1..next);
        let before = resolve_cloned(&versions[..versions.len() - 1], valid_at, known);
        let after = resolve_cloned(&versions, valid_at, known);
        assert_eq!(
            before, after,
            "a revision recorded at {next} changed the answer at {known}"
        );
        Ok(())
    })?;
    Ok(())
}
