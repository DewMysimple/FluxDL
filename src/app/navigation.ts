import { History, LayoutDashboard, ListVideo, Settings2 } from "lucide-react";

export type Page = "home" | "queue" | "history" | "settings" | "diagnostics";

export const pages = [
  { id: "home" as const, label: "工作台", icon: LayoutDashboard },
  { id: "queue" as const, label: "下载队列", icon: ListVideo },
  { id: "history" as const, label: "下载历史", icon: History },
  { id: "settings" as const, label: "设置", icon: Settings2 },
];
