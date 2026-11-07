//! Resolving a fact from its recorded versions.
//!
//! Two time axes meet here: the date a fact was in effect (`valid_at`) and the
//! revision at which the system knew about it (`known_revision`).

use chrono::NaiveDate;
use tokiori_model::{FactVersion, Resolution, resolve_cloned as resolve};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

fn version(
    value: &str,
    from: (i32, u32, u32),
    until: Option<(i32, u32, u32)>,
    recorded: (u64, Option<u64>),
) -> FactVersion<String> {
    FactVersion {
        valid: match until {
            None => tokiori_model::ValidTime::unbounded_from(day(from.0, from.1, from.2)),
            Some(u) => {
                tokiori_model::ValidTime::bounded(day(from.0, from.1, from.2), day(u.0, u.1, u.2))
                    .expect("non-empty")
            }
        },
        recorded_from: recorded.0,
        recorded_to: recorded.1,
        value: value.to_owned(),
    }
}

#[test]
fn a_single_open_version_is_found_at_any_later_date() {
    let versions = vec![version("sales", (2027, 4, 1), None, (10, None))];

    assert_eq!(
        resolve(&versions, day(2027, 7, 10), 10),
        Resolution::Found("sales".to_owned())
    );
    assert_eq!(
        resolve(&versions, day(2027, 4, 1), 10),
        Resolution::Found("sales".to_owned())
    );
}

#[test]
fn a_date_before_the_first_version_is_not_found() {
    let versions = vec![version("sales", (2027, 4, 1), None, (10, None))];

    assert_eq!(
        resolve(&versions, day(2027, 3, 31), 10),
        Resolution::NotFound
    );
}

#[test]
fn the_end_date_belongs_to_the_next_version() {
    let versions = vec![
        version("sales", (2027, 4, 1), Some((2027, 7, 1)), (10, None)),
        version("planning", (2027, 7, 1), None, (10, None)),
    ];

    assert_eq!(
        resolve(&versions, day(2027, 6, 30), 10),
        Resolution::Found("sales".to_owned())
    );
    assert_eq!(
        resolve(&versions, day(2027, 7, 1), 10),
        Resolution::Found("planning".to_owned())
    );
}

#[test]
fn a_later_revision_does_not_change_an_earlier_answer() {
    let versions = vec![
        version("sales", (2027, 4, 1), Some((2027, 7, 1)), (10, Some(11))),
        version("planning", (2027, 7, 1), None, (10, Some(11))),
        // r11 corrects the effective date of the move to 7/15.
        version("sales", (2027, 4, 1), Some((2027, 7, 15)), (11, None)),
        version("planning", (2027, 7, 15), None, (11, None)),
    ];

    assert_eq!(
        resolve(&versions, day(2027, 7, 10), 10),
        Resolution::Found("planning".to_owned()),
        "r10 believed the move took effect on 7/1"
    );
    assert_eq!(
        resolve(&versions, day(2027, 7, 10), 11),
        Resolution::Found("sales".to_owned()),
        "r11 knows the move only took effect on 7/15"
    );
    assert_eq!(
        resolve(&versions, day(2027, 7, 16), 10),
        Resolution::Found("planning".to_owned())
    );
    assert_eq!(
        resolve(&versions, day(2027, 7, 16), 11),
        Resolution::Found("planning".to_owned())
    );
}

#[test]
fn a_revision_before_the_first_recorded_version_is_history_unavailable() {
    let versions = vec![version("sales", (2027, 4, 1), None, (10, None))];

    assert_eq!(
        resolve(&versions, day(2027, 7, 10), 9),
        Resolution::HistoryUnavailable { earliest_known: 10 },
        "we do not know what the system believed before r10"
    );
}

#[test]
fn no_version_at_all_is_history_unavailable_not_not_found() {
    let versions: Vec<FactVersion<String>> = Vec::new();

    assert_eq!(
        resolve(&versions, day(2027, 7, 10), 5),
        Resolution::HistoryUnavailable { earliest_known: 0 }
    );
}

#[test]
fn a_revision_after_the_last_recorded_change_still_resolves() {
    let versions = vec![version("sales", (2027, 4, 1), None, (10, None))];

    assert_eq!(
        resolve(&versions, day(2027, 7, 10), 99),
        Resolution::Found("sales".to_owned())
    );
}

#[test]
fn a_version_replaced_at_a_later_revision_is_not_visible_after_it() {
    let versions = vec![
        version("sales", (2027, 4, 1), Some((2027, 7, 1)), (10, Some(12))),
        version("planning", (2027, 7, 1), None, (12, None)),
    ];

    assert_eq!(
        resolve(&versions, day(2027, 7, 10), 11),
        Resolution::NotFound,
        "r11 closed the sales version without recording a replacement"
    );
    assert_eq!(
        resolve(&versions, day(2027, 7, 10), 12),
        Resolution::Found("planning".to_owned()),
        "r12 recorded the replacement, so the query at r12 sees it"
    );
}

#[test]
fn a_gap_between_versions_is_not_found() {
    let versions = vec![
        version("sales", (2027, 4, 1), Some((2027, 6, 1)), (10, None)),
        version("planning", (2027, 8, 1), None, (10, None)),
    ];

    assert_eq!(
        resolve(&versions, day(2027, 7, 1), 10),
        Resolution::NotFound
    );
}

#[test]
fn a_tombstone_leaves_the_earlier_period_resolvable() {
    let versions = vec![version(
        "sales",
        (2027, 4, 1),
        Some((2027, 7, 1)),
        (10, None),
    )];

    assert_eq!(
        resolve(&versions, day(2027, 6, 1), 10),
        Resolution::Found("sales".to_owned())
    );
    assert_eq!(
        resolve(&versions, day(2027, 7, 1), 10),
        Resolution::NotFound
    );
}
