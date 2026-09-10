/**
 * RepoScope transport types are generated from the Rust `but_api` schemas.
 * Keep only the generic page adapter here: the SDK generator currently emits
 * concrete DTOs but cannot represent a generic TypeScript envelope.
 */
export type {
	AnalysisConfig,
	AnalysisConfigInput,
	AnalysisPhase,
	AnalysisRunStatus,
	AnalysisStatus,
	AuthorSummary,
	BusFactorReport,
	CodeAgeBucket,
	CommitDetail,
	CommitFileDetail,
	CommitSummary,
	CouplingSummary,
	DailyActivity,
	DirectoryOwnership,
	EvidenceItem,
	FileAgeStat,
	FileContent,
	FileSummary,
	Freshness,
	LanguageSummary,
	OwnershipCoverage,
	OwnershipRow,
	OverviewReport,
	RefSummary,
	ReportSnapshot,
	RepositoryDiagnostics,
	WorktreeSummary,
} from "@gitbutler/but-sdk";

import type { OverviewReport } from "@gitbutler/but-sdk";

/** Short local alias used by the RepoScope UI. */
export type Overview = OverviewReport;

/** Generic paging envelope shared by all RepoScope list endpoints. */
export type Page<T> = {
	items: T[];
	total: number;
	page: number;
	pageSize: number;
};
