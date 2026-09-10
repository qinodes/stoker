//! Presentation-neutral job use cases.

use uuid::Uuid;

use crate::application::error::{ApplicationError, ApplicationResult};
use crate::application::model::{
    CommitSelection, CreateJobInput, DescriptionUpdate, JobFilter, PreparedJobInput,
};
use crate::application::ports::{
    DescriptionUpdater, JobArtifacts, JobCleaner, JobCreator, JobQueries, SchedulerCancelGateway,
    SchedulerCommitGateway, WorkingDirectoryResolver,
};
use crate::domain::Job;
use crate::domain::validation::{
    normalize_description, validate_description, validate_job_name, validate_job_user,
};

pub fn create_job(
    repository: &impl JobCreator,
    working_directories: &impl WorkingDirectoryResolver,
    input: CreateJobInput,
) -> ApplicationResult<Job> {
    validate_job_user(&input.user)?;
    validate_job_name(&input.name)?;
    validate_description(input.description.as_deref())?;
    let command = parse_command_line(&input.command_line)?;
    let cwd = working_directories
        .resolve_working_directory(&input.cwd)
        .map_err(|message| ApplicationError::InvalidWorkingDirectory {
            path: input.cwd.clone(),
            message,
        })?;
    repository
        .create_job(PreparedJobInput {
            user: input.user,
            name: input.name,
            description: normalize_description(input.description),
            cwd,
            command,
            command_line: input.command_line,
        })
        .map_err(Into::into)
}

pub fn query_jobs(repository: &impl JobQueries, filter: &JobFilter) -> ApplicationResult<Vec<Job>> {
    repository.list_jobs(filter).map_err(Into::into)
}

pub fn job_detail(repository: &impl JobQueries, id: Uuid) -> ApplicationResult<Job> {
    repository.get_job(id).map_err(Into::into)
}

pub fn update_description(
    repository: &impl DescriptionUpdater,
    input: DescriptionUpdate,
) -> ApplicationResult<Job> {
    validate_description(input.description.as_deref())?;
    repository
        .update_description(DescriptionUpdate {
            description: normalize_description(input.description),
            ..input
        })
        .map_err(Into::into)
}

pub async fn commit_jobs(
    scheduler: &impl SchedulerCommitGateway,
    selection: CommitSelection,
) -> ApplicationResult<Vec<Job>> {
    if let CommitSelection::User(user) = &selection {
        validate_job_user(user)?;
    }
    scheduler.commit(selection).await.map_err(Into::into)
}

pub async fn cancel_job(
    scheduler: &impl SchedulerCancelGateway,
    id: Uuid,
) -> ApplicationResult<Job> {
    scheduler.cancel(id).await.map_err(Into::into)
}

pub fn clean_jobs(
    repository: &impl JobCleaner,
    artifacts: &impl JobArtifacts,
) -> ApplicationResult<Vec<Job>> {
    let jobs = repository.clean_terminal_jobs()?;
    for job in &jobs {
        artifacts.remove_job_artifacts(job.id)?;
    }
    Ok(jobs)
}

/// Parse the shell-like syntax stored for later execution. Error messages are
/// presentation-neutral; the CLI compatibility façade adds `--cmd` wording.
pub fn parse_command_line(input: &str) -> ApplicationResult<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut token_started = false;
    let mut quote = None;
    let mut chars = input.chars().peekable();

    while let Some(character) = chars.next() {
        match quote {
            Some('\'') => {
                if character == '\'' {
                    quote = None;
                } else {
                    token.push(character);
                }
            }
            Some('"') => {
                if character == '"' {
                    quote = None;
                } else if character == '\\' && matches!(chars.peek(), Some('"') | Some('\\')) {
                    token.push(chars.next().expect("peeked character exists"));
                } else {
                    token.push(character);
                }
            }
            Some(_) => unreachable!("command parser only uses single or double quotes"),
            None if character.is_whitespace() => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
            }
            None if character == '\'' || character == '"' => {
                quote = Some(character);
                token_started = true;
            }
            None if character == '\\' => {
                if matches!(
                    chars.peek(),
                    Some(' ') | Some('\t') | Some('\n') | Some('\'') | Some('"') | Some('\\')
                ) {
                    token.push(chars.next().expect("peeked character exists"));
                } else {
                    token.push(character);
                }
                token_started = true;
            }
            None if character == '&' && chars.peek() == Some(&'&') => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
                chars.next();
                tokens.push("&&".to_owned());
            }
            None => {
                token.push(character);
                token_started = true;
            }
        }
    }

    if let Some(quote) = quote {
        return Err(ApplicationError::InvalidCommand {
            message: format!("contains an unterminated {quote} quote"),
        });
    }
    if token_started {
        tokens.push(token);
    }
    if tokens.is_empty() {
        return Err(ApplicationError::InvalidCommand {
            message: "must not be empty".to_owned(),
        });
    }
    Ok(tokens)
}
