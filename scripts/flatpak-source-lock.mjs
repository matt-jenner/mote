// Offline Cargo archive inventory and input attestation. npm's cache/index
// layout remains owned by flatpak-node-generator and is checked before reuse.
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const [root, output] = process.argv.slice(2);
if (!root || !output)
	throw new Error("expected repository and candidate directory");
const read = (file) => fs.readFileSync(path.join(root, file), "utf8");
const digest = (file) => createHash("sha256").update(read(file)).digest("hex");
const nodeSources = JSON.parse(
	fs.readFileSync(path.join(output, "node-sources.json"), "utf8"),
);
for (const pkg of Object.values(
	JSON.parse(read("package-lock.json")).packages,
)) {
	if (pkg.link || !pkg.resolved) continue;
	const [algorithm, base64] = (pkg.integrity ?? "").split("-");
	const hex = Buffer.from(base64 ?? "", "base64").toString("hex");
	if (
		!hex ||
		!nodeSources.some(
			(source) => source.url === pkg.resolved && source[algorithm] === hex,
		)
	)
		throw new Error(
			`missing or stale npm source: ${pkg.resolved}; run flatpak-node-generator`,
		);
}
const packages = new Map();
for (const lockfile of ["Cargo.lock", "apps/desktop/src-tauri/Cargo.lock"]) {
	for (const block of read(lockfile).split("[[package]]").slice(1)) {
		const value = (key) =>
			block.match(new RegExp(`^${key} = "([^"\\n]+)"`, "m"))?.[1];
		const source = value("source");
		if (!source) continue;
		if (source !== "registry+https://github.com/rust-lang/crates.io-index")
			throw new Error(`unsupported Cargo source: ${source}`);
		const name = value("name");
		const version = value("version");
		const checksum = value("checksum");
		if (!name || !version || !/^[0-9a-f]{64}$/.test(checksum))
			throw new Error(`invalid package in ${lockfile}`);
		const key = `${name}-${version}`;
		if (packages.has(key) && packages.get(key).checksum !== checksum)
			throw new Error(`conflicting Cargo checksum: ${key}`);
		packages.set(key, { name, version, checksum });
	}
}
const sources = [...packages]
	.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
	.flatMap(([key, pkg]) => [
		{
			type: "archive",
			"archive-type": "tar-gzip",
			url: `https://static.crates.io/crates/${pkg.name}/${key}.crate`,
			sha256: pkg.checksum,
			dest: `cargo/vendor/${key}`,
		},
		{
			type: "inline",
			contents: JSON.stringify({ package: pkg.checksum, files: {} }),
			dest: `cargo/vendor/${key}`,
			"dest-filename": ".cargo-checksum.json",
		},
	]);
sources.push({
	type: "inline",
	contents:
		'[source.vendored-sources]\ndirectory = "cargo/vendor"\n\n[source.crates-io]\nreplace-with = "vendored-sources"\n',
	dest: "cargo",
	"dest-filename": "config",
});
const pins = Object.fromEntries(
	read("packaging/heic/versions.env")
		.split("\n")
		.filter((line) => /^[A-Z0-9_]+=/.test(line))
		.map((line) => {
			const index = line.indexOf("=");
			return [line.slice(0, index), line.slice(index + 1)];
		}),
);
const native = {};
for (const name of ["libde265", "libheif"]) {
	const key = name.toUpperCase();
	native[name] = {
		version: pins[`${key}_VERSION`],
		url: pins[`${key}_URL`],
		sha256: pins[`${key}_SHA256`],
	};
}
const previous = JSON.parse(
	fs.readFileSync(path.join(output, "source-lock.json"), "utf8"),
);
const lock = {
	generator: previous.generator,
	cargoGenerator: "scripts/flatpak-source-lock.mjs",
	lockfiles: Object.fromEntries(
		[
			"package-lock.json",
			"Cargo.lock",
			"apps/desktop/src-tauri/Cargo.lock",
			"packaging/heic/versions.env",
		].map((file) => [file, digest(file)]),
	),
	native,
};
// All validation is complete before any candidate files are written.
fs.writeFileSync(
	path.join(output, "cargo-sources.json"),
	`${JSON.stringify(sources, null, 4)}\n`,
);
fs.writeFileSync(
	path.join(output, "source-lock.json"),
	`${JSON.stringify(lock, null, 2)}\n`,
);
