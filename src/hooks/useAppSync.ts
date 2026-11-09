import { useEffect } from "react";
import { api, isDesktop } from "../lib/api";
import { errorText } from "../lib/utils";
import { useApp } from "../stores/app";
interface SyncCallbacks {
  onClose?: () => void;
  onShutdownFailed?: () => void;
}
interface Subscription {
  subscribe: () => Promise<() => void>;
  unlisten?: () => void;
  pending?: Promise<void>;
}
export function useAppSync({ onClose, onShutdownFailed }: SyncCallbacks = {}) {
  const refresh = useApp((s) => s.refresh);
  const beginSync = useApp((s) => s.beginSync);
  const applyEvent = useApp((s) => s.applyEvent);
  const notify = useApp((s) => s.notify);
  useEffect(() => {
    let disposed = false;
    const disposeListener = (unlisten: () => void) => {
      try {
        Promise.resolve(unlisten()).catch((error) => {
          console.warn("FluxDL订阅清理失败：", errorText(error));
        });
      } catch (error) {
        console.warn("FluxDL订阅清理失败：", errorText(error));
      }
    };
    const subscriptions: Subscription[] = isDesktop
      ? [
          {
            subscribe: () =>
              api.onChange((event) => {
                if (!disposed) applyEvent(event);
              }),
          },
          {
            subscribe: () =>
              api.onClose(() => {
                if (!disposed) onClose?.();
              }),
          },
          {
            subscribe: () =>
              api.onShutdownFailed((message) => {
                if (!disposed) {
                  onShutdownFailed?.();
                  notify(message);
                }
              }),
          },
        ]
      : [];
    const ensureSubscriptions = async () => {
      if (disposed) return;
      await Promise.all(
        subscriptions.map((subscription) => {
          if (subscription.unlisten) return Promise.resolve();
          if (subscription.pending) return subscription.pending;
          subscription.pending = Promise.resolve()
            .then(subscription.subscribe)
            .then((unlisten) => {
              if (disposed) disposeListener(unlisten);
              else subscription.unlisten = unlisten;
            })
            .catch((error) => {
              if (!disposed) notify("无法监听本地状态：" + errorText(error));
            })
            .finally(() => {
              subscription.pending = undefined;
            });
          return subscription.pending;
        }),
      );
    };
    const stop = beginSync(ensureSubscriptions);
    void refresh();
    const focus = () => {
      if (!disposed) void refresh();
    };
    window.addEventListener("focus", focus);
    return () => {
      disposed = true;
      stop();
      window.removeEventListener("focus", focus);
      for (const subscription of subscriptions)
        if (subscription.unlisten) disposeListener(subscription.unlisten);
    };
  }, [refresh, beginSync, applyEvent, notify, onClose, onShutdownFailed]);
}
