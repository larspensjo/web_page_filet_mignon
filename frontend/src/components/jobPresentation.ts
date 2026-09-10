import type { JobResultKind, SelectedJobVisibility } from "../ipc/types";

const LOCAL_DATE_TIME = new Intl.DateTimeFormat("en-GB", {
	dateStyle: "medium",
	timeStyle: "short",
});

export function jobTitle(job: {
	summary_title: string | null;
	url: string;
}): string {
	return job.summary_title ?? job.url;
}

export function priorityClass(priority: number): string {
	return priority >= 4 ? "priority-high" : "priority-normal";
}

export function isFailureOutcome(
	outcome: JobResultKind,
): outcome is Extract<JobResultKind, { Failed: { reason: string } }> {
	return typeof outcome !== "string";
}

export function formatFetchedTime(timestamp: string): string {
	const value = new Date(timestamp);
	return Number.isNaN(value.getTime())
		? timestamp
		: LOCAL_DATE_TIME.format(value);
}

export function visibilityExplanation(
	visibility: SelectedJobVisibility,
): string {
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
