import { ErrorToast } from "./components/feedback";
import { useWindowAppearance } from "./hooks/useWindowAppearance";
import { QuickCaptureWindow } from "./windows/QuickCaptureWindow";
import { WorkspaceWindow } from "./windows/WorkspaceWindow";
export default function App() {
  const quick =
    new URLSearchParams(window.location.search).get("window") === "quick";
  useWindowAppearance();
  return (
    <>
      <div className={`application ${quick ? "quick-application" : ""}`}>
        {quick ? <QuickCaptureWindow /> : <WorkspaceWindow />}
      </div>
      <ErrorToast />
    </>
  );
}
