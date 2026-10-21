use std::path::{Path, PathBuf};

use clap::Parser;
use monoplan_core::{Doc, JsonExport};
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct ExportArgs {
    /// Write the JSON export to this path instead of stdout.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

/// Daemon side: the export as pretty JSON. The client decides whether
/// it goes to stdout or `--out`.
pub fn render(doc: &Doc) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&doc.export_json())?)
}

/// Client side: deliver the daemon's rendering.
pub fn deliver(json: &str, out: Option<&Path>) -> anyhow::Result<()> {
    if let Some(path) = out {
        std::fs::write(path, json.as_bytes())?;
    } else {
        println!("{json}");
    }
    Ok(())
}

pub fn write_export(export: &JsonExport, out: Option<&Path>) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(export)?;
    if let Some(path) = out {
        std::fs::write(path, bytes)?;
    } else {
        println!("{}", String::from_utf8(bytes)?);
    }
    Ok(())
}
