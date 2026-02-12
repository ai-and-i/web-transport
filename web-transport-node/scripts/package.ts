// Script to build and package a workspace for distribution
// This creates a dist/ folder with the correct paths and dependencies for publishing

import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

console.log("✍️  Rewriting package.json...");
const pkg = JSON.parse(readFileSync("package.json", "utf8"));

function rewritePath(p: string): string {
	return p.replace(/^\.\/src/, ".");
}

function rewriteExtension(p: string): string {
	return p.replace(/\.ts(x)?$/, ".js");
}

pkg.main &&= rewriteExtension(rewritePath(pkg.main));
pkg.types &&= rewritePath(pkg.types);

if (pkg.exports) {
	for (const key in pkg.exports) {
		const val = pkg.exports[key];
		if (typeof val === "string") {
			pkg.exports[key] = rewriteExtension(rewritePath(val));
		} else if (typeof val === "object") {
			for (const sub in val) {
				if (typeof val[sub] === "string") {
					val[sub] = rewriteExtension(rewritePath(val[sub]));
				}
			}
		}
	}
}

if (pkg.sideEffects) {
	pkg.sideEffects = pkg.sideEffects.map(rewriteExtension).map(rewritePath);
}

if (pkg.files) {
	pkg.files = pkg.files.map(rewritePath);
}

pkg.devDependencies = undefined;
pkg.scripts = undefined;

// Write the rewritten package.json
writeFileSync("dist/package.json", JSON.stringify(pkg, null, 2));

// Copy static files
console.log("📄 Copying README.md...");
copyFileSync("README.md", join("dist", "README.md"));
copyFileSync("LICENSE-MIT", join("dist", "LICENSE-MIT"));
copyFileSync("LICENSE-APACHE", join("dist", "LICENSE-APACHE"));

// Copy native binaries if present
const nativeDir = "native";
if (existsSync(nativeDir)) {
	const outDir = join("dist", nativeDir);
	mkdirSync(outDir, { recursive: true });
	for (const entry of readdirSync(nativeDir)) {
		if (entry.endsWith(".node")) {
			copyFileSync(join(nativeDir, entry), join(outDir, entry));
		}
	}
}

console.log("📦 Package built successfully in dist/");
