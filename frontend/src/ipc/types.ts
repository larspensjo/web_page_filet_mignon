export type SnapshotEnvelope = {
	generation: number;
	schema_version: number;
	view: { jobs: JobRowView[] } & Record<string, unknown>;
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
};
