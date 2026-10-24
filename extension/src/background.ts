import { addDownload, NativeClient, startEngine, status } from "./native";
import type { Reply, Request } from "./messages";
import { HOST_NAME } from "./protocol";

const client = new NativeClient(() => chrome.runtime.connectNative(HOST_NAME));

async function handle(request: Request): Promise<Reply> {
  switch (request.kind) {
    case "status":
      return { kind: "status", status: await status(client) };
    case "startEngine":
      return { kind: "outcome", outcome: await startEngine(client) };
    case "add":
      return { kind: "outcome", outcome: await addDownload(client, { url: request.url }) };
  }
}

chrome.runtime.onMessage.addListener((message: Request, _sender, sendResponse) => {
  void handle(message).then(sendResponse);
  return true; // the answer comes later
});
