use anyhow::Context;
use chrono_tz::Tz;

use super::model::{ResolvedTimezone, StokerConfig, TimezoneSource};
use super::paths::StokerPaths;
use super::repository;

pub fn system_timezone_name() -> anyhow::Result<String> {
    iana_time_zone::get_timezone().context("detect operating system timezone")
}

pub fn resolve_timezone(
    paths: &StokerPaths,
    cli_timezone: Option<&str>,
) -> anyhow::Result<ResolvedTimezone> {
    let (name, source) = if let Some(value) = cli_timezone {
        (value.to_owned(), TimezoneSource::Cli)
    } else if let Some(value) = repository::read(paths)?.timezone {
        (value, TimezoneSource::Config)
    } else {
        (system_timezone_name()?, TimezoneSource::System)
    };
    let timezone = parse_timezone(&name)?;
    Ok(ResolvedTimezone {
        name,
        source,
        timezone,
    })
}

pub(super) fn validate_config(config: &StokerConfig) -> anyhow::Result<()> {
    if let Some(value) = &config.timezone {
        parse_timezone(value)?;
    }
    Ok(())
}

fn parse_timezone(value: &str) -> anyhow::Result<Tz> {
    value.parse::<Tz>().map_err(|_| {
        anyhow::anyhow!(
            "unknown timezone {value:?}; use an IANA timezone such as Asia/Taipei or UTC"
        )
    })
}
