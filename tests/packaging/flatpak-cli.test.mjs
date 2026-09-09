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

test("Flatpak helper preserves the application and bundle naming contract", () => {
	const source = fs.readFileSync(script, "utf8");
	assert.match(source, /app_id="io\.github\.matt_jenner\.mote"/);
	assert.match(source, /Mote-%s-%s\.flatpak/);
	assert.match(source, /MOTE_FLATPAK_ARCH/);
	assert.match(source, /require\(process\.argv\[1\]\)\.version/);
});

test("Flatpak helper updates the current user's installed bundle", () => {
	const source = fs.readFileSync(script, "utf8");
	assert.match(source, /flatpak install --user --noninteractive --or-update/);
});

test("Flatpak validation runs from the repository root", () => {
	const source = fs.readFileSync(script, "utf8");
	assert.match(source, /cd -- "\$repository_root"/);
	assert.match(source, /npm run test:flatpak/);
});
