export type JobListMode = "Results" | "SinceCheckpoint" | "Last24Hours";
export type Stage =
	| "Queued"
	| "Downloading"
	| "Sanitizing"
	| "Converting"
	| "Tokenizing"
	| "Writing"
	| "Done";
export type JobResultKind = "Success" | { Failed: { reason: string } };
export type JobOrigin = "Direct" | { Indirect: { source_job_id: number } };
export type SelectedJobVisibility =
	| "Visible"
	| "OutsideScope"
	| "QueryMismatch"
	| "Capped";
export type FilterReason =
	| "BlockedHost"
	| "VerySmallContent"
	| "PaywallShellTitle"
	| "SmallMediumContent"
	| "BoilerplateDensity"
	| "HighLinkDensity"
	| "StubPhraseLowContent";
export type JobFilterStatus =
	| { HardExcluded: { reasons: FilterReason[] } }
	| { ReviewNeeded: { reasons: FilterReason[] } }
	| "AutoIncluded";

export type BodyKey =
	| "Preview"
	| "TriageMarkdown"
	| "SummaryMarkdown"
	| "PollStatsMarkdown";

export type BodyRef = {
	key: BodyKey;
	content_hash: string;
	byte_len: number;
};

export type BodyResponse = {
	content_hash: string;
	text: string;
};

export type TriageAnnotationView = {
	priority: number;
	category: string;
	tags: string[];
};

export type JobListRowView = {
	job_id: number;
	url: string;
	stage: Stage;
	outcome: JobResultKind | null;
	tokens: number | null;
	bytes: number | null;
	link_count: number;
	downloaded_link_count: number;
	origin: JobOrigin;
	triage_annotation: TriageAnnotationView | null;
	has_summary: boolean;
	summary_title: string | null;
	summary_tokens: number | null;
	filter_status: JobFilterStatus | null;
	has_analysis: boolean;
	is_since_checkpoint: boolean;
	fetched_utc: string | null;
};

export type SelectedJobView = {
	job_id: number;
	url: string;
	summary_title: string | null;
	stage: Stage;
	outcome: JobResultKind | null;
	tokens: number | null;
	bytes: number | null;
	origin: JobOrigin;
	triage_annotation: TriageAnnotationView | null;
	has_summary: boolean;
	summary_tokens: number | null;
	filter_status: JobFilterStatus | null;
	fetched_utc: string | null;
	list_visibility: SelectedJobVisibility;
	links: ExtractedLinkView[];
};

export type ExtractedLinkView = {
	index: number;
	url: string;
	label: string;
	kind: "Hyperlink" | "Image" | "Email";
	download_state:
		| "NotDownloaded"
		| "Downloading"
		| { Downloaded: { path: string } }
		| { Failed: { error: string } };
	age_suspect: boolean;
};

export type DesktopJobListView = {
	mode: JobListMode;
	query: string;
	rows: JobListRowView[];
	selected_job: SelectedJobView | null;
	scoped_count: number;
	visible_count: number;
	truncated: boolean;
	hidden_without_fetch_time: number;
};

export type SignalCandidateRow = {
	job_id: number;
	url: string;
	score: number;
	score_band: "High" | "Mid" | "Low";
	source_tier: "Tier1" | "Tier2" | "Tier3";
	gist_truncated: string;
	themes: string[];
	dupes_count: number;
	state_label: "Scoring" | "Scored" | { Failed: { reason: string } };
	signal_key: string;
	outcome:
		| "Selected"
		| { Deduplicated: { kept_gist: string } }
		| "BelowThreshold"
		| "Excluded"
		| null;
};

export type StageProgress = {
	stage:
		| "ScanningSources"
		| "DownloadingArticles"
		| "LoadingArticles"
		| "Triaging"
		| "Summarizing"
		| "ScoringSignals";
	status: "Pending" | "Active" | "Done" | "Failed";
	completed: number;
	failed: number;
	total: number;
	started_at_utc: string | null;
	ended_at_utc: string | null;
};

export type ActivityEntry = {
	seq: number;
	url: string;
	title: string | null;
	stage: StageProgress["stage"];
	outcome:
		| "Started"
		| "Succeeded"
		| { Failed: { reason: string } }
		| { Skipped: { reason: string } };
};

export type RunProgressView = {
	stages: StageProgress[];
	run_active: boolean;
	activity: ActivityEntry[];
};

export type RunCompletionNotice = {
	new_result_count: number;
	completed_at_utc: string;
};

export type StopFinishButtonState =
	| "Disabled"
	| { Enabled: { policy: "Finish" | "Immediate" } };

export type WorkspaceView = "Review" | "Trends" | "PollStats" | "Blacklist";

export type ArchivePartialCoverageView = {
	triaged: number;
	actionable_total: number;
};

export type LlmQuotaSeverity =
	| "Normal"
	| "Warning"
	| "Danger"
	| "Exhausted"
	| "Unavailable";

export type LlmQuotaView = {
	label: string;
	used: number;
	limit: number | null;
	percent: number | null;
	severity: LlmQuotaSeverity;
};

export type LastPasteStats = {
	enqueued: number;
	skipped: number;
};

export type ArchiveTokenEstimates = {
	full_tokens: number;
	summary_tokens: number;
	summary_coverage: number;
};

export type SignalCandidateDialogDefault =
	| "OnAllSettled"
	| "OffPartial"
	| "OffDisabled"
	| "OffEmpty";

/** Channel-2 payload of `UiCommand::ShowArchiveDialog`; never part of a snapshot. */
export type ArchiveDialogRequest = {
	request_id: number;
	article_count: number;
	since_utc: string | null;
	default_basename: string;
	default_file_exists: boolean;
	export_dir: string;
	pending_pre_triage_count: number;
	token_estimates: ArchiveTokenEstimates;
	signal_candidate_default: SignalCandidateDialogDefault;
	signal_candidate_count: number;
	signal_candidate_scoring_done: number;
	signal_candidate_scoring_total: number;
	signal_candidate_token_estimates: ArchiveTokenEstimates;
};

export type UiCommand = { ShowArchiveDialog: ArchiveDialogRequest };

export type SnapshotEnvelope = {
	generation: number;
	schema_version: number;
	view: {
		workspace_view: WorkspaceView;
		job_count: number;
		archive_filtered_count: number;
		archive_token_estimate: number;
		token_limit: number;
		archive_partial_coverage: ArchivePartialCoverageView | null;
		raw_unprocessed_count: number;
		llm_quota: LlmQuotaView;
		last_paste_stats: LastPasteStats | null;
		checkpoint_status_message: string | null;
		preview_text: BodyRef | null;
		right_pane: {
			triage_markdown: BodyRef | null;
			summary_markdown: BodyRef | null;
			poll_stats_markdown: BodyRef | null;
		};
		desktop_job_list: DesktopJobListView;
		signal_candidate_rows: SignalCandidateRow[];
		run_progress: RunProgressView;
		run_completion_notice: RunCompletionNotice | null;
		poll_sources_enabled: boolean;
		triage_can_start: boolean;
		summaries_can_start: boolean;
		stop_finish_button: StopFinishButtonState;
	} & Record<string, unknown>;
	fatal_message: string | null;
};
