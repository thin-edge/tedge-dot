import { entryRoute, gatewayTabRoute, newFileRoute, readEditorContext } from './editor-context';
import { connectorFile, libraryFile } from './naming';

const params = (values: Record<string, string>) => ({ get: (name: string) => values[name] ?? null });

describe('editor context', () => {
  it('reads the gateway, and a fixed file only with a gateway', () => {
    expect(readEditorContext('connector', params({}))).toEqual({});
    expect(readEditorContext('connector', params({ gateway: '12345' }))).toEqual({ gatewayId: '12345' });
    expect(readEditorContext('connector', params({ gateway: '12345', file: 'opcua2' }))).toEqual({
      gatewayId: '12345',
      file: connectorFile('opcua2'),
    });
    expect(readEditorContext('connector', params({ file: 'opcua2' }))).toEqual({});
  });

  it('reads a library with its protocol', () => {
    expect(readEditorContext('library', params({ gateway: '7', file: 'opcua-demo-pump', protocol: 'opcua' }))).toEqual({
      gatewayId: '7',
      file: libraryFile('opcua', 'opcua-demo-pump'),
    });
    // Without a valid protocol, a library name is not fixed.
    expect(readEditorContext('library', params({ gateway: '7', file: 'opcua-demo-pump' }))).toEqual({ gatewayId: '7' });
  });

  it('leaves out what is not valid', () => {
    expect(readEditorContext('connector', params({ gateway: '12a' }))).toEqual({});
    expect(readEditorContext('connector', params({ gateway: '../x' }))).toEqual({});
    expect(readEditorContext('connector', params({ gateway: '1', file: '../tedge' }))).toEqual({ gatewayId: '1' });
    expect(readEditorContext('connector', params({ gateway: '1', file: 'points.meter' }))).toEqual({ gatewayId: '1' });
    expect(readEditorContext('connector', params({ gateway: '1', file: 'opcua2.toml' }))).toEqual({ gatewayId: '1' });
  });

  it('builds the routes it reads back', () => {
    const library = libraryFile('modbus', 'meter');
    const route = newFileRoute('library', '42', library);
    expect(route).toEqual({
      commands: ['/ot-connectors', 'new-library'],
      queryParams: { gateway: '42', file: 'meter', protocol: 'modbus' },
    });
    expect(readEditorContext('library', params(route.queryParams))).toEqual({ gatewayId: '42', file: library });
    expect(newFileRoute('connector', '42')).toEqual({ commands: ['/ot-connectors', 'new'], queryParams: { gateway: '42' } });
    expect(entryRoute(9001, '42')).toEqual({ commands: ['/ot-connectors', '9001'], queryParams: { gateway: '42' } });
    expect(gatewayTabRoute('42')).toEqual({ commands: ['/device', '42', 'ot-connectors'], queryParams: {} });
  });
});
