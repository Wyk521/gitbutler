<script lang="ts">
	import GeneralSettings from "$components/projectSettings/GeneralSettings.svelte";
	import GitForm from "$components/projectSettings/GitForm.svelte";
	import PreferencesForm from "$components/projectSettings/PreferencesForm.svelte";
	import SettingsModalLayout from "$components/settings/SettingsModalLayout.svelte";
	import { projectSettingsPages } from "$lib/settings/projectSettingsPages";
	import type { ProjectSettingsModalState, ProjectSettingsPageId } from "$lib/state/uiState.svelte";

	type Props = {
		data: ProjectSettingsModalState;
	};

	const { data }: Props = $props();

	const pages = projectSettingsPages;

	let currentSelectedId = $derived(data.selectedId || pages.at(0)?.id);

	function selectPage(pageId: ProjectSettingsPageId) {
		currentSelectedId = pageId;
	}
</script>

	<SettingsModalLayout
	title="项目设置"
	{pages}
	selectedId={currentSelectedId}
	onSelectPage={selectPage}
>
	{#snippet content({ currentPage })}
		{#if currentPage}
			{#if currentPage.id === "project"}
				<GeneralSettings projectId={data.projectId} />
			{:else if currentPage.id === "git"}
				<GitForm projectId={data.projectId} />
			{:else if currentPage.id === "experimental"}
				<PreferencesForm projectId={data.projectId} />
			{:else}
				未找到设置页面：{currentPage.id}
			{/if}
		{:else}
			未找到设置页面：{currentSelectedId}
		{/if}
	{/snippet}
</SettingsModalLayout>
