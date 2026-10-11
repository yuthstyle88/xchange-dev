// S-3 #575: no role name anywhere in web/src/screens/staff/** (SD:S2, SD:§8.1 row S-3, #575 §4 criterion 3).
// Access is a Permission read from staff_me.permissions, never a role literal. Written before the implementation
// by the ticket author (claude/709ec749). Do not edit, delete or weaken.
//
// The role names are built from pieces here, so this file itself matches neither this test nor the §4 grep.
import { describe, expect, it } from "vitest";

const ROLES = [
  ["TEL", "LER"],
  ["BRANCH", "_MANAGER"],
  ["RATE", "_MANAGER"],
  ["FIN", "ANCE"],
  ["FRO", "NT"],
  ["BACK", "END"],
  ["EXEC", "UTIVE"],
  ["AD", "MIN"],
  ["MAR", "KETING"],
  ["VERIFICATION", "_REVIEWER"],
  ["MON", "ITOR"],
  ["MAN", "AGER"],
  ["OW", "NER"],
].map((p) => p.join(""));

const PATTERN = new RegExp(`\\b(${ROLES.join("|")})\\b`);

const FILES = import.meta.glob<string>(["./**/*.ts", "./**/*.tsx", "./**/*.css"], {
  query: "?raw",
  import: "default",
  eager: true,
});

describe("no role name in web/src/screens/staff/**", () => {
  it("G1 the scan reaches the folder's files, including its nested parts/ (Vite leaves out the importing file)", () => {
    expect(Object.keys(FILES)).toContain("./StaffPage.test.tsx");
    expect(Object.keys(FILES)).toContain("./parts/can.test.ts");
  });

  it("G2 no file names a role", () => {
    const hits: string[] = [];
    for (const [file, src] of Object.entries(FILES)) {
      src.split("\n").forEach((line, i) => {
        if (PATTERN.test(line)) hits.push(`${file}:${i + 1}: ${line.trim()}`);
      });
    }
    expect(hits).toEqual([]);
  });

  it("G3 the shell's own files exist (an empty folder would pass G2 vacuously)", () => {
    for (const f of ["./main.tsx", "./StaffPage.tsx", "./cards.ts", "./i18n.ts", "./parts/can.ts", "./parts/command.ts"]) {
      expect(Object.keys(FILES), `${f} exists`).toContain(f);
    }
  });
});
