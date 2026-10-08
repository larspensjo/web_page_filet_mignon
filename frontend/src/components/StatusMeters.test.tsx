import emptyOutputFolderStarted from "@fixtures/snapshots/idle_empty_output_folder_started.json";
import withCorpus from "@fixtures/snapshots/idle_with_corpus.json";
import reusedResults from "@fixtures/snapshots/run_in_progress_with_reused_results.json";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type {
	ArchiveMeterView,
	RunState,
	SnapshotEnvelope,
} from "../ipc/types";
import { quotaTone, StatusMeters } from "./StatusMeters";

const corpus = withCorpus as unknown as SnapshotEnvelope;

function renderMeter(
	meter: Partial<ArchiveMeterView> = {},
	run_state: RunState = "Idle",
) {
	render(
		<StatusMeters
			view={{
				...corpus.view,
				run_state,
				archive_meter: {
					selected_count: 110,
					target: 150,
					token_estimate: 60_000,
					unsettled_count: 0,
					status: "Scored",
					...meter,
				},
			}}
		/>,
	);
	return document.querySelector(
		'[data-meter="archive-articles"]',
	) as HTMLElement;
}

describe("StatusMeters", () => {
	afterEach(cleanup);

	it("pins the meter fields the chrome reads from every snapshot", () => {
		expect(corpus.view).toMatchObject({
			archive_meter: {
				selected_count: expect.any(Number),
				target: 150,
				token_estimate: expect.any(Number),
				unsettled_count: expect.any(Number),
				status: "NotScoredYet",
			},
			llm_quota: { label: expect.any(String), severity: expect.any(String) },
		});
		const reused = reusedResults as unknown as SnapshotEnvelope;
		expect(reused.view.archive_partial_coverage).toEqual({
			triaged: 1,
			actionable_total: 5,
		});
		render(<StatusMeters view={reused.view} />);
		expect(screen.getByText("1 / 150 articles")).toBeInTheDocument();
		expect(
			screen.getByText("~5 tokens · 4 still processing"),
		).toBeInTheDocument();
		expect(screen.queryByText("1 of 5 triaged")).not.toBeInTheDocument();
	});

	it("renders completed empty-folder startup without a loading hint", () => {
		const started = emptyOutputFolderStarted as unknown as SnapshotEnvelope;
		render(<StatusMeters view={started.view} />);
		expect(screen.getByText("0 / 150 articles")).toBeInTheDocument();
		expect(screen.getByText("Not scored yet")).toBeInTheDocument();
		expect(
			screen.queryByText("Loading saved results…"),
		).not.toBeInTheDocument();
	});

	it("renders one labelled archive meter and one quota meter from the fixture", () => {
		render(<StatusMeters view={corpus.view} />);
		expect(screen.getByText("0 / 150 articles")).toBeInTheDocument();
		expect(
			screen.getByText("Not scored yet · 2 unfinished"),
		).toBeInTheDocument();
		expect(screen.getByText("LLM calls 0 / unlimited")).toBeInTheDocument();
		expect(screen.getAllByRole("progressbar")).toHaveLength(1);
		expect(screen.getByRole("progressbar")).toHaveAttribute(
			"aria-valuenow",
			"0",
		);
		expect(document.querySelectorAll(".meter--muted")).toHaveLength(2);
	});

	it.each([
		[0, 0, "muted"],
		[110, 73, "muted"],
		[149, 99, "muted"],
		[150, 100, "attention"],
		[200, 100, "attention"],
	])(
		"renders %i selected with %i percent and %s tone, never warning",
		(count, percent, tone) => {
			const meter = renderMeter({ selected_count: count });
			expect(screen.getByText(`${count} / 150 articles`)).toBeInTheDocument();
			expect(screen.getByRole("progressbar")).toHaveAttribute(
				"aria-valuenow",
				`${percent}`,
			);
			expect(
				screen.getByRole("progressbar").style.getPropertyValue("--meter-fill"),
			).toBe(`${Math.min(100, (count / 150) * 100)}%`);
			expect(meter).toHaveClass(`meter--${tone}`);
			expect(meter).not.toHaveClass("meter--warning");
		},
	);

	it.each([
		["Loading", 0, "Loading saved results…"],
		["Unavailable", 0, "Saved results unavailable"],
		["NotScoredYet", 0, "Not scored yet"],
		["Scored", 110, "~60k tokens"],
		["Scored", 0, "None selected yet"],
	] as const)(
		"renders the exact %s detail with %i selected",
		(status, selected_count, detail) => {
			renderMeter({ status, selected_count });
			expect(screen.getByText(detail)).toBeInTheDocument();
			expect(screen.queryByText(/tokens/)).toBe(
				selected_count > 0 ? screen.getByText(detail) : null,
			);
		},
	);

	it.each(["Loading", "Unavailable"] as const)(
		"suppresses backlog for %s even if supplied",
		(status) => {
			const meter = renderMeter(
				{ status, selected_count: 0, unsettled_count: 40 },
				"Active",
			);
			expect(meter.textContent).not.toMatch(
				/still processing|unfinished|tokens/,
			);
		},
	);

	it.each(["NotScoredYet", "Scored"] as const)(
		"adds backlog to a zero-selection %s hint",
		(status) => {
			renderMeter({ status, selected_count: 0, unsettled_count: 40 });
			expect(
				screen.getByText(
					`${status === "Scored" ? "None selected yet" : "Not scored yet"} · 40 unfinished`,
				),
			).toBeInTheDocument();
			expect(screen.queryByText(/tokens/)).not.toBeInTheDocument();
		},
	);

	it("renders Active backlog as still processing", () => {
		renderMeter({ unsettled_count: 40 }, "Active");
		expect(
			screen.getByText("~60k tokens · 40 still processing"),
		).toBeInTheDocument();
	});

	it("renders Stopping backlog as still processing", () => {
		renderMeter({ unsettled_count: 40 }, { Stopping: { in_flight: 1 } });
		expect(
			screen.getByText("~60k tokens · 40 still processing"),
		).toBeInTheDocument();
	});

	it("renders Idle backlog as unfinished", () => {
		renderMeter({ unsettled_count: 40 }, "Idle");
		expect(screen.getByText("~60k tokens · 40 unfinished")).toBeInTheDocument();
	});

	it("keeps quota escalation independent of archive-count tone", () => {
		render(
			<StatusMeters
				view={{
					...corpus.view,
					archive_meter: { ...corpus.view.archive_meter, selected_count: 150 },
					llm_quota: {
						label: "LLM calls 95 / 100",
						used: 95,
						limit: 100,
						percent: 95,
						severity: "Danger",
					},
				}}
			/>,
		);
		expect(
			document.querySelector('[data-meter="archive-articles"]'),
		).toHaveClass("meter--attention");
		expect(
			document.querySelector('[data-meter="archive-articles"]'),
		).not.toHaveClass("meter--warning");
		expect(document.querySelector('[data-meter="llm-quota"]')).toHaveClass(
			"meter--warning",
		);
		expect(screen.getAllByRole("progressbar")).toHaveLength(2);
	});

	it("maps quota severities to tones", () => {
		expect(quotaTone("Normal")).toBe("muted");
		expect(quotaTone("Unavailable")).toBe("muted");
		expect(quotaTone("Warning")).toBe("attention");
		expect(quotaTone("Danger")).toBe("warning");
		expect(quotaTone("Exhausted")).toBe("warning");
	});
});
