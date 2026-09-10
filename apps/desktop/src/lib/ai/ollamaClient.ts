import {
	LONG_DEFAULT_BRANCH_TEMPLATE,
	LONG_DEFAULT_COMMIT_TEMPLATE,
	SHORT_DEFAULT_PR_TEMPLATE,
} from "$lib/ai/prompts";
import { type AIClient, type Prompt } from "$lib/ai/types";

export const DEFAULT_OLLAMA_ENDPOINT = "http://127.0.0.1:11434";
export const DEFAULT_OLLAMA_MODEL_NAME = "llama3";

export class OllamaClient implements AIClient {
	defaultCommitTemplate = LONG_DEFAULT_COMMIT_TEMPLATE;
	defaultBranchTemplate = LONG_DEFAULT_BRANCH_TEMPLATE;
	defaultPRTemplate = SHORT_DEFAULT_PR_TEMPLATE;

	constructor(
		_endpoint: string,
		_modelName: string,
	) {}

	async evaluate(_prompt: Prompt): Promise<string> {
		throw new Error("RepoScope Desktop 离线模式已禁用 AI 功能");
	}
}
