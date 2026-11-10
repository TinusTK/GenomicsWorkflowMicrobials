// Writes latest.json: the list the installed app reads to update itself.
// For each installer of a release it holds the download link and the
// signature made at build time (the .sig file beside the installer).
//
//   node .github/update-index.mjs <folder with .sig files> <owner/repo> <tag> <assets.json> <out>
//
// <assets.json> is `gh release view <tag> --json assets`: only files that are
// really on the release are listed.

import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';

const [sigDir, repo, tag, assetsFile, out] = process.argv.slice(2);
if (!out) {
  console.error('usage: update-index.mjs <sig folder> <owner/repo> <tag> <assets.json> <out>');
  process.exit(2);
}

/** Which installed copies take which file. `os-arch-kind` is looked up first, then `os-arch`. */
export function targetsFor(file) {
  if (file.endsWith('-setup.exe')) return ['windows-x86_64-nsis', 'windows-x86_64'];
  if (file.endsWith('.msi')) return ['windows-x86_64-msi'];
  if (file.endsWith('.AppImage')) return ['linux-x86_64-appimage', 'linux-x86_64'];
  if (file.endsWith('.deb')) return ['linux-x86_64-deb'];
  if (file.endsWith('.rpm')) return ['linux-x86_64-rpm'];
  // one Mac build holds both chips
  if (file.endsWith('.app.tar.gz')) return ['darwin-aarch64-app', 'darwin-aarch64', 'darwin-x86_64-app', 'darwin-x86_64'];
  return [];
}

function sigFiles(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? sigFiles(path) : name.endsWith('.sig') ? [path] : [];
  });
}

const onRelease = new Set(JSON.parse(readFileSync(assetsFile, 'utf8')).assets.map((a) => a.name));
const platforms = {};
for (const sig of sigFiles(sigDir)) {
  // GitHub stores "Genomics Workflow Microbials_x.exe" as "Genomics.Workflow.Microbials_x.exe"
  const asset = basename(sig, '.sig').replaceAll(' ', '.');
  const targets = targetsFor(asset);
  if (!targets.length) continue;
  if (!onRelease.has(asset)) {
    console.error(`skipped ${asset}: it is not on the release`);
    continue;
  }
  const entry = {
    signature: readFileSync(sig, 'utf8').trim(),
    url: `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(asset)}`,
  };
  for (const t of targets) platforms[t] = entry;
}

for (const needed of ['windows-x86_64', 'linux-x86_64', 'darwin-aarch64']) {
  if (!platforms[needed]) {
    console.error(`no signed installer for ${needed}: the app there could not update itself`);
    process.exit(1);
  }
}

const index = {
  version: tag.replace(/^v/, ''),
  notes: `https://github.com/${repo}/releases/tag/${tag}`,
  pub_date: new Date().toISOString(),
  platforms,
};
writeFileSync(out, JSON.stringify(index, null, 2) + '\n');
console.log(`${out}: version ${index.version}, ${Object.keys(platforms).sort().join(', ')}`);
