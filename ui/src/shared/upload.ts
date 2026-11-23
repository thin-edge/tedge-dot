/**
 * Uploading an existing connector file or point library: what the file becomes in the repository,
 * and whether it can be added as it is.
 */
import { type FileKind, type ManagedFile, connectorFile, isValidConnectorName, isValidName, isValidProtocol, libraryFile, normalizeName } from './naming';
import { validateConnectorFile, validateLibraryFile } from './validation';

export type UploadStatus = 'ready' | 'problems' | 'syntax' | 'not-connector' | 'bad-name';

export interface UploadCheck {
  /** The uploaded file's own name. */
  source: string;
  /** The file name on gateways: the uploaded name without `.toml`. */
  fileName: string;
  status: UploadStatus;
  /** Why the file is not ready, for people; empty when it is. */
  reason: string;
  /** What the file is, as far as its content tells: a library has a [library] table. */
  kind: FileKind;
  /** What the file becomes, once its kind and (for a library) protocol are known. */
  file?: ManagedFile;
}

/** Checks one uploaded file the way the editor checks a file before saving. */
export function checkUpload(source: string, text: string): UploadCheck {
  const fileName = normalizeName(source);
  // Also for a file that does not parse, so it opens in the right editor.
  const kind: FileKind = /^\s*\[\s*library\s*\]/m.test(text) && !/^\s*\[\s*connector\s*\]/m.test(text) ? 'library' : 'connector';
  const badName = {
    source,
    fileName,
    kind,
    status: 'bad-name' as const,
    reason: 'The file name may only use letters, digits, ".", "_" and "-", and must start with a letter or digit.',
  };
  if (!isValidName(fileName)) {
    return badName;
  }
  const connector = validateConnectorFile(text);
  if (connector.syntax) {
    return { source, fileName, kind, status: 'syntax', reason: `Syntax error on line ${connector.syntax.line}: ${connector.syntax.message}` };
  }
  const config = connector.config ?? {};

  if (config['connector'] === undefined && config['library'] !== undefined) {
    const protocol = config['library']?.protocol;
    if (typeof protocol !== 'string' || !isValidProtocol(protocol)) {
      return { source, fileName, kind: 'library', status: 'problems', reason: 'The [library] table names no protocol.' };
    }
    const file = libraryFile(protocol, fileName);
    const library = validateLibraryFile(text);
    if (library.fields.length) {
      return { source, fileName, kind: file.kind, file, status: 'problems', reason: problemText(library.fields) };
    }
    return { source, fileName, kind: file.kind, file, status: 'ready', reason: '' };
  }
  if (config['connector'] === undefined) {
    return { source, fileName, kind, status: 'not-connector', reason: 'This file has neither a [connector] nor a [library] table.' };
  }
  if (!isValidConnectorName(fileName)) {
    return { ...badName, reason: 'A connector file name may not start with "points.", which point libraries use.' };
  }
  const file = connectorFile(fileName);
  if (connector.fields.length) {
    return { source, fileName, kind: file.kind, file, status: 'problems', reason: problemText(connector.fields) };
  }
  return { source, fileName, kind: file.kind, file, status: 'ready', reason: '' };
}

function problemText(fields: { label: string; message: string }[]): string {
  const n = fields.length;
  return `${n} ${n === 1 ? 'problem' : 'problems'}, for example ${fields[0].label}: ${fields[0].message}`;
}

/** Whether a file that is not ready can still be fixed in the editor. */
export function canOpenInEditor(check: UploadCheck): boolean {
  return check.status === 'problems' || check.status === 'syntax' || check.status === 'bad-name';
}
