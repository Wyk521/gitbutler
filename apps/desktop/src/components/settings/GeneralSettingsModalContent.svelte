<script lang="ts">
	import AppearanceSettings from "$components/projectSettings/AppearanceSettings.svelte";
	import ExperimentalSettings from "$components/settings/ExperimentalSettings.svelte";
	import OfflineGeneralSettings from "$components/settings/OfflineGeneralSettings.svelte";
	import GitSettings from "$components/settings/GitSettings.svelte";
	import LanesAndBranchesSettings from "$components/settings/LanesAndBranchesSettings.svelte";
	import OfflineCapabilityNotice from "$components/settings/OfflineCapabilityNotice.svelte";
	import SettingsModalLayout from "$components/settings/SettingsModalLayout.svelte";
	import { generalSettingsPages } from "$lib/settings/generalSettingsPages";
	import type { GeneralSettingsModalState, GeneralSettingsPageId } from "$lib/state/uiState.svelte";

	type Props = {
		data: GeneralSettingsModalState;
	};

	const { data }: Props = $props();

	let currentSelectedId = $derived(data.selectedId || generalSettingsPages[0]!.id);

	function selectPage(pageId: GeneralSettingsPageId) {
		currentSelectedId = pageId;
	}
</script>

	<SettingsModalLayout
	title="全局设置"
	pages={generalSettingsPages}
	selectedId={currentSelectedId}
	onSelectPage={selectPage}
>
	{#snippet content({ currentPage })}
		{#if currentPage}
			{#if currentPage.id === "general"}
				<OfflineGeneralSettings />
			{:else if currentPage.id === "appearance"}
				<AppearanceSettings />
			{:else if currentPage.id === "lanes-and-branches"}
				<LanesAndBranchesSettings />
			{:else if currentPage.id === "git"}
				<GitSettings />
			{:else if currentPage.id === "integrations"}
				<OfflineCapabilityNotice title="在线集成已关闭" detail="此版本不登录、不访问 GitHub、GitLab、Bitbucket 或其他远程服务。" />
			{:else if currentPage.id === "ai"}
				<OfflineCapabilityNotice title="联网 AI 已关闭" detail="RepoScope Desktop 不会把代码、差异或凭据发送到外部 AI 服务。" />
			{:else if currentPage.id === "telemetry"}
				<OfflineCapabilityNotice title="遥测已关闭" detail="本版本不收集、上传分析数据或崩溃报告。" />
			{:else if currentPage.id === "experimental"}
				<ExperimentalSettings />
			{:else}
				未找到设置页面：{currentPage.id}
			{/if}
		{:else}
		未找到设置页面：{currentSelectedId}
		{/if}
	{/snippet}

</SettingsModalLayout>
