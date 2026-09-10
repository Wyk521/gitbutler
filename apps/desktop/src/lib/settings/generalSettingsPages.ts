import { type IconName } from "@gitbutler/ui";

interface SettingsPage {
	id: string;
	label: string;
	icon: IconName;
	adminOnly?: boolean;
}

export const generalSettingsPages = [
	{
		id: "general",
		label: "常规",
		icon: "settings",
	},
	{
		id: "appearance",
		label: "外观",
		icon: "appearance",
	},
	{
		id: "lanes-and-branches",
		label: "工作区与分支",
		icon: "lanes",
	},
	{
		id: "git",
		label: "Git 设置",
		icon: "git",
	},
	{
		id: "experimental",
		label: "实验功能",
		icon: "lab",
	},
] as const satisfies readonly SettingsPage[];

export type GeneralSettingsPage = (typeof generalSettingsPages)[number];
