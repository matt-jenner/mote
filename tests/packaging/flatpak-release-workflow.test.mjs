import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { parse } from "yaml";

const root = path.resolve(import.meta.dirname, "../..");
const workflowPath = path.join(root, ".github/workflows/release-flatpak.yml");
const githubTokenExpression = ["$", "{{ github.token }}"].join("");
const eventNameExpression = ["$", "{{ github.event_name }}"].join("");
const releaseTagExpression = ["$", "{{ github.event.release.tag_name }}"].join(
	"",
);
const checkoutRefExpression = [
	"$",
	"{{ github.event.release.tag_name || github.ref }}",
].join("");

function workflow() {
	return parse(fs.readFileSync(workflowPath, "utf8"));
}

function buildJob() {
	return workflow().jobs["build-flatpak"];
}

test("interface CI runs build contract tests immediately after installing dependencies", () => {
	const ci = parse(
		fs.readFileSync(path.join(root, ".github/workflows/ci.yml"), "utf8"),
	);
	const steps = ci.jobs.interface.steps;
	const installIndex = steps.findIndex((step) => step.run === "npm ci");
	assert.ok(installIndex >= 0, "interface CI must install dependencies");
	assert.equal(steps[installIndex + 1]?.run, "npm run test:build");
});

test("release validates the disabled package as well as the default package", () => {
	const step = buildJob().steps.find((step) =>
		step.run?.includes("package --no-heic"),
	);
	assert.ok(step, "missing disabled package inspection build");
	const ci = parse(
		fs.readFileSync(path.join(root, ".github/workflows/ci.yml"), "utf8"),
	);
	assert.ok(
		ci.jobs.interface.steps.some((step) => step.run === "npm run test:flatpak"),
	);
	assert.deepEqual(ci.jobs["hosted-container"].strategy.matrix.heic, [
		"enabled",
		"disabled",
	]);
	assert.deepEqual(ci.jobs["hosted-container"].strategy.matrix.os, [
		"ubuntu-latest",
		"ubuntu-24.04-arm",
	]);
	assert.ok(
		ci.jobs["hosted-container"].steps.some((step) =>
			step.run?.includes("hosted-smoke.sh --no-heic"),
		),
	);
});

test("Flatpak builds run for a published release or deliberate manual dispatch", () => {
	const releaseWorkflow = workflow();

	assert.deepEqual(releaseWorkflow.on, {
		release: { types: ["published"] },
		workflow_dispatch: null,
	});
	assert.equal(buildJob()["timeout-minutes"], 90);
});

test("the release tag is checked before installing runtimes or building", () => {
	const steps = buildJob().steps;
	const checkout = steps.find((step) =>
		step.uses?.startsWith("actions/checkout@"),
	);
	const validateIndex = steps.findIndex(
		(step) => step.name === "Validate release tag",
	);
	const dependenciesIndex = steps.findIndex(
		(step) => step.name === "Install Flatpak dependencies",
	);
	const buildIndex = steps.findIndex((step) => step.name === "Build Flatpak");

	assert.ok(validateIndex >= 0);
	assert.ok(validateIndex < dependenciesIndex);
	assert.ok(dependenciesIndex < buildIndex);
	assert.equal(checkout.with.ref, checkoutRefExpression);
	assert.match(steps[validateIndex].run, /tauri\.conf\.json/);
	assert.match(steps[validateIndex].run, /expected_tag="v\$\{app_version\}"/);
	assert.match(steps[validateIndex].run, /EVENT_NAME.*release/);
	assert.equal(steps[validateIndex].env.EVENT_NAME, eventNameExpression);
	assert.equal(steps[validateIndex].env.RELEASE_TAG, releaseTagExpression);
});

test("the existing packaging command builds with the locked Flatpak SDKs", () => {
	const steps = buildJob().steps;
	const install = steps.find(
		(step) => step.name === "Install Flatpak dependencies",
	).run;
	const build = steps.find((step) => step.name === "Build Flatpak").run;

	assert.match(install, /flatpak-builder/);
	for (const runtime of [
		"org.gnome.Platform//49",
		"org.gnome.Sdk//49",
		"org.freedesktop.Sdk.Extension.node24//25.08",
		"org.freedesktop.Sdk.Extension.rust-stable//25.08",
	]) {
		assert.match(install, new RegExp(runtime));
	}
	assert.equal(build, "npm run flatpak -- package");
});

test("published releases receive the bundle while manual checks retain one short-lived artifact", () => {
	const releaseWorkflow = workflow();
	const upload = buildJob().steps.find(
		(step) => step.name === "Upload Flatpak to release",
	);
	const uploadManual = buildJob().steps.find(
		(step) => step.name === "Upload manual-check artifact",
	);

	assert.equal(releaseWorkflow.permissions.contents, "write");
	assert.match(
		upload.run,
		/gh release upload "\$RELEASE_TAG" dist\/flatpak\/\*\.flatpak/,
	);
	assert.doesNotMatch(upload.run, /--clobber/);
	assert.equal(upload.env.GH_TOKEN, githubTokenExpression);
	assert.equal(upload.env.RELEASE_TAG, releaseTagExpression);
	assert.match(upload.if, /github\.event_name == 'release'/);
	assert.match(uploadManual.if, /github\.event_name == 'workflow_dispatch'/);
	assert.equal(uploadManual.with["retention-days"], 1);
});
