// Puts the changelog into the plugin zip, next to README.md: copies it to src/generated/, which
// angular.json packages as an asset. Release builds take the repository's CHANGELOG.md, which
// release-please writes; a test build points TEDGE_DOT_UI_CHANGELOG at its own changelog. Run
// before every build (see package.json).
import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ui = join(dirname(fileURLToPath(import.meta.url)), '..');
const source = resolve(ui, process.env.TEDGE_DOT_UI_CHANGELOG || join('..', 'CHANGELOG.md'));
if (!existsSync(source)) {
  throw new Error(`No changelog at ${source}; set TEDGE_DOT_UI_CHANGELOG to the file to package`);
}
const out = join(ui, 'src', 'generated');
mkdirSync(out, { recursive: true });
copyFileSync(source, join(out, 'CHANGELOG.md'));
console.log(`packaging ${source} as CHANGELOG.md`);
