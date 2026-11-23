import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { canOpenInEditor, checkUpload } from './upload';

const repo = join(__dirname, '..', '..', '..');
const read = (...path: string[]) => readFileSync(join(repo, ...path), 'utf8');

describe('checkUpload', () => {
  it('accepts a valid connector file and names it after the file', () => {
    const check = checkUpload('modbus.toml', read('demo', 'config', 'modbus.toml'));
    expect(check).toEqual({ source: 'modbus.toml', fileName: 'modbus', status: 'ready', reason: '' });
  });

  it('reports a syntax error with its line', () => {
    const check = checkUpload('broken.toml', '[connector]\nprotocol = "modbus\n');
    expect(check.status).toBe('syntax');
    expect(check.reason).toMatch(/^Syntax error on line 2/);
    expect(canOpenInEditor(check)).toBe(true);
  });

  it('reports schema problems with the first one', () => {
    const check = checkUpload('pump.toml', '[connector]\nprotocol = "modbus"\nlog_level = "loud"\n');
    expect(check.status).toBe('problems');
    expect(check.reason).toMatch(/^1 problem, for example connector › log_level/);
  });

  it('recognises a point library', () => {
    const check = checkUpload('demo-sim.toml', read('demo', 'points.d', 'modbus', 'demo-sim.toml'));
    expect(check.status).toBe('library');
    expect(canOpenInEditor(check)).toBe(false);
  });

  it('refuses a file that is not a connector file', () => {
    expect(checkUpload('tedge.toml', '[mqtt]\nport = 1883\n').status).toBe('not-connector');
  });

  it('refuses a file name the connector directory cannot hold', () => {
    const check = checkUpload('line 3.toml', '[connector]\nprotocol = "modbus"\n');
    expect(check.status).toBe('bad-name');
    expect(check.fileName).toBe('line 3');
  });
});
