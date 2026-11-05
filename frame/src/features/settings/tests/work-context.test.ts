import { expect, test } from "vitest";
import { parseSkill, renderSkill, sinceLabel, skillName, skillTitle } from "../lib/work-context";

test("a skill's text splits into fields and renders back the way Rust reads it", () => {
  const text =
    '﻿---\nname: weekly-review\ndescription: "Review my week."\ntools: [search_notes, list_tabs]\nrole: light\nunknown: kept out\n---\n\n# Weekly review\n- Read my notes.\n';
  const fields = parseSkill(text);
  expect(fields).toEqual({
    name: "weekly-review",
    description: "Review my week.",
    tools: "search_notes, list_tabs",
    role: "light",
    body: "# Weekly review\n- Read my notes.",
  });
  expect(renderSkill(fields)).toBe(
    "---\nname: weekly-review\ndescription: Review my week.\ntools: [search_notes, list_tabs]\nrole: light\n---\n\n# Weekly review\n- Read my notes.\n",
  );
  expect(parseSkill("no frontmatter").body).toBe("no frontmatter");
});

test("names read as titles and titles become names", () => {
  expect(skillTitle("trip-planning")).toBe("Trip planning");
  expect(skillName("  Friday review: Zażółć!  ")).toBe("friday-review-zazolc");
  expect(skillName("x".repeat(60))).toHaveLength(40);
});

test("times read the way people say them", () => {
  const now = Date.UTC(2026, 8, 28, 12);
  expect(sinceLabel(String(now - 3_600_000), now)).toBe("today");
  expect(sinceLabel(String(now - 86_400_000), now)).toBe("yesterday");
  expect(sinceLabel(String(now - 3 * 86_400_000), now)).toBe("3 days ago");
  // Older timestamps are displayed in the user's local time zone.
  const august19 = new Date(2026, 7, 19, 12).getTime();
  expect(sinceLabel(String(august19), now)).toBe("Aug 19");
  expect(sinceLabel(null, now)).toBe("");
});
