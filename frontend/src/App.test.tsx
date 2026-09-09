import aiUnavailable from "@fixtures/snapshots/ai_unavailable.json";
import emptyCorpus from "@fixtures/snapshots/idle_empty_corpus.json";
import withCorpus from "@fixtures/snapshots/idle_with_corpus.json";
import withSelection from "@fixtures/snapshots/idle_with_selection.json";
import runFinishedWithNotice from "@fixtures/snapshots/run_finished_with_notice.json";
import runInProgressWithFailures from "@fixtures/snapshots/run_in_progress_with_failures.json";
import { invoke } from "@tauri-apps/api/core";
import {
	act,
	cleanup,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { JOBS_SEARCH_DEBOUNCE_MS } from "./constants";
import type {
	BodyResponse,
	SelectedJobVisibility,
	SnapshotEnvelope,
} from "./ipc/types";

const listeners = new Map<string, (event: { payload: unknown }) => void>();
const corpus = withCorpus as unknown as SnapshotEnvelope;
const empty = emptyCorpus as unknown as SnapshotEnvelope;
const selected = withSelection as unknown as SnapshotEnvelope;
const reviewFixtures = [
	["idle empty corpus", emptyCorpus],
	["idle with corpus", withCorpus],
	["run in progress with failures", runInProgressWithFailures],
	["run finished with notice", runFinishedWithNotice],
	["AI unavailable", aiUnavailable],
	["idle with selection", withSelection],
] as const;
let snapshot: SnapshotEnvelope | null = corpus;
let bodyResponses = new Map<string, BodyResponse | null>();

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn(
		async (name: string, callback: (event: { payload: unknown }) => void) => {
			listeners.set(name, callback);
			return () => listeners.delete(name);
		},
	),
}));
vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(async (name: string, payload?: { key?: string }) => {
		if (name === "get_snapshot") return snapshot;
		if (name === "fetch_body") return bodyResponses.get(payload?.key ?? "");
		return undefined;
	}),
}));

async function renderLoaded() {
	render(<App />);
	await screen.findByRole("heading", { name: "Jobs" });
	await waitFor(() => expect(listeners.has("harvester://snapshot")).toBe(true));
}

function intents() {
	return vi
		.mocked(invoke)
		.mock.calls.filter(([name]) => name === "dispatch_intent");
}

function emitSnapshot(next: SnapshotEnvelope) {
	const listener = listeners.get("harvester://snapshot");
	if (!listener) throw new Error("snapshot listener must be registered");
	act(() => listener({ payload: next }));
}

describe("job list", () => {
	afterEach(() => {
		cleanup();
		vi.useRealTimers();
	});
	beforeEach(() => {
		listeners.clear();
		snapshot = corpus;
		bodyResponses = new Map();
		vi.mocked(invoke).mockClear();
	});

	it("pins the desktop job-list contract in every bridge fixture", () => {
		for (const fixture of reviewFixtures.map(([, fixture]) => fixture)) {
			expect(fixture.view).not.toHaveProperty("jobs");
			expect(fixture.view.desktop_job_list).toMatchObject({
				mode: expect.any(String),
				query: expect.any(String),
				rows: expect.any(Array),
				scoped_count: expect.any(Number),
				visible_count: expect.any(Number),
				truncated: expect.any(Boolean),
				hidden_without_fetch_time: expect.any(Number),
			});
			for (const row of fixture.view.desktop_job_list.rows) {
				expect(row).toMatchObject({
					job_id: expect.any(Number),
					url: expect.any(String),
					fetched_utc: expect.anything(),
				});
			}
		}
	});

	it("renders exactly the rows core sent", async () => {
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					rows: [
						{
							...corpus.view.desktop_job_list.rows[0],
							is_since_checkpoint: false,
							url: "https://example.invalid/core-row",
						},
					],
				},
			},
		};
		await renderLoaded();
		expect(
			await screen.findByText("https://example.invalid/core-row"),
		).toBeInTheDocument();
		expect(
			screen.queryByText("https://example.invalid/fixture"),
		).not.toBeInTheDocument();
	});

	it("shows a neutral state before the first snapshot arrives", async () => {
		snapshot = null;
		await renderLoaded();
		expect(screen.getByText("Loading jobs…")).toBeInTheDocument();
		expect(screen.queryByText("No jobs yet.")).not.toBeInTheDocument();
		expect(
			screen.queryByText("No jobs since checkpoint."),
		).not.toBeInTheDocument();
		expect(screen.queryByText("No matches.")).not.toBeInTheDocument();
	});

	it("renders no job rows in Results mode and falls back to signal candidates", async () => {
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					mode: "Results",
					rows: [],
				},
				signal_candidate_rows: [
					{
						job_id: 7,
						url: "https://example.invalid/candidate",
						score: 90,
						score_band: "High",
						source_tier: "Tier3",
						gist_truncated: "Candidate gist",
						themes: [],
						dupes_count: 0,
						state_label: "Scored",
						signal_key: "candidate",
						outcome: null,
					},
				],
			},
		};
		await renderLoaded();
		expect(
			await screen.findByRole("button", { name: "Candidate gist" }),
		).toBeInTheDocument();
		expect(
			screen.queryByText("https://example.invalid/fixture"),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("textbox", { name: "Search jobs" }),
		).not.toBeInTheDocument();
	});

	it("explains when Results mode has no candidates", async () => {
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					mode: "Results",
					rows: [],
				},
				signal_candidate_rows: [],
			},
		};
		await renderLoaded();
		expect(screen.getByText("No result candidates.")).toBeInTheDocument();
	});

	it("shows the truncation hint with both counts", async () => {
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					truncated: true,
					scoped_count: 400,
					visible_count: 12,
				},
			},
		};
		await renderLoaded();
		expect(
			await screen.findByText("Showing 12 of 400 jobs."),
		).toBeInTheDocument();
	});

	it("notes jobs hidden for a missing fetch time", async () => {
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					hidden_without_fetch_time: 3,
				},
			},
		};
		await renderLoaded();
		expect(
			await screen.findByText(
				"3 jobs are hidden because their fetch time is missing.",
			),
		).toBeInTheDocument();
	});

	it("dispatches SetJobsSearchQuery once after the debounce", async () => {
		await renderLoaded();
		vi.useFakeTimers();
		const input = screen.getByRole("textbox", { name: "Search jobs" });
		fireEvent.change(input, { target: { value: "a" } });
		fireEvent.change(input, { target: { value: "ab" } });
		fireEvent.change(input, { target: { value: "abc" } });
		act(() => vi.advanceTimersByTime(JOBS_SEARCH_DEBOUNCE_MS));
		expect(intents()).toEqual([
			[
				"dispatch_intent",
				{ payload: { type: "SetJobsSearchQuery", payload: { text: "abc" } } },
			],
		]);
	});

	it("clearing the search cancels the pending query dispatch", async () => {
		await renderLoaded();
		vi.useFakeTimers();
		const input = screen.getByRole("textbox", { name: "Search jobs" });
		fireEvent.change(input, { target: { value: "pending" } });
		fireEvent.change(input, { target: { value: "" } });
		act(() => vi.advanceTimersByTime(JOBS_SEARCH_DEBOUNCE_MS));
		expect(intents()).toEqual([
			["dispatch_intent", { payload: { type: "ClearJobsSearch" } }],
		]);
		fireEvent.change(input, { target: { value: "pending" } });
		fireEvent.keyDown(input, { key: "Escape" });
		act(() => vi.advanceTimersByTime(JOBS_SEARCH_DEBOUNCE_MS));
		expect(intents()).toHaveLength(2);
		expect(intents()).toEqual([
			["dispatch_intent", { payload: { type: "ClearJobsSearch" } }],
			["dispatch_intent", { payload: { type: "ClearJobsSearch" } }],
		]);
	});

	it("changing the list mode cancels the pending query dispatch", async () => {
		await renderLoaded();
		vi.useFakeTimers();
		fireEvent.change(screen.getByRole("textbox", { name: "Search jobs" }), {
			target: { value: "pending" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Results" }));
		emitSnapshot({
			...corpus,
			generation: corpus.generation + 1,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					mode: "Results",
				},
			},
		});
		expect(
			screen.queryByRole("textbox", { name: "Search jobs" }),
		).not.toBeInTheDocument();
		act(() => vi.advanceTimersByTime(JOBS_SEARCH_DEBOUNCE_MS));
		expect(intents()).toEqual([
			[
				"dispatch_intent",
				{ payload: { type: "SetJobListMode", payload: { mode: "Results" } } },
			],
		]);
	});

	it("does not keep an undispatched query across a mode change", async () => {
		await renderLoaded();
		vi.useFakeTimers();
		fireEvent.change(screen.getByRole("textbox", { name: "Search jobs" }), {
			target: { value: "rust" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Results" }));
		emitSnapshot({
			...corpus,
			generation: corpus.generation + 1,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					mode: "Results",
				},
			},
		});
		expect(
			screen.queryByRole("textbox", { name: "Search jobs" }),
		).not.toBeInTheDocument();
		fireEvent.click(screen.getByRole("button", { name: "Since checkpoint" }));
		emitSnapshot({
			...corpus,
			generation: corpus.generation + 2,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					mode: "SinceCheckpoint",
					query: "",
				},
			},
		});
		expect(screen.getByRole("textbox", { name: "Search jobs" })).toHaveValue(
			"",
		);
		act(() => vi.advanceTimersByTime(JOBS_SEARCH_DEBOUNCE_MS));
		expect(intents()).toEqual([
			[
				"dispatch_intent",
				{ payload: { type: "SetJobListMode", payload: { mode: "Results" } } },
			],
			[
				"dispatch_intent",
				{
					payload: {
						type: "SetJobListMode",
						payload: { mode: "SinceCheckpoint" },
					},
				},
			],
		]);
	});

	it("unmounting cancels the pending query dispatch", async () => {
		await renderLoaded();
		vi.useFakeTimers();
		fireEvent.change(screen.getByRole("textbox", { name: "Search jobs" }), {
			target: { value: "pending" },
		});
		cleanup();
		act(() => vi.advanceTimersByTime(JOBS_SEARCH_DEBOUNCE_MS));
		expect(intents()).toEqual([]);
	});

	it("dispatches SetJobListMode from the two-mode selector", async () => {
		await renderLoaded();
		fireEvent.click(screen.getByRole("button", { name: "Results" }));
		expect(intents()).toEqual([
			[
				"dispatch_intent",
				{ payload: { type: "SetJobListMode", payload: { mode: "Results" } } },
			],
		]);
	});

	it("dispatches SelectJob when a job row is clicked", async () => {
		await renderLoaded();
		const rowButton = screen
			.getByText("https://example.invalid/fixture")
			.closest("button");
		if (!rowButton) throw new Error("fixture job must render as a button");
		fireEvent.click(rowButton);
		expect(intents()).toEqual([
			[
				"dispatch_intent",
				{ payload: { type: "SelectJob", payload: { job_id: 1 } } },
			],
		]);
	});

	it("renders failed job outcomes without passing an object to React", async () => {
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					rows: [
						{
							...corpus.view.desktop_job_list.rows[0],
							outcome: { Failed: { reason: "http 503" } },
						},
					],
				},
			},
		};
		await renderLoaded();
		expect(screen.getByText("Failed")).toBeInTheDocument();
	});

	it("formats fetched timestamps as readable metadata", async () => {
		const timestamp = "2023-11-14T22:13:20Z";
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					rows: [
						{
							...corpus.view.desktop_job_list.rows[0],
							fetched_utc: timestamp,
						},
					],
				},
			},
		};
		await renderLoaded();
		expect(screen.queryByText(timestamp)).not.toBeInTheDocument();
		expect(screen.getByText(/2023/)).toBeInTheDocument();
	});

	it("dispatches SelectJob when a Results candidate row is clicked", async () => {
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					mode: "Results",
					rows: [],
				},
				signal_candidate_rows: [
					{
						job_id: 7,
						url: "https://example.invalid/candidate",
						score: 90,
						score_band: "High",
						source_tier: "Tier3",
						gist_truncated: "Candidate gist",
						themes: [],
						dupes_count: 0,
						state_label: "Scored",
						signal_key: "candidate",
						outcome: null,
					},
				],
			},
		};
		await renderLoaded();
		fireEvent.click(
			await screen.findByRole("button", { name: "Candidate gist" }),
		);
		expect(intents()).toEqual([
			[
				"dispatch_intent",
				{ payload: { type: "SelectJob", payload: { job_id: 7 } } },
			],
		]);
	});

	it("renders the selected job in the reading pane when it is outside the list scope", async () => {
		snapshot = selected;
		await renderLoaded();
		const selectedJob = selected.view.desktop_job_list.selected_job;
		if (!selectedJob) throw new Error("selection fixture must select a job");
		expect(selectedJob.list_visibility).toBe("OutsideScope");
		expect(
			selected.view.desktop_job_list.rows.map((row) => row.url),
		).not.toContain(selectedJob.url);
		expect(
			screen.getByRole("heading", { name: selectedJob.url }),
		).toBeInTheDocument();
		expect(
			screen.getByRole("button", {
				name: "Open example.invalid in browser",
			}),
		).toBeInTheDocument();
		expect(
			screen.getByText("Selected job is outside this list's scope."),
		).toBeInTheDocument();
	});

	it("opens the selected source through the payload-free core intent", async () => {
		snapshot = selected;
		await renderLoaded();
		fireEvent.click(
			screen.getByRole("button", {
				name: "Open example.invalid in browser",
			}),
		);
		expect(intents()).toContainEqual([
			"dispatch_intent",
			{ payload: { type: "OpenSelectedInBrowser" } },
		]);
	});

	it("explains each selected-job visibility reason", async () => {
		const selectedJob = selected.view.desktop_job_list.selected_job;
		if (!selectedJob) throw new Error("selection fixture must select a job");
		for (const [visibility, message] of Object.entries({
			Visible: "Selected job is visible in this list.",
			OutsideScope: "Selected job is outside this list's scope.",
			QueryMismatch: "Selected job does not match this search.",
			Capped: "Selected job is outside the displayed row limit.",
		}) as [SelectedJobVisibility, string][]) {
			snapshot = {
				...selected,
				view: {
					...selected.view,
					desktop_job_list: {
						...selected.view.desktop_job_list,
						selected_job: { ...selectedJob, list_visibility: visibility },
					},
				},
			};
			const rendered = render(<App />);
			expect(await screen.findByText(message)).toBeInTheDocument();
			rendered.unmount();
		}
	});

	it("marks a row as selected only when core sends the selected job id", async () => {
		const selectedJob = selected.view.desktop_job_list.selected_job;
		if (!selectedJob) throw new Error("selection fixture must select a job");
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					selected_job: {
						...selectedJob,
						job_id: corpus.view.desktop_job_list.rows[0].job_id,
					},
				},
			},
		};
		await renderLoaded();
		expect(
			screen
				.getAllByText("https://example.invalid/fixture")[0]
				.closest("button"),
		).toHaveAttribute("aria-pressed", "true");
	});

	it.each(reviewFixtures)(
		"renders the review workspace for the %s fixture",
		async (_, fixture) => {
			snapshot = fixture as unknown as SnapshotEnvelope;
			await renderLoaded();
			expect(screen.getByRole("heading", { name: "Jobs" })).toBeInTheDocument();
			expect(document.querySelector(".reading-pane")).not.toBeNull();
		},
	);

	it("renders only body text from a matching body response and opens extracted links by index", async () => {
		const rich = runFinishedWithNotice as unknown as SnapshotEnvelope;
		const reference = rich.view.right_pane.summary_markdown;
		if (!reference)
			throw new Error("finished-run fixture must contain a summary body ref");
		snapshot = rich;
		bodyResponses.set(reference.key, {
			content_hash: reference.content_hash,
			text: "<span>untrusted markup</span> **Rendered summary** [Fixture link](https://fixture.invalid/link)",
		});
		await renderLoaded();
		expect(await screen.findByText("Rendered summary")).toBeInTheDocument();
		expect(document.querySelector(".markdown-body span")).toBeNull();
		fireEvent.click(screen.getByRole("button", { name: "Fixture link" }));
		expect(intents()).toContainEqual([
			"dispatch_intent",
			{
				payload: {
					type: "OpenExtractedLink",
					payload: { job_id: 1, link_index: 0 },
				},
			},
		]);
	});

	it("never renders remote Markdown images", async () => {
		const rich = runFinishedWithNotice as unknown as SnapshotEnvelope;
		const reference = rich.view.right_pane.summary_markdown;
		if (!reference)
			throw new Error("finished-run fixture must contain a summary body ref");
		snapshot = rich;
		bodyResponses.set(reference.key, {
			content_hash: reference.content_hash,
			text: "![Remote image](https://fixture.invalid/image.png)",
		});
		await renderLoaded();
		expect(await screen.findByText("Remote image")).toBeInTheDocument();
		expect(document.querySelector(".markdown-body img")).toBeNull();
	});

	it("keeps a mismatched body response visibly stale until the next snapshot", async () => {
		const rich = runFinishedWithNotice as unknown as SnapshotEnvelope;
		const reference = rich.view.right_pane.summary_markdown;
		if (!reference)
			throw new Error("finished-run fixture must contain a summary body ref");
		snapshot = rich;
		bodyResponses.set(reference.key, {
			content_hash: "different-body-hash",
			text: "stale content",
		});
		await renderLoaded();
		expect(
			await screen.findByText("Article content changed; refreshing…"),
		).toBeInTheDocument();
		expect(screen.queryByText("stale content")).not.toBeInTheDocument();
	});

	it("treats a null response for a live body reference as unavailable", async () => {
		const rich = runFinishedWithNotice as unknown as SnapshotEnvelope;
		const reference = rich.view.right_pane.summary_markdown;
		if (!reference)
			throw new Error("finished-run fixture must contain a summary body ref");
		snapshot = rich;
		bodyResponses.set(reference.key, null);
		await renderLoaded();
		expect(
			await screen.findByText("Article content is unavailable."),
		).toBeInTheDocument();
		expect(
			screen.queryByText("Article content changed; refreshing…"),
		).not.toBeInTheDocument();
	});

	it("fetches only the summary and offers no raw-text mode", async () => {
		const rich = runFinishedWithNotice as unknown as SnapshotEnvelope;
		const reference = rich.view.right_pane.summary_markdown;
		if (!reference)
			throw new Error("finished-run fixture must contain a summary body ref");
		snapshot = rich;
		bodyResponses.set(reference.key, {
			content_hash: reference.content_hash,
			text: "Summary only",
		});
		await renderLoaded();
		await screen.findByText("Summary only");
		expect(
			vi.mocked(invoke).mock.calls.filter(([name]) => name === "fetch_body"),
		).toEqual([["fetch_body", { key: "SummaryMarkdown" }]]);
		expect(
			screen.queryByRole("button", { name: "Raw text" }),
		).not.toBeInTheDocument();
	});

	it("distinguishes an empty corpus, an empty scope and an empty search result", async () => {
		snapshot = empty;
		let rendered = render(<App />);
		expect(await screen.findByText("No jobs yet.")).toBeInTheDocument();
		rendered.unmount();
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					rows: [],
					query: "",
				},
			},
		};
		rendered = render(<App />);
		expect(
			await screen.findByText("No jobs since checkpoint."),
		).toBeInTheDocument();
		rendered.unmount();
		snapshot = {
			...corpus,
			view: {
				...corpus.view,
				desktop_job_list: {
					...corpus.view.desktop_job_list,
					rows: [],
					query: "query",
				},
			},
		};
		rendered = render(<App />);
		expect(await screen.findByText("No matches.")).toBeInTheDocument();
		rendered.unmount();
	});

	it("dispatches PollSources through the restricted intent channel", async () => {
		await renderLoaded();
		fireEvent.click(screen.getByRole("button", { name: "Poll Sources" }));
		await waitFor(() =>
			expect(invoke).toHaveBeenCalledWith("dispatch_intent", {
				payload: { type: "PollSources" },
			}),
		);
	});
});
