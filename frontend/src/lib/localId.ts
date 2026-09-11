let nextLocalId = 0;

/**
 * Creates an identifier that is unique within the current browser session.
 * Intended only for transient UI state; it must not be persisted or used as a
 * security token. The counter deliberately avoids Web Crypto APIs because they
 * are unavailable when Aurum is opened over plain HTTP from a LAN IP address.
 */
export function createLocalId(prefix: string): string {
  nextLocalId += 1;
  return `${prefix}-${Date.now().toString(36)}-${nextLocalId.toString(36)}`;
}
