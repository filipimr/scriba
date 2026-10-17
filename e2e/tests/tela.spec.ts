import { expect, test } from "@playwright/test";

const url = () => process.env.SCRIBA_URL!;
const origin = () => new URL(url()).origin;

test("a página inicial carrega", async ({ page }) => {
  await page.goto(url());
  await expect(page).toHaveURL(`${origin()}/`);
  await expect(page.getByRole("heading", { name: "Solte um vídeo ou áudio aqui" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Recentes" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Tema: Sistema" })).toBeVisible();
});

test("sem o token a página é recusada", async ({ request }) => {
  const res = await request.get(`${origin()}/`);
  expect(res.status()).toBe(401);
});

// A referência visual é gerada no runner do CI (workflow e2e-snapshots): a renderização
// local difere em fontes e DPI, então localmente a comparação é pulada.
for (const scheme of ["light", "dark"] as const) {
  test(`bate com a referência visual (${scheme})`, async ({ page }) => {
    test.skip(!process.env.CI, "referência visual só no CI");
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto(url());
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot(`inicio-${scheme}.png`, { fullPage: true });
  });
}

test("a alternância de tema persiste após recarregar", async ({ page }) => {
  await page.goto(url());
  const html = page.locator("html");
  await expect(html).not.toHaveAttribute("data-theme");

  await page.getByRole("button", { name: "Tema: Sistema" }).click();
  await expect(page.getByRole("button", { name: "Tema: Claro" })).toBeVisible();
  await expect(html).toHaveAttribute("data-theme", "light");

  await page.getByRole("button", { name: "Tema: Claro" }).click();
  await expect(html).toHaveAttribute("data-theme", "dark");

  await page.reload();
  await expect(html).toHaveAttribute("data-theme", "dark");
  await expect(page.getByRole("button", { name: "Tema: Escuro" })).toBeVisible();

  // Volta ao modo Sistema para não afetar os outros testes.
  await page.getByRole("button", { name: "Tema: Escuro" }).click();
  await expect(html).not.toHaveAttribute("data-theme");
});
