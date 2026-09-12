import { useCallback, useEffect, useRef, useState } from "react";
import { AddUrlModal } from "./components/AddUrlModal";
import { ArchiveModal } from "./components/ArchiveModal";
import { JobList } from "./components/JobList";
import { ReadingPane } from "./components/ReadingPane";
import { RunSurface } from "./components/RunSurface";
import { StatusMeters } from "./components/StatusMeters";
import { JOBS_SEARCH_DEBOUNCE_MS } from "./constants";
import { dispatchIntent } from "./ipc/intent";
import { IPC_SCHEMA_VERSION } from "./ipc/schemaVersion";
import type { ArchiveDialogRequest, JobListMode } from "./ipc/types";
import { useUiCommand } from "./ipc/uiCommand";
import { useProbe } from "./ipc/useProbe";
import { useSnapshot } from "./ipc/useSnapshot";
import "./styles/base.css";
import "./styles/tokens.css";
import "./styles/workspace.css";
import "./styles/chrome.css";

function isShortcut(event: KeyboardEvent, key: string): boolean {
	return (
		(event.ctrlKey || event.metaKey) &&
		!event.altKey &&
		event.key.toLowerCase() === key
	);
}

export function App() {
	const snapshot = useSnapshot();
	const [searchText, setSearchText] = useState("");
	const [archiveRequest, setArchiveRequest] =
		useState<ArchiveDialogRequest | null>(null);
	const [addUrlOpen, setAddUrlOpen] = useState(false);
	const [searchFocusRequests, setSearchFocusRequests] = useState(0);
	const searchInputRef = useRef<HTMLInputElement>(null);
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
	useUiCommand((command) => setArchiveRequest(command.ShowArchiveDialog));
	const list = snapshot?.view.desktop_job_list;
	const mode: JobListMode = list?.mode ?? "SinceCheckpoint";
	const query = list?.query ?? "";
	useEffect(() => {
		cancelPendingSearchDispatch();
		setSearchText(mode === "SinceCheckpoint" ? query : "");
		return cancelPendingSearchDispatch;
	}, [mode, query, cancelPendingSearchDispatch]);

	// Focus is frontend-local: after RevealJobsSearch is dispatched, the
	// search box is focused once it exists in the DOM.
	// biome-ignore lint/correctness/useExhaustiveDependencies: mode re-runs the focus once the input mounts.
	useEffect(() => {
		if (searchFocusRequests === 0) return;
		const input = searchInputRef.current;
		if (!input) return;
		input.focus();
		input.select();
	}, [searchFocusRequests, mode]);

	const clearSearch = useCallback(() => {
		cancelPendingSearchDispatch();
		setSearchText("");
		void dispatchIntent({ type: "ClearJobsSearch" });
	}, [cancelPendingSearchDispatch]);

	const modalOpen = archiveRequest !== null || addUrlOpen;
	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.key === "Escape") {
				if (archiveRequest) {
					void dispatchIntent({ type: "CancelArchiveDialog" });
					setArchiveRequest(null);
				} else if (addUrlOpen) setAddUrlOpen(false);
				else if (searchText) clearSearch();
				return;
			}
			if (modalOpen) return;
			if (isShortcut(event, "l")) {
				event.preventDefault();
				setAddUrlOpen(true);
			} else if (isShortcut(event, "f")) {
				event.preventDefault();
				void dispatchIntent({ type: "RevealJobsSearch" });
				setSearchFocusRequests((count) => count + 1);
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [archiveRequest, addUrlOpen, modalOpen, searchText, clearSearch]);

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
	const selected = list?.selected_job;
	const selectedCandidate = selected
		? (candidates.find(
				(candidate) =>
					candidate.job_id === selected.job_id && candidate.signal_key !== "",
			) ?? null)
		: null;

	return (
		<main>
			<header>
				<h1>Harvester</h1>
				<div className="header-chrome">
					{snapshot && <StatusMeters view={snapshot.view} />}
					{snapshot?.view.checkpoint_status_message && (
						<span className="checkpoint-status" role="status">
							{snapshot.view.checkpoint_status_message}
						</span>
					)}
					<div className="header-actions">
						<button
							className="ghost-button"
							type="button"
							title="Ctrl+L"
							onClick={() => setAddUrlOpen(true)}
						>
							Add URLs
						</button>
						<button
							type="button"
							disabled={!snapshot}
							onClick={() => void dispatchIntent({ type: "OpenArchiveDialog" })}
						>
							Archive…
						</button>
					</div>
				</div>
			</header>
			{snapshot && <RunSurface view={snapshot.view} />}
			<div className="workspace">
				<JobList
					list={list}
					candidates={candidates}
					searchText={searchText}
					searchInputRef={searchInputRef}
					onSearchChange={changeSearch}
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
					selected={selected}
					summary={snapshot?.view.right_pane.summary_markdown}
					candidate={selectedCandidate}
					onToggleExclusion={(signalKey) =>
						void dispatchIntent({
							type: "ToggleSignalCandidateExclusion",
							payload: { signal_key: signalKey },
						})
					}
				/>
			</div>
			{archiveRequest && (
				<ArchiveModal
					key={archiveRequest.request_id}
					request={archiveRequest}
					partialCoverage={snapshot?.view.archive_partial_coverage}
					onClose={() => setArchiveRequest(null)}
				/>
			)}
			{addUrlOpen && (
				<AddUrlModal
					lastPasteStats={snapshot?.view.last_paste_stats}
					onClose={() => setAddUrlOpen(false)}
				/>
			)}
		</main>
	);
}
