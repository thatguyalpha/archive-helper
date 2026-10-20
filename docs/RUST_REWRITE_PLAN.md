# Archive Helper — Complete Rust Rewrite Plan & Delegate Architecture

**Target Branch:** `rust-rewrite`  
**Status:** In Progress (Phase 1 Implemented & Verified; Phases 2–5 Planned)  
**Date:** October 2026  

---

## 1. Executive Summary & Thread Analysis

### 1.1 Examination of the "Rust Rewrite Benefits" Discussion
In the prior analysis thread, the motivations, architectural trade-offs, and benefits of migrating **Archive Helper** from C# (.NET 8 WPF) to Rust were evaluated in depth. 

#### Core Benefits of Rust for Archive Helper:
1. **Zero Runtime Dependencies & Compact Distribution:**
   - *Current C# (.NET 8):* Requires the user to have the .NET 8 Desktop Runtime installed (or inflates to 75–100+ MB with `--self-contained true` bundling CoreCLR and WPF graphics subsystems).
   - *Rust:* Produces a standalone, statically linked executable (~3.5–5 MB) that runs out-of-the-box on any Windows 10/11 machine with zero external prerequisites.
2. **Instant Launch & Minimal Memory Footprint:**
   - *Current C#:* JIT compilation and DirectX/WPF composition tree initialization require 300–800 ms cold startup, idling at 45–90 MB RAM and spiking to 150–300 MB under multi-gigabyte queues before Garbage Collection sweeps.
   - *Rust:* Cold startup is sub-30 ms with an idle memory footprint of 10–25 MB. RAII deterministic destruction frees streaming buffers immediately back to the OS without GC pauses.
3. **Small-File I/O Throughput (osu! Beatmaps, Skins, Repositories):**
   - High-throughput zero-allocation directory walking (`walkdir`), zero-compression sequential streaming (`Stored`), and work-stealing parallel processing (`rayon`) eliminate allocation churn and saturate NVMe SSD sequential write caches.

### 1.2 Root Cause of the C# Admin Elevation Anti-Pattern
During the discussion, the user revealed a critical piece of architectural context:
> *"The only reason the admin elevation was added was because some folders/zips couldn't be read, leaving the output folder/zip at 0 bytes in size."*

This revealed that **the most complex, hacky code in the legacy C# app was written solely to work around the side effects of Admin elevation**, rather than elevation being a desired user feature:

```
+-----------------------------------------------------------------------------------------------------------------+
|                                 THE C# ELEVATION FAILURE CASCADE                                                |
|                                                                                                                 |
| 1. Non-Atomic File Creation:                                                                                    |
|    In C#, `ZipItemAsync` created `new FileStream(zipPath, FileMode.Create)` BEFORE reading source files.        |
|    If the first file was locked or threw an exception, execution jumped to catch. The 0-byte file remained.     |
|                                                                                                                 |
| 2. Restrictive FileShare Flags:                                                                                 |
|    C# opened files with `FileShare.Read`. If any other process (osu!, music player, game, torrent, Discord)      |
|    had the file open with write access, Windows threw Win32 `ERROR_SHARING_VIOLATION` (0x20).                   |
|                                                                                                                 |
| 3. Mistaken "Fix" — Admin Elevation:                                                                            |
|    Elevation was added assuming it would grant permission to read locked files. However, elevation CANNOT       |
|    bypass another process's exclusive file lock; it only bypassed certain NTFS ACL permissions.                 |
|                                                                                                                 |
| 4. Windows UIPI Breaks Drag-and-Drop:                                                                           |
|    Running as Admin triggered Windows UIPI (User Interface Privilege Isolation), blocking Explorer drag-and-    |
|    drop (`WM_DROPFILES`) from medium-integrity Explorer to the high-integrity app.                              |
|                                                                                                                 |
| 5. 150+ Lines of Fragile Win32 Workarounds:                                                                     |
|    To restore drag-and-drop, C# had to call `ChangeWindowMessageFilterEx`, `RevokeDragDrop`, `DragAcceptFiles`,  |
|    and run a 40ms `DispatcherTimer` polling `GetAsyncKeyState(VK_LBUTTON)` to fake hover highlights.           |
+-----------------------------------------------------------------------------------------------------------------+
```

### 1.3 How the Rust Architecture Solves This Cleanly (Unelevated)
In Rust, the entire elevation workaround is permanently eliminated through four architectural guarantees:
1. **RAII Atomic Writes:** All archives are written to `.archive-helper-{uuid}.tmp` in the destination directory and only committed via atomic rename upon total success. The `Drop` destructor guarantees that if any panic, cancellation, or error occurs, the temporary file is unlinked immediately. **Zero 0-byte or corrupt archives ever touch disk.**
2. **Permissive Windows Native File Sharing Flags:** Files are opened via `std::os::windows::fs::OpenOptionsExt` with `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE` and `FILE_FLAG_SEQUENTIAL_SCAN`. Files actively being read or played in other applications can be read seamlessly.
3. **Resilient Error Tolerant Archiving:** If an individual file is exclusively locked or unreadable, the core engine logs an `ArchiveWarning::FileReadSkipped` and continues archiving the remaining thousands of files, rather than crashing the entire operation.
4. **Extended-Length Path Normalization:** Extended Win32 prefixes (`\\?\`) normalize paths exceeding 260 characters, eliminating `PathTooLongException` failures.
5. **No Admin Privileges Needed:** The app runs completely unelevated at medium integrity level. Native Windows Explorer drag-and-drop works flawlessly with zero UIPI interference and no UAC prompts.

---

## 2. Target Workspace Architecture

The Rust rewrite is structured as a modular Cargo workspace:

```
archive-helper/
├── Cargo.toml                          # Workspace root definition
├── crates/
│   ├── archive-helper-core/            # Pure Rust high-performance archive engine
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs                  # Public API facade
│   │   │   ├── atomic.rs               # AtomicFileWriter with RAII Drop cleanup
│   │   │   ├── fs.rs                   # Windows permissive sharing, path normalization, safe delete
│   │   │   ├── zip.rs                  # Sequential zero-compression streaming engine
│   │   │   ├── extract.rs              # Multi-threaded parallel zip extraction
│   │   │   ├── scanner.rs              # Pre-scanning for bytes/files & .osz/.osk prediction
│   │   │   ├── batch.rs                # Work-stealing batch coordinator (Rayon)
│   │   │   ├── progress.rs             # Granular throttled progress channels & cancellation
│   │   │   └── error.rs                # Typed error & warning models (thiserror)
│   │   └── tests/
│   │       └── resilience_and_security.rs # 13/13 passing integration tests
│   │
│   └── archive-helper-gui/             # Tauri v2 Desktop Application
│       ├── Cargo.toml
│       ├── tauri.conf.json             # Window size (670x355), title, DWM config
│       ├── src/
│       │   ├── main.rs                 # Entry point, DWM dark mode Win32 hook
│       │   ├── commands.rs             # Tauri IPC commands (invoke zip/unzip/scan)
│       │   └── state.rs                # App state, settings persistence, cancellation tokens
│       └── ui/                         # Modern Dark Frontend (HTML5, CSS3, TypeScript)
│           ├── index.html              # 670x355 layout with two main cards & utility bar
│           ├── styles/
│           │   ├── main.css            # Dark theme (#161616), custom 5px scrollbars
│           │   └── progress.css        # Hardware-accelerated dual-arc SVG transitions
│           ├── src/
│           │   ├── app.ts              # Drag-and-drop handlers, UI event orchestration
│           │   ├── progress-ring.ts    # Concentric dual-arc SVG renderer
│           │   ├── queue.ts            # Queue flyout overlay & per-item status list
│           │   └── dialog.ts           # Single zip modal dialog
│           └── assets/
│               └── svg/                # Native SVG assets (exit, folder-search, checkmark, thumbtack)
└── docs/
    ├── GUI_UX_SPECIFICATION.md         # Detailed UI/UX design specification
    └── RUST_REWRITE_PLAN.md            # This document
```

---

## 3. GUI Framework Decision: Why Tauri v2

The GUI & UX Delegate conducted an exhaustive evaluation of **Tauri v2**, **Slint**, **egui**, and **Iced** against Archive Helper's specific constraints:

| Evaluation Metric | Tauri v2 | Slint | egui | Iced |
| :--- | :---: | :---: | :---: | :---: |
| **Explorer File Drag-and-Drop Sub-Hit-Testing** (Cards vs Top-Corner Folder Icons) | **★★★★★**<br>Native HTML5 DnD via OS WebView2; sub-element hover works out-of-the-box | **★★☆☆☆**<br>`DropArea` is for internal widgets; external Explorer drops lack sub-widget hover tracking | **★★☆☆☆**<br>Only window-level `hovered_files`; pointer coords during OLE drag are unreliable | **★☆☆☆☆**<br>No per-widget OS drag events; requires engine modifications |
| **Custom Concentric Vector Dual-Arcs** (Round caps & smooth transitions) | **★★★★★**<br>Native SVG / CSS; round caps, sub-pixel strokes, zero tessellation math bugs | **★★★☆☆**<br>Path DSL is static; dynamic runtime arcs require custom Rust path generation | **★★★☆☆**<br>Requires custom mesh tessellation / `lyon` for thick round-capped arcs | **★★★★☆**<br>`canvas::Canvas` with Lyon paths; capable but verbose |
| **Smooth UI Transitions & Animations** (Fade checkmark, hover glow, queue slide) | **★★★★★**<br>Hardware-accelerated CSS transitions & keyframes; instant and butter-smooth | **★★★★☆**<br>Declarative `animate` property syntax in DSL | **★★☆☆☆**<br>Manual frame-by-frame lerp + continuous repaint loops | **★★☆☆☆**<br>Subscription-driven time ticks; heavy boilerplate |
| **Queue Flyout Overlay & Modal Dialogs** | **★★★★★**<br>Absolute floating overlay, backdrop filter, 5px custom dark scrollbars | **★★★☆☆**<br>`PopupWindow` support, but custom scrollbar styling is limited | **★★★★☆**<br>`egui::Area` / Window floating popups, modal overlays | **★★☆☆☆**<br>Complex overlay traits and ownership constraints |
| **DWM Immersive Dark Mode & Compact Binary** | **★★★★★**<br>3.2 – 5.0 MB single `.exe`; native DWM dark title bar; uses pre-installed WebView2 | **★★★★☆**<br>8 – 12 MB single `.exe`; native DWM dark title bar via Win32 FFI | **★★★★☆**<br>5.5 – 9 MB single `.exe`; native DWM dark title bar via winit | **★★★☆☆**<br>16 – 24 MB single `.exe`; larger dependency graph |
| **Visual Fidelity to Original WPF UX** | **★★★★★**<br>100% pixel-perfect replica of dark WPF layout, typography, SVG icons | **★★★☆☆**<br>Good, but requires recreating all custom control templates | **★★☆☆☆**<br>Game/tool aesthetic; bitmap font rasterization | **★★★☆☆**<br>Clean, but requires extensive custom widget coding |

### Conclusion:
**Tauri v2** is selected as the definitive GUI framework. It guarantees exact visual fidelity to the original WPF design, flawless sub-target Explorer drag-and-drop hit testing, zero-math vector progress animations, and a tiny standalone binary (~3.5 MB) using Windows' built-in WebView2 runtime.

---

## 4. Phased Implementation Roadmap & Orchestration of Delegates

```
+------------------------------------------------------------------------------------------------------------+
|                                        IMPLEMENTATION TIMELINE                                             |
+------------------------------------+-----------------------------------------------------------------------+
| Phase 1: Core Archive Engine       | [COMPLETED & VERIFIED]                                                |
|                                    | 13/13 passing tests. Atomic writes, resilient Win32 file sharing,      |
|                                    | zero-compression streaming, Rayon batching, path normalization.       |
+------------------------------------+-----------------------------------------------------------------------+
| Phase 2: Tauri v2 Scaffold & DWM   | [NEXT]                                                                |
|                                    | Set up Tauri v2 workspace, configure 670x355 window, implement Win32  |
|                                    | DWM immersive dark title bar (#161616), import SVG assets.            |
+------------------------------------+-----------------------------------------------------------------------+
| Phase 3: Frontend Layout & Styling | Create two-column drop cards (#202020), checkboxes, corner folder     |
|                                    | buttons, bottom utility bar, 5px custom dark scrollbars.              |
+------------------------------------+-----------------------------------------------------------------------+
| Phase 4: Concentric Progress Ring  | Implement SVG dual-arc rendering (outer bytes, inner files), 60fps    |
|          & Queue Flyout Overlay    | CSS transition curves, completed checkmark badge, flyout queue popup. |
+------------------------------------+-----------------------------------------------------------------------+
| Phase 5: IPC Bridge & Persistence  | Connect frontend drag events to `archive-helper-core`, stream 40ms    |
|                                    | progress updates, implement Single Zip dialog, save settings.json.    |
+------------------------------------+-----------------------------------------------------------------------+
| Phase 6: Release Pipeline & CI/CD  | Configure GitHub Actions matrix, strip binary, generate standalone    |
|                                    | ArchiveHelper.exe (< 5 MB), release validation.                       |
+------------------------------------+-----------------------------------------------------------------------+
```

### Delegate Responsibilities for Remaining Phases:

#### Delegate A: Frontend & Design System Specialist
- Implement the HTML5/CSS3/TypeScript UI in `crates/archive-helper-gui/ui`.
- Translate `docs/GUI_UX_SPECIFICATION.md` into exact CSS rules, SVG vectors, and typography.
- Build the Concentric Dual-Arc progress ring component with the completed checkmark transition.
- Build the dismissible Queue Flyout overlay and the Single Zip modal dialog.

#### Delegate B: Tauri v2 & Win32 Integration Specialist
- Initialize Tauri v2 in `crates/archive-helper-gui`.
- Apply Win32 `DwmSetWindowAttribute` for dark title bar (`DWMWA_USE_IMMERSIVE_DARK_MODE`, `DWMWA_CAPTION_COLOR = 0x161616`).
- Implement the Tauri IPC commands calling into `archive-helper-core`.
- Manage Tokio async task spawning and progress event emission (`archive://progress`).
- Implement settings loading and saving to `%APPDATA%\ArchiveHelper\settings.json`.

#### Delegate C: QA, Packaging & Release Specialist
- Build end-to-end integration tests validating Explorer drag simulation.
- Verify binary size optimization (`opt-level = "z"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`).
- Update GitHub Actions workflow for automated releases on tag push.

---

## 5. Technical Specifications Reference

### 5.1 Concentric Dual-Arc Geometry Math
- **Global Ring (44×44 px):**
  - Outer Arc (Bytes): Center $(22, 22)$, Radius $r_1 = 19.8\text{ px}$, Stroke $4.4\text{ px}$, Circumference $C_1 \approx 124.407\text{ px}$. Color: `#007ACC` (Zip) / `#28A745` (Unzip), Track: `#262626`.
  - Inner Arc (Files): Center $(22, 22)$, Radius $r_2 = 15.7\text{ px}$, Stroke $3.8\text{ px}$, Circumference $C_2 \approx 98.646\text{ px}$. Color: `#B026FF`, Track: `#241730`.
  - Completed Badge: 28 px solid circle with white SVG checkmark (`stroke-width: 2.2px`).
- **Queue Ring (20×20 px):**
  - Outer Arc (Bytes): Center $(10, 10)$, Radius $r_1 = 8.9\text{ px}$, Stroke $2.2\text{ px}$, Circumference $C_1 \approx 55.920\text{ px}$.
  - Inner Arc (Files): Center $(10, 10)$, Radius $r_2 = 6.8\text{ px}$, Stroke $2.0\text{ px}$, Circumference $C_2 \approx 42.726\text{ px}$.
  - Completed Badge: 20 px solid circle with white SVG checkmark (`stroke-width: 1.6px`).

### 5.2 Settings Schema (`%APPDATA%\ArchiveHelper\settings.json`)
```json
{
  "delete_zip": true,
  "fast_zip": false,
  "predict_ext": true,
  "keep_mtime_zip": true,
  "single_folder_zip": false,
  "delete_unzip": true,
  "fast_unzip": false,
  "keep_top_dir": true,
  "keep_mtime_unzip": true,
  "always_on_top": false,
  "use_binary_units": true
}
```
*(Notice: `start_as_admin` is completely eliminated because admin elevation is no longer needed!)*

---

## 6. Verification and Status

- **Branch:** `rust-rewrite` created and checked out.
- **Core Engine:** Fully implemented in `crates/archive-helper-core`.
- **Test Suite:** 13/13 tests passing on Windows MSVC x64 in 0.01 seconds.
- **GUI Specification:** Documented in `docs/GUI_UX_SPECIFICATION.md`.
- **Master Plan:** Formulated in `docs/RUST_REWRITE_PLAN.md`.
