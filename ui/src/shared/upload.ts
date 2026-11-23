/**
 * Uploading existing connector files: what each file becomes in the repository, and whether it
 * can be added as it is.
 */
import { isValidName, normalizeName } from './naming';
import { validateConnectorFile } from './validation';

export type UploadStatus = 'ready' | 'problems' | 'syntax' | 'library' | 'not-connector' | 'bad-name';

export interface UploadCheck {
  /** The uploaded file's own name. */
  source: string;
  /** The connector file name on gateways: the uploaded name without `.toml`. */
  fileName: string;
  status: UploadStatus;
  /** Why the file is not ready, for people; empty when it is. */
  reason: string;
}

/** Checks one uploaded file the way the editor checks a file before saving. */
export function checkUpload(source: string, text: string): UploadCheck {
  const fileName = normalizeName(source);
  if (!isValidName(fileName)) {
    return {
      source,
      fileName,
      status: 'bad-name',
      reason: 'The file name may only use letters, digits, ".", "_" and "-", and must start with a letter or digit.',
    };
  }
  const result = validateConnectorFile(text);
  if (result.syntax) {
    return { source, fileName, status: 'syntax', reason: `Syntax error on line ${result.syntax.line}: ${result.syntax.message}` };
  }
  const config = result.config ?? {};
  if (config['connector'] === undefined) {
    if (config['library'] !== undefined) {
      return {
        source,
        fileName,
        status: 'library',
        reason: 'This is a point library, not a connector file. Point libraries go in points.d on the gateway.',
      };
    }
    return { source, fileName, status: 'not-connector', reason: 'This file has no [connector] table, so it is not a connector file.' };
  }
  if (result.fields.length) {
    const n = result.fields.length;
    return { source, fileName, status: 'problems', reason: `${n} ${n === 1 ? 'problem' : 'problems'}, for example ${result.fields[0].label}: ${result.fields[0].message}` };
  }
  return { source, fileName, status: 'ready', reason: '' };
}

/** Whether a file that is not ready can still be fixed in the editor. */
export function canOpenInEditor(check: UploadCheck): boolean {
  return check.status === 'problems' || check.status === 'syntax' || check.status === 'bad-name';
}
