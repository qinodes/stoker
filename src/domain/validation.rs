use super::error::{DomainError, ValidationField};
use super::job::JobState;

/// Maximum number of Unicode characters allowed in a user-provided job name.
pub const MAX_JOB_NAME_LENGTH: usize = 128;
pub const MAX_JOB_USER_LENGTH: usize = 50;
pub const MAX_JOB_DESCRIPTION_LENGTH: usize = 200;

pub fn validate_job_name(name: &str) -> Result<(), DomainError> {
    validate_required(name, ValidationField::JobName, MAX_JOB_NAME_LENGTH)
}

pub fn validate_job_user(user: &str) -> Result<(), DomainError> {
    validate_required(user, ValidationField::JobUser, MAX_JOB_USER_LENGTH)
}

pub fn validate_description(description: Option<&str>) -> Result<(), DomainError> {
    let Some(description) = description else {
        return Ok(());
    };
    validate_length(
        description,
        ValidationField::Description,
        MAX_JOB_DESCRIPTION_LENGTH,
    )
}

pub fn normalize_description(description: Option<String>) -> Option<String> {
    description.filter(|value| !value.trim().is_empty())
}

pub fn parse_job_state(value: &str) -> Result<JobState, DomainError> {
    match value {
        "DRAFT" => Ok(JobState::Draft),
        "QUEUED" => Ok(JobState::Queued),
        "STARTING" => Ok(JobState::Starting),
        "RUNNING" => Ok(JobState::Running),
        "CANCELLING" => Ok(JobState::Cancelling),
        "SUCCEEDED" => Ok(JobState::Succeeded),
        "FAILED" => Ok(JobState::Failed),
        "CANCELLED" => Ok(JobState::Cancelled),
        "LOST" => Ok(JobState::Lost),
        _ => Err(DomainError::UnknownJobState {
            value: value.to_owned(),
        }),
    }
}

fn validate_required(value: &str, field: ValidationField, max: usize) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::EmptyValue { field });
    }
    validate_length(value, field, max)
}

fn validate_length(value: &str, field: ValidationField, max: usize) -> Result<(), DomainError> {
    let actual = value.chars().count();
    if actual > max {
        return Err(DomainError::ValueTooLong { field, max, actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DomainErrorCode, ValidationField};

    #[test]
    fn required_fields_cover_empty_limit_unicode_and_too_long_paths() {
        assert_eq!(
            validate_job_name(" \t").unwrap_err(),
            DomainError::EmptyValue {
                field: ValidationField::JobName
            }
        );
        assert_eq!(
            validate_job_user("").unwrap_err(),
            DomainError::EmptyValue {
                field: ValidationField::JobUser
            }
        );
        assert!(validate_job_name(&"🙂".repeat(MAX_JOB_NAME_LENGTH)).is_ok());
        let error = validate_job_name(&"🙂".repeat(MAX_JOB_NAME_LENGTH + 1)).unwrap_err();
        assert_eq!(error.code(), DomainErrorCode::ValueTooLong);
        assert!(matches!(
            error,
            DomainError::ValueTooLong {
                field: ValidationField::JobName,
                max: MAX_JOB_NAME_LENGTH,
                actual
            } if actual == MAX_JOB_NAME_LENGTH + 1
        ));
    }

    #[test]
    fn optional_description_covers_none_blank_limit_and_too_long_paths() {
        assert!(validate_description(None).is_ok());
        assert!(validate_description(Some("")).is_ok());
        assert!(validate_description(Some(&"🙂".repeat(MAX_JOB_DESCRIPTION_LENGTH))).is_ok());
        assert!(matches!(
            validate_description(Some(&"x".repeat(MAX_JOB_DESCRIPTION_LENGTH + 1))),
            Err(DomainError::ValueTooLong {
                field: ValidationField::Description,
                ..
            })
        ));
        assert_eq!(normalize_description(Some("   ".to_owned())), None);
        assert_eq!(
            normalize_description(Some(" text ".to_owned())),
            Some(" text ".to_owned())
        );
    }

    #[test]
    fn typed_job_state_parser_covers_every_state_and_unknown_values() {
        for state in [
            JobState::Draft,
            JobState::Queued,
            JobState::Starting,
            JobState::Running,
            JobState::Cancelling,
            JobState::Succeeded,
            JobState::Failed,
            JobState::Cancelled,
            JobState::Lost,
        ] {
            assert_eq!(parse_job_state(state.as_str()).unwrap(), state);
        }
        assert_eq!(
            parse_job_state("paused").unwrap_err(),
            DomainError::UnknownJobState {
                value: "paused".to_owned()
            }
        );
    }
}
