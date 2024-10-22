# virtual-desktop-manager-rs

<!-- Badge style inspired by https://github.com/dnaka91/advent-of-code/blob/de37024ba3b385694e14f79c849370c0f605f054/README.md -->

<!-- [![Build Status][build-img]][build-url] -->
[![Documentation][doc-img]][doc-url]

<!--
[build-img]: https://img.shields.io/github/actions/workflow/status/Lej77/virtual-desktop-manager-rs/ci.yml?branch=main&style=for-the-badge
[build-url]: https://github.com/Lej77/virtual-desktop-manager-rs/actions/workflows/ci.yml
 -->
<!-- https://shields.io/badges/static-badge -->
[doc-img]: https://img.shields.io/badge/docs.rs-virtual_desktop_manager-4d76ae?style=for-the-badge
[doc-url]: https://lej77.github.io/virtual-desktop-manager-rs/virtual_desktop_manager/index.html

This is a Win32 application implemented using [`native-windows-gui`] that runs as a tray icon and uses a [fork](https://github.com/Lej77/VirtualDesktopAccessor) of [Ciantic/VirtualDesktopAccessor - `winvd` library with virtual desktop bindings for Rust](https://github.com/Ciantic/VirtualDesktopAccessor) to move windows between different virtual desktops on Windows 10 and Windows 11.

Note that this program started out as a rewrite of the C# program [Lej77/VirtualDesktopManager: A WinForms application that can move windows between different virtual desktops](https://github.com/Lej77/VirtualDesktopManager).

[`native-windows-gui`]: https://github.com/gabdube/native-windows-gui

## Features of the original C# program

Not all of the original features are implemented in the Rust rewrite:

- [x] Tray icon shows one-based index of current virtual desktop.

  - The icons were adapted from another project at: [m0ngr31/VirtualDesktopManager](https://github.com/m0ngr31/VirtualDesktopManager)

- [x] Left click tray icon to open a configuration window where you can setup rules for automatically moving windows to specific virtual desktops.

  - [x] Window titles and process names can be used to determine what windows to move.

  - [x] Hint: you can double click on "filters" (rules) to select them in the right sidebar, then you can easily change the filter's options.

  - [x] When specifying a rules "window title" or "process name" there are text boxes with multiple lines. To match a name verbatim use only a single line. The program is made to allow multiple mismatched characters between each lines' text.

  - [ ] The "Root Parent Index" and "Parent Index" columns are not very useful.

- [x] Middle click tray icon to apply the configured rules and automatically move windows.

- [x] Right click tray icon to open the context menu and switch to a different virtual desktop.

  - [x] There is an option for "smooth" switching where animations are used when transitions to the traget desktop. This is implemented by opening an invisible window, moving it to the target desktop and then focusing it.

  - [ ] The textbox in the top of the context menu allows writing the index of a target desktop to easily switch to it.

    - [ ] There are also some other handy quick keys, such as "s" to toggle smooth switching or "+"/"-" to target a neighboring desktop.

- [x] The context menu has an option to "Stop flashing windows", this refers to window icons in the toolbar that can start flashing orange when a window wants your attention. Such taskbar icons are visibile on all virtual desktops and so the purpose of moving a window to another desktop is not quite achived. Therefore this program can stop such flashing.

  - [x] This feature can also be configured to be used every time the automatic window rules are applied.

- [x] The configuration window also has an option for the program to request admin permission when started. This is useful since otherwise the program can't move windows opened by processes that have admin permissions.

- [x] All settings are automatically saved to a settings file next to the executable. The rules specified in the configuration window can also be exported and imported manually.

- [ ] The program can also be used in "server" mode if started with the right command line flags, use the "--help" flag to see more information about command line usage.

  - The "server" mode allows re-using the program's code from a scripting language like "JavaScript" for more advanced usage. A default JavaScript/TypeScript client for the Deno JavaScript runtime is included inside the executable and can be emitted with the right command line flags.

## Differences from original C# program

- The Rust program continues to work after `explorer.exe` is restarted while the C# program could no longer interact with virtual desktops and would stop updating the current desktop index.

- The Rust program will detect if the taskbar is using a light theme and invert the colors on the tray icon.

- The Rust program has an alternative tray icon that only shows the current desktop index as the whole icon without any border or framing.

- The Rust program has better performance when finding information about all open windows.

- The Rust program has a setting to enable auto start.

- The Rust executable has inbuilt support for controlling virtual desktops but if a [`VirtualDesktopAccessor.dll`] file is found (by for example placing it next to the executable) then it will instead load that file and use it for controlling virtual desktops. Since the virtual desktop library need to be updated regularly to work with newer Windows versions this allows the executable to continue working without having to be recompiled by simply updating that DLL file.

  - Note that some operations might have worse performance when using the DLL file since not all features of the underlying library is exposed. Some differences are:

    - We need to regularly call the library to check if the number of virtual desktops have changed (i.e. to see if virtual desktops were created/deleted).

    - We don't get notified when virtual desktop names change. The desktop name is shown in the tooltip of the tray icon so this means that we can't cache desktop names and need to query them every time we change virtual desktop.

- The Rust program has a "quick switch" context menu as an alternative to the text field in the C# program's context menu.

  - Once the "quick switch" menu is open you can simply start entering the one-based index of the wanted desktop. If there are more than 10 desktops then `1` becomes ambiguous and could mean either `1` or `10`. In order to go to the first desktop there are several choices:

    - Configure an extra shortcut key in the configuration window's setting sidebar. With this you can for example use the `,` key to switch to the first desktop.

    - Press `space bar` after entering `1`. This will interpret the currently entered keys as the full number.

    - Start entering some leading `0` digits. This would mean entering `01` to switch to the first desktop.

- The Rust program doesn't determine if a window is the "main" window of a process. This wasn't very useful so this feature was never implemented.

- The Rust program seems to sometimes fail to smoothly switch current desktop on Windows 10 (that is to switch desktop using an animation) while the C# program seems to be less affected by this issue.

[`VirtualDesktopAccessor.dll`]: https://github.com/Ciantic/VirtualDesktopAccessor/releases/
