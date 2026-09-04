import withCorpus from "@fixtures/snapshots/idle_with_corpus.json";
import { invoke } from "@tauri-apps/api/core";
import {
	cleanup,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";

const listeners = new Map<string, (event: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn(
		async (name: string, callback: (event: { payload: unknown }) => void) => {
			listeners.set(name, callback);
			return () => listeners.delete(name);
		},
	),
}));
vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(async (name: string) =>
		name === "get_snapshot" ? withCorpus : undefined,
	),
}));

describe("job list", () => {
	afterEach(cleanup);
	beforeEach(() => {
		listeners.clear();
		vi.mocked(invoke).mockClear();
	});
	it("renders a real bridge fixture instead of a handwritten snapshot", async () => {
		render(<App />);
		expect(
			await screen.findByText("https://example.invalid/fixture"),
		).toBeInTheDocument();
	});

	it("blocks rendering when the frontend schema does not match", async () => {
		render(<App />);
		await waitFor(() =>
			expect(listeners.has("harvester://snapshot")).toBe(true),
		);
		listeners.get("harvester://snapshot")?.({
			payload: {
				...withCorpus,
				generation: withCorpus.generation + 1,
				schema_version: withCorpus.schema_version + 1,
			},
		});

		expect(
			await screen.findByText(/frontend bundle is out of date/),
		).toBeInTheDocument();
	});

	it("dispatches PollSources through the restricted intent channel", async () => {
		render(<App />);
		fireEvent.click(
			await screen.findByRole("button", { name: "Poll Sources" }),
		);

		await waitFor(() =>
			expect(invoke).toHaveBeenCalledWith("dispatch_intent", {
				payload: { type: "PollSources" },
			}),
		);
	});
});
