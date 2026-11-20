/**
 * TOML highlighting for Monaco, which has no TOML mode of its own: comments, table headers,
 * keys, strings, numbers (including 0o, 0x and 0b), booleans and dates.
 */

export const TOML_LANGUAGE_ID = 'toml';

export const tomlLanguage = {
  defaultToken: '',
  tokenPostfix: '.toml',
  tokenizer: {
    root: [
      [/#.*$/, 'comment'],
      [/^\s*\[\[[^\]]*\]\]/, 'type'],
      [/^\s*\[[^\]]*\]/, 'type'],
      [/[A-Za-z0-9_.-]+(?=\s*=)/, 'key'],
      [/"""/, 'string', '@mlBasic'],
      [/'''/, 'string', '@mlLiteral'],
      [/"([^"\\]|\\.)*"/, 'string'],
      [/'[^']*'/, 'string'],
      [/\b(true|false)\b/, 'keyword'],
      [/\d{4}-\d{2}-\d{2}([Tt ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?([Zz]|[+-]\d{2}:\d{2})?)?/, 'number'],
      [/[+-]?(0x[0-9A-Fa-f_]+|0o[0-7_]+|0b[01_]+)/, 'number'],
      [/[+-]?(inf|nan)\b/, 'number'],
      [/[+-]?\d[\d_]*(\.\d[\d_]*)?([eE][+-]?\d+)?/, 'number'],
    ],
    mlBasic: [
      [/"""/, 'string', '@pop'],
      [/./, 'string'],
    ],
    mlLiteral: [
      [/'''/, 'string', '@pop'],
      [/./, 'string'],
    ],
  },
};
