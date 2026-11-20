import { Component, OnInit, inject } from '@angular/core';
import { ActivatedRoute, Router } from '@angular/router';
import { IManagedObject, InventoryService } from '@c8y/client';
import { AlertService, CoreModule, gettext } from '@c8y/ngx-components';
import { EditorComponent, loadMonacoEditor } from '@c8y/ngx-components/editor';
import { RepositoryService, RepositoryType, SharedRepositoryModule } from '@c8y/ngx-components/repository/shared';
import type * as Monaco from 'monaco-editor';

import { PROTOCOL_IDS, emitToml, newConfig, type ProtocolId } from '../generated/config-builder';
import { isValidName, nameOf, normalizeName, typeOf } from '../shared/naming';
import { TOML_LANGUAGE_ID, tomlLanguage } from '../shared/toml-language';
import { type ValidationResult, isValid, validateConnectorFile } from '../shared/validation';

const MARKER_OWNER = 'tedge-dot';

/** Adds or edits one connector file in the configuration repository. */
@Component({
  selector: 'tdot-connector-editor',
  imports: [CoreModule, SharedRepositoryModule, EditorComponent],
  template: `
    <c8y-title>{{ (isNew ? 'Add connector file' : 'Edit connector file') | translate }}</c8y-title>

    <div class="card content-fullpage d-flex d-col">
      <div class="card-header separator">
        <div class="card-title">{{ isNew ? ('New connector file' | translate) : entry?.['name'] }}</div>
      </div>
      <div class="card-block d-flex d-col flex-grow min-height-0">
        <div class="row">
          <div class="col-md-3">
            <c8y-form-group [hasError]="nameTouched && !name.trim()">
              <label for="tdot-name">{{ 'Name' | translate }}</label>
              <input id="tdot-name" class="form-control" [(ngModel)]="name" (blur)="nameTouched = true" />
            </c8y-form-group>
          </div>
          <div class="col-md-3">
            <c8y-form-group [hasError]="fileTouched && !fileValid">
              <label for="tdot-file">{{ 'File on the gateway' | translate }}</label>
              <div class="input-group">
                <span class="input-group-addon">/etc/tedge/plugins/ot/</span>
                <input
                  id="tdot-file"
                  class="form-control"
                  [(ngModel)]="fileName"
                  [disabled]="!isNew"
                  (blur)="fileTouched = true"
                  placeholder="opcua-pump"
                />
                <span class="input-group-addon">.toml</span>
              </div>
              @if (fileTouched && !fileValid) {
                <c8y-messages>
                  <c8y-message>{{ 'Letters, digits, ".", "_" and "-" only, starting with a letter or digit.' | translate }}</c8y-message>
                </c8y-messages>
              }
            </c8y-form-group>
          </div>
          <div class="col-md-3">
            <c8y-form-group>
              <label for="tdot-device-type">{{ 'Device type' | translate }}</label>
              <input id="tdot-device-type" class="form-control" [(ngModel)]="deviceType" [placeholder]="'Any' | translate" />
            </c8y-form-group>
          </div>
          @if (isNew) {
            <div class="col-md-3">
              <c8y-form-group>
                <label for="tdot-start">{{ 'Start from' | translate }}</label>
                <div class="c8y-select-wrapper">
                  <select id="tdot-start" class="form-control" [(ngModel)]="startFrom" (ngModelChange)="useTemplate($event)">
                    @for (p of protocols; track p) {
                      <option [ngValue]="p">{{ p }}</option>
                    }
                  </select>
                </div>
              </c8y-form-group>
            </div>
          }
          <div class="col-md-12">
            <c8y-form-group>
              <label for="tdot-description">{{ 'Description' | translate }}</label>
              <input id="tdot-description" class="form-control" [(ngModel)]="description" />
            </c8y-form-group>
          </div>
        </div>

        <div class="tdot-editor flex-grow">
          <c8y-editor
            [(ngModel)]="content"
            (ngModelChange)="validate()"
            [editorOptions]="editorOptions"
            (editorInit)="onEditorInit($event)"
          ></c8y-editor>
        </div>

        <div class="p-t-8">
          @if (result.syntax) {
            <p class="text-danger">
              <i c8yIcon="exclamation-circle"></i>
              {{ 'Line' | translate }} {{ result.syntax.line }}, {{ 'column' | translate }} {{ result.syntax.column }}:
              {{ result.syntax.message }}
            </p>
          } @else if (result.fields.length) {
            <ul class="list-unstyled text-danger m-b-0">
              @for (f of result.fields; track f.path + f.message) {
                <li><i c8yIcon="exclamation-circle"></i> {{ f.label }}: {{ f.message }}</li>
              }
            </ul>
          } @else {
            <p class="text-success m-b-0">
              <i c8yIcon="check-circle"></i>
              {{ 'Matches the contract schema. The gateway checks point libraries and files when it loads the file.' | translate }}
            </p>
          }
        </div>
      </div>
      <div class="card-footer separator text-right">
        <button class="btn btn-default" type="button" (click)="cancel()">{{ 'Cancel' | translate }}</button>
        <button class="btn btn-primary" type="button" [disabled]="!canSave || saving" (click)="save()">
          {{ 'Save' | translate }}
        </button>
      </div>
    </div>
  `,
  styles: [
    `
      .tdot-editor {
        min-height: 360px;
        height: 55vh;
        border: 1px solid var(--c8y-form-control-border-color-default, #ccc);
      }
      .tdot-editor c8y-editor {
        display: block;
        height: 100%;
      }
    `,
  ],
})
export class ConnectorEditorComponent implements OnInit {
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly inventory = inject(InventoryService);
  private readonly repository = inject(RepositoryService);
  private readonly alert = inject(AlertService);

  readonly protocols = PROTOCOL_IDS;
  readonly editorOptions: Monaco.editor.IStandaloneEditorConstructionOptions = {
    language: TOML_LANGUAGE_ID,
    minimap: { enabled: false },
    automaticLayout: true,
    scrollBeyondLastLine: false,
    tabSize: 2,
  };

  isNew = true;
  entry?: IManagedObject;
  name = '';
  description = '';
  fileName = '';
  deviceType = '';
  startFrom: ProtocolId = 'opcua';
  content = '';
  result: ValidationResult = { fields: [] };
  nameTouched = false;
  fileTouched = false;
  saving = false;

  private monaco?: typeof Monaco;
  private editor?: Monaco.editor.IStandaloneCodeEditor;

  get fileValid(): boolean {
    return isValidName(normalizeName(this.fileName));
  }

  get canSave(): boolean {
    return !!this.name.trim() && this.fileValid && isValid(this.result);
  }

  async ngOnInit(): Promise<void> {
    const id = this.route.snapshot.paramMap.get('id');
    if (!id || id === 'new') {
      this.useTemplate(this.startFrom);
      return;
    }
    this.isNew = false;
    try {
      const { data } = await this.inventory.detail(id);
      this.entry = data;
      this.name = data['name'] ?? '';
      this.description = data['description'] ?? '';
      this.deviceType = data['deviceType'] ?? '';
      this.fileName = nameOf(data['configurationType']) ?? '';
      this.content = data['url'] ? await this.repository.getBinaryText(data['url'], { allowExternal: false }) : '';
      this.validate();
    } catch (e) {
      this.alert.addServerFailure(e);
    }
  }

  useTemplate(protocol: ProtocolId): void {
    this.content = emitToml(newConfig(protocol));
    this.validate();
  }

  async onEditorInit(editor: Monaco.editor.IStandaloneCodeEditor): Promise<void> {
    this.editor = editor;
    const monaco = await loadMonacoEditor();
    this.monaco = monaco;
    if (!monaco.languages.getLanguages().some(l => l.id === TOML_LANGUAGE_ID)) {
      monaco.languages.register({ id: TOML_LANGUAGE_ID, extensions: ['.toml'] });
      monaco.languages.setMonarchTokensProvider(TOML_LANGUAGE_ID, tomlLanguage as Monaco.languages.IMonarchLanguage);
    }
    const model = editor.getModel();
    if (model) {
      monaco.editor.setModelLanguage(model, TOML_LANGUAGE_ID);
    }
    this.applyMarkers();
  }

  validate(): void {
    this.result = validateConnectorFile(this.content ?? '');
    this.applyMarkers();
  }

  private applyMarkers(): void {
    const model = this.editor?.getModel();
    if (!this.monaco || !model) {
      return;
    }
    const syntax = this.result.syntax;
    this.monaco.editor.setModelMarkers(
      model,
      MARKER_OWNER,
      syntax
        ? [
            {
              startLineNumber: syntax.line,
              startColumn: syntax.column,
              endLineNumber: syntax.line,
              endColumn: syntax.column + 1,
              message: syntax.message,
              severity: this.monaco.MarkerSeverity.Error,
            },
          ]
        : []
    );
  }

  cancel(): void {
    void this.router.navigate(['/ot-connectors']);
  }

  async save(): Promise<void> {
    if (!this.canSave) {
      return;
    }
    this.saving = true;
    const name = normalizeName(this.fileName);
    try {
      const file = new File([this.content], `${name}.toml`, { type: 'text/plain' });
      await this.repository.create(
        {
          version: this.name.trim(),
          description: this.description.trim(),
          deviceType: this.deviceType.trim() || undefined,
          configurationType: typeOf(name),
          binary: { file },
        },
        RepositoryType.CONFIGURATION,
        this.entry
      );
      this.alert.success(gettext('Connector file saved.'));
      void this.router.navigate(['/ot-connectors']);
    } catch (e) {
      this.alert.addServerFailure(e);
    } finally {
      this.saving = false;
    }
  }
}
