<script lang="ts">
	import { goto } from "$app/navigation";
	import ProjectSettingsShortcutHandler from "$components/settings/ProjectSettingsShortcutHandler.svelte";
	import FullviewLoading from "$components/shared/FullviewLoading.svelte";
	import NotOnGitButlerBranch from "$components/shared/NotOnGitButlerBranch.svelte";
	import ProjectShortcutHandler from "$components/shared/ProjectShortcutHandler.svelte";
	import ReduxResult from "$components/shared/ReduxResult.svelte";
	import AppLayout from "$components/views/AppLayout.svelte";
	import NoBaseBranch from "$components/views/NoBaseBranch.svelte";
	import ProblemLoadingRepo from "$components/views/ProblemLoadingRepo.svelte";
	import { BACKEND } from "$lib/backend";
	import { BASE_BRANCH_SERVICE } from "$lib/baseBranch/baseBranchService.svelte";
	import { showError } from "$lib/error/showError";
	import { MODE_SERVICE } from "$lib/mode/modeService";
	import { showInfo, showWarning } from "$lib/notifications/toasts";
	import { PROJECTS_SERVICE } from "$lib/project/projectsService";
	import { RepoScopeService } from "$lib/reposcope/service.svelte";
	import { isInsightsPath } from "$lib/routes/routes.svelte";
	import { FILE_SELECTION_MANAGER } from "$lib/selection/fileSelectionManager.svelte";
	import { UNCOMMITTED_SERVICE } from "$lib/selection/uncommittedService.svelte";
	import { SETTINGS_SERVICE } from "$lib/settings/appSettings";
	import { STACK_SERVICE } from "$lib/stacks/stackService.svelte";
	import { CLIENT_STATE } from "$lib/state/clientState.svelte";
	import { combineResults } from "$lib/state/helpers";
	import { invalidatesList, ReduxTag } from "$lib/state/tags";
	import { WORKTREE_SERVICE } from "$lib/worktree/worktreeService.svelte";
	import { inject } from "@gitbutler/core/context";
	import { mergeUnlisten } from "@gitbutler/ui/utils/mergeUnlisten";
	import { untrack, type Snippet } from "svelte";
	import type { LayoutData } from "./$types";

	const { data, children: pageChildren }: { data: LayoutData; children: Snippet } = $props();

	// =============================================================================
	// PROJECT SETUP & CORE STATE
	// =============================================================================

	const { projectId } = $derived(data);

	// Core services
	const settingsService = inject(SETTINGS_SERVICE);
	const settingsStore = settingsService.appSettings;
	const projectsService = inject(PROJECTS_SERVICE);
	const clientState = inject(CLIENT_STATE);

	// Project data
	const projectsQuery = $derived(projectsService.projects());
	const projects = $derived(projectsQuery.response);
	const currentProject = $derived(projects?.find((p) => p.id === projectId));
	const reposcopeService = new RepoScopeService();
	let autoAnalysisProject = $state<string | undefined>();
	$effect(() => {
		if (!projectId || autoAnalysisProject === projectId) return;
		autoAnalysisProject = projectId;
		void (async () => {
			try {
				const existing = await reposcopeService.status(projectId);
				if (!existing || existing.status === "interrupted") {
					await reposcopeService.start(projectId);
				}
			} catch {
				// The insights page exposes the actionable error.  Project setup must
				// remain usable when a repository is temporarily unavailable.
			}
		})();
	});

	// =============================================================================
	// REPOSITORY & BRANCH MANAGEMENT
	// =============================================================================

	const baseBranchService = inject(BASE_BRANCH_SERVICE);

	const baseBranchQuery = $derived(baseBranchService.baseBranch(projectId));
	const baseBranch = $derived(baseBranchQuery.response);

	// =============================================================================
	// WORKSPACE & MODE MANAGEMENT
	// =============================================================================

	const modeService = inject(MODE_SERVICE);
	const stackService = inject(STACK_SERVICE);
	const worktreeService = inject(WORKTREE_SERVICE);

	const modeQuery = $derived(modeService.mode(projectId));

	// =============================================================================
	// FILE SELECTION & WORKTREE MANAGEMENT
	// ================================================ReorderDropzoneFactory

	const uncommittedService = inject(UNCOMMITTED_SERVICE);
	const idSelection = inject(FILE_SELECTION_MANAGER);

	const worktreeDataQuery = $derived(worktreeService.worktreeData(projectId));
	const worktreeData = $derived(worktreeDataQuery.response);

	// Bridge between RTKQ and custom slice
	$effect(() => {
		if (worktreeData) {
			untrack(() => {
				uncommittedService.updateData({
					changes: worktreeData.rawChanges,
					assignments: worktreeData.hunkAssignments,
				});
			});
		}
	});

	// Clear expired file selections
	const affectedPaths = $derived(worktreeData?.rawChanges.map((c) => c.path));
	$effect(() => {
		if (affectedPaths) {
			untrack(() => {
				idSelection.retain(affectedPaths);
			});
		}
	});

	// =============================================================================
	// WINDOW TITLE
	// =============================================================================

	const backend = inject(BACKEND);
	$effect(() => {
		let baseTitle: string;
		let windowTitle: string;
		const projectTitle = currentProject?.title;

		Promise.all([backend.getAppInfo(), backend.getWindowTitle()]).then(
			([appInfo, currentTitle]) => {
				baseTitle = appInfo.name;

				if (!currentTitle.includes(" — ")) {
					windowTitle = currentTitle;
				}

				if (projectTitle) {
					backend.setWindowTitle(`${projectTitle} — ${baseTitle}`);
				}
			},
		);

		return () => {
			if (windowTitle) {
				backend.setWindowTitle(windowTitle);
			} else if (baseTitle) {
				backend.setWindowTitle(baseTitle);
			}
		};
	});

	// =============================================================================
	// FEED & UPDATES MANAGEMENT
	// =============================================================================

	const headResponse = $derived(modeService.head(projectId));
	const head = $derived(headResponse.response);

	// Invalidate caches in response to backend events.
	$effect(() =>
		mergeUnlisten(
			backend.listen(`project://${projectId}/hunk-assignment-update`, () => {
				stackService.invalidateStacksAndDetails();
			}),
			// A symbolic HEAD change can switch branches without changing the commit SHA.
			backend.listen(`project://${projectId}/git/head`, () => {
				stackService.invalidateStacksAndDetails();
			}),
			backend.listen(`project://${projectId}/worktree_changes`, () => {
				clientState.dispatch(
					clientState.backendApi.util.invalidateTags([invalidatesList(ReduxTag.Diff)]),
				);
			}),
			// Activity that requires re-reading workspace state — emitted on
			// remote-ref updates (push, external fetch) and on external
			// writes to `virtual_branches.toml` (e.g. by the `but` CLI).
			backend.listen(`project://${projectId}/workspace-activity`, () => {
				clientState.dispatch(
					clientState.backendApi.util.invalidateTags([
						invalidatesList(ReduxTag.Stacks),
						invalidatesList(ReduxTag.StackDetails),
						invalidatesList(ReduxTag.WorktreeChanges),
						invalidatesList(ReduxTag.IntegrationSteps),
						invalidatesList(ReduxTag.BranchListing),
					]),
				);
			}),
		),
	);

	// If the head changes, invalidate stacks and details
	// We need to track the previous head value to avoid infinite loops
	let previousHead = $state<string | undefined>(undefined);
	$effect(() => {
		if (head && head !== previousHead) {
			untrack(() => {
				previousHead = head;
				stackService.invalidateStacksAndDetails();
			});
		}
	});

	// =============================================================================
	// PROJECT LIFECYCLE & NAVIGATION
	// =============================================================================

	// RepoScope Desktop never schedules a fetch. Local watcher events above
	// still invalidate read-side state after edits made in this application.
	$effect(() => {
		if (!projectId) goto("/onboarding");
	});

	// Set active project and handle notifications
	async function setActiveProjectOrRedirect(projectId: string) {
		const dontShowAgainKey = `git-filters--dont-show-again--${projectId}`;
		try {
			const info = await projectsService.setActiveProject(projectId);

			if (!info) return;

			if (!info.is_exclusive) {
				showInfo(
					"该项目已在另一个窗口打开",
					"同时在多个窗口打开可能导致状态不同步",
				);
			}

			if (info.db_error) {
				showError("项目数据库异常", info.db_error);
			}

			if (info.headsup && localStorage.getItem(dontShowAgainKey) !== "1") {
				showWarning("重要提示", info.headsup, {
					label: "不再提示",
					onClick: (dismiss) => {
						localStorage.setItem(dontShowAgainKey, "1");
						dismiss();
					},
				});
			}
		} catch (error: unknown) {
			showError("设置当前项目失败", error);
		}
	}

	$effect(() => {
		setActiveProjectOrRedirect(projectId);
	});

	// Clear backend API state when project changes
	$effect(() => {
		if (projectId) {
			clientState.backendApi.util.resetApiState();
		}
	});

</script>

<ProjectSettingsShortcutHandler {projectId} />
<ProjectShortcutHandler />

{#if isInsightsPath()}
	<div class="view-wrap insights-wrap">
		<AppLayout {projectId}>
			{@render pageChildren()}
		</AppLayout>
	</div>
{:else}
	<ReduxResult {projectId} result={combineResults(baseBranchQuery.result, modeQuery.result)}>
		{#snippet children([baseBranch, mode], { projectId })}
			{#if !baseBranch}
				<NoBaseBranch {projectId} />
			{:else if baseBranch}
				{#if mode.type === "OpenWorkspace" || mode.type === "Edit" || ($settingsStore?.featureFlags.singleBranch && mode.subject.branchName)}
					<div class="view-wrap" role="group" ondragover={(e) => e.preventDefault()}>
						<AppLayout {projectId} sidebarDisabled={mode.type === "Edit"}>
							{@render pageChildren()}
						</AppLayout>
					</div>
				{:else if mode.type === "OutsideWorkspace"}
					<NotOnGitButlerBranch {projectId} {baseBranch}>
						{@render pageChildren()}
					</NotOnGitButlerBranch>
				{/if}
			{/if}
		{/snippet}
		{#snippet loading()}
			<FullviewLoading />
		{/snippet}
		{#snippet error(baseError)}
			<ProblemLoadingRepo {projectId} error={baseError} />
		{/snippet}
	</ReduxResult>
{/if}


<style>
	.view-wrap {
		display: flex;
		position: relative;
		width: 100%;
	}

	.insights-wrap {
		min-height: 100%;
	}
</style>
