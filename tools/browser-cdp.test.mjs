import assert from "node:assert/strict";
import { test } from "node:test";
import { connectCdp } from "./browser-cdp.mjs";

class Socket extends EventTarget {
  readyState = 1;
  sent = [];
  send(data) { this.sent.push(JSON.parse(data)); }
  receive(message) { this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(message) })); }
}

test("CDP matches concurrent responses and reports protocol errors", async () => {
  const socket = new Socket();
  const connection = await connectCdp(socket);
  const first = connection.call("Page.enable");
  const second = connection.call("Runtime.enable");
  const failed = assert.rejects(second, /unsupported/);
  socket.receive({ id: 2, error: { message: "unsupported" } });
  socket.receive({ id: 1, result: { enabled: true } });
  assert.deepEqual(await first, { enabled: true });
  await failed;
});

test("unanswered requests time out and late responses are ignored", async () => {
  const socket = new Socket();
  const connection = await connectCdp(socket, { timeout: 10 });
  await assert.rejects(connection.call("Runtime.evaluate"), /timeout: Runtime.evaluate/);
  socket.receive({ id: 1, result: {} });
});

for (const event of ["close", "error"]) {
  test(`socket ${event} rejects all pending and future requests`, async () => {
    const socket = new Socket();
    const connection = await connectCdp(socket);
    const pending = [connection.call("Page.enable"), connection.call("Runtime.enable")];
    const rejected = pending.map(promise => assert.rejects(promise, /CDP socket/));
    socket.dispatchEvent(new Event(event));
    await Promise.all(rejected);
    await assert.rejects(connection.call("Page.navigate"), /CDP socket/);
    assert.throws(() => connection.check(), /CDP socket/);
  });
}

test("socket opening has a deadline", async () => {
  const socket = new Socket();
  socket.readyState = 0;
  await assert.rejects(connectCdp(socket, { timeout: 10 }), /open timeout/);
});

test("asynchronous event failures reject pending requests", async () => {
  const socket = new Socket();
  const connection = await connectCdp(socket, { onEvent: async () => { throw new Error("event failed"); } });
  const pending = assert.rejects(connection.call("Page.navigate"), /event failed/);
  socket.receive({ method: "Fetch.requestPaused" });
  await pending;
});
