import fixture from "@fixtures/snapshots/idle_with_selection.json";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useBody } from "../ipc/body";
import type { SnapshotEnvelope } from "../ipc/types";
import { ReadingPane } from "./ReadingPane";

vi.mock("../ipc/body", () => ({ useBody: vi.fn() }));
vi.mock("../ipc/intent", () => ({ dispatchIntent: vi.fn() }));
afterEach(cleanup);
const selected = (fixture as unknown as SnapshotEnvelope).view.desktop_job_list
	.selected_job;
function show(message?: string) {
	return render(
		<ReadingPane
			selected={selected}
			summary={null}
			aiUnavailableMessage={message}
			candidate={null}
			onToggleExclusion={vi.fn()}
		/>,
	);
}

it("distinguishes an unsummarised current settings article from unavailable AI", () => {
	vi.mocked(useBody).mockReturnValue({ kind: "unavailable" });
	const pane = show();
	expect(
		screen.getByText("Not summarized under the current settings."),
	).toBeInTheDocument();
	pane.unmount();
	show("AI unavailable — API key is missing.");
	expect(
		screen.getByText("AI unavailable — API key is missing."),
	).toBeInTheDocument();
	expect(
		screen.queryByText("Not summarized under the current settings."),
	).not.toBeInTheDocument();
});

it("shows a saved summary when AI is unavailable", () => {
	vi.mocked(useBody).mockReturnValue({
		kind: "ready",
		text: "Saved current summary",
	});
	show("AI unavailable — API key is missing.");
	expect(screen.getByText("Saved current summary")).toBeInTheDocument();
	expect(
		screen.queryByText("AI unavailable — API key is missing."),
	).not.toBeInTheDocument();
});
