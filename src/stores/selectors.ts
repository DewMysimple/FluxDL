import { useMemo } from "react";
import { isActive } from "../lib/utils";
import { useApp, type AppStore } from "./app";

export type TaskListing = "all" | "active" | "queued" | "completed" | "history";

export function selectTaskIds(
  state: AppStore,
  listing: TaskListing,
  query = "",
): string[] {
  let ids = state.taskIds.filter((id) => {
    const task = state.tasksById[id];
    if (listing === "active") return isActive(task.status);
    if (listing === "queued") return task.status === "queued";
    if (listing === "completed" || listing === "history")
      return task.status === "completed";
    return true;
  });
  if (listing === "history")
    ids = ids.sort((a, b) =>
      state.tasksById[b].updatedAt.localeCompare(state.tasksById[a].updatedAt),
    );
  if (query) {
    const needle = query.toLocaleLowerCase();
    ids = ids.filter((id) => {
      const media = state.tasksById[id].media;
      return `${media.title} ${media.source}`
        .toLocaleLowerCase()
        .includes(needle);
    });
  }
  return ids;
}

/** Per-consumer cache, invalidated by membership/search/order, not progress. */
export function createTaskListingSelector(listing: TaskListing, query = "") {
  let revision = -1;
  let sourceIds: string[] | undefined;
  let ids: string[] = [];
  return (state: AppStore) => {
    if (revision !== state.listingRevision || sourceIds !== state.taskIds) {
      revision = state.listingRevision;
      sourceIds = state.taskIds;
      ids = selectTaskIds(state, listing, query);
    }
    return ids;
  };
}

export function useTaskIds(listing: TaskListing, query = "") {
  const selector = useMemo(
    () => createTaskListingSelector(listing, query),
    [listing, query],
  );
  return useApp(selector);
}
