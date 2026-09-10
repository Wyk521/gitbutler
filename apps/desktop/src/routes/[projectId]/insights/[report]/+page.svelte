<script lang="ts">
	import { page } from "$app/state";
	import RepoScopeReport from "$components/reposcope/RepoScopeReport.svelte";

	const projectId = $derived(page.params.projectId!);
	const report = $derived(page.params.report ?? "overview");
</script>

<RepoScopeReport {projectId} {report} />
