import assert from "node:assert/strict";
import { test } from "node:test";
import worker from "./worker.mjs";

const event = {
  event: "launch", installation_id: "00000000-0000-4000-8000-000000000001",
  version: "0.164.0", os: "linux", arch: "x86_64", first_launch: true, updated: false,
};

test("launch writes only the advertised anonymous fields", async () => {
  const points = [];
  const response = await worker.fetch(new Request("https://test/launch", {
    method: "POST", body: JSON.stringify({ ...event, email: "ignored@example.com" }),
  }), { COWDEX_ANALYTICS: { writeDataPoint: point => points.push(point) } });
  assert.equal(response.status, 204);
  assert.deepEqual(points, [{ indexes: [event.installation_id], blobs: ["launch", "0.164.0", "linux", "x86_64"], doubles: [1, 1, 0] }]);
});

test("invalid and oversized payloads never write a point", async () => {
  for (const [body, status] of [["invalid", 400], [JSON.stringify({ ...event, version: "x".repeat(65) }), 400], [" ".repeat(1025), 413]]) {
    const response = await worker.fetch(new Request("https://test/launch", { method: "POST", body }), {
      COWDEX_ANALYTICS: { writeDataPoint() { assert.fail("unexpected point"); } },
    });
    assert.equal(response.status, status);
  }
});
