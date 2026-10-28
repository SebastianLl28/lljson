# SPEC.md — jsonlens

## Visión
Herramienta nativa para Linux que une lo que hoy está separado: diff semántico de JSON
(como `jd`), vista visual lado a lado (como Meld) y formateo/transformación (como DevToys).
Offline, arranque instantáneo y fluida con archivos enormes (hasta 1 millón de nodos)
sin congelarse.

## Stack
- **Lenguaje:** Rust, edición 2024. Toolchain mínima: **Rust 1.92** (la exige `gtk4` 0.11).
- **UI:** GTK4 + libadwaita (gtk4-rs). Plan B si la UI se complica demasiado: egui.
- **Parseo:** serde_json o simd-json, o parser propio si hace falta para construir el arena
  directamente y soportar el modo tolerante. Se elige la opción que cumpla las metas de
  rendimiento (decisión a tomar y justificar con benchmarks en la Fase 1).
- **Hash de subárboles:** xxh3 (`xxhash-rust`), determinista y rápido.
  `ahash` para tablas hash internas (p. ej. emparejar elementos de arrays por clave).
- **Lectura de archivos grandes:** memmap2.
- **Filtro jq (post-MVP):** jaq (implementación de jq en Rust puro).
- **Benchmarks:** criterion.
- **Workspace de Cargo con tres crates:**
  - `crates/core` → librería: parser, árbol, hash, formateador, diff. Sin dependencias de UI.
  - `crates/cli`  → binario de terminal (`jsonlens-cli`) que usa `core` (pruebas y scripts).
  - `crates/app`  → app GTK4 (`jsonlens`) que usa `core`.

## Versiones de dependencias (fijadas)
Verificadas en crates.io el **2026-10-28** (última versión estable de cada crate).
Se declaran en `[workspace.dependencies]` y `Cargo.lock` se versiona.
No se actualizan sin aprobación.

| Crate | Versión | Uso | Crate(s) |
|---|---|---|---|
| `gtk4` | 0.11.5 | UI (requiere Rust ≥ 1.92) | app |
| `libadwaita` | 0.9.2 | Widgets y estilo GNOME (depende de gtk4 ^0.11) | app |
| `glib` | 0.22.10 | Bucle principal, subclases de GObject | app |
| `gio` | 0.22.10 | `GListModel`, archivos, portapapeles | app |
| `serde` | 1.0.229 | Serialización (opciones, config) | core, cli |
| `serde_json` | 1.0.154 | Parser candidato / referencia en tests | core |
| `simd-json` | 0.19.0 | Parser candidato (publicada el 2026-10-27, muy reciente: validar en Fase 1) | core |
| `xxhash-rust` | 0.8.19 | Hash xxh3 de subárboles (feature `xxh3`) | core |
| `ahash` | 0.8.12 | HashMap rápido para emparejar por clave | core |
| `memmap2` | 0.9.12 | Lectura mapeada de archivos grandes | core |
| `thiserror` | 2.0.21 | Tipos de error en `core` | core |
| `anyhow` | 1.0.104 | Manejo de errores en binarios | cli, app |
| `clap` | 4.6.7 | Argumentos de línea de comandos (feature `derive`) | cli, app |
| `async-channel` | 2.5.0 | Canal hilo de fondo → hilo de UI | app |
| `criterion` | 0.8.2 | Benchmarks (dev-dependency) | core |
| `jaq-core` | 3.1.2 | Filtro jq (post-MVP) | core |
| `jaq-std` | 3.0.4 | Librería estándar de jaq (post-MVP) | core |
| `jaq-json` | 2.0.3 | Valores JSON para jaq (post-MVP) | core |
| `egui` / `eframe` | 0.37.0 | Solo si se activa el plan B | app |

**Librerías del sistema detectadas** (CachyOS/Arch): `gtk4` 4.24.1 y `libadwaita` 1.10.0.
Features de compatibilidad recomendadas: `gtk4` con `v4_18` y `libadwaita` con `v1_7` como
mínimo (subirlas solo si se necesita una API concreta), para que el Flatpak funcione sobre
runtimes de GNOME no tan recientes.

## Funcionalidades del MVP

### Comparación
- **Diff semántico:** ignora el orden de las claves y los espacios; compara estructura y valores.
- **Vista lado a lado** con scroll sincronizado y filas alineadas (filas vacías de relleno
  donde un lado tiene contenido que el otro no).
- **Colores:** añadido (verde), eliminado (rojo), modificado (amarillo), resaltando el valor exacto.
- **Cambio de tipo** (`"1"` vs `1`) detectado como diferencia explícita.
- **Arrays** comparados por índice o por una clave configurable (p. ej. `"id"`).
- **Ignorar rutas/claves** configurables (p. ej. `timestamp`, `updatedAt`).
- **Navegar entre diferencias:** siguiente/anterior con atajos de teclado.
- **Colapsar secciones iguales;** expandir/colapsar nodos.
- **Barra resumen:** N añadidos, N eliminados, N modificados.

### Entrada
- Pegar texto en cada panel, abrir archivos, arrastrar y soltar.
- Abrir desde terminal: `jsonlens a.json b.json` (y `-` para leer de stdin).
- Comparar el portapapeles contra un archivo.
- **Modo tolerante:** acepta comas finales y comentarios (JSONC), con opción "autocorregir".
- Errores de sintaxis claros con línea y columna.

### Formateo
- Formatear (indentación configurable), minificar, ordenar claves.

### Utilidades
- Copiar la ruta de un nodo (p. ej. `$.users[3].email`).
- Copiar el valor de un nodo.
- Tema claro/oscuro siguiendo al sistema.

## Funcionalidades post-MVP
- Filtro jq/JSONPath aplicado a ambos lados antes de comparar (funcionalidad estrella).
- Exportar el diff como JSON Patch (RFC 6902) y Merge Patch (RFC 7396).
- Desescapar JSON embebido en strings.
- NDJSON y JSON5.
- Conversión JSON ↔ YAML ↔ TOML ↔ CSV.
- Pasar cambios de un lado al otro (merge).
- Recarga automática si el archivo cambia en disco.
- Paleta de comandos (Ctrl+K).
- Tolerancia numérica configurable (`1.0` vs `1.0000001`).
- Integración "Comparar con…" en Nautilus.

## Arquitectura obligatoria (clave para el rendimiento)
1. **Árbol en arena:** los nodos viven en un `Vec<Node>` plano y se referencian por índice
   (`u32`), sin `Box`/`Rc` por nodo. Los strings se almacenan como rangos sobre el buffer
   original siempre que sea posible (solo se copian si contienen escapes).
2. **Hash por subárbol calculado durante el parseo.** En el diff, si dos subárboles tienen
   el mismo hash se consideran iguales y no se recorren. El hash de un objeto no depende del
   orden de sus claves.
3. **Parseo y diff en hilos de fondo.** El hilo de la UI **nunca** se bloquea.
   La UI muestra el JSON formateado de inmediato y aplica los colores del diff al terminar,
   con indicador de progreso si la operación tarda más de ~150 ms.
4. **Vista virtualizada:** la UI consume una lista plana de filas visibles
   `[nodo, profundidad, estado_diff, fila_alineada]`. Se usa `GtkListView`/`GtkColumnView`
   con un `GListModel` propio. **Prohibido crear un widget por nodo.**
5. **Texto bajo demanda:** el texto de cada fila se genera al mostrarse (en el `bind` de la
   fábrica de filas), no todo por adelantado.
6. **Colapsar/expandir** modifica solo la lista plana; no reconstruye el árbol.
7. **Archivos grandes** se leen con `memmap2`.

### Esquema de datos de referencia (orientativo)
```text
Document { source: Buffer (mmap o String), nodes: Vec<Node>, root: NodeId }
Node     { kind: Null|Bool|Number|String|Array|Object, parent: NodeId,
           first_child: NodeId, next_sibling: NodeId, key: Option<Span>,
           value: Span, hash: u64, line: u32 }
DiffResult { left_status: Vec<DiffStatus>, right_status: Vec<DiffStatus>,
             pairs: Vec<(Option<NodeId>, Option<NodeId>)>, summary: Summary }
Row      { node: NodeId, depth: u16, status: DiffStatus, aligned_row: u32 }
```

## Metas de rendimiento
Referencia: archivo de **1 millón de nodos**, máquina normal (4+ núcleos, SSD, 16 GB RAM).

| Métrica | Meta |
|---|---|
| Arranque de la app | < 300 ms |
| Parseo | < 1 s |
| Diff de dos archivos casi idénticos | < 500 ms |
| Scroll | fluido a 60 fps |
| Memoria con ambos archivos cargados | < 500 MB |

Las mediciones se hacen con criterion (core) y con mediciones manuales documentadas (app).
