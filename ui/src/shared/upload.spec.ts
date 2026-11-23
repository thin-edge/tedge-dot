import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { canOpenInEditor, checkUpload } from './upload';

const repo = join(__dirname, '..', '..', '..');
const read = (...path: string[]) => readFileSync(join(repo, ...path), 'utf8');

describe('checkUpload', () => {
  it('accepts a valid connector file and names it after the file', () => {
    const check = checkUpload('modbus.toml', read('demo', 'config', 'modbus.toml'));
    expect(check).toMatchObject({ source: 'modbus.toml', fileName: 'modbus', status: 'ready', reason: '' });
    expect(check.file?.type).toBe('tedge-dot-modbus');
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

  it('adds a point library as a library of its protocol', () => {
    const check = checkUpload('acme-meter.toml', read('demo', 'points.d', 'modbus', 'demo-sim.toml'));
    expect(check.status).toBe('ready');
    expect(check.file?.type).toBe('tedge-dot-points.modbus.acme-meter');
    expect(check.kind).toBe('library');
  });

  it('tells a broken library from a broken connector file', () => {
    expect(checkUpload('x.toml', '[library]\nprotocol = "modbus\n').kind).toBe('library');
    expect(checkUpload('x.toml', '[connector]\nprotocol = "modbus\n').kind).toBe('connector');
    const noProtocol = checkUpload('x.toml', '[library]\ntype = "meter"\n');
    expect([noProtocol.status, noProtocol.kind, canOpenInEditor(noProtocol)]).toEqual(['problems', 'library', true]);
  });

  it('refuses a connector file name reserved for libraries', () => {
    expect(checkUpload('points.x.toml', read('demo', 'config', 'modbus.toml')).status).toBe('bad-name');
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
