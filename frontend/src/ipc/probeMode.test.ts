import { describe, expect, it } from "vitest";
import { isProbePage } from "./probeMode";

describe("isProbePage", () => {
	it("recognizes probe query parameters", () => {
		expect(isProbePage("?probe=1&case=x")).toBe(true);
	});

	it("rejects an empty query", () => {
		expect(isProbePage("")).toBe(false);
	});

	it("rejects queries without probe", () => {
		expect(isProbePage("?other=1")).toBe(false);
	});
});
