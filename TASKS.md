# TASKS.md — Plan de trabajo de jsonlens

Marca cada tarea con `[x]` al completarla (build, test y clippy en verde + commit).
No se avanza de fase sin confirmación.

## Fase 0 — Preparación
- [x] Verificar/instalar dependencias del sistema: toolchain de Rust ≥ 1.92 (rustup),
      gtk4 y libadwaita de desarrollo, pkgconf. Detectar la distribución y darme el comando.
- [x] Crear workspace de Cargo con los crates `core`, `cli` y `app` (edición 2024,
      `[workspace.dependencies]` con las versiones de SPEC.md, `.gitignore`, `Cargo.lock` versionado).
- [x] App "hola mundo" con libadwaita que abra una ventana (confirmar que todo compila).

## Fase 1 — Núcleo: parser y árbol
- [x] Estructura de árbol en arena (`Vec<Node>`, índices `u32`, strings como rangos).
- [x] Parser estricto.
- [x] Modo tolerante (comas finales, comentarios `//` y `/* */`) y opción "autocorregir".
- [x] Errores con línea y columna.
- [x] Elegir estrategia de parseo (serde_json / simd-json / propio) con benchmark preliminar.
- [x] Tests unitarios con casos normales y extremos (vacíos, anidamiento profundo,
      unicode y escapes, números grandes, archivos truncados).

## Fase 2 — Hash y formateador
- [x] Hash por subárbol (xxh3) durante el parseo, independiente del orden de claves.
- [x] Formatear (indentación configurable).
- [x] Minificar.
- [x] Ordenar claves.
- [x] Tests.

## Fase 3 — Motor de diff
- [x] Diff semántico usando hashes para saltar subárboles iguales.
- [x] Arrays por índice y por clave configurable.
- [x] Detección de cambio de tipo.
- [x] Ignorar rutas.
- [x] Resultado del diff con resumen (añadidos/eliminados/modificados).
- [x] Tests.

## Fase 4 — CLI y benchmarks
- [x] CLI: `jsonlens-cli diff a.json b.json`, `format`, `minify`, soporte de stdin (`-`).
- [x] Generador de JSON de prueba (1 millón de nodos, con variantes con pocos cambios).
- [x] Benchmarks con criterion (parseo, hash, diff, formateo, memoria).
- [x] Reportar resultados contra las metas de rendimiento.

## Fase 5 — App: estructura
- [x] Ventana libadwaita con dos paneles lado a lado.
- [ ] Abrir archivos, pegar, arrastrar y soltar.
- [ ] Argumentos de terminal (`jsonlens a.json b.json`, `-` para stdin).
- [ ] Parseo en hilo de fondo con indicador de progreso (> ~150 ms).

## Fase 6 — App: vista virtualizada
- [ ] Lista plana de filas + `GListModel` propio + `GtkListView`.
- [ ] Colores de sintaxis.
- [ ] Expandir/colapsar (modificando solo la lista plana).
- [ ] Scroll sincronizado y filas alineadas entre paneles.
- [ ] Probar con el archivo de 1 millón de nodos (fluidez y memoria).

## Fase 7 — App: diff visual
- [ ] Colores del diff por valor.
- [ ] Barra resumen.
- [ ] Navegación siguiente/anterior diferencia con atajos.
- [ ] Colapsar secciones iguales.

## Fase 8 — App: utilidades del MVP
- [ ] Configurar clave de arrays e ignorar rutas.
- [ ] Copiar ruta y valor de nodo.
- [ ] Comparar portapapeles contra archivo.
- [ ] Botones de formatear/minificar/ordenar.
- [ ] Tema claro/oscuro del sistema.

## Fase 9 — Empaquetado
- [ ] Archivo `.desktop`, ícono, metainfo AppStream.
- [ ] Manifiesto Flatpak y build local de prueba.
- [ ] README con capturas e instrucciones.

## Fase 10 — Post-MVP (en el orden que yo elija)
- [ ] Filtro jq/JSONPath aplicado a ambos lados antes de comparar (jaq).
- [ ] Exportar diff como JSON Patch (RFC 6902) y Merge Patch (RFC 7396).
- [ ] Desescapar JSON embebido en strings.
- [ ] Soporte NDJSON y JSON5.
- [ ] Conversión JSON ↔ YAML ↔ TOML ↔ CSV.
- [ ] Pasar cambios de un lado al otro (merge).
- [ ] Recarga automática si el archivo cambia en disco.
- [ ] Paleta de comandos (Ctrl+K).
- [ ] Tolerancia numérica configurable.
- [ ] Integración "Comparar con…" en Nautilus.
