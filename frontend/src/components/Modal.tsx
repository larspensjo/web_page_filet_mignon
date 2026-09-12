import { type ReactNode, useEffect, useRef } from "react";

type ModalProps = {
	title: string;
	titleId: string;
	onClose: () => void;
	children: ReactNode;
};

/**
 * Minimal dialog shell: backdrop, labelled dialog, initial focus, and Escape
 * closing. Visibility is owned by the caller; nothing here is core state.
 */
export function Modal({ title, titleId, onClose, children }: ModalProps) {
	const dialog = useRef<HTMLDivElement>(null);

	useEffect(() => {
		const previous = document.activeElement;
		const first = dialog.current?.querySelector<HTMLElement>(
			"input, textarea, button, [tabindex]",
		);
		(first ?? dialog.current)?.focus();
		return () => {
			if (previous instanceof HTMLElement) previous.focus();
		};
	}, []);

	return (
		<div className="modal-backdrop">
			<div
				className="modal"
				role="dialog"
				aria-modal="true"
				aria-labelledby={titleId}
				ref={dialog}
				tabIndex={-1}
				onKeyDown={(event) => {
					if (event.key === "Escape") {
						event.stopPropagation();
						onClose();
					}
				}}
			>
				<h2 id={titleId} className="modal-title">
					{title}
				</h2>
				{children}
			</div>
		</div>
	);
}
