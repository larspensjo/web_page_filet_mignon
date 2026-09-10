import runFinishedWithNotice from "@fixtures/snapshots/run_finished_with_notice.json";
import runInProgressWithFailures from "@fixtures/snapshots/run_in_progress_with_failures.json";
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

describe("RunSurface", () => {
	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
		vi.useRealTimers();
	});

	it("renders the six projected stages, including failure counts and muted skips", () => {
		render(<RunSurface view={inProgress.view} />);

		expect(screen.getByText("Scanning sources")).toBeInTheDocument();
		expect(screen.getByText("Triaging")).toBeInTheDocument();
		expect(screen.getAllByText("Done")).toHaveLength(3);
		expect(screen.getByText("Estimating…")).toBeInTheDocument();
		expect(screen.getByText(/1 done, 1 failed/)).toBeInTheDocument();
		expect(document.querySelectorAll(".stage-row")).toHaveLength(6);
		expect(document.querySelectorAll(".stage-row .stage-bar")).toHaveLength(6);
		expect(document.querySelector(".run-eta")).toBeNull();
		expect(document.querySelector(".stage-row--muted")).not.toBeNull();
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
		expect(
			screen.getByRole("button", { name: "Run triage + summaries" }),
		).not.toBeDisabled();
		fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
		expect(dispatchIntent).toHaveBeenCalledWith({
			type: "DismissRunFinishedNotice",
		});
	});

	it("gates the three actions from projected view flags and dispatches their intents", () => {
		const view = {
			...inProgress.view,
			triage_can_start: true,
		};
		render(<RunSurface view={view} />);

		const poll = screen.getByRole("button", { name: "Poll Sources" });
		const pipeline = screen.getByRole("button", {
			name: "Run triage + summaries",
		});
		const stop = screen.getByRole("button", { name: "Stop" });
		expect(poll).not.toBeDisabled();
		expect(pipeline).not.toBeDisabled();
		expect(stop).not.toBeDisabled();

		fireEvent.click(poll);
		fireEvent.click(pipeline);
		fireEvent.click(stop);
		expect(dispatchIntent).toHaveBeenNthCalledWith(1, { type: "PollSources" });
		expect(dispatchIntent).toHaveBeenNthCalledWith(2, { type: "RunPipeline" });
		expect(dispatchIntent).toHaveBeenNthCalledWith(3, { type: "StopOrFinish" });
	});

	it("computes ETA only from each active stage's own rate", () => {
		const stages: StageProgress[] = [
			{
				stage: "ScanningSources",
				status: "Done",
				completed: 100,
				failed: 0,
				total: 100,
				started_at_utc: "2023-11-14T22:10:00Z",
				ended_at_utc: "2023-11-14T22:12:00Z",
			},
			{
				stage: "Triaging",
				status: "Active",
				completed: 2,
				failed: 0,
				total: 4,
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

	it("reports an active zero-total stage as in progress", () => {
		const stages = inProgress.view.run_progress.stages.map((stage) => ({
			...stage,
			status:
				stage.stage === "Triaging" ? ("Active" as const) : ("Done" as const),
			completed: stage.stage === "Triaging" ? 0 : stage.total,
			failed: 0,
			total: stage.stage === "Triaging" ? 0 : stage.total,
		}));
		const view = {
			...inProgress.view,
			run_progress: { ...inProgress.view.run_progress, stages },
		};

		render(<RunSurface view={view} />);

		expect(
			document.querySelector('[data-stage="Triaging"] .stage-status'),
		).toHaveTextContent("In progress");
	});

	it("keeps the finishing fallback on an active stage with no remaining work", () => {
		const stages = inProgress.view.run_progress.stages.map((stage) => ({
			...stage,
			status:
				stage.stage === "Triaging" ? ("Active" as const) : ("Done" as const),
			completed: stage.total,
		}));
		const view = {
			...inProgress.view,
			run_progress: { ...inProgress.view.run_progress, stages },
		};

		render(<RunSurface view={view} />);

		expect(screen.getByText("Finishing…")).toBeInTheDocument();
	});
});
