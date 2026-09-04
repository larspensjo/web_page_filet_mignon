import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { probeAcknowledgedEvent } from "./useSnapshot";

type ProbeFinished = { finalGeneration: number };

export function useProbe() {
	useEffect(() => {
		const query = new URLSearchParams(window.location.search);
		if (!query.has("probe")) return;
		const durationMs = Number(query.get("durationMs"));
		if (!Number.isFinite(durationMs) || durationMs <= 0) return;
		let frames = 0;
		let slowFrames = 0;
		let previous = 0;
		let cspFetchRejected = false;
		let frameId = 0;
		let timeout = 0;
		let started = false;
		let durationElapsed = false;
		let highestAcknowledged = 0;
		let finalGeneration: number | undefined;
		let reported = false;
		let unlistenFinished: (() => void) | undefined;
		const onViolation = (event: SecurityPolicyViolationEvent) => {
			if (event.violatedDirective.startsWith("connect-src"))
				cspFetchRejected = true;
		};
		const frame = (now: number) => {
			frames += 1;
			if (now - previous > 50) slowFrames += 1;
			previous = now;
			frameId = requestAnimationFrame(frame);
		};
		const reportIfComplete = () => {
			if (
				reported ||
				!durationElapsed ||
				finalGeneration === undefined ||
				highestAcknowledged < finalGeneration
			)
				return;
			reported = true;
			void invoke("probe_report", {
				report: { frames, slowFrames, cspFetchRejected },
			});
		};
		const startMeasurement = () => {
			if (started) return;
			started = true;
			previous = performance.now();
			window.addEventListener("securitypolicyviolation", onViolation);
			void fetch("https://probe.invalid/csp-case").catch(() => undefined);
			frameId = requestAnimationFrame(frame);
			timeout = window.setTimeout(() => {
				durationElapsed = true;
				cancelAnimationFrame(frameId);
				reportIfComplete();
			}, durationMs);
		};
		const onAcknowledged = (event: Event) => {
			highestAcknowledged = Math.max(
				highestAcknowledged,
				(event as CustomEvent<number>).detail,
			);
			startMeasurement();
			reportIfComplete();
		};
		window.addEventListener(probeAcknowledgedEvent, onAcknowledged);
		void (async () => {
			unlistenFinished = await listen<ProbeFinished>(
				"harvester://probe-finished",
				(event) => {
					finalGeneration = event.payload.finalGeneration;
					reportIfComplete();
				},
			);
		})();
		return () => {
			window.clearTimeout(timeout);
			cancelAnimationFrame(frameId);
			window.removeEventListener(probeAcknowledgedEvent, onAcknowledged);
			window.removeEventListener("securitypolicyviolation", onViolation);
			unlistenFinished?.();
		};
	}, []);
}
