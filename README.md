<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/logo-white.svg">
    <img src="docs/brand/logo.svg" width="96" height="96" alt="Logo do Scriba">
  </picture>
</p>

<h1 align="center">Scriba</h1>

<p align="center">Grava e transcreve — offline, num único <code>.exe</code> para Windows.</p>

<p align="center">
  <a href="https://github.com/filipimr/scriba/actions/workflows/ci.yml"><img src="https://github.com/filipimr/scriba/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/filipimr/scriba/releases/latest"><img src="https://img.shields.io/github/v/release/filipimr/scriba" alt="Última release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/filipimr/scriba" alt="Licença GPL-3.0"></a>
</p>

## O que o Scriba faz

- **Transcreve** vídeos e áudios em português do Brasil (MP4, MOV, MKV, MP3, M4A, WAV e OGG), inclusive reuniões de várias horas. Os modelos de IA (Whisper large-v3-turbo e `small`) vêm dentro do executável: você nunca escolhe nem baixa um modelo.
- **Grava** o microfone e o som do PC em trilhas separadas. A transcrição sai rotulada "Você" e "Outros", sem o eco dos alto-falantes.
- **Entrega o texto** num `transcricao.txt` e num `transcricao.md`, com marcações de tempo. O leitor do app tem Copiar texto, Baixar .txt e .md e Abrir pasta.
- **Descarta a mídia** depois do texto. Se você quiser, guarda o áudio num `.m4a` (AAC mono, ~30 MB por hora) na pasta que escolher.

## Baixar

1. Baixe o `scriba.exe` da [última release](https://github.com/filipimr/scriba/releases/latest). São cerca de 800 MB, porque os modelos vêm dentro dele.
2. Confira o checksum com o `scriba.exe.sha256` publicado junto, no PowerShell:

   ```powershell
   (Get-FileHash .\scriba.exe -Algorithm SHA256).Hash.ToLower()
   ```

   O resultado tem que ser igual ao texto do `.sha256`.

Requisitos: Windows 10 ou 11, 64 bits. Nada para instalar.

## Primeira execução

Dê dois cliques no `scriba.exe`. O executável não é assinado, então o Windows SmartScreen avisa "O Windows protegeu o computador". Clique em **Mais informações** e depois em **Executar assim mesmo**. O aviso só aparece na primeira vez.

O Scriba abre a interface no navegador padrão, em `http://127.0.0.1:47800`, e fica na bandeja do Windows, perto do relógio. O ícone pode estar escondido na seta **^** da barra de tarefas.

- Clique no ícone para abrir o app de novo. Executar o `scriba.exe` outra vez também abre o mesmo app.
- No clique direito, **Sair** fecha o Scriba. Se uma gravação, uma transcrição ou um "Guardar o áudio?" seria perdido, ele pergunta antes.

## Como usar

- **Enviar:** solte um vídeo ou áudio na área "Solte um vídeo ou áudio aqui", ou clique em **Escolher arquivo**. O card mostra a estimativa e o andamento.
- **Gravar:** clique em **Gravar**, ou use **Ctrl+Shift+R** em qualquer tela. Os medidores mostram o microfone e o som do PC. Ao **Parar**, a gravação é transcrita. Em **Ajustes**, você escolhe o microfone e a saída de som.
- **Guardar o áudio?** No fim de cada transcrição, o card pergunta. **Descartar** apaga a mídia na hora. **Guardar áudio** abre a janela "Salvar como" do Windows. Sem resposta, a mídia é descartada depois de 24 h, ou quando o Scriba fecha.
- **Recentes:** a lista guarda as 50 transcrições mais recentes, também depois de fechar e abrir o app. As mais antigas continuam na pasta das saídas.

Uma gravação interrompida (o app fechado no meio, uma queda de energia) volta ao abrir o Scriba e é transcrita.

## Onde ficam os arquivos

| O quê | Onde |
| --- | --- |
| Transcrições (`transcricao.txt`, `transcricao.md`) | `Documentos\Scriba`, uma pasta por transcrição (`AAAA-MM-DD HHmm - nome`) |
| Dados internos (ajustes, gravações em andamento) | `%LOCALAPPDATA%\Scriba` |
| Logs (até 25 MB) | `%LOCALAPPDATA%\Scriba\logs\scriba.log` |

## GPU ou processador

Com uma GPU dedicada com driver Vulkan (NVIDIA, AMD), o Scriba transcreve nela com o modelo grande: 1 h de áudio leva uns 2 min numa RTX 3050. Sem GPU, ele transcreve no processador com o modelo `small`: 1 h leva uns 8 min. O rodapé do app mostra qual dos dois está em uso.

## Privacidade

Tudo roda no seu computador e funciona sem internet. Nem o áudio, nem o texto, nem os logs saem da sua máquina. A interface abre no navegador, mas é servida pelo próprio executável em `127.0.0.1`, com um token de sessão e checagem de Host e Origin, para que nenhum site aberto no navegador consiga acionar o app.

## Opções de linha de comando

| Opção | O que faz |
| --- | --- |
| `--port <n>` | Porta local preferida (padrão 47800). Se estiver ocupada, usa outra livre; `0` deixa o sistema escolher |
| `--no-open` | Não abre o navegador ao iniciar |
| `--no-tray` | Não põe o ícone na bandeja |
| `--data-dir <pasta>` | Dados internos do app (padrão `%LOCALAPPDATA%\Scriba`) |
| `--output-dir <pasta>` | Onde ficam as transcrições (padrão `Documentos\Scriba`) |
| `--self-test` | Transcreve uma amostra embutida e diz onde rodou (GPU ou processador), para conferir o executável |

O `scriba.exe` não abre janela de console. Para ver a saída do `--self-test` e o código de saída, rode no PowerShell:

```powershell
.\scriba.exe --self-test | Out-Host
$LASTEXITCODE
```

## Problemas comuns

- **"O Windows bloqueou o microfone":** em Configurações do Windows → Privacidade e segurança → Microfone, ligue "Acesso ao microfone" e "Permitir que aplicativos da área de trabalho acessem o microfone".
- **"Não encontrei um microfone":** conecte um microfone (ou confira se ele está ligado nas Configurações de som) e grave de novo.
- **Microfone Bluetooth:** enquanto grava, o fone Bluetooth troca para o modo de chamada e o som fica pior. Prefira o microfone do computador ou um com fio.
- **Algo deu errado:** o `scriba.log`, em `%LOCALAPPDATA%\Scriba\logs`, diz o que aconteceu. Anexe-o ao abrir uma issue.

## Como remover

Feche o Scriba (clique direito no ícone da bandeja → Sair) e apague o `scriba.exe` e a pasta `%LOCALAPPDATA%\Scriba`. As transcrições em `Documentos\Scriba` são suas: apague-as só se quiser.

## Desenvolvimento

O build é nativo e estático (FFmpeg, whisper.cpp e o CRT dentro do exe), por isso o ambiente tem alguns passos únicos:

1. Em um PowerShell de administrador, rode `powershell -ExecutionPolicy Bypass -File scripts\setup.ps1`. Ele instala Rust, VS Build Tools, LLVM, CMake, vcpkg e Vulkan SDK, e define `CARGO_TARGET_DIR=C:\dev\t\scriba` (o build do whisper.cpp estoura o limite de 260 caracteres de caminho dentro do repositório). Com `-Sandbox`, ele só habilita o Windows Sandbox (depois, reinicie o Windows).
2. Compile o FFmpeg estático uma vez (cerca de 12–20 min; depois fica em cache):

   ```powershell
   C:\dev\vcpkg\vcpkg.exe install --triplet x64-windows-static-release --x-install-root=vcpkg_installed
   ```

3. Baixe os modelos com `cargo xtask model` (ou só os pequenos, usados nos testes, com `--only base,silero`).

Comandos do dia a dia:

| Tarefa | Comando |
| --- | --- |
| Rodar o app (com a GPU) | `cargo run --features vulkan` |
| Testes | `cargo nextest run --workspace --features "spikes/vulkan scriba/test-support"` |
| Lint | `cargo fmt --all --check` e `cargo clippy --workspace --all-targets --features "spikes/vulkan scriba/test-support" -- -D warnings` |
| E2E (Playwright no Edge) | `cargo build --features test-support`, depois `npm ci` e `npx playwright test` em `e2e/` |
| CSS a partir do design system | `cargo xtask css` |
| Exe de release (o mesmo do CI) | `cargo xtask dist` → `dist\scriba.exe` |
| Exe numa máquina limpa | `cargo xtask sandbox-test` (Windows Sandbox, sem rede e sem GPU) |

As features do app: `vulkan` (transcrição na GPU), `embedded-model` (o large, o `small` e o VAD dentro do exe; `embedded-cpu-model` só o pacote da CPU) e `test-support` (o pipeline e o áudio falsos, para os testes). Testes que dependem de GPU ou de dispositivos de áudio reais ficam fora do CI e rodam localmente com `-- --ignored`. Todo PR passa pelo CI (formatação, clippy, testes com cobertura mínima, E2E e `cargo deny`) antes do merge, e a release passa pelo smoke em `scripts/smoke.ps1`.

Documentos do projeto:

- [`ROADMAP.md`](ROADMAP.md): decisões de produto e arquitetura, fases com checklists, política de testes e registro de decisões.
- [`DESIGN-scriba.md`](DESIGN-scriba.md): o design system da interface, com os tokens que viram CSS.
- [`CLAUDE.md`](CLAUDE.md): notas de trabalho e comandos para o agente que ajuda no desenvolvimento.

## Arquitetura

Um monólito em Rust, distribuído como um único executável:

- **Interface:** servidor Axum local com htmx, Tailwind compilado e a fonte Inter, tudo embutido no binário.
- **Transcrição:** whisper-rs (whisper.cpp) com aceleração Vulkan e os modelos carregados da memória. O `vulkan-1.dll` é carregado só quando usado, então o exe abre também sem driver de GPU.
- **Mídia:** ffmpeg-next sobre um FFmpeg estático com x264.
- **Captura:** cpal (WASAPI), com microfone e som do sistema gravados em trilhas separadas.
- **Bandeja:** Win32 direto (crate `windows`), sem janela de console.

## Licença

GPL-3.0-or-later (veja [`LICENSE`](LICENSE)); o executável inclui o x264, que é GPL. Cada release publica o `THIRD_PARTY_LICENSES.html` com as licenças das dependências.

O Scriba se apoia em [whisper.cpp](https://github.com/ggml-org/whisper.cpp), no modelo [Whisper](https://github.com/openai/whisper) da OpenAI, no [Silero VAD](https://github.com/snakers4/silero-vad), em [FFmpeg](https://ffmpeg.org) e [x264](https://www.videolan.org/developers/x264.html), na fonte [Inter](https://rsms.me/inter/), em [htmx](https://htmx.org), nos ícones [Lucide](https://lucide.dev) e no [Tailwind CSS](https://tailwindcss.com).
