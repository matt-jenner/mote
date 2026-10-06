import fs from "node:fs";
import path from "node:path";
import { parse } from "yaml";

const root = path.resolve(import.meta.dirname, "..");
const mode = process.argv[2];
if (!["enabled", "disabled"].includes(mode) || process.argv.length !== 3) {
	throw new Error("usage: render-flatpak-manifest.mjs enabled|disabled");
}
const directory = path.join(root, "packaging/flatpak");
const manifest = parse(
	fs.readFileSync(
		path.join(directory, "io.github.matt_jenner.mote.yml"),
		"utf8",
	),
);
if (mode === "disabled") {
	manifest.modules = manifest.modules.filter(({ name }) => name === "mote");
	const app = manifest.modules[0];
	app["build-commands"] = app["build-commands"].map((command) =>
		command.startsWith("npm exec ")
			? `${command} -- --no-default-features --features mote-defaults`
			: command,
	);
}
// The manifest lives in the managed temporary directory. Resolve paths before
// handing it to flatpak-builder; its offline YAML does not expand shell env.
for (const module of manifest.modules) {
	module.sources = module.sources.map((source) => {
		if (typeof source === "string") return path.resolve(directory, source);
		if (source.path)
			return { ...source, path: path.resolve(directory, source.path) };
		return source;
	});
}
process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
