import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { parseToml, type Config } from '../generated/config-builder';
import {
  LibraryCatalog,
  checkReferences,
  isPathReference,
  mergePoint,
  packagedLibraries,
  referencedLibraries,
  referencedLibraryFiles,
  resolveDevice,
} from './libraries';
import { commentLines } from './form-model';
import { emitLibraryToml, sameConfig, writeKeepingLayout } from './toml-write';
import { validateLibraryFile } from './validation';

// The library of impl/rust/crates/sdk/src/library.rs's tests, so the cases match tedge-dot's.
const BASE = `[library]
protocol = "modbus"
type     = "acme-meter-v2"

[[point]]
id       = "boiler_temp"
datatype = "float32"
unit     = "°C"
address  = { table = "holding", address = 7, count = 2 }
meta     = { parameter = { title = "Boiler", group = "heat" } }

[[point]]
id       = "pump_run"
datatype = "bool"
address  = { table = "coil", address = 1 }
`;

const library = (protocol: string, name: string, text: string) => ({
  protocol,
  name,
  source: 'repository' as const,
  config: parseToml(text),
});

const catalog = new LibraryCatalog([
  library('modbus', 'base', BASE),
  library('modbus', 'extras', '[library]\nprotocol = "modbus"\n\n[[point]]\nid = "site_extra"\ndatatype = "uint16"\naddress = { table = "holding", address = 99, count = 1 }\n'),
  library('modbus', 'tweaks', '[library]\nprotocol = "modbus"\ntype = "ignored-later-type"\n\n[[point]]\nid = "boiler_temp"\npoll_interval = "30s"\nmeta = { parameter = { title = "Boiler 2" } }\n'),
  ...packagedLibraries(),
]);

const device = (refs: string[], points: Config[] = [], extra: Config = {}): Config => ({ name: 'plc1', points_from: refs, point: points, ...extra });

describe('resolveDevice', () => {
  it('applies several libraries in order', () => {
    const r = resolveDevice(device(['base', 'extras']), 'modbus', catalog);
    expect(r.inherited.map(p => [p.point['id'], p.library])).toEqual([
      ['boiler_temp', 'base'],
      ['pump_run', 'base'],
      ['site_extra', 'extras'],
    ]);
  });

  it('lets a later library patch a point it repeats, keeping what it does not restate', () => {
    const r = resolveDevice(device(['base', 'tweaks']), 'modbus', catalog);
    expect(r.inherited).toHaveLength(2);
    const boiler = r.inherited[0].point;
    expect(boiler['poll_interval']).toBe('30s');
    expect(boiler['unit']).toBe('°C');
    expect(boiler['address']).toEqual({ table: 'holding', address: 7, count: 2 });
    expect(boiler['meta']).toEqual({ parameter: { title: 'Boiler 2', group: 'heat' } });
  });

  it('lets the device extend and override the libraries', () => {
    const own = [
      { id: 'boiler_temp', unit: 'K' },
      { id: 'local_only', datatype: 'uint16', address: { table: 'holding', address: 42, count: 1 } },
    ];
    const r = resolveDevice(device(['base'], own), 'modbus', catalog);
    expect(r.inherited.map(p => p.point['id'])).toEqual(['pump_run']);
    expect([...r.overrides]).toEqual([[0, 'base']]);
  });

  it('inherits the type of the first library that names one, unless the device has its own', () => {
    expect(resolveDevice(device(['extras', 'base', 'tweaks']), 'modbus', catalog).type).toEqual({ name: 'acme-meter-v2', library: 'base' });
    expect(resolveDevice(device(['base'], [], { type: 'own' }), 'modbus', catalog).type).toBeUndefined();
  });

  it('reports references no library resolves, and looks only under the file protocol', () => {
    expect(resolveDevice(device(['base', 'nope']), 'modbus', catalog).missing).toEqual(['nope']);
    expect(resolveDevice(device(['base']), 'opcua', catalog).missing).toEqual(['base']);
  });

  it('resolves a packaged library', () => {
    const r = resolveDevice(device(['demo-sim']), 'modbus', catalog);
    expect(r.missing).toEqual([]);
    expect(r.inherited.length).toBeGreaterThan(3);
  });
});

describe('mergePoint', () => {
  it('merges meta recursively and replaces map and address whole', () => {
    const base = { id: 'p', address: { table: 'holding', address: 1 }, map: { '0': 'off', '1': 'on' }, meta: { a: { b: 1, c: 2 } } };
    const merged = mergePoint(base, { address: { table: 'input', address: 5 }, map: { '2': 'fault' }, meta: { a: { c: 3 } } });
    expect(merged['address']).toEqual({ table: 'input', address: 5 });
    expect(merged['map']).toEqual({ '2': 'fault' });
    expect(merged['meta']).toEqual({ a: { b: 1, c: 3 } });
    expect(base.meta.a.c).toBe(2);
  });
});

describe('references', () => {
  const file = (refs: unknown[]) => ({ connector: { protocol: 'modbus' }, device: [{ name: 'plc1', points_from: refs }] });

  it('accepts names of repository and packaged libraries', () => {
    expect(checkReferences(file(['base', 'demo-sim']), catalog)).toEqual([]);
  });

  it('refuses unknown names and paths at the device', () => {
    const problems = checkReferences(file(['acme-meter-v3', 'libs/acme.toml']), catalog);
    expect(problems.map(p => [p.path, p.message])).toEqual([
      ['/device/0/points_from', 'no modbus point library is named "acme-meter-v3"'],
      ['/device/0/points_from', '"libs/acme.toml" is a path; reference a point library by its name'],
    ]);
    expect(problems[0].label).toBe('device "plc1" › points_from');
  });

  it('lists the referenced names once, without paths', () => {
    const config = { device: [{ points_from: ['base', 'x.toml'] }, { points_from: ['base', 'extras'] }] };
    expect(referencedLibraries(config)).toEqual(['base', 'extras']);
    expect(isPathReference('a/b')).toBe(true);
    expect(isPathReference('demo-sim')).toBe(false);
  });

  it('sends only the referenced libraries the repository holds', () => {
    const config = { connector: { protocol: 'modbus' }, device: [{ points_from: ['demo-sim', 'base', 'gone'] }] };
    const files = referencedLibraryFiles(config, ['tedge-dot-points.modbus.base', 'tedge-dot-points.opcua.gone']);
    expect(files.map(f => f.type)).toEqual(['tedge-dot-points.modbus.base']);
  });
});

describe('LibraryCatalog', () => {
  it('lets a repository library shadow a packaged one of the same name', () => {
    const own = new LibraryCatalog([...packagedLibraries(), library('modbus', 'demo-sim', BASE)]);
    expect(own.get('modbus', 'demo-sim')?.source).toBe('repository');
    expect(own.names('modbus')).toEqual(['demo-sim']);
    expect(new LibraryCatalog(packagedLibraries()).names('opcua')).toEqual(['demo-sim']);
  });
});

describe('libraries in the editor', () => {
  const demo = readFileSync(join(__dirname, '..', '..', '..', 'demo', 'points.d', 'modbus', 'demo-sim.toml'), 'utf8');

  it('validates a library with the point-library schema', () => {
    const ok = validateLibraryFile(demo);
    expect(ok.syntax).toBeUndefined();
    expect(ok.fields).toEqual([]);
    const bad = validateLibraryFile('[library]\nprotocol = "modbus"\n\n[[point]]\nid = "a"\ndatatype = "float33"\naddress = { table = "holding", address = 0, count = 1 }\n');
    expect(bad.fields.map(f => f.label)).toContain('point "a" › datatype');
  });

  it('refuses a point id repeated within one library', () => {
    const text = BASE + '\n[[point]]\nid = "pump_run"\ndatatype = "bool"\naddress = { table = "coil", address = 2 }\n';
    expect(validateLibraryFile(text).fields).toContainEqual({ path: '/point/2/id', message: 'repeats the point id "pump_run" of point 2', label: 'point "pump_run" › id' });
  });

  it('adds a point at the end of a commented library and keeps its comments', () => {
    const before = parseToml(demo);
    const next = { ...before, point: [...before['point'], { id: 'new_point', datatype: 'uint16', access: 'read', address: { table: 'holding', address: 90, count: 1 } }] };
    const written = writeKeepingLayout(demo, next);
    expect(written).toBeDefined();
    expect(sameConfig(parseToml(written!), next)).toBe(true);
    expect(commentLines(written!)).toBe(commentLines(demo));
    expect(written!.trimEnd().split('\n').slice(-5)[0]).toBe('[[point]]');
  });

  it('writes a new library in the config builder layout', () => {
    const text = emitLibraryToml({ library: { protocol: 'modbus' }, point: [{ id: 't', datatype: 'uint16', address: { table: 'holding', address: 0, count: 1 } }] });
    expect(text).toMatch(/^\[library\]\nprotocol = "modbus"\n\n\[\[point\]\]\nid {7}= "t"/);
    expect(validateLibraryFile(text).fields).toEqual([]);
  });
});
