// After `npm run installer`: keep only the setup for the current version and delete older
// ones from target\release\bundle\nsis, so the folder never fills up with old releases.
import { readFileSync, readdirSync, rmSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { version } = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const dir = join(root, "target", "release", "bundle", "nsis");
const current = `_${version}_`;

if (!existsSync(dir)) process.exit(0);
const setups = readdirSync(dir).filter((f) => /^Atoll_.+-setup\.exe$/i.test(f));
// Never delete anything unless the current version's setup is really there.
if (!setups.some((f) => f.includes(current))) {
  console.warn(`[prune] no setup for ${version} found; kept the old ones`);
  process.exit(0);
}
for (const f of setups) {
  if (!f.includes(current)) {
    rmSync(join(dir, f));
    console.log(`[prune] deleted old installer ${f}`);
  }
}
