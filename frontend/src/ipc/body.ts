import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { BodyRef, BodyResponse } from "./types";

export type BodyState =
	| { kind: "unavailable" }
	| { kind: "loading" }
	| { kind: "ready"; text: string }
	| { kind: "changed" }
	| { kind: "error" };

export function useBody(reference: BodyRef | null | undefined): BodyState {
	const [state, setState] = useState<BodyState>({ kind: "unavailable" });
	const bodyKey = reference?.key;
	const contentHash = reference?.content_hash;

	useEffect(() => {
		let current = true;
		if (!bodyKey || !contentHash) {
			setState({ kind: "unavailable" });
			return () => {
				current = false;
			};
		}

		setState({ kind: "loading" });
		void invoke<BodyResponse | null>("fetch_body", { key: bodyKey })
			.then((response) => {
				if (!current) return;
				if (!response) {
					setState({ kind: "error" });
					return;
				}
				if (response.content_hash !== contentHash) {
					setState({ kind: "changed" });
					return;
				}
				setState({ kind: "ready", text: response.text });
			})
			.catch(() => {
				if (current) setState({ kind: "error" });
			});

		return () => {
			current = false;
		};
	}, [bodyKey, contentHash]);

	return state;
}
