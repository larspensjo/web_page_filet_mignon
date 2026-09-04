import { invoke } from "@tauri-apps/api/core";

export async function dispatchIntent(intent: {
	type: string;
	payload?: unknown;
}): Promise<void> {
	await invoke("dispatch_intent", { payload: intent });
}
