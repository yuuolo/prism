// Builds a signed release installer plus the updater manifest:
//  1. compiles prism-core and prism-service (the TUN service) and places
//     them as Tauri sidecars (binaries/<name>-<target-triple>.exe),
//  2. runs `tauri build`, signing the installer with the updater key
//     (TAURI_SIGNING_PRIVATE_KEY, or the file in PRISM_SIGNING_KEY,
//     default ~/.tauri/prism-updater.key),
//  3. writes <target>/release/bundle/nsis/latest.json, which the app checks at
//     https://github.com/yuuolo/prism/releases/latest/download/latest.json.
// Upload the installer, its .sig and latest.json to the GitHub release.
// Release notes for the manifest: PRISM_RELEASE_NOTES_FILE (optional).
// CARGO_TARGET_DIR moves <target> (default ./target), e.g. while a release
// build from ./target is running and its executables are locked.
import { execSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const REPO = "yuuolo/prism";
const run = (cmd) => execSync(cmd, { stdio: "inherit" });
const triple = execSync("rustc -vV").toString().match(/host: (\S+)/)[1];
const ext = process.platform === "win32" ? ".exe" : "";
const version = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).version;
const target = process.env.CARGO_TARGET_DIR || "target";

if (!process.env.TAURI_SIGNING_PRIVATE_KEY) {
  const keyFile = process.env.PRISM_SIGNING_KEY || join(homedir(), ".tauri", "prism-updater.key");
  if (!existsSync(keyFile)) {
    console.error(`Updater signing key not found: ${keyFile}\nSet PRISM_SIGNING_KEY or TAURI_SIGNING_PRIVATE_KEY.`);
    process.exit(1);
  }
  process.env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(keyFile, "utf8").trim();
}
process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= "";

const sidecars = ["prism-core", "prism-service"];
run(`cargo build --release ${sidecars.map((s) => `-p ${s}`).join(" ")}`);
mkdirSync("src-tauri/binaries", { recursive: true });
for (const s of sidecars) {
  copyFileSync(join(target, "release", `${s}${ext}`), `src-tauri/binaries/${s}-${triple}${ext}`);
}

// Tauri copies sidecars next to the main executable (without the triple),
// which is exactly where the app looks for them.
const extra = JSON.stringify({ bundle: { externalBin: sidecars.map((s) => `binaries/${s}`) } });
run(`npx tauri build --config ${JSON.stringify(extra)}`);

const dir = join(target, "release", "bundle", "nsis");
const installer = `Prism_${version}_x64-setup.exe`;
const signature = readFileSync(join(dir, `${installer}.sig`), "utf8").trim();
const notesFile = process.env.PRISM_RELEASE_NOTES_FILE;
const manifest = {
  version,
  notes: notesFile ? readFileSync(notesFile, "utf8") : `Prism v${version}`,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature,
      url: `https://github.com/${REPO}/releases/download/v${version}/${installer}`,
    },
  },
};
writeFileSync(join(dir, "latest.json"), JSON.stringify(manifest, null, 2));
console.log(`\nSigned ${installer}; wrote ${dir}/latest.json`);
