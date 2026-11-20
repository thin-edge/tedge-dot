/**
 * Registering a connector file in thin-edge.io's list of managed files.
 *
 * thin-edge.io's file plugin manages the files listed in
 * /etc/tedge/plugins/tedge-configuration-plugin.toml, and offers that list itself as the type
 * `tedge-configuration-plugin`. To add a connector file from Cumulocity, the UI takes a snapshot
 * of the list, appends one `[[files]]` entry, and sends it back. thin-edge.io re-reads the list
 * and then offers the new type.
 *
 * The edit only ever appends: everything already in the file stays byte for byte, including
 * comments and entries this plugin knows nothing about.
 */
import { parse } from 'smol-toml';

import { pathOf, typeOf } from './naming';

/** One entry of the list, as thin-edge.io reads it. */
export interface RegisteredFile {
  path: string;
  /** The entry's type; thin-edge.io uses the path when the entry names none. */
  type: string;
}

export type RegistrationPlan =
  /** The file is registered under its type already: nothing to send. */
  | { kind: 'registered' }
  /** The path, or the type, is taken by another entry: registering would clash. */
  | { kind: 'conflict'; path: string; type: string; message: string }
  /** Append `block`; `text` is the whole new file. */
  | { kind: 'append'; block: string; text: string };

/** The entries of a tedge-configuration-plugin.toml. Throws when the text is not TOML. */
export function registeredFiles(text: string): RegisteredFile[] {
  const parsed = parse(text) as { files?: unknown };
  const files = Array.isArray(parsed.files) ? parsed.files : [];
  return files
    .filter((f): f is Record<string, unknown> => typeof f === 'object' && f !== null)
    .filter(f => typeof f['path'] === 'string')
    .map(f => {
      const path = f['path'] as string;
      const type = typeof f['type'] === 'string' && f['type'] ? (f['type'] as string) : path;
      return { path, type };
    });
}

/** The block appended for `name`. */
export function registrationBlock(name: string): string {
  return [
    '# Added by the tedge-dot OT connectors plugin for Device Management',
    '[[files]]',
    `path = '${pathOf(name)}'`,
    `type = '${typeOf(name)}'`,
    "user = 'tedge'",
    "group = 'tedge'",
    'mode = 0o644',
    '',
  ].join('\n');
}

/**
 * What registering connector file `name` means for the list in `text`.
 *
 * Throws when the current list cannot be read, or when the appended text would not read back
 * with exactly one new entry (for example because the list is written in a form that an appended
 * table would break). In both cases nothing should be sent.
 */
export function planRegistration(text: string, name: string): RegistrationPlan {
  let entries: RegisteredFile[];
  try {
    entries = registeredFiles(text);
  } catch (e) {
    throw new Error(`The gateway's tedge-configuration-plugin.toml is not valid TOML: ${errorText(e)}`);
  }
  const path = pathOf(name);
  const type = typeOf(name);

  const samePath = entries.find(e => e.path === path);
  if (samePath) {
    return samePath.type === type
      ? { kind: 'registered' }
      : {
          kind: 'conflict',
          path,
          type: samePath.type,
          message: `${path} is already registered as the type "${samePath.type}".`,
        };
  }
  const sameType = entries.find(e => e.type === type);
  if (sameType) {
    return {
      kind: 'conflict',
      path: sameType.path,
      type,
      message: `The type "${type}" is already registered for ${sameType.path}.`,
    };
  }

  const block = registrationBlock(name);
  const separator = text === '' || text.endsWith('\n\n') ? '' : text.endsWith('\n') ? '\n' : '\n\n';
  const next = text + separator + block;

  let after: RegisteredFile[];
  try {
    after = registeredFiles(next);
  } catch (e) {
    throw new Error(`An entry cannot be appended to this tedge-configuration-plugin.toml: ${errorText(e)}`);
  }
  const added = after.filter(e => e.path === path && e.type === type);
  if (after.length !== entries.length + 1 || added.length !== 1) {
    throw new Error('An entry cannot be appended to this tedge-configuration-plugin.toml: it does not read back.');
  }
  return { kind: 'append', block, text: next };
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message.split('\n')[0] : String(e);
}
