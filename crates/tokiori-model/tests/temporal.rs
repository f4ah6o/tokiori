//! Half-open valid intervals and business-date conversion to UTC.

use chrono::{NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use tokiori_model::{TimeError, ValidTime, ValidTimeError, business_day_start};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

#[test]
fn the_end_date_is_excluded_from_the_interval() {
    let july = ValidTime::bounded(date(2027, 7, 1), date(2027, 8, 1)).expect("non-empty");

    assert!(!july.contains(date(2027, 6, 30)));
    assert!(july.contains(date(2027, 7, 1)));
    assert!(july.contains(date(2027, 7, 31)));
    assert!(!july.contains(date(2027, 8, 1)));
}

#[test]
fn adjacent_intervals_do_not_overlap() {
    let first = ValidTime::bounded(date(2027, 4, 1), date(2027, 7, 1)).expect("non-empty");
    let second = ValidTime::bounded(date(2027, 7, 1), date(2027, 8, 1)).expect("non-empty");

    assert!(first.contains(date(2027, 6, 30)));
    assert!(!first.contains(date(2027, 7, 1)));
    assert!(second.contains(date(2027, 7, 1)));
}

#[test]
fn an_empty_interval_is_rejected() {
    let same = ValidTime::bounded(date(2027, 7, 1), date(2027, 7, 1));
    assert_eq!(
        same,
        Err(ValidTimeError::EmptyInterval {
            from: date(2027, 7, 1),
            until: date(2027, 7, 1)
        })
    );

    let inverted = ValidTime::bounded(date(2027, 7, 2), date(2027, 7, 1));
    assert!(matches!(
        inverted,
        Err(ValidTimeError::EmptyInterval { .. })
    ));
}

#[test]
fn an_unbounded_interval_contains_every_later_day() {
    let open = ValidTime::unbounded_from(date(2027, 7, 1));

    assert!(!open.contains(date(2027, 6, 30)));
    assert!(open.contains(date(2027, 7, 1)));
    assert!(open.contains(date(2099, 12, 31)));
}

#[test]
fn a_business_day_starts_at_local_midnight_in_the_model_zone() {
    let tokyo: Tz = "Asia/Tokyo".parse().expect("IANA zone");
    let start = business_day_start(date(2027, 7, 1), tokyo).expect("unambiguous midnight");

    assert_eq!(start, Utc.with_ymd_and_hms(2027, 6, 30, 15, 0, 0).unwrap());
}

#[test]
fn the_server_local_time_zone_never_affects_the_conversion() {
    let utc: Tz = "UTC".parse().expect("IANA zone");
    let start = business_day_start(date(2027, 7, 1), utc).expect("unambiguous midnight");

    assert_eq!(start, Utc.with_ymd_and_hms(2027, 7, 1, 0, 0, 0).unwrap());
}

#[test]
fn a_local_midnight_that_dst_skips_is_rejected() {
    let santiago: Tz = "America/Santiago".parse().expect("IANA zone");
    let err = business_day_start(date(2027, 9, 5), santiago)
        .expect_err("Santiago skips 00:00 on 2027-09-05");

    assert!(
        matches!(err, TimeError::NonexistentLocalTime { .. }),
        "{err:?}"
    );
}
