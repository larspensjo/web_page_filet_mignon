import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { type BodyState, useBody } from "../ipc/body";
import { dispatchIntent } from "../ipc/intent";
import type { BodyRef, SelectedJobView } from "../ipc/types";
import {
	formatFetchedTime,
	jobTitle,
	priorityClass,
	visibilityExplanation,
} from "./jobPresentation";

type ReadingPaneProps = {
	selected: SelectedJobView | null | undefined;
	summary: BodyRef | null | undefined;
};

function BodyContent({
	state,
	selected,
}: {
	state: BodyState;
	selected: SelectedJobView;
}) {
	if (state.kind === "unavailable")
		return <p className="empty-state">No summary available.</p>;
	if (state.kind === "loading")
		return <p className="empty-state">Loading article…</p>;
	if (state.kind === "changed")
		return <p className="empty-state">Article content changed; refreshing…</p>;
	if (state.kind === "error")
		return <p className="empty-state">Article content is unavailable.</p>;
	return <MarkdownBody selected={selected} text={state.text} />;
}

function MarkdownBody({
	text,
	selected,
}: {
	text: string;
	selected: SelectedJobView;
}) {
	return (
		<div className="markdown-body">
			<ReactMarkdown
				components={{
					a: ({ children, href }) => {
						const link = selected.links.find(
							(candidate) => candidate.url === href,
						);
						if (!link)
							return <span className="unavailable-link">{children}</span>;
						return (
							<button
								className="markdown-link"
								type="button"
								onClick={() =>
									void dispatchIntent({
										type: "OpenExtractedLink",
										payload: {
											job_id: selected.job_id,
											link_index: link.index,
										},
									})
								}
							>
								{children}
							</button>
						);
					},
					img: ({ alt }) => (
						<span className="unavailable-image">
							{alt ?? "Image unavailable"}
						</span>
					),
				}}
				remarkPlugins={[
					remarkGfm,
					suppressDuplicateLeadingHeading(jobTitle(selected)),
				]}
				skipHtml
			>
				{text}
			</ReactMarkdown>
		</div>
	);
}

type MarkdownNode = {
	type: string;
	value?: string;
	alt?: string;
	children?: MarkdownNode[];
};

type MarkdownRoot = {
	children: MarkdownNode[];
};

function markdownText(node: MarkdownNode): string {
	return (
		node.value ??
		node.alt ??
		node.children?.map((child) => markdownText(child)).join("") ??
		""
	);
}

function suppressDuplicateLeadingHeading(title: string) {
	return () => (root: MarkdownRoot) => {
		const first = root.children[0];
		if (first?.type === "heading" && markdownText(first) === title)
			root.children.shift();
	};
}

function AnnotationBand({ selected }: { selected: SelectedJobView }) {
	const annotation = selected.triage_annotation;
	if (!annotation) return null;
	return (
		<div className="annotation-band">
			<span className={`priority-badge ${priorityClass(annotation.priority)}`}>
				P{annotation.priority}
			</span>
			<span className="category-label">{annotation.category}</span>
			{annotation.tags.map((tag) => (
				<span className="pill" key={tag}>
					{tag}
				</span>
			))}
		</div>
	);
}

export function ReadingPane({ selected, summary }: ReadingPaneProps) {
	const summaryBody = useBody(summary);
	if (!selected)
		return (
			<section className="reading-pane" aria-label="Reading pane">
				<p className="empty-state">Select an article to read its summary.</p>
			</section>
		);

	const sourceDomain = domain(selected.url);
	return (
		<section className="reading-pane" aria-labelledby="reading-title">
			<header className="document-header">
				<div>
					<h2 id="reading-title">{jobTitle(selected)}</h2>
					<button
						className="source-link"
						type="button"
						aria-label={`Open ${sourceDomain} in browser`}
						onClick={() =>
							void dispatchIntent({ type: "OpenSelectedInBrowser" })
						}
					>
						{sourceDomain}
					</button>
					{selected.list_visibility !== "Visible" && (
						<p className="selection-visibility">
							{visibilityExplanation(selected.list_visibility)}
						</p>
					)}
				</div>
				<div className="document-meta">
					{selected.tokens !== null && (
						<span className="pill">{selected.tokens} tokens</span>
					)}
					{selected.fetched_utc && (
						<span className="pill">
							{formatFetchedTime(selected.fetched_utc)}
						</span>
					)}
				</div>
			</header>
			<AnnotationBand selected={selected} />
			<div className="reading-body">
				<BodyContent selected={selected} state={summaryBody} />
			</div>
		</section>
	);
}

function domain(url: string): string {
	try {
		return new URL(url).hostname;
	} catch {
		return url;
	}
}
