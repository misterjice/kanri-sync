/* SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors

SPDX-License-Identifier: GPL-3.0-or-later
*/

/** True on phones/tablets where the primary input is a finger. */
export const isTouchDevice = (): boolean =>
  typeof window !== "undefined" && window.matchMedia?.("(pointer: coarse)").matches === true;

/**
 * Delay before a touch turns into a drag (press and hold to pick up a card
 * or column). Any movement before then is treated as a scroll instead.
 */
export const touchDragDelay = (): number => (isTouchDevice() ? 500 : 0);

/**
 * Touch workarounds, installed once at startup on touch devices:
 *
 * 1. smooth-dnd (card/column drag-and-drop) adds classes to <body> on
 *    touchstart that set `touch-action: none` on the whole page, and only
 *    removes them on `mouseup`. Touch gestures often never produce a mouseup,
 *    so scrolling stayed disabled until some later tap. Clear them whenever a
 *    finger lifts.
 * 2. Keep the focused text field visible when the on-screen keyboard opens.
 */
export const installTouchFixes = (): void => {
  if (!isTouchDevice()) return;
  const release = () => {
    document.body.classList.remove("smooth-dnd-disable-touch-action", "smooth-dnd-no-user-select");
  };
  document.addEventListener("touchend", release, { capture: true, passive: true });
  document.addEventListener("touchcancel", release, { capture: true, passive: true });

  document.addEventListener("focusin", (e) => {
    const el = e.target as HTMLElement | null;
    if (!el) return;
    const editable = el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable;
    if (!editable) return;
    // Wait for the keyboard to finish opening and the app to resize.
    setTimeout(() => el.scrollIntoView({ block: "center", behavior: "smooth" }), 350);
  });
};
