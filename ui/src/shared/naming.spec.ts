import { isConnectorType, isValidName, nameOf, normalizeName, pathOf, typeOf } from './naming';

describe('naming', () => {
  it('maps a file name to its type and path and back', () => {
    expect(typeOf('opcua-pump')).toBe('tedge-dot-opcua-pump');
    expect(pathOf('opcua-pump')).toBe('/etc/tedge/plugins/ot/opcua-pump.toml');
    expect(nameOf('tedge-dot-opcua-pump')).toBe('opcua-pump');
  });

  it('recognises connector types only', () => {
    expect(isConnectorType('tedge-dot-line-3')).toBe(true);
    expect(isConnectorType('tedge.toml')).toBe(false);
    expect(isConnectorType('tedge-configuration-plugin')).toBe(false);
    expect(isConnectorType('tedge-dot-')).toBe(false);
    expect(isConnectorType('tedge-dot-../x')).toBe(false);
    expect(isConnectorType(undefined)).toBe(false);
  });

  it('accepts plain file names and refuses anything that could leave the directory', () => {
    for (const ok of ['modbus', 'opcua-pump', 'line_3', 'Line.3', '7']) {
      expect(isValidName(ok)).toBe(true);
    }
    for (const bad of ['', '../tedge', 'a/b', '.hidden', '-x', 'a b', 'x'.repeat(101)]) {
      expect(isValidName(bad)).toBe(false);
    }
  });

  it('drops spaces and a .toml suffix from what a user types', () => {
    expect(normalizeName('  opcua-pump.toml ')).toBe('opcua-pump');
    expect(normalizeName('line-3.TOML')).toBe('line-3');
    expect(normalizeName('modbus')).toBe('modbus');
  });
});
