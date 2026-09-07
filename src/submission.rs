//! Shared job submission validation used by the CLI and browser UI.

use std::path::PathBuf;

use anyhow::Context;

use crate::config::normalize_path;
use crate::domain::{Job, NewJob};
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
) -> anyhow::Result<Job> {
    if user.trim().is_empty() {
        anyhow::bail!("--user must not be empty");
    }
    if name.trim().is_empty() {
        anyhow::bail!("--name must not be empty");
    }
    let command = parse_command_line(&command_line)?;
    let cwd = resolve_working_directory(cwd)?;
    let id = store.create_shell_job(
        NewJob {
            name,
            user,
            cwd,
            command,
        },
        command_line,
    )?;
    Ok(store.get_job(id)?)
}

/// Resolve and validate a working directory immediately before persistence.
/// This prevents a browser selection from becoming stale between browsing and
/// submission, while keeping the stored path usable by command interpreters.
pub fn resolve_working_directory(path: PathBuf) -> anyhow::Result<PathBuf> {
    if !path.is_absolute() {
        anyhow::bail!("working directory must be an absolute path");
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolve working directory {}", path.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!("working directory is not a directory: {}", path.display());
    }
    Ok(normalize_path(canonical))
}

/// Parse the shell-like command syntax accepted by `stoker add --cmd`.
pub fn parse_command_line(input: &str) -> anyhow::Result<Vec<String>> {
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
        anyhow::bail!("--cmd contains an unterminated {quote} quote");
    }
    if token_started {
        tokens.push(token);
    }
    if tokens.is_empty() {
        anyhow::bail!("--cmd must not be empty");
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::parse_command_line;

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
