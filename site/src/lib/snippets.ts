// Select part of a repository file for a guide, by marker comments rather than line numbers,
// so the guide follows the file when lines move.
//
// A marker is a comment line that contains `site:start <name>` or `site:end <name>`, in the
// file's own comment syntax:
//
//     # site:start device
//     [[device]]
//     ...
//     # site:end device
//
// The marker lines are never shown, also not when the whole file is included.

const MARKER = /site:(start|end)\s+([\w-]+)/;

export function selectSnippet(text: string, name?: string, file = 'file'): string {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  if (!name) return dedent(lines.filter((l) => !MARKER.test(l))).join('\n').trimEnd() + '\n';

  const start = lines.findIndex((l) => matches(l, 'start', name));
  if (start < 0) throw new Error(`${file}: no "site:start ${name}" marker`);
  const end = lines.findIndex((l, i) => i > start && matches(l, 'end', name));
  if (end < 0) throw new Error(`${file}: no "site:end ${name}" marker after its start`);
  const body = lines.slice(start + 1, end).filter((l) => !MARKER.test(l));
  return dedent(body).join('\n').trimEnd() + '\n';
}

function matches(line: string, kind: string, name: string): boolean {
  const m = MARKER.exec(line);
  return !!m && m[1] === kind && m[2] === name;
}

function dedent(lines: string[]): string[] {
  const indents = lines.filter((l) => l.trim()).map((l) => /^[ \t]*/.exec(l)![0].length);
  const min = indents.length ? Math.min(...indents) : 0;
  return lines.map((l) => l.slice(min));
}
