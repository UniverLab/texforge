---
name: texforge
description: >
  CLI tool for compiling LaTeX documents to PDF without TeX Live, MiKTeX, or any external LaTeX distribution.
  Use this skill whenever the user wants to create a LaTeX project, compile .tex files to PDF, format or lint
  LaTeX documents, spell-check prose, inspect the produced PDF, manage templates, or work with the texforge
  CLI. Activate when the user mentions "texforge new", "texforge build", "texforge fmt", "texforge check",
  "texforge init", "texforge template", "texforge outline", "texforge stats", "texforge preview",
  "texforge pdf", "texforge spell", "texforge doctor", "texforge skill", creating a thesis or academic document,
  compiling LaTeX without installing TeX Live, using tectonic as a LaTeX engine, spell-checking a LaTeX
  document, project.toml configuration, or any workflow involving .tex files and the texforge tool.
---

# texforge CLI

CLI para compilar LaTeX a PDF. Usa [tectonic](https://tectonic-typesetting.github.io/) como motor interno — no requiere TeX Live ni MiKTeX. Tectonic se instala automáticamente en el primer `texforge build`.

Este texto viaja dentro del binario: `texforge skill` imprime exactamente esta versión, así que siempre coincide con la instalada. La copia del skill store de canopy sirve para descubrir la herramienta; ante la duda, el binario es la fuente.

## Instalación

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/UniverLab/texforge/main/scripts/install.sh | sh
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/UniverLab/texforge/main/scripts/install.ps1 | iex
```

```bash
# Via cargo
cargo install texforge
```

Elegí **una** vía y quedate con ella. El script instala en `~/.local/bin`; cargo instala en `~/.cargo/bin`. Si usás las dos quedan dos binarios y cuál corre depende del orden del `PATH` — el síntoma es que actualizás y nada cambia. `texforge doctor` y `which -a texforge` te dicen cuál estás ejecutando.

Tectonic se descarga e instala automáticamente en `~/.texforge/bin/` la primera vez que se ejecuta `texforge build`. No se requiere ningún paso adicional.

## Mapa de comandos

| Comando | Para qué |
|---|---|
| `new` / `init` | crear o migrar un proyecto |
| `build` | compilar a PDF |
| `clean` | borrar artefactos de build |
| `check` | linter + corrector ortográfico, sin compilar |
| `fmt` | formatear `.tex` y `.bib` |
| `outline` | árbol de secciones |
| `stats` | conteo de palabras |
| `preview` | rasterizar el PDF a PNG |
| `pdf` | inspeccionar el PDF ya compilado |
| `spell` | diccionario personal |
| `template` | gestionar templates |
| `config` | configuración global |
| `doctor` | diagnosticar el entorno |
| `update` | actualizar el binario |
| `uninstall` | borrar lo que texforge gestiona |
| `skill` | imprimir este skill |

## Comandos

### `texforge new <name>`

```bash
texforge new mi-tesis                 # template "general" (embebido, funciona offline)
texforge new mi-tesis -t apa-general  # template específico
texforge new mi-tesis --values datos.toml
```

`--values` rellena los placeholders del template desde un archivo `.toml`, `.yaml` o `.yml`; el archivo se copia al proyecto como `values.toml` y los defaults del template se aplican cuando falta una clave. Un TOML con datetime se rechaza con el motivo.

Estructura generada:
```
mi-tesis/
├── project.toml
├── main.tex
├── sections/body.tex
├── bib/references.bib
└── assets/images/
```

`project.toml`:
```toml
[documento]
titulo = "mi-tesis"
autor = "Author"
template = "general"

[compilacion]
entry = "main.tex"
bibliografia = "bib/references.bib"
```

### `texforge init`

Wizard interactivo. Si detecta un `.tex` con `\documentclass`, ofrece migrar el proyecto existente. Si no, guía la creación de uno nuevo pidiendo nombre y template.

```bash
# En un directorio vacío — crea proyecto nuevo
cd mis-documentos/
texforge init

# En un proyecto LaTeX existente — migra
cd mi-tesis-existente/
texforge init
# Detectado: main.tex, refs.bib
# Pide titulo y autor, genera project.toml
```

Después de `init`, todos los comandos (`build`, `fmt`, `check`) funcionan normalmente.

### `texforge build`

```bash
texforge build
```

Compila `main.tex` → PDF en `build/`. Los errores se muestran con archivo, línea y sugerencia — nunca logs crudos de tectonic.

Antes de compilar, intercepta entornos de diagramas y bloques `code`, los renderiza, y trabaja sobre copias en `build/` — los `.tex` originales nunca se modifican.

#### `texforge build --watch`

```bash
texforge build --watch             # recompila al guardar (debounce 2s por defecto)
texforge build --watch --delay 5   # debounce de 5s
```

Sesión persistente con timer en vivo, contador de builds y solo el último resultado (sin acumular logs). Salida coloreada; `Ctrl+C` para salir.

#### Build reproducible y preview en vivo

```bash
texforge build --reproducible            # fija SOURCE_DATE_EPOCH: mismo fuente = mismo PDF byte a byte
texforge build --reproducible 1700000000 # con epoch propio
texforge build --watch --preview         # escribe un PNG a una ruta fija tras cada build
texforge build --watch --preview --preview-page 2 --preview-out /tmp/live.png
```

`--preview` requiere `--watch`. Cualquier visor de imágenes que recargue al cambiar el archivo se convierte en un preview en vivo. `--verbose` imprime cada warning del motor en vez del resumen por archivo.

### `texforge clean`

```bash
texforge clean  # elimina build/ (PDF, logs, diagramas renderizados)
```

### Diagramas embebidos — Mermaid

```latex
% Sin opciones (defaults: width=\linewidth, pos=H, sin caption)
\begin{mermaid}
flowchart LR
  A[Input] --> B[Process] --> C[Output]
\end{mermaid}

% Con opciones
\begin{mermaid}[width=0.6\linewidth, caption=Flujo del sistema, pos=t]
flowchart TD
  X --> Y --> Z
\end{mermaid}
```

Renderizado a PNG en Rust puro — sin browser, sin Node.js, sin Inkscape.

| Opción | Default | Descripción |
|---|---|---|
| `width` | `\linewidth` | Ancho de la imagen |
| `pos` | `H` | Posición del figure (`H`, `t`, `b`, `h`, `p`) |
| `caption` | _(ninguno)_ | Pie de figura |

### Diagramas embebidos — Graphviz / DOT

```latex
\begin{graphviz}[caption=Pipeline, width=0.8\linewidth]
digraph G {
  rankdir=LR
  A -> B -> C
  B -> D
}
\end{graphviz}
```

Mismas opciones que mermaid. Renderizado via `layout-rs` — Rust puro, sin binario `dot` externo.

### Diagramas embebidos — D2

```latex
\begin{d2}[caption=Arquitectura, width=0.7\linewidth]
user -> api: request
api -> db: query
db -> api: rows
api -> user: response
\end{d2}
```

Sintaxis [D2](https://d2lang.com) (contenedores, shapes, `sql_table`, etc.). Mismas opciones que mermaid/graphviz. Renderizado via `d2-little` — Rust puro, sin binario `d2` externo ni Node.js.

### Listados de código

Un entorno `code` se resalta en Rust (syntect) y se reescribe a LaTeX plano: no hace falta `listings.sty`, `minted` ni shell-escape.

```latex
\begin{code}[lang=python, numbers=true, caption={Fibonacci}, label={lst:fib}]
def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)
\end{code}
```

- `file=` toma el bloque de un archivo del proyecto (ruta relativa a la raíz, nunca absoluta ni con `..`); `lines=10-40`, `lines=10-`, `lines=-40` o `lines=25` recorta un rango 1-based inclusivo y solo vale junto a `file=`.
- `style=` elige entre los cinco estilos: `light` (default), `light-mono`, `dark`, `dark-mono` e `ink` (negro puro sobre blanco, para impresión). El estilo también se puede fijar por lenguaje en `[highlight.by_lang]` de `project.toml`.
- `lang=` acepta el nombre o una extensión; sin `lang=` se infiere del archivo. Un lenguaje desconocido no rompe el build: avisa y renderiza monocromo.

```latex
\begin{code}[lang=python, file=scripts/modelo.py, lines=10-40, style=ink]
\end{code}
```

Un archivo faltante, un rango inválido o `lines=` sin `file=` fallan el build nombrando el bloque y la causa; `texforge check` los reporta sin compilar.

### `texforge fmt [--check]`

```bash
texforge fmt           # formatea en lugar
texforge fmt --check   # solo verifica, útil en CI
```

Formateador determinista (estilo `cargo fmt`): re-indenta `.tex` por nivel de anidamiento — entornos (`\begin`/`\end`) **y** llaves sin cerrar — recortando espacios finales y colapsando líneas en blanco repetidas. No reflowea párrafos ni toca verbatim/lstlisting/minted.

También formatea `.bib`: un campo por línea, indentación de 2 espacios, `=` alineados, tipos y campos en minúscula, coma final. Si no puede parsear con seguridad, deja el archivo intacto.

### `texforge check`

Linter estático más corrector ortográfico — valida sin compilar.

```bash
texforge check                   # ERRORS + WARNINGS
texforge check --deny-warnings   # los warnings también hacen fallar (CI)
```

**Referencias rotas** (`ERROR`): `\input` faltantes, imágenes `\includegraphics` no encontradas, archivos `\lstinputlisting` o `\inputminted` ausentes, `\cite` no definidas en el `.bib`, pares `\ref`/`\label` rotos y entornos sin cerrar.

**Divergencia silenciosa** — construcciones que compilan y producen algo distinto de lo que el autor quiso:

| Regla | Dispara con | NO dispara con | Severidad |
|---|---|---|---|
| `~` como "aproximadamente" | `~60M`, `~5 años` | `Figura~\ref{}`, `Dr.~Smith` | warning |
| `%` sin escapar | `50%` | `% un comentario real` | warning |
| `...` literal | tres puntos en prosa | `\ldots`, `\dots` | warning |
| comillas rectas | `"texto"` | comillas tipográficas | warning |
| guión como rango | `2020-2024` | `well-written` | warning |
| `&`, `#`, `_`, `$` sin escapar | fuera de math y tablas | dentro de math o tablas | **error** |

Solo los `ERROR` cambian el código de salida (a menos que uses `--deny-warnings`).

#### Ortografía

`check` reporta `Unknown word` como warning. El idioma se resuelve, de mayor a menor prioridad:

1. Lo que declare el documento: `\usepackage[spanish]{babel}` o `polyglossia`
2. El default del usuario: `texforge config language`
3. `english`

Si el documento y la config no coinciden, gana el documento y `check` lo avisa nombrando los dos idiomas — el override nunca es silencioso.

Los diccionarios viven en `~/.texforge/dicts/` y se descargan solos la primera vez. El par Hunspell del español expande afijos, así que reconoce `soluciones` a partir de `solución` sin almacenarla. Si no hay diccionario para el idioma resuelto, el corrector se salta con un mensaje que lo nombra — nunca corrige contra el idioma equivocado en silencio.

### `texforge spell`

Diccionario personal. Las palabras que agregues se aceptan en `check` sin editar archivos a mano.

```bash
texforge spell add docker cqrs webflux    # al diccionario personal (global, por defecto)
texforge spell add --local nombredetesis  # solo para este proyecto
texforge spell list                       # ver las palabras del alcance
texforge spell remove cqrs
```

**El alcance por defecto es global** (`~/.texforge/spell-words`), porque casi todo lo que uno agrega es cierto de la persona y no de un documento. `--local` es la excepción; escribe en `spell-whitelist.txt` o `.texforge/spell-words` — el que ya exista. `--global` sigue aceptándose como forma explícita del default. `--local` fuera de un proyecto **falla** en vez de caer al global. Los dos alcances se **suman**. Los archivos son texto plano, una palabra por línea, con `#` para comentarios; agregar es siempre *append*.

### `texforge outline`

```bash
texforge outline          # árbol de secciones
texforge outline --json   # para consumir desde un script
```

Resuelve los títulos a texto legible: quita el marcado (`\textit`, `\href`, líderes de puntos) y desescapa los especiales, así que `\subsection*{\textit{Fundador \& Lead Engineer}}` sale como `Fundador & Lead Engineer`.

### `texforge stats`

```bash
texforge stats                # palabras por sección
texforge stats --by file      # palabras por archivo .tex
texforge stats --json
```

Cuenta prosa, no marcado — los comandos, los entornos verbatim y los argumentos que no son texto no suman.

### `texforge preview`

```bash
texforge preview                          # todas las páginas a <proyecto>/preview
texforge preview --page 2                 # solo la página 2
texforge preview --scale 1.5 --out /tmp/p # a 1.5 px por punto PDF
```

Rasteriza el PDF ya compilado a PNG, en Rust puro — útil para revisar el resultado sin abrir un visor y para que un agente pueda *ver* la página.

### `texforge pdf`

Inspecciona el PDF ya compilado. Cuatro vistas:

```bash
texforge pdf text          # el texto como lo ve un lector o un ATS
texforge pdf text --raw    # sin normalizar las ligaduras
texforge pdf info          # páginas, fuentes (¿embebidas?), metadatos
texforge pdf pages         # qué sección abre cada página (diff-friendly)
texforge pdf check         # las palabras significativas del fuente ¿están en el PDF?
```

`pdf text` normaliza ligaduras (`ﬁ` → `fi`) y rejunta palabras partidas por guión. `pdf pages` da una línea por página, pensado para diffear entre builds. `pdf check` avisa de palabras que no sobrevivieron a la compilación. El truco `Artif{}icial` (grupo vacío para romper la ligadura `fi`) se entiende correctamente, también en el corrector ortográfico.

### `texforge doctor`

```bash
texforge doctor
```

Reporta el estado **verificado** del entorno gestionado, no lo que la config afirma: Tectonic (presencia y versión), la caché y su tamaño, las fuentes disponibles, los diccionarios instalados y si el directorio actual es un proyecto texforge. Es lo primero que hay que correr cuando algo no funciona y no está claro por qué.

### `texforge config`

Gestiona la configuración global (`~/.texforge/`), usada para rellenar placeholders al crear proyectos y como default de idioma para el corrector.

```bash
texforge config                      # wizard interactivo
texforge config list                 # muestra toda la configuración actual
texforge config name                 # muestra el valor de una clave
texforge config name "Jane Doe"      # asigna un valor
```

Claves disponibles: `name`, `email`, `institution`, `language`. `language` es solo el **default**: un documento que declara su idioma con `babel` o `polyglossia` gana sobre esta clave.

### `texforge template`

```bash
texforge template list               # instalados + disponibles en el registry remoto (por defecto)
texforge template list --local       # solo los instalados localmente (sin consultar el registry)
texforge template add apa-general    # descarga desde el registry
texforge template add ./mi-template  # instala una carpeta local
texforge template add mi-template --force  # reemplaza una instalada
texforge template remove apa-general
texforge template validate apa-general
texforge template refresh            # refresca la caché (por TTL)
texforge template source add acme https://github.com/acme/texforge-templates.git  # agrega un registry extra (--ref, --no-check)
texforge template source list        # oficial primero (default) + configurados (--json para JSON)
texforge template source remove acme # quita el source (los templates instalados quedan)
```

`template add` acepta un nombre del registry o una ruta a una carpeta local. Para conocer los templates disponibles, **siempre consultá la CLI** — no asumas una lista fija, el registry remoto se actualiza por separado. Cada entrada muestra nombre y descripción; usá ese nombre con `texforge new -t <nombre>` o `texforge template add <nombre>`.

### `texforge update`

```bash
texforge update          # busca la última release estable y pregunta antes de instalar (default: NO)
texforge update --check  # solo informa: no descarga ni toca nada
texforge update --yes    # instala sin preguntar
```

Códigos de salida de `update`: `0` = al día (o actualización instalada / rechazada / binario gestionado por cargo), `1` = `--check` encontró una actualización (o falló la instalación tras un chequeo exitoso), `2` = el chequeo no se pudo completar (red, DNS, TLS, HTTP ≥ 400 o respuesta ilegible; la causa va a stderr).

### `texforge uninstall`

```bash
texforge uninstall                       # borra lo que texforge gestiona bajo ~/.texforge
texforge uninstall --yes                 # sin confirmación
texforge uninstall --dry-run             # muestra el plan sin borrar
texforge uninstall --include-spell-words # borra también tu diccionario personal (se preserva por defecto)
```

El binario nunca se elimina. El diccionario personal contiene tu escritura y se preserva salvo con `--include-spell-words`.

### `texforge skill`

```bash
texforge skill               # imprime este SKILL.md, tal cual
texforge skill --references  # además imprime los archivos de referencia, cada uno precedido por === references/<nombre> ===
texforge skill --list        # lista los nombres de las referencias, uno por línea
```

El texto está embebido en el binario en tiempo de build: no se instala ni se sincroniza aparte, y siempre coincide con la versión instalada. La línea `This skill was printed by texforge v...` dice qué versión lo imprimió. stdout lleva solo el skill; los errores van a stderr. `--references` y `--list` son mutuamente excluyentes.

## Contributing back

When you notice a texforge bug, or a defect in an official template (wrong output, crash, broken placeholder, misleading error), tell the user and suggest opening an issue: in `UniverLab/texforge` for the tool itself, or in `UniverLab/texforge-templates` for a template defect. Draft the issue for the user to review — a title, the output of `texforge --version`, the exact command run, expected versus observed behaviour, and a minimal reproduction — and leave it to them. Never open the issue yourself, never push or publish anything on your own, and never include private document content; every suggestion ends with the user's decision.

When the user has produced an interesting document (a layout, class or structure others could reuse), suggest contributing it as a template to `UniverLab/texforge-templates`. Mention that placeholders are written as `{{name}}` and that `texforge template add <local-folder>` installs a folder locally to test it first. The user decides whether to contribute.

If the user wants their own templates collection, offer to create their own templates repository — but only conditionally: if `ghscaff` offers a texforge templates boilerplate (check `ghscaff --help` or its docs), offer to scaffold the repository with it; if not, say that a repository of templates is any GitHub repo with one folder per template and a `template.toml` each, and let the user decide. Mention `texforge template source add <name> <repo>` as the way to use such a repository when available in the installed version (check `texforge template --help`).

## Códigos de salida

| Código | Significado |
|---|---|
| `0` | Éxito: el comando hizo lo que pedía |
| `1` | Fallo del comando: errores de lint en `check`, fallo de `build`, `fmt --check` con archivos a cambiar, `update --check` con actualización disponible, o cualquier error con su causa en stderr |
| `2` | Error de uso: flags o argumentos inválidos (clap), o el chequeo de `update` no se pudo completar |

## Flujo típico — proyecto nuevo

```bash
texforge new mi-documento
cd mi-documento
texforge check    # detectar errores antes de compilar
texforge fmt      # formatear
texforge build    # compilar a PDF
```

## Flujo típico — proyecto existente

```bash
cd mi-proyecto-latex/
texforge init     # genera project.toml detectando entry y bib
texforge check
texforge build
```

## Flujo típico — revisar el resultado

```bash
texforge build
texforge pdf info     # ¿fuentes embebidas? ¿metadatos correctos?
texforge pdf text     # ¿lo lee un ATS?
texforge pdf pages    # ¿dónde cae cada sección?
texforge preview      # verlo
```

## Si `texforge build` falla

1. Correr `texforge check` primero — resuelve la mayoría de errores sin compilar
2. Si es el primer build, tectonic se está descargando — verificar conexión a internet
3. Para errores de sintaxis LaTeX, el output indica archivo y línea exacta
4. Si nada de eso explica el fallo, `texforge doctor` — dice si el entorno está en pie

## Si `texforge check` reporta ruido

- **Cientos de `Unknown word`**: probablemente el idioma resuelto no es el del documento. La primera línea de la salida dice cuál está usando. Declaralo con `\usepackage[spanish]{babel}` o ajustá `texforge config language`.
- **Nombres propios y jerga**: `texforge spell add <palabras>`, una vez, y valen en todos tus proyectos.
- **Un warning que es correcto y molesto**: no hay silenciado por regla; las reglas de divergencia silenciosa están hechas para no disparar en el uso legítimo. Si una lo hace, es un bug del linter y vale reportarlo.
