import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { parse } from "yaml";

const root = path.resolve(import.meta.dirname, "../..");
const workflowPath = path.join(root, ".github/workflows/build-windows.yml");
const expression = (contents) => ["$", `{{ ${contents} }}`].join("");

function workflow() {
	assert.ok(fs.existsSync(workflowPath), "Windows release workflow is missing");
	return parse(fs.readFileSync(workflowPath, "utf8"));
}

function releaseJob() {
	return workflow().jobs["windows-release"];
}

test("Windows builds run only for a published release or deliberate manual dispatch", () => {
	const releaseWorkflow = workflow();
	assert.deepEqual(releaseWorkflow.on, {
		release: { types: ["published"] },
		workflow_dispatch: null,
	});
	assert.deepEqual(Object.keys(releaseWorkflow.jobs), ["windows-release"]);
	assert.equal(releaseJob().strategy, undefined);
	assert.equal(releaseJob()["runs-on"], "windows-latest");
});

test("Windows release validates its tag before installing dependencies", () => {
	const steps = releaseJob().steps;
	const checkout = steps.find((step) =>
		step.uses?.startsWith("actions/checkout@"),
	);
	const metadataIndex = steps.findIndex(
		(step) => step.name === "Resolve release metadata",
	);
	const installIndex = steps.findIndex((step) => step.run === "npm ci");
	assert.equal(
		checkout.with.ref,
		expression("github.event.release.tag_name || github.ref"),
	);
	assert.equal(checkout.with["persist-credentials"], false);
	assert.ok(metadataIndex >= 0 && metadataIndex < installIndex);
	assert.equal(
		steps[metadataIndex].env.RELEASE_TAG,
		expression("github.event.release.tag_name"),
	);
	assert.match(steps[metadataIndex].run, /expectedTag = "v\$appVersion"/);
});

test("one unsigned HEIC-enabled x64 NSIS installer is built", () => {
	const steps = releaseJob().steps;
	assert.ok(
		steps.some((step) => step.uses?.startsWith("ilammy/msvc-dev-cmd@")),
	);
	const builds = steps.filter((step) =>
		step.run?.includes("desktop:build:windows"),
	);
	assert.equal(builds.length, 1);
	assert.match(builds[0].run, /desktop:build:windows -- --ci/);
	assert.doesNotMatch(builds[0].run, /--no-heic/);
});

test("release receives the installer while a manual check lasts one day", () => {
	const releaseWorkflow = workflow();
	const steps = releaseJob().steps;
	const uploadRelease = steps.find(
		(step) => step.name === "Upload installer to release",
	);
	const uploadManual = steps.find(
		(step) => step.name === "Upload manual-check artifact",
	);
	assert.equal(releaseWorkflow.permissions.contents, "write");
	assert.equal(uploadRelease.env.GH_TOKEN, expression("github.token"));
	assert.equal(
		uploadRelease.env.RELEASE_TAG,
		expression("github.event.release.tag_name"),
	);
	assert.match(
		uploadRelease.run,
		/gh release upload "\$env:RELEASE_TAG" "\$env:INSTALLER_PATH"/,
	);
	assert.doesNotMatch(uploadRelease.run, /--clobber/);
	assert.match(uploadRelease.if, /github\.event_name == 'release'/);
	assert.match(uploadManual.if, /github\.event_name == 'workflow_dispatch'/);
	assert.equal(uploadManual.with["retention-days"], 1);
});
