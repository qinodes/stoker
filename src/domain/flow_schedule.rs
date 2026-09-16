use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;

use super::flow::{DailyOccurrence, ScheduleSpec};

impl ScheduleSpec {
    pub fn timezone(&self) -> Option<&str> {
        match self {
            Self::Once { .. } => None,
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
}
