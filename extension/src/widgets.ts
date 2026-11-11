// Small pieces of page shared by the popup and the button on videos: the quality and format
// lists, and what to say when a link was downloaded before. Everything is built with the DOM
// (never HTML text), and the texts come in through `t`.

import type { Outcome } from "./protocol";
import { cleanChoice, formatsFor, QUALITIES, type VideoChoice } from "./videochoice";

export type Say = (text: string, vars?: Record<string, string>) => string;

const KEY = "mohona.videoChoice";

/** The last choice, kept in the extension's own storage (never in the page's). */
export async function savedChoice(): Promise<VideoChoice> {
  try {
    const found = (await chrome.storage.local.get(KEY)) as Record<string, unknown>;
    return cleanChoice(found[KEY] as Partial<VideoChoice> | null | undefined);
  } catch {
    return cleanChoice(null);
  }
}

async function keepChoice(choice: VideoChoice): Promise<void> {
  try {
    await chrome.storage.local.set({ [KEY]: choice });
  } catch {
    /* only a convenience */
  }
}

/** Two lists, quality and format. `value()` is the choice to send with the download. */
export function videoPicker(t: Say, start: VideoChoice): { element: HTMLElement; value: () => VideoChoice } {
  const current = { ...start };
  const element = document.createElement("div");
  element.className = "pick";
  const quality = document.createElement("select");
  quality.setAttribute("aria-label", t("Quality"));
  for (const q of QUALITIES) {
    const option = document.createElement("option");
    option.value = q;
    option.textContent = q === "best" ? t("Best available") : q === "audio" ? t("Sound only") : `${q}p`;
    quality.append(option);
  }
  quality.value = current.quality;
  const format = document.createElement("select");
  format.setAttribute("aria-label", t("Format"));
  const fill = (): void => {
    format.replaceChildren();
    for (const f of formatsFor(quality.value)) {
      const option = document.createElement("option");
      option.value = f;
      option.textContent = f.toUpperCase();
      format.append(option);
    }
    format.value = quality.value === "audio" ? current.audio : current.container;
  };
  const value = (): VideoChoice => {
    const audio = quality.value === "audio";
    const next = cleanChoice({
      quality: quality.value,
      container: audio ? current.container : format.value,
      audio: audio ? format.value : current.audio,
    });
    Object.assign(current, next);
    void keepChoice(next);
    return next;
  };
  quality.addEventListener("change", () => {
    value();
    fill();
  });
  format.addEventListener("change", value);
  fill();
  element.append(quality, format);
  return { element, value };
}

/** What can be done about a link that was downloaded before. */
export interface Again {
  /** Open the finished file (null when it worked, else what went wrong). */
  open(): Promise<string | null>;
  /** Show it in its folder. */
  reveal(): Promise<string | null>;
  /** Download it anew, replacing the earlier one. */
  again(): Promise<void>;
}

type Duplicate = Extract<Outcome, { kind: "duplicate" }>;

/**
 * The answer to "this is in the list already": Open and Show in folder when the file is there,
 * a warning and "Download again" when it is gone or never finished, plain words while it runs.
 */
export function duplicateNote(t: Say, found: Duplicate, act: Again): HTMLElement {
  const box = document.createElement("div");
  box.className = "note";
  const text = document.createElement("p");
  text.title = found.message;
  const buttons = document.createElement("div");
  buttons.className = "actions";
  const add = (label: string, run: () => Promise<unknown>, primary = false): void => {
    const b = document.createElement("button");
    b.textContent = label;
    if (primary) b.classList.add("go");
    b.addEventListener("click", async () => {
      b.disabled = true;
      await run();
      b.disabled = false;
    });
    buttons.append(b);
  };
  const failure = (): HTMLElement => {
    const el = document.createElement("small");
    el.className = "problem";
    return el;
  };
  const problem = failure();
  const tryOpen = (run: () => Promise<string | null>) => async () => {
    const error = await run();
    problem.textContent = error ? t("Could not open it: {message}", { message: error }) : "";
  };
  const finished = found.status === "completed" || found.status === undefined;
  if (found.status === "completed" && found.exists && found.id !== undefined) {
    text.textContent = t("Already downloaded.");
    add(t("Open"), tryOpen(act.open), true);
    add(t("Show in folder"), tryOpen(act.reveal));
  } else if (found.status === "completed" || (finished && !found.exists)) {
    text.textContent = t("Downloaded before, but the file is no longer in its folder. Download it again?");
    add(t("Download again"), act.again, true);
  } else if (found.status === "failed" || found.status === "cancelled") {
    text.textContent = t("Already in the list, but it did not finish. Download it again?");
    add(t("Download again"), act.again, true);
  } else if (found.status === undefined) {
    text.textContent = found.message;
  } else {
    text.textContent = t("Already in the list, still in progress.");
  }
  box.append(text, buttons, problem);
  return box;
}
