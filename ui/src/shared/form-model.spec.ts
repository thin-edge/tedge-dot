import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { emitToml, getAt, newConfig, parseToml, protocolInfo, setAt } from '../generated/config-builder';
import {
  addDevice,
  addPoint,
  commentLines,
  displayValue,
  extraKeys,
  pointSummary,
  removeDevice,
  removePoint,
  rewriteChanges,
  shapeFields,
  splitList,
  switchProtocol,
  uniqueId,
} from './form-model';
import { validateConnectorFile } from './validation';

const repo = join(__dirname, '..', '..', '..');
const packaged = (name: string) => readFileSync(join(repo, 'packaging', 'config', name), 'utf8');

describe('rewriteChanges and commentLines', () => {
  it('a file in the builder layout is written back unchanged', () => {
    const text = emitToml(newConfig('modbus'));
    expect(rewriteChanges(text)).toBe(false);
    expect(commentLines(text)).toBe(0);
  });

  it('a packaged file with comments would be rewritten', () => {
    const text = packaged('opcua.toml');
    expect(rewriteChanges(text)).toBe(true);
    expect(commentLines(text)).toBeGreaterThan(5);
  });

  it('a file in its own layout would be rewritten, though it has no comments', () => {
    const text = '[connector]\nprotocol      = "modbus"\n\n[mqtt]\nhost = "127.0.0.1"\nport = 1883\n';
    expect(rewriteChanges(text)).toBe(true);
    expect(commentLines(text)).toBe(0);
  });

  it('counts whole-line and trailing comments, not # inside strings', () => {
    const text = [
      '# a comment',
      'id = "temp#1"          # trailing',
      "node = 'ns=2;s=#x'",
      'path = "a\\"#b"',
      'doc = """',
      '# inside a multi-line string',
      '"""',
      '  # indented',
    ].join('\n');
    expect(commentLines(text)).toBe(3);
  });
});

describe('shapeFields and extraKeys', () => {
  it('turns a protocol shape into fields', () => {
    const fields = shapeFields(protocolInfo('modbus').protocol_address);
    const byKey = Object.fromEntries(fields.map(f => [f.key, f]));
    expect(byKey['transport'].kind).toBe('choice');
    expect(byKey['transport'].options).toContain('tcp');
    expect(byKey['port'].kind).toBe('integer');
    expect(byKey['host'].kind).toBe('text');
  });

  it('a shape without properties has no fields', () => {
    expect(shapeFields({ type: 'object' })).toEqual([]);
    expect(shapeFields(undefined)).toEqual([]);
  });

  it('lists the keys the form has no field for', () => {
    expect(extraKeys({ host: 'a', vendor_flag: true }, ['host', 'port'])).toEqual(['vendor_flag']);
    expect(extraKeys(undefined, ['host'])).toEqual([]);
  });
});

describe('devices and points', () => {
  it('adds a device from the template with the next free name', () => {
    const config = addDevice(newConfig('modbus'));
    expect(config.device.map((d: any) => d.name)).toEqual(['plc1', 'plc2']);
    expect(config.device[1].protocol_address).toEqual(protocolInfo('modbus').newDevice.protocol_address);
  });

  it('adds a point with an id no other point on the device has', () => {
    let config = newConfig('opcua');
    config = addPoint(config, 0);
    config = addPoint(config, 0);
    expect(config.device[0].point.map((p: any) => p.id)).toEqual(['temperature', 'temperature_2', 'temperature_3']);
  });

  it('removes a device and a point', () => {
    let config = addPoint(addDevice(newConfig('modbus')), 0);
    config = removePoint(config, 0, 0);
    expect(config.device[0].point.map((p: any) => p.id)).toEqual(['temperature_2']);
    config = removeDevice(config, 0);
    expect(config.device.map((d: any) => d.name)).toEqual(['plc2']);
  });
});

describe('switchProtocol', () => {
  it('gives devices and points the new protocol addresses and keeps ids and policies', () => {
    const modbus = setAt(newConfig('modbus'), '/device/0/point/0/report', { on_change: true });
    const opcua = switchProtocol(modbus, 'opcua');
    expect(opcua.connector.protocol).toBe('opcua');
    expect(opcua.connector.service_name).toBe('tedge-dot-opcua');
    expect(opcua.device[0].name).toBe('plc1');
    expect(opcua.device[0].protocol_address).toEqual(protocolInfo('opcua').newDevice.protocol_address);
    expect(opcua.device[0].point[0].id).toBe('temperature');
    expect(opcua.device[0].point[0].report).toEqual({ on_change: true });
    expect(opcua.device[0].point[0].address).toEqual(protocolInfo('opcua').newPoint.address);
    expect(opcua.device[0].point[0].datatype).toBe(protocolInfo('opcua').newPoint.datatype);
    expect(validateConnectorFile(emitToml(opcua)).fields).toEqual([]);
  });

  it('keeps a service name that is not the old default, and drops the old [connection]', () => {
    const config = { ...newConfig('modbus'), connector: { protocol: 'modbus', service_name: 'line-3' }, connection: { timeout: '1s' } };
    const next = switchProtocol(config, 'opcua');
    expect(next.connector.service_name).toBe('line-3');
    expect(next['connection']).toBeUndefined();
  });

  it('drops OPC UA only keys when leaving OPC UA', () => {
    const config = setAt(setAt(newConfig('opcua'), '/connector/sampling_interval', '1s'), '/device/0/point/0/subscribe', false);
    const next = switchProtocol(config, 'modbus');
    expect(next.connector.sampling_interval).toBeUndefined();
    expect(next.device[0].point[0].subscribe).toBeUndefined();
  });
});

describe('form edits keep what the form does not show', () => {
  it('a point keeps its meta table when its unit changes', () => {
    const text = [
      '[connector]',
      'protocol = "opcua"',
      '',
      '[[device]]',
      'name = "Pump01"',
      'protocol_address = { endpoint = "opc.tcp://127.0.0.1:4840/" }',
      '',
      '  [[device.point]]',
      '  id = "flow"',
      '  datatype = "float64"',
      '  address = { node_id = "ns=2;s=Pump01.Flow" }',
      '  meta = { measurement = { fragment = "process" } }',
      '',
    ].join('\n');
    const edited = emitToml(setAt(parseToml(text), '/device/0/point/0/unit', 'm3/h'));
    const again = parseToml(edited);
    expect(getAt(again, '/device/0/point/0/unit')).toBe('m3/h');
    expect(getAt(again, '/device/0/point/0/meta')).toEqual({ measurement: { fragment: 'process' } });
  });

  it('a port typed in the form is written as a number', () => {
    const config = setAt(newConfig('modbus'), '/device/0/protocol_address/port', 5020);
    expect(emitToml(config)).toMatch(/port\s*=\s*5020\b/);
  });
});

describe('small helpers', () => {
  it('uniqueId, splitList and displayValue', () => {
    expect(uniqueId('a', ['b'])).toBe('a');
    expect(uniqueId('a', ['a', 'a_2'])).toBe('a_3');
    expect(splitList(' demo-sim, , other ')).toEqual(['demo-sim', 'other']);
    expect(displayValue(undefined)).toBe('');
    expect(displayValue(5020)).toBe('5020');
    expect(displayValue({ a: 1 })).toBe('{"a":1}');
  });

  it('pointSummary says how a point is read', () => {
    const opcua = newConfig('opcua');
    expect(pointSummary(opcua, opcua.device[0], opcua.device[0].point[0])).toBe('float64 · subscribed');
    const modbus = newConfig('modbus');
    expect(pointSummary(modbus, { poll_interval: '5s' }, { datatype: 'uint16', report: {} })).toBe('uint16 · polled 5s · own report');
  });
});
