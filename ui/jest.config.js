/**
 * Unit tests for the plugin's logic. The logic lives in plain TypeScript files that do not
 * import Angular or @c8y/ngx-components, so the tests need neither a browser nor TestBed.
 */
module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  roots: ['<rootDir>/src'],
  testMatch: ['**/*.spec.ts'],
  clearMocks: true,
  transform: {
    '^.+\\.ts$': ['ts-jest', { tsconfig: { module: 'commonjs', esModuleInterop: true, resolveJsonModule: true } }]
  },
  transformIgnorePatterns: ['/node_modules/(?!smol-toml)'],
};
