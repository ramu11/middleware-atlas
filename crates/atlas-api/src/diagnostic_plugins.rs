//! Local-first diagnostic plugins.
//!
//! These adapters stage bounded inputs, summarize thread dumps locally, and execute
//! fixed Eclipse MAT operations in a constrained local container. They prepare bounded
//! report context for the separate agent-chat request; they do not call the model directly.
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
    process::Command,
    time::timeout,
};

const MAX_THREAD_DUMP_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct PreparedFile {
    pub name: String,
    pub relative_path: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreparedPlugin {
    pub tool_run_id: String,
    pub tool_id: String,
    pub tool_version: String,
    pub status: String,
    pub artifact_id: String,
    pub artifact_filename: String,
    pub artifact_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    pub summary: String,
    pub workspace_relative_path: String,
    pub files: Vec<PreparedFile>,
    pub warnings: Vec<String>,
    pub llm_analysis: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct ThreadDumpSummary {
    bytes_analyzed: u64,
    thread_headers_detected: u64,
    thread_state_entries: u64,
    thread_count_estimate: u64,
    state_counts: BTreeMap<String, u64>,
    blocked_thread_names: Vec<String>,
    monitor_lock_mentions: u64,
    explicit_deadlock_marker_detected: bool,
}

#[allow(clippy::too_many_arguments)]
pub async fn prepare(
    data_dir: &Path,
    case_id: &str,
    tool_run_id: &str,
    problem_description: &str,
    artifact_id: &str,
    artifact_kind: &str,
    artifact_filename: &str,
    artifact_size_bytes: i64,
    artifact_sha256: &str,
) -> Result<PreparedPlugin, String> {
    let (tool_id, is_heap) = match artifact_kind {
        "heap_dump" => ("mat.heap_dump.prepare", true),
        "thread_dump" => ("jvm.thread_dump.prepare", false),
        _ => {
            return Err(format!(
                "No diagnostic plugin supports artifact kind '{artifact_kind}'."
            ));
        }
    };

    let source = data_dir
        .join("artifacts")
        .join(case_id)
        .join(artifact_id)
        .join("original");
    let metadata = fs::symlink_metadata(&source).await.map_err(|_| {
        "The stored artifact file is missing; upload it again before preparing analysis.".to_owned()
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("The stored artifact is not a regular file and cannot be analyzed.".to_owned());
    }
    if !is_heap && metadata.len() > MAX_THREAD_DUMP_BYTES {
        return Err(format!(
            "Thread dump is {} bytes; the local text parser limit is {} bytes. Upload a smaller or appropriately split thread dump.",
            metadata.len(),
            MAX_THREAD_DUMP_BYTES
        ));
    }

    let (workspace_relative_path, workspace) = if is_heap {
        (
            format!("work/{case_id}/heap/{artifact_id}"),
            data_dir
                .join("work")
                .join(case_id)
                .join("heap")
                .join(artifact_id),
        )
    } else {
        (
            format!("work/{case_id}/{tool_run_id}"),
            data_dir.join("work").join(case_id).join(tool_run_id),
        )
    };
    let input_dir = workspace.join("input");
    fs::create_dir_all(&input_dir)
        .await
        .map_err(|error| format!("Could not create a private plugin workspace: {error}"))?;
    set_private_dir(&workspace)
        .map_err(|error| format!("Could not secure plugin workspace: {error}"))?;
    set_private_dir(&input_dir)
        .map_err(|error| format!("Could not secure plugin input directory: {error}"))?;

    let input_name = safe_leaf(artifact_filename);
    let staged_input = input_dir.join(&input_name);
    // A hard link preserves the dump's expected extension without copying a potentially
    // very large heap dump. The workspace and artifact store are under the same data dir.
    match fs::symlink_metadata(&staged_input).await {
        Ok(existing) if existing.is_file() && !existing.file_type().is_symlink() && existing.len() == metadata.len() => {},
        Ok(_) => return Err("Existing staged input is not a regular file or its size differs from the saved artifact.".to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::hard_link(&source, &staged_input).await.map_err(|error| {
                format!("Could not stage the artifact without duplicating its bytes ({error}). Ensure data/work and data/artifacts are on the same filesystem, then retry.")
            })?;
        }
        Err(error) => return Err(format!("Could not validate the existing staged input: {error}")),
    }

    let mut files = vec![PreparedFile {
        name: input_name.clone(),
        relative_path: format!("{workspace_relative_path}/input/{input_name}"),
        purpose: if is_heap {
            "MAT input heap dump"
        } else {
            "Thread dump input for local summary"
        }
        .to_owned(),
    }];
    let mut warnings = Vec::new();

    let problem_file = format!("# Problem description\n\n{problem_description}\n");
    write_text(
        &workspace,
        &workspace_relative_path,
        "problem-description.md",
        &problem_file,
        "Original problem description",
        &mut files,
    )
    .await?;

    let request = json!({
        "schema_version": "1",
        "tool_id": tool_id,
        "operation": "prepare",
        "case_id": case_id,
        "tool_run_id": tool_run_id,
        "problem_description": problem_description,
        "artifact": {
            "id": artifact_id,
            "kind": artifact_kind,
            "filename": artifact_filename,
            "size_bytes": artifact_size_bytes,
            "sha256": artifact_sha256,
            "input_file": format!("input/{input_name}"),
        },
        "privacy": {
            "raw_dump_sent_to_model": false,
            "model_invoked": false,
        }
    });
    write_json(
        &workspace,
        &workspace_relative_path,
        "analysis-request.json",
        &request,
        "Versioned request manifest for a downstream agent/tool worker",
        &mut files,
    )
    .await?;

    let summary = if is_heap {
        let operation_manifest = json!({
            "schema_version": "1",
            "execution_status": "prepared_not_executed",
            "executable": "ParseHeapDump.sh (Linux/macOS) or ParseHeapDump.bat (Windows)",
            "input_file": format!("input/{input_name}"),
            "operations": [
                {"id": "org.eclipse.mat.api:suspects", "purpose": "Leak-suspect report"},
                {"id": "org.eclipse.mat.api:overview", "purpose": "Heap overview, top consumers and class histogram"},
                {"id": "org.eclipse.mat.api:top_components", "purpose": "Memory usage by component"}
            ],
            "note": "This preparation plugin creates the request files only. It does not execute MAT or claim these reports exist."
        });
        write_json(
            &workspace,
            &workspace_relative_path,
            "mat-operations.json",
            &operation_manifest,
            "Approved MAT batch operations for a future constrained worker",
            &mut files,
        )
        .await?;
        let readme = format!(
            "# Heap dump preparation\n\n- Input: `input/{input_name}` (hard link to the stored artifact; no second copy was made).\n- Request: `analysis-request.json`.\n- Suggested MAT operations: `mat-operations.json`.\n\nThis file documents the preparation stage. The API may then run the listed fixed MAT operation in a constrained local Docker/Podman container; check `mat-execution.json` and the returned tool status to confirm whether MAT actually ran. If no usable container runtime is available, this workspace remains prepared without MAT reports. Do not send the raw heap dump or MAT index files to an LLM.\n"
        );
        write_text(
            &workspace,
            &workspace_relative_path,
            "README.md",
            &readme,
            "How this prepared heap workspace is intended to be consumed",
            &mut files,
        )
        .await?;
        format!(
            "Prepared a MAT request for '{}'; MAT reports have not been executed.",
            artifact_filename
        )
    } else {
        let bytes = fs::read(&source).await.map_err(|error| {
            format!("Could not read the thread dump for local parsing: {error}")
        })?;
        let bytes_analyzed = bytes.len() as u64;
        let parsed = tokio::task::spawn_blocking(move || {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            summarize_thread_dump(&text, bytes_analyzed)
        })
        .await
        .map_err(|error| format!("Thread-dump parsing worker failed: {error}"))?;
        let summary_json = serde_json::to_value(&parsed)
            .map_err(|error| format!("Could not serialize thread dump summary: {error}"))?;
        write_json(
            &workspace,
            &workspace_relative_path,
            "thread-dump-summary.json",
            &summary_json,
            "Bounded structured thread-state and lock summary",
            &mut files,
        )
        .await?;
        let summary_markdown = render_thread_summary(&parsed);
        write_text(
            &workspace,
            &workspace_relative_path,
            "thread-dump-summary.md",
            &summary_markdown,
            "Human-readable thread-state summary",
            &mut files,
        )
        .await?;
        let context = format!(
            "# Thread dump analysis context\n\n## Problem description\n\n{problem_description}\n\n## Source artifact\n\n- Filename: `{artifact_filename}`\n- SHA-256: `{artifact_sha256}`\n- Bytes parsed locally: {}\n\n## Prepared evidence summary\n\n{}\n\n## Analysis guardrails\n\n- Use the original staged thread dump for stack-level verification.\n- Treat WAITING and TIMED_WAITING as potentially normal; do not label them a hang by themselves.\n- A missing explicit deadlock marker does not prove that no deadlock exists.\n- Correlate repeated dumps and the supplied problem description before asserting a root cause.\n- This file is local preparation context; no model was called.\n",
            parsed.bytes_analyzed, summary_markdown
        );
        write_text(
            &workspace,
            &workspace_relative_path,
            "thread-dump-context.md",
            &context,
            "Problem plus bounded evidence context for a future agent step",
            &mut files,
        )
        .await?;
        let readme = format!(
            "# Thread dump preparation\n\n- Input: `input/{input_name}` (hard link to the stored artifact).\n- Structured summary: `thread-dump-summary.json`.\n- Human summary: `thread-dump-summary.md`.\n- Problem/evidence context: `thread-dump-context.md`.\n\nThe local parser counts recognizable thread states and lock mentions. It is a triage aid, not a definitive deadlock detector or RCA. Validate findings against full stack traces and, where available, multiple dumps.\n"
        );
        write_text(
            &workspace,
            &workspace_relative_path,
            "README.md",
            &readme,
            "How to consume and interpret the prepared thread dump files",
            &mut files,
        )
        .await?;
        if parsed.thread_state_entries == 0 {
            warnings.push("No standard 'java.lang.Thread.State:' entries were found; the file may use a JVM-specific format or not be a thread dump.".to_owned());
        }
        format!(
            "Parsed {} thread-state entries; estimated {} threads; explicit deadlock marker: {}.",
            parsed.thread_state_entries,
            parsed.thread_count_estimate,
            if parsed.explicit_deadlock_marker_detected {
                "present"
            } else {
                "not found"
            }
        )
    };

    Ok(PreparedPlugin {
        tool_run_id: tool_run_id.to_owned(),
        tool_id: tool_id.to_owned(),
        tool_version: "1".to_owned(),
        status: "prepared".to_owned(),
        artifact_id: artifact_id.to_owned(),
        artifact_filename: artifact_filename.to_owned(),
        artifact_sha256: artifact_sha256.to_owned(),
        job_id: None,
        summary,
        workspace_relative_path,
        files,
        warnings,
        llm_analysis: None,
    })
}

fn summarize_thread_dump(text: &str, bytes_analyzed: u64) -> ThreadDumpSummary {
    let mut summary = ThreadDumpSummary {
        bytes_analyzed,
        ..ThreadDumpSummary::default()
    };
    let mut current_thread: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('"') && trimmed[1..].contains('"') {
            summary.thread_headers_detected += 1;
            current_thread = trimmed.split('"').nth(1).map(str::to_owned);
        }
        if let Some((_, state_tail)) = line.split_once("java.lang.Thread.State:") {
            let state = state_tail
                .trim()
                .split(['(', ' '])
                .next()
                .unwrap_or("UNKNOWN");
            *summary
                .state_counts
                .entry(state.to_ascii_uppercase())
                .or_insert(0) += 1;
            summary.thread_state_entries += 1;
            if state.eq_ignore_ascii_case("BLOCKED")
                && summary.blocked_thread_names.len() < 25
                && let Some(name) = current_thread.as_ref()
                && !summary.blocked_thread_names.contains(name)
            {
                summary.blocked_thread_names.push(name.clone());
            }
        }
        if line.contains("locked <")
            || line.contains("waiting to lock <")
            || line.contains("parking to wait for")
        {
            summary.monitor_lock_mentions += 1;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("found one java-level deadlock")
            || lower.starts_with("java-level deadlock detected")
            || lower.starts_with("deadlock detected:")
        {
            summary.explicit_deadlock_marker_detected = true;
        }
    }
    summary.thread_count_estimate = summary
        .thread_headers_detected
        .max(summary.thread_state_entries);
    summary
}

fn render_thread_summary(summary: &ThreadDumpSummary) -> String {
    let states = if summary.state_counts.is_empty() {
        "- No standard JVM thread-state entries were parsed.\n".to_owned()
    } else {
        summary
            .state_counts
            .iter()
            .map(|(state, count)| format!("- **{state}:** {count}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    let blocked = if summary.blocked_thread_names.is_empty() {
        "- No BLOCKED thread names were extracted from the recognized format.\n".to_owned()
    } else {
        summary
            .blocked_thread_names
            .iter()
            .map(|name| format!("- `{}`", name.replace('`', "'")))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    format!(
        "# Thread dump summary\n\n- Bytes analyzed: {}\n- Thread headers detected: {}\n- Thread-state entries: {}\n- Estimated thread count: {}\n- Lock-related line mentions: {}\n- Explicit deadlock marker: {}\n\n## State counts\n\n{}\n## BLOCKED thread names (up to 25)\n\n{}\n## Interpretation cautions\n\n- Thread-state counts are triage signals, not root-cause proof.\n- WAITING/TIMED_WAITING can be normal.\n- A missing explicit deadlock marker does not rule out a deadlock.\n- Correlate blocked stacks, lock owners, the problem description, and additional dumps.\n",
        summary.bytes_analyzed,
        summary.thread_headers_detected,
        summary.thread_state_entries,
        summary.thread_count_estimate,
        summary.monitor_lock_mentions,
        if summary.explicit_deadlock_marker_detected {
            "detected"
        } else {
            "not detected"
        },
        states,
        blocked
    )
}

async fn write_text(
    workspace: &Path,
    workspace_relative_path: &str,
    name: &str,
    content: &str,
    purpose: &str,
    files: &mut Vec<PreparedFile>,
) -> Result<(), String> {
    let path = workspace.join(name);
    fs::write(&path, content)
        .await
        .map_err(|error| format!("Could not write prepared file '{name}': {error}"))?;
    set_private_file(&path)
        .map_err(|error| format!("Could not secure prepared file '{name}': {error}"))?;
    files.push(PreparedFile {
        name: name.to_owned(),
        relative_path: format!("{workspace_relative_path}/{name}"),
        purpose: purpose.to_owned(),
    });
    Ok(())
}

async fn write_json<T: serde::Serialize>(
    workspace: &Path,
    workspace_relative_path: &str,
    name: &str,
    value: &T,
    purpose: &str,
    files: &mut Vec<PreparedFile>,
) -> Result<(), String> {
    let content = serde_json::to_string_pretty(value)
        .map_err(|error| format!("Could not serialize prepared file '{name}': {error}"))?;
    write_text(
        workspace,
        workspace_relative_path,
        name,
        &content,
        purpose,
        files,
    )
    .await
}

fn safe_leaf(filename: &str) -> String {
    let leaf = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("diagnostic-input.bin");
    let mut safe = String::new();
    for character in leaf.chars().filter(|character| !character.is_control()) {
        if safe.len() + character.len_utf8() > 180 {
            break;
        }
        safe.push(character);
    }
    let safe = safe.trim();
    if safe.is_empty() || safe == "." || safe == ".." {
        "diagnostic-input.bin".to_owned()
    } else {
        safe.to_owned()
    }
}

fn set_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

fn set_private_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

/// Baseline artifact for a same-case heap dump comparison.
#[derive(Debug, Clone)]
pub struct HeapBaseline {
    pub artifact_id: String,
    pub filename: String,
    pub sha256: String,
}

const MAX_MAT_CONTEXT_BYTES: usize = 128 * 1024;
const MAX_MAT_REPORT_FILES: usize = 80;

/// Execute one of the fixed MAT operations inside the project-owned Docker image.
/// Returns false when the Docker engine is unavailable; this is a non-fatal preparation state.
#[allow(clippy::too_many_arguments)]
pub async fn execute_heap_operation(
    data_dir: &Path,
    case_id: &str,
    prepared: &mut PreparedPlugin,
    operation: &str,
    class_filter: Option<&str>,
    top_n: usize,
    object_id: Option<&str>,
    baseline: Option<HeapBaseline>,
) -> Result<bool, String> {
    let supported = matches!(
        operation,
        "analyze_heap_dump"
            | "generate_leak_suspects"
            | "generate_heap_overview"
            | "get_histogram"
            | "get_dominator_tree"
            | "find_paths_to_gc_roots"
            | "compare_heap_dumps"
    );
    if !supported {
        return Err(format!("Unsupported heap operation '{operation}'."));
    }
    if prepared.tool_id != "mat.heap_dump.prepare" {
        return Err("MAT operations require a prepared heap dump artifact.".to_owned());
    }
    if !(1..=500).contains(&top_n) {
        return Err("top_n must be between 1 and 500.".to_owned());
    }
    if let Some(filter) = class_filter
        && (filter.len() > 256 || filter.chars().any(char::is_control))
    {
        return Err(
            "class_filter must be at most 256 characters and contain no control characters."
                .to_owned(),
        );
    }
    let normalized_object_id = if operation == "find_paths_to_gc_roots" {
        Some(validate_object_id(object_id.ok_or_else(|| {
            "object_id is required for find_paths_to_gc_roots.".to_owned()
        })?)?)
    } else {
        None
    };
    if operation == "compare_heap_dumps" && baseline.is_none() {
        return Err("baseline_id is required for compare_heap_dumps.".to_owned());
    }
    if operation != "compare_heap_dumps" && baseline.is_some() {
        return Err("baseline_id is only valid for compare_heap_dumps.".to_owned());
    }
    prepared.tool_id = format!("mat.heap.{operation}");
    let _job_permit = mat_job_semaphore()
        .acquire_owned()
        .await
        .map_err(|_| "MAT job semaphore was closed unexpectedly.".to_owned())?;

    let workspace = data_dir
        .join("work")
        .join(case_id)
        .join("heap")
        .join(&prepared.artifact_id);
    let input_name = safe_leaf(&prepared.artifact_filename);
    let input_path = workspace.join("input").join(&input_name);
    let input_meta = fs::symlink_metadata(&input_path)
        .await
        .map_err(|_| "Prepared heap dump input is missing; retry plugin preparation.".to_owned())?;
    if !input_meta.is_file() || input_meta.file_type().is_symlink() {
        return Err("Prepared MAT input must be a regular non-symlink file.".to_owned());
    }

    let cancel_marker = workspace.join(format!(".cancel-requested-{}", prepared.tool_run_id));
    let baseline_container_path = if let Some(baseline) = baseline.as_ref() {
        let ext = Path::new(&baseline.filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "hprof" | "phd" | "heapdump") {
            return Err(
                "The baseline artifact filename must have a .hprof, .phd, or .heapdump extension."
                    .to_owned(),
            );
        }
        let source = data_dir
            .join("artifacts")
            .join(case_id)
            .join(&baseline.artifact_id)
            .join("original");
        let metadata = fs::symlink_metadata(&source)
            .await
            .map_err(|_| "Baseline heap dump file is missing from local storage.".to_owned())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Baseline artifact must be a regular non-symlink file.".to_owned());
        }
        let staged_name = format!("baseline-{}.{}", baseline.artifact_id, ext);
        let staged = workspace.join("input").join(&staged_name);
        match fs::symlink_metadata(&staged).await {
            Ok(existing) if existing.is_file() && !existing.file_type().is_symlink() && existing.len() == metadata.len() => {},
            Ok(_) => return Err("Existing staged baseline is not a regular file or its size differs from the saved artifact.".to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::hard_link(&source, &staged).await.map_err(|error| format!("Could not stage baseline dump without copying it ({error})."))?;
            }
            Err(error) => return Err(format!("Could not validate the staged baseline: {error}")),
        }
        Some(format!("/work/input/{staged_name}"))
    } else {
        None
    };

    let runtime = match detect_container_runtime().await {
        Ok(Some(runtime)) => runtime,
        Ok(None) => {
            prepared.status = "prepared".to_owned();
            prepared.warnings.push("Neither a usable Docker nor Podman runtime was found. The MAT request files were prepared, but Eclipse MAT was not executed. Install/start a supported local container engine, or set ATLAS_MAT_RUNTIME to docker or podman.".to_owned());
            prepared.summary = format!(
                "Prepared '{}' for MAT; no usable container runtime was available, so no MAT report was generated.",
                prepared.artifact_filename
            );
            write_mat_execution_manifest(
                &workspace,
                prepared,
                operation,
                "skipped_runtime_unavailable",
                &[],
                None,
                class_filter,
                baseline.as_ref(),
                "not_attempted",
            )
            .await?;
            upsert_prepared_file(
                prepared,
                &workspace,
                "mat-execution.json",
                "MAT execution status and report manifest",
            );
            return Ok(false);
        }
        Err(message) => return Err(message),
    };

    let image = ensure_mat_image(&runtime).await?;
    if fs::metadata(&cancel_marker).await.is_ok() {
        return Err("MAT job cancellation was requested before container execution; no analysis container was started.".to_owned());
    }
    // MAT creates index files beside its input. The staged files are hard links for
    // efficient preparation, so replace them with independent copies before mounting
    // the workspace writable; this prevents MAT/parser code from modifying the saved
    // source artifact through a shared inode. Copies are cached for subsequent queries.
    let source_path = data_dir
        .join("artifacts")
        .join(case_id)
        .join(&prepared.artifact_id)
        .join("original");
    let copy_marker = workspace
        .join("input")
        .join(format!(".mat-copy-{}.ready", prepared.artifact_id));
    ensure_independent_copy(
        &source_path,
        &input_path,
        &copy_marker,
        &prepared.tool_run_id,
        &prepared.artifact_sha256,
    )
    .await?;
    if let Some(baseline) = baseline.as_ref() {
        let ext = Path::new(&baseline.filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let baseline_name = format!("baseline-{}.{}", baseline.artifact_id, ext);
        let baseline_source = data_dir
            .join("artifacts")
            .join(case_id)
            .join(&baseline.artifact_id)
            .join("original");
        let baseline_staged = workspace.join("input").join(&baseline_name);
        let baseline_marker = workspace
            .join("input")
            .join(format!(".mat-copy-{}.ready", baseline.artifact_id));
        ensure_independent_copy(
            &baseline_source,
            &baseline_staged,
            &baseline_marker,
            &prepared.tool_run_id,
            &baseline.sha256,
        )
        .await?;
    }
    if fs::metadata(&cancel_marker).await.is_ok() {
        return Err(
            "MAT job cancellation was requested during staging; no analysis container was started."
                .to_owned(),
        );
    }
    let container_name = format!("atlas-mat-{}", prepared.tool_run_id.replace('-', ""));
    let log_path = workspace.join("mat-container.log");
    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&log_path)
        .map_err(|error| format!("Could not create MAT execution log: {error}"))?;
    set_private_file(&log_path)
        .map_err(|error| format!("Could not secure MAT execution log: {error}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("Could not open MAT execution log: {error}"))?;
    let memory = safe_resource_env("ATLAS_MAT_DOCKER_MEMORY", "6g", true)?;
    let cpus = safe_resource_env("ATLAS_MAT_DOCKER_CPUS", "2", false)?;
    let java_heap = safe_resource_env("ATLAS_MAT_JAVA_MAX_HEAP", "4g", true)?;
    let timeout_seconds = std::env::var("ATLAS_MAT_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(7200)
        .clamp(30, 86400);

    let mut command = Command::new(&runtime);
    command.args(["run", "--rm", "--name", &container_name]);
    // Use the runtime's default container identity; do not force host UID/GID on id-mapped mounts.
    command.args([
        "--network",
        "none",
        "--read-only",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--pids-limit",
        "256",
        "--memory",
        &memory,
        "--cpus",
        &cpus,
        "--tmpfs",
        "/tmp:rw,noexec,nosuid,size=512m",
        "--env",
        "HOME=/tmp",
        "--env",
        &format!("MAT_JAVA_MAX_HEAP={java_heap}"),
    ]);
    // MAT needs a writable isolated workspace; on SELinux hosts label only this job's workspace.
    // Do not bind-mount the persisted original artifact directory.
    let workspace_mount_source = std::fs::canonicalize(&workspace)
        .map_err(|error| format!("Could not resolve MAT workspace mount path: {error}"))?;
    let selinux_enforcing = if cfg!(target_os = "linux") {
        fs::read_to_string("/sys/fs/selinux/enforce")
            .await
            .map(|contents| contents.trim() == "1")
            .unwrap_or(false)
    } else {
        false
    };
    let mount_source = if selinux_enforcing {
        format!("{}:/work:Z", workspace_mount_source.display())
    } else {
        format!("{}:/work", workspace_mount_source.display())
    };
    command
        .arg("--volume")
        .arg(mount_source)
        .args(["--workdir", "/work"])
        .arg(&image)
        .arg(format!("/work/input/{input_name}"))
        .arg(operation);
    match operation {
        "analyze_heap_dump" | "get_dominator_tree" => {
            command.arg(top_n.to_string());
        }
        "find_paths_to_gc_roots" => {
            command.arg(normalized_object_id.as_deref().unwrap_or_default());
        }
        "compare_heap_dumps" => {
            command.arg(baseline_container_path.as_deref().unwrap_or_default());
        }
        _ => {}
    }
    command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start MAT container: {error}"))?;
    let exit_status = match timeout(Duration::from_secs(timeout_seconds), child.wait()).await {
        Ok(result) => result
            .map_err(|error| format!("Failed while waiting for the MAT container: {error}"))?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = Command::new(&runtime)
                .args(["rm", "--force", &container_name])
                .output()
                .await;
            let tail = read_file_tail(&log_path, 5000).await.unwrap_or_default();
            return Err(format!(
                "MAT operation '{operation}' exceeded the {timeout_seconds}s timeout. Container cleanup was requested. Log tail: {tail}"
            ));
        }
    };
    if !exit_status.success() {
        let tail = read_file_tail(&log_path, 7000).await.unwrap_or_default();
        return Err(format!(
            "MAT container failed with exit status {exit_status}. Log tail: {tail}"
        ));
    }

    let baseline_path = baseline_container_path.as_deref();
    let report_paths = collect_report_paths(&workspace, &input_path, baseline_path).await?;
    let mut report_relative_paths = Vec::new();
    for report in &report_paths {
        let rel = report
            .strip_prefix(&workspace)
            .unwrap_or(report)
            .to_string_lossy()
            .replace('\\', "/");
        report_relative_paths.push(rel.clone());
        upsert_prepared_file_by_path(
            prepared,
            &workspace,
            report,
            "Eclipse MAT report or query output",
        );
    }
    upsert_prepared_file(
        prepared,
        &workspace,
        "mat-container.log",
        "Eclipse MAT container log",
    );

    if let Some(filter) = class_filter.filter(|value| !value.trim().is_empty()) {
        let filtered = filter_histogram_reports(&workspace, &report_paths, filter).await?;
        if filtered > 0 {
            upsert_prepared_file(
                prepared,
                &workspace,
                "histogram-filtered.txt",
                "Local filtered subset of MAT histogram output",
            );
        } else {
            prepared.warnings.push(format!("No histogram text lines matched class_filter '{filter}'. The unfiltered MAT histogram remains available."));
        }
    }

    let _context_path = create_llm_context(
        &workspace,
        &prepared.artifact_filename,
        operation,
        &report_paths,
        class_filter,
        baseline.as_ref(),
    )
    .await?;
    upsert_prepared_file(
        prepared,
        &workspace,
        "llm-context.md",
        "Bounded text report excerpts prepared for optional LLM analysis; raw heap dump excluded",
    );
    let llm_analysis_status = "deferred_to_agent_chat";
    write_mat_execution_manifest(
        &workspace,
        prepared,
        operation,
        "succeeded",
        &report_relative_paths,
        Some("llm-context.md"),
        class_filter,
        baseline.as_ref(),
        llm_analysis_status,
    )
    .await?;
    upsert_prepared_file(
        prepared,
        &workspace,
        "mat-execution.json",
        "MAT execution status and report manifest",
    );

    prepared.status = "succeeded".to_owned();
    prepared.tool_id = format!("mat.heap.{operation}");
    prepared.summary = format!(
        "Eclipse MAT operation '{operation}' completed and generated {} report/query file(s). A bounded `llm-context.md` was prepared for the separate local agent-chat request. Raw heap files and MAT index files were not included.",
        report_paths.len()
    );
    let readme = format!(
        "# Eclipse MAT analysis results\n\n- Input: `input/{input_name}` (isolated copy; the saved source artifact remains untouched).\n- Operation: `{operation}`\n- MAT image: `{image}`\n- Result context for optional LLM analysis: `llm-context.md`\n- Execution manifest: `mat-execution.json`\n- Container log: `mat-container.log`\n\nReports were generated by the local Docker engine with network access disabled, a read-only container root filesystem, dropped Linux capabilities, and configured memory/CPU/time limits. Raw heap dumps and MAT indexes must not be sent to an LLM. The LLM context contains bounded excerpts from text/CSV reports only.\n"
    );
    fs::write(workspace.join("README.md"), readme)
        .await
        .map_err(|error| format!("Could not update MAT README: {error}"))?;
    upsert_prepared_file(
        prepared,
        &workspace,
        "README.md",
        "How to consume MAT output and LLM context",
    );
    Ok(true)
}

/// Stream a file through SHA-256 without loading a heap dump into memory.
async fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .await
        .map_err(|error| format!("Could not open file for SHA-256 verification: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("Could not stream file for SHA-256 verification: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Verify both saved and staged bytes; markers alone are never accepted as proof.
async fn ensure_independent_copy(
    source: &Path,
    staged: &Path,
    marker: &Path,
    run_id: &str,
    expected_sha256: &str,
) -> Result<(), String> {
    let source_meta = fs::symlink_metadata(source)
        .await
        .map_err(|error| format!("Could not inspect saved heap input: {error}"))?;
    if !source_meta.is_file() || source_meta.file_type().is_symlink() {
        return Err("Saved heap input must be a regular non-symlink file.".to_owned());
    }
    let staged_meta = fs::symlink_metadata(staged)
        .await
        .map_err(|error| format!("Could not inspect staged heap input: {error}"))?;
    if !staged_meta.is_file() || staged_meta.file_type().is_symlink() {
        return Err("Staged heap input must be a regular non-symlink file.".to_owned());
    }
    if staged_meta.len() != source_meta.len() {
        return Err("Staged heap input size differs from the saved artifact.".to_owned());
    }
    let normalized_expected = expected_sha256.trim().to_ascii_lowercase();
    if normalized_expected.len() != 64
        || !normalized_expected.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(
            "Saved artifact SHA-256 is missing or invalid; refusing to stage unverified bytes."
                .to_owned(),
        );
    }
    let source_hash = sha256_file(source).await?;
    if source_hash != normalized_expected {
        return Err("Saved artifact bytes no longer match the SHA-256 recorded at upload; refusing MAT execution.".to_owned());
    }
    let marker_text = fs::read_to_string(marker).await.unwrap_or_default();
    if staged_meta.len() == source_meta.len()
        && marker_text
            .lines()
            .any(|line| line == format!("sha256={normalized_expected}"))
    {
        let staged_hash = sha256_file(staged).await?;
        if staged_hash == normalized_expected {
            return Ok(());
        }
    }
    let temp = staged.with_file_name(format!(".mat-copy-{run_id}.part"));
    let copied = match fs::copy(source, &temp).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = fs::remove_file(&temp).await;
            return Err(format!(
                "Could not create the isolated MAT input copy (needs roughly {} additional bytes of free disk space): {error}",
                source_meta.len()
            ));
        }
    };
    if copied != source_meta.len() {
        let _ = fs::remove_file(&temp).await;
        return Err(
            "The isolated MAT input copy has a different size than the saved artifact.".to_owned(),
        );
    }
    let copied_hash = sha256_file(&temp).await?;
    if copied_hash != normalized_expected {
        let _ = fs::remove_file(&temp).await;
        return Err(
            "Isolated MAT copy SHA-256 does not match the saved artifact; refusing analysis."
                .to_owned(),
        );
    }
    if let Err(error) = set_private_file(&temp) {
        let _ = fs::remove_file(&temp).await;
        return Err(format!("Could not secure isolated MAT input copy: {error}"));
    }
    if let Err(error) = fs::remove_file(staged).await {
        let _ = fs::remove_file(&temp).await;
        return Err(format!(
            "Could not replace the staged hard link with an isolated copy: {error}"
        ));
    }
    if let Err(error) = fs::rename(&temp, staged).await {
        let _ = fs::remove_file(&temp).await;
        return Err(format!(
            "Could not activate isolated MAT input copy: {error}"
        ));
    }
    let note = format!(
        "schema_version=1\nsize_bytes={}\nsha256={}\n",
        source_meta.len(),
        normalized_expected
    );
    fs::write(marker, note)
        .await
        .map_err(|error| format!("Could not write MAT input copy marker: {error}"))?;
    set_private_file(marker)
        .map_err(|error| format!("Could not secure MAT input copy marker: {error}"))?;
    Ok(())
}

pub(crate) fn validate_object_id(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if hex.is_empty() || hex.len() > 16 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(
            "object_id must be a hexadecimal MAT object address (for example 0x7f1234ab)."
                .to_owned(),
        );
    }
    Ok(format!("0x{}", hex.to_ascii_lowercase()))
}

fn safe_resource_env(name: &str, default: &str, allow_unit: bool) -> Result<String, String> {
    let value = std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned());
    let valid = if allow_unit {
        let number = value.trim_end_matches(['k', 'K', 'm', 'M', 'g', 'G']);
        !number.is_empty()
            && number.parse::<u64>().is_ok()
            && value
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, 'k' | 'K' | 'm' | 'M' | 'g' | 'G'))
    } else {
        value
            .parse::<f64>()
            .is_ok_and(|n| n.is_finite() && n > 0.0 && n <= 64.0)
            && value.chars().all(|c| c.is_ascii_digit() || c == '.')
    };
    if !valid {
        return Err(format!("{name} has an invalid resource limit value."));
    }
    Ok(value)
}

async fn detect_container_runtime() -> Result<Option<String>, String> {
    let preference = std::env::var("ATLAS_MAT_RUNTIME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "auto".to_owned())
        .to_ascii_lowercase();
    // `podman-docker` can provide a `docker` compatibility command. Detect that
    // shim so Podman-specific ownership and SELinux mount options are still used.
    let docker_is_podman_shim = if preference == "auto" {
        match timeout(
            Duration::from_secs(5),
            Command::new("docker").arg("--version").output(),
        )
        .await
        {
            Ok(Ok(output)) => {
                let combined = format!(
                    "{} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
                .to_ascii_lowercase();
                combined.contains("podman")
            }
            _ => false,
        }
    } else {
        false
    };
    let candidates: &[&str] = match preference.as_str() {
        "auto" if docker_is_podman_shim => &["podman"],
        "auto" => &["docker", "podman"],
        "docker" => &["docker"],
        "podman" => &["podman"],
        _ => return Err("ATLAS_MAT_RUNTIME must be 'auto', 'docker', or 'podman'.".to_owned()),
    };

    for runtime in candidates {
        // Calling the engine, not just looking for its binary, confirms it is usable.
        // A missing CLI or stopped daemon is treated as a candidate miss, not a panic.
        let result = timeout(
            Duration::from_secs(8),
            Command::new(*runtime).arg("info").output(),
        )
        .await;
        if matches!(result, Ok(Ok(ref output)) if output.status.success()) {
            return Ok(Some((*runtime).to_owned()));
        }
    }
    Ok(None)
}

async fn ensure_mat_image(runtime: &str) -> Result<String, String> {
    let image = std::env::var("ATLAS_MAT_IMAGE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "middleware-atlas/mat:1.17.0".to_owned());
    if image.len() > 255
        || !image
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'))
    {
        return Err("ATLAS_MAT_IMAGE is not a valid simple container image reference.".to_owned());
    }
    let inspected = timeout(
        Duration::from_secs(10),
        Command::new(runtime)
            .args(["image", "inspect", &image])
            .output(),
    )
    .await;
    if matches!(inspected, Ok(Ok(ref output)) if output.status.success()) {
        return Ok(image);
    }
    let build_context = if let Some(value) = std::env::var("ATLAS_MAT_BUILD_CONTEXT")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        PathBuf::from(value)
    } else {
        let project_root = if let Some(value) = std::env::var("ATLAS_PROJECT_DIR")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            PathBuf::from(value)
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
        };
        project_root.join("tools/mat")
    };
    if !build_context.join("Dockerfile").is_file() || !build_context.join("mat-job").is_file() {
        return Err(format!(
            "MAT image '{image}' is missing and build context '{}' does not contain Dockerfile and mat-job. Build it from tools/mat or set ATLAS_MAT_BUILD_CONTEXT.",
            build_context.display()
        ));
    }
    let result = timeout(
        Duration::from_secs(1800),
        Command::new(runtime)
            .args(["build", "--pull", "--quiet", "--tag", &image])
            .arg(&build_context)
            .output(),
    )
    .await
    .map_err(|_| "Building the pinned Eclipse MAT image exceeded the 1800s timeout.".to_owned())?
    .map_err(|error| format!("Could not invoke container runtime '{runtime}' build: {error}"))?;
    if !result.status.success() {
        let detail = String::from_utf8_lossy(&result.stderr);
        return Err(format!(
            "Could not build Eclipse MAT image '{image}' with {runtime}. Check network access and the official Eclipse download; build output: {}",
            detail.chars().take(3000).collect::<String>()
        ));
    }
    Ok(image)
}

async fn read_file_tail(path: &Path, max_bytes: usize) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .await
        .map_err(|error| error.to_string())?;
    let length = file
        .metadata()
        .await
        .map_err(|error| error.to_string())?
        .len();
    let start = length.saturating_sub(max_bytes as u64);
    file.seek(SeekFrom::Start(start))
        .await
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(max_bytes.min(length as usize));
    file.take(max_bytes as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn collect_report_paths(
    workspace: &Path,
    input_path: &Path,
    baseline_container_path: Option<&str>,
) -> Result<Vec<PathBuf>, String> {
    let mut results = Vec::new();
    let baseline_name = baseline_container_path.and_then(|value| value.rsplit('/').next());
    collect_report_paths_inner(workspace, input_path, baseline_name, &mut results, 0).await?;
    results.sort();
    results.truncate(MAX_MAT_REPORT_FILES);
    Ok(results)
}

fn collect_report_paths_inner<'a>(
    directory: &'a Path,
    input_path: &'a Path,
    baseline_name: Option<&'a str>,
    results: &'a mut Vec<PathBuf>,
    depth: usize,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
    Box::pin(async move {
        if depth > 10 || results.len() >= MAX_MAT_REPORT_FILES {
            return Ok(());
        }
        let mut entries = fs::read_dir(directory)
            .await
            .map_err(|error| format!("Could not scan MAT outputs: {error}"))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| error.to_string())?
        {
            if results.len() >= MAX_MAT_REPORT_FILES {
                break;
            }
            let path = entry.path();
            let file_type = entry.file_type().await.map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                collect_report_paths_inner(&path, input_path, baseline_name, results, depth + 1)
                    .await?;
                continue;
            }
            if !file_type.is_file() || path == input_path {
                continue;
            }
            let leaf = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
            if baseline_name == Some(leaf) {
                continue;
            }
            let extension = path
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(extension.as_str(), "txt" | "csv" | "html" | "htm" | "zip") {
                let metadata = entry.metadata().await.map_err(|error| error.to_string())?;
                if metadata.len() <= 100 * 1024 * 1024 {
                    results.push(path);
                }
            }
        }
        Ok(())
    })
}

async fn filter_histogram_reports(
    workspace: &Path,
    report_paths: &[PathBuf],
    filter: &str,
) -> Result<usize, String> {
    let needle = filter.to_ascii_lowercase();
    let mut matches = Vec::new();
    for path in report_paths {
        if path.extension().and_then(|v| v.to_str()).unwrap_or("") != "txt" {
            continue;
        }
        let text = fs::read_to_string(path).await.unwrap_or_default();
        if !text.to_ascii_lowercase().contains("histogram")
            && !path
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains("query")
        {
            continue;
        }
        for line in text.lines() {
            if line.to_ascii_lowercase().contains(&needle) && matches.len() < 2000 {
                matches.push(line.to_owned());
            }
        }
    }
    if matches.is_empty() {
        return Ok(0);
    }
    let content = format!(
        "# Filtered MAT histogram\n\nClass filter: `{}`\nMatched lines: {}\n\n{}\n",
        filter.replace('`', "'"),
        matches.len(),
        matches.join("\n")
    );
    fs::write(workspace.join("histogram-filtered.txt"), content)
        .await
        .map_err(|error| format!("Could not write filtered histogram: {error}"))?;
    set_private_file(&workspace.join("histogram-filtered.txt"))
        .map_err(|error| error.to_string())?;
    Ok(matches.len())
}

fn read_zip_report_excerpts(path: &Path) -> Result<Vec<(String, String)>, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("Could not open MAT report archive: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("Could not inspect MAT report archive: {error}"))?;
    let mut output = Vec::new();
    for index in 0..archive.len().min(128) {
        if output.len() >= 20 {
            break;
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Could not read MAT report archive entry: {error}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        let extension = Path::new(&name)
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "txt" | "csv" | "html" | "htm") {
            continue;
        }
        let mut bytes = Vec::with_capacity(12 * 1024);
        std::io::Read::by_ref(&mut entry)
            .take(12 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not read bounded report excerpt from ZIP: {error}"))?;
        let raw = String::from_utf8_lossy(&bytes);
        let excerpt = if matches!(extension.as_str(), "html" | "htm") {
            clean_html_excerpt(&raw)
        } else {
            raw.to_string()
        };
        if !excerpt.trim().is_empty() {
            let safe_name = name
                .chars()
                .filter(|c| !c.is_control())
                .take(180)
                .collect::<String>();
            output.push((safe_name, excerpt));
        }
    }
    Ok(output)
}

async fn create_llm_context(
    workspace: &Path,
    artifact_filename: &str,
    operation: &str,
    report_paths: &[PathBuf],
    class_filter: Option<&str>,
    baseline: Option<&HeapBaseline>,
) -> Result<PathBuf, String> {
    let problem = fs::read_to_string(workspace.join("problem-description.md"))
        .await
        .unwrap_or_else(|_| "No problem description was supplied.".to_owned());
    let mut output = format!(
        "# Bounded MAT evidence context\n\n## Tool operation\n\n`{operation}`\n\n## Artifact\n\n`{artifact_filename}`\n\n## Problem description\n\n{problem}\n\n## Evidence excerpts\n\nOnly bounded text/CSV excerpts from MAT-generated reports are included here. Raw heap dumps and MAT index/cache files are excluded. Treat these excerpts as evidence, not as a confirmed root cause.\n"
    );
    if let Some(filter) = class_filter {
        output.push_str(&format!(
            "\n## Requested class filter\n\n`{}`\n",
            filter.replace('`', "'")
        ));
    }
    if let Some(baseline) = baseline {
        output.push_str(&format!(
            "\n## Comparison baseline\n\n- Artifact ID: `{}`\n- Filename: `{}`\n- SHA-256: `{}`\n",
            baseline.artifact_id, baseline.filename, baseline.sha256
        ));
    }
    let mut included = 0usize;
    for path in report_paths {
        if output.len() >= MAX_MAT_CONTEXT_BYTES || included >= 20 {
            break;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(extension.as_str(), "txt" | "csv") {
            let report_file = fs::File::open(path)
                .await
                .map_err(|error| format!("Could not open MAT text report: {error}"))?;
            let mut bytes = Vec::with_capacity(50 * 1024);
            report_file
                .take(50 * 1024)
                .read_to_end(&mut bytes)
                .await
                .map_err(|error| format!("Could not read bounded MAT text report: {error}"))?;
            let text = String::from_utf8_lossy(&bytes);
            let mut excerpt = text.to_string();
            if operation == "get_histogram"
                && let Some(filter) = class_filter.filter(|value| !value.trim().is_empty())
            {
                let lines = excerpt
                    .lines()
                    .filter(|line| {
                        line.to_ascii_lowercase()
                            .contains(&filter.to_ascii_lowercase())
                    })
                    .take(100)
                    .collect::<Vec<_>>();
                if !lines.is_empty() {
                    excerpt = lines.join("\n");
                }
            }
            let available = MAX_MAT_CONTEXT_BYTES.saturating_sub(output.len());
            if available <= 128 {
                break;
            }
            let bounded = excerpt
                .chars()
                .take(available.saturating_sub(128))
                .collect::<String>();
            if !bounded.trim().is_empty() {
                let label = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("report");
                output.push_str(&format!("\n\n### {}\n\n{}", label, bounded));
                included += 1;
            }
        } else if extension == "zip" {
            let zip_path = path.to_path_buf();
            let excerpts = tokio::task::spawn_blocking(move || read_zip_report_excerpts(&zip_path))
                .await
                .map_err(|error| format!("MAT ZIP extraction worker failed: {error}"))??;
            for (label, excerpt) in excerpts {
                if output.len() >= MAX_MAT_CONTEXT_BYTES || included >= 20 {
                    break;
                }
                let available = MAX_MAT_CONTEXT_BYTES.saturating_sub(output.len());
                if available <= 128 {
                    break;
                }
                let bounded = excerpt
                    .chars()
                    .take(available.saturating_sub(128))
                    .collect::<String>();
                output.push_str(&format!(
                    "\n\n### MAT archive excerpt: {}!{}\n\n{}",
                    path.file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or("report.zip"),
                    label,
                    bounded
                ));
                included += 1;
            }
        }
    }
    if included == 0 {
        output.push_str("\n\nNo readable text/CSV/HTML report excerpts were found in the report files returned by MAT. The model must not infer findings from the raw dump and should ask for specific missing evidence if needed.\n");
    }
    let path = workspace.join("llm-context.md");
    fs::write(&path, output)
        .await
        .map_err(|error| format!("Could not write bounded MAT LLM context: {error}"))?;
    set_private_file(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

fn clean_html_excerpt(input: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    let mut last_space = false;
    for character in input.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            '\n' | '\r' | '\t' => {
                if !last_space {
                    output.push(' ');
                }
                last_space = true;
            }
            value if value.is_whitespace() => {
                if !last_space {
                    output.push(' ');
                }
                last_space = true;
            }
            value => {
                output.push(value);
                last_space = false;
            }
        }
        if output.len() >= 12 * 1024 {
            break;
        }
    }
    output
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[allow(clippy::too_many_arguments)]
async fn write_mat_execution_manifest(
    workspace: &Path,
    prepared: &PreparedPlugin,
    operation: &str,
    execution_status: &str,
    report_paths: &[String],
    context_file: Option<&str>,
    class_filter: Option<&str>,
    baseline: Option<&HeapBaseline>,
    llm_analysis_status: &str,
) -> Result<(), String> {
    let value = json!({
        "schema_version": "1",
        "tool_run_id": &prepared.tool_run_id,
        "tool_id": format!("mat.heap.{operation}"),
        "operation": operation,
        "execution_status": execution_status,
        "artifact_id": &prepared.artifact_id,
        "artifact_filename": &prepared.artifact_filename,
        "artifact_sha256": &prepared.artifact_sha256,
        "mat_image": std::env::var("ATLAS_MAT_IMAGE").unwrap_or_else(|_| "middleware-atlas/mat:1.17.0".to_owned()),
        "reports": report_paths,
        "llm_context_file": context_file,
        "llm_analysis_status": llm_analysis_status,
        "llm_analysis_file": if llm_analysis_status == "succeeded" { Some("llm-analysis.md") } else { None::<&str> },
        "class_filter": class_filter,
        "baseline": baseline.map(|b| json!({"artifact_id":b.artifact_id,"filename":b.filename,"sha256":b.sha256})),
        "raw_heap_dump_sent_to_model": false,
        "bounded_report_context_sent_to_model": llm_analysis_status == "succeeded",
    });
    let path = workspace.join("mat-execution.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?,
    )
    .await
    .map_err(|error| format!("Could not write MAT execution manifest: {error}"))?;
    set_private_file(&path).map_err(|error| error.to_string())?;
    let operations = json!({
        "schema_version": "1",
        "execution_status": execution_status,
        "requested_operation": operation,
        "reports": report_paths,
        "llm_context_file": context_file,
        "llm_analysis_status": llm_analysis_status,
        "class_filter": class_filter,
        "baseline_artifact_id": baseline.map(|item| item.artifact_id.as_str()),
        "note": "Raw heap dumps and MAT indexes are never sent to the model. The local agent-chat handler may attach bounded llm-context.md/report excerpts when ATLAS_LLM_SEND_HEAP_CONTEXT is enabled.",
    });
    let operations_path = workspace.join("mat-operations.json");
    fs::write(
        &operations_path,
        serde_json::to_vec_pretty(&operations).map_err(|error| error.to_string())?,
    )
    .await
    .map_err(|error| format!("Could not update MAT operations manifest: {error}"))?;
    set_private_file(&operations_path).map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn record_mat_failure(
    data_dir: &Path,
    case_id: &str,
    prepared: &mut PreparedPlugin,
    operation: &str,
) -> Result<(), String> {
    let workspace = data_dir
        .join("work")
        .join(case_id)
        .join("heap")
        .join(&prepared.artifact_id);
    write_mat_execution_manifest(
        &workspace,
        prepared,
        operation,
        "failed",
        &[],
        None,
        None,
        None,
        "not_attempted",
    )
    .await?;
    upsert_prepared_file(
        prepared,
        &workspace,
        "mat-execution.json",
        "Failed MAT execution manifest and error status",
    );
    upsert_prepared_file(
        prepared,
        &workspace,
        "mat-operations.json",
        "MAT operation status manifest",
    );
    if fs::metadata(workspace.join("mat-container.log"))
        .await
        .is_ok()
    {
        upsert_prepared_file(
            prepared,
            &workspace,
            "mat-container.log",
            "Eclipse MAT container log",
        );
    }
    Ok(())
}

/// Serializes operations for the same saved heap artifact so cached MAT indexes cannot race.
pub fn heap_artifact_lock(case_id: &str, artifact_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let key = format!("{case_id}:{artifact_id}");
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = locks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.retain(|_, weak| weak.strong_count() > 0);
    if let Some(existing) = guard.get(&key).and_then(Weak::upgrade) {
        return existing;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    guard.insert(key, Arc::downgrade(&lock));
    lock
}

/// Request termination of the MAT container corresponding to a persisted job UUID.
/// This is best-effort: the job row remains authoritative and the worker finalizes its state.
pub async fn cancel_mat_job(job_id: &str) {
    let container_name = format!("atlas-mat-{}", job_id.replace('-', ""));
    if let Ok(Some(runtime)) = detect_container_runtime().await {
        let _ = timeout(
            Duration::from_secs(10),
            Command::new(runtime)
                .args(["rm", "--force", &container_name])
                .output(),
        )
        .await;
    }
}

fn mat_job_semaphore() -> Arc<tokio::sync::Semaphore> {
    static SEMAPHORE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    Arc::clone(SEMAPHORE.get_or_init(|| {
        let concurrency = std::env::var("ATLAS_MAT_MAX_CONCURRENT_JOBS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, 8);
        Arc::new(tokio::sync::Semaphore::new(concurrency))
    }))
}

fn upsert_prepared_file(
    prepared: &mut PreparedPlugin,
    workspace: &Path,
    relative: &str,
    purpose: &str,
) {
    let path = workspace.join(relative);
    upsert_prepared_file_by_path(prepared, workspace, &path, purpose);
}

fn upsert_prepared_file_by_path(
    prepared: &mut PreparedPlugin,
    workspace: &Path,
    path: &Path,
    purpose: &str,
) {
    let relative = path
        .strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("output")
        .to_owned();
    let item = PreparedFile {
        name,
        relative_path: relative.clone(),
        purpose: purpose.to_owned(),
    };
    if let Some(existing) = prepared
        .files
        .iter_mut()
        .find(|file| file.relative_path == relative)
    {
        *existing = item;
    } else {
        prepared.files.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::{safe_leaf, summarize_thread_dump};

    #[test]
    fn summarizes_standard_hotspot_thread_states() {
        let dump = r#""Worker-1" #41 prio=5 os_prio=0 tid=0x1 nid=0x1 waiting for monitor entry
   java.lang.Thread.State: BLOCKED (on object monitor)
        at example.Service.call(Service.java:42)
"Worker-2" #42 prio=5 os_prio=0 tid=0x2 nid=0x2 waiting on condition
   java.lang.Thread.State: WAITING (parking)
Found one Java-level deadlock:
"#;
        let summary = summarize_thread_dump(dump, dump.len() as u64);
        assert_eq!(summary.thread_state_entries, 2);
        assert_eq!(summary.state_counts.get("BLOCKED"), Some(&1));
        assert_eq!(summary.state_counts.get("WAITING"), Some(&1));
        assert!(summary.explicit_deadlock_marker_detected);
        assert_eq!(summary.blocked_thread_names, vec!["Worker-1"]);
    }

    #[test]
    fn input_name_is_reduced_to_a_leaf() {
        assert_eq!(safe_leaf("../../logs/thread-dump.txt"), "thread-dump.txt");
        assert_eq!(safe_leaf(".."), "diagnostic-input.bin");
    }
}
