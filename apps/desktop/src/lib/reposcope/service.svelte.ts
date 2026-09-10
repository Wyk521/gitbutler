import { BACKEND } from "$lib/backend";
import { inject } from "@gitbutler/core/context";
import type {
	AnalysisConfig,
	AnalysisStatus,
	AuthorSummary,
	BusFactorReport,
	CodeAgeBucket,
	CommitSummary,
	CommitDetail,
	CouplingSummary,
	DailyActivity,
	DirectoryOwnership,
	FileSummary,
	FileAgeStat,
	FileContent,
	LanguageSummary,
	Overview,
	OwnershipRow,
	Page,
	RefSummary,
	ReportSnapshot,
	RepositoryDiagnostics,
} from "./types";

/** Build a local-only source preview URL with encoded path and optional line. */
export function filePreviewHref(
	projectId: string,
	path: string,
	line?: number,
	revision?: string,
) {
	const params = new URLSearchParams({ path });
	if (line !== undefined) params.set("line", String(Math.max(1, Math.floor(line))));
	if (revision) params.set("revision", revision);
	return `/${projectId}/insights/file?${params.toString()}`;
}

export class RepoScopeService {
	private readonly backend = inject(BACKEND);

	async status(projectId: string) {
		return await this.backend.invoke<AnalysisStatus | null>("reposcope_analysis_status", { projectId });
	}

	async start(projectId: string) {
		return await this.backend.invoke<AnalysisStatus>("reposcope_analysis_start", {
			projectId,
			config: null,
		});
	}

	async config(projectId: string) {
		return await this.backend.invoke<AnalysisConfig>("reposcope_analysis_config", { projectId });
	}

	async saveConfig(projectId: string, config: Partial<AnalysisConfig>) {
		return await this.backend.invoke<AnalysisConfig>("reposcope_analysis_config_set", {
			projectId,
			config,
		});
	}

	async cancel(projectId: string) {
		return await this.backend.invoke<AnalysisStatus>("reposcope_analysis_cancel", { projectId });
	}

	async overview(projectId: string) {
		return await this.backend.invoke<Overview | null>("reposcope_overview", { projectId });
	}

	async activity(projectId: string) {
		return await this.backend.invoke<DailyActivity[]>("reposcope_activity", { projectId });
	}

	async commits(projectId: string, page = 1, pageSize = 50, query?: string) {
		return await this.backend.invoke<Page<CommitSummary>>("reposcope_commits", {
			projectId,
			page,
			pageSize,
			query: query ?? null,
		});
	}

	async commitDetail(projectId: string, oid: string) {
		return await this.backend.invoke<CommitDetail | null>("reposcope_commit_detail", {
			projectId,
			oid,
		});
	}

	async files(projectId: string, page = 1, pageSize = 50) {
		return await this.backend.invoke<Page<FileSummary>>("reposcope_hotspots", {
			projectId,
			page,
			pageSize,
		});
	}

	async fileEvolution(projectId: string, page = 1, pageSize = 50, query?: string) {
		return await this.backend.invoke<Page<FileSummary>>("reposcope_files", {
			projectId,
			page,
			pageSize,
			query: query ?? null,
		});
	}

	async fileDetail(projectId: string, path: string) {
		return await this.backend.invoke<FileSummary | null>("reposcope_file_detail", {
			projectId,
			path,
		});
	}

	async fileContent(projectId: string, path: string, line = 1, revision?: string) {
		return await this.backend.invoke<FileContent>("reposcope_file_content", {
			projectId,
			path,
			line,
			revision: revision ?? null,
		});
	}

	async authors(projectId: string, page = 1, pageSize = 50, query?: string) {
		return await this.backend.invoke<Page<AuthorSummary>>("reposcope_authors", {
			projectId,
			page,
			pageSize,
			query: query ?? null,
		});
	}

	async coupling(projectId: string, page = 1, pageSize = 50) {
		return await this.backend.invoke<Page<CouplingSummary>>("reposcope_coupling", {
			projectId,
			page,
			pageSize,
		});
	}

	async ownership(projectId: string, page = 1, pageSize = 50) {
		return await this.backend.invoke<Page<OwnershipRow>>("reposcope_ownership", {
			projectId,
			page,
			pageSize,
		});
	}

	async directoryOwnership(projectId: string, page = 1, pageSize = 50) {
		return await this.backend.invoke<Page<DirectoryOwnership>>("reposcope_directory_ownership", {
			projectId,
			page,
			pageSize,
		});
	}

	async codeAge(projectId: string) {
		return await this.backend.invoke<CodeAgeBucket[]>("reposcope_code_age", { projectId });
	}

	async fileAgeStats(projectId: string, page = 1, pageSize = 50) {
		return await this.backend.invoke<Page<FileAgeStat>>("reposcope_file_age_stats", {
			projectId,
			page,
			pageSize,
		});
	}

	async busFactor(projectId: string) {
		return await this.backend.invoke<BusFactorReport>("reposcope_bus_factor", { projectId });
	}

	async refs(projectId: string, page = 1, pageSize = 100) {
		return await this.backend.invoke<Page<RefSummary>>("reposcope_refs", {
			projectId,
			page,
			pageSize,
		});
	}

	async languages(projectId: string) {
		return await this.backend.invoke<LanguageSummary[]>("reposcope_languages", { projectId });
	}

	async diagnostics(projectId: string) {
		return await this.backend.invoke<RepositoryDiagnostics>("reposcope_repository_diagnostics", {
			projectId,
		});
	}

	async report(projectId: string, kind: "delivery" | "footprints" | "regions" | "dependencies") {
		return await this.backend.invoke<ReportSnapshot>(`reposcope_${kind}`, { projectId });
	}

	listen(projectId: string, callback: (status: AnalysisStatus) => void) {
		return this.backend.listen<AnalysisStatus>(`project://${projectId}/reposcope-analysis`, (event) =>
			callback(event.payload),
		);
	}

	/**
	 * Refresh the displayed freshness after a local Git/worktree event.  The
	 * watcher only invalidates this read-side state; it never starts a heavy
	 * analysis automatically.
	 */
	listenRepositoryChanges(projectId: string, callback: () => void) {
		const events = [
			`project://${projectId}/git/head`,
			`project://${projectId}/worktree_changes`,
			`project://${projectId}/workspace-activity`,
		];
		return Promise.all(
			events.map((event) => this.backend.listen(event, callback)),
		).then((unlisteners) => () => {
			for (const unlisten of unlisteners) unlisten();
		});
	}
}
