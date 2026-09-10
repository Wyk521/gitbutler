//! Deterministic RepoScope metrics.

use crate::model::{AuthorSummary, BusFactorReport, CouplingSummary, FileSummary, LanguageSummary};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A single file change from one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitChange {
    /// Display path (lossy UTF-8 is only used at this domain boundary).
    pub path: String,
    /// Previous path when Git recognized a rename or copy.
    pub previous_path: Option<String>,
    /// Lines added.
    pub additions: u64,
    /// Lines removed.
    pub deletions: u64,
}

/// The stable classification vocabulary used by the old RepoScope reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitClassification {
    /// New functionality.
    Feat,
    /// Bug fixes.
    Fix,
    /// Refactoring.
    Refactor,
    /// Documentation.
    Docs,
    /// Tests.
    Test,
    /// Maintenance.
    Chore,
    /// Build changes.
    Build,
    /// Continuous integration.
    Ci,
    /// Anything that does not match a known prefix.
    Other,
}

impl CommitClassification {
    /// Return the wire value expected by existing RepoScope clients.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Feat => "feat",
            Self::Fix => "fix",
            Self::Refactor => "refactor",
            Self::Docs => "docs",
            Self::Test => "test",
            Self::Chore => "chore",
            Self::Build => "build",
            Self::Ci => "ci",
            Self::Other => "other",
        }
    }
}

/// Classify a commit subject using conventional-commit prefixes and the
/// compatibility keywords used by the previous implementation.
#[must_use]
pub fn classify_commit(subject: &str) -> CommitClassification {
    let lower = subject.trim().to_ascii_lowercase();
    let token = lower.split([':', '(', '!', ' ']).next().unwrap_or_default();
    let classified = match token {
        "feat" | "feature" | "add" | "added" => CommitClassification::Feat,
        "fix" | "bugfix" | "hotfix" | "bug" => CommitClassification::Fix,
        "refactor" | "refactoring" | "perf" | "style" => CommitClassification::Refactor,
        "docs" | "doc" | "documentation" => CommitClassification::Docs,
        "test" | "tests" | "testing" => CommitClassification::Test,
        "chore" | "cleanup" | "maint" | "maintenance" => CommitClassification::Chore,
        "build" | "compile" => CommitClassification::Build,
        "ci" | "cd" | "pipeline" => CommitClassification::Ci,
        _ => CommitClassification::Other,
    };
    if classified != CommitClassification::Other {
        return classified;
    }
    // The Python baseline also classified non-conventional messages that
    // explicitly mention a fix/bug.  Keep that compatibility without making
    // broad keyword matches for every category.
    if lower.contains("修复") || lower.contains("bug") || lower.contains("fix") {
        CommitClassification::Fix
    } else {
        CommitClassification::Other
    }
}

/// Calculate a file hotspot score using the compatibility 55/30/15 weighting.
///
/// Each input is normalized with `log1p` against the largest value in the
/// report, which keeps a single generated file from flattening every other
/// score.  The returned value is always in `0..=100`.
#[must_use]
pub fn compute_hotspot_score(
    churn: u64,
    commit_frequency: u64,
    current_lines: u64,
    maxima: (u64, u64, u64),
) -> f64 {
    fn normalized(value: u64, maximum: u64) -> f64 {
        if maximum == 0 {
            0.0
        } else {
            (value as f64).ln_1p() / (maximum as f64).ln_1p()
        }
    }
    (55.0 * normalized(churn, maxima.0)
        + 30.0 * normalized(commit_frequency, maxima.1)
        + 15.0 * normalized(current_lines, maxima.2))
    .clamp(0.0, 100.0)
}

/// Compute the least number of contributors whose values cover half of the
/// total.  Zero-valued inputs return zero rather than the misleading value one.
#[must_use]
pub fn minimum_covering_half(values: &[(String, u64)]) -> u64 {
    let total: u64 = values.iter().map(|(_, value)| *value).sum();
    if total == 0 {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by_key(|right| std::cmp::Reverse(right.1));
    let mut covered = 0_u64;
    for (index, (_, value)) in sorted.iter().enumerate() {
        covered = covered.saturating_add(*value);
        if covered.saturating_mul(2) >= total {
            return (index + 1) as u64;
        }
    }
    sorted.len() as u64
}

/// Compute bus factor from current LOC ownership and historical commit counts.
#[must_use]
pub fn compute_bus_factor(
    owned_lines: &[(String, u64)],
    commit_counts: &[(String, u64)],
) -> BusFactorReport {
    BusFactorReport {
        loc: minimum_covering_half(owned_lines),
        commits: minimum_covering_half(commit_counts),
        insufficient_data: owned_lines.is_empty() && commit_counts.is_empty(),
    }
}

/// Compute language totals and percentages from `(language, lines)` file rows.
#[must_use]
pub fn compute_language_totals(
    files: impl IntoIterator<Item = (String, u64)>,
) -> Vec<LanguageSummary> {
    let mut totals: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for (language, lines) in files {
        let entry = totals.entry(language).or_default();
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(lines);
    }
    let all_lines: u64 = totals.values().map(|(_, lines)| *lines).sum();
    let mut result: Vec<_> = totals
        .into_iter()
        .map(|(language, (files, lines))| LanguageSummary {
            language,
            files,
            lines,
            percentage: if all_lines == 0 {
                0.0
            } else {
                lines as f64 * 100.0 / all_lines as f64
            },
        })
        .collect();
    result.sort_by(|left, right| {
        right
            .lines
            .cmp(&left.lines)
            .then_with(|| left.language.cmp(&right.language))
    });
    result
}

#[derive(Debug, Default)]
struct FileTotals {
    additions: u64,
    deletions: u64,
    commits: u64,
    last_changed_at: Option<i64>,
    touched_commits: BTreeSet<String>,
    authors: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct AuthorTotals {
    name: String,
    email: String,
    commits: u64,
    additions: u64,
    deletions: u64,
    files: BTreeSet<String>,
    first_commit_at: Option<i64>,
    last_commit_at: Option<i64>,
}

/// An in-memory accumulator used by the history scanner and fixture tests.
#[derive(Debug, Default)]
pub struct MetricAccumulator {
    file_totals: BTreeMap<String, FileTotals>,
    authors: HashMap<String, AuthorTotals>,
}

impl MetricAccumulator {
    /// Create an empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one commit and its first-parent file changes.
    pub fn ingest_commit(
        &mut self,
        oid: &str,
        author_name: &str,
        author_email: &str,
        authored_at: i64,
        changes: &[CommitChange],
    ) {
        let identity = format!("{}\0{}", author_name.trim(), author_email.trim());
        let author = self
            .authors
            .entry(identity.clone())
            .or_insert_with(|| AuthorTotals {
                name: author_name.to_owned(),
                email: author_email.to_owned(),
                ..AuthorTotals::default()
            });
        author.commits = author.commits.saturating_add(1);
        author.first_commit_at = Some(
            author
                .first_commit_at
                .map_or(authored_at, |value| value.min(authored_at)),
        );
        author.last_commit_at = Some(
            author
                .last_commit_at
                .map_or(authored_at, |value| value.max(authored_at)),
        );
        for change in changes {
            author.additions = author.additions.saturating_add(change.additions);
            author.deletions = author.deletions.saturating_add(change.deletions);
            author.files.insert(change.path.clone());
            let totals = self.file_totals.entry(change.path.clone()).or_default();
            totals.additions = totals.additions.saturating_add(change.additions);
            totals.deletions = totals.deletions.saturating_add(change.deletions);
            if totals.touched_commits.insert(oid.to_owned()) {
                totals.commits = totals.commits.saturating_add(1);
            }
            totals.last_changed_at = Some(
                totals
                    .last_changed_at
                    .map_or(authored_at, |current| current.max(authored_at)),
            );
            totals.authors.insert(identity.clone());
        }
    }

    /// Add current worktree file lines and optional language information.
    pub fn set_current_file(&mut self, path: &str, language: Option<&str>, lines: u64) {
        let totals = self.file_totals.entry(path.to_owned()).or_default();
        let _ = (language, lines);
        // A current-only file has no history, but still belongs in the report.
        totals.last_changed_at.get_or_insert(0);
    }

    /// Produce file rows with hotspot scores.  `current_lines` is keyed by the
    /// path after rename-chain normalization.
    #[must_use]
    pub fn files(&self, current_lines: &HashMap<String, u64>) -> Vec<FileSummary> {
        let maxima = self.file_totals.values().fold((0, 0, 0), |maxima, totals| {
            (
                maxima
                    .0
                    .max(totals.additions.saturating_add(totals.deletions)),
                maxima.1.max(totals.commits),
                maxima.2,
            )
        });
        let max_lines = current_lines.values().copied().max().unwrap_or(0);
        self.file_totals
            .iter()
            // File evolution is a current-snapshot report.  History-only
            // paths (deleted files) still contribute to commit totals and
            // couplings, but must not inflate the current file/LOC metrics or
            // produce rows that the worktree cannot preview.
            .filter(|(path, _)| current_lines.contains_key(path.as_str()))
            .map(|(path, totals)| {
                let lines = current_lines.get(path).copied().unwrap_or(0);
                FileSummary {
                    path: path.clone(),
                    raw_path_hex: None,
                    language: None,
                    tracked: false,
                    lines,
                    additions: totals.additions,
                    deletions: totals.deletions,
                    commit_count: totals.commits,
                    author_count: totals.authors.len() as u64,
                    hotspot_score: compute_hotspot_score(
                        totals.additions.saturating_add(totals.deletions),
                        totals.commits,
                        lines,
                        (maxima.0, maxima.1, max_lines),
                    ),
                    last_changed_at: (totals.last_changed_at != Some(0))
                        .then_some(totals.last_changed_at)
                        .flatten(),
                }
            })
            .collect()
    }

    /// Produce contributors ordered by commit count and then identity.
    #[must_use]
    pub fn authors(&self) -> Vec<AuthorSummary> {
        let mut result: Vec<_> = self
            .authors
            .iter()
            .map(|(identity, totals)| AuthorSummary {
                identity: identity.clone(),
                name: totals.name.clone(),
                email: totals.email.clone(),
                commit_count: totals.commits,
                additions: totals.additions,
                deletions: totals.deletions,
                owned_lines: 0,
                files_touched: totals.files.len() as u64,
                first_commit_at: totals.first_commit_at,
                last_commit_at: totals.last_commit_at,
            })
            .collect();
        result.sort_by(|left, right| {
            right
                .commit_count
                .cmp(&left.commit_count)
                .then_with(|| left.identity.cmp(&right.identity))
        });
        result
    }
}

/// Count pairs of files changed together in the same commit.
///
/// Commits with more than `max_files` are ignored to avoid quadratic memory
/// use.  Only pairs occurring at least twice are returned, capped at `limit`.
#[must_use]
pub fn compute_couplings(
    commits: impl IntoIterator<Item = Vec<String>>,
    max_files: usize,
    limit: usize,
) -> Vec<CouplingSummary> {
    let mut counts: HashMap<(String, String), u64> = HashMap::new();
    for mut files in commits {
        files.sort();
        files.dedup();
        if files.is_empty() || files.len() > max_files {
            continue;
        }
        for (left_index, left) in files.iter().enumerate() {
            for right in files.iter().skip(left_index + 1) {
                *counts.entry((left.clone(), right.clone())).or_default() += 1;
            }
        }
    }
    let maximum = counts.values().copied().max().unwrap_or(0);
    let mut rows: Vec<_> = counts
        .into_iter()
        .filter_map(|((left, right), co_changes)| {
            (co_changes >= 2).then(|| {
                CouplingSummary {
                    left,
                    right,
                    co_changes,
                    // The legacy RepoScope API exposed this as a percentage
                    // relative to the most frequently co-changing pair.
                    strength: if maximum == 0 {
                        0.0
                    } else {
                        co_changes as f64 / maximum as f64 * 100.0
                    },
                }
            })
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .co_changes
            .cmp(&left.co_changes)
            .then_with(|| right.strength.total_cmp(&left.strength))
            .then_with(|| left.left.cmp(&right.left))
            .then_with(|| left.right.cmp(&right.right))
    });
    rows.truncate(limit);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotspot_uses_compatibility_weights() {
        let score = compute_hotspot_score(100, 10, 100, (100, 10, 100));
        assert!(
            (score - 100.0).abs() < f64::EPSILON,
            "maximum inputs score 100"
        );
    }

    #[test]
    fn coupling_requires_two_commits() {
        let rows = compute_couplings(
            vec![vec!["a".into(), "b".into()], vec!["a".into(), "b".into()]],
            80,
            500,
        );
        assert_eq!(rows[0].co_changes, 2, "a pair repeated twice is retained");
        assert!((rows[0].strength - 100.0).abs() < f64::EPSILON);
        assert!(compute_couplings(vec![vec!["a".into(), "b".into()]], 80, 500).is_empty());
    }

    #[test]
    fn classify_known_prefixes() {
        assert_eq!(
            classify_commit("fix(parser): handle UTF-8"),
            CommitClassification::Fix
        );
        assert_eq!(
            classify_commit("something happened"),
            CommitClassification::Other
        );
        assert_eq!(classify_commit("修复导入边界"), CommitClassification::Fix);
    }

    #[test]
    fn bus_factor_counts_half() {
        let report = compute_bus_factor(
            &[("a".into(), 60), ("b".into(), 40)],
            &[("a".into(), 1), ("b".into(), 1)],
        );
        assert_eq!(report.loc, 1, "one author covers at least half of LOC");
        assert_eq!(
            report.commits, 1,
            "one author covers at least half of commits"
        );
    }

    #[test]
    fn file_rows_only_include_the_current_snapshot() {
        let mut accumulator = MetricAccumulator::new();
        accumulator.ingest_commit(
            "deadbeef",
            "Alice",
            "alice@example.com",
            1,
            &[CommitChange {
                path: "deleted.rs".into(),
                previous_path: None,
                additions: 10,
                deletions: 0,
            }],
        );
        accumulator.set_current_file("live.rs", Some("Rust"), 3);
        let rows = accumulator.files(&HashMap::from([(String::from("live.rs"), 3)]));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "live.rs");
    }
}
