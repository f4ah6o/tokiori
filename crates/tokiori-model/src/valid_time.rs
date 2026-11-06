use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A half-open effective interval, from the start date up to but excluding the
/// end date.
///
/// An absent end date means unbounded, which is how an open-ended fact is
/// stored. The half-open convention is what makes "the old membership ends on
/// July 1 and the new one starts on July 1" expressible without overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidTime {
    pub from: NaiveDate,
    pub until: Option<NaiveDate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ValidTimeError {
    #[error(
        "valid interval starting {from} and ending {until} is empty; the end must be after the start"
    )]
    EmptyInterval { from: NaiveDate, until: NaiveDate },
}

impl ValidTime {
    #[must_use]
    pub fn unbounded_from(from: NaiveDate) -> Self {
        Self { from, until: None }
    }

    pub fn bounded(from: NaiveDate, until: NaiveDate) -> Result<Self, ValidTimeError> {
        if until <= from {
            return Err(ValidTimeError::EmptyInterval { from, until });
        }
        Ok(Self {
            from,
            until: Some(until),
        })
    }

    /// Whether the day falls inside the interval. The end date is excluded.
    #[must_use]
    pub fn contains(&self, day: NaiveDate) -> bool {
        day >= self.from && self.until.is_none_or(|until| day < until)
    }
}
