import type { Page } from "@playwright/test";
import { expect, test } from "../fixtures";

// A gravação ao vivo com o áudio falso (`--fake-audio`: tons pulsando no lugar do microfone e
// do som do PC) e o pipeline falso, que grava uma transcrição rotulada fixa.

const panel = (page: Page) => page.locator("#gravar");
const recordButton = (page: Page) => panel(page).getByRole("button", { name: "Gravar", exact: true });
const stopButton = (page: Page) => panel(page).getByRole("button", { name: "Parar", exact: true });
const bar = (page: Page) => page.locator(".recording-bar");
const recordingCard = (page: Page) => page.locator("article.job-card", { has: page.getByRole("heading", { name: /^Gravação de / }) });

/** O tempo mostrado no painel, em segundos. */
async function seconds(page: Page): Promise<number> {
  const text = (await panel(page).locator("[data-tempo]").textContent()) ?? "";
  const [h, m, s] = text.split(":").map(Number);
  return h * 3600 + m * 60 + s;
}

test("gravar, ver o tempo e os medidores, parar e ler a transcrição rotulada", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await recordButton(page).click();
  await expect(stopButton(page)).toBeVisible();
  await expect(page.locator("#avisos")).toHaveText("Gravação iniciada.");
  await expect(bar(page)).toBeVisible();
  await expect(bar(page)).toContainText("Gravando");

  // O tempo avança e os medidores mexem (o SSE atualiza ~10 vezes por segundo).
  await expect.poll(() => seconds(page), { timeout: 5_000 }).toBeGreaterThanOrEqual(2);
  const meter = panel(page).locator('meter[data-nivel="0"]');
  const values = new Set<number>();
  await expect
    .poll(
      async () => {
        values.add(await meter.evaluate((m: HTMLMeterElement) => m.value));
        return values.size;
      },
      { intervals: [100] },
    )
    .toBeGreaterThan(3);
  expect(Math.max(...values)).toBeGreaterThan(0.3);
  // O tempo fica fora da região viva: só o início e o fim são anunciados.
  await expect(page.locator("#avisos [data-tempo]")).toHaveCount(0);

  await stopButton(page).click();
  await expect(recordButton(page)).toBeVisible();
  await expect(bar(page)).toHaveCount(0);
  await expect(page.locator("#avisos")).toHaveText("Gravação parada. Transcrevendo.");
  const job = recordingCard(page);
  await expect(job).toHaveCount(1);
  await expect(job.locator(".lucide-mic")).toBeVisible();

  await expect(job.getByText("Guardar o áudio?")).toBeVisible({ timeout: 20_000 });
  await job.getByRole("link", { name: "Ver transcrição" }).click();
  const speakers = page.locator(".transcript-speaker");
  await expect(speakers.first()).toHaveText("Outros:");
  await expect(speakers.nth(1)).toHaveText("Você:");
  await expect(page.getByText("Bom dia, pessoal.")).toBeVisible();

  await page.goBack();
  await expect(recordingCard(page).getByText("Guardar o áudio?")).toBeVisible();
});

test("Ctrl+Shift+R grava e para sem recarregar a página, também no leitor", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  // Recarregar apagaria esta marca.
  await page.evaluate(() => ((window as unknown as { marca: number }).marca = 1));
  await page.keyboard.press("Control+Shift+R");
  await expect(stopButton(page)).toBeVisible();
  await expect(bar(page)).toBeVisible();
  await page.keyboard.press("Control+Shift+R");
  await expect(recordButton(page)).toBeVisible();
  await expect(bar(page)).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { marca?: number }).marca)).toBe(1);
  await expect(recordingCard(page)).toHaveCount(1);

  // No leitor, sem o painel: a barra aparece e o Parar dela para.
  await expect(recordingCard(page).getByText("Guardar o áudio?")).toBeVisible({ timeout: 20_000 });
  await recordingCard(page).getByRole("link", { name: "Ver transcrição" }).click();
  await page.evaluate(() => ((window as unknown as { marca: number }).marca = 2));
  await page.keyboard.press("Control+Shift+R");
  await expect(bar(page)).toBeVisible();
  await expect(bar(page).locator("[data-tempo]")).not.toHaveText("00:00:00", { timeout: 5_000 });
  await page.keyboard.press("Control+Shift+R");
  await expect(bar(page)).toHaveCount(0);
  await expect(page.locator("#avisos")).toHaveText("Gravação parada. Transcrevendo.");
  expect(await page.evaluate(() => (window as unknown as { marca?: number }).marca)).toBe(2);
});

test("recarregar no meio mantém a gravação e o tempo", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await recordButton(page).click();
  await expect.poll(() => seconds(page), { timeout: 5_000 }).toBeGreaterThanOrEqual(2);
  await page.reload();
  await expect(stopButton(page)).toBeVisible();
  await expect(bar(page)).toBeVisible();
  // O servidor já renderiza o tempo certo, e ele continua avançando.
  expect(await seconds(page)).toBeGreaterThanOrEqual(2);
  await expect.poll(() => seconds(page), { timeout: 5_000 }).toBeGreaterThanOrEqual(4);
  await stopButton(page).click();
  await expect(recordingCard(page)).toHaveCount(1);
});

test("o microfone desconectado para a gravação sozinho, com o toast", async ({ page, scriba }) => {
  scriba.audioScript("perder-mic");
  await page.goto(scriba.url);
  await recordButton(page).click();
  await expect(stopButton(page)).toBeVisible();
  await expect(page.locator(".toast")).toHaveText("O microfone foi desconectado. A gravação parou e vai ser transcrita.", {
    timeout: 10_000,
  });
  await expect(recordButton(page)).toBeVisible();
  await expect(bar(page)).toHaveCount(0);
  await expect(recordingCard(page)).toHaveCount(1);
  await expect(recordingCard(page).getByText("Guardar o áudio?")).toBeVisible({ timeout: 20_000 });
});

test("sem microfone, o erro aparece no painel", async ({ page, scriba }) => {
  scriba.audioScript("sem-microfone");
  await page.goto(scriba.url);
  await recordButton(page).click();
  await expect(panel(page).getByRole("alert")).toHaveText("Não encontrei um microfone. Conecte um e tente de novo.");
  await expect(recordButton(page)).toBeVisible();
  await expect(bar(page)).toHaveCount(0);
  // De novo, com o microfone de volta.
  scriba.audioScript("");
  await recordButton(page).click();
  await expect(stopButton(page)).toBeVisible();
  await expect(panel(page).getByRole("alert")).toHaveCount(0);
  await stopButton(page).click();
});

test("o exe morto no meio da gravação: a gravação volta ao abrir de novo e conclui", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await recordButton(page).click();
  // Passa de um flush (5 s): o WAV tem o que foi gravado até ali.
  await expect.poll(() => seconds(page), { timeout: 15_000 }).toBeGreaterThanOrEqual(6);
  await scriba.restart();
  await page.goto(scriba.url);
  await expect(recordButton(page)).toBeVisible();
  const job = recordingCard(page);
  await expect(job).toHaveCount(1);
  await expect(job.getByText("Guardar o áudio?")).toBeVisible({ timeout: 20_000 });
});

test("cancelar a transcrição de uma gravação pede confirmação", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  // Um arquivo lento na frente segura a gravação na fila.
  await page.locator("#arquivo").setInputFiles({ name: "lento.mkv", mimeType: "application/octet-stream", buffer: Buffer.alloc(2048, 7) });
  await recordButton(page).click();
  await expect.poll(() => seconds(page), { timeout: 5_000 }).toBeGreaterThanOrEqual(1);
  await stopButton(page).click();
  const job = recordingCard(page);
  const cancel = job.getByRole("button", { name: /^Cancelar Gravação de / });

  await cancel.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading")).toHaveText("Descartar a gravação?");
  await expect(dialog).toContainText("As trilhas gravadas serão apagadas.");
  await expect(dialog.getByRole("button", { name: "Cancelar" })).toBeFocused();
  await dialog.getByRole("button", { name: "Cancelar" }).click();
  await expect(dialog).toBeHidden();
  await expect(job.getByText("Na fila")).toBeVisible();

  await cancel.click();
  await dialog.getByRole("button", { name: "Descartar" }).click();
  await expect(job.getByText("Cancelado")).toBeVisible();

  // O Remover de um job concluído continua com o texto de antes.
  await page.getByRole("button", { name: "Cancelar lento.mkv" }).click();
  await expect(page.getByRole("button", { name: "Remover lento.mkv" })).toBeVisible();
});

test("os Ajustes salvam o microfone e avisam do Bluetooth", async ({ page, scriba }) => {
  await page.goto(scriba.url);
  await panel(page).getByText("Ajustes").click();
  const mic = page.getByLabel("Microfone", { exact: true });
  await expect(mic).toBeVisible();
  await expect(mic.locator("option").first()).toHaveText("Padrão do Windows (Microfone simulado)");
  await expect(page.getByText("Microfone Bluetooth: a qualidade do som cai durante a gravação.")).toHaveCount(0);

  await mic.selectOption({ label: "Fone Bluetooth simulado" });
  await expect(page.getByText("Microfone Bluetooth: a qualidade do som cai durante a gravação.")).toBeVisible();

  // A escolha fica depois de recarregar e vale na gravação.
  await page.reload();
  await panel(page).getByText("Ajustes").click();
  await expect(page.getByLabel("Microfone", { exact: true })).toHaveValue("falso:fone-bt");
  await expect(page.getByText("Microfone Bluetooth: a qualidade do som cai durante a gravação.")).toBeVisible();
});

// As referências visuais são geradas no runner do CI (workflow e2e-snapshots).
for (const scheme of ["light", "dark"] as const) {
  test(`gravando bate com a referência visual (${scheme})`, async ({ page, scriba }) => {
    test.skip(!process.env.CI, "referência visual só no CI");
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto(scriba.url);
    await recordButton(page).click();
    await expect(stopButton(page)).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    // O tempo e os medidores mudam a cada instante.
    await expect(page).toHaveScreenshot(`gravando-${scheme}.png`, {
      mask: [page.locator("[data-tempo]"), page.locator(".level-meter meter")],
    });
    await stopButton(page).click();
  });

  test(`Ajustes batem com a referência visual (${scheme})`, async ({ page, scriba }) => {
    test.skip(!process.env.CI, "referência visual só no CI");
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto(scriba.url);
    await panel(page).getByText("Ajustes").click();
    await page.getByLabel("Microfone", { exact: true }).selectOption({ label: "Fone Bluetooth simulado" });
    await expect(page.getByText("Microfone Bluetooth: a qualidade do som cai durante a gravação.")).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await expect(panel(page)).toHaveScreenshot(`ajustes-${scheme}.png`);
  });

  test(`card e leitor da gravação batem com a referência visual (${scheme})`, async ({ page, scriba }) => {
    test.skip(!process.env.CI, "referência visual só no CI");
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto(scriba.url);
    await recordButton(page).click();
    await expect.poll(() => seconds(page), { timeout: 5_000 }).toBeGreaterThanOrEqual(1);
    await stopButton(page).click();
    const job = recordingCard(page);
    await expect(job.getByText("Guardar o áudio?")).toBeVisible({ timeout: 20_000 });
    await page.evaluate(() => document.fonts.ready);
    // O nome e a data da gravação mudam a cada execução.
    await expect(job).toHaveScreenshot(`card-gravacao-${scheme}.png`, {
      mask: [job.locator(".job-card-name"), job.locator(".job-card-meta")],
    });
    await job.getByRole("link", { name: "Ver transcrição" }).click();
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot(`leitor-gravacao-${scheme}.png`, {
      mask: [page.locator(".transcript-title"), page.locator(".transcript-meta")],
    });
  });
}
