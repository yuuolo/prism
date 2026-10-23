// Builds a signed release installer plus the updater manifest:
//  1. compiles prism-core and places it as a Tauri sidecar
//     (binaries/prism-core-<target-triple>.exe),
//  2. runs `tauri build`, signing the installer with the updater key
//     (TAURI_SIGNING_PRIVATE_KEY, or the file in PRISM_SIGNING_KEY,
//     default ~/.tauri/prism-updater.key),
//  3. writes target/release/bundle/nsis/latest.json, which the app checks at
//     https://github.com/yuuolo/prism/releases/latest/download/latest.json.
// Upload the installer, its .sig and latest.json to the GitHub release.
// Release notes for the manifest: PRISM_RELEASE_NOTES_FILE (optional).
import { execSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const REPO = "yuuolo/prism";
const run = (cmd) => execSync(cmd, { stdio: "inherit" });
const triple = execSync("rustc -vV").toString().match(/host: (\S+)/)[1];
const ext = process.platform === "win32" ? ".exe" : "";
const version = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).version;

if (!process.env.TAURI_SIGNING_PRIVATE_KEY) {
  const keyFile = process.env.PRISM_SIGNING_KEY || join(homedir(), ".tauri", "prism-updater.key");
  if (!existsSync(keyFile)) {
    console.error(`Updater signing key not found: ${keyFile}\nSet PRISM_SIGNING_KEY or TAURI_SIGNING_PRIVATE_KEY.`);
    process.exit(1);
  }
  process.env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(keyFile, "utf8").trim();
}
process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= "";

run("cargo build --release -p prism-core");
mkdirSync("src-tauri/binaries", { recursive: true });
copyFileSync(`target/release/prism-core${ext}`, `src-tauri/binaries/prism-core-${triple}${ext}`);

// Tauri copies sidecars next to the main executable (without the triple),
// which is exactly where the app looks for prism-core.
const extra = JSON.stringify({ bundle: { externalBin: ["binaries/prism-core"] } });
run(`npx tauri build --config ${JSON.stringify(extra)}`);

const dir = "target/release/bundle/nsis";
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
