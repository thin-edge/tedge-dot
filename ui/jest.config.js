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
    '^.+\\.ts$': ['ts-jest', { tsconfig: { module: 'commonjs', esModuleInterop: true, resolveJsonModule: true } }],
    // toml-patch ships ES modules only; Jest runs CommonJS.
    '^.+/node_modules/@decimalturn/toml-patch/.+\\.js$': ['ts-jest', { tsconfig: { module: 'commonjs', allowJs: true } }],
  },
  transformIgnorePatterns: ['/node_modules/(?!(smol-toml|@decimalturn/toml-patch)/)'],
};
