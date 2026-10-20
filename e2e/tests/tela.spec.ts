import type { Page } from "@playwright/test";
import { expect, test } from "../fixtures";

test("a página inicial carrega", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await expect(page).toHaveURL(`${scriba.origin}/`);
  await expect(page.getByRole("heading", { name: "Solte um vídeo ou áudio aqui" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Recentes" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Tema: Sistema" })).toBeVisible();
  // Sem as caixinhas: todo arquivo extrai o áudio e transcreve.
  await expect(page.getByRole("checkbox")).toHaveCount(0);
  await expect(page.locator("footer")).toContainText("· Transcrição no processador");
});

test("sem o token a página é recusada", async ({ request, scriba }) => {
  const res = await request.get(`${scriba.origin}/`);
  expect(res.status()).toBe(401);
});

// A referência visual é gerada no runner do CI (workflow e2e-snapshots): a renderização
// local difere em fontes e DPI, então localmente a comparação é pulada.
for (const scheme of ["light", "dark"] as const) {
  test(`bate com a referência visual (${scheme})`, async ({ page, scriba }) => {
    test.skip(!process.env.CI, "referência visual só no CI");
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto(scriba.url);
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot(`inicio-${scheme}.png`, { fullPage: true });
  });
}

/** Clica e espera o htmx assentar a troca: o botão novo só responde depois de processado. */
async function clickAndSettle(page: Page, name: string) {
  const settled = page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        document.body.addEventListener("htmx:afterSettle", () => resolve(), { once: true }),
      ),
  );
  await page.getByRole("button", { name }).click();
  await settled;
}

test("a alternância de tema persiste após recarregar", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  const html = page.locator("html");
  await expect(html).not.toHaveAttribute("data-theme");

  await clickAndSettle(page, "Tema: Sistema");
  await expect(page.getByRole("button", { name: "Tema: Claro" })).toBeVisible();
  await expect(html).toHaveAttribute("data-theme", "light");

  await clickAndSettle(page, "Tema: Claro");
  await expect(html).toHaveAttribute("data-theme", "dark");

  await page.reload();
  await expect(html).toHaveAttribute("data-theme", "dark");
  await expect(page.getByRole("button", { name: "Tema: Escuro" })).toBeVisible();
});
