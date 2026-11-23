import { Injectable } from '@angular/core';

import { type FileKind } from './naming';

export interface PendingFile {
  kind: FileKind;
  /** The file name, without `.toml`. */
  fileName: string;
  text: string;
  /** The entry's name, description and device type; the name defaults to the file name. */
  name?: string;
  description?: string;
  deviceType?: string;
  /** For a copy: the name of the entry it was made from. */
  copyOf?: string;
}

/**
 * Hands a file to the editor, which opens it as a new entry: an uploaded file that needs fixing,
 * or a copy of an entry. The editor takes it once.
 */
@Injectable({ providedIn: 'root' })
export class PendingFileService {
  private pending?: PendingFile;

  hand(file: PendingFile): void {
    this.pending = file;
  }

  take(): PendingFile | undefined {
    const file = this.pending;
    this.pending = undefined;
    return file;
  }
}
