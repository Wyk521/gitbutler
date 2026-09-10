import { classify } from "$lib/error/errorClassification";
import { redactSensitiveText } from "$lib/error/redact";
import { showToast, type Toast } from "$lib/notifications/toasts";

type ExtraAction = NonNullable<Toast["extraAction"]>;

export function showError(title: string, error: unknown, extraAction?: ExtraAction, id?: string) {
	const classified = classify(error, title);
	if (classified.severity === "silent") {
		return;
	}

	showToast({
		id,
		title: redactSensitiveText(classified.title),
		message: classified.userMessage
			? redactSensitiveText(classified.userMessage)
			: undefined,
		error: redactSensitiveText(classified.message),
		style: classified.severity === "warning" ? "warning" : "danger",
		extraAction: extraAction ?? classified.actionHint,
	});
}
