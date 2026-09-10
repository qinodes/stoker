//! Domain compatibility façade.
//!
//! Existing callers may continue importing `stoker::domain::*`. New core code
//! should use the typed validation API in [`validation`] and [`DomainError`].

pub mod error;
pub mod job;
pub mod validation;

pub use error::{DomainError, DomainErrorCode, ValidationField};
pub use job::{Job, JobState, NewJob};
pub use validation::{
    MAX_JOB_DESCRIPTION_LENGTH, MAX_JOB_NAME_LENGTH, MAX_JOB_USER_LENGTH, normalize_description,
    parse_job_state,
};

/// Compatibility wrapper preserving the pre-refactor `Result<(), String>` API.
pub fn validate_job_name(name: &str) -> Result<(), String> {
    validation::validate_job_name(name).map_err(|error| error.to_string())
}

/// Compatibility wrapper preserving the pre-refactor `Result<(), String>` API.
pub fn validate_job_user(user: &str) -> Result<(), String> {
    validation::validate_job_user(user).map_err(|error| error.to_string())
}

/// Compatibility wrapper preserving the pre-refactor `Result<(), String>` API.
pub fn validate_description(description: Option<&str>) -> Result<(), String> {
    validation::validate_description(description).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{validate_description, validate_job_name, validate_job_user};

    #[test]
    fn compatibility_validation_facade_keeps_string_errors() {
        assert_eq!(
            validate_job_name(" ").unwrap_err(),
            "name must not be empty"
        );
        assert_eq!(
            validate_job_user(" ").unwrap_err(),
            "user must not be empty"
        );
        assert_eq!(
            validate_description(Some(&"x".repeat(201))).unwrap_err(),
            "description must be 200 characters or fewer"
        );
    }
}
