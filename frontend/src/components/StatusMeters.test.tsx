import withCorpus from "@fixtures/snapshots/idle_with_corpus.json";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { SnapshotEnvelope } from "../ipc/types";
import { quotaTone, StatusMeters, tokenMeterTone } from "./StatusMeters";

const corpus = withCorpus as unknown as SnapshotEnvelope;

describe("StatusMeters", () => {
	afterEach(cleanup);

	it("pins the meter fields the chrome reads from every snapshot", () => {
		expect(corpus.view).toMatchObject({
			archive_token_estimate: expect.any(Number),
			token_limit: expect.any(Number),
			archive_filtered_count: expect.any(Number),
			raw_unprocessed_count: expect.any(Number),
			llm_quota: {
				label: expect.any(String),
				severity: expect.any(String),
			},
		});
		expect(corpus.view).toHaveProperty("archive_partial_coverage");
	});

	it("renders one labelled archive meter and one quota meter from the fixture", () => {
		render(<StatusMeters view={corpus.view} />);
		expect(screen.getByText("Archive 84 / 100k tokens")).toBeInTheDocument();
		expect(screen.getByText("2 filtered · 2 raw")).toBeInTheDocument();
		expect(screen.getByText("LLM calls 0 / unlimited")).toBeInTheDocument();
		const bars = screen.getAllByRole("progressbar");
		expect(bars).toHaveLength(1);
		expect(bars[0]).toHaveAttribute("aria-valuenow", "0");
		expect(document.querySelectorAll(".meter--muted")).toHaveLength(2);
	});

	it("prefers the partial-coverage line and escalates near the limit", () => {
		render(
			<StatusMeters
				view={{
					...corpus.view,
					archive_token_estimate: 95_000,
					archive_partial_coverage: { triaged: 3, actionable_total: 8 },
					llm_quota: {
						label: "LLM calls 95 / 100",
						used: 95,
						limit: 100,
						percent: 95,
						severity: "Danger",
					},
				}}
			/>,
		);
		expect(screen.getByText("3 of 8 triaged")).toBeInTheDocument();
		expect(document.querySelector('[data-meter="archive-tokens"]')).toHaveClass(
			"meter--attention",
		);
		expect(document.querySelector('[data-meter="llm-quota"]')).toHaveClass(
			"meter--warning",
		);
		expect(screen.getAllByRole("progressbar")).toHaveLength(2);
	});

	it("maps thresholds to tones", () => {
		expect(tokenMeterTone(0)).toBe("muted");
		expect(tokenMeterTone(79.9)).toBe("muted");
		expect(tokenMeterTone(80)).toBe("attention");
		expect(tokenMeterTone(100)).toBe("warning");
		expect(quotaTone("Normal")).toBe("muted");
		expect(quotaTone("Unavailable")).toBe("muted");
		expect(quotaTone("Warning")).toBe("attention");
		expect(quotaTone("Danger")).toBe("warning");
		expect(quotaTone("Exhausted")).toBe("warning");
	});
});
