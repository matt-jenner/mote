import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import test from "node:test";

const publicDir = "apps/interface/public";
const webBrandIconDir = "docs/brand/icons/web";

function sha256(filename) {
	return createHash("sha256").update(fs.readFileSync(filename)).digest("hex");
}

for (const filename of ["favicon.svg", "favicon.ico"]) {
	test(`web app ships the approved Mote ${filename}`, () => {
		const shippingIcon = `${publicDir}/${filename}`;
		assert.ok(fs.existsSync(shippingIcon), `${shippingIcon} is missing`);
		assert.equal(
			sha256(shippingIcon),
			sha256(`${webBrandIconDir}/${filename}`),
		);
	});
}

test("web app declares the SVG favicon with an ICO fallback", () => {
	const html = fs.readFileSync("apps/interface/index.html", "utf8");
	const icoLink =
		'<link rel="icon" href="/favicon.ico" sizes="16x16 32x32 48x48" />';
	const svgLink =
		'<link rel="icon" href="/favicon.svg" type="image/svg+xml" sizes="any" />';

	assert.ok(html.includes(icoLink), "HTML is missing the sized ICO fallback");
	assert.ok(html.includes(svgLink), "HTML is missing the scalable SVG favicon");
	assert.ok(
		html.indexOf(icoLink) < html.indexOf(svgLink),
		"SVG favicon must follow the ICO fallback so equally suitable browsers prefer it",
	);
});
