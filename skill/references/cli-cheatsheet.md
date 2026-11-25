# texforge — hoja de referencia

Referencia compacta para agentes. El detalle completo vive en `texforge skill`;
esto es la tabla de bolsillo.

## Comandos

| Comando | Qué hace |
|---|---|
| `texforge new <name>` | crea un proyecto |
| `texforge new <name> -t <template>` | crea con otro template |
| `texforge new <name> --values <file>` | rellena placeholders desde TOML/YAML |
| `texforge init` | wizard: crear o migrar |
| `texforge build` | compila a PDF |
| `texforge build --watch` | recompila al guardar |
| `texforge build --reproducible` | PDF byte a byte reproducible |
| `texforge clean` | borra artefactos de build |
| `texforge fmt` | formatea `.tex` y `.bib` |
| `texforge fmt --check` | verifica sin modificar |
| `texforge check` | linter + ortografía |
| `texforge check --deny-warnings` | warnings como errores |
| `texforge spell add <words>` | agrega al diccionario personal |
| `texforge spell add --local <words>` | agrega al proyecto |
| `texforge spell list` | lista el diccionario |
| `texforge spell remove <words>` | quita palabras |
| `texforge outline` | árbol de secciones |
| `texforge outline --json` | árbol en JSON |
| `texforge stats` | palabras por sección |
| `texforge stats --by file` | palabras por archivo |
| `texforge stats --json` | conteo en JSON |
| `texforge preview` | PDF a PNG |
| `texforge preview --page <N>` | una sola página |
| `texforge preview --scale <N>` | escala de rasterizado |
| `texforge preview --out <DIR>` | directorio de salida |
| `texforge pdf text` | texto como lo lee un ATS |
| `texforge pdf info` | páginas, fuentes, metadatos |
| `texforge pdf pages` | sección que abre cada página |
| `texforge pdf check` | fuente vs PDF |
| `texforge doctor` | diagnostica el entorno |
| `texforge config` | wizard de configuración |
| `texforge config list` | muestra la configuración |
| `texforge template list` | instalados + registry |
| `texforge template add <name>` | instala un template |
| `texforge template remove <name>` | desinstala |
| `texforge template validate <name>` | valida compatibilidad |
| `texforge template refresh` | refresca la caché |
| `texforge update` | actualiza el binario |
| `texforge update --check` | solo consulta |
| `texforge uninstall` | borra lo gestionado |
| `texforge skill` | imprime el skill embebido |
| `texforge skill --references` | skill + referencias |
| `texforge skill --list` | nombres de referencias |

## Códigos de salida

| Código | Significado |
|---|---|
| `0` | éxito |
| `1` | el comando falló (lint, build, formato pendiente, actualización disponible) |
| `2` | uso inválido o chequeo de `update` incompleto |

## `project.toml` mínimo

```toml
[documento]
titulo = "mi-tesis"
autor = "Author"
template = "general"

[compilacion]
entry = "main.tex"
bibliografia = "bib/references.bib"

[highlight]
style = "light"   # light | light-mono | dark | dark-mono | ink
```

## Listados

```latex
\begin{code}[lang=rust, file=src/lib.rs, lines=10-40, style=ink, numbers=true]
\end{code}
```

`file=` es relativo a la raíz y no puede escapar del proyecto; `lines=` acepta
`10-40`, `10-`, `-40` o `25` y exige `file=`. Sin `lang=` el lenguaje se infiere
de la extensión.

## Diagramas

```latex
\begin{mermaid}[width=0.8\linewidth, caption=Flujo, pos=t]
flowchart LR
  A --> B
\end{mermaid}
```

`mermaid`, `graphviz` y `d2` comparten las opciones `width`, `caption` y `pos`.
Todo se renderiza en Rust puro: sin Node.js, sin binarios externos.
