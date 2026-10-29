# BENCHMARKS.md — Resultados contra las metas de rendimiento

Medido el **2026-10-28** (Fase 4).

- **Máquina:** Intel Core i7-1355U (10 núcleos / 12 hilos), 38 GB de RAM, NVMe (Micron),
  CachyOS (kernel 7.3.0), Rust 1.99.1, perfil `release`.
- **Archivo:** generado con `jsonlens-cli generate` (semilla 1): array de 83 334 registros,
  **1 000 009 nodos**, 20,7 MiB indentado con 2 espacios (13,7 MiB minificado). Incluye
  escapes, unicode, números decimales, `null` y anidamiento de 3 niveles.
  La variante "10 cambios" se genera con `--modify 10`.
- **Cómo reproducir:**
  ```sh
  cargo bench -p jsonlens-core                  # criterion: parse, hash, diff, format
  cargo bench -p jsonlens-core --bench memory   # RSS con los dos documentos cargados
  ```

## Resumen frente a las metas de SPEC.md

| Métrica | Meta | Resultado | Estado |
|---|---|---|---|
| Parseo (1M nodos) | < 1 s | **53 ms** (estricto), 57 ms (tolerante) | ✅ ~19× de margen |
| Diff de dos archivos casi idénticos | < 500 ms | **8,2 ms** solo el diff; **72 ms** de punta a punta (leer + parsear ambos + diff) | ✅ ~7× de margen de punta a punta |
| Memoria con ambos archivos cargados | < 500 MB | **158 MiB** de RSS (2 × 1M nodos + diff) | ✅ ~3× de margen |
| Arranque de la app | < 300 ms | **~210 ms** hasta tener las 1 250 012 filas en la vista (primer frame a ~70–100 ms) | ✅ |
| Scroll a 60 fps | fluido | **59–59,4 fps** de media, p99 ≤ 33 ms (monitor de 60 Hz) | ✅ |
| Memoria de la app con ambos archivos | < 500 MB | **~240–270 MiB** de RSS | ✅ |

## Detalle (criterion, mediana)

### Parseo — `benches/parse.rs`
| Opción | Tiempo | Rendimiento |
|---|---|---|
| jsonlens estricto | 52,9 ms | 391 MiB/s |
| jsonlens tolerante | 57,3 ms | 361 MiB/s |
| serde_json → `Value` | 199,9 ms | 103 MiB/s |
| simd-json → `BorrowedValue` | 73,1 ms | 283 MiB/s |
| simd-json → tape | 26,9 ms | 768 MiB/s |

Incluye la construcción del arena y el hash de todos los subárboles. La decisión de la
Fase 1 (parser propio) se mantiene: solo el tape de simd-json es más rápido y no da arena,
modo tolerante ni rangos sobre el texto original.

### Hash — `benches/hash.rs`
| Caso | Tiempo |
|---|---|
| Recalcular todos los hashes sobre el arena (estimación) | 28,1 ms |
| Comparar dos documentos por el hash de la raíz | < 1 ns |

El hash se calcula dentro del parser y no se puede aislar; la estimación repite el mismo
trabajo de xxh3 sobre el arena ya construido (sin la forma canónica de los números), así
que es una cota aproximada de su peso dentro de los ~53 ms del parseo.

### Diff — `benches/diff.rs`
| Caso | Cambios encontrados | Tiempo |
|---|---|---|
| Idénticos | 0 | 0,33 ms |
| 10 cambios (por índice) | 10 modificados | 8,2 ms |
| 10 cambios mixtos, arrays por clave `id` | 1 añadido, 2 eliminados, 7 modificados (2 de tipo) | 15,8 ms |
| 10 cambios ignorando `score` y `$[*].address.zip` | 10 modificados | 9,6 ms |
| 1 registro insertado al principio, por índice | 649 918 (todo se desplaza) | 84,1 ms |
| 1 registro insertado al principio, por clave `id` | 1 añadido | 13,6 ms |

El peor caso razonable (una inserción que desplaza todos los índices) sigue a 6× de la meta.
Con `--array-key id` el mismo caso se reduce al único cambio real.

### Formateo — `benches/format.rs` (entrada minificada)
| Operación | Tiempo | Rendimiento |
|---|---|---|
| Formatear (2 espacios) | 30,6 ms | 448 MiB/s |
| Minificar | 22,1 ms | 618 MiB/s |
| Ordenar claves | 54,2 ms | 252 MiB/s |

### Memoria — `benches/memory.rs`
| Etapa | RSS |
|---|---|
| Inicial | 24,3 MiB |
| Tras abrir (mmap) y parsear los dos documentos | 157,7 MiB (+133,4) |
| Tras el diff | 157,7 MiB (+0,0) |

Teórico: 41,3 MiB de texto (mmap) + 91,6 MiB de arena (`Node` = 48 bytes × 2 000 018 nodos).
El diff no sube el RSS porque reutiliza memoria que el parser ya liberó. Queda por medir
la memoria de la UI (lista plana de filas y widgets visibles) en las fases 6 y 7.

### De punta a punta con la CLI (10 ejecuciones, mediana)
| Comando | Tiempo | Pico de RSS |
|---|---|---|
| `diff -q a.json b.json` (10 cambios) | 72 ms | ~150 MiB |
| `diff -q --array-key id a.json b.json` | 83 ms | ~153 MiB |
| `format a.json` | 79 ms | ~153 MiB |
| `minify a.json` | 71 ms | ~153 MiB |

Incluye el arranque del proceso, la lectura con mmap, el parseo (los dos documentos en
paralelo en `diff`) y la escritura de la salida a `/dev/null`.

## App GTK con 1 millón de nodos (Fase 6)

Medido el **2026-10-28**, perfil `release` con la feature `devtools`, Wayland, monitor de 60 Hz,
misma máquina que arriba. Archivos: `generate` (semilla 1) y `generate --modify 10 --type-change 5`,
21,7 MB cada uno, 1 250 012 filas alineadas, altura de contenido 23,75 M px.

```sh
jsonlens-cli generate > a.json
jsonlens-cli generate --modify 10 --type-change 5 > b.json
JSONLENS_TRACE=1 JSONLENS_SCROLL_TEST=10 cargo run --release -p jsonlens --features devtools -- a.json b.json
```

| Métrica | Resultado (3 ejecuciones) |
|---|---|
| Primer frame | 71–101 ms |
| Documentos cargados (cada uno, en paralelo) | 52–62 ms |
| Filas listas en la vista | 208–224 ms desde el arranque |
| Scroll continuo (5 s, 3000 px/s) | 58,4–59,4 fps; p50/p95 16,7 ms; 2–3 frames > 25 ms de ~295; máx 50–100 ms |
| Saltos aleatorios (5 s) | 59,0–59,2 fps; p95 16,7 ms; p99 31–33 ms; 4–5 frames > 25 ms; máx 33 ms |
| RSS al final | 242–273 MiB (VmHWM igual) |

Los frames lentos aislados (≈1 %) quedan por investigar; el p95 está en el intervalo de
refresco del monitor. Limitación conocida: **expandir la raíz tarda ~54 ms en el hilo de la UI**
(`benches/rows.rs`), por encima de los 16 ms de un frame.

### Navegación entre diferencias — `benches/rows.rs` (Fase 7)
| Caso | Tiempo |
|---|---|
| `find_change`, peor caso: recorrer las 1 250 012 filas sin hallar ninguna | 3,0 ms |

Se ejecuta en el hilo de la UI (meta: < 16 ms por frame).

### Ocultar secciones iguales (Fase 7)
Con los dos archivos de 1 M de nodos (`--modify 10 --type-change 5`), al activar el interruptor
las filas pasan de 1 250 012 a **83 456**. La reconstrucción (diff + filas) corre en un hilo de
fondo y tarda ~190 ms hasta que la vista se actualiza; el hilo de la UI solo recibe el resultado.
