import type { CSSProperties } from "react";
import type { SnapshotEnvelope } from "../ipc/types";
import { formatCompactTokens } from "./jobPresentation";

type MeterView = SnapshotEnvelope["view"];

const TOKEN_ATTENTION_PERCENT = 80;

type Tone = "muted" | "attention" | "warning";

function Meter({
	label,
	detail,
	percent,
	tone,
	name,
}: {
	label: string;
	detail: string | null;
	percent: number | null;
	tone: Tone;
	name: string;
}) {
	return (
		<div className={`meter meter--${tone}`} data-meter={name}>
			<div className="meter-text">
				<span className="meter-label">{label}</span>
				{detail && <span className="meter-detail">{detail}</span>}
			</div>
			{percent !== null && (
				<div
					className="meter-bar"
					role="progressbar"
					aria-label={label}
					aria-valuemin={0}
					aria-valuemax={100}
					aria-valuenow={Math.round(percent)}
					style={{ "--meter-fill": `${percent}%` } as CSSProperties}
				/>
			)}
		</div>
	);
}

export function tokenMeterTone(percent: number): Tone {
	if (percent >= 100) return "warning";
	if (percent >= TOKEN_ATTENTION_PERCENT) return "attention";
	return "muted";
}

export function quotaTone(severity: MeterView["llm_quota"]["severity"]): Tone {
	switch (severity) {
		case "Normal":
		case "Unavailable":
			return "muted";
		case "Warning":
			return "attention";
		case "Danger":
		case "Exhausted":
			return "warning";
	}
}

/** The spec's single labelled meters: muted by default, vivid only near thresholds. */
export function StatusMeters({ view }: { view: MeterView }) {
	const limit = view.token_limit;
	const percent =
		limit > 0 ? Math.min(100, (view.archive_token_estimate / limit) * 100) : 0;
	const coverage = view.archive_partial_coverage;
	const detail = coverage
		? `${coverage.triaged} of ${coverage.actionable_total} triaged`
		: `${view.archive_filtered_count} filtered · ${view.raw_unprocessed_count} raw`;
	const quota = view.llm_quota;

	return (
		<div className="status-meters">
			<Meter
				name="archive-tokens"
				label={`Archive ${formatCompactTokens(view.archive_token_estimate)} / ${formatCompactTokens(limit)} tokens`}
				detail={detail}
				percent={percent}
				tone={tokenMeterTone(percent)}
			/>
			<Meter
				name="llm-quota"
				label={quota.label}
				detail={null}
				percent={quota.percent}
				tone={quotaTone(quota.severity)}
			/>
		</div>
	);
}
