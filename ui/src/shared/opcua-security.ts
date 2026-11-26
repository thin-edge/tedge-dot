/**
 * The OPC UA security fields of the form: `security_policy` and `security_mode` in `[connection]`
 * and in each device's `protocol_address` as dropdowns, with what an empty field means there, and
 * the edit that keeps a mode valid when its policy becomes None.
 *
 * The checks themselves are the config builder's (src/generated/config-builder.ts), the same the
 * connectors make, so the form shows them at the field like any other check error.
 */
import { getAt, opcuaDefaultMode, opcuaMode, opcuaPolicy, setAt, type Config } from '../generated/config-builder';
import { OPCUA_SECURITY_MODES, OPCUA_SECURITY_POLICIES } from '../generated/form-options';
import { type FieldChoice } from './form-model';

export type ChoiceMap = Readonly<Record<string, FieldChoice>>;

export interface SecurityChoices {
  connection: ChoiceMap;
  /** By device index. */
  devices: readonly ChoiceMap[];
}

const POLICIES = OPCUA_SECURITY_POLICIES.map(p => p.name);
const DEPRECATED: Readonly<Record<string, string>> = Object.fromEntries(
  OPCUA_SECURITY_POLICIES.filter(p => p.deprecated).map(p => [p.name, 'deprecated'])
);
const POLICY_PATH = /^(\/connection|\/device\/\d+\/protocol_address)\/security_policy$/;

const table = (value: unknown): Config =>
  value && typeof value === 'object' && !Array.isArray(value) ? (value as Config) : {};

/** "Empty: sign_and_encrypt, the default of Basic256Sha256", for a policy value or no policy. */
function defaultModeHint(policyValue: unknown): string {
  const policy = policyValue === undefined ? 'None' : opcuaPolicy(policyValue)?.name;
  return policy ? `Empty: ${opcuaDefaultMode(policy)}, the default of ${policy}` : "Empty: the policy's default";
}

/** The dropdowns of the security fields, for `[connection]` and for each device. */
export function securityChoices(config: Config): SecurityChoices {
  const connection = table(config['connection']);
  const devices: unknown[] = Array.isArray(config['device']) ? config['device'] : [];
  const policyHint =
    connection['security_policy'] !== undefined ? `Empty: ${String(connection['security_policy'])}, the connection setting` : 'Empty: None';
  return {
    connection: {
      security_policy: { options: POLICIES, notes: DEPRECATED, hint: 'Empty: None' },
      security_mode: { options: OPCUA_SECURITY_MODES, hint: defaultModeHint(connection['security_policy']) },
    },
    devices: devices.map(device => {
      const own = table(table(device)['protocol_address']);
      let modeHint: string;
      if (own['security_policy'] !== undefined) {
        // A device with its own policy gets that policy's default mode, not the connection's.
        modeHint = defaultModeHint(own['security_policy']);
      } else if (connection['security_mode'] !== undefined) {
        modeHint = `Empty: ${String(connection['security_mode'])}, the connection setting`;
      } else {
        modeHint = defaultModeHint(connection['security_policy']);
      }
      return {
        security_policy: { options: POLICIES, notes: DEPRECATED, hint: policyHint },
        security_mode: { options: OPCUA_SECURITY_MODES, hint: modeHint },
      };
    }),
  };
}

/**
 * After an edit at `path`: when it set a policy to None, a mode set next to it to sign or
 * sign_and_encrypt becomes none, which is the only mode None allows.
 */
export function linkSecurity(config: Config, path: string): Config {
  const match = POLICY_PATH.exec(path);
  if (!match || config['connector']?.protocol !== 'opcua' || opcuaPolicy(getAt(config, path))?.name !== 'None') {
    return config;
  }
  const modePath = `${match[1]}/security_mode`;
  const mode = opcuaMode(getAt(config, modePath));
  return mode && mode !== 'none' ? setAt(config, modePath, 'none') : config;
}
