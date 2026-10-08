import type { CSSProperties } from "react";
import type { SnapshotEnvelope } from "../ipc/types";
import { formatCompactTokens } from "./jobPresentation";

type MeterView = SnapshotEnvelope["view"];

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

/** Core supplies the archive selection and backlog; the desktop formats their status. */
export function StatusMeters({ view }: { view: MeterView }) {
	const meter = view.archive_meter;
	const percent =
		meter.target > 0
			? Math.min(100, (meter.selected_count / meter.target) * 100)
			: 0;
	let detail: string;
	switch (meter.status) {
		case "Loading":
			detail = "Loading saved results…";
			break;
		case "Unavailable":
			detail = "Saved results unavailable";
			break;
		case "NotScoredYet":
			detail = "Not scored yet";
			break;
		case "Scored":
			detail =
				meter.selected_count > 0
					? `~${formatCompactTokens(meter.token_estimate)} tokens`
					: "None selected yet";
			break;
	}
	if (
		meter.unsettled_count > 0 &&
		meter.status !== "Loading" &&
		meter.status !== "Unavailable"
	) {
		const backlog =
			view.run_state === "Idle" ? "unfinished" : "still processing";
		detail += ` · ${meter.unsettled_count} ${backlog}`;
	}
	const quota = view.llm_quota;

	return (
		<div className="status-meters">
			<Meter
				name="archive-articles"
				label={`${meter.selected_count} / ${meter.target} articles`}
				detail={detail}
				percent={percent}
				tone={meter.selected_count >= meter.target ? "attention" : "muted"}
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
