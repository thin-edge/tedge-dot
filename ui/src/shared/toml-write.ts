/**
 * Writing a form edit back into a connector file's text, keeping its comments and layout.
 *
 * `@decimalturn/toml-patch` applies the difference between the file and the edited configuration
 * to the text: changed values, added and removed keys, removed devices and points. It does not
 * place new `[[device]]` and `[[device.point]]` tables correctly (3.3.2), so new devices and
 * points are written here first, in the config builder's layout, at the end of their device's
 * section. toml-patch appends a new key below a table's trailing comments and later treats the
 * comments right above a key as the key's own, so a blank line goes between them. The result
 * counts only when it parses to exactly the edited configuration; otherwise the caller decides
 * whether to rewrite the whole file.
 */
import { patch } from '@decimalturn/toml-patch';

import { emitToml, parseToml, type Config } from '../generated/config-builder';

/** The file's text with `next` written into it, or undefined when that cannot keep the layout. */
export function writeKeepingLayout(text: string, next: Config): string | undefined {
  let current: Config;
  try {
    current = parseToml(text);
  } catch {
    return undefined;
  }
  const withTables = addNewTables(text, current, next);
  if (withTables === undefined) {
    return undefined;
  }
  let written: string;
  try {
    written = separateInsertedKeys(withTables, patch(withTables, next));
  } catch {
    return undefined;
  }
  try {
    return sameConfig(parseToml(written), next) ? written : undefined;
  } catch {
    return undefined;
  }
}

/** Whether two configurations hold the same values, whatever the order of their keys. */
export function sameConfig(a: unknown, b: unknown): boolean {
  return canonical(a) === canonical(b);
}

function canonical(value: unknown): string {
  if (value instanceof Date) {
    return JSON.stringify(value.toISOString());
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonical).join(',')}]`;
  }
  if (value && typeof value === 'object') {
    const entries = Object.entries(value as Record<string, unknown>)
      .filter(([, v]) => v !== undefined)
      .sort(([x], [y]) => (x < y ? -1 : x > y ? 1 : 0));
    return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${canonical(v)}`).join(',')}}`;
  }
  return JSON.stringify(value);
}

const isComment = (line: string | undefined) => !!line && line.trim().startsWith('#');

/**
 * `after` with a blank line before each run of lines that toml-patch inserted (not changed)
 * right below a comment, so the comment does not become part of the new key.
 */
function separateInsertedKeys(before: string, after: string): string {
  const a = before.split('\n');
  const b = after.split('\n');
  // Longest common subsequence of lines, to tell inserted lines from changed ones.
  const lcs: number[][] = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  type Op = { kind: 'keep' | 'insert' | 'delete'; line?: string };
  const ops: Op[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      ops.push({ kind: 'keep', line: b[j++] });
      i++;
    } else if (j < b.length && (i === a.length || lcs[i][j + 1] >= lcs[i + 1][j])) {
      ops.push({ kind: 'insert', line: b[j++] });
    } else {
      ops.push({ kind: 'delete' });
      i++;
    }
  }
  const out: string[] = [];
  for (let k = 0; k < ops.length; ) {
    if (ops[k].kind === 'keep') {
      out.push(ops[k++].line!);
      continue;
    }
    const hunk: Op[] = [];
    while (k < ops.length && ops[k].kind !== 'keep') {
      hunk.push(ops[k++]);
    }
    const inserted = hunk.filter(op => op.kind === 'insert').map(op => op.line!);
    const pureInsert = hunk.every(op => op.kind === 'insert');
    if (pureInsert && isComment(out[out.length - 1]) && inserted[0]?.trim()) {
      out.push('');
    }
    out.push(...inserted);
  }
  return out.join('\n');
}

type HeaderKind = 'device' | 'point' | 'other';

interface Header {
  line: number;
  kind: HeaderKind;
}

const HEADER = /^\s*(\[\[?)\s*([A-Za-z0-9_.\-"' ]+?)\s*(\]\]?)\s*(#.*)?$/;

/** The table headers of a TOML text, skipping lines inside multi-line strings. */
function headers(lines: string[]): Header[] {
  const found: Header[] = [];
  let multiline: string | undefined;
  lines.forEach((line, index) => {
    if (multiline) {
      if (countOf(line, multiline) % 2 === 1) {
        multiline = undefined;
      }
      return;
    }
    const m = HEADER.exec(line);
    if (m) {
      const array = m[1] === '[[' && m[3] === ']]';
      const name = m[2].replace(/\s+/g, '');
      found.push({ line: index, kind: array && name === 'device' ? 'device' : array && name === 'device.point' ? 'point' : 'other' });
      return;
    }
    for (const quote of ['"""', "'''"]) {
      if (countOf(line, quote) % 2 === 1) {
        multiline = quote;
      }
    }
  });
  return found;
}

function countOf(line: string, part: string): number {
  return line.split(part).length - 1;
}

/** The line after which a section's content ends: its last line that is not blank or a comment. */
function lastContentLine(lines: string[], from: number, to: number): number {
  for (let i = to - 1; i > from; i--) {
    const trimmed = lines[i].trim();
    if (trimmed && !trimmed.startsWith('#')) {
      return i;
    }
  }
  return from;
}

/** The last line of a section that is not blank: a new device goes after its comments too. */
function lastNonBlankLine(lines: string[], from: number, to: number): number {
  for (let i = to - 1; i > from; i--) {
    if (lines[i].trim()) {
      return i;
    }
  }
  return from;
}

/** The lines of a device's `[[device]]` section: from its header to the next non-point header. */
function deviceSection(lines: string[], found: Header[], d: number): { start: number; end: number } | undefined {
  const devices = found.filter(h => h.kind === 'device');
  const header = devices[d];
  if (!header) {
    return undefined;
  }
  const next = found.find(h => h.line > header.line && h.kind !== 'point');
  return { start: header.line, end: next ? next.line : lines.length };
}

function blockLines(config: Config, marker: string): string[] {
  const lines = emitToml(config).replace(/\n$/, '').split('\n');
  const start = lines.findIndex(l => l === marker);
  return start < 0 ? [] : lines.slice(start);
}

const pointBlock = (point: Config) => blockLines({ device: [{ point: [point] }] }, '  [[device.point]]');
const deviceBlock = (device: Config) => blockLines({ device: [device] }, '[[device]]');

/** A library's `[[point]]` table in the config builder's layout, without the device indent. */
export function libraryPointBlock(point: Config): string[] {
  return pointBlock(point).map((line, i) => (i === 0 ? '[[point]]' : line.replace(/^ {2}/, '')));
}

/** A point library in the config builder's layout: `[library]`, then one `[[point]]` per point. */
export function emitLibraryToml(library: Config): string {
  const { point, ...rest } = library;
  const lines = emitToml(rest).replace(/\n$/, '').split('\n');
  for (const p of (Array.isArray(point) ? point : []) as Config[]) {
    lines.push('', ...libraryPointBlock(p));
  }
  return lines.join('\n') + '\n';
}

/**
 * The top-level tables, such as `[connection]`, that `next` has and `current` does not, in the
 * config builder's layout and order, separated by blank lines. toml-patch would append them
 * after the last `[[device.point]]`, indented like it.
 */
function newTables(current: Config, next: Config): string[] {
  const added = Object.fromEntries(
    Object.entries(next).filter(
      ([key, value]) => current[key] === undefined && key !== 'device' && value && typeof value === 'object' && !Array.isArray(value)
    )
  );
  if (!Object.keys(added).length) {
    return [];
  }
  return emitToml(added).replace(/\n$/, '').split('\n');
}

/**
 * The text with the top-level tables `next` has beyond `current` above the first device, and the
 * tables for the devices and points it has beyond `current` appended where they belong. Only
 * additions at the end are handled, which is what the form does.
 */
function addNewTables(text: string, current: Config, next: Config): string | undefined {
  // A point library: new `[[point]]` tables go at the end of the file.
  if (next['library'] !== undefined && Array.isArray(next['point'])) {
    const have: Config[] = Array.isArray(current['point']) ? current['point'] : [];
    const want: Config[] = next['point'];
    if (want.length <= have.length) {
      return text;
    }
    const lines = text.split('\n');
    const after = lastNonBlankLine(lines, -1, lines.length);
    lines.splice(after + 1, 0, ...want.slice(have.length).flatMap(p => ['', ...libraryPointBlock(p)]));
    return lines.join('\n');
  }
  const currentDevices: Config[] = Array.isArray(current['device']) ? current['device'] : [];
  const nextDevices: Config[] = Array.isArray(next['device']) ? next['device'] : [];
  const lines = text.split('\n');
  const inserts: { after: number; block: string[] }[] = [];

  const shared = Math.min(currentDevices.length, nextDevices.length);
  for (let d = 0; d < shared; d++) {
    const have: Config[] = Array.isArray(currentDevices[d]['point']) ? currentDevices[d]['point'] : [];
    const want: Config[] = Array.isArray(nextDevices[d]['point']) ? nextDevices[d]['point'] : [];
    if (want.length <= have.length) {
      continue;
    }
    const section = deviceSection(lines, headers(lines), d);
    if (!section) {
      return undefined;
    }
    const after = lastContentLine(lines, section.start, section.end);
    inserts.push({ after, block: want.slice(have.length).flatMap(p => ['', ...pointBlock(p)]) });
  }

  const tables = newTables(current, next);
  if (tables.length) {
    const firstDevice = headers(lines).find(h => h.kind === 'device');
    if (firstDevice) {
      // Above the first device and the comments right above it, which belong to the device.
      let at = firstDevice.line;
      while (at > 0 && isComment(lines[at - 1])) {
        at--;
      }
      // `after` is the line the block follows; the block ends with the blank line before the device.
      inserts.push({ after: at - 1, block: [...(at > 0 && lines[at - 1].trim() ? [''] : []), ...tables, ''] });
    }
    // Without devices, toml-patch appends the tables at the end, which is where they belong.
  }

  if (nextDevices.length > currentDevices.length) {
    const found = headers(lines);
    const last = deviceSection(lines, found, currentDevices.length - 1);
    // After the last device's own trailing comments, which belong to it; at the end without devices.
    const after = last ? lastNonBlankLine(lines, last.start, last.end) : lastNonBlankLine(lines, -1, lines.length);
    inserts.push({ after, block: nextDevices.slice(currentDevices.length).flatMap(dev => ['', ...deviceBlock(dev)]) });
  }

  if (!inserts.length) {
    return text;
  }
  // From the bottom up, so earlier line numbers stay valid.
  for (const { after, block } of inserts.sort((a, b) => b.after - a.after)) {
    lines.splice(after + 1, 0, ...block);
  }
  return lines.join('\n');
}
