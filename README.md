# 08 — Browser Engine

**Stack:** Rust · `html5ever` (study then replace) · custom CSS parser (`cssparser` crate) · `tiny-skia` or `wgpu` for painting · QuickJS (via `rquickjs`) for JS · `hyper` for network · Servo as reference

## Full Vision
HTML5 parser, CSS 3 cascade, flexbox + grid, compositor, JS engine integration, network stack, service workers, top-1000 sites render correctly.

## MVP (1 weekend)
Parse HTML → DOM → block layout → paint text+boxes to window. Supports `color`, `margin`, `display:block`.

## Milestones
- **M1 (Week 2):** HTML parser + DOM tree + CSS parser + selector matching
- **M2 (Week 5):** Block + inline layout + painting to canvas/window
- **M3 (Week 10):** Flexbox + broader CSS property set + fonts (harfbuzz)
- **M4 (Week 16):** JS engine (QuickJS) + DOM bindings + events
- **M5 (Week 24):** Network stack (HTTPS) + renders 10 real websites

## Key References
- "Let's build a browser engine!" (Matt Brubeck)
- Servo architecture
- CSS 2.1 spec
