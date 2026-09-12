import { type FormEvent, useState } from "react";
import { dispatchIntent } from "../ipc/intent";
import type {
	ArchiveDialogRequest,
	ArchivePartialCoverageView,
	ArchiveTokenEstimates,
} from "../ipc/types";
import { formatCompactTokens } from "./jobPresentation";
import { Modal } from "./Modal";

type ArchiveModalProps = {
	request: ArchiveDialogRequest;
	partialCoverage: ArchivePartialCoverageView | null | undefined;
	onClose: () => void;
};

/** Mirrors core's `is_safe_archive_basename`; core still rejects on its own. */
export function isSafeArchiveBasename(name: string): boolean {
	if (!name || name === "." || name === "..") return false;
	if (/[/\\\0]/.test(name)) return false;
	return !/^[A-Za-z]:/.test(name);
}

function formatSince(sinceUtc: string): string {
	const since = new Date(sinceUtc);
	if (Number.isNaN(since.getTime())) return sinceUtc;
	const days = Math.max(
		0,
		Math.floor((Date.now() - since.getTime()) / 86_400_000),
	);
	return `${since.toISOString().slice(0, 10)} (${days} days ago)`;
}

function estimateLine(
	estimates: ArchiveTokenEstimates,
	count: number,
	mode: "full" | "summary",
): string {
	if (mode === "full")
		return `~${formatCompactTokens(estimates.full_tokens)} tokens (${count} articles)`;
	return `~${formatCompactTokens(estimates.summary_tokens)} tokens (${estimates.summary_coverage}/${count} with summaries)`;
}

function candidateNotice(request: ArchiveDialogRequest): {
	text: string;
	warning: boolean;
} {
	const count = request.signal_candidate_count;
	switch (request.signal_candidate_default) {
		case "OnAllSettled":
			return {
				text: `${count} candidates selected (threshold + dedup)`,
				warning: false,
			};
		case "OffPartial":
			return {
				text: `Scoring in progress (${request.signal_candidate_scoring_done}/${request.signal_candidate_scoring_total}). Turn on to export only settled candidates (${count} selected).`,
				warning: true,
			};
		case "OffEmpty":
			return {
				text: `No candidates above threshold (${request.signal_candidate_scoring_total} scored). Exporting the full triage set.`,
				warning: true,
			};
		case "OffDisabled":
			return {
				text: "No candidates settled yet - exporting the full triage set.",
				warning: true,
			};
	}
}

export function ArchiveModal({
	request,
	partialCoverage,
	onClose,
}: ArchiveModalProps) {
	const [basename, setBasename] = useState(request.default_basename);
	const [useSummaries, setUseSummaries] = useState(true);
	const [useSignalCandidates, setUseSignalCandidates] = useState(
		request.signal_candidate_default === "OnAllSettled",
	);
	const [setCheckpoint, setSetCheckpoint] = useState(true);

	const basenameValid = isSafeArchiveBasename(basename);
	const candidatesAvailable = request.signal_candidate_count > 0;
	const candidatesOn = useSignalCandidates && candidatesAvailable;
	const canExport = request.article_count > 0 && basenameValid;
	const overwrite =
		request.default_file_exists && basename === request.default_basename;
	const notice = candidateNotice(request);
	const exportCount = candidatesOn
		? request.signal_candidate_count
		: request.article_count;

	const cancel = () => {
		void dispatchIntent({ type: "CancelArchiveDialog" });
		onClose();
	};
	const submit = (event: FormEvent) => {
		event.preventDefault();
		if (!canExport) return;
		void dispatchIntent({
			type: "SubmitArchiveDialog",
			payload: {
				request_id: request.request_id,
				basename,
				set_checkpoint: setCheckpoint,
				use_summaries: useSummaries,
				use_signal_candidates: candidatesOn,
			},
		});
		onClose();
	};

	return (
		<Modal
			title="Archive export"
			titleId="archive-modal-title"
			onClose={cancel}
		>
			<form className="modal-form" onSubmit={submit}>
				<dl className="modal-facts">
					<dt>Articles</dt>
					<dd>
						{request.article_count}{" "}
						{request.since_utc ? "since checkpoint" : "in total"}
					</dd>
					{request.since_utc && (
						<>
							<dt>Checkpoint</dt>
							<dd>{formatSince(request.since_utc)}</dd>
						</>
					)}
					<dt>Full archive</dt>
					<dd>
						{estimateLine(
							request.token_estimates,
							request.article_count,
							"full",
						)}
					</dd>
					<dt>Summary archive</dt>
					<dd>
						{estimateLine(
							request.token_estimates,
							request.article_count,
							"summary",
						)}
					</dd>
					{candidatesAvailable && (
						<>
							<dt>Candidates, full</dt>
							<dd>
								{estimateLine(
									request.signal_candidate_token_estimates,
									request.signal_candidate_count,
									"full",
								)}
							</dd>
							<dt>Candidates, summary</dt>
							<dd>
								{estimateLine(
									request.signal_candidate_token_estimates,
									request.signal_candidate_count,
									"summary",
								)}
							</dd>
						</>
					)}
				</dl>

				<p
					className={`modal-note${notice.warning ? " modal-note--warning" : ""}`}
				>
					{notice.text}
				</p>
				{partialCoverage && (
					<p className="modal-note modal-note--warning">
						{partialCoverage.triaged} of {partialCoverage.actionable_total}{" "}
						triaged - run triage to export the rest.
					</p>
				)}
				{request.article_count === 0 && (
					<p className="modal-note modal-note--warning">
						No articles match the current filter.
					</p>
				)}
				{request.pending_pre_triage_count > 0 && (
					<p className="modal-note modal-note--warning">
						{request.pending_pre_triage_count}{" "}
						{request.pending_pre_triage_count === 1
							? "article awaits triage and is"
							: "articles await triage and are"}{" "}
						not included in this export.
					</p>
				)}

				<label className="modal-field">
					<span>Output file</span>
					<input
						value={basename}
						aria-invalid={!basenameValid}
						onChange={(event) => setBasename(event.target.value)}
					/>
				</label>
				{!basenameValid && (
					<p className="modal-note modal-note--warning">
						Enter a file name without path separators.
					</p>
				)}
				{basenameValid && overwrite && (
					<p className="modal-note modal-note--warning">
						{request.export_dir}/{basename} already exists and will be
						overwritten.
					</p>
				)}

				<label className="modal-check">
					<input
						type="checkbox"
						checked={useSummaries}
						onChange={(event) => setUseSummaries(event.target.checked)}
					/>
					<span>Use summaries (recommended)</span>
				</label>
				<label className="modal-check">
					<input
						type="checkbox"
						checked={candidatesOn}
						disabled={!candidatesAvailable}
						onChange={(event) => setUseSignalCandidates(event.target.checked)}
					/>
					<span>Use signal-candidate selection</span>
				</label>
				<label className="modal-check">
					<input
						type="checkbox"
						checked={setCheckpoint}
						onChange={(event) => setSetCheckpoint(event.target.checked)}
					/>
					<span>Set checkpoint to now after export</span>
				</label>

				<div className="modal-actions">
					<button className="ghost-button" type="button" onClick={cancel}>
						Cancel
					</button>
					<button type="submit" disabled={!canExport}>
						Export {exportCount} {exportCount === 1 ? "article" : "articles"}
					</button>
				</div>
			</form>
		</Modal>
	);
}
