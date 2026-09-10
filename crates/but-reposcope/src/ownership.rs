//! Bounded line ownership and code-age analysis.
//!
//! GitButler already carries `git2` for the small set of capabilities that do
//! not have a suitable `gix` equivalent yet.  This module is the explicit
//! boundary for that dependency: it only reads blame data, never invokes a
//! command-line process, and never mutates the repository.

use crate::CancellationToken;
use crate::model::{
    CodeAgeBucket, DirectoryOwnership, FileAgeStat, OwnershipCoverage, OwnershipRow,
};
use crate::worktree::WorktreeFile;
use anyhow::{Context as _, Result};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The fixed age buckets retained for compatibility with the previous
/// RepoScope report.
pub const AGE_BUCKETS: &[(&str, &str, Option<u64>)] = &[
    ("30d", "不足 1 个月", Some(30)),
    ("90d", "1–3 个月", Some(90)),
    ("180d", "3–6 个月", Some(180)),
    ("365d", "6–12 个月", Some(365)),
    ("730d", "1–2 年", Some(730)),
    ("1825d", "2–5 年", Some(1825)),
    ("older", "超过 5 年", None),
];

/// Return the compatibility bucket for an age in days.
#[must_use]
pub fn age_bucket(days: u64) -> &'static str {
    AGE_BUCKETS
        .iter()
        .find(|(_, _, upper)| upper.is_none_or(|upper| days < upper))
        .map_or("older", |(key, _, _)| key)
}

/// A blame attribution compressed to one contiguous hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameAttribution {
    /// Author display name.
    pub name: String,
    /// Redacted author email.
    pub email: String,
    /// Author time in Unix seconds.
    pub authored_at: i64,
    /// Number of contiguous lines attributed to this author.
    pub lines: u64,
}

/// The result of the bounded ownership pass.
#[derive(Debug, Clone, Default)]
pub struct OwnershipAnalysis {
    /// File/author rows, ordered by path and then owner.
    pub rows: Vec<OwnershipRow>,
    /// Directory aggregates, ordered by line count.
    pub directories: Vec<DirectoryOwnership>,
    /// Fixed age buckets.
    pub code_age: Vec<CodeAgeBucket>,
    /// Per-file age summaries.
    pub file_ages: Vec<FileAgeStat>,
    /// Explicit coverage of the bounded pass.
    pub coverage: OwnershipCoverage,
    /// Repository totals used by the Bus Factor calculation.
    pub owner_lines: Vec<(String, u64)>,
    /// Number of files for which blame could not be read.
    pub failed_files: u64,
}

/// A small adapter around the `git2` blame API.
pub struct BlameAdapter {
    repository: git2::Repository,
    worktree: PathBuf,
    tracked_paths: BTreeSet<String>,
}

impl BlameAdapter {
    /// Open a normal repository and load its index paths once.
    pub fn open(repository: impl AsRef<Path>) -> Result<Option<Self>> {
        let repository = git2::Repository::discover(repository.as_ref())
            .with_context(|| format!("打开 blame 仓库失败: {}", repository.as_ref().display()))?;
        let Some(worktree) = repository.workdir().map(Path::to_owned) else {
            return Ok(None);
        };
        let tracked_paths = repository
            .index()
            .map(|index| {
                index
                    .iter()
                    .map(|entry| String::from_utf8_lossy(&entry.path).replace('\\', "/"))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(Self {
            repository,
            worktree,
            tracked_paths,
        }))
    }

    /// Return whether a displayed worktree path is tracked by the index.
    #[must_use]
    pub fn is_tracked(&self, path: &str) -> bool {
        self.tracked_paths.contains(path)
    }

    /// Return a read-only view of the index paths loaded by this adapter.
    #[must_use]
    pub fn tracked_paths(&self) -> &BTreeSet<String> {
        &self.tracked_paths
    }

    /// Read blame hunks for a validated, relative text path.
    pub fn blame_file(&self, path: &str) -> Result<Vec<BlameAttribution>> {
        let relative = validate_relative_path(path)?;
        let root = self
            .worktree
            .canonicalize()
            .with_context(|| format!("解析工作树失败: {}", self.worktree.display()))?;
        let candidate = root.join(&relative);
        reject_symlink_components(&root, &relative)?;
        let metadata = std::fs::symlink_metadata(&candidate)
            .with_context(|| format!("读取 blame 文件失败: {path}"))?;
        if metadata.file_type().is_symlink() {
            anyhow::bail!("不对符号链接执行 blame: {path}");
        }
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("解析 blame 文件失败: {path}"))?;
        if !canonical.starts_with(&root) {
            anyhow::bail!("blame 文件超出仓库边界: {path}");
        }
        if !metadata.is_file() {
            anyhow::bail!("blame 目标不是普通文件: {path}");
        }
        let blame = self.repository.blame_file(&relative, None)?;
        let mut result = Vec::new();
        for hunk in blame.iter() {
            let Some(signature) = hunk.final_signature() else {
                continue;
            };
            let name = String::from_utf8_lossy(signature.name_bytes())
                .trim()
                .to_owned();
            let email = String::from_utf8_lossy(signature.email_bytes())
                .trim()
                .to_ascii_lowercase();
            result.push(BlameAttribution {
                name: if name.is_empty() {
                    "未知作者".to_owned()
                } else {
                    redact_text(&name)
                },
                email: redact_email(&email),
                authored_at: signature.when().seconds(),
                lines: hunk.lines_in_hunk() as u64,
            });
        }
        Ok(result)
    }
}

/// Read the repository index paths without running a Git subprocess.
///
/// A missing or unreadable index is treated as an empty set.  This is used to
/// annotate worktree rows; it must never make the core history scan fail.
#[must_use]
pub fn tracked_paths(repository: impl AsRef<Path>) -> BTreeSet<String> {
    git2::Repository::discover(repository.as_ref())
        .ok()
        .and_then(|repository| repository.index().ok())
        .map(|index| {
            index
                .iter()
                .map(|entry| String::from_utf8_lossy(&entry.path).replace('\\', "/"))
                .collect()
        })
        .unwrap_or_default()
}

/// Run blame for at most `max_files` largest tracked text files.
///
/// `workers` controls the number of independent read-only `git2` handles used
/// for the bounded blame pass.  A handle is never shared across threads because
/// libgit2 repositories are not guaranteed to be `Sync`; every worker opens its
/// own repository and the caller merges results deterministically afterwards.
pub fn analyze_ownership<F>(
    repository: impl AsRef<Path>,
    files: &[WorktreeFile],
    max_files: usize,
    workers: usize,
    cancellation: &CancellationToken,
    progress: F,
) -> Result<OwnershipAnalysis>
where
    F: FnMut(u64, u64),
{
    analyze_ownership_in_scope(
        repository,
        files,
        max_files,
        workers,
        cancellation,
        None,
        progress,
    )
}

/// Run the bounded blame pass for paths relative to an optional worktree
/// scope.  The prefix is repository-relative (for example src) while the
/// returned rows remain relative to the selected scope (main.rs).
pub fn analyze_ownership_in_scope<F>(
    repository: impl AsRef<Path>,
    files: &[WorktreeFile],
    max_files: usize,
    workers: usize,
    cancellation: &CancellationToken,
    scope_prefix: Option<&str>,
    mut progress: F,
) -> Result<OwnershipAnalysis>
where
    F: FnMut(u64, u64),
{
    let repository = repository.as_ref().to_owned();
    let files_total = files.len() as u64;
    let lines_total = files.iter().map(|file| file.lines).sum();
    let adapter = match BlameAdapter::open(&repository) {
        Ok(Some(adapter)) => adapter,
        Ok(None) => return Ok(empty_analysis(files_total, lines_total)),
        Err(error) => {
            tracing::warn!(?error, "无法打开 git2 blame 适配器，保留核心分析结果");
            return Ok(empty_analysis(files_total, lines_total));
        }
    };
    let mut selected: Vec<_> = files
        .iter()
        .filter(|file| {
            let blame_path = scoped_repository_path(&file.path, scope_prefix);
            adapter.is_tracked(&blame_path)
        })
        .collect();
    selected.sort_by(|left, right| {
        right
            .lines
            .cmp(&left.lines)
            .then_with(|| left.path.cmp(&right.path))
    });
    selected.truncate(max_files.max(1));
    let selected: Vec<(String, String, u64)> = selected
        .into_iter()
        .map(|file| {
            (
                file.path.clone(),
                scoped_repository_path(&file.path, scope_prefix),
                file.lines,
            )
        })
        .collect();
    drop(adapter);
    let total = selected.len() as u64;
    let now = now_seconds();
    let mut analysis = OwnershipAnalysis {
        coverage: OwnershipCoverage {
            files_total,
            files_analyzed: 0,
            lines_total,
            lines_analyzed: 0,
            percentage: 0.0,
        },
        ..OwnershipAnalysis::default()
    };
    let mut repository_totals: BTreeMap<String, (String, u64)> = BTreeMap::new();
    let mut directory_totals: BTreeMap<String, BTreeMap<String, (String, u64)>> = BTreeMap::new();
    let mut age_totals: BTreeMap<&'static str, (u64, u64)> = AGE_BUCKETS
        .iter()
        .map(|(key, _, _)| (*key, (0, 0)))
        .collect();

    if total > 0 {
        let worker_count = workers.clamp(1, 8).min(total as usize);
        let chunk_size = selected.len().div_ceil(worker_count);
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut handles = Vec::with_capacity(worker_count);
        for chunk in selected.chunks(chunk_size) {
            let worker_repository = repository.clone();
            let worker_files = chunk.to_vec();
            let worker_cancellation = cancellation.clone();
            let worker_sender = sender.clone();
            handles.push(std::thread::spawn(move || {
                let adapter = BlameAdapter::open(&worker_repository).ok().flatten();
                for (index, (display_path, blame_path, _lines)) in
                    worker_files.into_iter().enumerate()
                {
                    let attributions = if worker_cancellation.is_cancelled() {
                        None
                    } else {
                        adapter
                            .as_ref()
                            .and_then(|adapter| adapter.blame_file(&blame_path).ok())
                            .filter(|rows| !rows.is_empty())
                    };
                    // Every selected file produces one message, including a
                    // failed/empty blame result, so the coordinator can make
                    // progress without waiting on a missing worker message.
                    let _ = worker_sender.send((index, display_path, attributions));
                }
            }));
        }
        drop(sender);

        let mut completed = 0_u64;
        while completed < total {
            let (_index, path, attributions) =
                receiver.recv().context("读取并发 blame 结果失败")?;
            completed = completed.saturating_add(1);
            if let Some(attributions) = attributions {
                merge_attributions(
                    &mut analysis,
                    &mut repository_totals,
                    &mut directory_totals,
                    &mut age_totals,
                    &path,
                    attributions,
                    now,
                );
            } else {
                analysis.failed_files = analysis.failed_files.saturating_add(1);
            }
            progress(completed, total);
        }
        for handle in handles {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("并发 blame worker 异常退出"))?;
        }
        if cancellation.is_cancelled() {
            anyhow::bail!("分析已取消");
        }
    }
    analysis.coverage.percentage = percentage(
        analysis.coverage.lines_analyzed,
        analysis.coverage.lines_total,
    );
    analysis.owner_lines = repository_totals
        .into_iter()
        .map(|(identity, (_, lines))| (identity, lines))
        .collect();
    analysis.rows.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.author_identity.cmp(&right.author_identity))
    });
    analysis.file_ages.sort_by(|left, right| {
        right
            .average_days
            .cmp(&left.average_days)
            .then_with(|| left.path.cmp(&right.path))
    });
    analysis.directories = directory_totals
        .into_iter()
        .map(|(directory, owners)| {
            let lines: u64 = owners.values().map(|(_, lines)| *lines).sum();
            let dominant_author = owners
                .values()
                .max_by_key(|(_, lines)| *lines)
                .map(|(name, _)| name.clone());
            let directory_prefix = directory_prefix(&directory);
            DirectoryOwnership {
                directory,
                dominant_author,
                lines,
                files: analysis
                    .rows
                    .iter()
                    .filter(|row| row.path.starts_with(&directory_prefix))
                    .map(|row| row.path.as_str())
                    .collect::<BTreeSet<_>>()
                    .len() as u64,
            }
        })
        .collect();
    analysis.directories.sort_by(|left, right| {
        right
            .lines
            .cmp(&left.lines)
            .then_with(|| left.directory.cmp(&right.directory))
    });
    let total_age_lines: u64 = age_totals.values().map(|(_, lines)| *lines).sum();
    analysis.code_age = AGE_BUCKETS
        .iter()
        .map(|(key, label, _)| {
            let (files, lines) = age_totals.get(key).copied().unwrap_or_default();
            CodeAgeBucket {
                bucket: (*key).to_owned(),
                files,
                lines,
                label: (*label).to_owned(),
                percentage: percentage(lines, total_age_lines),
            }
        })
        .collect();
    Ok(analysis)
}

fn merge_attributions(
    analysis: &mut OwnershipAnalysis,
    repository_totals: &mut BTreeMap<String, (String, u64)>,
    directory_totals: &mut BTreeMap<String, BTreeMap<String, (String, u64)>>,
    age_totals: &mut BTreeMap<&'static str, (u64, u64)>,
    path: &str,
    attributions: Vec<BlameAttribution>,
    now: i64,
) {
    let mut by_author: BTreeMap<String, (String, u64, u128, u64)> = BTreeMap::new();
    let mut age_sum = 0_u128;
    let mut age_count = 0_u64;
    let mut oldest_days = 0_u64;
    let mut newest_days = u64::MAX;
    let mut file_buckets = BTreeSet::new();
    for attribution in attributions {
        let identity = identity(&attribution.name, &attribution.email);
        let owner = by_author
            .entry(identity.clone())
            .or_insert_with(|| (attribution.name.clone(), 0, 0, 0));
        owner.1 = owner.1.saturating_add(attribution.lines);
        let repository_owner = repository_totals
            .entry(identity.clone())
            .or_insert_with(|| (attribution.name.clone(), 0));
        repository_owner.1 = repository_owner.1.saturating_add(attribution.lines);
        let age = age_days(now, attribution.authored_at);
        owner.2 = owner
            .2
            .saturating_add(u128::from(age).saturating_mul(u128::from(attribution.lines)));
        owner.3 = owner.3.saturating_add(attribution.lines);
        age_sum =
            age_sum.saturating_add(u128::from(age).saturating_mul(u128::from(attribution.lines)));
        age_count = age_count.saturating_add(attribution.lines);
        oldest_days = oldest_days.max(age);
        newest_days = newest_days.min(age);
        let bucket = age_bucket(age);
        if let Some((_bucket_files, bucket_lines)) = age_totals.get_mut(bucket) {
            file_buckets.insert(bucket);
            *bucket_lines = bucket_lines.saturating_add(attribution.lines);
        }
        for directory in parent_directories(path) {
            let owner = directory_totals
                .entry(directory)
                .or_default()
                .entry(identity.clone())
                .or_insert_with(|| (attribution.name.clone(), 0));
            owner.1 = owner.1.saturating_add(attribution.lines);
        }
    }
    for bucket in file_buckets {
        if let Some((bucket_files, _)) = age_totals.get_mut(bucket) {
            *bucket_files = bucket_files.saturating_add(1);
        }
    }
    let file_lines: u64 = by_author.values().map(|(_, lines, _, _)| *lines).sum();
    for (author_identity, (author_name, lines, author_age_sum, author_age_count)) in by_author {
        analysis.rows.push(OwnershipRow {
            path: path.to_owned(),
            author_identity,
            author_name,
            lines,
            percentage: percentage(lines, file_lines),
            age_days: average_age(author_age_sum, author_age_count),
        });
    }
    analysis.file_ages.push(FileAgeStat {
        path: path.to_owned(),
        average_days: average_age(age_sum, age_count),
        oldest_days,
        newest_days: if newest_days == u64::MAX {
            0
        } else {
            newest_days
        },
        lines: file_lines,
    });
    analysis.coverage.files_analyzed = analysis.coverage.files_analyzed.saturating_add(1);
    analysis.coverage.lines_analyzed = analysis.coverage.lines_analyzed.saturating_add(file_lines);
}

fn empty_analysis(files_total: u64, lines_total: u64) -> OwnershipAnalysis {
    OwnershipAnalysis {
        coverage: OwnershipCoverage {
            files_total,
            lines_total,
            ..OwnershipCoverage::default()
        },
        code_age: AGE_BUCKETS
            .iter()
            .map(|(key, label, _)| CodeAgeBucket {
                bucket: (*key).to_owned(),
                files: 0,
                lines: 0,
                label: (*label).to_owned(),
                percentage: 0.0,
            })
            .collect(),
        ..OwnershipAnalysis::default()
    }
}

fn scoped_repository_path(path: &str, scope_prefix: Option<&str>) -> String {
    let Some(prefix) = scope_prefix
        .map(|value| value.trim_matches('/'))
        .filter(|value| !value.is_empty())
    else {
        return path.to_owned();
    };
    format!("{}/{}", prefix.trim_matches('/'), path)
}

fn validate_relative_path(path: &str) -> Result<PathBuf> {
    if path.is_empty() || path.contains('\0') {
        anyhow::bail!("blame 路径为空或包含非法字符");
    }
    let normalized = path.replace('\\', "/");
    let mut output = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => output.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("blame 路径必须位于仓库内")
            }
        }
    }
    if output.as_os_str().is_empty() {
        anyhow::bail!("blame 路径为空");
    }
    Ok(output)
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<()> {
    let mut current = root.to_owned();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        if let Ok(metadata) = std::fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            anyhow::bail!("不对符号链接执行 blame: {}", relative.display());
        }
    }
    Ok(())
}

fn parent_directories(path: &str) -> impl Iterator<Item = String> {
    let mut directories = Vec::new();
    let mut current = Path::new(path).parent();
    while let Some(directory) = current {
        if directory.as_os_str().is_empty() {
            break;
        }
        directories.push(directory.to_string_lossy().replace('\\', "/"));
        current = directory.parent();
    }
    directories.into_iter()
}

fn directory_prefix(directory: &str) -> String {
    format!("{directory}/")
}

fn identity(name: &str, email: &str) -> String {
    format!("{name}\0{email}")
}

fn percentage(value: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        value as f64 * 100.0 / total as f64
    }
}

fn average_age(sum: u128, count: u64) -> u64 {
    if count == 0 {
        0
    } else {
        (sum / u128::from(count)).min(u128::from(u64::MAX)) as u64
    }
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs().min(i64::MAX as u64) as i64)
}

fn age_days(now: i64, authored_at: i64) -> u64 {
    now.saturating_sub(authored_at).max(0) as u64 / 86_400
}

fn redact_text(value: &str) -> String {
    let value = value.replace(['\r', '\n', '\0'], " ");
    static URL_USERINFO: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static SECRET: OnceLock<Regex> = OnceLock::new();
    let value = URL_USERINFO
        .get_or_init(|| {
            Regex::new(r#"(?i)(\b[a-z][a-z0-9+.-]*://)[^/\s:@]+(?::[^/\s@]*)?@"#)
                .expect("valid ownership URL credential regex")
        })
        .replace_all(&value, "$1[已脱敏]@");
    let value = BEARER
        .get_or_init(|| {
            Regex::new(r#"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+"#)
                .expect("valid ownership bearer credential regex")
        })
        .replace_all(&value, "Bearer [已脱敏]");
    SECRET
        .get_or_init(|| {
            Regex::new(
                r#"(?i)["']?(user(?:name)?|login|password|passwd|pwd|token|secret|api[_-]?key|access[_-]?token|client[_-]?secret|authorization|private[_-]?key)["']?\s*([:=])\s*(?:"[^"]*"|'[^']*'|[^,;\s}\]]+)"#,
            )
            .expect("valid ownership redaction regex")
        })
        .replace_all(&value, "$1$2[已脱敏]")
        .into_owned()
}

fn redact_email(value: &str) -> String {
    let Some((local, domain)) = value.split_once('@') else {
        return "[已脱敏]".to_owned();
    };
    let visible = local.chars().next().unwrap_or('*');
    format!("{visible}***@{domain}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::{WorktreeScanOptions, scan_worktree};
    use tempfile::tempdir;

    #[test]
    fn age_boundaries_are_stable() {
        assert_eq!(age_bucket(0), "30d");
        assert_eq!(age_bucket(29), "30d");
        assert_eq!(age_bucket(30), "90d");
        assert_eq!(age_bucket(730), "1825d");
        assert_eq!(age_bucket(1_825), "older");
    }

    #[test]
    fn directory_parents_are_posix_and_do_not_include_root() {
        let values: Vec<_> = parent_directories("src/nested/main.rs").collect();
        assert_eq!(values, vec!["src/nested", "src"]);
    }

    #[test]
    fn email_is_redacted_before_it_can_be_stored() {
        assert_eq!(redact_email("alice@example.com"), "a***@example.com");
        assert_eq!(redact_email("not-an-email"), "[已脱敏]");
    }

    #[test]
    fn configured_blame_workers_merge_a_real_repository() -> Result<()> {
        let directory = tempdir()?;
        let repository = git2::Repository::init(directory.path())?;
        std::fs::write(directory.path().join("main.rs"), "fn main() {}\n")?;
        let mut index = repository.index()?;
        index.add_path(Path::new("main.rs"))?;
        let tree_id = index.write_tree()?;
        index.write()?;
        let tree = repository.find_tree(tree_id)?;
        let signature = git2::Signature::now("测试作者", "tester@example.com")?;
        repository.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "feat: initial source",
            &tree,
            &[],
        )?;

        let scan = scan_worktree(directory.path(), WorktreeScanOptions::default())?;
        let result = analyze_ownership(
            directory.path(),
            &scan.files,
            2_000,
            2,
            &CancellationToken::new(),
            |_, _| {},
        )?;
        assert_eq!(result.coverage.files_analyzed, 1);
        assert_eq!(result.coverage.lines_analyzed, 1);
        assert_eq!(result.owner_lines.len(), 1);
        Ok(())
    }
}
