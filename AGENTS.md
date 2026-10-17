# Archive Helper — Agent Guidelines & Conventions

These instructions apply to all AI coding agents working in this repository.

---

## 1. Performance & Benchmarking Guidelines
* **Performance Files Location**: Any files created to measure, profile, benchmark, or improve performance (including benchmark CLI runners, profiling scripts, measurement tools, benchmark result datasets, and visualization chart generators) **must be placed in the `.benchmark/` directory**.
* **Git Cleanliness**: The `.benchmark/` directory is ignored by Git in `.gitignore` to keep the production repository clean and free of transient test or benchmark artifacts.

---

## 2. UI & Design System Conventions
* **Design Parity**: The desktop GUI (`crates/archive-helper-gui`) must maintain a strict **1:1 visual match** with the original WPF layout:
  * **Window Outer Rect**: `670 × 355 px` (Inner Client Area: `654 × 316 px`).
  * **Background Color**: `#161616` with Windows 11/10 DWM Immersive Dark caption title bar.
  * **Drop Cards**: Two landscape cards (`20px` padding, `18px` horizontal gap), `#202020` background, `2px #383838` border, `10px` corner radius.
  * **Utility Button Cluster**: 4 buttons (`36 × 36 px`, `7px` radius) at bottom-right: `Unit Mode (MiB/GiB)`, `Admin Shield`, `Thumbtack (Always on Top)`, and `Exit`.
  * **Browser Context Menu**: Default Chromium/WebView2 right-click context menus (`Back`, `Reload`, `Inspect`) are strictly disabled.
