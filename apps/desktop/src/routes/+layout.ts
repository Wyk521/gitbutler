import createBackend from "$lib/backend";
import { SettingsService } from "$lib/settings/appSettings";
import { EventContext } from "$lib/telemetry/eventContext";
import { PostHogWrapper } from "$lib/telemetry/posthog";
import lscache from "lscache";
import type { LayoutLoad } from "./$types";

// call on startup so we don't accumulate old items
lscache.flushExpired();

export const ssr = false;
export const prerender = false;
export const csr = true;

// eslint-disable-next-line
export const load: LayoutLoad = async () => {
	const backend = createBackend();

	const homeDir = await backend.homeDirectory();

	const eventContext = new EventContext();

	const settingsService = new SettingsService(backend);
	const appSettings = await settingsService.fetchAppSettings();

	const posthog = new PostHogWrapper(settingsService, backend, eventContext);
	return {
		homeDir,
		backend,
		settingsService,
		appSettings,
		posthog,
		eventContext,
	};
};
