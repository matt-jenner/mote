import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const script = path.join(root, "scripts/flatpak.sh");

test("Flatpak helper exposes the complete local packaging interface", () => {
	const help = execFileSync("bash", [script, "help"], {
		cwd: root,
		encoding: "utf8",
	});
	for (const command of [
		"check",
		"build",
		"bundle",
		"package",
		"install",
		"run",
		"inspect",
	]) {
		assert.match(help, new RegExp(`^  ${command}(?: |$)`, "m"));
	}
});

test("Flatpak helper is valid Bash and never escalates privileges", () => {
	execFileSync("bash", ["-n", script], { cwd: root });
	const source = fs.readFileSync(script, "utf8");
	assert.doesNotMatch(source, /\bsudo\b/);
	assert.match(source, /--noninteractive/);
	assert.match(
		source,
		/--runtime-repo=https:\/\/flathub\.org\/repo\/flathub\.flatpakrepo/,
	);
	assert.match(source, /--show-permissions/);
});
