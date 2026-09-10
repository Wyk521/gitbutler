//! Stable domain and transport-neutral models used by the analysis engine.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The lifecycle state of an analysis run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum AnalysisRunStatus {
    /// Waiting in the project queue.
    Queued,
    /// Currently scanning the repository.
    Running,
    /// All required reports completed.
    Complete,
    /// Core reports completed while one or more optional reports failed.
    Partial,
    /// The run failed before it could produce a usable result.
    Failed,
    /// The user cancelled the run.
    Cancelled,
    /// The process exited while the run was active.
    Interrupted,
}

/// A coarse phase reported while a scan is in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum AnalysisPhase {
    /// Enumerating refs and commits.
    History,
    /// Reading the current worktree.
    Worktree,
    /// Computing aggregate metrics.
    Metrics,
    /// Computing line ownership and age.
    Ownership,
    /// Computing optional audit evidence reports.
    Evidence,
    /// Writing the new SQLite batch.
    Persisting,
    /// The run is waiting to start.
    Queued,
}

/// How current an active result is relative to the repository fingerprints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
pub enum Freshness {
    /// The ref and worktree fingerprints both match the active run.
    #[serde(rename = "fresh")]
    Fresh,
    /// A ref changed after the active run completed.
    #[serde(rename = "historyStale")]
    HistoryStale,
    /// Files in the worktree changed after the active run completed.
    #[serde(rename = "worktreeStale")]
    WorktreeStale,
    /// Both fingerprints changed.
    #[serde(rename = "interrupted")]
    Interrupted,
}

/// Progress and freshness information exposed by the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct AnalysisStatus {
    /// The SQLite run identifier, if a run has been queued.
    pub run_id: Option<String>,
    /// Lifecycle status.
    pub status: AnalysisRunStatus,
    /// Current work phase.
    pub phase: AnalysisPhase,
    /// Progress in the inclusive range `0..=1`.
    pub progress: f64,
    /// Number of commits/files already processed in the current phase.
    pub processed: u64,
    /// Estimated total commits/files in the current phase.
    pub total: u64,
    /// Stable, user-safe error text, when the run failed.
    pub error: Option<String>,
    /// Freshness of the active result, when one exists.
    pub freshness: Option<Freshness>,
}

impl AnalysisStatus {
    /// Construct a running progress update.
    #[must_use]
    pub fn running(run_id: &str, phase: AnalysisPhase, processed: u64, total: u64) -> Self {
        let progress = if total == 0 {
            0.0
        } else {
            (processed as f64 / total as f64).clamp(0.0, 1.0)
        };
        Self {
            run_id: Some(run_id.to_owned()),
            status: AnalysisRunStatus::Running,
            phase,
            progress,
            processed,
            total,
            error: None,
            freshness: None,
        }
    }

    /// Construct a terminal progress update.
    #[must_use]
    pub fn finished(run_id: &str, status: AnalysisRunStatus) -> Self {
        Self {
            run_id: Some(run_id.to_owned()),
            status,
            phase: AnalysisPhase::Persisting,
            progress: 1.0,
            processed: 0,
            total: 0,
            error: None,
            freshness: None,
        }
    }
}

/// Configuration persisted per project.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct AnalysisConfig {
    /// Maximum file size read from a worktree, in bytes.
    pub max_file_bytes: u64,
    /// Maximum number of tracked files included in blame ownership analysis.
    pub ownership_max_files: usize,
    /// Worker count for expensive ownership operations.
    pub workers: usize,
    /// Maximum files in a commit for pairwise coupling analysis.
    pub coupling_max_files: usize,
    /// Maximum number of persisted coupling pairs.
    pub coupling_top_pairs: usize,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            max_file_bytes: 20_000_000,
            ownership_max_files: 2_000,
            workers: 4,
            coupling_max_files: 80,
            coupling_top_pairs: 500,
        }
    }
}

impl AnalysisConfig {
    /// Clamp persisted or programmatic settings to the safety and resource
    /// limits supported by the desktop scanner.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.max_file_bytes = self.max_file_bytes.clamp(1, 20_000_000);
        self.ownership_max_files = self.ownership_max_files.clamp(1, 100_000);
        self.workers = self.workers.clamp(1, 8);
        self.coupling_max_files = self.coupling_max_files.clamp(2, 200);
        self.coupling_top_pairs = self.coupling_top_pairs.clamp(1, 5_000);
        self
    }
}

/// Encoding metadata for a path that could not be represented as UTF-8.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct PathEncoding {
    /// Lossy display form used by the UI.
    pub display: String,
    /// Hexadecimal bytes of the original Git path.
    pub raw_hex: String,
}

impl fmt::Display for PathEncoding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.display)
    }
}

/// A paginated response shared by all list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    /// Rows in this page.
    pub items: Vec<T>,
    /// Number of rows matching the query before paging.
    pub total: u64,
    /// One-based page number.
    pub page: u32,
    /// Number of rows requested.
    pub page_size: u32,
}

impl<T> Page<T> {
    /// Clamp an API page size to the supported `1..=100` range.
    #[must_use]
    pub fn normalize(page: u32, page_size: u32) -> (u32, u32) {
        (page.max(1), page_size.clamp(1, 100))
    }
}

/// High-level repository totals.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct OverviewReport {
    /// Number of unique commits in the visible history.
    pub commit_count: u64,
    /// Number of files in the current worktree snapshot.
    pub file_count: u64,
    /// Number of contributors.
    pub author_count: u64,
    /// Number of local and remote branches in the visible refs.
    pub branch_count: u64,
    /// Number of tags in the visible refs.
    pub tag_count: u64,
    /// Number of current text files not present in the Git index.
    pub untracked_files: u64,
    /// Current source lines across included files.
    pub lines: u64,
    /// Total additions across commits.
    pub additions: u64,
    /// Total deletions across commits.
    pub deletions: u64,
    /// Churn (`additions + deletions`).
    pub churn: u64,
    /// Net growth (`additions - deletions`).
    pub net_growth: i64,
    /// Earliest author timestamp in UTC milliseconds.
    pub first_commit_at: Option<i64>,
    /// Latest author timestamp in UTC milliseconds.
    pub last_commit_at: Option<i64>,
    /// Whether the repository was opened without a worktree.
    pub bare: bool,
}

/// A commit row suitable for a list and drill-down link.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct CommitSummary {
    /// Full object id in hexadecimal.
    pub oid: String,
    /// Short object id for display.
    pub short_oid: String,
    /// Commit subject, with credentials redacted.
    pub subject: String,
    /// Author display name, redacted when necessary.
    pub author_name: String,
    /// Redacted author email.
    pub author_email: String,
    /// Author time in UTC milliseconds.
    pub authored_at: i64,
    /// Number of parents.
    pub parent_count: u32,
    /// Lines added against the first parent.
    pub additions: u64,
    /// Lines removed against the first parent.
    pub deletions: u64,
    /// Number of changed files.
    pub files_changed: u64,
    /// Stable commit category (feat/fix/refactor/docs/test/chore/build/ci/other).
    pub category: String,
}

/// One file entry shown when a commit is opened.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct CommitFileDetail {
    /// Current path after rename-chain normalization.
    pub path: String,
    /// Previous path when the change is a rename.
    pub previous_path: Option<String>,
    /// Lines added in the file diff.
    pub additions: u64,
    /// Lines removed in the file diff.
    pub deletions: u64,
}

/// A commit summary together with its file changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    /// Commit metadata.
    pub commit: CommitSummary,
    /// Files changed against the first parent.
    pub files: Vec<CommitFileDetail>,
}

/// A file-level evolution row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileSummary {
    /// Display path relative to the worktree.
    pub path: String,
    /// Original bytes when the path was not UTF-8.
    pub raw_path_hex: Option<String>,
    /// Detected language, if known.
    pub language: Option<String>,
    /// Whether this path was present in the repository index at scan time.
    /// Untracked and ignored text files are still included in the current
    /// worktree totals, but are explicitly marked here.
    pub tracked: bool,
    /// Current line count.
    pub lines: u64,
    /// Cumulative additions.
    pub additions: u64,
    /// Cumulative deletions.
    pub deletions: u64,
    /// Number of commits touching the file.
    pub commit_count: u64,
    /// Number of distinct historical authors touching the file.
    pub author_count: u64,
    /// Hotspot score in `0..=100`.
    pub hotspot_score: f64,
    /// Most recent change in UTC milliseconds.
    pub last_changed_at: Option<i64>,
}

/// Contributor totals.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct AuthorSummary {
    /// Stable redacted identity key.
    pub identity: String,
    /// Display name.
    pub name: String,
    /// Redacted email.
    pub email: String,
    /// Number of commits.
    pub commit_count: u64,
    /// Number of distinct files touched by this author.
    pub files_touched: u64,
    /// Lines added.
    pub additions: u64,
    /// Lines removed.
    pub deletions: u64,
    /// Current owned lines when ownership is available.
    pub owned_lines: u64,
    /// Earliest commit authored in UTC milliseconds.
    pub first_commit_at: Option<i64>,
    /// Latest commit authored in UTC milliseconds.
    pub last_commit_at: Option<i64>,
}

/// One UTC calendar day's activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct DailyActivity {
    /// ISO date (`YYYY-MM-DD`) in UTC.
    pub date: String,
    /// Number of commits.
    pub commits: u64,
    /// Additions.
    pub additions: u64,
    /// Deletions.
    pub deletions: u64,
    /// Distinct active authors.
    pub active_authors: u64,
}

/// Language totals based on current files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct LanguageSummary {
    /// Canonical language name.
    pub language: String,
    /// Number of files.
    pub files: u64,
    /// Lines of code.
    pub lines: u64,
    /// Percentage of current lines.
    pub percentage: f64,
}

/// A co-change pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct CouplingSummary {
    /// First path.
    pub left: String,
    /// Second path.
    pub right: String,
    /// Number of commits touching both files.
    pub co_changes: u64,
    /// Percentage relative to the most frequently co-changing pair.
    pub strength: f64,
}

/// Bus-factor result with both LOC and commit interpretations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct BusFactorReport {
    /// Minimum authors covering 50% of current lines.
    pub loc: u64,
    /// Minimum authors covering 50% of historical commits.
    pub commits: u64,
    /// Whether no ownership data was available.
    pub insufficient_data: bool,
}

/// A visible local ref and its peeled tip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct RefSummary {
    /// Full refname in a lossy display form.
    pub name: String,
    /// Original refname bytes when needed.
    pub raw_name_hex: Option<String>,
    /// Peeled commit id, if the ref points at a commit.
    pub target: Option<String>,
    /// Whether this ref was excluded as GitButler internal state.
    pub excluded: bool,
}

/// Current worktree totals and scan limits.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSummary {
    /// Included files.
    pub files: u64,
    /// Included lines.
    pub lines: u64,
    /// Included text files that are not present in the Git index.
    pub untracked_files: u64,
    /// Skipped binary files.
    pub binary_files: u64,
    /// Skipped files over the configured size limit.
    pub oversized_files: u64,
    /// Skipped symbolic links.
    pub symlink_files: u64,
}

/// One line-ownership row.  The first implementation populates this table
/// from the isolated blame adapter; omitted rows are represented by coverage,
/// never by guessed owners.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct OwnershipRow {
    /// Worktree-relative path.
    pub path: String,
    /// Stable redacted owner identity.
    pub author_identity: String,
    /// Display name.
    pub author_name: String,
    /// Lines attributed to this author.
    pub lines: u64,
    /// Percentage of the file attributed to this author.
    pub percentage: f64,
    /// Average age of the attributed lines in days.
    pub age_days: u64,
}

/// Coverage information for the bounded blame stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct OwnershipCoverage {
    /// Number of source files in the current snapshot.
    pub files_total: u64,
    /// Number of files actually sent to blame.
    pub files_analyzed: u64,
    /// Lines in all included source files.
    pub lines_total: u64,
    /// Lines represented by successful blame results.
    pub lines_analyzed: u64,
    /// `lines_analyzed / lines_total * 100`.
    pub percentage: f64,
}

/// Age statistics for one successfully blamed file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileAgeStat {
    /// Worktree-relative path.
    pub path: String,
    /// Average line age in days.
    pub average_days: u64,
    /// Oldest line age in days.
    pub oldest_days: u64,
    /// Newest line age in days.
    pub newest_days: u64,
    /// Lines represented by this row.
    pub lines: u64,
}

/// Directory-level ownership aggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct DirectoryOwnership {
    /// Directory path.
    pub directory: String,
    /// Dominant author, if one exists.
    pub dominant_author: Option<String>,
    /// Lines covered by the dominant author.
    pub lines: u64,
    /// Number of files represented.
    pub files: u64,
}

/// A code-age bucket using the fixed RepoScope boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodeAgeBucket {
    /// Human-readable bucket label.
    pub bucket: String,
    /// Chinese compatibility label used by the old report.
    pub label: String,
    /// Files in this bucket.
    pub files: u64,
    /// Lines in this bucket.
    pub lines: u64,
    /// Percentage of blamed lines in this bucket.
    pub percentage: f64,
}

/// A redacted audit evidence item.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct EvidenceItem {
    /// Evidence category (region, dependency, delivery, footprint).
    pub category: String,
    /// Evidence kind.
    pub kind: String,
    /// Redacted value.
    pub value: String,
    /// Source path.
    pub path: Option<String>,
    /// One-based source line.
    pub line: Option<u64>,
    /// A cautious, non-factual conclusion.
    pub conclusion: String,
    /// Optional structured fields for report-specific consumers.  Values are
    /// already redacted before they cross the analysis boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// A report whose optional computation may finish independently.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReportSnapshot {
    /// Report identifier.
    pub kind: String,
    /// Whether the report has data in the active batch.
    pub available: bool,
    /// Redacted evidence rows.
    pub items: Vec<EvidenceItem>,
    /// Explicit limitations or failed optional stages.
    pub limitations: Vec<String>,
    /// Normalized report score when the source report defines one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Normalized risk/quality level, when defined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// Cautious report-level conclusion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    /// Number of files read by this optional stage.
    #[serde(default)]
    pub scanned_files: u64,
    /// Whether the stage hit one of its hard scan limits.
    #[serde(default)]
    pub truncated: bool,
}

/// Read-only repository diagnostics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDiagnostics {
    /// Whether the repository is bare.
    pub bare: bool,
    /// Worktree path, if present.
    pub worktree: Option<String>,
    /// Number of visible refs.
    pub refs: u64,
    /// Whether a stash ref exists.
    pub stash: bool,
    /// Whether a HEAD reflog exists.
    pub reflog: bool,
    /// Number of linked worktrees reported by Git metadata.
    pub worktrees: u64,
    /// Whether `.gitmodules` exists.
    pub submodules: bool,
    /// Number of hook files listed (hooks are never executed).
    pub hooks: u64,
    /// Whether Git LFS metadata is present.
    pub lfs: bool,
    /// Whether the object database is shallow.
    pub shallow: bool,
}

/// Safe file-content response with an explicit truncation flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "export-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    /// Validated relative path.
    pub path: String,
    /// Full commit id when the content was read from a historical tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Detected language label for the source path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// UTF-8 source content.
    pub content: String,
    /// Number of lines in the returned (possibly truncated) content.
    #[serde(default)]
    pub total_lines: u64,
    /// One-based line requested by the caller, clamped to the response.
    #[serde(default)]
    pub highlight_line: u64,
    /// Whether the response was truncated at the transport limit.
    pub truncated: bool,
}

#[cfg(feature = "export-schema")]
mod sdk_types {
    use super::*;

    but_schemars::register_sdk_type!(AnalysisRunStatus);
    but_schemars::register_sdk_type!(AnalysisPhase);
    but_schemars::register_sdk_type!(Freshness);
    but_schemars::register_sdk_type!(AnalysisStatus);
    but_schemars::register_sdk_type!(AnalysisConfig);
    but_schemars::register_sdk_type!(OverviewReport);
    but_schemars::register_sdk_type!(CommitSummary);
    but_schemars::register_sdk_type!(CommitFileDetail);
    but_schemars::register_sdk_type!(CommitDetail);
    but_schemars::register_sdk_type!(FileSummary);
    but_schemars::register_sdk_type!(AuthorSummary);
    but_schemars::register_sdk_type!(DailyActivity);
    but_schemars::register_sdk_type!(LanguageSummary);
    but_schemars::register_sdk_type!(CouplingSummary);
    but_schemars::register_sdk_type!(BusFactorReport);
    but_schemars::register_sdk_type!(RefSummary);
    but_schemars::register_sdk_type!(WorktreeSummary);
    but_schemars::register_sdk_type!(OwnershipRow);
    but_schemars::register_sdk_type!(OwnershipCoverage);
    but_schemars::register_sdk_type!(FileAgeStat);
    but_schemars::register_sdk_type!(DirectoryOwnership);
    but_schemars::register_sdk_type!(CodeAgeBucket);
    but_schemars::register_sdk_type!(EvidenceItem);
    but_schemars::register_sdk_type!(ReportSnapshot);
    but_schemars::register_sdk_type!(RepositoryDiagnostics);
    but_schemars::register_sdk_type!(FileContent);
}
