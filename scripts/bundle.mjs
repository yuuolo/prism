// Builds a release bundle plus, when signing, the updater manifest:
//  1. compiles prism-core and prism-service (the TUN service) and places
//     them as Tauri sidecars (binaries/<name>-<target-triple>[.exe]),
//  2. runs `tauri build`: the NSIS installer on Windows, Prism.app + .dmg on
//     macOS (tauri.macos.conf.json), signing the updater artifact with the
//     updater key (TAURI_SIGNING_PRIVATE_KEY, or the file in
//     PRISM_SIGNING_KEY, default ~/.tauri/prism-updater.key),
//  3. on Windows writes <target>/release/bundle/nsis/latest.json, which the
//     app checks at
//     https://github.com/yuuolo/prism/releases/latest/download/latest.json.
// Upload the installer, its .sig and latest.json to the GitHub release.
// Release notes for the manifest: PRISM_RELEASE_NOTES_FILE (optional).
// CARGO_TARGET_DIR moves <target> (default ./target), e.g. while a release
// build from ./target is running and its executables are locked.
// PRISM_TARGET builds for another target triple (x86_64-apple-darwin on an
// Apple Silicon Mac, say). PRISM_UPDATER_SIGN=0 skips updater signing
// (CI without the key); the macOS update archive is then signed locally.
import { execSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const REPO = "yuuolo/prism";
const run = (cmd) => execSync(cmd, { stdio: "inherit" });
const host = execSync("rustc -vV").toString().match(/host: (\S+)/)[1];
const triple = process.env.PRISM_TARGET || host;
const windows = triple.includes("windows");
const ext = windows ? ".exe" : "";
const version = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).version;
const target = process.env.CARGO_TARGET_DIR || "target";
const sign = process.env.PRISM_UPDATER_SIGN !== "0";
// cargo puts --target builds under <target>/<triple>/.
const out = process.env.PRISM_TARGET ? join(target, triple, "release") : join(target, "release");
const targetArg = process.env.PRISM_TARGET ? ` --target ${triple}` : "";

if (sign && !process.env.TAURI_SIGNING_PRIVATE_KEY) {
  const keyFile = process.env.PRISM_SIGNING_KEY || join(homedir(), ".tauri", "prism-updater.key");
  if (!existsSync(keyFile)) {
    console.error(`Updater signing key not found: ${keyFile}\nSet PRISM_SIGNING_KEY or TAURI_SIGNING_PRIVATE_KEY, or PRISM_UPDATER_SIGN=0.`);
    process.exit(1);
  }
  process.env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(keyFile, "utf8").trim();
}
if (sign) {
  process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= "";
}

const sidecars = ["prism-core", "prism-service"];
run(`cargo build --release${targetArg} ${sidecars.map((s) => `-p ${s}`).join(" ")}`);
mkdirSync("src-tauri/binaries", { recursive: true });
for (const s of sidecars) {
  copyFileSync(join(out, `${s}${ext}`), `src-tauri/binaries/${s}-${triple}${ext}`);
}

// Tauri copies sidecars next to the main executable (without the triple),
// which is exactly where the app looks for them.
const extra = { bundle: { externalBin: sidecars.map((s) => `binaries/${s}`) } };
if (!sign) {
  extra.bundle.createUpdaterArtifacts = false;
}
run(`npx tauri build${targetArg} --config ${JSON.stringify(JSON.stringify(extra))}`);

if (!windows) {
  console.log(`\nBuilt ${join(out, "bundle", "macos", "Prism.app")} and the .dmg in ${join(out, "bundle", "dmg")}`);
  process.exit(0);
}

const dir = join(out, "bundle", "nsis");
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
