import { defineConfig } from "@playwright/test";

// A fixture `scriba` (fixtures.ts) sobe um scriba.exe real por teste (--no-open, porta 0,
// diretório temporário). Os testes rodam em série para a medida de tempo do pipeline falso
// não variar com a carga da máquina.
export default defineConfig({
  testDir: "tests",
  workers: 1,
  fullyParallel: false,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [["github"], ["junit", { outputFile: "junit.xml" }]] : "list",
  use: {
    // O Edge já vem no Windows (e no runner): nada para baixar.
    channel: "msedge",
    viewport: { width: 1280, height: 900 },
  },
  expect: {
    toHaveScreenshot: { maxDiffPixelRatio: 0.01 },
  },
});
