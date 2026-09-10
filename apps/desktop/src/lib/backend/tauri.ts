import { IpcError, isNormalizedError } from "$lib/error/normalizedError";
import { getName, getVersion, getVersion as tauriGetVersion } from "@tauri-apps/api/app";
import { invoke as invokeTauri } from "@tauri-apps/api/core";
import { documentDir as documentDirTauri } from "@tauri-apps/api/path";
import { join as joinPathTauri } from "@tauri-apps/api/path";
import { getCurrentWindow, type Window } from "@tauri-apps/api/window";
import {
	writeText as tauriWriteText,
	readText as tauriReadText,
} from "@tauri-apps/plugin-clipboard-manager";
import { open as filePickerTauri, type OpenDialogOptions } from "@tauri-apps/plugin-dialog";
import { readFile as tauriReadFile } from "@tauri-apps/plugin-fs";
import { error as logErrorToFile } from "@tauri-apps/plugin-log";
import { platform } from "@tauri-apps/plugin-os";
import { relaunch as relaunchTauri } from "@tauri-apps/plugin-process";
import { Store } from "@tauri-apps/plugin-store";
import { readable } from "svelte/store";
import type { AppInfo, DiskStore, IBackend, UnlistenFn } from "$lib/backend/backend";
import type { EventCallback, EventName } from "@tauri-apps/api/event";
import { isOfflineCommand } from "$lib/backend/offline";
import { redactSensitiveText } from "$lib/error/redact";

export default class Tauri implements IBackend {
	platformName = platform();
	private appWindow: Window | undefined;

	systemTheme = readable<string | null>(null, (set) => {
		if (!this.appWindow) {
			this.appWindow = getCurrentWindow();
		}
		this.appWindow.theme().then((value) => {
			set(value);
		});

		this.appWindow.onThemeChanged((e) => {
			set(e.payload);
		});
	});

	async getColdStartDeepLinkUrls(): Promise<string[]> {
		return [];
	}

	async initDeepLinking(_handlers: unknown, _coldStartUrls: string[]): Promise<UnlistenFn> {
		// Deep-link login/open flows are intentionally absent from the offline
		// product. Keep the interface for shared bootstrap code.
		return () => undefined;
	}
	invoke = tauriInvoke;
	listen = tauriListen;
	checkUpdate = async () => null;
	currentVersion = tauriGetVersion;
	readFile = tauriReadFile;
	openExternalUrl = tauriOpenExternalUrl;
	relaunch = relaunchTauri;
	documentDir = documentDirTauri;
	joinPath = joinPathTauri;
	getAppInfo = tauriGetAppInfo;
	readTextFromClipboard = tauriReadText;
	writeTextToClipboard = tauriWriteText;

	async filePicker<T extends OpenDialogOptions>(options?: T) {
		return await filePickerTauri<T>(options);
	}

	async homeDirectory(): Promise<string> {
		// TODO: Find a workaround to avoid this dynamic import
		// https://github.com/sveltejs/kit/issues/905
		return await (await import("@tauri-apps/api/path")).homeDir();
	}

	async loadDiskStore(fileName: string): Promise<DiskStore> {
		const store = await Store.load(fileName, { autoSave: true, defaults: {} });
		return new TauriDiskStore(store);
	}

	async getWindowTitle(): Promise<string> {
		if (!this.appWindow) {
			this.appWindow = getCurrentWindow();
		}
		return await this.appWindow.title();
	}

	setWindowTitle(title: string): void {
		if (!this.appWindow) {
			this.appWindow = getCurrentWindow();
		}
		this.appWindow.setTitle(title);
	}
}

// Kept as pure parsing helpers for the existing unit fixtures.  RepoScope
// Desktop does not register a deep-link plugin or dispatch these URLs at
// runtime, so this compatibility code cannot authenticate or open a path.
const DEEP_LINK_SCHEMES = ["but", "but-dev", "but-nightly"] as const;
const DEEP_LINK_TOP_LEVEL_PATHS = ["open", "login"] as const;
type DeepLinkTopLevelPath = (typeof DEEP_LINK_TOP_LEVEL_PATHS)[number];

function isValidDeepLinkTopLevelPath(path: string): path is DeepLinkTopLevelPath {
	return DEEP_LINK_TOP_LEVEL_PATHS.includes(path as DeepLinkTopLevelPath);
}

export function isValidDeepLinkUrl(url: string): boolean {
	return parseDeepLinkUrl(url) !== null;
}

export function parseDeepLinkUrl(url: string): [DeepLinkTopLevelPath, URLSearchParams] | null {
	let parsedUrl: URL;
	try {
		parsedUrl = new URL(url);
	} catch {
		return null;
	}

	const scheme = parsedUrl.protocol.slice(0, -1);
	if (!DEEP_LINK_SCHEMES.some((validScheme) => validScheme === scheme)) return null;
	if (!isValidDeepLinkTopLevelPath(parsedUrl.hostname)) return null;
	if (parsedUrl.pathname !== "" && parsedUrl.pathname !== "/") return null;
	if (parsedUrl.username || parsedUrl.password || parsedUrl.port || parsedUrl.hash) return null;

	return [parsedUrl.hostname, parsedUrl.searchParams];
}

class TauriDiskStore implements DiskStore {
	constructor(private store: Store) {}

	async set(key: string, value: unknown): Promise<void> {
		return await this.store.set(key, value);
	}

	async get<T>(key: string, defaultValue: undefined): Promise<T | undefined>;
	async get<T>(key: string, defaultValue: T): Promise<T>;
	async get<T>(key: string, defaultValue?: T): Promise<T | undefined> {
		const value = await this.store.get<T>(key);
		return value !== undefined ? value : defaultValue;
	}
}

export async function tauriLogErrorToFile(error: string) {
	try {
		await logErrorToFile(error);
	} catch (e: unknown) {
		console.warn("unable to log error to file", e);
	}
}

export function tauriPathSeparator(): string {
	const platformName = platform();
	return platformName === "windows" ? "\\" : "/";
}

async function tauriGetAppInfo(): Promise<AppInfo> {
	const [appName, appVersion] = await Promise.all([getName(), getVersion()]);
	return { name: appName, version: appVersion };
}

async function tauriInvoke<T>(command: string, params: Record<string, unknown> = {}): Promise<T> {
	// This commented out code can be used to delay/reject an api call
	// return new Promise<T>((resolve, reject) => {
	// 	if (command.startsWith('apply')) {
	// 		setTimeout(() => {
	// 			reject('testing the error page');
	// 		}, 500);
	// 	} else {
	// 		resolve(invokeTauri<T>(command, params));
	// 	}
	// }).catch((reason) => {
	// 	const userError = UserError.fromError(reason);
	// 	console.error(`ipc->${command}: ${JSON.stringify(params)}`, userError);
	// 	throw userError;
	// });

	try {
		if (isOfflineCommand(command)) {
			throw new Error(`RepoScope Desktop 离线模式已禁用命令：${command}`);
		}
		return await invokeTauri<T>(command, params);
	} catch (error: unknown) {
		if (isNormalizedError(error)) {
			console.error(
				`ipc->${command}: ${redactSensitiveText(JSON.stringify(params))}`,
				redactSensitiveText(error.message),
			);
			// Re-throw as a proper Error subclass so the stack points at the
			// caller and Sentry can fingerprint by name + message instead of
			// bucketing every raw `{name, message, code}` rejection together.
			throw new IpcError(error, command);
		}
		throw error;
	}
}

function tauriListen<T>(event: EventName, handle: EventCallback<T>) {
	const appWindow = getCurrentWindow();
	const unlisten = appWindow.listen(event, handle);
	return async () => await unlisten.then((unlistenFn) => unlistenFn());
}

async function tauriOpenExternalUrl(href: string): Promise<void> {
	let parsed: URL;
	try {
		parsed = new URL(href);
	} catch {
		throw new Error("RepoScope Desktop 只允许打开本机文件或编辑器 URI");
	}
	const localEditorSchemes = new Set([
		"file:",
		"vscode:",
		"vscode-insiders:",
		"vscodium:",
		"zed:",
		"windsurf:",
		"cursor:",
		"trae:",
		"antigravity-ide:",
	]);
	const hostAllowed =
		parsed.hostname === "" ||
		parsed.hostname.toLowerCase() === "file" ||
		parsed.hostname.toLowerCase() === "localhost";
	const sensitiveQuery = [...parsed.searchParams.keys()].some((key) =>
		[
			"token",
			"secret",
			"password",
			"passwd",
			"pwd",
			"credential",
			"authorization",
			"api_key",
			"apikey",
			"access_key",
			"access_token",
			"private_key",
			"username",
			"user",
		].some((needle) => key.toLowerCase() === needle || key.toLowerCase().includes(needle)),
	);
	if (
		!localEditorSchemes.has(parsed.protocol) ||
		!hostAllowed ||
		parsed.username ||
		parsed.password ||
		parsed.hash ||
		sensitiveQuery
	) {
		throw new Error("RepoScope Desktop 离线模式不打开外部网址");
	}
	await invokeTauri("open_local_target", { url: href });
}
