<script lang="ts">
	import "@gitbutler/design-core/utility";
	import "@gitbutler/design-core/core";
	import "../styles/styles.css";
	import { dev } from "$app/environment";
	import { page } from "$app/state";
	import GlobalSettingsShortcutHandler from "$components/settings/GlobalSettingsShortcutHandler.svelte";
	import ReloadShortcutHandler from "$components/settings/ReloadShortcutHandler.svelte";
	import ThemeShortcutHandler from "$components/settings/ThemeShortcutHandler.svelte";
	import ToggleSidebarShortcutHandler from "$components/settings/ToggleSidebarShortcutHandler.svelte";
	import ZoomShortcutHandler from "$components/settings/ZoomShortcutHandler.svelte";
	import FocusCursor from "$components/shared/FocusCursor.svelte";
	import GitInputPrompt from "$components/shared/GitInputPrompt.svelte";
	import ReloadWarning from "$components/shared/ReloadWarning.svelte";
	import ToastController from "$components/shared/ToastController.svelte";
	import GlobalModalRouter from "$components/views/GlobalModalRouter.svelte";
	import { initDependencies } from "$lib/bootstrap/deps";
	import { fModeEnabled } from "$lib/config/uiFeatureFlags";
	import { PROJECTS_SERVICE } from "$lib/project/projectsService";
	import { TERMINAL_SERVICE } from "$lib/settings/terminalService";
	import { createKeybind } from "$lib/shortcuts/hotkeys";
	import { SHORTCUT_SERVICE } from "$lib/shortcuts/shortcutService";
	import { CLIENT_STATE } from "$lib/state/clientState.svelte";
	import { initUserSettings, UI_STATE } from "$lib/state/uiState.svelte";
	import { inject } from "@gitbutler/core/context";
	import { ChipToastContainer } from "@gitbutler/ui";
	import { FOCUS_MANAGER } from "@gitbutler/ui/focus/focusManager";
	import { untrack, type Snippet } from "svelte";
	import type { LayoutData } from "./$types";

	const { data, children }: { data: LayoutData; children: Snippet } = $props();
	const projectId = $derived(page.params.projectId);

	// =============================================================================
	// BOOTSTRAP & INIT
	// =============================================================================

	const { backend } = untrack(() => data);
	initDependencies(untrack(() => data));

	const clientState = inject(CLIENT_STATE);
	const uiState = inject(UI_STATE);
	const terminalService = inject(TERMINAL_SERVICE);

	clientState.initPersist().then(async () => {
		await initUserSettings(uiState, backend.platformName, terminalService);
	});

	// =============================================================================
	// CORE REACTIVE STATE & EFFECTS
	// =============================================================================

	// Project tracking
	const projectsService = inject(PROJECTS_SERVICE);
	$effect(() => {
		if (projectId) {
			projectsService.setLastOpenedProject(projectId);
		}
	});

	// Keyboard shortcuts
	const shortcutService = inject(SHORTCUT_SERVICE);
	$effect(() => shortcutService.listen());

	// =============================================================================
	// DEBUG & DEVELOPMENT TOOLS
	// =============================================================================

	function handleKeyDown(e: KeyboardEvent) {
		// Explicitly detect cmd/ctrl + A since Tauri gets in the way of default behavior.
		// To get default behavior you can add a "Select All" predefined menu item to the
		// Edit menu, but that prevents the event from reaching the webview.
		if (
			(e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) &&
			(e.metaKey || e.ctrlKey) &&
			e.key === "a" &&
			e.target
		) {
			e.target.select();
			e.preventDefault();
		} else {
			handleKeyBind(e);
		}
	}

	function blockOfflineNavigation(event: MouseEvent) {
		if (import.meta.env.VITEST || import.meta.env.VITE_REPOSCOPE_OFFLINE === "false") return;
		const element = event.target;
		if (!(element instanceof Element)) return;
		const anchor = element.closest("a");
		const href = anchor?.getAttribute("href");
		if (!href) return;
		try {
			const url = new URL(href, window.location.href);
			if (["http:", "https:", "ws:", "wss:", "mailto:"].includes(url.protocol)) {
				event.preventDefault();
				event.stopPropagation();
			}
		} catch {
			// Invalid links are left to the browser's normal error handling.
		}
	}

	// Debug keyboard shortcuts
	const handleKeyBind = createKeybind({
		// Show commit graph visualization
		"d o t": async () => {
			const projectId = page.params.projectId;
			await backend.invoke("show_graph_svg", { projectId });
		},
		// Log environment variables
		"e n v": async () => {
			let env = await backend.invoke("env_vars");
			// eslint-disable-next-line no-console
			console.log(env);
			(window as any).tauriEnv = env;
			// eslint-disable-next-line no-console
			console.log("Also written to window.tauriEnv");
		},
	});

	const focusManager = inject(FOCUS_MANAGER);
	$effect(() => focusManager.listen());

	// Pass F mode feature flag to focus manager
	$effect(() => {
		focusManager.setFModeEnabled($fModeEnabled);
	});
</script>

<svelte:window
	ondrop={(e) => e.preventDefault()}
	ondragover={(e) => e.preventDefault()}
	onclick={blockOfflineNavigation}
	onkeydown={handleKeyDown}
/>

<svelte:head>
	<title>RepoScope Desktop</title>
</svelte:head>

<div class="app-root" role="application" oncontextmenu={(e) => !dev && e.preventDefault()}>
	{@render children()}
</div>
<ToastController />
<ChipToastContainer />
<GitInputPrompt />
<ZoomShortcutHandler />
<GlobalSettingsShortcutHandler />
<ReloadShortcutHandler />
<ThemeShortcutHandler />
<ToggleSidebarShortcutHandler />
<GlobalModalRouter />
<FocusCursor />

{#if import.meta.env.MODE === "development"}
	<ReloadWarning />
{/if}

<style lang="postcss">
	.app-root {
		display: flex;
		height: 100%;
		cursor: default;
	}
</style>
