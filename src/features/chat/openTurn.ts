const ATTEMPTS = 20;
const WAIT_MS = 150;

export function openTurn(turnId: string) {
  let attempts = 0;
  const look = () => {
    const turn = document.querySelector(`article[data-turn-id="${CSS.escape(turnId)}"]`);
    if (turn) {
      turn.scrollIntoView({ block: "center" });
      return;
    }
    attempts += 1;
    if (attempts >= ATTEMPTS) return;
    const older = [...document.querySelectorAll("button")].find(
      (button) => button.textContent === "Load older messages" && !button.disabled,
    );
    older?.click();
    setTimeout(look, WAIT_MS);
  };
  setTimeout(look, WAIT_MS);
}
