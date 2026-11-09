import { create } from "zustand";
import { api, previewSnapshot } from "../lib/api";
import { errorText } from "../lib/utils";
import type { AppSnapshot } from "../lib/types";
interface AppStore {
  snapshot: AppSnapshot;
  loading: boolean;
  error: string | null;
  setSnapshot: (snapshot: AppSnapshot) => void;
  refresh: () => Promise<void>;
  notify: (error: unknown) => void;
  clearError: () => void;
}
export const useApp = create<AppStore>((set) => ({
  snapshot: previewSnapshot,
  loading: true,
  error: null,
  setSnapshot: (snapshot) => set({ snapshot, loading: false }),
  refresh: async () => {
    try {
      set({ snapshot: await api.snapshot(), loading: false });
    } catch (error) {
      set({ error: errorText(error), loading: false });
    }
  },
  notify: (error) => set({ error: errorText(error) }),
  clearError: () => set({ error: null }),
}));
