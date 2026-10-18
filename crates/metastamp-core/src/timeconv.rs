//! Time zone conversion and W3CDTF formatting.
//!
//! BulkFileChanger converts the user's local time with
//! `TzSpecificLocalTimeToSystemTime`, i.e. with the UTC offset in force on the
//! *target* date. `chrono`'s `from_local_datetime` gives the same behaviour,
//! so a summer date entered in winter is still converted correctly.

use std::time::SystemTime;

use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};

use crate::error::{Error, Result};

/// Converts a wall-clock time in `tz` to UTC.
///
/// When the time is ambiguous (the hour repeated when clocks go back) the
/// earlier instant is used. Times skipped when clocks go forward are rejected.
pub fn to_utc<Tz: TimeZone>(tz: &Tz, local: NaiveDateTime) -> Result<DateTime<Utc>> {
    match tz.from_local_datetime(&local) {
        LocalResult::Single(dt) => Ok(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(earliest, _) => Ok(earliest.with_timezone(&Utc)),
        LocalResult::None => Err(Error::NonexistentLocalTime(local)),
    }
}

/// Converts a wall-clock time in the system's local time zone to UTC.
pub fn local_to_utc(local: NaiveDateTime) -> Result<DateTime<Utc>> {
    to_utc(&chrono::Local, local)
}

/// Formats a timestamp the way Office writes `dcterms:created` and
/// `dcterms:modified`: `YYYY-MM-DDTHH:MM:SSZ`, whole seconds, UTC.
pub fn to_w3cdtf(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Parses the W3CDTF profiles found in real `core.xml` files.
///
/// Accepts full timestamps with or without a zone (no zone means UTC) and
/// plain dates. Returns `None` for anything else.
pub fn parse_w3cdtf(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Some(dt.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(value, format) {
            return Some(naive.and_utc());
        }
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|naive| naive.and_utc())
}

pub fn to_system_time(dt: DateTime<Utc>) -> SystemTime {
    dt.into()
}

pub fn from_system_time(time: SystemTime) -> DateTime<Utc> {
    time.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use chrono_tz::Europe::Rome;

    fn naive(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    #[test]
    fn uses_the_offset_of_the_target_date() {
        // Rome is UTC+1 in winter and UTC+2 in summer.
        let winter = to_utc(&Rome, naive(2024, 1, 15, 12, 0)).unwrap();
        let summer = to_utc(&Rome, naive(2024, 7, 15, 12, 0)).unwrap();
        assert_eq!(to_w3cdtf(&winter), "2024-01-15T11:00:00Z");
        assert_eq!(to_w3cdtf(&summer), "2024-07-15T10:00:00Z");
    }

    #[test]
    fn ambiguous_time_picks_the_earlier_instant() {
        // 2024-10-27 02:30 happens twice in Rome.
        let dt = to_utc(&Rome, naive(2024, 10, 27, 2, 30)).unwrap();
        assert_eq!(to_w3cdtf(&dt), "2024-10-27T00:30:00Z");
    }

    #[test]
    fn nonexistent_time_is_rejected() {
        // 2024-03-31 02:30 is skipped in Rome.
        let err = to_utc(&Rome, naive(2024, 3, 31, 2, 30)).unwrap_err();
        assert!(matches!(err, Error::NonexistentLocalTime(_)));
    }

    #[test]
    fn parses_w3cdtf_variants() {
        let expected = "2023-05-06T07:08:09Z";
        for input in [
            "2023-05-06T07:08:09Z",
            "2023-05-06T09:08:09+02:00",
            "2023-05-06T07:08:09.000Z",
            "2023-05-06T07:08:09",
        ] {
            assert_eq!(
                to_w3cdtf(&parse_w3cdtf(input).unwrap()),
                expected,
                "{input}"
            );
        }
        assert_eq!(
            to_w3cdtf(&parse_w3cdtf("2023-05-06").unwrap()),
            "2023-05-06T00:00:00Z"
        );
        assert!(parse_w3cdtf("yesterday").is_none());
    }

    #[test]
    fn system_time_round_trip() {
        let dt = parse_w3cdtf("2001-02-03T04:05:06Z").unwrap();
        assert_eq!(from_system_time(to_system_time(dt)), dt);
    }
}
