import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { parseToml } from '../generated/config-builder';
import { copyFileName, duplicateText } from './duplicate';
import { commentLines } from './form-model';

const demo = (name: string) => readFileSync(join(__dirname, '..', '..', '..', 'demo', 'config', name), 'utf8');

describe('duplicate', () => {
  it('names the copy <name>-copy, with a number when that is taken', () => {
    expect(copyFileName('line-3', ['line-3'])).toBe('line-3-copy');
    expect(copyFileName('line-3', ['line-3', 'line-3-copy'])).toBe('line-3-copy_2');
  });

  it('gives a connector file copy its own service name and keeps the rest of the text', () => {
    const text = demo('opcua.toml');
    const copy = duplicateText('connector', text, 'opcua-copy');
    expect(parseToml(copy)['connector'].service_name).toBe('tedge-dot-opcua-copy');
    expect(commentLines(copy)).toBe(commentLines(text));
    const changed = copy.split('\n').filter(line => !text.split('\n').includes(line));
    expect(changed).toEqual([expect.stringMatching(/service_name\s*=\s*"tedge-dot-opcua-copy"/)]);
  });

  it('adds the service name to a connector file without one', () => {
    const copy = duplicateText('connector', '[connector]\nprotocol = "modbus"\n', 'pumps-copy');
    expect(parseToml(copy)['connector']).toEqual({ protocol: 'modbus', service_name: 'tedge-dot-pumps-copy' });
  });

  it('copies a point library, and a file that does not parse, unchanged', () => {
    const library = '[library]\nprotocol = "modbus"\n';
    expect(duplicateText('library', library, 'acme-copy')).toBe(library);
    expect(duplicateText('connector', '[connector\n', 'x-copy')).toBe('[connector\n');
  });
});
