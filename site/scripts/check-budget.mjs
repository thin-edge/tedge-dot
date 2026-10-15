#!/usr/bin/env node
// The landing page may load at most 100 kB of JavaScript (uncompressed) on page load: the
// module scripts in dist/index.html and every module they import statically. Pagefind, which
// loads only when a visitor opens search, is not counted.

import { existsSync, readFileSync, statSync } from 'node:fs';
import { join, posix } from 'node:path';

const BUDGET = 100 * 1024;
const dist = new URL('../dist/', import.meta.url).pathname;
const BASE = '/tedge-dot/';
const html = readFileSync(join(dist, 'index.html'), 'utf8');

const seen = new Set();
let total = 0;
const visit = (url) => {
  if (seen.has(url) || !url.startsWith(BASE) || url.includes('/pagefind/')) return;
  seen.add(url);
  const file = join(dist, url.slice(BASE.length));
  if (!existsSync(file)) throw new Error(`missing ${file}`);
  total += statSync(file).size;
  const code = readFileSync(file, 'utf8');
  for (const m of code.matchAll(/(?:^|[;\n}])\s*import\s*(?:[^'"()]*?from\s*)?["']([^"']+)["']/g)) {
    visit(m[1].startsWith('.') ? posix.join(posix.dirname(url), m[1]) : m[1]);
  }
};
for (const m of html.matchAll(/<script[^>]*\ssrc="([^"]+)"/g)) visit(m[1]);
const inline = [...html.matchAll(/<script(?![^>]*\ssrc=)[^>]*>([\s\S]*?)<\/script>/g)].reduce((n, m) => n + m[1].length, 0);
total += inline;

const kb = (n) => `${(n / 1024).toFixed(1)} kB`;
console.log(`landing page JavaScript: ${kb(total)} (${seen.size} files + ${kb(inline)} inline), budget ${kb(BUDGET)}`);
if (total > BUDGET) {
  console.error('The landing page is over its JavaScript budget.');
  process.exit(1);
}
