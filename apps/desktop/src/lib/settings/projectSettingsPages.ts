import { type IconName } from "@gitbutler/ui";

interface SettingsPage {
	id: string;
	label: string;
	icon: IconName;
	adminOnly?: boolean;
}

export const projectSettingsPages = [
	{
		id: "project",
		label: "项目",
		icon: "user",
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

export type ProjectSettingsPage = (typeof projectSettingsPages)[number];
