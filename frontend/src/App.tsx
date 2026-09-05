import { useCallback, useEffect, useRef, useState } from "react";
import { JOBS_SEARCH_DEBOUNCE_MS } from "./constants";
import { dispatchIntent } from "./ipc/intent";
import { IPC_SCHEMA_VERSION } from "./ipc/schemaVersion";
import type { JobListMode, SelectedJobVisibility } from "./ipc/types";
import { useProbe } from "./ipc/useProbe";
import { useSnapshot } from "./ipc/useSnapshot";
import "./styles/base.css";
import "./styles/tokens.css";

function visibilityExplanation(visibility: SelectedJobVisibility): string {
	switch (visibility) {
		case "Visible":
			return "Selected job is visible in this list.";
		case "OutsideScope":
			return "Selected job is outside this list's scope.";
		case "QueryMismatch":
			return "Selected job does not match this search.";
		case "Capped":
			return "Selected job is outside the displayed row limit.";
	}
}

export function App() {
	const snapshot = useSnapshot();
	const [searchText, setSearchText] = useState("");
	const pendingSearchDispatch = useRef<ReturnType<typeof setTimeout> | null>(
		null,
	);
	const cancelPendingSearchDispatch = useCallback(() => {
		if (pendingSearchDispatch.current !== null) {
			clearTimeout(pendingSearchDispatch.current);
			pendingSearchDispatch.current = null;
		}
	}, []);

	useProbe();
	const list = snapshot?.view.desktop_job_list;
	const mode: JobListMode = list?.mode ?? "SinceCheckpoint";
	const query = list?.query ?? "";
	useEffect(() => {
		cancelPendingSearchDispatch();
		setSearchText(mode === "SinceCheckpoint" ? query : "");
		return cancelPendingSearchDispatch;
	}, [mode, query, cancelPendingSearchDispatch]);

	if (snapshot?.fatal_message)
		return (
			<main className="blocking">
				<h1>Harvester stopped responding — see engine.log</h1>
				<p>{snapshot.fatal_message}</p>
			</main>
		);
	if (snapshot && snapshot.schema_version !== IPC_SCHEMA_VERSION)
		return (
			<main className="blocking">
				frontend bundle is out of date — run <code>npm run build</code>
			</main>
		);

	const clearSearch = () => {
		cancelPendingSearchDispatch();
		setSearchText("");
		void dispatchIntent({ type: "ClearJobsSearch" });
	};
	const changeMode = (nextMode: JobListMode) => {
		void dispatchIntent({
			type: "SetJobListMode",
			payload: { mode: nextMode },
		});
	};
	const changeSearch = (text: string) => {
		cancelPendingSearchDispatch();
		setSearchText(text);
		if (!text) {
			clearSearch();
			return;
		}
		pendingSearchDispatch.current = setTimeout(() => {
			pendingSearchDispatch.current = null;
			void dispatchIntent({ type: "SetJobsSearchQuery", payload: { text } });
		}, JOBS_SEARCH_DEBOUNCE_MS);
	};
	const rows = list?.rows ?? [];
	const candidates = snapshot?.view.signal_candidate_rows ?? [];
	const emptyMessage =
		snapshot?.view.job_count === 0
			? "No jobs yet."
			: query
				? "No matches."
				: "No jobs since checkpoint.";

	return (
		<main>
			<header>
				<h1>Harvester</h1>
				<button
					type="button"
					onClick={() => void dispatchIntent({ type: "PollSources" })}
				>
					Poll Sources
				</button>
			</header>
			<section aria-label="Jobs">
				<h2>Jobs</h2>
				<fieldset>
					<legend>Job list mode</legend>
					<button
						type="button"
						aria-pressed={mode === "SinceCheckpoint"}
						onClick={() => changeMode("SinceCheckpoint")}
					>
						Since checkpoint
					</button>
					<button
						type="button"
						aria-pressed={mode === "Results"}
						onClick={() => changeMode("Results")}
					>
						Results
					</button>
				</fieldset>
				{mode === "SinceCheckpoint" && (
					<input
						aria-label="Search jobs"
						value={searchText}
						onChange={(event) => changeSearch(event.target.value)}
						onKeyDown={(event) => event.key === "Escape" && clearSearch()}
					/>
				)}
				{list?.truncated && (
					<p>
						Showing {list.visible_count} of {list.scoped_count} jobs.
					</p>
				)}
				{list && list.hidden_without_fetch_time > 0 && (
					<p>
						{list.hidden_without_fetch_time} jobs are hidden because their fetch
						time is missing.
					</p>
				)}
				{mode === "Results" ? (
					candidates.length === 0 ? (
						<p>No result candidates.</p>
					) : (
						candidates.map((candidate) => (
							<button
								type="button"
								key={candidate.job_id}
								onClick={() =>
									void dispatchIntent({
										type: "SelectJob",
										payload: { job_id: candidate.job_id },
									})
								}
							>
								{candidate.gist_truncated || candidate.url}
							</button>
						))
					)
				) : list === undefined ? (
					<p>Loading jobs…</p>
				) : rows.length === 0 ? (
					<p>{emptyMessage}</p>
				) : (
					<ul>
						{rows.map((job) => (
							<li key={job.job_id}>
								<button
									type="button"
									onClick={() =>
										void dispatchIntent({
											type: "SelectJob",
											payload: { job_id: job.job_id },
										})
									}
								>
									<strong>{job.summary_title ?? job.url}</strong>
									<span>{job.stage}</span>
								</button>
							</li>
						))}
					</ul>
				)}
				{list?.selected_job && (
					<section aria-label="Selected job">
						<strong>
							{list.selected_job.summary_title ?? list.selected_job.url}
						</strong>
						<a href={list.selected_job.url}>{list.selected_job.url}</a>
						<p>{visibilityExplanation(list.selected_job.list_visibility)}</p>
					</section>
				)}
			</section>
		</main>
	);
}
