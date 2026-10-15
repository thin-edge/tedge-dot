#!/usr/bin/env node
// Check the box-drawing diagrams in the repository's Markdown: every box traced from its
// top-left corner must have straight left and right sides and a matching bottom edge, and an
// arrowhead must sit on the line it ends. A box
// whose border is off by a column renders ragged on GitHub and on the site alike.
//
//   node site/scripts/lint-diagrams.mjs [files...]   (default: every page in site/docs-map.yaml)

import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import YAML from 'yaml';

const repo = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const VERT = new Set([...'│├┤┼┬┴╞╡║╟╢']); // a vertical side may be crossed or teed
const HORZ = new Set([...'─┬┴┼═╤╧├┤┤▶◀►◄<>']);

export function lintDiagram(lines) {
  const grid = lines.map((l) => [...l]);
  const at = (r, c) => grid[r]?.[c] ?? ' ';
  const problems = [];
  for (let r = 0; r < grid.length; r++) {
    for (let c = 0; c < grid[r].length; c++) {
      if (at(r, c) !== '┌') continue;
      // Top edge: run right to the matching corner. The edge may carry a title
      // (┌── Linux device ──┐), so anything but another corner is part of it.
      let right = c + 1;
      while (right < grid[r].length && !'┐┌'.includes(at(r, right))) right++;
      if (at(r, right) !== '┐') continue; // not a box (e.g. a corner of a line)
      if (!HORZ.has(at(r, c + 1)) || !HORZ.has(at(r, right - 1))) continue; // corners must start and end a line
      // Left side: run down to the bottom-left corner.
      let bottom = r + 1;
      while (VERT.has(at(bottom, c))) bottom++;
      // A branch drawn with corners (┌──┴──┐ over arrows) is not a box: its sides end in arrows.
      if (/[▼▲]/.test(at(bottom, c))) continue;
      if (at(bottom, c) !== '└') {
        problems.push(`box at line ${r + 1}, col ${c + 1}: left side breaks at line ${bottom + 1} ('${at(bottom, c)}')`);
        continue;
      }
      for (let k = r + 1; k < bottom; k++) {
        if (!VERT.has(at(k, right))) {
          const near = [-2, -1, 1, 2].find((d) => VERT.has(at(k, right + d)));
          problems.push(
            `box at line ${r + 1}, col ${c + 1}: right side at line ${k + 1} is '${at(k, right)}'` +
              (near ? ` (the border is ${Math.abs(near)} column${Math.abs(near) > 1 ? 's' : ''} ${near < 0 ? 'left' : 'right'})` : ''),
          );
        }
      }
      if (at(bottom, right) !== '┘') problems.push(`box at line ${r + 1}, col ${c + 1}: bottom-right corner is '${at(bottom, right)}'`);
    }
  }
  // An arrowhead (▼ ▲) must sit in the column of the stroke it ends, not next to it.
  const stroke = (r, c) => '│┬┴├┤┼'.includes(at(r, c));
  for (let r = 0; r < grid.length; r++) {
    for (let c = 0; c < grid[r].length; c++) {
      const from = at(r, c) === '▼' ? r - 1 : at(r, c) === '▲' ? r + 1 : -1;
      if (from < 0 || stroke(from, c)) continue;
      if (stroke(from, c - 1) || stroke(from, c + 1)) {
        problems.push(`arrow '${at(r, c)}' at line ${r + 1}, col ${c + 1} is one column off the line it ends`);
      }
    }
  }
  return problems;
}

export function diagramsIn(markdown) {
  const lines = markdown.split('\n');
  const out = [];
  for (let i = 0; i < lines.length; i++) {
    const open = /^( *)(`{3,}|~{3,})/.exec(lines[i]);
    if (!open) continue;
    let j = i + 1;
    while (j < lines.length && !new RegExp(`^ *${open[2][0] === '`' ? '`' : '~'}{${open[2].length},}\\s*$`).test(lines[j])) j++;
    const body = lines.slice(i + 1, j).map((l) => l.slice(open[1].length));
    if (body.some((l) => l.includes('┌'))) out.push({ line: i + 2, body });
    i = j;
  }
  return out;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const files = process.argv.length > 2
    ? process.argv.slice(2)
    : YAML.parse(readFileSync(join(repo, 'site/docs-map.yaml'), 'utf8')).pages.map((p) => p.source);
  let count = 0;
  for (const file of files) {
    const text = readFileSync(resolve(repo, file), 'utf8');
    for (const d of diagramsIn(text)) {
      for (const p of lintDiagram(d.body)) {
        const m = /line (\d+)/.exec(p);
        console.log(`${file}:${d.line + Number(m[1]) - 1}: ${p.replace(/line (\d+)/g, (_, n) => `line ${d.line + Number(n) - 1}`)}`);
        count++;
      }
    }
  }
  if (count) {
    console.error(`${count} diagram problem${count > 1 ? "s" : ""}`);
    process.exit(1);
  }
  console.log('diagrams: every box border lines up');
}
