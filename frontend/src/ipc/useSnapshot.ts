import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type { SnapshotEnvelope } from "./types";

export const probeAcknowledgedEvent = "harvester:probe-acknowledged";

export function useSnapshot() {
	const [snapshot, setSnapshot] = useState<SnapshotEnvelope | null>(null);
	const applied = useRef(0);
	useEffect(() => {
		const probe = new URLSearchParams(window.location.search).has("probe");
		const apply = (candidate: SnapshotEnvelope | null) => {
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
			unlistenSnapshot = await listen<SnapshotEnvelope>(
				"harvester://snapshot",
				(event) => apply(event.payload),
			);
			apply(await invoke<SnapshotEnvelope | null>("get_snapshot"));
		})();
		return () => {
			unlistenSnapshot?.();
		};
	}, []);
	return snapshot;
}
