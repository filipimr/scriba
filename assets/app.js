// Aplica no <html> o tema que o servidor acabou de salvar (evento HX-Trigger "scriba-theme").
document.body.addEventListener("scriba-theme", (e) => {
  const mode = e.detail.mode;
  if (mode === "system") {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = mode;
  }
});
