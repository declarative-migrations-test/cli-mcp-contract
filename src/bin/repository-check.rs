#![forbid(unsafe_code)]

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const EXPECTED_PRODUCT_SHA: &str = "a5e868acc0206fa9c3e91b5e36e0b1b111805885";
const MAX_SCANNED_FILE_BYTES: u64 = 1_000_000;

#[derive(Debug, Deserialize)]
struct Manifest {
    organization: String,
    repository: String,
    production_dependency: ProductionDependency,
}

#[derive(Debug, Deserialize)]
struct ProductionDependency {
    commit: String,
    path: String,
    repository: String,
    transport: String,
}

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_path = root.join("bootstrap-manifest.json");
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?,
    )
    .context("parsing bootstrap-manifest.json")?;

    require_files(
        &root,
        &[
            "README.md",
            "AGENTS.md",
            "LICENSE",
            ".gitmodules",
            "bootstrap-manifest.json",
            "Cargo.toml",
            "Cargo.lock",
            "src/lib.rs",
            "src/bin/dpm-mcp-adapter.rs",
            "src/bin/repository-check.rs",
            "src/bin/contract-certify.rs",
            ".github/workflows/ci.yml",
        ],
    )?;

    if manifest.production_dependency.commit != EXPECTED_PRODUCT_SHA {
        bail!(
            "production dependency pin drifted in the manifest: expected {EXPECTED_PRODUCT_SHA}, observed {}",
            manifest.production_dependency.commit
        );
    }
    if manifest.production_dependency.repository
        != "declarative-migrations/declarative-postgres-migrate.rs"
        || manifest.production_dependency.path != "vendor/declarative-postgres-migrate.rs"
        || manifest.production_dependency.transport != "git-submodule"
    {
        bail!("production dependency coordinates drifted in the manifest");
    }

    let vendor = root.join(&manifest.production_dependency.path);
    let actual_commit = command_stdout(
        Command::new("git")
            .arg("-C")
            .arg(&vendor)
            .args(["rev-parse", "HEAD"]),
        "inspecting the production dependency submodule",
    )?;
    if actual_commit != EXPECTED_PRODUCT_SHA {
        bail!(
            "production dependency checkout drifted: expected {EXPECTED_PRODUCT_SHA}, observed {actual_commit}"
        );
    }

    let tracked_python = command_stdout(
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["ls-files", "*.py"]),
        "listing tracked Python files",
    )?;
    if !tracked_python.is_empty() {
        bail!("tracked Python files are not allowed:\n{tracked_python}");
    }

    scan_tree(&root, &root)?;
    println!("validated {}/{}", manifest.organization, manifest.repository);
    Ok(())
}

fn require_files(root: &Path, required: &[&str]) -> Result<()> {
    let missing = required
        .iter()
        .filter(|path| !root.join(path).is_file())
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        bail!("missing required files: {}", missing.join(", "))
    }
}

fn command_stdout(command: &mut Command, description: &str) -> Result<String> {
    let output = command
        .output()
        .with_context(|| description.to_owned())?;
    if !output.status.success() {
        bail!(
            "{description} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn scan_tree(root: &Path, directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("reading directory {}", directory.display()))?
    {
        let entry = entry.context("reading directory entry")?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("relativizing {}", path.display()))?;
        if should_skip(relative) {
            continue;
        }
        let metadata = entry
            .metadata()
            .with_context(|| format!("reading metadata for {}", path.display()))?;
        if metadata.is_dir() {
            scan_tree(root, &path)?;
            continue;
        }
        if !metadata.is_file() || metadata.len() > MAX_SCANNED_FILE_BYTES {
            continue;
        }
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let conflict_markers = ["<".repeat(7), "=".repeat(7), ">".repeat(7)];
        if conflict_markers
            .iter()
            .any(|marker| text.contains(marker))
        {
            bail!("conflict marker in {}", relative.display());
        }
        if credential_shaped(text) {
            bail!("credential-shaped content in {}", relative.display());
        }
    }
    Ok(())
}

fn should_skip(relative: &Path) -> bool {
    relative.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(".git" | "vendor" | "target" | "artifacts")
        )
    })
}

fn credential_shaped(text: &str) -> bool {
    let private_key_marker = ["BEGIN", "PRIVATE KEY"].join(" ");
    if text.contains(&private_key_marker) {
        return true;
    }
    ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|prefix| contains_token(text, prefix))
}

fn contains_token(text: &str, prefix: &str) -> bool {
    let mut remainder = text;
    while let Some(index) = remainder.find(prefix) {
        let candidate = &remainder[index + prefix.len()..];
        if candidate
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric())
            .count()
            >= 20
        {
            return true;
        }
        remainder = &candidate[candidate.chars().next().map_or(0, char::len_utf8)..];
    }
    false
}
