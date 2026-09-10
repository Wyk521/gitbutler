import { initSentry } from "$lib/analytics/sentry";
import { PostHogWrapper } from "$lib/telemetry/posthog";
import type { AppSettings } from "@gitbutler/but-sdk";

export async function initAnalyticsIfEnabled(
	appSettings: AppSettings,
	postHog: PostHogWrapper,
	confirmedOverride?: boolean,
) {
	// Release builds of RepoScope Desktop are offline by construction.  This
	// guard protects older settings/onboarding components that may still call
	// the shared helper.
	if (import.meta.env.VITE_REPOSCOPE_OFFLINE !== "false") return;
	if (import.meta.env.MODE === "development" || import.meta.env.CI) return;

	const confirmed = confirmedOverride ?? appSettings.onboardingComplete;

	if (confirmed) {
		if (appSettings.telemetry.appErrorReportingEnabled) {
			initSentry();
		}
		if (appSettings.telemetry.appMetricsEnabled) {
			await postHog.init();
		}
	}
}
