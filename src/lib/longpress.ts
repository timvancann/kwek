// Touch-and-hold, as a Svelte action: `use:longpress={() => ...}`. Touch only (a mouse has its
// hover toolbar); a finger that moves is scrolling, not holding. The tap that ends a hold does
// not also count as a click.

const HOLD_MS = 450;
const SLOP_PX = 10;

export function longpress(node: HTMLElement, onHold: () => void) {
  let cb = onHold;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let start: { x: number; y: number } | null = null;
  let held = false;

  const cancel = () => {
    clearTimeout(timer);
    start = null;
  };
  const down = (e: PointerEvent) => {
    if (e.pointerType !== "touch") return;
    held = false;
    start = { x: e.clientX, y: e.clientY };
    timer = setTimeout(() => {
      held = true;
      start = null;
      navigator.vibrate?.(10);
      cb();
    }, HOLD_MS);
  };
  const move = (e: PointerEvent) => {
    if (start && Math.hypot(e.clientX - start.x, e.clientY - start.y) > SLOP_PX) cancel();
  };
  const click = (e: MouseEvent) => {
    if (!held) return;
    held = false;
    e.preventDefault();
    e.stopPropagation();
  };
  // The webview's own long-press menu (select text, copy) would open over the sheet.
  const menu = (e: Event) => {
    if (held || start) e.preventDefault();
  };

  node.addEventListener("pointerdown", down);
  node.addEventListener("pointermove", move);
  node.addEventListener("pointerup", cancel);
  node.addEventListener("pointercancel", cancel);
  node.addEventListener("click", click, true);
  node.addEventListener("contextmenu", menu);
  return {
    update(next: () => void) {
      cb = next;
    },
    destroy() {
      cancel();
      node.removeEventListener("pointerdown", down);
      node.removeEventListener("pointermove", move);
      node.removeEventListener("pointerup", cancel);
      node.removeEventListener("pointercancel", cancel);
      node.removeEventListener("click", click, true);
      node.removeEventListener("contextmenu", menu);
    },
  };
}
