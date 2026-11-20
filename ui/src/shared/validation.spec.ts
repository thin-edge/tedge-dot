import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { describePointer, isValid, validateConnectorFile } from './validation';

const repo = join(__dirname, '..', '..', '..');
const packaged = (name: string) => readFileSync(join(repo, 'packaging', 'config', name), 'utf8');

const PUMP = `[connector]
protocol     = "opcua"
service_name = "tedge-dot-opcua-pump"

[[device]]
name             = "Pump01"
protocol_address = { endpoint = "opc.tcp://127.0.0.1:4840/" }

  [[device.point]]
  id       = "flow"
  mode     = "typed"
  datatype = "float64"
  address  = { node_id = "ns=2;s=Pump01.Flow" }
`;

describe('validateConnectorFile', () => {
  it('accepts the packaged connector files', () => {
    for (const name of ['opcua.toml', 'modbus.toml', 'snmp.toml', 'canbus.toml']) {
      const result = validateConnectorFile(packaged(name));
      expect({ name, fields: result.fields, syntax: result.syntax }).toEqual({ name, fields: [], syntax: undefined });
    }
  });

  it('accepts a device with a point', () => {
    expect(isValid(validateConnectorFile(PUMP))).toBe(true);
  });

  it('reports a syntax error at its line', () => {
    const result = validateConnectorFile(PUMP.replace('"tedge-dot-opcua-pump"', '"tedge-dot-opcua-pump'));
    expect(result.syntax?.line).toBe(3);
    expect(isValid(result)).toBe(false);
  });

  it('reports a schema error with the key it concerns', () => {
    const result = validateConnectorFile(PUMP.replace('"float64"', '"float33"'));
    expect(result.syntax).toBeUndefined();
    expect(result.fields.map(f => f.label)).toContain('device "Pump01" › point "flow" › datatype');
  });
});

describe('describePointer', () => {
  it('names devices and points by their handles, and counts the others', () => {
    const config = { device: [{ name: 'A', point: [{}, { id: 'p' }] }, {}] };
    expect(describePointer(config, '/device/0/point/1/address')).toBe('device "A" › point "p" › address');
    expect(describePointer(config, '/device/1/protocol_address')).toBe('device 2 › protocol_address');
    expect(describePointer(config, '/connector/protocol')).toBe('connector › protocol');
    expect(describePointer(config, '')).toBe('file');
  });
});
