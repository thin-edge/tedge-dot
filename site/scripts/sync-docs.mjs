#!/usr/bin/env node
// Generate the site's reference pages from the repository's Markdown (see docs-map.yaml).
//
// Runs before every `astro dev` and `astro build`. The generated pages are git-ignored: the
// repository file is the only source, and this script is how the site renders it.

import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync, copyFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import YAML from 'yaml';
import { betweenMarkers, convertPage, renderPackageTable, replaceBetweenMarkers } from './lib/sync.mjs';
import { renderConfigReference } from './lib/schema-reference.mjs';

const site = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(site, '..');
const docsDir = join(site, 'src/content/docs');
const imageDir = join(site, 'public/repo');
export const BASE = '/tedge-dot';

export function loadMap() {
  const map = YAML.parse(readFileSync(join(site, 'docs-map.yaml'), 'utf8'));
  const seen = new Set();
  for (const page of map.pages) {
    if (seen.has(page.slug)) throw new Error(`docs-map.yaml: duplicate slug ${page.slug}`);
    seen.add(page.slug);
    if (page.section && !map.sections.includes(page.section)) {
      throw new Error(`docs-map.yaml: ${page.source} has unknown section "${page.section}"`);
    }
  }
  return map;
}

/** Repository files whose content the site rewrites before publishing. */
const TRANSFORMS = {
  'doc/install.md': (markdown) => {
    const packages = YAML.parse(readFileSync(join(site, 'src/data/packages.yaml'), 'utf8'));
    const out = replaceBetweenMarkers(markdown, 'packages', renderPackageTable(packages));
    if (out === null) throw new Error('doc/install.md: missing <!-- site:packages --> markers');
    // Site-only: point at the interactive picker right after the table.
    return out.replace(
      '<!-- /site:packages -->\n',
      '<!-- /site:packages -->\n\n:::tip[Not sure?]\nThe [package picker](/tedge-dot/tools/package-picker/) asks a few questions about your gateway and recommends one.\n:::\n',
    );
  },
};

function main() {
  const map = loadMap();
  const slugs = new Map(map.pages.map((p) => [p.source, p.slug]));
  const ctx = {
    slugs,
    repository: map.repository,
    branch: map.branch,
    base: BASE,
    imageBase: `${BASE}/repo`,
    isDirectory: (p) => {
      const abs = join(repo, p);
      return existsSync(abs) && statSync(abs).isDirectory();
    },
  };

  // Start clean, so a page removed from the map disappears from the site.
  rmSync(join(docsDir, 'reference'), { recursive: true, force: true });
  rmSync(imageDir, { recursive: true, force: true });

  for (const page of map.pages) {
    const sourceFile = join(repo, page.source);
    if (!existsSync(sourceFile)) throw new Error(`docs-map.yaml: ${page.source} does not exist`);
    let markdown = readFileSync(sourceFile, 'utf8');
    if (TRANSFORMS[page.source]) markdown = TRANSFORMS[page.source](markdown);
    const { content, images } = convertPage(markdown, page, ctx);
    const out = join(docsDir, `${page.slug}.md`);
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, content);
    for (const image of images) {
      const from = join(repo, image);
      if (!existsSync(from)) throw new Error(`${page.source}: image ${image} does not exist`);
      mkdirSync(dirname(join(imageDir, image)), { recursive: true });
      copyFileSync(from, join(imageDir, image));
    }
  }
  // The configuration file reference, generated from the contract schema.
  const schema = JSON.parse(readFileSync(join(repo, 'doc/contract/schemas/config.schema.json'), 'utf8'));
  const configRef = join(docsDir, 'reference/configuration/config-file.md');
  mkdirSync(dirname(configRef), { recursive: true });
  writeFileSync(configRef, renderConfigReference(schema));

  console.log(`sync-docs: ${map.pages.length} pages from the repository`);
}

export { betweenMarkers };

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
