# AGENTS.md — Guía para agentes de desarrollo

Proyecto: `slint-layer-shell` — plataforma Slint custom sobre Wayland layer-shell.
Toda la infraestructura (conexión wayland, event loop, globals, Skia) es **compartida**
entre todas las ventanas. No la dupliques por ventana bajo ninguna circunstancia.

> **Regla de oro:** si vas a tocar el pipeline de renderizado, lee primero la
> sección 2 completa. La mayoría de los bugs históricos de este crate fueron
> "ventana en blanco" causados por violar una de las reglas R1–R5.

---

## 1. Arquitectura

- **Un solo** `Connection` wayland, un **solo** `EventLoop` de calloop y un
  **solo** `SkiaSharedContext`, viviendo en `AppData` dentro del thread-local `APP`.
- `ensure_app()` inicializa todo una vez; es idempotente.
- `SLINT_EVENTS` (cola de tareas) y `SLINT_EVENTFD` (fd de despertar) son
  thread-locals **separados de `APP`**: `proxy_handles()` se llama desde
  `set_platform()` mientras `APP` está prestado — no puede tocar `APP`.
- Flujo del bucle principal (`run_inner`):
  1. `timeout = TimerList::next_timeout()`
  2. `event_loop.dispatch(timeout)`
  3. `data.sweep(qh)` → pinta ventanas con trabajo pendiente
  4. **flush**: `dispatch(Some(Duration::ZERO))` → ejecuta idles encolados
     durante el sweep (calloop corre los idles al FINAL de cada dispatch;
     sin este flush, hide/show esperan hasta el próximo despertar real)
  5. segundo `sweep` barato + `handler.on_call()`

---

## 2. ⚠️ REGLAS DE RENDERIZADO (crítico)

### R1 — `needs_redraw` se consume UNA sola vez
- El gate de `render_window` consulta con `.get()`. **Solo** `draw()` /
  `draw_if_needed()` hace `needs_redraw.replace(false)` y pinta.
- Si un gate consume el flag antes, skia nunca pinta y se adjunta un buffer
  recién creado (memoria en ceros = transparente). Síntoma: *"separa espacio
  pero no se ve nada"*.
- Patrón correcto (`render_window`):
  ```rust
  let first_configure = inner.first_configure.get();
  if !first_configure && !inner.adapter.needs_redraw.get() { return; }
  inner.adapter.draw_if_needed();      // consume + pinta
  inner.first_configure.set(false);
  // …damage/attach/frame/commit…
  ```

### R2 — attach solo tras configure vigente
- Per wlr-layer-shell: todo commit que cambia configuración de capa
  (`set_size`, margins, anchor, `exclusive_zone`, interactividad de teclado,
  remapeo tras `hide`) obliga a esperar un nuevo `configure` antes de
  adjuntar buffers.
- Por eso esos métodos ponen `configured.set(false)`; el handler de
  `configure` restaura `configured=true` + `sweep()`.
- Síntomas de violación:
  - `wl_display@1.error: "layerSurface was not configured, but a buffer was attached"`
  - Tras ese error el socket queda legible para siempre → busy-spin 100% CPU.
- Excepciones que NO requieren gating (estado de superficie, no de capa):
  input/opaque regions, viewport, attach(None) de `hide`.

### R3 — buffer age correcto
- En `with_buffer` (skia_non_docs.rs): buffer fresco → `age=0`
  (**repaint completo**); resto → `age=1` (**repaint parcial**).
- Semántica exacta en i-slint-renderer-skia `lib.rs`: `age==0` usa dirty
  region = ventana completa; `age>=1` repinta solo el historial de dirty
  regions. Un buffer nuevo reportando `age>=1` = contenido basura visible.
- `refresh_buffer()` crea slot NUEVO en cada cambio de escala; debe dejar
  `primary_slot` apuntando al mismo slot del `Buffer` que se guarda en
  `inner.buffer`. Canvas y buffer adjunto deben ser SIEMPRE la misma memoria.

### R4 — frame callbacks solo con trabajo pendiente
- `animating = timers pendientes || needs_redraw`. Solo entonces
  `surface.frame(...)` + commit con daño.
- Pedir frames perpetuamente (código viejo) = ciclo infinito de repaints.
  El objetivo del proyecto es **idle ≈ 0% CPU**: no lo rompas.

### R5 — primer frame intacto
- Estado inicial obligatorio: `first_configure=true`, `needs_redraw=true`,
  `configured=false`. El primer attach debe ocurrir únicamente después de
  (configure recibido) ∧ (skia pintó realmente).
- Al depurar renders invisibles, sospecha primero: flag consumido dos veces,
  attach sin configure, o canvas/buffer desincronizados (en ese orden).

---

## 3. Recetas de debugging

```bash
# Trazar protocolo completo (buscar errores y orden de eventos)
WAYLAND_DEBUG=1 cargo run --example demo 2> wl.log
grep "Protocol error" wl.log

# Contadores temporales: patrón eprintln tras variable de entorno
if std::env::var_os("SLINT_LS_DEBUG_SPIN").is_some() { eprintln!(…); }
# ELIMINAR toda instrumentación temporal antes del commit

# Verificar idle CPU (muestrear tras 8s, cuando ya no hay actividad)
cargo build --release --example demo
DEMO_AUTOTEST=1 ./target/release/examples/demo & sleep 8
ps -o pid,stat,%cpu,time -p $(pgrep -x demo)

# Si gira al 100% sin logs: stack con gdb
gdb -p $(pgrep -x demo) -batch -ex bt
```

Interpretación rápida:
| Síntoma | Causa probable |
|---|---|
| Ventana en blanco desde el inicio | R1 (flag doble-consumido) |
| En blanco tras hide/show | R2 (attach pre-configure) |
| Contenido basura/girado | R3 (age/canvas-buffer desincronizados) |
| 100% CPU estable | socket muerto por error de protocolo (ver R2) o flush faltante |

---

## 4. Verificación obligatoria antes de commit

1. `cargo clippy --all-targets` — cero warnings nuevos.
2. Demo **visible** (confirmación humana: el agente no ve la pantalla).
3. `DEMO_AUTOTEST=1`: hide/show inmediatos, sin `Protocol error` en stderr.
4. CPU idle ~0% tras 8 segundos.

---

## 5. Commits

- Conventional commits: tipo en inglés, cuerpo en español (≤72 cols).
- Cambios internos/rendimiento = `perf`; `feat` solo si la API pública
  (`windows!`, `run_windows!`, `WindowConf`, `spawn`, `WinHandle`) cambió.

---

## 6. Mapa del código

| Archivo | Responsabilidad |
|---|---|
| `src/lib.rs` | API pública: `windows!`, `run_windows!`, `run_event_loop` |
| `src/configure.rs` | `WindowConf` + builder (width/height/anchor/margins/layer…) |
| `src/event_macros.rs` | Macro `windows!` → genera `<Comp>Wl::spawn/hide/toggle…` |
| `src/wayland_adapter.rs` | `AppData`/`SharedStates`, `ensure_app`, `create_window`, `WaylandWindow`/`WinHandle`, `render_window`, `sweep`, handlers de capa/escala |
| `src/wayland_adapter/win_impl.rs` | Input: pointer/keyboard → `WindowEvent` de slint |
| `src/wayland_adapter/way_helper.rs` | `set_config`, `PointerState`+cursores, registro del source eventfd |
| `src/wayland_adapter/viewporter.rs` | Escalado por viewport |
| `src/wayland_adapter/fractional_scaling.rs` | wp_fractional_scale_v1 |
| `src/skia_non_docs.rs` | `SkiaSoftwareBufferReal` (RenderBuffer, age), `SkiaWindowAdapter` |
| `src/slint_adapter.rs` | `SlintPlatform`, registro `ADAPTERS`, proxy de eventos slint→calloop |
| `examples/demo.rs` | Barra + panel flotante; `DEMO_AUTOTEST=1` prueba scripted |

Dependencias pinneadas: `slint`/`i-slint-core`/`i-slint-renderer-skia` 1.17,
`smithay-client-toolkit` 0.20 (calloop vía sus reexports — no añadir calloop
directo). Al actualizar slint/sctk, re-verificar R2–R4 contra las fuentes del
crate (`~/.cargo/registry/src/.../i-slint-renderer-skia-*/lib.rs`).

---

## 7. Trampas conocidas

- `LoopHandle`/`WinHandle` **no son Send**: closures hacia el loop deben
  quedar en el hilo principal. Para programar acciones usa
  `slint::Timer::single_shot`, no threads + `invoke_from_event_loop`.
- `SlotPool::create_buffer` asigna slot nuevo en cada llamada (el pool
  crece); no llamarlo en caliente, solo en cambios de escala/tamaño.
- El timer de slint usa ms enteros (`i_slint_core::animations::Instant`);
  calcular timeouts manualmente, no esperar `Duration` nativo.
