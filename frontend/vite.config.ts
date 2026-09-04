import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [react()],
	resolve: {
		alias: {
			"@fixtures": fileURLToPath(
				new URL("../crates/harvester_ui_bridge/fixtures", import.meta.url),
			),
		},
	},
	test: { environment: "jsdom", setupFiles: ["./src/test/setup.ts"] },
});
