export type JobListMode = "Results" | "SinceCheckpoint";
export type SelectedJobVisibility =
	| "Visible"
	| "OutsideScope"
	| "QueryMismatch"
	| "Capped";

export type JobListRowView = {
	job_id: number;
	url: string;
	stage: string;
	outcome: string | null;
	tokens: number | null;
	bytes: number | null;
	link_count: number;
	downloaded_link_count: number;
	origin: unknown;
	triage_annotation: unknown | null;
	has_summary: boolean;
	summary_title: string | null;
	summary_tokens: number | null;
	filter_status: unknown | null;
	has_analysis: boolean;
	is_since_checkpoint: boolean;
	fetched_utc: string | null;
};

export type SelectedJobView = {
	job_id: number;
	url: string;
	summary_title: string | null;
	stage: string;
	outcome: string | null;
	tokens: number | null;
	bytes: number | null;
	origin: unknown;
	triage_annotation: unknown | null;
	has_summary: boolean;
	summary_tokens: number | null;
	filter_status: unknown | null;
	fetched_utc: string | null;
	list_visibility: SelectedJobVisibility;
	links: unknown[];
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
	score_band: string;
	source_tier: string;
	gist_truncated: string;
	themes: string[];
	dupes_count: number;
	state_label: unknown;
	signal_key: string;
	outcome: unknown | null;
};

export type SnapshotEnvelope = {
	generation: number;
	schema_version: number;
	view: {
		job_count: number;
		desktop_job_list: DesktopJobListView;
		signal_candidate_rows: SignalCandidateRow[];
	} & Record<string, unknown>;
	fatal_message: string | null;
};
