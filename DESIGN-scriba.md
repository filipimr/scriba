---
version: 1.0
name: scriba
description: Design system do Scriba, app local que grava e transcreve. Linguagem herdada da Apple — UI que recua, um único acento azul para tudo que é clicável, pílulas como gramática de ação, títulos com tracking negativo — aplicada a um utilitário de trabalho com marca monocromática em traço fino.

colors:
  brand-ink: "#000000"
  primary: "#0066cc"
  primary-focus: "#0071e3"
  primary-on-dark: "#2997ff"
  on-primary: "#ffffff"
  link: "#0066cc"
  ink: "#1d1d1f"
  ink-secondary: "#6e6e73"
  ink-tertiary: "#86868b"
  on-dark: "#ffffff"
  on-dark-secondary: "#a1a1a6"
  canvas: "#ffffff"
  canvas-parchment: "#f5f5f7"
  surface-pearl: "#fafafc"
  surface-dark: "#1d1d1f"
  surface-black: "#000000"
  hairline: "#e0e0e0"
  divider-soft: "#f0f0f0"
  status-success: "#116329"
  status-success-bg: "#e6f4ea"
  status-warning: "#7d4e00"
  status-warning-bg: "#fff4d6"
  status-error: "#d70015"
  status-error-bg: "#fde8ea"
  recording: "#ff3b30"
  recording-bg: "#ffebe9"
  danger: "#d70015"

themes:
  default: system
  light: {}
  dark:
    primary: "#0071e3"
    primary-focus: "#409cff"
    link: "#2997ff"
    ink: "#f5f5f7"
    ink-secondary: "#a1a1a6"
    ink-tertiary: "#6e6e73"
    canvas: "#1c1c1e"
    canvas-parchment: "#000000"
    surface-pearl: "#2c2c2e"
    surface-dark: "#2c2c2e"
    hairline: "#38383a"
    divider-soft: "#2c2c2e"
    status-success: "#30d158"
    status-success-bg: "#0f2e1a"
    status-warning: "#ffd60a"
    status-warning-bg: "#332a00"
    status-error: "#ff6961"
    status-error-bg: "#3a1214"
    recording: "#ff453a"
    recording-bg: "#3a1412"

typography:
  display:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 40px
    fontWeight: 600
    lineHeight: 1.1
    letterSpacing: -0.4px
  title:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 28px
    fontWeight: 600
    lineHeight: 1.14
    letterSpacing: -0.3px
  headline:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 21px
    fontWeight: 600
    lineHeight: 1.19
    letterSpacing: -0.2px
  body:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 17px
    fontWeight: 400
    lineHeight: 1.44
    letterSpacing: -0.374px
  body-strong:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 17px
    fontWeight: 600
    lineHeight: 1.24
    letterSpacing: -0.374px
  transcript:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 17px
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: -0.2px
  callout:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 15px
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: -0.24px
  caption:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.43
    letterSpacing: -0.224px
  caption-strong:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 14px
    fontWeight: 600
    lineHeight: 1.29
    letterSpacing: -0.224px
  timestamp:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.43
    letterSpacing: 0
    fontVariantNumeric: tabular-nums
  footnote:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.33
    letterSpacing: -0.12px

rounded:
  none: 0px
  sm: 8px
  md: 12px
  lg: 18px
  icon: 22%
  pill: 9999px

spacing:
  xxs: 4px
  xs: 8px
  sm: 12px
  md: 16px
  lg: 24px
  xl: 32px
  xxl: 48px
  section: 64px

motion:
  press-scale: 0.97
  duration-fast: 150ms
  duration-base: 250ms
  easing: "cubic-bezier(0.25, 0.1, 0.25, 1)"
  recording-pulse: 1.6s

elevation:
  floating: "0 8px 30px rgba(0, 0, 0, 0.12)"
  backdrop: "saturate(180%) blur(20px)"

components:
  app-header:
    backgroundColor: "{colors.surface-black}"
    textColor: "{colors.on-dark}"
    typography: "{typography.caption}"
    height: 52px
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.on-primary}"
    typography: "{typography.body}"
    rounded: "{rounded.pill}"
    padding: 11px 22px
  button-primary-active:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.on-primary}"
    rounded: "{rounded.pill}"
  button-primary-disabled:
    backgroundColor: "{colors.divider-soft}"
    textColor: "{colors.ink-tertiary}"
    rounded: "{rounded.pill}"
  button-secondary:
    backgroundColor: transparent
    textColor: "{colors.link}"
    typography: "{typography.body}"
    rounded: "{rounded.pill}"
    padding: 11px 22px
  button-utility:
    backgroundColor: "{colors.surface-dark}"
    textColor: "{colors.on-dark}"
    typography: "{typography.caption}"
    rounded: "{rounded.sm}"
    padding: 8px 15px
    height: 44px
  button-icon:
    backgroundColor: "{colors.divider-soft}"
    textColor: "{colors.ink}"
    rounded: "{rounded.pill}"
    size: 44px
  button-danger:
    backgroundColor: "{colors.danger}"
    textColor: "{colors.on-primary}"
    typography: "{typography.body}"
    rounded: "{rounded.pill}"
    padding: 11px 22px
  theme-toggle:
    backgroundColor: transparent
    textColor: "{colors.on-dark-secondary}"
    rounded: "{rounded.pill}"
    size: 36px
  text-link:
    backgroundColor: transparent
    textColor: "{colors.link}"
    typography: "{typography.body}"
  text-link-on-dark:
    backgroundColor: transparent
    textColor: "{colors.primary-on-dark}"
    typography: "{typography.body}"
  dropzone:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.ink-secondary}"
    typography: "{typography.headline}"
    rounded: "{rounded.lg}"
    padding: 48px
  dropzone-dragover:
    backgroundColor: "{colors.surface-pearl}"
    textColor: "{colors.link}"
    rounded: "{rounded.lg}"
  job-card:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.ink}"
    typography: "{typography.body-strong}"
    rounded: "{rounded.lg}"
    padding: 24px
  audio-prompt:
    backgroundColor: "{colors.canvas-parchment}"
    textColor: "{colors.ink}"
    typography: "{typography.callout}"
    rounded: "{rounded.md}"
    padding: 12px 16px
  progress-bar:
    backgroundColor: "{colors.divider-soft}"
    fillColor: "{colors.primary}"
    rounded: "{rounded.pill}"
    height: 4px
  status-badge:
    backgroundColor: "{colors.divider-soft}"
    textColor: "{colors.ink}"
    typography: "{typography.caption-strong}"
    rounded: "{rounded.pill}"
    padding: 4px 10px
  status-badge-active:
    backgroundColor: "{colors.divider-soft}"
    textColor: "{colors.link}"
    rounded: "{rounded.pill}"
  status-badge-success:
    backgroundColor: "{colors.status-success-bg}"
    textColor: "{colors.status-success}"
    rounded: "{rounded.pill}"
  status-badge-warning:
    backgroundColor: "{colors.status-warning-bg}"
    textColor: "{colors.status-warning}"
    rounded: "{rounded.pill}"
  status-badge-error:
    backgroundColor: "{colors.status-error-bg}"
    textColor: "{colors.status-error}"
    rounded: "{rounded.pill}"
  option-toggle:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.ink}"
    typography: "{typography.callout}"
    rounded: "{rounded.pill}"
    padding: 10px 16px
  option-toggle-selected:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.ink}"
    rounded: "{rounded.pill}"
  device-select:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.ink}"
    typography: "{typography.callout}"
    rounded: "{rounded.md}"
    padding: 10px 14px
    height: 44px
  record-panel:
    backgroundColor: "{colors.surface-dark}"
    textColor: "{colors.on-dark}"
    typography: "{typography.title}"
    rounded: "{rounded.lg}"
    padding: 48px
  record-button:
    backgroundColor: "{colors.canvas}"
    iconColor: "{colors.recording}"
    rounded: "{rounded.pill}"
    size: 72px
  record-button-recording:
    backgroundColor: "{colors.recording}"
    iconColor: "{colors.on-primary}"
    rounded: "{rounded.pill}"
    size: 72px
  record-button-disabled:
    backgroundColor: "{colors.canvas}"
    iconColor: "{colors.recording}"
    rounded: "{rounded.pill}"
    size: 72px
    opacity: 0.5
  level-meter:
    backgroundColor: "{colors.on-dark-secondary}"
    fillColor: "{colors.on-dark}"
    typography: "{typography.footnote}"
    rounded: "{rounded.pill}"
    height: 4px
  recording-bar:
    backgroundColor: "{colors.canvas-parchment}"
    textColor: "{colors.ink}"
    typography: "{typography.body}"
    height: 64px
    padding: 12px 32px
  transcript-view:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.ink}"
    typography: "{typography.transcript}"
    rounded: "{rounded.lg}"
    maxWidth: 720px
    padding: 48px
  transcript-timestamp:
    textColor: "{colors.ink-secondary}"
    typography: "{typography.timestamp}"
  transcript-speaker:
    textColor: "{colors.ink}"
    typography: "{typography.caption-strong}"
  alert-inline:
    backgroundColor: "{colors.status-error-bg}"
    textColor: "{colors.status-error}"
    typography: "{typography.callout}"
    rounded: "{rounded.md}"
    padding: 12px 16px
  toast:
    backgroundColor: "{colors.surface-dark}"
    textColor: "{colors.on-dark}"
    typography: "{typography.callout}"
    rounded: "{rounded.md}"
    padding: 12px 20px
  confirm-dialog:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.ink}"
    typography: "{typography.headline}"
    rounded: "{rounded.lg}"
    padding: 32px
  empty-state:
    backgroundColor: transparent
    textColor: "{colors.ink-secondary}"
    typography: "{typography.callout}"
    padding: 64px
  older-note:
    backgroundColor: transparent
    textColor: "{colors.ink-secondary}"
    typography: "{typography.callout}"
  footer:
    backgroundColor: "{colors.canvas-parchment}"
    textColor: "{colors.ink-secondary}"
    typography: "{typography.footnote}"
    padding: 32px
---

## Visão geral

O Scriba é um utilitário: o usuário chega com um arquivo ou uma reunião e quer sair com o texto. O design existe para sumir. A linguagem vem da Apple — superfícies claras, um único azul para tudo que é clicável, pílulas como forma de ação, títulos com tracking negativo — mas o vocabulário é de app, não de vitrine: em vez de tiles de produto, há uma área de soltar arquivos, cards de job, um painel de gravação e um leitor de transcrição.

A marca é monocromática. O logo é preto e branco, desenhado em traço fino de pontas arredondadas, e essa mesma linha guia os ícones. A cor fica reservada para duas funções: **azul = ação** e **cores de status = estado**.

**Características-chave:**

- UI que recua: nada de gradientes, texturas ou sombras decorativas.
- Um acento interativo (`{colors.primary}` #0066cc); cores de status nunca viram botão ou link.
- Duas gramáticas de botão: pílula para ações (`{rounded.pill}`) e retângulo compacto para utilidades (`{rounded.sm}`).
- Inter embutida no binário, com tracking negativo nos títulos e corpo em 17 px.
- Ritmo de superfícies: header preto → área de trabalho em parchment → cards brancos com hairline. O painel escuro marca o "modo gravação".
- Leitura longa é o produto final: a transcrição tem tipografia própria, com entrelinha generosa e marcações de tempo tabulares.
- Dois temas, claro e escuro, que o usuário alterna no header. O padrão segue o tema do Windows.

## Marca e logo

O arquivo de origem está em `docs/brand/logo-original.jpg`. As versões vetoriais ficam em `docs/brand/`: `logo.svg` (quadrado preto, caneta branca), `logo-white.svg` (quadrado branco, caneta preta, para fundos escuros como o header), `logo-mark.svg` (só o traço, em `currentColor`) e `logo-16.svg` (versão simplificada para favicon e bandeja).

- **Ícone do app:** quadrado preto (`{colors.brand-ink}`) com cantos `{rounded.icon}` (~22%) e caneta-tinteiro em traço branco, ligada a uma linha de assinatura com laço.
- **Wordmark:** "Scriba" em Inter 600, tracking −0,02em, ao lado do ícone, na cor do texto da superfície (branco no header, `{colors.ink}` no claro).
- **Área de respiro:** pelo menos 25% da largura do ícone em volta dele.
- **Tamanhos mínimos:** 24 px para o ícone completo. Abaixo disso (favicon e bandeja em 16 px) usa-se a versão simplificada, só a caneta, sem o laço e sem a linha.
- **Cores permitidas:** preto sobre fundo claro, ou branco sobre fundo escuro (o quadrado vira branco e a caneta, preta).
- **Não:** recolorir (nem com o azul de ação), aplicar sombra ou gradiente, distorcer proporções, girar a caneta, contornar o quadrado.

### Ícone do exe e da bandeja

Os `.ico` saem de `cargo xtask icon` a partir dos SVGs de `docs/brand/` e ficam versionados em `assets/brand/`; o `build.rs` os embute no exe. São elementos nativos do Windows, fora dos tokens do YAML.

- **Exe** (`scriba.ico`, de 16 a 256 px): o ícone do app, quadrado preto e caneta branca, que se lê no Explorer claro e no escuro, na barra de tarefas e no Alt+Tab. Até 24 px, a versão simplificada do `logo-16.svg`; de 32 a 64 px, o logo completo com o traço dobrado (4 na grade de 100) para não sumir; acima, o traço original.
- **Bandeja** (`bandeja-escura.ico` e `bandeja-clara.ico`, de 16 a 48 px): só o glifo, sem o quadrado, como os ícones do sistema na área de notificação. Glifo preto na barra clara, branco na barra escura, trocado na hora em que o tema muda. Até 24 px, a caneta do `logo-16.svg` com o traço um pouco mais grosso (1,6 em vez de 1,25).
- **Dica:** "Scriba"; durante a gravação, "Scriba — gravando".
- **Clique** no ícone abre o app no navegador. **Menu** (clique direito): "Abrir o Scriba" (o padrão, em negrito), um separador e "Sair".
- **Janelas nativas** (`MessageBox`, no estilo do sistema):
  - "Sair do Scriba?", só quando algo se perde, com um parágrafo por caso: "A gravação em andamento para agora. O que foi gravado será transcrito quando o Scriba abrir de novo.", "A transcrição de X será cancelada." (ou "As N transcrições na fila serão canceladas.") e "O áudio sem resposta será descartado.", e no fim "Sair mesmo assim?". Botões OK e Cancelar, com o Cancelar como padrão.
  - "O Scriba já está aberto, mas não respondeu." quando a segunda execução não acha a primeira.
  - "Não consegui abrir o Scriba: … Detalhes em …\logs." no erro fatal.
  - No logoff ou desligamento com o app fechando, o Windows mostra o motivo "Finalizando a gravação…".

## Iconografia

- Ícones de linha da biblioteca **Lucide** (licença ISC), embutidos como SVG inline.
- Traço de 1,75 px em grade de 24 px, com pontas e junções arredondadas — o mesmo desenho do logo.
- Cor herdada do texto (`currentColor`). Nunca preenchidos, exceto o círculo do botão de gravar.
- Tamanhos: 16 px junto de caption, 20 px junto de body, 24 px em botões de ícone.
- Nos botões com texto, o ícone vem antes do rótulo: copy em "Copiar texto", download em "Baixar .txt" e "Baixar .md", folder-open em "Abrir pasta"; chevron-left no link "Recentes" do leitor.

## Cores

### Ação

- **Action Blue** (`{colors.primary}` — #0066cc): todo elemento clicável — botões primários, links, opção selecionada. Contraste 5,6:1 sobre branco.
- **Focus Blue** (`{colors.primary-focus}` — #0071e3): anel de foco do teclado (2 px sólido).
- **Sky Blue** (`{colors.primary-on-dark}` — #2997ff): links sobre superfícies escuras, onde o Action Blue some.

### Superfícies

- **Canvas** (`{colors.canvas}` — #ffffff): cards, leitor de transcrição, diálogos.
- **Parchment** (`{colors.canvas-parchment}` — #f5f5f7): fundo da área de trabalho e do rodapé. Faz os cards brancos se destacarem sem borda pesada.
- **Pearl** (`{colors.surface-pearl}` — #fafafc): realce da dropzone ao arrastar um arquivo.
- **Dark** (`{colors.surface-dark}` — #1d1d1f): painel de gravação e toasts.
- **Black** (`{colors.surface-black}` — #000000): só o header, ecoando o quadrado do logo.

### Texto

- **Ink** (`{colors.ink}` — #1d1d1f): títulos e corpo.
- **Ink secondary** (`{colors.ink-secondary}` — #6e6e73): metadados, marcações de tempo, estados vazios. Passa AA sobre branco e parchment.
- **Ink tertiary** (`{colors.ink-tertiary}` — #86868b): só texto desabilitado e não essencial (não passa AA para texto pequeno).
- **On dark** (`{colors.on-dark}` — #ffffff) e **on dark secondary** (`{colors.on-dark-secondary}` — #a1a1a6): texto sobre superfícies escuras.

### Status (exceção documentada)

A regra do acento único vale para **ação**. Estado precisa de cor própria, então o sistema tem uma família de status que nunca é usada em botões ou links — só em badges, alertas, ícones de estado e no indicador de gravação. Cada cor tem um fundo suave para badges e alertas.

| Token | Cor | Fundo | Uso |
| --- | --- | --- | --- |
| `{colors.status-success}` | #116329 | #e6f4ea | Job concluído |
| `{colors.status-warning}` | #7d4e00 | #fff4d6 | Avisos (ex.: microfone Bluetooth, pouco espaço em disco) |
| `{colors.status-error}` | #d70015 | #fde8ea | Falhas e validação |
| `{colors.recording}` | #ff3b30 | #ffebe9 | Gravação em andamento (ponto pulsante, botão de gravar) |

O vermelho de gravação (#ff3b30) não tem contraste para texto pequeno; use-o só como preenchimento de ícone ou círculo, nunca como cor de texto.

### Hairlines

- **Hairline** (`{colors.hairline}` — #e0e0e0): borda de 1 px em cards, selects e dropzone (tracejada).
- **Divider soft** (`{colors.divider-soft}` — #f0f0f0): trilho de barras de progresso, fundo de badge neutro e de botões de ícone.

Sem gradientes, em lugar nenhum.

## Temas

O Scriba tem tema **claro** e **escuro**. O usuário escolhe entre três modos no `{component.theme-toggle}` do header:

- **Sistema** (padrão): segue a preferência do Windows via `prefers-color-scheme` e troca sozinho quando ela muda.
- **Claro** e **Escuro**: fixam o tema.

### Como funciona

- Os componentes nunca mudam por tema; só os **valores** das cores mudam. Os tokens em `colors:` são o tema claro, e `themes.dark` sobrescreve só as cores que diferem. Tipografia, espaçamento, raios e movimento são os mesmos nos dois temas.
- No CSS, cada cor vira uma variável (`--color-primary`). `:root` recebe o tema claro; `[data-theme="dark"]` recebe as sobrescritas; o modo Sistema aplica o escuro dentro de `@media (prefers-color-scheme: dark)`.
- A escolha fica salva na configuração do app e o servidor já renderiza o `data-theme` no `<html>`, então a página nunca pisca no tema errado ao abrir.
- A troca é instantânea, sem transição de cores.

### Tema escuro

| Papel | Claro | Escuro |
| --- | --- | --- |
| Fundo da página (`canvas-parchment`) | #f5f5f7 | #000000 |
| Cards e leitor (`canvas`) | #ffffff | #1c1c1e |
| Painel de gravação, toasts (`surface-dark`) | #1d1d1f | #2c2c2e |
| Texto (`ink`) | #1d1d1f | #f5f5f7 |
| Texto secundário (`ink-secondary`) | #6e6e73 | #a1a1a6 |
| Hairline | #e0e0e0 | #38383a |
| Botão primário (`primary`) | #0066cc | #0071e3 |
| Links e texto azul (`link`) | #0066cc | #2997ff |
| Sucesso / aviso / erro | #116329 / #7d4e00 / #d70015 | #30d158 / #ffd60a / #ff6961 |
| Gravando (`recording`) | #ff3b30 | #ff453a |

Regras do tema escuro:

- **Azul de botão e azul de texto se separam.** No escuro, o preenchimento do botão (`primary` #0071e3, 4,7:1 com texto branco) é mais escuro que o azul de links (`link` #2997ff, 5,6:1 sobre o card). Por isso, todo texto azul usa `{colors.link}`, e só preenchimentos usam `{colors.primary}`.
- **Profundidade pela luz, não pela sombra.** A página é preta, os cards são um degrau mais claros (#1c1c1e) e o painel de gravação é mais um degrau (#2c2c2e). Quanto mais à frente, mais claro.
- **Header:** no escuro, o header preto encosta na página preta; uma hairline de 1 px em `{colors.hairline}` marca a borda inferior.
- **Logo:** no tema escuro e no header, o ícone usa a versão invertida (quadrado branco, caneta preta) ou a branca sem quadrado — nunca o quadrado preto sobre fundo preto.
- **Cores de status** ficam mais claras e saturadas, com fundos escuros e de baixo brilho.
- **`{colors.danger}`** (#d70015) é o mesmo nos dois temas, porque precisa sustentar texto branco no `button-danger`.
- **Sombra flutuante** (`{elevation.floating}`) quase não aparece sobre preto; no escuro, `toast` e `confirm-dialog` também recebem borda hairline.

## Tipografia

### Família

- **Inter** (variável, licença OFL), embutida no binário, com `font-feature-settings: "ss03"` para aproximar o "a" do SF Pro. O SF Pro não existe no Windows, então não entra na pilha.
- Fallback: `system-ui, sans-serif` (Segoe UI no Windows).
- Números de tempo e duração usam `font-variant-numeric: tabular-nums`, para não "tremerem" enquanto mudam.

### Hierarquia

| Token | Tamanho | Peso | Entrelinha | Tracking | Uso |
| --- | --- | --- | --- | --- | --- |
| `{typography.display}` | 40px | 600 | 1.1 | -0.4px | Título de tela vazia ("Solte um arquivo ou comece a gravar") |
| `{typography.title}` | 28px | 600 | 1.14 | -0.3px | Título da transcrição, timer do painel de gravação |
| `{typography.headline}` | 21px | 600 | 1.19 | -0.2px | Títulos de seção, chamada da dropzone, diálogos |
| `{typography.body}` | 17px | 400 | 1.44 | -0.374px | Texto padrão e botões |
| `{typography.body-strong}` | 17px | 600 | 1.24 | -0.374px | Nome do arquivo no card de job |
| `{typography.transcript}` | 17px | 400 | 1.6 | -0.2px | Texto da transcrição (leitura longa) |
| `{typography.callout}` | 15px | 400 | 1.4 | -0.24px | Opções, selects, alertas, toasts |
| `{typography.caption}` | 14px | 400 | 1.43 | -0.224px | Metadados (tamanho, duração), navegação do header |
| `{typography.caption-strong}` | 14px | 600 | 1.29 | -0.224px | Badges, rótulo do falante ("Você", "Outros") |
| `{typography.timestamp}` | 14px | 400 | 1.43 | 0 | `[hh:mm:ss]` na transcrição, com números tabulares |
| `{typography.footnote}` | 12px | 400 | 1.33 | -0.12px | Rodapé, legendas de medidores |

### Princípios

- **Tracking negativo nos títulos**, como na Apple; nunca abaixo de 12 px.
- **Corpo em 17 px**, não 16. A entrelinha é 1,44 (e não 1,47) porque a Inter tem x-height maior que o SF Pro.
- **A transcrição é o produto**: entrelinha 1,6 e largura máxima de 720 px (~70 caracteres por linha) para leitura confortável de textos longos.
- **Escada de pesos 400 / 600.** Sem 500, sem 700, sem 300.

## Layout

### Espaçamento

- Base de 8 px: `{spacing.xxs}` 4 · `{spacing.xs}` 8 · `{spacing.sm}` 12 · `{spacing.md}` 16 · `{spacing.lg}` 24 · `{spacing.xl}` 32 · `{spacing.xxl}` 48 · `{spacing.section}` 64.
- Padding de card: `{spacing.lg}` (24 px). Espaço entre cards: `{spacing.sm}` (12 px).
- Entre blocos da página: `{spacing.xxl}` (48 px).

### Estrutura da página

- Container central de **980 px**; o leitor de transcrição limita o texto a 720 px.
- De cima para baixo: `app-header` (preto, fixo) → painel de gravação (escuro) e dropzone lado a lado no desktop → lista "Recentes" de `job-card` → rodapé. Uma tela só, sem abas.
- Enquanto grava, a `recording-bar` fica fixa no rodapé da janela, em todas as páginas (início e leitor); a página ganha 64 px embaixo para ela não cobrir o rodapé, e os toasts sobem acima dela.
- Desktop primeiro: o app roda no PC do usuário. Mesmo assim, a página não quebra em janelas estreitas.

### Quebras

| Largura | Mudança |
| --- | --- |
| ≥ 1024px | Gravação e dropzone lado a lado |
| 768–1023px | Gravação acima, dropzone abaixo |
| 480–767px | Padding de card e do leitor cai para 16 px; ações do job-card e do leitor quebram em linha |
| < 480px | Header mostra só o ícone; títulos `display` caem para 28 px |

## Elevação

| Nível | Tratamento | Uso |
| --- | --- | --- |
| Plano | Sem sombra, sem borda | Header, painel de gravação, rodapé |
| Hairline | 1 px `{colors.hairline}` | Cards, selects, dropzone |
| Backdrop | `{elevation.backdrop}` sobre parchment a 80% | `recording-bar` fixa no rodapé da janela |
| Flutuante | `{elevation.floating}` | Só `toast` e `confirm-dialog` |

A hierarquia vem da troca de superfície (parchment → branco, claro → escuro), não de sombras. A única sombra do sistema é a dos elementos que de fato flutuam sobre a página.

## Formas

| Token | Valor | Uso |
| --- | --- | --- |
| `{rounded.sm}` | 8px | Botões utilitários |
| `{rounded.md}` | 12px | Selects, alertas, toasts |
| `{rounded.lg}` | 18px | Cards, dropzone, painel de gravação, diálogos, leitor |
| `{rounded.icon}` | 22% | Ícone do app |
| `{rounded.pill}` | 9999px | Botões de ação, opções, badges, botão de gravar, barras de progresso |

## Movimento

- **Pressionar:** todo botão encolhe para `scale({motion.press-scale})` — a microinteração do sistema.
- **Durações:** `{motion.duration-fast}` (150 ms) para estados e `{motion.duration-base}` (250 ms) para entrada de toasts e diálogos, com `{motion.easing}`.
- **Gravação:** o ponto vermelho da `recording-bar` pulsa (opacidade 1 → 0,4) em ciclo de `{motion.recording-pulse}`. O preenchimento do `level-meter` anima a largura em 100 ms, o intervalo das leituras.
- **Progresso:** a barra anima a largura; nada de spinners indeterminados quando há porcentagem.
- Com `prefers-reduced-motion`, todas as animações são desligadas; o pulso vira um ponto estático.

## Componentes

### Navegação

**`app-header`** — Barra preta (`{colors.surface-black}`) de 52 px, fixa. À esquerda, ícone do app (24 px, versão branca) e wordmark, que levam à página inicial. Na ponta direita, o `{component.theme-toggle}`. Sem abas: gravar e enviar arquivos ficam lado a lado na mesma tela, e a `recording-bar` acompanha a gravação nas outras páginas.

**`theme-toggle`** — Botão de ícone de 36 px no header, que alterna em ciclo Sistema → Claro → Escuro. O ícone mostra o modo atual (monitor, sol, lua), em `{colors.on-dark-secondary}`, e o tooltip e o `aria-label` dizem o nome do modo ("Tema: Sistema"). A troca é aplicada na hora e salva na configuração.

**`recording-bar`** — Aparece fixa no rodapé da janela enquanto há gravação, em todas as páginas (início e leitor), inclusive depois de recarregar. Parchment a 80% com `{elevation.backdrop}`, 64 px de altura, padding 12 × 32 px (16 px nas laterais abaixo de 480 px). À esquerda: ponto `{colors.recording}` pulsante, "Gravando" e o timer em `{typography.timestamp}` `{colors.ink-secondary}`, com números tabulares. À direita: `{component.button-primary}` "Parar" (tooltip "Parar (Ctrl+Shift+R)"), desabilitado enquanto o pedido está a caminho. Some quando a gravação para, pelo Parar ou sozinha.

### Botões

**`button-primary`** — Ação principal: fundo `{colors.primary}`, texto branco em `{typography.body}`, pílula, padding 11 × 22 px. Pressionado: `{component.button-primary-active}` com `scale(0.97)`. Desabilitado: `{component.button-primary-disabled}`. Foco: anel de 2 px em `{colors.primary-focus}` com 2 px de afastamento.

**`button-secondary`** — Segunda ação ao lado de uma primária ("Baixar .txt" e "Baixar .md" ao lado de "Copiar texto" ou "Ver transcrição"): fundo transparente, texto e borda de 1 px em `{colors.link}`, pílula.

**`button-utility`** — Ações compactas e de baixa ênfase dentro de cards e do leitor ("Abrir pasta"): fundo `{colors.surface-dark}`, texto branco em `{typography.caption}`, `{rounded.sm}`, padding 8 × 15 px e 44 px de altura (o alvo mínimo de clique), com ícone de 16 px.

**`button-icon`** — Botão quadrado de 44 px com só um ícone (cancelar, excluir, fechar): fundo `{colors.divider-soft}`, ícone em `{colors.ink}`, pílula. Sempre com `aria-label`.

**`button-danger`** — Só dentro de `confirm-dialog`, para a ação destrutiva confirmada ("Excluir", "Descartar"). Fundo `{colors.danger}`, igual nos dois temas. É a única exceção em que uma cor de status preenche um botão, e só depois que o usuário pediu a ação.

**`text-link`** / **`text-link-on-dark`** — Links no texto, em `{colors.link}` (que se ajusta ao tema) e em `{colors.primary-on-dark}` dentro de superfícies sempre escuras, como o header e o painel de gravação. Sem sublinhado; com ícone, ele vem antes do texto ("‹ Recentes" no leitor), e a área de clique tem 44 px de altura.

### Arquivos e jobs

**`dropzone`** — Área de soltar arquivos: fundo `{colors.canvas}`, borda tracejada de 1,5 px em `{colors.hairline}`, `{rounded.lg}`, padding 48 px. Centro: ícone de upload (24 px), chamada "Solte um vídeo ou áudio aqui" em `{typography.headline}`, linha "ou" e `{component.button-primary}` "Escolher arquivo". Sem opções: todo arquivo extrai o áudio e transcreve.

- **Arrastando:** `{component.dropzone-dragover}` — fundo pearl, borda sólida em `{colors.primary}`, texto azul.
- **Enviando:** a chamada dá lugar ao nome do arquivo e a uma `{component.progress-bar}` do upload.
- **Erro:** `{component.alert-inline}` logo abaixo ("Formato não suportado. Use MP4, MOV, MKV, MP3, M4A, WAV ou OGG.").

**`option-toggle`** — Opção ligável em formato de pílula (um checkbox): fundo branco, borda de 1 px `{colors.hairline}`, `{typography.callout}`. Selecionada (`{component.option-toggle-selected}`): borda de 2 px e ícone de check à esquerda, ambos em `{colors.primary}`. Sem seleção, não há check: a borda fina e a ausência do ícone marcam o estado sem depender só de cor. Desde a etapa 4.4 nenhuma tela o usa (a dropzone perdeu "Transcrever" e "Compactar"); fica no sistema para ajustes futuros.

**`job-card`** — Um job na lista "Recentes": fundo branco, borda hairline, `{rounded.lg}`, padding 24 px.

- Linha 1: ícone do tipo (vídeo, áudio, gravação), nome em `{typography.body-strong}` (truncado com reticências quando não cabe) e `{component.status-badge}` à direita.
- Linha 2: metadados em `{typography.caption}` `{colors.ink-secondary}` (duração · tamanho · data: "47 min · 312 MB · 20/10/2026 14:30"). Sem a duração no arquivo, só tamanho e data.
- À direita do card, um único `{component.button-icon}` de ação, que mantém o foco do teclado quando o card se atualiza.
- Na fila: o badge, o botão de cancelar e, abaixo dos metadados, a estimativa no mesmo estilo: "Cerca de 4 min na GPU", "Menos de 1 min no processador".
- Ativo: etapa atual ("Transcrevendo… 42%") e `{component.progress-bar}`, com o botão de cancelar. Depois do pedido, a etapa vira "Cancelando…" e o botão fica inativo até o pipeline parar.
- Concluído: uma linha própria abaixo dos metadados com `{component.button-primary}` "Ver transcrição", `{component.button-secondary}` "Baixar .txt" e "Baixar .md" e `{component.button-utility}` "Abrir pasta". A linha quebra quando não cabe. Para leitores de tela, cada ação leva o nome do arquivo ("Baixar .txt de reuniao.mp4"). Abaixo dela, o `{component.audio-prompt}` com "Guardar o áudio?".
- Guardando o áudio: o botão de ação vira cancelar ("Cancelar o áudio de reuniao.mp4"), como num job ativo; com a janela "Salvar como" aberta, ele fica inativo.
- Falhou: mensagem em `{component.alert-inline}` e, logo abaixo, `{component.button-secondary}` "Tentar de novo", que devolve o job à fila com o mesmo arquivo.
- Concluído, Falhou ou Cancelado: o botão de ação vira "Remover" (lixeira), que tira o job da lista. Em Falhou e Cancelado, remover só apaga a cópia temporária do envio e não pede confirmação; em Concluído, a pasta da transcrição vai para a Lixeira, depois do `{component.confirm-dialog}`.

**`audio-prompt`** — A pergunta do fim de cada transcrição, dentro do card concluído: fundo `{colors.canvas-parchment}` (a troca de superfície separa a pergunta das ações do resultado, sem borda nem cor), texto `{colors.ink}` em `{typography.callout}`, `{rounded.md}`, padding 12 × 16 px. Estados:

- **Pendente:** "Guardar o áudio?" e, abaixo, dois `{component.button-secondary}`: "Guardar áudio" e "Descartar". Os dois têm o mesmo peso, porque nenhuma resposta é a errada, e levam o nome do arquivo para leitores de tela ("Guardar áudio de reuniao.mp4"). "Descartar" não pede confirmação: a mídia é descartável e o original do usuário nunca é tocado.
- **Escolhendo:** "Escolhendo onde guardar…" enquanto a janela "Salvar como" do Windows está aberta. Cancelar a janela volta à pergunta.
- **Guardando:** "Guardando áudio… 42%" com a `{component.progress-bar}`; depois do pedido de cancelar, "Cancelando…". Cancelar volta à pergunta.
- **Guardado:** "Áudio guardado em C:\…\2026-10-20 1430 - Reunião.m4a" (o caminho quebra em qualquer ponto) e o `{component.button-utility}` "Mostrar no Explorer" (ícone folder-search), que abre a pasta com o arquivo selecionado.
- **Descartado:** o bloco some e o card volta a ter só as ações do resultado. Também é assim numa transcrição do histórico (reaberta de um `job.json`) sem áudio guardado.
- **Expirado:** a pergunta ficou 24 h sem resposta e o áudio foi descartado: só o texto "O áudio foi descartado depois de 24 h sem resposta.", sem botões.
- **Erro:** a mensagem vem num `{component.toast}` ("Não consegui guardar o áudio. Confira o espaço livre e se a pasta permite gravação, e tente de novo.") e a pergunta volta, para tentar de novo ou descartar.

**`progress-bar`** — Trilho de 4 px em `{colors.divider-soft}`, preenchimento `{colors.primary}`, pílula. É o elemento nativo `<progress>`, que já expõe o papel `progressbar` e o valor; a política de segurança do app (CSP) bloqueia a largura por `style` inline.

**`status-badge`** — Pílula pequena em `{typography.caption-strong}`, padding 4 × 10 px.

| Estado | Variante |
| --- | --- |
| Na fila | `{component.status-badge}` (neutro) |
| Processando | `{component.status-badge-active}` (texto em `{colors.link}`, que passa AA nos dois temas) |
| Concluído | `{component.status-badge-success}` |
| Aviso | `{component.status-badge-warning}` |
| Falhou | `{component.status-badge-error}` |
| Cancelado | `{component.status-badge}` (neutro) |

### Gravação

**`record-panel`** — O "modo gravação": card escuro (`{colors.surface-dark}`), `{rounded.lg}`, padding 48 px (16 px abaixo de 768 px), texto branco. Centro: `{component.record-button}` com o rótulo abaixo, timer em `{typography.title}` com números tabulares, e dois `{component.level-meter}` ("Você" e "Outros"). Abaixo, os `{component.device-select}` de microfone e saída, recolhidos num `<details>` "Ajustes". Dica fixa em `{typography.footnote}` `{colors.on-dark-secondary}`: "Use fone de ouvido para não duplicar falas."

- **Parado:** botão "Gravar", timer em 00:00:00, medidores vazios.
- **Gravando:** botão "Parar" (`{component.record-button-recording}`); o timer e os medidores andam ~10 vezes por segundo. Recarregar a página já mostra o painel gravando, com o tempo certo.
- **Desabilitado:** enquanto o Gravar ou o Parar está a caminho do servidor, o botão fica em `{component.record-button-disabled}` (opacidade 0,5), e um segundo clique não faz nada.
- **Erro:** a gravação não começou (sem microfone, microfone bloqueado pela privacidade do Windows, pouco disco): `{component.alert-inline}` abaixo dos medidores, com o que fazer ("Não encontrei um microfone. Conecte um e tente de novo."). O botão volta a "Gravar"; o próximo Gravar limpa o erro.
- **Avisos durante a gravação**, abaixo dos medidores: "Microfone sem sinal" em `{component.status-badge-warning}` depois de 10 s de silêncio no microfone; e, se o computador suspendeu, a nota "O computador suspendeu: o tempo parado ficou de fora." em `{typography.footnote}` `{colors.on-dark-secondary}`.
- **Fim automático** (dispositivo desconectado, disco quase cheio, limite de 12 h): o painel volta a parado, o card da gravação entra no topo de "Recentes" e um `{component.toast}` diz o motivo ("O microfone foi desconectado. A gravação parou e vai ser transcrita.").

**`record-button`** — Círculo de 72 px. Parado: fundo branco com círculo vermelho (`{colors.recording}`) de 28 px dentro e rótulo "Gravar" abaixo. Gravando (`{component.record-button-recording}`): fundo vermelho com quadrado branco de 24 px (parar) e rótulo "Parar". O rótulo vai fora do botão, em branco, porque o vermelho não tem contraste para texto. Pressionado: `scale(0.97)`. Foco: o anel padrão. Desabilitado: `{component.record-button-disabled}`. O tooltip e o `aria-label` dizem a ação ("Gravar (Ctrl+Shift+R)"); o botão mantém o id entre Gravar e Parar, e o foco fica nele.

**`level-meter`** — O `<meter>` nativo (0 a 1), dentro de um `<label>` com o rótulo "Você" ou "Outros" em `{typography.footnote}` à esquerda. Barra horizontal de 4 px: trilho `{colors.on-dark-secondary}` a 30%, preenchimento branco proporcional ao nível RMS em escala de decibéis (−60 dBFS vazio, 0 dBFS cheio, para a fala ocupar o meio da barra). Sem cor de alerta; se o sinal ficar mudo por 10 s, aparece um aviso em `{component.status-badge-warning}` ("Microfone sem sinal").

**`device-select`** — Select nativo estilizado, com o rótulo ("Microfone", "Saída do som") em `{typography.footnote}` `{colors.on-dark-secondary}` acima: fundo branco, borda hairline, `{rounded.md}`, 44 px de altura, `{typography.callout}`.

- A 1ª opção é "Padrão do Windows (nome do dispositivo de agora)", que acompanha as trocas do Windows durante a gravação; depois, cada dispositivo, com o padrão marcado "(padrão)".
- A lista é lida de novo a cada abertura dos Ajustes. A escolha é salva na hora e vale na próxima gravação; um dispositivo salvo que sumiu volta a "Padrão do Windows".
- Durante a gravação, os selects ficam desabilitados (fundo `{colors.divider-soft}`, texto `{colors.ink-tertiary}`), com a nota "Os dispositivos mudam depois que esta gravação parar."
- Microfone Bluetooth (o escolhido, ou o padrão quando é ele): `{component.status-badge-warning}` abaixo do select, "Microfone Bluetooth: a qualidade do som cai durante a gravação." Só avisa; não bloqueia.

**Card de gravação** — O mesmo `{component.job-card}`, com o ícone `mic` e o nome "Gravação de 20/10, 14:30". O Cancelar (na fila ou transcrevendo) e o Remover de uma gravação que falhou abrem o `{component.confirm-dialog}` "Descartar a gravação?", porque as trilhas são apagadas.

### Transcrição

**`transcript-view`** — A tela de leitura (`/jobs/{id}`), lida do `.txt` em disco. Fundo branco, `{rounded.lg}`, coluna de no máximo 720 px, padding 48 px (16 px abaixo de 768 px).

- Acima do título, o `{component.text-link}` "‹ Recentes" (ícone chevron-left), que volta à lista.
- Cabeçalho: título (nome do arquivo ou "Gravação de 16/10, 14:30") em `{typography.title}`, metadados em `{typography.caption}` `{colors.ink-secondary}` (os mesmos do card: "47 min · 312 MB · 20/10/2026 14:30"), e as ações "Copiar texto" (primária), "Baixar .txt" e "Baixar .md" (secundárias), "Abrir pasta" (utilitária).
- Parágrafos em `{typography.transcript}`, cada um começando com a marcação de tempo (`{component.transcript-timestamp}`, "00:00:08", na mesma linha) e, em gravações, o rótulo do falante (`{component.transcript-speaker}`: "Você:" / "Outros:", em peso 600 e `{colors.ink}`). Texto fora do formato (editado à mão) aparece como está, sem tempo e com as quebras de linha.
- O texto é selecionável e copiável como texto puro. "Copiar texto" copia o `.txt` inteiro, com o cabeçalho e as marcações de tempo.
- Ao copiar, um `{component.toast}` confirma: "Texto copiado".
- Sem fala: `{component.empty-state}` "Nenhuma fala encontrada neste arquivo.", com as ações.
- Transcrição ausente (apagada fora do app): `{component.alert-inline}` "Não encontrei a transcrição em C:\…\2026-10-20 1430 - Reunião.", sem as ações. Um job fora da lista (a lista começa vazia quando o app é reaberto): "Não encontrei esta transcrição na lista de Recentes.".

### Feedback

**`alert-inline`** — Mensagem presa a um elemento (erro de upload, falha do job): fundo e texto da família de status, `{rounded.md}`, ícone à esquerda, `{typography.callout}`. Variantes por troca das cores de erro, aviso e sucesso.

**`toast`** — Confirmação passageira no rodapé central: fundo `{colors.surface-dark}`, texto branco em `{typography.callout}`, `{rounded.md}`, padding 12 × 20 px, `{elevation.floating}` (no escuro, com borda hairline, como o diálogo). Some após 3 s. Anunciado com `aria-live="polite"`; um novo substitui o anterior.

- Também mostra o erro de uma ação ("Não encontrei a pasta “2026-10-20 1430 - Reunião”. Ela foi movida ou apagada."), no mesmo estilo e sem cor de status: o texto diz o que houve. O erro fica 6 s, para dar tempo de ler.

**`confirm-dialog`** — Só para ações destrutivas (excluir job e arquivos). Fundo branco, `{rounded.lg}`, padding 32 px, `{elevation.floating}`, sobre um véu preto a 40%, com até 440 px de largura. Título em `{typography.headline}`, texto explicando o que será apagado, `{component.button-secondary}` "Cancelar" e `{component.button-danger}` "Excluir", alinhados à direita. Remover um job concluído: "Excluir a transcrição?" / "A pasta “2026-10-19 1430 - Reunião” vai para a Lixeira.". Cancelar a transcrição de uma gravação (ou remover uma que falhou): "Descartar a gravação?" / "As trilhas gravadas serão apagadas.", com o botão "Descartar".

- Abre com o foco em "Cancelar", a escolha segura.
- Foco preso no diálogo (`<dialog>` modal); Esc cancela.
- Ao fechar, o foco volta ao botão que abriu o diálogo.

**`empty-state`** — Lista "Recentes" sem itens: ícone de 24 px e texto em `{typography.callout}` `{colors.ink-secondary}` ("Nada por aqui ainda. Grave uma reunião ou envie um arquivo."), padding 64 px.

**`older-note`** — O fim da lista "Recentes" quando o histórico tem mais de 50 transcrições: "As mais antigas continuam em Documentos\Scriba." em `{typography.callout}` `{colors.ink-secondary}`, centralizado, com o `{component.button-utility}` "Abrir pasta" (ícone folder-open) ao lado, que abre a pasta das saídas. A linha quebra quando não cabe.

**`footer`** — Parchment, `{typography.footnote}` em `{colors.ink-secondary}`: versão do app e onde a transcrição roda ("Scriba 0.1.0 · Transcrição na GPU: NVIDIA GeForce RTX 3050" ou "· Transcrição no processador"), "Funciona offline · nada sai do seu computador", e link para a pasta de saídas.

## Acessibilidade

- Contraste AA (4,5:1) em todo texto, nos dois temas; `{colors.ink-tertiary}` e `{colors.recording}` nunca carregam texto essencial.
- Anel de foco visível de 2 px em `{colors.primary-focus}` em todo elemento interativo; nunca `outline: none` sem substituto.
- Navegação completa por teclado. Atalho global **Ctrl+Shift+R** para Gravar / Parar em qualquer página, mostrado no tooltip do botão; ele toma o lugar do recarregar do navegador (conferido no Edge, que não o reserva).
- `aria-live="polite"` na entrada e no fim de cada job (não a cada %) e nos toasts; a gravação anuncia só o início ("Gravação iniciada.") e o fim ("Gravação parada. Transcrevendo."); o timer fica fora da região viva.
- Alvos de clique de no mínimo 44 × 44 px.
- Estado nunca depende só de cor: badges têm texto, o botão de gravar troca de forma (círculo → quadrado).

## Voz e microcopy

- Português do Brasil, frases curtas, tratamento por "você".
- Botões com verbos diretos: "Gravar", "Parar", "Escolher arquivo", "Ver transcrição", "Copiar texto", "Baixar .txt", "Baixar .md", "Abrir pasta", "Tentar de novo", "Excluir", "Guardar áudio", "Descartar", "Mostrar no Explorer".
- Confirmações curtas, sem ponto: "Texto copiado".
- Gravação: "Gravando", "Ajustes", "Microfone", "Saída do som", "Padrão do Windows (…)", "(padrão)", "Microfone sem sinal"; avisos para leitores de tela "Gravação iniciada." e "Gravação parada. Transcrevendo."; fim automático "O microfone foi desconectado. A gravação parou e vai ser transcrita.", "A gravação chegou ao limite de 12 h e parou.".
- Histórico e prazo do áudio: "As mais antigas continuam em Documentos\Scriba." e "O áudio foi descartado depois de 24 h sem resposta.".
- Bandeja: "Abrir o Scriba", "Sair", a dica "Scriba — gravando" e a confirmação "Sair do Scriba?" (textos na seção "Ícone do exe e da bandeja").
- Etapas no gerúndio: "Na fila", "Extraindo áudio…", "Transcrevendo… 42%", "Guardando áudio… 42%", "Cancelando…".
- Estimativas arredondadas e sem falsa precisão: "Cerca de 4 min na GPU", "Menos de 1 min no processador". "Processador" no lugar de "CPU".
- Erros dizem o que aconteceu e o que fazer: "Este arquivo não tem áudio. Escolha um vídeo com som." — nunca códigos ou jargão técnico.
- Sem exclamações, sem emojis.

## O que fazer e o que evitar

### Fazer

- Usar `{colors.primary}` no preenchimento de toda ação e `{colors.link}` em todo texto azul.
- Conferir cada tela nos dois temas.
- Usar a família de status só para estado (badges, alertas, indicador de gravação).
- Títulos em `{typography.display}`, `{typography.title}` ou `{typography.headline}`, com tracking negativo.
- Corpo em `{typography.body}` (17 px); transcrição em `{typography.transcript}`.
- Separar áreas por troca de superfície (parchment → branco; claro → escuro) antes de recorrer a bordas.
- `scale(0.97)` como estado pressionado de todo botão.
- Ícones Lucide em traço de 1,75 px, na cor do texto.

### Evitar

- Segunda cor de ação, ou status usado como botão (exceto `button-danger` dentro do diálogo).
- Sombras em cards, botões ou texto; gradientes em qualquer lugar.
- Peso 500, 700 ou 300.
- Spinner sem porcentagem quando o progresso é conhecido.
- Texto vermelho de gravação ou texto em `{colors.ink-tertiary}` com informação essencial.
- Recolorir, sombrear ou distorcer o logo.
- Cor fixa em componente (`#fff`, `black`) em vez de token: quebra o tema escuro.
- Imagens ou ícones com cor embutida; ícones usam `currentColor`.

## Guia de iteração

1. Um componente por vez, referenciando a chave do YAML (`{component.job-card}`).
2. Variantes (`-active`, `-selected`, `-recording`, `-error`) são entradas próprias em `components:`.
3. Sempre `{token.refs}`; nunca hex solto no CSS ou nos templates.
4. Documentar só os estados padrão, pressionado, foco, desabilitado e erro — não hover.
5. Mudou um token no YAML? Atualize a prosa que o descreve, e vice-versa. Os tokens viram CSS pelo `cargo xtask css`. Cor nova precisa de valor nos dois temas (ou explicar por que é igual).
6. Na dúvida sobre ênfase, troque a superfície antes de adicionar cor ou borda.

## Lacunas conhecidas

- **Logo vetorial:** resolvido na Fase 2 (SVGs em `docs/brand/`); o `.ico` do executável e os da bandeja saíram na etapa 7.2 (`cargo xtask icon`).
- **Ilustrações de estado vazio:** a v1 usa apenas ícones Lucide.
- **Ícone da bandeja do Windows:** resolvido na etapa 7.2, com as variantes clara e escura trocadas pelo tema da barra de tarefas, a dica, o menu e as janelas nativas (seção "Ícone do exe e da bandeja"). O menu e as `MessageBox` seguem o estilo do Windows, não os tokens; o contraste do glifo na barra com cor de destaque personalizada não foi conferido.
- **Card concluído e leitor:** resolvido na etapa 4.5: as ações do card concluído ("Ver transcrição", "Baixar .txt", "Baixar .md", "Abrir pasta") numa linha própria, o `transcript-view` com o "‹ Recentes" e os estados sem fala e de transcrição ausente, o `toast` (também para os erros de ação) e o `button-utility` com 44 px de altura. Resolvidos na 4.4: a dropzone sem as caixinhas, o estilo do "Tentar de novo", o backend no rodapé e a estimativa no card, e o texto do `confirm-dialog`.
- **Guardar o áudio:** resolvido na Fase 5 com o `audio-prompt` (YAML e prosa): a pergunta no card concluído, os estados escolhendo, guardando e guardado, o "Mostrar no Explorer" e o erro em toast. A janela "Salvar como" é a nativa do Windows, fora do design system. Na Fase 6, a gravação reaproveita o mesmo componente. Na etapa 7.3, o estado expirado (24 h sem resposta) e a `older-note` no fim do histórico.
- **Gravação ao vivo:** resolvido na etapa 6.3: os estados do `record-panel` e do `record-button` (gravando, desabilitado, erro, avisos, fim automático), o `level-meter` como `<meter>`, o `device-select` com "Padrão do Windows" e o aviso Bluetooth, a `recording-bar` em todas as páginas, o card de gravação e a confirmação do descarte. O Ctrl+Shift+R não é reservado pelo Edge e ficou como estava.
- **Abas do `app-header`:** removidas na Fase 6 (uma tela só, com o painel e a dropzone lado a lado); o "Sair" também saiu, porque o app não tem login. Se uma segunda tela aparecer, as abas voltam a ser desenhadas.
- **Link para a pasta de saídas no rodapé:** descrito em `footer`, ainda não implementado; o "Abrir pasta" de cada job cobre o caso por enquanto.
