//! Validation and SQLite column mapping for flow schedules.

use chrono::{DateTime, Utc};

use crate::domain::flow::{ExecutionMode, SchedulePeriodUnit, ScheduleSpec};

use super::error::StoreError;

pub(super) fn validate_schedule(
    mode: ExecutionMode,
    schedule: Option<&ScheduleSpec>,
) -> Result<(), StoreError> {
    match (mode, schedule) {
        (ExecutionMode::Serial, Some(_)) => Err(StoreError::InvalidData(
            "serial definitions cannot have --once-at, --daily, --every, --first-at, or --schedule-timezone".into(),
        )),
        (ExecutionMode::Scheduled, None) => Err(StoreError::InvalidData(
            "scheduled definitions require --once-at, --daily, or --every".into(),
        )),
        (ExecutionMode::Scheduled, Some(ScheduleSpec::Daily { timezone, .. })) => {
            timezone
                .parse::<chrono_tz::Tz>()
                .map_err(|_| StoreError::InvalidData(format!("unknown timezone {timezone:?}")))?;
            Ok(())
        }
        (ExecutionMode::Scheduled, Some(ScheduleSpec::Periodic { every, .. })) => {
            if every.value == 0 {
                return Err(StoreError::InvalidData(
                    "periodic schedule must be at least one minute or one hour".into(),
                ));
            }
            every.duration().map_err(StoreError::InvalidData)?;
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(super) struct ScheduleColumns {
    pub kind: Option<&'static str>,
    pub at: Option<String>,
    pub daily: Option<String>,
    pub timezone: Option<String>,
    pub period_value: Option<i64>,
    pub period_unit: Option<&'static str>,
    pub period_first_at: Option<String>,
}

pub(super) fn schedule_columns(schedule: Option<&ScheduleSpec>) -> ScheduleColumns {
    match schedule {
        None => ScheduleColumns {
            kind: None,
            at: None,
            daily: None,
            timezone: None,
            period_value: None,
            period_unit: None,
            period_first_at: None,
        },
        Some(ScheduleSpec::Once { at }) => ScheduleColumns {
            kind: Some("once"),
            at: Some(at.to_rfc3339()),
            daily: None,
            timezone: None,
            period_value: None,
            period_unit: None,
            period_first_at: None,
        },
        Some(ScheduleSpec::Daily { time, timezone }) => ScheduleColumns {
            kind: Some("daily"),
            at: None,
            daily: Some(time.format("%H:%M").to_string()),
            timezone: Some(timezone.clone()),
            period_value: None,
            period_unit: None,
            period_first_at: None,
        },
        Some(ScheduleSpec::Periodic { every, first_at }) => ScheduleColumns {
            kind: Some("periodic"),
            at: None,
            daily: None,
            timezone: None,
            period_value: Some(i64::from(every.value)),
            period_unit: Some(match every.unit {
                SchedulePeriodUnit::Minutes => "minutes",
                SchedulePeriodUnit::Hours => "hours",
            }),
            period_first_at: first_at.map(|value| value.to_rfc3339()),
        },
    }
}

pub(super) fn activate_periodic_schedule(
    schedule: &mut Option<ScheduleSpec>,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    let Some(ScheduleSpec::Periodic { every, first_at }) = schedule else {
        return Ok(());
    };
    every.duration().map_err(StoreError::InvalidData)?;
    let activated = match first_at {
        Some(value) if *value <= now => {
            return Err(StoreError::InvalidData(
                "--first-at must be in the future when the schedule is committed or applied".into(),
            ));
        }
        Some(value) => *value,
        None => now
            .checked_add_signed(every.duration().map_err(StoreError::InvalidData)?)
            .ok_or_else(|| StoreError::InvalidData("periodic first occurrence overflow".into()))?,
    };
    *first_at = Some(activated);
    Ok(())
}
