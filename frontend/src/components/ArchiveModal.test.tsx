import showArchiveDialog from "@fixtures/ui_commands/show_archive_dialog.json";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { dispatchIntent } from "../ipc/intent";
import type { ArchiveDialogRequest, UiCommand } from "../ipc/types";
import { ArchiveModal, isSafeArchiveBasename } from "./ArchiveModal";

vi.mock("../ipc/intent", () => ({ dispatchIntent: vi.fn() }));

const fixture = (showArchiveDialog as unknown as UiCommand).ShowArchiveDialog;

function renderModal(overrides: Partial<ArchiveDialogRequest> = {}) {
	const onClose = vi.fn();
	render(
		<ArchiveModal
			request={{ ...fixture, ...overrides }}
			partialCoverage={null}
			onClose={onClose}
		/>,
	);
	return onClose;
}

describe("ArchiveModal", () => {
	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it("pins the channel-2 payload shape the modal depends on", () => {
		expect(fixture).toMatchObject({
			request_id: expect.any(Number),
			article_count: expect.any(Number),
			default_basename: expect.any(String),
			default_file_exists: expect.any(Boolean),
			export_dir: expect.any(String),
			pending_pre_triage_count: expect.any(Number),
			token_estimates: {
				full_tokens: expect.any(Number),
				summary_tokens: expect.any(Number),
				summary_coverage: expect.any(Number),
			},
			signal_candidate_default: expect.any(String),
			signal_candidate_count: expect.any(Number),
			signal_candidate_token_estimates: expect.any(Object),
		});
		expect(fixture).toHaveProperty("since_utc");
	});

	it("renders the fixture's counts, estimates and overwrite warning", () => {
		renderModal();
		expect(
			screen.getByRole("dialog", { name: "Archive export" }),
		).toBeVisible();
		expect(screen.getByText("2 in total")).toBeInTheDocument();
		expect(screen.getByText("~84 tokens (2 articles)")).toBeInTheDocument();
		expect(
			screen.getByText("~84 tokens (0/2 with summaries)"),
		).toBeInTheDocument();
		expect(
			screen.getByText(/output\/archive\.md already exists/),
		).toBeInTheDocument();
		expect(
			screen.getByText(
				"No candidates settled yet - exporting the full triage set.",
			),
		).toBeInTheDocument();
		expect(
			screen.getByRole("checkbox", { name: "Use signal-candidate selection" }),
		).toBeDisabled();
	});

	it("submits the fixture's request id with the form values and closes", () => {
		const onClose = renderModal();
		fireEvent.change(screen.getByLabelText("Output file"), {
			target: { value: "weekly.md" },
		});
		fireEvent.click(
			screen.getByRole("checkbox", {
				name: "Set checkpoint to now after export",
			}),
		);
		fireEvent.click(screen.getByRole("button", { name: /^Export/ }));
		expect(dispatchIntent).toHaveBeenCalledWith({
			type: "SubmitArchiveDialog",
			payload: {
				request_id: fixture.request_id,
				basename: "weekly.md",
				set_checkpoint: false,
				use_summaries: true,
				use_signal_candidates: false,
			},
		});
		expect(onClose).toHaveBeenCalled();
	});

	it("refuses to export with an unsafe basename or an empty corpus", () => {
		renderModal();
		const exportButton = screen.getByRole("button", { name: /^Export/ });
		fireEvent.change(screen.getByLabelText("Output file"), {
			target: { value: "../escape.md" },
		});
		expect(exportButton).toBeDisabled();
		expect(
			screen.getByText("Enter a file name without path separators."),
		).toBeInTheDocument();
		fireEvent.submit(exportButton.closest("form") as HTMLFormElement);
		expect(dispatchIntent).not.toHaveBeenCalled();
		cleanup();

		renderModal({ article_count: 0 });
		expect(screen.getByRole("button", { name: /^Export/ })).toBeDisabled();
		expect(
			screen.getByText("No articles match the current filter."),
		).toBeInTheDocument();
	});

	it("defaults the candidate selection on when core says all candidates settled", () => {
		renderModal({
			signal_candidate_default: "OnAllSettled",
			signal_candidate_count: 3,
			signal_candidate_token_estimates: {
				full_tokens: 3000,
				summary_tokens: 1200,
				summary_coverage: 3,
			},
		});
		expect(
			screen.getByRole("checkbox", { name: "Use signal-candidate selection" }),
		).toBeChecked();
		expect(
			screen.getByText("3 candidates selected (threshold + dedup)"),
		).toBeInTheDocument();
		expect(
			screen.getByText("~1k tokens (3/3 with summaries)"),
		).toBeInTheDocument();
		fireEvent.click(screen.getByRole("button", { name: "Export 3 articles" }));
		expect(dispatchIntent).toHaveBeenCalledWith({
			type: "SubmitArchiveDialog",
			payload: expect.objectContaining({ use_signal_candidates: true }),
		});
	});

	it("shows the pending-triage and partial-coverage lines", () => {
		render(
			<ArchiveModal
				request={{ ...fixture, pending_pre_triage_count: 4 }}
				partialCoverage={{ triaged: 5, actionable_total: 9 }}
				onClose={vi.fn()}
			/>,
		);
		expect(
			screen.getByText(/4 articles await triage and are not included/),
		).toBeInTheDocument();
		expect(
			screen.getByText(/5 of 9 triaged - run triage to export the rest/),
		).toBeInTheDocument();
	});

	it("cancel and Escape dispatch CancelArchiveDialog and close", () => {
		const onClose = renderModal();
		fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
		expect(dispatchIntent).toHaveBeenCalledWith({
			type: "CancelArchiveDialog",
		});
		expect(onClose).toHaveBeenCalledTimes(1);
		fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
		expect(onClose).toHaveBeenCalledTimes(2);
	});

	it("mirrors core's basename safety rule", () => {
		expect(isSafeArchiveBasename("archive.md")).toBe(true);
		for (const bad of ["", ".", "..", "a/b.md", "a\\b.md", "C:x.md", "a\0.md"])
			expect(isSafeArchiveBasename(bad)).toBe(false);
	});
});
