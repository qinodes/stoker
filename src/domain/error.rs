use std::fmt;

/// Stable machine-readable categories for domain validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainErrorCode {
    EmptyValue,
    ValueTooLong,
    UnknownJobState,
}

/// User-controlled fields governed by domain validation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationField {
    JobName,
    JobUser,
    Description,
}

impl ValidationField {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::JobName => "name",
            Self::JobUser => "user",
            Self::Description => "description",
        }
    }
}

impl fmt::Display for ValidationField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Framework-neutral domain validation error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("{field} must not be empty")]
    EmptyValue { field: ValidationField },
    #[error("{field} must be {max} characters or fewer")]
    ValueTooLong {
        field: ValidationField,
        max: usize,
        actual: usize,
    },
    #[error("unknown job state {value:?}")]
    UnknownJobState { value: String },
}

impl DomainError {
    pub const fn code(&self) -> DomainErrorCode {
        match self {
            Self::EmptyValue { .. } => DomainErrorCode::EmptyValue,
            Self::ValueTooLong { .. } => DomainErrorCode::ValueTooLong,
            Self::UnknownJobState { .. } => DomainErrorCode::UnknownJobState,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DomainError, DomainErrorCode, ValidationField};

    #[test]
    fn every_domain_error_has_a_stable_code_and_message() {
        let cases = [
            (
                DomainError::EmptyValue {
                    field: ValidationField::JobName,
                },
                DomainErrorCode::EmptyValue,
                "name must not be empty",
            ),
            (
                DomainError::ValueTooLong {
                    field: ValidationField::Description,
                    max: 200,
                    actual: 201,
                },
                DomainErrorCode::ValueTooLong,
                "description must be 200 characters or fewer",
            ),
            (
                DomainError::UnknownJobState {
                    value: "paused".to_owned(),
                },
                DomainErrorCode::UnknownJobState,
                "unknown job state \"paused\"",
            ),
        ];

        for (error, code, message) in cases {
            assert_eq!(error.code(), code);
            assert_eq!(error.to_string(), message);
        }
    }
}
