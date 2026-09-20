use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use uuid::Uuid;

use super::codec::{canonical_hash, format_document, parse_document};
use super::error::FlowSourceError;
use super::model::FlowSourceDocument;

pub fn write_export(
    directory: &Path,
    document: &FlowSourceDocument,
) -> Result<PathBuf, FlowSourceError> {
    fs::create_dir_all(directory)
        .map_err(|source| FlowSourceError::filesystem(directory, source))?;
    let target = directory.join(export_name(document));
    let contents = format_document(document)?;
    if target.is_file() {
        if existing_matches(&target, &contents)? {
            return Ok(target);
        }
        return Err(FlowSourceError::invalid(format!(
            "refusing to overwrite existing export {} with different contents",
            target.display()
        )));
    }

    let temporary = directory.join(format!(".flow-export-{}.tmp", Uuid::new_v4()));
    let result = write_then_rename(&temporary, &target, &contents);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|()| target)
}

pub fn write_snapshot(
    root: &Path,
    document: &FlowSourceDocument,
) -> Result<PathBuf, FlowSourceError> {
    let directory = root.join("flows").join("snapshots");
    fs::create_dir_all(&directory)
        .map_err(|source| FlowSourceError::filesystem(&directory, source))?;
    let hash = hash_value(&document.base.hash);
    let suffix = format!("-r{:010}-{}.json", document.base.revision, short_hash(hash));
    if let Some(existing) = find_suffix(&directory, &suffix)? {
        verify_artifact(&existing, &document.base.hash)?;
        return Ok(existing);
    }
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let target = directory.join(format!("snapshot-{timestamp}{suffix}"));
    write_immutable(&target, &format_document(document)?)?;
    Ok(target)
}

pub fn archive_source(
    root: &Path,
    document: &FlowSourceDocument,
) -> Result<PathBuf, FlowSourceError> {
    let directory = root.join("flows").join("sources");
    fs::create_dir_all(&directory)
        .map_err(|source| FlowSourceError::filesystem(&directory, source))?;
    let hash = canonical_hash(document)?;
    let target = directory.join(format!("{}.json", hash_value(&hash)));
    if target.is_file() {
        verify_artifact(&target, &hash)?;
        return Ok(target);
    }
    write_immutable(&target, &format_document(document)?)?;
    Ok(target)
}

pub fn verify_artifact(path: &Path, expected_hash: &str) -> Result<(), FlowSourceError> {
    let bytes = fs::read(path).map_err(|source| FlowSourceError::filesystem(path, source))?;
    let document = parse_document(&bytes)?;
    let actual = canonical_hash(&document)?;
    if actual != expected_hash {
        return Err(FlowSourceError::invalid(format!(
            "artifact {} failed SHA-256 verification: expected {expected_hash}, got {actual}",
            path.display()
        )));
    }
    Ok(())
}

fn write_immutable(target: &Path, contents: &[u8]) -> Result<(), FlowSourceError> {
    let directory = target
        .parent()
        .ok_or_else(|| FlowSourceError::invalid("artifact path has no parent"))?;
    let temporary = directory.join(format!(".flow-artifact-{}.tmp", Uuid::new_v4()));
    let result = write_then_rename(&temporary, target, contents).and_then(|()| {
        let mut permissions = fs::metadata(target)
            .map_err(|source| FlowSourceError::filesystem(target, source))?
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(target, permissions)
            .map_err(|source| FlowSourceError::filesystem(target, source))
    });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn find_suffix(directory: &Path, suffix: &str) -> Result<Option<PathBuf>, FlowSourceError> {
    let entries =
        fs::read_dir(directory).map_err(|source| FlowSourceError::filesystem(directory, source))?;
    Ok(entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(suffix))
        }))
}

fn hash_value(hash: &str) -> &str {
    hash.strip_prefix("sha256:").unwrap_or(hash)
}

fn short_hash(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

fn export_name(document: &FlowSourceDocument) -> String {
    let hash = document
        .base
        .hash
        .strip_prefix("sha256:")
        .unwrap_or(&document.base.hash);
    let short = hash.get(..12).unwrap_or(hash);
    format!("flows-r{:010}-{short}.json", document.base.revision)
}

fn write_then_rename(
    temporary: &Path,
    target: &Path,
    contents: &[u8],
) -> Result<(), FlowSourceError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary)
        .map_err(|source| FlowSourceError::filesystem(temporary, source))?;
    file.write_all(contents)
        .map_err(|source| FlowSourceError::filesystem(temporary, source))?;
    file.sync_all()
        .map_err(|source| FlowSourceError::filesystem(temporary, source))?;
    drop(file);

    match fs::rename(temporary, target) {
        Ok(()) => Ok(()),
        Err(_) if target.is_file() => {
            let matches = existing_matches(target, contents);
            let cleanup = fs::remove_file(temporary)
                .map_err(|source| FlowSourceError::filesystem(temporary, source));
            cleanup?;
            let matches = matches?;
            if matches {
                Ok(())
            } else {
                Err(FlowSourceError::invalid(format!(
                    "refusing to overwrite existing export {} with different contents",
                    target.display()
                )))
            }
        }
        Err(source) => Err(FlowSourceError::filesystem(target, source)),
    }
}

fn existing_matches(path: &Path, expected: &[u8]) -> Result<bool, FlowSourceError> {
    fs::read(path)
        .map(|contents| contents == expected)
        .map_err(|source| FlowSourceError::filesystem(path, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_source::{FLOW_SOURCE_SCHEMA_VERSION, FlowSourceBase};

    fn empty_document() -> FlowSourceDocument {
        let mut document = FlowSourceDocument {
            schema_version: FLOW_SOURCE_SCHEMA_VERSION,
            base: FlowSourceBase {
                revision: 0,
                hash: format!("sha256:{}", "0".repeat(64)),
            },
            flows: vec![],
        };
        document.base.hash = canonical_hash(&document).unwrap();
        document
    }

    #[test]
    fn export_is_atomic_and_same_document_reuses_target() {
        let directory = tempfile::tempdir().unwrap();
        let document = empty_document();
        let first = write_export(directory.path(), &document).unwrap();
        let second = write_export(directory.path(), &document).unwrap();
        assert_eq!(first, second);
        assert!(first.is_file());
        assert_eq!(
            fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .count(),
            1
        );
    }

    #[test]
    fn failed_final_rename_removes_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let document = empty_document();
        let target = directory.path().join(export_name(&document));
        fs::create_dir(&target).unwrap();
        let error = write_export(directory.path(), &document).unwrap_err();
        assert!(error.to_string().contains("could not access"));
        assert!(
            fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp"))
        );
    }

    #[test]
    fn export_refuses_to_overwrite_different_existing_contents() {
        let directory = tempfile::tempdir().unwrap();
        let document = empty_document();
        let target = directory.path().join(export_name(&document));
        fs::write(&target, b"different").unwrap();
        let error = write_export(directory.path(), &document).unwrap_err();
        assert!(error.to_string().contains("refusing to overwrite"));
        assert_eq!(fs::read(&target).unwrap(), b"different");
        assert!(
            fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp"))
        );
    }

    #[test]
    fn snapshot_is_ordered_deduplicated_read_only_and_verified() {
        let directory = tempfile::tempdir().unwrap();
        let document = empty_document();
        let first = write_snapshot(directory.path(), &document).unwrap();
        let second = write_snapshot(directory.path(), &document).unwrap();
        assert_eq!(first, second);
        assert!(
            first
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("r0000000000")
        );
        assert!(fs::metadata(&first).unwrap().permissions().readonly());
        verify_artifact(&first, &document.base.hash).unwrap();

        make_writable(&first);
        fs::write(&first, b"tampered").unwrap();
        assert!(verify_artifact(&first, &document.base.hash).is_err());
    }

    #[test]
    fn source_archive_uses_content_hash_and_deduplicates() {
        let directory = tempfile::tempdir().unwrap();
        let document = empty_document();
        let first = archive_source(directory.path(), &document).unwrap();
        let second = archive_source(directory.path(), &document).unwrap();
        assert_eq!(first, second);
        assert!(fs::metadata(&first).unwrap().permissions().readonly());
        assert_eq!(fs::read_dir(first.parent().unwrap()).unwrap().count(), 1);
    }

    fn make_writable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(permissions.mode() | 0o200);
            fs::set_permissions(path, permissions).unwrap();
        }
        #[cfg(windows)]
        #[allow(clippy::permissions_set_readonly_false)]
        {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions).unwrap();
        }
    }
}
