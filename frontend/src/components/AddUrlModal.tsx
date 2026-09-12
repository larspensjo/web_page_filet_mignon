import {
	type ClipboardEvent,
	type FormEvent,
	type KeyboardEvent,
	useState,
} from "react";
import { dispatchIntent } from "../ipc/intent";
import type { LastPasteStats } from "../ipc/types";
import { Modal } from "./Modal";

type AddUrlModalProps = {
	lastPasteStats: LastPasteStats | null | undefined;
	onClose: () => void;
};

function submitUrls(text: string) {
	void dispatchIntent({ type: "SetUrlInput", payload: { text } });
	void dispatchIntent({ type: "SubmitUrls" });
}

/**
 * Add URL: paste submits immediately, as the Win32 drop box did; typed text
 * submits on the button or Ctrl+Enter. Core parses and deduplicates.
 */
export function AddUrlModal({ lastPasteStats, onClose }: AddUrlModalProps) {
	const [text, setText] = useState("");
	const [submitted, setSubmitted] = useState(false);

	const submit = (value: string) => {
		if (!value.trim()) return;
		submitUrls(value);
		setSubmitted(true);
		setText("");
	};
	const onSubmit = (event: FormEvent) => {
		event.preventDefault();
		submit(text);
	};
	const onPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
		const pasted = event.clipboardData.getData("text");
		if (!pasted.trim()) return;
		event.preventDefault();
		submit(text ? `${text}\n${pasted}` : pasted);
	};
	const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
		if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
			event.preventDefault();
			submit(text);
		}
	};

	return (
		<Modal title="Add URLs" titleId="add-url-modal-title" onClose={onClose}>
			<form className="modal-form" onSubmit={onSubmit}>
				<label className="modal-field">
					<span>One URL per line. Pasting submits immediately.</span>
					<textarea
						aria-label="URLs to add"
						rows={6}
						value={text}
						onChange={(event) => setText(event.target.value)}
						onPaste={onPaste}
						onKeyDown={onKeyDown}
					/>
				</label>
				{submitted && lastPasteStats && (
					<p className="modal-note" role="status">
						Queued {lastPasteStats.enqueued}, skipped {lastPasteStats.skipped}.
					</p>
				)}
				<div className="modal-actions">
					<button className="ghost-button" type="button" onClick={onClose}>
						Close
					</button>
					<button type="submit" disabled={!text.trim()}>
						Add
					</button>
				</div>
			</form>
		</Modal>
	);
}
