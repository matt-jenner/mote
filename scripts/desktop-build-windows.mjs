import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const tauriDirectory = path.join(repositoryRoot, "apps/desktop/src-tauri");
const interfaceDirectory = path.join(repositoryRoot, "apps/interface");
const outputDirectory = path.join(repositoryRoot, "dist/windows");

function run(command, arguments_, environment) {
	const result = spawnSync(command, arguments_, {
		cwd: repositoryRoot,
		env: environment,
		stdio: "inherit",
	});
	if (result.error) throw result.error;
	if (result.status !== 0) {
		const error = new Error(
			`${command} failed with exit code ${result.status}`,
		);
		error.exitCode = result.status || 1;
		throw error;
	}
}

function requireFile(file) {
	if (!fs.statSync(file, { throwIfNoEntry: false })?.isFile()) {
		throw new Error(`Required build input is missing: ${file}`);
	}
}

function removeManagedPath(candidate, parent) {
	const relative = path.relative(parent, candidate);
	if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) {
		throw new Error(`Refusing to remove unmanaged path: ${candidate}`);
	}
	fs.rmSync(candidate, { recursive: true, force: true });
}

function processIsAlive(pid) {
	try {
		process.kill(pid, 0);
		return true;
	} catch (error) {
		return error.code !== "ESRCH";
	}
}

function reapAbandonedBuilds(temporaryRoot) {
	for (const entry of fs.readdirSync(temporaryRoot, { withFileTypes: true })) {
		if (!entry.isDirectory() || !entry.name.startsWith("mote-build-windows-")) {
			continue;
		}
		const directory = path.join(temporaryRoot, entry.name);
		const ownerFile = path.join(directory, ".mote-owner-pid");
		let owner;
		try {
			owner = fs.readFileSync(ownerFile, "utf8").trim();
		} catch {
			continue;
		}
		if (!/^\d+$/.test(owner) || processIsAlive(Number(owner))) continue;
		removeManagedPath(directory, temporaryRoot);
	}
}

function buildResources(nativePrefix) {
	return {
		[path.join(nativePrefix, "bin/heif.dll")]: "heif.dll",
		[path.join(nativePrefix, "bin/libde265.dll")]: "libde265.dll",
		[path.join(repositoryRoot, "THIRD_PARTY_NOTICES.md")]:
			"licenses/THIRD_PARTY_NOTICES.md",
		[path.join(repositoryRoot, "packaging/licenses/LGPL-3.0-or-later.txt")]:
			"licenses/LGPL-3.0-or-later.txt",
		[path.join(repositoryRoot, "packaging/licenses/libheif.md")]:
			"licenses/libheif.md",
		[path.join(repositoryRoot, "packaging/licenses/libde265.md")]:
			"licenses/libde265.md",
		[path.join(repositoryRoot, "packaging/heic/README.md")]:
			"licenses/HEIC-REBUILD.md",
		[path.join(repositoryRoot, "packaging/heic/decode-only.cmake")]:
			"licenses/decode-only.cmake",
	};
}

function publish(candidate, destination) {
	const staging = `${destination}.next.${process.pid}`;
	const backup = `${destination}.previous.${process.pid}`;
	fs.copyFileSync(candidate, staging);
	let movedPrevious = false;
	try {
		if (fs.existsSync(destination)) {
			fs.renameSync(destination, backup);
			movedPrevious = true;
		}
		fs.renameSync(staging, destination);
		if (movedPrevious) fs.rmSync(backup, { force: true });
	} catch (error) {
		fs.rmSync(staging, { force: true });
		if (movedPrevious && !fs.existsSync(destination)) {
			fs.renameSync(backup, destination);
		}
		throw error;
	}
}

function main() {
	let noHeic = false;
	let ci = false;
	for (const argument of process.argv.slice(2)) {
		if (argument === "--no-heic" && !noHeic) {
			noHeic = true;
		} else if (argument === "--ci" && !ci) {
			ci = true;
		} else {
			throw new Error("usage: desktop-build-windows.mjs [--no-heic] [--ci]");
		}
	}
	const configuration = JSON.parse(
		fs.readFileSync(path.join(tauriDirectory, "tauri.conf.json"), "utf8"),
	);
	const version = configuration.version;
	if (!/^\d+\.\d+\.\d+$/.test(version)) {
		throw new Error(`Invalid desktop version: ${version}`);
	}
	const icon = path.join(repositoryRoot, "docs/brand/icons/windows/Mote.ico");
	requireFile(icon);

	const temporaryRoot = path.resolve(
		process.env.MOTE_BUILD_TMP_ROOT || os.tmpdir(),
	);
	if (temporaryRoot === path.parse(temporaryRoot).root) {
		throw new Error(`Refusing unsafe Mote temporary root: ${temporaryRoot}`);
	}
	fs.mkdirSync(temporaryRoot, { recursive: true });
	reapAbandonedBuilds(temporaryRoot);
	const buildDirectory = fs.mkdtempSync(
		path.join(temporaryRoot, "mote-build-windows-"),
	);
	fs.writeFileSync(
		path.join(buildDirectory, ".mote-owner-pid"),
		`${process.pid}\n`,
	);

	try {
		const nativeRoot = path.join(
			repositoryRoot,
			"build/heic-native/windows-x64",
		);
		const nativePrefix = path.join(nativeRoot, "installed/x64-windows");
		const powershell = process.env.MOTE_PWSH || "pwsh";
		const environment = {
			...process.env,
			CARGO_TARGET_DIR: path.join(buildDirectory, "cargo-target"),
		};
		let resources;
		if (!noHeic) {
			run(
				powershell,
				[
					"-NoLogo",
					"-NoProfile",
					"-File",
					path.join(repositoryRoot, "packaging/heic/build-windows.ps1"),
					"-Arch",
					"x64",
				],
				environment,
			);
			environment.VCPKG_ROOT = nativeRoot;
			environment.VCPKGRS_TRIPLET = "x64-windows";
			environment.VCPKGRS_DYNAMIC = "1";
			environment.MOTE_HEIC_PREFIX = nativePrefix;
			const pathName =
				Object.keys(environment).find(
					(name) => name.toLowerCase() === "path",
				) ?? "PATH";
			const currentPath = environment[pathName];
			environment[pathName] = currentPath
				? `${path.join(nativePrefix, "bin")}${path.delimiter}${currentPath}`
				: path.join(nativePrefix, "bin");
			resources = buildResources(nativePrefix);
			for (const source of Object.keys(resources)) requireFile(source);
		}
		const override = JSON.stringify({
			bundle: {
				targets: ["nsis"],
				icon: [icon],
				windows: {
					certificateThumbprint: null,
					signCommand: null,
					timestampUrl: null,
				},
				...(resources ? { resources } : {}),
			},
		});
		const tauriArguments = [
			"build",
			"--bundles",
			"nsis",
			"--config",
			override,
		];
		if (noHeic) {
			tauriArguments.push(
				"--no-default-features",
				"--features",
				"mote-defaults",
			);
		}
		if (ci) tauriArguments.push("--ci");
		const tauriCli = path.join(
			repositoryRoot,
			"node_modules/@tauri-apps/cli/tauri.js",
		);
		requireFile(tauriCli);
		run(process.execPath, [tauriCli, ...tauriArguments], environment);

		const binary = path.join(
			environment.CARGO_TARGET_DIR,
			"release/photo-viewer-desktop.exe",
		);
		const verificationArguments = [
			"-NoLogo",
			"-NoProfile",
			"-File",
			path.join(repositoryRoot, "packaging/heic/verify-native-deps.ps1"),
		];
		if (!noHeic) verificationArguments.push("-Prefix", nativePrefix);
		verificationArguments.push("-Binary", binary, "-Arch", "x64");
		if (noHeic) verificationArguments.push("-NoHeic");
		run(powershell, verificationArguments, environment);

		const bundleDirectory = path.join(
			environment.CARGO_TARGET_DIR,
			"release/bundle/nsis",
		);
		const candidates = fs
			.readdirSync(bundleDirectory)
			.filter((name) => name.toLowerCase().endsWith("-setup.exe"));
		if (candidates.length !== 1) {
			throw new Error(
				`Expected one NSIS installer, found ${candidates.length}`,
			);
		}
		fs.mkdirSync(outputDirectory, { recursive: true });
		const destination = path.join(
			outputDirectory,
			`Mote-${version}-windows-x64-setup.exe`,
		);
		publish(path.join(bundleDirectory, candidates[0]), destination);
		console.log(`Windows installer: ${destination}`);
	} finally {
		if (fs.existsSync(buildDirectory)) {
			removeManagedPath(buildDirectory, temporaryRoot);
		}
		const interfaceOutput = path.join(interfaceDirectory, "dist");
		if (fs.existsSync(interfaceOutput)) {
			removeManagedPath(interfaceOutput, interfaceDirectory);
		}
	}
}

try {
	main();
} catch (error) {
	console.error(error.message);
	process.exitCode = error.exitCode || 1;
}
