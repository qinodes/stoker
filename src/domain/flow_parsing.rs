//! Parsing and timezone resolution for flow schedules.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;

use super::flow::{SchedulePeriod, SchedulePeriodUnit};

pub fn parse_once(value: &str) -> Result<DateTime<Utc>, String> {
    parse_rfc3339_instant(value, "--once-at")
}

pub fn parse_first_at(value: &str) -> Result<DateTime<Utc>, String> {
    parse_rfc3339_instant(value, "--first-at")
}

fn parse_rfc3339_instant(value: &str, option: &str) -> Result<DateTime<Utc>, String> {
    let has_offset = value.ends_with('Z')
        || value
            .rsplit_once(['+', '-'])
            .is_some_and(|(_, suffix)| suffix.contains(':'));
    if !has_offset || value.contains("-00:00") {
        return Err(format!(
            "{option} must be RFC 3339 with seconds and a UTC offset"
        ));
    }
    let seconds_part = value
        .split_once('T')
        .and_then(|(_, rest)| rest.get(0..8))
        .is_some_and(|time| {
            time.as_bytes().get(2) == Some(&b':') && time.as_bytes().get(5) == Some(&b':')
        });
    if !seconds_part {
        return Err(format!("{option} must include seconds"));
    }
    let is_leap_second = value
        .split_once('T')
        .and_then(|(_, rest)| rest.get(6..8))
        .is_some_and(|seconds| seconds == "60");
    if is_leap_second {
        return Err(format!("{option} does not accept leap-second timestamps"));
    }
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| format!("invalid {option} RFC 3339 timestamp: {error}"))
}

pub fn parse_daily(value: &str) -> Result<NaiveTime, String> {
    if value.len() != 5 || value.as_bytes().get(2) != Some(&b':') {
        return Err("--daily must use HH:mm (00:00 through 23:59)".to_owned());
    }
    NaiveTime::parse_from_str(value, "%H:%M")
        .map_err(|_| "--daily must use HH:mm (00:00 through 23:59)".to_owned())
}

pub fn parse_every(value: &str) -> Result<SchedulePeriod, String> {
    let (digits, unit) = if let Some(digits) = value.strip_suffix('m') {
        (digits, SchedulePeriodUnit::Minutes)
    } else if let Some(digits) = value.strip_suffix('h') {
        (digits, SchedulePeriodUnit::Hours)
    } else {
        return Err(
            "--every must be a positive integer followed by m or h, such as 15m or 2h".to_owned(),
        );
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(
            "--every must be a positive integer followed by m or h, such as 15m or 2h".to_owned(),
        );
    }
    let value = digits
        .parse::<u32>()
        .map_err(|_| "--every value is too large".to_owned())?;
    if value == 0 {
        let minimum = match unit {
            SchedulePeriodUnit::Minutes => "one minute",
            SchedulePeriodUnit::Hours => "one hour",
        };
        return Err(format!("--every must be at least {minimum}"));
    }
    Ok(SchedulePeriod { value, unit })
}

pub fn resolve_schedule_timezone(
    explicit: Option<&str>,
    configured: Option<&str>,
) -> Result<String, String> {
    let value = match (explicit, configured) {
        (Some(value), _) | (None, Some(value)) => value.to_owned(),
        (None, None) => iana_time_zone::get_timezone().map_err(|_| {
            "could not determine system timezone; provide --schedule-timezone".to_owned()
        })?,
    };
    value
        .parse::<Tz>()
        .map(|_| value.clone())
        .map_err(|_| format!("unknown schedule timezone {value:?}; use an IANA timezone"))
}

pub fn next_local_date_after(date: NaiveDate) -> Result<NaiveDate, String> {
    date.succ_opt()
        .ok_or_else(|| "schedule date overflow".to_owned())
}
