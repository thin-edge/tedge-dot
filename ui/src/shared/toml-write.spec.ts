import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { getAt, parseToml, setAt, type Config } from '../generated/config-builder';
import { addDevice, addPoint, commentLines, removeDevice, removePoint, switchProtocol } from './form-model';
import { sameConfig, writeKeepingLayout } from './toml-write';
import { validateConnectorFile } from './validation';

const repo = join(__dirname, '..', '..', '..');
const read = (...path: string[]) => readFileSync(join(repo, ...path), 'utf8');
const demo = (name: string) => read('demo', 'config', name);
const packaged = (name: string) => read('packaging', 'config', name);

/** Applies a form edit and checks the written text: same values as the edit, comments kept. */
function edit(text: string, change: (config: Config) => Config, keptComments = commentLines(text)): string {
  const next = change(parseToml(text));
  const written = writeKeepingLayout(text, next);
  expect(written).toBeDefined();
  expect(sameConfig(parseToml(written!), next)).toBe(true);
  expect(commentLines(written!)).toBe(keptComments);
  return written!;
}

const changedLines = (a: string, b: string) => {
  const before = a.split('\n');
  return b.split('\n').filter(l => !before.includes(l));
};

describe('writeKeepingLayout', () => {
  for (const name of ['opcua.toml', 'modbus.toml', 'snmp.toml', 'canbus.toml']) {
    describe(`demo/config/${name}`, () => {
      const text = demo(name);

      it('changes one value in place', () => {
        const written = edit(text, c => setAt(c, '/connector/poll_interval', '7s'));
        expect(changedLines(text, written)).toEqual([expect.stringMatching(/poll_interval\s*=\s*"7s"/)]);
      });

      it('adds and removes a key', () => {
        const added = edit(text, c => setAt(c, '/connector/max_inflight', 8));
        expect(getAt(parseToml(added), '/connector/max_inflight')).toBe(8);
        edit(added, c => setAt(c, '/connector/max_inflight', undefined));
      });

      it('adds a point to the device', () => {
        const written = edit(text, c => addPoint(c, 0));
        expect(validateConnectorFile(written).syntax).toBeUndefined();
      });

      it('adds a device', () => {
        const written = edit(text, c => addDevice(c));
        expect(parseToml(written)['device']).toHaveLength(2);
      });

      it('changes a value in a point', () => {
        edit(text, c => setAt(c, '/device/0/point/0/unit', 'test-unit'));
      });
    });
  }

  it('removes a point and a device', () => {
    const text = demo('opcua.toml');
    const twoPoints = edit(text, c => addPoint(c, 0));
    const onePointLess = writeKeepingLayout(twoPoints, removePoint(parseToml(twoPoints), 0, 1));
    expect(onePointLess).toBeDefined();
    const twoDevices = edit(text, c => addDevice(c));
    const back = writeKeepingLayout(twoDevices, removeDevice(parseToml(twoDevices), 1));
    expect(back).toBeDefined();
    expect(parseToml(back!)['device']).toHaveLength(1);
  });

  it('adds the first device to a file without devices', () => {
    const text = packaged('modbus.toml');
    const written = edit(text, c => addDevice(c));
    expect(parseToml(written)['device']).toHaveLength(1);
    expect(written.startsWith(text.trimEnd().split('\n')[0])).toBe(true);
  });

  it('switches the protocol and keeps the comments', () => {
    edit(demo('modbus.toml'), c => switchProtocol(c, 'opcua'));
  });

  it('gives up on text that is not TOML', () => {
    expect(writeKeepingLayout('[connector\n', { connector: { protocol: 'modbus' } })).toBeUndefined();
  });
});

describe('sameConfig', () => {
  it('ignores key order and undefined values', () => {
    expect(sameConfig({ a: 1, b: { c: [1, 2] } }, { b: { c: [1, 2] }, a: 1, d: undefined })).toBe(true);
    expect(sameConfig({ a: [1, 2] }, { a: [2, 1] })).toBe(false);
  });
});
