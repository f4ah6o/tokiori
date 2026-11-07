use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use thiserror::Error;

/// A business date cannot be placed on the UTC timeline unambiguously.
///
/// Daylight-saving transitions make some local midnights nonexistent or
/// ambiguous. Tokiori refuses to guess: a shifted or duplicated instant would
/// silently move a business event to another day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TimeError {
    #[error("local midnight of {date} does not exist in {time_zone}")]
    NonexistentLocalTime { date: NaiveDate, time_zone: Tz },
    #[error("local midnight of {date} is ambiguous in {time_zone}")]
    AmbiguousLocalTime { date: NaiveDate, time_zone: Tz },
}

/// Converts a business date to the UTC instant at which that day starts in the
/// model's time zone.
///
/// The conversion always uses the declared IANA zone, never the server's local
/// time, so the same model yields the same instants on every host.
pub fn business_day_start(day: NaiveDate, time_zone: Tz) -> Result<DateTime<Utc>, TimeError> {
    let naive: NaiveDateTime = day
        .and_hms_opt(0, 0, 0)
        .expect("midnight always exists for a valid date");
    match time_zone.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Ok(dt.with_timezone(&Utc)),
        LocalResult::None => Err(TimeError::NonexistentLocalTime {
            date: day,
            time_zone,
        }),
        LocalResult::Ambiguous(_, _) => Err(TimeError::AmbiguousLocalTime {
            date: day,
            time_zone,
        }),
    }
}
