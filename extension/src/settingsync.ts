// Keeping the extension's settings and Mohona's copy the same. Mohona stores one text
// (the settings as JSON) so the desktop app and the extension edit the same thing.
// The copy changed last (by `updated`) wins; equal copies need nothing.

import { sanitize, type Settings } from "./settings";

export interface Plan {
  /** Take this copy from Mohona. */
  apply?: Settings;
  /** Give Mohona this copy. */
  push?: Settings;
}

export function reconcile(local: Settings, remoteText: string | null): Plan {
  if (remoteText === null) return local.updated > 0 ? { push: local } : {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(remoteText);
  } catch {
    return local.updated > 0 ? { push: local } : {};
  }
  if (typeof parsed !== "object" || parsed === null) return local.updated > 0 ? { push: local } : {};
  const remote = sanitize(parsed);
  if (remote.updated > local.updated) return { apply: remote };
  if (remote.updated < local.updated) return { push: local };
  return {};
}
