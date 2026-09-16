import type { ChangeEvent, RefObject } from "react";
import type {
	DesktopJobListView,
	JobListMode,
	JobListRowView,
	SignalCandidateRow,
} from "../ipc/types";
import {
	formatFetchedTime,
	isFailureOutcome,
	jobTitle,
	priorityClass,
} from "./jobPresentation";

type JobListProps = {
	list: DesktopJobListView | undefined;
	candidates: SignalCandidateRow[];
	searchText: string;
	searchInputRef: RefObject<HTMLInputElement | null>;
	onSearchChange: (text: string) => void;
	onClearSearch: () => void;
	onChangeMode: (mode: JobListMode) => void;
	onSelectJob: (jobId: number) => void;
	jobCount: number | undefined;
};

function JobRowMetadata({ job }: { job: JobListRowView }) {
	const failed = job.outcome ? isFailureOutcome(job.outcome) : false;
	if (!job.fetched_utc && !failed) return null;

	return (
		<div className="row-metadata job-row-metadata">
			{job.fetched_utc && (
				<span className="fetched-time">
					{formatFetchedTime(job.fetched_utc)}
				</span>
			)}
			{failed && <span className="pill failure-marker">Failed</span>}
		</div>
	);
}

function JobRow({
	job,
	selected,
	onSelect,
}: {
	job: JobListRowView;
	selected: boolean;
	onSelect: () => void;
}) {
	const annotation = job.triage_annotation;
	return (
		<li className="job-list-item">
			<button
				className="job-row"
				type="button"
				aria-pressed={selected}
				onClick={onSelect}
			>
				{annotation ? (
					<span
						className={`priority-badge ${priorityClass(annotation.priority)}`}
					>
						P{annotation.priority}
					</span>
				) : (
					<span className="priority-placeholder" aria-hidden="true" />
				)}
				<span className="row-content">
					<span className="row-title">{jobTitle(job)}</span>
					<JobRowMetadata job={job} />
				</span>
			</button>
		</li>
	);
}

function CandidateRow({
	candidate,
	selected,
	onSelect,
}: {
	candidate: SignalCandidateRow;
	selected: boolean;
	onSelect: () => void;
}) {
	return (
		<li className="job-list-item">
			<button
				className="job-row"
				type="button"
				aria-label={candidate.gist_truncated || candidate.url}
				aria-pressed={selected}
				onClick={onSelect}
			>
				<span className="priority-badge priority-normal">
					{candidate.score}
				</span>
				<span className="row-content">
					<span className="category-label">{candidate.source_tier}</span>
					<span className="row-title">
						{candidate.gist_truncated || candidate.url}
					</span>
					<span className="row-metadata">
						{candidate.outcome === "Excluded" && (
							<span className="pill excluded-marker">Excluded</span>
						)}
						<span className="pill">{candidate.score_band}</span>
						{candidate.themes.map((theme) => (
							<span className="pill" key={theme}>
								{theme}
							</span>
						))}
					</span>
				</span>
			</button>
		</li>
	);
}

function emptyMessage(
	list: DesktopJobListView | undefined,
	jobCount: number | undefined,
	mode: JobListMode,
) {
	if (!list) return "Loading jobs…";
	if (jobCount === 0) return "No jobs yet.";
	if (list.query) return "No matches.";
	switch (mode) {
		case "SinceCheckpoint":
			return "No jobs since checkpoint.";
		case "Last24Hours":
			return "Nothing fetched in the last 24 hours.";
		case "Results":
			return "No result candidates.";
		default: {
			const exhaustiveMode: never = mode;
			return exhaustiveMode;
		}
	}
}

export function JobList({
	list,
	candidates,
	searchText,
	searchInputRef,
	onSearchChange,
	onClearSearch,
	onChangeMode,
	onSelectJob,
	jobCount,
}: JobListProps) {
	const mode = list?.mode ?? "SinceCheckpoint";
	const selectedJobId = list?.selected_job?.job_id;
	const onChange = (event: ChangeEvent<HTMLInputElement>) =>
		onSearchChange(event.target.value);
	return (
		<section className="job-pane" aria-labelledby="jobs-title">
			<div className="pane-heading">
				<h2 id="jobs-title">Jobs</h2>
				<fieldset className="mode-selector">
					<legend className="visually-hidden">Job list mode</legend>
					<button
						type="button"
						aria-pressed={mode === "SinceCheckpoint"}
						onClick={() => onChangeMode("SinceCheckpoint")}
					>
						Since checkpoint
					</button>
					<button
						type="button"
						aria-pressed={mode === "Last24Hours"}
						onClick={() => onChangeMode("Last24Hours")}
					>
						Last 24h
					</button>
					<button
						type="button"
						aria-pressed={mode === "Results"}
						onClick={() => onChangeMode("Results")}
					>
						Results
					</button>
				</fieldset>
			</div>
			{mode !== "Results" && (
				<div className="search-control">
					<input
						ref={searchInputRef}
						aria-label="Search jobs"
						placeholder="Search jobs"
						value={searchText}
						onChange={onChange}
					/>
					{searchText && (
						<button
							className="ghost-button"
							type="button"
							onClick={onClearSearch}
						>
							Clear
						</button>
					)}
				</div>
			)}
			{list?.truncated && (
				<p className="list-hint">
					Showing {list.visible_count} of {list.scoped_count} jobs.
				</p>
			)}
			{list && list.hidden_without_fetch_time > 0 && (
				<p className="list-hint">
					{list.hidden_without_fetch_time} jobs are hidden because their fetch
					time is missing.
				</p>
			)}
			{mode === "Results" ? (
				candidates.length === 0 ? (
					<p className="empty-state">No result candidates.</p>
				) : (
					<ul className="job-list">
						{candidates.map((candidate) => (
							<CandidateRow
								candidate={candidate}
								key={candidate.job_id}
								selected={candidate.job_id === selectedJobId}
								onSelect={() => onSelectJob(candidate.job_id)}
							/>
						))}
					</ul>
				)
			) : list === undefined || list.rows.length === 0 ? (
				<p className="empty-state">{emptyMessage(list, jobCount, mode)}</p>
			) : (
				<ul className="job-list">
					{list.rows.map((job) => (
						<JobRow
							job={job}
							key={job.job_id}
							selected={job.job_id === selectedJobId}
							onSelect={() => onSelectJob(job.job_id)}
						/>
					))}
				</ul>
			)}
		</section>
	);
}
