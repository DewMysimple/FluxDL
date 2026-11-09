import { useState } from "react";
import { useApp } from "../stores/app";

export function useAction() {
  const [busy, setBusy] = useState(false);
  const notify = useApp((s) => s.notify);
  const refresh = useApp((s) => s.refresh);
  const run = async (action: () => Promise<unknown>, shouldRefresh = true) => {
    if (busy) return;
    setBusy(true);
    try {
      await action();
      if (shouldRefresh) await refresh();
    } catch (error) {
      notify(error);
    } finally {
      setBusy(false);
    }
  };
  return { busy, run };
}
