import {
  entriesOfKind,
  fileOfType,
  isConnectorType,
  isLibraryType,
  isValidConnectorName,
  isValidName,
  libraryFile,
  libraryOf,
  managedFile,
  nameOf,
  newestByType,
  normalizeName,
  pathOf,
  typeOf,
} from './naming';

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

  it('maps a point library to its type and path and back', () => {
    expect(libraryFile('modbus', 'acme-meter')).toEqual({
      kind: 'library',
      name: 'acme-meter',
      protocol: 'modbus',
      type: 'tedge-dot-points.modbus.acme-meter',
      path: '/etc/tedge/plugins/ot/points.d/modbus/acme-meter.toml',
    });
    expect(libraryOf('tedge-dot-points.modbus.acme-meter')).toEqual({ protocol: 'modbus', name: 'acme-meter' });
    expect(libraryOf('tedge-dot-points.aveva-pi.site.v2')).toEqual({ protocol: 'aveva-pi', name: 'site.v2' });
    expect(libraryOf('tedge-dot-points.modbus')).toBeUndefined();
    expect(libraryOf('tedge-dot-points.Mod/bus.x')).toBeUndefined();
  });

  it('keeps connector files and libraries apart', () => {
    expect(isLibraryType('tedge-dot-points.modbus.acme-meter')).toBe(true);
    expect(isConnectorType('tedge-dot-points.modbus.acme-meter')).toBe(false);
    expect(nameOf('tedge-dot-points.modbus.acme-meter')).toBeUndefined();
    expect(isValidConnectorName('points.modbus')).toBe(false);
    expect(isValidConnectorName('pointsfile')).toBe(true);
    expect(fileOfType('tedge-dot-line-3')?.kind).toBe('connector');
    expect(fileOfType('tedge-dot-points.snmp.ups')?.path).toBe('/etc/tedge/plugins/ot/points.d/snmp/ups.toml');
    expect(fileOfType('tedge.toml')).toBeUndefined();
  });

  it('splits repository entries by kind and picks the newest of a type', () => {
    const entries = [
      { name: 'a', configurationType: 'tedge-dot-line-3', lastUpdated: '2026-01-01T00:00:00Z' },
      { name: 'b', configurationType: 'tedge-dot-points.modbus.meter', lastUpdated: '2026-01-01T00:00:00Z' },
      { name: 'c', configurationType: 'tedge-dot-points.modbus.meter', lastUpdated: '2026-02-01T00:00:00Z' },
      { name: 'd', configurationType: 'tedge.toml' },
    ];
    expect(entriesOfKind(entries, 'connector').map(e => e.name)).toEqual(['a']);
    expect(entriesOfKind(entries, 'library').map(e => e.name)).toEqual(['b', 'c']);
    expect(newestByType(entries).get('tedge-dot-points.modbus.meter')?.name).toBe('c');
  });

  it('builds the file to register from a kind, a typed name and a protocol', () => {
    expect(managedFile('connector', ' opcua2.toml ')).toEqual({
      kind: 'connector',
      name: 'opcua2',
      type: 'tedge-dot-opcua2',
      path: '/etc/tedge/plugins/ot/opcua2.toml',
    });
    expect(managedFile('library', 'opcua-demo-pump', 'opcua')?.path).toBe('/etc/tedge/plugins/ot/points.d/opcua/opcua-demo-pump.toml');
    expect(managedFile('library', 'opcua-demo-pump', 'opcua')?.type).toBe('tedge-dot-points.opcua.opcua-demo-pump');
    // A connector file never takes a library's name; a library may start with "points.".
    expect(managedFile('connector', 'points.meter')).toBeUndefined();
    expect(managedFile('library', 'points.meter', 'modbus')?.name).toBe('points.meter');
    expect(managedFile('library', 'meter', '')).toBeUndefined();
    expect(managedFile('library', 'meter', 'Mod Bus')).toBeUndefined();
    expect(managedFile('connector', '../tedge')).toBeUndefined();
    expect(managedFile('connector', '')).toBeUndefined();
  });
});
