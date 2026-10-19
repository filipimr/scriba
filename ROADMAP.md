# Scriba — Roadmap

App local para Windows, distribuído como um único `.exe`, que **grava** (microfone + som do PC), **compacta** (vídeos e áudios) e **transcreve tudo o que foi falado**, entregando o **texto**. Este documento reúne as decisões de produto e de arquitetura, as fases de desenvolvimento com checklists, a política de testes, os riscos e o registro de decisões.

Última atualização: 2026-10-19.

## Status das fases

| Fase | Tema | Status |
| --- | --- | --- |
| 0 | Fundação e spikes de risco | Concluída |
| 1 | CI/CD no GitHub | Concluída |
| 2 | Esqueleto do app e design system | Concluída |
| 3 | Fila de jobs e upload | Concluída |
| 3.5 | Build nativo mais rápido | Em andamento |
| 4 | Transcrição de arquivos (MVP) | Não iniciada |
| 5 | Gravação ao vivo | Não iniciada |
| 6 | Compactação de vídeo e áudio | Não iniciada |
| 7 | Robustez, empacotamento e entrega final | Não iniciada |

Valores possíveis: Não iniciada, Em andamento, Concluída.

## Como vamos trabalhar

- **Uma fase por vez, separadamente.** Cada fase começa com um plano detalhado próprio, aprovado antes da execução, e só termina quando os critérios "Pronto quando" e os testes passam. Só então a próxima começa.
- **Ao abrir uma fase:** status "Em andamento" na tabela acima.
- **Ao fechar uma fase:**
  - Marcar os checklists.
  - Mudar o status para "Concluída".
  - Registrar as decisões e os números medidos no [Registro de decisões](#registro-de-decisões).
  - Atualizar o aviso de estado do `README.md` (o que o app já faz).
- **A partir da Fase 1, uma branch e um PR por fase**, com o CI verde antes do merge.
- **Definition of Done:** veja [Qualidade e testes](#qualidade-e-testes).
- Se um spike ou um teste derrubar uma decisão (por exemplo, se o exe único se mostrar inviável), este roadmap é revisado antes de seguir.

## Visão e escopo

Decisões de produto:

- **Objetivo:** gravar, compactar e transcrever. Os três são núcleo.
- **Saída:** a transcrição em texto de tudo que foi falado, venha de áudio, de vídeo ou da gravação ao vivo. O vídeo **não** recebe legendas.
- **Uso não comercial:** o libx264 (GPL) é permitido.
- **Modelo de IA invisível:** o usuário nunca vê, escolhe nem baixa o modelo. Ele vem embutido no executável e funciona offline.
- **Código no GitHub, com CI/CD.**
- **Toda funcionalidade com testes automatizados.**

O que fica de fora da v1 está no [Backlog](#backlog-fora-da-v1).

## Ambiente de referência

Levantamento de 2026-10-16 da máquina de desenvolvimento:

- Windows 11 Pro pt-BR, i7-13650HX (14 núcleos), 16 GB de RAM, RTX 3050 6 GB (drivers Vulkan e CUDA), 220 GB livres em disco.
- Voz SAPI "Microsoft Maria" (pt-BR), usada para gerar as fixtures de teste.
- Já instalados: Git, winget e Node.
- Instalados na Fase 0 por `scripts/setup.ps1`: Rust, VS Build Tools (C++), LLVM, CMake, vcpkg, FFmpeg (CLI, só para fixtures) e Vulkan SDK.
- GPUs Vulkan: dispositivo 0 = Intel integrada, dispositivo 1 = RTX 3050.

## Análise do PRD original

O PRD original (codinome "MediaPipe Core") propõe um monólito em Rust: Axum + HTMX, whisper.cpp e FFmpeg via FFI e captura de áudio com cpal, em 5 sprints.

### O que foi mantido

- Monólito Rust, sem Electron nem Python.
- whisper.cpp e FFmpeg via bindings, sem chamar processos externos.
- Fila em memória com canal Tokio.
- UI servida pelo próprio binário e aberta no navegador.

### O que foi ajustado

1. **Saída:** sai a legenda embutida no vídeo (Tarefa 3.2) e entra o `transcricao.txt`. Isso elimina libass e o mux de legenda.
2. **Tailwind e HTMX via CDN contradiziam "100% contido no executável".** O CDN exige internet, e o do Tailwind não é para produção. → CSS compilado com o Tailwind CLI standalone; CSS, htmx e fonte embutidos no binário. Funciona offline.
3. **O maior risco é o build nativo no Windows, não a UI** (FFmpeg estático, whisper.cpp e bindgen). → **Fase 0** com spikes antes de qualquer tela.
4. **Fatias verticais.** O PRD só fechava o ciclo (status e download) na Sprint 4. → Cada fase entrega um fluxo ponta a ponta. A primeira entrega de valor é arquivo → texto.
5. **Estados do job.** PENDING/PROCESSING/DONE não bastam. → Etapa, % de progresso, FALHOU e CANCELADO.
6. **Upload.** O Axum limita o corpo a 2 MB por padrão, e "processar pela memória" um vídeo de vários GB estoura a RAM. → Upload em streaming direto para o disco.
7. **Whisper e FFmpeg bloqueiam CPU e GPU.** → Worker em thread dedicada, fora do runtime Tokio. Modelo carregado uma vez. Callbacks de progresso e cancelamento (existem no whisper-rs 0.16).
8. **Gravação.** Mixar em tempo real fontes com relógios diferentes é frágil, e o loopback WASAPI **não entrega dados durante silêncio**. → **Duas trilhas** (mic e sistema) gravadas no disco, silêncio tocando na saída (técnica do OBS), mixagem só no fim. Bônus: transcrição rotulada **"Você" / "Outros"**.
9. **Segurança.** Um servidor local que liga o microfone pode ser acionado por qualquer site aberto no navegador (CSRF ou DNS rebinding). → Bind em 127.0.0.1, token de sessão, cookie SameSite=Strict e validação de Host/Origin.
10. **Alucinações do Whisper no silêncio.** Em pt-BR aparecem frases como "Legendas pela comunidade Amara.org" e "Obrigado por assistir". → VAD Silero, idioma fixo `pt`, `no_context` e filtro de frases-fantasma.
11. **Ciclo de vida.** O app precisa de ícone na bandeja (Abrir/Sair), instância única e encerramento limpo.
12. **Nome.** "MediaPipe" é um produto do Google. → **Scriba**. O arquivo de design é o `DESIGN-scriba.md`, e não `design.md`.
13. **Qualidade.** O PRD não tratava de testes nem de CI. → Testabilidade desde o desenho e CI/CD na Fase 1, antes de qualquer funcionalidade.

### Ordem das fases

- A transcrição vem antes da gravação, porque a gravação depende dela.
- A gravação vem antes da compactação de vídeo, porque já entrega a compactação do áudio gravado.

## Decisões de arquitetura

| Tema | Decisão | Consequência |
| --- | --- | --- |
| Plataforma | Windows 10/11 x64 na v1 | Loopback WASAPI suportado pelo cpal. macOS/Linux ficam no backlog |
| Transcrição | `whisper-rs` 0.16 com o modelo **large-v3-turbo q5_0 (~0,6 GB) embutido no .exe**, carregado da memória (`WhisperContext::new_from_buffer_with_params`) | Invisível e offline. O .exe fica com ~0,65 GB |
| Aceleração | Backend **Vulkan** (o `vulkan-1.dll` vem com o driver da GPU), com fallback para CPU | CUDA descartado: exigiria DLLs de centenas de MB ao lado do exe |
| Mídia | `ffmpeg-next` 9 + FFmpeg 9 **estático** via vcpkg com baseline fixado no repo: `ffmpeg[core,avcodec,avformat,swresample,swscale,x264,gpl,nvcodec]` | Sem DLLs do FFmpeg. O binário distribuído é GPL (veja a Fase 1) |
| Captura | `cpal` (WASAPI): mic e loopback em trilhas separadas, WAV mono 16-bit gravado no disco | À prova de crash. Ocupa ~345 MB/h por trilha até a compactação |
| Web | `axum` 0.8 + `askama` + htmx 2 + Tailwind v4 compilado + fonte Inter (substituta do SF Pro indicada pelo design doc), tudo embutido | Nenhum arquivo solto. Funciona offline |
| Estado | Fila em memória (`Arc<RwLock<HashMap>>` + `mpsc`), mais um manifesto `job.json` por saída | O histórico sobrevive a reinícios sem precisar de banco |
| Saídas | `Documentos\Scriba\AAAA-MM-DD HHmm - nome\` (`transcricao.txt`, `audio.m4a`/`video.mp4`). Temporários em `%LOCALAPPDATA%\Scriba` | Todos os caminhos são configuráveis, o que deixa os testes isolados |
| Compressão | Vídeo: x264 CRF ~26 (padrão) ou NVENC ("rápido"), até 1080p, áudio AAC. Áudio e gravações: AAC `.m4a` 96 kbps mono | Mantém o original se o ganho for menor que 5% |
| Testabilidade | Traits `Pipeline` (Fase 3), `Transcriber`, `MediaEngine`, `AudioSource` e `Clock`. Feature `test-support` com fakes e as flags `--fake-pipeline` e `--fake-audio`, fora do release | Toda a lógica roda no CI, sem placa de som nem GPU |

## Arquitetura

```text
Navegador (HTMX)  ⇄  Axum @127.0.0.1 [token + Host/Origin]
                        │ upload em streaming → %LOCALAPPDATA%\Scriba\uploads
                        ▼
                 JobStore (HashMap em memória) ──mpsc──▶ Worker (thread dedicada, 1 job por vez)
                        ▲                                   ├─ media::extract_audio  (FFmpeg → 16 kHz mono f32)
                        │                                   ├─ asr::transcribe       (whisper + VAD, progresso/cancelar)
                        │                                   ├─ transcript::write     (.txt com [hh:mm:ss] e Você/Outros)
                        │                                   └─ media::compress       (H.264/AAC)
                 Recorder (AudioSource: cpal mic + loopback + silêncio na saída | fake nos testes)
                   └─ ring buffers → thread de escrita → mic.wav / sistema.wav ──(Parar)──▶ novo job
```

Estrutura prevista (workspace Cargo):

```text
src/                     main.rs (startup, instância única, bandeja), web/, jobs/, media/, asr/, capture/, transcript/
crates/scriba-model/     modelo + VAD via include_bytes! (feature `embedded-model`, usada só no release)
templates/ ui/ assets/   askama, fonte do Tailwind + tokens gerados do DESIGN-scriba.md, assets embutidos
xtask/                   cargo xtask css | model | fixtures | test-local | sandbox-test | dist
tests/, tests/fixtures/  integração Rust; fixtures pequenas versionadas (geradas com a voz Maria)
e2e/                     Playwright (Node, só dev/CI): fluxos no navegador contra o binário real
.github/                 workflows ci.yml e release.yml, ação composta setup-native, dependabot, template de PR
vcpkg.json, deny.toml, scripts/setup.ps1 (ambiente local idempotente)
```

Em desenvolvimento, o modelo é carregado de `models/`, baixado por `cargo xtask model`, para o build continuar rápido. Só o release o embute.

## Qualidade e testes

Vale para todas as fases.

- **Definition of Done:** uma funcionalidade só está pronta com testes automatizados no nível certo e com o CI verde. Todo bug corrigido ganha um teste de regressão. O template de PR traz esse checklist.
- **Camadas de teste:**
  - **Unitários** (lógica pura) rodam no CI.
  - **Integração** roda no CI:
    - Router via `tower::oneshot`.
    - FFmpeg e Whisper com fixtures reais e um modelo pequeno (`base` q5_1).
    - Captura com `AudioSource` falso, com lacunas, jitter e erro de dispositivo.
  - **E2E** roda no CI: Playwright contra o binário real, com diretórios temporários e `--fake-audio`. Inclui comparação visual das telas (fidelidade ao design), com a referência gerada no próprio CI.
  - **Hardware** roda em `cargo xtask test-local`, automatizado e obrigatório antes de cada release, porque os runners do GitHub não têm placa de som nem GPU. Cobre mic e loopback reais, Vulkan, NVENC, desempenho com 1 h de áudio e o exe no Windows Sandbox (`cargo xtask sandbox-test`).
- **Ferramentas:**
  - `cargo nextest` (relatório JUnit no PR).
  - `cargo llvm-cov` com piso de cobertura: começa em 70% e sobe a cada fase.
  - `insta` (snapshots de texto e HTML) e `proptest` (alinhamento de trilhas, mescla Você/Outros).
- **Fixtures:** pequenas e versionadas em `tests/fixtures/`, regeneráveis com `cargo xtask fixtures`. A voz Maria lê textos conhecidos, e os testes verificam palavras-chave. Os runners não têm voz pt-BR, por isso os arquivos ficam no repo.

## Fases

### Fase 0 — Fundação e spikes de risco

Objetivo: provar, antes de qualquer tela, que o build nativo (FFmpeg estático, whisper.cpp e captura WASAPI) funciona nesta máquina e cabe em um único exe.

**Entregas:**

- [x] `scripts/setup.ps1` (idempotente) instala via winget: Rustup (MSVC), VS 2022 Build Tools (C++), LLVM, CMake e Vulkan SDK.
- [x] vcpkg com baseline fixado no repositório e FFmpeg estático compilado (~30–60 min).
- [x] Variáveis de ambiente do build em `.cargo/config.toml [env]`.
- [x] `git init`, workspace Cargo e `rust-toolchain.toml`.
- [x] `cargo xtask model`: baixa os modelos e confere o SHA-256.
- [x] `cargo xtask fixtures`: gera as fixtures de fala pt-BR com a voz Maria.
- [x] **Spike A, whisper:** transcrever uma fixture pt-BR. Medir CPU contra Vulkan e testar `new_from_buffer_with_params`.
- [x] **Spike B, FFmpeg:** extrair o áudio de um MP4 para 16 kHz mono e codificar H.264 com x264 e com NVENC. `dumpbin /dependents` não deve listar DLLs do FFmpeg.
- [x] **Spike C, captura:** gravar 30 s de mic + loopback, com 10 s de silêncio no meio. Os dois WAVs devem ter a mesma duração (±100 ms).
- [x] **Spike D, exe único:** crate do modelo com `include_bytes!`. Medir build, tamanho e carga. Tentar CRT estático.

**Testes:**

- [x] Cada spike vira um teste de integração. Os que dependem de hardware entram na suíte local.

**Pronto quando:**

- [x] Os 4 spikes passam nesta máquina.
- [x] As versões fixadas e as decisões estão no [Registro de decisões](#registro-de-decisões).

Observação: as instalações pedem confirmação de administrador (UAC). Na prática foram ~15–25 GB entre ferramentas, FFmpeg compilado, modelos (~0,6 GB) e builds; `setup.ps1 -LimparVcpkg` apaga os temporários do vcpkg.

### Fase 1 — CI/CD no GitHub

Objetivo: a partir daqui, todo PR passa por verificação automática e toda tag gera uma release testada.

**Entregas:**

- [x] Repositório no GitHub, com nome e visibilidade definidos pelo mantenedor.
- [x] Proteção da `main` exigindo CI verde.
- [x] Template de PR com o checklist de testes.
- [x] Dependabot para cargo, npm (`e2e/`) e GitHub Actions.
- [x] `ci.yml` (push e PR, com `concurrency` cancelando execuções antigas):
  - Job `checks` (windows-latest): `fmt --check`, `clippy -D warnings`, nextest (unit + integração), Playwright E2E e cobertura com piso.
  - Job `deny` (ubuntu, mais barato): `cargo deny` para vulnerabilidades (RustSec) e licenças compatíveis com GPL.
- [x] Ação composta `setup-native`:
  - Rust fixado.
  - vcpkg com cache de binários (`VCPKG_BINARY_SOURCES` + `actions/cache`).
  - `Swatinem/rust-cache`.
  - Cache do modelo de teste por SHA-256.
- [x] `release.yml` (tag `v*`):
  - Confere se a tag bate com a versão do Cargo e faz o build de release.
  - **Smoke test** do exe (`scriba.exe --self-test`: carrega o modelo, transcreve uma amostra embutida e codifica um clipe curto).
  - Gera o SHA-256 e publica a GitHub Release com notas automáticas e `THIRD_PARTY_LICENSES` (`cargo about`).
- [x] `LICENSE` GPL-3.0, exigida ao distribuir o exe com x264.

**Pronto quando:**

- [x] Um PR com teste quebrado fica bloqueado.
- [x] Um PR verde roda em menos de 20 min com cache. A 1ª execução compila o FFmpeg e leva ~1 h.
- [x] A tag `v0.1.0` publica uma release com o exe do esqueleto e o checksum.

Observação: em repositório privado, os minutos do GitHub Actions têm cota, e os runners Windows consomem mais. Cache, `concurrency` e filtros de caminho reduzem esse consumo.

### Fase 2 — Esqueleto do app e design system

Base no PRD: Sprint 1. Objetivo: o exe sobe o servidor local seguro e abre no navegador a tela base, fiel ao design.

**Entregas:**

- [x] Axum em 127.0.0.1 (porta preferida e fallback), com encerramento limpo.
- [x] Middleware de segurança: token na URL → cookie SameSite=Strict; validação de Host e Origin.
- [x] Assets embutidos (CSS, htmx, Inter, ícones) com cache.
- [x] Layout askama (`app-header`, painel de gravação e dropzone, "Recentes", `footer`) seguindo o `DESIGN-scriba.md`.
- [x] Logo vetorizado a partir de `docs/brand/logo-original.jpg`: SVG do ícone, versão branca e versão simplificada de 16 px (favicon e bandeja).
- [x] `cargo xtask css`: gera os tokens a partir do YAML (tema claro em `:root`, escuro em `[data-theme="dark"]` e em `prefers-color-scheme`) e compila o Tailwind.
- [x] Alternância de tema no header (Sistema → Claro → Escuro), salva na configuração e renderizada pelo servidor no `<html>`, sem piscar.
- [x] Instância única via `instance.json`.
- [x] Flags `--no-open`, `--port`, `--data-dir` e `--output-dir`.

**Testes:**

- [x] Unitários: token, Host e Origin; fallback de porta; lockfile; gerador de tokens CSS (snapshot).
- [x] Integração: rotas com 200/401/403; content-type e cache dos assets; segunda instância.
- [x] E2E: a página carrega e bate com a referência visual nos dois temas; a alternância de tema persiste após recarregar.
- [x] Unitário: contraste AA de todo par texto/fundo dos componentes, nos dois temas, calculado a partir do YAML.

**Pronto quando:**

- [x] `cargo run` abre a tela fiel ao design.
- [x] Requisição sem token retorna 401, e Host estranho retorna 403.
- [x] A segunda execução reaproveita a instância aberta.

### Fase 3 — Fila de jobs e upload

Base no PRD: Sprint 2 e o polling da Sprint 4. Objetivo: arquivos entram por upload, viram jobs em uma fila em memória e a UI acompanha o andamento, ainda com um pipeline falso.

**Entregas:**

- [x] Modelo de job: id ULID, tipo, opções (transcrever/compactar), etapa, %, erro e saídas.
- [x] Worker em `std::thread` com `blocking_recv`, `catch_unwind` por job e cancelamento por `AtomicBool`.
- [x] Upload multipart em streaming para o disco (limite de corpo desativado só nessa rota).
- [x] Rotas `POST /api/jobs`, `GET /api/jobs`, `GET /api/jobs/{id}`, `POST /api/jobs/{id}/cancel` e `DELETE /api/jobs/{id}`.
- [x] Polling de 1 s só para jobs ativos; a resposta 286 encerra o polling.
- [x] Pipeline falso atrás do trait, para validar a UI antes dos motores.

**Testes:**

- [x] Unitários: máquina de estados (proptest) e semântica de cancelamento.
- [x] Integração: upload grande gravado no disco em streaming, sem acumular na memória; panic no pipeline vira FALHOU.
- [x] E2E: upload → progresso → concluído; cancelar; falha exibida.

**Pronto quando:**

- [x] Um upload de 2 GB mantém a RAM estável.
- [x] Progresso, cancelamento e falha aparecem corretos na UI.

### Fase 3.5 — Build nativo mais rápido

Objetivo: cortar o build frio do FFmpeg quase pela metade, no CI e na máquina de desenvolvimento, e manter o cache do CI vivo.

**Contexto:** os dois builds frios de 2026-10-19, com a mesma configuração e runners diferentes, mediram onde vai o tempo do passo `setup-native`:

| Etapa | PR #11 | `main` |
| --- | --- | --- |
| Clone e bootstrap do vcpkg | ~40 s | ~42 s |
| x264, debug + release | 3,8 min | 2,4 min |
| FFmpeg, versão release | 11,3 min | 7,1 min |
| FFmpeg, versão debug | 11,3 min | 6,8 min |
| `setup-native` total | 29,5 min | 18,9 min |

A versão debug, cerca de 38% do passo, nunca é usada: o Rust linka sempre o CRT de release, e o `ffmpeg-sys-next`, com `FFMPEG_DIR`, procura as libs só em `<dir>/lib`. As libs debug ocupam 211 dos 430 MB do `vcpkg_installed` local. O cache também é frágil: o GitHub apaga um cache que fica 7 dias sem acesso, e a `main` não lê caches criados em PR, então mudar a chave custa um build frio na branch e outro na `main`.

**Entregas:**

- [x] Triplet `x64-windows-static-release`, versionado como overlay em `triplets/`, no lugar do `x64-windows-static`. Atualiza `.cargo/config.toml` (`FFMPEG_DIR`, `VCPKGRS_TRIPLET`), a ação `setup-native`, o README e o `CLAUDE.md`. Quem já tem o ambiente reinstala o FFmpeg uma vez.
- [x] Chave do cache do FFmpeg com o nome do triplet e o hash de `vcpkg.json` e `triplets/`.
- [x] CI semanal agendado na `main` (`schedule` no `ci.yml`). Ele mantém todos os caches vivos (FFmpeg, Rust, modelo, npm) e ainda pega quebras vindas de atualizações do runner e avisos novos do RustSec.
- [x] O `CLAUDE.md` passa a explicar o custo de mudar o `vcpkg.json` ou o triplet: um build frio na branch e outro na `main`.

**Testes:**

- [x] CI verde com cache frio (a chave nova força o build) e o tempo medido.
- [x] Localmente: instalar com o novo triplet, rodar `cargo test --workspace --features spikes/vulkan` e o `scriba --self-test` do build de release, e conferir no `dumpbin /dependents` que o exe não depende de DLLs do FFmpeg.

**Pronto quando:**

- [x] O log do vcpkg no CI não tem mais build de debug, e o build frio cai ~40%. Hoje leva de 19 a 30 min, conforme o runner; a estimativa é de 11 a 17 min.
- [x] O cache do FFmpeg cai de 77 MB para ~40 MB. Medido: 61 MB. A estimativa partia do tamanho em disco, que caiu pela metade (de 428 para 217 MB), mas no cache comprimido as libs debug pesavam menos.
- [ ] O CI agendado roda na `main` sem erro.

### Fase 4 — Transcrição de arquivos (MVP)

Base no PRD: Sprint 3.1 e Sprint 4. Objetivo: qualquer áudio ou vídeo vira texto em pt-BR. É a primeira entrega de valor.

**Entregas:**

- [ ] `media::extract_audio`: melhor faixa de áudio → 16 kHz mono f32, com progresso por pts e erros amigáveis (ex.: "arquivo sem áudio").
- [ ] `asr`:
  - Contexto carregado sob demanda e liberado após 10 min ocioso.
  - Idioma `pt`, VAD Silero e `no_context`.
  - Threads = núcleos − 2.
  - Progresso, cancelamento e filtro de frases-fantasma.
- [ ] Escolha do backend (achado do spike A: só CPU com o modelo grande leva ~8 h para 1 h de áudio):
  - Usar a GPU **dedicada** quando houver, e não o dispositivo Vulkan 0, que costuma ser a integrada. Se não houver dedicada, usar a integrada.
  - Sem GPU Vulkan, transcrever na CPU com um **modelo menor embutido**. A escolha entre `small` e `base` sai de um benchmark de precisão × tempo nas fixtures. O custo é o exe crescer ~60–190 MB.
  - Antes de fechar o fallback, investigar a lentidão da CPU (~87 s para 10,9 s), comparando com o whisper.cpp oficial e testando threads e OpenMP.
  - Mostrar na UI qual backend está em uso e uma estimativa de tempo antes de começar.
- [ ] `transcript`: parágrafos (pausa > 1,5 s quebra), `[hh:mm:ss]` em cada parágrafo e cabeçalho (origem, data, duração). Gera `transcricao.txt` e `job.json`.
- [ ] UI: leitor da transcrição, Copiar texto, Baixar .txt e Abrir pasta.
- [ ] Herdado da Fase 3:
  - O pipeline real substitui o falso no app; o falso vai para a feature `test-support`, com a flag `--fake-pipeline` para o E2E.
  - "Tentar de novo" no job que falhou (o envio fica guardado até a nova tentativa).
  - Remover um job que tem saídas passa pelo `confirm-dialog`.
  - Card com duração e data (probe do FFmpeg e hora local, a mesma da pasta `AAAA-MM-DD HHmm`).

**Testes:**

- [ ] Unitários: parágrafos, formato de tempo, filtro de frases-fantasma e cabeçalho (snapshots).
- [ ] Integração com o modelo pequeno:
  - Fixtures MP4, MP3, M4A, OGG e WAV → palavras-chave.
  - Silêncio puro → texto vazio, sem frases-fantasma.
  - Cancelar no meio interrompe a transcrição.
  - Arquivo sem áudio → erro amigável.
- [ ] E2E: upload → texto visível → copiar → .txt baixado com o conteúdo certo.
- [ ] Unitário: a escolha do backend prefere GPU dedicada, depois integrada, depois CPU com o modelo menor (com uma lista de dispositivos falsa).
- [ ] Local: benchmark com 1 h de áudio real em cada backend (RTX, Intel integrada e só CPU).

**Pronto quando:**

- [ ] Arquivos reais (MP4 do Teams, MP3, M4A, WAV, OGG) viram texto pt-BR correto.
- [ ] O tempo para transcrever 1 h de áudio na RTX 3050 está registrado.
- [ ] Metas para 1 h de áudio: ≤ 10 min na RTX 3050 e ≤ 1 h só na CPU (com o modelo menor). Os tempos medidos nos três backends ficam no registro de decisões.

### Fase 5 — Gravação ao vivo

Base no PRD: Sprint 5. Objetivo: um clique grava o microfone e o som do PC; ao parar, saem a transcrição rotulada "Você" / "Outros" e o áudio compactado.

**Entregas:**

- [ ] Lista de dispositivos (padrões marcados), com escolha nos ajustes.
- [ ] Recorder com três streams:
  - Mic.
  - Loopback: stream de entrada aberto no dispositivo de saída com `default_output_config`.
  - Silêncio tocando na mesma saída.

  Os callbacks só copiam para ring buffers SPSC (`rtrb`).
- [ ] Thread de escrita: downmix para mono, conversão para i16, WAV via `hound` com `flush` a cada 5 s. Alinhamento por timestamp. Medidores RMS.
- [ ] Rotas `POST /api/record/start`, `POST /api/record/stop` e `GET /api/record/status`.
- [ ] UI: tile escuro com botão grande e barra fixa "● Gravando 00:12:34 · Parar".
- [ ] Job de gravação:
  - Mixdown para `audio.m4a`.
  - Transcrição de cada trilha, mesclada por tempo com os rótulos Você/Outros.
  - Remoção do eco e geração do `transcricao.txt`.
  - WAVs apagados após o sucesso.
- [ ] Robustez:
  - Dispositivo removido: salva o que foi gravado.
  - Troca da saída padrão: reabre o loopback.
  - Checagem de espaço em disco e teto de ~12 h (limite de 4 GB do WAV).

**Testes:**

- [ ] Unitários: downmix e i16; alinhamento e preenchimento; mescla e dedupe de eco (proptest); teto de duração.
- [ ] Integração com `AudioSource` falso:
  - Lacunas e jitter → trilhas alinhadas (±10 ms).
  - Erro de dispositivo → arquivo finalizado.
  - Fixtures "Você"/"Outros" + eco → texto rotulado e sem duplicatas.
- [ ] E2E com `--fake-audio`: Gravar → timer → Parar → transcrição rotulada.
- [ ] Local: mic e loopback reais com 15 s de silêncio no meio.

**Pronto quando:**

- [ ] Falar com música ou reunião tocando, clicar em Parar e receber o texto com as duas vozes rotuladas.
- [ ] 15 s de silêncio no meio da gravação não desalinham as trilhas.

### Fase 6 — Compactação de vídeo e áudio

Base no PRD: Sprint 3.2 e Sprint 4.2. Objetivo: vídeos e áudios enviados ficam menores.

**Entregas:**

- [ ] `media::compress_video`:
  - Escala até 1080p, mantendo a proporção, com dimensões pares.
  - Codifica com x264 CRF 26 (padrão) ou NVENC p5 CQ 28.
  - Áudio AAC, MP4 `faststart`, progresso e cancelamento.
- [ ] `media::compress_audio` (reaproveitado da Fase 5) para áudios enviados.
- [ ] O original é mantido quando o ganho for menor que 5%.
- [ ] UI: tamanho antes → depois (−xx%).

**Testes:**

- [ ] Unitários: regra dos 5%, cálculo de dimensões e preset → parâmetros.
- [ ] Integração: vídeo sai H.264 com as dimensões e a duração esperadas (±0,1 s) e menor que o original; áudio sai AAC; cancelar no meio interrompe.
- [ ] E2E: upload com "Compactar" → tamanho antes/depois na tela.
- [ ] Local: NVENC.

**Pronto quando:**

- [ ] Uma gravação de 1 h do Teams fica menor e abre no Media Player e no VLC.

### Fase 7 — Robustez, empacotamento e entrega final

Objetivo: o `scriba.exe` final roda em qualquer Windows sem instalar nada e se comporta como um app de bandeja.

**Entregas:**

- [ ] `windows_subsystem = "windows"`, ícone na bandeja (Abrir/Sair), ícone e metadados do exe (`winresource`).
- [ ] Logs com rotação em `%LOCALAPPDATA%\Scriba\logs` e panic hook.
- [ ] Histórico reconstruído a partir dos `job.json`, `config.json` e limpeza de temporários antigos.
- [ ] `release.yml` completo: Vulkan SDK no runner, modelo embutido e `--self-test` exercitando também o fallback para CPU (o runner não tem GPU).
- [ ] `cargo xtask dist` faz o mesmo build localmente.
- [ ] README com o uso final (bandeja e release completa; ele existe desde a Fase 3) e `CLAUDE.md` com os comandos reais.

**Testes:**

- [ ] Unitários: config (carga, defaults e migração); limpeza por idade com `Clock` falso; ações do menu da bandeja via canal.
- [ ] Integração: histórico a partir de `job.json` de fixture; `--self-test`.
- [ ] Local: `cargo xtask sandbox-test` roda o exe no Windows Sandbox (máquina limpa) e lê o resultado.

**Pronto quando:**

- [ ] A tag gera uma release que passa no smoke test do CI e no Sandbox, com o fluxo completo de gravar, compactar e transcrever.

## Backlog (fora da v1)

- Transcrição ao vivo durante a gravação.
- Diarização real (vários falantes em "Outros").
- Resumo ou ata via LLM.
- Gravação de tela.
- HEVC/AV1.
- Exportar .srt e .docx.
- Atalho global para gravar.
- macOS/Linux.
- Runner self-hosted na máquina de desenvolvimento para rodar a suíte de hardware no CI (só com repositório privado, por segurança).
- FFmpeg enxuto: um port em overlay só com os decoders, encoders e formatos usados encurta o build e diminui o exe, mas pode recusar arquivos que hoje abrem. Avaliar junto com o tamanho do exe.

## Riscos e mitigações

| Risco | Mitigação |
| --- | --- |
| Build nativo falha (bindgen, CRT misto, FFmpeg) | A Fase 0 vem primeiro, com versões fixadas. Plano B: DLLs do FFmpeg ao lado do exe, distribuído em zip |
| 1ª execução do CI compila o FFmpeg e demora ~1 h | Cache de binários do vcpkg, cache aquecido na `main` e timeout do job ajustado |
| Runners sem placa de som nem GPU | `AudioSource` falso e `--fake-audio` no CI. Hardware coberto pela suíte local obrigatória antes da release |
| Testes E2E instáveis | Porta 0 e diretórios temporários por teste. Esperar eventos do HTMX em vez de `sleep`. Retry só no CI, com relatório |
| `ffmpeg-next` está em modo manutenção | Fixar a 9.x junto com o FFmpeg 9 do vcpkg. Alternativa: `rsmpeg` |
| `include_bytes!` de ~0,6 GB deixa o build lento | Crate separado, compilado só no release. Plano B: modelo anexado ao fim do exe e lido via mmap |
| O exe não abre em máquina sem Vulkan | Confirmado na Fase 0: o exe importa `vulkan-1.dll` diretamente. `--self-test` no runner e no Sandbox. Plano B: carregar o Vulkan com delay-load ou variante `scriba-cpu.exe` |
| Loopback mudo no silêncio ou deriva de relógio | Silêncio tocando na saída, alinhamento por timestamp e testes de lacuna |
| Eco (sem fone de ouvido) duplica falas | Dedupe por sobreposição e similaridade, mais uma dica na UI |
| Site malicioso liga o microfone | Token, SameSite=Strict, Host/Origin e bind em 127.0.0.1, todos com testes de 401/403 |

## Verificação

- **No CI, a cada PR:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo nextest run`, `npx playwright test` (em `e2e/`), `cargo llvm-cov` com piso e `cargo deny check`.
- **Localmente, antes de cada release:** `cargo xtask test-local` (hardware, GPU e desempenho) e `cargo xtask sandbox-test`.
- **Na release:** o smoke test do `scriba.exe --self-test` no runner precisa passar antes da publicação.
- **Em cada fase:** só fecha com o critério "Pronto quando" atendido e o CI verde.

## Registro de decisões

| Data | Fase | Decisão | Motivo |
| --- | --- | --- | --- |
| 2026-10-16 | Planejamento | A saída é a transcrição em texto; o vídeo não recebe legendas | O objetivo é ter o texto de tudo que foi falado |
| 2026-10-16 | Planejamento | Windows 10/11 x64 na v1 | Loopback WASAPI suportado pelo cpal; é o ambiente de uso |
| 2026-10-16 | Planejamento | Modelo large-v3-turbo q5_0 embutido no exe | Modelo invisível ao usuário e funcionamento offline |
| 2026-10-16 | Planejamento | Vulkan em vez de CUDA | Preserva o exe único: o loader do Vulkan vem com o driver |
| 2026-10-16 | Planejamento | FFmpeg estático com x264 (GPL) | Uso não comercial e melhor compressão. Exige `LICENSE` GPL-3.0 ao distribuir |
| 2026-10-16 | Planejamento | Tailwind compilado e assets embutidos, sem CDN | Funcionar offline, com tudo dentro do executável |
| 2026-10-16 | Planejamento | Gravação em duas trilhas, com mixagem só no fim | Robustez do loopback e rótulos Você/Outros |
| 2026-10-16 | Planejamento | CI/CD no GitHub Actions e testes automatizados obrigatórios | O código será hospedado no GitHub; Definition of Done |
| 2026-10-16 | Planejamento | `DESIGN-scriba.md` reescrito como design system de app: Inter embutida, componentes do Scriba, cores de status como exceção ao acento único, marca monocromática | O arquivo original descrevia o site de marketing da Apple; o logo define uma marca preto e branco em traço fino |
| 2026-10-16 | Planejamento | Temas claro e escuro, alternáveis no header; padrão segue o Windows | Pedido do usuário; tokens `themes.dark` no `DESIGN-scriba.md` |
| 2026-10-16 | Fase 0 | Versões fixadas: Rust 1.99.1 (`rust-toolchain.toml`), vcpkg baseline `7d5f3c6` com FFmpeg 9.0.2, `whisper-rs` 0.16.0 (`whisper-rs-sys` 0.15.0), `ffmpeg-next` 9.0.0, `cpal` 0.18.2, `hound` 3.5.1. Ferramentas: VS 2022 Build Tools 17.14, LLVM 23.1.3, CMake 4.4.4, Vulkan SDK 1.4.363.0 | Reprodutibilidade do build |
| 2026-10-16 | Fase 0 | Triplet `x64-windows-static` + `+crt-static` (também `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded` para o whisper.cpp). O exe depende só de DLLs do Windows e de `vulkan-1.dll` | Spike D: com `static-md` o exe exigia o VC++ Redistributable |
| 2026-10-16 | Fase 0 | `build.rs` declara as libs de link do FFmpeg estático: `libx264`, `secur32`, `crypt32`, `ncrypt`, `bcrypt`, `ole32`, `user32`, `mfuuid`, `strmiids`, `mfplat` | Com `FFMPEG_DIR`, o `ffmpeg-sys-next` não as declara |
| 2026-10-16 | Fase 0 | whisper.cpp com `GGML_NATIVE=OFF` e base AVX/AVX2/FMA/F16C | Com MSVC, o `GGML_NATIVE` não detecta a CPU e compila sem SIMD |
| 2026-10-16 | Fase 0 | `CARGO_TARGET_DIR=C:\dev\t\scriba` e vcpkg em `C:\dev\vcpkg` | O build Vulkan do whisper.cpp estoura MAX_PATH (260) dentro do repositório |
| 2026-10-16 | Fase 0 | Spikes em `crates/spikes` (testes de hardware com `#[ignore = "hardware"]`). CLI do FFmpeg (`Gyan.FFmpeg.Essentials`) só para gerar fixtures | Mantém o app limpo e o FFmpeg embutido enxuto |
| 2026-10-16 | Fase 0 | Medições: FFmpeg estático compila em 20,7 min (`static-md`) e 11,7 min (`static`, com cache do x264). Exe com modelo embutido: 604 MB; o modelo (574 MB) carrega da memória em ~2,8 s; rebuild só do crate do modelo, 19 s | Spike D |
| 2026-10-16 | Fase 0 | Medições, large-v3-turbo q5_0, fixture de 10,9 s (release): RTX 3050 via Vulkan ~0,9 s (~12× tempo real); a 1ª execução compila os shaders e leva ~11 s. Intel integrada ~15 s. CPU com 18 threads ~87 s (0,13× tempo real), escalando com as threads | Spike A |
| 2026-10-16 | Fase 0 | Achados para a Fase 4: (1) o Vulkan enumera a GPU integrada como dispositivo 0, então o app precisa escolher a GPU dedicada (tipo discreto); (2) o fallback de CPU com o modelo grande é inviável para áudios longos: investigar threads/OpenMP ou usar modelo menor sem GPU; (3) whisper em testes paralelos disputa a CPU (3 testes base: 498 s), então os testes de whisper devem rodar em série no CI; (4) o silêncio puro gera "[MÚSICA DE FUNDO]" sem VAD | Spike A |
| 2026-10-16 | Fase 0 | Política de backend da Fase 4: GPU dedicada → GPU integrada → CPU com modelo menor embutido (`small` ou `base`, por benchmark). Metas para 1 h de áudio: ≤ 10 min na RTX 3050 e ≤ 1 h na CPU | Estimativa linear do spike A: 1 h de áudio levaria ~5 min na RTX, ~80 min na integrada e ~8 h na CPU com o modelo grande |
| 2026-10-16 | Fase 0 | Medições: extração de áudio de MP4/MP3/M4A/OGG dá 10,93 s, igual ao WAV; x264 e NVENC codificam 2 s de vídeo; o exe de teste não lista DLLs do FFmpeg | Spike B |
| 2026-10-16 | Fase 0 | Medições: mic 29,930 s e loopback 30,010 s em 30 s sem som no PC (diferença de 80 ms, limite de 100 ms), com o stream de silêncio na saída. WASAPI compartilhado entrega f32 48 kHz estéreo | Spike C. A folga é pequena: o alinhamento por timestamp da Fase 5 é necessário |
| 2026-10-17 | Fase 1 | Repositório `filipimr/scriba` público | Proteção da `main` e Actions sem cota no plano Free; o código é GPL |
| 2026-10-17 | Fase 1 | `scriba --self-test` em CPU com o modelo `base` q5_1 e `fala.wav` embutidos (feature `embedded-test-model`); exe de 83 MB. O modelo grande e o Vulkan na release ficam para a Fase 7 | O runner não tem GPU nem Vulkan SDK; prova carga da memória, whisper e x264 |
| 2026-10-17 | Fase 1 | CI sem a feature `vulkan` (clippy e testes); testes de whisper em série via grupo do nextest; piso de cobertura de 70% só nos crates do app (spikes e xtask fora) | Runner sem Vulkan; achado (3) do spike A |
| 2026-10-17 | Fase 1 | Cache do FFmpeg = `vcpkg_installed/` inteiro, chaveado pelo hash do `vcpkg.json`, salvo mesmo se o job falhar. A versão do `vcpkg.json` fica fixa: mudar o arquivo recompila o FFmpeg | Simples e sem servidor de binários |
| 2026-10-17 | Fase 1 | CI roda em todo PR (sem `paths-ignore`); só o push na `main` ignora `.md` e `docs/` | Com checks obrigatórios, um PR só de documentação nunca receberia os checks e ficaria bloqueado |
| 2026-10-17 | Fase 1 | E2E: projeto Playwright em `e2e/` com um teste pulado até a Fase 2 trazer o servidor | O job já existe e roda no CI |
| 2026-10-17 | Fase 1 | Medições no runner windows-latest: sem cache do FFmpeg, 34–36 min (compilação do FFmpeg ~30 min, abaixo da estimativa de 1 h); com cache, `checks` 3 min 48 s e `deny` ~40 s; release sem cache 33 min; o self-test transcreve os 10,9 s da amostra em 48 s em CPU | O cache é por ref: PRs leem o da `main`, aquecido a cada push nela |
| 2026-10-17 | Fase 2 | Versões: `axum` 0.8.9, `askama` 0.16.1, `tokio` 1.53, htmx 2.0.11, Tailwind CLI standalone 4.3.3 (SHA-256 fixado no xtask), Inter 4.1 (variável, woff2), Lucide 1.59.0 (só os ícones usados, inline) | Reprodutibilidade; tudo embutido, sem CDN |
| 2026-10-17 | Fase 2 | `ui/tokens.css` e `assets/app.css` gerados e **versionados**; o CI só confere com `cargo xtask css --check` | O `cargo build` e o CI não dependem do Tailwind; o diff do CSS aparece no PR |
| 2026-10-17 | Fase 2 | Tokens no `@theme static` do Tailwind (todas as cores viram `--color-*`, usadas ou não); escuro em `[data-theme="dark"]` e em `prefers-color-scheme` para `:root:not([data-theme="light"])` | Bate com o "Como funciona" do design; o servidor renderiza o `data-theme`, sem piscar |
| 2026-10-17 | Fase 2 | Porta preferida 47800, com fallback para porta livre. Token de 32 bytes na URL (`?t=`) → cookie `HttpOnly; SameSite=Strict` + redirect; Host precisa ser `127.0.0.1`/`localhost` na porta; Origin obrigatório e igual em métodos não-GET; CSP `default-src 'self'` | Segurança do servidor local (CSRF e DNS rebinding) |
| 2026-10-17 | Fase 2 | Instância única: `instance.json` {id, porta, token} + `GET /health` (sem token, devolve só o id). A 2ª execução reabre o navegador na instância viva e sai | Dispensa checar PID; arquivo velho ou porta reaproveitada por outro programa não enganam |
| 2026-10-17 | Fase 2 | Favicon em SVG (`logo-16.svg`); o `.ico` fica para a Fase 7 (exe e bandeja). No header, `logo-white.svg` (quadrado branco, caneta preta) com traço mais grosso, legível a 24 px | Navegadores atuais aceitam favicon SVG |
| 2026-10-17 | Fase 2 | Abas "Arquivos"/"Gravar" e "Sair" do header ainda não existem: chegam com as telas (Fases 3–5) e a bandeja (Fase 7). Dropzone e botão de gravar aparecem desabilitados | Sem funcionalidade por trás nesta fase |
| 2026-10-17 | Fase 2 | E2E no Edge instalado (`channel: "msedge"`), sem baixar navegador. Referência visual gerada no runner e versionada em `e2e/tests/tela.spec.ts-snapshots/`: o workflow manual `e2e-snapshots.yml` (ou um run que falhe) publica as imagens como artefato para commit. Localmente a comparação visual é pulada | Renderização local e do runner diferem (fontes, DPI) |
| 2026-10-17 | Fase 2 | `about.hbs` credita os assets que não são crates: Inter (OFL 1.1), htmx (0BSD), Lucide (ISC), Tailwind (MIT) | O `cargo about` só enxerga crates |
| 2026-10-17 | Fase 2 | Medição: job `checks` com cache em ~7–9 min (7 min 20 s e 8 min 41 s nas duas execuções do PR; antes ~3 min 50 s); a diferença é o `cargo build` de debug para o E2E, separado do build instrumentado da cobertura | Ainda bem abaixo da meta de 20 min |
| 2026-10-19 | Fase 3 | Versões: `axum` com a feature `multipart` (`multer` 3.1.0) e `tokio` com `fs`, `io-util` e `sync`. Só nos testes: `proptest` 1.12.0, `insta` e `windows-sys` 0.61.2 (pico de memória do processo) | Reprodutibilidade; o exe só ganha o parser de multipart |
| 2026-10-19 | Fase 3 | A API da fila responde fragmentos HTML (htmx), sem JSON. Erro de envio volta com 422 (507 se a gravação falhar), `HX-Retarget` para o alerta da dropzone e `responseHandling` no `htmx-config` liberando a troca. O card ativo faz polling de 1 s trocando o card inteiro; no fim, 286 com o card final | O único cliente é a própria UI; o status HTTP continua certo para os testes |
| 2026-10-19 | Fase 3 | `JobId` no formato ULID gerado no próprio código (`getrandom`), monotônico no mesmo milissegundo; store em `RwLock<BTreeMap>` em vez do `HashMap` previsto | Sem crate novo; "Recentes" sai na ordem de criação pelo próprio id |
| 2026-10-19 | Fase 3 | Barra de progresso no `<progress>` nativo; o comportamento fica no `app.js`, sem `hx-on` | A CSP (`default-src 'self'`) bloqueia `style` inline e `eval`; o `<progress>` já é acessível |
| 2026-10-19 | Fase 3 | O `FakePipeline` é o pipeline do app até a Fase 4, com ganchos pelo nome do arquivo: `lento` (10× mais devagar), `falha` (erro amigável) e `panico`. Na Fase 4 ele vai para a feature `test-support` | O objetivo da fase é validar a fila e a UI antes dos motores |
| 2026-10-19 | Fase 3 | Envio gravado em `uploads/<id>.<ext>` (o nome do cliente só é exibido), apagado quando o job termina ou é removido; `uploads/` é esvaziada ao iniciar. As opções vêm antes do arquivo no formulário, e o servidor recusa o envio sem opção antes de gravar; num erro, lê o corpo até o fim para o navegador receber a resposta | Nenhum caminho controlado pelo cliente, nenhuma sobra entre execuções, erro rápido em envio grande |
| 2026-10-19 | Fase 3 | Cancelamento cooperativo: na fila, termina na hora; rodando, vira "Cancelando…" e a flag para o pipeline. Depois do pedido, `Ok` vale como Concluído, erro vira Cancelado e panic vira Falhou. "Remover" só em job terminado e sem confirmação, porque só apaga a cópia temporária; "Tentar de novo" e o `confirm-dialog` ficam para a Fase 4 | Semântica testada com proptest e com o worker real |
| 2026-10-19 | Fase 3 | Ctrl+C cancela o job ativo e espera o worker; as requisições em andamento têm 5 s (`SHUTDOWN_GRACE`) | Um upload longo ou parado não trava o encerramento |
| 2026-10-19 | Fase 3 | O botão de ação do card mantém o id entre Cancelar e Remover, e o htmx devolve o foco a ele a cada polling (teste E2E). A região `#avisos` anuncia só a entrada e o fim dos jobs | O card é trocado a cada segundo; anunciar cada % seria ruído |
| 2026-10-19 | Fase 3 | Design: `status-badge-active` com `colors.link`, porque o `primary` pedido para "Processando" dava 2,97:1 no escuro; `option-toggle` mostra o check só quando selecionada | Contraste AA e a regra "texto azul usa `colors.link`" |
| 2026-10-19 | Fase 3 | E2E com um `scriba.exe` por teste (fixture `scriba`), no lugar do `global-setup` compartilhado | Mitigação "diretórios temporários por teste"; nenhum estado vaza entre testes |
| 2026-10-19 | Fase 3 | Piso de cobertura sobe de 70% para 85% | A Fase 2 terminou com 92,6% das linhas cobertas no CI |
| 2026-10-19 | Fase 3 | Medição: upload de 2 GB pelo exe de debug em 12,4 s (165 MB/s), com o pico de working set indo de 14 MB para 19 MB (+5 MB). O CI repete com 256 MB e falha acima de +100 MB | Critério "upload de 2 GB com RAM estável" |
| 2026-10-19 | Fase 3 | `.gitattributes` com `* text=auto eol=lf`: todo texto sai com LF no checkout, em qualquer máquina | No runner, o `core.autocrlf` trocava os SVGs embutidos por CRLF e quebrava o snapshot HTML do card; os assets do exe ficam idênticos entre máquinas. Custo único: o `vcpkg.json` passa a sair com LF, o hash da chave do cache muda e o FFmpeg é recompilado uma vez na branch e uma na `main` |
| 2026-10-19 | Fase 3 | Medição no CI do PR: job `checks` em 39 min, dos quais 29,5 min recompilando o FFmpeg (cache refeito pelo `.gitattributes`); sem isso, ~9,5 min, com testes e cobertura em ~3 min e os 16 testes E2E em ~1 min. Cobertura de linhas: 94,1% (piso de 85%) | Com cache, ainda bem abaixo da meta de 20 min |
| 2026-10-19 | Planejamento | Cria a Fase 3.5 (build nativo mais rápido): triplet `x64-windows-static-release`, chave do cache com o triplet e CI semanal na `main`. Ficam de fora o cache NuGet do vcpkg (a chave inclui o hash do compilador, invalidado a cada atualização do MSVC no runner) e o clone raso do vcpkg (economiza menos de 30 s e o modo manifesto precisa do histórico); o FFmpeg enxuto vai para o Backlog | Os dois builds frios de 2026-10-19 levaram 29,5 e 18,9 min no `setup-native`, dos quais 11,3 e 6,8 min compilando o FFmpeg em debug, que nunca é linkado |
| 2026-10-19 | Fase 3.5 | Triplet `x64-windows-static-release` como overlay em `triplets/`, declarado no `vcpkg.json` (`vcpkg-configuration.overlay-triplets`), e não o triplet comunitário direto | O CI clona o `master` do vcpkg; a cópia versionada não muda por baixo e entra no hash da chave do cache |
| 2026-10-19 | Fase 3.5 | CI na `main` toda segunda às 06:00 UTC (`schedule`) e sob demanda (`workflow_dispatch`) | O GitHub apaga um cache sem acesso por 7 dias; o run também pega quebras do runner e avisos novos do RustSec |
| 2026-10-19 | Fase 3.5 | Medições: localmente o build frio do FFmpeg leva 8,0 min (x264 1,1 min, FFmpeg 6,4 min) e o `vcpkg_installed` cai de 428 para 217 MB; o exe de release segue com 85 MB, o `--self-test` passa e o `dumpbin` só lista DLLs do Windows. No CI do PR, `setup-native` frio em 16,3 min (antes 29,5 min num runner de velocidade parecida, −45%), com FFmpeg em 12 min e x264 em 2,4 min; job `checks` em 25,6 min; cache do FFmpeg de 77 para 61 MB. O único build de debug que sobra é o do `pkgconf` do host (15 s) | Critério da Fase 3.5 |

## Referências verificadas

- [cpal: loopback WASAPI](https://github.com/RustAudio/cpal/blob/master/src/host/wasapi/device.rs): `AUDCLNT_STREAMFLAGS_LOOPBACK` em stream de entrada aberto em dispositivo de saída.
- [ffmpeg-sys-next: build.rs](https://github.com/zmwangx/rust-ffmpeg-sys/blob/master/build.rs): vcpkg ou `FFMPEG_DIR`, feature `static`, bindgen.
- [ffmpeg-next no crates.io](https://crates.io/crates/ffmpeg-next) e [README do rust-ffmpeg](https://github.com/zmwangx/rust-ffmpeg): versão 9.0.0 (ago/2026) e modo manutenção.
- [vcpkg: port do ffmpeg](https://github.com/microsoft/vcpkg/tree/master/ports/ffmpeg): FFmpeg 9.0.2; features `x264`, `gpl` e `nvcodec`; NVENC carregado em runtime, sem CUDA SDK.
- [whisper-rs 0.16](https://docs.rs/whisper-rs/latest/whisper_rs/): features, VAD, callbacks e `new_from_buffer_with_params`.
- [Guia de build estático do FFmpeg no Windows](https://docs.rs/crate/ez-ffmpeg/0.17.0/source/docs/INSTALL.md): triplet `x64-windows-static-md`.
