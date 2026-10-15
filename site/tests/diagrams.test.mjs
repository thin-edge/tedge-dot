import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import YAML from 'yaml';
import { diagramsIn, lintDiagram } from '../scripts/lint-diagrams.mjs';

describe('box diagram linter', () => {
  it('accepts a straight box, also one crossed by arrows', () => {
    expect(lintDiagram(['┌──┐', '│ a├──▶', '└──┘'])).toEqual([]);
  });
  it('reports a right side one column off', () => {
    expect(lintDiagram(['┌──┐', '│ a │', '└──┘'])[0]).toContain('1 column right');
  });
  it('ignores a branch drawn with corners', () => {
    expect(lintDiagram(['──┴──', '┌──┴──┐', '│     │', '▼     ▼'])).toEqual([]);
  });
});

describe('the published docs', () => {
  const map = YAML.parse(readFileSync(new URL('../docs-map.yaml', import.meta.url), 'utf8'));
  for (const page of map.pages) {
    const text = readFileSync(new URL(`../../${page.source}`, import.meta.url), 'utf8');
    const diagrams = diagramsIn(text);
    if (!diagrams.length) continue;
    it(`${page.source}: every box border lines up`, () => {
      const problems = diagrams.flatMap((d) => lintDiagram(d.body).map((p) => `diagram at line ${d.line}: ${p}`));
      expect(problems).toEqual([]);
    });
  }
});

describe('titled boxes', () => {
  it('traces a box whose top edge has a title', () => {
    expect(lintDiagram(['┌─ title ─┐', '│         │', '│          │', '└─────────┘'])[0]).toContain('right side');
  });
});

describe('arrowheads', () => {
  it('reports an arrowhead one column off its line', () => {
    expect(lintDiagram(['  │', '   ▼'])[0]).toContain('one column off');
    expect(lintDiagram(['  │', '  ▼'])).toEqual([]);
  });
});
