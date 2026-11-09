import { CircleAlert, X } from "lucide-react";
import { useApp } from "../stores/app";
import { IconButton } from "./ui";

export function ErrorToast() {
  const error = useApp((state) => state.error);
  const clearError = useApp((state) => state.clearError);
  return error ? (
    <div className="error-toast" role="alert">
      <CircleAlert size={19} />
      <span>{error}</span>
      <IconButton label="关闭提示" onClick={clearError}>
        <X size={16} />
      </IconButton>
    </div>
  ) : null;
}
