import { focusManager } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function listenToWindowFocus() {
  focusManager.setEventListener((handleFocus) => {
    const gained = () => handleFocus(true);
    const lost = () => handleFocus(false);
    window.addEventListener("focus", gained);
    window.addEventListener("blur", lost);
    const unlisten = getCurrentWindow().onFocusChanged(({ payload }) => handleFocus(payload));
    return () => {
      window.removeEventListener("focus", gained);
      window.removeEventListener("blur", lost);
      void unlisten.then((stop) => stop());
    };
  });
}
