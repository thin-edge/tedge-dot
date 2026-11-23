/**
 * Checking a connector file before it is saved.
 *
 * The rules are the docs site's config builder's: TOML syntax, then the contract schema
 * (doc/contract/schemas/config.schema.json) with the protocol shapes from
 * site/src/data/protocols.json. scripts/sync-site.mjs copies that module into
 * src/generated, so the UI and the config builder always give the same verdict.
 *
 * What only the gateway knows, such as which point libraries exist there, is out of reach.
 */
import { parseToml, validateConfig, validateLibrary, type Config, type FieldError } from '../generated/config-builder';

export interface TomlSyntaxError {
  /** 1-based. */
  line: number;
  /** 1-based. */
  column: number;
  message: string;
}

export interface FieldProblem extends FieldError {
  /** Where the value is, for people: `device "Pump01" › point "flow" › datatype`. */
  label: string;
}

export interface ValidationResult {
  syntax?: TomlSyntaxError;
  fields: FieldProblem[];
  /** The parsed file, when it is valid TOML. */
  config?: Config;
}

/** Validates the text of a connector file. */
export function validateConnectorFile(text: string): ValidationResult {
  let config: Config;
  try {
    config = parseToml(text);
  } catch (e) {
    return { syntax: syntaxError(e), fields: [] };
  }
  const fields = validateConfig(config).map(f => ({ ...f, label: describePointer(config, f.path) }));
  return { fields, config };
}

/**
 * Validates the text of a point library (contract §3.4) with the contract's point-library schema.
 * A point id repeated within one library is an error too: there is no order to apply it in.
 */
export function validateLibraryFile(text: string): ValidationResult {
  let config: Config;
  try {
    config = parseToml(text);
  } catch (e) {
    return { syntax: syntaxError(e), fields: [] };
  }
  const fields: FieldError[] = validateLibrary(config);
  const seen = new Map<string, number>();
  (Array.isArray(config['point']) ? config['point'] : []).forEach((point: Config, index: number) => {
    const id = point?.['id'];
    if (typeof id !== 'string') {
      return;
    }
    if (seen.has(id)) {
      fields.push({ path: `/point/${index}/id`, message: `repeats the point id "${id}" of point ${seen.get(id)! + 1}` });
    } else {
      seen.set(id, index);
    }
  });
  return { fields: fields.map(f => ({ ...f, label: describePointer(config, f.path) })), config };
}

export function isValid(result: ValidationResult): boolean {
  return !result.syntax && result.fields.length === 0;
}

function syntaxError(e: unknown): TomlSyntaxError {
  const err = e as { line?: unknown; column?: unknown; message?: unknown };
  const message = typeof err?.message === 'string' ? err.message.split('\n')[0] : String(e);
  return {
    line: typeof err?.line === 'number' ? err.line : 1,
    column: typeof err?.column === 'number' ? err.column : 1,
    message: message.replace(/^Invalid TOML document:\s*/, ''),
  };
}

/**
 * A JSON pointer from the schema validation, said the way the file says it: devices by name and
 * points by id, where they have one.
 */
export function describePointer(config: Config, pointer: string): string {
  if (!pointer) {
    return 'file';
  }
  const segments = pointer
    .split('/')
    .slice(1)
    .map(s => s.replace(/~1/g, '/').replace(/~0/g, '~'));
  const parts: string[] = [];
  let node: any = config;
  for (let i = 0; i < segments.length; i++) {
    const key = segments[i];
    const next = segments[i + 1];
    if ((key === 'device' || key === 'point') && next !== undefined && /^\d+$/.test(next)) {
      const index = Number(next);
      const item = Array.isArray(node?.[key]) ? node[key][index] : undefined;
      const handle = key === 'device' ? item?.name : item?.id;
      parts.push(typeof handle === 'string' && handle ? `${key} "${handle}"` : `${key} ${index + 1}`);
      node = item;
      i++;
      continue;
    }
    parts.push(key);
    node = node?.[key];
  }
  return parts.join(' › ');
}
