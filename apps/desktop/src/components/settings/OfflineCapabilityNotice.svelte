<script lang="ts">
	let { title, detail }: { title: string; detail: string } = $props();
</script>

<div class="offline-capability" role="status">
	<div class="offline-capability__badge">离线</div>
	<h2 class="text-18 text-semibold">{title}</h2>
	<p class="text-13">{detail}</p>
</div>

<style>
	.offline-capability {
		display: grid;
		gap: 8px;
		padding: 20px;
		border: 1px solid var(--border-2);
		border-radius: var(--radius-m);
		background: var(--bg-1);
		color: var(--text-2);
	}

	.offline-capability h2,
	.offline-capability p {
		margin: 0;
	}

	.offline-capability h2 {
		color: var(--text-1);
	}

	.offline-capability__badge {
		width: fit-content;
		padding: 2px 8px;
		border-radius: 999px;
		background: var(--bg-2);
		color: var(--text-1);
		font-size: 11px;
		font-weight: 600;
	}
</style>
