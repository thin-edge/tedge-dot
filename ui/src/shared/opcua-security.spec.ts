import { newConfig, setAt, type Config } from '../generated/config-builder';
import { linkSecurity, securityChoices } from './opcua-security';
import { validateConnectorFile } from './validation';

const opcua = (address: Record<string, unknown> = {}, connection: Record<string, unknown> = {}): Config => {
  let config = newConfig('opcua');
  for (const [key, value] of Object.entries(address)) config = setAt(config, `/device/0/protocol_address/${key}`, value);
  for (const [key, value] of Object.entries(connection)) config = setAt(config, `/connection/${key}`, value);
  return config;
};
const device = '/device/0/protocol_address';

describe('securityChoices', () => {
  it('offers the policies with the deprecated ones last and marked, and the modes', () => {
    const { security_policy, security_mode } = securityChoices(opcua()).devices[0];
    expect(security_policy.options).toEqual(['None', 'Basic256Sha256', 'Aes128_Sha256_RsaOaep', 'Aes256_Sha256_RsaPss', 'Basic128Rsa15', 'Basic256']);
    expect(security_policy.notes).toEqual({ Basic128Rsa15: 'deprecated', Basic256: 'deprecated' });
    expect(security_mode.options).toEqual(['none', 'sign', 'sign_and_encrypt']);
  });

  it('says what an empty field means at the connection', () => {
    expect(securityChoices(opcua()).connection['security_policy'].hint).toBe('Empty: None');
    expect(securityChoices(opcua()).connection['security_mode'].hint).toBe('Empty: none, the default of None');
    expect(securityChoices(opcua({}, { security_policy: 'Basic256Sha256' })).connection['security_mode'].hint).toBe(
      'Empty: sign_and_encrypt, the default of Basic256Sha256'
    );
  });

  it('says what an empty field means on a device', () => {
    const hints = (config: Config) => {
      const d = securityChoices(config).devices[0];
      return [d['security_policy'].hint, d['security_mode'].hint];
    };
    expect(hints(opcua())).toEqual(['Empty: None', 'Empty: none, the default of None']);
    expect(hints(opcua({}, { security_policy: 'Basic256Sha256' }))).toEqual([
      'Empty: Basic256Sha256, the connection setting',
      'Empty: sign_and_encrypt, the default of Basic256Sha256',
    ]);
    expect(hints(opcua({}, { security_policy: 'Basic256Sha256', security_mode: 'sign' }))).toEqual([
      'Empty: Basic256Sha256, the connection setting',
      'Empty: sign, the connection setting',
    ]);
    // A device with its own policy gets that policy's default mode, not the connection's.
    expect(hints(opcua({ security_policy: 'Aes256_Sha256_RsaPss' }, { security_mode: 'sign' }))[1]).toBe(
      'Empty: sign_and_encrypt, the default of Aes256_Sha256_RsaPss'
    );
    expect(hints(opcua({ security_policy: 'basic256' }))[1]).toBe("Empty: the policy's default");
  });
});

describe('linkSecurity', () => {
  it('sets a mode next to a policy that became None to none', () => {
    const config = opcua({ security_policy: 'None', security_mode: 'sign_and_encrypt' });
    expect(linkSecurity(config, `${device}/security_policy`)['device'][0]['protocol_address']['security_mode']).toBe('none');
    const connection = opcua({}, { security_policy: 'None', security_mode: 'sign' });
    expect(linkSecurity(connection, '/connection/security_policy')['connection']['security_mode']).toBe('none');
  });

  it('changes nothing else', () => {
    const secure = opcua({ security_policy: 'Basic256Sha256', security_mode: 'none' });
    expect(linkSecurity(secure, `${device}/security_policy`)).toBe(secure);
    const unset = opcua({ security_policy: 'None' });
    expect(linkSecurity(unset, `${device}/security_policy`)).toBe(unset);
    const other = opcua({ security_policy: 'None', security_mode: 'sign' });
    expect(linkSecurity(other, `${device}/endpoint`)).toBe(other);
  });
});

describe('security checks in the editor', () => {
  const text = (address: string) => `[connector]
protocol     = "opcua"
service_name = "tedge-dot-opcua-line"

[[device]]
name             = "Pump01"
protocol_address = { endpoint = "opc.tcp://127.0.0.1:4840/", ${address} }
`;

  it('shows a mode that does not fit the policy at the mode field', () => {
    const result = validateConnectorFile(text('security_policy = "Basic256Sha256", security_mode = "none"'));
    expect(result.fields).toEqual([
      expect.objectContaining({ path: `${device}/security_mode`, label: 'device "Pump01" › protocol_address › security_mode' }),
    ]);
  });

  it('flags a deprecated policy without allow_deprecated_security', () => {
    expect(validateConnectorFile(text('security_policy = "Basic256"')).fields.map(f => f.path)).toEqual([`${device}/security_policy`]);
    expect(validateConnectorFile(text('security_policy = "Basic256", allow_deprecated_security = true')).fields).toEqual([]);
  });
});
