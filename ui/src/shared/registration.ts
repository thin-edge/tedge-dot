/**
 * Registering a connector file in thin-edge.io's list of managed files.
 *
 * thin-edge.io's file plugin manages the files listed in
 * /etc/tedge/plugins/tedge-configuration-plugin.toml, and offers that list itself as the type
 * `tedge-configuration-plugin`. To add connector files and point libraries from Cumulocity, the UI
 * takes a snapshot of the list, appends a `[[files]]` entry per file, and sends it back.
 * thin-edge.io re-reads the list and then offers the new types.
 *
 * The edit only ever appends: everything already in the file stays byte for byte, including
 * comments and entries this plugin knows nothing about.
 */
import { parse } from 'smol-toml';

import { type ManagedFile } from './naming';

/** One entry of the list, as thin-edge.io reads it. */
export interface RegisteredFile {
  path: string;
  /** The entry's type; thin-edge.io uses the path when the entry names none. */
  type: string;
}

export type RegistrationPlan =
  /** Every file is registered under its type already: nothing to send. */
  | { kind: 'registered' }
  /** A path, or a type, is taken by another entry: registering would clash. */
  | { kind: 'conflict'; path: string; type: string; message: string }
  /** Append `block` for `files`; `text` is the whole new list. */
  | { kind: 'append'; files: ManagedFile[]; block: string; text: string };

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

/**
 * The block appended for `file`. A point library lives in a protocol folder that does not exist
 * until the first library arrives; thin-edge.io's file plugin creates it with the parent settings.
 */
export function registrationBlock(file: ManagedFile): string {
  return [
    '# Added by the tedge-dot OT connectors plugin for Device Management',
    '[[files]]',
    `path = '${file.path}'`,
    `type = '${file.type}'`,
    "user = 'tedge'",
    "group = 'tedge'",
    'mode = 0o644',
    ...(file.kind === 'library' ? ["parent_user = 'tedge'", "parent_group = 'tedge'", 'parent_mode = 0o755'] : []),
    '',
  ].join('\n');
}

/**
 * What registering `files` means for the list in `text`: the files not registered yet are
 * appended in one edit.
 *
 * Throws when the current list cannot be read, or when the appended text would not read back
 * with exactly the new entries (for example because the list is written in a form that an
 * appended table would break). In both cases nothing should be sent.
 */
export function planRegistration(text: string, files: readonly ManagedFile[]): RegistrationPlan {
  let entries: RegisteredFile[];
  try {
    entries = registeredFiles(text);
  } catch (e) {
    throw new Error(`The gateway's tedge-configuration-plugin.toml is not valid TOML: ${errorText(e)}`);
  }

  const missing: ManagedFile[] = [];
  for (const file of files) {
    const samePath = entries.find(e => e.path === file.path);
    if (samePath) {
      if (samePath.type === file.type) {
        continue;
      }
      return {
        kind: 'conflict',
        path: file.path,
        type: samePath.type,
        message: `${file.path} is already registered as the type "${samePath.type}".`,
      };
    }
    const sameType = entries.find(e => e.type === file.type);
    if (sameType) {
      return {
        kind: 'conflict',
        path: sameType.path,
        type: file.type,
        message: `The type "${file.type}" is already registered for ${sameType.path}.`,
      };
    }
    if (!missing.some(m => m.type === file.type)) {
      missing.push(file);
    }
  }
  if (!missing.length) {
    return { kind: 'registered' };
  }

  const block = missing.map(registrationBlock).join('\n');
  const separator = text === '' || text.endsWith('\n\n') ? '' : text.endsWith('\n') ? '\n' : '\n\n';
  const next = text + separator + block;

  let after: RegisteredFile[];
  try {
    after = registeredFiles(next);
  } catch (e) {
    throw new Error(`An entry cannot be appended to this tedge-configuration-plugin.toml: ${errorText(e)}`);
  }
  const added = missing.every(m => after.filter(e => e.path === m.path && e.type === m.type).length === 1);
  if (after.length !== entries.length + missing.length || !added) {
    throw new Error('An entry cannot be appended to this tedge-configuration-plugin.toml: it does not read back.');
  }
  return { kind: 'append', files: missing, block, text: next };
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message.split('\n')[0] : String(e);
}
