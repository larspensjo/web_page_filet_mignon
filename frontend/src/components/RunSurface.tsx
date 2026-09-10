import { type CSSProperties, useEffect, useState } from "react";
import { dispatchIntent } from "../ipc/intent";
import type {
	ActivityEntry,
	SnapshotEnvelope,
	StageProgress,
	StopFinishButtonState,
} from "../ipc/types";

const ETA_REFRESH_INTERVAL_MS = 5_000;

type RunView = SnapshotEnvelope["view"];
type StageName = StageProgress["stage"];

export type ActiveStageEta = {
	stage: StageName;
	seconds: number;
};

/** Estimate each active stage independently, without mixing heterogeneous work units. */
export function estimateActiveStageEtas(
	stages: StageProgress[],
	now: Date | string,
): ActiveStageEta[] {
	const nowMs = Date.parse(typeof now === "string" ? now : now.toISOString());
	if (!Number.isFinite(nowMs)) return [];

	return stages.flatMap((stage) => {
		if (stage.status !== "Active" || stage.started_at_utc === null) return [];
		const startedAt = Date.parse(stage.started_at_utc);
		const total = Math.max(stage.total, 0);
		const settled = Math.min(
			Math.max(stage.completed, 0) + Math.max(stage.failed, 0),
			total,
		);
		const elapsedMs = nowMs - startedAt;
		if (
			!Number.isFinite(startedAt) ||
			total === 0 ||
			settled === 0 ||
			settled >= total ||
			elapsedMs <= 0
		)
			return [];

		return [
			{
				stage: stage.stage,
				seconds: Math.ceil(((total - settled) * elapsedMs) / settled / 1000),
			},
		];
	});
}

function formatStageEta(eta: ActiveStageEta): string {
	const duration =
		eta.seconds < 60
			? `${eta.seconds} sec`
			: `${Math.ceil(eta.seconds / 60)} min`;
	return `about ${duration} left`;
}

function stageLabel(stage: StageName): string {
	const labels: Record<StageName, string> = {
		ScanningSources: "Scanning sources",
		DownloadingArticles: "Downloading articles",
		LoadingArticles: "Loading articles",
		Triaging: "Triaging",
		Summarizing: "Summarizing",
		ScoringSignals: "Scoring signals",
	};
	return labels[stage];
}

function statusLabel(status: StageProgress["status"]): string {
	return {
		Pending: "Pending",
		Active: "In progress",
		Done: "Done",
		Failed: "Failed",
	}[status];
}

function useEtaNow(runActive: boolean): Date {
	const [nowMs, setNowMs] = useState(() => Date.now());
	useEffect(() => {
		if (!runActive) return;
		setNowMs(Date.now());
		const timer = window.setInterval(
			() => setNowMs(Date.now()),
			ETA_REFRESH_INTERVAL_MS,
		);
		return () => window.clearInterval(timer);
	}, [runActive]);
	return new Date(nowMs);
}

function stopEnabled(state: StopFinishButtonState): boolean {
	return typeof state === "object" && state !== null && "Enabled" in state;
}

function progressPercent(stage: StageProgress): number {
	if (stage.total <= 0) return 0;
	return Math.min(100, ((stage.completed + stage.failed) / stage.total) * 100);
}

function stageCount(stage: StageProgress): string {
	const completed = `${stage.completed} done`;
	return stage.failed > 0 ? `${completed}, ${stage.failed} failed` : completed;
}

function activityOutcome(entry: ActivityEntry): string {
	if (typeof entry.outcome === "string") return entry.outcome;
	if ("Failed" in entry.outcome)
		return `Failed: ${entry.outcome.Failed.reason}`;
	return `Skipped: ${entry.outcome.Skipped.reason}`;
}

function ActivityFeed({ activity }: { activity: ActivityEntry[] }) {
	if (activity.length === 0) return null;
	return (
		<section className="activity-feed" aria-labelledby="activity-feed-heading">
			<h3 id="activity-feed-heading">Activity</h3>
			<ul className="activity-list">
				{activity.map((entry) => (
					<li className="activity-entry" key={entry.seq}>
						<span className="activity-stage">{stageLabel(entry.stage)}</span>
						<span className="activity-outcome">{activityOutcome(entry)}</span>
						<span className="activity-url">{entry.title ?? entry.url}</span>
					</li>
				))}
			</ul>
		</section>
	);
}

function stageStatus(
	stage: StageProgress,
	eta: ActiveStageEta | undefined,
): string {
	if (stage.status !== "Active") return statusLabel(stage.status);
	if (eta) return formatStageEta(eta);
	if (stage.total === 0) return statusLabel(stage.status);
	return stage.completed + stage.failed < stage.total
		? "Estimating…"
		: "Finishing…";
}

function StageList({
	stages,
	stageEtas,
}: {
	stages: StageProgress[];
	stageEtas: ActiveStageEta[];
}) {
	const etaByStage = new Map(stageEtas.map((eta) => [eta.stage, eta]));

	return (
		<ol className="stage-list" aria-label="Pipeline stages">
			{stages.map((stage) => {
				const muted = stage.status === "Pending" && stage.total === 0;
				const percent = progressPercent(stage);
				return (
					<li
						className={`stage-row${muted ? " stage-row--muted" : ""}${
							stage.failed > 0 || stage.status === "Failed"
								? " stage-row--warning"
								: ""
						}`}
						data-stage={stage.stage}
						key={stage.stage}
					>
						<span className="stage-name">{stageLabel(stage.stage)}</span>
						<div className="stage-count">{stageCount(stage)}</div>
						<div
							className="stage-bar"
							role="progressbar"
							aria-label={`${stageLabel(stage.stage)} progress`}
							aria-valuemin={0}
							aria-valuemax={stage.total}
							aria-valuenow={Math.min(
								stage.total,
								stage.completed + stage.failed,
							)}
							style={{ "--stage-progress": `${percent}%` } as CSSProperties}
						/>
						<span className="stage-status">
							{stageStatus(stage, etaByStage.get(stage.stage))}
						</span>
					</li>
				);
			})}
		</ol>
	);
}

export function RunSurface({ view }: { view: RunView }) {
	const { run_progress: progress, run_completion_notice: notice } = view;
	const now = useEtaNow(progress.run_active);
	const stageEtas = progress.run_active
		? estimateActiveStageEtas(progress.stages, now)
		: [];
	const canRunPipeline = view.triage_can_start || view.summaries_can_start;
	const canStop = stopEnabled(view.stop_finish_button);

	return (
		<section className="run-surface" aria-labelledby="run-surface-heading">
			<div className="run-surface-header">
				<div className="run-summary">
					<h2 id="run-surface-heading">Run</h2>
					{progress.run_active ? (
						<span className="run-state">Running</span>
					) : (
						<span className="run-state">
							Idle · {view.job_count} articles · {view.archive_filtered_count}{" "}
							ready to archive
						</span>
					)}
				</div>
				<div className="run-actions">
					<button
						className="run-poll"
						type="button"
						disabled={!view.poll_sources_enabled}
						onClick={() => void dispatchIntent({ type: "PollSources" })}
					>
						Poll Sources
					</button>
					<button
						type="button"
						disabled={!canRunPipeline}
						onClick={() => void dispatchIntent({ type: "RunPipeline" })}
					>
						Run triage + summaries
					</button>
					<button
						className="run-stop"
						type="button"
						disabled={!canStop}
						onClick={() => void dispatchIntent({ type: "StopOrFinish" })}
					>
						Stop
					</button>
				</div>
			</div>

			{notice && (
				<div className="run-notice" role="status">
					<span>{`Run finished - ${notice.new_result_count} article${
						notice.new_result_count === 1 ? "" : "s"
					} scored`}</span>
					<button
						className="ghost-button"
						type="button"
						onClick={() =>
							void dispatchIntent({ type: "DismissRunFinishedNotice" })
						}
					>
						Dismiss
					</button>
				</div>
			)}

			{progress.run_active && (
				<div className="run-details">
					<StageList stages={progress.stages} stageEtas={stageEtas} />
					<ActivityFeed activity={progress.activity} />
				</div>
			)}
		</section>
	);
}
