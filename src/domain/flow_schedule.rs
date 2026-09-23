use std::fmt;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use super::flow::{DailyOccurrence, ScheduleSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SchedulePeriodUnit {
    Minutes,
    Hours,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulePeriod {
    pub value: u32,
    pub unit: SchedulePeriodUnit,
}

impl SchedulePeriod {
    pub fn duration(self) -> Result<Duration, String> {
        if self.value == 0 {
            return Err("schedule period must be at least one minute or one hour".to_owned());
        }
        let seconds_per_unit = match self.unit {
            SchedulePeriodUnit::Minutes => 60_i64,
            SchedulePeriodUnit::Hours => 3_600_i64,
        };
        let seconds = i64::from(self.value)
            .checked_mul(seconds_per_unit)
            .ok_or_else(|| "schedule period is too large".to_owned())?;
        Ok(Duration::seconds(seconds))
    }
}

impl fmt::Display for SchedulePeriod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let suffix = match self.unit {
            SchedulePeriodUnit::Minutes => 'm',
            SchedulePeriodUnit::Hours => 'h',
        };
        write!(f, "{}{suffix}", self.value)
    }
}

impl ScheduleSpec {
    pub fn timezone(&self) -> Option<&str> {
        match self {
            Self::Once { .. } | Self::Periodic { .. } => None,
            Self::Daily { timezone, .. } => Some(timezone),
        }
    }

    pub fn due_at(&self, date: NaiveDate) -> Result<Option<DateTime<Utc>>, String> {
        match self {
            Self::Once { at } => Ok(Some(*at)),
            Self::Daily { time, timezone } => {
                let zone: Tz = timezone
                    .parse()
                    .map_err(|_| format!("unknown timezone {timezone:?}"))?;
                let local = zone.from_local_datetime(&date.and_time(*time));
                Ok(match local {
                    chrono::LocalResult::None => None,
                    chrono::LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
                    chrono::LocalResult::Ambiguous(earlier, _) => Some(earlier.with_timezone(&Utc)),
                })
            }
            Self::Periodic { .. } => {
                Err("calendar-date resolution requires a once or daily schedule".to_owned())
            }
        }
    }

    pub fn next_daily_after(&self, instant: DateTime<Utc>) -> Result<DailyOccurrence, String> {
        let Self::Daily { time, timezone } = self else {
            return Err("next daily occurrence requires a daily schedule".to_owned());
        };
        let zone: Tz = timezone
            .parse()
            .map_err(|_| format!("unknown timezone {timezone:?}"))?;
        let local = instant.with_timezone(&zone);
        let mut date = local.date_naive();
        if local.time() >= *time {
            date = date
                .succ_opt()
                .ok_or_else(|| "daily schedule date overflow".to_owned())?;
        }
        for _ in 0..370 {
            if let Some(due_at) = self.due_at(date)? {
                return Ok(DailyOccurrence {
                    local_date: date,
                    due_at,
                });
            }
            date = date
                .succ_opt()
                .ok_or_else(|| "daily schedule date overflow".to_owned())?;
        }
        Err("daily schedule has no resolvable occurrence in the next year".to_owned())
    }

    pub fn periodic_window(
        &self,
        instant: DateTime<Utc>,
    ) -> Result<(Option<DateTime<Utc>>, DateTime<Utc>), String> {
        let Self::Periodic { every, first_at } = self else {
            return Err("periodic window requires an every schedule".to_owned());
        };
        let first_at =
            first_at.ok_or_else(|| "periodic schedule has not been activated yet".to_owned())?;
        if instant < first_at {
            return Ok((None, first_at));
        }
        let period = every.duration()?;
        let period_seconds = period.num_seconds();
        let elapsed_seconds = instant.signed_duration_since(first_at).num_seconds();
        let index = elapsed_seconds / period_seconds;
        let offset_seconds = index
            .checked_mul(period_seconds)
            .ok_or_else(|| "periodic schedule calculation overflow".to_owned())?;
        let current = first_at
            .checked_add_signed(chrono::Duration::seconds(offset_seconds))
            .ok_or_else(|| "periodic schedule timestamp overflow".to_owned())?;
        let next = current
            .checked_add_signed(period)
            .ok_or_else(|| "periodic schedule timestamp overflow".to_owned())?;
        Ok((Some(current), next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    #[test]
    fn calendar_resolution_rejects_incompatible_schedules_and_handles_dst_gap() {
        let at = Utc.with_ymd_and_hms(2026, 3, 8, 7, 30, 0).unwrap();
        let once = ScheduleSpec::Once { at };
        let periodic = ScheduleSpec::Periodic {
            every: SchedulePeriod {
                value: 1,
                unit: SchedulePeriodUnit::Hours,
            },
            first_at: None,
        };
        assert_eq!(once.timezone(), None);
        assert_eq!(once.due_at(at.date_naive()).unwrap(), Some(at));
        assert!(once.next_daily_after(at).unwrap_err().contains("daily"));
        assert!(
            periodic
                .due_at(at.date_naive())
                .unwrap_err()
                .contains("once or daily")
        );
        assert!(
            periodic
                .periodic_window(at)
                .unwrap_err()
                .contains("activated")
        );
        assert!(
            once.periodic_window(at)
                .unwrap_err()
                .contains("every schedule")
        );

        let daily = ScheduleSpec::Daily {
            time: NaiveTime::from_hms_opt(2, 30, 0).unwrap(),
            timezone: "America/New_York".into(),
        };
        assert_eq!(daily.timezone(), Some("America/New_York"));
        assert_eq!(
            daily
                .due_at(chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap())
                .unwrap(),
            None
        );
        let next = daily.next_daily_after(at - Duration::seconds(1)).unwrap();
        assert_eq!(
            next.local_date,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 9).unwrap()
        );
        let invalid = ScheduleSpec::Daily {
            time: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            timezone: "Nowhere/Invalid".into(),
        };
        assert!(
            invalid
                .due_at(at.date_naive())
                .unwrap_err()
                .contains("unknown timezone")
        );
        assert!(
            invalid
                .next_daily_after(at)
                .unwrap_err()
                .contains("unknown timezone")
        );
    }
}
