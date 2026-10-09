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
