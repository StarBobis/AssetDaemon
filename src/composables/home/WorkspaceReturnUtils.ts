/**
 * WorkspaceReturnUtils - builds the public API object returned by useWorkspace.
 *
 * Keeping the long return map here prevents the main composable from becoming a scroll-heavy
 * list of property names while still returning the exact same references and methods.
 */

/** Return object builder. */
export class WorkspaceReturnUtils {
  /** Return the given API map without changing identities or wrapping refs. */
  static build<T extends Record<string, unknown>>(workspaceApi: T): T {
    return workspaceApi
  }
}
