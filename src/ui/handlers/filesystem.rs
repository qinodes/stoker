use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use super::super::dto::{FsDirectoriesResponse, FsDirectory, FsLocation, FsRootsResponse};
use super::super::error::ApiError;
use super::super::state::ApiState;

const MAX_DIRECTORIES: usize = 500;
const MAX_ENTRIES: usize = 5_000;

#[derive(Debug, Deserialize)]
pub(in crate::ui) struct DirectoryQuery {
    path: Option<String>,
}

pub(in crate::ui) async fn roots(
    State(state): State<ApiState>,
) -> Result<Json<FsRootsResponse>, ApiError> {
    let startup = std::env::current_dir()
        .ok()
        .and_then(|path| path.canonicalize().ok())
        .map(crate::config::normalize_path)
        .filter(|path| path.is_dir());
    let home = home_directory()
        .and_then(|path| path.canonicalize().ok())
        .map(crate::config::normalize_path)
        .filter(|path| path.is_dir());

    let mut locations = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |kind: &'static str, label: String, path: PathBuf| {
        if let Some(value) = ui_path(path)
            && seen.insert(value.clone())
        {
            locations.push(FsLocation {
                kind,
                label,
                path: value,
            });
        }
    };
    if let Some(path) = home.clone() {
        add("home", "Home".to_owned(), path);
    }
    if let Some(path) = startup.clone() {
        add("startup", "UI startup directory".to_owned(), path);
    }
    let jobs = state.store.list_jobs(None).map_err(ApiError::internal)?;
    for job in jobs.iter().rev().take(8) {
        if job.cwd.is_dir() {
            let label = job
                .cwd
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| format!("Recent · {value}"))
                .unwrap_or_else(|| "Recent".to_owned());
            add("recent", label, job.cwd.clone());
        }
    }
    add_platform_roots(&mut add);
    let default_path = startup
        .or(home)
        .or_else(|| locations.first().map(|entry| PathBuf::from(&entry.path)))
        .and_then(ui_path);
    Ok(Json(FsRootsResponse {
        default_path,
        locations,
    }))
}

pub(in crate::ui) async fn directories(
    Query(query): Query<DirectoryQuery>,
) -> Result<Json<FsDirectoriesResponse>, ApiError> {
    let raw_path = query
        .path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::invalid_input("path is required"))?;
    let path = PathBuf::from(raw_path);
    if !path.is_absolute() {
        return Err(ApiError::invalid_input(
            "path must be an absolute directory",
        ));
    }
    let metadata = fs::metadata(&path).map_err(|error| fs_error(&path, error))?;
    if !metadata.is_dir() {
        return Err(ApiError::invalid_input("path is not a directory"));
    }
    let path = path
        .canonicalize()
        .map_err(|error| fs_error(&path, error))?;
    let parent = path.parent().map(PathBuf::from);
    let entries = fs::read_dir(&path).map_err(|error| fs_error(&path, error))?;
    let mut directories = Vec::new();
    let mut skipped_entries = 0;
    let mut truncated = false;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            skipped_entries += 1;
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            skipped_entries += 1;
            continue;
        };
        let candidate = entry.path();
        let Ok(metadata) = fs::metadata(&candidate) else {
            skipped_entries += 1;
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            skipped_entries += 1;
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        if directories.len() >= MAX_DIRECTORIES {
            truncated = true;
            break;
        }
        let Some(path) = candidate
            .canonicalize()
            .ok()
            .map(crate::config::normalize_path)
            .and_then(ui_path)
        else {
            skipped_entries += 1;
            continue;
        };
        directories.push(FsDirectory {
            name,
            path,
            is_symlink: file_type.is_symlink(),
        });
    }
    directories.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.name.cmp(&right.name))
    });
    Ok(Json(FsDirectoriesResponse {
        path: ui_path(path)
            .ok_or_else(|| ApiError::invalid_input("directory path is not valid UTF-8"))?,
        parent: parent.map(crate::config::normalize_path).and_then(ui_path),
        directories,
        truncated,
        skipped_entries,
    }))
}

fn fs_error(path: &std::path::Path, error: std::io::Error) -> ApiError {
    match error.kind() {
        std::io::ErrorKind::NotFound => ApiError::path_not_found(path),
        std::io::ErrorKind::PermissionDenied => {
            ApiError::forbidden(format!("cannot read directory {}", path.display()))
        }
        _ => ApiError::internal(error),
    }
}

fn ui_path(path: PathBuf) -> Option<String> {
    let path = crate::config::normalize_path(path);
    path.to_str().map(|value| {
        #[cfg(windows)]
        {
            value.replace('\\', "/")
        }
        #[cfg(not(windows))]
        {
            value.to_owned()
        }
    })
}

fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn add_platform_roots(add: &mut impl FnMut(&'static str, String, PathBuf)) {
    #[cfg(windows)]
    for letter in b'A'..=b'Z' {
        let path = PathBuf::from(format!("{}:\\", char::from(letter)));
        if path.is_dir() {
            add("drive", format!("{}:", char::from(letter)), path);
        }
    }
    #[cfg(not(windows))]
    add("root", "Filesystem root".to_owned(), PathBuf::from("/"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn directory_browser_lists_sorted_folders_and_rejects_invalid_paths() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("Zulu")).unwrap();
        fs::create_dir(directory.path().join("alpha")).unwrap();
        fs::write(directory.path().join("plain-file"), b"file").unwrap();
        let response = directories(Query(DirectoryQuery {
            path: Some(directory.path().to_string_lossy().into_owned()),
        }))
        .await
        .unwrap()
        .0;
        assert_eq!(
            response
                .directories
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "Zulu"]
        );
        assert!(!response.truncated);
        assert_eq!(response.skipped_entries, 0);
        assert!(response.parent.is_some());

        for path in [None, Some("  ".into()), Some("relative/path".into())] {
            let error = directories(Query(DirectoryQuery { path }))
                .await
                .unwrap_err();
            assert_eq!(
                error.code,
                super::super::super::error::ErrorCode::InvalidInput
            );
        }
        let missing = directories(Query(DirectoryQuery {
            path: Some(
                directory
                    .path()
                    .join("missing")
                    .to_string_lossy()
                    .into_owned(),
            ),
        }))
        .await
        .unwrap_err();
        assert_eq!(
            missing.code,
            super::super::super::error::ErrorCode::NotFound
        );
        let file = directories(Query(DirectoryQuery {
            path: Some(
                directory
                    .path()
                    .join("plain-file")
                    .to_string_lossy()
                    .into_owned(),
            ),
        }))
        .await
        .unwrap_err();
        assert_eq!(
            file.code,
            super::super::super::error::ErrorCode::InvalidInput
        );
    }

    #[test]
    fn ui_paths_are_absolute_and_platform_normalized() {
        let directory = tempfile::tempdir().unwrap();
        let path = ui_path(directory.path().canonicalize().unwrap()).unwrap();
        assert!(!path.is_empty());
        #[cfg(windows)]
        assert!(!path.contains('\\'));
    }
}
