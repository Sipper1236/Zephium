import "$styles/global.css";
import { expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import type { ProfilesView } from "$shared/ipc/bindings";
import { requestNew } from "$session/profile-sheet.svelte";
import ProfilesPage from "../components/sections/ProfilesPage.svelte";

const native = vi.hoisted(() => ({
  create: vi.fn(async () => ({ accepted: true, operation_id: null })),
  remove: vi.fn(async () => ({ accepted: true, operation_id: "deletion" })),
  settle: vi.fn(async (admission: Promise<unknown>) => {
    await admission;
    return { outcome: "applied", disposition: null };
  }),
}));
vi.mock("$domain/operations", () => ({ settle: native.settle }));
const roster: ProfilesView = {
  current: "01J00000000000000000000001",
  private: false,
  profiles: [
    {
      id: "01J00000000000000000000001",
      name: "Personal",
      kind: "default",
      avatar: { style: "initials", color: 1, symbol: 0, revision: 0 },
    },
    {
      id: "01J00000000000000000000002",
      name: "Work",
      kind: "named",
      avatar: { style: "initials", color: 2, symbol: 0, revision: 0 },
    },
  ],
};
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    profilesGet: async () => ({ status: "ok" as const, data: roster }),
    profilesCreate: native.create,
    profilesDelete: native.remove,
  });
});

test("an unconfirmed profile deletion stays visible and cannot be submitted twice", async () => {
  native.settle.mockImplementationOnce(async (admission) => {
    await admission;
    return { outcome: "failed", reason: "timeout", operation_id: "deletion" } as never;
  });
  const screen = await render(ProfilesPage);
  await screen.getByRole("button", { name: "Edit Work" }).click();
  await screen.getByRole("button", { name: "Delete Profile…" }).click();
  await screen.getByRole("button", { name: "Delete Profile", exact: true }).click();
  await expect.element(screen.getByText(/couldn't confirm that deletion finished/)).toBeVisible();
  await expect.element(screen.getByRole("button", { name: "Close", exact: true })).toBeVisible();
  expect(native.remove).toHaveBeenCalledOnce();
  expect(
    screen.getByRole("button", { name: "Delete Profile", exact: true }).elements(),
  ).toHaveLength(0);
});

test("New Profile from the sidebar creates and switches; from the list it only creates", async () => {
  await page.viewport(1200, 800);
  requestNew();
  let screen = await render(ProfilesPage);
  await screen.getByLabelText("Name").fill("School");
  await screen.getByRole("button", { name: "Create and Switch" }).click();
  await expect.poll(() => native.create.mock.calls.length).toBe(1);
  expect(native.create).toHaveBeenLastCalledWith(
    "School",
    expect.objectContaining({ style: "initials", photo: null }),
    true,
  );
  await screen.unmount();

  screen = await render(ProfilesPage);
  await screen.getByRole("button", { name: "New Profile…" }).click();
  await screen.getByLabelText("Name").fill("Work");
  await screen.getByRole("button", { name: "Create Profile" }).click();
  await expect.poll(() => native.create.mock.calls.length).toBe(2);
  expect(native.create).toHaveBeenLastCalledWith(
    "Work",
    expect.objectContaining({ style: "initials", photo: null }),
    false,
  );
});
