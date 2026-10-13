#!/usr/bin/env node
// Print the shell commands of a guide, in order: every line of its ```sh code blocks. The docs
// smoke test (scripts/smoke-guide.sh) runs them, so a guide whose commands stop working fails CI.
import { readFileSync } from 'node:fs';

const [file] = process.argv.slice(2);
const text = readFileSync(file, 'utf8');
for (const m of text.matchAll(/^([ \t]*)```(?:sh|bash|shell)[^\n]*\n([\s\S]*?)^\1```/gm)) {
  const indent = m[1].length;
  for (const line of m[2].split('\n')) {
    const cmd = line.slice(indent).trimEnd();
    if (cmd && !cmd.startsWith('#')) console.log(cmd);
  }
}
