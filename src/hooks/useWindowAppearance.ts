import { useEffect } from "react";
import { useApp } from "../stores/app";

export function useWindowAppearance() {
  const theme = useApp((state) => state.settings.theme);
  const density = useApp((state) => state.settings.density);
  const reducedMotion = useApp((state) => state.settings.reducedMotion);
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: light)");
    const apply = () => {
      document.documentElement.dataset.theme =
        theme === "system" ? (media.matches ? "light" : "dark") : theme;
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);
  useEffect(() => {
    document.documentElement.dataset.density = density;
  }, [density]);
  useEffect(() => {
    document.documentElement.dataset.reducedMotion = String(reducedMotion);
  }, [reducedMotion]);
}
