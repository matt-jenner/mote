import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { parse } from "yaml";

const root = path.resolve(import.meta.dirname, "../..");
const workflowPath = path.join(root, ".github/workflows/build-macos.yml");
const ciWorkflowPath = path.join(root, ".github/workflows/ci.yml");
const githubTokenExpression = ["$", "{{ github.token }}"].join("");
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

function releaseJob() {
	return workflow().jobs["macos-release"];
}

function ciWorkflow() {
	return parse(fs.readFileSync(ciWorkflowPath, "utf8"));
}

test("macOS builds run only for a published release or deliberate manual dispatch", () => {
	const releaseWorkflow = workflow();

	assert.deepEqual(releaseWorkflow.on, {
		release: { types: ["published"] },
		workflow_dispatch: null,
	});
	assert.deepEqual(Object.keys(releaseWorkflow.jobs), ["macos-release"]);
	assert.equal(releaseJob().strategy, undefined);
	assert.equal(releaseJob()["runs-on"], "macos-latest");
});

test("macOS release checks out and validates the exact release tag before building", () => {
	const steps = releaseJob().steps;
	const checkout = steps.find((step) =>
		step.uses?.startsWith("actions/checkout@"),
	);
	const metadataIndex = steps.findIndex(
		(step) => step.name === "Resolve release metadata",
	);
	const installIndex = steps.findIndex((step) => step.run === "npm ci");

	assert.equal(checkout.with.ref, checkoutRefExpression);
	assert.equal(checkout.with["persist-credentials"], false);
	assert.ok(metadataIndex >= 0);
	assert.ok(metadataIndex < installIndex);
	assert.equal(steps[metadataIndex].env.RELEASE_TAG, releaseTagExpression);
	assert.match(steps[metadataIndex].run, /tauri\.conf\.json/);
	assert.match(steps[metadataIndex].run, /expected_tag="v\$\{app_version\}"/);
});

test("one HEIC-enabled universal app is ad-hoc signed and packaged as a verified DMG", () => {
	const steps = releaseJob().steps;
	const builds = steps.filter((step) =>
		step.run?.includes("desktop:build:universal"),
	);
	const packageStep = steps.find(
		(step) => step.name === "Create and verify DMG",
	);

	assert.equal(builds.length, 1);
	assert.doesNotMatch(builds[0].run, /--no-heic/);
	assert.equal(builds[0].env.APPLE_SIGNING_IDENTITY, "-");
	assert.ok(
		steps.some((step) =>
			step.run?.includes(
				"verify-native-deps.sh --app dist/macos/Mote.app --arch universal",
			),
		),
	);
	assert.match(packageStep.run, /hdiutil create/);
	assert.match(packageStep.run, /hdiutil verify/);
	assert.match(packageStep.run, /hdiutil attach/);
	assert.match(packageStep.run, /lipo .* -verify_arch arm64 x86_64/);
	assert.match(packageStep.run, /codesign --verify --deep --strict/);
	assert.match(
		packageStep.run,
		/scripts\/verify-macos-app-launch\.sh "\$mounted_app"/,
	);
	assert.match(packageStep.run, /Mote-\$\{APP_VERSION\}-macOS\.dmg/);
	assert.match(packageStep.run, /rm -rf "\$stage" "\$mountpoint"/);
});

test("macOS bundle enables bundled ad-hoc native libraries", () => {
	const config = JSON.parse(
		fs.readFileSync(
			path.join(root, "apps/desktop/src-tauri/tauri.conf.json"),
			"utf8",
		),
	);
	const entitlements = fs.readFileSync(
		path.join(root, "apps/desktop/src-tauri/Entitlements.plist"),
		"utf8",
	);

	assert.equal(config.bundle.macOS.entitlements, "Entitlements.plist");
	assert.match(
		entitlements,
		/<key>com\.apple\.security\.cs\.disable-library-validation<\/key>\s*<true\/>/,
	);
});

test("macOS CI exercises launch verification behavior", () => {
	const job = ciWorkflow().jobs["macos-launch-verification"];

	assert.equal(job["runs-on"], "macos-latest");
	assert.ok(
		job.steps.some(
			(step) =>
				step.run ===
				"node --test tests/build/macos-launch-verification.test.mjs",
		),
	);
});

test("published releases receive the raw DMG while manual checks retain one short-lived artifact", () => {
	const releaseWorkflow = workflow();
	const steps = releaseJob().steps;
	const uploadRelease = steps.find(
		(step) => step.name === "Upload DMG to release",
	);
	const uploadManual = steps.find(
		(step) => step.name === "Upload manual-check artifact",
	);

	assert.equal(releaseWorkflow.permissions.contents, "write");
	assert.equal(uploadRelease.env.GH_TOKEN, githubTokenExpression);
	assert.equal(uploadRelease.env.RELEASE_TAG, releaseTagExpression);
	assert.match(
		uploadRelease.run,
		/gh release upload "\$RELEASE_TAG" "\$DMG_PATH"/,
	);
	assert.doesNotMatch(uploadRelease.run, /--clobber/);
	assert.match(uploadRelease.if, /github\.event_name == 'release'/);
	assert.match(uploadManual.if, /github\.event_name == 'workflow_dispatch'/);
	assert.equal(uploadManual.with["retention-days"], 1);
});
