//! Bounded, local-only evidence selection for the Ollama chat context.
//! This module never reads the original heap dump or MAT index/cache files.
use std::io::Read;
use std::path::{Path as FsPath, PathBuf};

pub const MAX_AGENT_CONTEXT_BYTES: usize = 48 * 1024;
const MAX_AGENT_CONTEXT_FILES: usize = 24;
const MAX_AGENT_FILE_EXCERPT_BYTES: usize = 12 * 1024;
const MAX_AGENT_ZIP_ENTRY_EXCERPT_BYTES: u64 = 12 * 1024;

pub fn utf8_prefix(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    let mut end = 0usize;
    for (index, character) in input.char_indices() {
        let next = index + character.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    &input[..end]
}

fn is_agent_context_markdown(name: &str) -> bool {
    matches!(
        name,
        "llm-context.md"
            | "thread-dump-context.md"
            | "thread-dump-summary.md"
            | "thread-dump-summary.json"
            | "problem-description.md"
            | "llm-analysis.md"
    )
}

fn clean_html_excerpt(input: &str) -> String {
    let mut output = String::with_capacity(input.len().min(MAX_AGENT_FILE_EXCERPT_BYTES));
    let mut in_tag = false;
    let mut previous_space = false;
    for character in input.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            '\n' | '\r' | '\t' => {
                if !previous_space {
                    output.push(' ');
                }
                previous_space = true;
            }
            value if value.is_whitespace() => {
                if !previous_space {
                    output.push(' ');
                }
                previous_space = true;
            }
            value => {
                output.push(value);
                previous_space = false;
            }
        }
        if output.len() >= MAX_AGENT_FILE_EXCERPT_BYTES {
            break;
        }
    }
    output
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn append_mat_zip_excerpt(
    path: &FsPath,
    relative: &str,
    output: &mut String,
    file_count: &mut usize,
) -> Result<(), String> {
    if output.len() >= MAX_AGENT_CONTEXT_BYTES || *file_count >= MAX_AGENT_CONTEXT_FILES {
        return Ok(());
    }
    let file = std::fs::File::open(path).map_err(|error| {
        format!(
            "Could not open MAT report archive {}: {error}",
            path.display()
        )
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        format!(
            "Could not inspect MAT report archive {}: {error}",
            path.display()
        )
    })?;
    let entry_limit = archive.len().min(128);
    for index in 0..entry_limit {
        if output.len() >= MAX_AGENT_CONTEXT_BYTES || *file_count >= MAX_AGENT_CONTEXT_FILES {
            break;
        }
        let mut entry = archive.by_index(index).map_err(|error| {
            format!(
                "Could not read entry {index} in MAT report archive {}: {error}",
                path.display()
            )
        })?;
        if entry.is_dir() {
            continue;
        }
        let entry_name = entry.name().to_owned();
        let extension = FsPath::new(&entry_name)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "txt" | "csv" | "html" | "htm") {
            continue;
        }
        let mut bytes = Vec::with_capacity(MAX_AGENT_FILE_EXCERPT_BYTES);
        let _ = std::io::Read::by_ref(&mut entry)
            .take(MAX_AGENT_ZIP_ENTRY_EXCERPT_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                format!(
                    "Could not extract bounded text from {}!{entry_name}: {error}",
                    path.display()
                )
            })?;
        let text = String::from_utf8_lossy(&bytes);
        let excerpt = if matches!(extension.as_str(), "html" | "htm") {
            clean_html_excerpt(&text)
        } else {
            text.to_string()
        };
        let excerpt = excerpt.trim();
        if excerpt.is_empty() {
            continue;
        }
        let label = entry_name
            .chars()
            .filter(|character| !character.is_control())
            .take(180)
            .collect::<String>();
        let header = format!("\n\n### MAT report excerpt: {relative}!{label}\n\n");
        let available = MAX_AGENT_CONTEXT_BYTES.saturating_sub(output.len() + header.len());
        if available == 0 {
            break;
        }
        let bounded = utf8_prefix(excerpt, available);
        output.push_str(&header);
        output.push_str(bounded);
        *file_count += 1;
    }
    Ok(())
}

pub fn collect_agent_context_sync(case_work: &FsPath) -> Result<String, String> {
    if !case_work.exists() {
        return Ok(String::new());
    }
    let mut entries: Vec<PathBuf> = Vec::new();
    let mut pending = vec![(case_work.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if depth > 8 || entries.len() >= 300 {
            continue;
        }
        let read_dir = match std::fs::read_dir(&directory) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "Could not scan local diagnostic workspace: {error}"
                ));
            }
        };
        for entry in read_dir {
            let entry = entry.map_err(|error| {
                format!("Could not inspect local diagnostic workspace: {error}")
            })?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("Could not inspect local diagnostic item: {error}"))?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                pending.push((path, depth + 1));
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let is_mat_report_zip = extension == "zip"
                && [
                    "_leak_suspects",
                    "_query",
                    "_system_overview",
                    "_top_components",
                    "_compare",
                ]
                .iter()
                .any(|token| name.to_ascii_lowercase().contains(token));
            if is_agent_context_markdown(name) || name == "mat-execution.json" || is_mat_report_zip
            {
                entries.push(path);
            }
        }
    }
    entries.sort();
    let mut output = String::new();
    let mut count = 0usize;
    for path in entries {
        if output.len() >= MAX_AGENT_CONTEXT_BYTES || count >= MAX_AGENT_CONTEXT_FILES {
            break;
        }
        let relative = path
            .strip_prefix(case_work)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .eq_ignore_ascii_case("zip")
        {
            append_mat_zip_excerpt(&path, &relative, &mut output, &mut count)?;
            continue;
        }
        let metadata = std::fs::metadata(&path).map_err(|error| {
            format!("Could not inspect context file {}: {error}", path.display())
        })?;
        if metadata.len() > 2 * 1024 * 1024 {
            continue;
        }
        let mut file = std::fs::File::open(&path)
            .map_err(|error| format!("Could not open context file {}: {error}", path.display()))?;
        let mut bytes = Vec::with_capacity(MAX_AGENT_FILE_EXCERPT_BYTES);
        std::io::Read::by_ref(&mut file)
            .take(MAX_AGENT_FILE_EXCERPT_BYTES as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not read context file {}: {error}", path.display()))?;
        let content = String::from_utf8_lossy(&bytes).into_owned();
        if content
            .to_ascii_lowercase()
            .contains("no text/csv report excerpts were found")
        {
            continue;
        }
        let header = format!("\n\n## Local evidence file: {relative}\n\n");
        let available = MAX_AGENT_CONTEXT_BYTES.saturating_sub(output.len() + header.len());
        if available == 0 {
            break;
        }
        let bounded = utf8_prefix(&content, available);
        if !bounded.trim().is_empty() {
            output.push_str(&header);
            output.push_str(bounded);
            count += 1;
        }
    }
    Ok(utf8_prefix(&output, MAX_AGENT_CONTEXT_BYTES).to_owned())
}
