import { afterEach, expect, test } from "vitest";
import { render } from "vitest-browser-svelte";
import "$styles/global.css";
import { browserImport, type ImportAdapter, type ImportJob } from "$domain/browser-import";
import Import from "../components/steps/Import.svelte";

afterEach(() => browserImport.provide(null));

function importer() {
  let emit: (job: ImportJob) => void = () => {};
  const adapter: ImportAdapter = {
    sources: async () => [
      {
        id: "chrome",
        browser: "chrome",
        name: "Chrome",
        profiles: [{ id: "Default", name: "Person 1" }],
        kinds: ["bookmarks", "history"],
        needsPermission: false,
        running: false,
      },
    ],
    start: async () => true,
    cancel: async () => {},
    onProgress: (listener) => {
      emit = listener;
      return () => {};
    },
    openPermissionSettings: async () => {},
  };
  return { adapter, emit: (job: ImportJob) => emit(job) };
}

const finished = (kinds: ImportJob["kinds"]): ImportJob => ({
  source: "chrome",
  profile: "Default",
  finished: true,
  cancelled: false,
  kinds,
});

test("an import that found nothing says so instead of counting zero items brought over", async () => {
  const fake = importer();
  browserImport.provide(fake.adapter);
  await browserImport.detect();
  const screen = await render(Import);
  await screen.getByRole("radio", { name: "Chrome" }).click();
  await screen.getByRole("button", { name: "Import", exact: true }).click();

  fake.emit(
    finished([
      { kind: "bookmarks", state: "done", done: 0, total: 0 },
      { kind: "history", state: "done", done: 0, total: 0 },
    ]),
  );
  await expect.element(screen.getByText("Nothing to import")).toBeVisible();
  await expect.element(screen.getByText(/items brought over/u)).not.toBeInTheDocument();

  fake.emit(
    finished([
      { kind: "bookmarks", state: "done", done: 12, total: 12 },
      { kind: "history", state: "done", done: 0, total: 0 },
    ]),
  );
  await expect.element(screen.getByText("12 items brought over")).toBeVisible();
});
