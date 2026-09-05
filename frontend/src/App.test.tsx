import emptyCorpus from "@fixtures/snapshots/idle_empty_corpus.json";
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
import type { JobListMode } from "./ipc/types";

const listeners = new Map<string, (event: { payload: unknown }) => void>();
const jobListModes = [
	"Results",
	"SinceCheckpoint",
] as const satisfies readonly JobListMode[];
const fixtureUrl = "https://example.invalid/fixture";
const outsideCheckpointUrl = "https://example.invalid/outside-checkpoint";
let snapshot: unknown = withCorpus;
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
		name === "get_snapshot" ? snapshot : undefined,
	),
}));

describe("job list", () => {
	afterEach(cleanup);
	beforeEach(() => {
		listeners.clear();
		snapshot = withCorpus;
		vi.mocked(invoke).mockClear();
	});
	it("renders a real bridge fixture instead of a handwritten snapshot", async () => {
		render(<App />);
		expect(await screen.findByText(fixtureUrl)).toBeInTheDocument();
	});

	it("pins job-list scope fields in both bridge fixtures", () => {
		for (const fixture of [emptyCorpus, withCorpus]) {
			expect(jobListModes).toContain(fixture.view.job_list_mode);
			expect(
				fixture.view.jobs.every(
					(job) => typeof job.is_since_checkpoint === "boolean",
				),
			).toBe(true);
		}
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

	it("renders the snapshot rows in its SinceCheckpoint job list mode", async () => {
		snapshot = {
			...withCorpus,
			view: {
				...withCorpus.view,
				jobs: [
					...withCorpus.view.jobs,
					{
						...withCorpus.view.jobs[0],
						is_since_checkpoint: false,
						job_id: 2,
						url: outsideCheckpointUrl,
					},
				],
			},
		};
		render(<App />);

		expect(await screen.findByText(fixtureUrl)).toBeInTheDocument();
		expect(screen.queryByText(outsideCheckpointUrl)).not.toBeInTheDocument();
	});

	it.each([undefined, "Results"] as const)(
		"renders every row when job list mode is %s",
		async (jobListMode) => {
			snapshot = {
				...withCorpus,
				view: {
					...withCorpus.view,
					job_list_mode: jobListMode,
					jobs: [
						...withCorpus.view.jobs,
						{
							...withCorpus.view.jobs[0],
							is_since_checkpoint: false,
							job_id: 2,
							url: outsideCheckpointUrl,
						},
					],
				},
			};
			render(<App />);

			expect(await screen.findByText(fixtureUrl)).toBeInTheDocument();
			expect(screen.getByText(outsideCheckpointUrl)).toBeInTheDocument();
		},
	);

	it("distinguishes an empty checkpoint scope from an empty corpus", async () => {
		snapshot = {
			...withCorpus,
			view: {
				...withCorpus.view,
				jobs: withCorpus.view.jobs.map((job) => ({
					...job,
					is_since_checkpoint: false,
				})),
			},
		};
		render(<App />);

		expect(
			await screen.findByText("No jobs since checkpoint."),
		).toBeInTheDocument();
		expect(screen.queryByText("No jobs yet.")).not.toBeInTheDocument();
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
