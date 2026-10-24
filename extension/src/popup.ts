import type { Reply, Request } from "./messages";
import type { Outcome } from "./protocol";

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

function ask(request: Request): Promise<Reply> {
  return chrome.runtime.sendMessage(request) as Promise<Reply>;
}

function describe(outcome: Outcome | null): string {
  if (!outcome) return "";
  switch (outcome.kind) {
    case "added":
      return "Added to Mohona.";
    case "duplicate":
      return `Not added: ${outcome.message}`;
    case "error":
      return `Could not add: ${outcome.message}`;
  }
}

async function refresh(): Promise<void> {
  const reply = await ask({ kind: "status" });
  if (reply.kind !== "status") return;
  const s = reply.status;
  const text = $("status");
  const start = $<HTMLButtonElement>("start");
  start.hidden = s.state !== "engine_stopped";
  $<HTMLButtonElement>("add").disabled = s.state === "host_missing" || s.state === "incompatible";
  switch (s.state) {
    case "ready":
      text.textContent = `Mohona ${s.version} is running.`;
      break;
    case "engine_stopped":
      text.textContent = "Mohona is not running.";
      break;
    case "host_missing":
      text.textContent = `Mohona is not connected: ${s.message}. Run "mohona-host install".`;
      break;
    case "incompatible":
      text.textContent = s.message;
      break;
  }
}

$("start").addEventListener("click", async () => {
  const reply = await ask({ kind: "startEngine" });
  if (reply.kind === "outcome") $("result").textContent = describe(reply.outcome);
  await refresh();
});

$<HTMLFormElement>("form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const url = $<HTMLInputElement>("url").value.trim();
  if (!url) return;
  const reply = await ask({ kind: "add", url });
  if (reply.kind === "outcome") $("result").textContent = describe(reply.outcome);
  await refresh();
});

void refresh();
