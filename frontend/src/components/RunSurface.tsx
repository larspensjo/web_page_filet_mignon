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

const ARTICLE_SCALE_STAGES: readonly StageName[] = [
	"DownloadingArticles",
	"Triaging",
	"Summarizing",
	"ScoringSignals",
];

export type ActiveStageEta = {
	stage: StageName;
	seconds: number;
};

export type StageRowPresentation = {
	stage: StageProgress;
	countText: string;
	bar: {
		remaining: number;
		scale: number;
		percent: number;
	} | null;
};

/** Return the admitted and completed work this run actually performs at a stage. */
export function newWork(stage: StageProgress): { total: number; done: number } {
	return {
		total: Math.max(0, stage.total - stage.reused),
		done: Math.max(0, stage.completed - stage.reused),
	};
}

/** Estimate each active stage independently, without mixing heterogeneous work units. */
export function estimateActiveStageEtas(
	stages: StageProgress[],
	now: Date | string,
): ActiveStageEta[] {
	const nowMs = Date.parse(typeof now === "string" ? now : now.toISOString());
	if (!Number.isFinite(nowMs)) return [];

	return stages.flatMap((stage) => {
		if (
			stage.status !== "Active" ||
			!stage.total_is_final ||
			stage.started_at_utc === null
		)
			return [];
		const startedAt = Date.parse(stage.started_at_utc);
		const { total, done } = newWork(stage);
		const settled = Math.min(done + Math.max(stage.failed, 0), total);
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

/** Present waiting work on a shared article scale using only the current snapshot. */
export function presentStageRows(
	stages: StageProgress[],
	stopping: boolean,
): StageRowPresentation[] {
	const articleScale = Math.max(
		0,
		...stages
			.filter((stage) => ARTICLE_SCALE_STAGES.includes(stage.stage))
			.map((stage) => newWork(stage).total),
	);
	return stages.map((stage) => {
		const loading = stage.stage === "LoadingArticles";
		const { total: newTotal, done: newDone } = newWork(stage);
		const remaining = Math.max(0, newTotal - newDone - stage.failed);
		const count = loading
			? `${stage.completed} done`
			: stopping
				? `${newDone} done`
				: newTotal === 0
					? "0 to do"
					: `${remaining} of ${newTotal} to do`;
		const countText =
			stage.failed > 0 ? `${count} · ${stage.failed} failed` : count;
		const scale =
			stage.stage === "ScanningSources" ? stage.total : articleScale;
		const displayedRemaining = stopping ? 0 : remaining;
		return {
			stage,
			countText,
			bar: loading
				? null
				: {
						remaining: displayedRemaining,
						scale,
						percent:
							scale > 0 ? Math.min(100, (displayedRemaining / scale) * 100) : 0,
					},
		};
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
	stopping: boolean,
): string {
	if (stopping || stage.status !== "Active") return statusLabel(stage.status);
	if (!stage.total_is_final)
		return stage.completed + stage.failed >= stage.total
			? "Waiting for articles"
			: "In progress";
	if (eta) return formatStageEta(eta);
	if (stage.total === 0) return statusLabel(stage.status);
	return stage.completed + stage.failed < stage.total
		? "Estimating…"
		: "Finishing…";
}

function StageList({
	stages,
	stageEtas,
	isStopping,
}: {
	stages: StageProgress[];
	stageEtas: ActiveStageEta[];
	isStopping: boolean;
}) {
	const etaByStage = new Map(stageEtas.map((eta) => [eta.stage, eta]));

	return (
		<ol className="stage-list" aria-label="Pipeline stages">
			{presentStageRows(stages, isStopping).map(({ stage, countText, bar }) => {
				const muted = stage.status === "Pending" && stage.total === 0;
				return (
					<li
						className={`stage-row${muted ? " stage-row--muted" : ""}${
							stage.failed > 0 || stage.status === "Failed"
								? " stage-row--warning"
								: ""
						}`}
						data-stage={stage.stage}
						data-status={stage.status}
						key={stage.stage}
					>
						<span className="stage-name">{stageLabel(stage.stage)}</span>
						<div className="stage-count">{countText}</div>
						{bar ? (
							// biome-ignore lint/a11y/useSemanticElements: The bar is drawn through --stage-progress; native meter styling requires vendor pseudo-elements.
							<div
								className="stage-bar"
								role="meter"
								aria-label={`${stageLabel(stage.stage)} work to do`}
								aria-valuemin={0}
								aria-valuemax={bar.scale}
								aria-valuenow={bar.remaining}
								aria-valuetext={countText}
								style={
									{ "--stage-progress": `${bar.percent}%` } as CSSProperties
								}
							/>
						) : (
							<span className="stage-bar-slot" aria-hidden="true" />
						)}
						<span className="stage-status">
							{stageStatus(stage, etaByStage.get(stage.stage), isStopping)}
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
	const canStop = stopEnabled(view.stop_finish_button);
	const isStopping =
		typeof view.run_state === "object" && "Stopping" in view.run_state;
	const stageEtas =
		progress.run_active && !isStopping
			? estimateActiveStageEtas(progress.stages, now)
			: [];
	const unfinishedCount =
		view.unfinished_work === "Unknown"
			? null
			: view.unfinished_work.Known.articles_with_work;
	const resumeLabel =
		unfinishedCount === null
			? "Process unfinished"
			: `Process unfinished (${unfinishedCount})`;

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
						className="run-primary"
						type="button"
						disabled={!view.run_enabled}
						onClick={() => void dispatchIntent({ type: "RunPipeline" })}
					>
						Run
					</button>
					<button
						type="button"
						className="run-resume"
						disabled={!view.resume_enabled}
						title={view.resume_disabled_reason ?? undefined}
						onClick={() =>
							void dispatchIntent({ type: "ResumeUnfinishedWork" })
						}
					>
						{resumeLabel}
					</button>
					<button
						className="run-stop"
						type="button"
						disabled={isStopping || !canStop}
						onClick={() => void dispatchIntent({ type: "StopOrFinish" })}
					>
						{isStopping ? "Stopping…" : "Stop"}
					</button>
				</div>
			</div>
			{view.ai_unavailable_message && (
				<p
					className={`run-reprocess-notice${view.ai_unavailable_message.startsWith("AI features unavailable: saved results could not be opened:") ? " run-reprocess-notice--warning" : ""}`}
					role="status"
				>
					{view.ai_unavailable_message}
				</p>
			)}
			{view.reprocess_notice && (
				<p className="run-reprocess-notice" role="status">
					{`This run is reprocessing ${view.reprocess_notice.articles} unfinished article${
						view.reprocess_notice.articles === 1 ? "" : "s"
					} (up to ${view.reprocess_notice.estimated_calls} model calls).`}
				</p>
			)}

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
					<StageList
						stages={progress.stages}
						stageEtas={stageEtas}
						isStopping={isStopping}
					/>
					<ActivityFeed activity={progress.activity} />
				</div>
			)}
		</section>
	);
}
