import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { dispatchIntent } from "../ipc/intent";
import { AddUrlModal } from "./AddUrlModal";

vi.mock("../ipc/intent", () => ({ dispatchIntent: vi.fn() }));

const submitCalls = (text: string) => [
	[{ type: "SetUrlInput", payload: { text } }],
	[{ type: "SubmitUrls" }],
];

describe("AddUrlModal", () => {
	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it("submits pasted text immediately and clears the box", () => {
		render(<AddUrlModal lastPasteStats={null} onClose={vi.fn()} />);
		const box = screen.getByRole("textbox", { name: "URLs to add" });
		fireEvent.paste(box, {
			clipboardData: {
				getData: () => "https://a.invalid\nhttps://b.invalid",
			},
		});
		expect(vi.mocked(dispatchIntent).mock.calls).toEqual(
			submitCalls("https://a.invalid\nhttps://b.invalid"),
		);
		expect(box).toHaveValue("");
	});

	it("submits typed text on Add and on Ctrl+Enter", () => {
		render(<AddUrlModal lastPasteStats={null} onClose={vi.fn()} />);
		const box = screen.getByRole("textbox", { name: "URLs to add" });
		const add = screen.getByRole("button", { name: "Add" });
		expect(add).toBeDisabled();
		fireEvent.change(box, { target: { value: "https://typed.invalid" } });
		fireEvent.click(add);
		expect(vi.mocked(dispatchIntent).mock.calls).toEqual(
			submitCalls("https://typed.invalid"),
		);
		fireEvent.change(box, { target: { value: "https://keyed.invalid" } });
		fireEvent.keyDown(box, { key: "Enter", ctrlKey: true });
		expect(vi.mocked(dispatchIntent).mock.calls.slice(2)).toEqual(
			submitCalls("https://keyed.invalid"),
		);
	});

	it("ignores blank submissions", () => {
		render(<AddUrlModal lastPasteStats={null} onClose={vi.fn()} />);
		const box = screen.getByRole("textbox", { name: "URLs to add" });
		fireEvent.paste(box, { clipboardData: { getData: () => "  \n" } });
		fireEvent.change(box, { target: { value: "   " } });
		fireEvent.keyDown(box, { key: "Enter", ctrlKey: true });
		expect(dispatchIntent).not.toHaveBeenCalled();
	});

	it("shows core's paste statistics only after a submission here", () => {
		const { rerender } = render(
			<AddUrlModal
				lastPasteStats={{ enqueued: 9, skipped: 9 }}
				onClose={vi.fn()}
			/>,
		);
		expect(screen.queryByRole("status")).not.toBeInTheDocument();
		fireEvent.paste(screen.getByRole("textbox", { name: "URLs to add" }), {
			clipboardData: { getData: () => "https://a.invalid" },
		});
		rerender(
			<AddUrlModal
				lastPasteStats={{ enqueued: 1, skipped: 0 }}
				onClose={vi.fn()}
			/>,
		);
		expect(screen.getByRole("status")).toHaveTextContent(
			"Queued 1, skipped 0.",
		);
	});

	it("closes on the Close button and on Escape", () => {
		const onClose = vi.fn();
		render(<AddUrlModal lastPasteStats={null} onClose={onClose} />);
		fireEvent.click(screen.getByRole("button", { name: "Close" }));
		fireEvent.keyDown(screen.getByRole("textbox", { name: "URLs to add" }), {
			key: "Escape",
		});
		expect(onClose).toHaveBeenCalledTimes(2);
	});
});
