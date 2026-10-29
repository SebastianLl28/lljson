# CLAUDE.md — Reglas de trabajo para lljson

Este archivo define cómo se trabaja en este repositorio. Es obligatorio en cada sesión.

## Al iniciar cada sesión
1. Lee `SPEC.md` (qué es la app, arquitectura obligatoria, metas de rendimiento y versiones fijadas).
2. Lee `TASKS.md` e identifica la fase en curso y la siguiente tarea sin marcar.
3. Resume brevemente en qué punto está el proyecto antes de empezar a trabajar.

## Flujo de trabajo
- Trabaja **una sola fase a la vez**. No empieces la siguiente fase sin mi confirmación explícita.
- Dentro de una fase, avanza tarea por tarea.
- Al terminar cada tarea, deben pasar sin errores:
  - `cargo build --workspace`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Marca la tarea como completada en `TASKS.md` (`[ ]` → `[x]`).
- Haz **commit de git al terminar cada tarea**, con un mensaje descriptivo
  (formato sugerido: `fase N: descripción breve`).
- Si una tarea no se puede completar, no la marques: explica qué falta y por qué.

## Arquitectura
- Respeta la sección **"Arquitectura obligatoria"** de `SPEC.md` al pie de la letra.
- Si crees que algo de la arquitectura o del stack debe cambiar, **pregúntame primero**
  y explica el motivo con datos (benchmarks, limitaciones de la API, etc.).
- `crates/core` no puede depender de GTK ni de ninguna librería de UI.
- **Nunca bloquees el hilo de la UI.** Parseo, diff, formateo de archivos grandes y lectura
  de disco van en hilos de fondo; los resultados vuelven al hilo principal por canal
  (`async-channel` + `glib::MainContext::spawn_local`).
- **Nunca crees un widget por nodo JSON.** La UI usa siempre vistas virtualizadas
  (`GtkListView`/`GtkColumnView` con un `GListModel` propio).

## Calidad del código
- Sin `unwrap()` ni `expect()` en código de la app ni de la librería. Los errores se propagan
  con `Result` y, en la app, se muestran al usuario (toast, banner o diálogo).
  Excepción permitida: tests, benchmarks y casos cuya imposibilidad esté demostrada
  y documentada con un comentario.
- Sin `unsafe` salvo que sea imprescindible (p. ej. `memmap2`), y siempre con un comentario
  `// SAFETY:` que justifique la invariante.
- Todo código nuevo en `core` lleva tests unitarios, incluidos casos extremos.

## Dependencias
- Usa **solo** las versiones fijadas en la tabla de dependencias de `SPEC.md`.
- Declara las dependencias en `[workspace.dependencies]` del `Cargo.toml` raíz y
  referéncialas desde cada crate con `workspace = true`. `Cargo.lock` se versiona.
- No añadas dependencias nuevas sin preguntarme.
- Si una API de gtk4-rs / libadwaita no compila, consulta la documentación de **esa versión
  exacta** en docs.rs (p. ej. `https://docs.rs/gtk4/0.11.5/gtk4/`) antes de adivinar.

## Rendimiento
- Las metas de rendimiento de `SPEC.md` son requisitos, no sugerencias.
- Si una meta no se cumple, avísame con los números del benchmark (criterion), la máquina
  y el archivo usado, y propone opciones antes de cambiar la arquitectura.
