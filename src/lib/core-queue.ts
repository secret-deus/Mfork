export type ProxyMode = "rule" | "global" | "direct";

/** A rejection belongs to its caller; it must never poison later operations. */
export function createSerialQueue() {
  let tail: Promise<unknown> = Promise.resolve();
  return function enqueue<T>(work: () => Promise<T>): Promise<T> {
    const result = tail.then(work, work);
    tail = result.catch(() => undefined);
    return result;
  };
}

export function confirmedMode(value: unknown): ProxyMode {
  if (typeof value !== "string") throw new Error("Cannot confirm current proxy mode");
  const mode = value.toLowerCase();
  if (mode !== "rule" && mode !== "global" && mode !== "direct") {
    throw new Error("Cannot confirm current proxy mode");
  }
  return mode;
}

/** Must run inside the same queue as mode/lifecycle changes and profile selection.
 *  `generate` returns the regenerated config *content*; `apply` sends it to
 *  the core as a payload (path-based reload is rejected in service mode). */
export async function applyPreservingMode(ops: {
  isCurrent: () => boolean;
  readMode: () => Promise<unknown>;
  generate: (mode: ProxyMode) => Promise<string | null>;
  apply: (payload: string) => Promise<void>;
  observed: (mode: ProxyMode) => void;
}): Promise<boolean> {
  if (!ops.isCurrent()) return false;
  const mode = confirmedMode(await ops.readMode());
  if (!ops.isCurrent()) return false;
  const payload = await ops.generate(mode);
  if (!payload || !ops.isCurrent()) return false;
  await ops.apply(payload);
  if (!ops.isCurrent()) return false;
  const actual = confirmedMode(await ops.readMode());
  if (!ops.isCurrent()) return false;
  ops.observed(actual);
  if (actual !== mode) throw new Error("Proxy mode changed while applying configuration; retry required");
  return true;
}
