// Every shipped config validates against the schema (plus the builder's protocol shapes) and
// survives the builder's parse -> emit -> parse round trip with the same meaning.
import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { emitToml, newConfig, parseToml, PROTOCOL_IDS, setAt, coerce, validateConfig, validateLibrary } from '../src/lib/config-builder';

const repo = new URL('../../', import.meta.url);
const files = (dir: string) =>
  readdirSync(new URL(dir, repo))
    .filter((f) => f.endsWith('.toml'))
    .map((f) => `${dir}${f}`);

const configs = [...files('demo/config/'), ...files('packaging/config/')];
const libraries = readdirSync(new URL('demo/points.d/', repo), { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .flatMap((d) => files(`demo/points.d/${d.name}/`));

describe('shipped configs', () => {
  it('are found', () => expect(configs.length).toBeGreaterThan(8));
  for (const file of configs) {
    it(`${file} validates and round-trips through the builder`, () => {
      const parsed = parseToml(readFileSync(new URL(file, repo), 'utf8'));
      expect(validateConfig(parsed), `${file} does not validate`).toEqual([]);
      const again = parseToml(emitToml(parsed));
      expect(again, `${file} changes meaning through the builder`).toEqual(parsed);
    });
  }
  for (const file of libraries) {
    it(`${file} validates as a point library`, () => {
      const parsed = parseToml(readFileSync(new URL(file, repo), 'utf8'));
      expect(validateLibrary(parsed), `${file} does not validate`).toEqual([]);
    });
  }
});

describe('builder', () => {
  for (const protocol of PROTOCOL_IDS) {
    it(`starts a valid ${protocol} config`, () => {
      expect(validateConfig(newConfig(protocol))).toEqual([]);
    });
  }

  it('builds a Modbus device with two holding-register points (spec scenario)', () => {
    let cfg = newConfig('modbus');
    cfg = setAt(cfg, '/device/0/point/1', {
      id: 'pressure',
      datatype: 'float32',
      access: 'read',
      address: { table: 'holding', address: 2, count: 2 },
    });
    expect(validateConfig(cfg)).toEqual([]);
    expect(emitToml(cfg)).toContain('[[device.point]]');
  });

  it("flags a non-numeric device port on the field (spec scenario)", () => {
    const cfg = setAt(newConfig('modbus'), '/device/0/protocol_address/port', coerce('five-oh-two', 'integer'));
    const errors = validateConfig(cfg);
    expect(errors).toContainEqual({ path: '/device/0/protocol_address/port', message: 'must be integer' });
  });

  it('names a missing required key on its own path', () => {
    const cfg = setAt(newConfig('modbus'), '/connector/protocol', undefined);
    expect(validateConfig(cfg).map((e) => e.path)).toContain('/connector/protocol');
  });

  it('emits inline tables and aligned keys', () => {
    const toml = emitToml(newConfig('modbus'));
    expect(toml).toContain('protocol_address = { transport = "tcp", host = "192.168.0.10", port = 502, unit_id = 1 }');
    expect(toml).toMatch(/^\[connector\]\nprotocol {6}= "modbus"\nservice_name {2}= /);
  });
});
