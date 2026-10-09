/* SPDX-FileCopyrightText: Copyright (c) 2022-2026 trobonox <hello@trobo.dev>

SPDX-License-Identifier: GPL-3.0-or-later

Kanri is an offline Kanban board app made using Tauri and Nuxt.
Copyright (C) 2022-2026 trobonox <hello@trobo.dev>

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

import type { Board } from "@/types/kanban-types";
import type { Ref } from "vue";

import { convertFileSrc } from "@tauri-apps/api/core";

import { getAverageColor, getContrast, rgbToHex } from "@/utils/colorUtils";

const imageLoads = (src: string) =>
  new Promise<boolean>((resolve) => {
    const img = new Image();
    img.onload = () => resolve(true);
    img.onerror = () => resolve(false);
    img.src = src;
  });

const DEFAULT_BLUR = "8px";
const DEFAULT_BRIGHTNESS = "100%";

type UseBackgroundImageOptions = {
  checkFileExists?: boolean;
  mutateBoardOnMissingFile?: boolean;
  syncBoardOnSetters?: boolean;
  computeTitleColor?: boolean;
};

export function useBackgroundImage(
  boardContent: Ref<Board | null | undefined>,
  options: UseBackgroundImageOptions = {}
) {
  const {
    checkFileExists = true,
    // Kanri Sync: never delete the setting just because the image file is
    // missing here (e.g. still being transferred from another device).
    mutateBoardOnMissingFile = false,
    syncBoardOnSetters = true,
    computeTitleColor = true,
  } = options;

  const bgCustom = ref("");
  const bgCustomNoResolution = ref("");
  const showCustomBgModal = ref(false);
  const bgImageLoaded = ref(false);
  const bgBlur = ref(DEFAULT_BLUR);
  const bgBrightness = ref(DEFAULT_BRIGHTNESS);
  const boardTitleColor = ref("");

  const cssVars = computed(() => {
    return {
      "--bg-brightness": bgBrightness.value,
      "--bg-custom-image": `url("${bgCustom.value}")`,
      "--blur-intensity": bgBlur.value,
    };
  });

  const refreshBoardTitleTextColor = async () => {
    if (!computeTitleColor) {
      boardTitleColor.value = "";
      return;
    }

    if (bgCustom.value == null || !/\S/.test(bgCustom.value)) {
      boardTitleColor.value = "";
      return;
    }

    const averageColorFromBackground = await getAverageColor(bgCustom.value);
    const hexColor = rgbToHex(
      averageColorFromBackground[0],
      averageColorFromBackground[1],
      averageColorFromBackground[2]
    );

    boardTitleColor.value = getContrast(hexColor);
  };

  const updateBoardBackground = () => {
    if (!syncBoardOnSetters) return;
    if (!boardContent.value) return;

    boardContent.value.background = {
      blur: bgBlur.value,
      brightness: bgBrightness.value,
      src: bgCustomNoResolution.value,
    };
  };

  const initBackgroundImage = async () => {
    if (!boardContent.value || !boardContent.value.background) {
      bgImageLoaded.value = true;
      return;
    }

    const background = boardContent.value.background;
    bgCustomNoResolution.value = background.src;

    // Synced background whose image has not arrived on this device yet.
    if (!background.src) {
      bgImageLoaded.value = true;
      return;
    }

    // Kanri Sync: check the file by loading it through the asset protocol
    // (the same way it is displayed). The fs plugin's exists() is refused on
    // Android for the synced-images folder, which hid every synced picture.
    let src = "";
    try {
      src = convertFileSrc(background.src);
    } catch (e) {
      console.error("Error converting file src: ", e);
    }

    if (!src || (checkFileExists && !(await imageLoads(src)))) {
      console.warn("Background image could not be loaded on this device");
      if (mutateBoardOnMissingFile) {
        boardContent.value.background = null;
      }
      bgImageLoaded.value = true;
      return;
    }

    bgCustom.value = src;
    bgBlur.value = background.blur;
    bgBrightness.value = background.brightness;

    if (computeTitleColor) {
      try {
        await refreshBoardTitleTextColor();
      } catch (e) {
        console.warn("Could not compute board title color", e);
      }
    }
    bgImageLoaded.value = true;
  };

  const setBackgroundImage = async (img: string) => {
    bgCustomNoResolution.value = img;
    bgCustom.value = convertFileSrc(img);
    updateBoardBackground();

    if (computeTitleColor) {
      await refreshBoardTitleTextColor().catch((e) =>
        console.warn("Could not compute board title color", e)
      );
    }
  };

  const resetBackground = () => {
    bgCustom.value = "";
    bgCustomNoResolution.value = "";
    bgBlur.value = DEFAULT_BLUR;
    bgBrightness.value = DEFAULT_BRIGHTNESS;

    if (syncBoardOnSetters && boardContent.value) {
      delete boardContent.value.background;
    }

    boardTitleColor.value = "";
  };

  const setBlur = (blurAmount: string) => {
    bgBlur.value = blurAmount;
    updateBoardBackground();
  };

  const setBrightness = (brightnessAmount: string) => {
    bgBrightness.value = brightnessAmount;
    updateBoardBackground();
  };

  return {
    bgCustom,
    bgCustomNoResolution,
    showCustomBgModal,
    bgImageLoaded,
    bgBlur,
    bgBrightness,
    boardTitleColor,
    cssVars,
    initBackgroundImage,
    refreshBoardTitleTextColor,
    setBackgroundImage,
    resetBackground,
    setBlur,
    setBrightness,
  };
}
