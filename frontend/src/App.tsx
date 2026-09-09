import { useCallback, useEffect, useRef, useState } from "react";
import { JobList } from "./components/JobList";
import { ReadingPane } from "./components/ReadingPane";
import { JOBS_SEARCH_DEBOUNCE_MS } from "./constants";
import { dispatchIntent } from "./ipc/intent";
import { IPC_SCHEMA_VERSION } from "./ipc/schemaVersion";
import type { JobListMode } from "./ipc/types";
import { useProbe } from "./ipc/useProbe";
import { useSnapshot } from "./ipc/useSnapshot";
import "./styles/base.css";
import "./styles/tokens.css";
import "./styles/workspace.css";

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
	const candidates = snapshot?.view.signal_candidate_rows ?? [];

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
			<div className="workspace">
				<JobList
					list={list}
					candidates={candidates}
					searchText={searchText}
					onSearchChange={changeSearch}
					onSearchKeyDown={(event) => event.key === "Escape" && clearSearch()}
					onClearSearch={clearSearch}
					onChangeMode={changeMode}
					onSelectJob={(jobId) =>
						void dispatchIntent({
							type: "SelectJob",
							payload: { job_id: jobId },
						})
					}
					jobCount={snapshot?.view.job_count}
				/>
				<ReadingPane
					selected={list?.selected_job}
					summary={snapshot?.view.right_pane.summary_markdown}
				/>
			</div>
		</main>
	);
}
