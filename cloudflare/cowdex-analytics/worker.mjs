export default {
  async fetch(request, env) {
    if (new URL(request.url).pathname !== "/launch") return new Response(null, { status: 404 });
    if (request.method !== "POST") return new Response(null, { status: 405 });
    // Cap the body while streaming, including requests without Content-Length.
    const reader = request.body?.getReader();
    let bytes = new Uint8Array();
    if (!reader) return new Response(null, { status: 400 });
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      if (bytes.length + value.length > 1024) {
        await reader.cancel();
        return new Response(null, { status: 413 });
      }
      const next = new Uint8Array(bytes.length + value.length);
      next.set(bytes);
      next.set(value, bytes.length);
      bytes = next;
    }
    let event;
    try {
      event = JSON.parse(new TextDecoder().decode(bytes));
    } catch {
      return new Response(null, { status: 400 });
    }
    if (
      !event || event.event !== "launch" ||
      typeof event.installation_id !== "string" ||
      !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(event.installation_id) ||
      typeof event.version !== "string" || !/^[0-9A-Za-z.+-]{1,64}$/.test(event.version) ||
      !["linux", "macos", "windows"].includes(event.os) ||
      !["x86_64", "aarch64"].includes(event.arch) ||
      typeof event.first_launch !== "boolean" || typeof event.updated !== "boolean"
    ) return new Response(null, { status: 400 });
    env.COWDEX_ANALYTICS.writeDataPoint({
      indexes: [event.installation_id],
      blobs: ["launch", event.version, event.os, event.arch],
      doubles: [1, Number(event.first_launch), Number(event.updated)],
    });
    return new Response(null, { status: 204 });
  },
};
