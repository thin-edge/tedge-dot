/**
 * Duplicating a repository entry: the copy's file name, and its text with what has to differ
 * from the original's.
 */
import { parseToml, setAt } from '../generated/config-builder';
import { uniqueId } from './form-model';
import { type FileKind } from './naming';
import { writeKeepingLayout } from './toml-write';

/** `<name>-copy`, or `<name>-copy_2` and so on when that file name is taken. */
export function copyFileName(name: string, taken: readonly string[]): string {
  return uniqueId(`${name}-copy`, taken);
}

/**
 * The copy's text. A connector file gets the service name `tedge-dot-<file name>`: the service
 * name must be unique on a gateway's broker, so a copy with the original's could not run next to
 * it. The rest, comments included, stays as it is. A point library and a file that does not
 * parse are copied unchanged.
 */
export function duplicateText(kind: FileKind, text: string, fileName: string): string {
  if (kind !== 'connector') {
    return text;
  }
  try {
    const next = setAt(parseToml(text), '/connector/service_name', `tedge-dot-${fileName}`);
    return writeKeepingLayout(text, next) ?? text;
  } catch {
    return text;
  }
}
