// Aplica no <html> o tema que o servidor acabou de salvar (evento HX-Trigger "scriba-theme").
document.body.addEventListener("scriba-theme", (e) => {
  const mode = e.detail.mode;
  if (mode === "system") {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = mode;
  }
});

// Soltar um arquivo fora da dropzone não pode abrir o arquivo na aba e fechar o app.
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", (e) => e.preventDefault());

// Remover um job concluído (hx-confirm) pede confirmação no <dialog> do design, não no
// window.confirm. O showModal prende o foco, o Esc cancela e o foco volta ao botão.
const dialog = document.getElementById("confirmar");
if (dialog) {
  document.body.addEventListener("htmx:confirm", (e) => {
    if (!e.detail.question) return;
    e.preventDefault();
    dialog.querySelector("#confirmar-texto").textContent = e.detail.question;
    dialog.returnValue = "";
    dialog.addEventListener(
      "close",
      () => {
        if (dialog.returnValue === "excluir") e.detail.issueRequest(true);
      },
      { once: true },
    );
    dialog.showModal();
  });
}

// Envio de arquivos: soltar na dropzone ou escolher no botão. O htmx faz o POST multipart; aqui
// só validamos antes de enviar (formatos do `accept`, mensagens vindas do servidor) e
// mostramos o progresso do upload.
const form = document.getElementById("envio");
if (form) {
  const input = form.querySelector("#arquivo");
  const errorBox = form.querySelector("#erro-envio");
  const fileName = form.querySelector(".dropzone-file");
  const bar = form.querySelector(".dropzone-uploading progress");
  const formats = input.accept.split(",");

  const showError = (message) => {
    const alert = document.getElementById("modelo-erro").content.firstElementChild.cloneNode(true);
    alert.querySelector("span").textContent = message;
    errorBox.replaceChildren(alert);
  };

  const send = (files) => {
    if (form.classList.contains("is-uploading") || files.length === 0) return;
    const supported = [...files].every((f) => formats.some((ext) => f.name.toLowerCase().endsWith(ext)));
    if (!supported) {
      input.value = "";
      return showError(form.dataset.erroFormato);
    }
    if (files !== input.files) input.files = files;
    errorBox.replaceChildren();
    fileName.textContent = files.length === 1 ? files[0].name : `${files.length} arquivos`;
    bar.value = 0;
    form.classList.add("is-uploading");
    form.requestSubmit();
  };

  input.addEventListener("change", () => send(input.files));

  form.addEventListener("dragover", (e) => {
    e.preventDefault();
    form.classList.add("dropzone-dragover");
  });
  form.addEventListener("dragleave", (e) => {
    if (!form.contains(e.relatedTarget)) form.classList.remove("dropzone-dragover");
  });
  form.addEventListener("drop", (e) => {
    e.preventDefault();
    form.classList.remove("dropzone-dragover");
    send(e.dataTransfer.files);
  });

  form.addEventListener("htmx:xhr:progress", (e) => {
    if (e.detail.lengthComputable && e.detail.total > 0) {
      bar.value = Math.round((e.detail.loaded / e.detail.total) * 100);
    }
  });
  form.addEventListener("htmx:afterRequest", () => {
    form.classList.remove("is-uploading");
    input.value = "";
  });
}
