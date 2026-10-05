// Bounded CDP transport for browser acceptance checks.
export async function connectCdp(socket, { timeout = 20_000, onEvent = () => {} } = {}) {
  let nextId = 0;
  let failure;
  const pending = new Map();
  const fail = error => {
    failure ??= error;
    for (const request of pending.values()) request.reject(failure);
    pending.clear();
  };
  socket.addEventListener("close", () => fail(new Error("CDP socket closed")));
  socket.addEventListener("error", () => fail(new Error("CDP socket error")));
  socket.addEventListener("message", ({ data }) => {
    try {
      const message = JSON.parse(data);
      if (message.id) {
        const request = pending.get(message.id);
        if (!request) return;
        pending.delete(message.id);
        if (message.error) request.reject(new Error(message.error.message));
        else request.resolve(message.result);
      } else {
        Promise.resolve(onEvent(message)).catch(fail);
      }
    } catch (error) {
      fail(error);
    }
  });
  if (socket.readyState !== 1) {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => finish(new Error("CDP socket open timeout")), timeout);
      const opened = () => finish();
      const closed = () => finish(new Error("CDP socket closed before opening"));
      const errored = () => finish(new Error("CDP socket error before opening"));
      function finish(error) {
        clearTimeout(timer);
        socket.removeEventListener("open", opened);
        socket.removeEventListener("close", closed);
        socket.removeEventListener("error", errored);
        if (error) { fail(error); reject(error); }
        else resolve();
      }
      socket.addEventListener("open", opened);
      socket.addEventListener("close", closed);
      socket.addEventListener("error", errored);
      if (socket.readyState > 1) closed();
    });
  }
  return {
    call(method, params = {}) {
      if (failure) return Promise.reject(failure);
      const id = ++nextId;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          pending.delete(id);
          reject(new Error(`CDP request timeout: ${method}`));
        }, timeout);
        const settle = callback => value => { clearTimeout(timer); callback(value); };
        pending.set(id, { resolve: settle(resolve), reject: settle(reject) });
        try { socket.send(JSON.stringify({ id, method, params })); }
        catch (error) { fail(error); }
      });
    },
    check() { if (failure) throw failure; },
  };
}
