# TO DO LIST

- Feature: setting for what left click on the tray icon should do.

- Feature: warn when not all window filters could be applied successfully.

- Auto start as regular user.

- Auto update.

  - Context menu item to check for update.

  - CLI command to update.

- Click on error notifications to show message box with the full error.

- Look into `human-panic` and `better-panic`.

- Bug: Smooth switching using the mouse in the context menu doesn't always work.

  - Maybe add a delay after clicking before the switch is preformed.

  - Could be because of the animations in the context menu, they don't seem to play when using the access keys (which don't have this problem).

  - Integration tests to check that smoothly switching desktop works consistently.

- Show desktop name in tray tooltip and maybe in context menu.

- Consider implementing auto Start using [planif - Rust](https://docs.rs/planif/latest/planif/), a wrapper around Task Scheduler APIs in `windows-rs`.

- Filters: use window titles and process names to determine where to move windows.

  - This Rust project interacts with visible windows on Windows: [switche/switche-back/src/win_apis.rs at a27522fc86c011c9c79dc95aed904c3a4cecacb9 · yakrider/switche](https://github.com/yakrider/switche/blob/a27522fc86c011c9c79dc95aed904c3a4cecacb9/switche-back/src/win_apis.rs)

  - Allow using scripts to control the target window.

    - Embed a small JavaScript runtime.

      - Can we also transpile TypeScript?

    - Embed a LUA runtime (should have small binary size).

- Global hotkeys.

  - global-hotkey = "0.5.3"

- Write abstraction layer over the GUI toolkit so that we can more easily switch without rewriting too much logic.

  - Maybe take inspiration from Firefox's crash reporter (which was rewritten in Rust).

    - Blog: [Porting a cross-platform GUI application to Rust - Mozilla Hacks - the Web developer blog](https://hacks.mozilla.org/2024/04/porting-a-cross-platform-gui-application-to-rust/)

    - Windows code for their layout algorithm: [layout.rs - mozsearch](https://searchfox.org/mozilla-central/rev/47a0a01e1f7ad0451c6ba6c790d5c6855df512c1/toolkit/crashreporter/client/app/src/ui/windows/layout.rs)

  - Maybe don't write a "meta" UI framework, instead separate the program into these parts:

    - A simple abstraction for the tray and context menu since they don't expose that many features.

    - Reusable logic for the configuration window.

    - Use native-windows-gui to implement the tray and context menu abstraction as well as to implement a configuration window.

    - Use winsafe to implement the tray and context menu abstraction as well as to implement a configuration window.

    - Use tray-icon to implement the tray and context menu abstraction.

    - Use Slint to implement the configuration window.

    - Now we can pick and choose a configuration window UI and a tray backend.
