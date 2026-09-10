<script lang="ts">
	import ReduxResult from "$components/shared/ReduxResult.svelte";
	import { GIT_CONFIG_SERVICE } from "$lib/config/gitConfigService";
	import { PROJECTS_SERVICE } from "$lib/project/projectsService";
	import { inject } from "@gitbutler/core/context";
	import { CardGroup, Toggle } from "@gitbutler/ui";

	type Props = {
		projectId: string;
	};

	const { projectId }: Props = $props();

	const gbConfig = inject(GIT_CONFIG_SERVICE);
	const projectService = inject(PROJECTS_SERVICE);

	const isGerritProject = $derived(projectService.isGerritProject(projectId));
</script>

<div class="stack-v">
	<ReduxResult {projectId} result={isGerritProject.result}>
		{#snippet children(itIsAGerritProject)}
		<CardGroup.Item standalone labelFor="gerritModeToggle">
			{#snippet title()}
				Gerrit 配置
			{/snippet}

			{#snippet caption()}
				为该项目启用或禁用 Gerrit 模式。设置只保存在本机。
				{/snippet}

				{#snippet actions()}
					<Toggle
						id="gerritModeToggle"
						checked={itIsAGerritProject}
						onclick={() => gbConfig.setGerritMode(projectId, !itIsAGerritProject)}
					/>
				{/snippet}
			</CardGroup.Item>
		{/snippet}
	</ReduxResult>
</div>
