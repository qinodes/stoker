use std::path::{Path, PathBuf};

use crate::config::{command_cwd, normalize_path};
use crate::domain::flow::FlowDefinition;

use super::error::FlowSourceError;
use super::model::{FlowSourceCwd, FlowSourceCwdMap, FlowSourceDocument};

pub fn resolve_document(
    document: &FlowSourceDocument,
    definition_path: &Path,
) -> Result<Vec<FlowDefinition>, FlowSourceError> {
    document.validate()?;
    let base = definition_directory(definition_path)?;
    document
        .flows
        .iter()
        .map(|source| {
            let mut definition = source.to_domain()?;
            for (task, source_task) in definition.tasks.iter_mut().zip(&source.tasks) {
                task.cwd = command_cwd(&resolve_cwd(
                    source_task.cwd.as_ref(),
                    &base,
                    current_platform(),
                )?);
            }
            Ok(definition)
        })
        .collect()
}

/// Resolves browser-provided document content relative to the workspace root.
/// The browser never supplies a server-side definition path.
pub fn resolve_workspace_document(
    document: &FlowSourceDocument,
    workspace_root: &Path,
) -> Result<Vec<FlowDefinition>, FlowSourceError> {
    resolve_document(document, &workspace_root.join("flow-source.json"))
}

fn definition_directory(path: &Path) -> Result<PathBuf, FlowSourceError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| FlowSourceError::filesystem(path, source))?
            .join(path)
    };
    absolute
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .ok_or_else(|| FlowSourceError::invalid("definition file must have a parent directory"))
}

fn resolve_cwd(
    cwd: Option<&FlowSourceCwd>,
    base: &Path,
    platform: &str,
) -> Result<PathBuf, FlowSourceError> {
    let selected = match cwd {
        None => None,
        Some(FlowSourceCwd::Path(path)) => Some(path.as_str()),
        Some(FlowSourceCwd::Platform(paths)) => select_platform_path(paths, platform),
    };
    let absolute = selected.is_some_and(|path| Path::new(path).is_absolute());
    let candidate = match selected {
        Some(path) if absolute => PathBuf::from(path),
        Some(path) => base.join(path),
        None => base.to_path_buf(),
    };
    if !candidate.is_dir() {
        return Err(FlowSourceError::invalid(format!(
            "task cwd {} is not a directory",
            candidate.display()
        )));
    }
    if absolute {
        return Ok(normalize_path(candidate));
    }
    candidate
        .canonicalize()
        .map(normalize_path)
        .map_err(|source| FlowSourceError::filesystem(&candidate, source))
}

fn select_platform_path<'a>(paths: &'a FlowSourceCwdMap, platform: &str) -> Option<&'a str> {
    let selected = match platform {
        "windows" => paths.windows.as_deref(),
        "linux" => paths.linux.as_deref(),
        "macos" => paths.macos.as_deref(),
        _ => None,
    };
    selected.or(paths.default.as_deref())
}

const fn current_platform() -> &'static str {
    #[cfg(target_os = "windows")]
    return "windows";
    #[cfg(target_os = "linux")]
    return "linux";
    #[cfg(target_os = "macos")]
    return "macos";
    #[allow(unreachable_code)]
    "other"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::flow::{DependencyMode, DependencyStatus};
    use crate::flow_source::{
        FLOW_SOURCE_SCHEMA_VERSION, FlowSourceBase, FlowSourceDependency, FlowSourceFlow,
        FlowSourceSchedule, FlowSourceTask,
    };

    fn paths() -> FlowSourceCwdMap {
        FlowSourceCwdMap {
            default: Some("default".into()),
            windows: Some("windows".into()),
            linux: Some("linux".into()),
            macos: Some("macos".into()),
        }
    }

    fn document(cwd: Option<FlowSourceCwd>) -> FlowSourceDocument {
        FlowSourceDocument {
            schema_version: FLOW_SOURCE_SCHEMA_VERSION,
            base: FlowSourceBase {
                revision: 0,
                hash: format!("sha256:{}", "0".repeat(64)),
            },
            flows: vec![FlowSourceFlow {
                id: "flow".into(),
                name: "flow".into(),
                owner: "owner".into(),
                enabled: true,
                schedule: FlowSourceSchedule::Once {
                    at: "2099-01-01T00:00:00Z".into(),
                },
                tasks: vec![FlowSourceTask {
                    id: "task".into(),
                    name: "task".into(),
                    cwd,
                    command: "echo ok".into(),
                    retry: 0,
                    depend_mode: DependencyMode::All,
                    depends_on: Vec::<FlowSourceDependency>::new(),
                }],
            }],
        }
    }

    #[test]
    fn platform_selection_prefers_exact_then_default() {
        let paths = paths();
        assert_eq!(select_platform_path(&paths, "windows"), Some("windows"));
        assert_eq!(select_platform_path(&paths, "linux"), Some("linux"));
        assert_eq!(select_platform_path(&paths, "macos"), Some("macos"));
        assert_eq!(select_platform_path(&paths, "other"), Some("default"));
        let only_linux = FlowSourceCwdMap {
            default: None,
            windows: None,
            linux: Some("linux".into()),
            macos: None,
        };
        assert_eq!(select_platform_path(&only_linux, "windows"), None);
    }

    #[test]
    fn relative_platform_path_and_omitted_path_resolve_from_definition() {
        let directory = tempfile::tempdir().unwrap();
        let selected = directory.path().join("selected");
        std::fs::create_dir(&selected).unwrap();
        let cwd = FlowSourceCwd::Platform(FlowSourceCwdMap {
            default: Some("selected".into()),
            windows: None,
            linux: None,
            macos: None,
        });
        let resolved =
            resolve_document(&document(Some(cwd)), &directory.path().join("flows.json")).unwrap();
        assert_eq!(
            Path::new(&resolved[0].tasks[0].cwd),
            normalize_path(selected.canonicalize().unwrap())
        );

        let resolved =
            resolve_document(&document(None), &directory.path().join("flows.json")).unwrap();
        assert_eq!(
            Path::new(&resolved[0].tasks[0].cwd),
            normalize_path(directory.path().canonicalize().unwrap())
        );
    }

    #[test]
    fn absolute_cwd_preserves_its_source_text_for_a_stable_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let cwd = command_cwd(directory.path());
        let resolved = resolve_document(
            &document(Some(FlowSourceCwd::Path(cwd.clone()))),
            &directory.path().join("flows.json"),
        )
        .unwrap();

        assert_eq!(resolved[0].tasks[0].cwd, cwd);
    }

    #[test]
    fn missing_or_non_directory_selected_path_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let missing = document(Some(FlowSourceCwd::Path("missing".into())));
        assert!(resolve_document(&missing, &directory.path().join("flows.json")).is_err());

        let file = directory.path().join("file");
        std::fs::write(&file, b"not a directory").unwrap();
        let file_path = document(Some(FlowSourceCwd::Path("file".into())));
        let error = resolve_document(&file_path, &directory.path().join("flows.json")).unwrap_err();
        assert!(error.to_string().contains("is not a directory"));
    }

    #[test]
    fn resolution_preserves_dependencies_and_commands() {
        let directory = tempfile::tempdir().unwrap();
        let mut document = document(None);
        document.flows[0].tasks.push(FlowSourceTask {
            id: "next".into(),
            name: "next".into(),
            cwd: None,
            command: "echo next".into(),
            retry: 2,
            depend_mode: DependencyMode::All,
            depends_on: vec![FlowSourceDependency {
                task_id: "task".into(),
                status: DependencyStatus::Succeeded,
            }],
        });
        let resolved = resolve_document(&document, &directory.path().join("flows.json")).unwrap();
        assert_eq!(resolved[0].tasks[1].command, "echo next");
        assert_eq!(resolved[0].tasks[1].retry, 2);
        assert_eq!(
            resolved[0].tasks[1].dependencies[0].upstream_task_id,
            "task"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_resolution_removes_extended_path_prefixes() {
        let directory = tempfile::tempdir().unwrap();
        let resolved =
            resolve_document(&document(None), &directory.path().join("flows.json")).unwrap();
        assert!(!resolved[0].tasks[0].cwd.starts_with(r"\\?\"));
    }
}
