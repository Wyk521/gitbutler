//! Safe, offline scanning of the current worktree.

use crate::model::WorktreeSummary;
use anyhow::{Context as _, Result};
use bstr::{BString, ByteSlice};
use std::collections::HashMap;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::Path;

/// Worktree paths which are dependency, build or cache output by convention.
const EXCLUDED_DIRECTORIES: &[&str] = &[
    ".git",
    ".gitbutler",
    "node_modules",
    "vendor",
    "dist",
    "build",
    "target",
    "out",
    "bin",
    "obj",
    ".venv",
    "venv",
    "coverage",
    ".idea",
    ".gradle",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    "pytest-of-root",
    "logs",
    ".next",
    ".nuxt",
    ".cache",
    ".turbo",
    "__pycache__",
];

/// Lock and generated files that should not affect source-language totals.
const EXCLUDED_FILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "Cargo.lock",
    "composer.lock",
    "Gemfile.lock",
    "packages.lock.json",
    "Podfile.lock",
    "Cartfile.resolved",
    "mix.lock",
    "shrinkwrap.yaml",
];

/// Options for a worktree scan.
#[derive(Debug, Clone)]
pub struct WorktreeScanOptions {
    /// Maximum bytes read from one text file.
    pub max_file_bytes: u64,
}

impl Default for WorktreeScanOptions {
    fn default() -> Self {
        Self {
            max_file_bytes: 20_000_000,
        }
    }
}

/// One included text file in the worktree.
#[derive(Debug, Clone)]
pub struct WorktreeFile {
    /// Relative path using `/` separators.
    pub path: String,
    /// Original path bytes when the platform path was not valid UTF-8.
    pub raw_path_hex: Option<String>,
    /// Detected language.
    pub language: Option<String>,
    /// Whether the path is present in the Git index.  This is populated by
    /// the history layer; filesystem scans intentionally do not run Git.
    pub tracked: bool,
    /// Number of source lines.
    pub lines: u64,
    /// File size in bytes.
    pub bytes: u64,
}

/// The complete worktree scan and its explicit skip counters.
#[derive(Debug, Clone, Default)]
pub struct WorktreeScan {
    /// Included text files.
    pub files: Vec<WorktreeFile>,
    /// Summary counters for UI diagnostics.
    pub summary: WorktreeSummary,
}

/// Scan the real filesystem below `root` without following symbolic links.
///
/// Git ignore rules are intentionally not applied: ignored source files are
/// part of RepoScope's current-code view.  Dependency/build/cache directories
/// and lock files remain excluded to preserve the previous RepoScope totals.
pub fn scan_worktree(root: impl AsRef<Path>, options: WorktreeScanOptions) -> Result<WorktreeScan> {
    let root = root.as_ref();
    let metadata =
        fs::metadata(root).with_context(|| format!("读取工作目录失败: {}", root.display()))?;
    if !metadata.is_dir() {
        return Ok(WorktreeScan::default());
    }
    let mut result = WorktreeScan::default();
    let mut stack = vec![root.to_owned()];
    while let Some(directory) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(path = %directory.display(), ?error, "跳过无法读取的工作目录");
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!(?error, "跳过无法读取的工作树条目");
                    continue;
                }
            };
            let entry_path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    tracing::warn!(path = %entry_path.display(), ?error, "跳过无法识别的工作树条目");
                    continue;
                }
            };
            if file_type.is_symlink() {
                result.summary.symlink_files = result.summary.symlink_files.saturating_add(1);
                continue;
            }
            if file_type.is_dir() {
                if is_excluded_directory(entry.file_name().to_string_lossy().as_ref()) {
                    continue;
                }
                stack.push(entry_path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if is_excluded_file(&file_name) || file_name == ".git" || file_name.ends_with(".iml") {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    tracing::warn!(path = %entry_path.display(), ?error, "跳过无法 stat 的工作树文件");
                    continue;
                }
            };
            if metadata.len() > options.max_file_bytes {
                result.summary.oversized_files = result.summary.oversized_files.saturating_add(1);
                continue;
            }
            match read_text_file(&entry_path, &metadata, options.max_file_bytes) {
                Ok(ReadTextFile::Text(bytes)) => {
                    let raw_path = raw_path_hex(root, &entry_path);
                    let relative = relative_path(root, &entry_path);
                    let language = detect_language(&relative);
                    let lines = count_lines(&bytes);
                    result.summary.files = result.summary.files.saturating_add(1);
                    result.summary.lines = result.summary.lines.saturating_add(lines);
                    result.files.push(WorktreeFile {
                        raw_path_hex: raw_path,
                        language,
                        tracked: false,
                        path: relative,
                        lines,
                        bytes: metadata.len(),
                    });
                }
                Ok(ReadTextFile::Binary) => {
                    result.summary.binary_files = result.summary.binary_files.saturating_add(1);
                }
                Ok(ReadTextFile::Oversized) => {
                    // The file can grow between the metadata check above and
                    // the read.  Count it as oversized and never retain bytes
                    // beyond the configured safety bound.
                    result.summary.oversized_files =
                        result.summary.oversized_files.saturating_add(1);
                }
                Err(error) => {
                    tracing::warn!(path = %entry_path.display(), ?error, "跳过无法读取的工作树文件");
                }
            }
        }
    }
    result
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(result)
}

/// Scan a committed tree when a repository has no worktree (for example a
/// bare repository).  The returned rows use the same limits and language
/// detection as [`scan_worktree`], but never write an index or materialize a
/// checkout on disk.
pub fn scan_tree(
    repository: &gix::Repository,
    tree_id: gix::ObjectId,
    options: WorktreeScanOptions,
) -> Result<WorktreeScan> {
    let tree = repository.find_tree(tree_id)?;
    let mut result = WorktreeScan::default();
    scan_tree_entries(
        repository,
        &tree,
        &BString::default(),
        &options,
        &mut result,
    )?;
    result
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(result)
}

fn scan_tree_entries(
    repository: &gix::Repository,
    tree: &gix::Tree<'_>,
    prefix: &BString,
    options: &WorktreeScanOptions,
    result: &mut WorktreeScan,
) -> Result<()> {
    for entry in tree.iter() {
        let entry = entry?;
        let filename = entry.filename();
        let mut path = prefix.clone();
        if !path.is_empty() {
            path.push(b'/');
        }
        path.extend_from_slice(filename);
        if entry.mode().is_tree() {
            if is_excluded_directory(filename.to_str_lossy().as_ref()) {
                continue;
            }
            let child = repository.find_tree(entry.id().detach())?;
            scan_tree_entries(repository, &child, &path, options, result)?;
            continue;
        }
        if entry.mode().is_link() {
            result.summary.symlink_files = result.summary.symlink_files.saturating_add(1);
            continue;
        }
        let filename_display = filename.to_str_lossy();
        if is_excluded_file(filename_display.as_ref()) {
            continue;
        }
        let blob = repository.find_blob(entry.id().detach())?;
        if blob.data.len() as u64 > options.max_file_bytes {
            result.summary.oversized_files = result.summary.oversized_files.saturating_add(1);
            continue;
        }
        if blob.data.iter().take(8_192).any(|byte| *byte == 0) {
            result.summary.binary_files = result.summary.binary_files.saturating_add(1);
            continue;
        }
        let path_display = path.to_str_lossy().replace('\\', "/");
        let language = detect_language(&path_display);
        let lines = count_lines(&blob.data);
        result.summary.files = result.summary.files.saturating_add(1);
        result.summary.lines = result.summary.lines.saturating_add(lines);
        result.files.push(WorktreeFile {
            raw_path_hex: (!path.is_utf8()).then(|| {
                let raw_path: &[u8] = path.as_ref();
                hex::encode(raw_path)
            }),
            path: path_display,
            language,
            tracked: true,
            lines,
            bytes: blob.data.len() as u64,
        });
    }
    Ok(())
}

enum ReadTextFile {
    Text(Vec<u8>),
    Binary,
    Oversized,
}

fn read_text_file(path: &Path, metadata: &Metadata, max_file_bytes: u64) -> Result<ReadTextFile> {
    let mut file =
        File::open(path).with_context(|| format!("打开工作树文件失败: {}", path.display()))?;
    let read_limit = max_file_bytes.saturating_add(1);
    let mut bytes = Vec::with_capacity(metadata.len().min(max_file_bytes) as usize);
    file.by_ref()
        .take(read_limit)
        .read_to_end(&mut bytes)
        .with_context(|| format!("读取工作树文件失败: {}", path.display()))?;
    if bytes.len() as u64 > max_file_bytes {
        return Ok(ReadTextFile::Oversized);
    }
    if bytes.iter().take(8_192).any(|byte| *byte == 0) {
        return Ok(ReadTextFile::Binary);
    }
    Ok(ReadTextFile::Text(bytes))
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn raw_path_hex(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).unwrap_or(path);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = relative.as_os_str().as_bytes();
        (!bytes.is_utf8()).then(|| hex::encode(bytes))
    }
    #[cfg(not(unix))]
    {
        let _ = relative;
        None
    }
}

fn is_excluded_directory(name: &str) -> bool {
    EXCLUDED_DIRECTORIES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
        || name.to_ascii_lowercase().starts_with("pip-")
        || name.to_ascii_lowercase().starts_with(".run-data")
}

fn is_excluded_file(name: &str) -> bool {
    EXCLUDED_FILES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
}

/// Detect a stable language label from a relative path.
#[must_use]
pub fn detect_language(path: &str) -> Option<String> {
    let basename = path.rsplit('/').next().unwrap_or(path);
    let special = match basename.to_ascii_lowercase().as_str() {
        "dockerfile" => Some("Dockerfile"),
        "makefile" | "gnumakefile" => Some("Makefile"),
        "jenkinsfile" => Some("Groovy"),
        "gemfile" | "rakefile" => Some("Ruby"),
        "go.mod" | "go.sum" => Some("Go Modules"),
        "build.gradle" | "settings.gradle" => Some("Gradle"),
        "gradle.properties" => Some("Properties"),
        ".gitignore" | ".gitattributes" => Some("Git 配置"),
        ".editorconfig" => Some("EditorConfig"),
        _ => None,
    };
    if special.is_some() {
        return special.map(str::to_owned);
    }
    let Some(extension) = basename
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
    else {
        return Some("其他文本".to_owned());
    };
    let language = match extension.as_str() {
        "rs" => "Rust",
        "ts" | "tsx" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "py" | "pyi" => "Python",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "go" => "Go",
        "c" => "C",
        "h" | "cc" | "cpp" | "cxx" | "hpp" => "C/C++",
        "cs" => "C#",
        "swift" => "Swift",
        "rb" => "Ruby",
        "php" => "PHP",
        "ex" | "exs" => "Elixir",
        "erl" | "hrl" => "Erlang",
        "scala" => "Scala",
        "sh" | "bash" | "zsh" => "Shell",
        "sql" => "SQL",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "scss" => "SCSS",
        "sass" => "Sass",
        "less" => "Less",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "md" | "mdx" => "Markdown",
        "rst" => "reStructuredText",
        "adoc" => "AsciiDoc",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "xml" => "XML",
        "ini" => "INI",
        "conf" | "cfg" => "配置",
        "properties" => "Properties",
        "proto" => "Protocol Buffers",
        "dart" => "Dart",
        "groovy" => "Groovy",
        "r" => "R",
        "lua" => "Lua",
        "pl" => "Perl",
        "m" => "Objective-C",
        "mm" => "Objective-C++",
        "tf" => "Terraform",
        "hcl" => "HCL",
        "graphql" | "gql" => "GraphQL",
        "fs" | "fsx" => "F#",
        "vb" => "Visual Basic",
        "asm" | "s" => "Assembly",
        "ps1" => "PowerShell",
        "bat" | "cmd" => "批处理",
        "csv" => "CSV",
        "txt" => "纯文本",
        "ipynb" => "Jupyter Notebook",
        "styl" => "Stylus",
        _ => "其他文本",
    };
    Some(language.to_owned())
}

fn count_lines(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        0
    } else {
        let newline_count = bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
        if bytes.last() == Some(&b'\n') {
            newline_count
        } else {
            newline_count.saturating_add(1)
        }
    }
}

/// Aggregate current lines by language, retaining all files including files
/// whose extension is unknown under the `Unknown` label.
#[must_use]
pub fn language_lines(scan: &WorktreeScan) -> Vec<(String, u64)> {
    let mut totals: HashMap<String, u64> = HashMap::new();
    for file in &scan.files {
        *totals
            .entry(
                file.language
                    .clone()
                    .unwrap_or_else(|| "其他文本".to_owned()),
            )
            .or_default() += file.lines;
    }
    let mut result: Vec<_> = totals.into_iter().collect();
    result.sort_by(|left, right| left.0.cmp(&right.0));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn skips_binary_symlink_and_lock_files() -> Result<()> {
        let dir = tempdir()?;
        fs::write(dir.path().join("main.rs"), "fn main() {}\n")?;
        fs::write(dir.path().join("Cargo.lock"), "lock")?;
        fs::write(dir.path().join("blob.bin"), [0, 1, 2])?;
        fs::create_dir(dir.path().join("target"))?;
        fs::write(dir.path().join("target/out.rs"), "generated\n")?;
        let scan = scan_worktree(dir.path(), WorktreeScanOptions::default())?;
        assert_eq!(scan.summary.files, 1, "only the source file is included");
        assert_eq!(
            scan.summary.binary_files, 1,
            "NUL bytes identify binary files"
        );
        assert_eq!(scan.files[0].language.as_deref(), Some("Rust"));
        Ok(())
    }

    #[test]
    fn includes_ignored_style_hidden_sources() -> Result<()> {
        let dir = tempdir()?;
        fs::create_dir(dir.path().join(".generated"))?;
        fs::write(
            dir.path().join(".generated/source.ts"),
            "export const x = 1",
        )?;
        let scan = scan_worktree(dir.path(), WorktreeScanOptions::default())?;
        assert_eq!(
            scan.summary.files, 1,
            "hidden ignored-style source remains visible"
        );
        assert_eq!(scan.files[0].lines, 1);
        Ok(())
    }

    #[test]
    fn read_limit_is_enforced_even_when_metadata_is_stale() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("source.rs");
        fs::write(&path, "0123456789")?;
        let metadata = fs::metadata(&path)?;
        assert!(matches!(
            read_text_file(&path, &metadata, 4)?,
            ReadTextFile::Oversized
        ));
        Ok(())
    }

    #[test]
    fn preserves_unknown_text_and_legacy_language_labels() {
        assert_eq!(
            detect_language("notes/custom.extension").as_deref(),
            Some("其他文本")
        );
        assert_eq!(
            detect_language("config/app.properties").as_deref(),
            Some("Properties")
        );
        assert_eq!(detect_language("Dockerfile").as_deref(), Some("Dockerfile"));
    }
}
