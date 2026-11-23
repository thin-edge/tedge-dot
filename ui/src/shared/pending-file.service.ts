import { Injectable } from '@angular/core';

import { type FileKind } from './naming';

export interface PendingFile {
  kind: FileKind;
  /** The file name, without `.toml`. */
  fileName: string;
  text: string;
}

/**
 * Hands an uploaded file that needs fixing from the upload dialog to the editor, which opens it
 * as a new entry. The editor takes it once.
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
