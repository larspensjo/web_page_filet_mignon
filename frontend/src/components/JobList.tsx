import type { ChangeEvent, KeyboardEvent } from "react";
import type {
	DesktopJobListView,
	JobListMode,
	JobListRowView,
	SignalCandidateRow,
} from "../ipc/types";
import {
	formatFetchedTime,
	jobTitle,
	outcomeLabel,
	priorityClass,
} from "./jobPresentation";

type JobListProps = {
	list: DesktopJobListView | undefined;
	candidates: SignalCandidateRow[];
	searchText: string;
	onSearchChange: (text: string) => void;
	onSearchKeyDown: (event: KeyboardEvent<HTMLInputElement>) => void;
	onClearSearch: () => void;
	onChangeMode: (mode: JobListMode) => void;
	onSelectJob: (jobId: number) => void;
	jobCount: number | undefined;
};

function MetadataPills({ job }: { job: JobListRowView }) {
	return (
		<div className="row-metadata">
			{job.tokens !== null && <span className="pill">{job.tokens} tokens</span>}
			{job.fetched_utc && (
				<span className="pill">{formatFetchedTime(job.fetched_utc)}</span>
			)}
			{job.has_summary && <span className="pill">Summary</span>}
			{job.outcome && <span className="pill">{outcomeLabel(job.outcome)}</span>}
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
					<span className="category-label">
						{annotation?.category ?? job.stage}
					</span>
					<span className="row-title">{jobTitle(job)}</span>
					<MetadataPills job={job} />
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
) {
	if (!list) return "Loading jobs…";
	if (jobCount === 0) return "No jobs yet.";
	return list.query ? "No matches." : "No jobs since checkpoint.";
}

export function JobList({
	list,
	candidates,
	searchText,
	onSearchChange,
	onSearchKeyDown,
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
						aria-pressed={mode === "Results"}
						onClick={() => onChangeMode("Results")}
					>
						Results
					</button>
				</fieldset>
			</div>
			{mode === "SinceCheckpoint" && (
				<div className="search-control">
					<input
						aria-label="Search jobs"
						placeholder="Search jobs"
						value={searchText}
						onChange={onChange}
						onKeyDown={onSearchKeyDown}
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
				<p className="empty-state">{emptyMessage(list, jobCount)}</p>
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
