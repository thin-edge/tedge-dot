import { Component, computed, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { CoreModule, DropAreaComponent, type DroppedFile } from '@c8y/ngx-components';
import { RepositoryService, RepositoryType, SharedRepositoryModule } from '@c8y/ngx-components/repository/shared';
import { BsModalRef } from 'ngx-bootstrap/modal';

import { type FileKind } from '../shared/naming';
import { PendingFileService } from '../shared/pending-file.service';
import { type UploadCheck, canOpenInEditor, checkUpload } from '../shared/upload';

interface Upload {
  check: UploadCheck;
  text: string;
}

type State = 'pick' | 'saving' | 'done';

/**
 * Adds an existing connector file or point library to the configuration repository. The file is
 * checked like the editor checks a file; a valid file is added as it is, another can be opened in
 * the editor. What the file is follows from its content: a point library has a [library] table.
 */
@Component({
  selector: 'tdot-upload-file',
  imports: [CoreModule, SharedRepositoryModule, DropAreaComponent],
  template: `
    <div class="modal-header dialog-header">
      <i c8yIcon="upload"></i>
      <h4>{{ (kind === 'library' ? 'Upload point library' : 'Upload connector file') | translate }}</h4>
    </div>
    <div class="modal-body">
      @if (state() === 'pick') {
        <c8y-form-group>
          <c8y-drop-area
            class="drop-area-sm"
            accept=".toml"
            [maxAllowedFiles]="1"
            [title]="'Drop a TOML file or click to browse' | translate"
            [attr.aria-label]="'Drop a TOML file or click to browse' | translate"
            (dropped)="onDropped($event)"
          ></c8y-drop-area>
          <c8y-messages
            [helpMessage]="(kind === 'library'
              ? 'The file becomes a repository entry named after it: a modbus library acme-meter.toml installs as /etc/tedge/plugins/ot/points.d/modbus/acme-meter.toml.'
              : 'The file becomes a repository entry named after it: line-3.toml installs as /etc/tedge/plugins/ot/line-3.toml.') | translate"
          ></c8y-messages>
        </c8y-form-group>
        <c8y-form-group>
          <label for="tdot-upload-device-type">{{ 'Device type' | translate }}</label>
          <input
            id="tdot-upload-device-type"
            class="form-control"
            [placeholder]="('e.g.' | translate) + ' thin-edge.io'"
            [value]="deviceType()"
            (input)="deviceType.set($any($event.target).value)"
          />
          <c8y-messages [helpMessage]="'Empty: offered to every device type' | translate"></c8y-messages>
        </c8y-form-group>
        @if (uploads().length) {
          <ul class="list-group">
            @for (u of uploads(); track u.check.source) {
              <li class="list-group-item d-flex a-i-start">
                <i
                  class="m-r-8 m-t-4"
                  [c8yIcon]="u.check.status === 'ready' ? 'check-circle' : 'exclamation-circle'"
                  [class.text-success]="u.check.status === 'ready'"
                  [class.text-danger]="u.check.status !== 'ready'"
                ></i>
                <div class="flex-grow">
                  <strong>{{ u.check.source }}</strong>
                  @if (u.check.file; as f) {
                    <span class="text-muted"> › {{ f.path }}</span>
                    @if (f.kind !== kind) {
                      <span class="tag tag--info m-l-8">{{ (f.kind === 'library' ? 'point library' : 'connector file') | translate }}</span>
                    }
                  }
                  @if (u.check.reason) {
                    <p class="text-muted small m-b-0">{{ u.check.reason }}</p>
                  }
                </div>
                @if (canFix(u)) {
                  <button type="button" class="btn btn-link btn-sm" (click)="openInEditor(u)">{{ 'Open in editor' | translate }}</button>
                }
              </li>
            }
          </ul>
        }
      } @else if (state() === 'saving') {
        <p><i c8yIcon="spinner" class="icon-spin"></i> {{ 'Adding the file to the repository' | translate }}…</p>
      } @else {
        @if (added().length) {
          <div class="alert alert-success" role="status">
            {{ 'Added to the repository:' | translate }} {{ added().join(', ') }}
          </div>
        }
        @if (failed().length) {
          <div class="alert alert-danger" role="alert">
            @for (f of failed(); track f) {
              <div>{{ f }}</div>
            }
          </div>
        }
      }
    </div>
    <div class="modal-footer">
      @if (state() === 'pick') {
        <button type="button" class="btn btn-default" (click)="close()">{{ 'Cancel' | translate }}</button>
        <button type="button" class="btn btn-primary" [disabled]="!ready().length" (click)="addReady()">
          {{ 'Add file' | translate }}
        </button>
      } @else if (state() === 'saving') {
        <button type="button" class="btn btn-primary btn-pending" aria-busy="true" disabled>{{ 'Adding' | translate }}</button>
      } @else {
        <button type="button" class="btn btn-primary" (click)="close()">{{ 'Close' | translate }}</button>
      }
    </div>
  `,
})
export class UploadFileComponent {
  private readonly repository = inject(RepositoryService);
  private readonly modalRef = inject(BsModalRef);
  private readonly router = inject(Router);
  private readonly pending = inject(PendingFileService);

  /** Set by the opener: what the dialog is for, which only changes its texts. */
  kind: FileKind = 'connector';
  /** Set by the opener: called after the file was added. */
  onFinished?: () => void;

  readonly state = signal<State>('pick');
  readonly uploads = signal<Upload[]>([]);
  readonly deviceType = signal('');
  readonly added = signal<string[]>([]);
  readonly failed = signal<string[]>([]);
  readonly ready = computed(() => this.uploads().filter(u => u.check.status === 'ready'));

  async onDropped(files: DroppedFile[]): Promise<void> {
    const uploads: Upload[] = [];
    for (const dropped of files ?? []) {
      const text = String(await dropped.readAsText());
      uploads.push({ check: checkUpload(dropped.file.name, text), text });
    }
    this.uploads.set(uploads);
  }

  canFix(upload: Upload): boolean {
    return canOpenInEditor(upload.check);
  }

  openInEditor(upload: Upload): void {
    const kind = upload.check.kind;
    this.pending.hand({ kind, fileName: upload.check.fileName, text: upload.text });
    this.modalRef.hide();
    void this.router.navigate(['/ot-connectors', kind === 'library' ? 'new-library' : 'new']);
  }

  async addReady(): Promise<void> {
    this.state.set('saving');
    const added: string[] = [];
    const failed: string[] = [];
    const deviceType = this.deviceType().trim() || undefined;
    for (const { check, text } of this.ready()) {
      if (!check.file) {
        continue;
      }
      try {
        await this.repository.create(
          {
            version: check.fileName,
            description: '',
            deviceType,
            configurationType: check.file.type,
            binary: { file: new File([text], `${check.fileName}.toml`, { type: 'text/plain' }) },
          },
          RepositoryType.CONFIGURATION
        );
        added.push(check.fileName);
      } catch (e) {
        failed.push(`${check.source}: ${e instanceof Error ? e.message : 'could not be added'}`);
      }
    }
    this.added.set(added);
    this.failed.set(failed);
    this.state.set('done');
    if (added.length) {
      this.onFinished?.();
    }
  }

  close(): void {
    this.modalRef.hide();
  }
}
