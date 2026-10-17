import { defineConfig } from "@playwright/test";

// O global-setup sobe o scriba.exe real (--no-open, porta 0, diretório temporário) e
// publica a URL de entrada em SCRIBA_URL. Os testes rodam em série: compartilham a
// instância e o tema salvo nela.
export default defineConfig({
  testDir: "tests",
  globalSetup: "./global-setup.ts",
  workers: 1,
  fullyParallel: false,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [["github"], ["junit", { outputFile: "junit.xml" }]] : "list",
  use: {
    // O Edge já vem no Windows (e no runner): nada para baixar.
    channel: "msedge",
    viewport: { width: 1280, height: 900 },
  },
  // No CI, uma referência ausente é gravada (e publicada como artefato para commit) em vez
  // de falhar; o workflow e2e-snapshots regenera todas.
  updateSnapshots: process.env.CI ? "missing" : "none",
  expect: {
    toHaveScreenshot: { maxDiffPixelRatio: 0.01 },
  },
});
