/**
 * Checking a connector file before it is saved.
 *
 * The rules are the docs site's config builder's: TOML syntax, then the contract schema
 * (doc/contract/schemas/config.schema.json) with the protocol shapes from
 * site/src/data/protocols.json. scripts/sync-validation.mjs copies that module into
 * src/generated, so the UI and the config builder always give the same verdict.
 *
 * What only the gateway knows, such as which point libraries exist there, is out of reach.
 */
import { parseToml, validateConfig, type Config, type FieldError } from '../generated/config-builder';

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
