import { call } from "../../lib/api";
import type {
  DiagnosticFilter,
  DiagnosticPage,
  Overview,
  Health,
} from "./types";
export const diagnosticsApi = {
  health: () => call<Health>("get_diagnostics_health"),
  query: (filter: DiagnosticFilter = {}) =>
    call<DiagnosticPage>("query_diagnostic_events", { filter }),
  overview: (filter: DiagnosticFilter = {}) =>
    call<Overview>("get_diagnostics_overview", { filter }),
  refresh: () => call<void>("refresh_diagnostic_report"),
  open: () => call<void>("open_diagnostics_directory"),
  export: (filter: DiagnosticFilter = {}) =>
    call<string | null>("export_diagnostics_bundle", { filter }),
  reveal: () => call<void>("reveal_diagnostics_bundle"),
};
