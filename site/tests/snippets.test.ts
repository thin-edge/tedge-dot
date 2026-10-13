import { describe, expect, it } from 'vitest';
import { selectSnippet } from '../src/lib/snippets';

const file = `# head
# site:start device
  [[device]]
  name = "plc1"
# site:end device
tail
`;

describe('selectSnippet', () => {
  it('selects the lines between markers, dedented', () => {
    expect(selectSnippet(file, 'device')).toBe('[[device]]\nname = "plc1"\n');
  });

  it('includes the whole file without its marker lines', () => {
    expect(selectSnippet(file)).toBe('# head\n  [[device]]\n  name = "plc1"\ntail\n');
  });

  it('names the file and marker when a marker is missing', () => {
    expect(() => selectSnippet(file, 'points', 'demo/config/modbus.toml')).toThrow(
      'demo/config/modbus.toml: no "site:start points" marker',
    );
  });
});
