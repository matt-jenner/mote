import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { parse } from "yaml";

const repositoryRoot = path.resolve(import.meta.dirname, "../..");
const composeFile = path.join(repositoryRoot, "deploy/compose.yaml");
const containerEngine = process.env.CONTAINER_ENGINE ?? "podman";

function renderCompose(extraEnvironment = {}) {
	return execFileSync(
		containerEngine,
		["compose", "-f", composeFile, "config"],
		{
			cwd: repositoryRoot,
			env: {
				...process.env,
				PHOTO_PATH: path.join(
					repositoryRoot,
					"apps/interface/public/demo-photos",
				),
				...extraEnvironment,
			},
			encoding: "utf8",
			stdio: ["ignore", "pipe", "pipe"],
		},
	);
}

function environmentWithStoragePaths() {
	return {
		...process.env,
		PHOTO_VIEWER_DATA_PATH: path.join(repositoryRoot, "runtime/data"),
		PHOTO_VIEWER_CACHE_PATH: path.join(repositoryRoot, "runtime/cache"),
	};
}

function environmentWithout(storagePath) {
	const environment = environmentWithStoragePaths();
	delete environment[storagePath];
	return environment;
}

for (const storagePath of [
	"PHOTO_VIEWER_DATA_PATH",
	"PHOTO_VIEWER_CACHE_PATH",
]) {
	test(`hosted Compose rejects a missing ${storagePath}`, () => {
		assert.throws(
			() =>
				execFileSync(
					containerEngine,
					["compose", "-f", composeFile, "config"],
					{
						cwd: repositoryRoot,
						env: environmentWithout(storagePath),
						encoding: "utf8",
						stdio: ["ignore", "pipe", "pipe"],
					},
				),
			(error) => {
				const output = `${error.stdout ?? ""}${error.stderr ?? ""}`;
				return error.status !== 0 && output.includes(storagePath);
			},
		);
	});
}

test("hosted Compose bind-mounts the catalogue and derivative cache", () => {
	const dataPath = path.join(repositoryRoot, "runtime/data");
	const cachePath = path.join(repositoryRoot, "runtime/cache");
	const configuration = parse(
		renderCompose({
			PHOTO_VIEWER_DATA_PATH: dataPath,
			PHOTO_VIEWER_CACHE_PATH: cachePath,
		}),
	);
	const volumes = configuration.services["photo-viewer"].volumes;
	const declaredVolumes = parse(readFileSync(composeFile, "utf8")).services[
		"photo-viewer"
	].volumes;
	const expectedMounts = new Map([
		["/photos", path.join(repositoryRoot, "apps/interface/public/demo-photos")],
		["/var/lib/photo-viewer", dataPath],
		["/var/cache/photo-viewer", cachePath],
	]);

	for (const [target, source] of expectedMounts) {
		const shortMount = volumes.find(
			(volume) =>
				typeof volume === "string" && volume.startsWith(`${source}:${target}:`),
		);
		const longMount = volumes.find(
			(volume) =>
				typeof volume === "object" &&
				volume.source === source &&
				volume.target === target,
		);
		assert.ok(shortMount ?? longMount, `missing host bind mount for ${target}`);
		if (longMount) {
			assert.equal(longMount.type, "bind");
		}
		const declaredMount = declaredVolumes.find((volume) =>
			volume.includes(`:${target}:`),
		);
		assert.ok(declaredMount, `missing declared mount for ${target}`);
		const options = declaredMount.split(`:${target}:`)[1].split(",");
		assert.ok(options.includes("Z"), `missing SELinux relabel for ${target}`);
	}
	assert.equal(configuration.volumes, undefined);
});
