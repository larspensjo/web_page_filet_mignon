export type JobListMode = "All" | "Results" | "SinceCheckpoint";

export type SnapshotEnvelope = {
	generation: number;
	schema_version: number;
	view: {
		jobs: JobRowView[];
		job_list_mode?: JobListMode;
	} & Record<string, unknown>;
	fatal_message: string | null;
};
export type JobRowView = {
	job_id: number;
	url: string;
	stage: string;
	outcome: string | null;
	tokens: number | null;
	bytes: number | null;
	has_summary: boolean;
	summary_title: string | null;
	is_since_checkpoint: boolean;
};
