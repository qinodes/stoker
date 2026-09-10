//! Shared job submission validation used by the CLI and browser UI.

use std::path::PathBuf;

use crate::adapters::SystemWorkingDirectoryResolver;
use crate::application::jobs;
use crate::application::ports::WorkingDirectoryResolver;
use crate::application::{ApplicationError, CreateJobInput};
use crate::domain::Job;
use crate::store::Store;

/// Create a DRAFT shell job after applying the same validation used by both
/// submission surfaces. The original command line is preserved for display
/// and backwards-compatible execution semantics.
pub fn create_shell_job(
    store: &Store,
    user: String,
    name: String,
    cwd: PathBuf,
    command_line: String,
    description: Option<String>,
) -> anyhow::Result<Job> {
    jobs::create_job(
        store,
        &SystemWorkingDirectoryResolver,
        CreateJobInput {
            user,
            name,
            description,
            cwd,
            command_line,
        },
    )
    .map_err(cli_submission_error)
}

/// Resolve and validate a working directory immediately before persistence.
/// This prevents a browser selection from becoming stale between browsing and
/// submission, while keeping the stored path usable by command interpreters.
pub fn resolve_working_directory(path: PathBuf) -> anyhow::Result<PathBuf> {
    SystemWorkingDirectoryResolver
        .resolve_working_directory(&path)
        .map_err(|message| anyhow::anyhow!("working directory {}: {message}", path.display()))
}

/// Parse the shell-like command syntax accepted by `stoker add --cmd`.
pub fn parse_command_line(input: &str) -> anyhow::Result<Vec<String>> {
    jobs::parse_command_line(input).map_err(cli_submission_error)
}

fn cli_submission_error(error: ApplicationError) -> anyhow::Error {
    match error {
        ApplicationError::InvalidInput(error) => anyhow::anyhow!("--{error}"),
        ApplicationError::InvalidCommand { message } => anyhow::anyhow!("--cmd {message}"),
        other => anyhow::Error::new(other),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::parse_command_line;
    use crate::domain::{
        MAX_JOB_DESCRIPTION_LENGTH, MAX_JOB_NAME_LENGTH, MAX_JOB_USER_LENGTH, validate_description,
        validate_job_name, validate_job_user,
    };

    #[test]
    fn job_name_validation_accepts_limit_and_rejects_longer_names() {
        assert!(validate_job_name(&"a".repeat(MAX_JOB_NAME_LENGTH)).is_ok());
        assert_eq!(
            validate_job_name(&"a".repeat(MAX_JOB_NAME_LENGTH + 1)).unwrap_err(),
            "name must be 128 characters or fewer"
        );
        assert!(validate_job_name(&"🙂".repeat(MAX_JOB_NAME_LENGTH)).is_ok());
    }

    #[test]
    fn job_user_validation_accepts_limit_and_rejects_longer_names() {
        assert!(validate_job_user(&"a".repeat(MAX_JOB_USER_LENGTH)).is_ok());
        assert_eq!(
            validate_job_user(&"a".repeat(MAX_JOB_USER_LENGTH + 1)).unwrap_err(),
            "user must be 50 characters or fewer"
        );
        assert!(validate_job_user(&"🙂".repeat(MAX_JOB_USER_LENGTH)).is_ok());
    }

    #[test]
    fn description_validation_accepts_optional_unicode_at_limit() {
        assert!(validate_description(None).is_ok());
        assert!(validate_description(Some(&"🙂".repeat(MAX_JOB_DESCRIPTION_LENGTH))).is_ok());
        assert_eq!(
            validate_description(Some(&"x".repeat(MAX_JOB_DESCRIPTION_LENGTH + 1))).unwrap_err(),
            "description must be 200 characters or fewer"
        );
    }

    #[test]
    fn parses_quotes_escapes_and_shell_separator() {
        assert_eq!(
            parse_command_line(r#"echo "hello world" && printf 'x y'"#).unwrap(),
            vec!["echo", "hello world", "&&", "printf", "x y"]
        );
    }

    #[test]
    fn rejects_empty_and_unterminated_commands() {
        assert!(parse_command_line("  ").is_err());
        assert!(parse_command_line("echo 'unfinished").is_err());
    }

    #[test]
    fn path_validation_requires_an_existing_directory() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(super::resolve_working_directory(file.path().to_path_buf()).is_err());
        assert!(super::resolve_working_directory(Path::new("relative").to_path_buf()).is_err());
    }
}
