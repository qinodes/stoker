//! Filesystem orchestration for declarative Flow definition commands.

use anyhow::{Context, Result};
use crossterm::style::Color;

use crate::domain::flow::ExecutionMode;
use crate::flow_source;
use crate::{StokerPaths, Store, output};

use super::extended_args::{FlowExportArgs, FlowSyncArgs};

pub(super) fn export(store: &Store, args: FlowExportArgs) -> Result<()> {
    if store.current_mode()? != ExecutionMode::Scheduled {
        anyhow::bail!("flow export is only available in scheduled mode");
    }
    let document = store.export_flow_source()?;
    let path = flow_source::write_export(&args.dir, &document)
        .with_context(|| format!("export Flow definitions to {}", args.dir.display()))?;
    let colors = output::stdout_color_enabled();
    println!(
        "{} {} flow(s) at revision {} to {}.",
        output::paint_bold("Exported", Color::Green, colors),
        document.flows.len(),
        output::paint(document.base.revision, Color::Cyan, colors),
        path.display()
    );
    Ok(())
}

pub(super) fn snapshot(paths: &StokerPaths, store: &Store) -> Result<()> {
    if store.current_mode()? != ExecutionMode::Scheduled {
        anyhow::bail!("flow snapshot is only available in scheduled mode");
    }
    let document = store.export_flow_source()?;
    let path = flow_source::write_snapshot(&paths.root, &document)
        .context("write immutable Flow snapshot")?;
    let colors = output::stdout_color_enabled();
    println!(
        "{} {} flow(s) at revision {} to {}.",
        output::paint_bold("Snapshotted", Color::Green, colors),
        document.flows.len(),
        output::paint(document.base.revision, Color::Cyan, colors),
        path.display()
    );
    Ok(())
}

pub(super) fn sync(paths: &StokerPaths, store: &Store, args: FlowSyncArgs) -> Result<()> {
    let bytes = std::fs::read(&args.file)
        .with_context(|| format!("read Flow definition {}", args.file.display()))?;
    let document = flow_source::parse_document(&bytes)
        .with_context(|| format!("parse Flow definition {}", args.file.display()))?;
    let definitions = flow_source::resolve_document(&document, &args.file)
        .with_context(|| format!("resolve Flow definition {}", args.file.display()))?;
    let result = store.sync_flow_source(&paths.root, &document, &definitions, args.dry_run)?;
    let colors = output::stdout_color_enabled();
    let (label, label_color) = if args.dry_run {
        ("Dry run", Color::Yellow)
    } else if result.changed {
        ("Synced", Color::Green)
    } else {
        ("No changes", Color::Green)
    };
    println!(
        "{}: {} {} {} {}; revision {}, {}.",
        output::paint_bold(label, label_color, colors),
        output::paint(format!("+{}", result.diff.added), Color::Cyan, colors),
        output::paint(format!("~{}", result.diff.updated), Color::Cyan, colors),
        output::paint(format!("-{}", result.diff.removed), Color::Cyan, colors),
        output::paint(format!("={}", result.diff.unchanged), Color::Cyan, colors),
        output::paint(result.revision, Color::Cyan, colors),
        output::paint(&result.hash, Color::Cyan, colors)
    );
    if !args.dry_run {
        println!(
            "{}",
            output::paint(
                "Queue remains locked; review the result, then run `stoker queue unlock`.",
                Color::Yellow,
                colors
            )
        );
    }
    Ok(())
}
