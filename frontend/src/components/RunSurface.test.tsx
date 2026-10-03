import aiUnavailable from "@fixtures/snapshots/ai_unavailable.json";
import exportUnavailable from "@fixtures/snapshots/export_unavailable_during_run.json";
import overlappingActiveStages from "@fixtures/snapshots/overlapping_active_stages.json";
import reprocessNotice from "@fixtures/snapshots/reprocess_notice.json";
import runFinishedWithNotice from "@fixtures/snapshots/run_finished_with_notice.json";
import runInProgressWithFailures from "@fixtures/snapshots/run_in_progress_with_failures.json";
import stoppedWithUnfinishedWork from "@fixtures/snapshots/stopped_with_unfinished_work_export_enabled.json";
import stoppingWithInFlightWork from "@fixtures/snapshots/stopping_with_in_flight_work.json";
import unfinishedWorkAvailable from "@fixtures/snapshots/unfinished_work_available.json";
import {
	act,
	cleanup,
	fireEvent,
	render,
	screen,
	within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { dispatchIntent } from "../ipc/intent";
import type { SnapshotEnvelope, StageProgress } from "../ipc/types";
import {
	estimateActiveStageEtas,
	presentStageRows,
	RunSurface,
} from "./RunSurface";

vi.mock("../ipc/intent", () => ({ dispatchIntent: vi.fn() }));

const inProgress = runInProgressWithFailures as unknown as SnapshotEnvelope;
const finished = runFinishedWithNotice as unknown as SnapshotEnvelope;
const overlapping = overlappingActiveStages as unknown as SnapshotEnvelope;
const unfinished = unfinishedWorkAvailable as unknown as SnapshotEnvelope;
const stopping = stoppingWithInFlightWork as unknown as SnapshotEnvelope;
const stoppedUnfinished =
	stoppedWithUnfinishedWork as unknown as SnapshotEnvelope;
const exportBusy = exportUnavailable as unknown as SnapshotEnvelope;
const reprocess = reprocessNotice as unknown as SnapshotEnvelope;

type StageName = StageProgress["stage"];

function makeStages(
	overrides: Partial<Record<StageName, Partial<StageProgress>>> = {},
): StageProgress[] {
	return inProgress.view.run_progress.stages.map(({ stage }) => ({
		stage,
		status: "Pending",
		completed: 0,
		reused: 0,
		failed: 0,
		total: 0,
		total_is_final: false,
		started_at_utc: null,
		ended_at_utc: null,
		...overrides[stage],
	}));
}

function viewWithStages(stages: StageProgress[]): SnapshotEnvelope["view"] {
	return {
		...inProgress.view,
		run_state: "Active",
		run_progress: { ...inProgress.view.run_progress, stages },
	};
}

function stageRow(stage: StageName): HTMLElement {
	const row = document.querySelector<HTMLElement>(`[data-stage="${stage}"]`);
	expect(row).not.toBeNull();
	return row as HTMLElement;
}

function meterPercent(meter: HTMLElement): number {
	return Number.parseFloat(meter.style.getPropertyValue("--stage-progress"));
}

describe("RunSurface", () => {
	it("renders the AI unavailable message on the Run surface", () => {
		const snapshot = aiUnavailable as unknown as SnapshotEnvelope;
		const { rerender } = render(<RunSurface view={snapshot.view} />);
		expect(
			screen.getByText(snapshot.view.ai_unavailable_message as string),
		).toBeVisible();
		expect(
			screen.getByText(snapshot.view.ai_unavailable_message as string),
		).not.toHaveClass("run-reprocess-notice--warning");
		const refusal =
			"AI features unavailable: saved results could not be opened: .summary_cache.ron: parse RON failed. Restore the file from backup or move it aside, then restart.";
		rerender(
			<RunSurface
				view={{ ...snapshot.view, ai_unavailable_message: refusal }}
			/>,
		);
		expect(screen.getByText(refusal)).toBeVisible();
		expect(screen.getByText(refusal)).toHaveClass(
			"run-reprocess-notice--warning",
		);
	});
	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
		vi.useRealTimers();
	});

	it("renders the six projected stages, failure counts and waiting stages", () => {
		render(<RunSurface view={inProgress.view} />);

		expect(screen.getByText("Scanning sources")).toBeInTheDocument();
		expect(screen.getByText("Triaging")).toBeInTheDocument();
		expect(screen.getAllByText("Done")).toHaveLength(3);
		expect(screen.getByText("Estimating…")).toBeInTheDocument();
		expect(screen.getByText("0 of 2 to do · 1 failed")).toBeInTheDocument();
		expect(document.querySelectorAll(".stage-row")).toHaveLength(6);
		expect(screen.getAllByRole("meter")).toHaveLength(5);
		expect(document.querySelectorAll(".stage-bar-slot")).toHaveLength(1);
		expect(
			stageRow("LoadingArticles").querySelector(".stage-bar-slot"),
		).toHaveAttribute("aria-hidden", "true");
		expect(document.querySelector(".run-eta")).toBeNull();
		expect(
			document.querySelector('[data-stage="ScoringSignals"] .stage-status'),
		).toHaveTextContent("Waiting for articles");
	});

	it("shows remaining work rather than completed work, including in-flight items", () => {
		const stages = makeStages({
			Triaging: { status: "Active", total: 69, completed: 61 },
		});
		render(<RunSurface view={viewWithStages(stages)} />);
		const meter = screen.getByRole("meter", { name: "Triaging work to do" });
		expect(
			within(stageRow("Triaging")).getByText("8 of 69 to do"),
		).toBeVisible();
		expect(meter).toHaveAttribute("aria-valuemin", "0");
		expect(meter).toHaveAttribute("aria-valuenow", "8");
		expect(meter).toHaveAttribute("aria-valuetext", "8 of 69 to do");
		expect(
			presentStageRows(stages, false).find(
				(row) => row.stage.stage === "Triaging",
			),
		).toMatchObject({
			countText: "8 of 69 to do",
			bar: { remaining: 8, scale: 69 },
		});
	});

	it("shares the largest current article total as the scale even when that stage is Done", () => {
		const stages = makeStages({
			ScanningSources: { total: 999 },
			LoadingArticles: { total: 999, completed: 999 },
			DownloadingArticles: { status: "Done", total: 69, completed: 69 },
			Summarizing: { status: "Active", total: 22, completed: 15, failed: 3 },
		});
		const { rerender } = render(<RunSurface view={viewWithStages(stages)} />);
		const meter = screen.getByRole("meter", { name: "Summarizing work to do" });
		expect(
			within(stageRow("Summarizing")).getByText("4 of 22 to do · 3 failed"),
		).toBeVisible();
		expect(meter).toHaveAttribute("aria-valuenow", "4");
		expect(meter).toHaveAttribute("aria-valuetext", "4 of 22 to do · 3 failed");
		expect(meterPercent(meter)).toBeCloseTo(5.8, 1);
		for (const name of [
			"DownloadingArticles",
			"Triaging",
			"Summarizing",
			"ScoringSignals",
		] as const) {
			expect(within(stageRow(name)).getByRole("meter")).toHaveAttribute(
				"aria-valuemax",
				"69",
			);
		}
		// Any article stage can supply the scale, and a new snapshot replaces it.
		for (const name of [
			"DownloadingArticles",
			"Triaging",
			"Summarizing",
			"ScoringSignals",
		] as const) {
			rerender(
				<RunSurface
					view={viewWithStages(
						makeStages({
							[name]: { status: "Done", total: 100, completed: 100 },
						}),
					)}
				/>,
			);
			expect(
				within(stageRow("DownloadingArticles")).getByRole("meter"),
			).toHaveAttribute("aria-valuemax", "100");
		}
		rerender(
			<RunSurface
				view={viewWithStages(makeStages({ Triaging: { total: 10 } }))}
			/>,
		);
		expect(within(stageRow("Triaging")).getByRole("meter")).toHaveAttribute(
			"aria-valuemax",
			"10",
		);
	});

	it("scales Scanning sources against its own total independently of article totals", () => {
		const stages = makeStages({
			ScanningSources: { status: "Active", total: 12, completed: 8 },
			DownloadingArticles: { total: 69 },
		});
		render(<RunSurface view={viewWithStages(stages)} />);
		const meter = screen.getByRole("meter", {
			name: "Scanning sources work to do",
		});
		expect(
			within(stageRow("ScanningSources")).getByText("4 of 12 to do"),
		).toBeVisible();
		expect(meter).toHaveAttribute("aria-valuemax", "12");
		expect(meter).toHaveAttribute("aria-valuenow", "4");
		expect(meterPercent(meter)).toBeCloseTo(33.3, 1);
	});

	it("keeps Loading articles as a done count with an aria-hidden bar placeholder", () => {
		const stages = makeStages({
			LoadingArticles: { total: 69, completed: 69 },
		});
		const { rerender } = render(<RunSurface view={viewWithStages(stages)} />);
		const assertLoading = (count: string) => {
			const row = stageRow("LoadingArticles");
			expect(within(row).getByText(count)).toBeVisible();
			expect(within(row).queryByRole("meter")).toBeNull();
			expect(row.children[2]).toHaveClass("stage-bar-slot");
			expect(row.children[2]).toHaveAttribute("aria-hidden", "true");
			expect(row.children[3]).toHaveClass("stage-status");
		};
		assertLoading("69 done");
		expect(
			presentStageRows(stages, false).find(
				(row) => row.stage.stage === "LoadingArticles",
			)?.bar,
		).toBeNull();
		rerender(
			<RunSurface
				view={viewWithStages(
					makeStages({
						LoadingArticles: { status: "Failed", total: 1, failed: 1 },
					}),
				)}
			/>,
		);
		assertLoading("0 done · 1 failed");
	});

	it("shows zero-total Pending rows as muted with empty meters and 0 to do", () => {
		render(<RunSurface view={viewWithStages(makeStages())} />);
		const row = stageRow("Triaging");
		expect(row).toHaveClass("stage-row--muted");
		expect(within(row).getByText("0 to do")).toBeVisible();
		const meter = within(row).getByRole("meter");
		expect(meter).toHaveAttribute("aria-valuenow", "0");
		expect(meter).toHaveAttribute("aria-valuemax", "0");
		expect(meterPercent(meter)).toBe(0);
	});

	it("floors remaining work at zero when completions and failures exceed the total", () => {
		const stages = makeStages({
			Triaging: { total: 4, completed: 3, failed: 2 },
		});
		render(<RunSurface view={viewWithStages(stages)} />);
		const meter = screen.getByRole("meter", { name: "Triaging work to do" });
		expect(meter).toHaveAttribute("aria-valuenow", "0");
		expect(meter).toHaveAttribute("aria-valuetext", "0 of 4 to do · 2 failed");
		expect(meterPercent(meter)).toBe(0);
		expect(
			presentStageRows(stages, false).find(
				(row) => row.stage.stage === "Triaging",
			)?.bar,
		).toEqual({ remaining: 0, scale: 4, percent: 0 });
	});

	it("empties all Stopping meters and shows done counts as in-flight work drains", () => {
		const { rerender } = render(<RunSurface view={stopping.view} />);
		const assertStopping = (stages: StageProgress[]) => {
			expect(screen.getAllByRole("meter")).toHaveLength(5);
			for (const meter of screen.getAllByRole("meter")) {
				expect(meter).toHaveAttribute("aria-valuenow", "0");
				expect(meterPercent(meter)).toBe(0);
			}
			for (const stage of stages) {
				const count = `${stage.completed} done${stage.failed > 0 ? ` · ${stage.failed} failed` : ""}`;
				expect(
					stageRow(stage.stage).querySelector(".stage-count"),
				).toHaveTextContent(new RegExp(`^${count}$`));
				const bar = within(stageRow(stage.stage)).queryByRole("meter");
				if (bar) expect(bar).toHaveAttribute("aria-valuetext", count);
			}
		};
		assertStopping(stopping.view.run_progress.stages);
		expect(
			stageRow("Triaging").querySelector(".stage-count"),
		).toHaveTextContent(/^0 done$/);
		const stages = stopping.view.run_progress.stages.map((stage) => ({
			...stage,
			completed: stage.stage === "Triaging" ? 1 : stage.completed,
			failed: stage.stage === "LoadingArticles" ? 1 : stage.failed,
		}));
		rerender(
			<RunSurface
				view={{
					...stopping.view,
					run_progress: { ...stopping.view.run_progress, stages },
				}}
			/>,
		);
		assertStopping(stages);
		expect(
			stageRow("Triaging").querySelector(".stage-count"),
		).toHaveTextContent(/^1 done$/);
	});

	it("uses plain stage statuses only while Stopping and preserves Active ETAs and fallbacks", () => {
		vi.useFakeTimers();
		vi.setSystemTime(new Date("2023-11-14T22:13:50Z"));
		const { rerender } = render(<RunSurface view={stopping.view} />);
		expect(
			stageRow("Triaging").querySelector(".stage-status"),
		).toHaveTextContent(/^In progress$/);
		expect(screen.queryByText("Estimating…")).toBeNull();
		const stages = makeStages({
			ScanningSources: { status: "Done", total: 12, completed: 12 },
			DownloadingArticles: { status: "Active", total: 4, total_is_final: true },
			LoadingArticles: { status: "Failed", total: 1, failed: 1 },
			Triaging: {
				status: "Active",
				total: 4,
				completed: 2,
				total_is_final: true,
				started_at_utc: "2023-11-14T22:13:20Z",
			},
			Summarizing: {
				status: "Active",
				total: 2,
				completed: 2,
				total_is_final: true,
			},
			ScoringSignals: { status: "Active" },
		});
		const activeView = viewWithStages(stages);
		rerender(<RunSurface view={activeView} />);
		const activeStatuses = [
			"Done",
			"Estimating…",
			"Failed",
			"about 30 sec left",
			"Finishing…",
			"Waiting for articles",
		];
		stages.forEach((stage, index) => {
			expect(
				stageRow(stage.stage).querySelector(".stage-status")?.textContent,
			).toBe(activeStatuses[index]);
		});
		rerender(
			<RunSurface
				view={{ ...activeView, run_state: { Stopping: { in_flight: 1 } } }}
			/>,
		);
		const stoppingStatuses = [
			"Done",
			"In progress",
			"Failed",
			"In progress",
			"In progress",
			"In progress",
		];
		stages.forEach((stage, index) => {
			expect(
				stageRow(stage.stage).querySelector(".stage-status")?.textContent,
			).toBe(stoppingStatuses[index]);
		});
		for (const text of [
			/about .* left/,
			"Estimating…",
			"Finishing…",
			"Waiting for articles",
		])
			expect(screen.queryByText(text)).toBeNull();
		rerender(<RunSurface view={activeView} />);
		stages.forEach((stage, index) => {
			expect(
				stageRow(stage.stage).querySelector(".stage-status")?.textContent,
			).toBe(activeStatuses[index]);
		});
	});

	it("retains warning styling for failures and Failed stages in Active and Stopping runs", () => {
		const stages = makeStages({
			Summarizing: { status: "Active", total: 22, completed: 15, failed: 3 },
			LoadingArticles: { status: "Failed", total: 1, failed: 1 },
			ScoringSignals: { status: "Failed", total: 1 },
		});
		const view = viewWithStages(stages);
		const { rerender } = render(<RunSurface view={view} />);
		for (const run_state of [
			"Active",
			{ Stopping: { in_flight: 1 } },
		] as const) {
			rerender(<RunSurface view={{ ...view, run_state }} />);
			for (const name of [
				"Summarizing",
				"LoadingArticles",
				"ScoringSignals",
			] as const)
				expect(stageRow(name)).toHaveClass("stage-row--warning");
		}
	});

	it("replays arrival, draining, regrowth and Stop with snapshot-pure remaining bars", () => {
		const pressed = makeStages();
		const poll = makeStages({
			ScanningSources: { status: "Active", total: 12 },
		});
		const firstWave = makeStages({
			ScanningSources: { status: "Active", total: 12, completed: 8 },
			DownloadingArticles: { status: "Active", total: 30, completed: 10 },
			LoadingArticles: { status: "Active", total: 10, completed: 10 },
			Triaging: { status: "Active", total: 10 },
		});
		const backlog = makeStages({
			ScanningSources: { status: "Done", total: 12, completed: 12 },
			DownloadingArticles: { status: "Active", total: 69, completed: 40 },
			LoadingArticles: { status: "Active", total: 40, completed: 40 },
			Triaging: { status: "Active", total: 40, completed: 20 },
			Summarizing: { status: "Active", total: 15, completed: 10 },
			ScoringSignals: { status: "Active", total: 5, completed: 1 },
		});
		const drained = makeStages({
			ScanningSources: { status: "Done", total: 12, completed: 12 },
			DownloadingArticles: { status: "Done", total: 69, completed: 69 },
			LoadingArticles: { status: "Active", total: 69, completed: 69 },
			Triaging: { status: "Active", total: 69, completed: 61 },
			Summarizing: { status: "Active", total: 22, completed: 15, failed: 3 },
			ScoringSignals: { status: "Active", total: 5, completed: 5 },
		});
		const regrowth = drained.map((stage) =>
			stage.stage === "ScoringSignals"
				? { ...stage, total: 12, completed: 5 }
				: stage,
		);
		type ExpectedRow = [count: string, percent: number | null];
		const regrowthRows: ExpectedRow[] = [
			["0 of 12 to do", 0],
			["0 of 69 to do", 0],
			["69 done", null],
			["8 of 69 to do", 11.6],
			["4 of 22 to do · 3 failed", 5.8],
			["7 of 12 to do", 10.1],
		];
		const steps: {
			stages: StageProgress[];
			rows: ExpectedRow[];
			stopping?: boolean;
		}[] = [
			{
				stages: pressed,
				rows: [
					["0 to do", 0],
					["0 to do", 0],
					["0 done", null],
					["0 to do", 0],
					["0 to do", 0],
					["0 to do", 0],
				],
			},
			{
				stages: poll,
				rows: [
					["12 of 12 to do", 100],
					["0 to do", 0],
					["0 done", null],
					["0 to do", 0],
					["0 to do", 0],
					["0 to do", 0],
				],
			},
			{
				stages: firstWave,
				rows: [
					["4 of 12 to do", 33.3],
					["20 of 30 to do", 66.7],
					["10 done", null],
					["10 of 10 to do", 33.3],
					["0 to do", 0],
					["0 to do", 0],
				],
			},
			{
				stages: backlog,
				rows: [
					["0 of 12 to do", 0],
					["29 of 69 to do", 42.0],
					["40 done", null],
					["20 of 40 to do", 29.0],
					["5 of 15 to do", 7.2],
					["4 of 5 to do", 5.8],
				],
			},
			{
				stages: drained,
				rows: [
					["0 of 12 to do", 0],
					["0 of 69 to do", 0],
					["69 done", null],
					["8 of 69 to do", 11.6],
					["4 of 22 to do · 3 failed", 5.8],
					["0 of 5 to do", 0],
				],
			},
			{ stages: regrowth, rows: regrowthRows },
			{
				stages: regrowth.map((stage) => ({ ...stage, total_is_final: true })),
				stopping: true,
				rows: [
					["12 done", 0],
					["69 done", 0],
					["69 done", null],
					["61 done", 0],
					["15 done · 3 failed", 0],
					["5 done", 0],
				],
			},
		];
		const assertRows = (stages: StageProgress[], expected: ExpectedRow[]) => {
			expect(screen.getAllByRole("meter")).toHaveLength(5);
			expect(document.querySelectorAll(".stage-row")).toHaveLength(6);
			stages.forEach((stage, index) => {
				const row = stageRow(stage.stage);
				const [count, percent] = expected[index];
				expect(row.querySelector(".stage-count")?.textContent).toBe(count);
				const meter = within(row).queryByRole("meter");
				if (percent === null) {
					expect(meter).toBeNull();
					expect(row.children[2]).toHaveClass("stage-bar-slot");
				} else {
					expect(meter).not.toBeNull();
					expect(meter).toHaveAttribute("aria-valuetext", count);
					expect(Number(meterPercent(meter as HTMLElement).toFixed(1))).toBe(
						percent,
					);
				}
			});
		};
		const { rerender, unmount } = render(
			<RunSurface view={viewWithStages(pressed)} />,
		);
		for (const step of steps) {
			const view = viewWithStages(step.stages);
			rerender(
				<RunSurface
					view={
						step.stopping
							? { ...view, run_state: { Stopping: { in_flight: 1 } } }
							: view
					}
				/>,
			);
			assertRows(step.stages, step.rows);
			if (step.stopping) {
				const statuses = [
					"Done",
					"Done",
					"In progress",
					"In progress",
					"In progress",
					"In progress",
				];
				step.stages.forEach((stage, index) => {
					expect(
						stageRow(stage.stage).querySelector(".stage-status")?.textContent,
					).toBe(statuses[index]);
				});
				for (const meter of screen.getAllByRole("meter"))
					expect(meter).toHaveAttribute("aria-valuenow", "0");
			}
		}
		unmount();
		render(<RunSurface view={viewWithStages(regrowth)} />);
		assertRows(regrowth, regrowthRows);
	});

	it("collapses to the projected idle summary when no run is active", () => {
		const view = {
			...finished.view,
			job_count: 312,
			archive_filtered_count: 47,
			run_completion_notice: null,
			run_progress: { stages: [], run_active: false, activity: [] },
		};
		render(<RunSurface view={view} />);

		expect(
			screen.getByText("Idle · 312 articles · 47 ready to archive"),
		).toBeInTheDocument();
		expect(
			screen.queryByRole("list", { name: "Pipeline stages" }),
		).not.toBeInTheDocument();
	});

	it("replaces the activity feed on successive snapshots without accumulating history", () => {
		const firstActivity = [1, 2].map((seq) => ({
			seq,
			url: `https://fixture.invalid/activity/${seq}`,
			title: null,
			stage: "Triaging" as const,
			outcome: "Succeeded" as const,
		}));
		const firstView = {
			...inProgress.view,
			run_progress: {
				...inProgress.view.run_progress,
				activity: firstActivity,
			},
		};
		const { rerender } = render(<RunSurface view={firstView} />);
		expect(document.querySelectorAll(".activity-entry")).toHaveLength(2);

		const secondActivity = [
			{
				seq: 3,
				url: "https://fixture.invalid/activity/3",
				title: null,
				stage: "Triaging" as const,
				outcome: "Succeeded" as const,
			},
		];
		rerender(
			<RunSurface
				view={{
					...firstView,
					run_progress: {
						...firstView.run_progress,
						activity: secondActivity,
					},
				}}
			/>,
		);

		expect(document.querySelectorAll(".activity-entry")).toHaveLength(1);
		expect(
			screen.getByText("https://fixture.invalid/activity/3"),
		).toBeInTheDocument();
		expect(
			screen.queryByText("https://fixture.invalid/activity/2"),
		).not.toBeInTheDocument();
	});

	it("renders the persistent completion notice from the finished fixture", () => {
		render(<RunSurface view={finished.view} />);

		expect(
			screen.getByText("Run finished - 1 article scored"),
		).toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Run" })).not.toBeDisabled();
		fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
		expect(dispatchIntent).toHaveBeenCalledWith({
			type: "DismissRunFinishedNotice",
		});
	});

	it("renders a single primary Run and sends a Full run intent", () => {
		render(<RunSurface view={finished.view} />);
		const run = screen.getByRole("button", { name: "Run" });
		expect(run).toHaveClass("run-primary");
		expect(run).toBeEnabled();
		expect(screen.queryByRole("button", { name: "Poll Sources" })).toBeNull();
		fireEvent.click(run);
		expect(dispatchIntent).toHaveBeenCalledWith({ type: "RunPipeline" });
	});

	it("renders the core unfinished count and sends a payload-free Resume intent", () => {
		render(<RunSurface view={unfinished.view} />);
		const summary = (
			unfinished.view.unfinished_work as {
				Known: { articles_with_work: number };
			}
		).Known;
		const resume = screen.getByRole("button", {
			name: `Process unfinished (${summary.articles_with_work})`,
		});
		expect(resume).toBeEnabled();
		fireEvent.click(resume);
		expect(dispatchIntent).toHaveBeenCalledWith({
			type: "ResumeUnfinishedWork",
		});
	});

	it("disables both run requests while active and reports Stopping… during a drain", () => {
		render(<RunSurface view={exportBusy.view} />);
		expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
		const stop = screen.getByRole("button", { name: "Stop" });
		expect(stop).toBeEnabled();
		fireEvent.click(stop);
		expect(dispatchIntent).toHaveBeenCalledWith({ type: "StopOrFinish" });
		expect(
			screen.getByRole("button", { name: /Process unfinished/ }),
		).toBeDisabled();
		cleanup();

		render(<RunSurface view={stopping.view} />);
		expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
		expect(screen.getByRole("button", { name: "Stopping…" })).toBeDisabled();
	});

	it("renders concurrent active rows and withholds ETAs while totals are open", () => {
		render(<RunSurface view={overlapping.view} />);
		const openTotalRows = overlapping.view.run_progress.stages.filter(
			(stage) => stage.status === "Active" && !stage.total_is_final,
		);
		expect(openTotalRows.length).toBeGreaterThan(0);
		expect(
			document.querySelectorAll('.stage-row[data-status="Active"]').length,
		).toBeGreaterThanOrEqual(2);
		for (const stage of openTotalRows) {
			expect(
				document.querySelector(`[data-stage="${stage.stage}"] .stage-status`),
			).not.toHaveTextContent(/about .* left/);
		}
		expect(screen.getAllByText("Waiting for articles").length).toBeGreaterThan(
			0,
		);
	});

	it("keeps Run available after Stop when unfinished work remains", () => {
		render(<RunSurface view={stoppedUnfinished.view} />);
		expect(screen.getByRole("button", { name: "Run" })).toBeEnabled();
		expect(
			screen.getByRole("button", { name: /Process unfinished \(/ }),
		).toBeEnabled();
	});

	it("renders the reducer reprocess notice as a muted status", () => {
		render(<RunSurface view={reprocess.view} />);
		expect(
			document.querySelector('.stage-row[data-stage="ScanningSources"]'),
		).toHaveClass("stage-row--muted");
		expect(screen.getByRole("status")).toHaveTextContent(
			/This run is reprocessing 151 unfinished articles/,
		);
		expect(document.querySelector(".run-reprocess-notice")).not.toHaveClass(
			"run-notice",
		);
	});

	it("computes ETA only from each active stage's own rate", () => {
		const stages: StageProgress[] = [
			{
				stage: "ScanningSources",
				status: "Done",
				completed: 100,
				reused: 0,
				failed: 0,
				total: 100,
				total_is_final: true,
				started_at_utc: "2023-11-14T22:10:00Z",
				ended_at_utc: "2023-11-14T22:12:00Z",
			},
			{
				stage: "Triaging",
				status: "Active",
				completed: 2,
				reused: 0,
				failed: 0,
				total: 4,
				total_is_final: true,
				started_at_utc: "2023-11-14T22:13:20Z",
				ended_at_utc: null,
			},
		];

		expect(estimateActiveStageEtas(stages, "2023-11-14T22:13:50Z")).toEqual([
			{ stage: "Triaging", seconds: 30 },
		]);
	});

	it("updates a stalled active-stage estimate without a new snapshot", () => {
		vi.useFakeTimers();
		vi.setSystemTime(new Date("2023-11-14T22:13:30Z"));
		const stages: StageProgress[] = [
			{
				stage: "Triaging",
				status: "Active",
				completed: 2,
				reused: 0,
				failed: 0,
				total: 4,
				total_is_final: true,
				started_at_utc: "2023-11-14T22:13:20Z",
				ended_at_utc: null,
			},
		];
		const view = {
			...inProgress.view,
			run_progress: { ...inProgress.view.run_progress, stages },
		};
		render(<RunSurface view={view} />);
		expect(
			document.querySelector('[data-stage="Triaging"] .stage-status'),
		).toHaveTextContent("about 10 sec left");

		act(() => vi.advanceTimersByTime(5_000));

		expect(
			document.querySelector('[data-stage="Triaging"] .stage-status'),
		).toHaveTextContent("about 15 sec left");
	});

	it("reports an open zero-total stage as waiting for articles", () => {
		const stages = inProgress.view.run_progress.stages.map((stage) => ({
			...stage,
			status:
				stage.stage === "Triaging" ? ("Active" as const) : ("Done" as const),
			completed: stage.stage === "Triaging" ? 0 : stage.total,
			failed: 0,
			total: stage.stage === "Triaging" ? 0 : stage.total,
			total_is_final: stage.stage === "Triaging" ? false : stage.total_is_final,
		}));
		const view = {
			...inProgress.view,
			run_progress: { ...inProgress.view.run_progress, stages },
		};

		render(<RunSurface view={view} />);

		expect(
			document.querySelector('[data-stage="Triaging"] .stage-status'),
		).toHaveTextContent("Waiting for articles");
	});

	it("keeps the finishing fallback on an active stage with no remaining work", () => {
		const stages = inProgress.view.run_progress.stages.map((stage) => ({
			...stage,
			status:
				stage.stage === "Triaging" ? ("Active" as const) : ("Done" as const),
			completed: stage.total,
			total_is_final: true,
		}));
		const view = {
			...inProgress.view,
			run_progress: { ...inProgress.view.run_progress, stages },
		};

		render(<RunSurface view={view} />);

		expect(screen.getByText("Finishing…")).toBeInTheDocument();
	});
});
