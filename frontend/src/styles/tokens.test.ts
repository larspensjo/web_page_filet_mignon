import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { designTokens } from "./tokenValues";

describe("visual design tokens", () =>
	it("maps every custom property to its documented value", () => {
		const source = readFileSync("src/styles/tokens.css", "utf8");
		const parsed = Object.fromEntries(
			[...source.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/gi)].map(
				([, name, value]) => [name, value.trim().toLowerCase()],
			),
		);
		expect(Object.keys(parsed).sort()).toEqual(
			Object.keys(designTokens).sort(),
		);
		for (const [name, value] of Object.entries(designTokens))
			expect(parsed[name]).toBe(value.toLowerCase());
	}));
