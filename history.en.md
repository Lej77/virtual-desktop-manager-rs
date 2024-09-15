# History

- master/HEAD
- 0.1.3 (2024-09-15)
  - Fix: more robust loading of initial desktop count.
  - Fix: refocus last window when switching using animations.
- 0.1.2 (2024-09-12)
  - Fix: recheck virtual desktop count if it is 1 at startup since that might be incorrect.
  - Fix: allow recursive events so that "space bar" shortcut in quick switch context menu works.
  - Fix: don't show empty desktop names in tray tooltip.
  - Feature: indent desktop name in tray tooltip.
  - Feature: "smooth desktop switch" options works for Windows 11.
- 0.1.1 (2024-08-08)
  - Fix: recheck virtual desktop count if it fails at startup.
  - Fix: normalize strings from `\r\n` to `\n` for "window title" and "process name" fields of filters/rules.
- 0.1.0 (2024-08-05)
  - First release with most features from the original C# program.
