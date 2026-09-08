import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const failures = [];

const read = (relativePath) =>
  readFileSync(path.join(repoRoot, relativePath), "utf8");
const readJson = (relativePath) => JSON.parse(read(relativePath));
const expectEqual = (label, actual, expected) => {
  if (actual !== expected) {
    failures.push(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
};
const expectMatch = (label, contents, pattern) => {
  if (!pattern.test(contents)) failures.push(`${label}: required value was not found`);
};

const packageJson = readJson("package.json");
const packageLock = readJson("package-lock.json");
const tauriConfig = readJson("src-tauri/tauri.conf.json");
const cargoManifest = read("src-tauri/Cargo.toml");
const cargoLock = read("Cargo.lock");
const license = read("LICENSE");
const notices = read("THIRD_PARTY_NOTICES.md");
const version = packageJson.version;

if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  failures.push(`package.json version is not a supported SemVer release: ${version}`);
}

const cargoVersion = cargoManifest.match(/^version = "([^"]+)"/m)?.[1];
const cargoName = cargoManifest.match(/^name = "([^"]+)"/m)?.[1];
const lockedPackage = cargoLock.match(
  /\[\[package\]\]\r?\nname = "cc-session-manager"\r?\nversion = "([^"]+)"/,
);

expectEqual("package-lock root name", packageLock.name, "cc-session-manager");
expectEqual("package-lock root version", packageLock.version, version);
expectEqual("package-lock workspace name", packageLock.packages?.[""]?.name, "cc-session-manager");
expectEqual("package-lock workspace version", packageLock.packages?.[""]?.version, version);
expectEqual("Tauri product name", tauriConfig.productName, "AgentVault");
expectEqual("Tauri version", tauriConfig.version, version);
expectEqual("Tauri bundle identifier", tauriConfig.identifier, "dev.cc.session-manager");
expectEqual("Rust package name", cargoName, "cc-session-manager");
expectEqual("Rust package version", cargoVersion, version);
expectEqual("Cargo.lock application version", lockedPackage?.[1], version);
expectMatch("Rust default binary", cargoManifest, /^default-run = "cc-session-manager"$/m);
expectMatch(
  "desktop binary",
  cargoManifest,
  /\[\[bin\]\]\r?\nname = "cc-session-manager"/,
);
expectMatch("CLI binary", cargoManifest, /\[\[bin\]\]\r?\nname = "cc-sessions"/);
expectMatch("upstream license", license, /MIT License/);
expectMatch("upstream copyright", license, /Copyright \(c\) 2026 ccpopy/);
expectMatch(
  "locked upstream commit",
  notices,
  /1c912b2bb35e328881f543dfef1eeb5ff510f2bf/,
);

for (const source of [
  "src-tauri/src/app_update.rs",
  "src-tauri/src/fs_ops.rs",
  "src/lib/api.ts",
]) {
  if (read(source).includes("ccpopy/cc-sessions/releases")) {
    failures.push(`${source}: AgentVault release code must not fall back to cc-sessions releases`);
  }
}

const releaseNotes = path.join("docs", "releases", `${version}.md`);
if (!existsSync(path.join(repoRoot, releaseNotes))) {
  failures.push(`release notes are missing: ${releaseNotes}`);
}

for (const icon of [
  "src-tauri/icons/32x32.png",
  "src-tauri/icons/128x128.png",
  "src-tauri/icons/128x128@2x.png",
  "src-tauri/icons/icon.ico",
  "src-tauri/icons/icon.icns",
]) {
  const iconPath = path.join(repoRoot, icon);
  if (!existsSync(iconPath) || statSync(iconPath).size === 0) {
    failures.push(`required Tauri icon is missing or empty: ${icon}`);
  }
}

const tagIndex = process.argv.indexOf("--tag");
if (tagIndex >= 0) {
  const tag = process.argv[tagIndex + 1];
  if (!tag || tagIndex + 2 !== process.argv.length) {
    failures.push("usage: check-release.mjs [--tag v<version>]");
  } else {
    expectEqual("release tag", tag, `v${version}`);
  }
} else if (process.argv.length > 2) {
  failures.push("usage: check-release.mjs [--tag v<version>]");
}

if (failures.length > 0) {
  for (const failure of failures) console.error(`- ${failure}`);
  process.exitCode = 1;
} else {
  console.log(`Release metadata verified: AgentVault v${version}`);
  console.log("Compatibility identifiers and upstream licensing are unchanged.");
}
