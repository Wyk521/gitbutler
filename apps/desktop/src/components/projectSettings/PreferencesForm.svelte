<script lang="ts">
	import ReduxResult from "$components/shared/ReduxResult.svelte";
	import SettingsSection from "$components/shared/SettingsSection.svelte";
	import { PROJECTS_SERVICE } from "$lib/project/projectsService";
	import { inject } from "@gitbutler/core/context";
	import { CardGroup, Toggle } from "@gitbutler/ui";

	const { projectId }: { projectId: string } = $props();
	const projectsService = inject(PROJECTS_SERVICE);
	const projectQuery = $derived(projectsService.getProject(projectId));
</script>

<ReduxResult {projectId} result={projectQuery.result}>
	{#snippet children(project)}
		<SettingsSection gap={8}>
			<CardGroup.Item standalone labelFor="omitCertificateCheck">
				{#snippet title()}
					忽略主机证书检查
				{/snippet}
				{#snippet caption()}
					启用后，使用 SSH 时忽略主机证书检查。
				{/snippet}
				{#snippet actions()}
					<Toggle
						id="omitCertificateCheck"
						checked={project.omit_certificate_check}
						onchange={async (value: boolean) => {
							await projectsService.updateProject({
								...project,
								omit_certificate_check: value,
							});
						}}
					/>
				{/snippet}
			</CardGroup.Item>
		</SettingsSection>
	{/snippet}
</ReduxResult>
