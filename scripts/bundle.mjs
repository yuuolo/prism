// Builds a release installer: compiles prism-core, places it as a Tauri
// sidecar (binaries/prism-core-<target-triple>.exe) and runs `tauri build`.
import { execSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";

const run = (cmd) => execSync(cmd, { stdio: "inherit" });
const triple = execSync("rustc -vV").toString().match(/host: (\S+)/)[1];
const ext = process.platform === "win32" ? ".exe" : "";

run("cargo build --release -p prism-core");
mkdirSync("src-tauri/binaries", { recursive: true });
copyFileSync(`target/release/prism-core${ext}`, `src-tauri/binaries/prism-core-${triple}${ext}`);

// Tauri copies sidecars next to the main executable (without the triple),
// which is exactly where the app looks for prism-core.
const extra = JSON.stringify({ bundle: { externalBin: ["binaries/prism-core"] } });
run(`npx tauri build --config ${JSON.stringify(extra)}`);
