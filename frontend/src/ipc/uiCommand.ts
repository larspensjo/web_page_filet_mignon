import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import type { UiCommand } from "./types";

export const UI_COMMAND_EVENT = "harvester://ui-command";

/**
 * Channel 2: effects the host cannot service itself. Only
 * `ShowArchiveDialog` exists; anything else is dropped without applying.
 */
export function useUiCommand(handler: (command: UiCommand) => void): void {
	const latest = useRef(handler);
	latest.current = handler;
	useEffect(() => {
		let unlisten: (() => void) | undefined;
		let disposed = false;
		void listen<unknown>(UI_COMMAND_EVENT, (event) => {
			const command = decodeUiCommand(event.payload);
			if (command) latest.current(command);
		}).then((stop) => {
			if (disposed) stop();
			else unlisten = stop;
		});
		return () => {
			disposed = true;
			unlisten?.();
		};
	}, []);
}

export function decodeUiCommand(payload: unknown): UiCommand | null {
	if (typeof payload !== "object" || payload === null) return null;
	const record = payload as Record<string, unknown>;
	const request = record.ShowArchiveDialog;
	if (
		typeof request !== "object" ||
		request === null ||
		typeof (request as { request_id?: unknown }).request_id !== "number"
	)
		return null;
	return { ShowArchiveDialog: request as UiCommand["ShowArchiveDialog"] };
}
