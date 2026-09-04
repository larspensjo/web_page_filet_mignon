import { dispatchIntent } from "./ipc/intent";
import { IPC_SCHEMA_VERSION } from "./ipc/schemaVersion";
import { useProbe } from "./ipc/useProbe";
import { useSnapshot } from "./ipc/useSnapshot";
import "./styles/base.css";
import "./styles/tokens.css";

export function App() {
	const snapshot = useSnapshot();
	useProbe();
	if (snapshot?.fatal_message)
		return (
			<main className="blocking">
				<h1>Harvester stopped responding — see engine.log</h1>
				<p>{snapshot.fatal_message}</p>
			</main>
		);
	if (snapshot && snapshot.schema_version !== IPC_SCHEMA_VERSION)
		return (
			<main className="blocking">
				frontend bundle is out of date — run <code>npm run build</code>
			</main>
		);
	const jobs = snapshot?.view.jobs ?? [];
	return (
		<main>
			<header>
				<h1>Harvester</h1>
				<button
					type="button"
					onClick={() => void dispatchIntent({ type: "PollSources" })}
				>
					Poll Sources
				</button>
			</header>
			<section aria-label="Jobs">
				<h2>Jobs</h2>
				{jobs.length === 0 ? (
					<p>No jobs yet.</p>
				) : (
					<ul>
						{jobs.map((job) => (
							<li key={job.job_id}>
								<strong>{job.summary_title ?? job.url}</strong>
								<span>{job.stage}</span>
							</li>
						))}
					</ul>
				)}
			</section>
		</main>
	);
}
