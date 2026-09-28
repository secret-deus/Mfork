/** Saving and applying are separate commits: an apply error never makes saved data dirty. */
export async function persistOverride(ops: {
  write: (() => Promise<unknown>) | null;
  saved: () => void;
  apply: (() => Promise<void>) | null;
}): Promise<void> {
  if (ops.write) {
    await ops.write();
    ops.saved();
  }
  if (ops.apply) await ops.apply();
}
