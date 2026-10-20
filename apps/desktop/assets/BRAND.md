# AgentTidy desktop identity

Direction: graphite, porcelain silver, a single emerald alignment accent.
The folded archival monogram refers to ordered agent sessions. The mark is
shared by the native desktop icon, favicon, workspace header and startup page.

## Assets and exports

- Source: `app-icon.png` (transparent, square generated artwork).
- Web: `../public/brand/agenttidy-icon.png` (512px), `../public/favicon.png`.
- Desktop: `../src-tauri/icons/`, including ICNS and ICO.
- Rebuild: `python3 apps/desktop/assets/generate_app_icon.py` from repo root.
  The script uses the installed Tauri CLI and copies only desktop outputs.
  Mobile assets are not part of this redesign. No Python image library needed.

The source is stored in the project so export is reproducible without requesting
another AI image. Keep alpha around the rounded tile; do not flatten against black.

## Generation provenance

Created on 2026-10-20 with the built-in `image_gen` tool, not the CLI fallback.
The following is the final generation prompt (no reference images):

> Use case: logo-brand. Asset type: final macOS desktop app icon for AgentTidy, a premium local AI agent session and storage manager. Create ONE production-ready square app icon, not a presentation or contact sheet. The icon is a beautifully crafted soft rounded-square dark graphite tile, inset on a fully transparent canvas with comfortable macOS icon optical padding (tile occupies about 84% of canvas), very subtle natural drop shadow. Inside the tile, a striking original abstract monogram suggesting three ordered archival layers converging into an architectural letter A / precise folded ribbon: sculptural satin silver/porcelain white solid surfaces, clean generous negative space, distinctive silhouette with a central dark cutout, sophisticated restrained isometric depth. A tiny emerald green alignment tab on one edge signals order/completion. Industrial precision, immaculate bevels, tactile but restrained material, luxury professional creative utility. Dark charcoal near #111515, cool silver #e7ecea, accent #52d5a4. The symbol should occupy around 65% of tile with thick simple shapes and remain recognizable at 32 pixels. Soft upper left studio lighting. Perfectly centered, front-facing tile, no tilted mockup. No text, no letters printed, no wordmark, no robot, no broom, no sparkle, no brains, no purple gradients, no blue AI orb, no busy detail. Outside the rounded square must be genuinely transparent. High resolution crisp edges, polished app-store quality.

## Startup behavior

`StartupScreen` displays real read-only scan state. It has no percentage meter
or minimum display duration. Successful first scan triggers a 420ms exit
transition (immediate with reduced motion); failure keeps retry and diagnostic
access. Background browsing is always available. The workspace icon opens the
same composition as an About view without starting another scan. Reopening the
brand view never resets a plan, selection or execution lifecycle.

`index.html` paints a matching static shell before JavaScript, and the Tauri
window background matches it. The normal workspace keeps its light appearance.
