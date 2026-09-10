<script lang="ts">
	import { BACKEND } from "$lib/backend";
	import { PROJECTS_SERVICE } from "$lib/project/projectsService";
	import { SETTINGS_SERVICE } from "$lib/settings/appSettings";
	import { TERMINAL_SERVICE } from "$lib/settings/terminalService";
	import {
		UI_STATE,
		type CodeEditorSettings,
		type TerminalSettings,
	} from "$lib/state/uiState.svelte";
	import { inject } from "@gitbutler/core/context";
	import { Button, CardGroup, Select, SelectItem } from "@gitbutler/ui";
	import { onMount } from "svelte";

	const backend = inject(BACKEND);
	const settingsService = inject(SETTINGS_SERVICE);
	const projectsService = inject(PROJECTS_SERVICE);
	const terminalService = inject(TERMINAL_SERVICE);
	const uiState = inject(UI_STATE);

	const defaultCodeEditor = uiState.global.defaultCodeEditor;
	const defaultTerminal = uiState.global.defaultTerminal;
	const appSettings = settingsService.appSettings;

	const editorOptions: CodeEditorSettings[] = [
		{ schemeIdentifer: "vscodium", displayName: "VSCodium" },
		{ schemeIdentifer: "vscode", displayName: "VSCode" },
		{ schemeIdentifer: "vscode-insiders", displayName: "VSCode Insiders" },
		{ schemeIdentifer: "zed", displayName: "Zed" },
		{ schemeIdentifer: "cursor", displayName: "Cursor" },
	];
	const editorOptionsForSelect = editorOptions.map((option) => ({
		label: option.displayName,
		value: option.schemeIdentifer,
	}));

	let terminalOptions: TerminalSettings[] = $state([]);
	let terminalOptionsForSelect: Array<{ label: string; value: string }> = $state([]);

	onMount(async () => {
		try {
			terminalOptions = await terminalService.getTerminalOptionsForPlatform(backend.platformName);
			terminalOptionsForSelect = terminalOptions.map((option) => ({
				label: option.displayName,
				value: option.identifier,
			}));
		} catch {
			terminalOptions = [];
			terminalOptionsForSelect = [];
		}
	});

	async function clearLocalProjectIndex() {
		await settingsService.deleteAllData();
		projectsService.unsetLastOpenedProject();
	}
</script>

<CardGroup>
	<CardGroup.Item>
		{#snippet title()}运行模式{/snippet}
		{#snippet caption()}RepoScope Desktop 只读取本机仓库；不会登录、联网、同步或上传数据。{/snippet}
		{#snippet actions()}<span class="offline-pill">严格离线</span>{/snippet}
	</CardGroup.Item>
	<CardGroup.Item>
		{#snippet title()}默认代码编辑器{/snippet}
		{#snippet actions()}
			<Select
				value={defaultCodeEditor.current.schemeIdentifer}
				options={editorOptionsForSelect}
				onselect={(value) => {
					const selected = editorOptions.find((option) => option.schemeIdentifer === value);
					if (selected) defaultCodeEditor.set(selected);
				}}
			>
				{#snippet itemSnippet({ item, highlighted })}
					<SelectItem selected={item.value === defaultCodeEditor.current.schemeIdentifer} {highlighted}>
						{item.label}
					</SelectItem>
				{/snippet}
			</Select>
		{/snippet}
	</CardGroup.Item>
	{#if terminalOptionsForSelect.length > 0}
		<CardGroup.Item>
			{#snippet title()}默认终端{/snippet}
			{#snippet actions()}
				<Select
					value={defaultTerminal.current.identifier}
					options={terminalOptionsForSelect}
					onselect={(value) => {
						const selected = terminalOptions.find((option) => option.identifier === value);
						if (selected) defaultTerminal.set(selected);
					}}
				>
					{#snippet itemSnippet({ item, highlighted })}
						<SelectItem selected={item.value === defaultTerminal.current.identifier} {highlighted}>
							{item.label}
						</SelectItem>
					{/snippet}
				</Select>
			{/snippet}
		</CardGroup.Item>
	{/if}
</CardGroup>

<CardGroup>
	<CardGroup.Item>
		{#snippet title()}关于 RepoScope Desktop{/snippet}
		{#snippet caption()}
			基于 GitButler 构建的内部离线版本。版权所有 GitButler Inc。分析数据库独立存放在项目数据目录，不迁移旧版 SQLite。
			<a class="license-link" href="/licenses/FSL-1.1-MIT.txt" target="_blank" rel="noreferrer"
				>查看完整 FSL-1.1-MIT 许可证</a
			>
		{/snippet}
		{#snippet actions()}
			<span class="version">{backend.platformName} · {appSettings ? "本地数据" : "等待设置"}</span>
		{/snippet}
	</CardGroup.Item>
	<CardGroup.Item>
		{#snippet title()}清理本机项目索引{/snippet}
		{#snippet caption()}仅删除 RepoScope/GitButler 的本机项目配置，不删除仓库文件。{/snippet}
		{#snippet actions()}
			<Button kind="outline" onclick={clearLocalProjectIndex}>清理索引</Button>
		{/snippet}
	</CardGroup.Item>
</CardGroup>

<style>
	.offline-pill {
		padding: 4px 9px;
		border-radius: 999px;
		background: var(--bg-2);
		color: var(--text-1);
		font-size: 12px;
		font-weight: 600;
	}

	.version {
		color: var(--text-2);
		font-size: 12px;
	}

	.license-link {
		display: inline-block;
		margin-top: 6px;
		color: var(--text-1);
	}
</style>
