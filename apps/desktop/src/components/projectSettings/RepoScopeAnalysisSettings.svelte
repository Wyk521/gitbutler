<script lang="ts">
	import { RepoScopeService } from "$lib/reposcope/service.svelte";
	import type { AnalysisConfig } from "$lib/reposcope/types";
	import { Button, CardGroup } from "@gitbutler/ui";
	import { onMount } from "svelte";

	const { projectId }: { projectId: string } = $props();
	const service = new RepoScopeService();
	const defaults: AnalysisConfig = {
		maxFileBytes: 20_000_000,
		ownershipMaxFiles: 2_000,
		workers: 4,
		couplingMaxFiles: 80,
		couplingTopPairs: 500,
	};
	let config = $state<AnalysisConfig>({ ...defaults });
	let maxFileMb = $state(20);
	let loading = $state(true);
	let saving = $state(false);
	let message = $state("");

	function integerInRange(value: number, minimum: number, maximum: number, fallback: number) {
		const parsed = Number.isFinite(value) ? Math.round(value) : fallback;
		return Math.min(maximum, Math.max(minimum, parsed));
	}

	onMount(async () => {
		try {
			config = await service.config(projectId);
			maxFileMb = Math.max(1, Math.round(config.maxFileBytes / 1_000_000));
		} catch {
			config = { ...defaults };
		} finally {
			loading = false;
		}
	});

	async function save() {
		saving = true;
		message = "";
		try {
			maxFileMb = integerInRange(maxFileMb, 1, 20, 20);
			config.ownershipMaxFiles = integerInRange(config.ownershipMaxFiles, 1, 100_000, defaults.ownershipMaxFiles);
			config.workers = integerInRange(config.workers, 1, 8, defaults.workers);
			config.couplingMaxFiles = integerInRange(config.couplingMaxFiles, 2, 200, defaults.couplingMaxFiles);
			config.couplingTopPairs = integerInRange(config.couplingTopPairs, 1, 5_000, defaults.couplingTopPairs);
			config = await service.saveConfig(projectId, {
				...config,
				maxFileBytes: maxFileMb * 1_000_000,
			});
			message = "已保存；下次分析时生效";
		} catch (error) {
			message = error instanceof Error ? error.message : "保存分析设置失败";
		} finally {
			saving = false;
		}
	}
</script>

<CardGroup>
	<CardGroup.Item>
		{#snippet title()}RepoScope 分析设置{/snippet}
		{#snippet caption()}设置只写入项目数据目录，不修改 Git 仓库；大文件和所有权分析均有硬上限。{/snippet}
		{#snippet actions()}<span class="offline-pill">本地保存</span>{/snippet}
	</CardGroup.Item>
	{#if loading}
		<CardGroup.Item>
			{#snippet title()}正在读取设置…{/snippet}
		</CardGroup.Item>
	{:else}
		<CardGroup.Item>
			{#snippet title()}单文件上限（MB）{/snippet}
			{#snippet actions()}<input type="number" min="1" max="20" step="1" bind:value={maxFileMb} />{/snippet}
		</CardGroup.Item>
		<CardGroup.Item>
			{#snippet title()}所有权最多分析文件数{/snippet}
			{#snippet actions()}<input type="number" min="1" max="100000" step="1" bind:value={config.ownershipMaxFiles} />{/snippet}
		</CardGroup.Item>
		<CardGroup.Item>
			{#snippet title()}分析并发数{/snippet}
			{#snippet actions()}<input type="number" min="1" max="8" step="1" bind:value={config.workers} />{/snippet}
		</CardGroup.Item>
		<CardGroup.Item>
			{#snippet title()}单提交耦合文件上限{/snippet}
			{#snippet actions()}<input type="number" min="2" max="200" step="1" bind:value={config.couplingMaxFiles} />{/snippet}
		</CardGroup.Item>
		<CardGroup.Item>
			{#snippet title()}最多保留耦合对{/snippet}
			{#snippet actions()}<input type="number" min="1" max="5000" step="1" bind:value={config.couplingTopPairs} />{/snippet}
		</CardGroup.Item>
		<CardGroup.Item>
			{#snippet actions()}<Button kind="outline" onclick={save} disabled={saving}>{saving ? "保存中…" : "保存分析设置"}</Button>{#if message}<span class="message">{message}</span>{/if}{/snippet}
		</CardGroup.Item>
	{/if}
</CardGroup>

<style>
	input {
		width: 112px;
		padding: 6px 8px;
		border: 1px solid var(--border-2);
		border-radius: var(--radius-s);
		background: var(--bg-2);
		color: var(--text-1);
	}

	.offline-pill {
		padding: 4px 9px;
		border-radius: 999px;
		background: var(--bg-2);
		color: var(--text-1);
		font-size: 11px;
	}

	.message {
		margin-left: 10px;
		color: var(--text-2);
		font-size: 12px;
	}
</style>
