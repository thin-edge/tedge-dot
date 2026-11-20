import { planRegistration, registeredFiles, registrationBlock } from './registration';

/** The shape thin-edge.io 2.1 writes, with one connector file registered by hand. */
const LIST = `[[files]]
path = '/etc/tedge/tedge.toml'
type = 'tedge.toml'
user = 'tedge'
group = 'tedge'
mode = 0o444

[[files]]
path = '/etc/mosquitto/conf.d/mosquitto.conf'
type = 'mosquitto.conf'

# tedge-dot OPC-UA demo: the connector configuration (added by opcua-demo.sh)
[[files]]
path = '/etc/tedge/plugins/ot/opcua-pump.toml'
type = 'tedge-dot-opcua-pump'
user = 'tedge'
group = 'tedge'
`;

describe('registeredFiles', () => {
  it('reads the entries, with the path as the type when none is given', () => {
    const entries = registeredFiles(LIST + "\n[[files]]\npath = '/etc/x.conf'\n");
    expect(entries.map(e => e.type)).toEqual([
      'tedge.toml',
      'mosquitto.conf',
      'tedge-dot-opcua-pump',
      '/etc/x.conf',
    ]);
  });
});

describe('planRegistration', () => {
  it('appends one entry and keeps the rest byte for byte', () => {
    const plan = planRegistration(LIST, 'line-3');
    expect(plan.kind).toBe('append');
    if (plan.kind !== 'append') return;
    expect(plan.text.startsWith(LIST)).toBe(true);
    expect(plan.text.slice(LIST.length)).toBe('\n' + registrationBlock('line-3'));
    const added = registeredFiles(plan.text).filter(e => e.type === 'tedge-dot-line-3');
    expect(added).toEqual([{ path: '/etc/tedge/plugins/ot/line-3.toml', type: 'tedge-dot-line-3' }]);
  });

  it('writes the owner and mode thin-edge.io applies to a new file', () => {
    expect(registrationBlock('line-3')).toContain("user = 'tedge'\ngroup = 'tedge'\nmode = 0o644\n");
  });

  it('sends nothing for a file registered under its type', () => {
    expect(planRegistration(LIST, 'opcua-pump')).toEqual({ kind: 'registered' });
  });

  it('refuses a path registered under another type', () => {
    const list = LIST.replace("type = 'tedge-dot-opcua-pump'", "type = 'pump'");
    const plan = planRegistration(list, 'opcua-pump');
    expect(plan).toMatchObject({ kind: 'conflict', type: 'pump' });
  });

  it('refuses a type registered for another path', () => {
    const list = LIST.replace('/etc/tedge/plugins/ot/opcua-pump.toml', '/opt/pump.toml');
    expect(planRegistration(list, 'opcua-pump')).toMatchObject({ kind: 'conflict', path: '/opt/pump.toml' });
  });

  it('separates the block from a file without a final newline', () => {
    const plan = planRegistration(LIST.trimEnd(), 'line-3');
    expect(plan.kind === 'append' && plan.text).toBe(LIST.trimEnd() + '\n\n' + registrationBlock('line-3'));
  });

  it('starts an empty list', () => {
    expect(planRegistration('', 'line-3')).toEqual({
      kind: 'append',
      block: registrationBlock('line-3'),
      text: registrationBlock('line-3'),
    });
  });

  it('refuses a list that is not TOML', () => {
    expect(() => planRegistration('[[files]\npath = 1', 'line-3')).toThrow(/not valid TOML/);
  });

  it('refuses a list an appended table would break', () => {
    const inline = "files = [{ path = '/etc/tedge/tedge.toml', type = 'tedge.toml' }]\n";
    expect(() => planRegistration(inline, 'line-3')).toThrow(/cannot be appended/);
  });
});
