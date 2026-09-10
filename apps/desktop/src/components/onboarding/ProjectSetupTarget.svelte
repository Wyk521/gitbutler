<script lang="ts">
	import { goto } from "$app/navigation";
	import ProjectNameLabel from "$components/shared/ProjectNameLabel.svelte";
	import ReduxResult from "$components/shared/ReduxResult.svelte";
	import gerritLogoSvg from "$lib/assets/gerrit-logo.svg?raw";
	import { getBestBranch, getBestRemote, getBranchRemoteFromRef } from "$lib/branches/branchUtils";
	import { projectLandDirectly } from "$lib/config/config";
	import { GIT_CONFIG_SERVICE } from "$lib/config/gitConfigService";
	import { classify, type ClassifiedError } from "$lib/error/errorClassification";
	import { PROJECTS_SERVICE } from "$lib/project/projectsService";
	import { SETTINGS_SERVICE } from "$lib/settings/appSettings";
	import { combineResults } from "$lib/state/helpers";
	import { unique } from "$lib/utils/array";
	import { inject } from "@gitbutler/core/context";
	import {
		Button,
		CardGroup,
		Checkbox,
		Icon,
		InfoMessage,
		Select,
		SelectItem,
		TestId,
		Toggle,
	} from "@gitbutler/ui";
	import { slide } from "svelte/transition";
	import type { RemoteBranchInfo } from "$lib/baseBranch/baseBranch";

	interface Props {
		projectId: string;
		projectName: string;
		remoteBranches: RemoteBranchInfo[];
		onBranchSelected: (branch: [branchName: string, pushRemote: string]) => Promise<void>;
		onOpenProject: () => Promise<boolean>;
	}

	const { projectId, projectName, remoteBranches, onBranchSelected, onOpenProject }: Props =
		$props();

	const gitConfig = inject(GIT_CONFIG_SERVICE);
	const settingsStore = inject(SETTINGS_SERVICE).appSettings;

	const gbConfig = $derived(gitConfig.gbConfig(projectId));
	const gerritMode = $derived(gbConfig.response?.gitbutlerGerritMode ?? false);

	const landDirectly = $derived(projectLandDirectly(projectId));

	let loading = $state<boolean>(false);
	let targetWasSet = $state(false);
	let targetError = $state<ClassifiedError>();
	let showMoreInfo = $state<boolean>(false);

	// split all the branches by the first '/' and gather the unique remote names
	// then turn remotes into an array of objects with a 'name' and 'value' key
	const remotes = $derived(
		unique(remoteBranches.map((b) => getBranchRemoteFromRef(b.name))).filter(
			(r): r is string => !!r,
		),
	);

	let selectedBranch = $state<RemoteBranchInfo | undefined>(undefined);
	const defaultBranch = $derived(getBestBranch(remoteBranches.slice()));
	const branch = $derived(selectedBranch ?? defaultBranch);

	let selectedRemote = $state<string | undefined>(undefined);
	const defaultRemote = $derived(
		(branch && getBranchRemoteFromRef(branch.name)) ?? getBestRemote(remotes),
	);
	const remote = $derived(selectedRemote ?? defaultRemote);

	async function onSetTargetClick() {
		if (!branch || !remote || loading) return;
		loading = true;
		targetError = undefined;
		if (!targetWasSet) {
			try {
				await onBranchSelected([branch.name, remote]);
				targetWasSet = true;
			} catch (error) {
				const classified = classify(error);
				if (classified.severity !== "silent") targetError = classified;
				loading = false;
				return;
			}
		}
		if (await onOpenProject()) return;
		loading = false;
	}

	const projectsService = inject(PROJECTS_SERVICE);
	async function deleteProjectAndGoBack() {
		await projectsService.deleteProject(projectId);
		await projectsService.fetchProjects();
		await goto("/");
	}

	const itSmellsLikeGerrit = $derived(projectsService.areYouGerritKiddingMe(projectId));
	const projectIsGerrit = $derived(projectsService.isGerritProject(projectId));
</script>

<div class="project-setup">
	<div class="stack-v gap-4">
		<ProjectNameLabel {projectName} />
		<h1 class="text-serif-42">配置你的<i>工作区</i></h1>
	</div>

	<div class="project-setup__fields">
		<div class="project-setup__field-wrap" data-testid={TestId.ProjectSetupPageTargetBranchSelect}>
			<Select
				value={branch?.name}
				options={remoteBranches.map((b) => ({ label: b.name, value: b.name }))}
				disabled={loading || targetWasSet}
				wide
				onselect={(value) => {
					selectedBranch = { name: value };
				}}
				label="目标分支"
				searchable
			>
				{#snippet itemSnippet({ item, highlighted })}
					<SelectItem selected={item.value === branch?.name} {highlighted}>
						{item.label}
					</SelectItem>
				{/snippet}
			</Select>

			<p class="text-12 text-body project-setup__field-caption">
				用于对比和整合的主分支，通常是
				<code class="code-string">origin/master</code> 或
				<code class="code-string">upstream/main</code>。这里只读取本机已有的远程跟踪引用。
			</p>
		</div>

		{#if remotes.length > 1}
			<div class="project-setup__field-wrap">
				<Select
					value={remote}
					options={remotes.map((r) => ({ label: r, value: r }))}
					disabled={loading || targetWasSet}
					onselect={(value) => {
						const newSelectedRemote = remotes.find((r) => r === value);
						selectedRemote = newSelectedRemote ?? remote;
					}}
				>
					{#snippet itemSnippet({ item, highlighted })}
						<SelectItem selected={item.value === remote} {highlighted}>
							{item.label}
						</SelectItem>
					{/snippet}
				</Select>

				<p class="text-12 text-body clr-text-2">
					检测到多个本机远程跟踪命名空间。可在这里选择新分支默认关联的本地远程名。
				</p>
			</div>
		{/if}

		<ReduxResult
			{projectId}
			result={combineResults(itSmellsLikeGerrit.result, projectIsGerrit.result)}
		>
			{#snippet error()}
				<!-- Fail silently to detect the gerritness of a project -->
				<div></div>
			{/snippet}
			{#snippet children([isGerrit])}
				{#if isGerrit}
					<CardGroup.Item standalone labelFor="gerritToggle">
						{#snippet iconSide()}
							{@html gerritLogoSvg}
						{/snippet}
						{#snippet title()}
							启用 Gerrit 项目模式
						{/snippet}
						{#snippet caption()}
							该项目看起来可能使用 Gerrit。
							<br />
							是否启用 Gerrit 模式？
							<br />
							之后可在项目设置中调整。
						{/snippet}
						{#snippet actions()}
							<Toggle
								id="gerritToggle"
								checked={gerritMode}
								onclick={() => {
									gitConfig.setGerritMode(projectId, !gerritMode);
								}}
							/>
						{/snippet}
					</CardGroup.Item>
				{/if}
			{/snippet}
		</ReduxResult>
	</div>

	<label for="landDirectly" class="land-directly">
		<Checkbox name="landDirectly" small bind:checked={$landDirectly} />
		<span class="text-12 clr-text-2">直接整合到主分支（跳过评审流程）</span>
	</label>

	<!-- With the singleBranch feature flag, setting the target only updates project
	     metadata and the user stays on their current branch, so don't promise a
	     switch to gitbutler/workspace. -->
	{#if !$settingsStore?.featureFlags.singleBranch}
		<div
			class="project-setup__info"
			role="presentation"
			onclick={() => (showMoreInfo = !showMoreInfo)}
		>
			<div class="project-setup__fold-icon" class:rotate-icon={showMoreInfo}>
				<Icon name="chevron-right" />
			</div>

			<div class="stack-v gap-6 full-width">
				<div class="project-setup__info__title">
					<svg
						width="16"
						height="13"
						viewBox="0 0 16 13"
						fill="none"
						xmlns="http://www.w3.org/2000/svg"
					>
						<path
							d="M2 12L3.5 7.5M14 12L12.5 7.5M12.5 7.5L11 3H5L3.5 7.5M12.5 7.5H3.5"
							stroke="#D96842"
							stroke-width="1.5"
						/>
						<path
							d="M1.24142 3H14.7586C14.8477 3 14.8923 2.89229 14.8293 2.82929L13.0293 1.02929C13.0105 1.01054 12.9851 1 12.9586 1H3.04142C3.0149 1 2.98946 1.01054 2.97071 1.02929L1.17071 2.82929C1.10771 2.89229 1.15233 3 1.24142 3Z"
							fill="#FF9774"
							stroke="#FF9774"
							stroke-width="1.5"
						/>
					</svg>

					<h3 class="text-13 text-body text-semibold">
						RepoScope Desktop 会将活动分支切换到 <span class="text-bold"
							>gitbutler/workspace</span
						>
					</h3>
				</div>

				{#if showMoreInfo}
					<p class="text-12 text-body" transition:slide={{ duration: 200 }}>
						为了同时管理多个分支，应用会创建并维护特殊分支
						<span class="text-bold">gitbutler/workspace</span>。你仍可随时在普通 Git 分支和
						工作区之间切换；RepoScope 分析不会写入这些引用。
					</p>
				{/if}
			</div>
		</div>
	{/if}

	{#if targetError}
		{@const error = targetError}
		<InfoMessage style={error.severity === "warning" ? "warning" : "danger"}>
			{#snippet content()}
				{error.userMessage ?? error.message}
			{/snippet}
		</InfoMessage>
	{/if}

	<div class="action-buttons">
		<Button kind="outline" disabled={loading} onclick={deleteProjectAndGoBack}>取消</Button>
		<Button
			style="pop"
			{loading}
			onclick={onSetTargetClick}
			icon="chevron-right"
			testId={TestId.ProjectSetupPageTargetContinueButton}
			id="set-base-branch"
		>
			{targetWasSet ? "打开项目" : "继续"}
		</Button>
	</div>
</div>

<style lang="postcss">
	.project-setup {
		display: flex;
		flex-direction: column;
		gap: 20px;
	}

	.project-setup__fields {
		display: flex;
		flex-direction: column;
		gap: 20px;
	}

	.project-setup__field-wrap {
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.project-setup__field-caption {
		width: 90%;
		color: var(--text-2);
	}

	.land-directly {
		display: flex;
		align-items: center;
		gap: 8px;
		cursor: pointer;
	}

	.action-buttons {
		display: flex;
		justify-content: flex-end;
		width: 100%;
		gap: 8px;
	}

	/* BANNER */
	.project-setup__info {
		display: flex;
		padding: 14px 16px;
		gap: 8px;
		border-radius: var(--radius-m);
		background-color: var(--bg-mute);
		cursor: pointer;

		&:hover {
			& .project-setup__fold-icon {
				color: var(--text-2);
			}
		}
	}

	.project-setup__info__title {
		display: inline;
		width: 100%;
		gap: 8px;

		svg {
			display: inline;
			margin-right: 8px;
			float: left;
			transform: translateY(4px);
		}
	}

	.project-setup__fold-icon {
		display: flex;
		align-self: flex-start;
		padding-top: 2px;
		color: var(--text-3);
		transition:
			transform var(--transition-medium),
			color var(--transition-fast);

		&.rotate-icon {
			transform: rotate(90deg);
		}
	}
</style>
