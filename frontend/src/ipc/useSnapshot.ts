import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { isProbePage } from "./probeMode";
import { IPC_SCHEMA_VERSION } from "./schemaVersion";
import type { SnapshotEnvelope } from "./types";

export const probeAcknowledgedEvent = "harvester:probe-acknowledged";

function decodeSnapshot(payload: unknown): SnapshotEnvelope | null {
	if (typeof payload !== "object" || payload === null) return null;
	const snapshot = payload as SnapshotEnvelope;
	if (
		typeof snapshot.generation !== "number" ||
		typeof snapshot.schema_version !== "number" ||
		(snapshot.fatal_message !== null &&
			typeof snapshot.fatal_message !== "string")
	)
		return null;
	// The app displays these envelopes without rendering their view.
	if (snapshot.fatal_message || snapshot.schema_version !== IPC_SCHEMA_VERSION)
		return snapshot;
	// Tolerate a bad key line: hide it rather than drop the whole snapshot.
	const keyStatusLine = snapshot.view?.key_status_line;
	if (
		!snapshot.view ||
		keyStatusLine === null ||
		typeof keyStatusLine === "string"
	)
		return snapshot;
	return { ...snapshot, view: { ...snapshot.view, key_status_line: null } };
}

export function useSnapshot() {
	const [snapshot, setSnapshot] = useState<SnapshotEnvelope | null>(null);
	const applied = useRef(0);
	useEffect(() => {
		const probe = isProbePage();
		const apply = (payload: unknown) => {
			const candidate = decodeSnapshot(payload);
			if (candidate && candidate.generation > applied.current) {
				applied.current = candidate.generation;
				setSnapshot(candidate);
				if (probe) {
					void invoke("probe_ack", { generation: candidate.generation }).then(
						() =>
							window.dispatchEvent(
								new CustomEvent<number>(probeAcknowledgedEvent, {
									detail: candidate.generation,
								}),
							),
					);
				}
			}
		};
		let unlistenSnapshot: (() => void) | undefined;
		void (async () => {
			unlistenSnapshot = await listen<unknown>(
				"harvester://snapshot",
				(event) => apply(event.payload),
			);
			apply(await invoke<unknown>("get_snapshot"));
		})();
		return () => {
			unlistenSnapshot?.();
		};
	}, []);
	return snapshot;
}
