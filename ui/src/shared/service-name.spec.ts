import { emitToml, newConfig } from '../generated/config-builder';
import { followFileName, isAutomaticServiceName, packagedClash, serviceNameOf, setServiceName } from './service-name';

const template = emitToml(newConfig('opcua'));

describe('service name of a new connector file', () => {
  it('replaces a template service name with the file name', () => {
    expect(template).toMatch(/service_name\s*=\s*"tedge-dot-opcua"/);
    const followed = followFileName(template, 'opcua2');
    expect(followed?.serviceName).toBe('tedge-dot-opcua2');
    expect(followed?.text).toMatch(/service_name\s*=\s*"tedge-dot-opcua2"/);
    expect(followed?.text).not.toMatch(/"tedge-dot-opcua"/);
  });

  it('keeps following the file name while the user types', () => {
    const first = followFileName(template, 'op')!;
    const second = followFileName(first.text, 'opc', first.serviceName)!;
    expect(second.serviceName).toBe('tedge-dot-opc');
    // Without knowing what it set before, a typed name counts as the user's.
    expect(followFileName(first.text, 'opc')).toBeUndefined();
  });

  it("leaves the user's own service name alone", () => {
    const own = template.replace('"tedge-dot-opcua"', '"line-3-pumps"');
    expect(followFileName(own, 'line-3')).toBeUndefined();
  });

  it('keeps comments and the rest of the file', () => {
    const text = `# line 3\n[connector]\nprotocol = "modbus"\nservice_name = "tedge-dot-modbus" # the service\npoll_interval = "2s"\n`;
    expect(followFileName(text, 'line-3')?.text).toBe(
      `# line 3\n[connector]\nprotocol = "modbus"\nservice_name = "tedge-dot-line-3" # the service\npoll_interval = "2s"\n`
    );
  });

  it('changes nothing for an invalid file name, a file that does not parse, or a name already in step', () => {
    expect(followFileName(template, '')).toBeUndefined();
    expect(followFileName(template, '../x')).toBeUndefined();
    expect(followFileName(template, 'points.meter')).toBeUndefined();
    expect(followFileName('[connector\nprotocol = ', 'line-3')).toBeUndefined();
    expect(followFileName(template, 'opcua')).toBeUndefined();
  });

  it('knows which names were set for the user', () => {
    expect(serviceNameOf('line-3')).toBe('tedge-dot-line-3');
    expect(isAutomaticServiceName(undefined)).toBe(true);
    expect(isAutomaticServiceName('tedge-dot-modbus')).toBe(true);
    expect(isAutomaticServiceName('tedge-dot-line-3', 'tedge-dot-line-3')).toBe(true);
    expect(isAutomaticServiceName('tedge-dot-line-3')).toBe(false);
    expect(isAutomaticServiceName(42)).toBe(false);
  });

  it('sets a service name, keeping the rest of the file', () => {
    const text = `[connector]\nprotocol = "modbus" # bus\nservice_name = "a"\n`;
    expect(setServiceName(text, 'tedge-dot-line-3')).toBe(`[connector]\nprotocol = "modbus" # bus\nservice_name = "tedge-dot-line-3"\n`);
    expect(setServiceName('[library]\nprotocol = "modbus"\n', 'x')).toBeUndefined();
    expect(setServiceName('[connector\n', 'x')).toBeUndefined();
  });
});

describe('packagedClash', () => {
  const packaged = [{ file: 'opcua', serviceName: 'tedge-dot-opcua' }];

  it('names the example file whose service name another file uses', () => {
    expect(packagedClash('tedge-dot-opcua', 'opcua2', packaged)).toEqual({ file: 'opcua', serviceName: 'tedge-dot-opcua' });
    expect(packagedClash('tedge-dot-opcua2', 'opcua2', packaged)).toBeUndefined();
    expect(packagedClash(undefined, 'opcua2', packaged)).toBeUndefined();
  });

  it('does not count a file that replaces the example', () => {
    expect(packagedClash('tedge-dot-opcua', 'opcua', packaged)).toBeUndefined();
    expect(packagedClash('tedge-dot-opcua', 'opcua.toml', packaged)).toBeUndefined();
  });

  it('knows the examples the packages install', () => {
    expect(packagedClash('tedge-dot-modbus', 'line-3')?.file).toBe('modbus');
    // PROFIBUS ships with the C package only.
    expect(packagedClash('tedge-dot-profibus', 'line-3')?.file).toBe('profibus');
  });
});
