//! Resolving a fact from its recorded versions.
//!
//! A fact is not overwritten when it changes. Each change records a new version
//! that is valid for some business period and known from some revision onward.
//! Answering a query means choosing, among those versions, the one that was both
//! in effect on the requested date and known at the requested revision.
//!
//! The function here is pure so the same code answers queries in PostgreSQL and
//! in the property tests that compare it against a naive reference.

use chrono::NaiveDate;

use crate::valid_time::ValidTime;

/// One recorded version of a fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactVersion<V> {
    /// The business period in which this value was in effect.
    pub valid: ValidTime,
    /// First revision at which this version was the current knowledge.
    pub recorded_from: u64,
    /// Revision at which a newer version replaced it; `None` while current.
    pub recorded_to: Option<u64>,
    pub value: V,
}

/// What a query learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution<V> {
    /// The fact held this value on the requested date, as of that revision.
    Found(V),
    /// The fact was known not to exist on that date at that revision.
    NotFound,
    /// The requested revision predates anything recorded for this fact.
    ///
    /// This is not the same as [`Resolution::NotFound`]: the system did not
    /// know, which is a different answer from knowing it did not exist.
    HistoryUnavailable { earliest_known: u64 },
}

/// Chooses the version that was in effect on `valid_at` and known at
/// `known_revision`.
///
/// A version is a candidate when `recorded_from <= known_revision` and
/// `recorded_to` is either absent or not earlier than `known_revision`.
/// `recorded_to` is the revision that *replaced* the version, so a query at
/// exactly that revision still sees the replacement, not a gap. Among
/// candidates, the one whose valid interval contains `valid_at` wins.
///
/// The value is returned by reference: callers that need an owned value clone
/// it, and callers that only inspect it pay nothing.
#[must_use]
pub fn resolve<V>(
    versions: &[FactVersion<V>],
    valid_at: NaiveDate,
    known_revision: u64,
) -> Resolution<&V> {
    let earliest_known = versions.iter().map(|v| v.recorded_from).min();

    // A version closed at `recorded_to` is still the knowledge *at* that
    // revision: the closing revision is the one that replaced it, and it
    // records its own version. So the window is inclusive at both ends.
    let candidates = versions.iter().filter(|version| {
        version.recorded_from <= known_revision
            && version.recorded_to.is_none_or(|to| known_revision <= to)
            && version.valid.contains(valid_at)
    });

    // Several revisions can describe the same date (that is what a correction
    // is). The newest knowledge wins, not whichever happens to come first.
    let best = candidates.max_by_key(|version| version.recorded_from);
    if let Some(version) = best {
        return Resolution::Found(&version.value);
    }

    match earliest_known {
        Some(earliest) if known_revision < earliest => Resolution::HistoryUnavailable {
            earliest_known: earliest,
        },
        None => Resolution::HistoryUnavailable { earliest_known: 0 },
        Some(_) => Resolution::NotFound,
    }
}

/// [`resolve`], returning an owned copy of the value.
#[must_use]
pub fn resolve_cloned<V: Clone>(
    versions: &[FactVersion<V>],
    valid_at: NaiveDate,
    known_revision: u64,
) -> Resolution<V> {
    match resolve(versions, valid_at, known_revision) {
        Resolution::Found(value) => Resolution::Found(value.clone()),
        Resolution::NotFound => Resolution::NotFound,
        Resolution::HistoryUnavailable { earliest_known } => {
            Resolution::HistoryUnavailable { earliest_known }
        }
    }
}
