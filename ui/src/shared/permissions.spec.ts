import { canAddEntries } from './permissions';

const user = (...roles: string[]) => (wanted: string[]) => wanted.some(role => roles.includes(role));

describe('canAddEntries', () => {
  it('allows inventory admin or inventory create alone', () => {
    expect(canAddEntries(user('ROLE_INVENTORY_ADMIN'))).toBe(true);
    expect(canAddEntries(user('ROLE_INVENTORY_CREATE'))).toBe(true);
  });

  it('allows managed-object rights only together with file rights', () => {
    expect(canAddEntries(user('ROLE_MANAGED_OBJECT_CREATE', 'ROLE_BINARY_CREATE'))).toBe(true);
    expect(canAddEntries(user('ROLE_MANAGED_OBJECT_ADMIN', 'ROLE_BINARY_ADMIN'))).toBe(true);
    expect(canAddEntries(user('ROLE_MANAGED_OBJECT_CREATE'))).toBe(false);
    expect(canAddEntries(user('ROLE_BINARY_CREATE'))).toBe(false);
  });

  it('refuses a user who can only read', () => {
    expect(canAddEntries(user('ROLE_INVENTORY_READ', 'ROLE_DEVICE_CONTROL_ADMIN'))).toBe(false);
  });
});
