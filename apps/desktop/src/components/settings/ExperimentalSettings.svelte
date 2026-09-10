<script lang="ts">
	import { fModeEnabled } from "$lib/config/uiFeatureFlags";
	import { SETTINGS_SERVICE } from "$lib/settings/appSettings";
import { inject } from "@gitbutler/core/context";
import { CardGroup, Toggle } from "@gitbutler/ui";

	const settingsService = inject(SETTINGS_SERVICE);
	const settingsStore = settingsService.appSettings;

</script>

<p class="text-12 text-body experimental-settings__text">
	以下功能仍在开发或测试阶段，可能尚未完整稳定。
	<br />
	请在确认影响后再启用。
</p>

<CardGroup>
	<CardGroup.Item labelFor="f-mode">
		{#snippet title()}
			F 键快速导航
		{/snippet}
		{#snippet caption()}
			使用两字母快捷键快速定位界面按钮。
		{/snippet}
		{#snippet actions()}
			<Toggle
				id="f-mode"
				checked={$fModeEnabled}
				onclick={() => fModeEnabled.set(!$fModeEnabled)}
			/>
		{/snippet}
	</CardGroup.Item>

	<CardGroup.Item labelFor="worktree-manipulation">
		{#snippet title()}
			链接 Worktree
		{/snippet}
		{#snippet caption()}
			启用链接 Git Worktree 的实验性支持。
		{/snippet}
		{#snippet actions()}
			<Toggle
				id="worktree-manipulation"
				checked={$settingsStore?.featureFlags.worktreeManipulation}
				onclick={() =>
					settingsService.updateFeatureFlags({
						worktreeManipulation: !$settingsStore?.featureFlags.worktreeManipulation,
					})}
			/>
		{/snippet}
	</CardGroup.Item>
</CardGroup>

<style>
	.experimental-settings__text {
		margin-bottom: 10px;
		color: var(--text-2);
	}
</style>
