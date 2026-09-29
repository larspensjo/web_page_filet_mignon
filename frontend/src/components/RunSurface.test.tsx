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
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { dispatchIntent } from "../ipc/intent";
import type { SnapshotEnvelope, StageProgress } from "../ipc/types";
import { estimateActiveStageEtas, RunSurface } from "./RunSurface";

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
		expect(screen.getByText(/1 done, 1 failed/)).toBeInTheDocument();
		expect(document.querySelectorAll(".stage-row")).toHaveLength(6);
		expect(document.querySelectorAll(".stage-row .stage-bar")).toHaveLength(6);
		expect(document.querySelector(".run-eta")).toBeNull();
		expect(
			document.querySelector('[data-stage="ScoringSignals"] .stage-status'),
		).toHaveTextContent("Waiting for articles");
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
