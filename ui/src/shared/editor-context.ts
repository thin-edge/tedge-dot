/**
 * Where the editor was opened from, carried in its URL so that a reload keeps it. Opened from a
 * gateway's OT connectors tab, the editor offers to send the saved entry to that gateway and then
 * returns to the tab. For a type the gateway already offers, the new file's name, and a library's
 * protocol, are fixed to that type's.
 *
 * Plain TypeScript, without Angular, so it is unit-tested as it is.
 */
import { type FileKind, type ManagedFile, managedFile } from './naming';

export interface EditorContext {
  /** The gateway whose OT connectors tab opened the editor. */
  gatewayId?: string;
  /** For a new entry: the file it has to be, because the gateway already offers its type. */
  file?: ManagedFile;
}

/** Query parameters, as the router's `ParamMap` gives them. */
export interface ParamReader {
  get(name: string): string | null;
}

/** A route as `Router.navigate` takes it. */
export interface Route {
  commands: string[];
  queryParams: Record<string, string>;
}

const ID = /^[0-9]+$/;

/** The editor's context for a file of `kind`; a parameter that is not valid is left out. */
export function readEditorContext(kind: FileKind, params: ParamReader): EditorContext {
  const gatewayId = params.get('gateway') ?? '';
  if (!ID.test(gatewayId)) {
    return {};
  }
  const name = params.get('file');
  const file = name ? managedFile(kind, name, params.get('protocol') ?? '') : undefined;
  // Only the plain file name the tab sends, not one the editor would have to normalize.
  return file && file.name === name ? { gatewayId, file } : { gatewayId };
}

/** The editor for a new file of `kind`, opened from the tab of `gatewayId`, fixed to `file` if given. */
export function newFileRoute(kind: FileKind, gatewayId: string, file?: ManagedFile): Route {
  const queryParams: Record<string, string> = { gateway: gatewayId };
  if (file) {
    queryParams['file'] = file.name;
    if (file.protocol) {
      queryParams['protocol'] = file.protocol;
    }
  }
  return { commands: ['/ot-connectors', kind === 'library' ? 'new-library' : 'new'], queryParams };
}

/** The editor for the entry `entryId`, opened from the tab of `gatewayId`. */
export function entryRoute(entryId: string | number, gatewayId: string): Route {
  return { commands: ['/ot-connectors', String(entryId)], queryParams: { gateway: gatewayId } };
}

/** The OT connectors tab of `gatewayId`. */
export function gatewayTabRoute(gatewayId: string): Route {
  return { commands: ['/device', gatewayId, 'ot-connectors'], queryParams: {} };
}
