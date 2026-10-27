import { beforeEach, expect, test, vi } from "vitest";

const native = vi.hoisted(() => ({ settle: vi.fn(), remove: vi.fn(), forgetDrafts: vi.fn() }));
vi.mock("$domain/operations", () => ({ settle: native.settle }));
vi.mock("$shared/lib/drafts", () => ({ forgetDrafts: native.forgetDrafts }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ profilesDelete: native.remove });
});

beforeEach(() => {
  vi.clearAllMocks();
  native.remove.mockResolvedValue({ accepted: true, operation_id: "deletion" });
});

test.each([
  { outcome: "deferred", disposition: null },
  { outcome: "failed", reason: "timeout", operation_id: "deletion" },
  { outcome: "failed", reason: "transport", operation_id: "deletion" },
  { outcome: "failed", reason: "timeout" },
])("keeps drafts when deletion is unconfirmed: $outcome", async (settlement) => {
  native.settle.mockResolvedValue(settlement);
  const { remove } = await import("../profiles.svelte");
  expect(await remove("profile")).toBe("unconfirmed");
  expect(native.forgetDrafts).not.toHaveBeenCalled();
});

test("forgets drafts only after deletion is confirmed", async () => {
  native.settle.mockResolvedValue({ outcome: "applied", disposition: null });
  const { remove } = await import("../profiles.svelte");
  expect(await remove("profile")).toBe("removed");
  expect(native.forgetDrafts).toHaveBeenCalledWith("profile");
});

test("reports a rejected deletion as failed", async () => {
  native.settle.mockResolvedValue({ outcome: "rejected", disposition: null });
  const { remove } = await import("../profiles.svelte");
  expect(await remove("profile")).toBe("failed");
  expect(native.forgetDrafts).not.toHaveBeenCalled();
});
