import "$styles/global.css";
import { expect, test } from "vitest";
import { render } from "vitest-browser-svelte";
import CliRow from "../components/connections/CliRow.svelte";

test("a closed command broker is shown as unavailable", async () => {
  const screen = await render(CliRow, {
    id: "gh",
    cli: { id: "gh", status: "unavailable", version: null, account: null },
  });
  await expect.element(screen.getByText("Unavailable in this build")).toBeVisible();
  await expect.element(screen.getByText("Couldn’t check its sign-in")).not.toBeInTheDocument();
  await expect.element(screen.getByRole("button")).not.toBeInTheDocument();
});

test("an actual failed status check keeps its own message", async () => {
  const screen = await render(CliRow, {
    id: "gh",
    cli: { id: "gh", status: "unknown", version: null, account: null },
  });
  await expect.element(screen.getByText("Couldn’t check its sign-in")).toBeVisible();
});
