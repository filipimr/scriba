import { defineConfig } from "@playwright/test";

// A partir da Fase 2, o webServer sobe o scriba.exe real com --no-open, porta 0 e
// diretórios temporários.
export default defineConfig({
  testDir: "tests",
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [["github"], ["junit", { outputFile: "junit.xml" }]] : "list",
});
